use std::{fs, path::Path};

use serde_json::json;

use super::{DataPolicy, DiscoveryRoots, GameDataEvidence, discover};
use anyhow::Context as _;

fn source(root: &Path, profiles: serde_json::Value) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("launcher_profiles.json"),
        serde_json::to_vec(&json!({"profiles": profiles}))?,
    )?;
    for id in ["1.20.1", "1.21.1"] {
        let directory = root.join("versions").join(id);
        fs::create_dir_all(&directory)?;
        fs::write(
            directory.join(format!("{id}.json")),
            serde_json::to_vec(&json!({"id":id, "mainClass":"net.minecraft.client.main.Main"}))?,
        )?;
    }
    Ok(())
}

#[test]
fn explicit_paths_classify_root_custom_and_shared_custom_by_actual_identity() -> anyhow::Result<()>
{
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?.join("source");
    let custom = temp.path().canonicalize()?.join("custom");
    let isolated = temp.path().canonicalize()?.join("isolated");
    fs::create_dir(&custom)?;
    fs::create_dir(&isolated)?;
    source(
        &root,
        json!({
            "root":{"gameDir":".","lastVersionId":"1.20.1"},
            "one":{"gameDir":custom,"lastVersionId":"1.20.1"},
            "two":{"gameDir":custom,"lastVersionId":"1.21.1"},
            "isolated":{"gameDir":isolated,"lastVersionId":"1.21.1"}
        }),
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root.clone()),
        official_launcher: None,
    });
    let candidate = |name: &str| {
        report
            .candidates
            .iter()
            .find(|candidate| candidate.name == name)
            .context("candidate missing")
    };
    assert_eq!(
        candidate("root")?.data,
        GameDataEvidence::Shared {
            directory: root.clone()
        }
    );
    assert_eq!(
        candidate("one")?.data,
        GameDataEvidence::Shared {
            directory: custom.clone()
        }
    );
    assert_eq!(
        candidate("two")?.data,
        GameDataEvidence::Shared { directory: custom }
    );
    assert_eq!(
        candidate("isolated")?.data,
        GameDataEvidence::Isolated {
            directory: isolated.clone()
        }
    );
    assert_eq!(
        candidate("isolated")?.data_directory(DataPolicy::Isolated)?,
        isolated
    );
    assert_eq!(
        candidate("isolated")?.data_directory(DataPolicy::Shared)?,
        root
    );
    Ok(())
}

#[test]
fn duplicate_scan_and_mutable_folder_clues_do_not_promote_evidence() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?.join("source");
    let custom = temp.path().canonicalize()?.join("custom");
    fs::create_dir(&custom)?;
    source(
        &root,
        json!({"one":{"gameDir":custom,"lastVersionId":"1.20.1"}}),
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root.clone()),
        official_launcher: Some(root.clone()),
    });
    assert_eq!(report.candidates.len(), 2);
    assert!(report.candidates.iter().all(|candidate| candidate.data
        == GameDataEvidence::Isolated {
            directory: custom.clone()
        }));
    fs::remove_file(root.join("launcher_profiles.json"))?;
    fs::create_dir_all(root.join("versions/1.20.1/saves/World"))?;
    fs::create_dir_all(root.join("mods"))?;
    let folders = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root),
        official_launcher: None,
    });
    assert!(
        folders
            .candidates
            .iter()
            .all(|candidate| candidate.data == GameDataEvidence::Unknown)
    );
    Ok(())
}

#[test]
fn missing_empty_and_wrong_type_game_dirs_keep_unknown_without_source_mutation()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?.join("source");
    source(
        &root,
        json!({
            "missing":{"lastVersionId":"1.20.1"},
            "empty":{"lastVersionId":"1.20.1","gameDir":""},
            "wrong":{"lastVersionId":"1.20.1","gameDir":12},
            "absent":{"lastVersionId":"1.20.1","gameDir":"does-not-exist"}
        }),
    )?;
    let before = fs::read(root.join("launcher_profiles.json"))?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root.clone()),
        official_launcher: None,
    });
    assert_eq!(report.candidates.len(), 4);
    assert!(
        report
            .candidates
            .iter()
            .filter(|candidate| candidate.name != "absent")
            .all(|candidate| candidate.data == GameDataEvidence::Unknown
                && candidate.metadata_warning.is_some())
    );
    let planned = report
        .candidates
        .iter()
        .find(|candidate| candidate.name == "absent")
        .context("planned directory candidate missing")?;
    assert_eq!(
        planned.data,
        GameDataEvidence::Isolated {
            directory: root.join("does-not-exist")
        }
    );
    assert_eq!(
        planned.data_directory(DataPolicy::Isolated)?,
        root.join("does-not-exist")
    );
    assert_eq!(fs::read(root.join("launcher_profiles.json"))?, before);
    assert!(!root.join("does-not-exist").exists());
    Ok(())
}

#[test]
fn shared_collision_selects_missing_version_local_minecraft_without_creating_it()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?.join("source");
    let shared = root.join("versions/1.20.1");
    source(
        &root,
        json!({
            "one":{"lastVersionId":"1.20.1","gameDir":shared},
            "two":{"lastVersionId":"1.21.1","gameDir":shared}
        }),
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root.clone()),
        official_launcher: None,
    });
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.name == "one")
        .context("missing candidate")?;
    assert_eq!(candidate.data_directory(DataPolicy::Shared)?, shared);
    assert_eq!(
        candidate.data_directory(DataPolicy::Isolated)?,
        shared.join(".minecraft")
    );
    assert!(!shared.join(".minecraft").exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&shared, shared.join(".minecraft"))?;
        assert!(candidate.data_directory(DataPolicy::Isolated).is_err());
    }
    Ok(())
}

#[test]
fn malformed_and_conflicting_configuration_retains_warning_and_unknown_location()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?.join("source");
    source(
        &root,
        json!({"one":{"lastVersionId":"1.20.1","gameDir":"."}}),
    )?;
    fs::write(
        root.join("launcher_profiles_microsoft_store.json"),
        serde_json::to_vec(&json!({"profiles":{"one":{"lastVersionId":"1.21.1","gameDir":"."}}}))?,
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root.clone()),
        official_launcher: None,
    });
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(report.candidates[0].data, GameDataEvidence::Unknown);
    assert!(
        report.candidates[0]
            .metadata_warning
            .as_ref()
            .is_some_and(|warning| warning.contains("Conflicting"))
    );
    fs::remove_file(root.join("launcher_profiles_microsoft_store.json"))?;
    fs::write(root.join("launcher_profiles.json"), b"invalid JSON")?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root),
        official_launcher: None,
    });
    assert_eq!(report.candidates.len(), 2);
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.data == GameDataEvidence::Unknown
                && candidate.metadata_warning.is_some())
    );
    Ok(())
}
