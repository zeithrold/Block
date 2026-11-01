//! The declared-resource download boundary: optional SHA-512, ordered alternate
//! sources, deduplicated grouping, and secrecy of signed source URLs.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Context as _;
use block_download::{DownloadEvent, DownloadOptions, ProxyMode, Resource, Store};
use sha1::{Digest as _, Sha1};
use sha2::Sha512;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const OBJECT: &[u8] = b"block synthetic modpack object";

struct Route {
    status: u16,
    body: &'static [u8],
    delay: Duration,
}

struct Fixture {
    base: String,
    counts: BTreeMap<String, Arc<AtomicUsize>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn signed(&self, path: &str, signature: &str) -> String {
        format!("{}{path}?X-Amz-Signature={signature}", self.base)
    }

    fn requests(&self, path: &str) -> usize {
        self.counts
            .get(path)
            .map_or(0, |count| count.load(Ordering::SeqCst))
    }

    fn total_requests(&self) -> usize {
        self.counts
            .values()
            .map(|count| count.load(Ordering::SeqCst))
            .sum()
    }
}

/// A local HTTP fixture serving one fixed response per request path.
async fn fixture(
    routes: &[(&'static str, u16, &'static [u8], Duration)],
) -> anyhow::Result<Fixture> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let mut table = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for (path, status, body, delay) in routes {
        table.insert(
            (*path).to_owned(),
            Route {
                status: *status,
                body,
                delay: *delay,
            },
        );
        counts.insert((*path).to_owned(), Arc::new(AtomicUsize::new(0)));
    }
    let table = Arc::new(table);
    let served = counts.clone();
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        while let Ok((mut socket, _)) = listener.accept().await {
            let table = table.clone();
            let served = served.clone();
            connections.spawn(async move {
                let mut request = [0; 4096];
                let Ok(length) = socket.read(&mut request).await else {
                    return;
                };
                let line = String::from_utf8_lossy(&request[..length]);
                let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let path = target.split('?').next().unwrap_or("/").to_owned();
                if let Some(count) = served.get(&path) {
                    count.fetch_add(1, Ordering::SeqCst);
                }
                let (status, body, delay) = table
                    .get(&path)
                    .map_or((404, b"missing".as_slice(), Duration::ZERO), |route| {
                        (route.status, route.body, route.delay)
                    });
                tokio::time::sleep(delay).await;
                let header = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if socket.write_all(header.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(body).await;
                }
            });
        }
    });
    Ok(Fixture { base, counts, task })
}

fn digest512(body: &[u8]) -> String {
    format!("{:x}", Sha512::digest(body))
}

/// Declare every checksum this object can be held to.
fn declared(url: &str, body: &[u8]) -> Resource {
    Resource {
        sha1: format!("{:x}", Sha1::digest(body)),
        size: body.len() as u64,
        url: url.to_owned(),
        sha512: Some(digest512(body)),
        alternate_urls: Vec::new(),
    }
}

fn options(max_retries: u32) -> DownloadOptions {
    DownloadOptions {
        max_concurrent: 4,
        max_retries,
        proxy: ProxyMode::Direct,
        ..DownloadOptions::default()
    }
}

fn open_store(root: &std::path::Path, offline: bool, max_retries: u32) -> anyhow::Result<Store> {
    Store::with_options(
        root.to_owned(),
        offline,
        Arc::new(|_| {}),
        options(max_retries),
    )
}

fn recording_store(
    root: &std::path::Path,
    max_retries: u32,
    events: Arc<Mutex<Vec<DownloadEvent>>>,
) -> anyhow::Result<Store> {
    Store::with_options(
        root.to_owned(),
        false,
        Arc::new(move |event| {
            if let Ok(mut events) = events.lock() {
                events.push(event);
            }
        }),
        options(max_retries),
    )
}

fn recorded(events: &Arc<Mutex<Vec<DownloadEvent>>>) -> anyhow::Result<Vec<DownloadEvent>> {
    events
        .lock()
        .map(|events| events.clone())
        .map_err(|_| anyhow::anyhow!("recorded events are unavailable"))
}

#[test]
fn legacy_resources_stay_readable_and_optional_declarations_round_trip() -> anyhow::Result<()> {
    let legacy = serde_json::json!({
        "sha1": "da39a3ee5e6b4b0d3255bfef95601890afd80709",
        "size": 0,
        "url": "https://example.invalid/objects/empty",
    });
    let resource: Resource = serde_json::from_value(legacy.clone())?;
    assert_eq!(resource.sha512, None);
    assert!(resource.alternate_urls.is_empty());
    assert_eq!(serde_json::to_value(&resource)?, legacy);

    let full = Resource {
        sha1: format!("{:x}", Sha1::digest(OBJECT)),
        size: OBJECT.len() as u64,
        url: "https://example.invalid/objects/primary".into(),
        sha512: Some(digest512(OBJECT)),
        alternate_urls: vec![
            "https://cdn.invalid/objects/second".into(),
            "https://mirror.invalid/objects/third".into(),
        ],
    };
    let encoded = serde_json::to_vec(&full)?;
    assert_eq!(serde_json::from_slice::<Resource>(&encoded)?, full);
    Ok(())
}

