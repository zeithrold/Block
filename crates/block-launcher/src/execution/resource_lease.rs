//! A cache-wide reader/writer lease held through preparation and Java exit.

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

use anyhow::{Context as _, ensure};

#[derive(Debug)]
pub(crate) struct ResourceLease(File);

#[derive(Clone, Copy)]
enum Access {
    Shared,
    Exclusive,
}

impl ResourceLease {
    pub(crate) fn shared(root: &Path) -> anyhow::Result<Self> {
        Self::acquire(root, Access::Shared)
    }

    pub(crate) fn exclusive(root: &Path) -> anyhow::Result<Self> {
        Self::acquire(root, Access::Exclusive)
    }

    fn acquire(root: &Path, access: Access) -> anyhow::Result<Self> {
        let root = root
            .canonicalize()
            .context("cache directory is unavailable")?;
        let path = root.join("resources.lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "cache resource lock is not a regular file"
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect cache resource lock"),
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .context("cannot open cache resource lock")?;
        match access {
            Access::Shared => fs2::FileExt::try_lock_shared(&file).context(
                "shared resources are being installed or repaired; retry after completion",
            )?,
            Access::Exclusive => fs2::FileExt::try_lock_exclusive(&file).context(
                "shared resources are in use; stop running games before installing or repairing",
            )?,
        }
        Ok(Self(file))
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        // Release ownership now, even if a concurrently spawned process briefly
        // inherited this open description before its exec closes the descriptor.
        if let Err(error) = fs2::FileExt::unlock(&self.0) {
            tracing::warn!(event = "cache_resource_unlock_failed", %error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ResourceLease;

    #[test]
    fn running_readers_share_resources_and_exclude_repair() -> anyhow::Result<()> {
        let cache = tempfile::tempdir()?;
        let first = ResourceLease::shared(cache.path())?;
        let second = ResourceLease::shared(cache.path())?;
        assert!(ResourceLease::exclusive(cache.path()).is_err());
        drop(first);
        assert!(ResourceLease::exclusive(cache.path()).is_err());
        drop(second);
        assert!(ResourceLease::exclusive(cache.path()).is_ok());
        Ok(())
    }

    #[test]
    fn repair_excludes_readers_and_other_writers_until_drop() -> anyhow::Result<()> {
        let cache = tempfile::tempdir()?;
        let repair = ResourceLease::exclusive(cache.path())?;
        assert!(ResourceLease::shared(cache.path()).is_err());
        assert!(ResourceLease::exclusive(cache.path()).is_err());
        drop(repair);
        assert!(ResourceLease::shared(cache.path()).is_ok());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn cache_aliases_have_one_resource_lock_identity() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let cache = root.path().join("cache");
        let alias = root.path().join("alias");
        std::fs::create_dir(&cache)?;
        std::os::unix::fs::symlink(&cache, &alias)?;
        let _reader = ResourceLease::shared(&cache)?;
        assert!(ResourceLease::exclusive(&alias).is_err());
        assert!(ResourceLease::shared(&alias).is_ok());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn resource_lock_symlink_is_rejected_without_touching_target() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let target = root.path().join("user-data");
        let original = b"preserve this file";
        std::fs::write(&target, original)?;
        std::os::unix::fs::symlink(&target, root.path().join("resources.lock"))?;
        assert!(ResourceLease::shared(root.path()).is_err());
        assert!(ResourceLease::exclusive(root.path()).is_err());
        assert_eq!(std::fs::read(target)?, original);
        Ok(())
    }
}
