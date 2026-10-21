use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use block_download::{DownloadOptions, ProxyMode, Resource, Store};
use sha1::{Digest as _, Sha1};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    sync::Mutex,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    request_lines: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(
    bodies: BTreeMap<String, Vec<u8>>,
    statuses: Vec<u16>,
    delay: Duration,
) -> anyhow::Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let counter = requests.clone();
    let peak = max_active.clone();
    let lines = request_lines.clone();
    let bodies = Arc::new(bodies);
    let statuses = Arc::new(statuses);
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        while let Ok((mut socket, _)) = listener.accept().await {
            let counter = counter.clone();
            let active = active.clone();
            let peak = peak.clone();
            let lines = lines.clone();
            let bodies = bodies.clone();
            let statuses = statuses.clone();
            connections.spawn(async move {
                let mut request = [0; 4096];
                let Ok(length) = socket.read(&mut request).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&request[..length]);
                let line = request.lines().next().unwrap_or_default().to_owned();
                let target = line.split_whitespace().nth(1).unwrap_or("/");
                let path = reqwest::Url::parse(target)
                    .map_or_else(|_| target.to_owned(), |url| url.path().to_owned());
                let body = bodies.get(&path).map_or(b"hello".as_slice(), Vec::as_slice);
                lines.lock().await.push(line);
                let index = counter.fetch_add(1, Ordering::SeqCst);
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                let status = statuses
                    .get(index)
                    .or_else(|| statuses.last())
                    .copied()
                    .unwrap_or(200);
                tokio::time::sleep(delay).await;
                let header = format!(
                    "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if socket.write_all(header.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(body).await;
                }
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(Server {
        url,
        requests,
        max_active,
        request_lines,
        task,
    })
}

fn resource(url: String, body: &[u8]) -> Resource {
    Resource {
        sha1: format!("{:x}", Sha1::digest(body)),
        size: body.len() as u64,
        url,
    }
}

fn options() -> DownloadOptions {
    DownloadOptions {
        proxy: ProxyMode::Direct,
        ..DownloadOptions::default()
    }
}

fn store(root: &std::path::Path, options: DownloadOptions) -> anyhow::Result<Store> {
    Store::with_options(root.to_owned(), false, Arc::new(|_| {}), options)
}

#[test]
fn options_preserve_defaults_round_trip_and_reject_invalid_limits_and_proxy_urls()
-> anyhow::Result<()> {
    let defaults = DownloadOptions::default();
    assert_eq!(defaults.max_concurrent, 8);
    assert_eq!(defaults.max_retries, 2);
    assert_eq!(defaults.proxy, ProxyMode::System);
    assert_eq!(serde_json::from_str::<DownloadOptions>("{}")?, defaults);
    let custom = DownloadOptions {
        max_concurrent: 3,
        max_retries: 1,
        bytes_per_second: Some(4096),
        proxy: ProxyMode::Http {
            url: "https://127.0.0.1:8443".into(),
        },
    };
    custom.validate()?;
    assert_eq!(
        serde_json::from_slice::<DownloadOptions>(&serde_json::to_vec(&custom)?)?,
        custom
    );
    let root = tempfile::tempdir()?;
    for invalid in [
        DownloadOptions {
            max_concurrent: 0,
            ..options()
        },
        DownloadOptions {
            max_concurrent: 17,
            ..options()
        },
        DownloadOptions {
            max_retries: 11,
            ..options()
        },
        DownloadOptions {
            bytes_per_second: Some(0),
            ..options()
        },
    ] {
        assert!(store(root.path(), invalid).is_err());
    }
    for url in [
        "127.0.0.1:8080",
        "socks5://127.0.0.1:1080",
        "http://",
        "http://user:password@localhost",
        "http://localhost/path",
        "http://localhost?token=secret",
    ] {
        let invalid = DownloadOptions {
            proxy: ProxyMode::Http { url: url.into() },
            ..options()
        };
        assert!(
            store(root.path(), invalid).is_err(),
            "accepted invalid proxy URL"
        );
    }
    Ok(())
}

#[tokio::test]
async fn concurrency_is_bounded_across_fetch_all_and_a_cloned_store() -> anyhow::Result<()> {
    let bodies: BTreeMap<_, _> = (0..6)
        .map(|i| (format!("/{i}"), format!("object-{i}").into_bytes()))
        .collect();
    let server = server(bodies.clone(), vec![200], Duration::from_millis(60)).await?;
    let resources: Vec<_> = bodies
        .iter()
        .map(|(path, body)| resource(format!("{}{path}", server.url), body))
        .collect();
    let root = tempfile::tempdir()?;
    let store = store(
        root.path(),
        DownloadOptions {
            max_concurrent: 2,
            ..options()
        },
    )?;
    let cloned = store.clone();
    let token = CancellationToken::new();
    let (batch, separate) = tokio::join!(
        store.fetch_all(&resources[..5], &token),
        cloned.fetch(&resources[5], &token)
    );
    batch?;
    separate?;
    assert_eq!(server.requests.load(Ordering::SeqCst), 6);
    assert_eq!(server.max_active.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn retries_are_additional_attempts_and_stop_at_the_configured_boundary() -> anyhow::Result<()>
{
    for retries in [0, 1] {
        let server = server(BTreeMap::new(), vec![503], Duration::ZERO).await?;
        let root = tempfile::tempdir()?;
        let store = store(
            root.path(),
            DownloadOptions {
                max_retries: retries,
                ..options()
            },
        )?;
        let item = resource(format!("{}/object", server.url), b"hello");
        assert!(store.fetch(&item, &CancellationToken::new()).await.is_err());
        assert_eq!(server.requests.load(Ordering::SeqCst), retries as usize + 1);
        assert!(!store.object_path(&item.sha1)?.exists());
    }
    Ok(())
}

#[tokio::test]
async fn explicit_http_proxy_routes_metadata_and_objects_and_direct_bypasses_it()
-> anyhow::Result<()> {
    let proxy = server(BTreeMap::new(), vec![200], Duration::ZERO).await?;
    let origin = server(BTreeMap::new(), vec![200], Duration::ZERO).await?;
    let root = tempfile::tempdir()?;
    let proxied = store(
        &root.path().join("proxy"),
        DownloadOptions {
            proxy: ProxyMode::Http {
                url: proxy.url.clone(),
            },
            ..options()
        },
    )?;
    assert_eq!(
        proxied
            .metadata("index.json", "http://block-origin.invalid/index.json")
            .await?,
        b"hello"
    );
    proxied
        .fetch(
            &resource("http://block-origin.invalid/object".into(), b"hello"),
            &CancellationToken::new(),
        )
        .await?;
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 2);
    assert!(
        proxy
            .request_lines
            .lock()
            .await
            .iter()
            .all(|line| line.starts_with("GET http://block-origin.invalid/"))
    );
    let direct = store(&root.path().join("direct"), options())?;
    direct
        .fetch(
            &resource(format!("{}/object", origin.url), b"hello"),
            &CancellationToken::new(),
        )
        .await?;
    assert_eq!(origin.requests.load(Ordering::SeqCst), 1);
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn aggregate_rate_limit_really_delays_parallel_object_completion() -> anyhow::Result<()> {
    let bodies = BTreeMap::from([
        ("/0".into(), vec![0_u8; 4096]),
        ("/1".into(), vec![1_u8; 4096]),
    ]);
    let server = server(bodies.clone(), vec![200], Duration::ZERO).await?;
    let resources: Vec<_> = bodies
        .iter()
        .map(|(path, body)| resource(format!("{}{path}", server.url), body))
        .collect();
    let root = tempfile::tempdir()?;
    let store = store(
        root.path(),
        DownloadOptions {
            bytes_per_second: Some(16_384),
            ..options()
        },
    )?;
    let cloned = store.clone();
    let token = CancellationToken::new();
    let start = Instant::now();
    let (first, second) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            store.fetch(&resources[0], &token),
            cloned.fetch(&resources[1], &token)
        )
    })
    .await?;
    first?;
    second?;
    assert!(start.elapsed() >= Duration::from_millis(490));
    Ok(())
}

#[tokio::test]
async fn cancellation_interrupts_a_long_rate_limit_wait() -> anyhow::Result<()> {
    let server = server(BTreeMap::new(), vec![200], Duration::ZERO).await?;
    let root = tempfile::tempdir()?;
    let store = store(
        root.path(),
        DownloadOptions {
            bytes_per_second: Some(1),
            ..options()
        },
    )?;
    let item = resource(format!("{}/object", server.url), b"hello");
    let token = CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    });
    let result = tokio::time::timeout(Duration::from_secs(1), store.fetch(&item, &token)).await?;
    assert!(result.is_err());
    assert!(!store.object_path(&item.sha1)?.exists());
    Ok(())
}
