//! Concrete installation/profile orchestration above the process-only executor.
use crate::{
    catalog::{
        Catalog, GameDirectoryGuard, InstallStatus, Instance, LaunchTarget, LoaderKind, PackStatus,
        Profile, ProfileConfiguration,
    },
    executor::{ExecutionSink, Executor},
    install::neoforge::{self, InstalledLoader},
    launch::{LaunchExecutor, LaunchPlan},
    manifest::model::VersionManifest,
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

struct PackImportGuard(std::fs::File);

impl Drop for PackImportGuard {
    fn drop(&mut self) {
        // A concurrent spawn can inherit the open description until exec.
        // End import ownership now, even while that descriptor is still open.
        if let Err(error) = fs2::FileExt::unlock(&self.0) {
            tracing::warn!(event = "pack_import_unlock_failed", %error);
        }
    }
}

impl PackImportGuard {
    fn acquire(parent: &Path, operation_id: &str) -> anyhow::Result<Self> {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join(format!(".block-pack-{operation_id}.lock")))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("this pack import is already in progress")?;
        Ok(Self(lock))
    }
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

    /// Inspect a local archive and recover the selected optional entries when
    /// resuming an existing pack installation.
    ///
    /// # Errors
    /// Rejects invalid archives, missing pack metadata and a different pack.
    pub fn inspect_pack(
        &self,
        source: &Path,
        instance_id: Option<&str>,
    ) -> anyhow::Result<(
        crate::modpack::PackInspection,
        crate::modpack::PackSelection,
    )> {
        let mut inspection = crate::modpack::inspect(source)?;
        let Some(instance_id) = instance_id else {
            return Ok((inspection, crate::modpack::PackSelection::default()));
        };
        let state = self.catalog.snapshot()?;
        let instance = state
            .instances
            .iter()
            .find(|item| item.id == instance_id)
            .context("installation no longer exists")?;
        ensure!(instance.pack.is_some(), "instance has no pack");
        let descriptor = crate::modpack::descriptor::load(&instance.directory)?;
        ensure!(
            descriptor.matches_inspection(&inspection),
            "the selected archive does not match this pack; select the original pack"
        );
        if inspection.format == crate::modpack::PackFormat::Plain {
            inspection.select_target(
                descriptor.minecraft_version.clone(),
                descriptor.loader.clone(),
            )?;
        }
        let selection = crate::modpack::PackSelection {
            optional_files: descriptor
                .files
                .iter()
                .filter(|file| !file.required && file.selected)
                .map(|file| file.path.clone())
                .collect(),
            optional_curse_files: descriptor
                .missing_files
                .iter()
                .filter(|file| !file.required && file.selected)
                .map(|file| (file.project_id, file.file_id))
                .collect(),
        };
        Ok((inspection, selection))
    }

    /// Stage a local pack and commit exactly one new Instance for this operation.
    ///
    /// Reusing the same operation ID resumes a published but uncommitted pack.
    /// Source URLs are used only while the selected archive is open.
    ///
    /// # Errors
    /// Rejects changed or unsafe archives, stale selections, conflicting
    /// operation IDs, cancellation, and storage failures.
    pub fn create_pack(
        &self,
        operation_id: &str,
        source: &Path,
        inspected: &crate::modpack::PackInspection,
        selection: &crate::modpack::PackSelection,
        mut configuration: ProfileConfiguration,
        cancel: &CancellationToken,
    ) -> anyhow::Result<(Instance, Profile)> {
        let id = uuid::Uuid::parse_str(operation_id)?;
        ensure!(id.to_string() == operation_id, "invalid pack operation ID");
        ensure!(!cancel.is_cancelled(), "pack import cancelled");
        ensure!(
            inspected.target_confirmed(),
            "select the pack's Minecraft and loader target"
        );
        crate::catalog::validate_name(&configuration.name)?;
        let minecraft_version = inspected
            .minecraft_version
            .clone()
            .context("pack Minecraft version is missing")?;
        let parent = &self.storage.library;
        std::fs::create_dir_all(parent)?;
        let _guard = PackImportGuard::acquire(parent, operation_id)?;
        let destination = parent.join(operation_id);
        match std::fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "pack destination already exists; it was preserved"
                );
                let existing = crate::modpack::descriptor::load(&destination)
                    .context("pack destination already exists; it was preserved")?;
                let expected = crate::modpack::descriptor::from_inspection(inspected, selection)?;
                ensure!(
                    existing == expected,
                    "pack destination belongs to a different import; it was preserved"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let stage = tempfile::Builder::new()
                    .prefix(".block-pack-instance-")
                    .tempdir_in(parent)?;
                crate::modpack::descriptor::stage(
                    source,
                    inspected,
                    selection,
                    stage.path(),
                    cancel,
                )?;
                ensure!(!cancel.is_cancelled(), "pack import cancelled");
                let stage = stage.keep();
                match std::fs::symlink_metadata(&destination) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Ok(_) => anyhow::bail!("pack destination already exists; it was preserved"),
                    Err(error) => return Err(error.into()),
                }
                std::fs::rename(stage, &destination)?;
            }
            Err(error) => return Err(error.into()),
        }
        configuration.target_id = inspected.loader.as_ref().map_or_else(
            || "vanilla".to_owned(),
            |(kind, version)| format!("{}-{version}", kind.name().to_lowercase()),
        );
        let name = configuration.name.clone();
        self.catalog.create_pack_instance(
            operation_id.to_owned(),
            name,
            minecraft_version,
            destination,
            inspected.loader.clone(),
            configuration,
        )
    }

    /// Resume a pack install using the original, locally selected archive.
    ///
    /// # Errors
    /// Leaves the target retryable on incomplete files, mismatched source,
    /// cancellation or install failures, and never modifies played game data.
    pub async fn install_pack(
        &self,
        instance_id: &str,
        source: &Path,
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        let state = self.catalog.snapshot()?;
        let instance = state
            .instances
            .iter()
            .find(|item| item.id == instance_id)
            .context("installation no longer exists")?;
        let attachment = instance.pack.as_ref().context("instance has no pack")?;
        let mut target = instance
            .targets
            .iter()
            .find(|item| item.id == attachment.target_id)
            .context("pack launch target is missing")?
            .clone();
        target.status = InstallStatus::Installing;
        self.catalog.set_target(instance_id, &target)?;
        let result = async {
            let descriptor = crate::modpack::descriptor::load(&instance.directory)?;
            ensure!(
                !descriptor.has_missing_selected(),
                "pack files cannot be verified from their CurseForge IDs"
            );
            ensure!(
                descriptor.minecraft_version == instance.minecraft_version
                    && descriptor.loader.as_ref().map_or(
                        target.kind == LoaderKind::Vanilla,
                        |(kind, version)| {
                            *kind == target.kind
                                && target.loader_version.as_deref() == Some(version)
                        }
                    ),
                "pack target does not match its declared loader"
            );
            let inspection = crate::modpack::inspect(source)?;
            crate::modpack::prepare::prepare(
                &self.store,
                instance,
                &descriptor,
                &inspection,
                cancel,
            )
            .await?;
            self.install_target(instance, &target, cancel).await
        }
        .await;
        match result {
            Ok((path, optifine_mod)) => {
                target.manifest = Some(path);
                target.optifine_mod = optifine_mod;
                target.status = InstallStatus::Ready;
                self.catalog.finish_pack_install(instance_id, &target)
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
        if let Some(pack) = &instance.pack
            && pack.target_id == target.id
        {
            ensure!(
                pack.status == PackStatus::Ready,
                "pack installation is incomplete; reselect the original archive to resume"
            );
        }
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
        if let Some(pack) = &instance.pack {
            ensure!(
                pack.status == PackStatus::Ready,
                "pack installation is not ready"
            );
            ensure!(
                target.id == pack.target_id,
                "this target does not match the pack's declared loader"
            );
            let descriptor = crate::modpack::descriptor::load(&instance.directory)?;
            ensure!(
                !descriptor.has_missing_selected()
                    && descriptor.minecraft_version == instance.minecraft_version
                    && descriptor.loader.as_ref().map_or(
                        target.kind == LoaderKind::Vanilla,
                        |(kind, version)| {
                            *kind == target.kind
                                && target.loader_version.as_deref() == Some(version)
                        }
                    ),
                "the installed pack metadata does not match its launch target"
            );
            let baseline = instance.directory.join(".block-pack").join("baseline");
            let metadata = std::fs::symlink_metadata(&baseline)
                .context("the installed pack baseline is missing")?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "the installed pack baseline is not a plain directory"
            );
        }
        let manifest = target
            .manifest
            .as_ref()
            .context("installed metadata missing; choose Repair")?;
        let directory = match run_id {
            Some(id) => GameDirectoryGuard::for_session(instance, profile, id)?,
            None => GameDirectoryGuard::for_profile(instance, profile)?,
        };
        if instance.pack.is_some() {
            crate::modpack::initialize::initialize(instance, &directory, cancel)?;
        }
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

    /// Announce applied substitutions so launches never hide them.
    fn announce_compatibility(&self, plan: &ExecutionPlan) {
        if let Some(note) = &plan.compatibility {
            (self.events)(crate::executor::ExecutionEvent::Log {
                line: format!("Compatibility: {note}"),
                stderr: false,
            });
        }
        if let Some(note) = &plan.runtime_substitution {
            (self.events)(crate::executor::ExecutionEvent::Log {
                line: format!("Runtime: {note}"),
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
    manifest: &VersionManifest,
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
    manifest: &VersionManifest,
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

#[cfg(test)]
#[path = "library/pack_tests.rs"]
mod pack_tests;

async fn resolve_for_host(
    recipe: &mut LaunchRecipe,
    context: &ResolveContext,
) -> anyhow::Result<crate::resolver::ResolvedMetadata> {
    // Managed recipes stay managed: the resolver provisions the official Mojang
    // runtime, or the official Azul runtime when the catalog has no native ARM64
    // build. The old Linux ARM64 "/usr/bin/java" substitution bypassed that
    // reviewed provisioning and failed on hosts without a local Java 8.
    resolve(recipe, context).await
}

#[cfg(test)]
mod instance_defaults_tests {
    use crate::{
        catalog::{Catalog, InstanceConfiguration, InstanceSettings},
        manifest::{
            decode,
            model::{VersionIndex, VersionType},
            resources::AssetObjects,
        },
        platform::Target,
        recipe::{ParsedRecipe, RecipeSource, RuntimeSelection},
        resolver::ResolvedMetadata,
    };
    use std::{collections::BTreeMap, path::Path};

    #[test]
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
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifests/official");
        let index: VersionIndex =
            decode(&std::fs::read(fixtures.join("version_manifest_v2.json"))?)?;
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
                manifest: decode(&std::fs::read(fixture)?)?,
                manifest_sha1: "0".repeat(40),
            },
            // Compilation logic is host-independent; pin the reviewed fixture to
            // Linux x64 so a Linux ARM64 CI host does not require substitutions here.
            target: Target {
                os: crate::platform::OperatingSystem::Linux,
                arch: crate::platform::Architecture::X86_64,
                os_version: "6.0".into(),
            },
            assets: AssetObjects {
                objects: BTreeMap::new(),
                virtual_layout: None,
                map_to_resources: None,
            },
            runtime: None,
            library_overrides: BTreeMap::new(),
            companion_libraries: Vec::new(),
            classifier_overrides: BTreeMap::new(),
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
        let legacy = decode(&std::fs::read(fixtures.join("../1.12.2.json"))?)?;
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
        beta.kind = VersionType::OldBeta;
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
