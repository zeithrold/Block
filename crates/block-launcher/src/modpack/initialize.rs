//! One-time seeding of a managed Instance's game directory from its pack baseline.
//!
//! Pack content is prepared once inside the installation and is published into its
//! actual game directory the first time that Instance launches. The caller
//! already holds the real game directory's [`GameDirectoryGuard`] and keeps holding
//! it here: the lock inode, the game directory itself and every played file stay
//! untouched, and the source is always the installation's prepared baseline rather
//! than a path taken from pack metadata.
//!
//! A marker in the game directory records which stages exist and exactly which
//! top-level entries this operation owns, so an interrupted run is either resumed
//! or reported as blocked instead of being mistaken for an initialized directory.

use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};

use super::archive::validate_pack_path;
use crate::{
    CancellationToken,
    catalog::{GameDirectoryGuard, Instance, InstanceOwnership},
};

/// Ownership marker written into the game directory before pack content exists.
const MARKER_FILE: &str = ".block-pack-init.json";
/// Private staging directory; the marker names the exact one it owns.
const STAGE_PREFIX: &str = ".block-pack-stage-";
/// Names this operation owns and never publishes from pack content.
const LAUNCHER_ENTRIES: [&str; 5] = [
    MARKER_FILE,
    ".block-game.lock",
    ".block-session.json",
    ".block-pack",
    ".block-pack.lock",
];
/// A marker never needs to be larger than the entry list it declares.
const MAX_MARKER_BYTES: u64 = 64 * 1024;
const SCHEMA_VERSION: u32 = 1;

/// Publish the installation's prepared baseline into this Instance's game directory.
///
/// The caller holds `guard` continuously for the whole call. A completed marker makes
/// the call a no-op; otherwise pack content is staged beside the locked directory and
/// then renamed in entry by entry. Existing destination entries are never replaced.
///
/// # Errors
/// Rejects unexpected existing data, unsafe pack content, a malformed marker and any
/// conflicting publication. Preexisting data is preserved and the launch stays blocked.
pub(crate) fn initialize(
    instance: &Instance,
    guard: &GameDirectoryGuard,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    ensure!(
        !cancel.is_cancelled(),
        "pack baseline initialization cancelled"
    );
    let root = controlled_root(instance, guard)?;
    let game = guard.directory();
    let baseline = root.join(".block-pack").join("baseline");
    match read_marker(game)? {
        None => begin(game, &baseline, cancel),
        Some(Marker::Complete) => Ok(()),
        Some(Marker::Staging { stage }) => stage_and_publish(game, &baseline, &stage, cancel),
        Some(Marker::Publishing { stage, entries }) => {
            publish_phase(game, &stage_path(game, &stage)?, &entries, cancel)
        }
    }
}

/// Prove that a managed pack owns the baseline and the guard covers its exact gameDir.
fn controlled_root(instance: &Instance, guard: &GameDirectoryGuard) -> anyhow::Result<PathBuf> {
    ensure!(
        instance.ownership == InstanceOwnership::Managed,
        "registered source installations cannot initialize a pack baseline"
    );
    let pack = instance.pack.as_ref().context("instance has no pack")?;
    ensure!(
        instance.target_id == pack.target_id,
        "selected target does not match the pack"
    );
    let root = instance
        .resource_directory
        .canonicalize()
        .context("installation directory is unavailable; reselect its location before playing")?;
    ensure!(
        root == instance.resource_directory && guard.directory() == instance.game_directory,
        "the locked directory does not match the Instance; it was preserved: {}",
        guard.directory().display()
    );
    Ok(root)
}

/// Start a first run by recording ownership, then stage and publish the baseline.
fn begin(game: &Path, baseline: &Path, cancel: &CancellationToken) -> anyhow::Result<()> {
    ensure_publishable_destination(game)?;
    let stage = format!("{STAGE_PREFIX}{}", uuid::Uuid::new_v4());
    // The marker names its stage before the stage exists, so an interrupted run can
    // never leave an unattributed directory beside the game directory.
    write_marker(
        game,
        &Marker::Staging {
            stage: stage.clone(),
        },
    )?;
    stage_and_publish(game, baseline, &stage, cancel)
}

