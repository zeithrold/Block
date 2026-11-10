mod common;
use block_launcher::catalog::{Catalog, DataPolicy, GameDirectoryGuard};
use std::{fs, sync::Arc};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::symlink_dir;

#[test]
fn explicit_loader_targets_survive_reopen_and_instance_selection() -> anyhow::Result<()> {
    use block_launcher::catalog::{InstanceConfiguration, LoaderKind};
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    for (kind, prefix, minecraft, version) in [
        (LoaderKind::Fabric, "fabric", "1.20.6", "0.16.14"),
        (LoaderKind::Quilt, "quilt", "1.20.6", "0.29.2"),
        (LoaderKind::NeoForge, "neoforge", "1.21.1", "21.1.219"),
        (LoaderKind::NeoForge, "neoforge", "26.3", "26.3.0.95"),
        (
            LoaderKind::NeoForge,
            "neoforge",
            "1.20.1",
            "1.20.1-47.1.106",
        ),
    ] {
        let target_id = format!("{prefix}-{version}");
        let instance = catalog.create_instance_with_loader(
            minecraft.into(),
            None,
            Some((kind.clone(), version.into())),
            InstanceConfiguration {
                name: format!("{prefix} test"),
                target_id: target_id.clone(),
                ..Default::default()
            },
        )?;
        let state = Catalog::open(root.path())?.snapshot()?;
        let (saved_instance, target) = state.selection(&instance.id)?;
        assert_eq!(saved_instance.id, instance.id);
        assert_eq!(saved_instance.targets.len(), 2);
        assert_eq!(saved_instance.target_id, target_id);
        assert_eq!(target.kind, kind);
        assert_eq!(target.loader_version.as_deref(), Some(version));
        assert_eq!(
            target.status,
            block_launcher::catalog::InstallStatus::Pending
        );
    }
    Ok(())
}

#[test]
fn invalid_loader_identity_creates_no_instance_directory_or_catalog_entry() -> anyhow::Result<()> {
    use block_launcher::catalog::{InstanceConfiguration, LoaderKind};
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    for (kind, version) in [
        (LoaderKind::Fabric, "../escape"),
        (LoaderKind::Quilt, "bad/version"),
        (LoaderKind::Vanilla, "1"),
        (LoaderKind::NeoForge, "21.1.219"),
    ] {
        let directory = root.path().join("must-not-exist");
        assert!(
            catalog
                .create_instance_with_loader(
                    "1.20.6".into(),
                    Some(directory.clone()),
                    Some((kind, version.into())),
                    InstanceConfiguration {
                        name: "Invalid".into(),
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert!(!directory.exists());
        assert!(catalog.snapshot()?.instances.is_empty());
    }
    Ok(())
}

#[test]
fn managed_default_is_a_self_contained_game_root() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    assert_eq!(instance.resource_directory, instance.game_directory);
    assert_eq!(
        instance
            .resource_directory
            .file_name()
            .and_then(|name| name.to_str()),
        Some(instance.id.as_str())
    );
    assert_eq!(instance.data_policy, DataPolicy::Isolated);
    assert_eq!(
        instance.ownership,
        block_launcher::catalog::InstanceOwnership::Managed
    );
    assert!(!instance.game_directory.join("profiles").exists());
    Ok(())
}

#[test]
fn preparation_namespace_cannot_be_selected_as_game_or_resource_data() -> anyhow::Result<()> {
    use block_launcher::catalog::InstanceConfiguration;
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let reserved = catalog.root().join("preparations");
    assert!(
        catalog
            .create_instance(
                "Invalid".into(),
                "1.20.6".into(),
                Some(reserved.join("new")),
                None,
            )
            .is_err()
    );
    assert!(!reserved.exists());
    let instance = common::instance(&catalog)?;
    let before = fs::read(catalog.root().join("block.toml"))?;
    for game_directory in [reserved.join("nested"), catalog.root().to_owned()] {
        let mut values = InstanceConfiguration::from(&instance);
        values.game_directory = game_directory;
        assert!(catalog.save_instance(&instance.id, values).is_err());
        assert_eq!(fs::read(catalog.root().join("block.toml"))?, before);
    }
    assert!(!reserved.exists());
    Ok(())
}

#[test]
fn managed_creation_rejects_a_file_location_without_changing_existing_data() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let original = common::instance(&catalog)?;
    let before = fs::read(catalog.root().join("block.toml"))?;
    let file = original.resource_directory.join("options.txt");
    fs::write(&file, b"existing options")?;
    let error = catalog
        .create_instance("Invalid".into(), "1.20.6".into(), Some(file.clone()), None)
        .err()
        .ok_or_else(|| anyhow::anyhow!("A file was accepted as a directory"))?;
    assert!(
        error
            .to_string()
            .contains("selected path is not a directory"),
        "{error:#}"
    );
    assert_eq!(fs::read(catalog.root().join("block.toml"))?, before);
    assert_eq!(fs::read(file)?, b"existing options");
    Ok(())
}

#[test]
fn deleting_instance_keeps_resources_worlds_and_other_instances() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    let other = common::instance(&catalog)?;
    let saves = instance.game_directory.join("saves/World");
    fs::create_dir_all(&saves)?;
    fs::write(saves.join("level.dat"), b"existing world")?;
    let immutable = root.path().join("objects");
    fs::create_dir_all(&immutable)?;
    fs::write(immutable.join("sentinel"), b"cached")?;
    catalog.delete_instance(&instance.id)?;
    assert_eq!(fs::read(saves.join("level.dat"))?, b"existing world");
    assert_eq!(fs::read(immutable.join("sentinel"))?, b"cached");
    let state = catalog.snapshot()?;
    assert_eq!(state.instances.len(), 1);
    assert_eq!(state.instances[0].id, other.id);
    Ok(())
}

