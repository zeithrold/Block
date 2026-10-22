//! Official Fabric v2 and Quilt v3 catalogs and verified client installations.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use anyhow::{Context as _, ensure};
use block_download::Resource;
use minecraft_manifest::model::{
    Argument, Arguments, Artifact, Library, LibraryDownloads, VersionManifest,
};
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

use crate::{catalog::LoaderKind, resolver::ResolveContext};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
}

#[derive(Deserialize)]
struct VersionEntry {
    loader: LoaderEntry,
    intermediary: Mapping,
}

#[derive(Deserialize)]
struct LoaderEntry {
    version: String,
    stable: Option<bool>,
}

#[derive(Deserialize)]
struct Mapping {
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Overlay {
    id: String,
    inherits_from: String,
    main_class: String,
    #[serde(default)]
    arguments: OverlayArguments,
    libraries: Vec<OverlayLibrary>,
}

#[derive(Default, Deserialize)]
struct OverlayArguments {
    #[serde(default)]
    game: Vec<Argument>,
    #[serde(default)]
    jvm: Vec<Argument>,
}

#[derive(Deserialize)]
struct OverlayLibrary {
    name: String,
    url: String,
    sha1: Option<String>,
    size: Option<u64>,
}

fn endpoint(kind: &LoaderKind) -> anyhow::Result<&'static str> {
    match kind {
        LoaderKind::Fabric => Ok("https://meta.fabricmc.net/v2/"),
        LoaderKind::Quilt => Ok("https://meta.quiltmc.org/v3/"),
        _ => anyhow::bail!("Expected a Fabric or Quilt loader"),
    }
}

pub(crate) fn validate_version(version: &str) -> anyhow::Result<()> {
    ensure!(
        !version.is_empty()
            && version.len() <= 128
            && version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-_+".contains(&byte))
            && version != "."
            && version != "..",
        "Invalid loader version"
    );
    Ok(())
}

fn url(origin: &str, minecraft: &str, version: Option<&str>) -> anyhow::Result<reqwest::Url> {
    ensure!(
        !minecraft.is_empty() && minecraft.len() <= 80 && !minecraft.chars().any(char::is_control),
        "Invalid Minecraft version"
    );
    let mut url = reqwest::Url::parse(origin)?.join("versions/loader/")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| anyhow::anyhow!("Invalid loader API URL"))?;
        segments.pop_if_empty().push(minecraft);
        if let Some(version) = version {
            validate_version(version)?;
            segments.push(version).push("profile").push("json");
        }
    }
    Ok(url)
}

async fn metadata(url: &reqwest::Url, context: &ResolveContext) -> anyhow::Result<Vec<u8>> {
    let key = format!("loader-{:x}.json", Sha1::digest(url.as_str().as_bytes()));
    tokio::select! { biased;
        () = context.cancel.cancelled() => anyhow::bail!("Loader request cancelled"),
        result = tokio::time::timeout(Duration::from_secs(45), context.store.metadata_pinned(&key, url.as_str())) => {
            let bytes = result.context("Official loader metadata timed out")?
                .context("Could not read official loader metadata; retry or choose another build")?;
            ensure!(bytes.len() <= 4 * 1024 * 1024, "Loader metadata is too large");
            Ok(bytes)
        }
    }
}

/// List builds reported compatible by the loader's official API for this game.
/// The caller owns cancellation and must discard results from superseded drafts.
pub async fn versions(
    kind: &LoaderKind,
    minecraft: &str,
    context: &ResolveContext,
) -> anyhow::Result<Vec<LoaderVersion>> {
    versions_from(endpoint(kind)?, minecraft, context).await
}

async fn versions_from(
    origin: &str,
    minecraft: &str,
    context: &ResolveContext,
) -> anyhow::Result<Vec<LoaderVersion>> {
    let bytes = metadata(&url(origin, minecraft, None)?, context).await?;
    builds_from_list(&bytes, loader_label(origin), minecraft)
}

/// Human-readable loader name for the official metadata origin (only used in
/// user-facing no-builds messages).
fn loader_label(origin: &str) -> &'static str {
    if origin.contains("quilt") {
        "Quilt"
    } else {
        "Fabric"
    }
}

