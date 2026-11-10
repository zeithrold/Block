//! Canonical actual game-directory locks and sticky supervisor reservations.
use super::{Instance, normalize_directory, validate_directory};
use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

const SESSION_MARKER: &str = ".block-session.json";
const MAX_SESSION_MARKER_BYTES: u64 = 2048;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionReservation {
    schema_version: u32,
    run_id: String,
    supervisor_pid: u32,
}

/// A process owns this canonical mutable directory until the guard is dropped.
#[derive(Debug)]
pub struct GameDirectoryGuard {
    directory: PathBuf,
    _lock: File,
}

impl Drop for GameDirectoryGuard {
    fn drop(&mut self) {
        // A concurrent process spawn can briefly inherit the open description
        // before exec closes it. Release ownership when this guard ends.
        let _ = fs2::FileExt::unlock(&self._lock);
    }
}

impl GameDirectoryGuard {
    /// # Errors
    /// Rejects concurrent game access, sticky session reservations and symlinked lock files.
    pub fn acquire(directory: &Path) -> anyhow::Result<Self> {
        validate_directory(directory)?;
        let planned = normalize_directory(directory)?;
        fs::create_dir_all(&planned)?;
        let directory = planned.canonicalize()?;
        ensure!(
            directory == planned,
            "game directory changed while creating it"
        );
        reject_session_reservation(&directory)?;
        let lock = open_lock(&directory.join(".block-game.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("this game directory is already preparing or running; stop that game first")?;
        reject_session_reservation(&directory)?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }

    /// Lock exactly the Instance's declared game directory.
    /// # Errors
    /// Rejects replaced paths before creating missing data or accepting ownership.
    pub fn for_instance(instance: &Instance) -> anyhow::Result<Self> {
        ensure!(
            instance.resource_directory.canonicalize()? == instance.resource_directory,
            "resource directory changed; reselect its location before playing"
        );
        ensure!(
            normalize_directory(&instance.game_directory)? == instance.game_directory,
            "game directory changed; reselect its location before playing"
        );
        let guard = Self::acquire(&instance.game_directory)?;
        ensure!(
            guard.directory() == instance.game_directory,
            "game directory changed while acquiring its lock"
        );
        Ok(guard)
    }

    /// Reserve the mutable directory for a supervised game session.
    ///
    /// The reservation deliberately survives this guard and process termination.
    /// Its owner must clear it only after confirming the Java child is terminal.
    ///
    /// # Errors
    /// Rejects invalid run IDs, existing reservations and the same path/lock hazards
    /// as `for_instance`. An existing reservation is never automatically replaced.
    pub fn for_session(instance: &Instance, run_id: &str) -> anyhow::Result<Self> {
        validate_run_id(run_id)?;
        let guard = Self::for_instance(instance)?;
        let reservation = SessionReservation {
            schema_version: 1,
            run_id: run_id.into(),
            supervisor_pid: std::process::id(),
        };
        let mut temp = tempfile::NamedTempFile::new_in(guard.directory())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temp.write_all(&serde_json::to_vec(&reservation)?)?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(guard.directory().join(SESSION_MARKER))
            .context("another game session reserved this directory")?;
        Ok(guard)
    }

    /// Release exactly this run's reservation after the Java child has ended.
    ///
    /// Only the owning supervisor after confirmed Java exit, or an operator that
    /// verified both processes ended and established exclusive access, may call this.
    /// This function does not acquire the directory lock, infer death from a PID,
    /// or automatically repair stale state.
    ///
    /// # Errors
    /// Rejects missing, malformed, oversized, symlinked or differently owned markers.
    pub fn clear_session_reservation(directory: &Path, run_id: &str) -> anyhow::Result<()> {
        validate_run_id(run_id)?;
        let (path, metadata, reservation) = read_session_reservation(directory)?;
        ensure!(
            reservation.run_id == run_id,
            "session reservation belongs to another run"
        );
        let current = fs::symlink_metadata(&path)?;
        ensure!(
            current.is_file()
                && !current.file_type().is_symlink()
                && same_file(&metadata, &current),
            "session reservation changed while releasing; it was preserved"
        );
        fs::remove_file(path)?;
        Ok(())
    }

    pub(crate) fn session_reservation_owner(directory: &Path) -> anyhow::Result<Option<String>> {
        match read_session_reservation(directory) {
            Ok((_, _, reservation)) => Ok(Some(reservation.run_id)),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.directory.canonicalize()? == self.directory,
            "game directory changed while preparing the launch"
        );
        let current = fs::symlink_metadata(self.directory.join(".block-game.lock"))?;
        ensure!(
            current.is_file()
                && !current.file_type().is_symlink()
                && same_file(&current, &self._lock.metadata()?),
            "game directory lock changed while preparing the launch"
        );
        Ok(())
    }
}

fn read_session_reservation(
    directory: &Path,
) -> anyhow::Result<(PathBuf, Metadata, SessionReservation)> {
    validate_directory(directory)?;
    ensure!(
        directory.canonicalize()? == directory,
        "game directory changed before inspecting its session reservation"
    );
    let path = directory.join(SESSION_MARKER);
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "refusing a non-file or symlinked session reservation"
    );
    ensure!(
        metadata.len() <= MAX_SESSION_MARKER_BYTES,
        "session reservation is too large"
    );
    let file = File::open(&path)?;
    ensure!(
        same_file(&metadata, &file.metadata()?),
        "session reservation changed while opening"
    );
    let mut bytes = Vec::new();
    file.take(MAX_SESSION_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_SESSION_MARKER_BYTES,
        "session reservation grew while reading"
    );
    let reservation: SessionReservation =
        serde_json::from_slice(&bytes).context("invalid session reservation; it was preserved")?;
    ensure!(
        reservation.schema_version == 1,
        "unsupported session reservation schema"
    );
    validate_run_id(&reservation.run_id)?;
    let current = fs::symlink_metadata(&path)?;
    ensure!(
        current.is_file() && !current.file_type().is_symlink() && same_file(&metadata, &current),
        "session reservation changed while reading"
    );
    Ok((path, metadata, reservation))
}

pub(super) fn same_file(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if left.dev() != right.dev() || left.ino() != right.ino() {
            return false;
        }
    }
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.created().ok() == right.created().ok()
}

