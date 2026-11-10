use std::{
    fs::{self, File},
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

impl super::ExecutionPlan {
    /// Independently validate all declared source files and the complete asset tree.
    /// # Errors
    /// Rejects undeclared asset indexes/objects, changed files and unsafe paths.
    pub fn verify_existing_resources(&self) -> anyhow::Result<()> {
        let mut declared = std::collections::BTreeMap::new();
        for resource in &self.existing_resources {
            resource.verify()?;
            if let Some(previous) = declared.insert(&resource.path, resource) {
                ensure!(
                    previous == resource,
                    "Conflicting existing resource descriptors"
                );
            }
        }
        let Some(root) = &self.existing_asset_root else {
            return Ok(());
        };
        ensure!(
            root.is_dir() && root.canonicalize()? == *root,
            "Existing asset root changed"
        );
        let id = self
            .command
            .variables
            .get("assets_index_name")
            .context("Missing source asset index ID")?;
        crate::executor::confined_path(root, id)?;
        ensure!(!id.contains('/'), "Invalid source asset index ID");
        let index_path = root.join("indexes").join(format!("{id}.json"));
        let index = declared
            .get(&index_path)
            .context("Existing asset index is not declared")?;
        ensure!(index.size <= 64 * 1024 * 1024, "Asset index is too large");
        let assets: crate::manifest::resources::AssetObjects =
            crate::manifest::decode(&index.read_verified(64 * 1024 * 1024)?)?;
        for (name, asset) in assets.objects {
            ensure!(
                asset.hash.len() == 40 && asset.hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Invalid asset object hash"
            );
            let path = root
                .join("objects")
                .join(&asset.hash[..2])
                .join(&asset.hash);
            ensure!(
                declared.get(&path).is_some_and(
                    |resource| resource.sha1 == asset.hash && resource.size == asset.size
                ),
                "Existing asset object is not declared: {}",
                path.display()
            );
            if assets.virtual_layout == Some(true) {
                let named = crate::executor::confined_path(&root.join("virtual").join(id), &name)?;
                ensure!(
                    declared.get(&named).is_some_and(
                        |resource| resource.sha1 == asset.hash && resource.size == asset.size
                    ),
                    "Existing virtual asset is not declared: {}",
                    named.display()
                );
            }
        }
        Ok(())
    }
}

/// A resource input belongs either to Block's cache or to a declared source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourcePath {
    Cache { relative: String },
    Existing { path: PathBuf },
}

impl ResourcePath {
    /// Resolve a declared input after its plan's integrity preflight.
    /// # Errors
    /// Rejects unsafe cache-relative paths and replaced existing path identities.
    pub fn resolve(&self, cache: &Path) -> anyhow::Result<PathBuf> {
        match self {
            Self::Cache { relative } => crate::executor::confined_path(cache, relative),
            Self::Existing { path } => {
                ensure!(
                    path.is_absolute() && path.canonicalize()? == *path,
                    "Existing resource path changed"
                );
                Ok(path.clone())
            }
        }
    }
}

/// A content-pinned, read-only file in an existing installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingResource {
    pub path: PathBuf,
    pub sha1: String,
    pub size: u64,
}

impl ExistingResource {
    /// Verify the declared source identity and content without changing it.
    /// # Errors
    /// Rejects aliases, non-regular files, replacement during reading, and size/hash changes.
    pub fn verify(&self) -> anyhow::Result<()> {
        (|| {
            ensure!(
                self.sha1.len() == 40 && self.sha1.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Invalid source SHA-1: {}",
                self.path.display()
            );
            let size = fs::metadata(&self.path)
                .with_context(|| {
                    format!(
                        "Missing existing resource {}; explicit Repair is required",
                        self.path.display()
                    )
                })?
                .len();
            ensure!(
                size == self.size,
                "Existing resource changed or is corrupt: {}; explicit Repair is required",
                self.path.display()
            );
            let (sha1, size) = digest_file(&self.path)?;
            ensure!(
                size == self.size && sha1.eq_ignore_ascii_case(&self.sha1),
                "Existing resource changed or is corrupt: {}; explicit Repair is required",
                self.path.display()
            );
            Ok(())
        })()
        .map_err(|error| crate::catalog::ExistingSourceFailure::mark(&self.path, error))
    }

    pub(crate) fn capture(path: PathBuf) -> anyhow::Result<Self> {
        let (sha1, size) = digest_file(&path)
            .map_err(|error| crate::catalog::ExistingSourceFailure::mark(&path, error))?;
        Ok(Self { path, sha1, size })
    }

    pub(crate) fn read_verified(&self, limit: u64) -> anyhow::Result<Vec<u8>> {
        (|| {
            ensure!(self.size <= limit, "Source metadata exceeds its size limit");
            let metadata = fs::symlink_metadata(&self.path)?;
            ensure!(
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && self.path.canonicalize()? == self.path,
                "Source metadata path changed"
            );
            let file = File::open(&self.path)?;
            ensure!(
                same_file(&metadata, &file.metadata()?),
                "Source metadata changed while opening"
            );
            let mut bytes = Vec::new();
            file.take(
                limit
                    .checked_add(1)
                    .context("Metadata read limit overflow")?,
            )
            .read_to_end(&mut bytes)?;
            ensure!(
                u64::try_from(bytes.len())? == self.size
                    && format!("{:x}", Sha1::digest(&bytes)).eq_ignore_ascii_case(&self.sha1)
                    && same_file(&metadata, &fs::symlink_metadata(&self.path)?)
                    && self.path.canonicalize()? == self.path,
                "Source metadata changed while reading"
            );
            Ok(bytes)
        })()
        .map_err(|error| crate::catalog::ExistingSourceFailure::mark(&self.path, error))
    }
}

fn digest_file(path: &Path) -> anyhow::Result<(String, u64)> {
    ensure!(path.is_absolute(), "Source resource path must be absolute");
    let metadata = fs::symlink_metadata(path).with_context(|| {
        format!(
            "Missing existing resource {}; explicit Repair is required",
            path.display()
        )
    })?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && path.canonicalize()? == path,
        "Source resource is not a canonical regular file: {}",
        path.display()
    );
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    ensure!(
        same_file(&metadata, &opened),
        "Source resource changed while opening: {}",
        path.display()
    );
    let mut digest = Sha1::new();
    let mut size = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(u64::try_from(read)?)
            .context("Source size overflow")?;
        digest.update(&buffer[..read]);
    }
    ensure!(
        size == metadata.len()
            && same_file(&opened, &file.metadata()?)
            && same_file(&opened, &fs::symlink_metadata(path)?)
            && path.canonicalize()? == path,
        "Source resource changed while reading: {}",
        path.display()
    );
    Ok((format!("{:x}", digest.finalize()), size))
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if left.dev() != right.dev() || left.ino() != right.ino() {
            return false;
        }
    }
    left.is_file() == right.is_file()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
}