/// Rebuild the marker-owned stage from the baseline, then publish it.
fn stage_and_publish(
    game: &Path,
    baseline: &Path,
    stage_name: &str,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    ensure_publishable_destination(game)?;
    let stage = stage_path(game, stage_name)?;
    reset_stage(&stage)?;
    fs::create_dir(&stage).context("cannot create the pack staging directory")?;
    copy_baseline(baseline, &stage, cancel)?;
    ensure!(
        !cancel.is_cancelled(),
        "pack baseline initialization cancelled"
    );
    let entries = staged_entries(&stage)?;
    write_marker(
        game,
        &Marker::Publishing {
            stage: stage_name.to_owned(),
            entries: entries.clone(),
        },
    )?;
    publish_phase(game, &stage, &entries, cancel)
}

/// Move exactly the declared entries into the game directory, then mark completion.
fn publish_phase(
    game: &Path,
    stage: &Path,
    entries: &[String],
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    for name in entries {
        ensure!(
            !cancel.is_cancelled(),
            "pack baseline publication cancelled"
        );
        publish_entry(&stage.join(name), &game.join(name), name)?;
    }
    ensure!(
        !cancel.is_cancelled(),
        "pack baseline publication cancelled"
    );
    write_marker(game, &Marker::Complete)?;
    remove_empty_stage(stage);
    Ok(())
}

/// Publish one declared entry, treating an entry already under the marker as done.
fn publish_entry(staged: &Path, destination: &Path, name: &str) -> anyhow::Result<()> {
    match (entry_metadata(staged)?, entry_metadata(destination)?) {
        (Some(_), Some(_)) => anyhow::bail!(
            "a pack baseline entry already exists in the game directory; it was preserved: {name}"
        ),
        (Some(staged_metadata), None) => {
            ensure!(
                !staged_metadata.file_type().is_symlink(),
                "refusing a symlinked pack staging entry; it was preserved: {name}"
            );
            fs::rename(staged, destination)
                .with_context(|| format!("cannot publish pack baseline entry: {name}"))?;
            Ok(())
        }
        (None, Some(published)) => {
            ensure!(
                is_plain_entry(&published),
                "a published pack baseline entry was replaced by a link or special file; it was preserved: {name}"
            );
            Ok(())
        }
        (None, None) => anyhow::bail!(
            "a declared pack baseline entry is missing from both the stage and the game directory: {name}"
        ),
    }
}

/// Recursively copy the prepared baseline, refusing links and special files.
fn copy_baseline(baseline: &Path, stage: &Path, cancel: &CancellationToken) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(baseline)
        .context("the prepared pack baseline is missing for this Instance")?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "the prepared pack baseline is not a plain directory; it was preserved: {}",
        baseline.display()
    );
    for entry in fs::read_dir(baseline)? {
        let entry = entry?;
        ensure!(
            !cancel.is_cancelled(),
            "pack baseline initialization cancelled"
        );
        let name = pack_entry_name(&entry)?;
        validate_entry_name(&name)?;
        copy_tree(&entry.path(), &stage.join(&name), cancel)?;
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path, cancel: &CancellationToken) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "the pack baseline contains a symlink; it was preserved: {}",
        source.display()
    );
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            ensure!(
                !cancel.is_cancelled(),
                "pack baseline initialization cancelled"
            );
            let name = pack_entry_name(&entry)?;
            copy_tree(&entry.path(), &destination.join(name), cancel)?;
        }
    } else {
        ensure!(
            metadata.is_file(),
            "the pack baseline contains a special file; it was preserved: {}",
            source.display()
        );
        let mut input = File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            ensure!(
                !cancel.is_cancelled(),
                "pack baseline initialization cancelled"
            );
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
        }
    }
    Ok(())
}

