//! Read-only discovery and safe copying of existing Minecraft game directories.
//!
//! Discovery accepts explicit roots so callers choose which launcher locations
//! to inspect. Import copies only game data into a new directory; it never
//! modifies the discovered source.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_PROFILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_VERSION_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROFILES: usize = 256;
const MAX_INHERITANCE_DEPTH: usize = 16;

// This finite top-level allowlist prevents a custom gameDir from causing a
// whole home folder, launcher installation, or unrelated private data to copy.
const MINECRAFT_DATA_DIRECTORIES: &[&str] = &[
    "assets",
    "config",
    "defaultconfigs",
    "libraries",
    "mods",
    "resourcepacks",
    "saves",
    "screenshots",
    "server-resource-packs",
    "shaderpacks",
    "versions",
];
const MINECRAFT_DATA_FILES: &[&str] = &[
    "options.txt",
    "optionsof.txt",
    "optionsshaders.txt",
    "servers.dat",
    "servers.dat_old",
];

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
    pub game_directory: PathBuf,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub candidate_id: String,
    pub source_directory: PathBuf,
    pub target_directory: PathBuf,
    pub files_copied: u64,
    pub bytes_copied: u64,
}

#[derive(Debug)]
pub enum ImportError {
    Unsupported(String),
    Overlap,
    DestinationExists(PathBuf),
    Symlink(PathBuf),
    Cancelled,
    UnsafePath(PathBuf),
    Io(io::Error),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(formatter, "candidate is not importable: {reason}"),
            Self::Overlap => formatter.write_str("source and destination overlap"),
            Self::DestinationExists(path) => {
                write!(formatter, "destination already exists: {}", path.display())
            }
            Self::Symlink(path) => write!(
                formatter,
                "unsafe symbolic link in source: {}",
                path.display()
            ),
            Self::Cancelled => formatter.write_str("copy cancelled"),
            Self::UnsafePath(path) => write!(formatter, "unsafe path: {}", path.display()),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Io(error) = self {
            Some(error)
        } else {
            None
        }
    }
}

impl From<io::Error> for ImportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

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
    report
}

/// Copy a candidate to a new independent directory using a private sibling
/// staging directory and atomic rename. Cancellation or failure removes only
/// the staging directory created by this call.
pub fn copy_candidate(
    candidate: &GameCandidate,
    destination: &Path,
    cancellation: &CancellationToken,
) -> Result<ImportResult, ImportError> {
    if !candidate.importable {
        return Err(ImportError::Unsupported(
            candidate
                .unsupported_reason
                .clone()
                .unwrap_or_else(|| "candidate is not supported".into()),
        ));
    }
    if cancellation.is_cancelled() {
        return Err(ImportError::Cancelled);
    }

    let source = fs::canonicalize(&candidate.game_directory)?;
    let source_meta = fs::symlink_metadata(&candidate.game_directory)?;
    if source_meta.file_type().is_symlink() || !source_meta.is_dir() {
        return Err(ImportError::Symlink(candidate.game_directory.clone()));
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent)?;
    let leaf = destination
        .file_name()
        .ok_or_else(|| ImportError::UnsafePath(destination.to_owned()))?;
    if Path::new(leaf)
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ImportError::UnsafePath(destination.to_owned()));
    }
    let target = parent.join(leaf);
    if target.exists() {
        return Err(ImportError::DestinationExists(target));
    }
    if paths_overlap(&source, &target) {
        return Err(ImportError::Overlap);
    }

    let stage = parent.join(format!(".block-import-{}.partial", Uuid::new_v4()));
    fs::create_dir(&stage)?;
    let result = copy_tree(&source, &stage, cancellation);
    let (files_copied, bytes_copied) = match result {
        Ok(counts) => counts,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
    };
    if cancellation.is_cancelled() {
        let _ = fs::remove_dir_all(&stage);
        return Err(ImportError::Cancelled);
    }
    if let Err(error) = fs::rename(&stage, &target) {
        let _ = fs::remove_dir_all(&stage);
        return Err(error.into());
    }
    Ok(ImportResult {
        candidate_id: candidate.id.clone(),
        source_directory: source,
        target_directory: target,
        files_copied,
        bytes_copied,
    })
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
    let (versions, version_warning) = discover_local_versions(root);
    let mut warnings = warnings;
    if let Some(warning) = version_warning {
        warnings.push(warning);
    }
    if versions.is_empty() && warnings.is_empty() {
        warnings.push("No identifiable Minecraft launch configurations were found".into());
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

fn discover_profiles(root: &Path, source: SourceKind) -> (Vec<GameCandidate>, Vec<String>, bool) {
    let mut candidates = Vec::new();
    let mut warnings = Vec::new();
    let mut has_profiles = false;
    let mut seen_profile_keys = BTreeSet::new();
    let profile_files = [
        "launcher_profiles.json",
        "launcher_profiles_microsoft_store.json",
    ];
    for filename in profile_files {
        let path = root.join(filename);
        if !path.exists() {
            continue;
        }
        let parsed = match read_json_bounded(&path, MAX_PROFILE_BYTES) {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!("Could not read {filename}: {error}"));
                continue;
            }
        };
        let profiles = parsed
            .get("profiles")
            .and_then(serde_json::Value::as_object);
        let Some(profiles) = profiles else {
            warnings.push(format!("{filename} has no profiles object"));
            continue;
        };
        for (key, profile) in profiles.iter().take(MAX_PROFILES) {
            if !profile.is_object() {
                continue;
            }
            let identifiable = ["name", "gameDir", "lastVersionId"]
                .iter()
                .any(|field| profile.get(field).is_some());
            if !identifiable {
                continue;
            }
            if !seen_profile_keys.insert(key.clone()) {
                continue;
            }
            has_profiles = true;
            let profile_name = profile
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(key);
            let configured_directory = profile
                .get("gameDir")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| root.to_owned());
            let directory = if configured_directory.is_absolute() {
                configured_directory
            } else {
                root.join(configured_directory)
            };
            let configured_version = profile
                .get("lastVersionId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let mut candidate =
                make_candidate(source, root, &directory, profile_name, configured_version);
            candidate.id = format!(
                "{}:{}:{key}",
                match source {
                    SourceKind::AdjacentMinecraft => "adjacent",
                    SourceKind::OfficialLauncher => "launcher",
                },
                root.display()
            );
            candidates.push(candidate);
        }
        if profiles.len() > MAX_PROFILES {
            warnings.push(format!("{filename} contains more than {MAX_PROFILES} profiles; remaining profiles were ignored"));
        }
    }
    (candidates, warnings, has_profiles)
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
        let game_directory = if has_mutable_game_data(&directory) {
            directory
        } else {
            root.to_owned()
        };
        candidates.push(make_candidate(
            SourceKind::AdjacentMinecraft,
            root,
            &game_directory,
            &id,
            Some(id.clone()),
        ));
    }
    (candidates, warnings_to_option(warnings))
}