#[tokio::test]
async fn sha512_is_verified_for_fresh_transfers_and_for_cache_hits() -> anyhow::Result<()> {
    let fixture = fixture(&[("/object", 200, OBJECT, Duration::ZERO)]).await?;
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), false, 0)?;
    let item = declared(&fixture.url("/object"), OBJECT);
    let cancel = CancellationToken::new();
    let path = store.fetch(&item, &cancel).await?;
    assert_eq!(tokio::fs::read(&path).await?, OBJECT);

    let offline = open_store(root.path(), true, 0)?;
    offline.fetch(&item, &cancel).await?;
    assert_eq!(fixture.requests("/object"), 1);

    // SHA-1 and size agree with the download, but the declared SHA-512 does
    // not, so the cache copy must not be accepted either.
    let mut mismatched = item.clone();
    mismatched.sha512 = Some(digest512(b"another synthetic object"));
    let error = store
        .fetch(&mismatched, &cancel)
        .await
        .err()
        .context("a mismatched SHA-512 was accepted")?;
    let text = format!("{error:#}");
    assert!(text.contains("SHA-512 mismatch"), "{text}");
    assert!(!text.contains("127.0.0.1"), "{text}");
    assert_eq!(tokio::fs::read(&path).await?, OBJECT);
    assert!(offline.fetch(&mismatched, &cancel).await.is_err());
    assert_eq!(fixture.requests("/object"), 2);

    let mut oversized = item.clone();
    oversized.size += 1;
    assert!(store.fetch(&oversized, &cancel).await.is_err());
    assert_eq!(tokio::fs::read(&path).await?, OBJECT);
    Ok(())
}

#[tokio::test]
async fn a_cached_object_must_satisfy_sha512_not_only_sha1_and_size() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let item = declared("https://example.invalid/objects/cached", OBJECT);
    let online = open_store(&root.path().join("online"), false, 0)?;
    let object = online.object_path(&item.sha1)?;
    std::fs::create_dir_all(object.parent().context("object path has no directory")?)?;
    std::fs::write(&object, OBJECT)?;

    // Offline, only the cache can satisfy the fetch, so acceptance is visible.
    let offline = open_store(&root.path().join("online"), true, 0)?;
    offline.fetch(&item, &CancellationToken::new()).await?;

    let mut mismatched = item.clone();
    mismatched.sha512 = Some(digest512(b"different synthetic object"));
    let error = offline
        .fetch(&mismatched, &CancellationToken::new())
        .await
        .err()
        .context("a cache copy with the wrong SHA-512 was accepted")?;
    assert!(
        format!("{error:#}").contains("missing or corrupt offline resource"),
        "{error:#}"
    );
    assert_eq!(tokio::fs::read(&object).await?, OBJECT);
    Ok(())
}

