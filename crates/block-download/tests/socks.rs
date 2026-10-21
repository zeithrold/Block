use anyhow::Context as _;
use block_download::{DownloadOptions, ProxyMode, Resource, Store};
use sha1::{Digest as _, Sha1};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

async fn greeting(stream: &mut TcpStream) -> anyhow::Result<()> {
    let mut header = [0; 2];
    stream.read_exact(&mut header).await?;
    anyhow::ensure!(
        header[0] == 5 && (1..=8).contains(&header[1]),
        "Invalid SOCKS5 greeting"
    );
    let mut methods = vec![0; usize::from(header[1])];
    stream.read_exact(&mut methods).await?;
    anyhow::ensure!(methods.contains(&0), "Missing anonymous SOCKS5 method");
    Ok(())
}

async fn connect(stream: &mut TcpStream) -> anyhow::Result<(u8, String, u16)> {
    stream.write_all(&[5, 0]).await?;
    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    anyhow::ensure!(header[..3] == [5, 1, 0], "Expected SOCKS5 CONNECT");
    let host = match header[3] {
        1 => {
            let mut address = [0; 4];
            stream.read_exact(&mut address).await?;
            std::net::Ipv4Addr::from(address).to_string()
        }
        3 => {
            let length = stream.read_u8().await?;
            let mut address = vec![0; usize::from(length)];
            stream.read_exact(&mut address).await?;
            String::from_utf8(address)?
        }
        4 => {
            let mut address = [0; 16];
            stream.read_exact(&mut address).await?;
            std::net::Ipv6Addr::from(address).to_string()
        }
        _ => anyhow::bail!("Unexpected SOCKS5 address type"),
    };
    let port = stream.read_u16().await?;
    Ok((header[3], host, port))
}

async fn fixture(
    statuses: Vec<u16>,
    reply: u8,
) -> anyhow::Result<(
    String,
    tokio::task::JoinHandle<anyhow::Result<Vec<(u8, String, u16, String)>>>,
)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("socks5h://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for status in statuses {
            let (mut stream, _) =
                tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
            greeting(&mut stream).await?;
            let (kind, host, port) = connect(&mut stream).await?;
            stream
                .write_all(&[5, reply, 0, 1, 127, 0, 0, 1, 0, 0])
                .await?;
            if reply != 0 {
                requests.push((kind, host, port, String::new()));
                continue;
            }
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let byte = stream.read_u8().await?;
                request.push(byte);
                anyhow::ensure!(request.len() < 8192, "Oversized request");
            }
            let request = String::from_utf8(request)?;
            let head = request.starts_with("HEAD ");
            requests.push((
                kind,
                host,
                port,
                request.lines().next().unwrap_or_default().into(),
            ));
            stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: 15\r\nRetry-After: 0\r\nConnection: close\r\n\r\n").as_bytes()).await?;
            if !head {
                stream.write_all(b"fixture payload").await?;
            }
        }
        Ok(requests)
    });
    Ok((url, task))
}

fn store(root: &std::path::Path, url: String, retries: u32) -> anyhow::Result<Store> {
    Store::with_options(
        root.into(),
        false,
        Arc::new(|_| {}),
        DownloadOptions {
            proxy: ProxyMode::manual(url)?,
            max_retries: retries,
            ..Default::default()
        },
    )
}
fn resource(url: &str) -> Resource {
    Resource {
        url: url.into(),
        sha1: format!("{:x}", Sha1::digest(b"fixture payload")),
        size: 15,
    }
}

