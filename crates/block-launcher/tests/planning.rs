use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use block_launcher::{
    command::JavaCommand,
    executor::confined_path,
    plan::compile,
    platform::{Architecture, OperatingSystem, Target},
    recipe::{Compatibility, LaunchRecipe, ParsedRecipe, RecipeSource, RuntimeSelection},
    resolver::ResolvedMetadata,
    rules::allowed,
    session::{OfflineSession, SessionProvider as _},
};
use minecraft_manifest::{
    decode,
    model::{Rule, VersionIndex, VersionManifest},
    resources::AssetObjects,
};
use serde_json::json;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../minecraft-manifest/tests/fixtures")
}

fn recipe() -> anyhow::Result<LaunchRecipe> {
    Ok(LaunchRecipe {
        schema_version: 1,
        instance: "integration".into(),
        source: RecipeSource::Mojang {
            version: "1.20.6".into(),
        },
        session: OfflineSession::Offline {
            username: "RecipePlayer".into(),
        },
        runtime: RuntimeSelection::Local {
            java: std::env::current_exe()?,
        },
        target: None,
        compatibility: Compatibility::Official,
        memory_mib: 1024,
        jvm_args: Vec::new(),
        features: BTreeMap::new(),
    })
}

fn target(os: OperatingSystem) -> Target {
    Target {
        os,
        arch: Architecture::X86_64,
        os_version: "10.0.22000".into(),
    }
}

fn asset_fixtures() -> anyhow::Result<BTreeMap<String, AssetObjects>> {
    let mut assets = BTreeMap::new();
    for entry in fs::read_dir(fixtures().join("resources"))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid filename"))?;
        if name.starts_with("asset-") {
            let sha1 = name
                .strip_suffix(".json")
                .and_then(|name| name.rsplit('-').next())
                .ok_or_else(|| anyhow::anyhow!("missing asset digest"))?;
            assets.insert(sha1.to_owned(), decode(&fs::read(path)?)?);
        }
    }
    Ok(assets)
}

#[test]
fn all_919_versions_compile_for_the_three_x64_os_families() -> anyhow::Result<()> {
    let root = fixtures().join("official");
    let index: VersionIndex = decode(&fs::read(root.join("version_manifest_v2.json"))?)?;
    let assets = asset_fixtures()?;
    let recipe = recipe()?;
    let mut count = 0;
    for entry in &index.versions {
        let manifest: VersionManifest = decode(&fs::read(
            root.join("versions").join(format!("{}.json", entry.sha1)),
        )?)?;
        for os in [
            OperatingSystem::Windows,
            OperatingSystem::Linux,
            OperatingSystem::Macos,
        ] {
            let metadata = ResolvedMetadata {
                parsed: ParsedRecipe {
                    manifest: manifest.clone(),
                    manifest_sha1: entry.sha1.clone(),
                },
                target: target(os),
                assets: assets
                    .get(&manifest.asset_index.sha1)
                    .ok_or_else(|| anyhow::anyhow!("missing asset fixture"))?
                    .clone(),
                runtime: None,
                native_overrides: BTreeMap::new(),
            };
            let plan = compile(&recipe, &metadata)
                .map_err(|error| anyhow::anyhow!("{} / {os:?}: {error:#}", entry.id))?;
            assert_eq!(plan.version, entry.id);
            assert_eq!(plan.command.main_class, manifest.main_class);
            assert!(plan.resources.len() >= metadata.assets.objects.len() + 2);
            let directory = tempfile::tempdir()?;
            let command =
                JavaCommand::bind(&plan, directory.path(), "fixed-run-id", &recipe.session)?;
            assert!(
                command
                    .preview()
                    .arguments
                    .iter()
                    .all(|argument| !argument.contains("${"))
            );
            count += 1;
        }
    }
    assert_eq!(count, 919 * 3);
    Ok(())
}

#[test]
fn ordered_rules_features_and_windows_range_boundary_select_one_gc() -> anyhow::Result<()> {
    let rules: Vec<Rule> = serde_json::from_value(json!([
        {"action":"allow"}, {"action":"disallow","os":{"name":"windows"}},
        {"action":"allow","os":{"name":"windows","versionRange":{"min":"10.0.17134"}},"features":{"test":true}}
    ]))?;
    let features = BTreeMap::from([("test".into(), true)]);
    let mut windows = target(OperatingSystem::Windows);
    windows.os_version = "10.0.17133".into();
    assert!(!allowed(Some(&rules), &windows, &features)?);
    windows.os_version = "10.0.17134".into();
    assert!(allowed(Some(&rules), &windows, &features)?);
    assert!(!allowed(Some(&rules), &windows, &BTreeMap::new())?);
    let maximum: Vec<Rule> = serde_json::from_value(
        json!([{"action":"allow","os":{"versionRange":{"max":"10.0.17134"}}}]),
    )?;
    assert!(!allowed(Some(&maximum), &windows, &features)?);
    assert!(allowed(None, &windows, &features)?);
    assert!(allowed(Some(&[]), &windows, &features)?);
    Ok(())
}