fn make_candidate(
    source: SourceKind,
    source_root: &Path,
    game_directory: &Path,
    name: &str,
    configured_version: Option<String>,
) -> GameCandidate {
    let mut metadata = resolve_version(source_root, configured_version.as_deref());
    if game_directory != source_root
        && (metadata.minecraft_version.is_none() || metadata.loader.is_none())
    {
        let fallback = resolve_version(game_directory, configured_version.as_deref());
        if metadata.minecraft_version.is_none() {
            metadata.minecraft_version = fallback.minecraft_version;
        }
        if metadata.loader.is_none() {
            metadata.loader = fallback.loader;
        }
        if fallback.metadata_found {
            if metadata.metadata_found {
                metadata.supported &= fallback.supported;
            } else {
                metadata.supported = fallback.supported;
            }
            metadata.metadata_found = true;
        }
        if metadata.warning.is_some() {
            metadata.warning = fallback.warning.or(metadata.warning);
        }
    }
    let directory_ok = is_real_directory(game_directory);
    let importable = directory_ok
        && metadata.supported
        && metadata.minecraft_version.is_some()
        && metadata.loader.is_some();
    let unsupported_reason = if !directory_ok {
        Some("Configured game directory is missing or is a symbolic link".into())
    } else if metadata.minecraft_version.is_none() {
        Some("Minecraft version could not be resolved from local metadata".into())
    } else if !metadata.supported || metadata.loader.is_none() {
        Some("Game loader or launch metadata is unknown or unsupported".into())
    } else {
        None
    };
    let id = format!(
        "{}:{}:{}:{}",
        match source {
            SourceKind::AdjacentMinecraft => "adjacent",
            SourceKind::OfficialLauncher => "launcher",
        },
        game_directory.display(),
        configured_version.as_deref().unwrap_or(""),
        name
    );
    GameCandidate {
        id,
        source,
        source_root: source_root.to_owned(),
        name: name.to_owned(),
        game_directory: game_directory.to_owned(),
        configured_version,
        minecraft_version: metadata.minecraft_version,
        loader: metadata.loader,
        latest_release: latest_version(game_directory, "release")
            .or_else(|| latest_version(source_root, "release")),
        latest_snapshot: latest_version(game_directory, "snapshot")
            .or_else(|| latest_version(source_root, "snapshot")),
        importable,
        unsupported_reason,
        metadata_warning: metadata.warning,
    }
}

struct VersionMetadata {
    minecraft_version: Option<String>,
    loader: Option<DetectedLoader>,
    warning: Option<String>,
    supported: bool,
    metadata_found: bool,
}

