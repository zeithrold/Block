//! Bounded inspection of a plain ZIP that ships a Minecraft game directory.
//!
//! A plain archive declares no target, so the game root and the launch target
//! are both read from the archive's own entries. Version profiles are read-only
//! data: `versions/<id>/<id>.json` counts through its own `id`, its
//! `inheritsFrom` and its library or entry-point metadata, never through the
//! ZIP name, a mod file name or a directory name. Nothing is extracted or run.

use std::{collections::BTreeSet, fs::File};

use anyhow::{bail, ensure};
use serde_json::Value;
use zip::ZipArchive;

use super::{
    LoaderKind, PackContent, PackFormat, PackInspection, TargetEvidence,
    archive::{self, Entry},
};
use crate::game_import;

/// Game directories launchers create for the Minecraft installation itself.
const NAMED_ROOTS: &[&str] = &[".minecraft", "minecraft"];

/// Direct children that mark a Minecraft game-content root. Launcher and
/// library directories are deliberately absent: shared installation data is
/// not evidence of the content a pack provides.
const CONTENT_DIRECTORIES: &[&str] = &[
    "config",
    "kubejs",
    "mods",
    "resourcepacks",
    "saves",
    "scripts",
    "shaderpacks",
    "versions",
];
const CONTENT_FILES: &[&str] = &["options.txt"];

/// The legacy entry point of supported Forge 1.12.x profiles. Custom and
/// OptiFine installations use it too, so it never identifies a target alone.
const LAUNCHWRAPPER: &str = "net.minecraft.launchwrapper.Launch";

/// Inspect a plain archive as a Minecraft game directory.
///
/// The archive owner has validated every entry path and rejected the standard
/// pack formats, so this parser only selects the game root and reports the
/// target the archive's own metadata can prove.
///
/// # Errors
/// Rejects an unusable name, a missing game root, several possible game roots,
/// and metadata that cannot be read within the archive owner's bounds.
pub(super) fn parse(
    archive: &mut ZipArchive<File>,
    entries: &[Entry],
    source_name: &str,
) -> anyhow::Result<PackInspection> {
    ensure!(
        !source_name.trim().is_empty() && !source_name.chars().any(char::is_control),
        "pack has no usable name"
    );
    let root = game_root(entries)?;
    let target = resolve_target(archive, entries, &root)?;
    Ok(PackInspection {
        format: PackFormat::Plain,
        name: source_name.to_owned(),
        version: None,
        minecraft_version: target.minecraft_version,
        loader: target.loader,
        target_evidence: target.evidence,
        has_mods: has_mods(entries, &root),
        files: Vec::new(),
        missing_files: Vec::new(),
        content: PackContent::Plain { root },
        content_entries: Vec::new(),
        archive_prefix: String::new(),
    })
}

/// The single archive prefix that holds the pack's game content.
///
/// The archive root, a `.minecraft` or `minecraft` directory, and one outer
/// wrapper around either are candidates. A prefix only counts through its own
/// direct children, so a named root is never credited for containing one, and
/// arbitrary nesting never becomes a game root.
fn game_root(entries: &[Entry]) -> anyhow::Result<String> {
    let mut roots = BTreeSet::new();
    for entry in entries {
        let mut parts = entry.path.split('/');
        let Some(wrapper) = parts.next() else {
            continue;
        };
        if marker_child(entry, "") {
            roots.insert(String::new());
        }
        let Some(second) = parts.next() else {
            continue;
        };
        if !CONTENT_DIRECTORIES.contains(&wrapper.to_ascii_lowercase().as_str())
            && marker_child(entry, wrapper)
        {
            roots.insert(wrapper.to_owned());
        }
        if NAMED_ROOTS.contains(&second.to_ascii_lowercase().as_str())
            && marker_child(entry, &format!("{wrapper}/{second}"))
        {
            roots.insert(format!("{wrapper}/{second}"));
        }
    }
    let mut roots = roots.into_iter();
    let Some(first) = roots.next() else {
        bail!("pack contains no Minecraft game content directory");
    };
    let rest: Vec<String> = roots.map(|root| named(&root).to_owned()).collect();
    ensure!(
        rest.is_empty(),
        "pack contains several possible Minecraft game content directories: {} and {}",
        named(&first),
        rest.join(", ")
    );
    Ok(first)
}

