//! Account skins and provider icons: bounded fetches, validated images and a
//! data-directory cache. Nothing here decodes an image for pixels; it validates
//! the bytes the UI later hands to the renderer, so a corrupt cache entry or a
//! hostile response falls back to initials or a monogram instead of a broken image.

use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, ensure};
use flate2::read::ZlibDecoder;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

/// Skins are tiny PNG files; anything larger is not a texture we accept.
pub const MAX_SKIN_BYTES: usize = 512 * 1024;
/// Provider icons are bounded separately from skins.
pub const MAX_ICON_BYTES: usize = 256 * 1024;

const MAX_ICON_DIMENSION: u32 = 1024;

/// A validated skin PNG, kept as the source image so the UI can crop the face
/// and hat layers without a decoder of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkinImage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A validated provider icon in a format the UI can render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Svg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderIcon {
    pub bytes: Vec<u8>,
    pub kind: ImageKind,
}

/// Validate a Minecraft skin PNG and its face-compatible dimensions.
///
/// # Errors
/// Rejects oversized files, malformed chunks or CRCs, unsupported PNG layouts
/// and images that cannot hold the 64-unit skin grid.
pub fn validate_skin(bytes: &[u8]) -> anyhow::Result<SkinImage> {
    ensure!(
        bytes.len() <= MAX_SKIN_BYTES,
        "skin image exceeds the size limit"
    );
    let decoded = decode_skin_png(bytes)?;
    Ok(SkinImage {
        bytes: bytes.to_vec(),
        width: decoded.0,
        height: decoded.1,
    })
}

fn decode_skin_png(bytes: &[u8]) -> anyhow::Result<(u32, u32)> {
    let (width, height, compressed) = png_chunks(bytes, true)?;
    ensure!(
        (64..=MAX_ICON_DIMENSION).contains(&width)
            && (32..=MAX_ICON_DIMENSION).contains(&height)
            && width % 64 == 0
            && (height == width || height * 2 == width),
        "skin image does not use a Minecraft skin layout"
    );
    let expected = usize::try_from(
        height
            .checked_mul(1 + width * 4)
            .context("skin image is too large")?,
    )
    .context("skin image is too large")?;
    let mut inflated = Vec::with_capacity(expected.min(MAX_SKIN_BYTES));
    ZlibDecoder::new(compressed.as_slice())
        .take((expected + 1) as u64)
        .read_to_end(&mut inflated)
        .context("skin image data is corrupt")?;
    ensure!(inflated.len() == expected, "skin image data is truncated");
    for row in inflated.chunks_exact(1 + usize::try_from(width).context("skin width")? * 4) {
        ensure!(row[0] <= 4, "skin image uses an invalid row filter");
    }
    let _ = compressed;
    Ok((width, height))
}

/// Validate a provider icon by its real bytes, not its URL or declared type.
///
/// # Errors
/// Rejects oversized files and formats the UI cannot render.
pub fn validate_icon(bytes: &[u8]) -> anyhow::Result<ProviderIcon> {
    ensure!(
        bytes.len() <= MAX_ICON_BYTES,
        "provider icon exceeds the size limit"
    );
    let kind = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let (width, height, _) = png_chunks(bytes, false)?;
        ensure!(
            (1..=MAX_ICON_DIMENSION).contains(&width) && (1..=MAX_ICON_DIMENSION).contains(&height),
            "provider icon dimensions are out of range"
        );
        ImageKind::Png
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        ensure!(
            bytes.ends_with(&[0xff, 0xd9])
                || bytes.windows(2).rev().any(|pair| pair == [0xff, 0xd9]),
            "provider icon JPEG is truncated"
        );
        ImageKind::Jpeg
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ensure!(bytes.len() > 13, "provider icon GIF is truncated");
        ImageKind::Gif
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageKind::Webp
    } else if bytes.starts_with(b"BM") && bytes.len() > 26 {
        ImageKind::Bmp
    } else if looks_like_svg(bytes) {
        ImageKind::Svg
    } else {
        anyhow::bail!("provider icon is not a supported image type");
    };
    Ok(ProviderIcon {
        bytes: bytes.to_vec(),
        kind,
    })
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    let text = text.trim_start();
    (text.starts_with("<svg") || text.starts_with("<?xml"))
        && text.to_ascii_lowercase().contains("<svg")
}

