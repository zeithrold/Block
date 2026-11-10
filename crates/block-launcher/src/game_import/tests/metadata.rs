use super::super::classification::classify_version;
use super::super::discovery::make_candidate;
use super::super::metadata::{latest_version, read_json_bounded};
use super::super::{LoaderKind, SourceKind, looks_like_minecraft_version};
use super::write_json;
use std::fs;
const MAX_VERSION_BYTES: u64 = 8 * 1024 * 1024;

#[test]
fn concrete_versions_include_year_releases_without_accepting_custom_components() {
    for version in [
        "26.3",
        "26.4-snapshot-4",
        "1.0",
        "1.20.1",
        "1.21.4-pre2",
        "1.21.4-rc1",
        "24w14a",
        "b1.7.3",
        "a1.2.6",
    ] {
        assert!(looks_like_minecraft_version(version), "{version}");
    }
    for version in [
        "latest-release",
        "latest-snapshot",
        "../1.20.1",
        "1.20.1-optifine",
        "private-profile-name",
    ] {
        assert!(!looks_like_minecraft_version(version), "{version}");
    }
}

#[test]
fn corrupt_latest_alias_falls_back_to_concrete_local_metadata_without_recursing()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("minecraft");
    write_json(
        &root.join("version_manifest_v2.json"),
        r#"{"latest":{"release":"latest-release","snapshot":"latest-release"}}"#,
    )?;
    write_json(
        &root.join("versions/26.3/26.3.json"),
        r#"{"id":"26.3","type":"release","releaseTime":"2026-10-01T00:00:00Z","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    let candidate = make_candidate(
        SourceKind::OfficialLauncher,
        &root,
        "Current release",
        Some("latest-release".into()),
    );
    assert!(candidate.importable);
    assert_eq!(candidate.minecraft_version.as_deref(), Some("26.3"));
    let empty = temp.path().join("empty");
    write_json(
        &empty.join("version_manifest_v2.json"),
        r#"{"latest":{"release":"latest-release"}}"#,
    )?;
    let unresolved = make_candidate(
        SourceKind::OfficialLauncher,
        &empty,
        "Unresolved",
        Some("latest-release".into()),
    );
    assert!(!unresolved.importable);
    Ok(())
}

#[test]
fn local_release_alias_uses_latest_cached_version_metadata() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join(".minecraft");
    write_json(
        &root.join("versions/1.20.1/1.20.1.json"),
        r#"{"id":"1.20.1","type":"release","releaseTime":"2023-06-07T00:00:00+00:00","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    write_json(
        &root.join("versions/1.21.1/1.21.1.json"),
        r#"{"id":"1.21.1","type":"release","releaseTime":"2024-06-13T00:00:00+00:00","mainClass":"net.minecraft.client.main.Main"}"#,
    )?;
    fs::create_dir_all(&root)?;
    assert_eq!(latest_version(&root, "release").as_deref(), Some("1.21.1"));
    let candidate = make_candidate(
        SourceKind::OfficialLauncher,
        &root,
        "Latest release",
        Some("latest-release".into()),
    );
    assert!(candidate.importable);
    assert_eq!(candidate.minecraft_version.as_deref(), Some("1.21.1"));
    assert_eq!(
        candidate.loader.as_ref().map(|loader| loader.kind),
        Some(LoaderKind::Vanilla)
    );
    Ok(())
}

#[test]
fn detects_loader_ids_but_missing_metadata_is_not_importable() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let game = temp.path().join("game");
    fs::create_dir_all(&game)?;
    let candidate = make_candidate(
        SourceKind::AdjacentMinecraft,
        &game,
        "game",
        Some("1.20.1-neoforge-20.4.237".into()),
    );
    assert!(!candidate.importable);
    assert_eq!(
        candidate
            .loader
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("NeoForge loader was not detected"))?
            .kind,
        LoaderKind::NeoForge
    );
    assert_eq!(candidate.minecraft_version.as_deref(), Some("1.20.1"));
    assert!(candidate.metadata_warning.is_some());
    assert_eq!(
        classify_version("forge-1.12.2-14.23.5.2860").0.kind,
        LoaderKind::Forge
    );
    assert_eq!(
        classify_version("forge-1.12.2-14.23.5.2860").1.as_deref(),
        Some("1.12.2")
    );
    assert_eq!(
        classify_version("quilt-loader-0.26.4-1.21").0.kind,
        LoaderKind::Quilt
    );
    assert_eq!(classify_version("1.21.1").0.kind, LoaderKind::Vanilla);
    Ok(())
}

#[cfg(unix)]
#[test]
fn bounded_json_reader_rejects_metadata_symlinks() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let target = temp.path().join("real.json");
    let link = temp.path().join("linked.json");
    fs::write(&target, r#"{"id":"1.21.1"}"#)?;
    symlink(&target, &link)?;
    assert!(read_json_bounded(&target, MAX_VERSION_BYTES).is_ok());
    assert!(read_json_bounded(&link, MAX_VERSION_BYTES).is_err());
    Ok(())
}
