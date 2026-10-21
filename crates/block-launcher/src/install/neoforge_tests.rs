use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::Context as _;
use minecraft_manifest::{model::VersionIndex, resources::AssetObjects};
use serde_json::json;

use super::{
    InstalledArtifact, InstalledLoader, MINECRAFT_VERSION, VERSION, adapt_plan,
    cached_artifacts_valid, coordinate_key, file_digest, import_libraries, merge_manifest,
    safe_relative,
};
use crate::{
    Store,
    plan::{PrepareAction, compile},
    platform::{Architecture, OperatingSystem, Target},
    recipe::{Compatibility, LaunchRecipe, ParsedRecipe, RecipeSource, RuntimeSelection},
    resolver::{ResolveContext, ResolvedMetadata},
    session::OfflineSession,
};

fn base() -> anyhow::Result<minecraft_manifest::model::VersionManifest> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../minecraft-manifest/tests/fixtures/official");
    let index: VersionIndex =
        serde_json::from_slice(&std::fs::read(root.join("version_manifest_v2.json"))?)?;
    let entry = index
        .versions
        .iter()
        .find(|entry| entry.id == MINECRAFT_VERSION)
        .context("missing fixture")?;
    Ok(serde_json::from_slice(&std::fs::read(
        root.join("versions").join(format!("{}.json", entry.sha1)),
    )?)?)
}

