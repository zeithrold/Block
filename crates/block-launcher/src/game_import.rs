//! Read-only discovery and explicit data-location selection for existing games.

use crate::catalog::DataPolicy;
use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod classification;
mod discovery;
mod filtering;
mod metadata;
mod profiles;
pub(crate) use classification::{
    detect_loader, is_known_main_class, looks_like_minecraft_version, minecraft_version_from_id,
};
pub use discovery::discover;
pub(crate) use filtering::is_excluded_entry;

#[derive(Debug, Clone, Default)]
pub struct DiscoveryRoots {
    /// The adjacent `.minecraft` directory, when present.
    pub adjacent_minecraft: Option<PathBuf>,
    /// An official launcher data directory containing launcher profile files.
    pub official_launcher: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    AdjacentMinecraft,
    OfficialLauncher,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSourceReport {
    pub kind: SourceKind,
    pub root: PathBuf,
    pub candidates_found: usize,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameCandidate {
    /// Stable for the duration of one discovery result; suitable for UI keys.
    pub id: String,
    pub source: SourceKind,
    pub source_root: PathBuf,
    /// Profile label when available, otherwise a directory-derived label.
    pub name: String,
    pub resource_directory: PathBuf,
    pub version_id: Option<String>,
    pub data: GameDataEvidence,
    pub configured_version: Option<String>,
    pub minecraft_version: Option<String>,
    pub loader: Option<DetectedLoader>,
    pub latest_release: Option<String>,
    pub latest_snapshot: Option<String>,
    pub importable: bool,
    pub unsupported_reason: Option<String>,
    pub metadata_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedLoader {
    pub kind: LoaderKind,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoaderKind {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub sources: Vec<ImportSourceReport>,
    /// One item per identifiable launch configuration, including unsupported
    /// configurations so the UI can explain why they cannot be imported.
    pub candidates: Vec<GameCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameDataEvidence {
    Shared { directory: PathBuf },
    Isolated { directory: PathBuf },
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSelection {
    pub candidate: GameCandidate,
    pub data_policy: DataPolicy,
    pub game_directory: PathBuf,
}

impl GameCandidate {
    /// Return the exact offered location without creating directories.
    /// # Errors
    /// Rejects unresolved version IDs, files, unsafe aliases and shared collisions.
    pub fn data_directory(&self, policy: DataPolicy) -> anyhow::Result<PathBuf> {
        let root = crate::catalog::normalize_directory(&self.resource_directory)?;
        let selected = match (&self.data, policy) {
            (GameDataEvidence::Shared { directory }, DataPolicy::Shared)
            | (GameDataEvidence::Isolated { directory }, DataPolicy::Isolated) => directory.clone(),
            (GameDataEvidence::Isolated { .. } | GameDataEvidence::Unknown, DataPolicy::Shared) => {
                root.clone()
            }
            (GameDataEvidence::Shared { .. } | GameDataEvidence::Unknown, DataPolicy::Isolated) => {
                let id = self
                    .version_id
                    .as_deref()
                    .context("No concrete local version is selected")?;
                ensure!(
                    metadata::safe_version_id(id)
                        && !matches!(id, "latest-release" | "latest-snapshot"),
                    "Invalid concrete local version ID"
                );
                let path = root.join("versions").join(id);
                if let GameDataEvidence::Shared { directory } = &self.data {
                    let shared = crate::catalog::normalize_directory(directory)?;
                    if crate::catalog::normalize_directory(&path)? == shared {
                        let alternate =
                            crate::catalog::normalize_directory(&path.join(".minecraft"))?;
                        ensure!(
                            alternate != shared,
                            "Isolated game directory aliases the shared location"
                        );
                        return Ok(alternate);
                    }
                }
                path
            }
        };
        crate::catalog::normalize_directory(&selected)
    }
}

#[cfg(test)]
mod evidence_tests;
#[cfg(test)]
mod tests;