/// Turn an official loader version list into the builds the picker can offer.
///
/// A version without published builds may answer with a different document
/// shape (or an empty array); both must read as "no builds for this version"
/// instead of leaking a serde parse error to the UI.
fn builds_from_list(
    bytes: &[u8],
    loader: &str,
    minecraft: &str,
) -> anyhow::Result<Vec<LoaderVersion>> {
    let entries = match serde_json::from_slice::<Vec<VersionEntry>>(bytes) {
        Ok(entries) => entries,
        Err(_) => return no_builds(loader, minecraft),
    };
    ensure!(entries.len() <= 4096, "Loader version list is too large");
    let mut builds = Vec::new();
    for entry in entries {
        validate_version(&entry.loader.version)?;
        if entry.intermediary.version != minecraft {
            continue;
        }
        let stable = entry
            .loader
            .stable
            .unwrap_or_else(|| !entry.loader.version.contains('-'));
        if !builds
            .iter()
            .any(|build: &LoaderVersion| build.version == entry.loader.version)
        {
            builds.push(LoaderVersion {
                version: entry.loader.version,
                stable,
            });
        }
    }
    if builds.is_empty() {
        return no_builds(loader, minecraft);
    }
    Ok(builds)
}

fn no_builds<T>(loader: &str, minecraft: &str) -> anyhow::Result<T> {
    anyhow::bail!(
        "No {loader} builds were found for Minecraft {minecraft}; retry or choose another version"
    )
}

/// Merge the official launch profile with Mojang metadata and verify all loader
/// artifacts before publishing the resulting manifest. Retrying reuses verified
/// objects; failure never marks a catalog target ready.
pub async fn install(
    kind: &LoaderKind,
    version: &str,
    base: &VersionManifest,
    context: &ResolveContext,
) -> anyhow::Result<PathBuf> {
    install_from(kind, endpoint(kind)?, version, base, context).await
}