/// Reject a game directory that holds anything but launcher-owned entries.
fn ensure_publishable_destination(game: &Path) -> anyhow::Result<()> {
    let mut unexpected: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(game)? {
        let entry = entry?;
        let name = entry.file_name();
        let allowed = name
            .to_str()
            .is_some_and(|name| LAUNCHER_ENTRIES.contains(&name));
        let valid_metadata_lock = name != ".block-pack.lock" || entry.file_type()?.is_file();
        if !allowed || !valid_metadata_lock {
            unexpected.push(PathBuf::from(name));
        }
    }
    ensure!(
        unexpected.is_empty(),
        "this game directory already contains data, so the pack baseline was not published; existing files were preserved: {}",
        unexpected
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

/// Remove the stage this marker proves it owns; nothing else is ever deleted.
fn reset_stage(stage: &Path) -> anyhow::Result<()> {
    let metadata = match fs::symlink_metadata(stage) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "refusing a non-directory pack staging path; it was preserved: {}",
        stage.display()
    );
    fs::remove_dir_all(stage).context("cannot remove an interrupted pack staging directory")?;
    Ok(())
}

/// Resolve the marker-owned stage beside the game directory on the same filesystem.
fn stage_path(game: &Path, stage: &str) -> anyhow::Result<PathBuf> {
    validate_stage(stage)?;
    Ok(game
        .parent()
        .context("game directory has no parent")?
        .join(stage))
}

fn remove_empty_stage(stage: &Path) {
    // Publication moved every owned entry out; a stage holding anything else is left
    // in place because the completed marker already makes retries no-ops.
    let empty = fs::read_dir(stage).is_ok_and(|mut entries| entries.next().is_none());
    if empty {
        let _ = fs::remove_dir(stage);
    }
}

fn staged_entries(stage: &Path) -> anyhow::Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(stage)? {
        entries.push(pack_entry_name(&entry?)?);
    }
    entries.sort();
    Ok(entries)
}

fn pack_entry_name(entry: &fs::DirEntry) -> anyhow::Result<String> {
    let name = entry.file_name();
    let name = name
        .to_str()
        .context("pack content has a name that cannot be published portably")?;
    validate_pack_path(name)?;
    Ok(name.to_owned())
}

/// A publishable name is one portable directory entry, never launcher state.
fn validate_entry_name(name: &str) -> anyhow::Result<()> {
    validate_pack_path(name)?;
    ensure!(
        !name.contains('/'),
        "pack content declares a nested entry where one directory entry is required: {name}"
    );
    ensure!(
        !LAUNCHER_ENTRIES.contains(&name),
        "pack content may not publish launcher state; it was preserved: {name}"
    );
    Ok(())
}

fn validate_stage(stage: &str) -> anyhow::Result<()> {
    let name = stage
        .strip_prefix(STAGE_PREFIX)
        .context("the pack initialization marker names an unsafe stage; it was preserved")?;
    ensure!(
        uuid::Uuid::parse_str(name).is_ok_and(|parsed| parsed.to_string() == name),
        "the pack initialization marker names an unsafe stage; it was preserved"
    );
    Ok(())
}

fn entry_metadata(path: &Path) -> anyhow::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn is_plain_entry(metadata: &Metadata) -> bool {
    !metadata.file_type().is_symlink() && (metadata.is_file() || metadata.is_dir())
}

fn read_marker(game: &Path) -> anyhow::Result<Option<Marker>> {
    let path = game.join(MARKER_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "the pack initialization marker is not a plain file; it was preserved: {}",
        path.display()
    );
    ensure!(
        metadata.len() <= MAX_MARKER_BYTES,
        "the pack initialization marker is too large; it was preserved"
    );
    let mut bytes = Vec::new();
    File::open(&path)?
        .take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        u64::try_from(bytes.len())? <= MAX_MARKER_BYTES,
        "the pack initialization marker grew while reading; it was preserved"
    );
    let document: MarkerDocument = serde_json::from_slice(&bytes)
        .context("invalid pack initialization marker; it was preserved")?;
    parse_marker(document).map(Some)
}

