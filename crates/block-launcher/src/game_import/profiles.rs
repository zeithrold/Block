use super::discovery::make_candidate;
use super::metadata::read_json_bounded;
use super::{GameCandidate, GameDataEvidence, SourceKind};
use std::path::Path;
const MAX_PROFILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROFILES: usize = 256;

pub(super) fn discover_profiles(
    root: &Path,
    source: SourceKind,
) -> (Vec<GameCandidate>, Vec<String>, bool) {
    let mut candidates: Vec<GameCandidate> = Vec::new();
    let mut warnings = Vec::new();
    let mut has_profiles = false;
    let mut seen_profile_keys = std::collections::BTreeMap::new();
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
            let identity = (
                profile.get("gameDir").cloned(),
                profile.get("lastVersionId").cloned(),
            );
            if let Some(previous) = seen_profile_keys.insert(key.clone(), identity.clone()) {
                if previous != identity {
                    let warning = format!("Conflicting launcher configuration for {key}");
                    warnings.push(warning.clone());
                    for candidate in &mut candidates {
                        if candidate.id.ends_with(&format!(":{key}")) {
                            candidate.data = GameDataEvidence::Unknown;
                            candidate.metadata_warning = Some(warning.clone());
                        }
                    }
                }
                continue;
            }
            has_profiles = true;
            let profile_name = profile
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(key);
            let configured_version = profile
                .get("lastVersionId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let mut candidate = make_candidate(source, root, profile_name, configured_version);
            match profile
                .get("gameDir")
                .and_then(serde_json::Value::as_str)
                .filter(|path| !path.trim().is_empty())
            {
                Some(path) => {
                    let path = Path::new(path);
                    let path = if path.is_absolute() {
                        path.to_owned()
                    } else {
                        root.join(path)
                    };
                    match crate::catalog::normalize_directory(&path) {
                        Ok(directory) => {
                            candidate.data = if directory == candidate.resource_directory {
                                GameDataEvidence::Shared { directory }
                            } else {
                                GameDataEvidence::Isolated { directory }
                            };
                        }
                        Err(error) => {
                            let reason = error.to_string();
                            candidate.metadata_warning = Some(format!(
                                "{}; {reason}",
                                candidate
                                    .metadata_warning
                                    .as_deref()
                                    .unwrap_or("Uncertain data location")
                            ));
                        }
                    }
                }
                None => {
                    candidate.metadata_warning = Some(format!(
                        "{}; missing, empty or unsupported gameDir",
                        candidate
                            .metadata_warning
                            .as_deref()
                            .unwrap_or("Uncertain data location")
                    ))
                }
            }
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