/// Walk PNG chunks, verifying lengths and CRCs, and return the RGBA8 header
/// plus the compressed image data. Ancillary chunks are allowed.
fn png_chunks(bytes: &[u8], rgba_only: bool) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    ensure!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image is not a PNG"
    );
    let mut offset = 8;
    let mut width = 0_u32;
    let mut height = 0_u32;
    let mut saw_header = false;
    let mut saw_end = false;
    let mut compressed = Vec::new();
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .context("PNG chunk length")?,
        );
        let length = usize::try_from(length).context("PNG chunk length")?;
        let end = offset
            .checked_add(12)
            .and_then(|start| start.checked_add(length))
            .context("PNG chunk length")?;
        ensure!(end <= bytes.len(), "PNG chunk is truncated");
        let kind = &bytes[offset + 4..offset + 8];
        let data = &bytes[offset + 8..offset + 8 + length];
        let expected = u32::from_be_bytes(
            bytes[offset + 8 + length..end]
                .try_into()
                .context("PNG chunk checksum")?,
        );
        let mut crc_input = Vec::with_capacity(4 + length);
        crc_input.extend_from_slice(kind);
        crc_input.extend_from_slice(data);
        ensure!(
            crc32(&crc_input) == expected,
            "PNG chunk checksum does not match"
        );
        match kind {
            b"IHDR" => {
                ensure!(!saw_header && length == 13, "PNG header is invalid");
                width = u32::from_be_bytes(data[0..4].try_into().context("PNG width")?);
                height = u32::from_be_bytes(data[4..8].try_into().context("PNG height")?);
                ensure!(width > 0 && height > 0, "PNG dimensions are invalid");
                if rgba_only {
                    ensure!(
                        data[8] == 8
                            && data[9] == 6
                            && data[10] == 0
                            && data[11] == 0
                            && data[12] == 0,
                        "skin image must be non-interlaced RGBA8"
                    );
                }
                saw_header = true;
            }
            b"IDAT" => {
                ensure!(saw_header, "PNG image data precedes its header");
                compressed.extend_from_slice(data);
                ensure!(
                    compressed.len() <= MAX_SKIN_BYTES * 4,
                    "PNG image data is too large"
                );
            }
            b"IEND" => {
                ensure!(saw_header, "PNG end precedes its header");
                saw_end = true;
                break;
            }
            _ => {}
        }
        offset = end;
    }
    ensure!(saw_header, "PNG header is missing");
    ensure!(saw_end, "PNG end chunk is missing");
    ensure!(!compressed.is_empty(), "PNG image data is missing");
    Ok((width, height, compressed))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Split a URL into its host when it is an HTTP(S) URL without credentials.
#[must_use]
pub fn url_host(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = if let Some((inner, port)) = authority.rsplit_once(':') {
        if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) {
            inner
        } else {
            authority
        }
    } else {
        authority
    };
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

/// Whether a host is declared by a provider's skin domain list. Exact entries
/// match one host; entries starting with `.` or `*.` also match subdomains.
#[must_use]
pub fn declared_host(host: &str, domains: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    domains.iter().any(|domain| {
        let domain = domain.trim_end_matches('.').to_ascii_lowercase();
        if let Some(suffix) = domain.strip_prefix("*.") {
            return host == suffix || host.ends_with(&format!(".{suffix}"));
        }
        if let Some(suffix) = domain.strip_prefix('.') {
            return host == suffix || host.ends_with(&format!(".{suffix}"));
        }
        host == domain
    })
}

/// Whether a skin URL is on a declared host and uses HTTP(S) without credentials.
#[must_use]
pub fn skin_url_allowed(url: &str, domains: &[String]) -> bool {
    url_host(url).is_some_and(|host| declared_host(&host, domains))
}

/// Validate an icon URL against the provider API host it was discovered from.
#[must_use]
pub fn icon_url_allowed(url: &str, api_url: &str) -> bool {
    let Some(host) = url_host(url) else {
        return false;
    };
    let Some(api_host) = url_host(api_url) else {
        return false;
    };
    declared_host(&host, std::slice::from_ref(&api_host))
}

/// Bounded HTTP client for skins and provider icons.
#[derive(Clone)]
pub struct ImageFetch {
    client: reqwest::Client,
}

impl std::fmt::Debug for ImageFetch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ImageFetch").finish_non_exhaustive()
    }
}

