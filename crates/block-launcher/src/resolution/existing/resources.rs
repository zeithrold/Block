use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};

use super::{ExistingReceipt, ExistingReceiptEntry};
use crate::{
    manifest::{model::VersionManifest, resources::AssetObjects},
    plan::{ExistingResource, ResourcePath},
};

pub(super) struct Inputs<'a> {
    pub root: &'a Path,
    pub cache: &'a Path,
    previous: BTreeMap<String, ExistingReceiptEntry>,
    pub entries: BTreeMap<String, ExistingReceiptEntry>,
    declared_paths: BTreeSet<PathBuf>,
    pub resources: Vec<ExistingResource>,
    pub paths: BTreeMap<String, ResourcePath>,
    pub asset_root: Option<PathBuf>,
    pub purpose: Purpose,
    pub upstream: BTreeMap<String, block_download::Resource>,
    pub pending: BTreeSet<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    Launch,
    Repair,
}

impl<'a> Inputs<'a> {
    pub(super) fn new(root: &'a Path, cache: &'a Path, previous: Option<ExistingReceipt>) -> Self {
        Self {
            root,
            cache,
            previous: previous
                .map(|receipt| {
                    receipt
                        .entries
                        .into_iter()
                        .map(|entry| (entry.logical_path.clone(), entry))
                        .collect()
                })
                .unwrap_or_default(),
            entries: BTreeMap::new(),
            declared_paths: BTreeSet::new(),
            resources: Vec::new(),
            paths: BTreeMap::new(),
            asset_root: None,
            purpose: Purpose::Launch,
            upstream: BTreeMap::new(),
            pending: BTreeSet::new(),
        }
    }

    pub(super) fn declare(&mut self, logical: &str, resource: block_download::Resource) {
        if resource.sha1.len() == 40
            && resource.sha1.bytes().all(|byte| byte.is_ascii_hexdigit())
            && reqwest::Url::parse(&resource.url).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
            })
        {
            self.upstream.insert(logical.to_owned(), resource);
        }
    }

    pub(super) fn select(
        &mut self,
        logical: &str,
        expected: Option<(&str, u64)>,
    ) -> anyhow::Result<ExistingResource> {
        let original = crate::executor::confined_path(self.root, logical)?;
        let previous = self
            .entries
            .get(logical)
            .or_else(|| self.previous.get(logical));
        let previous = previous.filter(|entry| {
            expected.is_none_or(|(sha1, size)| {
                entry.sha1.eq_ignore_ascii_case(sha1) && entry.size == size
            })
        });
        let path = match previous.map(|entry| &entry.path) {
            Some(ResourcePath::Cache { relative }) => {
                crate::executor::confined_path(self.cache, relative)?
            }
            Some(ResourcePath::Existing { path }) => {
                ensure!(
                    *path == original,
                    "Existing receipt redirects source resource: {logical}"
                );
                original
            }
            None => original,
        };
        let expected = expected.or_else(|| previous.map(|entry| (entry.sha1.as_str(), entry.size)));
        let resource = match expected {
            Some((sha1, size)) => {
                let resource = ExistingResource {
                    path,
                    sha1: sha1.to_owned(),
                    size,
                };
                if let Err(error) = resource.verify() {
                    match self.purpose {
                        Purpose::Launch => return Err(error),
                        Purpose::Repair => {
                            ensure!(self.upstream.get(logical).is_some_and(|upstream| upstream.sha1.eq_ignore_ascii_case(&resource.sha1) && upstream.size == resource.size),
                                "Cannot repair {logical}: no trustworthy declared upstream descriptor; source bytes are preserved");
                            self.pending.insert(logical.to_owned());
                        }
                    }
                }
                resource
            }
            None => ExistingResource::capture(path).with_context(|| format!("Cannot resolve {logical}: no trustworthy upstream descriptor is available for repairing a missing local-only resource"))?,
        };
        let selection = match previous.map(|entry| &entry.path) {
            Some(ResourcePath::Cache { relative }) => ResourcePath::Cache {
                relative: relative.clone(),
            },
            Some(ResourcePath::Existing { .. }) | None => ResourcePath::Existing {
                path: resource.path.clone(),
            },
        };
        self.paths.insert(logical.to_owned(), selection.clone());
        self.entries.insert(
            logical.into(),
            ExistingReceiptEntry {
                logical_path: logical.into(),
                sha1: resource.sha1.clone(),
                size: resource.size,
                path: selection,
            },
        );
        if self.declared_paths.insert(resource.path.clone()) {
            self.resources.push(resource.clone());
        }
        Ok(resource)
    }
}

pub(super) fn asset_index(
    manifest: &VersionManifest,
    inputs: &mut Inputs<'_>,
) -> anyhow::Result<ExistingResource> {
    let index = &manifest.asset_index;
    crate::executor::confined_path(inputs.root, &index.id)?;
    ensure!(!index.id.contains('/'), "Unsafe asset index ID");
    inputs.declare(
        &format!("assets/indexes/{}.json", index.id),
        block_download::Resource {
            sha1: index.sha1.clone(),
            size: index.size,
            url: index.url.clone(),
            sha512: None,
            alternate_urls: Vec::new(),
        },
    );
    let resource = inputs.select(
        &format!("assets/indexes/{}.json", index.id),
        Some((&index.sha1, index.size)),
    )?;
    ensure!(
        resource.size <= 64 * 1024 * 1024,
        "Existing asset index is too large"
    );
    Ok(resource)
}

pub(super) fn assets(
    manifest: &VersionManifest,
    inputs: &mut Inputs<'_>,
) -> anyhow::Result<AssetObjects> {
    let index = &manifest.asset_index;
    let resource = asset_index(manifest, inputs)?;
    let root = resource
        .path
        .parent()
        .and_then(Path::parent)
        .context("Asset index has no root")?
        .to_owned();
    let assets: AssetObjects = crate::manifest::decode(&resource.read_verified(64 * 1024 * 1024)?)
        .map_err(|error| super::ExistingSourceFailure::mark(&resource.path, error))?;
    for (name, asset) in &assets.objects {
        ensure!(
            asset.hash.len() == 40 && asset.hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            super::ExistingSourceFailure::mark(
                &resource.path,
                anyhow::anyhow!("Invalid existing asset hash")
            )
        );
        let relative = format!("objects/{}/{}", &asset.hash[..2], asset.hash);
        let descriptor = block_download::Resource {
            sha1: asset.hash.clone(),
            size: asset.size,
            url: format!(
                "https://resources.download.minecraft.net/{}/{}",
                &asset.hash[..2],
                asset.hash
            ),
            sha512: None,
            alternate_urls: Vec::new(),
        };
        inputs.declare(&format!("assets/{relative}"), descriptor.clone());
        let object = inputs.select(
            &format!("assets/{relative}"),
            Some((&asset.hash, asset.size)),
        )?;
        ensure!(
            inputs.purpose == Purpose::Repair || object.path == root.join(&relative),
            "Existing assets require one complete validated asset root; explicit Repair is required"
        );
        if assets.virtual_layout == Some(true) {
            crate::executor::confined_path(&root, name)?;
            let relative = format!("virtual/{}/{name}", index.id);
            inputs.declare(&format!("assets/{relative}"), descriptor);
            let object = inputs.select(
                &format!("assets/{relative}"),
                Some((&asset.hash, asset.size)),
            )?;
            ensure!(
                inputs.purpose == Purpose::Repair || object.path == root.join(relative),
                "Existing virtual asset layout is incomplete"
            );
        }
    }
    inputs.asset_root = Some(root);
    Ok(assets)
}
