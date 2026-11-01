use std::{fs, io::Write as _, path::Path};

use anyhow::Context as _;
use sha1::Digest as _;
use tokio_util::sync::CancellationToken;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::{extract_overrides, inspect};

fn index(game: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "fixture-1",
        "name": "Synthetic pack",
        "dependencies": {"minecraft": game},
        "files": [{
            "path": "mods/example.jar",
            "hashes": {
                "sha1": format!("{:x}", sha1::Sha1::digest(b"download")),
                "sha512": format!("{:x}", sha2::Sha512::digest(b"download"))
            },
            "fileSize": 8,
            "downloads": ["http://127.0.0.1:1234/example"]
        }]
    }))
    .unwrap_or_default()
}

fn pack(path: &Path, files: &[(&str, &[u8])]) -> anyhow::Result<()> {
    let mut zip = ZipWriter::new(fs::File::create(path)?);
    for (name, bytes) in files {
        zip.start_file(*name, SimpleFileOptions::default())?;
        zip.write_all(bytes)?;
    }
    zip.finish()?;
    Ok(())
}

#[test]
fn prism_archive_rejects_disabled_loader_and_keeps_enabled_loader() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("prism.zip");
    for disabled in [None, Some(false), Some(true)] {
        let mut loader = serde_json::json!({
            "uid": "net.fabricmc.fabric-loader",
            "version": "0.16.10"
        });
        if let Some(disabled) = disabled {
            loader["disabled"] = serde_json::json!(disabled);
        }
        let metadata = serde_json::to_vec(&serde_json::json!({
            "formatVersion": 1,
            "components": [
                {"uid": "net.minecraft", "version": "1.20.1"},
                loader
            ]
        }))?;
        pack(
            &source,
            &[
                ("mmc-pack.json", &metadata),
                ("instance.cfg", b"[General]\nname=Prism fixture\n"),
                ("minecraft/options.txt", b"synthetic options\n"),
            ],
        )?;
        let result = inspect(&source);
        if disabled == Some(true) {
            assert!(result.is_err(), "a disabled loader was silently enabled");
            if let Err(error) = result {
                let message = format!("{error:#}");
                assert!(message.contains("disabled"), "{message}");
                assert!(message.contains("not supported"), "{message}");
            }
        } else {
            let inspection = result?;
            assert_eq!(
                inspection.loader,
                Some((crate::catalog::LoaderKind::Fabric, String::from("0.16.10")))
            );
        }
    }
    Ok(())
}

#[test]
fn overlays_are_ordered_and_scripts_are_copied_without_server_overrides() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("fixture.mrpack");
    let index = index("1.20.1");
    pack(
        &source,
        &[
            ("modrinth.index.json", &index),
            ("overrides/config/example.txt", b"base"),
            ("overrides/kubejs/scripts/example.js", b"not executed"),
            ("client-overrides/config/example.txt", b"client"),
            ("server-overrides/config/server.txt", b"server"),
        ],
    )?;
    let before = fs::read(&source)?;
    let mut inspection = inspect(&source)?;
    assert!(inspection.select_target("1.21.1".into(), None).is_err());
    let target = temp.path().join("overrides");
    extract_overrides(&source, &inspection, &target, &CancellationToken::new())?;
    assert_eq!(fs::read(target.join("config/example.txt"))?, b"client");
    assert_eq!(
        fs::read(target.join("kubejs/scripts/example.js"))?,
        b"not executed"
    );
    assert!(!target.join("config/server.txt").exists());
    assert_eq!(fs::read(&source)?, before);
    Ok(())
}

#[test]
fn rejects_escape_case_conflicts_and_file_directory_conflicts() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("fixture.mrpack");
    let index = index("1.20.1");
    for (name, extra) in [
        ("escape", vec![("../outside", b"escape".as_slice())]),
        (
            "case",
            vec![
                ("overrides/config/a.txt", b"one".as_slice()),
                ("overrides/Config/b.txt", b"two".as_slice()),
            ],
        ),
        (
            "file-directory",
            vec![
                ("overrides/config", b"one".as_slice()),
                ("client-overrides/config/b.txt", b"two".as_slice()),
            ],
        ),
        (
            "download-case",
            vec![("overrides/Mods/example.jar", b"override".as_slice())],
        ),
        ("drive", vec![("overrides/C:/outside", b"x".as_slice())]),
    ] {
        let mut files = vec![("modrinth.index.json", index.as_slice())];
        files.extend(extra);
        pack(&source, &files)?;
        assert!(inspect(&source).is_err(), "{name} must be rejected");
    }
    Ok(())
}