/// A root with no path text of its own is the archive root.
fn named(root: &str) -> &str {
    if root.is_empty() { "(zip root)" } else { root }
}

/// Whether the entry is a direct child of `root` that marks game content.
fn marker_child(entry: &Entry, root: &str) -> bool {
    let Some(rest) = relative(entry, root) else {
        return false;
    };
    let (first, deeper) = rest
        .split_once('/')
        .map_or((rest, false), |(first, _)| (first, true));
    let child = first.to_ascii_lowercase();
    // A deeper path implies that its first component is a directory.
    if deeper || entry.directory {
        CONTENT_DIRECTORIES.contains(&child.as_str())
    } else {
        CONTENT_FILES.contains(&child.as_str())
    }
}

/// The entry path below an archive prefix.
fn relative<'a>(entry: &'a Entry, root: &str) -> Option<&'a str> {
    let rest = if root.is_empty() {
        entry.path.as_str()
    } else {
        entry.path.strip_prefix(root)?.strip_prefix('/')?
    };
    (!rest.is_empty()).then_some(rest)
}

/// The `versions/<id>/<id>.json` profiles inside the selected game root.
fn launch_metadata<'a>(entries: &'a [Entry], root: &str) -> Vec<&'a Entry> {
    let mut profiles: Vec<&Entry> = entries
        .iter()
        .filter(|entry| !entry.directory && is_profile(entry, root))
        .collect();
    // Evidence is reported and selected independently of ZIP entry order.
    profiles.sort_by(|left, right| left.path.cmp(&right.path));
    profiles
}

/// A profile whose directory and file names agree, as launchers write it.
fn is_profile(entry: &Entry, root: &str) -> bool {
    let Some(rest) = relative(entry, root) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix("versions/") else {
        return false;
    };
    let Some((id, file)) = rest.split_once('/') else {
        return false;
    };
    file.strip_suffix(".json") == Some(id)
}

/// The launch target the selected game root's own metadata proves.
struct Target {
    minecraft_version: Option<String>,
    loader: Option<(LoaderKind, String)>,
    evidence: TargetEvidence,
}

/// Resolve the target from the version profiles in `root`.
///
/// One exact supported target is evidence. Several distinct targets, or a
/// target Block cannot pin, leave the choice to the user; a version survives as
/// a hint only when every profile that identified one agrees.
fn resolve_target(
    archive: &mut ZipArchive<File>,
    entries: &[Entry],
    root: &str,
) -> anyhow::Result<Target> {
    let mut profiles = Vec::new();
    for entry in launch_metadata(entries, root) {
        profiles.push(read_profile(
            &entry.path,
            &archive::metadata(archive, entry)?,
        )?);
    }
    let mut pins: Vec<Pin<'_>> = Vec::new();
    for pin in profiles.iter().filter_map(|profile| profile.pin()) {
        if !pins
            .iter()
            .any(|known| known.minecraft == pin.minecraft && known.loader == pin.loader)
        {
            pins.push(pin);
        }
    }
    if profiles.iter().any(|profile| profile.pin().is_none()) {
        return Ok(choose_manually(&profiles));
    }
    let [pin] = pins.as_slice() else {
        return Ok(choose_manually(&profiles));
    };
    if pin.loader.is_none() && has_mods(entries, root) {
        // Mods prove neither Vanilla nor the loader they need.
        return Ok(choose_manually(&profiles));
    }
    Ok(Target {
        minecraft_version: Some(pin.minecraft.to_owned()),
        loader: pin.loader.cloned(),
        evidence: TargetEvidence::Detected {
            source: pin.source.to_owned(),
        },
    })
}

/// A target the user still has to choose, with a hint when one survives.
fn choose_manually(profiles: &[Profile]) -> Target {
    Target {
        minecraft_version: version_hint(profiles),
        loader: None,
        evidence: TargetEvidence::NeedsSelection,
    }
}

