use block_launcher::catalog::{Catalog, DataPolicy, GameDirectoryGuard};
use std::{fs, sync::Arc};

#[test]
fn explicit_loader_targets_survive_reopen_and_profile_selection() -> anyhow::Result<()> {
    use block_launcher::catalog::{LoaderKind, ProfileConfiguration};
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
        let (instance, profile) = catalog.create_instance_with_loader(
            format!("{prefix} test"),
            minecraft.into(),
            None,
            Some((kind.clone(), version.into())),
            ProfileConfiguration {
                name: "Default".into(),
                target_id: target_id.clone(),
                ..Default::default()
            },
        )?;
        let state = Catalog::open(root.path())?.snapshot()?;
        let (saved_instance, saved_profile, target) = state.selection(&profile.id)?;
        assert_eq!(saved_instance.id, instance.id);
        assert_eq!(saved_instance.targets.len(), 2);
        assert_eq!(saved_profile.target_id, target_id);
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
    use block_launcher::catalog::{LoaderKind, ProfileConfiguration};
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
                    "Invalid".into(),
                    "1.20.6".into(),
                    Some(directory.clone()),
                    Some((kind, version.into())),
                    ProfileConfiguration {
                        name: "Default".into(),
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
fn installations_own_multiple_persistent_profiles() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, first) = catalog.create_instance(
        "Survival".into(),
        "1.20.6".into(),
        None,
        Some("20.6.144".into()),
    )?;
    let mut second = catalog.create_profile(&instance.id, "Creative".into())?;
    second.memory_mib = 4096;
    second.default_account = Some("a-public-player-id".into());
    catalog.save_profile(second.clone())?;
    let state = Catalog::open(root.path())?.snapshot()?;
    assert_eq!(state.instances.len(), 1);
    assert_eq!(state.profiles.len(), 2);
    let (_, saved, target) = state.selection(&second.id)?;
    assert_eq!(saved.memory_mib, 4096);
    assert_eq!(target.id, "vanilla");
    assert_eq!(
        state.selection(&first.id)?.2.loader_version.as_deref(),
        Some("20.6.144")
    );
    Ok(())
}

#[test]
fn isolated_copy_changes_data_path_and_shared_profiles_reuse_one_directory() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, mut original) =
        catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    let copy = catalog.copy_profile(&original.id, "Copy settings".into())?;
    assert_ne!(
        original.game_directory(&instance)?,
        copy.game_directory(&instance)?
    );
    original.data_policy = DataPolicy::Shared;
    catalog.save_profile(original.clone())?;
    let shared = catalog.copy_profile(&original.id, "Shared".into())?;
    assert_eq!(
        original.game_directory(&instance)?,
        shared.game_directory(&instance)?
    );
    Ok(())
}

#[test]
fn deleting_profile_keeps_shared_resources_worlds_and_other_profiles() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, profile) =
        catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    let copy = catalog.copy_profile(&profile.id, "Other".into())?;
    let saves = profile.game_directory(&instance)?.join("saves/World");
    fs::create_dir_all(&saves)?;
    fs::write(saves.join("level.dat"), b"existing world")?;
    let immutable = root.path().join("objects");
    fs::create_dir_all(&immutable)?;
    fs::write(immutable.join("sentinel"), b"cached")?;
    catalog.delete_profile(&profile.id)?;
    assert_eq!(fs::read(saves.join("level.dat"))?, b"existing world");
    assert!(immutable.join("sentinel").exists());
    let state = catalog.snapshot()?;
    assert_eq!(state.instances.len(), 1);
    assert_eq!(state.profiles.len(), 1);
    assert_eq!(state.profiles[0].id, copy.id);
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
    fs::write(root.path().join("catalog.json"), original)?;
    assert!(catalog.snapshot().is_err());
    assert!(
        catalog
            .create_instance("New".into(), "1.20.6".into(), None, None)
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("catalog.json"))?, original);
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
    assert_eq!(state.profiles.len(), 4);
    Ok(())
}

#[test]
fn profiles_cannot_override_account_identity_or_cross_installation_ownership() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (_, mut profile) = catalog.create_instance("A".into(), "1.20.6".into(), None, None)?;
    profile.game_args = vec!["--accessToken=an-untrusted-override".into()];
    assert!(catalog.save_profile(profile.clone()).is_err());
    profile.game_args.clear();
    let (other, _) = catalog.create_instance("B".into(), "1.20.6".into(), None, None)?;
    profile.instance_id = other.id;
    assert!(catalog.save_profile(profile).is_err());
    Ok(())
}

#[test]
fn existing_installation_root_cannot_be_reused_by_another_instance() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path().join("catalog"))?;
    let directory = root.path().join("game");
    catalog.create_instance("A".into(), "1.20.6".into(), Some(directory.clone()), None)?;
    assert!(
        catalog
            .create_instance(
                "B".into(),
                "1.20.6".into(),
                Some(directory.join("profiles")),
                None
            )
            .is_err()
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn isolated_profile_cannot_alias_another_profile_with_a_symlink() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, first) = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    let second = catalog.copy_profile(&first.id, "Other".into())?;
    let original = first.game_directory(&instance)?;
    fs::create_dir_all(&original)?;
    let alias = second.game_directory(&instance)?;
    fs::create_dir_all(
        alias
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing parent"))?,
    )?;
    std::os::unix::fs::symlink(&original, &alias)?;
    assert!(GameDirectoryGuard::for_profile(&instance, &second).is_err());
    assert!(GameDirectoryGuard::for_profile(&instance, &first).is_ok());
    Ok(())
}