fn resolve_version(metadata_root: &Path, configured: Option<&str>) -> VersionMetadata {
    let Some(configured) = configured else {
        return VersionMetadata {
            minecraft_version: None,
            loader: None,
            warning: Some("No launch version is configured; game data can still be copied".into()),
            supported: false,
            metadata_found: false,
        };
    };
    let resolved_alias = match configured {
        "latest-release" => latest_version(metadata_root, "release"),
        "latest-snapshot" => latest_version(metadata_root, "snapshot"),
        _ => None,
    };
    let configured_id = resolved_alias.as_deref().unwrap_or(configured);
    let versions_dir = metadata_root.join("versions");
    let mut version_id = configured_id.to_owned();
    let mut seen = BTreeSet::new();
    let mut mc_version = None;
    let mut loader = None;
    let mut found = false;
    let mut supported = true;
    for _ in 0..MAX_INHERITANCE_DEPTH {
        if !safe_version_id(&version_id) || !seen.insert(version_id.clone()) {
            break;
        }
        let path = versions_dir
            .join(&version_id)
            .join(format!("{version_id}.json"));
        let Ok(json) = read_json_bounded(&path, MAX_VERSION_BYTES) else {
            break;
        };
        found = true;
        let id = json
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&version_id);
        let detected = detect_loader(&json, id);
        let main_class = json
            .get("mainClass")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !main_class.is_empty()
            && !is_known_main_class(main_class)
            && !detected
                .as_ref()
                .is_some_and(|value| value.kind != LoaderKind::Vanilla)
        {
            supported = false;
        }
        if !looks_like_minecraft_version(id)
            && detected
                .as_ref()
                .is_none_or(|value| value.kind == LoaderKind::Vanilla)
            && json.get("inheritsFrom").is_some()
        {
            supported = false;
        }
        if loader.is_none() {
            loader = detected.clone();
        }
        mc_version = mc_version.or_else(|| minecraft_version_from_id(id));
        if let Some(inherits) = json.get("inheritsFrom").and_then(serde_json::Value::as_str) {
            mc_version = mc_version.or_else(|| minecraft_version_from_id(inherits));
            version_id = inherits.to_owned();
        } else {
            if mc_version.is_none() {
                mc_version = json
                    .get("inheritsFrom")
                    .and_then(serde_json::Value::as_str)
                    .and_then(minecraft_version_from_id);
            }
            break;
        }
    }
    if !found {
        let (detected, base) = classify_version(configured_id);
        let vanilla = detected.kind == LoaderKind::Vanilla;
        if !vanilla || base.as_deref().is_some_and(looks_like_minecraft_version) {
            loader = Some(detected);
        }
        mc_version = base.filter(|value| looks_like_minecraft_version(value));
        if vanilla && mc_version.is_none() {
            supported = false;
        }
    }
    if let Some(concrete_id) = resolved_alias.as_deref() {
        mc_version = mc_version.or_else(|| minecraft_version_from_id(concrete_id));
        if loader.is_none() {
            let alias_meta = resolve_version(metadata_root, Some(concrete_id));
            loader = alias_meta.loader;
            supported &= alias_meta.supported;
        }
    }
    if loader
        .as_ref()
        .is_some_and(|detected| detected.kind != LoaderKind::Vanilla && detected.version.is_none())
    {
        supported = false;
    }
    let warning = if !found || mc_version.is_none() || loader.is_none() || !supported {
        Some(format!(
            "Version or loader metadata for '{configured}' is incomplete or unsupported"
        ))
    } else {
        None
    };
    VersionMetadata {
        minecraft_version: mc_version,
        loader,
        warning,
        supported,
        metadata_found: found,
    }
}

fn is_known_main_class(main_class: &str) -> bool {
    matches!(
        main_class,
        "net.minecraft.client.main.Main"
            | "net.fabricmc.loader.impl.launch.knot.KnotClient"
            | "net.fabricmc.loader.launch.knot.KnotClient"
            | "org.quiltmc.loader.impl.launch.knot.KnotClient"
            | "cpw.mods.modlauncher.Launcher"
            | "cpw.mods.bootstraplauncher.BootstrapLauncher"
    )
}

fn detect_loader(json: &serde_json::Value, id: &str) -> Option<DetectedLoader> {
    let (from_id, base) = classify_version(id);
    if from_id.kind != LoaderKind::Vanilla {
        return Some(from_id);
    }
    let libraries = json.get("libraries").and_then(serde_json::Value::as_array);
    let coordinate = |expected_group: &str, expected_artifact: &str| {
        libraries?.iter().find_map(|library| {
            let name = library.get("name")?.as_str()?;
            let (group, artifact, version) = split_maven_coordinate(name)?;
            (group == expected_group && artifact == expected_artifact).then(|| version.to_owned())
        })
    };
    let detected = if let Some(version) = coordinate("net.neoforged", "neoforge") {
        Some(DetectedLoader {
            kind: LoaderKind::NeoForge,
            version: Some(version),
        })
    } else if let Some(version) = coordinate("net.minecraftforge", "forge") {
        Some(DetectedLoader {
            kind: LoaderKind::Forge,
            version: Some(version),
        })
    } else if let Some(version) = coordinate("net.fabricmc", "fabric-loader") {
        Some(DetectedLoader {
            kind: LoaderKind::Fabric,
            version: Some(version),
        })
    } else if let Some(version) = coordinate("org.quiltmc", "quilt-loader") {
        Some(DetectedLoader {
            kind: LoaderKind::Quilt,
            version: Some(version),
        })
    } else {
        let main_class = json
            .get("mainClass")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if main_class.contains("fabricmc") {
            Some(DetectedLoader {
                kind: LoaderKind::Fabric,
                version: None,
            })
        } else if main_class.contains("quiltmc") {
            Some(DetectedLoader {
                kind: LoaderKind::Quilt,
                version: None,
            })
        } else if main_class.contains("neoforged") {
            Some(DetectedLoader {
                kind: LoaderKind::NeoForge,
                version: None,
            })
        } else if main_class.contains("forge") || main_class.contains("modlauncher") {
            Some(DetectedLoader {
                kind: LoaderKind::Forge,
                version: None,
            })
        } else if looks_like_minecraft_version(id) || main_class == "net.minecraft.client.main.main"
        {
            Some(DetectedLoader {
                kind: LoaderKind::Vanilla,
                version: None,
            })
        } else {
            None
        }
    };
    detected.or_else(|| {
        (base.as_deref().is_some_and(looks_like_minecraft_version)).then_some(DetectedLoader {
            kind: LoaderKind::Vanilla,
            version: None,
        })
    })
}

fn split_maven_coordinate(name: &str) -> Option<(&str, &str, &str)> {
    let mut parts = name.split(':');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

fn minecraft_version_from_id(id: &str) -> Option<String> {
    let (loader, base) = classify_version(id);
    if loader.kind != LoaderKind::Vanilla {
        base.filter(|value| looks_like_minecraft_version(value))
    } else {
        looks_like_minecraft_version(id).then(|| id.to_owned())
    }
}

fn looks_like_minecraft_version(value: &str) -> bool {
    fn numeric(value: &str) -> bool {
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
    }
    let lower = value.to_ascii_lowercase();
    if lower.as_bytes().get(2) == Some(&b'w')
        && lower.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && lower.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
        && lower
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return true;
    }
    let concrete = lower
        .strip_prefix('a')
        .or_else(|| lower.strip_prefix('b'))
        .unwrap_or(&lower);
    let (base, suffix) = concrete
        .split_once('-')
        .map_or((concrete, None), |(base, suffix)| (base, Some(suffix)));
    let parts = base.split(['.', '_']).collect::<Vec<_>>();
    if parts.len() < 2 || !parts.iter().all(|part| numeric(part)) {
        return false;
    }
    suffix.is_none_or(|suffix| {
        suffix
            .strip_prefix("pre")
            .or_else(|| suffix.strip_prefix("rc"))
            .is_some_and(numeric)
            || suffix
                .strip_prefix("snapshot-")
                .or_else(|| suffix.strip_prefix("experimental-snapshot-"))
                .is_some_and(numeric)
    })
}

