use block_launcher::{
    catalog::{
        Catalog, GameDirectoryGuard, InstanceConfiguration, InstanceSettings, ProfileConfiguration,
        ProfileInheritance,
    },
    recipe::RuntimeSelection,
};
use std::{fs, path::PathBuf};

#[test]
fn legacy_missing_defaults_preserve_java_arguments_and_catalog_bytes() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (_, mut profile) = catalog.create_instance("Legacy".into(), "1.20.6".into(), None, None)?;
    let java = std::env::current_exe()?;
    profile.runtime = RuntimeSelection::Local { java: java.clone() };
    profile.game_args = vec![
        "--width".into(),
        "1200".into(),
        "--height=700".into(),
        "--fullscreen".into(),
        "--server".into(),
        "localhost".into(),
    ];
    catalog.save_profile(profile.clone())?;
    let path = root.path().join("catalog.json");
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    document["snapshot"]["instances"][0]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("instance fixture"))?
        .remove("settings");
    document["snapshot"]["profiles"][0]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("profile fixture"))?
        .remove("inheritance");
    let bytes = serde_json::to_vec(&document)?;
    fs::write(&path, &bytes)?;
    let state = Catalog::open(root.path())?.snapshot()?;
    let (instance, profile, _) = state.selection(&profile.id)?;
    assert!(instance.settings.is_none());
    assert_eq!(profile.inheritance, ProfileInheritance::default());
    assert_eq!(profile.effective_game_args(instance), profile.game_args);
    assert!(
        matches!(profile.effective_runtime(instance), RuntimeSelection::Local { java: path } if path == java)
    );
    assert_eq!(fs::read(path)?, bytes);
    Ok(())
}

#[test]
fn configured_defaults_are_consumed_and_explicit_profile_java_keeps_priority() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let java = std::env::current_exe()?;
    let (instance, profile) = catalog.create_instance_configured(
        "Configured".into(),
        "1.20.6".into(),
        None,
        None,
        ProfileConfiguration {
            name: "Profile".into(),
            runtime: RuntimeSelection::Local { java: java.clone() },
            game_args: vec![
                "--width".into(),
                "1600".into(),
                "--height=900".into(),
                "--fullscreen".into(),
                "--server".into(),
                "example.invalid".into(),
            ],
            ..ProfileConfiguration::default()
        },
    )?;
    let settings = instance
        .settings
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("new instance defaults missing"))?;
    assert_eq!(
        (settings.width, settings.height, settings.fullscreen),
        (1600, 900, true)
    );
    assert_eq!(
        profile.inheritance,
        ProfileInheritance {
            runtime: true,
            window: true
        }
    );
    catalog.save_instance(
        &instance.id,
        InstanceConfiguration {
            name: "Changed".into(),
            settings: InstanceSettings {
                width: 1440,
                height: 810,
                ..InstanceSettings::default()
            },
        },
    )?;
    let state = catalog.snapshot()?;
    let (instance, profile, _) = state.selection(&profile.id)?;
    assert!(
        matches!(profile.effective_runtime(instance), RuntimeSelection::Local { java: path } if path == java)
    );
    assert_eq!(
        profile.effective_game_args(instance),
        [
            "--server",
            "example.invalid",
            "--width",
            "1440",
            "--height",
            "810"
        ]
    );
    assert_eq!(profile.game_args[1], "1600");
    let mut managed = profile.clone();
    managed.runtime = RuntimeSelection::Managed;
    let mut instance = instance.clone();
    instance.settings = Some(InstanceSettings {
        runtime: RuntimeSelection::Local { java: java.clone() },
        ..InstanceSettings::default()
    });
    assert!(
        matches!(managed.effective_runtime(&instance), RuntimeSelection::Local { java: path } if path == java)
    );
    managed.inheritance.runtime = false;
    assert!(matches!(
        managed.effective_runtime(&instance),
        RuntimeSelection::Managed
    ));
    managed.inheritance.window = false;
    assert_eq!(managed.effective_game_args(&instance), managed.game_args);
    Ok(())
}

#[test]
fn instance_save_is_atomic_and_preserves_world_locks_and_mode_names() -> anyhow::Result<()> {
    for separated in [false, true] {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (instance, profile) =
            catalog.create_instance("Original".into(), "1.20.6".into(), None, None)?;
        if separated {
            catalog.separate_profiles()?;
        }
        let directory = profile.game_directory(&instance)?;
        fs::create_dir_all(directory.join("saves/Fixture"))?;
        fs::write(
            directory.join("saves/Fixture/level.dat"),
            b"test-only world marker",
        )?;
        let _guard = GameDirectoryGuard::for_profile(&instance, &profile)?;
        catalog.save_instance(
            &instance.id,
            InstanceConfiguration {
                name: "Renamed".into(),
                settings: InstanceSettings::default(),
            },
        )?;
        let state = Catalog::open(root.path())?.snapshot()?;
        let (saved, saved_profile, _) = state.selection(&profile.id)?;
        assert_eq!(saved.name, "Renamed");
        assert_eq!(
            saved_profile.name,
            if separated { "Original" } else { "Renamed" }
        );
        assert_eq!(saved.directory, instance.directory);
        assert_eq!(saved_profile.game_directory(saved)?, directory);
        assert!(GameDirectoryGuard::for_profile(saved, saved_profile).is_err());
        assert_eq!(
            fs::read(directory.join("saves/Fixture/level.dat"))?,
            b"test-only world marker"
        );
        let before = fs::read(root.path().join("catalog.json"))?;
        for settings in [
            InstanceSettings {
                width: 0,
                ..InstanceSettings::default()
            },
            InstanceSettings {
                height: 1,
                ..InstanceSettings::default()
            },
            InstanceSettings {
                runtime: RuntimeSelection::Local {
                    java: PathBuf::from("relative-java"),
                },
                ..InstanceSettings::default()
            },
        ] {
            assert!(
                catalog
                    .save_instance(
                        &instance.id,
                        InstanceConfiguration {
                            name: "Must not save".into(),
                            settings
                        }
                    )
                    .is_err()
            );
            assert_eq!(fs::read(root.path().join("catalog.json"))?, before);
        }
    }
    Ok(())
}

#[test]
fn invalid_creation_leaves_no_game_directory_and_unknown_fields_preserve_bytes()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let directory = root.path().join("new-game");
    assert!(
        catalog
            .create_instance_configured(
                "Invalid".into(),
                "1.20.6".into(),
                Some(directory.clone()),
                None,
                ProfileConfiguration {
                    game_args: vec!["--width".into(), "invalid".into()],
                    ..ProfileConfiguration::default()
                }
            )
            .is_err()
    );
    assert!(!directory.exists());
    assert!(catalog.snapshot()?.instances.is_empty());
    let (instance, profile) =
        catalog.create_instance("Guarded".into(), "1.20.6".into(), None, None)?;
    let fingerprint = catalog.snapshot()?.launch_fingerprint(&profile.id)?;
    catalog.save_instance(
        &instance.id,
        InstanceConfiguration {
            name: instance.name,
            settings: InstanceSettings {
                width: 1500,
                ..InstanceSettings::default()
            },
        },
    )?;
    assert_ne!(
        catalog.snapshot()?.launch_fingerprint(&profile.id)?,
        fingerprint
    );
    let path = root.path().join("catalog.json");
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    document["snapshot"]["instances"][0]["settings"]["gameDirectory"] =
        serde_json::json!("outside");
    let bytes = serde_json::to_vec(&document)?;
    fs::write(&path, &bytes)?;
    assert!(catalog.snapshot().is_err());
    assert_eq!(fs::read(path)?, bytes);
    Ok(())
}
