//! Persistent installation and launch settings. No account credentials or resolved commands.

mod storage;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};

use crate::recipe::{Compatibility, RuntimeSelection};

pub use storage::{Catalog, GameDirectoryGuard};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// New catalogs bundle one Profile with each Instance. Existing catalogs
    /// without a mode retain their original ability to share Instances.
    #[serde(default = "legacy_profile_mode")]
    pub profile_mode: ProfileMode,
    pub instances: Vec<Instance>,
    pub profiles: Vec<Profile>,
}

/// Product-level relationship between installations and their launch settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMode {
    #[default]
    Bundled,
    Separated,
}

const fn legacy_profile_mode() -> ProfileMode {
    ProfileMode::Separated
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub minecraft_version: String,
    /// User-selected installation directory; mutable game data stays below it.
    pub directory: PathBuf,
    pub targets: Vec<LaunchTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderKind {
    Vanilla,
    NeoForge,
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
    /// Durable installed metadata, not a resolved launch plan.
    pub manifest: Option<PathBuf>,
    pub status: InstallStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataPolicy {
    Shared,
    #[default]
    Isolated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub instance_id: String,
    pub target_id: String,
    pub runtime: RuntimeSelection,
    pub memory_mib: u32,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub data_policy: DataPolicy,
    pub default_account: Option<String>,
    #[serde(default)]
    pub compatibility: Compatibility,
}

impl Snapshot {
    /// # Errors
    /// Missing or dangling profile/instance/target references are rejected.
    pub fn selection(
        &self,
        profile_id: &str,
    ) -> anyhow::Result<(&Instance, &Profile, &LaunchTarget)> {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .context("profile no longer exists")?;
        let instance = self
            .instances
            .iter()
            .find(|i| i.id == profile.instance_id)
            .context("profile installation is missing")?;
        let target = instance
            .targets
            .iter()
            .find(|t| t.id == profile.target_id)
            .context("profile launch target is missing")?;
        Ok((instance, profile, target))
    }
}

impl Profile {
    /// Resolve predictable mutable data ownership without per-directory mapping.
    /// # Errors
    /// Rejects a profile attached to another installation or unsafe IDs.
    pub fn game_directory(&self, instance: &Instance) -> anyhow::Result<PathBuf> {
        ensure!(
            self.instance_id == instance.id,
            "profile belongs to another instance"
        );
        validate_id(&self.id)?;
        Ok(match self.data_policy {
            DataPolicy::Shared => instance.directory.join("game"),
            DataPolicy::Isolated => instance
                .directory
                .join("profiles")
                .join(&self.id)
                .join("game"),
        })
    }

    /// # Errors
    /// Invalid names, Java paths, memory bounds or process arguments are rejected.
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_id(&self.id)?;
        validate_id(&self.instance_id)?;
        validate_name(&self.name)?;
        ensure!(
            (256..=65536).contains(&self.memory_mib),
            "memory must be between256 and65536MiB"
        );
        ensure!(
            self.jvm_args
                .iter()
                .chain(&self.game_args)
                .all(|a| !a.contains('\0')),
            "arguments cannot contain NUL bytes"
        );
        let reserved = [
            "--accessToken",
            "--uuid",
            "--username",
            "--userType",
            "--xuid",
            "--clientId",
            "--gameDir",
            "--assetsDir",
            "--assetIndex",
            "--version",
            "--versionType",
        ];
        ensure!(
            !self.game_args.iter().any(|arg| reserved
                .iter()
                .any(|flag| arg == flag || arg.starts_with(&format!("{flag}=")))),
            "identity and installation arguments are managed by Block"
        );
        ensure!(
            !self.jvm_args.iter().any(|arg| matches!(
                arg.as_str(),
                "-cp" | "-classpath" | "--class-path"
            ) || ["-Xmx", "-Xms", "--class-path="]
                .iter()
                .any(|prefix| arg.starts_with(prefix))),
            "classpath and memory are managed by Block"
        );
        if let RuntimeSelection::Local { java } = &self.runtime {
            ensure!(java.is_absolute(), "custom Java path must be absolute");
        }
        Ok(())
    }
}

fn validate_id(value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid catalog identifier"
    );
    Ok(())
}

fn validate_name(value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 160 && !value.chars().any(char::is_control),
        "name must contain1..160 printable characters"
    );
    Ok(())
}

/// Installation-root validation does not silently reinterpret relative UI input.
fn validate_directory(path: &Path) -> anyhow::Result<()> {
    ensure!(
        path.is_absolute(),
        "installation directory must be absolute"
    );
    ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "installation directory contains parent traversal"
    );
    Ok(())
}
