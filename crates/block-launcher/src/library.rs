//! Concrete installation/profile orchestration above the process-only executor.
use crate::{
    catalog::{
        Catalog, GameDirectoryGuard, InstallStatus, Instance, LaunchTarget, LoaderKind, Profile,
    },
    executor::{ExecutionSink, Executor},
    install::neoforge::{self, InstalledLoader},
    launch::{LaunchExecutor, LaunchPlan},
    plan::{ExecutionPlan, JavaExecutable, compile},
    recipe::{LaunchRecipe, RecipeSource},
    resolver::{ResolveContext, resolve},
    session::{OfflineSession, SessionProvider},
};
use anyhow::{Context as _, ensure};
use block_download::{EventSink, Store};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Library {
    catalog: Catalog,
    store: Store,
    executor: Executor,
    events: ExecutionSink,
}

impl Library {
    /// # Errors
    /// Fails if the catalog/cache cannot be opened.
    pub fn new(root: PathBuf, downloads: EventSink, events: ExecutionSink) -> anyhow::Result<Self> {
        let catalog = Catalog::open(root.clone())?;
        let store = Store::new(root, false, downloads)?;
        Ok(Self {
            catalog,
            executor: Executor::new(store.clone(), events.clone()),
            store,
            events,
        })
    }
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Retry/repair checks original content hashes and only publishes ready targets after success.
    /// # Errors
    /// Preserves existing data and reports network/installer/Java errors with a retryable target state.
    pub async fn install(
        &self,
        instance_id: &str,
        target_id: &str,
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        let state = self.catalog.snapshot()?;
        let instance = state
            .instances
            .iter()
            .find(|i| i.id == instance_id)
            .context("installation no longer exists")?;
        let mut target = instance
            .targets
            .iter()
            .find(|t| t.id == target_id)
            .context("launch target missing")?
            .clone();
        target.status = InstallStatus::Installing;
        self.catalog.set_target(instance_id, &target)?;
        let result = self.install_target(instance, &target, cancel).await;
        match result {
            Ok(path) => {
                target.manifest = Some(path);
                target.status = InstallStatus::Ready;
                self.catalog.set_target(instance_id, &target)
            }
            Err(error) => {
                target.status = InstallStatus::Failed {
                    message: format!("{error:#}"),
                };
                self.catalog.set_target(instance_id, &target)?;
                Err(error)
            }
        }
    }

