use super::classification::classify_version;
use super::{
    DetectedLoader, LoaderKind, detect_loader, is_known_main_class, looks_like_minecraft_version,
    minecraft_version_from_id,
};
use std::path::Path;
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read as _},
    path::Component,
};
const MAX_VERSION_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROFILES: usize = 256;
const MAX_INHERITANCE_DEPTH: usize = 16;

pub(super) struct VersionMetadata {
    pub(super) minecraft_version: Option<String>,
    pub(super) loader: Option<DetectedLoader>,
    pub(super) warning: Option<String>,
    pub(super) supported: bool,
    pub(super) metadata_found: bool,
}

pub(super) fn resolve_version(metadata_root: &Path, configured: Option<&str>) -> VersionMetadata {
    let Some(configured) = configured else {
        return VersionMetadata {
            minecraft_version: None,
            loader: None,
            warning: Some("No launch version is configured".into()),
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
    let mut complete = false;
    for _ in 0..MAX_INHERITANCE_DEPTH {
        if !safe_version_id(&version_id) || !seen.insert(version_id.clone()) {
            supported = false;
            break;
        }
        let path = versions_dir
            .join(&version_id)
            .join(format!("{version_id}.json"));
        let Ok(json) = read_json_bounded(&path, MAX_VERSION_BYTES) else {
            supported = false;
            break;
        };
        found = true;
        if json
            .get("id")
            .is_some_and(|id| id.as_str() != Some(version_id.as_str()))
            || json
                .get("inheritsFrom")
                .is_some_and(|parent| parent.as_str().is_none())
        {
            supported = false;
            break;
        }
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
            complete = true;
            break;
        }
    }
    supported &= complete;
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

pub(super) fn latest_version(root: &Path, release_type: &str) -> Option<String> {
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
        if !safe_version_id(&id) || !entry.path().is_dir() {
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

pub(super) fn read_json_bounded(path: &Path, max_bytes: u64) -> io::Result<serde_json::Value> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file or exceeds size limit",
        ));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_err(io::Error::other)? > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(super) fn safe_version_id(id: &str) -> bool {
    !id.is_empty()
        && !id.chars().any(char::is_control)
        && !matches!(id, "latest-release" | "latest-snapshot")
        && id.len() <= 256
        && Path::new(id)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && !id
            .chars()
            .any(|character| character == '/' || character == '\\')
}