fn validate_run_id(run_id: &str) -> anyhow::Result<()> {
    let parsed = uuid::Uuid::parse_str(run_id).context("invalid game session run ID")?;
    ensure!(
        parsed.to_string() == run_id,
        "game session run ID must be a canonical UUID"
    );
    Ok(())
}

fn reject_session_reservation(directory: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(directory.join(SESSION_MARKER)) {
        Ok(_) => anyhow::bail!(
            "this game directory has a reserved session; reconnect to its supervisor or confirm the previous game has ended before recovering it"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn open_lock(path: &Path) -> anyhow::Result<File> {
    reject_symlink(path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let current = fs::symlink_metadata(path)?;
    ensure!(
        current.is_file()
            && !current.file_type().is_symlink()
            && same_file(&current, &file.metadata()?),
        "lock file changed while opening"
    );
    Ok(file)
}

pub(super) fn reject_symlink(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            !metadata.file_type().is_symlink(),
            "refusing a symlinked catalog/lock file"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(unix)]
#[cfg(test)]
mod guard_tests {
    use super::GameDirectoryGuard;

    #[test]
    fn releasing_directory_ownership_unlocks_inherited_file_descriptions() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let guard = GameDirectoryGuard::acquire(directory.path())?;
        let inherited = guard._lock.try_clone()?;
        assert!(GameDirectoryGuard::acquire(directory.path()).is_err());
        drop(guard);
        let next_owner = GameDirectoryGuard::acquire(directory.path())?;
        drop(inherited);
        assert!(GameDirectoryGuard::acquire(directory.path()).is_err());
        drop(next_owner);
        assert!(GameDirectoryGuard::acquire(directory.path()).is_ok());
        Ok(())
    }

    #[test]
    fn replacing_a_locked_directory_is_rejected_before_game_writes() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let directory = root.path().join("game");
        let previous = root.path().join("previous");
        let guard = GameDirectoryGuard::acquire(&directory)?;
        std::fs::rename(&directory, &previous)?;
        std::fs::create_dir(&directory)?;
        std::fs::write(directory.join(".block-game.lock"), [])?;
        assert!(guard.validate().is_err());
        std::fs::remove_dir_all(&directory)?;
        std::fs::rename(&previous, &directory)?;
        assert!(guard.validate().is_ok());
        Ok(())
    }
}
