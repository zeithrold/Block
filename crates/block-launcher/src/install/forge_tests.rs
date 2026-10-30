use std::{path::Path, sync::Arc};

use anyhow::Context as _;
use serde_json::json;
use sha1::{Digest as _, Sha1};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::{Store, manifest::model::VersionManifest, resolver::ResolveContext};

fn context(path: &Path, offline: bool, retries: u32) -> anyhow::Result<ResolveContext> {
    Ok(ResolveContext {
        store: Store::with_options(
            path.into(),
            offline,
            Arc::new(|_| {}),
            crate::DownloadOptions {
                proxy: crate::ProxyMode::Direct,
                max_retries: retries,
                ..Default::default()
            },
        )?,
        cancel: crate::CancellationToken::new(),
    })
}
fn xml(versions: &[&str]) -> Vec<u8> {
    format!("<metadata><groupId>net.minecraftforge</groupId><artifactId>forge</artifactId><versioning><versions>{}</versions></versioning></metadata>", versions.iter().map(|v|format!("<version>{v}</version>")).collect::<String>()).into_bytes()
}
async fn read_request(stream: &mut tokio::net::TcpStream) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut buffer = [0; 1024];
        let n = stream.read(&mut buffer).await?;
        anyhow::ensure!(n > 0 && bytes.len() + n <= 8192, "Invalid request");
        bytes.extend_from_slice(&buffer[..n]);
    }
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .into())
}
async fn fixture(
    responses: Vec<(u16, &'static str, Vec<u8>)>,
) -> anyhow::Result<(String, tokio::task::JoinHandle<anyhow::Result<Vec<String>>>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}/", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, headers, body) in responses {
            let (mut stream, _) = listener.accept().await?;
            let request = read_request(&mut stream).await?;
            let head = request.starts_with("HEAD ");
            requests.push(request);
            stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n", body.len()).as_bytes()).await?;
            if !head {
                stream.write_all(&body).await?;
            }
        }
        Ok(requests)
    });
    Ok((origin, task))
}

#[test]
fn version_coordinates_preserve_legacy_branch_suffixes_and_bind_exact_game() {
    for (game, version) in [
        ("1.8.9", "1.8.9-11.15.1.2318-1.8.9"),
        ("1.7.2", "1.7.2-10.12.0.1024-mc172"),
        ("1.12.2", "1.12.2-14.23.5.2860"),
        ("26.3", "26.3-66.1.0"),
    ] {
        assert!(super::validate_version(game, version).is_ok());
        assert!(super::validate_version("different", version).is_err());
    }
    for version in [
        "1.8.9-../../evil",
        "1.8.9-0",
        "1.8.90-11.15.1.1",
        "1.8.9-1.two.3",
    ] {
        assert!(super::validate_version("1.8.9", version).is_err());
    }
}

#[tokio::test]
async fn catalog_filters_sorts_deduplicates_and_replays_offline_without_changing_coordinates()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let (origin, task) = fixture(vec![(
        200,
        "",
        xml(&[
            "1.8.9-11.15.1.9",
            "1.8.9-11.15.1.2318-1.8.9",
            "1.8.9-11.15.1.9",
            "1.12.2-14.23.5.2860",
            "1.8.90-11.15.1.9999",
        ]),
    )])
    .await?;
    let builds = super::versions_from("1.8.9", &context(root.path(), false, 0)?, &origin).await?;
    assert_eq!(
        builds
            .iter()
            .map(|v| v.version.as_str())
            .collect::<Vec<_>>(),
        ["1.8.9-11.15.1.2318-1.8.9", "1.8.9-11.15.1.9"]
    );
    assert!(builds.iter().all(|v| v.stable));
    assert!(task.await??[0].starts_with("GET /net/minecraftforge/forge/maven-metadata.xml"));
    let cached = super::versions_from("1.8.9", &context(root.path(), true, 0)?, &origin).await?;
    assert_eq!(cached, builds);
    assert!(
        super::versions_from(
            "1.8.9",
            &context(root.path(), true, 0)?,
            "http://127.0.0.1:1/"
        )
        .await
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn catalog_rejects_invalid_identity_xml_and_http_errors() -> anyhow::Result<()> {
    for (status, body) in [
        (200, xml(&[])),
        (404, b"missing".to_vec()),
        (200, b"<!DOCTYPE x><metadata/>".to_vec()),
        (200, b"<broken".to_vec()),
    ] {
        let body = if status == 200 && body.starts_with(b"<metadata>") {
            String::from_utf8(body)?
                .replace("net.minecraftforge", "foreign.group")
                .into_bytes()
        } else {
            body
        };
        let root = tempfile::tempdir()?;
        let (origin, task) = fixture(vec![(status, "", body)]).await?;
        assert!(
            super::versions_from("1.20.1", &context(root.path(), false, 0)?, &origin)
                .await
                .is_err()
        );
        task.await??;
    }
    Ok(())
}

#[tokio::test]
async fn catalog_retries_throttling_and_cancels_long_backoff() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let (origin, task) = fixture(vec![
        (429, "Retry-After: 0\r\n", vec![]),
        (200, "", xml(&["1.20.1-47.4.13"])),
    ])
    .await?;
    assert_eq!(
        super::versions_from("1.20.1", &context(root.path(), false, 1)?, &origin)
            .await?
            .len(),
        1
    );
    assert_eq!(task.await??.len(), 2);
    let (origin, task) = fixture(vec![(429, "Retry-After: 3600\r\n", vec![])]).await?;
    let context = context(root.path(), false, 1)?;
    let cancel = context.cancel.clone();
    let pending =
        tokio::spawn(async move { super::versions_from("1.20.1", &context, &origin).await });
    task.await??;
    cancel.cancel();
    assert!(
        pending
            .await?
            .err()
            .context("Expected cancel")?
            .to_string()
            .contains("cancelled")
    );
    Ok(())
}

