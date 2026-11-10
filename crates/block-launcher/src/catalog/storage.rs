//! Atomic catalog section updates; all game data survives metadata deletion.
use super::game_directory::{open_lock, reject_symlink};
use super::validation::validate_snapshot;
use super::{
    GameDirectoryGuard, InstallStatus, InstanceConfiguration, InstanceOwnership, LaunchTarget,
    PackStatus, Snapshot, TargetSource, normalize_directory,
};
use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Catalog {
    pub(super) root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    snapshot: Snapshot,
}

impl Catalog {
    /// # Errors
    /// Fails if the Block data root cannot be created or canonicalized.
    pub fn open(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().canonicalize()?,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// # Errors
    /// Rejects unsupported or invalid catalogs without rewriting their bytes.
    pub fn snapshot(&self) -> anyhow::Result<Snapshot> {
        self.transaction(false, |state| Ok(state.clone()))
    }

    /// Save editable values without moving or creating game data.
    /// # Errors
    /// Invalid settings, missing IDs and active-directory changes are rejected atomically.
    pub fn save_instance(
        &self,
        id: &str,
        configuration: InstanceConfiguration,
    ) -> anyhow::Result<()> {
        self.save_configuration(id, configuration, None)
    }

    /// Replace the selected target's OptiFine component with the settings in one commit.
    /// # Errors
    /// Rejects invalid components, registered targets and invalid Instance configuration.
    pub fn save_instance_with_optifine(
        &self,
        id: &str,
        configuration: InstanceConfiguration,
        optifine: Option<crate::install::optifine::OptiFineJar>,
    ) -> anyhow::Result<()> {
        self.save_configuration(id, configuration, Some(optifine))
    }

    fn save_configuration(
        &self,
        id: &str,
        mut configuration: InstanceConfiguration,
        optifine: Option<Option<crate::install::optifine::OptiFineJar>>,
    ) -> anyhow::Result<()> {
        configuration.game_directory = normalize_directory(&configuration.game_directory)?;
        let mut directory_guard = None;
        self.transaction(true, |state| {
            let instance = state
                .instances
                .iter_mut()
                .find(|item| item.id == id)
                .context("instance no longer exists")?;
            if configuration.game_directory != instance.game_directory
                || configuration.data_policy != instance.data_policy
            {
                if instance.game_directory.exists() {
                    directory_guard = Some(GameDirectoryGuard::for_instance(instance)?);
                } else {
                    ensure!(
                        normalize_directory(&instance.game_directory)? == instance.game_directory,
                        "previous game directory changed"
                    );
                }
            }
            if configuration.data_policy == super::DataPolicy::Shared {
                ensure!(
                    configuration.game_directory.is_dir(),
                    "Shared data requires an existing game directory"
                );
            }
            configuration.apply(instance);
            if let Some(optifine) = optifine {
                ensure!(
                    instance.ownership == InstanceOwnership::Managed,
                    "registered source components are managed by their source launcher"
                );
                let target = instance
                    .targets
                    .iter_mut()
                    .find(|target| target.id == instance.target_id)
                    .context("launch target no longer exists")?;
                if let Some(selected) = &optifine {
                    selected.validate_for(
                        &instance.minecraft_version,
                        &target.kind,
                        target.loader_version.as_deref(),
                    )?;
                }
                target.optifine = optifine;
                target.optifine_mod = None;
            }
            Ok(())
        })
    }

    /// Delete catalog metadata only; accepted sessions and game files remain intact.
    /// # Errors
    /// Rejects a missing Instance or an invalid catalog.
    pub fn delete_instance(&self, id: &str) -> anyhow::Result<()> {
        self.transaction(true, |state| {
            ensure!(
                state.instances.iter().any(|item| item.id == id),
                "instance no longer exists"
            );
            state.instances.retain(|item| item.id != id);
            Ok(())
        })
    }

    /// # Errors
    /// Only replaces an already declared target without changing its source identity.
    pub fn set_target(&self, instance_id: &str, target: &LaunchTarget) -> anyhow::Result<()> {
        self.transaction(true, |state| {
            let instance = state
                .instances
                .iter_mut()
                .find(|item| item.id == instance_id)
                .context("instance no longer exists")?;
            let old = instance
                .targets
                .iter_mut()
                .find(|item| item.id == target.id)
                .context("launch target no longer exists")?;
            same_target(old, target)?;
            *old = target.clone();
            Ok(())
        })
    }

    pub(crate) fn finish_pack_install(
        &self,
        instance_id: &str,
        target: &LaunchTarget,
    ) -> anyhow::Result<()> {
        ensure!(
            target.status == InstallStatus::Ready,
            "pack target is not ready"
        );
        self.transaction(true, |state| {
            let instance = state
                .instances
                .iter_mut()
                .find(|item| item.id == instance_id)
                .context("instance no longer exists")?;
            let pack = instance.pack.as_mut().context("instance has no pack")?;
            ensure!(pack.target_id == target.id, "pack target does not match");
            let existing = instance
                .targets
                .iter_mut()
                .find(|item| item.id == target.id)
                .context("pack target is missing")?;
            same_target(existing, target)?;
            *existing = target.clone();
            pack.status = PackStatus::Ready;
            Ok(())
        })
    }

    pub(super) fn transaction<T>(
        &self,
        write: bool,
        operation: impl FnOnce(&mut Snapshot) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let storage = if write {
            // The configuration document is shared with storage settings, so a
            // writer must hold the data-root lease to keep its read-modify-write
            // cycle atomic and preserve the `[storage]` section.
            Some(crate::storage::acquire(&self.root)?)
        } else {
            None
        };
        let lock = open_lock(&self.root.join("catalog.lock"))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let path = self.root.join(crate::config::FILE);
        reject_symlink(&path)?;
        let mut table = crate::config::load(&self.root)?;
        let mut document = match table.get(crate::config::CATALOG_SECTION) {
            Some(section) => {
                ensure!(
                    section
                        .get("schema_version")
                        .and_then(toml::Value::as_integer)
                        == Some(2),
                    "unsupported catalog schema; expected version 2; original file was preserved"
                );
                serde_json::from_value::<Document>(crate::config::to_json(section))
                    .context("catalog is invalid; original file was preserved")?
            }
            None => Document {
                schema_version: 2,
                snapshot: Snapshot::default(),
            },
        };
        ensure!(
            document.schema_version == 2,
            "unsupported catalog schema; original file was preserved"
        );
        validate_snapshot(&document.snapshot)?;
        let result = operation(&mut document.snapshot)?;
        if write {
            validate_snapshot(&document.snapshot)?;
            if let Some(storage) = &storage {
                for instance in &document.snapshot.instances {
                    storage.validate_game_location(&instance.resource_directory)?;
                    storage.validate_game_location(&instance.game_directory)?;
                }
            }
            let section = crate::config::from_json(&serde_json::to_value(&document)?)?;
            table.insert(crate::config::CATALOG_SECTION.into(), section);
            crate::config::write(&self.root, &table)?;
        }
        Ok(result)
    }
}

fn same_target(old: &LaunchTarget, new: &LaunchTarget) -> anyhow::Result<()> {
    ensure!(
        old.kind == new.kind
            && old.loader_version == new.loader_version
            && old.optifine == new.optifine,
        "cannot change installed target identity"
    );
    match (&old.source, &new.source) {
        (TargetSource::Managed { .. }, TargetSource::Managed { .. }) => Ok(()),
        (
            TargetSource::Existing { version_id: old },
            TargetSource::Existing { version_id: new },
        ) if old == new => Ok(()),
        _ => anyhow::bail!("cannot change installed target source"),
    }
}
