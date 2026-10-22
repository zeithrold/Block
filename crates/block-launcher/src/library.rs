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
use block_download::{DownloadOptions, EventSink, Store};
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
    storage: crate::storage::StorageUse,
}

impl Library {
    /// # Errors
    /// Fails if the catalog/cache cannot be opened.
    pub fn new(root: PathBuf, downloads: EventSink, events: ExecutionSink) -> anyhow::Result<Self> {
        Self::with_options(root, downloads, events, DownloadOptions::default())
    }

    /// Open the library with a download policy used by installs and repairs.
    ///
    /// # Errors
    /// Rejects invalid settings before opening the catalog, and reports cache,
    /// catalog, and HTTP-client initialization failures.
    pub fn with_options(
        root: PathBuf,
        downloads: EventSink,
        events: ExecutionSink,
        options: DownloadOptions,
    ) -> anyhow::Result<Self> {
        options.validate()?;
        let catalog = Catalog::open(&root)?;
        let storage = crate::storage::acquire(catalog.root())?;
        let store = Store::with_options(storage.cache.clone(), false, downloads, options)?;
        Ok(Self {
            catalog,
            executor: Executor::new(store.clone(), events.clone()),
            store,
            events,
            storage,
        })
    }
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    #[must_use]
    pub const fn download_options(&self) -> &DownloadOptions {
        self.store.options()
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
            Ok((path, optifine_mod)) => {
                target.manifest = Some(path);
                target.optifine_mod = optifine_mod;
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
    ) -> anyhow::Result<(PathBuf, Option<block_download::Resource>)> {
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
        let mut optifine_mod = None;
        if target.kind != LoaderKind::Vanilla {
            (self.events)(crate::executor::ExecutionEvent::Stage {
                name: format!(
                    "Installing {} {}",
                    target.kind.name(),
                    target.loader_version.as_deref().unwrap_or("")
                ),
            });
        }
        let mut manifest = if matches!(target.kind, LoaderKind::NeoForge | LoaderKind::Forge) {
            let java = match &prepared.plan.command.java {
                JavaExecutable::Managed { path } => {
                    crate::executor::confined_path(&prepared.root, path)?
                }
                JavaExecutable::Local { path } => path.clone(),
            };
            let version = target
                .loader_version
                .as_deref()
                .context("Loader build missing")?;
            let installed = if target.kind == LoaderKind::Forge {
                crate::install::forge::install(&metadata.parsed.manifest, version, &java, &context)
                    .await?
            } else {
                neoforge::install(&metadata.parsed.manifest, version, &java, &context).await?
            };
            installed.manifest_path
        } else if matches!(target.kind, LoaderKind::Fabric | LoaderKind::Quilt) {
            crate::install::fabric::install(
                &target.kind,
                target
                    .loader_version
                    .as_deref()
                    .context("Loader build missing")?,
                &metadata.parsed.manifest,
                &context,
            )
            .await?
        } else {
            let directory = self.store.root().join("installations").join(&instance.id);
            tokio::fs::create_dir_all(&directory).await?;
            let path = directory.join("vanilla.json");
            let bytes = serde_json::to_vec_pretty(&metadata.parsed.manifest)?;
            write_metadata(&path, &bytes)?;
            path
        };
        if let Some(selected) = &target.optifine {
            selected.validate_for(
                &instance.minecraft_version,
                &target.kind,
                target.loader_version.as_deref(),
            )?;
            let java = match &prepared.plan.command.java {
                JavaExecutable::Managed { path } => {
                    crate::executor::confined_path(&prepared.root, path)?
                }
                JavaExecutable::Local { path } => path.clone(),
            };
            (self.events)(crate::executor::ExecutionEvent::Stage {
                name: format!("Installing OptiFine {}", selected.version),
            });
            if target.kind == LoaderKind::Forge {
                optifine_mod = Some(
                    crate::install::optifine::forge_mod(
                        selected,
                        &instance.minecraft_version,
                        target.loader_version.as_deref(),
                        &context,
                    )
                    .await?,
                );
            } else {
                manifest = crate::install::optifine::install(
                    &metadata.parsed.manifest,
                    selected,
                    &java,
                    &context,
                )
                .await?;
            }
        }
        drop(prepared);
        // Verify/materialize the complete loader closure before marking the target ready.
        recipe.source = RecipeSource::LocalManifest {
            path: manifest.clone(),
        };
        let metadata = resolve_for_host(&mut recipe, &context).await?;
        let mut plan = compile(&recipe, &metadata)?;
        self.announce_compatibility(&plan);
        adapt(&mut plan, &metadata.parsed.manifest, target, &manifest)?;
        self.executor.prepare_repair(plan, cancel).await?;
        Ok((manifest, optifine_mod))
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
        let launch = self
            .prepare_launch(profile_id, session, cancel, None, None)
            .await?;
        LaunchExecutor::run(launch, self.events.clone(), cancel).await
    }

    /// Resolve an invocation inside the process which will own its lifetime and locks.
    /// # Errors
    /// Refuses stale session reservations, unavailable targets and corrupt installed resources.
    pub async fn prepare_launch(
        &self,
        profile_id: &str,
        session: &dyn SessionProvider,
        cancel: &CancellationToken,
        run_id: Option<&str>,
        profile_fingerprint: Option<&str>,
    ) -> anyhow::Result<LaunchPlan> {
        let state = self.catalog.snapshot()?;
        let (instance, profile, target) = state.selection(profile_id)?;
        if let Some(expected) = profile_fingerprint {
            ensure!(
                state.launch_fingerprint(profile_id)? == expected,
                "Launch settings changed while starting. Select the profile and click Play again."
            );
        }
        ensure!(
            target.status == InstallStatus::Ready,
            "installation is not ready; choose Repair before playing"
        );
        let manifest = target
            .manifest
            .as_ref()
            .context("installed metadata missing; choose Repair")?;
        let directory = match run_id {
            Some(id) => GameDirectoryGuard::for_session(instance, profile, id)?,
            None => GameDirectoryGuard::for_profile(instance, profile)?,
        };
        let mut recipe = recipe(
            instance,
            profile,
            RecipeSource::LocalManifest {
                path: manifest.clone(),
            },
        );
        let offline = Store::with_options(
            self.store.root().to_owned(),
            true,
            std::sync::Arc::new(|_| {}),
            self.store.options().clone(),
        )?;
        let context = ResolveContext {
            store: offline.clone(),
            cancel: cancel.clone(),
        };
        let metadata = resolve_for_host(&mut recipe, &context)
            .await
            .context("installation metadata unavailable; choose Repair")?;
        let mut plan = compile(&recipe, &metadata)?;
        self.announce_compatibility(&plan);
        apply_profile_arguments(&mut plan, instance, profile, &metadata.parsed.manifest)?;
        adapt(&mut plan, &metadata.parsed.manifest, target, manifest)?;
        let executor = Executor::new(offline, self.events.clone());
        let prepared = match run_id {
            Some(id) => executor.prepare_session(plan, cancel, id).await,
            None => executor.prepare(plan, cancel).await,
        }
        .context("installation integrity check failed; choose Repair")?;
        if target.kind == LoaderKind::Forge
            && let Some(selected) = &target.optifine
        {
            let resource = target
                .optifine_mod
                .as_ref()
                .context("OptiFine mod is not installed; choose Repair")?;
            let object = context.store.fetch(resource, cancel).await?;
            crate::install::optifine::place_mod(
                directory.directory(),
                selected,
                resource,
                &object,
            )?;
        }
        let mut launch = LaunchPlan::resolve(prepared, session, directory, host_overrides())?;
        launch.storage = Some(self.storage.lease.clone());
        Ok(launch)
    }

    /// Announce an applied compatibility substitution so launches never hide it.
    fn announce_compatibility(&self, plan: &ExecutionPlan) {
        if let Some(note) = &plan.compatibility {
            (self.events)(crate::executor::ExecutionEvent::Log {
                line: format!("Compatibility: {note}"),
                stderr: false,
            });
        }
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
        runtime: profile.effective_runtime(instance),
        target: None,
        compatibility: profile.compatibility,
        memory_mib: profile.memory_mib,
        jvm_args: profile.jvm_args.clone(),
        features: BTreeMap::new(),
    }
}

fn apply_profile_arguments(
    plan: &mut ExecutionPlan,
    instance: &Instance,
    profile: &Profile,
    manifest: &minecraft_manifest::model::VersionManifest,
) -> anyhow::Result<()> {
    let server = profile.server.launch_arguments(manifest)?;
    plan.command
        .game_args
        .extend(profile.effective_game_args(instance));
    plan.command.game_args.extend(server);
    Ok(())
}

fn adapt(
    plan: &mut ExecutionPlan,
    manifest: &minecraft_manifest::model::VersionManifest,
    target: &LaunchTarget,
    path: &Path,
) -> anyhow::Result<()> {
    if matches!(target.kind, LoaderKind::NeoForge | LoaderKind::Forge) {
        let receipt = path
            .parent()
            .context("loader metadata has no directory")?
            .join("installed.json");
        let installed: InstalledLoader = serde_json::from_slice(
            &std::fs::read(receipt).context("loader receipt missing; choose Repair")?,
        )?;
        ensure!(
            target.loader_version.as_deref() == Some(installed.version.as_str())
                && installed.manifest_path == path,
            "Installed loader target does not match the selected build; choose Repair"
        );
        if target.kind == LoaderKind::Forge {
            crate::install::forge::adapt_plan(plan, manifest, &installed)?;
        } else {
            neoforge::adapt_plan(plan, manifest, &installed)?;
        }
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

#[cfg(test)]
mod instance_defaults_tests {
    use crate::{
        catalog::{Catalog, InstanceConfiguration, InstanceSettings},
        platform::Target,
        recipe::{ParsedRecipe, RecipeSource, RuntimeSelection},
        resolver::ResolvedMetadata,
    };
    use std::{collections::BTreeMap, path::Path};

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "The integration follows persisted defaults through production compilation and installed-manifest Quick Play rejection."
    )]
    fn persisted_defaults_feed_the_real_recipe_and_compiled_launch_arguments() -> anyhow::Result<()>
    {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (instance, mut profile) =
            catalog.create_instance("Plan".into(), "1.20.6".into(), None, None)?;
        let java = std::env::current_exe()?;
        catalog.save_instance(
            &instance.id,
            InstanceConfiguration {
                name: instance.name.clone(),
                settings: InstanceSettings {
                    runtime: RuntimeSelection::Local { java: java.clone() },
                    width: 1410,
                    height: 810,
                    fullscreen: true,
                },
            },
        )?;
        profile.game_args = vec![
            "--width".into(),
            "999".into(),
            "--server".into(),
            "example.invalid".into(),
        ];
        catalog.save_profile(profile.clone())?;
        let state = catalog.snapshot()?;
        let (instance, profile, _) = state.selection(&profile.id)?;
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../minecraft-manifest/tests/fixtures/official");
        let index: minecraft_manifest::model::VersionIndex =
            minecraft_manifest::decode(&std::fs::read(fixtures.join("version_manifest_v2.json"))?)?;
        let entry = index
            .versions
            .iter()
            .find(|entry| entry.id == "1.20.6")
            .ok_or_else(|| anyhow::anyhow!("official fixture missing"))?;
        let fixture = fixtures
            .join("versions")
            .join(format!("{}.json", entry.sha1));
        let recipe = super::recipe(
            instance,
            profile,
            RecipeSource::LocalManifest {
                path: fixture.clone(),
            },
        );
        assert!(matches!(&recipe.runtime, RuntimeSelection::Local { java: path } if path == &java));
        let metadata = ResolvedMetadata {
            parsed: ParsedRecipe {
                manifest: minecraft_manifest::decode(&std::fs::read(fixture)?)?,
                manifest_sha1: "0".repeat(40),
            },
            target: Target::host()?,
            assets: minecraft_manifest::resources::AssetObjects {
                objects: BTreeMap::new(),
                virtual_layout: None,
                map_to_resources: None,
            },
            runtime: None,
            library_overrides: BTreeMap::new(),
            compatibility: None,
        };
        let mut plan = crate::plan::compile(&recipe, &metadata)?;
        super::apply_profile_arguments(&mut plan, instance, profile, &metadata.parsed.manifest)?;
        assert_eq!(
            plan.command
                .game_args
                .iter()
                .filter(|argument| argument.as_str() == "--width")
                .count(),
            1
        );
        assert!(plan.command.game_args.ends_with(&[
            "--server".into(),
            "example.invalid".into(),
            "--width".into(),
            "1410".into(),
            "--height".into(),
            "810".into(),
            "--fullscreen".into()
        ]));
        assert!(
            !plan
                .command
                .game_args
                .iter()
                .any(|argument| argument == "999")
        );
        // Persisted typed server preferences reach the same production argument
        // boundary after compilation; legacy raw arguments are kept only when
        // automatic Quick Play is disabled.
        let mut quick_play = profile.clone();
        quick_play
            .game_args
            .retain(|argument| argument != "--server" && argument != "example.invalid");
        quick_play.server = crate::catalog::ServerPreferences {
            address: "play.example.com:25566".into(),
            connect_on_launch: true,
        };
        catalog.save_profile(quick_play.clone())?;
        let restored = catalog.snapshot()?;
        let (instance, quick_play, _) = restored.selection(&quick_play.id)?;
        let mut plan = crate::plan::compile(&recipe, &metadata)?;
        super::apply_profile_arguments(&mut plan, instance, quick_play, &metadata.parsed.manifest)?;
        assert!(plan.command.game_args.ends_with(&[
            "--quickPlayMultiplayer".into(),
            "play.example.com:25566".into()
        ]));
        assert_eq!(
            plan.command
                .game_args
                .iter()
                .filter(|argument| argument.as_str() == "--quickPlayMultiplayer")
                .count(),
            1
        );
        // Versions that predate Quick Play receive the legacy server arguments.
        let legacy = minecraft_manifest::decode(&std::fs::read(fixtures.join("../1.12.2.json"))?)?;
        let mut legacy_plan = crate::plan::compile(&recipe, &metadata)?;
        super::apply_profile_arguments(&mut legacy_plan, instance, quick_play, &legacy)?;
        assert!(legacy_plan.command.game_args.ends_with(&[
            "--server".into(),
            "play.example.com".into(),
            "--port".into(),
            "25566".into()
        ]));
        // An official pre-release client accepts neither form; a failed
        // connection decision leaves the compiled plan untouched.
        let mut beta = legacy.clone();
        beta.kind = minecraft_manifest::model::VersionType::OldBeta;
        let mut beta_plan = crate::plan::compile(&recipe, &metadata)?;
        let before = beta_plan.command.game_args.clone();
        assert!(
            super::apply_profile_arguments(&mut beta_plan, instance, quick_play, &beta).is_err()
        );
        assert_eq!(
            beta_plan.command.game_args, before,
            "unsupported automatic connection leaves the compiled plan untouched"
        );
        Ok(())
    }
}
