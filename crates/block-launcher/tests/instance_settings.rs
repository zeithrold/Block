mod common;
use block_launcher::{
    catalog::{Catalog, DataPolicy, GameDirectoryGuard, InstanceConfiguration},
    recipe::RuntimeSelection,
};
use std::fs;

#[test]
fn typed_window_settings_override_only_owned_extra_flags() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let java = std::env::current_exe()?;
    let instance = catalog.create_instance_configured(
        "1.20.6".into(),
        None,
        None,
        InstanceConfiguration {
            name: "Configured".into(),
            runtime: RuntimeSelection::Local { java: java.clone() },
            game_args: vec![
                "--width".into(),
                "1600".into(),
                "--height=900".into(),
                "--fullscreen=true".into(),
                "--server".into(),
                "example.invalid".into(),
            ],
            width: 1440,
            height: 810,
            ..Default::default()
        },
    )?;
    assert!(matches!(&instance.runtime, RuntimeSelection::Local { java: path } if *path == java));
    assert_eq!(
        instance.launch_game_args(),
        [
            "--server",
            "example.invalid",
            "--width",
            "1440",
            "--height",
            "810"
        ]
    );
    let mut changed = instance.clone();
    changed.fullscreen = true;
    changed.game_args = vec![
        "--width".into(),
        "--server".into(),
        "localhost".into(),
        "--fullscreen".into(),
    ];
    assert_eq!(
        changed.launch_game_args(),
        [
            "--server",
            "localhost",
            "--width",
            "1440",
            "--height",
            "810",
            "--fullscreen"
        ]
    );
    assert_eq!(instance.game_args[1], "1600");
    Ok(())
}

#[test]
fn instance_save_is_atomic_and_preserves_worlds_and_live_locks() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    let world = instance.game_directory.join("saves/Fixture");
    fs::create_dir_all(&world)?;
    let world = world.join("level.dat");
    fs::write(&world, b"test-only world marker")?;
    let _guard = GameDirectoryGuard::for_instance(&instance)?;
    let mut configuration = InstanceConfiguration::from(&instance);
    configuration.name = "Renamed".into();
    configuration.width = 1500;
    catalog.save_instance(&instance.id, configuration.clone())?;
    let state = Catalog::open(root.path())?.snapshot()?;
    let (saved, _) = state.selection(&instance.id)?;
    assert_eq!(saved.name, "Renamed");
    assert_eq!(saved.width, 1500);
    assert_eq!(saved.game_directory, instance.game_directory);
    assert!(GameDirectoryGuard::for_instance(saved).is_err());
    assert_eq!(fs::read(&world)?, b"test-only world marker");
    let before = fs::read(common::path(root.path()))?;
    for change in [0, 1, 2, 3, 4] {
        let mut invalid = configuration.clone();
        match change {
            0 => invalid.width = 0,
            1 => invalid.height = 1,
            2 => {
                invalid.runtime = RuntimeSelection::Local {
                    java: "relative-java".into(),
                }
            }
            3 => invalid.game_directory = instance.game_directory.join("new-place"),
            4 => invalid.data_policy = DataPolicy::Shared,
            _ => unreachable!(),
        }
        assert!(catalog.save_instance(&instance.id, invalid).is_err());
        assert_eq!(fs::read(common::path(root.path()))?, before);
    }
    assert!(!instance.game_directory.join("new-place").exists());
    Ok(())
}

#[test]
fn changed_data_location_selects_a_path_without_moving_or_creating_files() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    fs::write(
        instance.game_directory.join("options.txt"),
        b"original options",
    )?;
    let new_directory = instance.resource_directory.join("future/game");
    let mut configuration = InstanceConfiguration::from(&instance);
    configuration.game_directory.clone_from(&new_directory);
    catalog.save_instance(&instance.id, configuration)?;
    assert!(!new_directory.exists());
    assert_eq!(
        fs::read(instance.game_directory.join("options.txt"))?,
        b"original options"
    );
    let saved = catalog.snapshot()?.selection(&instance.id)?.0.clone();
    assert_eq!(saved.game_directory, new_directory);
    let guard = GameDirectoryGuard::for_instance(&saved)?;
    assert_eq!(guard.directory(), new_directory);
    Ok(())
}

#[test]
fn invalid_creation_and_unknown_fields_leave_catalog_bytes_unchanged() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let directory = root.path().join("new-game");
    assert!(
        catalog
            .create_instance_configured(
                "1.20.6".into(),
                Some(directory.clone()),
                None,
                InstanceConfiguration {
                    name: "Invalid".into(),
                    width: 0,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert!(!directory.exists());
    assert!(catalog.snapshot()?.instances.is_empty());
    common::instance(&catalog)?;
    let mut document = common::load(root.path())?;
    common::first_instance_mut(&mut document)?.insert("gameDirectory".into(), "outside".into());
    common::save(root.path(), &document)?;
    let before = fs::read(common::path(root.path()))?;
    assert!(catalog.snapshot().is_err());
    assert_eq!(fs::read(common::path(root.path()))?, before);
    Ok(())
}

#[test]
fn sticky_session_rejects_directory_changes_and_survives_metadata_deletion() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    let run = uuid::Uuid::new_v4().to_string();
    let guard = GameDirectoryGuard::for_session(&instance, &run)?;
    drop(guard);
    let mut configuration = InstanceConfiguration::from(&instance);
    configuration.game_directory = instance.game_directory.join("other");
    assert!(catalog.save_instance(&instance.id, configuration).is_err());
    catalog.delete_instance(&instance.id)?;
    assert!(GameDirectoryGuard::for_instance(&instance).is_err());
    GameDirectoryGuard::clear_session_reservation(&instance.game_directory, &run)?;
    assert!(GameDirectoryGuard::for_instance(&instance).is_ok());
    Ok(())
}
