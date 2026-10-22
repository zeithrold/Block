use std::{collections::BTreeMap, path::PathBuf};

use anyhow::ensure;
use futures_util::future::BoxFuture;
use minecraft_manifest::model::VersionManifest;
use serde::{Deserialize, Serialize};

use crate::{platform::Target, resolver::ResolveContext, session::OfflineSession};

/// Declarative launch input. Recipes never contain access or refresh tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRecipe {
    pub schema_version: u32,
    pub instance: String,
    pub source: RecipeSource,
    pub session: OfflineSession,
    #[serde(default)]
    pub runtime: RuntimeSelection,
    #[serde(default)]
    pub target: Option<Target>,
    #[serde(default)]
    pub compatibility: Compatibility,
    #[serde(default = "default_memory")]
    pub memory_mib: u32,
    #[serde(default)]
    pub jvm_args: Vec<String>,
    #[serde(default)]
    pub features: BTreeMap<String, bool>,
}

const fn default_memory() -> u32 {
    2048
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecipeSource {
    Mojang { version: String },
    LocalManifest { path: PathBuf },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeSelection {
    #[default]
    Managed,
    Local {
        java: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    #[default]
    Official,
    /// Replace only selected LWJGL 3 Linux native artifacts with matching ARM64 classifiers.
    LinuxArm64Lwjgl3,
    /// Replace selected LWJGL 3 Windows native artifacts with the official
    /// Maven Central `natives-windows-arm64` classifiers.
    #[serde(rename = "windows_arm64_lwjgl3")]
    WindowsArm64Lwjgl3,
    /// Replace the affected macOS JNA core/platform pair together with JNA 5.13.0.
    #[serde(rename = "macos_jna_5_13")]
    MacosJna513,
}

#[derive(Debug, Clone)]
pub struct ParsedRecipe {
    pub manifest: VersionManifest,
    pub manifest_sha1: String,
}

/// Source adapters resolve recipes; no adapter spawns a process or modifies game files.
pub trait RecipeParser: Send + Sync {
    fn resolve<'a>(
        &'a self,
        source: &'a RecipeSource,
        context: &'a ResolveContext,
    ) -> BoxFuture<'a, anyhow::Result<ParsedRecipe>>;
}

impl LaunchRecipe {
    /// Read a recipe with file and Java paths relative to the recipe document.
    ///
    /// # Errors
    /// Fails on IO, an invalid schema, or invalid launch options.
    pub async fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let path = path.canonicalize()?;
        let directory = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("recipe has no parent directory"))?;
        let mut recipe: Self = serde_json::from_slice(&tokio::fs::read(&path).await?)?;
        if let RecipeSource::LocalManifest { path } = &mut recipe.source
            && path.is_relative()
        {
            *path = directory.join(&*path);
        }
        if let RuntimeSelection::Local { java } = &mut recipe.runtime
            && java.is_relative()
        {
            *java = directory.join(&*java);
        }
        recipe.validate()?;
        Ok(recipe)
    }

    /// Validate the input independently of IO or account providers.
    ///
    /// # Errors
    /// Fails on unsupported schemas, invalid instance IDs, or invalid options.
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported recipe schema: {}",
            self.schema_version
        );
        ensure!(
            !self.instance.is_empty()
                && self.instance.len() <= 64
                && self
                    .instance
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "instance ID must contain 1..64 ASCII letters, digits, underscores or hyphens"
        );
        ensure!(
            (crate::MIN_MEMORY_MIB..=crate::MAX_MEMORY_MIB).contains(&self.memory_mib),
            "memory_mib must be between 1024 and 32768"
        );
        self.session.validate()?;
        Ok(())
    }
}
