use super::metadata::{latest_version, read_json_bounded, resolve_version, safe_version_id};
use super::profiles::discover_profiles;
use super::{
    DiscoveryReport, DiscoveryRoots, GameCandidate, GameDataEvidence, ImportSourceReport,
    SourceKind,
};
use std::path::{Path, PathBuf};
use std::{collections::BTreeSet, fs, io};
const MAX_VERSION_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROFILES: usize = 256;

/// Inspect only the supplied roots. This performs no writes and uses bounded
/// reads for launcher profiles, version metadata, and version manifests.
#[must_use]
pub fn discover(roots: &DiscoveryRoots) -> DiscoveryReport {
    let mut report = DiscoveryReport::default();
    if let Some(root) = &roots.adjacent_minecraft {
        let (candidates, warning) = discover_adjacent(root);
        report.sources.push(ImportSourceReport {
            kind: SourceKind::AdjacentMinecraft,
            root: root.clone(),
            candidates_found: candidates.len(),
            warning,
        });
        report.candidates.extend(candidates);
    }
    if let Some(root) = &roots.official_launcher {
        let (candidates, warning) = discover_launcher(root);
        report.sources.push(ImportSourceReport {
            kind: SourceKind::OfficialLauncher,
            root: root.clone(),
            candidates_found: candidates.len(),
            warning,
        });
        report.candidates.extend(candidates);
    }
    let mut versions_by_directory = std::collections::BTreeMap::<PathBuf, BTreeSet<String>>::new();
    for candidate in &report.candidates {
        if let (GameDataEvidence::Isolated { directory }, Some(version)) =
            (&candidate.data, &candidate.version_id)
        {
            versions_by_directory
                .entry(directory.clone())
                .or_default()
                .insert(version.clone());
        }
    }
    for candidate in &mut report.candidates {
        if let GameDataEvidence::Isolated { directory } = &candidate.data
            && versions_by_directory
                .get(directory)
                .is_some_and(|versions| versions.len() > 1)
        {
            candidate.data = GameDataEvidence::Shared {
                directory: directory.clone(),
            };
        }
    }
    report
}

fn discover_adjacent(root: &Path) -> (Vec<GameCandidate>, Option<String>) {
    if !is_real_directory(root) {
        return (Vec::new(), Some("Minecraft directory was not found".into()));
    }
    let (profiles, warnings, profile_config_found) =
        discover_profiles(root, SourceKind::AdjacentMinecraft);
    if profile_config_found {
        return (profiles, warnings_to_option(warnings));
    }
    let (mut versions, version_warning) = discover_local_versions(root);
    let mut warnings = warnings;
    if let Some(warning) = version_warning {
        warnings.push(warning);
    }
    if versions.is_empty() && warnings.is_empty() {
        warnings.push("No identifiable Minecraft launch configurations were found".into());
    }
    if let Some(warning) = warnings_to_option(warnings.clone()) {
        for candidate in &mut versions {
            candidate.metadata_warning = Some(format!(
                "{}; {warning}",
                candidate
                    .metadata_warning
                    .as_deref()
                    .unwrap_or("Uncertain data location")
            ));
        }
    }
    (versions, warnings_to_option(warnings))
}

fn discover_launcher(root: &Path) -> (Vec<GameCandidate>, Option<String>) {
    if !is_real_directory(root) {
        return (
            Vec::new(),
            Some("Launcher data directory was not found".into()),
        );
    }
    let (candidates, warnings, _) = discover_profiles(root, SourceKind::OfficialLauncher);
    (candidates, warnings_to_option(warnings))
}

fn warnings_to_option(warnings: Vec<String>) -> Option<String> {
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

fn discover_local_versions(root: &Path) -> (Vec<GameCandidate>, Option<String>) {
    let versions = root.join("versions");
    let entries = match fs::read_dir(&versions) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return (Vec::new(), None);
        }
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("Could not inspect local versions: {error}")),
            );
        }
    };
    let mut candidates = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries {
        if candidates.len() >= MAX_PROFILES {
            warnings.push(format!("More than {MAX_PROFILES} local versions were found; remaining versions were ignored"));
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("Could not read a local version entry: {error}"));
                continue;
            }
        };
        let id = entry.file_name().to_string_lossy().into_owned();
        if !safe_version_id(&id) {
            continue;
        }
        let directory = entry.path();
        if !is_real_directory(&directory) {
            continue;
        }
        let manifest = directory.join(format!("{id}.json"));
        if read_json_bounded(&manifest, MAX_VERSION_BYTES).is_err() {
            continue;
        }
        candidates.push(make_candidate(
            SourceKind::AdjacentMinecraft,
            root,
            &id,
            Some(id.clone()),
        ));
    }
    (candidates, warnings_to_option(warnings))
}

pub(super) fn make_candidate(
    source: SourceKind,
    source_root: &Path,
    name: &str,
    configured_version: Option<String>,
) -> GameCandidate {
    let metadata = resolve_version(source_root, configured_version.as_deref());
    let version_id = match configured_version.as_deref() {
        Some("latest-release") => latest_version(source_root, "release"),
        Some("latest-snapshot") => latest_version(source_root, "snapshot"),
        Some(id) if safe_version_id(id) => Some(id.to_owned()),
        _ => None,
    }
    .filter(|_| metadata.metadata_found);
    let resource_directory = source_root
        .canonicalize()
        .unwrap_or_else(|_| source_root.to_owned());
    let importable = metadata.metadata_found
        && metadata.supported
        && metadata.minecraft_version.is_some()
        && metadata.loader.is_some()
        && version_id.is_some();
    let unsupported_reason = (!importable)
        .then(|| "Game loader or local launch metadata is missing, unknown or unsupported".into());
    GameCandidate {
        id: format!(
            "{source:?}:{}:{}:{name}",
            resource_directory.display(),
            configured_version.as_deref().unwrap_or("")
        ),
        source,
        source_root: source_root.to_owned(),
        resource_directory,
        name: name.to_owned(),
        version_id,
        data: GameDataEvidence::Unknown,
        configured_version,
        minecraft_version: metadata.minecraft_version,
        loader: metadata.loader,
        latest_release: latest_version(source_root, "release"),
        latest_snapshot: latest_version(source_root, "snapshot"),
        importable,
        unsupported_reason,
        metadata_warning: metadata.warning,
    }
}

fn is_real_directory(path: &Path) -> bool {
    path.is_dir()
}