/// The version every profile that identified one agrees on.
fn version_hint(profiles: &[Profile]) -> Option<String> {
    let mut versions = BTreeSet::new();
    for profile in profiles {
        if let Some(version) = profile.version() {
            versions.insert(version);
        }
    }
    if versions.len() == 1 {
        versions.into_iter().next().map(str::to_owned)
    } else {
        None
    }
}

/// Whether the selected root ships mod files, which no Vanilla profile explains.
fn has_mods(entries: &[Entry], root: &str) -> bool {
    entries.iter().any(|entry| {
        if entry.directory {
            return false;
        }
        relative(entry, root).is_some_and(|rest| {
            let mut parts = rest.split('/');
            parts
                .next()
                .is_some_and(|first| first.eq_ignore_ascii_case("mods"))
                && parts.next().is_some()
        })
    })
}

/// One distinct target the selected root pins, with the metadata evidencing it.
struct Pin<'a> {
    source: &'a str,
    minecraft: &'a str,
    loader: Option<&'a (LoaderKind, String)>,
}

/// What one version profile establishes about the pack's launch target.
enum Profile {
    /// One exact supported target, the only evidence `Detected` accepts.
    Pinned {
        source: String,
        minecraft: String,
        loader: Option<(LoaderKind, String)>,
    },
    /// An exact Minecraft version whose loader is unknown or unsupported.
    Hint { minecraft: String },
    /// No usable identity.
    Unknown,
}

impl Profile {
    fn pinned(source: &str, minecraft: String, loader: Option<(LoaderKind, String)>) -> Self {
        Self::Pinned {
            source: source.to_owned(),
            minecraft,
            loader,
        }
    }

    fn hint(minecraft: String) -> Self {
        Self::Hint { minecraft }
    }

    /// The distinct target this profile pins.
    fn pin(&self) -> Option<Pin<'_>> {
        match self {
            Self::Pinned {
                source,
                minecraft,
                loader,
            } => Some(Pin {
                source,
                minecraft,
                loader: loader.as_ref(),
            }),
            Self::Hint { .. } | Self::Unknown => None,
        }
    }

    /// The exact Minecraft version this profile identified.
    fn version(&self) -> Option<&str> {
        match self {
            Self::Pinned { minecraft, .. } | Self::Hint { minecraft } => Some(minecraft),
            Self::Unknown => None,
        }
    }
}

/// Read one version profile as the target it pins, or as a version hint.
///
/// The profile's own `id` identifies it: a ZIP name, a mod file name or a
/// version directory name says nothing about what the profile launches. An
/// entry point Block cannot identify is rejected rather than read as Vanilla,
/// and launchwrapper counts only behind an exact Forge build.
fn read_profile(source: &str, bytes: &[u8]) -> anyhow::Result<Profile> {
    let Ok(json) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(Profile::Unknown);
    };
    let main_class = json
        .get("mainClass")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure!(
        main_class.is_empty()
            || game_import::is_known_main_class(main_class)
            || main_class == LAUNCHWRAPPER,
        "Minecraft version metadata declares an unsupported custom entry point"
    );
    let Some(id) = json
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
    else {
        return Ok(Profile::Unknown);
    };
    let inherits = json.get("inheritsFrom").and_then(Value::as_str);
    let Some(minecraft) = game_import::minecraft_version_from_id(id)
        .or_else(|| inherits.and_then(game_import::minecraft_version_from_id))
    else {
        return Ok(Profile::Unknown);
    };
    let Some(loader) = game_import::detect_loader(&json, id) else {
        return Ok(Profile::hint(minecraft));
    };
    // Legacy Forge shares this entry point with custom and OptiFine setups, so
    // only an exact Forge build makes it evidence.
    let pinned_forge = loader.kind == game_import::LoaderKind::Forge && loader.version.is_some();
    if main_class == LAUNCHWRAPPER && !pinned_forge {
        return Ok(Profile::hint(minecraft));
    }
    if main_class.is_empty() {
        return Ok(Profile::hint(minecraft));
    }
    let modded = loader.kind != game_import::LoaderKind::Vanilla;
    let anchored = game_import::looks_like_minecraft_version(id) || inherits.is_none();
    if !modded && !anchored {
        return Ok(Profile::hint(minecraft));
    }
    if !modded {
        return Ok(Profile::pinned(source, minecraft, None));
    }
    let kind = catalog_kind(loader.kind);
    let Some(version) = loader.version else {
        return Ok(Profile::hint(minecraft));
    };
    let version = loader_version(&kind, &minecraft, &version);
    if validate_build(&kind, &minecraft, &version).is_err() {
        return Ok(Profile::hint(minecraft));
    }
    Ok(Profile::pinned(source, minecraft, Some((kind, version))))
}

