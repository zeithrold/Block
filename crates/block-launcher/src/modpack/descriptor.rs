//! Persisted, source-free description of an accepted local modpack.
//!
//! A descriptor records exactly what inspection declared and what the user
//! selected, so the same instance can be prepared later without its source
//! archive. Download URLs stay transient: they never enter this model, its
//! serialized form, its `Debug` output or its errors.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{ErrorKind, Read as _, Write as _},
    path::Path,
};

use anyhow::{Context as _, bail, ensure};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    ContentEntry, CurseFile, PackContent, PackFile, PackFormat, PackInspection, PackSelection,
};
use crate::catalog::LoaderKind;

/// Metadata directory this module owns beside an instance's game content.
const METADATA_DIRECTORY: &str = ".block-pack";
/// One attempt is staged in a sibling directory of the fixed metadata directory.
const PREPARING_PREFIX: &str = ".block-pack-preparing-";
/// Serializes publication of the fixed metadata directory.
const LOCK_FILE: &str = ".block-pack.lock";
/// Local overlay content extracted from the same accepted archive.
const OVERRIDES_DIRECTORY: &str = "overrides";
const DESCRIPTOR_FILE: &str = "pack.json";
const SCHEMA_VERSION: u32 = 1;
/// A descriptor for the largest accepted pack stays far below this bound.
const MAX_DESCRIPTOR_BYTES: u64 = 32 * 1024 * 1024;
const SHA1_HEX: usize = 40;
const SHA512_HEX: usize = 128;

/// The accepted pack, without sources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackDescriptor {
    pub(crate) schema_version: u32,
    pub(crate) format: PackFormat,
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    pub(crate) minecraft_version: String,
    pub(crate) loader: Option<(LoaderKind, String)>,
    pub(crate) files: Vec<DeclaredFile>,
    pub(crate) missing_files: Vec<DeclaredMissing>,
    /// Content-root metadata stays private to this module so a reader cannot
    /// treat a parsed root as separately validated state.
    content: PackContent,
    content_entries: Vec<ContentEntry>,
}

/// One file the pack declares, with the hashes and size that verify it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeclaredFile {
    pub(crate) path: String,
    pub(crate) sha1: String,
    pub(crate) sha512: String,
    pub(crate) size: u64,
    pub(crate) required: bool,
    pub(crate) selected: bool,
}

/// One CurseForge entry that names its file only by project and file ID.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeclaredMissing {
    pub(crate) project_id: u64,
    pub(crate) file_id: u64,
    pub(crate) required: bool,
    pub(crate) selected: bool,
}

/// Declare the accepted pack exactly as inspected and selected.
///
/// Every declared file and every ID-only entry is recorded. Required entries are
/// always selected; an optional entry is selected only when the matching
/// [`PackSelection`] set names it.
///
/// # Errors
/// Rejects a selection key that is not a declared optional entry, which includes
/// a stale key and any attempt to select or drop a required entry.
pub(crate) fn from_inspection(
    inspection: &PackInspection,
    selection: &PackSelection,
) -> anyhow::Result<PackDescriptor> {
    ensure!(
        inspection.target_confirmed(),
        "select the pack's Minecraft and loader target"
    );
    declared_optional_files(inspection, &selection.optional_files)?;
    declared_optional_curse_files(inspection, &selection.optional_curse_files)?;
    let descriptor = PackDescriptor {
        schema_version: SCHEMA_VERSION,
        format: inspection.format,
        name: inspection.name.clone(),
        version: inspection.version.clone(),
        minecraft_version: inspection
            .minecraft_version
            .clone()
            .context("pack Minecraft version is missing")?,
        loader: inspection.loader.clone(),
        files: inspection
            .files
            .iter()
            .map(|file| DeclaredFile {
                path: file.path.clone(),
                sha1: file.sha1.clone(),
                sha512: file.sha512.clone(),
                size: file.size,
                required: file.required,
                selected: file.required || selection.optional_files.contains(&file.path),
            })
            .collect(),
        missing_files: inspection
            .missing_files
            .iter()
            .map(|missing| DeclaredMissing {
                project_id: missing.project_id,
                file_id: missing.file_id,
                required: missing.required,
                selected: missing.required
                    || selection
                        .optional_curse_files
                        .contains(&(missing.project_id, missing.file_id)),
            })
            .collect(),
        content: inspection.content.clone(),
        content_entries: inspection.content_entries.clone(),
    };
    descriptor.validate()?;
    Ok(descriptor)
}

