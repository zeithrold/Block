use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context as _, ensure};
use serde_json::Value;
use sha1::{Digest as _, Sha1};

use super::{ExistingSourceFailure, resources::Inputs};
use crate::{manifest::model::VersionManifest, plan::ExistingResource, platform::Target};

#[path = "manifests/inheritance.rs"]
mod inheritance;
#[path = "manifests/libraries.rs"]
mod libraries;

pub(super) struct Chain {
    pub value: Value,
    pub client_id: String,
    pub digest: String,
    pub files: Vec<ExistingResource>,
}

pub(super) fn read_chain(root: &Path, selected: &str) -> anyhow::Result<Chain> {
    let mut current = selected.to_owned();
    let mut documents = Vec::new();
    let mut visited = BTreeSet::new();
    let mut digest = Sha1::new();
    let mut files = Vec::new();
    let mut total = 0_u64;
    let mut client_id = None;
    loop {
        crate::executor::confined_path(root, &current)?;
        ensure!(
            !current.contains('/')
                && !matches!(current.as_str(), "latest-release" | "latest-snapshot"),
            "Unsafe concrete version ID"
        );
        ensure!(
            visited.len() < 16 && visited.insert(current.clone()),
            ExistingSourceFailure::mark(
                root,
                anyhow::anyhow!("Existing version inheritance is cyclic or too deep")
            )
        );
        let path = root
            .join("versions")
            .join(&current)
            .join(format!("{current}.json"));
        let size = fs::metadata(&path)
            .with_context(|| format!("Missing existing version metadata: {}", path.display()))
            .map_err(|error| ExistingSourceFailure::mark(&path, error))?
            .len();
        ensure!(
            size <= 8 * 1024 * 1024,
            ExistingSourceFailure::mark(
                &path,
                anyhow::anyhow!("Existing version metadata is too large")
            )
        );
        let file = ExistingResource::capture(path.clone())?;
        let bytes = file.read_verified(8 * 1024 * 1024)?;
        total = total
            .checked_add(u64::try_from(bytes.len())?)
            .context("Metadata size overflow")?;
        ensure!(
            total <= 32 * 1024 * 1024,
            ExistingSourceFailure::mark(
                root,
                anyhow::anyhow!("Existing inheritance chain is too large")
            )
        );
        let value: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("Invalid existing version metadata: {}", path.display()))
            .map_err(|error| ExistingSourceFailure::mark(&path, error))?;
        ensure!(
            value.is_object() && value.get("id").and_then(Value::as_str) == Some(current.as_str()),
            ExistingSourceFailure::mark(
                &path,
                anyhow::anyhow!("Existing version metadata ID mismatch")
            )
        );
        ensure!(
            value.get("patches").is_none() && value.get("processors").is_none(),
            "Unsupported existing launcher format; installed version metadata is required"
        );
        digest.update(serde_json::to_vec(&current)?);
        digest.update(Sha1::digest(&bytes));
        files.push(file);
        if client_id.is_none() {
            if let Some(jar) = value.get("jar") {
                let jar = jar.as_str().context("Invalid inherited client ID")?;
                crate::executor::confined_path(root, jar)?;
                ensure!(!jar.contains('/'), "Unsafe inherited client ID");
                client_id = Some(jar.to_owned());
            } else if value.pointer("/downloads/client").is_some() {
                client_id = Some(current.clone());
            }
        }
        let parent = value
            .get("inheritsFrom")
            .map(|value| value.as_str().context("Invalid inherited version ID"))
            .transpose()
            .map_err(|error| ExistingSourceFailure::mark(&path, error))?
            .map(str::to_owned);
        documents.push(value);
        match parent {
            Some(parent) => current = parent,
            None => break,
        }
    }
    let mut value = documents
        .pop()
        .context("Empty existing inheritance chain")?;
    while let Some(child) = documents.pop() {
        inheritance::merge(&mut value, child)?;
    }
    Ok(Chain {
        value,
        client_id: client_id.context("Existing metadata has no declared client")?,
        digest: format!("{:x}", digest.finalize()),
        files,
    })
}

pub(super) fn normalize(
    mut value: Value,
    client_id: &str,
    target: &Target,
    inputs: &mut Inputs<'_>,
) -> anyhow::Result<VersionManifest> {
    let object = value.as_object_mut().context("Invalid version document")?;
    for key in [
        "inheritsFrom",
        "jar",
        "releaseTarget",
        "clientVersion",
        "comment",
        "_comment",
    ] {
        object.remove(key);
    }
    libraries::normalize(
        object
            .get_mut("libraries")
            .context("Missing existing libraries")?,
        target,
        inputs,
    )?;
    let mut manifest: VersionManifest = serde_json::from_value(value)
        .context("Unsupported or incomplete existing version metadata")
        .map_err(|error| ExistingSourceFailure::mark(inputs.root, error))?;
    let client = &manifest.downloads.client;
    inputs.declare(
        &format!("versions/{client_id}/{client_id}.jar"),
        crate::resolver::download_resource(client),
    );
    inputs.select(
        &format!("versions/{client_id}/{client_id}.jar"),
        Some((&client.sha1, client.size)),
    )?;
    if let Some(logging) = &manifest.logging {
        let file = &logging.client.file;
        inputs.declare(
            &format!("assets/log_configs/{}", file.id),
            block_download::Resource {
                sha1: file.sha1.clone(),
                size: file.size,
                url: file.url.clone(),
                sha512: None,
                alternate_urls: Vec::new(),
            },
        );
        inputs.select(
            &format!("assets/log_configs/{}", file.id),
            Some((&file.sha1, file.size)),
        )?;
    }
    if let Some(arguments) = manifest.minecraft_arguments.take() {
        manifest.arguments = Some(crate::manifest::model::Arguments {
            game: super::arguments::tokenize(&arguments)?
                .into_iter()
                .map(crate::manifest::model::Argument::Literal)
                .collect(),
            jvm: vec![
                crate::manifest::model::Argument::Literal(
                    "-Djava.library.path=${natives_directory}".into(),
                ),
                crate::manifest::model::Argument::Literal("-cp".into()),
                crate::manifest::model::Argument::Literal("${classpath}".into()),
            ],
            default_user_jvm: None,
        });
    }
    Ok(manifest)
}
