use super::super::{DataPolicy, DiscoveryRoots, LoaderKind, SourceKind, discover};
use super::write_json;
use std::fs;

#[test]
fn discovers_adjacent_and_launcher_profiles_and_resolves_inheritance() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let adjacent = temp.path().join(".minecraft");
    fs::create_dir_all(adjacent.join("saves/World"))?;
    fs::write(adjacent.join("saves/World/level.dat"), b"fixture-world")?;
    write_json(
        &adjacent.join("versions/1.19.4/1.19.4.json"),
        r#"{"id":"1.19.4","type":"release","releaseTime":"2022-12-07T00:00:00+00:00","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    write_json(
        &adjacent.join("versions/fabric-loader-0.15.11-1.20.1/fabric-loader-0.15.11-1.20.1.json"),
        r#"{"id":"fabric-loader-0.15.11-1.20.1","type":"release","releaseTime":"2023-06-07T00:00:00+00:00","inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.15.11"}],"mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient"}"#,
    )?;
    write_json(
        &adjacent.join("versions/1.20.1/1.20.1.json"),
        r#"{"id":"1.20.1","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    let launcher = temp.path().join("launcher");
    let profile_game = temp.path().join("profile-game");
    fs::create_dir_all(&profile_game)?;
    write_json(
        &launcher.join("launcher_profiles.json"),
        &format!(
            r#"{{"profiles":{{"fab":{{"name":"Modded","gameDir":{:?},"lastVersionId":"custom-fabric-profile"}}}}}}"#,
            profile_game.to_string_lossy()
        ),
    )?;
    write_json(
        &launcher.join("versions/custom-fabric-profile/custom-fabric-profile.json"),
        r#"{"id":"custom-fabric-profile","inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.5"}],"mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient"}"#,
    )?;
    write_json(
        &launcher.join("versions/1.20.1/1.20.1.json"),
        r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    write_json(
        &launcher.join("version_manifest_v2.json"),
        r#"{"latest":{"release":"1.21.1","snapshot":"25w01a"}}"#,
    )?;

    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(adjacent.clone()),
        official_launcher: Some(launcher),
    });
    assert_eq!(report.sources.len(), 2);
    assert_eq!(report.candidates.len(), 4);
    assert_eq!(report.sources[0].candidates_found, 3);
    let adjacent_candidates = report
        .candidates
        .iter()
        .filter(|candidate| candidate.source == SourceKind::AdjacentMinecraft)
        .collect::<Vec<_>>();
    assert_eq!(adjacent_candidates.len(), 3);
    assert!(
        adjacent_candidates
            .iter()
            .all(|candidate| candidate.importable)
    );
    assert!(adjacent_candidates.iter().any(|candidate| {
        candidate.minecraft_version.as_deref() == Some("1.19.4")
            && candidate
                .loader
                .as_ref()
                .is_some_and(|loader| loader.kind == LoaderKind::Vanilla)
    }));
    assert!(adjacent_candidates.iter().any(|candidate| {
        candidate.minecraft_version.as_deref() == Some("1.20.1")
            && candidate
                .loader
                .as_ref()
                .is_some_and(|loader| loader.kind == LoaderKind::Fabric)
    }));
    let modded = report
        .candidates
        .iter()
        .find(|candidate| candidate.name == "Modded")
        .ok_or_else(|| anyhow::anyhow!("custom gameDir profile was not discovered"))?;
    assert_eq!(modded.name, "Modded");
    assert_eq!(modded.minecraft_version.as_deref(), Some("1.20.1"));
    assert_eq!(
        modded.data_directory(DataPolicy::Isolated)?,
        profile_game.canonicalize()?
    );
    assert!(modded.importable);
    assert_eq!(
        modded
            .loader
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Fabric loader was not detected"))?
            .kind,
        LoaderKind::Fabric
    );
    assert_eq!(
        modded
            .loader
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Fabric loader was not detected"))?
            .version
            .as_deref(),
        Some("0.16.5")
    );
    assert_eq!(modded.latest_release.as_deref(), Some("1.21.1"));
    assert_eq!(modded.latest_snapshot.as_deref(), Some("25w01a"));
    Ok(())
}

#[test]
fn adjacent_empty_directory_has_no_game_candidates() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join(".minecraft");
    fs::create_dir(&root)?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root),
        official_launcher: None,
    });
    assert!(report.candidates.is_empty());
    assert_eq!(report.sources[0].candidates_found, 0);
    Ok(())
}

#[test]
fn adjacent_empty_profile_object_falls_back_to_local_versions() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join(".minecraft");
    write_json(&root.join("launcher_profiles.json"), r#"{"profiles":{}}"#)?;
    write_json(
        &root.join("versions/1.21.1/1.21.1.json"),
        r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root),
        official_launcher: None,
    });
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(
        report.candidates[0].configured_version.as_deref(),
        Some("1.21.1")
    );
    assert!(report.candidates[0].importable);
    Ok(())
}

#[test]
fn distinct_launcher_profiles_sharing_version_and_game_directory_remain_distinct()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("launcher");
    let game = temp.path().join("shared-game");
    fs::create_dir_all(&game)?;
    let profiles = format!(
        r#"{{"profiles":{{"work":{{"name":"Work","gameDir":{:?},"lastVersionId":"1.21.1"}},"play":{{"name":"Play","gameDir":{:?},"lastVersionId":"1.21.1"}}}}}}"#,
        game.to_string_lossy(),
        game.to_string_lossy()
    );
    write_json(&root.join("launcher_profiles.json"), &profiles)?;
    write_json(
        &root.join("launcher_profiles_microsoft_store.json"),
        &format!(
            r#"{{"profiles":{{"play":{{"name":"Duplicate","gameDir":{:?},"lastVersionId":"1.21.1"}}}}}}"#,
            game.to_string_lossy()
        ),
    )?;
    write_json(
        &root.join("versions/1.21.1/1.21.1.json"),
        r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: None,
        official_launcher: Some(root),
    });
    assert_eq!(report.candidates.len(), 2);
    assert_ne!(report.candidates[0].id, report.candidates[1].id);
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.importable)
    );
    Ok(())
}
