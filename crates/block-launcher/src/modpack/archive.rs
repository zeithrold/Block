use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::Path,
};

use anyhow::{Context as _, ensure};
use tokio_util::sync::CancellationToken;
use zip::ZipArchive;

use super::{
    ContentEntry, PackContent, PackFormat, PackInspection, TargetEvidence, curseforge, mrpack,
    plain, prism,
};

const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const CENTRAL_ENTRY_SIGNATURE: [u8; 4] = *b"PK\x01\x02";

// `ZipArchive` indexes entries by name and otherwise silently replaces a
// duplicate. Count the central-directory records before using that index so a
// duplicate can never disappear from validation.
fn open_archive(path: &Path) -> anyhow::Result<ZipArchive<File>> {
    let archive = ZipArchive::new(File::open(path)?)?;
    let expected = archive.len();
    let start = archive.central_directory_start();
    let mut file = archive.into_inner();
    file.seek(SeekFrom::Start(start))?;
    let mut count = 0_usize;
    loop {
        let mut signature = [0_u8; 4];
        file.read_exact(&mut signature)?;
        if signature != CENTRAL_ENTRY_SIGNATURE {
            break;
        }
        let mut header = [0_u8; 42];
        file.read_exact(&mut header)?;
        let name = u16::from_le_bytes([header[24], header[25]]);
        let extra = u16::from_le_bytes([header[26], header[27]]);
        let comment = u16::from_le_bytes([header[28], header[29]]);
        file.seek(SeekFrom::Current(
            i64::from(name) + i64::from(extra) + i64::from(comment),
        ))?;
        count += 1;
        ensure!(count <= MAX_ARCHIVE_ENTRIES, "pack has too many entries");
    }
    ensure!(count == expected, "pack has duplicate archive entries");
    file.rewind()?;
    Ok(ZipArchive::new(file)?)
}

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub(super) index: usize,
    pub(super) path: String,
    pub(super) directory: bool,
    pub(super) size: u64,
}

#[derive(Clone, Debug)]
struct PathKind {
    path: String,
    directory: bool,
    explicit: bool,
}

/// Portable paths cannot alias a parent, a platform root, or another spelling.
pub(super) fn validate_pack_path(path: &str) -> anyhow::Result<()> {
    ensure!(!path.is_empty(), "empty pack path");
    ensure!(
        !path.contains(['\\', ':', '\0']) && !path.starts_with('/'),
        "pack path is not a relative portable path"
    );
    for part in path.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.contains(['<', '>', '"', '|', '?', '*'])
                && !part.chars().any(char::is_control),
            "pack path contains an unsafe component"
        );
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "pack path uses a reserved filename"
        );
    }
    Ok(())
}

fn record_path(
    paths: &mut BTreeMap<String, PathKind>,
    path: &str,
    directory: bool,
    allow_overlay: bool,
) -> anyhow::Result<()> {
    validate_pack_path(path)?;
    let parts: Vec<_> = path.split('/').collect();
    let mut prefix = String::new();
    for (index, part) in parts.iter().enumerate() {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        let leaf = index + 1 == parts.len();
        let want_directory = !leaf || directory;
        let key = prefix.to_lowercase();
        if let Some(old) = paths.get_mut(&key) {
            ensure!(old.path == prefix, "pack paths conflict by case");
            ensure!(
                old.directory == want_directory,
                "pack has conflicting file and directory paths"
            );
            if leaf {
                ensure!(!old.explicit || allow_overlay, "pack has a duplicate path");
                old.explicit = true;
            }
        } else {
            paths.insert(
                key,
                PathKind {
                    path: prefix.clone(),
                    directory: want_directory,
                    explicit: leaf,
                },
            );
        }
    }
    Ok(())
}

fn entries(archive: &mut ZipArchive<File>) -> anyhow::Result<Vec<Entry>> {
    ensure!(
        archive.len() <= MAX_ARCHIVE_ENTRIES,
        "pack has too many entries"
    );
    let mut result = Vec::with_capacity(archive.len());
    let mut paths = BTreeMap::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = std::str::from_utf8(file.name_raw()).context("pack filename is not UTF-8")?;
        let directory = file.is_dir();
        let path = if directory {
            name.strip_suffix('/').context("invalid pack directory")?
        } else {
            name
        };
        record_path(&mut paths, path, directory, false)?;
        if let Some(mode) = file.unix_mode() {
            let kind = mode & 0o170_000;
            ensure!(
                matches!(kind, 0 | 0o100_000 | 0o040_000),
                "pack contains a link or special file"
            );
            ensure!(
                kind == 0 || (kind == 0o040_000) == directory,
                "pack entry type conflicts with its name"
            );
        }
        result.push(Entry {
            index,
            path: path.to_owned(),
            directory,
            size: file.size(),
        });
    }
    Ok(result)
}

