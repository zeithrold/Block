//! User-selected OptiFine archives, automatic patching and standalone launch metadata.

use std::{
    collections::BTreeSet,
    fs::File,
    io::{Cursor, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context as _, ensure};
use block_download::Resource;
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

use crate::{
    catalog::LoaderKind,
    manifest::model::{Argument, Artifact, Library, LibraryDownloads, VersionManifest},
    resolver::ResolveContext,
};

const MAX_JAR: u64 = 128 * 1024 * 1024;

/// Identity read from the archive, rather than its user-controlled filename.
/// This validates consistency, not a publisher signature. The user obtains the JAR from optifine.net.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptiFineJar {
    pub minecraft: String,
    pub version: String,
    pub launchwrapper: String,
    pub forge: Option<String>,
    pub resource: Resource,
}

impl OptiFineJar {
    /// Check the selected game and the compatibility declared in this release's changelog.
    pub fn validate_for(
        &self,
        minecraft: &str,
        loader: &LoaderKind,
        loader_version: Option<&str>,
    ) -> anyhow::Result<()> {
        ensure!(
            self.minecraft == minecraft,
            "This OptiFine JAR is for Minecraft {}",
            self.minecraft
        );
        match loader {
            LoaderKind::Vanilla => Ok(()),
            LoaderKind::Forge => {
                let supported = self
                    .forge
                    .as_deref()
                    .context("This OptiFine release does not declare Forge compatibility")?;
                let selected = loader_version.context("Select a Forge build first")?;
                let selected = selected
                    .strip_prefix(&format!("{minecraft}-"))
                    .unwrap_or(selected);
                ensure!(
                    selected == supported,
                    "This OptiFine release declares compatibility with Forge {supported}; select that build"
                );
                Ok(())
            }
            _ => anyhow::bail!(
                "OptiFine cannot be combined directly with {}",
                loader.name()
            ),
        }
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
        && value != "."
        && value != ".."
}

/// Whether a changelog token can be a Forge version rather than an issue reference.
fn forge_version_like(value: &str) -> bool {
    value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || b".-_+".contains(&byte))
}

fn entry(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
    limit: u64,
) -> anyhow::Result<Vec<u8>> {
    let mut file = zip
        .by_name(name)
        .with_context(|| format!("OptiFine archive is missing {name}"))?;
    ensure!(
        file.size() <= limit,
        "OptiFine archive member is too large: {name}"
    );
    let mut bytes = Vec::new();
    file.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "OptiFine archive member exceeds its limit"
    );
    Ok(bytes)
}

fn class_strings(bytes: &[u8]) -> anyhow::Result<Vec<String>> {
    ensure!(
        bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe]),
        "Invalid OptiFine identity class"
    );
    let mut input = bytes.get(8..).context("Truncated OptiFine class")?;
    let take = |input: &mut &[u8], count: usize| -> anyhow::Result<Vec<u8>> {
        let value = input
            .get(..count)
            .context("Truncated OptiFine constant pool")?
            .to_vec();
        *input = &input[count..];
        Ok(value)
    };
    let count = take(&mut input, 2)?;
    let count = u16::from_be_bytes([count[0], count[1]]);
    let mut index = 1;
    let mut strings = Vec::new();
    while index < count {
        let tag = take(&mut input, 1)?[0];
        let length = match tag {
            1 => {
                let length = take(&mut input, 2)?;
                let value = take(
                    &mut input,
                    usize::from(u16::from_be_bytes([length[0], length[1]])),
                )?;
                if value.starts_with(b"OptiFine_") {
                    strings
                        .push(String::from_utf8(value).context("Invalid OptiFine version string")?);
                }
                0
            }
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 4,
            5 | 6 => {
                index += 1;
                8
            }
            7 | 8 | 16 | 19 | 20 => 2,
            15 => 3,
            _ => anyhow::bail!("Invalid OptiFine class constant tag"),
        };
        take(&mut input, length)?;
        index += 1;
    }
    strings.sort();
    strings.dedup();
    Ok(strings)
}