#[tokio::test]
async fn catalog_deadline_and_cancel_interrupt_stalled_requests() -> anyhow::Result<()> {
    for timeout in [false, true] {
        let root = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}/", listener.local_addr()?);
        let context = context(root.path(), false, 0)?;
        let cancel = context.cancel.clone();
        let pending =
            tokio::spawn(async move { super::versions_from("1.20.1", &context, &origin).await });
        let (mut stream, _) = listener.accept().await?;
        read_request(&mut stream).await?;
        if timeout {
            tokio::time::pause();
            tokio::time::advance(std::time::Duration::from_secs(46)).await;
        } else {
            cancel.cancel();
        }
        let error = pending.await?.err().context("Expected bounded failure")?;
        assert!(
            error
                .to_string()
                .contains(if timeout { "timed out" } else { "cancelled" })
        );
        if timeout {
            tokio::time::resume();
        }
    }
    Ok(())
}

fn installer(version: &str, game: &str, old: bool, corrupt: bool) -> anyhow::Result<Vec<u8>> {
    use std::io::Write as _;
    let coordinate = format!("net.minecraftforge:forge:{version}");
    let library_path = super::super::neoforge::coordinate_path(&coordinate)?;
    let jar = b"synthetic verified universal fixture";
    let id = format!(
        "{game}-forge-{}",
        version
            .strip_prefix(&format!("{game}-"))
            .context("version")?
    );
    let overlay = json!({"id":id,"inheritsFrom":game,"mainClass":"net.minecraft.launchwrapper.Launch", "minecraftArguments":"--username ${auth_player_name} --tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker", "libraries": if old {json!([{"name":coordinate,"url":"https://maven.minecraftforge.net/"}])} else {json!([{"name":coordinate,"downloads":{"artifact":{"path":library_path,"url":"","sha1":if corrupt {"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()} else {format!("{:x}", Sha1::digest(jar))},"size":jar.len()}}}])}});
    let profile = if old {
        json!({"install":{"minecraft":game,"path":coordinate,"target":id,"filePath":"universal.jar"}, "versionInfo":overlay})
    } else {
        json!({"minecraft":game,"path":format!("{coordinate}:shim"),"version":id,"libraries":overlay["libraries"],"data":{},"processors":[]})
    };
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("install_profile.json", options)?;
    writer.write_all(&serde_json::to_vec(&profile)?)?;
    if !old {
        writer.start_file("version.json", options)?;
        writer.write_all(&serde_json::to_vec(&overlay)?)?;
    }
    writer.start_file(
        if old {
            "universal.jar".into()
        } else {
            format!("maven/{library_path}")
        },
        options,
    )?;
    writer.write_all(jar)?;
    Ok(writer.finish()?.into_inner())
}

#[tokio::test]
async fn legacy_v1_and_backported_v2_install_without_running_java_and_repair_corrupt_cache()
-> anyhow::Result<()> {
    for (old, corrupt) in [(false, false), (true, false), (false, true)] {
        let game = if old { "1.8.9" } else { "1.12.2" };
        let version = if old {
            "1.8.9-11.15.1.2318-1.8.9"
        } else {
            "1.12.2-14.23.5.2860"
        };
        let bytes = installer(version, game, old, corrupt)?;
        let root = tempfile::tempdir()?;
        let context = context(root.path(), false, 0)?;
        let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/manifests/official/versions");
        let path = std::fs::read_dir(fixture_root)?
            .next()
            .context("manifest fixture")??
            .path();
        let mut base: VersionManifest = serde_json::from_slice(&std::fs::read(path)?)?;
        base.id = game.into();
        let client = b"base client fixture";
        base.downloads.client.sha1 = format!("{:x}", Sha1::digest(client));
        base.downloads.client.size = client.len() as u64;
        let client_path = context.store.object_path(&base.downloads.client.sha1)?;
        std::fs::create_dir_all(client_path.parent().context("cache directory")?)?;
        std::fs::write(client_path, client)?;
        let (origin, task) = fixture(vec![
            (200, "", xml(&[version])),
            (200, "", format!("{:x}", Sha1::digest(&bytes)).into_bytes()),
            (200, "", vec![0; bytes.len()]),
            (200, "", bytes),
        ])
        .await?;
        let result = super::super::neoforge::install_from_kind(
            &base,
            version,
            Path::new("nonexistent-java"),
            &context,
            &origin,
            true,
        )
        .await;
        assert_eq!(task.await??.len(), 4);
        if corrupt {
            assert!(
                format!("{:#}", result.err().context("Expected checksum failure")?)
                    .contains("checksum")
            );
            assert!(
                !root
                    .path()
                    .join("loaders/forge")
                    .join(version)
                    .join("installed.json")
                    .exists()
            );
            continue;
        }
        let installed = result?;
        let manifest: VersionManifest =
            serde_json::from_slice(&std::fs::read(&installed.manifest_path)?)?;
        assert_eq!(manifest.main_class, "net.minecraft.launchwrapper.Launch");
        assert!(manifest.arguments.is_none());
        assert!(
            manifest
                .minecraft_arguments
                .context("legacy arguments")?
                .contains("--tweakClass")
        );
        assert_eq!(installed.artifacts.len(), 1);
        let cached = super::super::neoforge::install_from_kind(
            &base,
            version,
            Path::new("nonexistent-java"),
            &context,
            &origin,
            true,
        )
        .await?;
        assert_eq!(cached.manifest_sha1, installed.manifest_sha1);
        std::fs::write(
            context
                .store
                .object_path(&installed.artifacts[0].resource.sha1)?,
            b"corrupt",
        )?;
        let (origin, task) = fixture(vec![(200, "", xml(&[]))]).await?;
        assert!(
            super::super::neoforge::install_from_kind(
                &base,
                version,
                Path::new("nonexistent-java"),
                &context,
                &origin,
                true
            )
            .await
            .is_err()
        );
        task.await??;
    }
    Ok(())
}

#[tokio::test]
async fn installer_rejects_wrong_game_and_build_before_network() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let context = context(root.path(), false, 0)?;
    let path = root.path().join("installer.jar");
    std::fs::write(
        &path,
        installer("1.12.2-14.23.5.2860", "1.12.2", false, false)?,
    )?;
    assert!(
        super::read_installer(&path, "1.12.2-14.23.5.2859", "1.12.2", &context)
            .await
            .is_err()
    );
    assert!(
        super::read_installer(&path, "1.12.2-14.23.5.2860", "1.8.9", &context)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn current_installers_bind_null_and_shim_paths_to_the_exact_forge_coordinate()
-> anyhow::Result<()> {
    use std::io::{Read as _, Write as _};
    let root = tempfile::tempdir()?;
    let context = context(root.path(), false, 0)?;
    for (path_value, valid) in [
        (serde_json::Value::Null, true),
        (
            json!("net.minecraftforge:forge:1.12.2-14.23.5.2860:shim"),
            true,
        ),
        (
            json!("net.minecraftforge:forge:1.12.2-14.23.5.2859:shim"),
            false,
        ),
    ] {
        let bytes = installer("1.12.2-14.23.5.2860", "1.12.2", false, false)?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        let path = root.path().join("variant.jar");
        let mut output = zip::ZipWriter::new(std::fs::File::create(&path)?);
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            if entry.name() == "install_profile.json" {
                let mut profile: serde_json::Value = serde_json::from_slice(&bytes)?;
                profile["path"] = path_value.clone();
                bytes = serde_json::to_vec(&profile)?;
            }
            output.start_file(entry.name(), zip::write::SimpleFileOptions::default())?;
            output.write_all(&bytes)?;
        }
        output.finish()?;
        assert_eq!(
            super::read_installer(&path, "1.12.2-14.23.5.2860", "1.12.2", &context)
                .await
                .is_ok(),
            valid
        );
    }
    Ok(())
}
