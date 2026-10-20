//! Explicit repair of declared immutable views; mutable game data is never changed.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use block_download::{Resource, Store};
use sha1::{Digest as _, Sha1};
use tokio_util::sync::CancellationToken;

const RUN_ID_TOKEN: &str = "${run_id}";

use crate::{
    executor::confined_path,
    plan::{ExecutionPlan, PrepareAction},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedCopy {
    pub original: PathBuf,
    pub quarantine: PathBuf,
}

/// Preserve corrupt copies so normal materialization can restore their verified objects.
///
/// Call only for an explicit installation/Repair, after `Store::fetch_all` succeeds
/// and while holding the installation's exclusive preparation lock. Normal Play
/// must report corruption instead of invoking this function. Only known immutable
/// views and managed runtime copies are eligible; game directories are skipped.
///
/// # Errors
/// Rejects unsafe or symlinked paths, undeclared/mismatched source objects,
/// non-file destinations, cancellation, or files that cannot be safely renamed.
/// On Windows, close running games before retrying any sharing violation. Already
/// quarantined files remain preserved when a later file fails or repair is cancelled.
pub fn quarantine_corrupt_copies(
    store: &Store,
    plan: &ExecutionPlan,
    run_id: &str,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<QuarantinedCopy>> {
    ensure!(
        !run_id.is_empty()
            && run_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid repair run identifier"
    );
    let root = store.root().canonicalize()?;
    let resources: BTreeMap<_, _> = plan
        .resources
        .iter()
        .map(|resource| (resource.sha1.as_str(), resource))
        .collect();
    let mut candidates = BTreeMap::new();
    // Preflight every declared immutable copy before changing any destination.
    for action in &plan.actions {
        ensure!(!cancel.is_cancelled(), "repair cancelled");
        let PrepareAction::Copy { sha1, path, .. } = action else {
            continue;
        };
        let relative = path.replace(RUN_ID_TOKEN, run_id);
        let destination = confined_path(&root, &relative)?;
        ensure_write_path(&root, &destination)?;
        if !immutable_namespace(&relative) {
            continue;
        }
        let resource = resources
            .get(sha1.as_str())
            .context("immutable repair copy has no declared resource")?;
        let object = store.object_path(sha1)?;
        let source = root.join(object.strip_prefix(store.root())?);
        ensure_write_path(&root, &source)?;
        ensure!(
            matches_resource(&source, resource)?,
            "repair source object is missing or corrupt; retry the verified download first: {sha1}"
        );
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file(),
                    "immutable repair destination is not a regular file: {}",
                    destination.display()
                );
                if !matches_resource(&destination, resource).with_context(|| {
                    format!(
                        "cannot verify immutable resource {}; stop running games and retry Repair",
                        destination.display()
                    )
                })? && let Some(previous) = candidates.insert(destination, *resource)
                {
                    ensure!(previous == *resource, "conflicting repair copy descriptors");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut quarantined = Vec::new();
    for (destination, resource) in candidates {
        ensure!(!cancel.is_cancelled(), "repair cancelled");
        ensure_write_path(&root, &destination)?;
        // Another installation may have already repaired the shared view.
        if matches_resource(&destination, resource).with_context(|| {
            format!(
                "cannot verify immutable resource {}; stop running games and retry Repair",
                destination.display()
            )
        })? {
            continue;
        }
        #[cfg(windows)]
        check_file_available(&destination)?;
        let name = destination
            .file_name()
            .context("repair copy has no filename")?
            .to_str()
            .context("repair filename must be UTF-8")?;
        let quarantine =
            destination.with_file_name(format!("{name}.corrupt-{}", uuid::Uuid::new_v4()));
        ensure_write_path(&root, &quarantine)?;
        ensure!(
            !quarantine.exists(),
            "repair quarantine path already exists"
        );
        fs::rename(&destination, &quarantine).with_context(|| format!("cannot quarantine corrupt immutable resource {}; stop running games and retry Repair", destination.display()))?;
        quarantined.push(QuarantinedCopy {
            original: destination,
            quarantine,
        });
    }
    Ok(quarantined)
}

fn immutable_namespace(relative: &str) -> bool {
    let mut parts = relative.split('/');
    matches!(
        (parts.next(), parts.next()),
        (Some("runtimes"), Some(_))
            | (
                Some("views"),
                Some("libraries" | "clients" | "assets" | "logging" | "neoforge")
            )
    )
}

fn ensure_write_path(root: &Path, path: &Path) -> anyhow::Result<()> {
    let relative = path.strip_prefix(root)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "refusing repair through symlink: {}",
                current.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn matches_resource(path: &Path, resource: &Resource) -> anyhow::Result<bool> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "repair object is not a regular file");
    if metadata.len() != resource.size {
        return Ok(false);
    }
    let mut digest = Sha1::new();
    let mut buffer = vec![0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()) == resource.sha1)
}

#[cfg(windows)]
fn check_file_available(path: &Path) -> anyhow::Result<()> {
    use std::os::windows::fs::OpenOptionsExt as _;
    // Do not unlock, alter attributes, or replace resources held by another process.
    let probe = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .with_context(|| {
            format!(
                "immutable resource {} is in use; stop running games and retry Repair",
                path.display()
            )
        })?;
    drop(probe);
    Ok(())
}