#[test]
fn saved_plans_bind_identity_late_and_preserve_argument_boundaries() -> anyhow::Result<()> {
    let mut recipe = recipe()?;
    recipe.jvm_args.push("-Dtest.value=contains spaces".into());
    let manifest: VersionManifest = decode(&fs::read(fixtures().join("26.3.json"))?)?;
    let metadata = ResolvedMetadata {
        parsed: ParsedRecipe {
            manifest,
            manifest_sha1: "fixture".into(),
        },
        target: target(OperatingSystem::Macos),
        assets: AssetObjects {
            objects: BTreeMap::new(),
            virtual_layout: None,
            map_to_resources: None,
        },
        runtime: None,
        native_overrides: BTreeMap::new(),
    };
    let plan = compile(&recipe, &metadata)?;
    let serialized = serde_json::to_string(&plan)?;
    assert!(!serialized.contains("RecipePlayer"));
    let plan = serde_json::from_str(&serialized)?;
    let directory = tempfile::Builder::new()
        .prefix("block path with spaces ")
        .tempdir()?;
    let identity = OfflineSession::Offline {
        username: "OtherPlayer".into(),
    };
    let command = JavaCommand::bind(&plan, directory.path(), "test-run", &identity)?;
    let args = command.preview().arguments;
    assert!(args.contains(&"OtherPlayer".into()));
    assert!(args.contains(&"-Dtest.value=contains spaces".into()));
    assert!(!format!("{command:?}").contains("auth_access_token"));
    Ok(())
}

#[test]
fn offline_uuid_matches_java_name_uuid_and_paths_reject_cross_platform_escapes()
-> anyhow::Result<()> {
    let session = OfflineSession::Offline {
        username: "Steve".into(),
    }
    .session()?;
    assert_eq!(session.uuid, "5627dd98e6be3c21b8a8e92344183641");
    for path in [
        "../escape",
        "/absolute",
        "C:/escape",
        "sub\\escape",
        "safe/../../escape",
        "a:stream",
    ] {
        assert!(confined_path(Path::new("root"), path).is_err(), "{path}");
    }
    assert!(
        OfflineSession::Offline {
            username: "bad name".into()
        }
        .session()
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn recipe_relative_paths_resolve_against_the_document_not_process_cwd() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let document = root.path().join("launch.json");
    let value = json!({
        "schema_version":1,"instance":"relative",
        "source":{"kind":"local_manifest","path":"version.json"},
        "runtime":{"kind":"local","java":"runtime/bin/java"},
        "session":{"kind":"offline","username":"Player"}
    });
    tokio::fs::write(&document, serde_json::to_vec(&value)?).await?;
    let recipe = LaunchRecipe::load(&document).await?;
    let RecipeSource::LocalManifest { path } = recipe.source else {
        anyhow::bail!("wrong source");
    };
    let RuntimeSelection::Local { java } = recipe.runtime else {
        anyhow::bail!("wrong runtime");
    };
    assert_eq!(path, root.path().canonicalize()?.join("version.json"));
    assert_eq!(java, root.path().canonicalize()?.join("runtime/bin/java"));
    Ok(())
}

#[test]
fn provider_secrets_are_redacted_even_when_logs_only_print_the_raw_token() -> anyhow::Result<()> {
    struct Provider;
    impl block_launcher::session::SessionProvider for Provider {
        fn session(&self) -> anyhow::Result<block_launcher::session::GameSession> {
            Ok(block_launcher::session::GameSession {
                username: "Player".into(),
                uuid: "uuid".into(),
                access_token: "SENSITIVE_SESSION_TOKEN".into(),
                user_type: "msa".into(),
                xuid: "0".into(),
                client_id: String::new(),
            })
        }
    }
    let manifest: VersionManifest = decode(&fs::read(fixtures().join("1.12.2.json"))?)?;
    let metadata = ResolvedMetadata {
        parsed: ParsedRecipe {
            manifest,
            manifest_sha1: "fixture".into(),
        },
        target: target(OperatingSystem::Linux),
        assets: AssetObjects {
            objects: BTreeMap::new(),
            virtual_layout: None,
            map_to_resources: None,
        },
        runtime: None,
        native_overrides: BTreeMap::new(),
    };
    let mut plan = compile(&recipe()?, &metadata)?;
    plan.command.game_args = vec!["${auth_session}".into()];
    let root = tempfile::tempdir()?;
    let command = JavaCommand::bind(&plan, root.path(), "run", &Provider)?;
    assert!(!format!("{command:?}").contains("SENSITIVE_SESSION_TOKEN"));
    assert_eq!(
        command.redact("token is SENSITIVE_SESSION_TOKEN"),
        "token is <redacted>"
    );
    assert!(!serde_json::to_string(&command.preview())?.contains("SENSITIVE_SESSION_TOKEN"));
    Ok(())
}
