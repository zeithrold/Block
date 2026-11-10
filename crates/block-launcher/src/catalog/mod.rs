//! Persistent Instance metadata. No account credentials or resolved commands.

mod appearance;
mod creation;
mod directory_path;
mod fingerprint;
mod game_directory;
mod instance_settings;
mod registration;
mod server;
mod storage;
mod validation;
#[cfg(test)]
mod verification_tests;

use crate::recipe::{Compatibility, RuntimeSelection};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use appearance::{InstanceAppearance, InstanceColor, InstanceIcon};
pub(crate) use directory_path::normalize_directory;
pub(crate) use fingerprint::ExistingSourceFailure;
pub use game_directory::GameDirectoryGuard;
pub use instance_settings::InstanceConfiguration;
pub use server::{
    AutomaticConnection, ServerAddress, ServerPreferences, automatic_connection,
    supports_quick_play,
};
pub use storage::Catalog;
pub use validation::validate_name;
use validation::{validate_directory, validate_id, validate_version_id};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub instances: Vec<Instance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub minecraft_version: String,
    pub resource_directory: PathBuf,
    pub ownership: InstanceOwnership,
    pub game_directory: PathBuf,
    pub data_policy: DataPolicy,
    pub targets: Vec<LaunchTarget>,
    pub target_id: String,
    pub runtime: RuntimeSelection,
    pub memory_mib: u32,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub default_account: Option<String>,
    pub compatibility: Compatibility,
    pub appearance: InstanceAppearance,
    pub server: ServerPreferences,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub pack: Option<PackAttachment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceOwnership {
    Managed,
    Registered,
}

/// Pack installation state applies to the entire Instance, independently of
/// individual loader installation states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackAttachment {
    pub target_id: String,
    pub status: PackStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackStatus {
    Pending,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderKind {
    Vanilla,
    NeoForge,
    Forge,
    Fabric,
    Quilt,
}

impl LoaderKind {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Vanilla => "Vanilla",
            Self::NeoForge => "NeoForge",
            Self::Forge => "Forge",
            Self::Fabric => "Fabric",
            Self::Quilt => "Quilt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallStatus {
    Pending,
    Installing,
    Ready,
    Failed { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchTarget {
    pub id: String,
    pub kind: LoaderKind,
    pub loader_version: Option<String>,
    pub source: TargetSource,
    pub status: InstallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optifine: Option<crate::install::optifine::OptiFineJar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optifine_mod: Option<block_download::Resource>,
}

impl LaunchTarget {
    /// Describe both the loader and the selected optimization component.
    #[must_use]
    pub fn display_name(&self) -> String {
        let loader = self.loader_version.as_ref().map_or_else(
            || self.kind.name().to_owned(),
            |version| format!("{} {version}", self.kind.name()),
        );
        self.optifine.as_ref().map_or(loader.clone(), |selected| {
            if self.kind == LoaderKind::Vanilla {
                format!("OptiFine {}", selected.version)
            } else {
                format!("{loader} + OptiFine {}", selected.version)
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataPolicy {
    Shared,
    #[default]
    Isolated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetSource {
    Managed { manifest: Option<PathBuf> },
    Existing { version_id: String },
}

impl Snapshot {
    /// # Errors
    /// Rejects missing Instance and selected target references.
    pub fn selection(&self, instance_id: &str) -> anyhow::Result<(&Instance, &LaunchTarget)> {
        let instance = self
            .instances
            .iter()
            .find(|instance| instance.id == instance_id)
            .context("instance no longer exists")?;
        let target = instance
            .targets
            .iter()
            .find(|target| target.id == instance.target_id)
            .context("instance launch target is missing")?;
        Ok((instance, target))
    }
}

fn default_compatibility() -> Compatibility {
    if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Compatibility::LinuxArm64Lwjgl3
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Compatibility::WindowsArm64Lwjgl3
    } else {
        Compatibility::Official
    }
}
