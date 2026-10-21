use block_download::{DownloadEvent, DownloadOptions, ProxyMode, Store};
use sha2::{Digest as _, Sha256};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    url: String,
    count: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture(first: u16, body: &'static [u8]) -> anyhow::Result<Fixture> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/agent.jar", listener.local_addr()?);
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = [0; 4096];
            if stream.read(&mut request).await.is_err() {
                continue;
            }
            let number = observed.fetch_add(1, Ordering::SeqCst);
            let status = if number == 0 { first } else { 200 };
            let header = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nRetry-After: 1\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if stream.write_all(header.as_bytes()).await.is_ok() {
                let _ = stream.write_all(body).await;
            }
        }
    });
    Ok(Fixture { url, count, task })
}
fn options(retries: u32) -> DownloadOptions {
    DownloadOptions {
        proxy: ProxyMode::Direct,
        max_retries: retries,
        ..Default::default()
    }
}

#[tokio::test]
async fn sha256_artifacts_verify_reuse_and_repair_one_copy_under_concurrency() -> anyhow::Result<()>
{
    let fixture = fixture(200, b"official synthetic agent bytes").await?;
    let root = tempfile::tempdir()?;
    let digest = format!("{:x}", Sha256::digest(b"official synthetic agent bytes"));
    let cancel = CancellationToken::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let store = Store::with_options(
        root.path().into(),
        false,
        Arc::new(move |event| {
            if let Ok(mut events) = observed.lock() {
                events.push(event);
            }
        }),
        options(0),
    )?;
    let path = store.fetch_sha256(&fixture.url, &digest, &cancel).await?;
    let offline = Store::with_options(root.path().into(), true, Arc::new(|_| {}), options(0))?;
    assert_eq!(
        offline.fetch_sha256(&fixture.url, &digest, &cancel).await?,
        path
    );
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    tokio::fs::write(&path, b"corrupted synthetic agent").await?;
    assert!(
        offline
            .fetch_sha256(&fixture.url, &digest, &cancel)
            .await
            .is_err()
    );
    let (first, second) = tokio::join!(
        store.fetch_sha256(&fixture.url, &digest, &cancel),
        store.fetch_sha256(&fixture.url, &digest, &cancel)
    );
    assert_eq!(first?, path);
    assert_eq!(second?, path);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
    assert_eq!(
        tokio::fs::read(&path).await?,
        b"official synthetic agent bytes"
    );
    let events = events
        .lock()
        .map_err(|_| anyhow::anyhow!("Fixture events unavailable"))?;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                DownloadEvent::Artifact {
                    complete: true,
                    cached: false,
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(events.iter().any(|event| matches!(
        event,
        DownloadEvent::Artifact {
            complete: true,
            cached: true,
            ..
        }
    )));
    Ok(())
}

#[tokio::test]
async fn sha256_wrong_digests_and_permanent_errors_never_publish_success() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let store = Store::with_options(root.path().into(), false, Arc::new(|_| {}), options(0))?;
    let cancel = CancellationToken::new();
    let wrong = fixture(200, b"wrong bytes").await?;
    let digest = format!("{:x}", Sha256::digest(b"expected bytes"));
    let error = store
        .fetch_sha256(&wrong.url, &digest, &cancel)
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("Wrong artifact accepted"))?;
    assert!(format!("{error:#}").contains("SHA-256 mismatch"));
    assert!(
        !root
            .path()
            .join("objects-sha256")
            .join(&digest[..2])
            .join(format!("{digest}.jar"))
            .exists()
    );
    assert!(
        store
            .fetch_sha256(&wrong.url, "../unsafe", &cancel)
            .await
            .is_err()
    );
    assert_eq!(wrong.count.load(Ordering::SeqCst), 1);
    let missing = fixture(404, b"PRIVATE_PROVIDER_BODY").await?;
    let store = Store::with_options(root.path().into(), false, Arc::new(|_| {}), options(3))?;
    let error = store
        .fetch_sha256(&missing.url, &digest, &cancel)
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("Missing artifact accepted"))?;
    assert!(format!("{error:#}").contains("404"));
    assert!(!format!("{error:#}").contains("PRIVATE_PROVIDER_BODY"));
    assert_eq!(missing.count.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn sha256_throttling_honors_retry_after_and_reports_the_actual_retry() -> anyhow::Result<()> {
    let fixture = fixture(429, b"synthetic agent").await?;
    let root = tempfile::tempdir()?;
    let digest = format!("{:x}", Sha256::digest(b"synthetic agent"));
    let cancel = CancellationToken::new();
    let retries = Arc::new(AtomicUsize::new(0));
    let observed = retries.clone();
    let store = Store::with_options(
        root.path().into(),
        false,
        Arc::new(move |event| {
            if matches!(event, DownloadEvent::Artifact { retries: 1, .. }) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }),
        options(1),
    )?;
    let start = Instant::now();
    store.fetch_sha256(&fixture.url, &digest, &cancel).await?;
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
    assert!(retries.load(Ordering::SeqCst) > 0);
    Ok(())
}

#[tokio::test]
async fn sha256_cancel_during_io_preserves_the_existing_file_and_never_retries()
-> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/agent.jar", listener.local_addr()?);
    let root = tempfile::tempdir()?;
    let store = Store::with_options(root.path().into(), false, Arc::new(|_| {}), options(2))?;
    let digest = format!("{:x}", Sha256::digest(b"expected agent"));
    let path = root
        .path()
        .join("objects-sha256")
        .join(&digest[..2])
        .join(format!("{digest}.jar"));
    tokio::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("Missing artifact directory"))?,
    )
    .await?;
    tokio::fs::write(&path, b"previous corrupt file").await?;
    let cancel = CancellationToken::new();
    let pending = cancel.clone();
    let task = tokio::spawn(async move { store.fetch_sha256(&url, &digest, &pending).await });
    let (_held, _) = listener.accept().await?;
    cancel.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await??
            .is_err()
    );
    assert_eq!(tokio::fs::read(path).await?, b"previous corrupt file");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn sha256_stalled_requests_expire_without_publishing_an_artifact() -> anyhow::Result<()> {
    let runnable = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/agent.jar", listener.local_addr()?);
    let root = tempfile::tempdir()?;
    let store = Store::with_options(root.path().into(), false, Arc::new(|_| {}), options(0))?;
    let digest = format!("{:x}", Sha256::digest(b"expected bytes"));
    let task = tokio::spawn(async move {
        store
            .fetch_sha256(&url, &digest, &CancellationToken::new())
            .await
    });
    let (_held, _) = listener.accept().await?;
    tokio::time::advance(Duration::from_secs(181)).await;
    assert!(task.await?.is_err());
    runnable.abort();
    Ok(())
}
