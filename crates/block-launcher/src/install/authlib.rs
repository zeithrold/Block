//! Official Authlib agent discovery and verified artifact preparation.

use crate::{CancellationToken, Store};
use anyhow::{Context as _, ensure};
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};

const ORIGIN: &str = "https://authlib-injector.yushi.moe/";

#[derive(Debug, Clone)]
pub struct Agent {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Deserialize)]
struct Descriptor {
    build_number: u64,
    version: String,
    download_url: String,
    checksums: Checksums,
}
#[derive(Deserialize)]
struct Checksums {
    sha256: String,
}

/// Discover the current official build and download its SHA-256-verified JAR.
/// Offline stores reuse the cached descriptor and reverify their artifact.
pub async fn prepare(store: &Store, cancel: &CancellationToken) -> anyhow::Result<Agent> {
    prepare_from(store, cancel, ORIGIN).await
}

async fn prepare_from(
    store: &Store,
    cancel: &CancellationToken,
    origin: &str,
) -> anyhow::Result<Agent> {
    let origin = reqwest::Url::parse(origin)?;
    let endpoint = origin.join("artifact/latest.json")?;
    let bytes = tokio::select! {biased; ()=cancel.cancelled()=>anyhow::bail!("Agent preparation cancelled"),
        result=tokio::time::timeout(Duration::from_secs(30),store.metadata_pinned("authlib-agent-latest.json",endpoint.as_str()))=>result.context("Official Authlib agent metadata timed out")??,
    };
    ensure!(
        bytes.len() <= 64 * 1024,
        "Official Authlib agent descriptor is too large"
    );
    let descriptor: Descriptor =
        serde_json::from_slice(&bytes).context("Invalid official Authlib agent descriptor")?;
    let url = reqwest::Url::parse(&descriptor.download_url)?;
    ensure!(
        descriptor.build_number > 0
            && !descriptor.version.is_empty()
            && descriptor.version.len() <= 80
            && !descriptor.version.chars().any(char::is_control),
        "Invalid Authlib agent build"
    );
    ensure!(
        url.origin() == origin.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url
                .path()
                .starts_with(&format!("/artifact/{}/", descriptor.build_number))
            && url.path().ends_with(".jar"),
        "Authlib agent download must use its official artifact origin"
    );
    let path = store
        .fetch_sha256(url.as_str(), &descriptor.checksums.sha256, cancel)
        .await?;
    Ok(Agent {
        path: path.canonicalize()?,
        sha256: descriptor.checksums.sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DownloadOptions, ProxyMode};
    use sha2::{Digest as _, Sha256};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn store(root: &std::path::Path, offline: bool) -> anyhow::Result<Store> {
        Store::with_options(
            root.into(),
            offline,
            Arc::new(|_| {}),
            DownloadOptions {
                proxy: ProxyMode::Direct,
                max_retries: 0,
                ..Default::default()
            },
        )
    }
    async fn read_request(stream: &mut tokio::net::TcpStream) -> std::io::Result<String> {
        let mut bytes = Vec::new();
        while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
            let mut chunk = [0; 2048];
            let count = stream.read(&mut chunk).await?;
            if count == 0 || bytes.len() + count > 16 * 1024 {
                return Err(std::io::Error::other("Invalid fixture request"));
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned())
    }
    async fn fixture(
        descriptor: Option<serde_json::Value>,
    ) -> anyhow::Result<(
        String,
        tokio::task::JoinHandle<std::io::Result<Vec<String>>>,
    )> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}/", listener.local_addr()?);
        let jar = b"synthetic verified agent";
        let valid = descriptor.is_none();
        let descriptor=descriptor.unwrap_or_else(||serde_json::json!({"build_number":56,"version":"fixture","download_url":format!("{origin}artifact/56/agent.jar"),"checksums":{"sha256":format!("{:x}",Sha256::digest(jar))}}));
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut bodies = vec![descriptor.to_string().into_bytes()];
            if valid {
                bodies.push(jar.to_vec());
            }
            for body in bodies {
                let (mut stream, _) = listener.accept().await?;
                requests.push(read_request(&mut stream).await?);
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await?;
                stream.write_all(&body).await?;
            }
            Ok(requests)
        });
        Ok((origin, task))
    }

    #[tokio::test]
    async fn agent_uses_the_official_descriptor_and_revalidates_an_offline_cached_jar()
    -> anyhow::Result<()> {
        let (origin, task) = fixture(None).await?;
        let root = tempfile::tempdir()?;
        let cancel = CancellationToken::new();
        let agent = prepare_from(&store(root.path(), false)?, &cancel, &origin).await?;
        assert!(agent.path.is_absolute());
        assert_eq!(
            tokio::fs::read(&agent.path).await?,
            b"synthetic verified agent"
        );
        let requests = task.await??;
        assert_eq!(
            requests,
            [
                "GET /artifact/latest.json HTTP/1.1",
                "GET /artifact/56/agent.jar HTTP/1.1"
            ]
        );
        let offline = store(root.path(), true)?;
        assert_eq!(
            prepare_from(&offline, &cancel, &origin).await?.sha256,
            agent.sha256
        );
        tokio::fs::write(agent.path, b"corrupt cached agent").await?;
        assert!(prepare_from(&offline, &cancel, &origin).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn agent_rejects_foreign_artifacts_and_malformed_descriptors_before_downloading()
    -> anyhow::Result<()> {
        for descriptor in [
            serde_json::json!({}),
            serde_json::json!({"build_number":56,"version":"fixture","download_url":"https://foreign.example.test/agent.jar","checksums":{"sha256":"a".repeat(64)}}),
        ] {
            let (origin, task) = fixture(Some(descriptor)).await?;
            let root = tempfile::tempdir()?;
            assert!(
                prepare_from(
                    &store(root.path(), false)?,
                    &CancellationToken::new(),
                    &origin
                )
                .await
                .is_err()
            );
            assert_eq!(task.await??.len(), 1);
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn agent_metadata_times_out_and_cancellation_prevents_new_requests() -> anyhow::Result<()>
    {
        let runnable = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}/", listener.local_addr()?);
        let root = tempfile::tempdir()?;
        let store = store(root.path(), false)?;
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(prepare_from(&store, &cancel, &origin).await.is_err());
        let task =
            tokio::spawn(
                async move { prepare_from(&store, &CancellationToken::new(), &origin).await },
            );
        let (_held, _) = listener.accept().await?;
        tokio::time::advance(Duration::from_secs(31)).await;
        let error = task
            .await?
            .err()
            .ok_or_else(|| anyhow::anyhow!("Stalled descriptor was accepted"))?;
        assert!(error.to_string().contains("timed out"));
        runnable.abort();
        Ok(())
    }

    #[tokio::test]
    async fn agent_descriptor_redirects_cannot_change_the_trusted_origin() -> anyhow::Result<()> {
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let target_url = format!("http://{}/descriptor", target.local_addr()?);
        let landing = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}/", landing.local_addr()?);
        let redirect = tokio::spawn(async move {
            let (mut stream, _) = landing.accept().await?;
            read_request(&mut stream).await?;
            stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await
        });
        let target_task = tokio::spawn(async move {
            let (mut stream, _) = target.accept().await?;
            read_request(&mut stream).await?;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
        });
        let root = tempfile::tempdir()?;
        let error = prepare_from(
            &store(root.path(), false)?,
            &CancellationToken::new(),
            &origin,
        )
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("Foreign descriptor accepted"))?;
        assert!(error.to_string().contains("official origin"));
        assert!(
            !root
                .path()
                .join("metadata/authlib-agent-latest.json")
                .exists()
        );
        redirect.await??;
        target_task.await??;
        Ok(())
    }
}
