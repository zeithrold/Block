//! Managed Instance creation and pack operation idempotency.
use super::{
    Catalog, DataPolicy, InstallStatus, Instance, InstanceConfiguration, InstanceOwnership,
    LaunchTarget, LoaderKind, PackAttachment, PackStatus, TargetSource, normalize_directory,
    validate_id,
};
use crate::install::optifine::OptiFineJar;
use anyhow::ensure;
mod directory;
use directory::PreparedInstanceDirectory;
use std::path::PathBuf;

impl Catalog {
    /// Create a managed game using the default launch settings.
    /// # Errors
    /// Invalid configuration or persistence failures preserve existing data.
    pub fn create_instance(
        &self,
        name: String,
        minecraft_version: String,
        directory: Option<PathBuf>,
        loader_version: Option<String>,
    ) -> anyhow::Result<Instance> {
        let target_id = loader_version
            .as_ref()
            .map_or_else(|| "vanilla".into(), |version| format!("neoforge-{version}"));
        self.create_instance_configured(
            minecraft_version,
            directory,
            loader_version,
            InstanceConfiguration {
                name,
                target_id,
                ..Default::default()
            },
        )
    }

    /// # Errors
    /// Rejects invalid settings before creating the managed resource directory.
    pub fn create_instance_configured(
        &self,
        minecraft_version: String,
        directory: Option<PathBuf>,
        loader_version: Option<String>,
        configuration: InstanceConfiguration,
    ) -> anyhow::Result<Instance> {
        self.create_instance_with_loader(
            minecraft_version,
            directory,
            loader_version.map(|version| (LoaderKind::NeoForge, version)),
            configuration,
        )
    }

    /// # Errors
    /// Rejects unsupported loader identities and invalid Instance settings.
    pub fn create_instance_with_loader(
        &self,
        minecraft_version: String,
        directory: Option<PathBuf>,
        loader: Option<(LoaderKind, String)>,
        configuration: InstanceConfiguration,
    ) -> anyhow::Result<Instance> {
        self.create_instance_with_components(
            minecraft_version,
            directory,
            loader,
            None,
            configuration,
        )
    }

    /// # Errors
    /// Rejects incompatible components, invalid settings and failed atomic commits.
    pub fn create_instance_with_components(
        &self,
        minecraft_version: String,
        directory: Option<PathBuf>,
        loader: Option<(LoaderKind, String)>,
        optifine: Option<OptiFineJar>,
        configuration: InstanceConfiguration,
    ) -> anyhow::Result<Instance> {
        self.create_managed(
            None,
            minecraft_version,
            directory,
            loader,
            optifine,
            configuration,
        )
    }

    /// Commit a staged pack under its stable operation UUID.
    /// # Errors
    /// Rejects mismatching descriptors, reused identities and invalid configuration.
    pub fn create_pack_instance(
        &self,
        id: String,
        minecraft_version: String,
        directory: PathBuf,
        loader: Option<(LoaderKind, String)>,
        configuration: InstanceConfiguration,
    ) -> anyhow::Result<Instance> {
        validate_id(&id)?;
        let descriptor = crate::modpack::descriptor::load(&directory)?;
        ensure!(
            descriptor.minecraft_version == minecraft_version && descriptor.loader == loader,
            "staged pack metadata does not match the requested instance"
        );
        let declared = loader.as_ref().map_or_else(
            || "vanilla".into(),
            |(kind, version)| format!("{}-{version}", kind.name().to_lowercase()),
        );
        ensure!(
            configuration.target_id == declared,
            "pack target does not match its declared loader"
        );
        self.create_managed(
            Some(id),
            minecraft_version,
            Some(directory),
            loader,
            None,
            configuration,
        )
    }

