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
