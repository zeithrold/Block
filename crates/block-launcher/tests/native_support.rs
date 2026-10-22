use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use block_launcher::{
    CancellationToken, Store,
    executor::Executor,
    plan::{ExecutionPlan, compile},
    platform::{Architecture, OperatingSystem, Target},
    recipe::{Compatibility, LaunchRecipe, ParsedRecipe, RecipeSource, RuntimeSelection},
    resolver::{ResolveContext, ResolvedMetadata, resolve},
    session::OfflineSession,
    support::{NativeSupportReason, UnsupportedNativeSupport, manifest_support, plan_support},
};
use minecraft_manifest::{
    decode,
    model::{VersionIndex, VersionManifest},
    resources::AssetObjects,
};

fn fixture(version: &str) -> anyhow::Result<(VersionManifest, PathBuf)> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../minecraft-manifest/tests/fixtures/official");
    let index: VersionIndex = decode(&fs::read(root.join("version_manifest_v2.json"))?)?;
    let entry = index
        .versions
        .iter()
        .find(|entry| entry.id == version)
        .ok_or_else(|| anyhow::anyhow!("missing fixture {version}"))?;
    let path = root.join("versions").join(format!("{}.json", entry.sha1));
    Ok((decode(&fs::read(&path)?)?, path))
}

fn mac_arm64() -> Target {
    Target {
        os: OperatingSystem::Macos,
        arch: Architecture::Aarch64,
        os_version: "15.0".into(),
    }
}

fn recipe(target: Target, source: RecipeSource) -> LaunchRecipe {
    LaunchRecipe {
        schema_version: 1,
        instance: "native-policy-test".into(),
        source,
        session: OfflineSession::Offline {
            username: "Player".into(),
        },
        runtime: RuntimeSelection::Local {
            java: PathBuf::from("unused-java"),
        },
        target: Some(target),
        compatibility: Compatibility::Official,
        memory_mib: 1024,
        jvm_args: Vec::new(),
        features: BTreeMap::new(),
    }
}

fn compiled(manifest: VersionManifest, target: Target) -> anyhow::Result<ExecutionPlan> {
    let recipe = recipe(
        target.clone(),
        RecipeSource::Mojang {
            version: manifest.id.clone(),
        },
    );
    compile(
        &recipe,
        &ResolvedMetadata {
            parsed: ParsedRecipe {
                manifest,
                manifest_sha1: "0".repeat(40),
            },
            target,
            assets: AssetObjects {
                objects: BTreeMap::new(),
                virtual_layout: None,
                map_to_resources: None,
            },
            runtime: None,
            library_overrides: BTreeMap::new(),
            compatibility: None,
        },
    )
}

#[test]
fn real_metadata_establishes_native_support_without_a_version_number_cutoff() -> anyhow::Result<()>
{
    for (version, supported) in [
        ("1.12.2", false),
        ("1.18.2", false),
        ("1.19", true),
        ("1.20.6", true),
        ("26.3", true),
    ] {
        let (mut manifest, _) = fixture(version)?;
        let decision = manifest_support(&manifest, &mac_arm64(), &BTreeMap::new())?;
        assert_eq!(
            decision.is_supported(),
            supported,
            "{version}: {decision:?}"
        );
        manifest.id = if supported { "1.0" } else { "99.0" }.into();
        assert_eq!(
            manifest_support(&manifest, &mac_arm64(), &BTreeMap::new())?.is_supported(),
            supported
        );
    }
    let (manifest, _) = fixture("1.18.2")?;
    let error = compiled(manifest, mac_arm64())
        .err()
        .ok_or_else(|| anyhow::anyhow!("unexpectedly compiled x64-only native metadata"))?;
    assert!(error.downcast_ref::<UnsupportedNativeSupport>().is_some());
    Ok(())
}

#[test]
fn every_selected_lwjgl_module_needs_its_matching_arm64_artifact() -> anyhow::Result<()> {
    let (mut manifest, _) = fixture("1.20.6")?;
    let arm64 = manifest
        .libraries
        .iter_mut()
        .find(|library| {
            library.name.contains("lwjgl-openal:") && library.name.ends_with(":natives-macos-arm64")
        })
        .ok_or_else(|| anyhow::anyhow!("missing ARM64 fixture"))?;
    // A classifier merely existing in the document is insufficient if its rules exclude it.
    arm64.rules = Some(serde_json::from_value(
        serde_json::json!([{"action":"disallow"}]),
    )?);
    let decision = manifest_support(&manifest, &mac_arm64(), &BTreeMap::new())?;
    assert!(
        matches!(decision.reason().map(|reason| &reason.reason), Some(NativeSupportReason::MissingArm64Artifact { library }) if library.starts_with("lwjgl-openal-"))
    );
    assert!(
        decision
            .reason()
            .is_some_and(|reason| reason.to_string().contains("macOS ARM64"))
    );
    Ok(())
}