fn overlay() -> anyhow::Result<super::Overlay> {
    Ok(serde_json::from_value(json!({
        "id": format!("neoforge-{VERSION}"),
        "inheritsFrom": MINECRAFT_VERSION,
        "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
        "arguments": {"jvm":["-p", "${library_directory}/example/tool/1/tool-1.jar"], "game":["--launchTarget", "forgeclient"]},
        "libraries":[{"name":"example:tool:1@jar", "downloads":{"artifact":{"path":"example/tool/1/tool-1.jar", "sha1":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size":3, "url":"https://example.invalid/tool.jar"}}}]
    }))?)
}

#[test]
fn merge_inherits_verified_vanilla_client_and_arguments() -> anyhow::Result<()> {
    let original = base()?;
    let merged = merge_manifest(&original, overlay()?)?;
    assert_eq!(merged.downloads.client, original.downloads.client);
    assert_eq!(merged.asset_index, original.asset_index);
    assert_eq!(merged.java_version, original.java_version);
    assert_eq!(
        merged.arguments.as_ref().context("arguments")?.game.len(),
        original.arguments.as_ref().context("arguments")?.game.len() + 2
    );
    assert_eq!(
        merged.main_class,
        "cpw.mods.bootstraplauncher.BootstrapLauncher"
    );
    assert_eq!(original.id, MINECRAFT_VERSION);
    Ok(())
}

#[test]
fn merge_does_not_drop_distinct_native_classifiers() -> anyhow::Result<()> {
    let mut original = base()?;
    let library = overlay()?.libraries.remove(0);
    let mut native = library.clone();
    native.name = "example:tool:0:natives-linux".into();
    let mut previous = library;
    previous.name = "example:tool:0".into();
    original.libraries.extend([native, previous]);
    let merged = merge_manifest(&original, overlay()?)?;
    assert!(
        merged
            .libraries
            .iter()
            .any(|library| library.name == "example:tool:0:natives-linux")
    );
    assert!(
        !merged
            .libraries
            .iter()
            .any(|library| library.name == "example:tool:0")
    );
    assert_eq!(
        coordinate_key("example:tool:1@jar"),
        coordinate_key("example:tool:0")
    );
    Ok(())
}

#[test]
fn reject_different_inherited_game_version() -> anyhow::Result<()> {
    let mut loader = overlay()?;
    loader.inherits_from = "1.21".into();
    assert!(merge_manifest(&base()?, loader).is_err());
    Ok(())
}

#[test]
fn official_version_numbers_bind_releases_snapshots_and_historical_builds_to_the_right_game() {
    for (version, minecraft, stable) in [
        ("20.6.144", "1.20.6", true),
        ("21.0.1-beta", "1.21", false),
        ("21.1.219", "1.21.1", true),
        ("26.1.0.19-beta", "26.1", false),
        ("26.1.1.15-beta", "26.1.1", false),
        ("26.3.0.95", "26.3", true),
        ("26.1.0.0-alpha.15+pre-3", "26.1-pre-3", false),
        ("26.1.0.0-alpha.10+snapshot-6", "26.1-snapshot-6", false),
        ("1.20.1-47.1.106", "1.20.1", true),
    ] {
        assert!(
            super::validate_version(minecraft, version).is_ok(),
            "{version}"
        );
        let parsed = super::version_details(version);
        assert_eq!(parsed.map(|(_, rank)| rank.pre.is_empty()), Some(stable));
        assert!(super::validate_version("1.12.2", version).is_err());
    }
    for version in [
        "../20.6.144",
        "20.6",
        "1.20.1-../../escape",
        "26.1.0.0-alpha+unrecognized-1",
        "26.3.0.bad",
        "47.1.82",
    ] {
        assert!(super::version_details(version).is_none(), "{version}");
    }
}

#[test]
fn generated_client_paths_follow_the_installer_data_for_both_processor_generations()
-> anyhow::Result<()> {
    for (patched, extra, expected) in [
        (
            "[net.neoforged:neoforge:21.1.219:client]",
            Some("[net.minecraft:client:1.21.1-20240808.144430:extra]"),
            "net/neoforged/neoforge/21.1.219/neoforge-21.1.219-client.jar",
        ),
        (
            "[net.neoforged:forge:1.20.1-47.1.106:client]",
            None,
            "net/neoforged/forge/1.20.1-47.1.106/forge-1.20.1-47.1.106-client.jar",
        ),
        (
            "[net.neoforged:minecraft-client-patched:26.3.0.95]",
            None,
            "net/neoforged/minecraft-client-patched/26.3.0.95/minecraft-client-patched-26.3.0.95.jar",
        ),
    ] {
        let mut data = json!({"PATCHED":{"client":patched}});
        if let Some(extra) = extra {
            data["MC_EXTRA"] = json!({"client":extra});
        }
        let profile =
            serde_json::from_value(json!({"minecraft":"fixture", "libraries":[], "data":data}))?;
        assert_eq!(super::generated_client_paths(&profile)?[0], expected);
    }
    for data in [
        json!({}),
        json!({"PATCHED":{"client":"../outside"}}),
        json!({"PATCHED":{"client":"[group:artifact:../../outside]"}}),
    ] {
        let profile =
            serde_json::from_value(json!({"minecraft":"fixture", "libraries":[], "data":data}))?;
        assert!(super::generated_client_paths(&profile).is_err());
    }
    Ok(())
}

#[test]
fn canonical_paths_and_generated_outputs_survive_plan_binding() -> anyhow::Result<()> {
    let manifest = merge_manifest(&base()?, overlay()?)?;
    let metadata = ResolvedMetadata {
        parsed: ParsedRecipe {
            manifest: manifest.clone(),
            manifest_sha1: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        },
        target: Target {
            os: OperatingSystem::Windows,
            arch: Architecture::X86_64,
            os_version: "10".into(),
        },
        assets: AssetObjects {
            objects: BTreeMap::new(),
            virtual_layout: None,
            map_to_resources: None,
        },
        runtime: None,
        library_overrides: BTreeMap::new(),
    };
    let recipe = LaunchRecipe {
        schema_version: 1,
        instance: "neoforge-test".into(),
        source: RecipeSource::Mojang {
            version: MINECRAFT_VERSION.into(),
        },
        session: OfflineSession::Offline {
            username: "LoaderTest".into(),
        },
        runtime: RuntimeSelection::Local {
            java: std::env::current_exe()?,
        },
        target: None,
        compatibility: Compatibility::Official,
        memory_mib: 1024,
        jvm_args: Vec::new(),
        features: BTreeMap::new(),
    };
    let resource = block_download::Resource {
        sha1: "cccccccccccccccccccccccccccccccccccccccc".into(),
        size: 7,
        url: String::new(),
    };
    let installed = InstalledLoader {
        version: VERSION.into(),
        minecraft_version: MINECRAFT_VERSION.into(),
        manifest_path: Path::new("unused").into(),
        manifest_sha1: metadata.parsed.manifest_sha1.clone(),
        artifacts: vec![InstalledArtifact {
            path: "net/neoforged/neoforge/20.6.144/neoforge-20.6.144-client.jar".into(),
            resource: resource.clone(),
        }],
    };
    let mut plan = compile(&recipe, &metadata)?;
    adapt_plan(&mut plan, &manifest, &installed)?;
    assert!(
        plan.command
            .classpath
            .iter()
            .any(|path| path.ends_with("/example/tool/1/tool-1.jar"))
    );
    assert!(
        plan.command
            .classpath
            .iter()
            .any(|path| path.ends_with("/neoforge-20.6.144.jar"))
    );
    assert_eq!(
        plan.command.variables.get("classpath_separator"),
        Some(&";".into())
    );
    assert!(
        plan.resources
            .iter()
            .any(|candidate| candidate == &resource)
    );
    assert!(plan.actions.iter().any(|action| matches!(action, PrepareAction::Copy {sha1,path,..} if *sha1 == resource.sha1 && path.ends_with("/neoforge-20.6.144-client.jar"))));
    // Re-applying a persisted installed-target receipt must not duplicate resources/actions.
    let counts = (plan.resources.len(), plan.actions.len());
    adapt_plan(&mut plan, &manifest, &installed)?;
    assert_eq!(counts, (plan.resources.len(), plan.actions.len()));
    assert_eq!(plan.command.cwd, "instances/neoforge-test/game");
    Ok(())
}

#[test]
fn generated_artifacts_are_verified_in_cache_and_corruption_is_detected() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let stage = directory.path().join("libraries");
    std::fs::create_dir_all(stage.join("generated"))?;
    let path = stage.join("generated/client.jar");
    std::fs::write(&path, b"generated game bytes")?;
    let context = ResolveContext {
        store: Store::new(directory.path().join("cache"), true, Arc::new(|_| {}))?,
        cancel: crate::CancellationToken::new(),
    };
    let artifacts = import_libraries(&stage, &BTreeMap::new(), &context)?;
    let installed = InstalledLoader {
        version: VERSION.into(),
        minecraft_version: MINECRAFT_VERSION.into(),
        manifest_path: Path::new("unused").into(),
        manifest_sha1: String::new(),
        artifacts,
    };
    assert!(cached_artifacts_valid(&installed, &context)?);
    let artifact = &installed.artifacts[0];
    assert_eq!(artifact.resource.sha1, file_digest(&path)?);
    assert_eq!(artifact.resource.url, "");
    std::fs::write(
        context.store.object_path(&artifact.resource.sha1)?,
        b"corrupt game bytes",
    )?;
    assert!(!cached_artifacts_valid(&installed, &context)?);
    Ok(())
}

#[test]
fn checksum_mismatch_does_not_publish_installer_library() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let libraries = directory.path().join("libraries");
    std::fs::create_dir_all(&libraries)?;
    std::fs::write(libraries.join("tool.jar"), b"tampered")?;
    let context = ResolveContext {
        store: Store::new(directory.path().join("cache"), true, Arc::new(|_| {}))?,
        cancel: crate::CancellationToken::new(),
    };
    let resource = block_download::Resource {
        sha1: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        size: 8,
        url: "https://example.invalid/tool.jar".into(),
    };
    let expected = BTreeMap::from([("tool.jar".into(), resource.clone())]);
    assert!(import_libraries(&libraries, &expected, &context).is_err());
    assert!(!context.store.object_path(&resource.sha1)?.exists());
    Ok(())
}

#[test]
fn reject_platform_specific_escape_paths() {
    for path in [
        "../outside.jar",
        "/outside.jar",
        "C:/outside.jar",
        "a/../../outside.jar",
        "a\\outside.jar",
        "a//tool.jar",
        "a/./tool.jar",
        "a:stream",
        "",
    ] {
        assert!(safe_relative(path).is_err(), "accepted {path:?}");
    }
    assert!(safe_relative("net/neoforged/neoforge/20.6.144/neoforge-20.6.144-client.jar").is_ok());
}

#[test]
fn valid_content_objects_are_reused_without_replacing_the_file() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("generated.jar");
    std::fs::write(&source, b"immutable bytes")?;
    let context = ResolveContext {
        store: Store::new(directory.path().join("cache"), true, Arc::new(|_| {}))?,
        cancel: crate::CancellationToken::new(),
    };
    let resource = block_download::Resource {
        sha1: file_digest(&source)?,
        size: source.metadata()?.len(),
        url: String::new(),
    };
    super::publish_object(&source, &resource, &context)?;
    let destination = context.store.object_path(&resource.sha1)?;
    // Windows requires write access on the handle used to change file timestamps.
    let retained = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&destination)?;
    let old_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
    retained.set_times(std::fs::FileTimes::new().set_modified(old_time))?;
    super::publish_object(&source, &resource, &context)?;
    assert_eq!(destination.metadata()?.modified()?, old_time);
    assert_eq!(file_digest(&destination)?, resource.sha1);
    Ok(())
}