impl PackDescriptor {
    /// Whether this descriptor still describes the inspected declaration.
    ///
    /// Compares format, identity, loader, content-root metadata and every
    /// declared path, hash, size and required flag as normalized
    /// (order-independent) data. Transient download URLs are ignored on purpose:
    /// they may change between selections while describing the same pack.
    /// Selection intent is user state, not declaration data, so it is not
    /// compared here either.
    #[must_use]
    pub(crate) fn matches_inspection(&self, inspection: &PackInspection) -> bool {
        self.format == inspection.format
            && (self.format == PackFormat::Plain || self.name == inspection.name)
            && self.version == inspection.version
            && (self.format == PackFormat::Plain
                || (Some(self.minecraft_version.as_str())
                    == inspection.minecraft_version.as_deref()
                    && self.loader == inspection.loader))
            && self.content == inspection.content
            && self.content_entries == inspection.content_entries
            && normalized_file_keys(self.files.iter().map(stored_file_key))
                == normalized_file_keys(inspection.files.iter().map(inspected_file_key))
            && normalized_missing_keys(self.missing_files.iter().map(stored_missing_key))
                == normalized_missing_keys(
                    inspection.missing_files.iter().map(inspected_missing_key),
                )
    }

    /// Whether completion still depends on an entry known only by CurseForge ID.
    ///
    /// Such an entry has no verifiable file metadata, so a required one and an
    /// explicitly selected optional one both block. Required entries cannot be
    /// deselected, and empty required with no selected optional entry completes.
    #[must_use]
    pub(crate) fn has_missing_selected(&self) -> bool {
        self.missing_files.iter().any(|missing| missing.selected)
    }

    /// Reject stored data that no inspection could have produced.
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == SCHEMA_VERSION,
            "pack metadata schema version {} is not supported; only version {} is",
            self.schema_version,
            SCHEMA_VERSION
        );
        text(&self.name, "pack name")?;
        text(&self.minecraft_version, "Minecraft version")?;
        if let Some(version) = &self.version {
            text(version, "pack version")?;
        }
        if let Some((_, version)) = &self.loader {
            text(version, "loader version")?;
        }
        self.validate_content()?;
        let mut content = BTreeSet::new();
        for entry in &self.content_entries {
            super::archive::validate_pack_path(&entry.path)?;
            ensure!(
                content.insert(entry.path.as_str()),
                "pack metadata declares the same content entry twice"
            );
        }
        let mut paths = BTreeSet::new();
        for file in &self.files {
            destination_path(&file.path)?;
            ensure!(
                paths.insert(file.path.as_str()),
                "pack metadata declares the same file twice"
            );
            digest(&file.sha1, SHA1_HEX, "sha1 hash")?;
            digest(&file.sha512, SHA512_HEX, "sha512 hash")?;
            ensure!(
                !file.required || file.selected,
                "pack metadata leaves a required file unselected"
            );
        }
        let mut identities = BTreeSet::new();
        for missing in &self.missing_files {
            ensure!(
                missing.project_id > 0 && missing.file_id > 0,
                "pack metadata has an invalid missing file identity"
            );
            ensure!(
                identities.insert((missing.project_id, missing.file_id)),
                "pack metadata declares the same missing file twice"
            );
            ensure!(
                !missing.required || missing.selected,
                "pack metadata leaves a required missing file unselected"
            );
        }
        Ok(())
    }

    /// The content root is inspected declaration data, so it must agree with the
    /// declared format.
    fn validate_content(&self) -> anyhow::Result<()> {
        match (&self.format, &self.content) {
            (PackFormat::Modrinth, PackContent::Modrinth) => Ok(()),
            (PackFormat::CurseForge, PackContent::CurseForge { overrides }) => {
                super::archive::validate_pack_path(overrides)
            }
            (PackFormat::Plain, PackContent::Plain { root }) if root.is_empty() => Ok(()),
            (PackFormat::Prism, PackContent::Prism { root })
            | (PackFormat::Plain, PackContent::Plain { root }) => {
                super::archive::validate_pack_path(root)
            }
            _ => bail!("pack metadata format does not match its content root"),
        }
    }
}