fn classify_version(id: &str) -> (DetectedLoader, Option<String>) {
    let lower = id.to_ascii_lowercase();
    if let Some((base, rest)) = lower.split_once("-neoforge-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::NeoForge,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some((base, rest)) = lower.split_once("-forge-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::Forge,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some((base, rest)) = lower.split_once("-fabric-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::Fabric,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some((base, rest)) = lower.split_once("-quilt-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::Quilt,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some(rest) = lower.strip_prefix("fabric-loader-") {
        let mut parts = rest.splitn(2, '-');
        let loader = parts.next().unwrap_or_default();
        let mc = parts.next().map(str::to_owned);
        return (
            DetectedLoader {
                kind: LoaderKind::Fabric,
                version: Some(loader.to_owned()),
            },
            mc,
        );
    }
    if let Some(rest) = lower.strip_prefix("quilt-loader-") {
        let mut parts = rest.splitn(2, '-');
        let loader = parts.next().unwrap_or_default();
        let mc = parts.next().map(str::to_owned);
        return (
            DetectedLoader {
                kind: LoaderKind::Quilt,
                version: Some(loader.to_owned()),
            },
            mc,
        );
    }
    if let Some(rest) = lower.strip_prefix("neoforge-") {
        return (
            DetectedLoader {
                kind: LoaderKind::NeoForge,
                version: Some(rest.to_owned()),
            },
            None,
        );
    }
    if let Some(rest) = lower.strip_prefix("forge-") {
        let (base, loader_version) = if let Some((mc, loader)) = rest.split_once('-') {
            if mc.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                (Some(mc.to_owned()), Some(loader.to_owned()))
            } else {
                (None, Some(rest.to_owned()))
            }
        } else {
            (None, Some(rest.to_owned()))
        };
        return (
            DetectedLoader {
                kind: LoaderKind::Forge,
                version: loader_version,
            },
            base,
        );
    }
    (
        DetectedLoader {
            kind: LoaderKind::Vanilla,
            version: None,
        },
        Some(id.to_owned()),
    )
}

fn latest_version(root: &Path, release_type: &str) -> Option<String> {
    for filename in [
        "version_manifest_v2.json",
        "version_manifest.json",
        "versions/manifest.json",
    ] {
        let path = root.join(filename);
        let Ok(json) = read_json_bounded(&path, MAX_MANIFEST_BYTES) else {
            continue;
        };
        if let Some(version) = json
            .get("latest")
            .and_then(|latest| latest.get(release_type))
            .and_then(serde_json::Value::as_str)
            .filter(|version| safe_version_id(version) && looks_like_minecraft_version(version))
        {
            return Some(version.to_owned());
        }
    }
    let versions = fs::read_dir(root.join("versions")).ok()?;
    let mut latest: Option<(String, String)> = None;
    for entry in versions.take(MAX_PROFILES).flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if !safe_version_id(&id) || !is_real_directory(&entry.path()) {
            continue;
        }
        let path = entry.path().join(format!("{id}.json"));
        let Ok(json) = read_json_bounded(&path, MAX_VERSION_BYTES) else {
            continue;
        };
        let version_id = json
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&id);
        if json.get("inheritsFrom").is_some()
            || !looks_like_minecraft_version(version_id)
            || detect_loader(&json, version_id)
                .is_none_or(|loader| loader.kind != LoaderKind::Vanilla)
        {
            continue;
        }
        if json
            .get("type")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|kind| !kind.eq_ignore_ascii_case(release_type))
        {
            continue;
        }
        let Some(release_time) = json
            .get("releaseTime")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let version = version_id.to_owned();
        if latest
            .as_ref()
            .is_none_or(|(best_time, _)| release_time > *best_time)
        {
            latest = Some((release_time, version));
        }
    }
    latest.map(|(_, version)| version)
}

fn read_json_bounded(path: &Path, max_bytes: u64) -> io::Result<serde_json::Value> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file or exceeds size limit",
        ));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn safe_version_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && Path::new(id)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && !id
            .chars()
            .any(|character| character == '/' || character == '\\')
}

