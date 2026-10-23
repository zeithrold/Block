use std::fs;

use block_launcher::catalog::{Catalog, DataPolicy, GameDirectoryGuard};

#[test]
fn reservation_survives_dropped_guard_and_blocks_shared_profiles_and_aliases() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    catalog.separate_profiles()?;
    let (instance, mut profile) =
        catalog.create_instance("Test".into(), "1.20.6".into(), None, None)?;
    profile.data_policy = DataPolicy::Shared;
    catalog.save_profile(profile.clone())?;
    let shared = catalog.copy_profile(&profile.id, "Other shared profile".into())?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let guard = GameDirectoryGuard::for_session(&instance, &profile, &run_id)?;
    let game = guard.directory().to_path_buf();
    let bytes = fs::read(game.join(".block-session.json"))?;
    let marker: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(marker["schema_version"], 1);
    assert_eq!(marker["run_id"], run_id);
    assert_eq!(marker["supervisor_pid"], std::process::id());
    drop(guard);

    assert!(GameDirectoryGuard::for_profile(&instance, &shared).is_err());
    assert!(GameDirectoryGuard::for_session(&instance, &profile, &run_id).is_err());
    assert!(GameDirectoryGuard::acquire(&game.join(".")).is_err());
    assert_eq!(fs::read(game.join(".block-session.json"))?, bytes);
    GameDirectoryGuard::clear_session_reservation(&game, &run_id)?;
    let next =
        GameDirectoryGuard::for_session(&instance, &shared, &uuid::Uuid::new_v4().to_string())?;
    assert_eq!(next.directory(), game);
    Ok(())
}

#[test]
fn only_the_exact_canonical_run_id_can_release_a_reservation() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (instance, profile) =
        catalog.create_instance("Test".into(), "1.20.6".into(), None, None)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let guard = GameDirectoryGuard::for_session(&instance, &profile, &run_id)?;
    let marker = guard.directory().join(".block-session.json");
    let bytes = fs::read(&marker)?;
    assert!(
        GameDirectoryGuard::clear_session_reservation(
            guard.directory(),
            &uuid::Uuid::new_v4().to_string()
        )
        .is_err()
    );
    assert!(
        GameDirectoryGuard::clear_session_reservation(guard.directory(), &run_id.to_uppercase())
            .is_err()
    );
    assert_eq!(fs::read(&marker)?, bytes);
    GameDirectoryGuard::clear_session_reservation(guard.directory(), &run_id)?;
    assert!(!marker.exists());
    assert!(GameDirectoryGuard::for_profile(&instance, &profile).is_err());
    drop(guard);
    assert!(GameDirectoryGuard::for_profile(&instance, &profile).is_ok());
    Ok(())
}

#[test]
fn invalid_run_id_is_rejected_without_creating_a_marker() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (instance, profile) =
        catalog.create_instance("Test".into(), "1.20.6".into(), None, None)?;
    assert!(GameDirectoryGuard::for_session(&instance, &profile, "not-a-uuid").is_err());
    assert!(
        !profile
            .game_directory(&instance)?
            .join(".block-session.json")
            .exists()
    );
    assert!(GameDirectoryGuard::for_profile(&instance, &profile).is_ok());
    Ok(())
}

#[test]
fn malformed_or_oversized_marker_is_preserved_and_blocks_games() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let game = root.path().canonicalize()?;
    let marker = game.join(".block-session.json");
    let run_id = uuid::Uuid::new_v4().to_string();
    for bytes in [
        b"broken metadata".to_vec(),
        vec![b'x'; 4096],
        serde_json::to_vec(&serde_json::json!({
            "schema_version":2, "run_id":run_id, "supervisor_pid":1
        }))?,
    ] {
        fs::write(&marker, &bytes)?;
        assert!(GameDirectoryGuard::acquire(&game).is_err());
        assert!(GameDirectoryGuard::clear_session_reservation(&game, &run_id).is_err());
        assert_eq!(fs::read(&marker)?, bytes);
    }
    Ok(())
}

#[test]
fn symlinked_marker_is_never_removed() -> anyhow::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt as _, symlink as symlink_file};
    #[cfg(windows)]
    use std::os::windows::fs::symlink_file;
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (instance, profile) =
        catalog.create_instance("Test".into(), "1.20.6".into(), None, None)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let guard = GameDirectoryGuard::for_session(&instance, &profile, &run_id)?;
    let game = guard.directory().to_path_buf();
    let marker = game.join(".block-session.json");
    #[cfg(unix)]
    assert_eq!(fs::metadata(&marker)?.permissions().mode() & 0o777, 0o600);
    let outside = root.path().join("outside-session.json");
    fs::rename(&marker, &outside)?;
    symlink_file(&outside, &marker)?;
    drop(guard);
    assert!(GameDirectoryGuard::acquire(&game).is_err());
    assert!(GameDirectoryGuard::clear_session_reservation(&game, &run_id).is_err());
    assert!(fs::symlink_metadata(&marker)?.file_type().is_symlink());
    assert!(outside.exists());
    Ok(())
}
