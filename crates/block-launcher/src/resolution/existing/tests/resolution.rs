use super::super::manifests;
use super::fixtures::{Fixture, write};
use crate::plan::{PrepareAction, ResourcePath};

#[tokio::test]
async fn identical_content_library_and_client_keep_their_declared_source_locations()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let library = fixture
        .root()
        .join("libraries/example/override/1/override-1.jar");
    fs::write(&library, b"client fixture")?;
    let metadata = fixture.resolve().await?;
    let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    assert!(
        plan.command
            .classpath
            .contains(&ResourcePath::Existing { path: library })
    );
    assert_eq!(
        plan.command.classpath.last(),
        Some(&ResourcePath::Existing {
            path: fixture.root().join("versions/1.20.1/1.20.1.jar")
        })
    );
    Ok(())
}
use anyhow::Context as _;
use serde_json::json;
use std::fs;

#[cfg(unix)]
#[tokio::test]
async fn registered_pending_instance_prepares_play_without_repair_or_source_copy()
-> anyhow::Result<()> {
    use crate::{CancellationToken, catalog::InstallStatus, session::OfflineSession};
    use std::sync::Arc;

    let mut fixture = Fixture::new()?;
    let (library, instance) = fixture.registered(Arc::new(|_| {}))?;
    assert_eq!(instance.targets[0].status, InstallStatus::Pending);
    let manifest_path = fixture.root().join("versions/1.20.1/1.20.1.json");
    let before = fs::read(&manifest_path)?;
    let cancel = CancellationToken::new();
    let session = OfflineSession::Offline {
        username: "Player".into(),
    };
    let plan = library
        .prepare_launch(&instance.id, &session, &cancel, None, None)
        .await?;
    assert_eq!(plan.game_directory(), fixture.root());
    assert_eq!(plan.preview().cwd, fixture.root());
    assert!(plan.preview().arguments.iter().any(|argument| {
        argument.contains(
            &fixture
                .root()
                .join("versions/1.20.1/1.20.1.jar")
                .to_string_lossy()
                .to_string(),
        )
    }));
    assert!(
        library
            .prepare_launch(&instance.id, &session, &cancel, None, None)
            .await
            .is_err()
    );
    assert_eq!(fs::read(&manifest_path)?, before);
    assert!(!library.catalog().root().join("objects").exists());
    assert_eq!(
        library
            .catalog()
            .snapshot()?
            .selection(&instance.id)?
            .1
            .status,
        InstallStatus::Ready
    );
    drop(plan);
    let _reopened = library
        .prepare_launch(
            &instance.id,
            &OfflineSession::Offline {
                username: "Player".into(),
            },
            &cancel,
            None,
            None,
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn vanilla_uses_verified_source_classpath_and_complete_assets_without_copy_actions()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let before = fs::read(fixture.root().join("versions/1.20.1/1.20.1.json"))?;
    let metadata = fixture.resolve().await?;
    let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    plan.verify_existing_resources()?;
    assert_eq!(
        plan.existing_asset_root,
        Some(fixture.root().join("assets"))
    );
    assert!(plan.command.classpath.contains(&ResourcePath::Existing {
        path: fixture.root().join("versions/1.20.1/1.20.1.jar")
    }));
    assert!(plan.resources.is_empty());
    assert!(
        !plan
            .actions
            .iter()
            .any(|action| matches!(action, PrepareAction::Copy { .. }))
    );
    assert_eq!(
        fs::read(fixture.root().join("versions/1.20.1/1.20.1.json"))?,
        before
    );
    assert!(!fixture.context.store.root().join("views").exists());
    assert_eq!(plan.command.game_args[1], "${game_directory}");
    Ok(())
}

#[tokio::test]
async fn inherited_loaders_keep_child_entrypoint_library_precedence_and_module_inputs()
-> anyhow::Result<()> {
    for main in [
        "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "org.quiltmc.loader.impl.launch.knot.KnotClient",
        "cpw.mods.modlauncher.Launcher",
        "cpw.mods.bootstraplauncher.BootstrapLauncher",
    ] {
        let mut fixture = Fixture::new()?;
        let relative = "example/override/2/override-2.jar";
        write(
            &fixture.root().join("libraries").join(relative),
            b"child library",
        )?;
        fixture.select("loader", json!({
            "id":"loader","inheritsFrom":"1.20.1","mainClass":main,
            "libraries":[{"name":"example:override:2"}],
            "arguments":{"game":["--tweakClass","fixture.Tweaker"],"jvm":["-p",format!("${{library_directory}}/{relative}")]}
        }))?;
        let metadata = fixture.resolve().await?;
        let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
        assert_eq!(plan.command.main_class, main);
        assert_eq!(
            plan.command.classpath[0],
            ResourcePath::Existing {
                path: fixture.root().join("libraries").join(relative)
            }
        );
        assert!(
            !plan.command.classpath.contains(&ResourcePath::Existing {
                path: fixture
                    .root()
                    .join("libraries/example/override/1/override-1.jar")
            })
        );
        assert!(plan.command.path_variables.values().any(|value| *value
            == ResourcePath::Existing {
                path: fixture.root().join("libraries").join(relative)
            }));
        assert!(plan.command.game_args.contains(&"fixture.Tweaker".into()));
    }
    Ok(())
}

#[test]
fn inheritance_cycles_and_unsupported_or_unbounded_metadata_are_rejected() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.select("cycle", json!({"id":"cycle","inheritsFrom":"cycle"}))?;
    assert!(
        manifests::read_chain(fixture.root(), "cycle")
            .err()
            .context("cycle accepted")?
            .to_string()
            .contains("cyclic")
    );
    assert!(manifests::read_chain(fixture.root(), "../escape").is_err());
    fixture.select("missing", json!({"id":"missing","inheritsFrom":"absent"}))?;
    assert!(
        manifests::read_chain(fixture.root(), "missing")
            .err()
            .context("missing accepted")?
            .to_string()
            .contains("Missing")
    );
    write(
        &fixture.root().join("versions/huge/huge.json"),
        &vec![b' '; 8 * 1024 * 1024 + 1],
    )?;
    assert!(
        manifests::read_chain(fixture.root(), "huge")
            .err()
            .context("oversized metadata accepted")?
            .to_string()
            .contains("too large")
    );
    Ok(())
}
