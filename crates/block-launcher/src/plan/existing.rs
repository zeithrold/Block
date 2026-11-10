use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context as _;

use super::{ExecutionPlan, PrepareAction, ResourcePath};
use crate::{catalog::Instance, resolver::ResolvedMetadata};

/// Reuse the ordinary compiler, then bind its immutable inputs to verified sources.
pub(crate) fn compile_existing(
    instance: &Instance,
    metadata: &ResolvedMetadata,
) -> anyhow::Result<ExecutionPlan> {
    let existing = metadata
        .existing
        .as_ref()
        .context("Existing metadata is missing")?;
    let recipe = crate::resolver::existing_recipe(instance);
    let mut plan = super::compile_catalog(&recipe, metadata)?;
    let manifest = &metadata.parsed.manifest;
    let library_root = format!("views/libraries/{}", metadata.parsed.manifest_sha1);
    let asset_root = format!("views/assets/{}", manifest.asset_index.sha1);
    let mut mappings = BTreeMap::new();
    let mut classpath_index = 0;
    for library in &manifest.libraries {
        if !crate::rules::allowed(library.rules.as_deref(), &metadata.target, &recipe.features)? {
            continue;
        }
        if let Some(artifact) = &library.downloads.artifact {
            if !metadata.library_overrides.contains_key(&library.name) {
                let logical = format!("libraries/{}", artifact.path);
                let source = existing
                    .resource_paths
                    .get(&logical)
                    .context("Existing library selection is missing")?;
                let filename = artifact
                    .path
                    .rsplit('/')
                    .next()
                    .context("Existing library filename is missing")?;
                mappings.insert(
                    format!("{library_root}/{}-{filename}", artifact.sha1),
                    source.clone(),
                );
                *plan
                    .command
                    .classpath
                    .get_mut(classpath_index)
                    .context("Compiled library order changed")? = source.clone();
            }
            classpath_index += 1;
        }
    }
    let client = existing
        .resource_paths
        .get(&existing.client_path)
        .context("Existing client selection is missing")?;
    *plan
        .command
        .classpath
        .last_mut()
        .context("Compiled client is missing")? = client.clone();
    mappings.insert(
        format!(
            "views/clients/{}/client.jar",
            manifest.downloads.client.sha1
        ),
        client.clone(),
    );
    for (logical, source) in &existing.resource_paths {
        if let Some(relative) = logical.strip_prefix("assets/")
            && !relative.starts_with("log_configs/")
        {
            mappings.insert(format!("{asset_root}/{relative}"), source.clone());
        }
    }
    if let Some(logging) = &manifest.logging {
        let file = &logging.client.file;
        let source = existing
            .resource_paths
            .get(&format!("assets/log_configs/{}", file.id))
            .context("Existing logging selection is missing")?;
        mappings.insert(
            format!("views/logging/{}/{}", file.sha1, file.id),
            source.clone(),
        );
    }
    let mut actions = Vec::new();
    for action in plan.actions {
        match action {
            PrepareAction::Copy { ref path, .. } => {
                if !mappings.contains_key(path) {
                    actions.push(action);
                }
            }
            PrepareAction::Extract {
                ref sha1,
                directory,
                excludes,
            } => {
                if let Some(resource) = existing
                    .resources
                    .iter()
                    .find(|resource| resource.sha1 == *sha1)
                {
                    actions.push(PrepareAction::ExtractExisting {
                        resource: resource.clone(),
                        directory,
                        excludes,
                    });
                } else {
                    actions.push(PrepareAction::Extract {
                        sha1: sha1.clone(),
                        directory,
                        excludes,
                    });
                }
            }
            PrepareAction::GameResource {
                source,
                sha1,
                size,
                path,
            } => {
                actions.push(PrepareAction::GameResource {
                    source: existing
                        .resource_paths
                        .get(&format!("assets/objects/{}/{}", &sha1[..2], sha1))
                        .cloned()
                        .unwrap_or(source),
                    sha1,
                    size,
                    path,
                });
            }
            PrepareAction::Directory { .. }
            | PrepareAction::Link { .. }
            | PrepareAction::ExtractArchive { .. }
            | PrepareAction::ExtractExisting { .. } => actions.push(action),
        }
    }
    for path in &mut plan.command.classpath {
        if let ResourcePath::Cache { relative } = path
            && let Some(source) = mappings.get(relative)
        {
            *path = source.clone();
        }
    }
    for path in plan.command.path_variables.values_mut() {
        if let ResourcePath::Cache { relative } = path
            && let Some(source) = mappings.get(relative)
        {
            *path = source.clone();
        }
    }
    plan.command.path_variables.insert(
        "assets_root".into(),
        ResourcePath::Existing {
            path: existing.asset_root.clone(),
        },
    );
    if metadata.assets.virtual_layout == Some(true) {
        plan.command.path_variables.insert(
            "game_assets".into(),
            ResourcePath::Existing {
                path: existing
                    .asset_root
                    .join("virtual")
                    .join(&metadata.parsed.manifest.asset_index.id),
            },
        );
    }
    plan.command
        .path_variables
        .extend(existing.argument_paths.clone());
    let retained: BTreeSet<_> = actions
        .iter()
        .filter_map(|action| match action {
            PrepareAction::Copy { sha1, .. } | PrepareAction::Extract { sha1, .. } => {
                Some(sha1.as_str())
            }
            PrepareAction::Directory { .. }
            | PrepareAction::Link { .. }
            | PrepareAction::ExtractArchive { .. }
            | PrepareAction::ExtractExisting { .. }
            | PrepareAction::GameResource { .. } => None,
        })
        .collect();
    plan.resources
        .retain(|resource| retained.contains(resource.sha1.as_str()));
    plan.actions = actions;
    plan.existing_resources = existing.resources.clone();
    plan.existing_asset_root = Some(existing.asset_root.clone());
    Ok(plan)
}