async fn install_from(
    kind: &LoaderKind,
    origin: &str,
    version: &str,
    base: &VersionManifest,
    context: &ResolveContext,
) -> anyhow::Result<PathBuf> {
    validate_version(version)?;
    let builds = versions_from(origin, &base.id, context).await?;
    ensure!(
        builds.iter().any(|build| build.version == version),
        "This loader build is not available for the selected Minecraft version"
    );
    let bytes = metadata(&url(origin, &base.id, Some(version))?, context).await?;
    let overlay: Overlay =
        serde_json::from_slice(&bytes).context("Invalid official loader launch profile")?;
    let prefix = match kind {
        LoaderKind::Fabric => "fabric",
        LoaderKind::Quilt => "quilt",
        _ => anyhow::bail!("Expected Fabric or Quilt"),
    };
    ensure!(
        overlay.id == format!("{prefix}-loader-{version}-{}", base.id)
            && overlay.inherits_from == base.id,
        "Loader launch profile does not match the selected build and game"
    );
    ensure!(
        overlay.main_class.starts_with(match kind {
            LoaderKind::Fabric => "net.fabricmc.loader.",
            _ => "org.quiltmc.loader.",
        }) && overlay.main_class.ends_with(".KnotClient")
            && overlay.main_class.len() <= 256,
        "Unexpected loader client entry point"
    );
    ensure!(
        !overlay.libraries.is_empty() && overlay.libraries.len() <= 256,
        "Invalid loader library count"
    );
    let loader_coordinate = match kind {
        LoaderKind::Fabric => format!("net.fabricmc:fabric-loader:{version}"),
        _ => format!("org.quiltmc:quilt-loader:{version}"),
    };
    ensure!(
        overlay
            .libraries
            .iter()
            .any(|library| library.name == loader_coordinate)
            && overlay
                .libraries
                .iter()
                .any(|library| library.name == format!("net.fabricmc:intermediary:{}", base.id)),
        "Loader profile omitted its selected loader or game mappings"
    );
    let mut libraries = BTreeMap::new();
    let mut resources = Vec::new();
    for library in overlay.libraries {
        let path = maven_path(&library.name)?;
        let repository = reqwest::Url::parse(&library.url)?;
        let official = [
            "https://maven.fabricmc.net/",
            "https://maven.quiltmc.org/repository/release/",
            "https://repo1.maven.org/maven2/",
            "https://repo.maven.apache.org/maven2/",
        ]
        .contains(&repository.as_str());
        #[cfg(test)]
        let official = official || repository.as_str() == origin;
        ensure!(
            official,
            "Loader library does not use an official repository"
        );
        let artifact_url = repository.join(&path)?;
        let resource = match (library.sha1, library.size) {
            (Some(sha1), Some(size)) => Resource {
                sha1,
                size,
                url: artifact_url.into(),
            },
            _ => tokio::select! { biased;
                () = context.cancel.cancelled() => anyhow::bail!("Loader installation cancelled"),
                result = tokio::time::timeout(Duration::from_secs(45), context.store.describe_maven(artifact_url.as_str())) => result.context("Loader library metadata timed out")??,
            },
        };
        context.store.object_path(&resource.sha1)?;
        ensure!(
            resource.size > 0 && resource.size <= 128 * 1024 * 1024,
            "Invalid loader library size"
        );
        let artifact = Artifact {
            path,
            sha1: resource.sha1.clone(),
            size: resource.size,
            url: resource.url.clone(),
        };
        let key = coordinate_key(&library.name);
        let entry = Library {
            name: library.name,
            downloads: LibraryDownloads {
                artifact: Some(artifact),
                classifiers: None,
            },
            natives: None,
            extract: None,
            rules: None,
        };
        if let Some(previous) = libraries.insert(key, entry.clone()) {
            ensure!(previous == entry, "Conflicting loader library versions");
        }
        resources.push(resource);
    }
    context.store.fetch_all(&resources, &context.cancel).await?;
    let mut merged = base.clone();
    merged
        .libraries
        .retain(|library| !libraries.contains_key(&coordinate_key(&library.name)));
    merged.libraries.extend(libraries.into_values());
    merged.id = overlay.id;
    merged.main_class = overlay.main_class;
    let arguments = merged.arguments.get_or_insert_with(|| Arguments {
        game: merged
            .minecraft_arguments
            .take()
            .unwrap_or_default()
            .split_whitespace()
            .map(|value| Argument::Literal(value.into()))
            .collect(),
        jvm: vec![
            Argument::Literal("-Djava.library.path=${natives_directory}".into()),
            Argument::Literal("-cp".into()),
            Argument::Literal("${classpath}".into()),
        ],
        default_user_jvm: None,
    });
    arguments.game.extend(overlay.arguments.game);
    arguments.jvm.extend(overlay.arguments.jvm);
    let directory = context
        .store
        .root()
        .join("loaders")
        .join(prefix)
        .join(format!("{:x}", Sha1::digest(base.id.as_bytes())))
        .join(version);
    tokio::fs::create_dir_all(&directory).await?;
    ensure!(
        !context.cancel.is_cancelled(),
        "Loader installation cancelled"
    );
    let path = directory.join("manifest.json");
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    std::io::Write::write_all(&mut temporary, &serde_json::to_vec_pretty(&merged)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path)?;
    Ok(path)
}

