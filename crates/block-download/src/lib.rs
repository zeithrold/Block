//! Streaming, content-addressed downloads with atomic verified publication.

use std::{
    collections::{BTreeMap, btree_map::Entry},
    fmt,
    fs::File,
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, bail, ensure};
use futures_util::{StreamExt as _, TryStreamExt as _};
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};
use sha2::{Sha256, Sha512};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

/// Default size bound for SHA-256 artifacts downloaded by the generic store API.
const MAX_SHA256_ARTIFACT_SIZE: u64 = 16 * 1024 * 1024;

/// URL marker for an object already imported from a user-selected local file.
/// It names no fetchable source, so a missing copy must be re-imported.
const IMPORT_URL_PREFIX: &str = "block-import:";

/// Explicit proxy behavior for all HTTP requests made by a [`Store`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxyMode {
    /// Preserve reqwest's environment/system proxy discovery.
    #[default]
    System,
    /// Bypass proxies, including environment proxy variables.
    Direct,
    /// Use an HTTP or HTTPS proxy without credentials.
    Http { url: String },
    /// Use SOCKS5 with local DNS (`socks5`) or proxy DNS (`socks5h`).
    Socks5 { url: String },
}

impl ProxyMode {
    /// Select the manual protocol from its URL; download-option validation also
    /// checks the host, credentials and address components before saving or use.
    /// # Errors
    /// Rejects malformed URLs and unsupported manual proxy schemes.
    pub fn manual(url: String) -> anyhow::Result<Self> {
        let parsed = reqwest::Url::parse(&url).context("invalid proxy URL")?;
        ensure!(parsed.host_str().is_some(), "proxy URL requires a host");
        match parsed.scheme() {
            "http" | "https" => Ok(Self::Http { url }),
            "socks5" | "socks5h" => Ok(Self::Socks5 { url }),
            _ => bail!("proxy scheme must be http, https, socks5, or socks5h"),
        }
    }
}

/// Object-transfer settings; proxy selection also applies to metadata requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadOptions {
    /// Maximum simultaneous object transfers, shared by clones. Must be 1..=16.
    pub max_concurrent: usize,
    /// Additional attempts after the first request. Must be 0..=10.
    pub max_retries: u32,
    /// Aggregate object-body bytes per second, shared by clones. None is unlimited.
    pub bytes_per_second: Option<u64>,
    pub proxy: ProxyMode,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            max_concurrent: 8,
            max_retries: 2,
            bytes_per_second: None,
            proxy: ProxyMode::System,
        }
    }
}

impl DownloadOptions {
    /// Validate settings before saving or constructing a download store.
    ///
    /// # Errors
    /// Rejects out-of-range limits, zero rates, unsupported proxy schemes,
    /// credentials, or proxy URLs with paths, queries, or fragments.
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            (1..=16).contains(&self.max_concurrent),
            "Choose between 1 and 16 parallel downloads."
        );
        ensure!(self.max_retries <= 10, "download retries must be 0..=10");
        ensure!(
            self.bytes_per_second != Some(0),
            "download rate must be positive or unlimited"
        );
        if let ProxyMode::Http { url } | ProxyMode::Socks5 { url } = &self.proxy {
            let proxy = reqwest::Url::parse(url).context("invalid proxy URL")?;
            match self.proxy {
                ProxyMode::Http { .. } => ensure!(
                    matches!(proxy.scheme(), "http" | "https"),
                    "HTTP proxy scheme must be http or https"
                ),
                ProxyMode::Socks5 { .. } => ensure!(
                    matches!(proxy.scheme(), "socks5" | "socks5h"),
                    "SOCKS5 proxy scheme must be socks5 or socks5h"
                ),
                ProxyMode::System | ProxyMode::Direct => {}
            }
            ensure!(proxy.host_str().is_some(), "proxy URL requires a host");
            ensure!(
                proxy.username().is_empty() && proxy.password().is_none(),
                "proxy credentials are not supported"
            );
            ensure!(
                matches!(proxy.path(), "" | "/")
                    && proxy.query().is_none()
                    && proxy.fragment().is_none(),
                "proxy URL must contain only a scheme, host, and optional port"
            );
        }
        Ok(())
    }
}