#[test]
fn duplicate_zip_entries_are_rejected() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("fixture.mrpack");
    let index = index("1.20.1");
    pack(
        &source,
        &[
            ("modrinth.index.json", &index),
            ("overrides/a.txt", b"first"),
            ("overrides/b.txt", b"second"),
        ],
    )?;
    let mut bytes = fs::read(&source)?;
    let before = b"overrides/b.txt";
    let after = b"overrides/a.txt";
    for offset in 0..=bytes.len() - before.len() {
        if &bytes[offset..offset + before.len()] == before {
            bytes[offset..offset + after.len()].copy_from_slice(after);
        }
    }
    fs::write(&source, bytes)?;
    assert!(inspect(&source).is_err());
    Ok(())
}

#[test]
fn zip64_archive_with_comment_keeps_all_central_records() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("zip64.mrpack");
    let index = index("1.20.1");
    let mut zip = ZipWriter::new(fs::File::create(&source)?);
    zip.start_file("modrinth.index.json", SimpleFileOptions::default())?;
    zip.write_all(&index)?;
    for number in 0..65_535_u32 {
        zip.start_file(
            format!("unused/{number:05}.txt"),
            SimpleFileOptions::default(),
        )?;
    }
    zip.set_comment("synthetic ZIP64 archive comment");
    zip.finish()?;
    let bytes = fs::read(&source)?;
    assert!(
        bytes.windows(4).any(|window| window == b"PK\x06\x06"),
        "the fixture must contain a ZIP64 end record"
    );
    assert_eq!(inspect(&source)?.format, super::super::PackFormat::Modrinth);
    Ok(())
}

#[test]
fn wrapped_standard_pack_keeps_its_override_root() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("wrapped.zip");
    let index = index("1.20.1");
    pack(
        &source,
        &[
            ("wrapper/modrinth.index.json", &index),
            ("wrapper/overrides/config/pack.txt", b"wrapped"),
        ],
    )?;
    let inspection = inspect(&source)?;
    let target = temp.path().join("result");
    extract_overrides(&source, &inspection, &target, &CancellationToken::new())?;
    assert_eq!(fs::read(target.join("config/pack.txt"))?, b"wrapped");
    Ok(())
}

#[test]
fn broken_or_unsupported_pack_markers_never_fall_back_to_plain() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("fixture.zip");
    for (marker, value) in [
        ("wrapper/modrinth.index.json", b"{}".as_slice()),
        ("wrapper/manifest.json", b"{}".as_slice()),
        ("wrapper/mmc-pack.json", b"{}".as_slice()),
        ("wrapper/instance.cfg", b"name=Example".as_slice()),
        ("wrapper/mcbbs.packmeta", b"{}".as_slice()),
        ("wrapper/modpack.json", b"{}".as_slice()),
    ] {
        pack(
            &source,
            &[(marker, value), ("wrapper/mods/example.jar", b"fixture")],
        )?;
        assert!(inspect(&source).is_err(), "{marker} must not become Plain");
    }
    let missing_index = temp.path().join("missing-index.mrpack");
    pack(&missing_index, &[("mods/example.jar", b"fixture")])?;
    assert!(inspect(&missing_index).is_err());
    pack(
        &source,
        &[
            ("mcbbs.packmeta", b"{}"),
            ("manifest.json", b"{}"),
            ("overrides/config/example.txt", b"fixture"),
        ],
    )?;
    let error = inspect(&source).err().context("MCBBS must be rejected")?;
    assert!(format!("{error:#}").contains("launcher-specific"));
    Ok(())
}