fn paths_overlap(source: &Path, destination: &Path) -> bool {
    source.starts_with(destination) || destination.starts_with(source)
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn has_mutable_game_data(path: &Path) -> bool {
    ["config", "mods", "resourcepacks", "saves", "shaderpacks"]
        .iter()
        .any(|name| {
            fs::symlink_metadata(path.join(name))
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        })
        || ["options.txt", "optionsof.txt", "optionsshaders.txt"]
            .iter()
            .any(|name| {
                fs::symlink_metadata(path.join(name))
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            })
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    cancellation: &CancellationToken,
) -> Result<(u64, u64), ImportError> {
    let mut files = 0;
    let mut bytes = 0;
    copy_directory_contents(
        source,
        destination,
        cancellation,
        true,
        &mut files,
        &mut bytes,
    )?;
    Ok((files, bytes))
}

fn copy_directory_contents(
    source: &Path,
    destination: &Path,
    cancellation: &CancellationToken,
    top_level: bool,
    files: &mut u64,
    bytes: &mut u64,
) -> Result<(), ImportError> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if cancellation.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        let from = entry.path();
        let to = destination.join(&name);
        let metadata = fs::symlink_metadata(&from)?;
        if metadata.file_type().is_symlink() {
            return Err(ImportError::Symlink(from));
        }
        if is_excluded_entry(&name_text, metadata.is_dir(), top_level)
            || (top_level && !recognized_game_entry(&name_text, metadata.is_dir()))
        {
            continue;
        }
        if metadata.is_dir() {
            fs::create_dir(&to)?;
            copy_directory_contents(&from, &to, cancellation, false, files, bytes)?;
        } else if metadata.is_file() {
            copy_file(&from, &to, cancellation, bytes)?;
            *files += 1;
        }
    }
    Ok(())
}

fn copy_file(
    source: &Path,
    destination: &Path,
    cancellation: &CancellationToken,
    bytes: &mut u64,
) -> Result<(), ImportError> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancellation.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        *bytes += read as u64;
    }
    output.sync_all()?;
    Ok(())
}

fn recognized_game_entry(name: &str, is_directory: bool) -> bool {
    let normalized = name.to_ascii_lowercase();
    if is_directory {
        MINECRAFT_DATA_DIRECTORIES.contains(&normalized.as_str())
    } else {
        MINECRAFT_DATA_FILES.contains(&normalized.as_str())
    }
}

