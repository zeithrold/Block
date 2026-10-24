mod common;

use std::fs;

use block_launcher::{
    catalog::{
        Catalog, DataPolicy, ProfileAppearance, ProfileColor, ProfileConfiguration, ProfileIcon,
    },
    recipe::RuntimeSelection,
};

#[test]
fn legacy_profiles_default_appearance_without_rewriting_the_catalog() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (_, profile) = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    let path = common::path(root.path());
    let mut document = common::load(root.path())?;
    common::first_profile_mut(&mut document)?.remove("appearance");
    let original = toml::to_string_pretty(&document)?.into_bytes();
    fs::write(&path, &original)?;

    let reopened = Catalog::open(root.path())?.snapshot()?;
    assert_eq!(
        reopened.selection(&profile.id)?.1.appearance,
        ProfileAppearance::default()
    );
    assert_eq!(fs::read(&path)?, original);
    Ok(())
}

#[test]
fn appearance_survives_save_copy_and_reopen_without_copying_worlds() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, mut profile) =
        catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    assert_eq!(profile.appearance, ProfileAppearance::default());
    let world = profile.game_directory(&instance)?.join("saves/Test world");
    fs::create_dir_all(&world)?;
    fs::write(world.join("level.dat"), b"isolated test world")?;
    profile.appearance = ProfileAppearance {
        icon: ProfileIcon::FlaskConical,
        color: ProfileColor::Pink,
    };
    catalog.save_profile(profile.clone())?;
    let copy = catalog.copy_profile(&profile.id, "Copy settings".into())?;
    assert_eq!(copy.appearance, profile.appearance);
    assert_ne!(
        copy.game_directory(&instance)?,
        profile.game_directory(&instance)?
    );
    assert!(!copy.game_directory(&instance)?.exists());

    let snapshot = Catalog::open(root.path())?.snapshot()?;
    assert_eq!(
        snapshot.selection(&profile.id)?.1.appearance,
        profile.appearance
    );
    assert_eq!(
        snapshot.selection(&copy.id)?.1.appearance,
        profile.appearance
    );
    assert_eq!(fs::read(world.join("level.dat"))?, b"isolated test world");
    Ok(())
}

#[test]
fn unknown_appearance_values_reject_the_catalog_without_resetting_it() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, _) = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    let path = common::path(root.path());
    let original = common::load(root.path())?;

    for appearance in [
        common::table(&[
            ("icon", "../../external.svg".into()),
            ("color", "Gray".into()),
        ]),
        common::table(&[("icon", "Blocks".into()), ("color", "#ffffff".into())]),
        common::table(&[
            ("icon", "Blocks".into()),
            ("color", toml::Value::Integer(42)),
        ]),
        common::table(&[
            ("icon", "Blocks".into()),
            ("color", "Gray".into()),
            ("path", "external.svg".into()),
        ]),
        toml::Value::String("invalid appearance".into()),
    ] {
        let mut document = original.clone();
        common::first_profile_mut(&mut document)?.insert("appearance".into(), appearance);
        let invalid = toml::to_string_pretty(&document)?.into_bytes();
        fs::write(&path, &invalid)?;
        assert!(catalog.snapshot().is_err());
        assert!(
            catalog
                .create_profile(&instance.id, "Another".into())
                .is_err()
        );
        assert_eq!(fs::read(&path)?, invalid);
    }
    Ok(())
}

#[test]
fn configured_profile_commits_launch_fields_and_appearance_together() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, original) = catalog.create_instance(
        "Game".into(),
        "1.20.6".into(),
        None,
        Some("20.6.144".into()),
    )?;
    let configuration = ProfileConfiguration {
        name: "Creative 世界".into(),
        memory_mib: 6144,
        jvm_args: vec!["-Dexample.flag=true".into()],
        game_args: vec!["--server".into(), "test.invalid".into()],
        appearance: ProfileAppearance {
            icon: ProfileIcon::Hammer,
            color: ProfileColor::Blue,
        },
        ..ProfileConfiguration::from(&original)
    };
    let created = catalog.create_profile_configured(&instance.id, configuration.clone())?;
    let snapshot = Catalog::open(root.path())?.snapshot()?;
    assert_eq!(snapshot.profiles.len(), 2);
    let (_, saved, _) = snapshot.selection(&created.id)?;
    assert_eq!(saved.name, configuration.name);
    assert_eq!(saved.memory_mib, configuration.memory_mib);
    assert_eq!(saved.jvm_args, configuration.jvm_args);
    assert_eq!(saved.game_args, configuration.game_args);
    assert_eq!(saved.appearance, configuration.appearance);
    assert_eq!(saved.target_id, "neoforge-20.6.144");
    assert_eq!(saved.data_policy, DataPolicy::Isolated);
    assert!(!saved.game_directory(&instance)?.exists());
    Ok(())
}

