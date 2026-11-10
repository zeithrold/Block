//! Read-only normalization of local launcher versions into the shared compiler.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};

use super::{ResolveContext, ResolvedMetadata};
use crate::{
    catalog::{Instance, LaunchTarget, TargetSource},
    plan::{ExistingResource, ResourcePath},
    recipe::{LaunchRecipe, RecipeSource, RuntimeSelection},
    session::OfflineSession,
};

#[path = "existing/arguments.rs"]
mod arguments;
#[path = "existing/manifests.rs"]
mod manifests;
#[path = "existing/repair.rs"]
mod repair;
#[path = "existing/resources.rs"]
mod resources;
pub(crate) use repair::repair_existing;

use crate::catalog::ExistingSourceFailure;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingReceiptEntry {
    pub logical_path: String,
    pub sha1: String,
    pub size: u64,
    pub path: ResourcePath,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingReceipt {
    pub schema_version: u32,
    pub source_metadata_sha1: String,
    pub entries: Vec<ExistingReceiptEntry>,
}

#[derive(Debug, Clone)]
pub struct ExistingInstallation {
    pub receipt: ExistingReceipt,
    pub resources: Vec<ExistingResource>,
    pub asset_root: PathBuf,
    pub client_path: String,
    pub resource_paths: BTreeMap<String, ResourcePath>,
    pub argument_paths: BTreeMap<String, ResourcePath>,
}

pub(crate) fn existing_recipe(instance: &Instance) -> LaunchRecipe {
    LaunchRecipe {
        schema_version: 1,
        instance: instance.id.clone(),
        source: RecipeSource::LocalManifest {
            path: instance.resource_directory.clone(),
        },
        session: OfflineSession::Offline {
            username: "Player".into(),
        },
        runtime: instance.runtime.clone(),
        target: None,
        compatibility: instance.compatibility,
        memory_mib: instance.memory_mib,
        jvm_args: instance.jvm_args.clone(),
        features: BTreeMap::new(),
    }
}

pub(crate) async fn resolve_existing(
    instance: &Instance,
    selected: &LaunchTarget,
    context: &ResolveContext,
) -> anyhow::Result<ResolvedMetadata> {
    tokio::select! {
        biased;
        () = context.cancel.cancelled() => anyhow::bail!("Existing installation resolution cancelled"),
        result = resolve_inner(instance, selected, context, None) => result,
    }
}

async fn resolve_inner(
    instance: &Instance,
    selected: &LaunchTarget,
    context: &ResolveContext,
    receipt: Option<ExistingReceipt>,
) -> anyhow::Result<ResolvedMetadata> {
    instance.validate()?;
    let TargetSource::Existing { version_id } = &selected.source else {
        anyhow::bail!("Existing resolver requires an Existing target");
    };
    let root = &instance.resource_directory;
    (|| {
        ensure!(
            root.is_dir() && root.canonicalize()? == *root,
            "Existing resource directory changed"
        );
        Ok(())
    })()
    .map_err(|error| ExistingSourceFailure::mark(root, error))?;
    let chain = manifests::read_chain(root, version_id)?;
    let cache = context.store.root().canonicalize()?;
    let previous = match receipt {
        Some(receipt) => {
            ensure!(
                receipt.source_metadata_sha1 == chain.digest,
                "Existing metadata changed during Repair"
            );
            Some(receipt)
        }
        None => read_receipt(&cache, instance, selected, &chain.digest)?,
    };
    let recipe = existing_recipe(instance);
    let target = crate::platform::Target::host()?;
    let mut inputs = resources::Inputs::new(root, &cache, previous);
    inputs.resources.extend(chain.files.iter().cloned());
    let mut manifest = manifests::normalize(chain.value, &chain.client_id, &target, &mut inputs)?;
    ensure!(
        crate::game_import::is_known_main_class(&manifest.main_class),
        "Unsupported existing launch main class: {}",
        manifest.main_class
    );
    crate::support::manifest_support(&manifest, &target, &recipe.features)?.require()?;
    let assets = resources::assets(&manifest, &mut inputs)?;
    let argument_paths = arguments::normalize(&mut manifest, &target, &mut inputs)?;
    let parsed = crate::recipe::ParsedRecipe {
        manifest,
        manifest_sha1: chain.digest.clone(),
    };
    let (library_overrides, classifier_overrides, companion_libraries, compatibility) =
        super::compatibility_overrides(&recipe, &parsed, &target, context).await?;
    let runtime = match instance.runtime {
        RuntimeSelection::Managed => {
            Some(super::resolve_managed_runtime(&parsed.manifest, &target, context).await?)
        }
        RuntimeSelection::Local { .. } => None,
    };
    let asset_root = inputs
        .asset_root
        .take()
        .context("Missing validated existing assets root")?;
    Ok(ResolvedMetadata {
        parsed,
        target,
        assets,
        runtime,
        library_overrides,
        classifier_overrides,
        companion_libraries,
        compatibility,
        existing: Some(ExistingInstallation {
            receipt: ExistingReceipt {
                schema_version: 1,
                source_metadata_sha1: chain.digest,
                entries: inputs.entries.into_values().collect(),
            },
            resources: inputs.resources,
            asset_root,
            client_path: format!("versions/{0}/{0}.jar", chain.client_id),
            resource_paths: inputs.paths,
            argument_paths,
        }),
    })
}

fn read_receipt(
    cache: &Path,
    instance: &Instance,
    target: &LaunchTarget,
    digest: &str,
) -> anyhow::Result<Option<ExistingReceipt>> {
    let path = crate::executor::confined_path(
        cache,
        &format!("installations/{}/{}/existing.json", instance.id, target.id),
    )?;
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Could not read existing resource receipt"),
    };
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= 32 * 1024 * 1024,
        "Unsafe or oversized existing receipt"
    );
    let bytes = ExistingResource::capture(path.clone())?.read_verified(32 * 1024 * 1024)?;
    ensure!(
        bytes.len() <= 32 * 1024 * 1024 && path.canonicalize()? == path,
        "Unsafe or oversized existing receipt"
    );
    let receipt: ExistingReceipt = serde_json::from_slice(&bytes)?;
    ensure!(
        receipt.schema_version == 1,
        "Unsupported existing receipt schema"
    );
    Ok((receipt.source_metadata_sha1 == digest).then_some(receipt))
}

#[cfg(test)]
#[path = "existing/tests.rs"]
mod tests;