#[test]
fn canonical_game_directory_lock_excludes_second_writer_and_releases() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let directory = root.path().join("game");
    let guard = GameDirectoryGuard::acquire(&directory)?;
    assert!(GameDirectoryGuard::acquire(&directory.join(".")).is_err());
    drop(guard);
    assert!(GameDirectoryGuard::acquire(&directory).is_ok());
    Ok(())
}

#[test]
fn corrupt_catalog_is_never_silently_reset_or_overwritten() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let original = b"{broken user catalog";
    fs::write(root.path().join("block.toml"), original)?;
    assert!(catalog.snapshot().is_err());
    assert!(
        catalog
            .create_instance("New".into(), "1.20.6".into(), None, None)
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("block.toml"))?, original);
    Ok(())
}

#[test]
fn catalog_serializes_concurrent_creates_without_lost_updates() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Arc::new(Catalog::open(root.path())?);
    let mut jobs = Vec::new();
    for index in 0..4 {
        let catalog = catalog.clone();
        jobs.push(std::thread::spawn(move || {
            catalog.create_instance(format!("Game{index}"), "1.20.6".into(), None, None)
        }));
    }
    for job in jobs {
        job.join()
            .map_err(|_| anyhow::anyhow!("catalog worker failed"))??;
    }
    let state = catalog.snapshot()?;
    assert_eq!(state.instances.len(), 4);
    Ok(())
}

#[test]
fn instance_validation_rejects_managed_process_argument_overrides() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    let original = fs::read(common::path(root.path()))?;
    for argument in [
        "--accessToken=untrusted",
        "--gameDir",
        "--gameDir=/elsewhere",
        "--uuid=other",
        "--username=other",
    ] {
        let mut configuration = block_launcher::catalog::InstanceConfiguration::from(&instance);
        configuration.game_args = vec![argument.into()];
        assert!(
            catalog.save_instance(&instance.id, configuration).is_err(),
            "{argument}"
        );
        assert_eq!(fs::read(common::path(root.path()))?, original);
    }
    Ok(())
}

#[test]
fn overlapping_roots_are_legal_but_equal_actual_game_directories_share_a_lock() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path().join("catalog"))?;
    let directory = root.path().join("game");
    let first =
        catalog.create_instance("A".into(), "1.20.6".into(), Some(directory.clone()), None)?;
    let second = catalog.create_instance(
        "B".into(),
        "1.20.6".into(),
        Some(directory.join("versions/V")),
        None,
    )?;
    let shared = catalog.create_instance("C".into(), "1.20.6".into(), Some(directory), None)?;
    let _first = GameDirectoryGuard::for_instance(&first)?;
    let _second = GameDirectoryGuard::for_instance(&second)?;
    assert!(GameDirectoryGuard::for_instance(&shared).is_err());
    Ok(())
}

#[test]
fn planned_game_directory_replaced_by_symlink_is_rejected_without_writes() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let mut instance = common::instance(&catalog)?;
    let original = root.path().join("other");
    fs::create_dir(&original)?;
    instance.game_directory = instance.resource_directory.join("planned");
    symlink_dir(&original, &instance.game_directory)?;
    assert!(GameDirectoryGuard::for_instance(&instance).is_err());
    assert!(!original.join(".block-game.lock").exists());
    Ok(())
}

#[cfg(windows)]
#[test]
fn planned_game_directory_replaced_by_junction_is_rejected() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let mut instance = common::instance(&catalog)?;
    let original = root.path().join("other");
    fs::create_dir(&original)?;
    instance.game_directory = instance.resource_directory.join("planned");
    let output = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&instance.game_directory)
        .arg(&original)
        .output()?;
    anyhow::ensure!(output.status.success(), "could not create fixture junction");
    assert!(GameDirectoryGuard::for_instance(&instance).is_err());
    assert!(!original.join(".block-game.lock").exists());
    Ok(())
}