/// Atomically replace the marker.
///
/// The temporary file is created beside the game directory, not inside it: a marker
/// write interrupted by process death must never leave a stray file in a directory
/// that refuses to seed itself while unexpected entries exist.
fn write_marker(game: &Path, marker: &Marker) -> anyhow::Result<()> {
    let parent = game.parent().context("game directory has no parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .context("cannot create a pack initialization marker")?;
    serde_json::to_writer_pretty(file.as_file_mut(), &marker.document())?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(game.join(MARKER_FILE))
        .context("cannot update the pack initialization marker")?;
    Ok(())
}

fn parse_marker(document: MarkerDocument) -> anyhow::Result<Marker> {
    ensure!(
        document.schema_version == SCHEMA_VERSION,
        "unsupported pack initialization marker schema; it was preserved"
    );
    match (document.phase, document.stage, document.entries) {
        (Phase::Complete, None, entries) if entries.is_empty() => Ok(Marker::Complete),
        (Phase::Staging, Some(stage), entries) if entries.is_empty() => {
            validate_stage(&stage)?;
            Ok(Marker::Staging { stage })
        }
        (Phase::Publishing, Some(stage), entries) => {
            validate_stage(&stage)?;
            validate_entries(&entries)?;
            Ok(Marker::Publishing { stage, entries })
        }
        _ => anyhow::bail!("invalid pack initialization marker; it was preserved"),
    }
}

fn validate_entries(entries: &[String]) -> anyhow::Result<()> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        validate_entry_name(entry)
            .context("invalid pack initialization marker; it was preserved")?;
        ensure!(
            seen.insert(entry.as_str()),
            "invalid pack initialization marker; it was preserved"
        );
    }
    Ok(())
}

/// What the marker proves this operation owns in one game directory.
#[derive(Debug)]
enum Marker {
    Staging { stage: String },
    Publishing { stage: String, entries: Vec<String> },
    Complete,
}

impl Marker {
    fn document(&self) -> MarkerDocument {
        match self {
            Self::Staging { stage } => MarkerDocument {
                schema_version: SCHEMA_VERSION,
                phase: Phase::Staging,
                stage: Some(stage.clone()),
                entries: Vec::new(),
            },
            Self::Publishing { stage, entries } => MarkerDocument {
                schema_version: SCHEMA_VERSION,
                phase: Phase::Publishing,
                stage: Some(stage.clone()),
                entries: entries.clone(),
            },
            Self::Complete => MarkerDocument {
                schema_version: SCHEMA_VERSION,
                phase: Phase::Complete,
                stage: None,
                entries: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerDocument {
    schema_version: u32,
    phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stage: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Staging,
    Publishing,
    Complete,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::Context as _;

    use super::{MARKER_FILE, STAGE_PREFIX, initialize};
    use crate::{
        CancellationToken,
        catalog::{Catalog, DataPolicy, GameDirectoryGuard, Instance, PackAttachment, PackStatus},
    };

    const OPTIONS: &[u8] = b"baseline-options";
    const MOD: &[u8] = b"baseline-mod";
    const CONFIG: &[u8] = b"baseline-config";

    /// Prepare a real installation whose prepared baseline matches what the parent writes.
    fn instance_with_baseline(root: &Path) -> anyhow::Result<Instance> {
        let catalog = Catalog::open(root)?;
        let mut instance = catalog.create_instance(
            "Pack".into(),
            "1.20.6".into(),
            Some(root.join("pack")),
            None,
        )?;
        instance.pack = Some(PackAttachment {
            target_id: instance.target_id.clone(),
            status: PackStatus::Ready,
        });
        instance.game_directory = instance.resource_directory.join("isolated-game");
        let baseline = baseline(&instance);
        fs::create_dir_all(baseline.join("mods"))?;
        fs::create_dir_all(baseline.join("config"))?;
        fs::write(baseline.join("options.txt"), OPTIONS)?;
        fs::write(baseline.join("mods").join("example.jar"), MOD)?;
        fs::write(baseline.join("config").join("pack.toml"), CONFIG)?;
        Ok(instance)
    }

    fn baseline(instance: &Instance) -> PathBuf {
        instance
            .resource_directory
            .join(".block-pack")
            .join("baseline")
    }

    fn stage_name() -> String {
        format!("{STAGE_PREFIX}{}", uuid::Uuid::new_v4())
    }

    fn stage_path(game: &Path, stage: &str) -> anyhow::Result<PathBuf> {
        Ok(game
            .parent()
            .context("game directory has no parent")?
            .join(stage))
    }

    fn parent_of(game: &Path) -> anyhow::Result<&Path> {
        game.parent().context("game directory has no parent")
    }

    fn read(path: &Path) -> anyhow::Result<Vec<u8>> {
        fs::read(path).with_context(|| format!("cannot read {}", path.display()))
    }

    fn marker_phase(game: &Path) -> anyhow::Result<String> {
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(game.join(MARKER_FILE))?)?;
        Ok(document
            .get("phase")
            .and_then(serde_json::Value::as_str)
            .map_or_else(String::new, str::to_owned))
    }

    fn stages(directory: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(STAGE_PREFIX))
            {
                found.push(entry.path());
            }
        }
        Ok(found)
    }

    /// Model an interrupted run: the marker on disk is the only ownership record.
    fn write_staging_marker(game: &Path, stage: &str) -> anyhow::Result<()> {
        fs::write(
            game.join(MARKER_FILE),
            format!(r#"{{"schema_version":1,"phase":"staging","stage":"{stage}"}}"#),
        )?;
        Ok(())
    }

    fn write_publishing_marker(game: &Path, stage: &str, entries: &[&str]) -> anyhow::Result<()> {
        let entries = entries
            .iter()
            .map(|entry| format!("\"{entry}\""))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            game.join(MARKER_FILE),
            format!(
                r#"{{"schema_version":1,"phase":"publishing","stage":"{stage}","entries":[{entries}]}}"#
            ),
        )?;
        Ok(())
    }

    fn rejection(result: anyhow::Result<()>) -> anyhow::Result<String> {
        match result {
            Ok(()) => anyhow::bail!("pack baseline initialization unexpectedly succeeded"),
            Err(error) => Ok(format!("{error:#}")),
        }
    }

    #[test]
    fn a_fresh_game_directory_is_seeded_from_the_baseline() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        assert!(!game.exists());
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        initialize(&instance, &guard, &CancellationToken::new())?;
        assert_eq!(read(&game.join("options.txt"))?, OPTIONS);
        assert_eq!(read(&game.join("mods").join("example.jar"))?, MOD);
        assert_eq!(read(&game.join("config").join("pack.toml"))?, CONFIG);
        assert_eq!(marker_phase(&game)?, "complete");
        assert!(stages(parent_of(&game)?)?.is_empty());
        Ok(())
    }