#[tokio::test]
async fn concurrent_installation_wait_can_be_cancelled_and_lock_is_released() -> anyhow::Result<()>
{
    let directory = tempfile::tempdir()?;
    let first =
        super::acquire_installation_lock(directory.path(), &crate::CancellationToken::new())
            .await?;
    let cancelled = crate::CancellationToken::new();
    cancelled.cancel();
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::acquire_installation_lock(directory.path(), &cancelled),
    )
    .await?;
    assert!(second.is_err());
    drop(first);
    let _next =
        super::acquire_installation_lock(directory.path(), &crate::CancellationToken::new())
            .await?;
    Ok(())
}

fn catalog_xml(artifact: &str, versions: &[&str]) -> Vec<u8> {
    format!("<metadata><groupId>net.neoforged</groupId><artifactId>{artifact}</artifactId><versioning><versions>{}</versions></versioning></metadata>", versions.iter().map(|version| format!("<version>{version}</version>")).collect::<String>()).into_bytes()
}
fn catalog_context(root: &Path, offline: bool, retries: u32) -> anyhow::Result<ResolveContext> {
    Ok(ResolveContext {
        store: Store::with_options(
            root.into(),
            offline,
            Arc::new(|_| {}),
            crate::DownloadOptions {
                proxy: crate::ProxyMode::Direct,
                max_retries: retries,
                ..Default::default()
            },
        )?,
        cancel: crate::CancellationToken::new(),
    })
}
async fn read_catalog_request(stream: &mut tokio::net::TcpStream) -> anyhow::Result<String> {
    use tokio::io::AsyncReadExt as _;
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await?;
        anyhow::ensure!(
            count > 0 && bytes.len() + count <= 8192,
            "Invalid fixture request"
        );
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .into())
}
async fn catalog_fixture(
    responses: Vec<(u16, &'static str, Vec<u8>)>,
) -> anyhow::Result<(String, tokio::task::JoinHandle<anyhow::Result<Vec<String>>>)> {
    use tokio::io::AsyncWriteExt as _;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}/", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, headers, bytes) in responses {
            let (mut stream, _) = listener.accept().await?;
            requests.push(read_catalog_request(&mut stream).await?);
            stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n", bytes.len()).as_bytes()).await?;
            stream.write_all(&bytes).await?;
        }
        Ok(requests)
    });
    Ok((origin, task))
}

