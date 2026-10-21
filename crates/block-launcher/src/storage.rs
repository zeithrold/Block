//! Native library/cache locations and recoverable, user-confirmed relocation.

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

const CACHE_TREES: &[&str] = &[
    "objects",
    "objects-sha256",
    "metadata",
    "views",
    "runtimes",
    "loaders",
    "installations",
    "installer-staging",
];
const JOURNAL: &str = "storage-migration.json";
const BACKUP: &str = "storage-backup";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Locations {
    pub library: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub generation: Option<uuid::Uuid>,
}

#[derive(Clone, Debug)]
pub struct StorageUse {
    pub locations: Locations,
    pub library: PathBuf,
    pub cache: PathBuf,
    pub(crate) lease: Arc<File>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Copying,
    Published,
    Switched,
    RolledBack,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tree {
    source: PathBuf,
    destination: PathBuf,
    skip_runtime: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    id: uuid::Uuid,
    before: Locations,
    after: Locations,
    trees: Vec<Tree>,
    phase: Phase,
    source_digest: String,
    cleanup_started: bool,
}

#[derive(Clone, Debug)]
pub struct Cleanup {
    pub id: uuid::Uuid,
    pub previous_folders: Vec<PathBuf>,
    pub interrupted: bool,
    pub cleanup_started: bool,
}
#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: &'static str,
    pub completed_bytes: u64,
    pub total_bytes: u64,
}

impl Locations {
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        match fs::read(root.join("storage.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)
                .context("Storage configuration is invalid; it was preserved")?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
    pub fn library(&self, root: &Path) -> PathBuf {
        self.library
            .clone()
            .unwrap_or_else(|| root.join("instances"))
    }
    pub fn cache(&self, root: &Path) -> PathBuf {
        self.cache.clone().unwrap_or_else(|| root.to_owned())
    }
    pub fn validate(&self, root: &Path) -> anyhow::Result<Self> {
        let root = root.canonicalize()?;
        let library = resolve(&self.library(&root))?;
        let cache = resolve(&self.cache(&root))?;
        ensure!(
            cache == root || !root.starts_with(&cache),
            "Cache folder must not contain launcher configuration"
        );
        ensure!(
            library != root,
            "The default library must be a separate folder from launcher configuration"
        );
        for name in CACHE_TREES
            .iter()
            .copied()
            .chain(["sessions", "storage-backup"])
        {
            let reserved = root.join(name);
            ensure!(
                !library.starts_with(&reserved) && !reserved.starts_with(&library),
                "Library folder overlaps launcher or cache data"
            );
            if cache != root {
                ensure!(
                    !cache.starts_with(&reserved),
                    "Cache folder overlaps reserved launcher data"
                );
            }
        }
        if cache != root {
            ensure!(
                !library.starts_with(&cache) && !cache.starts_with(&library),
                "Library and cache folders must not overlap"
            );
        }
        Ok(Self {
            library: (library != root.join("instances")).then_some(library),
            cache: (cache != root).then_some(cache),
            generation: self.generation,
        })
    }
}

/// Hold through downloads, installation or the complete game lifetime.
pub fn acquire(root: &Path) -> anyhow::Result<StorageUse> {
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let lease = lock(&root, false)?;
    if let Some(journal) = journal(&root)? {
        ensure!(
            matches!(journal.phase, Phase::Switched | Phase::RolledBack),
            "Storage relocation was interrupted. Recover it before installing or launching"
        );
    }
    let locations = Locations::load(&root)?.validate(&root)?;
    Ok(StorageUse {
        library: locations.library(&root),
        cache: locations.cache(&root),
        locations,
        lease: Arc::new(lease),
    })
}

fn lock(root: &Path, exclusive: bool) -> anyhow::Result<File> {
    let path = root.join("storage.lock");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "Invalid storage lock file"
        );
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let result = if exclusive {
        fs2::FileExt::try_lock_exclusive(&file)
    } else {
        fs2::FileExt::try_lock_shared(&file)
    };
    result.context("Storage is busy. Finish downloads and stop games before relocating files")?;
    Ok(file)
}
fn resolve(path: &Path) -> anyhow::Result<PathBuf> {
    ensure!(
        !path.as_os_str().as_encoded_bytes().contains(&0),
        "Folder paths must not contain NUL bytes"
    );
    ensure!(
        path.is_absolute() && path.parent().is_some(),
        "Choose an absolute folder path, not a filesystem root"
    );
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)),
        "Folder paths must not contain parent traversal"
    );
    let mut parent = path;
    let mut tail = Vec::new();
    while !parent.exists() {
        tail.push(
            parent
                .file_name()
                .context("Invalid storage folder")?
                .to_owned(),
        );
        parent = parent.parent().context("Invalid storage parent")?;
    }
    ensure!(parent.is_dir(), "Storage parent is not a folder");
    let mut result = parent.canonicalize()?;
    for part in tail.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}
