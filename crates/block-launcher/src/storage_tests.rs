use super::*;
use crate::{
    catalog::{Catalog, InstallStatus},
    library::Library,
};

fn put(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    fs::create_dir_all(path.parent().context("Fixture parent")?)?;
    fs::write(path, bytes)?;
    Ok(())
}
fn requested(library: Option<PathBuf>, cache: Option<PathBuf>) -> Locations {
    Locations {
        library,
        cache,
        ..Default::default()
    }
}

fn finish_retained(root: &Path, remove: bool) -> anyhow::Result<()> {
    let state = cleanup_status(root)?.context("Retained copy")?;
    finish_cleanup(root, state.id, remove)
}

#[test]
fn changing_default_library_preserves_existing_games_and_only_affects_new_instances()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    let catalog = Catalog::open(&root)?;
    let (old, _) = catalog.create_instance("Old".into(), "1.20.6".into(), None, None)?;
    put(
        &old.directory.join("profiles/synthetic-data.txt"),
        b"existing synthetic game data",
    )?;
    let target = temp_path.join("new-games");
    assert!(
        apply(
            &root,
            requested(Some(target.clone()), None),
            false,
            &CancellationToken::new(),
            &|_| {}
        )?
        .is_none()
    );
    let (new, _) = catalog.create_instance("New".into(), "1.20.6".into(), None, None)?;
    assert_eq!(old.directory, catalog.snapshot()?.instances[0].directory);
    assert!(new.directory.starts_with(target));
    assert!(old.directory.join("profiles/synthetic-data.txt").is_file());
    Ok(())
}

#[test]
fn cache_move_rebases_installed_targets_retains_accounts_and_requires_explicit_cleanup()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    let catalog = Catalog::open(&root)?;
    let (instance, _) = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    let root = catalog.root().to_owned();
    let new = temp_path.join("cache");
    put(&root.join("objects/ab/payload"), b"verified cache fixture")?;
    put(&root.join("loaders/forge/test/manifest.json"), b"{}")?;
    put(
        &root.join("loaders/forge/test/installed.json"),
        &serde_json::to_vec(
            &serde_json::json!({"manifest_path":root.join("loaders/forge/test/manifest.json")}),
        )?,
    )?;
    put(
        &root.join("accounts.json"),
        b"public synthetic account metadata",
    )?;
    let mut target = instance.targets[0].clone();
    target.manifest = Some(root.join("loaders/forge/test/manifest.json"));
    target.status = InstallStatus::Ready;
    catalog.set_target(&instance.id, &target)?;
    let mut phases = std::collections::BTreeSet::new();
    let events = std::sync::Mutex::new(Vec::new());
    let cleanup = apply(
        &root,
        requested(None, Some(new.clone())),
        false,
        &CancellationToken::new(),
        &|progress| {
            if let Ok(mut list) = events.lock() {
                list.push(progress.phase);
            }
        },
    )?
    .context("Retained previous copy")?;
    assert!(!cleanup.interrupted);
    for event in events
        .into_inner()
        .map_err(|_| anyhow::anyhow!("Event lock"))?
    {
        phases.insert(event);
    }
    assert!(phases.contains("Copying") && phases.contains("Verifying"));
    let state = catalog.snapshot()?;
    assert_eq!(
        state.instances[0].targets[0].manifest,
        Some(new.join("loaders/forge/test/manifest.json"))
    );
    let descriptor: serde_json::Value =
        serde_json::from_slice(&fs::read(new.join("loaders/forge/test/installed.json"))?)?;
    assert_eq!(
        descriptor["manifest_path"],
        serde_json::to_value(new.join("loaders/forge/test/manifest.json"))?
    );
    assert!(root.join("objects/ab/payload").exists());
    assert!(!new.join("accounts.json").exists());
    assert_eq!(
        fs::read(root.join("accounts.json"))?,
        b"public synthetic account metadata"
    );
    assert!(finish_cleanup(&root, uuid::Uuid::new_v4(), true).is_err());
    assert!(root.join("objects/ab/payload").exists());
    finish_retained(&root, true)?;
    assert!(!root.join("objects").exists());
    assert!(new.join("objects/ab/payload").exists());
    assert!(instance.directory.exists());
    assert!(root.join("catalog.json").is_file());
    Ok(())
}

