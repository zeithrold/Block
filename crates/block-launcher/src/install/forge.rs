//! Official Forge catalogs and modern/legacy installer metadata.

use std::{collections::BTreeMap, fs::File, io::Read as _, path::Path, time::Duration};

use anyhow::{Context as _, ensure};
use minecraft_manifest::model::{Library, VersionManifest};
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest as _, Sha1};

use super::{
    fabric::LoaderVersion,
    neoforge::{InstalledLoader, InstallerProfile, Overlay},
};
use crate::resolver::ResolveContext;

const MAVEN: &str = "https://maven.minecraftforge.net/";

/// Preserve the complete official Maven version, including historical branch suffixes.
/// # Errors
/// Rejects a different game or unsafe/malformed build coordinates.
pub fn validate_version(minecraft: &str, version: &str) -> anyhow::Result<()> {
    super::fabric::validate_version(minecraft)?;
    super::fabric::validate_version(version)?;
    ensure!(
        rank(minecraft, version).is_some(),
        "Forge build does not match the selected Minecraft version"
    );
    Ok(())
}

fn rank(minecraft: &str, version: &str) -> Option<Vec<u64>> {
    super::fabric::validate_version(version).ok()?;
    let build = version.strip_prefix(minecraft)?.strip_prefix('-')?;
    let numbers = build
        .split('-')
        .next()?
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (numbers.len() >= 3 && numbers.len() <= 4).then_some(numbers)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    group_id: String,
    artifact_id: String,
    versioning: Versioning,
}
#[derive(Deserialize)]
struct Versioning {
    versions: Versions,
}
#[derive(Deserialize)]
struct Versions {
    #[serde(rename = "version", default)]
    versions: Vec<String>,
}

/// Read official builds for exactly the selected game, newest first.
/// # Errors
/// Reports unavailable/invalid catalogs, cancellation and deadlines without a fake empty list.
pub async fn versions(
    minecraft: &str,
    context: &ResolveContext,
) -> anyhow::Result<Vec<LoaderVersion>> {
    versions_from(minecraft, context, MAVEN).await
}

pub(crate) async fn versions_from(
    minecraft: &str,
    context: &ResolveContext,
    maven: &str,
) -> anyhow::Result<Vec<LoaderVersion>> {
    super::fabric::validate_version(minecraft)?;
    let url = reqwest::Url::parse(maven)?.join("net/minecraftforge/forge/maven-metadata.xml")?;
    let key = format!(
        "forge-catalog-{:x}.xml",
        Sha1::digest(url.as_str().as_bytes())
    );
    let bytes = tokio::select! {biased;
        () = context.cancel.cancelled() => anyhow::bail!("Forge catalog request cancelled"),
        result = tokio::time::timeout(Duration::from_secs(45), context.store.metadata_pinned(&key, url.as_str())) => result.context("Forge catalog request timed out")?.context("Could not load the official Forge catalog; retry")?,
    };
    ensure!(
        bytes.len() <= 4 * 1024 * 1024 && !String::from_utf8_lossy(&bytes).contains("<!DOCTYPE"),
        "Invalid Forge Maven metadata"
    );
    let metadata: Metadata =
        quick_xml::de::from_reader(bytes.as_slice()).context("Invalid Forge Maven catalog")?;
    ensure!(
        metadata.group_id == "net.minecraftforge"
            && metadata.artifact_id == "forge"
            && metadata.versioning.versions.versions.len() <= 65536,
        "Unexpected Forge Maven catalog identity"
    );
    let mut builds = BTreeMap::new();
    for version in metadata.versioning.versions.versions {
        if let Some(rank) = rank(minecraft, &version) {
            builds.insert(version, rank);
        }
    }
    let mut builds: Vec<_> = builds.into_iter().collect();
    builds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    Ok(builds
        .into_iter()
        .map(|(version, _)| LoaderVersion {
            stable: !["alpha", "beta", "-rc"]
                .iter()
                .any(|tag| version.contains(tag)),
            version,
        })
        .collect())
}

