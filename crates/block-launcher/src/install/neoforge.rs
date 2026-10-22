//! Official NeoForge catalogs and verified client installer execution.
//!
//! Installer work is confined to a fresh staging directory. Only verified library
//! objects and a merged version manifest are published; game saves are never touched.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context as _, ensure};
use block_download::Resource;
use minecraft_manifest::model::{Argument, Library, VersionManifest};
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

use crate::{
    plan::{ExecutionPlan, PrepareAction},
    resolver::{ResolveContext, download_resource},
};

#[cfg(test)]
const VERSION: &str = "20.6.144";
#[cfg(test)]
const MINECRAFT_VERSION: &str = "1.20.6";
const MAVEN: &str = "https://maven.neoforged.net/releases/";
const MAX_ARTIFACT_SIZE: u64 = 512 * 1024 * 1024;
const MAX_INSTALL_SIZE: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledArtifact {
    /// Validated, slash-separated path relative to the canonical libraries directory.
    pub path: String,
    pub resource: Resource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledLoader {
    pub version: String,
    pub minecraft_version: String,
    pub manifest_path: PathBuf,
    pub manifest_sha1: String,
    pub artifacts: Vec<InstalledArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Overlay {
    id: String,
    inherits_from: String,
    main_class: String,
    #[serde(default)]
    arguments: OverlayArguments,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    libraries: Vec<Library>,
}

#[derive(Debug, Default, Deserialize)]
struct OverlayArguments {
    #[serde(default)]
    game: Vec<Argument>,
    #[serde(default)]
    jvm: Vec<Argument>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InstallerProfile {
    minecraft: String,
    #[serde(default)]
    libraries: Vec<Library>,
    #[serde(default)]
    data: BTreeMap<String, SideData>,
}

#[derive(Debug, Deserialize)]
struct SideData {
    client: Option<String>,
}

#[derive(Deserialize)]
struct MavenMetadata {
    #[serde(rename = "groupId")]
    group: String,
    #[serde(rename = "artifactId")]
    artifact: String,
    versioning: MavenVersioning,
}
#[derive(Deserialize)]
struct MavenVersioning {
    versions: MavenVersions,
}
#[derive(Deserialize)]
struct MavenVersions {
    #[serde(rename = "version", default)]
    versions: Vec<String>,
}

fn version_details(version: &str) -> Option<(String, semver::Version)> {
    super::fabric::validate_version(version).ok()?;
    if let Some(loader) = version.strip_prefix("1.20.1-") {
        return Some(("1.20.1".into(), semver::Version::parse(loader).ok()?));
    }
    let (release, suffix) = version.split_once('-').unwrap_or((version, ""));
    let numbers = release
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let (mut minecraft, build) = match numbers.as_slice() {
        [major @ 20..=25, minor, build] => (
            if *minor == 0 {
                format!("1.{major}")
            } else {
                format!("1.{major}.{minor}")
            },
            *build,
        ),
        [year, release, patch, build] if *year >= 26 => (
            if *patch == 0 {
                format!("{year}.{release}")
            } else {
                format!("{year}.{release}.{patch}")
            },
            *build,
        ),
        _ => return None,
    };
    let (prerelease, game_suffix) = suffix.split_once('+').unwrap_or((suffix, ""));
    if !game_suffix.is_empty() {
        let (phase, number) = game_suffix.rsplit_once('-')?;
        if !["snapshot", "pre", "rc"].contains(&phase) || number.parse::<u32>().is_err() {
            return None;
        }
        minecraft.push('-');
        minecraft.push_str(game_suffix);
    }
    let mut rank = semver::Version::new(0, 0, build);
    rank.pre = semver::Prerelease::new(prerelease).ok()?;
    Some((minecraft, rank))
}

/// Validate the official version-number relationship before creating a catalog target.
pub fn validate_version(minecraft: &str, version: &str) -> anyhow::Result<()> {
    ensure!(
        version_details(version).is_some_and(|(game, _)| game == minecraft),
        "NeoForge build does not match the selected Minecraft version"
    );
    Ok(())
}

fn artifact(version: &str) -> &'static str {
    if version.starts_with("1.20.1-") {
        "forge"
    } else {
        "neoforge"
    }
}

fn manifest_id(version: &str) -> String {
    version.strip_prefix("1.20.1-").map_or_else(
        || format!("neoforge-{version}"),
        |loader| format!("1.20.1-forge-{loader}"),
    )
}

/// Read all official builds for a game, newest first, including historical and snapshot catalogs.
pub async fn versions(
    minecraft: &str,
    context: &ResolveContext,
) -> anyhow::Result<Vec<super::fabric::LoaderVersion>> {
    versions_from(minecraft, context, MAVEN).await
}

async fn versions_from(
    minecraft: &str,
    context: &ResolveContext,
    maven: &str,
) -> anyhow::Result<Vec<super::fabric::LoaderVersion>> {
    ensure!(
        !minecraft.is_empty() && minecraft.len() <= 80 && !minecraft.chars().any(char::is_control),
        "Invalid Minecraft version"
    );
    let artifact = if minecraft == "1.20.1" {
        "forge"
    } else {
        "neoforge"
    };
    let url = reqwest::Url::parse(maven)?
        .join(&format!("net/neoforged/{artifact}/maven-metadata.xml"))?;
    let key = format!(
        "neoforge-catalog-{artifact}-{:x}.xml",
        Sha1::digest(url.as_str().as_bytes())
    );
    let bytes = tokio::select! {biased;
        () = context.cancel.cancelled() => anyhow::bail!("NeoForge catalog request cancelled"),
        result = tokio::time::timeout(Duration::from_secs(45), context.store.metadata_pinned(&key, url.as_str())) => result.context("NeoForge catalog request timed out")?.context("Could not load the official NeoForge catalog; retry")?,
    };
    ensure!(
        bytes.len() <= 4 * 1024 * 1024 && !String::from_utf8_lossy(&bytes).contains("<!DOCTYPE"),
        "Invalid NeoForge Maven metadata"
    );
    let metadata: MavenMetadata =
        quick_xml::de::from_reader(bytes.as_slice()).context("Invalid NeoForge Maven catalog")?;
    ensure!(
        metadata.group == "net.neoforged"
            && metadata.artifact == artifact
            && metadata.versioning.versions.versions.len() <= 65536,
        "Unexpected NeoForge Maven catalog identity"
    );
    let mut builds: BTreeMap<String, semver::Version> = BTreeMap::new();
    for version in metadata.versioning.versions.versions {
        if let Some((game, rank)) = version_details(&version)
            && game == minecraft
        {
            builds.insert(version, rank);
        }
    }
    let mut builds: Vec<_> = builds.into_iter().collect();
    builds.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
    Ok(builds
        .into_iter()
        .map(|(version, rank)| super::fabric::LoaderVersion {
            version,
            stable: rank.pre.is_empty(),
        })
        .collect())
}

/// Install `NeoForge` into the shared immutable cache, leaving mutable game data alone.
///
/// `java` must be the already prepared native runtime for the selected game.
/// Failed attempts retain their isolated staging directory and installer log for
/// diagnosis. Retrying creates a new stage and cannot overwrite a valid installation.
///
/// # Errors
/// Fails for an unsupported game version, invalid upstream artifacts, unavailable
/// Java, cancellation, installer failure, or unsafe/generated library paths.
pub async fn install(
    base: &VersionManifest,
    version: &str,
    java: &Path,
    context: &ResolveContext,
) -> anyhow::Result<InstalledLoader> {
    install_from(base, version, java, context, MAVEN).await
}

async fn install_from(
    base: &VersionManifest,
    version: &str,
    java: &Path,
    context: &ResolveContext,
    maven: &str,
) -> anyhow::Result<InstalledLoader> {
    install_from_kind(base, version, java, context, maven, false).await
}

pub(crate) async fn install_from_kind(
    base: &VersionManifest,
    version: &str,
    java: &Path,
    context: &ResolveContext,
    maven: &str,
    forge: bool,
) -> anyhow::Result<InstalledLoader> {
    if forge {
        super::forge::validate_version(&base.id, version)?;
    } else {
        validate_version(&base.id, version)?;
    }
    let kind = if forge { "forge" } else { "neoforge" };
    ensure!(!context.cancel.is_cancelled(), "installation cancelled");
    let root = context.store.root().canonicalize()?;
    let destination = root.join("loaders").join(kind).join(version);
    tokio::fs::create_dir_all(&destination).await?;
    let _installation_lock = acquire_installation_lock(&destination, &context.cancel).await?;
    let receipt = destination.join("installed.json");
    if let Ok(bytes) = tokio::fs::read(&receipt).await
        && let Ok(installed) = serde_json::from_slice::<InstalledLoader>(&bytes)
        && installed.version == version
        && installed.minecraft_version == base.id
        && installed.manifest_path == destination.join("manifest.json")
        && installed.manifest_path.is_file()
        && file_digest(&installed.manifest_path)? == installed.manifest_sha1
        && cached_artifacts_valid(&installed, context)?
    {
        return Ok(installed);
    }
    let builds = if forge {
        super::forge::versions_from(&base.id, context, maven).await?
    } else {
        versions_from(&base.id, context, maven).await?
    };
    ensure!(
        builds.iter().any(|build| build.version == version),
        "The selected loader build is no longer available for this Minecraft version"
    );
    let artifact = if forge { "forge" } else { artifact(version) };
    let group = if forge {
        "net/minecraftforge"
    } else {
        "net/neoforged"
    };
    let installer_url =
        format!("{maven}{group}/{artifact}/{version}/{artifact}-{version}-installer.jar");
    let installer_resource = tokio::select! {biased;
        () = context.cancel.cancelled() => anyhow::bail!("Loader installation cancelled"),
        result = tokio::time::timeout(Duration::from_secs(45), context.store.describe_maven(&installer_url)) => result.context("Loader installer metadata timed out")??,
    };
    ensure!(
        installer_resource.size > 0 && installer_resource.size <= MAX_ARTIFACT_SIZE,
        "Invalid loader installer size"
    );
    let archive_path = context
        .store
        .fetch(&installer_resource, &context.cancel)
        .await?;
    let (overlay, profile) = if forge {
        super::forge::read_installer(&archive_path, version, &base.id, context).await?
    } else {
        read_installer(&archive_path)?
    };
    ensure!(
        profile.minecraft == base.id && overlay.inherits_from == base.id,
        "installer game version does not match selected version"
    );
    ensure!(
        forge || overlay.id == manifest_id(version),
        "unexpected NeoForge launch target"
    );
    let stage = root
        .join("installer-staging")
        .join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&stage).await?;
    let result = install_staged(base, java, context, &archive_path, &stage, overlay, profile)
        .await
        .with_context(|| {
            format!(
                "Loader installation failed; diagnostic directory: {}",
                stage.display()
            )
        })?;
    tokio::fs::create_dir_all(&destination).await?;
    let manifest_path = destination.join("manifest.json");
    atomic_json(&manifest_path, &result.0)?;
    let installed = InstalledLoader {
        version: version.into(),
        minecraft_version: base.id.clone(),
        manifest_sha1: file_digest(&manifest_path)?,
        manifest_path,
        artifacts: result.1,
    };
    // The receipt is the final commit marker. An interrupted attempt remains retryable.
    atomic_json(&receipt, &installed)?;
    Ok(installed)
}

async fn acquire_installation_lock(
    directory: &Path,
    cancel: &crate::CancellationToken,
) -> anyhow::Result<File> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("install.lock"))?;
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => return Ok(lock),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(error) => return Err(error.into()),
        }
        tokio::select! {
            () = cancel.cancelled() => anyhow::bail!("installation cancelled while waiting for loader installer lock"),
            () = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
    }
}