#[test]
fn game_move_preserves_custom_locations_and_keeps_preparation_files_separate() -> anyhow::Result<()>
{
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    let catalog = Catalog::open(&root)?;
    let (managed, profile) =
        catalog.create_instance("Managed".into(), "1.20.6".into(), None, None)?;
    let custom = temp_path.join("custom-game");
    let (custom_instance, _) =
        catalog.create_instance("Custom".into(), "1.20.6".into(), Some(custom.clone()), None)?;
    put(
        &managed.directory.join("profiles/synthetic-save.dat"),
        b"synthetic save fixture",
    )?;
    put(
        &managed.directory.join("prepared.lock.json"),
        b"cache preparation fixture",
    )?;
    let target = temp_path.join("moved-games");
    apply(
        &root,
        requested(Some(target.clone()), None),
        true,
        &CancellationToken::new(),
        &|_| {},
    )?;
    let state = catalog.snapshot()?;
    let updated = state
        .instances
        .iter()
        .find(|item| item.id == managed.id)
        .context("Managed instance")?;
    assert_eq!(updated.directory, target.join(&managed.id));
    assert_eq!(
        state
            .instances
            .iter()
            .find(|item| item.id == custom_instance.id)
            .context("Custom instance")?
            .directory,
        custom.canonicalize()?
    );
    assert!(
        updated
            .directory
            .join("profiles/synthetic-save.dat")
            .is_file()
    );
    assert!(!updated.directory.join("prepared.lock.json").exists());
    assert_eq!(state.profiles[0].id, profile.id);
    finish_retained(&root, true)?;
    assert!(!managed.directory.join("profiles").exists());
    assert!(managed.directory.join("prepared.lock.json").exists());
    assert!(custom.exists());
    Ok(())
}

#[test]
fn cancellation_rolls_back_without_removing_sources_and_retains_only_an_explicit_cleanup_copy()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    fs::create_dir(&root)?;
    put(&root.join("objects/data"), &vec![1; 128 * 1024])?;
    let destination = temp_path.join("cache");
    let cancel = CancellationToken::new();
    let result = apply(
        &root,
        requested(None, Some(destination.clone())),
        false,
        &cancel,
        &|progress| {
            if progress.phase == "Copying" {
                cancel.cancel();
            }
        },
    );
    assert!(result.is_err());
    assert!(root.join("objects/data").is_file());
    assert_eq!(Locations::load(&root)?.cache, None);
    assert!(cleanup_status(&root)?.is_some_and(|state| state.interrupted));
    finish_retained(&root, true)?;
    assert!(root.join("objects/data").exists());
    assert!(!destination.join("objects").exists());
    Ok(())
}

#[test]
fn live_library_leases_block_relocation_and_old_copy_edits_block_deletion() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    fs::create_dir(&root)?;
    put(&root.join("objects/data"), b"original")?;
    let destination = temp_path.join("cache");
    let library = Library::new(root.clone(), Arc::new(|_| {}), Arc::new(|_| {}))?;
    assert!(
        apply(
            &root,
            requested(None, Some(destination.clone())),
            false,
            &CancellationToken::new(),
            &|_| {}
        )
        .is_err()
    );
    drop(library);
    apply(
        &root,
        requested(None, Some(destination.clone())),
        false,
        &CancellationToken::new(),
        &|_| {},
    )?;
    put(&root.join("objects/data"), b"later edit")?;
    assert!(finish_retained(&root, true).is_err());
    assert_eq!(fs::read(root.join("objects/data"))?, b"later edit");
    assert_eq!(fs::read(destination.join("objects/data"))?, b"original");
    finish_retained(&root, false)?;
    assert!(root.join("objects/data").exists());
    assert!(cleanup_status(&root)?.is_none());
    Ok(())
}

#[test]
fn recovery_distinguishes_a_committed_generation_from_identical_folder_settings()
-> anyhow::Result<()> {
    for switched in [false, true] {
        let temp = tempfile::tempdir()?;
        let temp_path = temp.path().canonicalize()?;
        let root = temp_path.join("control");
        fs::create_dir(&root)?;
        let root = root.canonicalize()?;
        let source = temp_path.join("old");
        let destination = temp_path.join("new");
        put(&source.join("data"), b"synthetic")?;
        let id = uuid::Uuid::new_v4();
        let before = Locations::default();
        let after = Locations {
            generation: Some(id),
            ..before.clone()
        };
        let tree = Tree {
            source,
            destination,
            skip_runtime: false,
        };
        let mut done = 0;
        copy_tree(
            &tree,
            std::slice::from_ref(&tree),
            &CancellationToken::new(),
            9,
            &mut done,
            &|_| {},
        )?;
        let state = Journal {
            id,
            before: before.clone(),
            after: after.clone(),
            source_digest: tree_digest(std::slice::from_ref(&tree), &CancellationToken::new())?,
            trees: vec![tree],
            phase: Phase::Published,
            cleanup_started: false,
        };
        save(&root, &state, JOURNAL)?;
        if switched {
            save(&root, &after, "storage.json")?;
        }
        let cleanup = recover(&root)?.context("Cleanup state")?;
        assert_eq!(cleanup.interrupted, !switched);
        assert_eq!(
            Locations::load(&root)?,
            if switched { after } else { before }
        );
        finish_retained(&root, false)?;
    }
    Ok(())
}

