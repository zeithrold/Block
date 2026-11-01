//! Bounded parser for the standard CurseForge modpack `manifest.json`.
//!
//! Supported input has `manifestType` `minecraftModpack` and `manifestVersion` 1.
//! Such a manifest names content only by project and file ID, which cannot be
//! verified without the CurseForge service. Inspection therefore reports no
//! downloaded file and keeps every entry as a missing file for the project page.

use std::collections::BTreeSet;

use anyhow::{Context as _, bail, ensure};
use serde::Deserialize;

use super::{CurseFile, LoaderKind, PackContent, PackFormat, PackInspection};

/// Override directory of a manifest that omits `overrides`.
const DEFAULT_OVERRIDES: &str = "overrides";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    manifest_type: String,
    manifest_version: u64,
    name: String,
    #[serde(default)]
    version: Option<String>,
    minecraft: Minecraft,
    #[serde(default)]
    files: Vec<FileEntry>,
    #[serde(default)]
    overrides: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Minecraft {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<ModLoader>,
}

#[derive(Deserialize)]
struct ModLoader {
    id: String,
}

#[derive(Deserialize)]
struct FileEntry {
    #[serde(rename = "projectID")]
    project_id: u64,
    #[serde(rename = "fileID")]
    file_id: u64,
    /// Standard entries that omit the flag are required; do not downgrade them.
    #[serde(default = "required_by_default")]
    required: bool,
}

fn required_by_default() -> bool {
    true
}

/// Read a standard CurseForge manifest into a transient inspection.
///
/// # Errors
/// Rejects a nonstandard manifest identity, an unsupported or ambiguous loader,
/// an unusable identity field, and duplicate file entries.
pub(super) fn parse(bytes: &[u8]) -> anyhow::Result<PackInspection> {
    let manifest: Manifest =
        serde_json::from_slice(bytes).context("Invalid CurseForge modpack manifest")?;
    ensure!(
        manifest.manifest_type == "minecraftModpack",
        "CurseForge manifest type '{}' is not supported",
        manifest.manifest_type
    );
    ensure!(
        manifest.manifest_version == 1,
        "CurseForge manifest version {} is not supported; only version 1 is",
        manifest.manifest_version
    );
    let minecraft_version = token(&manifest.minecraft.version, "Minecraft version")?;
    let loader = loader(&minecraft_version, &manifest.minecraft.mod_loaders)?;
    let overrides = overrides(manifest.overrides)?;
    let missing_files = missing_files(&manifest.files)?;
    Ok(PackInspection {
        format: PackFormat::CurseForge,
        name: text(&manifest.name, "modpack name")?,
        version: manifest
            .version
            .as_deref()
            .map(|version| text(version, "modpack version"))
            .transpose()?,
        minecraft_version: Some(minecraft_version),
        loader,
        target_evidence: super::TargetEvidence::Manifest,
        has_mods: false,
        files: Vec::new(),
        missing_files,
        content: PackContent::CurseForge { overrides },
        content_entries: Vec::new(),
        archive_prefix: String::new(),
    })
}

/// Map the declared loader entry to the identity Block stores for a target.
///
/// Forge builds are stored as `<minecraft>-<loader>`, and NeoForge 1.20.1 still
/// ships from the Forge artifact, so it uses the same form. A pin that already
/// carries the pinned Minecraft version keeps its exact form.
fn loader(minecraft: &str, entries: &[ModLoader]) -> anyhow::Result<Option<(LoaderKind, String)>> {
    ensure!(
        entries.len() <= 1,
        "CurseForge modpack declares {} loader entries; import supports at most one",
        entries.len()
    );
    let Some(entry) = entries.first() else {
        return Ok(None);
    };
    let Some((prefix, version)) = entry.id.split_once('-') else {
        bail!("CurseForge loader identity '{}' is not supported", entry.id);
    };
    let kind = match prefix {
        "forge" => LoaderKind::Forge,
        "neoforge" => LoaderKind::NeoForge,
        "fabric" => LoaderKind::Fabric,
        "quilt" => LoaderKind::Quilt,
        _ => bail!("CurseForge loader '{prefix}' is not supported"),
    };
    let version = loader_version(&kind, minecraft, &token(version, "loader version")?);
    Ok(Some((kind, version)))
}

/// Block stores Forge builds as `<minecraft>-<loader>`; a pin that already
/// carries the pinned Minecraft version keeps its exact form.
fn loader_version(kind: &LoaderKind, minecraft: &str, version: &str) -> String {
    let already_prefixed = version
        .strip_prefix(minecraft)
        .is_some_and(|rest| rest.starts_with('-'));
    let game_prefixed = match kind {
        LoaderKind::Forge => true,
        LoaderKind::NeoForge => minecraft == "1.20.1",
        LoaderKind::Vanilla | LoaderKind::Fabric | LoaderKind::Quilt => false,
    };
    if game_prefixed && !already_prefixed {
        format!("{minecraft}-{version}")
    } else {
        version.to_owned()
    }
}

