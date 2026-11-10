use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use sha1::{Digest as _, Sha1};

use super::{InstallStatus, Instance, Snapshot, TargetSource, validate_version_id};

const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CHAIN_BYTES: usize = 32 * 1024 * 1024;
const MAX_INHERITANCE_DEPTH: usize = 16;

impl super::Catalog {
    /// Record an observation only while the captured Instance still owns it.
    /// # Errors
    /// Rejects superseded observations without recreating or replacing metadata.
    pub(crate) fn record_existing_verification(
        &self,
        expected: &Instance,
        status: InstallStatus,
    ) -> anyhow::Result<()> {
        ensure!(
            matches!(status, InstallStatus::Ready | InstallStatus::Failed { .. }),
            "invalid Existing verification result"
        );
        self.transaction(true, |state| {
            let current = state
                .instances
                .iter_mut()
                .find(|item| item.id == expected.id)
                .context("Existing verification observation was superseded")?;
            ensure!(
                serde_json::to_value(&*current)? == serde_json::to_value(expected)?,
                "Existing verification observation was superseded"
            );
            let target = current
                .targets
                .iter_mut()
                .find(|target| target.id == expected.target_id)
                .context("Existing verification target was superseded")?;
            ensure!(
                matches!(target.source, TargetSource::Existing { .. }),
                "verification requires an Existing target"
            );
            target.status = status;
            Ok(())
        })
    }
}

/// Identified source metadata/resource damage, distinct from launch environment failures.
#[derive(Debug)]
pub(crate) struct ExistingSourceFailure {
    pub(crate) path: PathBuf,
    error: anyhow::Error,
}

impl ExistingSourceFailure {
    /// Persistent status never includes arbitrary metadata values or source URLs.
    pub(crate) fn safe_message(&self, root: &Path) -> String {
        let resource = if self.path == root {
            "source metadata"
        } else if self.path.starts_with(root.join("versions")) {
            if self
                .path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                "version metadata"
            } else {
                "client file"
            }
        } else if self.path.starts_with(root.join("libraries")) {
            "library"
        } else if self.path.starts_with(root.join("assets/indexes")) {
            "asset index"
        } else if self.path.starts_with(root.join("assets")) {
            "asset"
        } else {
            "source resource"
        };
        let cause = match self
            .error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind)
        {
            Some(std::io::ErrorKind::NotFound) => "is missing",
            Some(_) => "could not be read safely",
            None => {
                if self.error.downcast_ref::<serde_json::Error>().is_some() {
                    "contains invalid metadata"
                } else {
                    "changed or is corrupt"
                }
            }
        };
        format!("Existing {resource} {cause}; explicit Repair is required.")
    }

    pub(crate) fn mark(path: &Path, error: anyhow::Error) -> anyhow::Error {
        Self {
            path: path.to_owned(),
            error,
        }
        .into()
    }
}

impl std::fmt::Display for ExistingSourceFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.error)
    }
}

impl std::error::Error for ExistingSourceFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

impl Snapshot {
    /// Hash launch settings and bounded local metadata, without authentication.
    /// # Errors
    /// Rejects missing selections, unsafe files and invalid inheritance chains.
    pub fn launch_fingerprint(&self, instance_id: &str) -> anyhow::Result<String> {
        let (instance, target) = self.selection(instance_id)?;
        instance.validate()?;
        let mut digest = Sha1::new();
        digest.update(serde_json::to_vec(&(instance, target))?);
        match &target.source {
            TargetSource::Managed { manifest } => {
                if let Some(path) = manifest {
                    digest.update(read_manifest(path)?);
                }
            }
            TargetSource::Existing { version_id } => {
                (|| {
                    ensure!(
                        instance.resource_directory.canonicalize()? == instance.resource_directory,
                        "resource directory changed before reading metadata"
                    );
                    let mut current = version_id.clone();
                    let mut visited = BTreeSet::new();
                    let mut total = 0;
                    loop {
                        validate_version_id(&current)?;
                        ensure!(
                            visited.len() < MAX_INHERITANCE_DEPTH
                                && visited.insert(current.clone()),
                            "version inheritance is cyclic or too deep"
                        );
                        let path = instance
                            .resource_directory
                            .join("versions")
                            .join(&current)
                            .join(format!("{current}.json"));
                        ensure!(
                            path.canonicalize().with_context(|| format!(
                                "Missing or unsafe existing version metadata: {}",
                                path.display()
                            ))? == path,
                            "version metadata path contains an alias"
                        );
                        let bytes = read_manifest(&path).with_context(|| {
                            format!("Cannot read existing version metadata: {}", path.display())
                        })?;
                        total += bytes.len();
                        ensure!(total <= MAX_CHAIN_BYTES, "version inheritance is too large");
                        digest.update(serde_json::to_vec(&current)?);
                        digest.update(Sha1::digest(&bytes));
                        let value: serde_json::Value = serde_json::from_slice(&bytes)
                            .with_context(|| {
                                format!("invalid selected version metadata: {}", path.display())
                            })?;
                        ensure!(value.is_object(), "version metadata must be an object");
                        if let Some(id) = value.get("id") {
                            ensure!(id.as_str() == Some(current.as_str()), "version ID mismatch");
                        }
                        match value.get("inheritsFrom") {
                            None => break,
                            Some(parent) => {
                                current = parent
                                    .as_str()
                                    .context("version parent must be a concrete ID")?
                                    .to_owned();
                            }
                        }
                    }
                    Ok(())
                })()
                .map_err(|error| {
                    ExistingSourceFailure::mark(&instance.resource_directory, error)
                })?;
            }
        }
        Ok(format!("{:x}", digest.finalize()))
    }
}

pub(super) fn read_manifest(path: &Path) -> anyhow::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "metadata must be a regular file"
    );
    ensure!(
        metadata.len() <= MAX_MANIFEST_BYTES,
        "metadata is too large"
    );
    let file = File::open(path)?;
    ensure!(
        super::game_directory::same_file(&metadata, &file.metadata()?),
        "metadata changed while opening"
    );
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        u64::try_from(bytes.len())? <= MAX_MANIFEST_BYTES,
        "metadata grew while reading"
    );
    ensure!(
        super::game_directory::same_file(&metadata, &fs::symlink_metadata(path)?),
        "metadata changed while reading"
    );
    Ok(bytes)
}