fn inspect_bytes(bytes: &[u8]) -> anyhow::Result<OptiFineJar> {
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_JAR,
        "Invalid OptiFine JAR size"
    );
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).context("Select an OptiFine installer JAR")?;
    ensure!(zip.len() <= 65536, "Too many OptiFine archive members");
    let mut names = BTreeSet::new();
    let mut expanded = 0_u64;
    for index in 0..zip.len() {
        let member = zip.by_index(index)?;
        ensure!(
            names.insert(member.name().to_owned()),
            "Duplicate OptiFine archive member"
        );
        ensure!(
            member.enclosed_name().is_some(),
            "Unsafe OptiFine archive member"
        );
        expanded = expanded
            .checked_add(member.size())
            .context("Invalid OptiFine archive size")?;
        ensure!(
            member.size() <= MAX_JAR && expanded <= 1024 * 1024 * 1024,
            "OptiFine archive expands beyond its limit"
        );
    }
    let manifest = String::from_utf8(entry(&mut zip, "META-INF/MANIFEST.MF", 65536)?)?;
    ensure!(
        manifest
            .lines()
            .any(|line| line.trim() == "Main-Class: optifine.InstallerFrame"),
        "This is not an OptiFine installer archive"
    );
    for name in [
        "optifine/Installer.class",
        "optifine/Patcher.class",
        "optifine/OptiFineTweaker.class",
    ] {
        ensure!(
            entry(&mut zip, name, 4 * 1024 * 1024)?.starts_with(&[0xca, 0xfe, 0xba, 0xbe]),
            "Invalid OptiFine installer class"
        );
    }
    let config = [
        "net/optifine/Config.class",
        "notch/net/optifine/Config.class",
        "Config.class",
        "VersionThread.class",
    ]
    .into_iter()
    .find(|name| names.contains(*name))
    .context("OptiFine identity class is missing")?;
    let versions = class_strings(&entry(&mut zip, config, 4 * 1024 * 1024)?)?;
    ensure!(
        versions.len() == 1,
        "OptiFine archive has an ambiguous version identity"
    );
    let version = versions[0]
        .strip_prefix("OptiFine_")
        .context("Invalid OptiFine version")?
        .to_owned();
    ensure!(identifier(&version), "Invalid OptiFine release identity");
    let (minecraft, edition) = version
        .split_once('_')
        .context("Invalid OptiFine release identity")?;
    ensure!(
        edition.starts_with("HD_") && identifier(minecraft),
        "Invalid OptiFine game/edition identity"
    );
    let changelog = String::from_utf8(entry(&mut zip, "changelog.txt", 4 * 1024 * 1024)?)?;
    let mut release = changelog.lines();
    ensure!(
        release.next().map(str::trim) == Some(format!("OptiFine {version}").as_str()),
        "OptiFine changelog does not match its identity"
    );
    // Some changelogs reference a Forge issue ("compatible with Forge #2847")
    // instead of declaring a version. Only a version-looking token is a
    // compatibility declaration; an issue reference must not fail the import.
    let forge = release
        .take_while(|line| !line.trim().is_empty())
        .find_map(|line| {
            line.trim()
                .strip_prefix("- compatible with Forge ")
                .map(str::trim)
                .filter(|value| forge_version_like(value))
                .map(str::to_owned)
        });
    ensure!(
        forge.as_ref().is_none_or(|version| identifier(version)),
        "Invalid OptiFine Forge compatibility"
    );
    let launchwrapper = String::from_utf8(entry(&mut zip, "launchwrapper-of.txt", 128)?)?
        .trim()
        .to_owned();
    ensure!(
        !launchwrapper.is_empty()
            && launchwrapper
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.'),
        "Invalid OptiFine LaunchWrapper version"
    );
    let wrapper = entry(
        &mut zip,
        &format!("launchwrapper-of-{launchwrapper}.jar"),
        8 * 1024 * 1024,
    )?;
    let mut wrapper = zip::ZipArchive::new(Cursor::new(wrapper))?;
    ensure!(
        wrapper
            .by_name("net/minecraft/launchwrapper/Launch.class")?
            .size()
            > 0,
        "Invalid embedded OptiFine LaunchWrapper"
    );
    let sha1 = format!("{:x}", Sha1::digest(bytes));
    Ok(OptiFineJar {
        minecraft: minecraft.to_owned(),
        version,
        launchwrapper,
        forge,
        resource: Resource {
            size: bytes.len() as u64,
            url: format!("block-import:{sha1}"),
            sha1,
        },
    })
}