struct RateLimiter {
    bytes_per_second: u64,
    last_chunk: Mutex<tokio::time::Instant>,
}

impl RateLimiter {
    async fn wait(&self, bytes: u64, cancel: &CancellationToken) -> anyhow::Result<()> {
        let mut last = tokio::select! {
            () = cancel.cancelled() => bail!("download cancelled"),
            last = self.last_chunk.lock() => last,
        };
        let nanos = (u128::from(bytes % self.bytes_per_second) * 1_000_000_000)
            .div_ceil(u128::from(self.bytes_per_second));
        let duration = Duration::from_secs(bytes / self.bytes_per_second)
            + Duration::from_nanos(u64::try_from(nanos)?);
        let deadline = (*last).max(tokio::time::Instant::now()) + duration;
        tokio::select! {
            () = cancel.cancelled() => bail!("download cancelled"),
            () = tokio::time::sleep_until(deadline) => {},
        }
        *last = deadline;
        drop(last);
        Ok(())
    }
}

/// An upstream-verified object. SHA-1 is mandated by Minecraft metadata; the
/// optional SHA-512 is a stronger check declared by some sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub sha1: String,
    pub size: u64,
    pub url: String,
    /// Additional checksum to verify on cached and freshly transferred objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    /// Fallback sources tried in declared order after [`Self::url`] fails.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternate_urls: Vec<String>,
}

impl Resource {
    /// Declared sources in attempt order, without duplicates or blank entries.
    fn urls(&self) -> Vec<&str> {
        let mut urls: Vec<&str> = Vec::new();
        for url in std::iter::once(&self.url).chain(&self.alternate_urls) {
            if !url.is_empty() && !urls.contains(&url.as_str()) {
                urls.push(url);
            }
        }
        urls
    }

    /// True when any declared source is a local import rather than a fetchable URL.
    fn has_import_source(&self) -> bool {
        self.urls()
            .iter()
            .any(|url| url.starts_with(IMPORT_URL_PREFIX))
    }

    /// Reject malformed declared digests before any cache or network use.
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            is_lowercase_hex(&self.sha1, 40),
            "invalid SHA-1: {}",
            self.sha1
        );
        if let Some(sha512) = &self.sha512 {
            ensure!(is_lowercase_hex(sha512, 128), "invalid SHA-512: {sha512}");
        }
        Ok(())
    }
}

/// Accumulates every digest declaration of one object, for cached copies and
/// for fresh transfers alike, so both paths accept exactly the same content.
struct ObjectDigests {
    sha1: Sha1,
    sha512: Option<Sha512>,
    size: u64,
}

impl ObjectDigests {
    fn new(expected: &Resource) -> Self {
        Self {
            sha1: Sha1::new(),
            sha512: expected.sha512.as_ref().map(|_| Sha512::new()),
            size: 0,
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        self.size += bytes.len() as u64;
        self.sha1.update(bytes);
        if let Some(sha512) = &mut self.sha512 {
            sha512.update(bytes);
        }
    }

    fn matches(self, expected: &Resource) -> bool {
        let sha512 = self.sha512.map(|hasher| format!("{:x}", hasher.finalize()));
        self.size == expected.size
            && format!("{:x}", self.sha1.finalize()) == expected.sha1
            && sha512.as_deref() == expected.sha512.as_deref()
    }
}

fn is_lowercase_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Artifact {
        sha256: String,
        received: u64,
        total: Option<u64>,
        complete: bool,
        cached: bool,
        retries: u32,
    },
    Started {
        sha1: String,
        size: u64,
    },
    Bytes {
        sha1: String,
        bytes: usize,
    },
    Complete {
        sha1: String,
        cached: bool,
    },
    Retry {
        sha1: String,
        /// Always empty: source URLs may embed temporary signatures, so they
        /// never leave the store through an event.
        url: String,
        /// Monotonic count of the retries and source changes of this object, so
        /// observers can restart current-file progress without ordering gaps.
        attempt: u32,
    },
}

