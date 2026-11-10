use anyhow::{Context as _, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Only empty directories created by this operation are eligible for rollback.
/// A concurrent writer makes `remove_dir` fail safely rather than losing its data.
#[derive(Debug)]
pub(super) struct PreparedInstanceDirectory {
    created: Vec<PathBuf>,
    pub(super) committed: bool,
}

impl PreparedInstanceDirectory {
    pub(super) fn new(directory: &Path) -> anyhow::Result<Self> {
        let mut missing = Vec::new();
        let mut ancestor = directory;
        loop {
            match fs::metadata(ancestor) {
                Ok(metadata) => {
                    ensure!(metadata.is_dir(), "installation path is not a directory");
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing.push(ancestor.to_path_buf());
                    ancestor = ancestor
                        .parent()
                        .context("installation path has no parent")?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        let mut prepared = Self {
            created: Vec::new(),
            committed: false,
        };
        for path in missing.into_iter().rev() {
            match fs::create_dir(&path) {
                Ok(()) => prepared.created.push(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    ensure!(path.is_dir(), "installation path is not a directory");
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(prepared)
    }
}

impl Drop for PreparedInstanceDirectory {
    fn drop(&mut self) {
        if !self.committed {
            for path in self.created.iter().rev() {
                let _ = fs::remove_dir(path);
            }
        }
    }
}