impl ImageFetch {
    /// # Errors
    /// Reports TLS client initialization failures.
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .user_agent("Block/0.1 (account images)")
            .build()
            .context("image HTTP client initialization")?;
        Ok(Self { client })
    }

    /// Fetch and validate a skin. The URL must be on a declared skin host.
    ///
    /// # Errors
    /// Rejects undeclared hosts, non-2xx responses, oversized bodies and invalid skins.
    pub async fn skin(
        &self,
        url: &str,
        domains: &[String],
        cancel: &CancellationToken,
    ) -> anyhow::Result<SkinImage> {
        ensure!(skin_url_allowed(url, domains), "skin host is not declared");
        let bytes = self.bounded(url, MAX_SKIN_BYTES, cancel).await?;
        validate_skin(&bytes)
    }

    /// Fetch and validate a provider icon. Redirects are not followed, so the
    /// host checked here is the host that served the bytes.
    ///
    /// # Errors
    /// Rejects undeclared hosts, non-2xx responses, oversized bodies and invalid icons.
    pub async fn icon(
        &self,
        url: &str,
        api_url: &str,
        cancel: &CancellationToken,
    ) -> anyhow::Result<ProviderIcon> {
        ensure!(icon_url_allowed(url, api_url), "icon host is not declared");
        let bytes = self.bounded(url, MAX_ICON_BYTES, cancel).await?;
        validate_icon(&bytes)
    }

    async fn bounded(
        &self,
        url: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Vec<u8>> {
        let request = self.client.get(url);
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => anyhow::bail!("image request cancelled"),
            result = request.send() => result.context("image request failed")?,
        };
        ensure!(
            response.status().is_success(),
            "image request returned an error status"
        );
        if let Some(length) = response.content_length() {
            ensure!(
                length <= limit as u64,
                "image response exceeds the size limit"
            );
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                biased;
                () = cancel.cancelled() => anyhow::bail!("image request cancelled"),
                item = stream.next() => item,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk.context("image response failed")?;
            ensure!(
                body.len() + chunk.len() <= limit,
                "image response exceeds the size limit"
            );
            body.extend_from_slice(&chunk);
        }
        ensure!(!body.is_empty(), "image response is empty");
        Ok(body)
    }
}

#[derive(Deserialize, Serialize)]
struct CacheSidecar {
    source: String,
    sha256: String,
    width: u32,
    height: u32,
}

/// Data-directory cache for validated skins and provider icons.
///
/// Entries are keyed by the stable identity and refreshed when the source URL
/// changes. Every read verifies the recorded digest and the image structure, so
/// a corrupt or replaced file is deleted and treated as a miss.
#[derive(Clone, Debug)]
pub struct ImageCache {
    root: PathBuf,
}

impl ImageCache {
    #[must_use]
    pub fn new(data_root: &Path) -> Self {
        Self {
            root: data_root.join("images"),
        }
    }

    fn directory(&self, kind: &str) -> PathBuf {
        self.root.join(kind)
    }