/// Snapshot an accepted archive into a task-owned instance directory.
///
/// Each attempt stages into a fresh `.block-pack-preparing-<uuid>` sibling and
/// publishes it under the fixed `.block-pack` name only after the overlays and a
/// complete `pack.json` are written, so an interrupted attempt is retryable and
/// stays behind for diagnosis. An already published descriptor is returned as-is
/// when it matches this descriptor and selection exactly, and is never replaced
/// otherwise.
///
/// # Errors
/// Rejects a selection this inspection cannot satisfy, a changed source,
/// unsafe archive content, cancellation, I/O failures, and an instance that
/// already carries different pack metadata.
pub(crate) fn stage(
    source: &Path,
    inspected: &PackInspection,
    selection: &PackSelection,
    instance_dir: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<PackDescriptor> {
    ensure!(!cancel.is_cancelled(), "pack import cancelled");
    let descriptor = from_inspection(inspected, selection)?;
    if let Some(published) = published(instance_dir)? {
        return only_if_same(published, &descriptor);
    }
    let attempt = instance_dir.join(format!("{PREPARING_PREFIX}{}", Uuid::new_v4()));
    fs::create_dir(&attempt)?;
    super::extract_overrides(
        source,
        inspected,
        &attempt.join(OVERRIDES_DIRECTORY),
        cancel,
    )?;
    persist(&attempt.join(DESCRIPTOR_FILE), &descriptor)?;

    let lock = lock_metadata(instance_dir)?;
    if let Some(published) = published(instance_dir)? {
        drop(lock);
        let _ = fs::remove_dir_all(&attempt);
        return only_if_same(published, &descriptor);
    }
    fs::rename(&attempt, instance_dir.join(METADATA_DIRECTORY))?;
    drop(lock);
    Ok(descriptor)
}

/// Read the descriptor this module persisted for an instance.
///
/// The path is fixed by this module, so a caller cannot point the loader at
/// another sidecar. Nothing is read from the source archive.
///
/// # Errors
/// Rejects missing, linked or nonregular metadata, oversized metadata, unknown or
/// malformed data, and a schema version other than 1.
pub(crate) fn load(instance_dir: &Path) -> anyhow::Result<PackDescriptor> {
    let directory = instance_dir.join(METADATA_DIRECTORY);
    let directory_metadata =
        fs::symlink_metadata(&directory).context("pack metadata directory is missing")?;
    ensure!(
        directory_metadata.is_dir() && !directory_metadata.file_type().is_symlink(),
        "pack metadata directory is not a plain directory"
    );
    let path = directory.join(DESCRIPTOR_FILE);
    let metadata = fs::symlink_metadata(&path).context("pack metadata is missing")?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "pack metadata is a link"
    );
    ensure!(metadata.is_file(), "pack metadata is not a regular file");
    ensure!(
        metadata.len() <= MAX_DESCRIPTOR_BYTES,
        "pack metadata is too large"
    );
    let mut bytes = Vec::new();
    File::open(&path)?
        .take(MAX_DESCRIPTOR_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_DESCRIPTOR_BYTES,
        "pack metadata is too large"
    );
    let descriptor: PackDescriptor =
        serde_json::from_slice(&bytes).context("pack metadata is invalid")?;
    descriptor.validate().context("pack metadata is invalid")?;
    Ok(descriptor)
}

/// Declaration data of one file, normalized for comparison without its source.
type FileKey<'a> = (&'a str, &'a str, &'a str, u64, bool);
type MissingKey = (u64, u64, bool);

fn stored_file_key(file: &DeclaredFile) -> FileKey<'_> {
    (
        file.path.as_str(),
        file.sha1.as_str(),
        file.sha512.as_str(),
        file.size,
        file.required,
    )
}

fn inspected_file_key(file: &PackFile) -> FileKey<'_> {
    (
        file.path.as_str(),
        file.sha1.as_str(),
        file.sha512.as_str(),
        file.size,
        file.required,
    )
}

fn stored_missing_key(missing: &DeclaredMissing) -> MissingKey {
    (missing.project_id, missing.file_id, missing.required)
}

fn inspected_missing_key(missing: &CurseFile) -> MissingKey {
    (missing.project_id, missing.file_id, missing.required)
}

fn normalized_file_keys<'a>(keys: impl Iterator<Item = FileKey<'a>>) -> Vec<FileKey<'a>> {
    let mut keys: Vec<_> = keys.collect();
    keys.sort_unstable();
    keys
}

fn normalized_missing_keys(keys: impl Iterator<Item = MissingKey>) -> Vec<MissingKey> {
    let mut keys: Vec<_> = keys.collect();
    keys.sort_unstable();
    keys
}

/// A selected optional file key must name one declared optional file.
fn declared_optional_files(
    inspection: &PackInspection,
    keys: &BTreeSet<String>,
) -> anyhow::Result<()> {
    for key in keys {
        let declared = inspection
            .files
            .iter()
            .find(|file| &file.path == key)
            .context("pack selection names a file this pack does not declare")?;
        ensure!(
            !declared.required,
            "pack selection cannot select a required file"
        );
    }
    Ok(())
}