async fn install_staged(
    base: &VersionManifest,
    java: &Path,
    context: &ResolveContext,
    installer: &Path,
    stage: &Path,
    overlay: Overlay,
    profile: InstallerProfile,
) -> anyhow::Result<(VersionManifest, Vec<InstalledArtifact>)> {
    let legacy = overlay.main_class == "net.minecraft.launchwrapper.Launch";
    let generated = if legacy {
        Vec::new()
    } else {
        generated_client_paths(&profile)?
    };
    let libraries = stage.join("libraries");
    let mut expected = BTreeMap::new();
    for library in profile.libraries.iter().chain(&overlay.libraries) {
        if let Some(artifact) = &library.downloads.artifact {
            safe_relative(&artifact.path)?;
            let resource = Resource {
                sha1: artifact.sha1.clone(),
                size: artifact.size,
                url: artifact.url.clone(),
            };
            ensure!(
                resource.url.is_empty() || resource.url.starts_with("https://"),
                "installer library requires HTTPS: {}",
                artifact.path
            );
            ensure!(
                resource.size <= MAX_ARTIFACT_SIZE,
                "installer library exceeds size limit"
            );
            if let Some(previous) = expected.insert(artifact.path.clone(), resource.clone()) {
                ensure!(
                    previous == resource,
                    "conflicting installer library: {}",
                    artifact.path
                );
            }
        }
    }
    // Some Forge generations ship their universal/client libraries inside the
    // verified installer instead of publishing a download URL for them.
    let mut archive = zip::ZipArchive::new(File::open(installer)?)?;
    for (path, resource) in &expected {
        // Patched clients are verified after processor execution, not extracted.
        if generated.contains(path) {
            continue;
        }
        if resource.url.is_empty() {
            let entry_path = super::forge::embedded_path(&mut archive, path)?;
            let mut entry = archive
                .by_name(&entry_path)
                .with_context(|| format!("Installer omitted embedded library: {path}"))?;
            ensure!(
                entry.size() == resource.size && entry.size() <= MAX_ARTIFACT_SIZE,
                "Invalid embedded installer library size"
            );
            let mut temporary = tempfile::NamedTempFile::new_in(stage)?;
            std::io::copy(&mut entry, &mut temporary)?;
            ensure!(
                file_digest(temporary.path())? == resource.sha1,
                "Embedded installer library checksum mismatch"
            );
            publish_object(temporary.path(), resource, context)?;
        }
    }
    drop(archive);
    let resources: Vec<_> = expected
        .values()
        .filter(|resource| !resource.url.is_empty())
        .cloned()
        .collect();
    context.store.fetch_all(&resources, &context.cancel).await?;
    for (path, resource) in &expected {
        if generated.contains(path) {
            continue;
        }
        copy_object(
            &context.store.object_path(&resource.sha1)?,
            &libraries.join(path),
        )
        .await?;
    }
    let client = context
        .store
        .fetch(&download_resource(&base.downloads.client), &context.cancel)
        .await?;
    let version_directory = stage.join("versions").join(&base.id);
    tokio::fs::create_dir_all(&version_directory).await?;
    atomic_json(&version_directory.join(format!("{}.json", base.id)), base)?;
    copy_object(&client, &version_directory.join(format!("{}.jar", base.id))).await?;
    atomic_json(
        &stage.join("launcher_profiles.json"),
        &serde_json::json!({"profiles":{}, "settings":{}, "version":3}),
    )?;
    if !legacy {
        let inputs =
            prepare_processor_inputs(base, &profile, stage, context, PROCESSOR_INDEX).await?;
        run_installer(java, installer, stage, &inputs, &context.cancel).await?;
    }
    let artifacts = import_libraries(&libraries, &expected, context)?;
    for required in generated {
        ensure!(
            artifacts
                .iter()
                .any(|artifact| artifact.path == required && artifact.resource.size > 0),
            "installer omitted generated client artifact: {required}"
        );
    }
    let manifest = merge_manifest(base, overlay)?;
    Ok((manifest, artifacts))
}

