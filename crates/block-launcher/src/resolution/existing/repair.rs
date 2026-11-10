use std::{fs, path::Path};

use anyhow::{Context as _, ensure};

use super::{
    ExistingReceipt, ExistingReceiptEntry, Instance, LaunchTarget, ResolveContext,
    ResolvedMetadata, TargetSource, manifests, read_receipt,
    resources::{self, Inputs, Purpose},
};
use crate::plan::{ExistingResource, ResourcePath};

/// Repair declared immutable inputs into a fresh cache view, preserving all source bytes.
/// The caller holds the exclusive cache resource lease throughout this operation.
pub(crate) async fn repair_existing(
    instance: &Instance,
    selected: &LaunchTarget,
    context: &ResolveContext,
) -> anyhow::Result<ResolvedMetadata> {
    instance.validate()?;
    ensure!(!context.cancel.is_cancelled(), "Existing Repair cancelled");
    let TargetSource::Existing { version_id } = &selected.source else {
        anyhow::bail!("Existing Repair requires an Existing target");
    };
    let root = &instance.resource_directory;
    ensure!(
        root.is_dir() && root.canonicalize()? == *root,
        "Existing resource directory changed"
    );
    let chain = manifests::read_chain(root, version_id)?;
    let cache = context.store.root().canonicalize()?;
    let previous = read_receipt(&cache, instance, selected, &chain.digest)?;
    let mut inputs = Inputs::new(root, &cache, previous.clone());
    inputs.purpose = Purpose::Repair;
    let target = crate::platform::Target::host()?;
    let mut manifest = manifests::normalize(chain.value, &chain.client_id, &target, &mut inputs)?;
    let destination = crate::executor::confined_path(
        &cache,
        &format!(
            "installations/{}/{}/repairs/{}",
            instance.id,
            selected.id,
            uuid::Uuid::new_v4()
        ),
    )?;

    // The index is executable metadata for the asset layout. Repair and verify
    // it before using its object names or hashes to describe further inputs.
    resources::asset_index(&manifest, &mut inputs)?;
    repair_pending(&mut inputs, &destination, context).await?;
    resources::assets(&manifest, &mut inputs)?;
    repair_pending(&mut inputs, &destination, context).await?;

    let asset_root = inputs
        .asset_root
        .as_ref()
        .context("Repair has no asset root")?;
    let layout_entries = inputs
        .entries
        .values()
        .filter(|entry| is_asset_layout(&entry.logical_path))
        .cloned()
        .collect::<Vec<_>>();
    let mut split_layout = false;
    for entry in &layout_entries {
        let relative = entry
            .logical_path
            .strip_prefix("assets/")
            .context("Asset logical path missing")?;
        split_layout |= entry.path.resolve(&cache)? != asset_root.join(relative);
    }
    if split_layout {
        // An assets root is one complete verified tree. A replacement object
        // cannot be combined with an unrelated original index through literals.
        for entry in layout_entries {
            ensure!(!context.cancel.is_cancelled(), "Existing Repair cancelled");
            let source = ExistingResource {
                path: entry.path.resolve(&cache)?,
                sha1: entry.sha1.clone(),
                size: entry.size,
            };
            let repaired = materialize(&cache, &destination, &entry.logical_path, &source)?;
            inputs.entries.insert(entry.logical_path, repaired);
        }
    }
    super::arguments::normalize(&mut manifest, &target, &mut inputs)?;
    ensure!(
        inputs.pending.is_empty(),
        "Source resources changed during Repair; retry after the external writer has stopped"
    );
    let mut entries: std::collections::BTreeMap<_, _> = previous
        .into_iter()
        .flat_map(|receipt| receipt.entries)
        .map(|entry| (entry.logical_path.clone(), entry))
        .collect();
    entries.extend(inputs.entries);
    let receipt = ExistingReceipt {
        schema_version: 1,
        source_metadata_sha1: chain.digest,
        entries: entries.into_values().collect(),
    };
    ensure!(!context.cancel.is_cancelled(), "Existing Repair cancelled");
    // Return only the ordinary strict resolver's verified result. A failed
    // repair never produces a launchable metadata value or publishes a receipt.
    super::resolve_inner(instance, selected, context, Some(receipt)).await
}

fn is_asset_layout(logical: &str) -> bool {
    ["assets/indexes/", "assets/objects/", "assets/virtual/"]
        .iter()
        .any(|prefix| logical.starts_with(prefix))
}

async fn repair_pending(
    inputs: &mut Inputs<'_>,
    destination: &Path,
    context: &ResolveContext,
) -> anyhow::Result<()> {
    for logical in std::mem::take(&mut inputs.pending) {
        ensure!(!context.cancel.is_cancelled(), "Existing Repair cancelled");
        let descriptor = inputs
            .upstream
            .get(&logical)
            .context("Repair has no trustworthy upstream descriptor")?;
        let path = context
            .store
            .fetch(descriptor, &context.cancel)
            .await?
            .canonicalize()?;
        ensure!(
            path.starts_with(inputs.cache),
            "Downloaded repair resource escaped the cache"
        );
        let source = ExistingResource {
            path,
            sha1: descriptor.sha1.clone(),
            size: descriptor.size,
        };
        let repaired = materialize(inputs.cache, destination, &logical, &source)?;
        inputs.entries.insert(logical, repaired);
    }
    Ok(())
}

fn materialize(
    cache: &Path,
    destination: &Path,
    logical: &str,
    source: &ExistingResource,
) -> anyhow::Result<ExistingReceiptEntry> {
    source.verify()?;
    let path = crate::executor::confined_path(destination, logical)?;
    crate::executor::ensure_write_path(cache, &path)?;
    let parent = path.parent().context("Repair destination has no parent")?;
    fs::create_dir_all(parent)?;
    crate::executor::ensure_write_path(cache, &path)?;
    if path != source.path {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut input = fs::File::open(&source.path)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
    }
    let resource = ExistingResource {
        path: path.clone(),
        sha1: source.sha1.clone(),
        size: source.size,
    };
    resource.verify()?;
    Ok(ExistingReceiptEntry {
        logical_path: logical.into(),
        sha1: source.sha1.clone(),
        size: source.size,
        path: ResourcePath::Cache {
            relative: path
                .strip_prefix(cache)?
                .to_str()
                .context("Repair cache path is not UTF-8")?
                .replace('\\', "/"),
        },
    })
}