/// A selected optional CurseForge key must name one declared optional entry.
fn declared_optional_curse_files(
    inspection: &PackInspection,
    keys: &BTreeSet<(u64, u64)>,
) -> anyhow::Result<()> {
    for key in keys {
        let declared = inspection
            .missing_files
            .iter()
            .find(|missing| (missing.project_id, missing.file_id) == *key)
            .context("pack selection names a file this pack does not declare")?;
        ensure!(
            !declared.required,
            "pack selection cannot select a required file"
        );
    }
    Ok(())
}

/// A declared file is written under the instance's game content, never into the
/// launcher's own metadata.
fn destination_path(path: &str) -> anyhow::Result<()> {
    super::archive::validate_pack_path(path)?;
    ensure!(
        !path
            .split('/')
            .next()
            .unwrap_or_default()
            .starts_with(".block-"),
        "pack metadata writes into a reserved launcher path"
    );
    Ok(())
}

fn digest(value: &str, digits: usize, what: &str) -> anyhow::Result<()> {
    ensure!(
        value.len() == digits
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "pack metadata has an invalid {what}"
    );
    Ok(())
}

/// Non-empty display text without control characters.
fn text(value: &str, what: &str) -> anyhow::Result<()> {
    ensure!(
        !value.trim().is_empty() && !value.chars().any(char::is_control),
        "pack metadata has no usable {what}"
    );
    Ok(())
}

/// The published descriptor of an instance that already carries metadata.
///
/// Nothing is read from the source archive, and the metadata directory must be a
/// real directory: a published descriptor is read, never replaced through a link.
fn published(instance_dir: &Path) -> anyhow::Result<Option<PackDescriptor>> {
    let metadata = instance_dir.join(METADATA_DIRECTORY);
    let existing = match fs::symlink_metadata(&metadata) {
        Ok(existing) => existing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        !existing.file_type().is_symlink(),
        "pack metadata is a link"
    );
    ensure!(existing.is_dir(), "pack metadata is not a directory");
    Ok(Some(load(instance_dir)?))
}

/// A published descriptor is immutable: only the same declaration and selection
/// may be accepted again, so a retry never rewrites accepted metadata.
fn only_if_same(
    published: PackDescriptor,
    expected: &PackDescriptor,
) -> anyhow::Result<PackDescriptor> {
    ensure!(
        published == *expected,
        "the instance already carries different pack metadata"
    );
    Ok(published)
}

fn lock_metadata(instance_dir: &Path) -> anyhow::Result<File> {
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(instance_dir.join(LOCK_FILE))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("this instance is already publishing pack metadata; retry the import")?;
    Ok(lock)
}

