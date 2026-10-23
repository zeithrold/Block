use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use anyhow::Context as _;
use block_download::{Resource, Store};
use block_launcher::{
    CancellationToken,
    plan::{CommandTemplate, ExecutionPlan, JavaExecutable, PrepareAction},
    platform::{Architecture, OperatingSystem, Target},
    repair::quarantine_corrupt_copies,
};
use sha1::{Digest as _, Sha1};

fn store(root: &Path) -> anyhow::Result<Store> {
    Store::new(root.to_owned(), true, Arc::new(|_| {}))
}

fn resource(store: &Store, bytes: &[u8]) -> anyhow::Result<Resource> {
    let resource = Resource {
        sha1: format!("{:x}", Sha1::digest(bytes)),
        size: bytes.len() as u64,
        url: String::new(),
    };
    let path = store.object_path(&resource.sha1)?;
    fs::create_dir_all(path.parent().context("object directory")?)?;
    fs::write(path, bytes)?;
    Ok(resource)
}

fn plan(resource: Resource, destination: &str) -> ExecutionPlan {
    ExecutionPlan {
        schema_version: 1,
        instance: "repair-test".into(),
        version: "test".into(),
        manifest_sha1: String::new(),
        target: Target {
            os: OperatingSystem::Linux,
            arch: Architecture::X86_64,
            os_version: String::new(),
        },
        actions: vec![PrepareAction::Copy {
            sha1: resource.sha1.clone(),
            path: destination.into(),
            executable: false,
        }],
        resources: vec![resource],
        compatibility: None,
        runtime_substitution: None,
        command: CommandTemplate {
            java: JavaExecutable::Managed {
                path: "runtimes/test/bin/java".into(),
            },
            java_major: 21,
            main_class: "test.Main".into(),
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            classpath: Vec::new(),
            variables: BTreeMap::new(),
            path_variables: BTreeMap::new(),
            cwd: "instances/repair-test/game".into(),
        },
    }
}

#[test]
fn same_size_corrupt_copy_is_preserved_and_validated_cache_remains_available() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let store = store(root.path())?;
    let resource = resource(&store, b"good jar bytes")?;
    let plan = plan(resource.clone(), "views/libraries/test/library.jar");
    let destination = root.path().join("views/libraries/test/library.jar");
    fs::create_dir_all(destination.parent().context("view directory")?)?;
    fs::write(&destination, b"evil jar bytes")?;
    let quarantined =
        quarantine_corrupt_copies(&store, &plan, "repair-1", &CancellationToken::new())?;
    assert_eq!(quarantined.len(), 1);
    assert_eq!(
        quarantined[0].original,
        store
            .root()
            .canonicalize()?
            .join("views/libraries/test/library.jar")
    );
    assert_eq!(
        quarantined[0].quarantine.parent(),
        quarantined[0].original.parent()
    );
    assert!(!destination.exists());
    assert_eq!(fs::read(&quarantined[0].quarantine)?, b"evil jar bytes");
    assert_eq!(
        fs::read(store.object_path(&resource.sha1)?)?,
        b"good jar bytes"
    );
    // The later materializer sees a missing view and can recreate it from the verified CAS.
    assert_eq!(
        quarantine_corrupt_copies(&store, &plan, "repair-2", &CancellationToken::new())?.len(),
        0
    );
    Ok(())
}

#[test]
fn valid_resources_and_all_mutable_game_files_are_untouched() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let store = store(root.path())?;
    let resource = resource(&store, b"good")?;
    let mut plan = plan(resource, "views/clients/test/client.jar");
    let client = root.path().join("views/clients/test/client.jar");
    fs::create_dir_all(client.parent().context("client directory")?)?;
    fs::write(&client, b"good")?;
    for path in [
        "instances/repair-test/game/saves/world/level.dat",
        "instances/repair-test/game/mods/user.jar",
        "instances/repair-test/game/config/user.json",
        "instances/repair-test/game/options.txt",
        "instances/repair-test/game/resources/user.txt",
    ] {
        let destination = root.path().join(path);
        fs::create_dir_all(destination.parent().context("game directory")?)?;
        fs::write(&destination, b"user")?;
        plan.actions.push(PrepareAction::Copy {
            sha1: plan.resources[0].sha1.clone(),
            path: path.into(),
            executable: false,
        });
    }
    assert_eq!(
        quarantine_corrupt_copies(&store, &plan, "repair-1", &CancellationToken::new())?.len(),
        0
    );
    assert_eq!(fs::read(client)?, b"good");
    for action in &plan.actions[1..] {
        if let PrepareAction::Copy { path, .. } = action {
            assert_eq!(fs::read(root.path().join(path))?, b"user");
        }
    }
    Ok(())
}