fn atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing storage parent")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
fn save(root: &Path, value: &impl Serialize, name: &str) -> anyhow::Result<()> {
    atomic(&root.join(name), &serde_json::to_vec_pretty(value)?)
}
fn journal(root: &Path) -> anyhow::Result<Option<Journal>> {
    match fs::read(root.join(JOURNAL)) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes)
                .context("Migration state is invalid; all copies were preserved")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[derive(Clone, Debug)]
struct Entry {
    relative: PathBuf,
    kind: Kind,
    size: u64,
}
#[derive(Clone, Debug)]
enum Kind {
    Directory,
    File,
    Link(PathBuf),
}
fn excluded(path: &Path, skip: bool) -> bool {
    if !skip {
        return false;
    }
    matches!(
        path.components()
            .next()
            .and_then(|p| p.as_os_str().to_str()),
        Some("runs" | "instance.lock" | "prepared.lock.json")
    )
}
fn scan(root: &Path, skip: bool) -> anyhow::Result<Vec<Entry>> {
    let mut result = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let path = root.join(&relative);
        let meta = fs::symlink_metadata(&path)?;
        if excluded(&relative, skip) {
            continue;
        }
        let kind = if meta.file_type().is_symlink() {
            Kind::Link(fs::read_link(&path)?)
        } else if meta.is_dir() {
            let mut children = fs::read_dir(&path)?
                .map(|e| e.map(|e| relative.join(e.file_name())))
                .collect::<Result<Vec<_>, _>>()?;
            children.sort();
            pending.extend(children.into_iter().rev());
            Kind::Directory
        } else {
            ensure!(meta.is_file(), "A storage tree contains a non-file entry");
            Kind::File
        };
        result.push(Entry {
            relative,
            kind,
            size: meta.len(),
        });
    }
    Ok(result)
}
fn digest_file(path: &Path, cancel: &CancellationToken) -> anyhow::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        ensure!(!cancel.is_cancelled(), "Storage relocation cancelled");
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(hash.finalize().into())
}
fn tree_digest(trees: &[Tree], cancel: &CancellationToken) -> anyhow::Result<String> {
    let mut hash = Sha256::new();
    for (index, tree) in trees.iter().enumerate() {
        hash.update(index.to_be_bytes());
        for entry in scan(&tree.source, tree.skip_runtime)? {
            ensure!(!cancel.is_cancelled(), "Storage relocation cancelled");
            let name = entry
                .relative
                .to_str()
                .context("Storage has a non-UTF8 file name")?;
            hash.update((name.len() as u64).to_be_bytes());
            hash.update(name.as_bytes());
            match entry.kind {
                Kind::File => {
                    hash.update(b"file");
                    hash.update(digest_file(&tree.source.join(&entry.relative), cancel)?);
                }
                Kind::Directory => hash.update(b"directory"),
                Kind::Link(target) => {
                    hash.update(b"link");
                    hash.update(target.as_os_str().as_encoded_bytes());
                }
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn rebase(path: &Path, trees: &[Tree]) -> PathBuf {
    let canonical = if path.is_absolute() {
        path.canonicalize().ok()
    } else {
        None
    };
    for candidate in std::iter::once(path).chain(canonical.as_deref()) {
        if let Some(path) = trees.iter().find_map(|tree| {
            candidate
                .strip_prefix(&tree.source)
                .ok()
                .map(|tail| tree.destination.join(tail))
        }) {
            return path;
        }
    }
    path.to_owned()
}
fn rewrite_arguments(value: &mut serde_json::Value, trees: &[Tree]) {
    match value {
        serde_json::Value::String(text) => {
            for tree in trees {
                let source = tree.source.to_string_lossy();
                let target = tree.destination.to_string_lossy();
                let mut output = String::new();
                let mut copied = 0;
                for (index, _) in text.match_indices(source.as_ref()) {
                    let end = index + source.len();
                    let before = text[..index].chars().next_back();
                    let after = text[end..].chars().next();
                    if before.is_none_or(|ch| matches!(ch, '=' | ':' | ';' | '\'' | '"' | ' '))
                        && after.is_none_or(|ch| {
                            matches!(ch, '/' | '\\' | ':' | ';' | '\'' | '"' | ' ')
                        })
                    {
                        output.push_str(&text[copied..index]);
                        output.push_str(&target);
                        copied = end;
                    }
                }
                if copied != 0 {
                    output.push_str(&text[copied..]);
                    *text = output;
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                rewrite_arguments(value, trees);
            }
        }
        _ => {}
    }
}
fn link_target(source: &Path, target: &Path, trees: &[Tree]) -> anyhow::Result<PathBuf> {
    let path = if target.is_absolute() {
        target.to_owned()
    } else {
        source.parent().context("Invalid link parent")?.join(target)
    };
    // Canonicalize the existing parent; keep a missing final target as a link.
    let mut normalized = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                ensure!(normalized.pop(), "Invalid link target");
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(rebase(&normalized, trees))
}
fn create_link(target: &Path, path: &Path, source: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let _ = source;
        std::os::unix::fs::symlink(target, path)?;
    }
    #[cfg(windows)]
    {
        if source.is_dir() {
            std::os::windows::fs::symlink_dir(target, path)?;
        } else {
            std::os::windows::fs::symlink_file(target, path)?;
        }
    }
    Ok(())
}
fn copy_tree(
    tree: &Tree,
    trees: &[Tree],
    cancel: &CancellationToken,
    total: u64,
    done: &mut u64,
    progress: &dyn Fn(Progress),
) -> anyhow::Result<()> {
    let entries = scan(&tree.source, tree.skip_runtime)?;
    for entry in &entries {
        ensure!(!cancel.is_cancelled(), "Storage relocation cancelled");
        let source = tree.source.join(&entry.relative);
        let destination = tree.destination.join(&entry.relative);
        if let Ok(metadata) = fs::symlink_metadata(&tree.destination) {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "The relocation destination was replaced"
            );
        }
        crate::executor::ensure_write_path(&tree.destination, &destination)?;
        match &entry.kind {
            Kind::Directory => fs::create_dir_all(&destination)?,
            Kind::File => {
                fs::create_dir_all(destination.parent().context("Invalid copy destination")?)?;
                let mut input = File::open(&source)?;
                let mut temp = tempfile::NamedTempFile::new_in(
                    destination.parent().context("Invalid copy parent")?,
                )?;
                let mut bytes = [0; 65536];
                loop {
                    ensure!(!cancel.is_cancelled(), "Storage relocation cancelled");
                    let n = input.read(&mut bytes)?;
                    if n == 0 {
                        break;
                    }
                    temp.write_all(&bytes[..n])?;
                    *done += n as u64;
                    progress(Progress {
                        phase: "Copying",
                        completed_bytes: *done,
                        total_bytes: total,
                    });
                }
                temp.as_file()
                    .set_permissions(input.metadata()?.permissions())?;
                temp.as_file().sync_all()?;
                temp.persist(destination)?;
            }
            Kind::Link(target) => {
                fs::create_dir_all(destination.parent().context("Invalid link parent")?)?;
                create_link(&link_target(&source, target, trees)?, &destination, &source)?;
            }
        }
    }
    for entry in entries
        .iter()
        .rev()
        .filter(|entry| matches!(entry.kind, Kind::Directory))
    {
        fs::set_permissions(
            tree.destination.join(&entry.relative),
            fs::metadata(tree.source.join(&entry.relative))?.permissions(),
        )?;
    }
    Ok(())
}
fn verify(
    trees: &[Tree],
    cancel: &CancellationToken,
    total: u64,
    progress: &dyn Fn(Progress),
) -> anyhow::Result<()> {
    let mut done = 0;
    for tree in trees {
        let source = scan(&tree.source, tree.skip_runtime)?;
        let destination = scan(&tree.destination, tree.skip_runtime)?;
        ensure!(
            source.len() == destination.len(),
            "The copied storage tree has different entries"
        );
        for (a, b) in source.iter().zip(&destination) {
            ensure!(!cancel.is_cancelled(), "Storage relocation cancelled");
            ensure!(
                a.relative == b.relative,
                "The copied storage tree has different paths"
            );
            match (&a.kind, &b.kind) {
                (Kind::Directory, Kind::Directory) => {}
                (Kind::File, Kind::File) => {
                    ensure!(
                        a.size == b.size
                            && digest_file(&tree.source.join(&a.relative), cancel)?
                                == digest_file(&tree.destination.join(&b.relative), cancel)?,
                        "Copied file verification failed"
                    );
                    done += a.size;
                    progress(Progress {
                        phase: "Verifying",
                        completed_bytes: done,
                        total_bytes: total,
                    });
                }
                (Kind::Link(target), Kind::Link(copied)) => ensure!(
                    *copied == link_target(&tree.source.join(&a.relative), target, trees)?,
                    "Copied link verification failed"
                ),
                _ => anyhow::bail!("Copied entry type differs"),
            }
        }
    }
    Ok(())
}

fn rewrite_value(value: &mut serde_json::Value, trees: &[Tree]) {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.as_str(),
                    "manifest" | "manifest_path" | "directory" | "game_directory" | "java" | "path"
                ) && let Some(path) = value.as_str()
                {
                    *value = serde_json::Value::String(
                        rebase(Path::new(path), trees)
                            .to_string_lossy()
                            .into_owned(),
                    );
                } else if matches!(key.as_str(), "jvm_args" | "game_args" | "jvm_arguments") {
                    rewrite_arguments(value, trees);
                } else {
                    rewrite_value(value, trees);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                rewrite_value(value, trees);
            }
        }
        _ => {}
    }
}
fn document_paths(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut documents = vec![root.join("catalog.json"), root.join("preferences.json")];
    let sessions = root.join("sessions");
    if sessions.exists() {
        documents.extend(
            scan(&sessions, false)?
                .into_iter()
                .filter(|entry| {
                    matches!(entry.kind, Kind::File)
                        && entry.relative.extension().is_some_and(|ext| ext == "json")
                })
                .map(|entry| sessions.join(entry.relative)),
        );
    }
    Ok(documents
        .into_iter()
        .filter(|path| path.is_file())
        .collect())
}
fn rewrite_documents(root: &Path, state: &Journal) -> anyhow::Result<()> {
    let documents = document_paths(root)?;
    let backup = root.join(BACKUP).join(state.id.to_string());
    for path in documents {
        let bytes = fs::read(&path)?;
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let original = value.clone();
        rewrite_value(&mut value, &state.trees);
        if value != original {
            let saved = backup.join(path.strip_prefix(root)?);
            fs::create_dir_all(saved.parent().context("Missing backup parent")?)?;
            atomic(&saved, &bytes)?;
            atomic(&path, &serde_json::to_vec_pretty(&value)?)?;
        }
    }
    // Installed target descriptors move with their verified manifests.
    for tree in state.trees.iter().filter(|tree| {
        tree.source
            .file_name()
            .is_some_and(|name| name == "loaders")
    }) {
        for entry in scan(&tree.destination, false)?.into_iter().filter(|entry| {
            matches!(entry.kind, Kind::File)
                && entry
                    .relative
                    .file_name()
                    .is_some_and(|name| name == "installed.json")
        }) {
            let path = tree.destination.join(entry.relative);
            let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            rewrite_value(&mut value, &state.trees);
            atomic(&path, &serde_json::to_vec_pretty(&value)?)?;
        }
    }
    Ok(())
}