fn maven_path(name: &str) -> anyhow::Result<String> {
    let parts: Vec<_> = name.split(':').collect();
    ensure!(
        (3..=4).contains(&parts.len()),
        "Invalid loader Maven coordinate"
    );
    for part in &parts {
        validate_version(part)?;
        ensure!(
            !part.split('.').any(str::is_empty),
            "Invalid Maven coordinate segment"
        );
    }
    let classifier = parts
        .get(3)
        .map_or_else(String::new, |value| format!("-{value}"));
    Ok(format!(
        "{}/{}/{}/{}-{}{classifier}.jar",
        parts[0].replace('.', "/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2]
    ))
}

fn coordinate_key(name: &str) -> Vec<String> {
    name.split(':')
        .enumerate()
        .filter(|(index, _)| *index != 2)
        .map(|(_, part)| part.to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use block_download::{DownloadOptions, ProxyMode, Store};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn context(
        root: &std::path::Path,
        offline: bool,
        retries: u32,
    ) -> anyhow::Result<ResolveContext> {
        Ok(ResolveContext {
            store: Store::with_options(
                root.into(),
                offline,
                Arc::new(|_| {}),
                DownloadOptions {
                    proxy: ProxyMode::Direct,
                    max_retries: retries,
                    ..Default::default()
                },
            )?,
            cancel: crate::CancellationToken::new(),
        })
    }

    fn base() -> anyhow::Result<VersionManifest> {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../minecraft-manifest/tests/fixtures/official");
        let index: minecraft_manifest::model::VersionIndex =
            serde_json::from_slice(&std::fs::read(fixtures.join("version_manifest_v2.json"))?)?;
        let entry = index
            .versions
            .iter()
            .find(|entry| entry.id == "1.20.6")
            .context("Missing game fixture")?;
        Ok(serde_json::from_slice(&std::fs::read(
            fixtures
                .join("versions")
                .join(format!("{}.json", entry.sha1)),
        )?)?)
    }

    async fn request(stream: &mut tokio::net::TcpStream) -> std::io::Result<String> {
        let mut bytes = Vec::new();
        while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let count = stream.read(&mut chunk).await?;
            if count == 0 || bytes.len() + count > 8192 {
                return Err(std::io::Error::other("Invalid request"));
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .next()
            .unwrap_or_default()
            .into())
    }

    async fn fixture(
        bodies: Vec<(u16, String, Vec<u8>)>,
    ) -> anyhow::Result<(String, tokio::task::JoinHandle<anyhow::Result<Vec<String>>>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}/", listener.local_addr()?);
        let replacement = origin.clone();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, headers, bytes) in bodies {
                let (mut stream, _) = listener.accept().await?;
                let line = request(&mut stream).await?;
                let head = line.starts_with("HEAD ");
                requests.push(line);
                let bytes = String::from_utf8(bytes)?
                    .replace("$ORIGIN", &replacement)
                    .into_bytes();
                stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n", bytes.len()).as_bytes()).await?;
                if !head {
                    stream.write_all(&bytes).await?;
                }
            }
            Ok(requests)
        });
        Ok((origin, task))
    }

    fn builds() -> Vec<u8> {
        serde_json::json!([
            {"loader":{"version":"0.16.14","stable":true},"intermediary":{"version":"1.20.6"}},
            {"loader":{"version":"0.16.14","stable":true},"intermediary":{"version":"1.20.6"}}
        ])
        .to_string()
        .into_bytes()
    }

    fn overlay(kind: &LoaderKind, inline_hash: bool) -> serde_json::Value {
        let (prefix, coordinate, main) = match kind {
            LoaderKind::Fabric => (
                "fabric",
                "net.fabricmc:fabric-loader:0.16.14",
                "net.fabricmc.loader.impl.launch.knot.KnotClient",
            ),
            _ => (
                "quilt",
                "org.quiltmc:quilt-loader:0.16.14",
                "org.quiltmc.loader.impl.launch.knot.KnotClient",
            ),
        };
        let mut libraries: Vec<_> = [coordinate, "net.fabricmc:intermediary:1.20.6"]
            .into_iter()
            .map(|name| serde_json::json!({"name":name,"url":"$ORIGIN"}))
            .collect();
        if inline_hash {
            for library in &mut libraries {
                library["sha1"] = format!("{:x}", Sha1::digest(b"fixture jar")).into();
                library["size"] = 11.into();
            }
        }
        serde_json::json!({"id":format!("{prefix}-loader-0.16.14-1.20.6"), "inheritsFrom":"1.20.6", "mainClass":main,
            "arguments":{"jvm":["-Dloader.fixture=true"], "game":[]},"libraries":libraries})
    }

    fn no_builds_error(bytes: &[u8], loader: &str, minecraft: &str) -> String {
        match builds_from_list(bytes, loader, minecraft) {
            Err(error) => error.to_string(),
            Ok(_) => "expected a no-builds error".into(),
        }
    }

    #[test]
    fn loader_list_without_intermediary_reads_as_no_builds() {
        // Quilt's metadata for a game version it has no builds for can answer
        // with entries that do not carry an `intermediary` mapping.
        let bytes = br#"[{"loader":{"version":"0.30.1"},"build":{}}]"#;
        let error = no_builds_error(bytes, "Quilt", "26.3");
        assert!(
            error.contains("No Quilt builds were found for Minecraft 26.3"),
            "{error}"
        );
        assert!(error.contains("retry or choose another version"), "{error}");
        assert!(!error.contains("serde"), "{error}");
        assert!(!error.contains("missing field"), "{error}");
    }

    #[test]
    fn empty_loader_list_reads_as_no_builds() {
        let error = no_builds_error(b"[]", "Fabric", "1.16.5");
        assert!(
            error.contains("No Fabric builds were found for Minecraft 1.16.5"),
            "{error}"
        );
    }

    #[test]
    fn loader_list_for_another_game_version_reads_as_no_builds() {
        let bytes = br#"[{"loader":{"version":"0.15.11","stable":true},"intermediary":{"version":"1.20.1"}}]"#;
        let error = no_builds_error(bytes, "Quilt", "1.18.2");
        assert!(
            error.contains("No Quilt builds were found for Minecraft 1.18.2"),
            "{error}"
        );
    }

    #[test]
    fn matching_loader_list_keeps_stable_and_prerelease_builds() {
        let bytes = br#"[{"loader":{"version":"0.30.1","stable":true},"intermediary":{"version":"1.18.2"}},{"loader":{"version":"0.31.0-beta.4"},"intermediary":{"version":"1.18.2"}}]"#;
        match builds_from_list(bytes, "Quilt", "1.18.2") {
            Ok(builds) => {
                assert_eq!(builds.len(), 2);
                assert!(
                    builds
                        .iter()
                        .any(|build| build.version == "0.30.1" && build.stable)
                );
                assert!(
                    builds
                        .iter()
                        .any(|build| build.version == "0.31.0-beta.4" && !build.stable)
                );
            }
            Err(error) => assert!(
                error.to_string().is_empty(),
                "valid loader list must parse: {error}"
            ),
        }
    }

    #[tokio::test]
    async fn fabric_and_quilt_install_verified_closures_without_changing_mojang_resources()
    -> anyhow::Result<()> {
        for kind in [LoaderKind::Fabric, LoaderKind::Quilt] {
            // The two synthetic libraries intentionally share one immutable object.
            let (origin, task) = fixture(vec![
                (200, String::new(), builds()),
                (
                    200,
                    String::new(),
                    overlay(&kind, true).to_string().into_bytes(),
                ),
                (200, String::new(), b"fixture jar".to_vec()),
            ])
            .await?;
            let root = tempfile::tempdir()?;
            let context = context(root.path(), false, 0)?;
            let base = base()?;
            let path = install_from(&kind, &origin, "0.16.14", &base, &context).await?;
            let merged: VersionManifest = serde_json::from_slice(&std::fs::read(&path)?)?;
            assert_eq!(merged.downloads, base.downloads);
            assert_eq!(merged.asset_index, base.asset_index);
            assert_eq!(merged.java_version, base.java_version);
            assert_eq!(merged.libraries.len(), base.libraries.len() + 2);
            assert!(
                merged
                    .arguments
                    .context("Arguments")?
                    .jvm
                    .contains(&Argument::Literal("-Dloader.fixture=true".into()))
            );
            let requests = task.await??;
            assert!(requests[0].starts_with("GET /versions/loader/1.20.6 "));
            assert_eq!(requests.len(), 3);
            let offline = self::context(root.path(), true, 0)?;
            assert_eq!(
                install_from(&kind, &origin, "0.16.14", &base, &offline).await?,
                path
            );
            std::fs::write(
                context
                    .store
                    .object_path(&format!("{:x}", Sha1::digest(b"fixture jar")))?,
                b"corrupt",
            )?;
            assert!(
                install_from(&kind, &origin, "0.16.14", &base, &offline)
                    .await
                    .is_err()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn missing_inline_hashes_use_repository_checksums_and_head_lengths() -> anyhow::Result<()>
    {
        let digest = format!("{:x}", Sha1::digest(b"fixture jar"));
        let (origin, task) = fixture(vec![
            (200, String::new(), builds()),
            (
                200,
                String::new(),
                overlay(&LoaderKind::Quilt, false).to_string().into_bytes(),
            ),
            (200, String::new(), digest.as_bytes().to_vec()),
            (200, String::new(), b"fixture jar".to_vec()),
            (200, String::new(), digest.as_bytes().to_vec()),
            (200, String::new(), b"fixture jar".to_vec()),
            (200, String::new(), b"fixture jar".to_vec()),
        ])
        .await?;
        let root = tempfile::tempdir()?;
        install_from(
            &LoaderKind::Quilt,
            &origin,
            "0.16.14",
            &base()?,
            &context(root.path(), false, 0)?,
        )
        .await?;
        let requests = task.await??;
        assert!(requests[2].contains(".jar.sha1 "));
        assert!(requests[3].starts_with("HEAD "));
        assert!(requests[4].contains("intermediary-1.20.6.jar.sha1 "));
        Ok(())
    }

    #[tokio::test]
    async fn wrong_game_identity_foreign_repositories_and_missing_libraries_fail_before_artifacts()
    -> anyhow::Result<()> {
        for case in 0..4 {
            let mut overlay = overlay(&LoaderKind::Fabric, true);
            match case {
                0 => overlay["inheritsFrom"] = "1.21.1".into(),
                1 => overlay["libraries"][0]["url"] = "https://untrusted.invalid/".into(),
                2 => {
                    overlay["libraries"][0]["name"] = "net.fabricmc:fabric-loader:../escape".into()
                }
                _ => overlay["mainClass"] = "net.example.Server".into(),
            }
            let (origin, task) = fixture(vec![
                (200, String::new(), builds()),
                (200, String::new(), overlay.to_string().into_bytes()),
            ])
            .await?;
            let root = tempfile::tempdir()?;
            assert!(
                install_from(
                    &LoaderKind::Fabric,
                    &origin,
                    "0.16.14",
                    &base()?,
                    &context(root.path(), false, 0)?
                )
                .await
                .is_err()
            );
            assert_eq!(task.await??.len(), 2);
            assert!(!root.path().join("loaders").exists());
        }
        for coordinate in [
            "../bad:artifact:1",
            "group:../bad:1",
            "group:artifact:../1",
            "group::1",
            "g:a:1@zip",
        ] {
            assert!(maven_path(coordinate).is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn unavailable_builds_and_malformed_catalogs_remain_retryable_errors()
    -> anyhow::Result<()> {
        for body in [
            b"[]".to_vec(),
            b"{}".to_vec(),
            b"[{\"loader\":{\"version\":\"0.16.14\"},\"intermediary\":{\"version\":\"wrong\"}}]"
                .to_vec(),
        ] {
            let (origin, task) = fixture(vec![(200, String::new(), body)]).await?;
            let root = tempfile::tempdir()?;
            assert!(
                install_from(
                    &LoaderKind::Fabric,
                    &origin,
                    "0.16.14",
                    &base()?,
                    &context(root.path(), false, 0)?
                )
                .await
                .is_err()
            );
            task.await??;
        }
        let (origin, task) = fixture(vec![(404, String::new(), b"expired build".to_vec())]).await?;
        let root = tempfile::tempdir()?;
        let error = versions_from(&origin, "1.20.6", &context(root.path(), false, 0)?)
            .await
            .err()
            .context("Expected HTTP error")?;
        assert!(format!("{error:#}").contains("404"));
        task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn catalog_throttling_retries_and_cancel_interrupts_backoff() -> anyhow::Result<()> {
        let (origin, task) = fixture(vec![
            (429, "Retry-After: 0\r\n".into(), Vec::new()),
            (200, String::new(), builds()),
        ])
        .await?;
        let root = tempfile::tempdir()?;
        assert_eq!(
            versions_from(&origin, "1.20.6", &context(root.path(), false, 1)?)
                .await?
                .len(),
            1
        );
        assert_eq!(task.await??.len(), 2);
        let (origin, task) =
            fixture(vec![(429, "Retry-After: 3600\r\n".into(), Vec::new())]).await?;
        let context = context(root.path(), false, 1)?;
        let cancel = context.cancel.clone();
        let pending = tokio::spawn(async move { versions_from(&origin, "1.20.6", &context).await });
        task.await??;
        cancel.cancel();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), pending)
                .await??
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn catalog_timeout_and_cancel_during_io_do_not_publish_installations()
    -> anyhow::Result<()> {
        for timeout in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let origin = format!("http://{}/", listener.local_addr()?);
            let root = tempfile::tempdir()?;
            let context = context(root.path(), false, 0)?;
            let cancel = context.cancel.clone();
            let pending =
                tokio::spawn(async move { versions_from(&origin, "1.20.6", &context).await });
            let (mut stream, _) = listener.accept().await?;
            request(&mut stream).await?;
            if timeout {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(46)).await;
            } else {
                cancel.cancel();
            }
            let error = pending
                .await?
                .err()
                .context("Expected bounded request failure")?;
            assert!(
                error
                    .to_string()
                    .contains(if timeout { "timed out" } else { "cancelled" })
            );
            if timeout {
                tokio::time::resume();
            }
            assert!(!root.path().join("loaders").exists());
        }
        Ok(())
    }
}