    fn key(identity: &str) -> String {
        let digest = Sha256::digest(identity.as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Cached skin for this identity and source URL, or `None` on any mismatch.
    #[must_use]
    pub fn skin(&self, identity: &str, source: &str) -> Option<SkinImage> {
        let key = Self::key(identity);
        let directory = self.directory("skins");
        let sidecar: CacheSidecar = read_json(&directory.join(format!("{key}.json")))?;
        if sidecar.source != source {
            return None;
        }
        let bytes = std::fs::read(directory.join(format!("{key}.png"))).ok()?;
        if !digest_matches(&bytes, &sidecar.sha256) {
            self.remove("skins", &key);
            return None;
        }
        match validate_skin(&bytes) {
            Ok(image) if image.width == sidecar.width && image.height == sidecar.height => {
                Some(image)
            }
            _ => {
                self.remove("skins", &key);
                None
            }
        }
    }

    /// # Errors
    /// Reports filesystem failures after the image was already validated.
    pub fn store_skin(
        &self,
        identity: &str,
        source: &str,
        image: &SkinImage,
    ) -> anyhow::Result<()> {
        let validated = validate_skin(&image.bytes)?;
        ensure!(
            validated.width == image.width && validated.height == image.height,
            "skin dimensions changed while caching"
        );
        let key = Self::key(identity);
        let sidecar = CacheSidecar {
            source: source.to_owned(),
            sha256: hex_digest(&image.bytes),
            width: image.width,
            height: image.height,
        };
        write_entry(
            &self.directory("skins"),
            &key,
            "png",
            &image.bytes,
            &sidecar,
        )
    }

    /// Cached icon for this provider and source URL, or `None` on any mismatch.
    #[must_use]
    pub fn icon(&self, provider: &str, source: &str) -> Option<ProviderIcon> {
        let key = Self::key(provider);
        let directory = self.directory("icons");
        let sidecar: CacheSidecar = read_json(&directory.join(format!("{key}.json")))?;
        if sidecar.source != source {
            return None;
        }
        let bytes = std::fs::read(directory.join(format!("{key}.img"))).ok()?;
        if !digest_matches(&bytes, &sidecar.sha256) {
            self.remove("icons", &key);
            return None;
        }
        match validate_icon(&bytes) {
            Ok(icon) => Some(icon),
            Err(_) => {
                self.remove("icons", &key);
                None
            }
        }
    }

    /// # Errors
    /// Reports filesystem failures after the icon was already validated.
    pub fn store_icon(
        &self,
        provider: &str,
        source: &str,
        icon: &ProviderIcon,
    ) -> anyhow::Result<()> {
        validate_icon(&icon.bytes)?;
        let key = Self::key(provider);
        let sidecar = CacheSidecar {
            source: source.to_owned(),
            sha256: hex_digest(&icon.bytes),
            width: 0,
            height: 0,
        };
        write_entry(&self.directory("icons"), &key, "img", &icon.bytes, &sidecar)
    }

    fn remove(&self, kind: &str, key: &str) {
        let directory = self.directory(kind);
        let _ = std::fs::remove_file(directory.join(format!("{key}.json")));
        for extension in ["png", "img"] {
            let _ = std::fs::remove_file(directory.join(format!("{key}.{extension}")));
        }
    }
}

fn digest_matches(bytes: &[u8], expected: &str) -> bool {
    hex_digest(bytes) == expected
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_entry(
    directory: &Path,
    key: &str,
    extension: &str,
    bytes: &[u8],
    sidecar: &CacheSidecar,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(directory)?;
    let image_path = directory.join(format!("{key}.{extension}"));
    let temporary = directory.join(format!("{key}.{extension}.tmp"));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, &image_path)?;
    let sidecar_path = directory.join(format!("{key}.json"));
    let sidecar_temporary = directory.join(format!("{key}.json.tmp"));
    std::fs::write(&sidecar_temporary, serde_json::to_vec(sidecar)?)?;
    std::fs::rename(&sidecar_temporary, &sidecar_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write as _;

    fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = Vec::new();
        crc_input.extend_from_slice(kind);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        out
    }

    fn skin_png(width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
        let mut header = Vec::new();
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        let row = 1 + width as usize * 4;
        let raw = vec![0_u8; row * height as usize];
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&raw)?;
        let compressed = encoder.finish()?;
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&chunk(b"IHDR", &header));
        out.extend_from_slice(&chunk(b"IDAT", &compressed));
        out.extend_from_slice(&chunk(b"IEND", &[]));
        Ok(out)
    }

    #[test]
    fn skin_validation_accepts_the_minecraft_grids() -> anyhow::Result<()> {
        for (width, height) in [(64, 64), (64, 32), (128, 128)] {
            let image = validate_skin(&skin_png(width, height)?)?;
            assert_eq!((image.width, image.height), (width, height));
        }
        assert!(validate_skin(&skin_png(63, 64)?).is_err());
        assert!(validate_skin(&skin_png(64, 48)?).is_err());
        assert!(validate_skin(b"not a png").is_err());
        Ok(())
    }

    #[test]
    fn skin_validation_rejects_corruption_and_oversize() -> anyhow::Result<()> {
        let mut png = skin_png(64, 64)?;
        png[40] ^= 0xff;
        assert!(validate_skin(&png).is_err());
        let mut huge = skin_png(64, 64)?;
        huge.resize(MAX_SKIN_BYTES + 1, 0);
        assert!(validate_skin(&huge).is_err());
        Ok(())
    }