    #[test]
    fn an_existing_empty_game_directory_is_seeded() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        fs::create_dir_all(&game)?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        // Launcher-owned names are not user data.
        fs::write(game.join(".block-session.json"), b"{}")?;
        initialize(&instance, &guard, &CancellationToken::new())?;
        assert_eq!(read(&game.join("options.txt"))?, OPTIONS);
        assert_eq!(marker_phase(&game)?, "complete");
        Ok(())
    }

    #[test]
    fn existing_user_data_is_rejected_and_preserved() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        fs::create_dir_all(game.join("saves"))?;
        fs::write(game.join("saves").join("level.dat"), b"world")?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        let message = rejection(initialize(&instance, &guard, &CancellationToken::new()))?;
        assert!(message.contains("already contains data"), "{message}");
        assert_eq!(read(&game.join("saves").join("level.dat"))?, b"world");
        assert!(!game.join(MARKER_FILE).exists());
        assert!(!game.join("options.txt").exists());
        Ok(())
    }

    #[test]
    fn a_completed_initialization_never_rewrites_played_data() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let mut instance = instance_with_baseline(root.path())?;
        instance.data_policy = DataPolicy::Shared;
        instance.game_directory = instance.resource_directory.clone();
        let game = instance.game_directory.clone();
        {
            let guard = GameDirectoryGuard::for_instance(&instance)?;
            initialize(&instance, &guard, &CancellationToken::new())?;
        }
        fs::write(game.join("options.txt"), b"played-options")?;
        fs::remove_file(game.join("mods").join("example.jar"))?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        initialize(&instance, &guard, &CancellationToken::new())?;
        assert_eq!(read(&game.join("options.txt"))?, b"played-options");
        assert!(!game.join("mods").join("example.jar").exists());
        assert_eq!(marker_phase(&game)?, "complete");
        Ok(())
    }

    #[test]
    fn an_interrupted_staging_step_stays_incomplete_and_resumes() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        fs::create_dir_all(&game)?;
        let stage = stage_name();
        write_staging_marker(&game, &stage)?;
        let partial = stage_path(&game, &stage)?;
        fs::create_dir_all(partial.join("mods"))?;
        fs::write(partial.join("mods").join("partial.jar"), b"partial")?;

        let guard = GameDirectoryGuard::for_instance(&instance)?;
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let message = rejection(initialize(&instance, &guard, &cancelled))?;
        assert!(message.contains("cancelled"), "{message}");
        assert_eq!(marker_phase(&game)?, "staging");
        assert!(!game.join("options.txt").exists());

        initialize(&instance, &guard, &CancellationToken::new())?;
        assert_eq!(marker_phase(&game)?, "complete");
        assert_eq!(read(&game.join("options.txt"))?, OPTIONS);
        assert_eq!(read(&game.join("mods").join("example.jar"))?, MOD);
        assert!(!game.join("mods").join("partial.jar").exists());
        assert!(stages(parent_of(&game)?)?.is_empty());
        Ok(())
    }

    #[test]
    fn an_interrupted_publication_resumes_without_overwriting_published_entries()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        fs::create_dir_all(&game)?;
        // options.txt was published before the interruption; mods was still staged.
        fs::write(game.join("options.txt"), OPTIONS)?;
        let stage = stage_name();
        let staged = stage_path(&game, &stage)?;
        fs::create_dir_all(staged.join("mods"))?;
        fs::write(staged.join("mods").join("example.jar"), MOD)?;
        write_publishing_marker(&game, &stage, &["mods", "options.txt"])?;

        let guard = GameDirectoryGuard::for_instance(&instance)?;
        initialize(&instance, &guard, &CancellationToken::new())?;
        assert_eq!(marker_phase(&game)?, "complete");
        assert_eq!(read(&game.join("options.txt"))?, OPTIONS);
        assert_eq!(read(&game.join("mods").join("example.jar"))?, MOD);
        assert!(stages(parent_of(&game)?)?.is_empty());
        Ok(())
    }

    #[test]
    fn a_conflicting_publication_stays_blocked_and_preserved() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        fs::create_dir_all(&game)?;
        fs::write(game.join("options.txt"), b"user-options")?;
        let stage = stage_name();
        let staged = stage_path(&game, &stage)?;
        fs::create_dir_all(&staged)?;
        fs::write(staged.join("options.txt"), OPTIONS)?;
        write_publishing_marker(&game, &stage, &["options.txt"])?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        for _ in 0..2 {
            let message = rejection(initialize(&instance, &guard, &CancellationToken::new()))?;
            assert!(message.contains("already exists"), "{message}");
            assert_eq!(read(&game.join("options.txt"))?, b"user-options");
            assert_eq!(marker_phase(&game)?, "publishing");
        }
        Ok(())
    }

    #[test]
    fn an_isolated_directory_seeds_from_the_baseline_rather_than_the_played_shared_game()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let mut instance = instance_with_baseline(root.path())?;
        instance.data_policy = DataPolicy::Shared;
        instance.game_directory = instance.resource_directory.clone();
        let shared = instance.game_directory.clone();
        {
            let guard = GameDirectoryGuard::for_instance(&instance)?;
            initialize(&instance, &guard, &CancellationToken::new())?;
        }
        fs::write(shared.join("options.txt"), b"played-options")?;

        instance.data_policy = DataPolicy::Isolated;
        instance.game_directory = instance.resource_directory.join("isolated-game");
        let isolated = instance.game_directory.clone();
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        initialize(&instance, &guard, &CancellationToken::new())?;
        assert_eq!(read(&isolated.join("options.txt"))?, OPTIONS);
        assert_eq!(read(&shared.join("options.txt"))?, b"played-options");
        Ok(())
    }

    #[test]
    fn switching_data_policy_keeps_the_other_directory_game_data() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let mut instance = instance_with_baseline(root.path())?;
        let isolated = instance.game_directory.clone();
        {
            let guard = GameDirectoryGuard::for_instance(&instance)?;
            initialize(&instance, &guard, &CancellationToken::new())?;
        }
        fs::write(isolated.join("options.txt"), b"isolated-options")?;

        instance.data_policy = DataPolicy::Shared;
        instance.game_directory = instance
            .resource_directory
            .parent()
            .context("Resource parent")?
            .join("shared-game");
        let shared = instance.game_directory.clone();
        {
            let guard = GameDirectoryGuard::for_instance(&instance)?;
            initialize(&instance, &guard, &CancellationToken::new())?;
        }
        assert_eq!(read(&shared.join("options.txt"))?, OPTIONS);

        instance.data_policy = DataPolicy::Isolated;
        instance.game_directory = instance.resource_directory.join("isolated-game");
        {
            let guard = GameDirectoryGuard::for_instance(&instance)?;
            initialize(&instance, &guard, &CancellationToken::new())?;
        }
        assert_eq!(read(&isolated.join("options.txt"))?, b"isolated-options");
        assert_eq!(marker_phase(&isolated)?, "complete");
        Ok(())
    }

    #[test]
    fn selecting_an_uninitialized_ancestor_preserves_the_existing_child_game() -> anyhow::Result<()>
    {
        let root = tempfile::tempdir()?;
        let mut instance = instance_with_baseline(root.path())?;
        let isolated = instance.game_directory.clone();
        {
            let guard = GameDirectoryGuard::for_instance(&instance)?;
            initialize(&instance, &guard, &CancellationToken::new())?;
        }
        fs::write(isolated.join("options.txt"), b"played-options")?;
        instance.data_policy = DataPolicy::Shared;
        instance.game_directory = instance.resource_directory.clone();
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        let message = rejection(initialize(&instance, &guard, &CancellationToken::new()))?;
        assert!(message.contains("already contains data"), "{message}");
        assert_eq!(read(&isolated.join("options.txt"))?, b"played-options");
        assert!(!instance.game_directory.join("options.txt").exists());
        assert!(!instance.game_directory.join(MARKER_FILE).exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_metadata_lock_does_not_authorize_baseline_publication() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        let outside = root.path().join("external-lock");
        fs::write(&outside, b"outside")?;
        std::os::unix::fs::symlink(&outside, guard.directory().join(".block-pack.lock"))?;
        let message = rejection(initialize(&instance, &guard, &CancellationToken::new()))?;
        assert!(message.contains("already contains data"), "{message}");
        assert_eq!(read(&outside)?, b"outside");
        assert!(!guard.directory().join("options.txt").exists());
        Ok(())
    }

    #[test]
    fn initialization_reuses_the_callers_game_directory_lock() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        initialize(&instance, &guard, &CancellationToken::new())?;
        assert!(GameDirectoryGuard::for_instance(&instance).is_err());
        drop(guard);
        assert!(GameDirectoryGuard::for_instance(&instance).is_ok());
        Ok(())
    }

    #[test]
    fn a_malformed_marker_is_rejected_and_preserved() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        fs::create_dir_all(&game)?;
        fs::write(game.join(MARKER_FILE), b"{ not a marker")?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        let message = rejection(initialize(&instance, &guard, &CancellationToken::new()))?;
        assert!(
            message.contains("invalid pack initialization marker"),
            "{message}"
        );
        assert_eq!(read(&game.join(MARKER_FILE))?, b"{ not a marker");
        assert!(!game.join("options.txt").exists());
        Ok(())
    }

    #[test]
    fn declared_entries_cannot_escape_the_game_directory() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let game = instance.game_directory.clone();
        fs::create_dir_all(&game)?;
        let stage = stage_name();
        write_publishing_marker(&game, &stage, &["../escape"])?;
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        let message = rejection(initialize(&instance, &guard, &CancellationToken::new()))?;
        assert!(message.contains("unsafe component"), "{message}");
        assert!(!parent_of(&game)?.join("escape").exists());
        assert!(!instance.resource_directory.join("escape").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_baseline_entry_is_rejected_then_repaired() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir()?;
        let instance = instance_with_baseline(root.path())?;
        let baseline = baseline(&instance);
        symlink(baseline.join("options.txt"), baseline.join("linked.jar"))?;
        let game = instance.game_directory.clone();
        let guard = GameDirectoryGuard::for_instance(&instance)?;
        let message = rejection(initialize(&instance, &guard, &CancellationToken::new()))?;
        assert!(message.contains("symlink"), "{message}");
        assert_eq!(marker_phase(&game)?, "staging");
        assert!(!game.join("linked.jar").exists());

        fs::remove_file(baseline.join("linked.jar"))?;
        initialize(&instance, &guard, &CancellationToken::new())?;
        assert_eq!(marker_phase(&game)?, "complete");
        assert_eq!(read(&game.join("options.txt"))?, OPTIONS);
        Ok(())
    }
}
