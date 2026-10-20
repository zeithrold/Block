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
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

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
    Started { sha1: String, size: u64 },
    Bytes { sha1: String, bytes: usize },
    Complete { sha1: String, cached: bool },
    Retry { url: String, attempt: u32 },
}

pub type EventSink = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

/// All persistent data belongs under the caller's data directory.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    client: reqwest::Client,
    events: EventSink,
    offline: bool,
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
    /// Create a reusable HTTP client and cache.
    ///
    /// # Errors
    /// Returns errors configuring HTTP/TLS or creating the data directory.
    pub fn new(root: PathBuf, offline: bool, events: EventSink) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            client: reqwest::Client::builder()
                .user_agent("Block/0.1 Minecraft launcher")
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(180))
                .build()?,
            events,
            offline,
        })
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
        let response = self.client.head(url).send().await?.error_for_status()?;
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
        let response = self.client.get(url).send().await?.error_for_status()?;
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
            .buffer_unordered(8)
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
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
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
        (self.events)(DownloadEvent::Started {
            sha1: resource.sha1.clone(),
            size: resource.size,
        });
        for attempt in 0..3 {
            let result = tokio::select! {
                () = cancel.cancelled() => bail!("download cancelled"),
                result = self.transfer(resource, directory) => result,
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
                    if permanent || attempt == 2 {
                        return Err(error).with_context(|| resource.url.clone());
                    }
                    (self.events)(DownloadEvent::Retry {
                        url: resource.url.clone(),
                        attempt: attempt + 1,
                    });
                    tokio::select! { () = cancel.cancelled() => bail!("download cancelled"), () = tokio::time::sleep(Duration::from_secs(u64::from(attempt + 1))) => {} }
                }
            }
        }
        bail!("download attempts exhausted")
    }

    async fn transfer(
        &self,
        resource: &Resource,
        directory: &Path,
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