pub(super) fn metadata(archive: &mut ZipArchive<File>, entry: &Entry) -> anyhow::Result<Vec<u8>> {
    ensure!(!entry.directory, "pack metadata is a directory");
    let mut file = archive.by_index(entry.index)?;
    ensure!(
        file.size() <= MAX_METADATA_BYTES,
        "pack metadata is too large"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn find<'a>(entries: &'a [Entry], path: &str) -> Option<&'a Entry> {
    entries.iter().find(|entry| entry.path == path)
}

fn prism_game_root(entries: &[Entry], prefix: &str) -> anyhow::Result<String> {
    let roots = [".minecraft", "minecraft"]
        .into_iter()
        .map(|name| format!("{prefix}{name}"))
        .filter(|root| {
            entries.iter().any(|entry| {
                (entry.path == *root && entry.directory)
                    || entry.path.starts_with(&format!("{root}/"))
            })
        })
        .collect::<Vec<_>>();
    ensure!(
        roots.len() == 1,
        "Prism pack needs exactly one game content directory"
    );
    for entry in entries {
        let relative = entry.path.strip_prefix(prefix).unwrap_or_default();
        let component = relative.split('/').next().unwrap_or_default();
        ensure!(
            !matches!(
                component.to_ascii_lowercase().as_str(),
                "patches" | "jarmods"
            ),
            "Prism custom patches and jar mods are not supported"
        );
        if let Some(relative) = selected_path(entry, &roots[0]) {
            let component = relative.split('/').next().unwrap_or_default();
            ensure!(
                !component.eq_ignore_ascii_case("jarmods"),
                "Prism custom patches and jar mods are not supported"
            );
        }
    }
    Ok(roots[0].clone())
}

fn parse(
    archive: &mut ZipArchive<File>,
    entries: &[Entry],
    source_name: &str,
    mrpack_extension: bool,
) -> anyhow::Result<PackInspection> {
    let mut candidates = Vec::new();
    for entry in entries {
        let (prefix, name) = entry
            .path
            .rsplit_once('/')
            .map_or(("", entry.path.as_str()), |(parent, name)| (parent, name));
        if prefix.contains('/') {
            continue;
        }
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };
        let normalized = name.to_ascii_lowercase();
        match normalized.as_str() {
            "mcbbs.packmeta" | "modpack.json" | "ftb.json" => {
                anyhow::bail!("this launcher-specific pack format is not supported");
            }
            "modrinth.index.json" | "manifest.json" | "mmc-pack.json" => {
                ensure!(
                    name == normalized,
                    "pack metadata filename has unexpected case"
                );
                let format = match name {
                    "modrinth.index.json" => PackFormat::Modrinth,
                    "manifest.json" => PackFormat::CurseForge,
                    _ => PackFormat::Prism,
                };
                candidates.push((format, entry, prefix));
            }
            "instance.cfg" => {
                ensure!(
                    name == normalized
                        && find(entries, &format!("{prefix}mmc-pack.json")).is_some(),
                    "Prism mmc-pack.json is missing"
                );
            }
            _ => {}
        }
    }
    ensure!(candidates.len() <= 1, "pack format is ambiguous");
    if mrpack_extension {
        ensure!(
            candidates
                .first()
                .is_some_and(|(format, _, _)| *format == PackFormat::Modrinth),
            "Modrinth pack metadata is missing"
        );
    }
    let mut inspection = if let Some((format, entry, prefix)) = candidates.first() {
        let bytes = metadata(archive, entry)?;
        let mut inspection = match format {
            PackFormat::Modrinth => mrpack::parse(&bytes)?,
            PackFormat::CurseForge => curseforge::parse(&bytes)?,
            PackFormat::Prism => {
                let cfg = find(entries, &format!("{prefix}instance.cfg"))
                    .context("Prism instance.cfg is missing")?;
                let game_root = prism_game_root(entries, prefix)?;
                prism::parse(&bytes, &metadata(archive, cfg)?, &game_root)?
            }
            PackFormat::Plain => anyhow::bail!("plain packs have no format marker"),
        };
        inspection.archive_prefix.clone_from(prefix);
        inspection
    } else {
        plain::parse(archive, entries, source_name)?
    };
    validate_content_paths(entries, &inspection)?;
    inspection.content_entries = content_entries(entries, &inspection);
    Ok(inspection)
}

fn archive_name(path: &Path) -> &str {
    path.file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Imported game")
}

fn is_mrpack(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mrpack"))
}

fn selected_path<'a>(entry: &'a Entry, root: &str) -> Option<&'a str> {
    if root.is_empty() {
        Some(&entry.path)
    } else {
        entry.path.strip_prefix(root)?.strip_prefix('/')
    }
}

