//! Streaming, content-addressed downloads with atomic verified publication.

use std::{
    collections::BTreeMap,
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
use sha2::Sha256;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

/// Explicit proxy behavior for all HTTP requests made by a [`Store`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxyMode {
    /// Preserve reqwest's environment/system proxy discovery.
    #[default]
    System,
    /// Bypass proxies, including environment proxy variables.
    Direct,
    /// Use an HTTP or HTTPS proxy. Authentication and SOCKS are not supported.
    Http { url: String },
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
            "download concurrency must be 1..=16"
        );
        ensure!(self.max_retries <= 10, "download retries must be 0..=10");
        ensure!(
            self.bytes_per_second != Some(0),
            "download rate must be positive or unlimited"
        );
        if let ProxyMode::Http { url } = &self.proxy {
            let proxy = reqwest::Url::parse(url).context("invalid proxy URL")?;
            ensure!(
                matches!(proxy.scheme(), "http" | "https"),
                "proxy scheme must be http or https"
            );
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

/// An upstream-verified object. SHA-1 is mandated by Minecraft metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub sha1: String,
    pub size: u64,
    pub url: String,
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
        url: String,
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
            ProxyMode::Http { url } => client.no_proxy().proxy(reqwest::Proxy::all(url)?),
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
        self.object_path(&sha1)?;
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
            sha1,
            size,
            url: url.into(),
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
        ensure!(
            sha1.len() == 40
                && sha1
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid SHA-1: {sha1}"
        );
        Ok(self.root.join("objects").join(&sha1[..2]).join(sha1))
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
    /// # Errors
    /// Fails on conflicting metadata, download/integrity failures, or cancellation.
    pub async fn fetch_all(
        &self,
        resources: &[Resource],
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        let mut unique = BTreeMap::new();
        for resource in resources {
            if let Some(previous) = unique.insert(&resource.sha1, resource) {
                ensure!(
                    previous.size == resource.size,
                    "conflicting sizes for {}",
                    resource.sha1
                );
            }
        }
        let resources: Vec<Resource> = unique.into_values().cloned().collect();
        futures_util::stream::iter(resources)
            .map(|resource| async move { self.fetch(&resource, cancel).await })
            .buffer_unordered(self.options.max_concurrent)
            .try_collect::<Vec<_>>()
            .await?;
        Ok(())
    }

    /// Download and verify one object. Concurrent processes share an OS file lock.
    ///
    /// # Errors
    /// Fails on a corrupt object, missing offline resource, HTTP/IO, or cancellation.
    pub async fn fetch(
        &self,
        resource: &Resource,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PathBuf> {
        ensure!(!cancel.is_cancelled(), "download cancelled");
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
        ensure!(
            !self.offline,
            "missing or corrupt offline resource: {} ({})",
            resource.sha1,
            resource.url
        );
        let _permit = tokio::select! {
            () = cancel.cancelled() => bail!("download cancelled"),
            permit = self.transfers.acquire() => permit?,
        };
        (self.events)(DownloadEvent::Started {
            sha1: resource.sha1.clone(),
            size: resource.size,
        });
        for attempt in 0..=self.options.max_retries {
            let result = tokio::select! {
                () = cancel.cancelled() => bail!("download cancelled"),
                result = self.transfer(resource, directory, cancel) => result,
            };
            match result {
                Ok(temporary) => {
                    temporary.persist(&path)?;
                    (self.events)(DownloadEvent::Complete {
                        sha1: resource.sha1.clone(),
                        cached: false,
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
                        return Err(error).with_context(|| resource.url.clone());
                    }
                    (self.events)(DownloadEvent::Retry {
                        sha1: resource.sha1.clone(),
                        url: resource.url.clone(),
                        attempt: attempt + 1,
                    });
                    tokio::select! { () = cancel.cancelled() => bail!("download cancelled"), () = tokio::time::sleep(Duration::from_secs(u64::from(attempt + 1))) => {} }
                }
            }
        }
        bail!("download attempts exhausted")
    }

    /// Fetch an upstream SHA-256 artifact with bounded size and the store's
    /// proxy, concurrency, retry and aggregate rate policies. Only a verified
    /// temporary file is published; cancellation never replaces the destination.
    pub async fn fetch_sha256(
        &self,
        url: &str,
        digest: &str,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PathBuf> {
        const MAX_SIZE: u64 = 16 * 1024 * 1024;
        ensure!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid SHA-256 digest"
        );
        ensure!(!cancel.is_cancelled(), "download cancelled");
        let directory = self.root.join("objects-sha256").join(&digest[..2]);
        tokio::fs::create_dir_all(&directory).await?;
        let path = directory.join(format!("{digest}.jar"));
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
            if size == 0 || size > MAX_SIZE {
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
                    total.is_none_or(|size| size > 0 && size <= MAX_SIZE),
                    "SHA-256 artifact exceeds the size limit"
                );
                let temporary = tempfile::NamedTempFile::new_in(&directory)?;
                let mut file = tokio::fs::File::from_std(temporary.reopen()?);
                let mut stream = response.bytes_stream();
                let mut hash = Sha256::new();
                while let Some(bytes) = stream.try_next().await? {
                    received = received.saturating_add(bytes.len() as u64);
                    ensure!(
                        received <= MAX_SIZE,
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

    async fn transfer(
        &self,
        resource: &Resource,
        directory: &Path,
        cancel: &CancellationToken,
    ) -> anyhow::Result<tempfile::NamedTempFile> {
        let response = self
            .client
            .get(&resource.url)
            .send()
            .await?
            .error_for_status()?;
        let temporary = tempfile::NamedTempFile::new_in(directory)?;
        let mut file = tokio::fs::File::from_std(temporary.reopen()?);
        let mut stream = response.bytes_stream();
        let mut hasher = Sha1::new();
        let mut received = 0_u64;
        while let Some(bytes) = stream.try_next().await? {
            received += bytes.len() as u64;
            ensure!(received <= resource.size, "resource exceeds declared size");
            if let Some(limiter) = &self.rate_limiter {
                limiter.wait(bytes.len() as u64, cancel).await?;
            }
            hasher.update(&bytes);
            file.write_all(&bytes).await?;
            (self.events)(DownloadEvent::Bytes {
                sha1: resource.sha1.clone(),
                bytes: bytes.len(),
            });
        }
        file.flush().await?;
        drop(file);
        ensure!(
            received == resource.size && format!("{:x}", hasher.finalize()) == resource.sha1,
            "resource size or SHA-1 mismatch"
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
    let mut hasher = Sha1::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        hasher.update(&buffer[..length]);
    }
    Ok(format!("{:x}", hasher.finalize()) == expected.sha1)
}