    async fn install_target(
        &self,
        instance: &Instance,
        target: &LaunchTarget,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PathBuf> {
        let state = self.catalog.snapshot()?;
        let profile = state
            .profiles
            .iter()
            .find(|p| p.instance_id == instance.id)
            .context("create a profile before installing")?;
        let mut recipe = recipe(
            instance,
            profile,
            RecipeSource::Mojang {
                version: instance.minecraft_version.clone(),
            },
        );
        let context = ResolveContext {
            store: self.store.clone(),
            cancel: cancel.clone(),
        };
        let metadata = resolve_for_host(&mut recipe, &context).await?;
        let prepared = self
            .executor
            .prepare_repair(compile(&recipe, &metadata)?, cancel)
            .await?;
        let manifest = if target.kind == LoaderKind::NeoForge {
            let java = match &prepared.plan.command.java {
                JavaExecutable::Managed { path } => {
                    crate::executor::confined_path(&prepared.root, path)?
                }
                JavaExecutable::Local { path } => path.clone(),
            };
            neoforge::install(&metadata.parsed.manifest, &java, &context)
                .await?
                .manifest_path
        } else {
            let directory = self.store.root().join("installations").join(&instance.id);
            tokio::fs::create_dir_all(&directory).await?;
            let path = directory.join("vanilla.json");
            let bytes = serde_json::to_vec_pretty(&metadata.parsed.manifest)?;
            write_metadata(&path, &bytes)?;
            path
        };
        drop(prepared);
        // Verify/materialize the complete loader closure before marking the target ready.
        recipe.source = RecipeSource::LocalManifest {
            path: manifest.clone(),
        };
        let metadata = resolve_for_host(&mut recipe, &context).await?;
        let mut plan = compile(&recipe, &metadata)?;
        adapt(&mut plan, &metadata.parsed.manifest, target, &manifest)?;
        self.executor.prepare_repair(plan, cancel).await?;
        Ok(manifest)
    }

    /// Session acquisition/refresh belongs to the caller. Launch never silently repairs or logs credentials.
    /// # Errors
    /// Missing/corrupt install resources have an explicit Repair path; game-directory conflicts fail before Java.
    pub async fn launch(
        &self,
        profile_id: &str,
        session: &dyn SessionProvider,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Option<i32>> {
        let state = self.catalog.snapshot()?;
        let (instance, profile, target) = state.selection(profile_id)?;
        ensure!(
            target.status == InstallStatus::Ready,
            "installation is not ready; choose Repair before playing"
        );
        let manifest = target
            .manifest
            .as_ref()
            .context("installed metadata missing; choose Repair")?;
        let directory = GameDirectoryGuard::for_profile(instance, profile)?;
        let mut recipe = recipe(
            instance,
            profile,
            RecipeSource::LocalManifest {
                path: manifest.clone(),
            },
        );
        let offline = Store::new(
            self.store.root().to_owned(),
            true,
            std::sync::Arc::new(|_| {}),
        )?;
        let context = ResolveContext {
            store: offline.clone(),
            cancel: cancel.clone(),
        };
        let metadata = resolve_for_host(&mut recipe, &context)
            .await
            .context("installation metadata unavailable; choose Repair")?;
        let mut plan = compile(&recipe, &metadata)?;
        plan.command.game_args.extend(profile.game_args.clone());
        adapt(&mut plan, &metadata.parsed.manifest, target, manifest)?;
        let executor = Executor::new(offline, self.events.clone());
        let prepared = executor
            .prepare(plan, cancel)
            .await
            .context("installation integrity check failed; choose Repair")?;
        let launch = LaunchPlan::resolve(prepared, session, directory, host_overrides())?;
        LaunchExecutor::run(launch, self.events.clone(), cancel).await
    }
}

fn recipe(instance: &Instance, profile: &Profile, source: RecipeSource) -> LaunchRecipe {
    LaunchRecipe {
        schema_version: 1,
        instance: instance.id.clone(),
        source,
        session: OfflineSession::Offline {
            username: "Preparation".into(),
        },
        runtime: profile.runtime.clone(),
        target: None,
        compatibility: profile.compatibility,
        memory_mib: profile.memory_mib,
        jvm_args: profile.jvm_args.clone(),
        features: BTreeMap::new(),
    }
}

fn adapt(
    plan: &mut ExecutionPlan,
    manifest: &minecraft_manifest::model::VersionManifest,
    target: &LaunchTarget,
    path: &Path,
) -> anyhow::Result<()> {
    if target.kind == LoaderKind::NeoForge {
        let receipt = path
            .parent()
            .context("loader metadata has no directory")?
            .join("installed.json");
        let installed: InstalledLoader = serde_json::from_slice(
            &std::fs::read(receipt).context("loader receipt missing; choose Repair")?,
        )?;
        neoforge::adapt_plan(plan, manifest, &installed)?;
    }
    Ok(())
}

fn write_metadata(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write as _;
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("metadata has no parent")?)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

fn host_overrides() -> BTreeMap<String, String> {
    ["LIBGL_ALWAYS_SOFTWARE", "GALLIUM_DRIVER", "ALSOFT_DRIVERS"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.into(), value)))
        .collect()
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("catalog", &self.catalog)
            .finish_non_exhaustive()
    }
}

async fn resolve_for_host(
    recipe: &mut LaunchRecipe,
    context: &ResolveContext,
) -> anyhow::Result<crate::resolver::ResolvedMetadata> {
    let system_fallback = cfg!(all(target_os = "linux", target_arch = "aarch64"))
        && matches!(recipe.runtime, crate::recipe::RuntimeSelection::Managed);
    if system_fallback {
        recipe.runtime = crate::recipe::RuntimeSelection::Local {
            java: PathBuf::from("/usr/bin/java"),
        };
    }
    let metadata = resolve(recipe, context).await?;
    if system_fallback {
        let major = metadata
            .parsed
            .manifest
            .java_version
            .as_ref()
            .map_or(8, |java| java.major_version);
        recipe.runtime = crate::recipe::RuntimeSelection::Local {
            java: crate::runtime::system_java(major)?,
        };
    }
    Ok(metadata)
}