#[test]
fn corrupt_source_blocks_every_rename_even_when_an_earlier_view_is_corrupt() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let store = store(root.path())?;
    let first = resource(&store, b"first verified")?;
    let second = resource(&store, b"second verified")?;
    let mut plan = plan(first, "views/libraries/first.jar");
    plan.actions.push(PrepareAction::Copy {
        sha1: second.sha1.clone(),
        path: "views/libraries/second.jar".into(),
        executable: false,
    });
    plan.resources.push(second.clone());
    fs::create_dir_all(root.path().join("views/libraries"))?;
    fs::write(
        root.path().join("views/libraries/first.jar"),
        b"first corrupt",
    )?;
    fs::write(
        root.path().join("views/libraries/second.jar"),
        b"second corrupt",
    )?;
    fs::write(store.object_path(&second.sha1)?, b"bad cache")?;
    assert!(
        quarantine_corrupt_copies(&store, &plan, "repair-1", &CancellationToken::new()).is_err()
    );
    assert_eq!(
        fs::read(root.path().join("views/libraries/first.jar"))?,
        b"first corrupt"
    );
    assert_eq!(
        fs::read(root.path().join("views/libraries/second.jar"))?,
        b"second corrupt"
    );
    assert_eq!(
        fs::read_dir(root.path().join("views/libraries"))?.count(),
        2
    );
    Ok(())
}

#[test]
fn undeclared_resources_traversal_and_directories_fail_without_mutation() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let store = store(root.path())?;
    let resource = resource(&store, b"good")?;
    let mut invalid = plan(resource.clone(), "views/libraries/tool.jar");
    invalid.resources.clear();
    assert!(
        quarantine_corrupt_copies(&store, &invalid, "repair-1", &CancellationToken::new()).is_err()
    );
    for path in [
        "views/../../outside",
        "views\\libraries\\bad.jar",
        "C:/outside",
        "/outside",
    ] {
        assert!(
            quarantine_corrupt_copies(
                &store,
                &plan(resource.clone(), path),
                "repair-1",
                &CancellationToken::new()
            )
            .is_err()
        );
    }
    let destination = root.path().join("views/libraries/directory.jar");
    fs::create_dir_all(&destination)?;
    assert!(
        quarantine_corrupt_copies(
            &store,
            &plan(resource, "views/libraries/directory.jar"),
            "repair-1",
            &CancellationToken::new()
        )
        .is_err()
    );
    assert!(destination.is_dir());
    Ok(())
}

#[test]
fn cancellation_preserves_corrupt_copy() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let store = store(root.path())?;
    let resource = resource(&store, b"good")?;
    let plan = plan(resource, "runtimes/test/bin/java");
    let destination = root.path().join("runtimes/test/bin/java");
    fs::create_dir_all(destination.parent().context("runtime directory")?)?;
    fs::write(&destination, b"bad")?;
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(quarantine_corrupt_copies(&store, &plan, "repair-1", &cancel).is_err());
    assert_eq!(fs::read(destination)?, b"bad");
    Ok(())
}

#[test]
fn symlinked_immutable_paths_never_modify_the_target() -> anyhow::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::{symlink as symlink_dir, symlink as symlink_file};
    #[cfg(windows)]
    use std::os::windows::fs::{symlink_dir, symlink_file};
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let store = store(root.path())?;
    let resource = resource(&store, b"good")?;
    fs::write(outside.path().join("tool.jar"), b"user")?;
    fs::create_dir_all(root.path().join("views"))?;
    symlink_dir(outside.path(), root.path().join("views/libraries"))?;
    assert!(
        quarantine_corrupt_copies(
            &store,
            &plan(resource.clone(), "views/libraries/tool.jar"),
            "repair-1",
            &CancellationToken::new()
        )
        .is_err()
    );
    assert_eq!(fs::read(outside.path().join("tool.jar"))?, b"user");
    #[cfg(unix)]
    fs::remove_file(root.path().join("views/libraries"))?;
    #[cfg(windows)]
    fs::remove_dir(root.path().join("views/libraries"))?;
    fs::create_dir(root.path().join("views/libraries"))?;
    symlink_file(
        outside.path().join("tool.jar"),
        root.path().join("views/libraries/tool.jar"),
    )?;
    assert!(
        quarantine_corrupt_copies(
            &store,
            &plan(resource, "views/libraries/tool.jar"),
            "repair-1",
            &CancellationToken::new()
        )
        .is_err()
    );
    assert_eq!(fs::read(outside.path().join("tool.jar"))?, b"user");
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_in_use_resource_requires_stopping_the_game() -> anyhow::Result<()> {
    use std::os::windows::fs::OpenOptionsExt as _;
    let root = tempfile::tempdir()?;
    let store = store(root.path())?;
    let resource = resource(&store, b"good")?;
    let plan = plan(resource, "views/libraries/tool.jar");
    let destination = root.path().join("views/libraries/tool.jar");
    fs::create_dir_all(destination.parent().context("library directory")?)?;
    fs::write(&destination, b"bad")?;
    // This models an active game reader sharing reads but refusing delete/rename.
    let _active = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&destination)?;
    let error = quarantine_corrupt_copies(&store, &plan, "repair-1", &CancellationToken::new())
        .err()
        .context("repair unexpectedly renamed an active resource")?;
    assert!(error.to_string().contains("stop running games"));
    assert_eq!(fs::read(destination)?, b"bad");
    Ok(())
}