/// The catalog loader for a detected loader.
fn catalog_kind(kind: game_import::LoaderKind) -> LoaderKind {
    match kind {
        game_import::LoaderKind::Vanilla => LoaderKind::Vanilla,
        game_import::LoaderKind::Fabric => LoaderKind::Fabric,
        game_import::LoaderKind::Quilt => LoaderKind::Quilt,
        game_import::LoaderKind::Forge => LoaderKind::Forge,
        game_import::LoaderKind::NeoForge => LoaderKind::NeoForge,
    }
}

/// Block stores Forge builds as `<minecraft>-<loader>`, and NeoForge 1.20.1
/// still ships from the Forge artifact, so it uses the same form.
fn loader_version(kind: &LoaderKind, minecraft: &str, version: &str) -> String {
    let game_prefixed = match kind {
        LoaderKind::Forge => true,
        LoaderKind::NeoForge => minecraft == "1.20.1",
        LoaderKind::Vanilla | LoaderKind::Fabric | LoaderKind::Quilt => false,
    };
    let already_prefixed = version
        .strip_prefix(minecraft)
        .is_some_and(|rest| rest.starts_with('-'));
    if game_prefixed && !already_prefixed {
        format!("{minecraft}-{version}")
    } else {
        version.to_owned()
    }
}