fn layer_roots(inspection: &PackInspection) -> Vec<String> {
    match &inspection.content {
        PackContent::Modrinth => vec![
            format!("{}overrides", inspection.archive_prefix),
            format!("{}client-overrides", inspection.archive_prefix),
        ],
        PackContent::CurseForge { overrides } => {
            vec![format!("{}{overrides}", inspection.archive_prefix)]
        }
        PackContent::Prism { root } | PackContent::Plain { root } => vec![root.clone()],
    }
}

fn content_entries(entries: &[Entry], inspection: &PackInspection) -> Vec<ContentEntry> {
    let mut result = Vec::new();
    for root in layer_roots(inspection) {
        for entry in entries {
            let Some(path) = selected_path(entry, &root) else {
                continue;
            };
            let version_metadata = inspection.format == PackFormat::Plain
                && path.starts_with("versions/")
                && path.ends_with(".json");
            if version_metadata || !excluded_for(inspection, path, entry.directory) {
                result.push(ContentEntry {
                    path: entry.path.clone(),
                    size: entry.size,
                    directory: entry.directory,
                });
            }
        }
    }
    result.sort();
    result
}

fn excluded(path: &str, directory: bool) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let last = index + 1 == components.len();
        if crate::game_import::is_excluded_entry(component, !last || directory, index == 0) {
            return true;
        }
    }
    false
}

fn excluded_for(inspection: &PackInspection, path: &str, directory: bool) -> bool {
    let first = path
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(first.as_str(), "logs" | "crash-reports") || excluded(path, directory) {
        return true;
    }
    if inspection.format != PackFormat::Plain {
        return false;
    }
    if matches!(
        first.as_str(),
        "versions" | "libraries" | "assets" | "runtime" | "runtimes" | "jre" | "java"
    ) || first.starts_with("launcher_")
        || first.starts_with(".block-")
    {
        return true;
    }
    !directory
        && !path.contains('/')
        && [".exe", ".jar", ".bat", ".cmd", ".ps1", ".sh"]
            .iter()
            .any(|extension| first.ends_with(extension))
}

fn validate_content_paths(entries: &[Entry], inspection: &PackInspection) -> anyhow::Result<()> {
    let mut paths = BTreeMap::new();
    for file in &inspection.files {
        ensure!(
            !excluded(&file.path, false),
            "pack declares a file reserved for launcher or credential data"
        );
        record_path(&mut paths, &file.path, false, false)?;
    }
    for root in layer_roots(inspection) {
        for entry in entries {
            let Some(path) = selected_path(entry, &root) else {
                continue;
            };
            if excluded_for(inspection, path, entry.directory) {
                continue;
            }
            ensure!(
                !path
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .starts_with(".block-"),
                "pack content uses a reserved launcher path"
            );
            record_path(&mut paths, path, entry.directory, true)?;
        }
    }
    Ok(())
}

/// Inspect a local archive without writing any game or launcher data.
///
/// # Errors
/// Rejects unsupported, ambiguous, malformed or unsafe packs.
pub fn inspect(path: &Path) -> anyhow::Result<PackInspection> {
    let mut archive = open_archive(path)?;
    let entries = entries(&mut archive)?;
    parse(&mut archive, &entries, archive_name(path), is_mrpack(path))
}

/// Copy only the approved local content into a new empty private directory.
///
/// The source is opened and its manifest and entries revalidated against the
/// inspection before copying. Client overrides are applied after overrides.
///
/// # Errors
/// Rejects changed metadata, unsafe entries, existing destination content,
/// cancellation, and I/O failures.
pub fn extract_overrides(
    path: &Path,
    inspected: &PackInspection,
    destination: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    ensure!(!cancel.is_cancelled(), "pack import cancelled");
    let mut archive = open_archive(path)?;
    let entries = entries(&mut archive)?;
    let mut current = parse(&mut archive, &entries, archive_name(path), is_mrpack(path))?;
    if current.format == PackFormat::Plain
        && inspected.target_evidence == TargetEvidence::UserSelected
    {
        current
            .minecraft_version
            .clone_from(&inspected.minecraft_version);
        current.loader.clone_from(&inspected.loader);
        current.target_evidence = TargetEvidence::UserSelected;
    }
    ensure!(
        &current == inspected,
        "pack metadata changed; inspect it again"
    );
    fs::create_dir(destination)?;
    for root in layer_roots(&current) {
        for entry in &entries {
            let Some(path) = selected_path(entry, &root) else {
                continue;
            };
            if excluded_for(&current, path, entry.directory) {
                continue;
            }
            ensure!(!cancel.is_cancelled(), "pack import cancelled");
            let target = destination.join(path);
            if entry.directory {
                fs::create_dir_all(target)?;
            } else {
                fs::create_dir_all(target.parent().context("pack file has no parent")?)?;
                let mut input = archive.by_index(entry.index)?;
                let mut output = File::create(target)?;
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    ensure!(!cancel.is_cancelled(), "pack import cancelled");
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    output.write_all(&buffer[..count])?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