fn generated_client_paths(profile: &InstallerProfile) -> anyhow::Result<Vec<String>> {
    let mut paths = Vec::new();
    for name in ["PATCHED", "MC_EXTRA"] {
        let coordinate = profile
            .data
            .get(name)
            .and_then(|value| value.client.as_deref());
        if let Some(coordinate) = coordinate {
            let coordinate = coordinate
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .context("Invalid installer client output coordinate")?;
            paths.push(coordinate_path(coordinate)?);
        } else {
            ensure!(
                name != "PATCHED",
                "Installer omitted its patched client output"
            );
        }
    }
    Ok(paths)
}

pub(crate) fn coordinate_path(coordinate: &str) -> anyhow::Result<String> {
    let (coordinate, extension) = coordinate.split_once('@').unwrap_or((coordinate, "jar"));
    ensure!(
        !extension.is_empty() && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "Invalid Maven extension"
    );
    let parts: Vec<_> = coordinate.split(':').collect();
    ensure!((3..=4).contains(&parts.len()), "Invalid Maven coordinate");
    for part in &parts {
        super::fabric::validate_version(part)?;
    }
    let classifier = parts
        .get(3)
        .map_or_else(String::new, |value| format!("-{value}"));
    let path = format!(
        "{}/{}/{}/{}-{}{classifier}.{extension}",
        parts[0].replace('.', "/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2]
    );
    safe_relative(&path)?;
    Ok(path)
}

const INSTALLER_CACHE_AGENT: &[u8] =
    include_bytes!("../../assets/installer-cache/installer-cache.jar");
const PROCESSOR_INDEX: &str = "https://launchermeta.mojang.com/mc/game/version_manifest.json";

async fn prepare_processor_inputs(
    base: &VersionManifest,
    profile: &InstallerProfile,
    stage: &Path,
    context: &ResolveContext,
    index_url: &str,
) -> anyhow::Result<PathBuf> {
    let mut inputs = BTreeMap::new();
    if profile
        .data
        .get("MOJMAPS")
        .and_then(|value| value.client.as_ref())
        .is_some()
    {
        let origin = reqwest::Url::parse(index_url)?.origin();
        let key = format!(
            "installer-game-index-{:x}.json",
            Sha1::digest(index_url.as_bytes())
        );
        let index = tokio::select! {biased;
            () = context.cancel.cancelled() => anyhow::bail!("Installer metadata cancelled"),
            result = tokio::time::timeout(Duration::from_secs(45), context.store.metadata_pinned(&key, index_url)) => result.context("Installer metadata timed out")??,
        };
        ensure!(
            index.len() <= 8 * 1024 * 1024,
            "Installer game index is too large"
        );
        let parsed: serde_json::Value = serde_json::from_slice(&index)?;
        let versions = parsed["versions"]
            .as_array()
            .context("Missing installer game versions")?;
        ensure!(
            versions.len() <= 65536,
            "Installer game index has too many versions"
        );
        let mut entries = versions
            .iter()
            .filter(|entry| entry["id"].as_str() == Some(base.id.as_str()));
        let entry = entries
            .next()
            .context("Selected game is missing from the official installer index")?;
        ensure!(entries.next().is_none(), "Duplicate installer game version");
        let version_url = entry["url"]
            .as_str()
            .context("Missing installer game metadata URL")?;
        let allowed = |url: &str| -> anyhow::Result<()> {
            let url = reqwest::Url::parse(url)?;
            ensure!(
                url.username().is_empty()
                    && url.password().is_none()
                    && (url.origin() == origin
                        || (url.scheme() == "https"
                            && matches!(
                                url.host_str(),
                                Some(
                                    "piston-meta.mojang.com"
                                        | "piston-data.mojang.com"
                                        | "launchermeta.mojang.com"
                                        | "launcher.mojang.com"
                                )
                            ))),
                "Unexpected installer game metadata origin"
            );
            Ok(())
        };
        allowed(version_url)?;
        let key = format!(
            "installer-game-version-{:x}.json",
            Sha1::digest(version_url.as_bytes())
        );
        let version = tokio::select! {biased;
            () = context.cancel.cancelled() => anyhow::bail!("Installer metadata cancelled"),
            result = tokio::time::timeout(Duration::from_secs(45), context.store.metadata_pinned(&key, version_url)) => result.context("Installer metadata timed out")??,
        };
        ensure!(
            version.len() <= 4 * 1024 * 1024,
            "Installer game metadata is too large"
        );
        let parsed: VersionManifest = serde_json::from_slice(&version)?;
        ensure!(
            parsed == *base,
            "Installer metadata differs from the verified selected game"
        );
        let mappings = base
            .downloads
            .client_mappings
            .as_ref()
            .context("Selected game has no official client mappings")?;
        allowed(&mappings.url)?;
        ensure!(
            mappings.size > 0 && mappings.size <= 128 * 1024 * 1024,
            "Invalid installer mappings size"
        );
        let mappings_path = context
            .store
            .fetch(&download_resource(mappings), &context.cancel)
            .await?;
        let index_path = stage.join("processor-game-index.json");
        let version_path = stage.join("processor-game-version.json");
        let input_path = stage.join("processor-mappings.txt");
        tokio::fs::write(&index_path, &index).await?;
        tokio::fs::write(&version_path, &version).await?;
        copy_object(&mappings_path, &input_path).await?;
        inputs.insert(index_url.to_owned(), index_path);
        inputs.insert(version_url.to_owned(), version_path);
        inputs.insert(mappings.url.clone(), input_path);
    }
    ensure!(
        !context.cancel.is_cancelled(),
        "Installer preparation cancelled"
    );
    let path = stage.join("processor-inputs.bin");
    let mut file = File::create(&path)?;
    file.write_all(&u32::try_from(inputs.len())?.to_be_bytes())?;
    let os = crate::platform::Target::host()?.os;
    for (url, path) in inputs {
        let path = crate::command::java_path(path, os)?;
        for value in [
            url.as_str(),
            path.to_str().context("Non-UTF8 installer input path")?,
        ] {
            ensure!(
                value.len() <= 1024 * 1024,
                "Installer input path is too long"
            );
            file.write_all(&u32::try_from(value.len())?.to_be_bytes())?;
            file.write_all(value.as_bytes())?;
        }
    }
    file.sync_all()?;
    Ok(path)
}

async fn run_installer(
    java: &Path,
    installer: &Path,
    stage: &Path,
    inputs: &Path,
    cancel: &crate::CancellationToken,
) -> anyhow::Result<()> {
    let agent = stage.join("installer-cache.jar");
    tokio::fs::write(&agent, INSTALLER_CACHE_AGENT).await?;
    let home = stage.join("home");
    tokio::fs::create_dir_all(&home).await?;
    let os = crate::platform::Target::host()?.os;
    let java = crate::command::java_path(java.to_owned(), os)?;
    let installer = crate::command::java_path(installer.to_owned(), os)?;
    let java_stage = crate::command::java_path(stage.to_owned(), os)?;
    let home = crate::command::java_path(home, os)?;
    let log_path = stage.join("installer.log");
    let log = File::create(&log_path)?;
    let mut command = tokio::process::Command::new(java);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command
        .arg("-Djava.awt.headless=true")
        .arg(format!(
            "-javaagent:installer-cache.jar={}",
            inputs.strip_prefix(stage)?.display()
        ))
        .arg(format!("-Duser.home={}", home.display()))
        .arg("-jar")
        .arg(installer)
        .arg("--offline")
        .arg("--installClient")
        .arg(java_stage)
        .current_dir(stage)
        .env_remove("MICROSOFT_CLIENT_SECRET")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .context("cannot start the prepared Java runtime for loader installer")?;
    let status = tokio::select! {
        () = cancel.cancelled() => {
            child.kill().await?;
            child.wait().await?;
            anyhow::bail!("installation cancelled; log: {}", log_path.display());
        }
        result = tokio::time::timeout(Duration::from_mins(15), child.wait()) => if let Ok(status) = result {
            status?
        } else {
                child.kill().await?;
                child.wait().await?;
                anyhow::bail!("installer timed out; log: {}", log_path.display());
        }
    };
    ensure!(
        status.success(),
        "installer exited with {status}; log: {}",
        log_path.display()
    );
    Ok(())
}

fn read_installer(path: &Path) -> anyhow::Result<(Overlay, InstallerProfile)> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let overlay = read_zip_json(&mut archive, "version.json")?;
    let profile = read_zip_json(&mut archive, "install_profile.json")?;
    Ok((overlay, profile))
}