#[tokio::test]
async fn sources_are_tried_in_declared_order_and_reset_progress_before_the_next_source()
-> anyhow::Result<()> {
    #[derive(Debug, PartialEq)]
    enum Step {
        Bytes(u64),
        Retry(u32),
    }

    const TAMPERED: &[u8] = b"tampered payload";
    let fixture = fixture(&[
        ("/missing", 404, b"absent".as_slice(), Duration::ZERO),
        ("/tampered", 200, TAMPERED, Duration::ZERO),
        ("/served", 200, OBJECT, Duration::ZERO),
    ])
    .await?;
    let root = tempfile::tempdir()?;
    let recorded_events = Arc::new(Mutex::new(Vec::new()));
    let store = recording_store(root.path(), 1, recorded_events.clone())?;
    let mut item = declared(&fixture.url("/missing"), OBJECT);
    item.alternate_urls = vec![fixture.url("/tampered"), fixture.url("/served")];
    let path = store.fetch(&item, &CancellationToken::new()).await?;
    assert_eq!(tokio::fs::read(&path).await?, OBJECT);
    assert_eq!(fixture.requests("/missing"), 1);
    assert_eq!(fixture.requests("/tampered"), 2);
    assert_eq!(fixture.requests("/served"), 1);

    let events = recorded(&recorded_events)?;
    let mut trace: Vec<Step> = Vec::new();
    for event in &events {
        match event {
            DownloadEvent::Bytes { bytes, .. } => {
                if let Some(Step::Bytes(total)) = trace.last_mut() {
                    *total += *bytes as u64;
                } else {
                    trace.push(Step::Bytes(*bytes as u64));
                }
            }
            DownloadEvent::Retry { attempt, url, .. } => {
                assert!(url.is_empty(), "retry event carried a source URL");
                trace.push(Step::Retry(*attempt));
            }
            _ => {}
        }
        let text = format!("{event:?}");
        assert!(!text.contains("127.0.0.1"), "{text}");
        assert!(
            !text.contains("/tampered") && !text.contains("/served"),
            "{text}"
        );
    }
    // The failing source is retried once, then replaced; each restart is
    // announced before the bytes it replaces, and ordinals never go backwards.
    assert_eq!(
        trace,
        vec![
            Step::Bytes(TAMPERED.len() as u64),
            Step::Retry(1),
            Step::Bytes(TAMPERED.len() as u64),
            Step::Retry(2),
            Step::Bytes(OBJECT.len() as u64),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn each_source_keeps_its_own_retry_policy_before_the_next_is_tried() -> anyhow::Result<()> {
    let fixture = fixture(&[
        ("/flaky", 503, b"retry later".as_slice(), Duration::ZERO),
        ("/served", 200, OBJECT, Duration::ZERO),
    ])
    .await?;
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), false, 1)?;
    let mut item = declared(&fixture.url("/flaky"), OBJECT);
    item.alternate_urls = vec![fixture.url("/served")];
    let path = store.fetch(&item, &CancellationToken::new()).await?;
    assert_eq!(tokio::fs::read(&path).await?, OBJECT);
    assert_eq!(fixture.requests("/flaky"), 2);
    assert_eq!(fixture.requests("/served"), 1);
    Ok(())
}

#[tokio::test]
async fn fetch_all_groups_objects_and_keeps_every_usable_source() -> anyhow::Result<()> {
    let fixture = fixture(&[
        ("/first", 404, b"absent".as_slice(), Duration::ZERO),
        ("/second", 200, OBJECT, Duration::ZERO),
        ("/third", 200, OBJECT, Duration::ZERO),
    ])
    .await?;
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), false, 0)?;
    let cancel = CancellationToken::new();

    let mut first = declared(&fixture.url("/first"), OBJECT);
    first.sha512 = None;
    let mut second = declared(&fixture.url("/second"), OBJECT);
    second.alternate_urls = vec![fixture.url("/third")];
    let sha1 = first.sha1.clone();
    store
        .fetch_all(&[first, second], &cancel)
        .await
        .context("a grouped object with usable alternate sources failed")?;
    assert_eq!(tokio::fs::read(store.object_path(&sha1)?).await?, OBJECT);
    assert_eq!(fixture.requests("/first"), 1);
    assert_eq!(fixture.requests("/second"), 1);
    assert_eq!(fixture.requests("/third"), 0);
    Ok(())
}

#[tokio::test]
async fn fetch_all_keeps_a_stronger_duplicate_declaration_and_rejects_conflicts()
-> anyhow::Result<()> {
    let fixture = fixture(&[("/object", 200, OBJECT, Duration::ZERO)]).await?;
    let plain = declared(&fixture.url("/object"), OBJECT);
    let cancel = CancellationToken::new();

    // One identical declaration carries SHA-512; the group must keep it.
    let undecided = Resource {
        sha512: None,
        ..plain.clone()
    };
    let mut constrained = plain.clone();
    constrained.sha512 = Some(digest512(b"a different synthetic object"));
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), false, 0)?;
    let error = store
        .fetch_all(&[undecided, constrained], &cancel)
        .await
        .err()
        .context("a retained SHA-512 declaration was not enforced")?;
    assert!(
        format!("{error:#}").contains("SHA-512 mismatch"),
        "{error:#}"
    );

    // Two different declared SHA-512 values for one SHA-1 are contradictory.
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), false, 0)?;
    let mut other = plain.clone();
    other.sha512 = Some(digest512(b"a different synthetic object"));
    let before = fixture.total_requests();
    let error = store
        .fetch_all(&[plain.clone(), other], &cancel)
        .await
        .err()
        .context("conflicting SHA-512 declarations were accepted")?;
    assert!(
        format!("{error:#}").contains("conflicting SHA-512"),
        "{error:#}"
    );

    // So are two different declared sizes.
    let mut larger = plain.clone();
    larger.size += 1;
    let error = store
        .fetch_all(&[plain, larger], &cancel)
        .await
        .err()
        .context("conflicting sizes were accepted")?;
    assert!(
        format!("{error:#}").contains("conflicting sizes"),
        "{error:#}"
    );
    assert_eq!(fixture.total_requests(), before);
    Ok(())
}