#[test]
fn loader_entrypoints_preserve_the_modern_native_architecture_decision() -> anyhow::Result<()> {
    let (manifest, _) = fixture("1.20.6")?;
    for main in [
        "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "org.quiltmc.loader.impl.launch.knot.KnotClient",
    ] {
        let mut overlay = manifest.clone();
        overlay.main_class = main.into();
        let plan = compiled(overlay, mac_arm64())?;
        assert!(plan_support(&plan).is_supported());
        assert_eq!(plan.command.main_class, main);
    }
    Ok(())
}

#[test]
fn x64_platform_planning_is_unchanged_for_legacy_and_modern_versions() -> anyhow::Result<()> {
    for version in ["1.0", "1.12.2", "1.18.2", "1.19", "1.20.6"] {
        let (manifest, _) = fixture(version)?;
        for os in [
            OperatingSystem::Macos,
            OperatingSystem::Linux,
            OperatingSystem::Windows,
        ] {
            let target = Target {
                os,
                arch: Architecture::X86_64,
                os_version: "10.0.22000".into(),
            };
            assert!(manifest_support(&manifest, &target, &BTreeMap::new())?.is_supported());
            assert!(plan_support(&compiled(manifest.clone(), target)?).is_supported());
        }
    }
    Ok(())
}

#[tokio::test]
async fn unsupported_resolution_stops_before_asset_or_runtime_fetching() -> anyhow::Result<()> {
    let (_, path) = fixture("1.18.2")?;
    let mut recipe = recipe(mac_arm64(), RecipeSource::LocalManifest { path });
    recipe.runtime = RuntimeSelection::Managed;
    let directory = tempfile::tempdir()?;
    let store = Store::new(directory.path().to_path_buf(), true, Arc::new(|_| {}))?;
    let context = ResolveContext {
        store,
        cancel: CancellationToken::new(),
    };
    let error = resolve(&recipe, &context)
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("unsupported resolution succeeded"))?;
    assert!(
        error.downcast_ref::<UnsupportedNativeSupport>().is_some(),
        "{error:#}"
    );
    Ok(())
}

#[tokio::test]
async fn saved_plan_guard_runs_before_instance_files_downloads_or_java_probes() -> anyhow::Result<()>
{
    let (manifest, _) = fixture("1.20.6")?;
    let plan = compiled(manifest, mac_arm64())?;
    let mut replay: ExecutionPlan = serde_json::from_slice(&serde_json::to_vec(&plan)?)?;
    replay
        .command
        .classpath
        .retain(|path| !path.contains("natives-macos-arm64"));
    let directory = tempfile::tempdir()?;
    let events = Arc::new(AtomicUsize::new(0));
    let observed = events.clone();
    let executor = Executor::new(
        Store::new(directory.path().to_path_buf(), true, Arc::new(|_| {}))?,
        Arc::new(move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
        }),
    );
    let error = executor
        .prepare(replay, &CancellationToken::new())
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("unsupported saved plan prepared"))?;
    assert!(
        error.downcast_ref::<UnsupportedNativeSupport>().is_some(),
        "{error:#}"
    );
    assert!(!directory.path().join("instances").exists());
    assert_eq!(events.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn entire_official_corpus_has_consistent_manifest_and_plan_native_decisions() -> anyhow::Result<()>
{
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../minecraft-manifest/tests/fixtures/official");
    let index: VersionIndex = decode(&fs::read(root.join("version_manifest_v2.json"))?)?;
    let mut eligible = 0;
    for entry in &index.versions {
        let manifest: VersionManifest = decode(&fs::read(
            root.join("versions").join(format!("{}.json", entry.sha1)),
        )?)?;
        let supported = manifest_support(&manifest, &mac_arm64(), &BTreeMap::new())?.is_supported();
        match compiled(manifest, mac_arm64()) {
            Ok(plan) => {
                assert!(
                    supported,
                    "{} compiled despite its native rejection",
                    entry.id
                );
                assert!(
                    plan_support(&plan).is_supported(),
                    "{} lost native evidence",
                    entry.id
                );
                eligible += 1;
            }
            Err(error) => {
                assert!(!supported, "{} failed with {error:#}", entry.id);
                assert!(error.downcast_ref::<UnsupportedNativeSupport>().is_some());
            }
        }
    }
    assert!(eligible > 0 && eligible < index.versions.len());
    eprintln!(
        "macOS ARM64 native eligibility: {eligible}/{} source versions",
        index.versions.len()
    );
    Ok(())
}
