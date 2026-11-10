//! Direct registration reads source metadata and commits only catalog metadata.
use super::{
    Catalog, InstallStatus, Instance, InstanceConfiguration, InstanceOwnership, LaunchTarget,
    LoaderKind, TargetSource, normalize_directory, validate_version_id,
};
use crate::game_import::{GameCandidate, LoaderKind as DetectedLoaderKind};
use anyhow::{Context as _, ensure};

impl Catalog {
    /// Register an existing source without copying, installing or creating source directories.
    /// # Errors
    /// Rejects unsupported candidates, invalid metadata and policy/path mismatches.
    pub fn register_existing(
        &self,
        candidate: &GameCandidate,
        mut configuration: InstanceConfiguration,
    ) -> anyhow::Result<Instance> {
        ensure!(
            candidate.importable,
            "candidate is not importable: {}",
            candidate
                .unsupported_reason
                .as_deref()
                .unwrap_or("unresolved installation")
        );
        let version_id = candidate
            .version_id
            .as_deref()
            .context("candidate has no concrete version ID")?;
        validate_version_id(version_id)?;
        let resource_directory = candidate.resource_directory.canonicalize()?;
        ensure!(
            resource_directory.is_dir(),
            "resource directory is not a directory"
        );
        let offered = normalize_directory(&candidate.data_directory(configuration.data_policy)?)?;
        configuration.game_directory = normalize_directory(&configuration.game_directory)?;
        ensure!(
            configuration.game_directory == offered,
            "data policy and game directory do not match the offered candidate choice"
        );
        let manifest = resource_directory
            .join("versions")
            .join(version_id)
            .join(format!("{version_id}.json"));
        ensure!(
            manifest.canonicalize()? == manifest,
            "version metadata path contains an alias"
        );
        let value: serde_json::Value =
            serde_json::from_slice(&super::fingerprint::read_manifest(&manifest)?)?;
        ensure!(value.is_object(), "version metadata must be an object");
        if let Some(id) = value.get("id") {
            ensure!(id.as_str() == Some(version_id), "version ID mismatch");
        }
        if let Some(parent) = value.get("inheritsFrom") {
            validate_version_id(parent.as_str().context("invalid inherited version ID")?)?;
        }
        let detected = candidate
            .loader
            .as_ref()
            .context("candidate loader is unresolved")?;
        let kind = match detected.kind {
            DetectedLoaderKind::Vanilla => LoaderKind::Vanilla,
            DetectedLoaderKind::Fabric => LoaderKind::Fabric,
            DetectedLoaderKind::Quilt => LoaderKind::Quilt,
            DetectedLoaderKind::Forge => LoaderKind::Forge,
            DetectedLoaderKind::NeoForge => LoaderKind::NeoForge,
        };
        let id = detected.version.as_ref().map_or_else(
            || kind.name().to_lowercase(),
            |version| format!("{}-{version}", kind.name().to_lowercase()),
        );
        let targets = vec![LaunchTarget {
            id,
            kind,
            loader_version: detected.version.clone(),
            source: TargetSource::Existing {
                version_id: version_id.to_owned(),
            },
            status: InstallStatus::Pending,
            optifine: None,
            optifine_mod: None,
        }];
        let instance = Instance {
            id: uuid::Uuid::new_v4().to_string(),
            minecraft_version: candidate
                .minecraft_version
                .clone()
                .context("candidate Minecraft version is unresolved")?,
            resource_directory,
            ownership: InstanceOwnership::Registered,
            targets,
            compatibility: super::default_compatibility(),
            pack: None,
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
        self.transaction(true, |state| {
            ensure!(
                normalize_directory(&instance.resource_directory)? == instance.resource_directory
                    && normalize_directory(&instance.game_directory)? == instance.game_directory,
                "candidate directory changed before registration"
            );
            state.instances.push(instance.clone());
            Ok(instance)
        })
    }
}