/// Install using verified official processors or the legacy universal/LaunchWrapper layout.
/// # Errors
/// Propagates metadata, verification, Java, cancellation and installer failures.
pub async fn install(
    base: &VersionManifest,
    version: &str,
    java: &Path,
    context: &ResolveContext,
) -> anyhow::Result<InstalledLoader> {
    super::neoforge::install_from_kind(base, version, java, context, MAVEN, true).await
}

pub(crate) async fn read_installer(
    path: &Path,
    version: &str,
    minecraft: &str,
    context: &ResolveContext,
) -> anyhow::Result<(Overlay, InstallerProfile)> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let profile: Value = super::neoforge::read_zip_json(&mut archive, "install_profile.json")?;
    let old = profile.get("install");
    let identity = old.unwrap_or(&profile);
    let coordinate = format!("net.minecraftforge:forge:{version}");
    let declared_path = identity.get("path").and_then(Value::as_str);
    let embedded_coordinate = profile
        .get("libraries")
        .and_then(Value::as_array)
        .is_some_and(|libraries| {
            libraries.iter().any(|library| {
                library
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        name == coordinate || name == format!("{coordinate}:universal")
                    })
            })
        });
    ensure!(
        identity.get("minecraft").and_then(Value::as_str) == Some(minecraft)
            && declared_path.map_or(embedded_coordinate, |path| path == coordinate),
        "Forge installer identity does not match the selected game/build"
    );
    let mut overlay: Value = if let Some(value) = profile.get("versionInfo") {
        value.clone()
    } else {
        super::neoforge::read_zip_json(&mut archive, "version.json")?
    };
    let id = overlay
        .get("id")
        .and_then(Value::as_str)
        .context("Missing Forge launch target")?;
    let declared_id = if old.is_some() {
        identity.get("target")
    } else {
        identity.get("version")
    };
    ensure!(
        declared_id.and_then(Value::as_str) == Some(id)
            && id
                .to_ascii_lowercase()
                .starts_with(&format!("{minecraft}-forge")),
        "Unexpected Forge launch target"
    );
    if overlay.get("inheritsFrom").is_none() {
        overlay["inheritsFrom"] = minecraft.into();
    }
    ensure!(
        overlay["inheritsFrom"] == minecraft,
        "Forge installer inherits a different game"
    );
    let libraries = overlay
        .get_mut("libraries")
        .and_then(Value::as_array_mut)
        .context("Missing Forge libraries")?;
    ensure!(libraries.len() <= 512, "Forge has too many libraries");
    let mut resolved = Vec::<Library>::new();
    ensure!(
        !context.cancel.is_cancelled(),
        "Forge installation cancelled"
    );
    for library in libraries
        .iter()
        .filter(|library| library.get("clientreq") != Some(&Value::Bool(false)))
    {
        ensure!(
            !context.cancel.is_cancelled(),
            "Forge installation cancelled"
        );
        let name = library["name"]
            .as_str()
            .context("Missing Forge library coordinate")?;
        let library_path = super::neoforge::coordinate_path(name)?;
        let downloads = if let Some(downloads) = library.get("downloads") {
            downloads.clone()
        } else if identity.get("path").and_then(Value::as_str) == Some(name) {
            let file = identity["filePath"]
                .as_str()
                .context("Missing embedded universal JAR")?;
            let entry = archive.by_name(file)?;
            ensure!(
                entry.size() > 0 && entry.size() <= 512 * 1024 * 1024,
                "Invalid universal JAR size"
            );
            let size = entry.size();
            let mut hash = Sha1::new();
            let mut reader = entry.take(size + 1);
            let mut buffer = [0; 65536];
            loop {
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
            }
            json!({"artifact":{"path":library_path,"url":"","size":size,"sha1":format!("{:x}", hash.finalize())}})
        } else {
            let repository = library
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("https://libraries.minecraft.net/");
            let repository = match repository {
                "http://files.minecraftforge.net/maven/"
                | "https://files.minecraftforge.net/maven/"
                | "http://maven.minecraftforge.net/" => MAVEN,
                "http://libraries.minecraft.net/" => "https://libraries.minecraft.net/",
                other => other,
            };
            let repository = reqwest::Url::parse(repository)?;
            ensure!(
                repository.scheme() == "https"
                    && matches!(
                        repository.host_str(),
                        Some("maven.minecraftforge.net" | "libraries.minecraft.net")
                    )
                    && repository.username().is_empty()
                    && repository.password().is_none(),
                "Unexpected legacy Forge library repository"
            );
            let url = repository.join(&library_path)?;
            let descriptor = async {
                if let Some(sha1) = library
                    .get("checksums")
                    .and_then(Value::as_array)
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
                {
                    context
                        .store
                        .describe_maven_with_sha1(url.as_str(), sha1)
                        .await
                } else {
                    context.store.describe_maven(url.as_str()).await
                }
            };
            let resource = tokio::select! {biased;
                () = context.cancel.cancelled() => anyhow::bail!("Forge library metadata cancelled"),
                result = tokio::time::timeout(Duration::from_secs(45), descriptor) => result.context("Forge library metadata timed out")??,
            };
            ensure!(
                resource.size > 0 && resource.size <= 512 * 1024 * 1024,
                "Invalid legacy Forge library size"
            );
            if let Some(checksums) = library.get("checksums").and_then(Value::as_array) {
                ensure!(
                    checksums
                        .iter()
                        .any(|value| value.as_str() == Some(resource.sha1.as_str())),
                    "Legacy Forge library checksum differs from installer"
                );
            }
            json!({"artifact":{"path":library_path,"url":resource.url,"size":resource.size,"sha1":resource.sha1}})
        };
        let mut normalized = json!({"name":name,"downloads":downloads});
        for key in ["natives", "extract", "rules"] {
            if let Some(value) = library.get(key) {
                normalized[key] = value.clone();
            }
        }
        resolved.push(serde_json::from_value(normalized)?);
    }
    *libraries = serde_json::to_value(resolved)?
        .as_array()
        .context("Invalid Forge libraries")?
        .clone();
    let normalized_profile = json!({"minecraft":minecraft, "libraries": profile.get("libraries").cloned().unwrap_or(json!([])), "data":profile.get("data").cloned().unwrap_or(json!({}))});
    Ok((
        serde_json::from_value(overlay)?,
        serde_json::from_value(normalized_profile)?,
    ))
}

pub(crate) fn embedded_path(
    archive: &mut zip::ZipArchive<File>,
    path: &str,
) -> anyhow::Result<String> {
    let normal = format!("maven/{path}");
    if archive.file_names().any(|name| name == normal) {
        return Ok(normal);
    }
    let profile: Value = super::neoforge::read_zip_json(archive, "install_profile.json")?;
    let install = &profile["install"];
    ensure!(
        super::neoforge::coordinate_path(
            install["path"]
                .as_str()
                .context("Missing universal coordinate")?
        )? == path,
        "Installer omitted embedded library: {path}"
    );
    Ok(install["filePath"]
        .as_str()
        .context("Missing universal JAR")?
        .into())
}

/// Preserve Forge's canonical Maven module paths in the compiled plan.
/// # Errors
/// Rejects a target/manifest mismatch or invalid installed artifacts.
pub fn adapt_plan(
    plan: &mut crate::plan::ExecutionPlan,
    manifest: &VersionManifest,
    installed: &InstalledLoader,
) -> anyhow::Result<()> {
    super::neoforge::adapt_plan_kind(plan, manifest, installed, true)
}

#[cfg(test)]
#[path = "forge_tests.rs"]
mod tests;
