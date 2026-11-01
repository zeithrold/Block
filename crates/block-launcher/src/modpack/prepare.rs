//! Materialize a pack's immutable baseline without changing played game data.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::Path,
    time::Duration,
};

use anyhow::{Context as _, ensure};
use block_download::{Resource, Store};
use tokio_util::sync::CancellationToken;

use super::{PackInspection, descriptor::PackDescriptor};
use crate::catalog::Instance;

/// Verify the selected downloads and publish the original pack baseline.
pub(crate) async fn prepare(
    store: &Store,
    instance: &Instance,
    descriptor: &PackDescriptor,
    inspection: &PackInspection,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    ensure!(
        descriptor.matches_inspection(inspection),
        "the selected archive does not match this pack; select the original pack"
    );
    ensure!(
        !descriptor.has_missing_selected(),
        "pack files cannot be verified from their CurseForge IDs"
    );
    ensure!(!cancel.is_cancelled(), "pack installation cancelled");
    let directory = instance.directory.join(".block-pack");
    crate::executor::ensure_write_path(&instance.directory, &directory)?;
    let metadata = fs::symlink_metadata(&directory)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "pack metadata directory is not a plain directory"
    );
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("install.lock"))?;
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => break,
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(error) => return Err(error.into()),
        }
        tokio::select! {
            () = cancel.cancelled() => anyhow::bail!("pack installation cancelled"),
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    let available: BTreeMap<_, _> = inspection
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let selected = descriptor
        .files
        .iter()
        .filter(|file| file.selected)
        .map(|declared| {
            let file = available
                .get(declared.path.as_str())
                .context("selected pack file is missing from the archive")?;
            let (url, alternate_urls) = file
                .urls
                .split_first()
                .context("selected pack file has no download source")?;
            Ok(Resource {
                sha1: file.sha1.clone(),
                sha512: Some(file.sha512.clone()),
                size: file.size,
                url: url.clone(),
                alternate_urls: alternate_urls.to_vec(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    store.fetch_all(&selected, cancel).await?;

    let stage = directory.join(format!("baseline-stage-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage)?;
    for file in descriptor.files.iter().filter(|file| file.selected) {
        ensure!(!cancel.is_cancelled(), "pack installation cancelled");
        super::validate_pack_path(&file.path)?;
        let destination = stage.join(&file.path);
        fs::create_dir_all(destination.parent().context("pack file has no parent")?)?;
        copy_file(&store.object_path(&file.sha1)?, &destination, false, cancel)?;
    }
    copy_overlay(&directory.join("overrides"), &stage, cancel)?;
    ensure!(!cancel.is_cancelled(), "pack installation cancelled");
    let baseline = directory.join("baseline");
    match fs::symlink_metadata(&baseline) {
        Ok(_) => {
            ensure!(
                same_tree(&baseline, &stage)?,
                "existing pack baseline differs; it was preserved"
            );
            fs::remove_dir_all(&stage)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::rename(&stage, &baseline)?;
        }
        Err(error) => return Err(error.into()),
    }
    drop(lock);
    Ok(())
}

fn copy_overlay(
    source: &Path,
    destination: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(source).context("pack overrides are missing")?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "pack overrides are not a plain directory"
    );
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        ensure!(!cancel.is_cancelled(), "pack installation cancelled");
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("pack content filename is not UTF-8")?;
        super::validate_pack_path(name)?;
        let from = entry.path();
        let to = destination.join(name);
        let kind = fs::symlink_metadata(&from)?;
        ensure!(
            !kind.file_type().is_symlink(),
            "pack content contains a link"
        );
        if kind.is_dir() {
            if !to.exists() {
                fs::create_dir(&to)?;
            }
            ensure!(to.is_dir(), "pack content conflicts with a file");
            copy_overlay(&from, &to, cancel)?;
        } else {
            ensure!(kind.is_file(), "pack content contains a special file");
            copy_file(&from, &to, true, cancel)?;
        }
    }
    Ok(())
}

fn copy_file(
    source: &Path,
    destination: &Path,
    replace: bool,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "pack source is not a plain file"
    );
    let mut input = File::open(source)?;
    let mut options = OpenOptions::new();
    options.write(true);
    if replace {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut output = options.open(destination)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        ensure!(!cancel.is_cancelled(), "pack installation cancelled");
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        output.write_all(&buffer[..count])?;
    }
}

fn same_tree(left: &Path, right: &Path) -> anyhow::Result<bool> {
    let left_metadata = fs::symlink_metadata(left)?;
    let right_metadata = fs::symlink_metadata(right)?;
    if left_metadata.file_type().is_symlink() || right_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    if left_metadata.is_file() && right_metadata.is_file() {
        if left_metadata.len() != right_metadata.len() {
            return Ok(false);
        }
        let (mut left, mut right) = (File::open(left)?, File::open(right)?);
        let (mut a, mut b) = ([0_u8; 64 * 1024], [0_u8; 64 * 1024]);
        loop {
            let count = left.read(&mut a)?;
            right.read_exact(&mut b[..count])?;
            if a[..count] != b[..count] {
                return Ok(false);
            }
            if count == 0 {
                return Ok(true);
            }
        }
    }
    if !left_metadata.is_dir() || !right_metadata.is_dir() {
        return Ok(false);
    }
    let mut left_names = fs::read_dir(left)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut right_names = fs::read_dir(right)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    left_names.sort();
    right_names.sort();
    if left_names != right_names {
        return Ok(false);
    }
    for name in left_names {
        if !same_tree(&left.join(&name), &right.join(name))? {
            return Ok(false);
        }
    }
    Ok(true)
}
