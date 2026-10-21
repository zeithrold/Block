use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use block_download::{Resource, Store};
use sha1::{Digest as _, Sha1};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(body: &'static [u8], first_status: u16, delay: Duration) -> anyhow::Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/object", listener.local_addr()?);
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buffer = [0; 4096];
            if socket.read(&mut buffer).await.is_err() {
                continue;
            }
            let index = counter.fetch_add(1, Ordering::Relaxed);
            let status = if index == 0 { first_status } else { 200 };
            tokio::time::sleep(delay).await;
            let header = format!(
                "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if socket.write_all(header.as_bytes()).await.is_ok() {
                let _ = socket.write_all(body).await;
            }
        }
    });
    Ok(Server {
        url,
        requests,
        task,
    })
}

fn resource(server: &Server, body: &[u8]) -> Resource {
    Resource {
        sha1: format!("{:x}", Sha1::digest(body)),
        size: body.len() as u64,
        url: server.url.clone(),
    }
}

fn store(root: &std::path::Path, offline: bool) -> anyhow::Result<Store> {
    Store::new(root.to_owned(), offline, Arc::new(|_| {}))
}

#[tokio::test]
async fn verified_objects_are_reused_offline_and_same_size_corruption_is_repaired()
-> anyhow::Result<()> {
    let server = server(b"hello", 200, Duration::ZERO).await?;
    let root = tempfile::tempdir()?;
    let store = store(root.path(), false)?;
    let item = resource(&server, b"hello");
    let token = CancellationToken::new();
    let path = store.fetch(&item, &token).await?;
    assert_eq!(tokio::fs::read(&path).await?, b"hello");
    let offline = Store::new(root.path().to_owned(), true, Arc::new(|_| {}))?;
    offline.fetch(&item, &token).await?;
    assert_eq!(server.requests.load(Ordering::Relaxed), 1);
    tokio::fs::write(&path, b"wrong").await?;
    assert!(offline.fetch(&item, &token).await.is_err());
    store.fetch(&item, &token).await?;
    assert_eq!(tokio::fs::read(path).await?, b"hello");
    assert_eq!(server.requests.load(Ordering::Relaxed), 2);
    Ok(())
}

#[tokio::test]
async fn concurrent_requests_publish_one_object_and_reject_conflicting_sizes() -> anyhow::Result<()>
{
    let server = server(b"hello", 200, Duration::from_millis(30)).await?;
    let root = tempfile::tempdir()?;
    let store = store(root.path(), false)?;
    let item = resource(&server, b"hello");
    let token = CancellationToken::new();
    let (a, b, c) = tokio::join!(
        store.fetch(&item, &token),
        store.fetch(&item, &token),
        store.fetch(&item, &token)
    );
    assert_eq!(a?, b?);
    c?;
    assert_eq!(server.requests.load(Ordering::Relaxed), 1);
    let mut wrong = item.clone();
    wrong.size += 1;
    assert!(store.fetch_all(&[item, wrong], &token).await.is_err());
    Ok(())
}

#[tokio::test]
async fn cancellation_drops_partial_objects_without_publishing_them() -> anyhow::Result<()> {
    let server = server(b"hello", 200, Duration::from_secs(10)).await?;
    let root = tempfile::tempdir()?;
    let store = store(root.path(), false)?;
    let item = resource(&server, b"hello");
    let token = CancellationToken::new();
    let stop = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        stop.cancel();
    });
    assert!(store.fetch(&item, &token).await.is_err());
    assert!(!store.object_path(&item.sha1)?.exists());
    Ok(())
}

#[tokio::test]
async fn permanent_http_errors_are_not_retried_but_server_errors_are() -> anyhow::Result<()> {
    let missing = server(b"hello", 404, Duration::ZERO).await?;
    let transient = server(b"world", 503, Duration::ZERO).await?;
    let root = tempfile::tempdir()?;
    let store = store(root.path(), false)?;
    let token = CancellationToken::new();
    assert!(
        store
            .fetch(&resource(&missing, b"hello"), &token)
            .await
            .is_err()
    );
    assert_eq!(missing.requests.load(Ordering::Relaxed), 1);
    store.fetch(&resource(&transient, b"world"), &token).await?;
    assert_eq!(transient.requests.load(Ordering::Relaxed), 2);
    Ok(())
}

#[tokio::test]
async fn invalid_digests_and_metadata_paths_are_rejected_without_network() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let store = store(root.path(), true)?;
    for digest in ["../secret", "C:/escape", "ABCDEF", ""] {
        assert!(store.object_path(digest).is_err());
    }
    for name in ["../secret", "C:/escape", "sub/file"] {
        assert!(store.metadata(name, "http://127.0.0.1:1").await.is_err());
    }
    assert!(
        store
            .metadata("version_manifest_v2.json", "http://127.0.0.1:1")
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn maven_head_uses_representation_length_instead_of_empty_head_body() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/artifact.jar", listener.local_addr()?);
    let body = b"test jar contents";
    let digest = format!("{:x}", Sha1::digest(body));
    let expected_digest = digest.clone();
    let task = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await?;
            let mut buffer = [0; 4096];
            let read = socket.read(&mut buffer).await?;
            let request = String::from_utf8_lossy(&buffer[..read]);
            let content: &[u8] = if request.contains("artifact.jar.sha1") {
                digest.as_bytes()
            } else {
                body
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                content.len()
            );
            socket.write_all(header.as_bytes()).await?;
            if !request.starts_with("HEAD ") {
                socket.write_all(content).await?;
            }
        }
        Ok::<_, anyhow::Error>(())
    });
    let directory = tempfile::tempdir()?;
    let store = store(directory.path(), false)?;
    let resource = store.describe_maven(&url).await?;
    assert_eq!(resource.size, body.len() as u64);
    assert_eq!(resource.sha1, expected_digest);
    let path = store.fetch(&resource, &CancellationToken::new()).await?;
    assert_eq!(tokio::fs::read(path).await?, body);
    task.await??;
    Ok(())
}

#[tokio::test]
async fn installer_digest_describes_artifact_without_a_sidecar_and_binds_offline_cache()
-> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/legacy.jar", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = [0; 4096];
        let n = socket.read(&mut request).await?;
        assert!(String::from_utf8_lossy(&request[..n]).starts_with("HEAD /legacy.jar "));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 123\r\nConnection: close\r\n\r\n")
            .await?;
        Ok::<_, anyhow::Error>(())
    });
    let root = tempfile::tempdir()?;
    let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let descriptor = store(root.path(), false)?
        .describe_maven_with_sha1(&url, digest)
        .await?;
    assert_eq!(descriptor.size, 123);
    task.await??;
    let offline = store(root.path(), true)?;
    assert_eq!(
        offline.describe_maven_with_sha1(&url, digest).await?,
        descriptor
    );
    assert!(
        offline
            .describe_maven_with_sha1(&url, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .await
            .is_err()
    );
    assert!(
        offline
            .describe_maven_with_sha1("http://127.0.0.1:1/other.jar", digest)
            .await
            .is_err()
    );
    assert!(
        offline
            .describe_maven_with_sha1(&url, "invalid")
            .await
            .is_err()
    );
    Ok(())
}
