mod common;
use block_launcher::{
    catalog::{
        Catalog, DataPolicy, InstanceAppearance, InstanceColor, InstanceConfiguration, InstanceIcon,
    },
    recipe::RuntimeSelection,
};
use std::fs;

#[test]
fn appearance_survives_save_and_reopen_without_touching_worlds() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    assert_eq!(instance.appearance, InstanceAppearance::default());
    let world = instance.game_directory.join("saves/Test world");
    fs::create_dir_all(&world)?;
    fs::write(world.join("level.dat"), b"isolated test world")?;
    let mut configuration = InstanceConfiguration::from(&instance);
    configuration.appearance = InstanceAppearance {
        icon: InstanceIcon::FlaskConical,
        color: InstanceColor::Pink,
    };
    catalog.save_instance(&instance.id, configuration.clone())?;
    let snapshot = Catalog::open(root.path())?.snapshot()?;
    assert_eq!(
        snapshot.selection(&instance.id)?.0.appearance,
        configuration.appearance
    );
    assert_eq!(fs::read(world.join("level.dat"))?, b"isolated test world");
    Ok(())
}

#[test]
fn unknown_appearance_values_reject_the_catalog_without_resetting_it() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
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
        common::first_instance_mut(&mut document)?.insert("appearance".into(), appearance);
        let invalid = toml::to_string_pretty(&document)?.into_bytes();
        fs::write(&path, &invalid)?;
        assert!(catalog.snapshot().is_err());
        assert!(
            catalog
                .save_instance(&instance.id, InstanceConfiguration::from(&instance))
                .is_err()
        );
        assert_eq!(fs::read(&path)?, invalid);
    }
    Ok(())
}

#[test]
fn configured_instance_commits_all_editable_fields_together() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let configuration = InstanceConfiguration {
        name: "Configured game".into(),
        target_id: "neoforge-20.6.144".into(),
        memory_mib: 8192,
        jvm_args: vec!["-Dconfigured=true".into()],
        game_args: vec!["--server".into(), "test.invalid".into()],
        default_account: Some("public-player-id".into()),
        width: 1440,
        height: 900,
        fullscreen: true,
        appearance: InstanceAppearance {
            icon: InstanceIcon::Leaf,
            color: InstanceColor::Green,
        },
        ..Default::default()
    };
    let instance = catalog.create_instance_configured(
        "1.20.6".into(),
        None,
        Some("20.6.144".into()),
        configuration.clone(),
    )?;
    let state = Catalog::open(root.path())?.snapshot()?;
    let (saved, _) = state.selection(&instance.id)?;
    assert_eq!(state.instances.len(), 1);
    assert_eq!(saved.name, configuration.name);
    assert_eq!(saved.memory_mib, 8192);
    assert_eq!(saved.jvm_args, configuration.jvm_args);
    assert_eq!(saved.game_args, configuration.game_args);
    assert_eq!(saved.default_account, configuration.default_account);
    assert_eq!(saved.appearance, configuration.appearance);
    assert_eq!(
        (saved.width, saved.height, saved.fullscreen),
        (1440, 900, true)
    );
    assert_eq!(saved.target_id, "neoforge-20.6.144");
    assert_eq!(saved.data_policy, DataPolicy::Isolated);
    Ok(())
}

#[test]
fn invalid_configuration_preserves_metadata_and_creates_no_game_data() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    let valid = InstanceConfiguration::from(&instance);
    let before = fs::read(common::path(root.path()))?;
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
        assert!(catalog.save_instance(&instance.id, configuration).is_err());
        assert_eq!(fs::read(common::path(root.path()))?, before);
        assert!(!instance.game_directory.join("profiles").exists());
    }
    assert!(catalog.save_instance("missing-instance", valid).is_err());
    assert_eq!(fs::read(common::path(root.path()))?, before);
    Ok(())
}

#[test]
fn invalid_instance_configuration_creates_neither_metadata_nor_directories() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path().join("catalog"))?;
    let destination = root.path().join("must-not-exist/nested/game");
    for configuration in [
        InstanceConfiguration {
            name: "Game".into(),
            target_id: "missing-target".into(),
            ..InstanceConfiguration::default()
        },
        InstanceConfiguration {
            name: "Game".into(),
            memory_mib: 0,
            ..InstanceConfiguration::default()
        },
        InstanceConfiguration {
            name: "Game".into(),
            jvm_args: vec!["-Xmx8G".into()],
            ..InstanceConfiguration::default()
        },
        InstanceConfiguration {
            name: "Game".into(),
            game_args: vec!["--username=unexpected".into()],
            ..InstanceConfiguration::default()
        },
    ] {
        assert!(
            catalog
                .create_instance_configured(
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
    let configuration = InstanceConfiguration {
        name: "Game".into(),
        ..InstanceConfiguration::default()
    };
    assert!(
        catalog
            .create_instance_configured(
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