    #[test]
    fn domain_matching_follows_the_declared_forms() {
        let domains = vec!["textures.example.test".into(), ".example.test".into()];
        assert!(declared_host("textures.example.test", &domains));
        assert!(declared_host("skins.example.test", &domains));
        assert!(!declared_host("evil.example.net", &domains));
        let wildcard = vec!["*.skins.test".into()];
        assert!(declared_host("textures.skins.test", &wildcard));
        assert!(declared_host("skins.test", &wildcard));
        assert!(skin_url_allowed(
            "https://textures.example.test/a.png",
            &domains
        ));
        assert!(!skin_url_allowed(
            "https://evil.example.net/a.png",
            &domains
        ));
        assert!(!skin_url_allowed(
            "https://user:pass@textures.example.test/a.png",
            &domains
        ));
        assert!(icon_url_allowed(
            "https://api.example.test/favicon.ico",
            "https://api.example.test/"
        ));
        assert!(!icon_url_allowed(
            "https://evil.example.net/favicon.ico",
            "https://api.example.test/"
        ));
    }

    #[test]
    fn cache_round_trips_and_rejects_corruption() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let cache = ImageCache::new(directory.path());
        let image = validate_skin(&skin_png(64, 64)?)?;
        cache.store_skin("account-one", "https://textures.example.test/skin", &image)?;
        assert_eq!(
            cache.skin("account-one", "https://textures.example.test/skin"),
            Some(image.clone())
        );
        assert_eq!(
            cache.skin("account-one", "https://textures.example.test/other"),
            None
        );
        assert_eq!(
            cache.skin("account-two", "https://textures.example.test/skin"),
            None
        );
        let key = ImageCache::key("account-one");
        let path = directory
            .path()
            .join("images")
            .join("skins")
            .join(format!("{key}.png"));
        let mut bytes = std::fs::read(&path)?;
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, bytes)?;
        assert_eq!(
            cache.skin("account-one", "https://textures.example.test/skin"),
            None
        );
        assert!(!path.exists(), "a corrupt cache entry is removed");
        Ok(())
    }

    #[test]
    fn icon_validation_accepts_images_and_rejects_markup() -> anyhow::Result<()> {
        let png = skin_png(64, 64)?;
        assert_eq!(validate_icon(&png)?.kind, ImageKind::Png);
        assert!(validate_icon(b"<html>not an image</html>").is_err());
        assert!(validate_icon(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").is_ok());
        let mut huge = png;
        huge.resize(MAX_ICON_BYTES + 1, 0);
        assert!(validate_icon(&huge).is_err());
        Ok(())
    }

    async fn fixture_server(
        status: &'static str,
        body: Vec<u8>,
    ) -> std::io::Result<(String, tokio::task::JoinHandle<()>)> {
        use tokio::{
            io::{AsyncReadExt as _, AsyncWriteExt as _},
            net::TcpListener,
        };
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0_u8; 2048];
                let _ = stream.read(&mut buffer).await;
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: image/png\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });
        Ok((format!("http://{address}"), task))
    }

    #[tokio::test]
    async fn fetch_validates_bodies_and_hosts() -> anyhow::Result<()> {
        let png = skin_png(64, 64)?;
        let (base, task) = fixture_server("200 OK", png.clone()).await?;
        let domains = vec!["127.0.0.1".to_owned()];
        let fetch = ImageFetch::new()?;
        let image = fetch
            .skin(
                &format!("{base}/skin.png"),
                &domains,
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!((image.width, image.height), (64, 64));
        assert!(
            fetch
                .skin(&format!("{base}/skin.png"), &[], &CancellationToken::new())
                .await
                .is_err(),
            "an undeclared host is rejected before the request"
        );
        task.await?;
        let (base, task) = fixture_server("200 OK", b"<html>no</html>".to_vec()).await?;
        assert!(
            fetch
                .skin(
                    &format!("{base}/skin.png"),
                    &domains,
                    &CancellationToken::new()
                )
                .await
                .is_err()
        );
        task.await?;
        let (base, task) = fixture_server("500 Internal Server Error", png.clone()).await?;
        assert!(
            fetch
                .skin(
                    &format!("{base}/skin.png"),
                    &domains,
                    &CancellationToken::new()
                )
                .await
                .is_err()
        );
        task.await?;
        Ok(())
    }

    #[tokio::test]
    async fn fetch_icons_use_the_api_host_allowlist() -> anyhow::Result<()> {
        let png = skin_png(64, 64)?;
        let (base, task) = fixture_server("200 OK", png).await?;
        let fetch = ImageFetch::new()?;
        let icon = fetch
            .icon(
                &format!("{base}/favicon.ico"),
                &format!("{base}/"),
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!(icon.kind, ImageKind::Png);
        task.await?;
        Ok(())
    }
}
