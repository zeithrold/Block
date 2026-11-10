mod common;

use std::{collections::BTreeMap, fs, path::Path};

use block_launcher::{
    catalog::{
        Catalog, DataPolicy, GameDirectoryGuard, InstallStatus, InstanceConfiguration,
        InstanceOwnership, TargetSource,
    },
    game_import::GameDataEvidence,
};

fn contents(root: &Path) -> anyhow::Result<BTreeMap<std::path::PathBuf, Vec<u8>>> {
    let mut result = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let key = path.strip_prefix(root)?.to_path_buf();
            if path.is_dir() {
                result.insert(key, Vec::new());
                pending.push(path);
            } else {
                result.insert(key, fs::read(path)?);
            }
        }
    }
    Ok(result)
}

#[test]
fn registration_reuses_shared_and_version_locations_without_changing_source_bytes()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("minecraft");
    let first = common::candidate(
        &source,
        "1.20.6",
        GameDataEvidence::Shared {
            directory: source.clone(),
        },
    )?;
    let second = common::candidate(&source, "1.21.1", GameDataEvidence::Unknown)?;
    fs::create_dir_all(source.join("saves/Existing"))?;
    fs::write(source.join("saves/Existing/level.dat"), b"existing world")?;
    fs::write(source.join("options.txt"), b"existing options")?;
    let before = contents(&source)?;
    let catalog = Catalog::open(root.path().join("block"))?;
    let shared = catalog.register_existing(
        &first,
        InstanceConfiguration {
            name: "First".into(),
            data_policy: DataPolicy::Shared,
            game_directory: source.clone(),
            ..Default::default()
        },
    )?;
    let other_shared = catalog.register_existing(
        &second,
        InstanceConfiguration {
            name: "Second shared".into(),
            data_policy: DataPolicy::Shared,
            game_directory: source.clone(),
            ..Default::default()
        },
    )?;
    let isolated = catalog.register_existing(
        &second,
        InstanceConfiguration {
            name: "Second isolated".into(),
            game_directory: source.join("versions/1.21.1"),
            ..Default::default()
        },
    )?;
    assert_eq!(contents(&source)?, before);
    assert_eq!(shared.resource_directory, source.canonicalize()?);
    assert_eq!(shared.game_directory, source.canonicalize()?);
    assert_eq!(shared.ownership, InstanceOwnership::Registered);
    assert_eq!(shared.targets[0].status, InstallStatus::Pending);
    assert_eq!(
        shared.targets[0].source,
        TargetSource::Existing {
            version_id: "1.20.6".into()
        }
    );
    assert_eq!(catalog.snapshot()?.instances.len(), 3);
    let guard = GameDirectoryGuard::for_instance(&shared)?;
    assert!(GameDirectoryGuard::for_instance(&other_shared).is_err());
    let _isolated = GameDirectoryGuard::for_instance(&isolated)?;
    drop(guard);
    assert!(GameDirectoryGuard::for_instance(&other_shared).is_ok());
    Ok(())
}

#[test]
fn registration_preserves_known_external_isolated_directory_and_deletion_preserves_both_roots()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("minecraft");
    let game = root.path().join("custom");
    fs::create_dir(&game)?;
    fs::write(game.join("options.txt"), b"custom data")?;
    let candidate = common::candidate(
        &source,
        "1.20.6",
        GameDataEvidence::Isolated {
            directory: game.clone(),
        },
    )?;
    let before_source = contents(&source)?;
    let before_game = contents(&game)?;
    let catalog = Catalog::open(root.path().join("block"))?;
    let instance = catalog.register_existing(
        &candidate,
        InstanceConfiguration {
            name: "Custom".into(),
            game_directory: game.clone(),
            ..Default::default()
        },
    )?;
    assert_eq!(instance.game_directory, game.canonicalize()?);
    catalog.delete_instance(&instance.id)?;
    assert_eq!(contents(&source)?, before_source);
    assert_eq!(contents(&game)?, before_game);
    Ok(())
}

#[test]
fn registration_rejects_mismatched_policy_invalid_versions_and_failed_commits_without_source_changes()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("minecraft");
    let candidate = common::candidate(&source, "1.20.6", GameDataEvidence::Unknown)?;
    let before = contents(&source)?;
    let catalog = Catalog::open(root.path().join("block"))?;
    let valid = InstanceConfiguration {
        name: "Game".into(),
        game_directory: source.join("versions/1.20.6"),
        ..Default::default()
    };
    let mut mismatch = valid.clone();
    mismatch.game_directory.clone_from(&source);
    assert!(catalog.register_existing(&candidate, mismatch).is_err());
    for version in [None, Some("../outside"), Some("latest-release")] {
        let mut invalid = candidate.clone();
        invalid.version_id = version.map(str::to_owned);
        assert!(catalog.register_existing(&invalid, valid.clone()).is_err());
    }
    let mut unsupported = candidate.clone();
    unsupported.importable = false;
    assert!(
        catalog
            .register_existing(&unsupported, valid.clone())
            .is_err()
    );
    fs::write(common::path(catalog.root()), b"broken catalog")?;
    assert!(catalog.register_existing(&candidate, valid).is_err());
    assert_eq!(contents(&source)?, before);
    assert_eq!(fs::read(common::path(catalog.root()))?, b"broken catalog");
    Ok(())
}

#[cfg(unix)]
#[test]
fn canonical_aliases_register_one_shared_lock_identity() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("minecraft");
    let candidate = common::candidate(&source, "1.20.6", GameDataEvidence::Unknown)?;
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&source, &alias)?;
    let catalog = Catalog::open(root.path().join("block"))?;
    let first = catalog.register_existing(
        &candidate,
        InstanceConfiguration {
            name: "Original".into(),
            data_policy: DataPolicy::Shared,
            game_directory: source.clone(),
            ..Default::default()
        },
    )?;
    let mut candidate = candidate;
    candidate.resource_directory.clone_from(&alias);
    let second = catalog.register_existing(
        &candidate,
        InstanceConfiguration {
            name: "Alias".into(),
            data_policy: DataPolicy::Shared,
            game_directory: alias,
            ..Default::default()
        },
    )?;
    assert_eq!(first.game_directory, second.game_directory);
    let _guard = GameDirectoryGuard::for_instance(&first)?;
    assert!(GameDirectoryGuard::for_instance(&second).is_err());
    Ok(())
}

#[test]
fn collision_choice_registers_a_missing_game_root_without_materializing_it() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("minecraft");
    let shared = source.join("versions/1.20.6");
    let candidate = common::candidate(
        &source,
        "1.20.6",
        GameDataEvidence::Shared {
            directory: shared.clone(),
        },
    )?;
    let selected = shared.join(".minecraft");
    let before = contents(&source)?;
    let catalog = Catalog::open(root.path().join("block"))?;
    let instance = catalog.register_existing(
        &candidate,
        InstanceConfiguration {
            name: "Isolated".into(),
            game_directory: selected.clone(),
            ..Default::default()
        },
    )?;
    assert!(!selected.exists());
    assert_eq!(contents(&source)?, before);
    assert_eq!(
        instance.game_directory,
        shared.canonicalize()?.join(".minecraft")
    );
    fs::write(&selected, b"not a directory")?;
    assert!(GameDirectoryGuard::for_instance(&instance).is_err());
    assert_eq!(fs::read(selected)?, b"not a directory");
    Ok(())
}