#[test]
fn invalid_configured_profile_preserves_metadata_and_creates_no_game_data() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (instance, original) =
        catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    let valid = ProfileConfiguration::from(&original);
    let path = common::path(root.path());
    let bundled = fs::read(&path)?;
    assert!(
        catalog
            .create_profile_configured(&instance.id, valid.clone())
            .is_err()
    );
    assert_eq!(fs::read(&path)?, bundled);

    catalog.separate_profiles()?;
    let saved = fs::read(&path)?;
    let mut invalid = Vec::new();
    let mut configuration = valid.clone();
    configuration.target_id = "missing-target".into();
    invalid.push(configuration);
    let mut configuration = valid.clone();
    configuration.name.clear();
    invalid.push(configuration);
    let mut configuration = valid.clone();
    configuration.memory_mib = 128;
    invalid.push(configuration);
    let mut configuration = valid.clone();
    configuration.jvm_args = vec!["-Xmx32G".into()];
    invalid.push(configuration);
    let mut configuration = valid.clone();
    configuration.game_args = vec!["--accessToken=untrusted".into()];
    invalid.push(configuration);
    let mut configuration = valid.clone();
    configuration.runtime = RuntimeSelection::Local {
        java: "relative/java".into(),
    };
    invalid.push(configuration);
    for configuration in invalid {
        assert!(
            catalog
                .create_profile_configured(&instance.id, configuration)
                .is_err()
        );
        assert_eq!(fs::read(&path)?, saved);
        assert!(!instance.directory.join("profiles").exists());
    }
    assert!(
        catalog
            .create_profile_configured("missing-instance", valid)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, saved);
    assert_eq!(catalog.snapshot()?.profiles.len(), 1);
    Ok(())
}

#[test]
fn configured_instance_saves_its_complete_profile_with_the_correct_mode_name() -> anyhow::Result<()>
{
    for separated in [false, true] {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        if separated {
            catalog.separate_profiles()?;
        }
        let configuration = ProfileConfiguration {
            name: "Independent profile".into(),
            target_id: "neoforge-20.6.144".into(),
            memory_mib: 8192,
            jvm_args: vec!["-Dconfigured=true".into()],
            appearance: ProfileAppearance {
                icon: ProfileIcon::Leaf,
                color: ProfileColor::Green,
            },
            ..ProfileConfiguration::default()
        };
        let (instance, profile) = catalog.create_instance_configured(
            "Configured game".into(),
            "1.20.6".into(),
            None,
            Some("20.6.144".into()),
            configuration.clone(),
        )?;
        assert_eq!(
            profile.name,
            if separated {
                "Independent profile"
            } else {
                "Configured game"
            }
        );
        assert_eq!(profile.memory_mib, configuration.memory_mib);
        assert_eq!(profile.jvm_args, configuration.jvm_args);
        assert_eq!(profile.appearance, configuration.appearance);
        assert_eq!(profile.target_id, "neoforge-20.6.144");
        let snapshot = Catalog::open(root.path())?.snapshot()?;
        assert_eq!(snapshot.instances.len(), 1);
        assert_eq!(snapshot.profiles.len(), 1);
        let (_, saved, _) = snapshot.selection(&profile.id)?;
        assert_eq!(saved.appearance, configuration.appearance);
        assert_eq!(saved.memory_mib, configuration.memory_mib);
        assert!(!saved.game_directory(&instance)?.exists());
    }
    Ok(())
}

#[test]
fn invalid_instance_configuration_creates_neither_metadata_nor_directories() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path().join("catalog"))?;
    let destination = root.path().join("must-not-exist/nested/game");
    for configuration in [
        ProfileConfiguration {
            name: "Game".into(),
            target_id: "missing-target".into(),
            ..ProfileConfiguration::default()
        },
        ProfileConfiguration {
            name: "Game".into(),
            memory_mib: 0,
            ..ProfileConfiguration::default()
        },
        ProfileConfiguration {
            name: "Game".into(),
            jvm_args: vec!["-Xmx8G".into()],
            ..ProfileConfiguration::default()
        },
        ProfileConfiguration {
            name: "Game".into(),
            game_args: vec!["--username=unexpected".into()],
            ..ProfileConfiguration::default()
        },
    ] {
        assert!(
            catalog
                .create_instance_configured(
                    "Game".into(),
                    "1.20.6".into(),
                    Some(destination.clone()),
                    None,
                    configuration,
                )
                .is_err()
        );
        assert!(!root.path().join("must-not-exist").exists());
        assert!(!common::path(catalog.root()).exists());
    }
    Ok(())
}

#[test]
fn failed_instance_commit_rolls_back_only_new_empty_directories() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path().join("catalog"))?;
    let original = b"broken catalog that must remain unchanged";
    fs::write(common::path(catalog.root()), original)?;
    let configuration = ProfileConfiguration {
        name: "Game".into(),
        ..ProfileConfiguration::default()
    };
    assert!(
        catalog
            .create_instance_configured(
                "Game".into(),
                "1.20.6".into(),
                Some(root.path().join("new-tree/nested/game")),
                None,
                configuration.clone(),
            )
            .is_err()
    );
    assert!(!root.path().join("new-tree").exists());
    assert_eq!(fs::read(common::path(catalog.root()))?, original);

    let user_directory = root.path().join("existing-user-data");
    fs::create_dir_all(&user_directory)?;
    fs::write(user_directory.join("sentinel"), b"preserve existing data")?;
    assert!(
        catalog
            .create_instance_configured(
                "Game".into(),
                "1.20.6".into(),
                Some(user_directory.clone()),
                None,
                configuration,
            )
            .is_err()
    );
    assert_eq!(
        fs::read(user_directory.join("sentinel"))?,
        b"preserve existing data"
    );
    assert_eq!(fs::read(common::path(catalog.root()))?, original);
    Ok(())
}