pub(crate) fn read_zip_json<T: serde::de::DeserializeOwned>(
    archive: &mut zip::ZipArchive<File>,
    name: &str,
) -> anyhow::Result<T> {
    let entry = archive.by_name(name)?;
    ensure!(
        entry.size() <= 4 * 1024 * 1024,
        "installer metadata is too large"
    );
    Ok(serde_json::from_reader(entry.take(4 * 1024 * 1024 + 1))?)
}

fn merge_manifest(base: &VersionManifest, overlay: Overlay) -> anyhow::Result<VersionManifest> {
    ensure!(
        overlay.inherits_from == base.id,
        "loader inherits a different game version"
    );
    let mut merged = base.clone();
    let keys: Vec<_> = overlay
        .libraries
        .iter()
        .map(|library| coordinate_key(&library.name))
        .collect();
    merged
        .libraries
        .retain(|library| !keys.contains(&coordinate_key(&library.name)));
    merged.libraries.extend(overlay.libraries);
    merged.id = overlay.id;
    merged.main_class = overlay.main_class;
    if let Some(arguments) = overlay.minecraft_arguments {
        merged.minecraft_arguments = Some(arguments);
        merged.arguments = None;
        return Ok(merged);
    }
    let arguments = merged
        .arguments
        .as_mut()
        .context("Loader requires modern game arguments")?;
    arguments.game.extend(overlay.arguments.game);
    arguments.jvm.extend(overlay.arguments.jvm);
    Ok(merged)
}