/// Atomically change defaults, copying selected managed data before switching.
/// Existing custom game folders are retained. No old copy is deleted here.
pub fn apply(
    root: &Path,
    requested: Locations,
    move_games: bool,
    cancel: &CancellationToken,
    progress: &dyn Fn(Progress),
) -> anyhow::Result<Option<Cleanup>> {
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let _lease = lock(&root, true)?;
    ensure!(
        journal(&root)?.is_none(),
        "Resolve the previous migration's retained copy before starting another move"
    );
    let before = Locations::load(&root)?.validate(&root)?;
    let mut after = requested.validate(&root)?;
    let id = uuid::Uuid::new_v4();
    after.generation = Some(id);
    let snapshot = crate::catalog::Catalog::open(&root)?.snapshot()?;
    ensure!(
        !crate::session_host::list_runs(&root)?
            .iter()
            .any(crate::session_host::RunRecord::is_active),
        "Stop or recover game sessions before relocating files"
    );
    let old_cache = before.cache(&root);
    let new_cache = after.cache(&root);
    let new_library = after.library(&root);
    let mut trees = Vec::new();
    if old_cache != new_cache {
        ensure!(
            !move_games,
            "Apply cache changes before moving existing games"
        );
        ensure!(
            old_cache == root
                || new_cache == root
                || (!new_cache.starts_with(&old_cache) && !old_cache.starts_with(&new_cache)),
            "Old and new cache folders must not overlap"
        );
        ensure!(
            new_cache == root || !new_cache.exists() || fs::read_dir(&new_cache)?.next().is_none(),
            "Choose an empty download cache folder"
        );
        for instance in &snapshot.instances {
            ensure!(
                new_cache == root
                    || (!instance.directory.starts_with(&new_cache)
                        && !new_cache.starts_with(&instance.directory)),
                "Cache folder overlaps game data"
            );
        }
        for name in CACHE_TREES {
            let source = old_cache.join(name);
            if source.exists() {
                trees.push(Tree {
                    source,
                    destination: new_cache.join(name),
                    skip_runtime: false,
                });
            }
        }
    }
    if move_games {
        for instance in &snapshot.instances {
            if instance
                .directory
                .file_name()
                .is_some_and(|name| name == instance.id.as_str())
                && instance.directory != new_library.join(&instance.id)
            {
                trees.push(Tree {
                    source: instance.directory.clone(),
                    destination: new_library.join(&instance.id),
                    skip_runtime: true,
                });
            }
        }
    }
    for tree in &trees {
        ensure!(
            !tree.destination.starts_with(&tree.source)
                && !tree.source.starts_with(&tree.destination),
            "Old and new data folders must not overlap"
        );
        let runtime_only = tree.destination.starts_with(old_cache.join("instances"))
            && tree.destination.is_dir()
            && scan(&tree.destination, true)?.len() == 1;
        ensure!(
            !tree.destination.exists() || (move_games && runtime_only),
            "A destination game or cache tree already exists"
        );
    }
    if trees.is_empty() {
        save(&root, &after, "storage.json")?;
        return Ok(None);
    }
    ensure!(!cancel.is_cancelled(), "Storage relocation cancelled");
    let mut requirements = BTreeMap::<PathBuf, u64>::new();
    let mut total = 0u64;
    for tree in &trees {
        let parent = tree
            .destination
            .parent()
            .context("Invalid destination parent")?;
        let mut existing = parent;
        while !existing.exists() {
            existing = existing
                .parent()
                .context("Missing destination filesystem")?;
        }
        let bytes =
            scan(&tree.source, tree.skip_runtime)?
                .iter()
                .try_fold(0u64, |sum, entry| {
                    sum.checked_add(if matches!(entry.kind, Kind::File) {
                        entry.size.saturating_add(4096)
                    } else {
                        4096
                    })
                    .context("Storage size is too large")
                })?;
        *requirements.entry(existing.canonicalize()?).or_default() += bytes;
        total += scan(&tree.source, tree.skip_runtime)?
            .iter()
            .filter(|e| matches!(e.kind, Kind::File))
            .map(|e| e.size)
            .sum::<u64>();
    }
    let metadata_bytes = document_paths(&root)?.iter().try_fold(0u64, |sum, path| {
        sum.checked_add(fs::metadata(path)?.len().saturating_add(4096))
            .context("Storage metadata is too large")
    })?;
    // Metadata is backed up and rewritten on the control filesystem. Reserve
    // it on destination checks too when both locations share the same disk.
    let metadata_bytes = metadata_bytes.saturating_mul(3);
    check_capacity(metadata_bytes, fs2::available_space(&root)?)?;
    for (path, bytes) in requirements {
        check_capacity(
            bytes.saturating_add(metadata_bytes),
            fs2::available_space(path)?,
        )?;
    }
    let source_digest = tree_digest(&trees, cancel)?;
    let mut state = Journal {
        id,
        before,
        after,
        trees,
        phase: Phase::Copying,
        source_digest,
        cleanup_started: false,
    };
    save(&root, &state, JOURNAL)?;
    let result = (|| -> anyhow::Result<()> {
        let mut done = 0;
        for tree in &state.trees {
            copy_tree(tree, &state.trees, cancel, total, &mut done, progress)?;
        }
        verify(&state.trees, cancel, total, progress)?;
        ensure!(
            tree_digest(&state.trees, cancel)? == state.source_digest,
            "Source data changed during relocation"
        );
        state.phase = Phase::Published;
        save(&root, &state, JOURNAL)?;
        rewrite_documents(&root, &state)?;
        ensure!(!cancel.is_cancelled(), "Storage relocation cancelled");
        save(&root, &state.after, "storage.json")?;
        state.phase = Phase::Switched;
        save(&root, &state, JOURNAL)?;
        Ok(())
    })();
    if let Err(error) = result {
        rollback(&root, &mut state)?;
        return Err(error);
    }
    cleanup_status(&root)
}
fn rollback(root: &Path, state: &mut Journal) -> anyhow::Result<()> {
    let backup = root.join(BACKUP).join(state.id.to_string());
    if backup.exists() {
        for entry in scan(&backup, false)?.into_iter().filter(|entry| {
            matches!(entry.kind, Kind::File)
                && entry.relative.extension().is_some_and(|ext| ext == "json")
        }) {
            atomic(
                &root.join(&entry.relative),
                &fs::read(backup.join(&entry.relative))?,
            )?;
        }
    }
    save(root, &state.before, "storage.json")?;
    state.phase = Phase::RolledBack;
    save(root, state, JOURNAL)
}
/// Recover the atomic switch before creating any download/launch workers.
pub fn recover(root: &Path) -> anyhow::Result<Option<Cleanup>> {
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    if let Some(mut state) = journal(&root)?
        && !matches!(state.phase, Phase::Switched | Phase::RolledBack)
    {
        let _lease = lock(&root, true)?;
        let current = Locations::load(&root)?;
        if current == state.after {
            state.phase = Phase::Switched;
            save(&root, &state, JOURNAL)?;
        } else {
            ensure!(
                current == state.before,
                "Storage configuration changed outside this migration; all copies were preserved"
            );
            rollback(&root, &mut state)?;
        }
    }
    cleanup_status(&root)
}
pub fn cleanup_status(root: &Path) -> anyhow::Result<Option<Cleanup>> {
    Ok(journal(root)?.map(|state| Cleanup {
        id: state.id,
        previous_folders: state
            .trees
            .iter()
            .map(|tree| {
                if state.phase == Phase::Switched {
                    tree.source.clone()
                } else {
                    tree.destination.clone()
                }
            })
            .collect(),
        interrupted: state.phase != Phase::Switched,
        cleanup_started: state.cleanup_started,
    }))
}
/// Explicitly retain the previous copy, or remove only the confirmed managed trees.
pub fn finish_cleanup(root: &Path, id: uuid::Uuid, remove: bool) -> anyhow::Result<()> {
    let root = root.canonicalize()?;
    let _lease = lock(&root, true)?;
    let mut state = journal(&root)?.context("There is no retained migration copy")?;
    ensure!(
        state.id == id,
        "The retained copy changed. Review it again before cleanup"
    );
    ensure!(
        matches!(state.phase, Phase::Switched | Phase::RolledBack),
        "Recover the interrupted migration first"
    );
    if remove {
        if state.phase == Phase::Switched && !state.cleanup_started {
            ensure!(
                tree_digest(&state.trees, &CancellationToken::new())? == state.source_digest,
                "The old copy changed after migration. No files were removed; keep it and review its contents"
            );
        }
        state.cleanup_started = true;
        save(&root, &state, JOURNAL)?;
        for tree in &state.trees {
            let path = if state.phase == Phase::Switched {
                &tree.source
            } else {
                &tree.destination
            };
            if !path.exists() {
                continue;
            }
            let metadata = fs::symlink_metadata(path)?;
            ensure!(
                metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && path.canonicalize()? == *path,
                "A retained folder was replaced; no files were removed from it"
            );
            if tree.skip_runtime
                || (state.phase == Phase::RolledBack
                    && path.starts_with(state.before.cache(&root).join("instances")))
            {
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    if !excluded(Path::new(&entry.file_name()), true) {
                        remove_owned(&entry.path())?;
                    }
                }
            } else {
                remove_owned(path)?;
            }
        }
    }
    let backup = root.join(BACKUP).join(state.id.to_string());
    if backup.exists() {
        remove_owned(&backup)?;
    }
    fs::remove_file(root.join(JOURNAL))?;
    Ok(())
}
fn remove_owned(path: &Path) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        #[cfg(windows)]
        if std::os::windows::fs::FileTypeExt::is_symlink_dir(&meta.file_type()) {
            fs::remove_dir(path)?;
            return Ok(());
        }
        fs::remove_file(path)?;
    } else if meta.is_file() {
        #[cfg(windows)]
        if meta.permissions().readonly() {
            let mut permissions = meta.permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
        fs::remove_file(path)?;
    } else {
        for entry in fs::read_dir(path)? {
            remove_owned(&entry?.path())?;
        }
        fs::remove_dir(path)?;
    }
    Ok(())
}

fn check_capacity(bytes: u64, available: u64) -> anyhow::Result<()> {
    ensure!(
        available >= bytes.saturating_add(16 * 1024 * 1024),
        "Not enough free space to copy and verify storage while keeping the original"
    );
    Ok(())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