#[test]
fn metadata_rewrite_failure_and_interrupted_cleanup_preserve_a_recoverable_state()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    let catalog = Catalog::open(&root)?;
    let (instance, _) = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;
    put(&root.join("objects/data"), b"cache")?;
    let mut target = instance.targets[0].clone();
    target.manifest = Some(root.canonicalize()?.join("objects/data"));
    target.status = InstallStatus::Ready;
    catalog.set_target(&instance.id, &target)?;
    put(&root.join("preferences.json"), b"invalid configuration")?;
    let original = fs::read(root.join("catalog.json"))?;
    assert!(
        apply(
            &root,
            requested(None, Some(temp_path.join("new-cache"))),
            false,
            &CancellationToken::new(),
            &|_| {}
        )
        .is_err()
    );
    assert_eq!(fs::read(root.join("catalog.json"))?, original);
    assert_eq!(
        catalog.snapshot()?.instances[0].directory,
        instance.directory
    );
    assert!(recover(&root)?.is_some_and(|state| state.interrupted));
    finish_retained(&root, true)?;
    assert!(root.join("objects/data").is_file());
    Ok(())
}

#[test]
fn path_overlap_occupied_targets_and_space_shortfalls_fail_before_copying() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    fs::create_dir(&root)?;
    put(&root.join("objects/data"), b"cache")?;
    assert!(
        apply(
            &root,
            requested(Some(root.join("objects")), None),
            false,
            &CancellationToken::new(),
            &|_| {}
        )
        .is_err()
    );
    assert!(
        apply(
            &root,
            requested(None, Some(root.join("objects/nested"))),
            false,
            &CancellationToken::new(),
            &|_| {}
        )
        .is_err()
    );
    let target = temp_path.join("occupied");
    put(&target.join("personal.txt"), b"retain")?;
    assert!(
        apply(
            &root,
            requested(None, Some(target.clone())),
            false,
            &CancellationToken::new(),
            &|_| {}
        )
        .is_err()
    );
    assert_eq!(fs::read(target.join("personal.txt"))?, b"retain");
    assert!(check_capacity(100, 99).is_err());
    assert!(check_capacity(100, 100 + 16 * 1024 * 1024).is_ok());
    assert!(!root.join(JOURNAL).exists());
    Ok(())
}

#[test]
fn cache_can_return_to_the_managed_root_without_touching_games_or_control_files()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let parent = temp.path().canonicalize()?;
    let root = parent.join("control");
    fs::create_dir(&root)?;
    put(&root.join("objects/data"), b"reusable object")?;
    put(
        &root.join("instances/not-a-cache/world.dat"),
        b"synthetic world",
    )?;
    let other = parent.join("other-cache");
    apply(
        &root,
        requested(None, Some(other.clone())),
        false,
        &CancellationToken::new(),
        &|_| {},
    )?;
    finish_retained(&root, true)?;
    apply(
        &root,
        Locations::default(),
        false,
        &CancellationToken::new(),
        &|_| {},
    )?;
    finish_retained(&root, true)?;
    assert!(Locations::load(&root)?.cache.is_none());
    assert_eq!(fs::read(root.join("objects/data"))?, b"reusable object");
    assert_eq!(
        fs::read(root.join("instances/not-a-cache/world.dat"))?,
        b"synthetic world"
    );
    assert!(root.join("storage.json").is_file());
    assert!(!other.join("objects").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn internal_and_external_links_are_preserved_without_copying_external_data() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let temp_path = temp.path().canonicalize()?;
    let root = temp_path.join("control");
    fs::create_dir(&root)?;
    let root = root.canonicalize()?;
    put(&root.join("runtimes/bin/java"), b"runtime")?;
    let external = temp_path.join("external");
    put(&external, b"not copied")?;
    std::os::unix::fs::symlink("java", root.join("runtimes/bin/internal"))?;
    std::os::unix::fs::symlink(&external, root.join("runtimes/bin/external"))?;
    std::os::unix::fs::symlink(
        "../../../external",
        root.join("runtimes/bin/relative-external"),
    )?;
    let destination = temp_path.join("nested/cache");
    apply(
        &root,
        requested(None, Some(destination.clone())),
        false,
        &CancellationToken::new(),
        &|_| {},
    )?;
    assert_eq!(
        fs::read_link(destination.join("runtimes/bin/internal"))?,
        PathBuf::from("java")
    );
    assert_eq!(
        fs::read_link(destination.join("runtimes/bin/external"))?,
        external
    );
    assert_eq!(
        fs::read_link(destination.join("runtimes/bin/relative-external"))?,
        external
    );
    finish_retained(&root, true)?;
    assert_eq!(fs::read(external)?, b"not copied");
    assert!(destination.join("runtimes/bin/internal").is_file());
    Ok(())
}