#[test]
fn manual_proxy_protocols_round_trip_and_reject_credentials_and_invalid_endpoints()
-> anyhow::Result<()> {
    for url in [
        "http://127.0.0.1:8080",
        "https://proxy.invalid:443",
        "socks5://127.0.0.1:1080",
        "socks5h://[::1]:1080",
        "SOCKS5H://proxy.invalid",
    ] {
        let options = DownloadOptions {
            proxy: ProxyMode::manual(url.into())?,
            ..Default::default()
        };
        options.validate()?;
        assert_eq!(
            serde_json::from_slice::<DownloadOptions>(&serde_json::to_vec(&options)?)?,
            options
        );
    }
    for url in [
        "socks4://127.0.0.1:1080",
        "ftp://proxy.invalid",
        "127.0.0.1:1080",
        "socks5://",
    ] {
        assert!(ProxyMode::manual(url.into()).is_err());
    }
    for url in [
        "socks5://user:password@localhost:1080",
        "socks5h://localhost/path",
        "socks5://localhost?secret=value",
        "socks5://localhost#fragment",
        "http://localhost:1080",
    ] {
        let options = DownloadOptions {
            proxy: ProxyMode::Socks5 { url: url.into() },
            ..Default::default()
        };
        assert!(options.validate().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn proxy_dns_routes_metadata_verified_objects_and_connection_probe_through_socks5()
-> anyhow::Result<()> {
    let (url, task) = fixture(vec![200, 200, 200], 0).await?;
    let root = tempfile::tempdir()?;
    let store = store(root.path(), url, 0)?;
    let endpoint = "http://not-resolved.invalid:12345/artifact";
    assert_eq!(
        store.metadata("socks-test", endpoint).await?,
        b"fixture payload"
    );
    let path = store
        .fetch(&resource(endpoint), &CancellationToken::new())
        .await?;
    assert_eq!(std::fs::read(path)?, b"fixture payload");
    assert_eq!(store.probe_connection(endpoint).await?, 200);
    let requests = task.await??;
    assert_eq!(requests.len(), 3);
    for (index, (kind, host, port, line)) in requests.iter().enumerate() {
        assert_eq!(*kind, 3);
        assert_eq!(host, "not-resolved.invalid");
        assert_eq!(*port, 12345);
        assert_eq!(
            line,
            if index == 2 {
                "HEAD /artifact HTTP/1.1"
            } else {
                "GET /artifact HTTP/1.1"
            }
        );
    }
    Ok(())
}

#[tokio::test]
async fn local_dns_uses_an_ip_address_and_throttled_metadata_retries_over_the_same_policy()
-> anyhow::Result<()> {
    let (url, task) = fixture(vec![429, 200], 0).await?;
    let root = tempfile::tempdir()?;
    let store = store(root.path(), url.replacen("socks5h:", "socks5:", 1), 1)?;
    assert_eq!(
        store
            .metadata("local-dns", "http://127.0.0.1:32123/metadata")
            .await?,
        b"fixture payload"
    );
    let requests = task.await??;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|(kind, host, port, _)| *kind == 1 && host == "127.0.0.1" && *port == 32123)
    );
    Ok(())
}

#[tokio::test]
async fn rejected_socks_connections_and_http_errors_never_publish_success() -> anyhow::Result<()> {
    for (reply, status) in [(5, 200), (7, 200), (0, 503)] {
        let (url, task) = fixture(vec![status], reply).await?;
        let root = tempfile::tempdir()?;
        let store = store(root.path(), url, 0)?;
        let resource = resource("http://unreachable.invalid:12456/file");
        let error = store
            .fetch(&resource, &CancellationToken::new())
            .await
            .err()
            .context("Expected transfer failure")?;
        assert!(!error.to_string().is_empty());
        assert!(!store.object_path(&resource.sha1)?.exists());
        task.await??;
    }
    Ok(())
}

#[tokio::test]
async fn stalled_socks_handshake_can_be_cancelled_and_connection_probe_expires()
-> anyhow::Result<()> {
    for timeout in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let root = tempfile::tempdir()?;
        let store = store(
            root.path(),
            format!("socks5h://{}", listener.local_addr()?),
            0,
        )?;
        let cancel = CancellationToken::new();
        let interrupted = cancel.clone();
        let resource = resource("http://never-resolved.invalid:23456/file");
        let destination = store.object_path(&resource.sha1)?;
        let pending = tokio::spawn(async move {
            if timeout {
                store.probe_connection(&resource.url).await.map(|_| ())
            } else {
                store.fetch(&resource, &interrupted).await.map(|_| ())
            }
        });
        let (mut stream, _) = listener.accept().await?;
        greeting(&mut stream).await?;
        if timeout {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(11)).await;
        } else {
            cancel.cancel();
        }
        let error = pending.await?.err().context("Expected bounded failure")?;
        if timeout {
            assert!(
                error
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(reqwest::Error::is_timeout)
            );
            tokio::time::resume();
        } else {
            assert!(error.to_string().contains("cancelled"));
        }
        assert!(!destination.exists());
    }
    Ok(())
}