    fn create_managed(
        &self,
        operation_id: Option<String>,
        minecraft_version: String,
        directory: Option<PathBuf>,
        loader: Option<(LoaderKind, String)>,
        optifine: Option<OptiFineJar>,
        mut configuration: InstanceConfiguration,
    ) -> anyhow::Result<Instance> {
        if let Some((kind, version)) = &loader {
            match kind {
                LoaderKind::NeoForge => {
                    crate::install::neoforge::validate_version(&minecraft_version, version)?
                }
                LoaderKind::Forge => {
                    crate::install::forge::validate_version(&minecraft_version, version)?
                }
                LoaderKind::Fabric | LoaderKind::Quilt => {
                    crate::install::fabric::validate_version(version)?
                }
                LoaderKind::Vanilla => anyhow::bail!("Vanilla does not have a loader version"),
            }
        }
        if let Some(selected) = &optifine {
            selected.validate_for(
                &minecraft_version,
                loader
                    .as_ref()
                    .map_or(&LoaderKind::Vanilla, |(kind, _)| kind),
                loader.as_ref().map(|(_, version)| version.as_str()),
            )?;
        }
        let storage = crate::storage::acquire(&self.root)?;
        let id = operation_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let resource_directory =
            normalize_directory(&directory.unwrap_or_else(|| storage.library.join(&id)))?;
        if configuration.game_directory.as_os_str().is_empty() {
            ensure!(
                configuration.data_policy == DataPolicy::Isolated,
                "Shared data requires an explicit existing game directory"
            );
            configuration.game_directory.clone_from(&resource_directory);
        } else {
            configuration.game_directory = normalize_directory(&configuration.game_directory)?;
        }
        storage.validate_game_location(&resource_directory)?;
        storage.validate_game_location(&configuration.game_directory)?;
        if configuration.data_policy == DataPolicy::Shared {
            ensure!(
                configuration.game_directory.is_dir(),
                "Shared data requires an existing game directory"
            );
        }
        let mut targets = vec![LaunchTarget {
            id: "vanilla".into(),
            kind: LoaderKind::Vanilla,
            loader_version: None,
            source: TargetSource::Managed { manifest: None },
            status: InstallStatus::Pending,
            optifine: if loader.is_none() {
                optifine.clone()
            } else {
                None
            },
            optifine_mod: None,
        }];
        if let Some((kind, version)) = loader {
            targets.push(LaunchTarget {
                id: format!("{}-{version}", kind.name().to_lowercase()),
                kind,
                loader_version: Some(version),
                source: TargetSource::Managed { manifest: None },
                status: InstallStatus::Pending,
                optifine,
                optifine_mod: None,
            });
        }
        let pack = operation_id.is_some().then(|| PackAttachment {
            target_id: configuration.target_id.clone(),
            status: PackStatus::Pending,
        });
        let instance = Instance {
            id,
            minecraft_version,
            resource_directory,
            ownership: InstanceOwnership::Managed,
            targets,
            compatibility: super::default_compatibility(),
            pack,
            name: configuration.name,
            target_id: configuration.target_id,
            runtime: configuration.runtime,
            memory_mib: configuration.memory_mib,
            jvm_args: configuration.jvm_args,
            game_args: configuration.game_args,
            default_account: configuration.default_account,
            appearance: configuration.appearance,
            server: configuration.server,
            width: configuration.width,
            height: configuration.height,
            fullscreen: configuration.fullscreen,
            data_policy: configuration.data_policy,
            game_directory: configuration.game_directory,
        };
        instance.validate()?;
        let mut prepared = None;
        let result = self.transaction(true, |state| {
            if operation_id.is_some()
                && let Some(existing) = state.instances.iter().find(|item| item.id == instance.id)
            {
                ensure!(
                    existing.ownership == InstanceOwnership::Managed
                        && existing.resource_directory == instance.resource_directory
                        && existing.game_directory == instance.game_directory
                        && existing.name == instance.name
                        && existing.minecraft_version == instance.minecraft_version
                        && existing.pack.as_ref().map(|pack| &pack.target_id)
                            == instance.pack.as_ref().map(|pack| &pack.target_id),
                    "pack operation ID already belongs to another instance"
                );
                return Ok(existing.clone());
            }
            ensure!(
                normalize_directory(&instance.resource_directory)? == instance.resource_directory,
                "resource directory changed before creation"
            );
            prepared = Some(PreparedInstanceDirectory::new(
                &instance.resource_directory,
            )?);
            ensure!(
                instance.resource_directory.canonicalize()? == instance.resource_directory,
                "resource directory changed while creating it"
            );
            state.instances.push(instance.clone());
            Ok(instance)
        })?;
        if let Some(prepared) = &mut prepared {
            prepared.committed = true;
        }
        Ok(result)
    }
}
