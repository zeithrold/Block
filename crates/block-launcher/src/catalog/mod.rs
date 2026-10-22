//! Persistent installation and launch settings. No account credentials or resolved commands.

mod appearance;
mod instance_settings;
mod server;
mod storage;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};

use crate::recipe::{Compatibility, RuntimeSelection};

pub use appearance::{ProfileAppearance, ProfileColor, ProfileIcon};
pub use instance_settings::{InstanceConfiguration, InstanceSettings, ProfileInheritance};
pub use server::{
    AutomaticConnection, ServerAddress, ServerPreferences, automatic_connection,
    supports_quick_play,
};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<InstanceSettings>,
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
    /// Durable installed metadata, not a resolved launch plan.
    pub manifest: Option<PathBuf>,
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
    #[serde(default)]
    pub appearance: ProfileAppearance,
    #[serde(default, skip_serializing_if = "ProfileInheritance::is_legacy")]
    pub inheritance: ProfileInheritance,
    #[serde(default)]
    pub server: ServerPreferences,
}

/// Settings supplied before a new Profile is committed to an existing Instance.
/// Installation identity, data ownership and compatibility stay catalog-managed.
#[derive(Debug, Clone)]
pub struct ProfileConfiguration {
    pub name: String,
    pub target_id: String,
    pub runtime: RuntimeSelection,
    pub memory_mib: u32,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub appearance: ProfileAppearance,
    pub inheritance: ProfileInheritance,
    pub server: ServerPreferences,
}

impl Default for ProfileConfiguration {
    fn default() -> Self {
        Self {
            name: String::new(),
            target_id: "vanilla".into(),
            runtime: RuntimeSelection::Managed,
            memory_mib: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            appearance: ProfileAppearance::default(),
            inheritance: ProfileInheritance::default(),
            server: ServerPreferences::default(),
        }
    }
}

impl From<&Profile> for ProfileConfiguration {
    fn from(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            target_id: profile.target_id.clone(),
            runtime: profile.runtime.clone(),
            memory_mib: profile.memory_mib,
            jvm_args: profile.jvm_args.clone(),
            game_args: profile.game_args.clone(),
            appearance: profile.appearance,
            inheritance: profile.inheritance,
            server: profile.server.clone(),
        }
    }
}

impl Snapshot {
    /// Include Instance defaults in the existing startup consistency guard.
    /// # Errors
    /// Rejects missing selection references and invalid serialization.
    pub fn launch_fingerprint(&self, profile_id: &str) -> anyhow::Result<String> {
        use sha1::Digest as _;
        let (instance, profile, _) = self.selection(profile_id)?;
        Ok(format!(
            "{:x}",
            sha1::Sha1::digest(serde_json::to_vec(&(profile, &instance.settings))?)
        ))
    }

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
        self.server.validate()?;
        if self.server.connect_on_launch {
            ensure!(
                !self.game_args.iter().any(|argument| [
                    "--quickPlayMultiplayer",
                    "--quickPlaySingleplayer",
                    "--quickPlayRealms",
                    "--server",
                    "--port"
                ]
                .iter()
                .any(|flag| argument == flag || argument.starts_with(&format!("{flag}=")))),
                "Automatic connection manages server launch arguments; remove conflicting extra game arguments"
            );
        }
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
