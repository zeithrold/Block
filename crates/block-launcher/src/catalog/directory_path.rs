use super::validate_directory;
use anyhow::{Context as _, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Canonicalize existing directories without materializing a planned directory.
pub(crate) fn normalize_directory(path: &Path) -> anyhow::Result<PathBuf> {
    validate_directory(path)?;
    let mut missing = Vec::new();
    let mut ancestor = path;
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let canonical = ancestor.canonicalize()?;
                ensure!(canonical.is_dir(), "selected path is not a directory");
                let mut normalized = canonical;
                for component in missing.iter().rev() {
                    normalized.push(component);
                }
                return Ok(normalized);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .context("directory has no final component")?
                        .to_owned(),
                );
                ancestor = ancestor
                    .parent()
                    .context("directory has no existing ancestor")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
