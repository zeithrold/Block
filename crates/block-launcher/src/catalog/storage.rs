use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};

use super::{
    DataPolicy, InstallStatus, Instance, LaunchTarget, LoaderKind, Profile, ProfileMode, Snapshot,
    validate_directory, validate_id, validate_name,
};
use crate::recipe::{Compatibility, RuntimeSelection};

#[derive(Debug, Clone)]
pub struct Catalog {
    root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    snapshot: Snapshot,
}

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

impl GameDirectoryGuard {
    /// # Errors
    /// Rejects concurrent game access, sticky session reservations and symlinked lock files.
    pub fn acquire(directory: &Path) -> anyhow::Result<Self> {
        validate_directory(directory)?;
        fs::create_dir_all(directory)?;
        let directory = directory.canonicalize()?;
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

    /// Acquire only the declared profile tree, rejecting aliasing through child symlinks.
    /// # Errors
    /// A replaced installation root or symlinked mutable child is rejected before creating data.
    pub fn for_profile(instance: &Instance, profile: &Profile) -> anyhow::Result<Self> {
        ensure!(
            instance.directory.canonicalize()? == instance.directory,
            "installation directory changed; reselect its location before playing"
        );
        let directory = profile.game_directory(instance)?;
        crate::executor::ensure_write_path(&instance.directory, &directory)?;
        let guard = Self::acquire(&directory)?;
        ensure!(
            guard.directory() == directory,
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
    /// as `for_profile`. An existing reservation is never automatically replaced.
    pub fn for_session(
        instance: &Instance,
        profile: &Profile,
        run_id: &str,
    ) -> anyhow::Result<Self> {
        validate_run_id(run_id)?;
        let guard = Self::for_profile(instance, profile)?;
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

fn same_file(left: &Metadata, right: &Metadata) -> bool {
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

impl Catalog {
    /// Open metadata separately from immutable cached resources.
    /// # Errors
    /// Fails if the data root cannot be created/canonicalized.
    pub fn open(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// # Errors
    /// Rejects corrupt catalogs without replacing or resetting their contents.
    pub fn snapshot(&self) -> anyhow::Result<Snapshot> {
        self.transaction(false, |state| Ok(state.clone()))
    }

    /// Permanently allow multiple Profiles to reuse one Instance.
    ///
    /// Only catalog metadata changes. Installed resources, worlds and profile
    /// directories remain in place. There is deliberately no reverse operation.
    /// # Errors
    /// Rejects repeated conversion or an invalid catalog, preserving its contents.
    pub fn separate_profiles(&self) -> anyhow::Result<()> {
        self.transaction(true, |state| {
            ensure!(
                state.profile_mode == ProfileMode::Bundled,
                "Separated Profile is already enabled; conversion cannot be reversed"
            );
            state.profile_mode = ProfileMode::Separated;
            Ok(())
        })
    }

    /// Create an installation and its first independent launch recipe atomically.
    /// # Errors
    /// Rejects invalid names, unsupported pins and overlapping installation roots.
    pub fn create_instance(
        &self,
        name: String,
        minecraft_version: String,
        directory: Option<PathBuf>,
        loader_version: Option<String>,
    ) -> anyhow::Result<(Instance, Profile)> {
        validate_name(&name)?;
        ensure!(
            !minecraft_version.is_empty()
                && minecraft_version.len() <= 80
                && !minecraft_version.chars().any(char::is_control),
            "invalid Minecraft version"
        );
        if let Some(version) = &loader_version {
            ensure!(
                minecraft_version == "1.20.6" && version == "20.6.144",
                "this MVP supports NeoForge20.6.144 with Minecraft1.20.6"
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        let directory = directory.unwrap_or_else(|| self.root.join("instances").join(&id));
        validate_directory(&directory)?;
        fs::create_dir_all(&directory)?;
        let directory = directory.canonicalize()?;
        let mut instance = Instance {
            id,
            name,
            minecraft_version,
            directory,
            targets: vec![LaunchTarget {
                id: "vanilla".into(),
                kind: LoaderKind::Vanilla,
                loader_version: None,
                manifest: None,
                status: InstallStatus::Pending,
            }],
        };
        let target_id = loader_version.map_or_else(
            || "vanilla".to_owned(),
            |version| {
                let id = format!("neoforge-{version}");
                instance.targets.push(LaunchTarget {
                    id: id.clone(),
                    kind: LoaderKind::NeoForge,
                    loader_version: Some(version),
                    manifest: None,
                    status: InstallStatus::Pending,
                });
                id
            },
        );
        let mut profile = new_profile(&instance, "Default".into());
        profile.target_id = target_id;
        self.transaction(true, |state| {
            if state.profile_mode == ProfileMode::Bundled {
                profile.name.clone_from(&instance.name);
            }
            ensure!(
                !state
                    .instances
                    .iter()
                    .any(|i| instance.directory.starts_with(&i.directory)
                        || i.directory.starts_with(&instance.directory)),
                "installation directory overlaps another instance"
            );
            state.instances.push(instance.clone());
            state.profiles.push(profile.clone());
            Ok((instance, profile))
        })
    }

    /// # Errors
    /// Fails for a missing installation or invalid name.
    pub fn create_profile(&self, instance_id: &str, name: String) -> anyhow::Result<Profile> {
        validate_name(&name)?;
        self.transaction(true, |state| {
            ensure!(
                state.profile_mode == ProfileMode::Separated,
                "Bundled Profile requires a new Instance; convert to Separated Profile to reuse one"
            );
            let instance = state
                .instances
                .iter()
                .find(|i| i.id == instance_id)
                .context("installation no longer exists")?;
            let profile = new_profile(instance, name);
            state.profiles.push(profile.clone());
            Ok(profile)
        })
    }

    /// # Errors
    /// Rejects stale IDs, cross-installation moves and missing targets.
    pub fn save_profile(&self, profile: Profile) -> anyhow::Result<()> {
        profile.validate()?;
        self.transaction(true, |state| {
            let instance = state
                .instances
                .iter()
                .find(|i| i.id == profile.instance_id)
                .context("installation no longer exists")?;
            ensure!(
                instance.targets.iter().any(|t| t.id == profile.target_id),
                "launch target is not installed in this instance"
            );
            let old = state
                .profiles
                .iter_mut()
                .find(|p| p.id == profile.id)
                .context("profile no longer exists")?;
            ensure!(
                old.instance_id == profile.instance_id,
                "moving profiles between installations is not supported"
            );
            *old = profile;
            Ok(())
        })
    }

    /// Copy settings, giving isolated data a new empty directory; never copy live worlds.
    /// # Errors
    /// Fails for a missing profile or invalid name.
    pub fn copy_profile(&self, id: &str, name: String) -> anyhow::Result<Profile> {
        validate_name(&name)?;
        self.transaction(true, |state| {
            ensure!(
                state.profile_mode == ProfileMode::Separated,
                "Bundled Profile cannot share an Instance; convert to Separated Profile before copying"
            );
            let mut profile = state
                .profiles
                .iter()
                .find(|p| p.id == id)
                .context("profile no longer exists")?
                .clone();
            profile.id = uuid::Uuid::new_v4().to_string();
            profile.name = name;
            state.profiles.push(profile.clone());
            Ok(profile)
        })
    }

    /// Remove only metadata. Bundled mode also removes the associated Instance
    /// metadata; shared resources and every user-data directory remain intact.
    /// # Errors
    /// Fails if the selected profile no longer exists.
    pub fn delete_profile(&self, id: &str) -> anyhow::Result<()> {
        self.transaction(true, |state| {
            let instance_id = state
                .profiles
                .iter()
                .find(|p| p.id == id)
                .context("profile no longer exists")?
                .instance_id
                .clone();
            state.profiles.retain(|p| p.id != id);
            if state.profile_mode == ProfileMode::Bundled {
                state
                    .instances
                    .retain(|instance| instance.id != instance_id);
            }
            Ok(())
        })
    }

    /// # Errors
    /// Only replaces an already declared target in its original installation.
    pub fn set_target(&self, instance_id: &str, target: &LaunchTarget) -> anyhow::Result<()> {
        self.transaction(true, |state| {
            let instance = state
                .instances
                .iter_mut()
                .find(|i| i.id == instance_id)
                .context("installation no longer exists")?;
            let old = instance
                .targets
                .iter_mut()
                .find(|t| t.id == target.id)
                .context("launch target no longer exists")?;
            ensure!(
                old.kind == target.kind && old.loader_version == target.loader_version,
                "cannot change installed target identity"
            );
            *old = target.clone();
            Ok(())
        })
    }

    fn transaction<T>(
        &self,
        write: bool,
        operation: impl FnOnce(&mut Snapshot) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let lock = open_lock(&self.root.join("catalog.lock"))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let path = self.root.join("catalog.json");
        reject_symlink(&path)?;
        let mut document = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Document>(&bytes)
                .context("catalog is invalid; original file was preserved")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Document {
                schema_version: 1,
                snapshot: Snapshot::default(),
            },
            Err(error) => return Err(error.into()),
        };
        ensure!(
            document.schema_version == 1,
            "unsupported catalog schema; original file was preserved"
        );
        validate_snapshot(&document.snapshot)?;
        let result = operation(&mut document.snapshot)?;
        if write {
            validate_snapshot(&document.snapshot)?;
            let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
            temp.write_all(&serde_json::to_vec_pretty(&document)?)?;
            temp.as_file().sync_all()?;
            temp.persist(path)?;
        }
        Ok(result)
    }
}

fn open_lock(path: &Path) -> anyhow::Result<File> {
    reject_symlink(path)?;
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?)
}

fn reject_symlink(path: &Path) -> anyhow::Result<()> {
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

fn new_profile(instance: &Instance, name: String) -> Profile {
    Profile {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        instance_id: instance.id.clone(),
        target_id: "vanilla".into(),
        runtime: RuntimeSelection::Managed,
        memory_mib: 2048,
        jvm_args: Vec::new(),
        game_args: Vec::new(),
        data_policy: DataPolicy::Isolated,
        default_account: None,
        compatibility: if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            Compatibility::LinuxArm64Lwjgl3
        } else {
            Compatibility::Official
        },
    }
}

fn validate_snapshot(state: &Snapshot) -> anyhow::Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    for instance in &state.instances {
        validate_id(&instance.id)?;
        validate_name(&instance.name)?;
        validate_directory(&instance.directory)?;
        ensure!(ids.insert(&instance.id), "duplicate installation ID");
        let mut targets = std::collections::BTreeSet::new();
        for target in &instance.targets {
            ensure!(
                !target.id.is_empty()
                    && target.id.len() <= 80
                    && target
                        .id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
                "invalid launch target ID"
            );
            ensure!(targets.insert(&target.id), "duplicate launch target ID");
        }
    }
    let mut profile_ids = std::collections::BTreeSet::new();
    let mut profiled_instances = std::collections::BTreeSet::new();
    for profile in &state.profiles {
        profile.validate()?;
        ensure!(profile_ids.insert(&profile.id), "duplicate profile ID");
        state.selection(&profile.id)?;
        if state.profile_mode == ProfileMode::Bundled {
            ensure!(
                profiled_instances.insert(&profile.instance_id),
                "Bundled Profile requires exactly one Profile per Instance"
            );
        }
    }
    if state.profile_mode == ProfileMode::Bundled {
        ensure!(
            profiled_instances.len() == state.instances.len(),
            "Bundled Profile requires exactly one Profile per Instance"
        );
    }
    Ok(())
}
