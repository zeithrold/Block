//! Local modpack inspection and content preparation.

use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

use crate::catalog::LoaderKind;

mod archive;
mod curseforge;
pub(crate) mod descriptor;
pub(crate) mod initialize;
mod mrpack;
mod plain;
pub(crate) mod prepare;
mod prism;

use archive::validate_pack_path;
pub use archive::{extract_overrides, inspect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackFormat {
    Modrinth,
    CurseForge,
    Prism,
    Plain,
}

/// Why an archive's Minecraft target may be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetEvidence {
    Manifest,
    Detected { source: String },
    NeedsSelection,
    UserSelected,
}

/// Inspection is transient: download URLs must not be serialized or logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackInspection {
    pub format: PackFormat,
    pub name: String,
    pub version: Option<String>,
    pub minecraft_version: Option<String>,
    pub loader: Option<(LoaderKind, String)>,
    pub target_evidence: TargetEvidence,
    pub has_mods: bool,
    pub files: Vec<PackFile>,
    pub missing_files: Vec<CurseFile>,
    content: PackContent,
    content_entries: Vec<ContentEntry>,
    archive_prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentEntry {
    path: String,
    size: u64,
    directory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
enum PackContent {
    Modrinth,
    CurseForge { overrides: String },
    Prism { root: String },
    Plain { root: String },
}

#[derive(Clone, PartialEq, Eq)]
pub struct PackFile {
    pub path: String,
    pub sha1: String,
    pub sha512: String,
    pub size: u64,
    pub urls: Vec<String>,
    pub required: bool,
}

impl fmt::Debug for PackFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PackFile")
            .field("path", &self.path)
            .field("sha1", &self.sha1)
            .field("sha512", &self.sha512)
            .field("size", &self.size)
            .field("url_count", &self.urls.len())
            .field("required", &self.required)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurseFile {
    pub project_id: u64,
    pub file_id: u64,
    pub required: bool,
}

/// Optional entries chosen during inspection. Required entries are implicit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackSelection {
    pub optional_files: BTreeSet<String>,
    pub optional_curse_files: BTreeSet<(u64, u64)>,
}

impl PackInspection {
    /// The target must be identified by the pack or explicitly chosen.
    #[must_use]
    pub fn target_confirmed(&self) -> bool {
        self.minecraft_version.is_some()
            && !matches!(self.target_evidence, TargetEvidence::NeedsSelection)
    }

    /// Choose an exact target for a plain archive.
    ///
    /// `None` means the user explicitly selected Vanilla. Standard pack targets
    /// are fixed by their manifest and cannot be overridden here.
    ///
    /// # Errors
    /// Rejects non-plain formats, invalid versions and unsupported loader pins.
    pub fn select_target(
        &mut self,
        minecraft_version: String,
        loader: Option<(LoaderKind, String)>,
    ) -> anyhow::Result<()> {
        use anyhow::ensure;
        ensure!(self.format == PackFormat::Plain, "pack target is fixed");
        ensure!(
            !minecraft_version.trim().is_empty()
                && minecraft_version.len() <= 80
                && !minecraft_version.chars().any(char::is_control)
                && !minecraft_version.contains(['/', '\\']),
            "select an exact Minecraft version"
        );
        let loader = loader
            .map(|(kind, mut version)| {
                if (kind == LoaderKind::Forge
                    || kind == LoaderKind::NeoForge && minecraft_version == "1.20.1")
                    && !version.starts_with(&format!("{minecraft_version}-"))
                {
                    version = format!("{minecraft_version}-{version}");
                }
                match kind {
                    LoaderKind::Vanilla => anyhow::bail!("Vanilla has no loader build"),
                    LoaderKind::Forge => {
                        crate::install::forge::validate_version(&minecraft_version, &version)?;
                    }
                    LoaderKind::NeoForge => {
                        crate::install::neoforge::validate_version(&minecraft_version, &version)?;
                    }
                    LoaderKind::Fabric | LoaderKind::Quilt => {
                        crate::install::fabric::validate_version(&version)?;
                    }
                }
                Ok((kind, version))
            })
            .transpose()?;
        self.minecraft_version = Some(minecraft_version);
        self.loader = loader;
        self.target_evidence = TargetEvidence::UserSelected;
        Ok(())
    }
}

impl CurseFile {
    #[must_use]
    pub fn project_url(&self) -> String {
        format!("https://www.curseforge.com/projects/{}", self.project_id)
    }
}
