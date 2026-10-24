mod common;

use std::{fs, sync::Arc};

use block_launcher::catalog::{Catalog, ProfileMode, Snapshot};

#[test]
fn new_catalog_bundles_and_rejects_shared_profiles_without_writing() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    assert_eq!(Snapshot::default().profile_mode, ProfileMode::Bundled);
    assert_eq!(catalog.snapshot()?.profile_mode, ProfileMode::Bundled);
    let (instance, profile) =
        catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    assert_eq!(profile.name, instance.name);
    let path = common::path(root.path());
    let original = fs::read(&path)?;
    assert!(
        catalog
            .create_profile(&instance.id, "Extra".into())
            .is_err()
    );
    assert!(catalog.copy_profile(&profile.id, "Copy".into()).is_err());
    assert_eq!(fs::read(path)?, original);
    let reopened = Catalog::open(root.path())?.snapshot()?;
    assert_eq!(reopened.profile_mode, ProfileMode::Bundled);
    assert_eq!(reopened.profiles.len(), 1);
    Ok(())
}

#[test]
fn conversion_is_durable_one_way_and_preserves_worlds_and_settings() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (instance, mut profile) =
        catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    profile.memory_mib = 4096;
    catalog.save_profile(profile.clone())?;
    let world = profile
        .game_directory(&instance)?
        .join("saves/World/level.dat");
    fs::create_dir_all(
        world
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing parent"))?,
    )?;
    fs::write(&world, b"existing world")?;
    let before = serde_json::to_value(catalog.snapshot()?)?;
    catalog.separate_profiles()?;
    let catalog = Catalog::open(root.path())?;
    let after = serde_json::to_value(catalog.snapshot()?)?;
    assert_eq!(after["profile_mode"], "separated");
    assert_eq!(after["instances"], before["instances"]);
    assert_eq!(after["profiles"], before["profiles"]);
    assert_eq!(fs::read(&world)?, b"existing world");
    let converted = fs::read(common::path(root.path()))?;
    assert!(catalog.separate_profiles().is_err());
    assert_eq!(fs::read(common::path(root.path()))?, converted);
    catalog.create_profile(&instance.id, "Extra".into())?;
    catalog.copy_profile(&profile.id, "Copy".into())?;
    assert_eq!(catalog.snapshot()?.profiles.len(), 3);
    let (_, separated_profile) =
        catalog.create_instance("Separated game".into(), "1.20.6".into(), None, None)?;
    assert_eq!(separated_profile.name, "Default");
    Ok(())
}

#[test]
fn legacy_catalogs_keep_separated_semantics_without_read_side_migration() -> anyhow::Result<()> {
    for shared in [false, true] {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (instance, _) =
            catalog.create_instance("Legacy".into(), "1.20.6".into(), None, None)?;
        catalog.separate_profiles()?;
        if shared {
            catalog.create_profile(&instance.id, "Existing extra".into())?;
        }
        let path = common::path(root.path());
        let mut document = common::load(root.path())?;
        common::snapshot_mut(&mut document)?.remove("profile_mode");
        let original = toml::to_string_pretty(&document)?.into_bytes();
        fs::write(&path, &original)?;
        let catalog = Catalog::open(root.path())?;
        assert_eq!(catalog.snapshot()?.profile_mode, ProfileMode::Separated);
        assert_eq!(fs::read(&path)?, original);
        catalog.create_profile(&instance.id, "New extra".into())?;
        assert_eq!(
            catalog.snapshot()?.profiles.len(),
            if shared { 3 } else { 2 }
        );
        let saved = common::load(root.path())?;
        assert_eq!(
            saved["catalog"]["snapshot"]["profile_mode"].as_str(),
            Some("separated")
        );
    }
    Ok(())
}

#[test]
fn bundled_delete_removes_only_game_metadata_and_preserves_all_files() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (instance, profile) =
        catalog.create_instance("Delete".into(), "1.20.6".into(), None, None)?;
    let world = profile
        .game_directory(&instance)?
        .join("saves/World/level.dat");
    fs::create_dir_all(
        world
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing parent"))?,
    )?;
    fs::write(&world, b"preserve world")?;
    let cache = root.path().join("objects/sentinel");
    fs::create_dir_all(
        cache
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing parent"))?,
    )?;
    fs::write(&cache, b"preserve cached resources")?;
    catalog.create_instance("Keep".into(), "1.20.6".into(), None, None)?;
    catalog.delete_profile(&profile.id)?;
    let state = Catalog::open(root.path())?.snapshot()?;
    assert_eq!(state.profile_mode, ProfileMode::Bundled);
    assert_eq!(state.instances.len(), 1);
    assert_eq!(state.profiles.len(), 1);
    assert_eq!(state.instances[0].name, "Keep");
    assert_eq!(fs::read(world)?, b"preserve world");
    assert_eq!(fs::read(cache)?, b"preserve cached resources");
    Ok(())
}

#[test]
fn explicit_bundled_catalog_rejects_shared_or_missing_profiles_without_resetting()
-> anyhow::Result<()> {
    for orphan in [false, true] {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (instance, _) = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
        catalog.separate_profiles()?;
        catalog.create_profile(&instance.id, "Extra".into())?;
        let path = common::path(root.path());
        let mut document = common::load(root.path())?;
        {
            let snapshot = common::snapshot_mut(&mut document)?;
            snapshot.insert("profile_mode".into(), toml::Value::String("bundled".into()));
            if orphan {
                snapshot.insert("profiles".into(), toml::Value::Array(Vec::new()));
            }
        }
        let original = toml::to_string_pretty(&document)?.into_bytes();
        fs::write(&path, &original)?;
        assert!(catalog.snapshot().is_err());
        assert!(catalog.separate_profiles().is_err());
        assert_eq!(fs::read(path)?, original);
    }
    Ok(())
}

#[test]
fn concurrent_conversion_commits_once() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Arc::new(Catalog::open(root.path())?);
    let mut workers = Vec::new();
    for _ in 0..4 {
        let catalog = Arc::clone(&catalog);
        workers.push(std::thread::spawn(move || {
            catalog.separate_profiles().is_ok()
        }));
    }
    let mut successes = 0;
    for worker in workers {
        if worker
            .join()
            .map_err(|_| anyhow::anyhow!("worker panicked"))?
        {
            successes += 1;
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(
        Catalog::open(root.path())?.snapshot()?.profile_mode,
        ProfileMode::Separated
    );
    Ok(())
}