fn coordinate_key(name: &str) -> Vec<&str> {
    let name = name.split('@').next().unwrap_or(name);
    // Replace a library version while retaining distinct native/classifier variants.
    name.split(':')
        .enumerate()
        .filter_map(|(index, part)| (index != 2).then_some(part))
        .collect()
}

fn import_libraries(
    libraries: &Path,
    expected: &BTreeMap<String, Resource>,
    context: &ResolveContext,
) -> anyhow::Result<Vec<InstalledArtifact>> {
    let mut pending = vec![libraries.to_path_buf()];
    let mut artifacts = Vec::new();
    let mut total = 0_u64;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.path().symlink_metadata()?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "installer emitted a symlink"
            );
            if metadata.is_dir() {
                pending.push(entry.path());
                continue;
            }
            ensure!(
                metadata.is_file() && metadata.len() <= MAX_ARTIFACT_SIZE,
                "invalid installer artifact"
            );
            total += metadata.len();
            ensure!(
                total <= MAX_INSTALL_SIZE && artifacts.len() < 1024,
                "installer output exceeds resource limits"
            );
            let path = entry
                .path()
                .strip_prefix(libraries)?
                .to_str()
                .context("non-UTF8 installer path")?
                .replace('\\', "/");
            safe_relative(&path)?;
            let sha1 = file_digest(&entry.path())?;
            let resource = if let Some(resource) = expected.get(&path) {
                ensure!(
                    resource.size == metadata.len() && resource.sha1 == sha1,
                    "installer changed verified upstream library: {path}"
                );
                resource.clone()
            } else {
                // Generated client patch/mapping files are derived by the verified official installer.
                Resource {
                    sha1,
                    size: metadata.len(),
                    url: String::new(),
                }
            };
            publish_object(&entry.path(), &resource, context)?;
            artifacts.push(InstalledArtifact { path, resource });
        }
    }
    ensure!(!artifacts.is_empty(), "installer produced no libraries");
    for path in expected.keys() {
        ensure!(
            artifacts.iter().any(|artifact| &artifact.path == path),
            "installer removed required library: {path}"
        );
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

fn publish_object(
    source: &Path,
    resource: &Resource,
    context: &ResolveContext,
) -> anyhow::Result<()> {
    let destination = context.store.object_path(&resource.sha1)?;
    if destination.is_file()
        && destination.metadata()?.len() == resource.size
        && file_digest(&destination)? == resource.sha1
    {
        return Ok(());
    }
    let directory = destination.parent().context("missing cache directory")?;
    std::fs::create_dir_all(directory)?;
    let object_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(destination.with_extension("lock"))?;
    fs2::FileExt::try_lock_exclusive(&object_lock)
        .context("this generated object is being prepared elsewhere; retry installation")?;
    if destination.is_file()
        && destination.metadata()?.len() == resource.size
        && file_digest(&destination)? == resource.sha1
    {
        return Ok(());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    std::io::copy(&mut File::open(source)?, &mut temporary)?;
    ensure!(
        file_digest(temporary.path())? == resource.sha1,
        "installer artifact changed during cache publication"
    );
    temporary.persist(destination)?;
    Ok(())
}

fn cached_artifacts_valid(
    installed: &InstalledLoader,
    context: &ResolveContext,
) -> anyhow::Result<bool> {
    if installed.artifacts.is_empty() {
        return Ok(false);
    }
    for artifact in &installed.artifacts {
        safe_relative(&artifact.path)?;
        let path = context.store.object_path(&artifact.resource.sha1)?;
        if !path.is_file()
            || path.metadata()?.len() != artifact.resource.size
            || file_digest(&path)? != artifact.resource.sha1
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn file_digest(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha1::new();
    let mut buffer = vec![0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

async fn copy_object(source: &Path, destination: &Path) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(destination.parent().context("missing library directory")?).await?;
    tokio::fs::copy(source, destination).await?;
    Ok(())
}

fn atomic_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("missing manifest directory")?)?;
    temporary.write_all(&serde_json::to_vec_pretty(value)?)?;
    temporary.persist(path)?;
    Ok(())
}

fn safe_relative(path: &str) -> anyhow::Result<()> {
    ensure!(
        !path.is_empty() && !path.contains(['\\', ':']) && !path.starts_with('/'),
        "unsafe library path: {path}"
    );
    ensure!(
        Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "unsafe library path: {path}"
    );
    ensure!(
        path.split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".."),
        "unsafe library path: {path}"
    );
    Ok(())
}

/// Preserve `NeoForge`'s canonical Maven module paths in an ordinary execution plan.
///
/// # Errors
/// Rejects mismatched targets, unsafe paths, duplicate resources, or missing library copies.
pub fn adapt_plan(
    plan: &mut ExecutionPlan,
    manifest: &VersionManifest,
    installed: &InstalledLoader,
) -> anyhow::Result<()> {
    adapt_plan_kind(plan, manifest, installed, false)
}

pub(crate) fn adapt_plan_kind(
    plan: &mut ExecutionPlan,
    manifest: &VersionManifest,
    installed: &InstalledLoader,
    forge: bool,
) -> anyhow::Result<()> {
    if forge {
        super::forge::validate_version(&installed.minecraft_version, &installed.version)?;
    }
    ensure!(
        (forge
            || (version_details(&installed.version)
                .is_some_and(|(minecraft, _)| minecraft == installed.minecraft_version)
                && manifest.id == manifest_id(&installed.version)))
            && plan.version == manifest.id
            && plan.manifest_sha1 == installed.manifest_sha1,
        "Loader installation/plan mismatch"
    );
    let kind = if forge { "forge" } else { "neoforge" };
    let library_root = format!("views/{kind}/{}/libraries", plan.manifest_sha1);
    let mut mappings = BTreeMap::new();
    let descriptors: BTreeMap<_, _> = manifest
        .libraries
        .iter()
        .filter_map(|library| library.downloads.artifact.as_ref())
        .map(|artifact| {
            (
                (
                    artifact.sha1.as_str(),
                    artifact.path.rsplit('/').next().unwrap_or_default(),
                ),
                artifact.path.as_str(),
            )
        })
        .collect();
    for action in &mut plan.actions {
        if let PrepareAction::Copy { sha1, path, .. } = action {
            if path.starts_with("views/libraries/") {
                let name = path.rsplit('/').next().context("empty library filename")?;
                let filename = name
                    .strip_prefix(&format!("{sha1}-"))
                    .context("unexpected compiled library filename")?;
                // Compatibility overrides may intentionally replace a base native artifact.
                if let Some(relative) = descriptors.get(&(sha1.as_str(), filename)) {
                    safe_relative(relative)?;
                    let replacement = format!("{library_root}/{relative}");
                    mappings.insert(path.clone(), replacement.clone());
                    *path = replacement;
                }
            } else if path.starts_with("views/clients/") {
                let parent = path
                    .rsplit_once('/')
                    .context("client path has no parent")?
                    .0;
                let replacement = format!("{parent}/{}.jar", manifest.id);
                mappings.insert(path.clone(), replacement.clone());
                *path = replacement;
            }
        }
    }
    for path in &mut plan.command.classpath {
        if let Some(replacement) = mappings.get(path) {
            *path = replacement.clone();
        }
    }
    for artifact in &installed.artifacts {
        safe_relative(&artifact.path)?;
        let path = format!("{library_root}/{}", artifact.path);
        if let Some(existing) = plan
            .resources
            .iter()
            .find(|resource| resource.sha1 == artifact.resource.sha1)
        {
            ensure!(
                existing.size == artifact.resource.size,
                "conflicting installed library sizes"
            );
        } else {
            plan.resources.push(artifact.resource.clone());
        }
        if let Some(PrepareAction::Copy { sha1, .. }) = plan.actions.iter().find(|action| matches!(action, PrepareAction::Copy { path: existing, .. } if *existing == path)) {
            ensure!(*sha1 == artifact.resource.sha1, "conflicting installed artifact at {path}");
        } else {
            plan.actions.push(PrepareAction::Copy { sha1: artifact.resource.sha1.clone(), path, executable: false });
        }
    }
    plan.command
        .path_variables
        .insert("library_directory".into(), library_root);
    let separator = if plan.target.os == crate::platform::OperatingSystem::Windows {
        ";"
    } else {
        ":"
    };
    plan.command
        .variables
        .insert("classpath_separator".into(), separator.into());
    Ok(())
}

#[cfg(test)]
#[path = "neoforge_tests.rs"]
mod tests;