#[test]
fn plain_pack_requires_explicit_target_and_excludes_launcher_state() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("plain.zip");
    pack(
        &source,
        &[
            ("mods/example.jar", b"synthetic mod data"),
            ("config/options.toml", b"config"),
            ("kubejs/scripts/example.js", b"not executed"),
            ("logs/latest.log", b"private log marker"),
            ("launcher_accounts.json", b"excluded account marker"),
            ("launcher_settings.json", b"launcher configuration"),
            ("libraries/source.jar", b"not imported"),
            ("versions/custom/custom.json", b"{}"),
            ("installer.exe", b"not executed"),
        ],
    )?;
    let original = fs::read(&source)?;
    let mut inspection = inspect(&source)?;
    assert_eq!(inspection.format, super::super::PackFormat::Plain);
    assert!(!inspection.target_confirmed());
    assert!(
        super::super::descriptor::from_inspection(
            &inspection,
            &super::super::PackSelection::default()
        )
        .is_err()
    );
    inspection.select_target("1.20.1".into(), None)?;
    assert!(inspection.target_confirmed());
    let target = temp.path().join("content");
    extract_overrides(&source, &inspection, &target, &CancellationToken::new())?;
    assert_eq!(
        fs::read(target.join("mods/example.jar"))?,
        b"synthetic mod data"
    );
    assert_eq!(fs::read(target.join("config/options.toml"))?, b"config");
    assert_eq!(
        fs::read(target.join("kubejs/scripts/example.js"))?,
        b"not executed"
    );
    for path in [
        "logs",
        "launcher_accounts.json",
        "launcher_settings.json",
        "libraries",
        "versions",
        "installer.exe",
    ] {
        assert!(!target.join(path).exists(), "{path} must not be imported");
    }
    assert_eq!(fs::read(&source)?, original);
    Ok(())
}

#[test]
fn rejects_archive_symlinks_and_replaced_manifest() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("fixture.mrpack");
    let first = index("1.20.1");
    pack(&source, &[("modrinth.index.json", &first)])?;
    let inspection = inspect(&source)?;
    let second = index("1.21.1");
    pack(&source, &[("modrinth.index.json", &second)])?;
    let destination = temp.path().join("changed");
    assert!(
        extract_overrides(
            &source,
            &inspection,
            &destination,
            &CancellationToken::new()
        )
        .is_err()
    );
    assert!(!destination.exists());

    pack(
        &source,
        &[
            ("modrinth.index.json", &first),
            ("overrides/config/added.txt", b"changed"),
        ],
    )?;
    assert!(
        extract_overrides(
            &source,
            &inspection,
            &destination,
            &CancellationToken::new()
        )
        .is_err()
    );
    assert!(!destination.exists());

    let mut zip = ZipWriter::new(fs::File::create(&source)?);
    zip.start_file("modrinth.index.json", SimpleFileOptions::default())?;
    zip.write_all(&first)?;
    zip.add_symlink("overrides/link", "../outside", SimpleFileOptions::default())?;
    zip.finish()?;
    assert!(inspect(&source).is_err());
    Ok(())
}

#[test]
fn cancellation_does_not_create_the_staging_directory() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("fixture.mrpack");
    let index = index("1.20.1");
    pack(&source, &[("modrinth.index.json", &index)])?;
    let inspection = inspect(&source)?;
    let target = temp.path().join("cancelled");
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(extract_overrides(&source, &inspection, &target, &cancel).is_err());
    assert!(!target.exists());
    Ok(())
}

#[test]
fn existing_destination_is_never_overwritten() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("fixture.mrpack");
    let index = index("1.20.1");
    pack(
        &source,
        &[
            ("modrinth.index.json", &index),
            ("overrides/options.txt", b"pack"),
        ],
    )?;
    let inspection = inspect(&source)?;
    let target = temp.path().join("existing");
    fs::create_dir(&target)?;
    fs::write(target.join("options.txt"), b"user")?;
    assert!(extract_overrides(&source, &inspection, &target, &CancellationToken::new()).is_err());
    assert_eq!(
        fs::read(target.join("options.txt")).context("existing data")?,
        b"user"
    );
    Ok(())
}
