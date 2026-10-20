//! Install the representative `NeoForge` 20.6 client using its verified official installer.
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

/// The supported, tested representative. Other generations require their own validation.
pub const VERSION: &str = "20.6.144";
pub const MINECRAFT_VERSION: &str = "1.20.6";
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
struct Overlay {
    id: String,
    inherits_from: String,
    main_class: String,
    arguments: OverlayArguments,
    libraries: Vec<Library>,
}

#[derive(Debug, Deserialize)]
struct OverlayArguments {
    #[serde(default)]
    game: Vec<Argument>,
    #[serde(default)]
    jvm: Vec<Argument>,
}

#[derive(Debug, Deserialize)]
struct InstallerProfile {
    minecraft: String,
    libraries: Vec<Library>,
}

/// Install `NeoForge` into the shared immutable cache, leaving mutable game data alone.
///
/// `java` must be the already prepared native Java 21 executable for this host.
/// Failed attempts retain their isolated staging directory and installer log for
/// diagnosis. Retrying creates a new stage and cannot overwrite a valid installation.
///
/// # Errors
/// Fails for an unsupported game version, invalid upstream artifacts, unavailable
/// Java, cancellation, installer failure, or unsafe/generated library paths.
pub async fn install(
    base: &VersionManifest,
    java: &Path,
    context: &ResolveContext,
) -> anyhow::Result<InstalledLoader> {
    ensure!(
        base.id == MINECRAFT_VERSION,
        "NeoForge {VERSION} requires Minecraft {MINECRAFT_VERSION}"
    );
    ensure!(!context.cancel.is_cancelled(), "installation cancelled");
    let root = context.store.root().canonicalize()?;
    let destination = root.join("loaders/neoforge").join(VERSION);
    tokio::fs::create_dir_all(&destination).await?;
    let _installation_lock = acquire_installation_lock(&destination, &context.cancel).await?;
    let receipt = destination.join("installed.json");
    if let Ok(bytes) = tokio::fs::read(&receipt).await
        && let Ok(installed) = serde_json::from_slice::<InstalledLoader>(&bytes)
        && installed.version == VERSION
        && installed.minecraft_version == base.id
        && installed.manifest_path == destination.join("manifest.json")
        && installed.manifest_path.is_file()
        && file_digest(&installed.manifest_path)? == installed.manifest_sha1
        && cached_artifacts_valid(&installed, context)?
    {
        return Ok(installed);
    }
    let installer_url = format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{VERSION}/neoforge-{VERSION}-installer.jar"
    );
    let installer_resource = context.store.describe_maven(&installer_url).await?;
    let archive_path = context
        .store
        .fetch(&installer_resource, &context.cancel)
        .await?;
    let (overlay, profile) = read_installer(&archive_path)?;
    ensure!(
        profile.minecraft == base.id && overlay.inherits_from == base.id,
        "installer game version does not match selected version"
    );
    ensure!(
        overlay.id == format!("neoforge-{VERSION}"),
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
                "NeoForge installation failed; diagnostic directory: {}",
                stage.display()
            )
        })?;
    tokio::fs::create_dir_all(&destination).await?;
    let manifest_path = destination.join("manifest.json");
    atomic_json(&manifest_path, &result.0)?;
    let installed = InstalledLoader {
        version: VERSION.into(),
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
            () = cancel.cancelled() => anyhow::bail!("installation cancelled while waiting for NeoForge installer lock"),
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
                resource.url.starts_with("https://"),
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
    let resources: Vec<_> = expected.values().cloned().collect();
    context.store.fetch_all(&resources, &context.cancel).await?;
    for (path, resource) in &expected {
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
    run_installer(java, installer, stage, &context.cancel).await?;
    let artifacts = import_libraries(&libraries, &expected, context)?;
    for required in [
        "net/neoforged/neoforge/20.6.144/neoforge-20.6.144-client.jar",
        "net/minecraft/client/1.20.6-20240627.102356/client-1.20.6-20240627.102356-extra.jar",
    ] {
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

async fn run_installer(
    java: &Path,
    installer: &Path,
    stage: &Path,
    cancel: &crate::CancellationToken,
) -> anyhow::Result<()> {
    let home = stage.join("home");
    tokio::fs::create_dir_all(&home).await?;
    let log_path = stage.join("installer.log");
    let log = File::create(&log_path)?;
    let mut child = tokio::process::Command::new(java)
        .arg("-Djava.awt.headless=true")
        .arg(format!("-Duser.home={}", home.display()))
        .arg("-jar")
        .arg(installer)
        .arg("--installClient")
        .arg(stage)
        .current_dir(stage)
        .env_remove("MICROSOFT_CLIENT_SECRET")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .context("cannot start the prepared Java runtime for NeoForge installer")?;
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

fn read_zip_json<T: serde::de::DeserializeOwned>(
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
    let arguments = merged
        .arguments
        .as_mut()
        .context("NeoForge requires modern game arguments")?;
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
    ensure!(
        installed.version == VERSION
            && installed.minecraft_version == MINECRAFT_VERSION
            && manifest.id == format!("neoforge-{VERSION}")
            && plan.version == manifest.id
            && plan.manifest_sha1 == installed.manifest_sha1,
        "NeoForge installation/plan mismatch"
    );
    let library_root = format!("views/neoforge/{}/libraries", plan.manifest_sha1);
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