/// Keep every declared entry as a missing file, rejecting repeated identities.
fn missing_files(entries: &[FileEntry]) -> anyhow::Result<Vec<CurseFile>> {
    let mut seen = BTreeSet::new();
    let mut files = Vec::with_capacity(entries.len());
    for entry in entries {
        ensure!(
            entry.project_id > 0 && entry.file_id > 0,
            "CurseForge file entry has an invalid project or file ID"
        );
        ensure!(
            seen.insert((entry.project_id, entry.file_id)),
            "Duplicate CurseForge file entry for project {} file {}",
            entry.project_id,
            entry.file_id
        );
        files.push(CurseFile {
            project_id: entry.project_id,
            file_id: entry.file_id,
            required: entry.required,
        });
    }
    Ok(files)
}

/// Resolve the declared override directory, defaulting only when it is absent.
fn overrides(value: Option<String>) -> anyhow::Result<String> {
    let Some(value) = value else {
        return Ok(DEFAULT_OVERRIDES.to_owned());
    };
    ensure!(
        !value.trim().is_empty(),
        "CurseForge modpack overrides directory is empty"
    );
    super::validate_pack_path(&value).context("Invalid CurseForge overrides directory")?;
    Ok(value)
}

/// Non-empty display text without control characters.
fn text(value: &str, what: &str) -> anyhow::Result<String> {
    ensure!(
        !value.trim().is_empty() && !value.chars().any(char::is_control),
        "CurseForge manifest has no usable {what}"
    );
    Ok(value.to_owned())
}