fn is_excluded_entry(name: &str, is_directory: bool, top_level: bool) -> bool {
    let normalized = name.to_ascii_lowercase();
    let launcher_profile =
        normalized.starts_with("launcher_profiles") || normalized.starts_with("launcher_accounts");
    let credential_file = !is_directory
        && matches!(
            normalized.as_str(),
            "account.json"
                | "accounts.json"
                | "accounts.dat"
                | "usercache.json"
                | "launcher_msa_credentials.bin"
                | "launcher_authlib_injector.json"
                | "credentials.json"
                | "credential.json"
                | "auth.json"
                | "secret.json"
                | "secrets.json"
                | "token.json"
                | "tokens.json"
                | "token"
                | "tokens"
                | "session.json"
                | "session.lock"
                | "session"
                | ".block-game.lock"
        );
    let environment_file = normalized == ".env" || normalized.starts_with(".env.");
    let sensitive_directory = is_directory
        && matches!(
            normalized.as_str(),
            ".git"
                | ".ssh"
                | ".aws"
                | ".gnupg"
                | ".kube"
                | ".azure"
                | ".docker"
                | ".config"
                | ".cache"
                | ".local"
        );
    let plain_sensitive_directory = top_level
        && is_directory
        && matches!(
            normalized.as_str(),
            "credentials" | "accounts" | "auth" | "sessions" | "tokens" | "secrets"
        );
    launcher_profile
        || credential_file
        || environment_file
        || sensitive_directory
        || plain_sensitive_directory
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_json(path: &Path, value: &str) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "fixture JSON path has no parent",
            )
        })?;
        fs::create_dir_all(parent)?;
        fs::write(path, value)
    }

    #[test]
    fn discovers_adjacent_and_launcher_profiles_and_resolves_inheritance() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let adjacent = temp.path().join(".minecraft");
        fs::create_dir_all(adjacent.join("saves/World"))?;
        fs::write(adjacent.join("saves/World/level.dat"), b"fixture-world")?;
        write_json(
            &adjacent.join("versions/1.19.4/1.19.4.json"),
            r#"{"id":"1.19.4","type":"release","releaseTime":"2022-12-07T00:00:00+00:00","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        write_json(
            &adjacent
                .join("versions/fabric-loader-0.15.11-1.20.1/fabric-loader-0.15.11-1.20.1.json"),
            r#"{"id":"fabric-loader-0.15.11-1.20.1","type":"release","releaseTime":"2023-06-07T00:00:00+00:00","inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.15.11"}],"mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient"}"#,
        )?;
        let launcher = temp.path().join("launcher");
        let profile_game = temp.path().join("profile-game");
        fs::create_dir_all(&profile_game)?;
        write_json(
            &launcher.join("launcher_profiles.json"),
            &format!(
                r#"{{"profiles":{{"fab":{{"name":"Modded","gameDir":{:?},"lastVersionId":"custom-fabric-profile"}}}}}}"#,
                profile_game.to_string_lossy()
            ),
        )?;
        write_json(
            &launcher.join("versions/custom-fabric-profile/custom-fabric-profile.json"),
            r#"{"id":"custom-fabric-profile","inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.5"}],"mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient"}"#,
        )?;
        write_json(
            &launcher.join("versions/1.20.1/1.20.1.json"),
            r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        write_json(
            &launcher.join("version_manifest_v2.json"),
            r#"{"latest":{"release":"1.21.1","snapshot":"25w01a"}}"#,
        )?;

        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: Some(adjacent.clone()),
            official_launcher: Some(launcher),
        });
        assert_eq!(report.sources.len(), 2);
        assert_eq!(report.candidates.len(), 3);
        assert_eq!(report.sources[0].candidates_found, 2);
        let adjacent_candidates = report
            .candidates
            .iter()
            .filter(|candidate| candidate.source == SourceKind::AdjacentMinecraft)
            .collect::<Vec<_>>();
        assert_eq!(adjacent_candidates.len(), 2);
        assert!(
            adjacent_candidates
                .iter()
                .all(|candidate| candidate.importable)
        );
        assert!(adjacent_candidates.iter().any(|candidate| {
            candidate.minecraft_version.as_deref() == Some("1.19.4")
                && candidate
                    .loader
                    .as_ref()
                    .is_some_and(|loader| loader.kind == LoaderKind::Vanilla)
        }));
        assert!(adjacent_candidates.iter().any(|candidate| {
            candidate.minecraft_version.as_deref() == Some("1.20.1")
                && candidate
                    .loader
                    .as_ref()
                    .is_some_and(|loader| loader.kind == LoaderKind::Fabric)
        }));
        let modded = report
            .candidates
            .iter()
            .find(|candidate| candidate.name == "Modded")
            .ok_or_else(|| anyhow::anyhow!("custom gameDir profile was not discovered"))?;
        assert_eq!(modded.name, "Modded");
        assert_eq!(modded.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(modded.game_directory, profile_game);
        assert!(modded.importable);
        assert_eq!(
            modded
                .loader
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Fabric loader was not detected"))?
                .kind,
            LoaderKind::Fabric
        );
        assert_eq!(
            modded
                .loader
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Fabric loader was not detected"))?
                .version
                .as_deref(),
            Some("0.16.5")
        );
        assert_eq!(modded.latest_release.as_deref(), Some("1.21.1"));
        assert_eq!(modded.latest_snapshot.as_deref(), Some("25w01a"));
        Ok(())
    }

    #[test]
    fn concrete_versions_include_year_releases_without_accepting_custom_components() {
        for version in [
            "26.3",
            "26.4-snapshot-4",
            "1.0",
            "1.20.1",
            "1.21.4-pre2",
            "1.21.4-rc1",
            "24w14a",
            "b1.7.3",
            "a1.2.6",
        ] {
            assert!(looks_like_minecraft_version(version), "{version}");
        }
        for version in [
            "latest-release",
            "latest-snapshot",
            "../1.20.1",
            "1.20.1-optifine",
            "private-profile-name",
        ] {
            assert!(!looks_like_minecraft_version(version), "{version}");
        }
    }

    #[test]
    fn corrupt_latest_alias_falls_back_to_concrete_local_metadata_without_recursing()
    -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("minecraft");
        write_json(
            &root.join("version_manifest_v2.json"),
            r#"{"latest":{"release":"latest-release","snapshot":"latest-release"}}"#,
        )?;
        write_json(
            &root.join("versions/26.3/26.3.json"),
            r#"{"id":"26.3","type":"release","releaseTime":"2026-10-01T00:00:00Z","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        let candidate = make_candidate(
            SourceKind::OfficialLauncher,
            &root,
            &root,
            "Current release",
            Some("latest-release".into()),
        );
        assert!(candidate.importable);
        assert_eq!(candidate.minecraft_version.as_deref(), Some("26.3"));
        let empty = temp.path().join("empty");
        write_json(
            &empty.join("version_manifest_v2.json"),
            r#"{"latest":{"release":"latest-release"}}"#,
        )?;
        let unresolved = make_candidate(
            SourceKind::OfficialLauncher,
            &empty,
            &empty,
            "Unresolved",
            Some("latest-release".into()),
        );
        assert!(!unresolved.importable);
        Ok(())
    }

    #[test]
    fn adjacent_empty_directory_has_no_game_candidates() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join(".minecraft");
        fs::create_dir(&root)?;
        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: Some(root),
            official_launcher: None,
        });
        assert!(report.candidates.is_empty());
        assert_eq!(report.sources[0].candidates_found, 0);
        Ok(())
    }

    #[test]
    fn adjacent_empty_profile_object_falls_back_to_local_versions() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join(".minecraft");
        write_json(&root.join("launcher_profiles.json"), r#"{"profiles":{}}"#)?;
        write_json(
            &root.join("versions/1.21.1/1.21.1.json"),
            r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: Some(root),
            official_launcher: None,
        });
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(
            report.candidates[0].configured_version.as_deref(),
            Some("1.21.1")
        );
        assert!(report.candidates[0].importable);
        Ok(())
    }

    #[test]
    fn local_release_alias_uses_latest_cached_version_metadata() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join(".minecraft");
        write_json(
            &root.join("versions/1.20.1/1.20.1.json"),
            r#"{"id":"1.20.1","type":"release","releaseTime":"2023-06-07T00:00:00+00:00","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        write_json(
            &root.join("versions/1.21.1/1.21.1.json"),
            r#"{"id":"1.21.1","type":"release","releaseTime":"2024-06-13T00:00:00+00:00","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        fs::create_dir_all(&root)?;
        assert_eq!(latest_version(&root, "release").as_deref(), Some("1.21.1"));
        let candidate = make_candidate(
            SourceKind::OfficialLauncher,
            &root,
            &root,
            "Latest release",
            Some("latest-release".into()),
        );
        assert!(candidate.importable);
        assert_eq!(candidate.minecraft_version.as_deref(), Some("1.21.1"));
        assert_eq!(
            candidate.loader.as_ref().map(|loader| loader.kind),
            Some(LoaderKind::Vanilla)
        );
        Ok(())
    }

    #[test]
    fn adjacent_version_directory_with_mutable_data_is_its_game_directory() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join(".minecraft");
        let version_directory = root.join("versions/custom-version");
        write_json(
            &version_directory.join("custom-version.json"),
            r#"{"id":"custom-version","inheritsFrom":"1.20.1","libraries":[{"name":"net.minecraftforge:forge:1.20.1-47.2.0"}],"mainClass":"cpw.mods.modlauncher.Launcher"}"#,
        )?;
        fs::create_dir_all(version_directory.join("saves/World"))?;
        fs::write(
            version_directory.join("saves/World/level.dat"),
            b"isolated world",
        )?;
        fs::write(version_directory.join("options.txt"), b"isolated options")?;
        write_json(
            &root.join("versions/1.20.1/1.20.1.json"),
            r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: Some(root.clone()),
            official_launcher: None,
        });
        let candidate = report
            .candidates
            .iter()
            .find(|candidate| candidate.configured_version.as_deref() == Some("custom-version"))
            .ok_or_else(|| anyhow::anyhow!("isolated version was not discovered"))?;
        assert_eq!(candidate.game_directory, version_directory);
        assert_eq!(candidate.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(
            candidate.loader.as_ref().map(|loader| loader.kind),
            Some(LoaderKind::Forge)
        );
        assert_eq!(
            candidate
                .loader
                .as_ref()
                .and_then(|loader| loader.version.as_deref()),
            Some("1.20.1-47.2.0")
        );
        Ok(())
    }

    #[test]
    fn distinct_launcher_profiles_sharing_version_and_game_directory_remain_distinct()
    -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("launcher");
        let game = temp.path().join("shared-game");
        fs::create_dir_all(&game)?;
        let profiles = format!(
            r#"{{"profiles":{{"work":{{"name":"Work","gameDir":{:?},"lastVersionId":"1.21.1"}},"play":{{"name":"Play","gameDir":{:?},"lastVersionId":"1.21.1"}}}}}}"#,
            game.to_string_lossy(),
            game.to_string_lossy()
        );
        write_json(&root.join("launcher_profiles.json"), &profiles)?;
        write_json(
            &root.join("launcher_profiles_microsoft_store.json"),
            &format!(
                r#"{{"profiles":{{"play":{{"name":"Duplicate","gameDir":{:?},"lastVersionId":"1.21.1"}}}}}}"#,
                game.to_string_lossy()
            ),
        )?;
        write_json(
            &root.join("versions/1.21.1/1.21.1.json"),
            r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: None,
            official_launcher: Some(root),
        });
        assert_eq!(report.candidates.len(), 2);
        assert_ne!(report.candidates[0].id, report.candidates[1].id);
        assert!(
            report
                .candidates
                .iter()
                .all(|candidate| candidate.importable)
        );
        Ok(())
    }

    #[test]
    fn detects_loader_ids_and_missing_metadata_stays_importable() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let game = temp.path().join("game");
        fs::create_dir_all(&game)?;
        let candidate = make_candidate(
            SourceKind::AdjacentMinecraft,
            &game,
            &game,
            "game",
            Some("1.20.1-neoforge-20.4.237".into()),
        );
        assert!(candidate.importable);
        assert_eq!(
            candidate
                .loader
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("NeoForge loader was not detected"))?
                .kind,
            LoaderKind::NeoForge
        );
        assert_eq!(candidate.minecraft_version.as_deref(), Some("1.20.1"));
        assert!(candidate.metadata_warning.is_some());
        assert_eq!(
            classify_version("forge-1.12.2-14.23.5.2860").0.kind,
            LoaderKind::Forge
        );
        assert_eq!(
            classify_version("forge-1.12.2-14.23.5.2860").1.as_deref(),
            Some("1.12.2")
        );
        assert_eq!(
            classify_version("quilt-loader-0.26.4-1.21").0.kind,
            LoaderKind::Quilt
        );
        assert_eq!(classify_version("1.21.1").0.kind, LoaderKind::Vanilla);
        Ok(())
    }

    #[test]
    fn copy_excludes_account_and_session_material_and_preserves_source() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("mods"))?;
        fs::create_dir_all(source.join("saves/Secret Base"))?;
        for world in ["accounts", "session", "secrets"] {
            fs::create_dir_all(source.join("saves").join(world))?;
            fs::write(source.join("saves").join(world).join("level.dat"), world)?;
        }
        fs::create_dir_all(source.join(".ssh"))?;
        fs::write(source.join("options.txt"), b"safe options")?;
        fs::write(source.join("mods/mod.jar"), b"mod bytes")?;
        fs::write(source.join("mods/token-helper.jar"), b"ordinary mod")?;
        fs::write(source.join("saves/Secret Base/level.dat"), b"world bytes")?;
        fs::write(source.join("saves/Secret Base/.env.local"), b"secret env")?;
        fs::write(source.join(".env"), b"secret env")?;
        fs::write(source.join(".env.local"), b"secret env")?;
        fs::write(source.join(".ssh/id_ed25519"), b"private key")?;
        fs::write(source.join("notes.txt"), b"unrelated top-level file")?;
        fs::write(source.join("launcher_accounts.json"), b"account secret")?;
        fs::write(source.join("session.json"), b"session secret")?;
        let candidate = make_candidate(
            SourceKind::AdjacentMinecraft,
            &source,
            &source,
            "source",
            Some("1.21.1".into()),
        );
        let original = fs::read(source.join("saves/Secret Base/level.dat"))?;
        let target = temp.path().join("Block/game-one");
        fs::create_dir_all(
            target
                .parent()
                .ok_or_else(|| anyhow::anyhow!("target has no parent"))?,
        )?;
        let result = copy_candidate(&candidate, &target, &CancellationToken::new())?;
        assert_eq!(result.files_copied, 7);
        assert_eq!(
            fs::read(target.join("saves/Secret Base/level.dat"))?,
            original
        );
        assert_eq!(
            fs::read(source.join("saves/Secret Base/level.dat"))?,
            original
        );
        for world in ["accounts", "session", "secrets"] {
            assert_eq!(
                fs::read(target.join("saves").join(world).join("level.dat"))?,
                world.as_bytes()
            );
        }
        assert!(!target.join("launcher_accounts.json").exists());
        assert!(!target.join("session.json").exists());
        assert!(!target.join(".env").exists());
        assert!(!target.join(".env.local").exists());
        assert!(!target.join("saves/Secret Base/.env.local").exists());
        assert!(!target.join(".ssh/id_ed25519").exists());
        assert!(!target.join("notes.txt").exists());
        assert_eq!(
            fs::read(target.join("mods/token-helper.jar"))?,
            b"ordinary mod"
        );
        assert_eq!(fs::read(target.join("options.txt"))?, b"safe options");
        Ok(())
    }

    #[test]
    fn custom_game_directory_loader_comes_from_local_library_metadata() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let launcher = temp.path().join("launcher");
        let custom_game = temp.path().join("custom-game");
        fs::create_dir_all(&custom_game)?;
        write_json(
            &launcher.join("launcher_profiles.json"),
            &format!(
                r#"{{"profiles":{{"custom":{{"name":"Custom","gameDir":{:?},"lastVersionId":"private-profile-name"}}}}}}"#,
                custom_game.to_string_lossy()
            ),
        )?;
        write_json(
            &launcher.join("versions/private-profile-name/private-profile-name.json"),
            r#"{"id":"private-profile-name","inheritsFrom":"1.21.1","libraries":[{"name":"org.quiltmc:quilt-loader:0.27.1"}],"mainClass":"org.quiltmc.loader.impl.launch.knot.KnotClient"}"#,
        )?;
        write_json(
            &launcher.join("versions/1.21.1/1.21.1.json"),
            r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: None,
            official_launcher: Some(launcher),
        });
        let candidate = report
            .candidates
            .first()
            .ok_or_else(|| anyhow::anyhow!("custom profile was not found"))?;
        assert!(candidate.importable);
        assert_eq!(candidate.minecraft_version.as_deref(), Some("1.21.1"));
        let loader = candidate
            .loader
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Quilt loader was not detected"))?;
        assert_eq!(loader.kind, LoaderKind::Quilt);
        assert_eq!(loader.version.as_deref(), Some("0.27.1"));
        Ok(())
    }

    #[test]
    fn unknown_custom_main_class_is_unsupported_instead_of_vanilla() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let launcher = temp.path().join("launcher");
        let game = temp.path().join("game");
        fs::create_dir_all(&game)?;
        write_json(
            &launcher.join("launcher_profiles.json"),
            &format!(
                r#"{{"profiles":{{"custom":{{"name":"Unknown","gameDir":{:?},"lastVersionId":"custom-client"}}}}}}"#,
                game.to_string_lossy()
            ),
        )?;
        write_json(
            &launcher.join("versions/custom-client/custom-client.json"),
            r#"{"id":"custom-client","inheritsFrom":"1.21.1","mainClass":"com.example.private.CustomClient"}"#,
        )?;
        write_json(
            &launcher.join("versions/1.21.1/1.21.1.json"),
            r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )?;
        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: None,
            official_launcher: Some(launcher),
        });
        let candidate = report
            .candidates
            .first()
            .ok_or_else(|| anyhow::anyhow!("profile was not found"))?;
        assert!(!candidate.importable);
        assert!(
            candidate
                .unsupported_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("unknown"))
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn bounded_json_reader_rejects_metadata_symlinks() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir()?;
        let target = temp.path().join("real.json");
        let link = temp.path().join("linked.json");
        fs::write(&target, r#"{"id":"1.21.1"}"#)?;
        symlink(&target, &link)?;
        assert!(read_json_bounded(&target, MAX_VERSION_BYTES).is_ok());
        assert!(read_json_bounded(&link, MAX_VERSION_BYTES).is_err());
        Ok(())
    }

    #[test]
    fn rejects_overlap_and_cancellation_without_publishing_partial_data() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("saves/world"))?;
        fs::write(source.join("saves/world/level.dat"), b"world")?;
        let candidate = make_candidate(
            SourceKind::AdjacentMinecraft,
            &source,
            &source,
            "source",
            Some("1.21.1".into()),
        );
        assert!(matches!(
            copy_candidate(
                &candidate,
                &source.join("nested"),
                &CancellationToken::new()
            ),
            Err(ImportError::Overlap)
        ));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let destination = temp.path().join("cancelled");
        assert!(matches!(
            copy_candidate(&candidate, &destination, &cancellation),
            Err(ImportError::Cancelled)
        ));
        assert!(!destination.exists());
        assert_eq!(fs::read(source.join("saves/world/level.dat"))?, b"world");
        Ok(())
    }

    #[test]
    fn each_game_copy_publishes_independently_and_symlinks_are_rejected() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("saves/world"))?;
        fs::write(source.join("saves/world/level.dat"), b"world")?;
        let candidate = make_candidate(
            SourceKind::AdjacentMinecraft,
            &source,
            &source,
            "source",
            Some("1.21.1".into()),
        );
        let first = temp.path().join("games/one");
        let second = temp.path().join("games/two");
        fs::create_dir_all(
            first
                .parent()
                .ok_or_else(|| anyhow::anyhow!("target has no parent"))?,
        )?;
        copy_candidate(&candidate, &first, &CancellationToken::new())?;
        copy_candidate(&candidate, &second, &CancellationToken::new())?;
        assert_eq!(fs::read(first.join("saves/world/level.dat"))?, b"world");
        assert_eq!(fs::read(second.join("saves/world/level.dat"))?, b"world");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(
                first.join("saves/world/level.dat"),
                source.join("linked.dat"),
            )?;
            let third = temp.path().join("games/three");
            assert!(matches!(
                copy_candidate(&candidate, &third, &CancellationToken::new()),
                Err(ImportError::Symlink(_))
            ));
            assert!(!third.exists());
        }
        Ok(())
    }
}