#[tokio::test]
async fn catalog_filters_numeric_builds_game_patches_snapshots_and_historical_artifacts()
-> anyhow::Result<()> {
    for (minecraft, artifact, input, expected) in [
        (
            "1.20.6",
            "neoforge",
            vec!["20.6.9", "21.1.219", "20.6.144", "20.6.144"],
            vec!["20.6.144", "20.6.9"],
        ),
        (
            "26.1",
            "neoforge",
            vec![
                "26.1.0.9-beta",
                "26.1.1.20-beta",
                "26.1.0.19-beta",
                "26.1.0.0-alpha.1+snapshot-1",
            ],
            vec!["26.1.0.19-beta", "26.1.0.9-beta"],
        ),
        (
            "26.1-snapshot-6",
            "neoforge",
            vec![
                "26.1.0.0-alpha.9+snapshot-6",
                "26.1.0.0-alpha.10+snapshot-6",
                "26.1.0.0-alpha.11+snapshot-7",
            ],
            vec![
                "26.1.0.0-alpha.10+snapshot-6",
                "26.1.0.0-alpha.9+snapshot-6",
            ],
        ),
        (
            "1.20.1",
            "forge",
            vec!["1.20.1-47.1.82", "47.1.82", "1.20.1-47.1.106"],
            vec!["1.20.1-47.1.106", "1.20.1-47.1.82"],
        ),
    ] {
        let (origin, task) =
            catalog_fixture(vec![(200, "", catalog_xml(artifact, &input))]).await?;
        let root = tempfile::tempdir()?;
        let builds =
            super::versions_from(minecraft, &catalog_context(root.path(), false, 0)?, &origin)
                .await?;
        assert_eq!(
            builds
                .iter()
                .map(|build| build.version.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            task.await??,
            [format!(
                "GET /net/neoforged/{artifact}/maven-metadata.xml HTTP/1.1"
            )]
        );
        assert_eq!(
            super::versions_from(minecraft, &catalog_context(root.path(), true, 0)?, &origin)
                .await?,
            builds
        );
        if minecraft == "1.20.1" {
            assert!(builds.iter().all(|build| build.stable));
        }
    }
    Ok(())
}

#[tokio::test]
async fn catalog_identity_invalid_xml_http_errors_and_delisted_installs_stay_errors()
-> anyhow::Result<()> {
    for body in [
        b"not XML".to_vec(),
        catalog_xml("foreign", &["20.6.144"]),
        b"<!DOCTYPE metadata><metadata/>".to_vec(),
    ] {
        let (origin, task) = catalog_fixture(vec![(200, "", body)]).await?;
        let root = tempfile::tempdir()?;
        assert!(
            super::versions_from("1.20.6", &catalog_context(root.path(), false, 0)?, &origin)
                .await
                .is_err()
        );
        task.await??;
    }
    let (origin, task) =
        catalog_fixture(vec![(404, "", b"catalog not available".to_vec())]).await?;
    let root = tempfile::tempdir()?;
    let error = super::versions_from("1.20.6", &catalog_context(root.path(), false, 0)?, &origin)
        .await
        .err()
        .context("Expected HTTP error")?;
    assert!(format!("{error:#}").contains("404"));
    task.await??;
    let (origin, task) =
        catalog_fixture(vec![(200, "", catalog_xml("neoforge", &["20.6.9"]))]).await?;
    let context = catalog_context(root.path(), false, 0)?;
    let result = super::install_from(
        &base()?,
        "20.6.144",
        &std::env::current_exe()?,
        &context,
        &origin,
    )
    .await;
    assert!(
        result
            .err()
            .is_some_and(|error| error.to_string().contains("no longer available"))
    );
    task.await??;
    assert!(
        !root
            .path()
            .join("loaders/neoforge/20.6.144/manifest.json")
            .exists()
    );
    Ok(())
}

#[tokio::test]
async fn catalog_throttling_retries_but_cancellation_interrupts_server_backoff()
-> anyhow::Result<()> {
    let (origin, task) = catalog_fixture(vec![
        (429, "Retry-After: 0\r\n", Vec::new()),
        (200, "", catalog_xml("neoforge", &["20.6.144"])),
    ])
    .await?;
    let root = tempfile::tempdir()?;
    assert_eq!(
        super::versions_from("1.20.6", &catalog_context(root.path(), false, 1)?, &origin)
            .await?
            .len(),
        1
    );
    assert_eq!(task.await??.len(), 2);
    let (origin, task) = catalog_fixture(vec![(429, "Retry-After: 3600\r\n", Vec::new())]).await?;
    let context = catalog_context(root.path(), false, 1)?;
    let cancel = context.cancel.clone();
    let pending =
        tokio::spawn(async move { super::versions_from("1.20.6", &context, &origin).await });
    task.await??;
    cancel.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), pending)
            .await??
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn catalog_timeout_and_cancellation_interrupt_stalled_io() -> anyhow::Result<()> {
    for timeout in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}/", listener.local_addr()?);
        let root = tempfile::tempdir()?;
        let context = catalog_context(root.path(), false, 0)?;
        let cancel = context.cancel.clone();
        let pending =
            tokio::spawn(async move { super::versions_from("1.20.6", &context, &origin).await });
        let (mut stream, _) = listener.accept().await?;
        read_catalog_request(&mut stream).await?;
        if timeout {
            tokio::time::pause();
            tokio::time::advance(std::time::Duration::from_secs(46)).await;
        } else {
            cancel.cancel();
        }
        let error = pending.await?.err().context("Expected bounded failure")?;
        assert!(
            error
                .to_string()
                .contains(if timeout { "timed out" } else { "cancelled" })
        );
        if timeout {
            tokio::time::resume();
        }
    }
    Ok(())
}