#[tokio::test]
async fn malformed_declared_digests_are_rejected_before_any_request() -> anyhow::Result<()> {
    let fixture = fixture(&[("/object", 200, OBJECT, Duration::ZERO)]).await?;
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), false, 0)?;
    let item = declared(&fixture.url("/object"), OBJECT);
    let cancel = CancellationToken::new();

    let mut uppercase = item.clone();
    uppercase.sha512 = Some(digest512(OBJECT).to_uppercase());
    let error = store
        .fetch(&uppercase, &cancel)
        .await
        .err()
        .context("a malformed SHA-512 declaration was accepted")?;
    assert!(
        format!("{error:#}").contains("invalid SHA-512"),
        "{error:#}"
    );

    let mut short = item.clone();
    short.sha1 = "abc".into();
    assert!(store.fetch(&short, &cancel).await.is_err());
    assert_eq!(fixture.total_requests(), 0);
    Ok(())
}

#[tokio::test]
async fn an_imported_object_keeps_its_user_facing_message() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), true, 0)?;
    let item = Resource {
        sha1: format!("{:x}", Sha1::digest(OBJECT)),
        size: OBJECT.len() as u64,
        url: "block-import:synthetic".into(),
        sha512: None,
        alternate_urls: Vec::new(),
    };
    let error = store
        .fetch(&item, &CancellationToken::new())
        .await
        .err()
        .context("a missing imported object was accepted")?;
    assert!(error.to_string().contains("Select the official JAR again"));
    Ok(())
}

#[tokio::test]
async fn cancellation_ends_the_fetch_without_trying_the_next_source() -> anyhow::Result<()> {
    let fixture = fixture(&[
        ("/stalled", 200, OBJECT, Duration::from_secs(10)),
        ("/served", 200, OBJECT, Duration::ZERO),
    ])
    .await?;
    let root = tempfile::tempdir()?;
    let store = open_store(root.path(), false, 2)?;
    let mut item = declared(&fixture.url("/stalled"), OBJECT);
    item.alternate_urls = vec![fixture.url("/served")];
    let sha1 = item.sha1.clone();
    let token = CancellationToken::new();
    let pending = token.clone();
    let handle = tokio::spawn(async move { store.fetch(&item, &pending).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    token.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), handle)
        .await??
        .err()
        .context("a cancelled download succeeded")?;
    let text = format!("{error:#}");
    assert!(text.contains("download cancelled"), "{text}");
    assert_eq!(fixture.requests("/stalled"), 1);
    assert_eq!(fixture.requests("/served"), 0);
    let checker = open_store(root.path(), true, 0)?;
    assert!(!checker.object_path(&sha1)?.exists());
    Ok(())
}

#[tokio::test]
async fn source_urls_and_signatures_never_reach_events_or_failures() -> anyhow::Result<()> {
    for (path, status, body, expected_cause) in [
        ("/signed", 503, b"retry later".as_slice(), "503"),
        ("/corrupt", 200, b"tampered".as_slice(), "SHA-1"),
    ] {
        let fixture = fixture(&[(path, status, body, Duration::ZERO)]).await?;
        let root = tempfile::tempdir()?;
        let recorded_events = Arc::new(Mutex::new(Vec::new()));
        let store = recording_store(root.path(), 1, recorded_events.clone())?;
        let item = declared(&fixture.signed(path, "PRIVATE_SOURCE_SIGNATURE"), OBJECT);
        let error = store
            .fetch(&item, &CancellationToken::new())
            .await
            .err()
            .context("a failing signed source was accepted")?;
        let mut text = format!("{error:#}");
        for link in error.chain() {
            text.push(' ');
            text.push_str(&link.to_string());
        }
        assert!(
            text.contains("Download failed from every declared source"),
            "{text}"
        );
        assert!(text.contains(expected_cause), "{text}");
        for secret in ["PRIVATE_SOURCE_SIGNATURE", "127.0.0.1", path] {
            assert!(!text.contains(secret), "{secret} leaked into {text}");
        }

        let events = recorded(&recorded_events)?;
        assert!(
            events
                .iter()
                .any(|event| matches!(event, DownloadEvent::Retry { .. }))
        );
        for event in &events {
            assert!(!format!("{event:?}").contains("PRIVATE_SOURCE_SIGNATURE"));
            if let DownloadEvent::Retry { url, .. } = event {
                assert!(url.is_empty(), "retry event carried a source URL");
            }
        }
    }
    Ok(())
}