fn persist(path: &Path, descriptor: &PackDescriptor) -> anyhow::Result<()> {
    let parent = path.parent().context("pack metadata has no parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(descriptor)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use sha1::Digest as _;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    type JsonCase = (&'static str, fn(&mut Value));

    fn digests(seed: &str) -> (String, String) {
        (
            format!("{:x}", sha1::Sha1::digest(seed.as_bytes())),
            format!("{:x}", sha2::Sha512::digest(seed.as_bytes())),
        )
    }

    fn file(path: &str, required: bool, urls: &[&str]) -> PackFile {
        let (sha1, sha512) = digests(path);
        PackFile {
            path: path.to_owned(),
            sha1,
            sha512,
            size: 1024,
            urls: urls.iter().map(|url| (*url).to_owned()).collect(),
            required,
        }
    }

    fn missing(project_id: u64, file_id: u64, required: bool) -> CurseFile {
        CurseFile {
            project_id,
            file_id,
            required,
        }
    }

    fn modrinth(files: Vec<PackFile>) -> PackInspection {
        PackInspection {
            format: PackFormat::Modrinth,
            name: "Example Pack".to_owned(),
            version: Some("1.0.0".to_owned()),
            minecraft_version: Some("1.20.1".to_owned()),
            loader: Some((LoaderKind::Fabric, "0.16.10".to_owned())),
            target_evidence: super::super::TargetEvidence::Manifest,
            has_mods: false,
            files,
            missing_files: Vec::new(),
            content: PackContent::Modrinth,
            content_entries: Vec::new(),
            archive_prefix: String::new(),
        }
    }

    fn curseforge(missing_files: Vec<CurseFile>) -> PackInspection {
        PackInspection {
            format: PackFormat::CurseForge,
            name: "Example Pack".to_owned(),
            version: Some("2.1.0".to_owned()),
            minecraft_version: Some("1.12.2".to_owned()),
            loader: Some((LoaderKind::Forge, "1.12.2-14.23.5.2860".to_owned())),
            target_evidence: super::super::TargetEvidence::Manifest,
            has_mods: false,
            files: Vec::new(),
            missing_files,
            content: PackContent::CurseForge {
                overrides: "overrides".to_owned(),
            },
            content_entries: Vec::new(),
            archive_prefix: String::new(),
        }
    }

    fn optional_files(keys: &[&str]) -> PackSelection {
        PackSelection {
            optional_files: keys.iter().map(|key| (*key).to_owned()).collect(),
            optional_curse_files: BTreeSet::new(),
        }
    }

    fn optional_curse_files(keys: &[(u64, u64)]) -> PackSelection {
        PackSelection {
            optional_files: BTreeSet::new(),
            optional_curse_files: keys.iter().copied().collect(),
        }
    }

    #[test]
    fn persisted_descriptor_carries_no_source_url() -> anyhow::Result<()> {
        let urls = [
            "https://cdn.example/mods/example.jar?token=private&signature=abc123",
            "http://127.0.0.1:8123/fallback/example.jar",
        ];
        let inspection = modrinth(vec![file("mods/example.jar", true, &urls)]);
        let descriptor = from_inspection(&inspection, &PackSelection::default())?;
        let document = serde_json::to_string(&descriptor)?;
        for forbidden in [
            "http",
            "cdn.example",
            "127.0.0.1",
            "token",
            "signature",
            "urls",
            "url",
        ] {
            assert!(!document.contains(forbidden), "{forbidden} in {document}");
        }
        let debug = format!("{descriptor:?}");
        assert!(!debug.contains("cdn.example"), "{debug}");
        assert!(!debug.contains("127.0.0.1"), "{debug}");
        Ok(())
    }

    #[test]
    fn round_trip_matches_reselection_and_rejects_declaration_changes() -> anyhow::Result<()> {
        let urls = ["https://cdn.example/mods/a.jar?token=one"];
        let inspection = modrinth(vec![
            file("mods/a.jar", true, &urls),
            file("mods/z.jar", false, &urls),
        ]);
        let descriptor = from_inspection(&inspection, &PackSelection::default())?;

        let loaded: PackDescriptor = serde_json::from_slice(&serde_json::to_vec(&descriptor)?)?;
        assert_eq!(loaded, descriptor);
        assert!(loaded.matches_inspection(&inspection));

        // A re-selection from other mirrors describes the same pack.
        let reselected = modrinth(vec![
            file("mods/a.jar", true, &["https://mirror.example/x.jar"]),
            file("mods/z.jar", false, &[]),
        ]);
        assert!(loaded.matches_inspection(&reselected));
        assert_eq!(
            from_inspection(&reselected, &PackSelection::default())?,
            descriptor
        );

        // Declaration order is not part of the declaration.
        let mut reordered = inspection.clone();
        reordered.files.reverse();
        assert!(descriptor.matches_inspection(&reordered));

        let mut changed = inspection.clone();
        changed.files[0].path = "mods/b.jar".to_owned();
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.files[0].sha1 = digests("other").0;
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.files[0].sha512 = digests("other").1;
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.files[0].size = 2048;
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.files[0].required = false;
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.name = "Other Pack".to_owned();
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.version = None;
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.minecraft_version = Some("1.21.1".to_owned());
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.loader = None;
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.format = PackFormat::Prism;
        assert!(!descriptor.matches_inspection(&changed));
        let mut changed = inspection.clone();
        changed.content = PackContent::CurseForge {
            overrides: "overrides".to_owned(),
        };
        assert!(!descriptor.matches_inspection(&changed));

        // A source that drops a declared file is a different pack too.
        let fewer = modrinth(vec![file("mods/a.jar", true, &urls)]);
        assert!(!descriptor.matches_inspection(&fewer));
        Ok(())
    }

    #[test]
    fn stale_or_contradictory_selections_are_rejected() -> anyhow::Result<()> {
        let inspection = modrinth(vec![
            file("mods/required.jar", true, &[]),
            file("mods/optional.jar", false, &[]),
        ]);
        for key in ["mods/required.jar", "mods/undeclared.jar"] {
            assert!(
                from_inspection(&inspection, &optional_files(&[key])).is_err(),
                "{key}"
            );
        }
        let chosen = from_inspection(&inspection, &optional_files(&["mods/optional.jar"]))?;
        assert!(chosen.files.iter().all(|file| file.selected));
        let defaults = from_inspection(&inspection, &PackSelection::default())?;
        assert_eq!(
            defaults
                .files
                .iter()
                .map(|file| (file.path.as_str(), file.selected))
                .collect::<Vec<_>>(),
            vec![("mods/required.jar", true), ("mods/optional.jar", false)]
        );

        let curse = curseforge(vec![
            missing(238_222, 2_917_633, true),
            missing(306_612, 3_489_031, false),
        ]);
        for key in [(238_222, 2_917_633), (1, 2)] {
            assert!(
                from_inspection(&curse, &optional_curse_files(&[key])).is_err(),
                "{key:?}"
            );
        }
        let chosen = from_inspection(&curse, &optional_curse_files(&[(306_612, 3_489_031)]))?;
        assert!(chosen.missing_files.iter().all(|missing| missing.selected));
        Ok(())
    }

    #[test]
    fn selected_id_only_entries_block_completion() -> anyhow::Result<()> {
        let required = from_inspection(
            &curseforge(vec![missing(7, 8, true)]),
            &PackSelection::default(),
        )?;
        assert!(required.has_missing_selected());

        let both = from_inspection(
            &curseforge(vec![missing(7, 8, true), missing(9, 10, false)]),
            &optional_curse_files(&[(9, 10)]),
        )?;
        assert!(both.has_missing_selected());

        let unselected = from_inspection(
            &curseforge(vec![missing(9, 10, false)]),
            &PackSelection::default(),
        )?;
        assert!(!unselected.has_missing_selected());

        let empty = from_inspection(&curseforge(Vec::new()), &PackSelection::default())?;
        assert!(!empty.has_missing_selected());

        // A declared file carries verifiable metadata, so it never blocks.
        let verified = from_inspection(
            &modrinth(vec![file("mods/required.jar", true, &[])]),
            &PackSelection::default(),
        )?;
        assert!(!verified.has_missing_selected());
        Ok(())
    }

    fn base_document() -> anyhow::Result<Value> {
        let inspection = modrinth(vec![file(
            "mods/a.jar",
            true,
            &["https://cdn.example/a.jar"],
        )]);
        Ok(serde_json::to_value(from_inspection(
            &inspection,
            &PackSelection::default(),
        )?)?)
    }

    fn curse_document() -> anyhow::Result<Value> {
        Ok(serde_json::to_value(from_inspection(
            &curseforge(vec![missing(7, 8, true), missing(9, 10, false)]),
            &PackSelection::default(),
        )?)?)
    }

    fn write_metadata(instance: &Path, document: &Value) -> anyhow::Result<()> {
        let metadata = instance.join(METADATA_DIRECTORY);
        fs::create_dir_all(&metadata)?;
        fs::write(
            metadata.join(DESCRIPTOR_FILE),
            serde_json::to_vec(document)?,
        )?;
        Ok(())
    }

    #[test]
    fn load_rejects_malformed_and_mismatched_metadata() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let instance = temp.path().join("instance");

        let base = base_document()?;
        write_metadata(&instance, &base)?;
        assert_eq!(serde_json::to_value(load(&instance)?)?, base);

        let cases: [JsonCase; 6] = [
            ("schema version", |document| {
                document["schema_version"] = json!(2)
            }),
            ("unknown field", |document| document["urls"] = json!([])),
            ("missing field", |document| {
                if let Some(object) = document.as_object_mut() {
                    let _ = object.remove("missing_files");
                }
            }),
            ("content root", |document| {
                document["content"] = json!({"format": "prism", "root": "minecraft"});
            }),
            ("format", |document| document["format"] = json!("prism")),
            ("empty name", |document| document["name"] = json!("  ")),
        ];
        for (case, mutate) in cases {
            let mut document = base.clone();
            mutate(&mut document);
            write_metadata(&instance, &document)?;
            assert!(load(&instance).is_err(), "{case}");
        }

        // Truncated documents never half-load.
        fs::write(
            instance.join(METADATA_DIRECTORY).join(DESCRIPTOR_FILE),
            br#"{"schema_version": 1, "format": "modrinth""#,
        )?;
        assert!(load(&instance).is_err());
        Ok(())
    }

    #[test]
    fn load_rejects_unsafe_paths_digests_and_flags() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let instance = temp.path().join("instance");
        let base = base_document()?;

        let cases: [JsonCase; 7] = [
            ("parent escape", |document| {
                document["files"][0]["path"] = json!("../escape.jar");
            }),
            ("absolute path", |document| {
                document["files"][0]["path"] = json!("/escape.jar");
            }),
            ("reserved metadata path", |document| {
                document["files"][0]["path"] = json!(".block-pack/pack.json");
            }),
            ("separator", |document| {
                document["files"][0]["path"] = json!("mods\\a.jar");
            }),
            ("duplicate path", |document| {
                let first = document["files"][0].clone();
                document["files"] = json!([first.clone(), first]);
            }),
            ("sha1", |document| {
                document["files"][0]["sha1"] = json!("abc")
            }),
            ("required flag", |document| {
                document["files"][0]["selected"] = json!(false);
            }),
        ];
        for (case, mutate) in cases {
            let mut document = base.clone();
            mutate(&mut document);
            write_metadata(&instance, &document)?;
            assert!(load(&instance).is_err(), "{case}");
        }

        let curse = curse_document()?;
        let cases: [JsonCase; 3] = [
            ("zero identity", |document| {
                document["missing_files"][0]["project_id"] = json!(0);
            }),
            ("duplicate identity", |document| {
                let first = document["missing_files"][0].clone();
                document["missing_files"] = json!([first.clone(), first]);
            }),
            ("required missing flag", |document| {
                document["missing_files"][0]["selected"] = json!(false);
            }),
        ];
        for (case, mutate) in cases {
            let mut document = curse.clone();
            mutate(&mut document);
            write_metadata(&instance, &document)?;
            assert!(load(&instance).is_err(), "{case}");
        }
        Ok(())
    }

    #[test]
    fn load_rejects_missing_and_nonregular_metadata() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let instance = temp.path().join("instance");
        assert!(load(&instance).is_err());
        fs::create_dir_all(instance.join(METADATA_DIRECTORY).join(DESCRIPTOR_FILE))?;
        assert!(load(&instance).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_linked_metadata() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir()?;
        let instance = temp.path().join("instance");
        let metadata = instance.join(METADATA_DIRECTORY);
        fs::create_dir_all(&metadata)?;
        let target = temp.path().join("elsewhere.json");
        fs::write(&target, serde_json::to_vec(&base_document()?)?)?;
        symlink(&target, metadata.join(DESCRIPTOR_FILE))?;
        assert!(load(&instance).is_err());
        Ok(())
    }

    fn synthetic_pack(path: &Path) -> anyhow::Result<()> {
        let (first_sha1, first_sha512) = digests("example");
        let (second_sha1, second_sha512) = digests("optional");
        let index = json!({
            "formatVersion": 1,
            "game": "minecraft",
            "name": "Synthetic pack",
            "versionId": "1.0.0",
            "dependencies": {"minecraft": "1.20.1"},
            "files": [
                {
                    "path": "mods/example.jar",
                    "hashes": {"sha1": first_sha1, "sha512": first_sha512},
                    "fileSize": 8,
                    "downloads": ["https://cdn.example/mods/example.jar?token=private&signature=abc"]
                },
                {
                    "path": "mods/optional.jar",
                    "hashes": {"sha1": second_sha1, "sha512": second_sha512},
                    "fileSize": 4,
                    "downloads": ["https://cdn.example/mods/optional.jar"],
                    "env": {"client": "optional"}
                }
            ]
        });
        let mut zip = ZipWriter::new(File::create(path)?);
        zip.start_file("modrinth.index.json", SimpleFileOptions::default())?;
        zip.write_all(&serde_json::to_vec(&index)?)?;
        zip.start_file("overrides/config/example.txt", SimpleFileOptions::default())?;
        zip.write_all(b"value")?;
        zip.finish()?;
        Ok(())
    }

    /// Sibling attempt directories this instance currently keeps.
    fn attempts(instance: &Path) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(instance)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if name.starts_with(PREPARING_PREFIX) {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    #[test]
    fn staging_publishes_schema_one_metadata_without_urls() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("synthetic.mrpack");
        synthetic_pack(&source)?;
        let inspected = super::super::inspect(&source)?;
        let instance = temp.path().join("instance");
        fs::create_dir(&instance)?;
        let selection = optional_files(&["mods/optional.jar"]);
        let descriptor = stage(
            &source,
            &inspected,
            &selection,
            &instance,
            &CancellationToken::new(),
        )?;

        let metadata = instance.join(METADATA_DIRECTORY);
        assert_eq!(descriptor.schema_version, 1);
        assert_eq!(descriptor, from_inspection(&inspected, &selection)?);
        assert!(descriptor.files.iter().all(|file| file.selected));
        assert!(!descriptor.has_missing_selected());
        let persisted = fs::read(metadata.join(DESCRIPTOR_FILE))?;
        let document = String::from_utf8(persisted.clone())?;
        for forbidden in ["http", "cdn.example", "token", "signature"] {
            assert!(!document.contains(forbidden), "{forbidden} in {document}");
        }
        assert_eq!(load(&instance)?, descriptor);
        assert_eq!(
            fs::read(metadata.join("overrides/config/example.txt"))?,
            b"value"
        );
        // Publication moves the complete attempt into place.
        assert!(attempts(&instance)?.is_empty());

        // Repeating the accepted import returns the published descriptor.
        let again = stage(
            &source,
            &inspected,
            &selection,
            &instance,
            &CancellationToken::new(),
        )?;
        assert_eq!(again, descriptor);
        assert_eq!(fs::read(metadata.join(DESCRIPTOR_FILE))?, persisted);
        assert!(attempts(&instance)?.is_empty());

        // A different selection is different metadata and is never adopted.
        assert!(
            stage(
                &source,
                &inspected,
                &PackSelection::default(),
                &instance,
                &CancellationToken::new()
            )
            .is_err()
        );
        assert_eq!(fs::read(metadata.join(DESCRIPTOR_FILE))?, persisted);
        assert!(attempts(&instance)?.is_empty());
        Ok(())
    }

    #[test]
    fn failed_or_cancelled_staging_never_overwrites_metadata() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let inspection = modrinth(vec![file("mods/example.jar", true, &[])]);
        let selection = PackSelection::default();
        let instance = temp.path().join("instance");
        fs::create_dir(&instance)?;

        // An unreadable source leaves its own attempt behind, unpublished.
        let broken = temp.path().join("broken.mrpack");
        fs::write(&broken, b"not a zip")?;
        assert!(
            stage(
                &broken,
                &inspection,
                &selection,
                &instance,
                &CancellationToken::new()
            )
            .is_err()
        );
        assert!(!instance.join(METADATA_DIRECTORY).exists());
        let failed = attempts(&instance)?;
        assert_eq!(failed.len(), 1, "{failed:?}");
        let failed = instance.join(&failed[0]);
        assert!(!failed.join(DESCRIPTOR_FILE).exists());
        assert!(!failed.join(OVERRIDES_DIRECTORY).exists());

        // Cancellation stages nothing at all.
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            stage(&broken, &inspection, &selection, &instance, &cancelled).is_err(),
            "cancelled staging"
        );
        assert_eq!(attempts(&instance)?.len(), 1);

        // The same instance accepts a later attempt and keeps the failed sibling.
        let source = temp.path().join("synthetic.mrpack");
        synthetic_pack(&source)?;
        let inspected = super::super::inspect(&source)?;
        let descriptor = stage(
            &source,
            &inspected,
            &selection,
            &instance,
            &CancellationToken::new(),
        )?;
        assert_eq!(load(&instance)?, descriptor);
        assert_eq!(attempts(&instance)?.len(), 1);

        // A published descriptor that disagrees is neither adopted nor replaced.
        let existing = temp.path().join("existing");
        let metadata = existing.join(METADATA_DIRECTORY);
        fs::create_dir_all(&metadata)?;
        let other = from_inspection(&inspected, &optional_files(&["mods/optional.jar"]))?;
        let bytes = serde_json::to_vec_pretty(&other)?;
        fs::write(metadata.join(DESCRIPTOR_FILE), &bytes)?;
        assert!(
            stage(
                &source,
                &inspected,
                &selection,
                &existing,
                &CancellationToken::new()
            )
            .is_err()
        );
        assert_eq!(fs::read(metadata.join(DESCRIPTOR_FILE))?, bytes);
        assert!(!metadata.join(OVERRIDES_DIRECTORY).exists());
        assert!(attempts(&existing)?.is_empty());

        // Unreadable published metadata is reported, not replaced either.
        let unreadable = temp.path().join("unreadable");
        let metadata = unreadable.join(METADATA_DIRECTORY);
        fs::create_dir_all(&metadata)?;
        fs::write(metadata.join(DESCRIPTOR_FILE), b"{\"marker\":true}")?;
        assert!(
            stage(
                &source,
                &inspected,
                &selection,
                &unreadable,
                &CancellationToken::new()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(metadata.join(DESCRIPTOR_FILE))?,
            b"{\"marker\":true}"
        );
        assert!(attempts(&unreadable)?.is_empty());
        Ok(())
    }
}
