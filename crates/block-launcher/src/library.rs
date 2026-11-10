//! Instance orchestration above the process-only executor.
use crate::{
    catalog::{
        Catalog, GameDirectoryGuard, InstallStatus, Instance, InstanceConfiguration,
        InstanceOwnership, LaunchTarget, LoaderKind, PackStatus, TargetSource,
    },
    executor::{ExecutionSink, Executor, ResourceLease},
    install::neoforge::{self, InstalledLoader},
    launch::{LaunchExecutor, LaunchPlan},
    manifest::model::VersionManifest,
    plan::{ExecutionPlan, JavaExecutable, compile_catalog as compile},
    recipe::{LaunchRecipe, RecipeSource},
    resolver::{ResolveContext, resolve},
    session::{OfflineSession, SessionProvider},
};
use anyhow::{Context as _, ensure};
use block_download::{DownloadOptions, EventSink, Store};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
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
        let descriptor = crate::modpack::descriptor::load(&instance.resource_directory)?;
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
        mut configuration: InstanceConfiguration,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Instance> {
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
        self.catalog.create_pack_instance(
            operation_id.to_owned(),
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
        let resources = Arc::new(ResourceLease::exclusive(self.store.root())?);
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
            let descriptor = crate::modpack::descriptor::load(&instance.resource_directory)?;
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
            self.install_target(instance, &target, cancel, resources)
                .await
        }
        .await;
        match result {
            Ok((path, optifine_mod)) => {
                target.source = TargetSource::Managed {
                    manifest: Some(path),
                };
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
        let resources = Arc::new(ResourceLease::exclusive(self.store.root())?);
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
        let result = self
            .install_target(instance, &target, cancel, resources)
            .await;
        match result {
            Ok((path, optifine_mod)) => {
                if matches!(target.source, TargetSource::Managed { .. }) {
                    target.source = TargetSource::Managed {
                        manifest: Some(path),
                    };
                }
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
        resources: Arc<ResourceLease>,
    ) -> anyhow::Result<(PathBuf, Option<block_download::Resource>)> {
        if let TargetSource::Existing { version_id } = &target.source {
            ensure!(
                instance.ownership == InstanceOwnership::Registered,
                "existing targets require a registered installation"
            );
            let context = ResolveContext {
                store: self.store.clone(),
                cancel: cancel.clone(),
            };
            let metadata = crate::resolver::repair_existing(instance, target, &context).await?;
            let mut plan = crate::plan::compile_existing(instance, &metadata)?;
            apply_instance_arguments(&mut plan, instance, &metadata.parsed.manifest)?;
            self.announce_compatibility(&plan);
            self.executor
                .prepare_repair_with_resources(plan, cancel, resources)
                .await?;
            let existing = metadata
                .existing
                .as_ref()
                .context("source resolution did not produce an installation receipt")?;
            let receipt = crate::executor::confined_path(
                self.store.root(),
                &format!("installations/{}/{}/existing.json", instance.id, target.id),
            )?;
            crate::executor::ensure_write_path(self.store.root(), &receipt)?;
            std::fs::create_dir_all(receipt.parent().context("receipt has no parent")?)?;
            write_metadata(&receipt, &serde_json::to_vec_pretty(&existing.receipt)?)?;
            return Ok((
                instance
                    .resource_directory
                    .join("versions")
                    .join(version_id)
                    .join(format!("{version_id}.json")),
                None,
            ));
        }
        let mut recipe = recipe(
            instance,
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
            .prepare_repair_with_resources(
                compile(&recipe, &metadata)?,
                cancel,
                Arc::clone(&resources),
            )
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
        self.executor
            .prepare_repair_with_resources(plan, cancel, resources)
            .await?;
        Ok((manifest, optifine_mod))
    }

    /// Session acquisition/refresh belongs to the caller. Launch never silently repairs or logs credentials.
    /// # Errors
    /// Missing/corrupt install resources have an explicit Repair path; game-directory conflicts fail before Java.
    pub async fn launch(
        &self,
        instance_id: &str,
        session: &dyn SessionProvider,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Option<i32>> {
        let launch = self
            .prepare_launch(instance_id, session, cancel, None, None)
            .await?;
        LaunchExecutor::run(launch, self.events.clone(), cancel).await
    }

    /// Resolve an invocation inside the process which will own its lifetime and locks.
    /// # Errors
    /// Refuses stale session reservations, unavailable targets and corrupt installed resources.
    pub async fn prepare_launch(
        &self,
        instance_id: &str,
        session: &dyn SessionProvider,
        cancel: &CancellationToken,
        run_id: Option<&str>,
        instance_fingerprint: Option<&str>,
    ) -> anyhow::Result<LaunchPlan> {
        let state = self.catalog.snapshot()?;
        let (instance, target) = state.selection(instance_id)?;
        let result = async {
        if let Some(expected) = instance_fingerprint {
            ensure!(
                state.launch_fingerprint(instance_id)? == expected,
                "Launch settings changed while starting. Select the instance and click Play again."
            );
        }
        ensure!(
            target.status == InstallStatus::Ready
                || (matches!(target.source, TargetSource::Existing { .. })
                    && target.status == InstallStatus::Pending),
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
            let descriptor = crate::modpack::descriptor::load(&instance.resource_directory)?;
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
            let baseline = instance
                .resource_directory
                .join(".block-pack")
                .join("baseline");
            let metadata = std::fs::symlink_metadata(&baseline)
                .context("the installed pack baseline is missing")?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "the installed pack baseline is not a plain directory"
            );
        }
        if matches!(target.source, TargetSource::Existing { .. }) {
            (|| {
                ensure!(
                    instance.resource_directory.is_dir()
                        && instance.resource_directory.canonicalize()? == instance.resource_directory,
                    "Existing resource directory changed"
                );
                Ok(())
            })().map_err(|error| crate::catalog::ExistingSourceFailure::mark(&instance.resource_directory, error))?;
        }
        let resources = Arc::new(ResourceLease::shared(self.store.root())?);
        let directory = match run_id {
            Some(id) => GameDirectoryGuard::for_session(instance, id)?,
            None => GameDirectoryGuard::for_instance(instance)?,
        };
        if instance.pack.is_some() {
            crate::modpack::initialize::initialize(instance, &directory, cancel)?;
        }
        let offline = Store::with_options(
            self.store.root().to_owned(),
            true,
            Arc::new(|_| {}),
            self.store.options().clone(),
        )?;
        let context = ResolveContext {
            store: offline.clone(),
            cancel: cancel.clone(),
        };
        let (metadata, mut plan) = match &target.source {
            TargetSource::Managed { .. } => {
                let manifest = managed_manifest(target)?;
                let mut recipe = recipe(
                    instance,
                    RecipeSource::LocalManifest {
                        path: manifest.to_owned(),
                    },
                );
                let metadata = resolve_for_host(&mut recipe, &context)
                    .await
                    .context("installation metadata unavailable; choose Repair")?;
                let mut plan = compile(&recipe, &metadata)?;
                adapt(&mut plan, &metadata.parsed.manifest, target, manifest)?;
                (metadata, plan)
            }
            TargetSource::Existing { .. } => {
                let metadata =
                    crate::resolver::resolve_existing(instance, target, &context).await?;
                let plan = crate::plan::compile_existing(instance, &metadata)?;
                (metadata, plan)
            }
        };
        self.announce_compatibility(&plan);
        apply_instance_arguments(&mut plan, instance, &metadata.parsed.manifest)?;
        let executor = Executor::new(offline, self.events.clone());
        let prepared = executor
            .prepare_installed(plan, cancel, run_id.map(str::to_owned), resources)
            .await
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
        }.await;
        if matches!(target.source, TargetSource::Existing { .. }) && !cancel.is_cancelled() {
            let status = match &result {
                Ok(_) => Some(InstallStatus::Ready),
                Err(error) => error
                    .downcast_ref::<crate::catalog::ExistingSourceFailure>()
                    .filter(|failure| failure.path.starts_with(&instance.resource_directory))
                    .map(|failure| InstallStatus::Failed {
                        message: failure.safe_message(&instance.resource_directory),
                    }),
            };
            if let Some(status) = status
                && let Err(_error) = self.catalog.record_existing_verification(instance, status)
            {
                // Publication is advisory to this captured plan. In particular,
                // deleting an Instance never revokes its accepted game lock.
                (self.events)(crate::executor::ExecutionEvent::Log {
                    line: "Existing verification status could not be saved; catalog metadata changed or is unavailable.".into(),
                    stderr: true,
                });
            }
        }
        result
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

fn recipe(instance: &Instance, source: RecipeSource) -> LaunchRecipe {
    LaunchRecipe {
        schema_version: 1,
        instance: instance.id.clone(),
        source,
        session: OfflineSession::Offline {
            username: "Preparation".into(),
        },
        runtime: instance.runtime.clone(),
        target: None,
        compatibility: instance.compatibility,
        memory_mib: instance.memory_mib,
        jvm_args: instance.jvm_args.clone(),
        features: BTreeMap::new(),
    }
}

fn apply_instance_arguments(
    plan: &mut ExecutionPlan,
    instance: &Instance,
    manifest: &VersionManifest,
) -> anyhow::Result<()> {
    let server = instance.server.launch_arguments(manifest)?;
    plan.command.game_args.extend(instance.launch_game_args());
    plan.command.game_args.extend(server);
    Ok(())
}

fn managed_manifest(target: &LaunchTarget) -> anyhow::Result<&Path> {
    let TargetSource::Managed { manifest } = &target.source else {
        anyhow::bail!("existing targets must resolve their source installation");
    };
    manifest
        .as_deref()
        .context("installed metadata missing; choose Repair")
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
        catalog::{Catalog, InstanceConfiguration},
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
        let instance = catalog.create_instance("Plan".into(), "1.20.6".into(), None, None)?;
        let java = std::env::current_exe()?;
        let mut configuration = InstanceConfiguration::from(&instance);
        configuration.runtime = RuntimeSelection::Local { java: java.clone() };
        configuration.width = 1410;
        configuration.height = 810;
        configuration.fullscreen = true;
        configuration.game_args = vec![
            "--width".into(),
            "999".into(),
            "--server".into(),
            "example.invalid".into(),
        ];
        catalog.save_instance(&instance.id, configuration)?;
        let state = catalog.snapshot()?;
        let (instance, _) = state.selection(&instance.id)?;
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
            existing: None,
        };
        let mut plan = crate::plan::compile(&recipe, &metadata)?;
        super::apply_instance_arguments(&mut plan, instance, &metadata.parsed.manifest)?;
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
        let mut quick_play = InstanceConfiguration::from(instance);
        quick_play
            .game_args
            .retain(|argument| argument != "--server" && argument != "example.invalid");
        quick_play.server = crate::catalog::ServerPreferences {
            address: "play.example.com:25566".into(),
            connect_on_launch: true,
        };
        catalog.save_instance(&instance.id, quick_play)?;
        let restored = catalog.snapshot()?;
        let (instance, _) = restored.selection(&instance.id)?;
        let mut plan = crate::plan::compile(&recipe, &metadata)?;
        super::apply_instance_arguments(&mut plan, instance, &metadata.parsed.manifest)?;
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
        super::apply_instance_arguments(&mut legacy_plan, instance, &legacy)?;
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
        assert!(super::apply_instance_arguments(&mut beta_plan, instance, &beta).is_err());
        assert_eq!(
            beta_plan.command.game_args, before,
            "unsupported automatic connection leaves the compiled plan untouched"
        );
        Ok(())
    }
}
