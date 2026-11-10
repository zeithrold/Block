use super::super::{DataPolicy, DiscoveryRoots, GameDataEvidence, LoaderKind, discover};
use super::write_json;
use std::fs;

#[test]
fn adjacent_version_directory_with_mutable_data_does_not_prove_isolation() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join(".minecraft");
    let version_directory = root.join("versions/custom-version");
    write_json(
        &version_directory.join("custom-version.json"),
        r#"{"id":"custom-version","inheritsFrom":"1.20.1","libraries":[{"name":"net.minecraftforge:forge:1.20.1-47.2.0"}],"mainClass":"cpw.mods.modlauncher.Launcher"}"#,
    )?;
    fs::create_dir_all(version_directory.join("saves/World"))?;
    fs::write(
        version_directory.join("saves/World/level.dat"),
        b"isolated world",
    )?;
    fs::write(version_directory.join("options.txt"), b"isolated options")?;
    write_json(
        &root.join("versions/1.20.1/1.20.1.json"),
        r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: Some(root.clone()),
        official_launcher: None,
    });
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.configured_version.as_deref() == Some("custom-version"))
        .ok_or_else(|| anyhow::anyhow!("isolated version was not discovered"))?;
    assert_eq!(candidate.data, GameDataEvidence::Unknown);
    assert_eq!(
        candidate.data_directory(DataPolicy::Isolated)?,
        version_directory.canonicalize()?
    );
    assert_eq!(candidate.minecraft_version.as_deref(), Some("1.20.1"));
    assert_eq!(
        candidate.loader.as_ref().map(|loader| loader.kind),
        Some(LoaderKind::Forge)
    );
    assert_eq!(
        candidate
            .loader
            .as_ref()
            .and_then(|loader| loader.version.as_deref()),
        Some("1.20.1-47.2.0")
    );
    Ok(())
}

#[test]
fn custom_game_directory_loader_comes_from_local_library_metadata() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let launcher = temp.path().join("launcher");
    let custom_game = temp.path().join("custom-game");
    fs::create_dir_all(&custom_game)?;
    write_json(
        &launcher.join("launcher_profiles.json"),
        &format!(
            r#"{{"profiles":{{"custom":{{"name":"Custom","gameDir":{:?},"lastVersionId":"private-profile-name"}}}}}}"#,
            custom_game.to_string_lossy()
        ),
    )?;
    write_json(
        &launcher.join("versions/private-profile-name/private-profile-name.json"),
        r#"{"id":"private-profile-name","inheritsFrom":"1.21.1","libraries":[{"name":"org.quiltmc:quilt-loader:0.27.1"}],"mainClass":"org.quiltmc.loader.impl.launch.knot.KnotClient"}"#,
    )?;
    write_json(
        &launcher.join("versions/1.21.1/1.21.1.json"),
        r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: None,
        official_launcher: Some(launcher),
    });
    let candidate = report
        .candidates
        .first()
        .ok_or_else(|| anyhow::anyhow!("custom profile was not found"))?;
    assert!(candidate.importable);
    assert_eq!(candidate.minecraft_version.as_deref(), Some("1.21.1"));
    let loader = candidate
        .loader
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Quilt loader was not detected"))?;
    assert_eq!(loader.kind, LoaderKind::Quilt);
    assert_eq!(loader.version.as_deref(), Some("0.27.1"));
    Ok(())
}

#[test]
fn unknown_custom_main_class_is_unsupported_instead_of_vanilla() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let launcher = temp.path().join("launcher");
    let game = temp.path().join("game");
    fs::create_dir_all(&game)?;
    write_json(
        &launcher.join("launcher_profiles.json"),
        &format!(
            r#"{{"profiles":{{"custom":{{"name":"Unknown","gameDir":{:?},"lastVersionId":"custom-client"}}}}}}"#,
            game.to_string_lossy()
        ),
    )?;
    write_json(
        &launcher.join("versions/custom-client/custom-client.json"),
        r#"{"id":"custom-client","inheritsFrom":"1.21.1","mainClass":"com.example.private.CustomClient"}"#,
    )?;
    write_json(
        &launcher.join("versions/1.21.1/1.21.1.json"),
        r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    let report = discover(&DiscoveryRoots {
        adjacent_minecraft: None,
        official_launcher: Some(launcher),
    });
    let candidate = report
        .candidates
        .first()
        .ok_or_else(|| anyhow::anyhow!("profile was not found"))?;
    assert!(!candidate.importable);
    assert!(
        candidate
            .unsupported_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("unknown"))
    );
    Ok(())
}