pub type EventSink = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

/// All persistent data belongs under the caller's data directory.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    client: reqwest::Client,
    events: EventSink,
    offline: bool,
    options: DownloadOptions,
    transfers: Arc<Semaphore>,
    rate_limiter: Option<Arc<RateLimiter>>,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("root", &self.root)
            .field("offline", &self.offline)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Check an endpoint through this store's actual HTTP client and proxy.
    /// No response body, object cache, credentials or preferences are written.
    ///
    /// # Errors
    /// Offline stores, transport failures, a ten-second deadline and HTTP errors
    /// are reported as failures rather than treating a connection as success.
    pub async fn probe_connection(&self, url: &str) -> anyhow::Result<u16> {
        ensure!(
            !self.offline,
            "Connection testing is unavailable in offline mode"
        );
        let response = self
            .client
            .head(url)
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?;
        Ok(response.status().as_u16())
    }

    /// Create a reusable HTTP client and cache.
    ///
    /// # Errors
    /// Returns errors configuring HTTP/TLS or creating the data directory.
    pub fn new(root: PathBuf, offline: bool, events: EventSink) -> anyhow::Result<Self> {
        Self::with_options(root, offline, events, DownloadOptions::default())
    }

    /// Create a store with validated transfer settings and a shared HTTP client.
    ///
    /// # Errors
    /// Rejects invalid settings and failures configuring HTTP/TLS or cache storage.
    pub fn with_options(
        root: PathBuf,
        offline: bool,
        events: EventSink,
        options: DownloadOptions,
    ) -> anyhow::Result<Self> {
        options.validate()?;
        let mut client = reqwest::Client::builder()
            .user_agent("Block/0.1 Minecraft launcher")
            .connect_timeout(Duration::from_secs(20));
        // Throttled transfers may legitimately take more than three minutes.
        // Keep network-stall detection without charging deliberate rate waits
        // against a total request deadline. Metadata keeps its total deadline.
        client = if options.bytes_per_second.is_some() {
            client.read_timeout(Duration::from_secs(180))
        } else {
            client.timeout(Duration::from_secs(180))
        };
        client = match &options.proxy {
            ProxyMode::System => client,
            ProxyMode::Direct => client.no_proxy(),
            ProxyMode::Http { url } | ProxyMode::Socks5 { url } => {
                client.no_proxy().proxy(reqwest::Proxy::all(url)?)
            }
        };
        let client = client.build()?;
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            client,
            events,
            offline,
            transfers: Arc::new(Semaphore::new(options.max_concurrent)),
            rate_limiter: options.bytes_per_second.map(|bytes_per_second| {
                Arc::new(RateLimiter {
                    bytes_per_second,
                    last_chunk: Mutex::new(tokio::time::Instant::now()),
                })
            }),
            options,
        })
    }

    #[must_use]
    pub const fn options(&self) -> &DownloadOptions {
        &self.options
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a Maven artifact descriptor, retaining it for fully offline replay.
    ///
    /// # Errors
    /// Fails if upstream checksum/length metadata is missing or HTTP/IO fails.
    pub async fn describe_maven(&self, url: &str) -> anyhow::Result<Resource> {
        let key = format!("{:x}", Sha1::digest(url.as_bytes()));
        let directory = self.root.join("metadata");
        tokio::fs::create_dir_all(&directory).await?;
        let path = directory.join(format!("maven-{key}.json"));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            let resource: Resource = serde_json::from_slice(&bytes)?;
            if resource.size > 0 {
                return Ok(resource);
            }
        }
        ensure!(!self.offline, "missing offline Maven descriptor: {url}");
        let digest = self
            .client
            .get(format!("{url}.sha1"))
            .timeout(Duration::from_secs(180))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let sha1 = digest
            .split_whitespace()
            .next()
            .context("empty Maven SHA-1")?
            .to_owned();
        self.describe_maven_with_sha1(url, &sha1).await
    }

    /// Describe an artifact whose digest is already supplied by verified installer metadata.
    /// Repositories need not publish a separate checksum file for this path.
    /// # Errors
    /// Rejects invalid digests, missing offline metadata and invalid HTTP length responses.
    pub async fn describe_maven_with_sha1(
        &self,
        url: &str,
        sha1: &str,
    ) -> anyhow::Result<Resource> {
        self.object_path(sha1)?;
        let directory = self.root.join("metadata");
        tokio::fs::create_dir_all(&directory).await?;
        let key = format!("{:x}", Sha1::digest(url.as_bytes()));
        let path = directory.join(format!("maven-{key}.json"));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            let resource: Resource = serde_json::from_slice(&bytes)?;
            if resource.size > 0 && resource.sha1 == sha1 && resource.url == url {
                return Ok(resource);
            }
        }
        ensure!(!self.offline, "missing offline Maven descriptor: {url}");
        let response = self
            .client
            .head(url)
            .timeout(Duration::from_secs(180))
            .send()
            .await?
            .error_for_status()?;
        // A HEAD response has a zero-byte body even when its Content-Length
        // describes a nonempty GET representation. Read the header explicitly.
        let size = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .context("Maven artifact has no Content-Length")?
            .to_str()?
            .parse()?;
        let resource = Resource {
            sha1: sha1.into(),
            size,
            url: url.into(),
            sha512: None,
            alternate_urls: Vec::new(),
        };
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        std::io::Write::write_all(&mut temporary, &serde_json::to_vec(&resource)?)?;
        temporary.persist(path)?;
        Ok(resource)
    }

    /// Return a confined object path after validating its upstream digest.
    ///
    /// # Errors
    /// Fails if the digest is not a lowercase SHA-1 value.
    pub fn object_path(&self, sha1: &str) -> anyhow::Result<PathBuf> {
        ensure!(is_lowercase_hex(sha1, 40), "invalid SHA-1: {sha1}");
        Ok(self.root.join("objects").join(&sha1[..2]).join(sha1))
    }

    /// Return a confined object path for an upstream SHA-256 artifact.
    ///
    /// # Errors
    /// Fails if the digest is not a lowercase SHA-256 value.
    pub fn sha256_object_path(&self, digest: &str) -> anyhow::Result<PathBuf> {
        ensure!(is_lowercase_hex(digest, 64), "invalid SHA-256 digest");
        Ok(self
            .root
            .join("objects-sha256")
            .join(&digest[..2])
            .join(format!("{digest}.jar")))
    }

    /// Fetch mutable root metadata, or read it from cache in offline mode.
    ///
    /// # Errors
    /// Fails on missing offline metadata, invalid names, HTTP errors, or IO.
    pub async fn metadata(&self, name: &str, url: &str) -> anyhow::Result<Vec<u8>> {
        self.metadata_policy(name, url, false).await
    }

    /// Read artifact discovery metadata without accepting a different final
    /// origin. The shared cache and proxy policy still apply.
    pub async fn metadata_pinned(&self, name: &str, url: &str) -> anyhow::Result<Vec<u8>> {
        self.metadata_policy(name, url, true).await
    }

    async fn metadata_policy(
        &self,
        name: &str,
        url: &str,
        pin_origin: bool,
    ) -> anyhow::Result<Vec<u8>> {
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'),
            "invalid metadata name"
        );
        let directory = self.root.join("metadata");
        tokio::fs::create_dir_all(&directory).await?;
        let path = directory.join(name);
        if self.offline {
            return tokio::fs::read(&path)
                .await
                .with_context(|| format!("missing offline metadata: {}", path.display()));
        }
        let mut attempt = 0;
        let response = loop {
            let response = self
                .client
                .get(url)
                .timeout(Duration::from_secs(180))
                .send()
                .await;
            let retryable = match &response {
                Ok(response) => {
                    response.status().as_u16() == 429 || response.status().is_server_error()
                }
                Err(error) => error.is_timeout() || error.is_connect(),
            };
            if !retryable || attempt >= self.options.max_retries {
                break response?.error_for_status()?;
            }
            let retry_after = response.as_ref().ok().and_then(|response| {
                response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)?
                    .to_str()
                    .ok()
            });
            let delay = retry_after
                .and_then(|value| {
                    value
                        .parse::<u64>()
                        .ok()
                        .map(Duration::from_secs)
                        .or_else(|| {
                            httpdate::parse_http_date(value).ok().map(|date| {
                                date.duration_since(std::time::SystemTime::now())
                                    .unwrap_or_default()
                            })
                        })
                })
                .unwrap_or_else(|| Duration::from_secs(u64::from(attempt + 1)));
            tokio::time::sleep(delay.min(Duration::from_secs(86400))).await;
            attempt += 1;
        };
        if pin_origin {
            ensure!(
                response.url().origin() == reqwest::Url::parse(url)?.origin(),
                "Artifact metadata redirected outside its official origin"
            );
        }
        ensure!(
            response.content_length().unwrap_or(0) <= 16 * 1024 * 1024,
            "root metadata is too large"
        );
        let mut stream = response.bytes_stream();
        let mut data = Vec::new();
        while let Some(bytes) = stream.try_next().await? {
            ensure!(
                data.len() + bytes.len() <= 16 * 1024 * 1024,
                "root metadata is too large"
            );
            data.extend_from_slice(&bytes);
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        std::io::Write::write_all(&mut temporary, &data)?;
        temporary.persist(&path)?;
        Ok(data)
    }

    /// Download unique objects with a bounded number of in-flight futures.
    ///
    /// Objects are grouped by SHA-1. Duplicates must declare one size and one
    /// SHA-512; a declaration carrying SHA-512 is retained, and their sources
    /// are combined in declaration order.
    ///
    /// # Errors
    /// Fails on malformed digests, conflicting metadata, download/integrity
    /// failures, or cancellation.
    pub async fn fetch_all(
        &self,
        resources: &[Resource],
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        let mut unique: BTreeMap<&str, Resource> = BTreeMap::new();
        for resource in resources {
            resource.validate()?;
            match unique.entry(&resource.sha1) {
                Entry::Vacant(slot) => {
                    slot.insert(resource.clone());
                }
                Entry::Occupied(mut slot) => {
                    let merged = slot.get_mut();
                    ensure!(
                        merged.size == resource.size,
                        "conflicting sizes for {}",
                        resource.sha1
                    );
                    match (&merged.sha512, &resource.sha512) {
                        (Some(known), Some(other)) if known != other => {
                            bail!("conflicting SHA-512 for {}", resource.sha1)
                        }
                        (None, Some(stronger)) => merged.sha512 = Some(stronger.clone()),
                        _ => {}
                    }
                    let mut urls: Vec<String> =
                        merged.urls().into_iter().map(str::to_owned).collect();
                    for url in resource.urls() {
                        if !urls.iter().any(|known| known.as_str() == url) {
                            urls.push(url.to_owned());
                        }
                    }
                    let mut urls = urls.into_iter();
                    if let Some(primary) = urls.next() {
                        merged.url = primary;
                    }
                    merged.alternate_urls = urls.collect();
                }
            }
        }
        futures_util::stream::iter(unique.into_values())
            .map(|resource| async move {
                self.fetch(&resource, cancel)
                    .await
                    .with_context(|| format!("failed to download {}", resource.sha1))
            })
            .buffer_unordered(self.options.max_concurrent)
            .try_collect::<Vec<_>>()
            .await?;
        Ok(())
    }

    /// Download and verify one object. Concurrent processes share an OS file lock.
    /// A cached copy must satisfy every declared digest; otherwise the declared
    /// sources are attempted in order, each under the usual retry policy. A retry,
    /// and a source change after a failed attempt that already reported bytes,
    /// restart current-file progress through a URL-redacted retry event.
    ///
    /// # Errors
    /// Fails on a corrupt object, missing offline resource, HTTP/IO, integrity,
    /// or cancellation. Messages never repeat a source URL, which may embed a
    /// temporary signature.
    pub async fn fetch(
        &self,
        resource: &Resource,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PathBuf> {
        ensure!(!cancel.is_cancelled(), "download cancelled");
        resource.validate()?;
        let path = self.object_path(&resource.sha1)?;
        let directory = path.parent().context("object directory is missing")?;
        tokio::fs::create_dir_all(directory).await?;
        let lock_path = path.with_extension("lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        loop {
            match fs2::FileExt::try_lock_exclusive(&lock) {
                Ok(()) => break,
                Err(error)
                    if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
                Err(error) => return Err(error.into()),
            }
            tokio::select! { () = cancel.cancelled() => bail!("download cancelled"), () = tokio::time::sleep(Duration::from_millis(50)) => {} }
        }
        let check_path = path.clone();
        let expected = resource.clone();
        let cached =
            tokio::task::spawn_blocking(move || verify_file(&check_path, &expected)).await??;
        if cached {
            (self.events)(DownloadEvent::Complete {
                sha1: resource.sha1.clone(),
                cached: true,
            });
            return Ok(path);
        }
        let sources: Vec<&str> = resource
            .urls()
            .into_iter()
            .filter(|url| !url.starts_with(IMPORT_URL_PREFIX))
            .collect();
        if sources.is_empty() {
            ensure!(
                !resource.has_import_source(),
                "A user-supplied installation file is missing or corrupt. Select the official JAR again."
            );
            bail!("The resource declares no downloadable source URL.");
        }
        ensure!(
            !self.offline,
            "missing or corrupt offline resource: {}",
            resource.sha1
        );
        let _permit = tokio::select! {
            () = cancel.cancelled() => bail!("download cancelled"),
            permit = self.transfers.acquire() => permit?,
        };
        (self.events)(DownloadEvent::Started {
            sha1: resource.sha1.clone(),
            size: resource.size,
        });
        let mut failure = None;
        let mut retries = 0_u32;
        let mut sources = sources.into_iter().peekable();
        while let Some(source) = sources.next() {
            for attempt in 0..=self.options.max_retries {
                let mut reported = 0_u64;
                let result = tokio::select! {
                    () = cancel.cancelled() => bail!("download cancelled"),
                    result = self.transfer(resource, source, directory, &mut reported, cancel) => result,
                };
                let error = match result {
                    Ok(temporary) => {
                        temporary.persist(&path)?;
                        (self.events)(DownloadEvent::Complete {
                            sha1: resource.sha1.clone(),
                            cached: false,
                        });
                        return Ok(path);
                    }
                    Err(error) => error,
                };
                let permanent = error
                    .downcast_ref::<reqwest::Error>()
                    .and_then(reqwest::Error::status)
                    .is_some_and(|status| {
                        status.is_client_error() && status.as_u16() != 408 && status.as_u16() != 429
                    });
                // A spent source, and any permanent rejection of it, both move
                // on to the next declared URL instead of retrying this one.
                if permanent || attempt == self.options.max_retries {
                    failure = Some(error);
                    // Bytes already announced for this source must not stay in
                    // current-file progress after another source takes over.
                    if reported > 0 && sources.peek().is_some() {
                        retries += 1;
                        self.retry_event(&resource.sha1, retries);
                    }
                    break;
                }
                retries += 1;
                self.retry_event(&resource.sha1, retries);
                tokio::select! { () = cancel.cancelled() => bail!("download cancelled"), () = tokio::time::sleep(Duration::from_secs(u64::from(attempt + 1))) => {} }
            }
        }
        Err(failure.unwrap_or_else(|| anyhow::anyhow!("no source was attempted")))
            .context("Download failed from every declared source")
    }

    /// Fetch an upstream SHA-256 artifact with bounded size and the store's
    /// proxy, concurrency, retry and aggregate rate policies. Only a verified
    /// temporary file is published; cancellation never replaces the destination.
    ///
    /// # Errors
    /// Fails for invalid digests, transport or size failures, or cancellation.
    pub async fn fetch_sha256(
        &self,
        url: &str,
        digest: &str,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PathBuf> {
        self.fetch_sha256_bounded(url, digest, MAX_SHA256_ARTIFACT_SIZE, cancel)
            .await
    }

    /// Fetch an upstream SHA-256 artifact under an explicit size bound. Used by
    /// larger official vendor archives (for example a Java runtime tarball)
    /// while [`Self::fetch_sha256`] keeps the default artifact bound.
    ///
    /// # Errors
    /// Fails for invalid digests or size bounds, transport or size failures, or
    /// cancellation.
    pub async fn fetch_sha256_bounded(
        &self,
        url: &str,
        digest: &str,
        max_size: u64,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PathBuf> {
        ensure!(max_size > 0, "invalid SHA-256 artifact size bound");
        ensure!(!cancel.is_cancelled(), "download cancelled");
        let path = self.sha256_object_path(digest)?;
        let directory = path
            .parent()
            .context("SHA-256 object has no parent")?
            .to_owned();
        tokio::fs::create_dir_all(&directory).await?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        loop {
            match fs2::FileExt::try_lock_exclusive(&lock) {
                Ok(()) => break,
                Err(error)
                    if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
                Err(error) => return Err(error.into()),
            }
            tokio::select! {biased; ()=cancel.cancelled()=>bail!("download cancelled"), ()=tokio::time::sleep(Duration::from_millis(50))=>{}}
        }
        let check = path.clone();
        let expected = digest.to_owned();
        let cached = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<u64>> {
            let Ok(mut file) = File::open(check) else {
                return Ok(None);
            };
            let size = file.metadata()?.len();
            if size == 0 || size > max_size {
                return Ok(None);
            }
            let mut hash = Sha256::new();
            let mut bytes = [0; 64 * 1024];
            loop {
                let count = file.read(&mut bytes)?;
                if count == 0 {
                    break;
                }
                hash.update(&bytes[..count]);
            }
            Ok((format!("{:x}", hash.finalize()) == expected).then_some(size))
        })
        .await??;
        if let Some(size) = cached {
            (self.events)(DownloadEvent::Artifact {
                sha256: digest.into(),
                received: size,
                total: Some(size),
                complete: true,
                cached: true,
                retries: 0,
            });
            return Ok(path);
        }
        ensure!(!self.offline, "missing or corrupt offline SHA-256 artifact");
        let _permit = tokio::select! {biased; ()=cancel.cancelled()=>bail!("download cancelled"),permit=self.transfers.acquire()=>permit?,};
        for attempt in 0..=self.options.max_retries {
            let mut retry_delay = Duration::from_secs(u64::from(attempt + 1));
            let mut received = 0u64;
            let mut total = None;
            (self.events)(DownloadEvent::Artifact {
                sha256: digest.into(),
                received,
                total,
                complete: false,
                cached: false,
                retries: attempt,
            });
            let transfer = async {
                let response = self.client.get(url).send().await?;
                ensure!(
                    response.url().origin() == reqwest::Url::parse(url)?.origin(),
                    "SHA-256 artifact redirected outside its official origin"
                );
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                    && let Some(value) = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                {
                    let seconds = value.parse::<u64>().ok().or_else(|| {
                        httpdate::parse_http_date(value)
                            .ok()
                            .and_then(|when| when.duration_since(std::time::SystemTime::now()).ok())
                            .map(|d| d.as_secs())
                    });
                    if let Some(seconds) = seconds {
                        retry_delay = Duration::from_secs(seconds.clamp(1, 86400));
                    }
                }
                let response = response.error_for_status()?;
                total = response.content_length();
                ensure!(
                    total.is_none_or(|size| size > 0 && size <= max_size),
                    "SHA-256 artifact exceeds the size limit"
                );
                let temporary = tempfile::NamedTempFile::new_in(&directory)?;
                let mut file = tokio::fs::File::from_std(temporary.reopen()?);
                let mut stream = response.bytes_stream();
                let mut hash = Sha256::new();
                while let Some(bytes) = stream.try_next().await? {
                    received = received.saturating_add(bytes.len() as u64);
                    ensure!(
                        received <= max_size,
                        "SHA-256 artifact exceeds the size limit"
                    );
                    if let Some(limiter) = &self.rate_limiter {
                        limiter.wait(bytes.len() as u64, cancel).await?;
                    }
                    hash.update(&bytes);
                    file.write_all(&bytes).await?;
                    (self.events)(DownloadEvent::Artifact {
                        sha256: digest.into(),
                        received,
                        total,
                        complete: false,
                        cached: false,
                        retries: attempt,
                    });
                }
                ensure!(
                    received > 0
                        && total.is_none_or(|size| size == received)
                        && format!("{:x}", hash.finalize()) == digest,
                    "artifact size or SHA-256 mismatch"
                );
                file.flush().await?;
                drop(file);
                Ok::<_, anyhow::Error>(temporary)
            };
            let result = tokio::select! {biased; ()=cancel.cancelled()=>bail!("download cancelled"),result=transfer=>result,};
            match result {
                Ok(temporary) => {
                    ensure!(!cancel.is_cancelled(), "download cancelled");
                    temporary.persist(&path)?;
                    (self.events)(DownloadEvent::Artifact {
                        sha256: digest.into(),
                        received,
                        total,
                        complete: true,
                        cached: false,
                        retries: attempt,
                    });
                    return Ok(path);
                }
                Err(error) => {
                    let permanent = error
                        .downcast_ref::<reqwest::Error>()
                        .and_then(reqwest::Error::status)
                        .is_some_and(|status| {
                            status.is_client_error()
                                && status.as_u16() != 408
                                && status.as_u16() != 429
                        });
                    if permanent || attempt == self.options.max_retries {
                        return Err(error).context("SHA-256 artifact download failed");
                    }
                    tokio::select! {biased; ()=cancel.cancelled()=>bail!("download cancelled"),()=tokio::time::sleep(retry_delay)=>{}}
                }
            }
        }
        bail!("download attempts exhausted")
    }

    /// Announce a retry without its source URL, so observers restart
    /// current-file progress while signed URLs stay inside the store.
    fn retry_event(&self, sha1: &str, attempt: u32) {
        (self.events)(DownloadEvent::Retry {
            sha1: sha1.to_owned(),
            url: String::new(),
            attempt,
        });
    }

    /// Stream one source into a temporary file. `reported` accumulates the body
    /// bytes already announced to the event sink, so a failed attempt can tell
    /// whether observers hold progress that the next source has to reset.
    async fn transfer(
        &self,
        resource: &Resource,
        source: &str,
        directory: &Path,
        reported: &mut u64,
        cancel: &CancellationToken,
    ) -> anyhow::Result<tempfile::NamedTempFile> {
        // Source URLs may embed temporary signatures, so transport errors are
        // stripped of the URL before they can reach a caller or an event sink.
        let response = self
            .client
            .get(source)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?;
        let temporary = tempfile::NamedTempFile::new_in(directory)?;
        let mut file = tokio::fs::File::from_std(temporary.reopen()?);
        let mut stream = response.bytes_stream();
        let mut digests = ObjectDigests::new(resource);
        while let Some(bytes) = stream
            .try_next()
            .await
            .map_err(reqwest::Error::without_url)?
        {
            digests.update(&bytes);
            ensure!(
                digests.size <= resource.size,
                "resource exceeds declared size"
            );
            if let Some(limiter) = &self.rate_limiter {
                limiter.wait(bytes.len() as u64, cancel).await?;
            }
            file.write_all(&bytes).await?;
            *reported += bytes.len() as u64;
            (self.events)(DownloadEvent::Bytes {
                sha1: resource.sha1.clone(),
                bytes: bytes.len(),
            });
        }
        file.flush().await?;
        drop(file);
        ensure!(
            digests.matches(resource),
            "resource size, SHA-1 or SHA-512 mismatch"
        );
        Ok(temporary)
    }
}

fn verify_file(path: &Path, expected: &Resource) -> anyhow::Result<bool> {
    let Ok(mut file) = File::open(path) else {
        return Ok(false);
    };
    if file.metadata()?.len() != expected.size {
        return Ok(false);
    }
    let mut digests = ObjectDigests::new(expected);
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        digests.update(&buffer[..length]);
    }
    Ok(digests.matches(expected))
}
