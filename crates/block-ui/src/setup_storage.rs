//! First-run game-library defaults and account-independent storage updates.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// Select the default game-library directory without writing to protected or
/// temporary install locations. Portable installs keep their game data beside
/// the application under `.minecraft`.
pub(super) fn default_game_parent(executable: &Path, user_library: &Path) -> PathBuf {
    let application = application_directory(executable);
    parent_for_location(
        &application,
        user_library,
        classify_install(executable, &application),
    )
}

fn parent_for_location(
    application: &Path,
    user_library: &Path,
    location: InstallLocation,
) -> PathBuf {
    match location {
        InstallLocation::Protected | InstallLocation::Unwritable => user_library.to_owned(),
        InstallLocation::Portable | InstallLocation::Writable => application.join(".minecraft"),
    }
}

/// Return the writable directory beside the application. On macOS this is the
/// parent of the `.app` bundle, so game data never modifies the signed bundle.
pub(super) fn application_directory(executable: &Path) -> PathBuf {
    let Some(parent) = executable.parent() else {
        return executable.to_owned();
    };
    parent
        .ancestors()
        .find(|ancestor| {
            ancestor
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .and_then(Path::parent)
        .map_or_else(|| parent.to_owned(), Path::to_owned)
}

/// Choose a safe parent for an import. If the preferred destination overlaps
/// its detected source, use the stable fallback library instead.
pub(super) fn import_parent(preferred: &Path, source: &Path, fallback: &Path) -> PathBuf {
    if paths_overlap(preferred, source) {
        fallback.to_owned()
    } else {
        preferred.to_owned()
    }
}

/// Persist a library choice while preserving cache and generation settings.
/// Existing game directories are not moved or rewritten.
pub(super) fn save_game_parent(root: &Path, parent: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(parent.is_absolute(), "Choose an absolute game-library path");
    anyhow::ensure!(
        parent.parent().is_some(),
        "Choose a game-library path below a folder"
    );
    fs::create_dir_all(root)?;
    let _storage_lease = block_launcher::storage::acquire(root)?;
    let mut locations = block_launcher::storage::Locations::load(root)?;
    locations.library = Some(parent.to_owned());
    locations = locations.validate(root)?;

    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    use std::io::Write as _;
    temporary.write_all(&serde_json::to_vec_pretty(&locations)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(root.join("storage.json"))?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallLocation {
    Protected,
    Unwritable,
    Portable,
    Writable,
}

fn classify_install(executable: &Path, application: &Path) -> InstallLocation {
    classify_install_state(
        protected_location(application),
        writable_directory(application),
        portable_install(executable, application),
    )
}

fn classify_install_state(protected: bool, writable: bool, portable: bool) -> InstallLocation {
    if protected {
        return InstallLocation::Protected;
    }
    if !writable {
        return InstallLocation::Unwritable;
    }
    if portable {
        InstallLocation::Portable
    } else {
        InstallLocation::Writable
    }
}

fn portable_install(executable: &Path, application: &Path) -> bool {
    executable
        .parent()
        .is_some_and(|parent| parent.join("portable.marker").is_file())
        || application.join("portable.marker").is_file()
}

fn writable_directory(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    tempfile::NamedTempFile::new_in(path).is_ok()
}

fn protected_location(path: &Path) -> bool {
    let path = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let path = path.trim_end_matches('/');
    let unix_roots = ["/applications", "/usr", "/opt", "/snap"];
    if unix_roots
        .iter()
        .any(|root| path == *root || path.starts_with(&format!("{root}/")))
    {
        return true;
    }
    if path == "/tmp"
        || path.starts_with("/tmp/")
        || path == "/var/tmp"
        || path.starts_with("/var/tmp/")
        || path.contains("/var/folders/") && path.contains("/t/")
    {
        return true;
    }
    path.split('/').any(|part| {
        matches!(
            part,
            "applications"
                | "downloads"
                | "download"
                | "temp"
                | "tmp"
                | "windows"
                | "winnt"
                | "system32"
                | "program files"
                | "program files (x86)"
        )
    })
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left = comparable_path(left);
    let right = comparable_path(right);
    left.starts_with(&right) || right.starts_with(&left)
}

fn comparable_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            break;
        };
        missing.push(name.to_owned());
        let Some(parent) = ancestor.parent() else {
            break;
        };
        ancestor = parent;
    }
    let mut result = ancestor
        .canonicalize()
        .unwrap_or_else(|_| ancestor.to_owned());
    for name in missing.into_iter().rev() {
        result.push(name);
    }
    lexical_normalize(result)
}

fn lexical_normalize(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_directory_uses_the_directory_beside_the_macos_bundle() {
        assert_eq!(
            application_directory(Path::new("/Applications/Block.app/Contents/MacOS/Block")),
            PathBuf::from("/Applications")
        );
        assert_eq!(
            application_directory(Path::new("/opt/block/bin/block")),
            PathBuf::from("/opt/block/bin")
        );
    }

    #[test]
    fn protected_and_user_selected_locations_are_classified() {
        for path in [
            "/Applications/Block.app",
            "/usr/local/bin",
            "/opt/block",
            "/snap/block/current",
            "/home/user/Downloads/Block",
            "/tmp/block",
            "C:/Program Files/Block",
            "C:/Windows/System32/Block",
        ] {
            assert!(protected_location(Path::new(path)), "{path}");
        }
        assert!(!protected_location(Path::new("/home/user/Apps/Block")));
    }

    #[test]
    fn default_parent_uses_user_library_for_protected_paths_and_adjacent_minecraft_for_portable() {
        let application = Path::new("/Applications");
        let user_library = Path::new("/Users/user/Library/Application Support/Block/instances");
        assert_eq!(
            parent_for_location(application, user_library, InstallLocation::Protected),
            user_library
        );
        let portable = Path::new("/Users/user/Apps");
        assert_eq!(
            parent_for_location(portable, user_library, InstallLocation::Portable),
            portable.join(".minecraft")
        );
        assert_eq!(
            parent_for_location(portable, user_library, InstallLocation::Writable),
            portable.join(".minecraft")
        );
    }

    #[test]
    fn portable_marker_is_recognized_at_application_or_executable_level() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let app = temp.path().join("Block.app");
        let executable_dir = app.join("Contents/MacOS");
        fs::create_dir_all(&executable_dir)?;
        fs::write(temp.path().join("portable.marker"), "")?;
        fs::remove_file(temp.path().join("portable.marker"))?;
        fs::write(executable_dir.join("portable.marker"), "")?;
        assert!(portable_install(
            &executable_dir.join("Block"),
            &application_directory(&executable_dir.join("Block"))
        ));
        assert_eq!(
            classify_install_state(false, true, true),
            InstallLocation::Portable
        );
        assert_eq!(
            classify_install_state(false, true, false),
            InstallLocation::Writable
        );
        assert_eq!(
            classify_install_state(false, false, true),
            InstallLocation::Unwritable
        );
        assert_eq!(
            classify_install_state(true, true, true),
            InstallLocation::Protected
        );
        Ok(())
    }

    #[test]
    fn import_parent_uses_fallback_when_destination_overlaps_source() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join(".minecraft"))?;
        let fallback = temp.path().join("Block/library");
        assert_eq!(
            import_parent(&source, &source.join(".minecraft"), &fallback),
            fallback
        );
        let preferred = temp.path().join("portable/.minecraft");
        assert_eq!(import_parent(&preferred, &source, &fallback), preferred);
        Ok(())
    }

    #[test]
    fn save_updates_library_without_changing_cache_generation_or_game_files() -> anyhow::Result<()>
    {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("config");
        fs::create_dir_all(&root)?;
        let game_parent = temp.path().join("games");
        fs::create_dir_all(&game_parent)?;
        let old_game = game_parent.join("existing");
        fs::create_dir_all(&old_game)?;
        fs::write(old_game.join("keep.dat"), b"original")?;
        let cache = temp.path().join("cache");
        let generation = uuid::Uuid::new_v4();
        let before = block_launcher::storage::Locations {
            library: None,
            cache: Some(cache.clone()),
            generation: Some(generation),
        };
        fs::write(
            root.join("storage.json"),
            serde_json::to_vec_pretty(&before)?,
        )?;

        save_game_parent(&root, &game_parent)?;
        let saved = block_launcher::storage::Locations::load(&root)?;
        assert_eq!(
            saved.library.as_deref(),
            Some(comparable_path(&game_parent).as_path())
        );
        assert_eq!(saved.cache, Some(comparable_path(&cache)));
        assert_eq!(saved.generation, Some(generation));
        assert_eq!(fs::read(old_game.join("keep.dat"))?, b"original");
        Ok(())
    }
}