/// Validate the exact build against the same catalogs a chosen target uses.
fn validate_build(kind: &LoaderKind, minecraft: &str, version: &str) -> anyhow::Result<()> {
    match kind {
        LoaderKind::Vanilla => bail!("Vanilla has no loader build"),
        LoaderKind::Forge => crate::install::forge::validate_version(minecraft, version),
        LoaderKind::NeoForge => crate::install::neoforge::validate_version(minecraft, version),
        LoaderKind::Fabric | LoaderKind::Quilt => crate::install::fabric::validate_version(version),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn inspect(files: &[(&str, &[u8])]) -> anyhow::Result<PackInspection> {
        inspect_as("Example Pack", files)
    }

    /// Write a synthetic archive and inspect it through the production parser
    /// with the entries the archive owner would have validated.
    fn inspect_as(name: &str, files: &[(&str, &[u8])]) -> anyhow::Result<PackInspection> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("pack.zip");
        let mut entries = Vec::new();
        {
            let mut zip = ZipWriter::new(File::create(&path)?);
            for (index, (entry, bytes)) in files.iter().enumerate() {
                let directory = entry.ends_with('/');
                if directory {
                    zip.add_directory(entry.trim_end_matches('/'), SimpleFileOptions::default())?;
                } else {
                    zip.start_file(*entry, SimpleFileOptions::default())?;
                    zip.write_all(bytes)?;
                }
                entries.push(Entry {
                    index,
                    path: entry.trim_end_matches('/').to_owned(),
                    directory,
                    size: bytes.len() as u64,
                });
            }
            zip.finish()?;
        }
        let mut zip = ZipArchive::new(File::open(&path)?)?;
        parse(&mut zip, &entries, name)
    }

    fn vanilla(version: &str) -> String {
        format!(
            r#"{{"id":"{version}","type":"release","mainClass":"net.minecraft.client.main.Main"}}"#
        )
    }

    fn fabric(version: &str, loader: &str) -> String {
        format!(
            r#"{{"id":"fabric-loader-{loader}-{version}","inheritsFrom":"{version}","libraries":[{{"name":"net.fabricmc:fabric-loader:{loader}"}}],"mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient"}}"#
        )
    }

    fn forge(version: &str, loader: &str) -> String {
        format!(
            r#"{{"id":"{version}-forge-{loader}","libraries":[{{"name":"net.minecraftforge:forge:{version}-{loader}"}}],"mainClass":"cpw.mods.modlauncher.Launcher"}}"#
        )
    }

    fn neoforge(version: &str, loader: &str) -> String {
        format!(
            r#"{{"id":"{version}-neoforge-{loader}","libraries":[{{"name":"net.neoforged:neoforge:{loader}"}}],"mainClass":"cpw.mods.modlauncher.Launcher"}}"#
        )
    }

    #[test]
    fn reads_root_named_and_single_wrapper_game_roots() -> anyhow::Result<()> {
        for (files, root) in [
            (vec![("mods/example.jar", b"jar".as_slice())], ""),
            (
                vec![(".minecraft/options.txt", b"fov:70".as_slice())],
                ".minecraft",
            ),
            (
                vec![("minecraft/resourcepacks/pack.zip", b"zip".as_slice())],
                "minecraft",
            ),
            (
                vec![
                    ("Pack/mods/example.jar", b"jar".as_slice()),
                    ("Pack/readme.txt", b"text".as_slice()),
                ],
                "Pack",
            ),
            (
                vec![("Pack/.minecraft/mods/example.jar", b"jar".as_slice())],
                "Pack/.minecraft",
            ),
            (
                vec![("Pack/minecraft/config/example.toml", b"cfg".as_slice())],
                "Pack/minecraft",
            ),
        ] {
            let inspection = inspect(&files)?;
            assert_eq!(inspection.format, PackFormat::Plain, "{files:?}");
            assert_eq!(inspection.name, "Example Pack", "{files:?}");
            assert_eq!(
                inspection.content,
                PackContent::Plain {
                    root: root.to_owned()
                },
                "{files:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn requires_a_recognizable_game_root() -> anyhow::Result<()> {
        for (index, files) in [
            vec![("readme.txt", b"text".as_slice())],
            vec![("example.jar", b"jar".as_slice())],
            vec![("gallery/src/mods/example.jar", b"jar".as_slice())],
            vec![("notes/2024/report.md", b"md".as_slice())],
        ]
        .into_iter()
        .enumerate()
        {
            assert!(inspect(&files).is_err(), "case {index}");
        }
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_game_roots() -> anyhow::Result<()> {
        for (index, files) in [
            vec![
                ("mods/a.jar", b"a".as_slice()),
                (".minecraft/mods/b.jar", b"b".as_slice()),
            ],
            vec![
                ("first/mods/a.jar", b"a".as_slice()),
                ("second/config/b.toml", b"b".as_slice()),
            ],
            vec![
                ("Pack/mods/a.jar", b"a".as_slice()),
                ("Pack/.minecraft/config/b.toml", b"b".as_slice()),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            assert!(inspect(&files).is_err(), "case {index}");
        }
        Ok(())
    }

    #[test]
    fn requests_a_target_when_no_reliable_metadata_exists() -> anyhow::Result<()> {
        for (index, files) in [
            vec![
                ("mods/example.jar", b"jar".as_slice()),
                ("config/example.toml", b"cfg".as_slice()),
            ],
            vec![
                ("mods/example.jar", b"jar".as_slice()),
                ("versions/1.20.1/1.20.1.json", b"not metadata".as_slice()),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            let inspection = inspect(&files)?;
            assert_eq!(inspection.format, PackFormat::Plain, "case {index}");
            assert_eq!(inspection.version, None, "case {index}");
            assert_eq!(inspection.minecraft_version, None, "case {index}");
            assert_eq!(inspection.loader, None, "case {index}");
            assert_eq!(
                inspection.target_evidence,
                TargetEvidence::NeedsSelection,
                "case {index}"
            );
            assert!(!inspection.target_confirmed(), "case {index}");
            assert!(inspection.files.is_empty(), "case {index}");
            assert!(inspection.missing_files.is_empty(), "case {index}");
            assert!(inspection.content_entries.is_empty(), "case {index}");
            assert_eq!(inspection.archive_prefix, "", "case {index}");
        }
        Ok(())
    }

    #[test]
    fn detects_a_vanilla_profile_only_without_mods() -> anyhow::Result<()> {
        let profile = vanilla("1.20.1");
        let source = "versions/1.20.1/1.20.1.json";
        let inspection = inspect(&[
            ("options.txt", b"fov:70".as_slice()),
            ("mods/", b"".as_slice()),
            (source, profile.as_bytes()),
        ])?;
        assert_eq!(
            inspection.target_evidence,
            TargetEvidence::Detected {
                source: source.to_owned()
            }
        );
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(inspection.loader, None);
        assert!(!inspection.has_mods);
        assert!(inspection.target_confirmed());

        let modded = inspect(&[
            ("mods/example.jar", b"jar".as_slice()),
            (source, profile.as_bytes()),
        ])?;
        assert_eq!(modded.target_evidence, TargetEvidence::NeedsSelection);
        assert_eq!(modded.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(modded.loader, None);
        assert!(modded.has_mods);
        assert!(!modded.target_confirmed());
        Ok(())
    }

    #[test]
    fn detects_pinned_fabric_forge_and_neoforge_profiles() -> anyhow::Result<()> {
        for (id, profile, kind, version) in [
            (
                "fabric-loader-0.15.11-1.20.1",
                fabric("1.20.1", "0.15.11"),
                LoaderKind::Fabric,
                "0.15.11",
            ),
            (
                "1.20.1-forge-47.2.0",
                forge("1.20.1", "47.2.0"),
                LoaderKind::Forge,
                "1.20.1-47.2.0",
            ),
            (
                "1.20.1-neoforge-47.1.106",
                neoforge("1.20.1", "47.1.106"),
                LoaderKind::NeoForge,
                "1.20.1-47.1.106",
            ),
        ] {
            let source = format!("versions/{id}/{id}.json");
            let inspection = inspect(&[
                (source.as_str(), profile.as_bytes()),
                ("mods/example.jar", b"jar".as_slice()),
            ])?;
            assert_eq!(
                inspection.minecraft_version.as_deref(),
                Some("1.20.1"),
                "{id}"
            );
            assert_eq!(inspection.loader, Some((kind, version.to_owned())), "{id}");
            assert_eq!(
                inspection.target_evidence,
                TargetEvidence::Detected { source },
                "{id}"
            );
            assert_eq!(
                inspection.content,
                PackContent::Plain {
                    root: String::new()
                },
                "{id}"
            );
            assert!(inspection.target_confirmed(), "{id}");
        }
        Ok(())
    }

    #[test]
    fn recognizes_launchwrapper_only_with_a_pinned_forge_loader() -> anyhow::Result<()> {
        let legacy = r#"{"id":"1.12.2-forge-14.23.5.2859","inheritsFrom":"1.12.2","libraries":[{"name":"net.minecraftforge:forge:1.12.2-14.23.5.2859"}],"mainClass":"net.minecraft.launchwrapper.Launch"}"#;
        let source = "versions/1.12.2-forge-14.23.5.2859/1.12.2-forge-14.23.5.2859.json";
        let inspection = inspect(&[
            (source, legacy.as_bytes()),
            ("mods/example.jar", b"jar".as_slice()),
        ])?;
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.12.2"));
        assert_eq!(
            inspection.loader,
            Some((LoaderKind::Forge, "1.12.2-14.23.5.2859".to_owned()))
        );
        assert_eq!(
            inspection.target_evidence,
            TargetEvidence::Detected {
                source: source.to_owned()
            }
        );
        assert!(inspection.target_confirmed());

        // The same entry point without a Forge build behind it: custom or
        // OptiFine semantics, which need an explicit choice.
        for (index, profile) in [
            r#"{"id":"1.12.2","mainClass":"net.minecraft.launchwrapper.Launch"}"#,
            r#"{"id":"1.12.2","inheritsFrom":"1.12.2","libraries":[{"name":"optifine:OptiFine:1.12.2_HD_U_G5"}],"mainClass":"net.minecraft.launchwrapper.Launch"}"#,
        ]
        .into_iter()
        .enumerate()
        {
            let inspection = inspect(&[
                ("versions/1.12.2/1.12.2.json", profile.as_bytes()),
                ("mods/example.jar", b"jar".as_slice()),
            ])?;
            assert_eq!(
                inspection.target_evidence,
                TargetEvidence::NeedsSelection,
                "case {index}"
            );
            assert_eq!(
                inspection.minecraft_version.as_deref(),
                Some("1.12.2"),
                "case {index}"
            );
            assert_eq!(inspection.loader, None, "case {index}");
            assert!(!inspection.target_confirmed(), "case {index}");
        }
        Ok(())
    }

    #[test]
    fn rejects_unknown_custom_entry_points_and_unanchored_profiles() -> anyhow::Result<()> {
        let custom = r#"{"id":"1.20.1","mainClass":"com.example.CustomMain"}"#;
        assert!(
            inspect(&[
                ("versions/1.20.1/1.20.1.json", custom.as_bytes()),
                ("mods/example.jar", b"jar".as_slice()),
            ])
            .is_err()
        );
        let custom_loader = r#"{"id":"fabric-loader-0.15.11-1.20.1","inheritsFrom":"1.20.1","mainClass":"com.example.CustomMain"}"#;
        assert!(
            inspect(&[
                (
                    "versions/fabric-loader-0.15.11-1.20.1/fabric-loader-0.15.11-1.20.1.json",
                    custom_loader.as_bytes(),
                ),
                ("mods/example.jar", b"jar".as_slice()),
            ])
            .is_err()
        );

        let inherited = r#"{"id":"custom-profile","inheritsFrom":"1.20.1"}"#;
        let inspection = inspect(&[
            (
                "versions/custom-profile/custom-profile.json",
                inherited.as_bytes(),
            ),
            ("config/example.toml", b"cfg".as_slice()),
        ])?;
        assert_eq!(inspection.target_evidence, TargetEvidence::NeedsSelection);
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(inspection.loader, None);
        Ok(())
    }

    #[test]
    fn never_picks_between_multiple_profiles() -> anyhow::Result<()> {
        let fabric = fabric("1.20.1", "0.15.11");
        let forge = forge("1.20.1", "47.2.0");
        let inspection = inspect(&[
            (
                "versions/fabric-loader-0.15.11-1.20.1/fabric-loader-0.15.11-1.20.1.json",
                fabric.as_bytes(),
            ),
            (
                "versions/1.20.1-forge-47.2.0/1.20.1-forge-47.2.0.json",
                forge.as_bytes(),
            ),
            ("mods/example.jar", b"jar".as_slice()),
        ])?;
        assert_eq!(inspection.target_evidence, TargetEvidence::NeedsSelection);
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(inspection.loader, None);
        assert!(!inspection.target_confirmed());
        Ok(())
    }

    #[test]
    fn an_unresolved_profile_prevents_automatic_target_selection() -> anyhow::Result<()> {
        let version = vanilla("1.20.1");
        let inspection = inspect(&[
            ("versions/1.20.1/1.20.1.json", version.as_bytes()),
            ("versions/other/other.json", b"{}"),
            ("config/settings.toml", b"settings"),
        ])?;
        assert_eq!(inspection.target_evidence, TargetEvidence::NeedsSelection);
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.20.1"));
        assert!(!inspection.target_confirmed());
        Ok(())
    }

    #[test]
    fn rejects_an_unusable_pack_name() -> anyhow::Result<()> {
        let files = [("mods/example.jar", b"jar".as_slice())];
        for name in ["", "   ", "Bad\u{7}Name"] {
            assert!(inspect_as(name, &files).is_err(), "{name:?}");
        }
        assert_eq!(
            inspect_as("Example Pack.zip", &files)?.name,
            "Example Pack.zip"
        );
        Ok(())
    }

    #[test]
    fn archive_owner_rejects_paths_that_could_fake_a_root() {
        for path in [
            "/abs/mods/example.jar",
            "../mods/example.jar",
            "mods/../example.jar",
            "C:/mods/example.jar",
            "mods\\example.jar",
        ] {
            assert!(archive::validate_pack_path(path).is_err(), "{path}");
        }
    }
}
