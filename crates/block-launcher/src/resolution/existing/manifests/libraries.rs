use std::collections::BTreeMap;

use anyhow::{Context as _, ensure};
use serde_json::{Value, json};

use super::super::resources::Inputs;
use crate::{manifest::model::Rule, platform::Target};

pub(super) fn normalize(
    libraries: &mut Value,
    target: &Target,
    inputs: &mut Inputs<'_>,
) -> anyhow::Result<()> {
    let libraries = libraries
        .as_array_mut()
        .context("Invalid local library list")?;
    ensure!(libraries.len() <= 2048, "Too many existing libraries");
    let mut selected = Vec::new();
    for mut library in std::mem::take(libraries) {
        let rules = library
            .get("rules")
            .map(|value| serde_json::from_value::<Vec<Rule>>(value.clone()))
            .transpose()?;
        if library.get("clientreq") == Some(&Value::Bool(false))
            || !crate::rules::allowed(rules.as_deref(), target, &BTreeMap::new())?
        {
            continue;
        }
        let object = library.as_object_mut().context("Invalid local library")?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .context("Missing library coordinate")?
            .to_owned();
        let base = object
            .remove("url")
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "https://libraries.minecraft.net/".into());
        for key in ["clientreq", "serverreq", "checksums", "comment"] {
            object.remove(key);
        }
        let native = object
            .get("natives")
            .and_then(|value| value.get(target.manifest_os()))
            .and_then(Value::as_str)
            .map(|value| value.replace("${arch}", target.bits()));
        let downloads = object
            .entry("downloads")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("Invalid existing library downloads")?;
        if let Some(artifact) = downloads.get_mut("artifact") {
            normalize_artifact(artifact, &name, &base, inputs)?;
        } else if downloads.is_empty() {
            let mut artifact = json!({});
            normalize_artifact(&mut artifact, &name, &base, inputs)?;
            downloads.insert("artifact".into(), artifact);
        }
        if let Some(classifier) = native {
            let classifiers = downloads
                .entry("classifiers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context("Invalid native classifiers")?;
            let artifact = classifiers
                .entry(classifier.clone())
                .or_insert_with(|| json!({}));
            let coordinate = format!("{name}:{classifier}");
            normalize_artifact(artifact, &coordinate, &base, inputs)?;
            classifiers.retain(|key, _| *key == classifier);
        } else {
            downloads.remove("classifiers");
        }
        selected.push(library);
    }
    *libraries = selected;
    Ok(())
}

fn normalize_artifact(
    value: &mut Value,
    coordinate: &str,
    base: &str,
    inputs: &mut Inputs<'_>,
) -> anyhow::Result<()> {
    let artifact = value
        .as_object_mut()
        .context("Invalid local library artifact")?;
    let relative = artifact
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .map_or_else(|| crate::install::neoforge::coordinate_path(coordinate), Ok)?;
    let expected = match (
        artifact.get("sha1").and_then(Value::as_str),
        artifact.get("size").and_then(Value::as_u64),
    ) {
        (Some(sha1), Some(size)) if !sha1.is_empty() => Some((sha1, size)),
        (None | Some(""), None | Some(_)) => None,
        _ => anyhow::bail!("Incomplete existing library integrity descriptor: {coordinate}"),
    };
    if let Some((sha1, size)) = expected {
        let url = artifact
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        inputs.declare(
            &format!("libraries/{relative}"),
            block_download::Resource {
                sha1: sha1.to_owned(),
                size,
                url: url.to_owned(),
                sha512: None,
                alternate_urls: Vec::new(),
            },
        );
    }
    let resource = inputs.select(&format!("libraries/{relative}"), expected)?;
    artifact.insert("path".into(), relative.clone().into());
    artifact.insert("sha1".into(), resource.sha1.into());
    artifact.insert("size".into(), resource.size.into());
    artifact
        .entry("url")
        .or_insert_with(|| format!("{}/{relative}", base.trim_end_matches('/')).into());
    Ok(())
}