/// Inspect a bounded local JAR without executing it or trusting its filename.
pub fn inspect(path: &Path) -> anyhow::Result<OptiFineJar> {
    ensure!(
        path.is_file() && path.metadata()?.len() <= MAX_JAR,
        "Select an OptiFine installer JAR no larger than 128 MiB"
    );
    inspect_bytes(&std::fs::read(path)?)
}

fn publish(bytes: &[u8], context: &ResolveContext) -> anyhow::Result<Resource> {
    ensure!(
        !context.cancel.is_cancelled(),
        "OptiFine installation cancelled"
    );
    let sha1 = format!("{:x}", Sha1::digest(bytes));
    let resource = Resource {
        size: bytes.len() as u64,
        url: format!("block-import:{sha1}"),
        sha1,
    };
    let destination = context.store.object_path(&resource.sha1)?;
    let parent = destination
        .parent()
        .context("Missing imported-object directory")?;
    std::fs::create_dir_all(parent)?;
    crate::executor::ensure_write_path(context.store.root(), &destination)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(destination.with_extension("lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("This imported file is being prepared elsewhere; retry")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(destination)?;
    Ok(resource)
}

/// Import the selected archive into the private cache. No network request or redistribution occurs.
pub fn import(path: &Path, context: &ResolveContext) -> anyhow::Result<OptiFineJar> {
    ensure!(
        path.is_file() && path.metadata()?.len() <= MAX_JAR,
        "Select an OptiFine installer JAR no larger than 128 MiB"
    );
    let bytes = std::fs::read(path)?;
    let identity = inspect_bytes(&bytes)?;
    publish(&bytes, context)?;
    Ok(identity)
}

fn local_library(name: String, filename: String, resource: &Resource) -> Library {
    Library {
        name,
        downloads: LibraryDownloads {
            artifact: Some(Artifact {
                path: filename,
                sha1: resource.sha1.clone(),
                size: resource.size,
                url: resource.url.clone(),
            }),
            classifiers: None,
        },
        natives: None,
        extract: None,
        rules: None,
    }
}

/// Place only the verified Forge mod while the caller holds the game-directory guard.
/// Existing different content remains untouched and requires the user to resolve the conflict.
pub(crate) fn place_mod(
    directory: &Path,
    selected: &OptiFineJar,
    resource: &Resource,
    source: &Path,
) -> anyhow::Result<()> {
    ensure!(
        identifier(&selected.version),
        "Invalid OptiFine mod filename"
    );
    let destination = directory
        .join("mods")
        .join(format!("OptiFine-{}.jar", selected.version));
    crate::executor::ensure_write_path(directory, &destination)?;
    let bytes = std::fs::read(source)?;
    ensure!(
        bytes.len() as u64 == resource.size
            && format!("{:x}", Sha1::digest(&bytes)) == resource.sha1,
        "OptiFine mod changed before launch; choose Repair"
    );
    if destination.exists() {
        ensure!(
            std::fs::read(&destination)? == bytes,
            "A different file already occupies {}. Keep it or move it before launching; Block will not overwrite it.",
            destination.display()
        );
        return Ok(());
    }
    let parent = destination.parent().context("Missing mods directory")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.persist_noclobber(destination)?;
    Ok(())
}

/// Verify the original archive for Forge, preserving its runtime patch metadata.
/// Forge's transformation service consumes the installer JAR directly; a standalone
/// patched library would introduce duplicate Minecraft packages into its module layer.
pub async fn forge_mod(
    selected: &OptiFineJar,
    minecraft: &str,
    forge: Option<&str>,
    context: &ResolveContext,
) -> anyhow::Result<Resource> {
    selected.validate_for(minecraft, &LoaderKind::Forge, forge)?;
    let path = context
        .store
        .fetch(&selected.resource, &context.cancel)
        .await?;
    ensure!(
        inspect(&path)? == *selected,
        "OptiFine archive changed; select the official JAR again"
    );
    Ok(selected.resource.clone())
}

/// Patch the verified official client using the selected archive's own headless Patcher entry point.
/// The output is the privately cached standalone runtime library.
pub async fn patch(
    base: &VersionManifest,
    selected: &OptiFineJar,
    java: &Path,
    context: &ResolveContext,
) -> anyhow::Result<Resource> {
    ensure!(
        selected.minecraft == base.id,
        "OptiFine does not match the selected Minecraft version"
    );
    let imported = context
        .store
        .fetch(&selected.resource, &context.cancel)
        .await?;
    ensure!(
        inspect(&imported)? == *selected,
        "The selected OptiFine archive changed; select the official JAR again"
    );
    let client = context
        .store
        .fetch(
            &Resource {
                sha1: base.downloads.client.sha1.clone(),
                size: base.downloads.client.size,
                url: base.downloads.client.url.clone(),
            },
            &context.cancel,
        )
        .await?;
    let root = context.store.root().canonicalize()?;
    let stage = root
        .join("installer-staging")
        .join(uuid::Uuid::new_v4().to_string());
    crate::executor::ensure_write_path(&root, &stage)?;
    std::fs::create_dir_all(&stage)?;
    let output = stage.join("OptiFine-mod.jar");
    let log = File::create(stage.join("installer.log"))?;
    let os = crate::platform::Target::host()?.os;
    let mut command = tokio::process::Command::new(crate::command::java_path(java.to_owned(), os)?);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    command
        .arg("-Djava.awt.headless=true")
        .arg("-cp")
        .arg(crate::command::java_path(imported, os)?)
        .arg("optifine.Patcher");
    for path in [
        client,
        context.store.object_path(&selected.resource.sha1)?,
        output.clone(),
    ] {
        command.arg(crate::command::java_path(path, os)?);
    }
    let mut child = command
        .current_dir(&stage)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .context("Could not start the OptiFine patcher")?;
    let status = tokio::select! { biased;
        () = context.cancel.cancelled() => { child.kill().await?; let _ = child.wait().await; anyhow::bail!("OptiFine installation cancelled"); }
        result = tokio::time::timeout(Duration::from_secs(120), child.wait()) => {
            match result { Ok(status) => status?, Err(_) => { child.kill().await?; let _ = child.wait().await; anyhow::bail!("OptiFine patching timed out; diagnostic directory: {}", stage.display()); } }
        }
    };
    ensure!(
        status.success(),
        "OptiFine patcher failed; diagnostic directory: {}",
        stage.display()
    );
    ensure!(
        output.is_file() && output.metadata()?.len() <= MAX_JAR,
        "Invalid OptiFine patch output"
    );
    let bytes = std::fs::read(output)?;
    let identity = inspect_bytes(&bytes)?;
    ensure!(
        identity.version == selected.version && identity.minecraft == base.id,
        "Patched OptiFine identity does not match the selected release"
    );
    let zip = zip::ZipArchive::new(Cursor::new(bytes.as_slice()))?;
    ensure!(
        !zip.file_names()
            .any(|name| name.starts_with("patch/") && name.ends_with(".xdelta")),
        "OptiFine patch output is incomplete"
    );
    publish(&bytes, context)
}

/// Automatically prepare a standalone OptiFine launch manifest with content-verified local artifacts.
pub async fn install(
    base: &VersionManifest,
    selected: &OptiFineJar,
    java: &Path,
    context: &ResolveContext,
) -> anyhow::Result<PathBuf> {
    selected.validate_for(&base.id, &LoaderKind::Vanilla, None)?;
    let patched = patch(base, selected, java, context).await?;
    let path = context
        .store
        .fetch(&selected.resource, &context.cancel)
        .await?;
    let bytes = std::fs::read(path)?;
    let wrapper = entry(
        &mut zip::ZipArchive::new(Cursor::new(bytes.as_slice()))?,
        &format!("launchwrapper-of-{}.jar", selected.launchwrapper),
        8 * 1024 * 1024,
    )?;
    let wrapper = publish(&wrapper, context)?;
    let mut manifest = base.clone();
    manifest.id = format!("{}-OptiFine", selected.version);
    manifest.main_class = "net.minecraft.launchwrapper.Launch".into();
    match &mut manifest.arguments {
        Some(arguments) => arguments.game.extend([
            Argument::Literal("--tweakClass".into()),
            Argument::Literal("optifine.OptiFineTweaker".into()),
        ]),
        None => {
            let arguments = manifest
                .minecraft_arguments
                .as_mut()
                .context("Minecraft launch arguments are missing")?;
            arguments.push_str(" --tweakClass optifine.OptiFineTweaker");
        }
    }
    manifest.libraries.insert(
        0,
        local_library(
            format!("optifine:launchwrapper-of:{}", selected.launchwrapper),
            format!(
                "optifine/launchwrapper-of/{0}/launchwrapper-of-{0}.jar",
                selected.launchwrapper
            ),
            &wrapper,
        ),
    );
    manifest.libraries.insert(
        0,
        local_library(
            format!("optifine:OptiFine:{}", selected.version),
            format!("optifine/OptiFine/{0}/OptiFine-{0}.jar", selected.version),
            &patched,
        ),
    );
    let directory = context
        .store
        .root()
        .join("loaders/optifine")
        .join(&selected.resource.sha1);
    crate::executor::ensure_write_path(context.store.root(), &directory)?;
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("manifest.json");
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    temporary.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    temporary.persist(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn archive(version: &str, changelog: &str, forged_config: bool) -> anyhow::Result<Vec<u8>> {
        let mut class = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52, 0, 2, 1];
        let identity = format!("OptiFine_{version}");
        class.extend(u16::try_from(identity.len())?.to_be_bytes());
        class.extend(identity.as_bytes());
        if forged_config {
            class[0] = 0;
        }
        let mut embedded = zip::ZipWriter::new(Cursor::new(Vec::new()));
        embedded.start_file(
            "net/minecraft/launchwrapper/Launch.class",
            zip::write::SimpleFileOptions::default(),
        )?;
        embedded.write_all(&[0xca, 0xfe, 0xba, 0xbe])?;
        let embedded = embedded.finish()?.into_inner();
        let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            (
                "META-INF/MANIFEST.MF",
                b"Manifest-Version: 1.0\r\nMain-Class: optifine.InstallerFrame\r\n".as_slice(),
            ),
            ("optifine/Installer.class", &[0xca, 0xfe, 0xba, 0xbe]),
            ("optifine/Patcher.class", &[0xca, 0xfe, 0xba, 0xbe]),
            ("optifine/OptiFineTweaker.class", &[0xca, 0xfe, 0xba, 0xbe]),
            ("notch/net/optifine/Config.class", class.as_slice()),
            ("changelog.txt", changelog.as_bytes()),
            ("launchwrapper-of.txt", b"2.3"),
            ("launchwrapper-of-2.3.jar", embedded.as_slice()),
        ] {
            output.start_file(name, zip::write::SimpleFileOptions::default())?;
            output.write_all(bytes)?;
        }
        Ok(output.finish()?.into_inner())
    }

    #[test]
    fn forge_issue_reference_is_not_a_compatibility_version() -> anyhow::Result<()> {
        let bytes = archive(
            "1.12.2_HD_U_G5",
            "OptiFine 1.12.2_HD_U_G5\r\n - compatible with Forge #2847\r\n\r\nOptiFine 1.12.2_HD_U_G4\r\n",
            false,
        )?;
        let identity = inspect_bytes(&bytes)?;
        assert_eq!(identity.minecraft, "1.12.2");
        assert!(identity.forge.is_none());
        match identity.validate_for("1.12.2", &LoaderKind::Forge, Some("1.12.2-14.23.5.2864")) {
            Ok(()) => anyhow::bail!("an undeclared Forge version must stay unsupported"),
            Err(error) => assert!(
                error
                    .to_string()
                    .contains("does not declare Forge compatibility"),
                "{error}"
            ),
        }
        Ok(())
    }

    #[test]
    fn optifine_identity_and_current_release_compatibility_ignore_filename_and_old_releases()
    -> anyhow::Result<()> {
        let bytes = archive(
            "1.21.1_HD_U_J1",
            "OptiFine 1.21.1_HD_U_J1\r\n - compatible with Forge 52.0.16\r\n\r\nOptiFine 1.20.4_HD_U_I8\r\n - compatible with Forge 49.0.31\r\n",
            false,
        )?;
        let identity = inspect_bytes(&bytes)?;
        assert_eq!(identity.minecraft, "1.21.1");
        assert_eq!(identity.forge.as_deref(), Some("52.0.16"));
        identity.validate_for("1.21.1", &LoaderKind::Vanilla, None)?;
        identity.validate_for("1.21.1", &LoaderKind::Forge, Some("1.21.1-52.0.16"))?;
        for (game, loader, version) in [
            ("1.21", LoaderKind::Vanilla, None),
            ("1.21.1", LoaderKind::Forge, None),
            ("1.21.1", LoaderKind::Forge, Some("1.21.1-52.1.20")),
            ("1.21.1", LoaderKind::NeoForge, Some("21.1.219")),
            ("1.21.1", LoaderKind::Fabric, Some("0.16.14")),
            ("1.21.1", LoaderKind::Quilt, Some("0.29.2")),
        ] {
            assert!(identity.validate_for(game, &loader, version).is_err());
        }
        let old_only = archive(
            "1.21.1_HD_U_J1",
            "OptiFine 1.21.1_HD_U_J1\n - not compatible with Forge\n\nOptiFine 1.20.4_HD_U_I8\n - compatible with Forge 49.0.31\n",
            false,
        )?;
        assert!(inspect_bytes(&old_only)?.forge.is_none());
        Ok(())
    }

    #[test]
    fn optifine_rejects_invalid_conflicting_and_truncated_release_identity() -> anyhow::Result<()> {
        for bytes in [
            archive("1.21.1_HD_U_J1", "OptiFine 1.20.4_HD_U_I8\n", false)?,
            archive("1.21.1_HD_U_J1", "OptiFine 1.21.1_HD_U_J1\n", true)?,
            b"not a JAR".to_vec(),
        ] {
            assert!(inspect_bytes(&bytes).is_err());
        }
        for bytes in [
            vec![0xca, 0xfe, 0xba, 0xbe],
            vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52, 0, 2, 1, 0, 20],
        ] {
            assert!(class_strings(&bytes).is_err());
        }
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("OptiFine_1.21.1_HD_U_J1.jar");
        std::fs::write(&path, b"arbitrary renamed content")?;
        assert!(inspect(&path).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn optifine_private_import_reopens_offline_rejects_corruption_and_honors_cancel()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let context = ResolveContext {
            store: crate::Store::new(root.path().to_owned(), false, Arc::new(|_| {}))?,
            cancel: crate::CancellationToken::new(),
        };
        let bytes = archive(
            "1.21.1_HD_U_J1",
            "OptiFine 1.21.1_HD_U_J1\n - compatible with Forge 52.0.16\n",
            false,
        )?;
        let path = root.path().join("selected.jar");
        std::fs::write(&path, &bytes)?;
        let selected = import(&path, &context)?;
        assert_eq!(
            forge_mod(&selected, "1.21.1", Some("1.21.1-52.0.16"), &context).await?,
            selected.resource
        );
        assert!(
            forge_mod(&selected, "1.21.1", Some("1.21.1-52.1.20"), &context)
                .await
                .is_err()
        );
        std::fs::remove_file(&path)?;
        let reopened = crate::Store::new(root.path().to_owned(), true, Arc::new(|_| {}))?;
        let cached = reopened.fetch(&selected.resource, &context.cancel).await?;
        assert_eq!(inspect(&cached)?, selected);
        std::fs::write(&cached, b"changed")?;
        let error = context
            .store
            .fetch(&selected.resource, &context.cancel)
            .await
            .err()
            .context("Corrupt import was accepted")?;
        assert!(error.to_string().contains("Select the official JAR again"));
        std::fs::write(&path, bytes)?;
        context.cancel.cancel();
        assert!(import(&path, &context).is_err());
        assert_eq!(std::fs::read(cached)?, b"changed");
        Ok(())
    }

    #[test]
    fn optifine_catalog_identity_persists_and_mod_placement_preserves_conflicting_files()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let selected = inspect_bytes(&archive(
            "1.21.1_HD_U_J1",
            "OptiFine 1.21.1_HD_U_J1\n - compatible with Forge 52.0.16\n",
            false,
        )?)?;
        let catalog = crate::catalog::Catalog::open(root.path())?;
        let (instance, profile) = catalog.create_instance_with_components(
            "OptiFine game".into(),
            "1.21.1".into(),
            None,
            None,
            Some(selected.clone()),
            crate::catalog::ProfileConfiguration {
                name: "OptiFine profile".into(),
                ..Default::default()
            },
        )?;
        let state = crate::catalog::Catalog::open(root.path())?.snapshot()?;
        let (_, _, target) = state.selection(&profile.id)?;
        assert_eq!(target.optifine, Some(selected.clone()));
        let mut changed = target.clone();
        changed.optifine = None;
        assert!(catalog.set_target(&instance.id, &changed).is_err());
        let before = catalog.snapshot()?.instances.len();
        assert!(
            catalog
                .create_instance_with_components(
                    "Incompatible".into(),
                    "1.20.6".into(),
                    None,
                    None,
                    Some(selected.clone()),
                    crate::catalog::ProfileConfiguration::default()
                )
                .is_err()
        );
        assert_eq!(catalog.snapshot()?.instances.len(), before);
        let bytes = b"verified synthetic generated mod";
        let source = root.path().join("generated.jar");
        std::fs::write(&source, bytes)?;
        let resource = Resource {
            sha1: format!("{:x}", Sha1::digest(bytes)),
            size: bytes.len() as u64,
            url: "block-import:synthetic".into(),
        };
        let game = root.path().join("test-game");
        std::fs::create_dir_all(&game)?;
        place_mod(&game, &selected, &resource, &source)?;
        place_mod(&game, &selected, &resource, &source)?;
        let installed = game.join("mods/OptiFine-1.21.1_HD_U_J1.jar");
        assert_eq!(std::fs::read(&installed)?, bytes);
        std::fs::write(&installed, b"existing user mod")?;
        assert!(place_mod(&game, &selected, &resource, &source).is_err());
        assert_eq!(std::fs::read(&installed)?, b"existing user mod");
        Ok(())
    }
}