/// A single version-like token.
fn token(value: &str, what: &str) -> anyhow::Result<String> {
    ensure!(
        !value.is_empty()
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
            }),
        "CurseForge manifest has an invalid {what} '{value}'"
    );
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loader(id: &str) -> String {
        format!(r#"{{"id":"{id}","primary":true}}"#)
    }

    fn manifest(minecraft: &str, loaders: &str, files: &str) -> String {
        format!(
            r#"{{"manifestType":"minecraftModpack","manifestVersion":1,"name":"Example Pack","version":"2.1.0","minecraft":{{"version":"{minecraft}","modLoaders":[{loaders}]}},"files":[{files}],"overrides":"src/overrides"}}"#
        )
    }

    fn minimal_manifest(manifest_type: &str, manifest_version: &str, name: &str) -> String {
        format!(
            r#"{{"manifestType":"{manifest_type}","manifestVersion":{manifest_version},"name":"{name}","minecraft":{{"version":"1.20.1","modLoaders":[]}},"files":[]}}"#
        )
    }

    #[test]
    fn parses_standard_manifest_into_unverified_missing_entries() -> anyhow::Result<()> {
        let bytes = manifest(
            "1.12.2",
            &loader("forge-14.23.5.2860"),
            r#"{"projectID":238222,"fileID":2917633,"required":true},{"projectID":306612,"fileID":3489031,"required":false},{"projectID":32274,"fileID":1}"#,
        );
        let inspection = parse(bytes.as_bytes())?;
        assert_eq!(inspection.format, PackFormat::CurseForge);
        assert_eq!(inspection.name, "Example Pack");
        assert_eq!(inspection.version.as_deref(), Some("2.1.0"));
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.12.2"));
        assert_eq!(
            inspection.loader,
            Some((LoaderKind::Forge, "1.12.2-14.23.5.2860".into()))
        );
        assert!(
            inspection.files.is_empty(),
            "project and file IDs must not become verified files"
        );
        assert_eq!(
            inspection.missing_files,
            vec![
                CurseFile {
                    project_id: 238222,
                    file_id: 2917633,
                    required: true,
                },
                CurseFile {
                    project_id: 306612,
                    file_id: 3489031,
                    required: false,
                },
                // An entry that omits the standard flag stays required.
                CurseFile {
                    project_id: 32274,
                    file_id: 1,
                    required: true,
                },
            ]
        );
        assert_eq!(
            inspection.content,
            PackContent::CurseForge {
                overrides: "src/overrides".into()
            }
        );
        Ok(())
    }

    #[test]
    fn standard_ids_stay_unverified_project_redirects() -> anyhow::Result<()> {
        let bytes = manifest(
            "1.20.1",
            "",
            r#"{"projectID":238222,"fileID":2917633,"required":true}"#,
        );
        let inspection = parse(bytes.as_bytes())?;
        assert_eq!(inspection.loader, None);
        assert!(
            inspection.files.is_empty(),
            "an ID-only manifest cannot declare a verified file"
        );
        assert_eq!(inspection.missing_files.len(), 1);
        let missing = &inspection.missing_files[0];
        assert!(missing.required);
        assert_eq!(
            missing.project_url(),
            "https://www.curseforge.com/projects/238222"
        );
        assert!(
            !missing.project_url().contains("files"),
            "the helper names the project, not the exact file page"
        );
        Ok(())
    }

    #[test]
    fn defaults_overrides_only_when_the_manifest_omits_it() -> anyhow::Result<()> {
        let inspection = parse(minimal_manifest("minecraftModpack", "1", "Bare Pack").as_bytes())?;
        assert_eq!(
            inspection.content,
            PackContent::CurseForge {
                overrides: "overrides".into()
            }
        );
        Ok(())
    }

    #[test]
    fn canonicalizes_minecraft_and_supported_loader_identities() -> anyhow::Result<()> {
        for (minecraft, identity, kind, version) in [
            (
                "1.12.2",
                "forge-14.23.5.2860",
                LoaderKind::Forge,
                "1.12.2-14.23.5.2860",
            ),
            (
                "1.20.1",
                "forge-47.1.106",
                LoaderKind::Forge,
                "1.20.1-47.1.106",
            ),
            (
                "1.20.1",
                "neoforge-47.1.106",
                LoaderKind::NeoForge,
                "1.20.1-47.1.106",
            ),
            (
                "1.21.4",
                "neoforge-21.1.219",
                LoaderKind::NeoForge,
                "21.1.219",
            ),
            ("1.21.4", "fabric-0.16.10", LoaderKind::Fabric, "0.16.10"),
            ("1.20.1", "quilt-0.27.1", LoaderKind::Quilt, "0.27.1"),
            // A pin that already carries the Minecraft version keeps its exact form.
            (
                "1.20.1",
                "forge-1.20.1-47.1.106",
                LoaderKind::Forge,
                "1.20.1-47.1.106",
            ),
            (
                "1.20.1",
                "neoforge-1.20.1-47.1.106",
                LoaderKind::NeoForge,
                "1.20.1-47.1.106",
            ),
        ] {
            let bytes = manifest(minecraft, &loader(identity), "");
            let inspection = parse(bytes.as_bytes())?;
            assert_eq!(inspection.minecraft_version.as_deref(), Some(minecraft));
            assert_eq!(
                inspection.loader,
                Some((kind, version.to_owned())),
                "{identity}"
            );
        }
        assert_eq!(parse(manifest("1.21.4", "", "").as_bytes())?.loader, None);
        Ok(())
    }

    #[test]
    fn rejects_unsupported_and_multiple_loaders() -> anyhow::Result<()> {
        for loaders in [
            loader("liteloader-1.12.2"),
            loader("forge"),
            loader("forge-"),
            loader("Forge-47.1.106"),
            format!(
                "{},{}",
                loader("forge-47.1.106"),
                loader("neoforge-47.1.106")
            ),
            format!("{},{}", loader("forge-47.1.106"), loader("forge-47.1.106")),
        ] {
            let bytes = manifest("1.20.1", &loaders, "");
            assert!(parse(bytes.as_bytes()).is_err(), "{loaders}");
        }
        Ok(())
    }

    #[test]
    fn rejects_duplicate_file_entries_but_keeps_distinct_identities() -> anyhow::Result<()> {
        let duplicated = manifest(
            "1.20.1",
            "",
            r#"{"projectID":238222,"fileID":2917633,"required":true},{"projectID":238222,"fileID":2917633,"required":false}"#,
        );
        assert!(parse(duplicated.as_bytes()).is_err());

        let distinct = manifest(
            "1.20.1",
            "",
            r#"{"projectID":238222,"fileID":2917633,"required":true},{"projectID":238222,"fileID":2917640,"required":false}"#,
        );
        let inspection = parse(distinct.as_bytes())?;
        assert_eq!(inspection.missing_files.len(), 2);
        Ok(())
    }

    #[test]
    fn rejects_nonstandard_manifest_identities() -> anyhow::Result<()> {
        for bytes in [
            minimal_manifest("minecraftWorld", "1", "Pack"),
            minimal_manifest("minecraftModpack", "2", "Pack"),
            minimal_manifest("minecraftModpack", "1", ""),
            minimal_manifest("minecraftModpack", "1", "   "),
            manifest("1.20 1", "", ""),
            manifest("1.20.1", "", r#"{"projectID":0,"fileID":1}"#),
            r#"{"manifestType":"minecraftModpack""#.to_owned(),
        ] {
            assert!(parse(bytes.as_bytes()).is_err(), "{bytes}");
        }
        let empty_overrides = manifest("1.20.1", "", "").replace("src/overrides", "");
        assert!(parse(empty_overrides.as_bytes()).is_err());
        Ok(())
    }
}
