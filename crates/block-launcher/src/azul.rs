//! Official Azul Zulu runtime fallback for ARM64 hosts.
//!
//! Mojang's `java-runtimes.json` catalog has no ARM64 entry for older Java
//! majors (for example the Java 8 `jre-legacy` component used by LWJGL 3-era
//! versions). The official Azul metadata API is then the only source for a
//! managed runtime. Package detail metadata supplies the archive checksum, so
//! every download is verified before it reaches the runtime store. Tests use a
//! loopback fixture server and never reach the network.

use anyhow::{Context as _, bail, ensure};
use block_download::Store;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::platform::{Architecture, OperatingSystem, Target};

/// Official Azul metadata v1 endpoint for package listings and details.
pub(crate) const AZUL_PACKAGES: &str = "https://api.azul.com/metadata/v1/zulu/packages/";

/// Size bound for a managed runtime archive; a Java runtime is far smaller.
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
/// A listing is normally one package; stop early instead of trusting a huge list.
const MAX_LISTED_PACKAGES: usize = 16;

/// Archive container published for a validated vendor runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

impl ArchiveKind {
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TarGz => "tar.gz",
            Self::Zip => "zip",
        }
    }

    const fn from_archive_type(value: &str) -> Option<Self> {
        match value.as_bytes() {
            b"tar.gz" => Some(Self::TarGz),
            b"zip" => Some(Self::Zip),
            _ => None,
        }
    }
}

/// A validated official Zulu runtime package selected for one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZuluRuntime {
    pub java_version: String,
    pub package: String,
    pub download_url: String,
    pub sha256: String,
    pub size: u64,
    pub archive: ArchiveKind,
}

impl ZuluRuntime {
    /// Explicit substitution description for the saved plan and launch log.
    #[must_use]
    pub fn note(&self, target: &Target, component: &str) -> String {
        format!(
            "Azul Zulu Java {} ({}) substituted for the missing official {} {} runtime; SHA-256 {} and package size from the official Azul metadata API ({})",
            self.java_version,
            self.package,
            target.runtime_platform(),
            component,
            self.sha256,
            AZUL_PACKAGES.trim_end_matches('/'),
        )
    }
}

/// Query mapping for the platforms where this fallback is supported.
fn query_platform(target: &Target) -> Option<(&'static str, &'static str, ArchiveKind)> {
    match (target.os, target.arch) {
        (OperatingSystem::Linux, Architecture::Aarch64) => {
            Some(("linux", "aarch64", ArchiveKind::TarGz))
        }
        (OperatingSystem::Windows, Architecture::Aarch64) => {
            Some(("windows", "aarch64", ArchiveKind::Zip))
        }
        _ => None,
    }
}

/// Whether the official Azul fallback covers this target.
#[must_use]
pub fn supports(target: &Target) -> bool {
    query_platform(target).is_some()
}

#[derive(Debug, Deserialize)]
struct PackageListEntry {
    name: String,
    package_uuid: String,
    java_version: Vec<u32>,
    distro_version: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct PackageDetail {
    name: String,
    download_url: String,
    sha256_hash: Option<String>,
    size: u64,
    archive_type: String,
    java_package_type: String,
    javafx_bundled: bool,
    lib_c_type: String,
    os: String,
    arch: String,
    hw_bitness: u32,
    release_status: String,
    availability_type: String,
    java_version: Vec<u32>,
}

/// Resolve the official Azul Zulu JRE for an ARM64 target and major version.
/// The returned package has not been downloaded yet.
///
/// # Errors
/// Fails when the target has no supported fallback, the metadata is missing or
/// malformed, no candidate satisfies the official filters, or the caller
/// cancels.
pub async fn resolve(
    store: &Store,
    target: &Target,
    java_major: u32,
    cancel: &CancellationToken,
) -> anyhow::Result<ZuluRuntime> {
    resolve_with_base(store, target, java_major, AZUL_PACKAGES, cancel).await
}

/// Download and sha256-verify the selected archive into the store.
///
/// # Errors
/// Fails on transport errors, a mismatched or missing checksum, a size that
/// differs from the official metadata, or cancellation.
pub async fn download(
    store: &Store,
    runtime: &ZuluRuntime,
    cancel: &CancellationToken,
) -> anyhow::Result<std::path::PathBuf> {
    ensure!(
        (1..=MAX_ARCHIVE_BYTES).contains(&runtime.size),
        "Azul runtime archive size is outside the supported bound"
    );
    let path = store
        .fetch_sha256_bounded(&runtime.download_url, &runtime.sha256, runtime.size, cancel)
        .await?;
    let actual = std::fs::metadata(&path)?.len();
    ensure!(
        actual == runtime.size,
        "Azul runtime archive size mismatch: expected {} bytes from the official metadata, downloaded {}",
        runtime.size,
        actual
    );
    Ok(path)
}

async fn resolve_with_base(
    store: &Store,
    target: &Target,
    java_major: u32,
    base: &str,
    cancel: &CancellationToken,
) -> anyhow::Result<ZuluRuntime> {
    if query_platform(target).is_none() {
        bail!(
            "official Azul runtime fallback is unavailable for {}",
            target.runtime_platform()
        );
    }
    tokio::select! {
        biased;
        () = cancel.cancelled() => bail!("runtime provisioning cancelled"),
        result = resolve_listing(store, target, java_major, base, cancel) => result,
    }
}

async fn resolve_listing(
    store: &Store,
    target: &Target,
    java_major: u32,
    base: &str,
    cancel: &CancellationToken,
) -> anyhow::Result<ZuluRuntime> {
    let Some((os, arch, expected_archive)) = query_platform(target) else {
        bail!(
            "official Azul runtime fallback is unavailable for {}",
            target.runtime_platform()
        );
    };
    let platform = format!("{os}/{arch}");
    let list_url = format!(
        "{base}?java_version={java_major}&os={os}&arch={arch}&java_package_type=jre&archive_type={}&release_status=ga&availability_types=CA&latest=true",
        expected_archive.as_str()
    );
    let list_name = format!("azul-zulu-{os}-{arch}-{java_major}.json");
    let bytes = store.metadata(&list_name, &list_url).await?;
    let mut listed: Vec<PackageListEntry> =
        serde_json::from_slice(&bytes).context("invalid Azul package list metadata")?;
    ensure!(
        !listed.is_empty(),
        "no official Azul Zulu Java {java_major} runtime for {platform}"
    );
    // Prefer the newest Java build, then the newest distribution build, so
    // selection does not depend on listing order.
    listed.sort_by(|left, right| {
        right
            .java_version
            .cmp(&left.java_version)
            .then_with(|| right.distro_version.cmp(&left.distro_version))
            .then_with(|| left.name.cmp(&right.name))
    });
    listed.truncate(MAX_LISTED_PACKAGES);

    let mut rejected = Vec::new();
    for entry in listed {
        ensure!(!cancel.is_cancelled(), "runtime provisioning cancelled");
        let detail_url = format!("{base}{}/", entry.package_uuid);
        let detail_name = format!("azul-zulu-{}.json", entry.package_uuid);
        let bytes = store.metadata(&detail_name, &detail_url).await?;
        let detail: PackageDetail =
            serde_json::from_slice(&bytes).context("invalid Azul package detail metadata")?;
        if let Err(reason) = validate_detail(&detail, java_major, os, expected_archive) {
            rejected.push(format!("{}: {reason}", entry.name));
            continue;
        }
        let sha256 = detail
            .sha256_hash
            .as_deref()
            .context("Azul package detail has no SHA-256 checksum")?
            .to_ascii_lowercase();
        return Ok(ZuluRuntime {
            java_version: format_java_version(&detail.java_version),
            package: detail.name,
            download_url: detail.download_url,
            sha256,
            size: detail.size,
            archive: expected_archive,
        });
    }
    bail!(
        "no usable official Azul Zulu Java {java_major} runtime for {platform} ({})",
        rejected.join("; ")
    )
}

fn validate_detail(
    detail: &PackageDetail,
    java_major: u32,
    os: &str,
    expected_archive: ArchiveKind,
) -> anyhow::Result<()> {
    ensure!(detail.os == os, "metadata OS is {}", detail.os);
    // The API reports aarch64 builds as arch "arm" with 64-bit bitness.
    ensure!(
        matches!(detail.arch.as_str(), "arm" | "aarch64") && detail.hw_bitness == 64,
        "metadata architecture is {}/{}",
        detail.arch,
        detail.hw_bitness
    );
    ensure!(
        detail.java_package_type == "jre",
        "package type is {}",
        detail.java_package_type
    );
    ensure!(!detail.javafx_bundled, "package bundles JavaFX");
    ensure!(
        detail.lib_c_type == "glibc",
        "libc is {}",
        detail.lib_c_type
    );
    ensure!(
        detail.release_status == "ga" && detail.availability_type == "CA",
        "release is {}/{}",
        detail.release_status,
        detail.availability_type
    );
    ensure!(
        ArchiveKind::from_archive_type(&detail.archive_type) == Some(expected_archive),
        "archive type is {}",
        detail.archive_type
    );
    ensure!(
        detail.java_version.first() == Some(&java_major),
        "Java version is {}",
        format_java_version(&detail.java_version)
    );
    let sha256 = detail
        .sha256_hash
        .as_deref()
        .context("Azul package detail has no SHA-256 checksum")?;
    ensure!(
        sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid SHA-256 checksum metadata"
    );
    ensure!(
        (1..=MAX_ARCHIVE_BYTES).contains(&detail.size),
        "archive size is outside the supported bound"
    );
    Ok(())
}

fn format_java_version(version: &[u32]) -> String {
    match version {
        [major, _minor, update] => format!("{major}u{update}"),
        _ => version
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("."),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use sha2::{Digest as _, Sha256};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    const LIST_PATH: &str = "/packages/";
    const PACKAGE_UUID: &str = "c0e1ff91-d8c6-4d28-afa8-8b3ee12cbae9";
    const FX_UUID: &str = "11111111-1111-4111-8111-111111111111";
    const ARCHIVE_NAME: &str = "zulu8.96.0.205-ca-jre8.0.504-linux_aarch64.tar.gz";
    const ARCHIVE_PATH: &str = "/files/zulu.tar.gz";

    struct Fixture {
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    struct Route {
        path: String,
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
    }

    fn route(path: &str, body: impl Into<Vec<u8>>) -> Route {
        Route {
            path: path.to_owned(),
            status: 200,
            content_type: "application/json",
            body: body.into(),
        }
    }

    fn archive_route(body: Vec<u8>) -> Route {
        Route {
            path: ARCHIVE_PATH.to_owned(),
            status: 200,
            content_type: "application/gzip",
            body,
        }
    }

    /// Start a loopback HTTP server; `build` receives the base URL so routes can
    /// reference it. Nothing here reaches the internet.
    fn fixture(build: impl FnOnce(&str) -> Vec<Route>) -> anyhow::Result<(String, Fixture)> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let base = format!("http://{address}");
        let routes = Arc::new(build(&base));
        let handle = tokio::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                return;
            };
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let routes = Arc::clone(&routes);
                tokio::spawn(async move {
                    let _ = serve(stream, &routes).await;
                });
            }
        });
        Ok((base, Fixture { handle }))
    }

    async fn serve(mut stream: tokio::net::TcpStream, routes: &[Route]) -> anyhow::Result<()> {
        let mut buffer = [0_u8; 8192];
        let count = stream.read(&mut buffer).await?;
        let request = String::from_utf8_lossy(&buffer[..count]);
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let path = target.split('?').next().unwrap_or("/");
        let response = routes.iter().find(|route| route.path == path);
        let (status, content_type, body) = response
            .map_or((404, "text/plain", b"not found".to_vec()), |route| {
                (route.status, route.content_type, route.body.clone())
            });
        let head = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&body).await?;
        stream.shutdown().await?;
        Ok(())
    }

    fn target() -> Target {
        Target {
            os: OperatingSystem::Linux,
            arch: Architecture::Aarch64,
            os_version: "6.1".into(),
        }
    }

    fn store() -> anyhow::Result<(tempfile::TempDir, Store)> {
        let directory = tempfile::tempdir()?;
        let store = Store::new(directory.path().to_owned(), false, Arc::new(|_| ()))?;
        Ok((directory, store))
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Small valid runtime archive shaped like the official Zulu tarball.
    fn runtime_archive() -> anyhow::Result<Vec<u8>> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let java = b"#!/bin/sh\nexit 0\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(java.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, "zulu-test/bin/java", java.as_slice())?;
        let release = b"JAVA_VERSION=\"1.8.0_504\"\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(release.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, "zulu-test/release", release.as_slice())?;
        let encoder = builder.into_inner()?;
        Ok(encoder.finish()?)
    }

    fn list_body() -> String {
        json!([
            {
                "name": "zulu8.96.0.205-ca-fx-jre8.0.504-linux_aarch64.tar.gz",
                "package_uuid": FX_UUID,
                "java_version": [8, 0, 504],
                "distro_version": [8, 96, 0, 205],
                "latest": true
            },
            {
                "name": ARCHIVE_NAME,
                "package_uuid": PACKAGE_UUID,
                "java_version": [8, 0, 504],
                "distro_version": [8, 96, 0, 205],
                "latest": true
            }
        ])
        .to_string()
    }

    fn detail_body(
        name: &str,
        sha256: &str,
        javafx: bool,
        size: u64,
        download_url: &str,
    ) -> String {
        json!({
            "name": name,
            "download_url": download_url,
            "sha256_hash": sha256,
            "size": size,
            "archive_type": "tar.gz",
            "java_package_type": "jre",
            "javafx_bundled": javafx,
            "lib_c_type": "glibc",
            "os": "linux",
            "arch": "arm",
            "hw_bitness": 64,
            "release_status": "ga",
            "availability_type": "CA",
            "java_version": [8, 0, 504]
        })
        .to_string()
    }

    #[tokio::test]
    async fn official_plain_jre_is_selected_and_checksum_verified() -> anyhow::Result<()> {
        let archive = runtime_archive()?;
        let digest = sha256(&archive);
        let (base, _fixture) = fixture(|base| {
            let archive_url = format!("{base}{ARCHIVE_PATH}");
            vec![
                route(LIST_PATH, list_body()),
                route(
                    &format!("/packages/{FX_UUID}/"),
                    detail_body(
                        "zulu8.96.0.205-ca-fx-jre8.0.504-linux_aarch64.tar.gz",
                        &digest,
                        true,
                        archive.len() as u64,
                        &archive_url,
                    ),
                ),
                route(
                    &format!("/packages/{PACKAGE_UUID}/"),
                    detail_body(
                        ARCHIVE_NAME,
                        &digest,
                        false,
                        archive.len() as u64,
                        &archive_url,
                    ),
                ),
                archive_route(archive.clone()),
            ]
        })?;
        let (_directory, store) = store()?;
        let cancel = CancellationToken::new();
        let runtime =
            resolve_with_base(&store, &target(), 8, &format!("{base}{LIST_PATH}"), &cancel).await?;
        // The JavaFX bundle is listed first but must be skipped.
        assert_eq!(runtime.package, ARCHIVE_NAME);
        assert_eq!(runtime.sha256, digest);
        assert_eq!(runtime.archive, ArchiveKind::TarGz);
        assert_eq!(runtime.java_version, "8u504");
        assert!(runtime.download_url.ends_with(ARCHIVE_PATH));
        let path = download(&store, &runtime, &cancel).await?;
        assert!(path.is_file());
        assert_eq!(
            std::fs::metadata(&path)?.len(),
            u64::try_from(archive.len())?
        );
        Ok(())
    }

    #[tokio::test]
    async fn checksum_mismatch_is_rejected() -> anyhow::Result<()> {
        let archive = runtime_archive()?;
        let wrong = "a".repeat(64);
        let (base, _fixture) = fixture(|base| {
            let archive_url = format!("{base}{ARCHIVE_PATH}");
            vec![
                route(LIST_PATH, list_body()),
                route(
                    &format!("/packages/{FX_UUID}/"),
                    detail_body("fx", &wrong, true, 1, &archive_url),
                ),
                route(
                    &format!("/packages/{PACKAGE_UUID}/"),
                    detail_body(
                        ARCHIVE_NAME,
                        &wrong,
                        false,
                        archive.len() as u64,
                        &archive_url,
                    ),
                ),
                archive_route(archive.clone()),
            ]
        })?;
        let (_directory, store) = store()?;
        let cancel = CancellationToken::new();
        let runtime =
            resolve_with_base(&store, &target(), 8, &format!("{base}{LIST_PATH}"), &cancel).await?;
        let error = download(&store, &runtime, &cancel)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("mismatched checksum must fail"))?;
        assert!(format!("{error:#}").contains("SHA-256"), "{error:#}");
        Ok(())
    }

    #[tokio::test]
    async fn empty_catalog_reports_missing_package() -> anyhow::Result<()> {
        let (base, _fixture) = fixture(|_base| vec![route(LIST_PATH, "[]")])?;
        let (_directory, store) = store()?;
        let cancel = CancellationToken::new();
        let error = resolve_with_base(&store, &target(), 8, &format!("{base}{LIST_PATH}"), &cancel)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("missing package must fail"))?;
        assert!(
            format!("{error:#}").contains("no official Azul Zulu Java 8 runtime"),
            "{error:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn package_detail_404_is_reported() -> anyhow::Result<()> {
        let (base, _fixture) = fixture(|_base| vec![route(LIST_PATH, list_body())])?;
        let (_directory, store) = store()?;
        let cancel = CancellationToken::new();
        let error = resolve_with_base(&store, &target(), 8, &format!("{base}{LIST_PATH}"), &cancel)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("missing detail must fail"))?;
        assert!(format!("{error:#}").contains("404"), "{error:#}");
        Ok(())
    }

    #[tokio::test]
    async fn malformed_metadata_is_rejected() -> anyhow::Result<()> {
        let (base, _fixture) = fixture(|_base| vec![route(LIST_PATH, "{not json")])?;
        let (_directory, store) = store()?;
        let cancel = CancellationToken::new();
        let error = resolve_with_base(&store, &target(), 8, &format!("{base}{LIST_PATH}"), &cancel)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("malformed metadata must fail"))?;
        assert!(
            format!("{error:#}").contains("invalid Azul package list metadata"),
            "{error:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_stops_provisioning() -> anyhow::Result<()> {
        let (base, _fixture) = fixture(|_base| vec![route(LIST_PATH, list_body())])?;
        let (_directory, store) = store()?;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = resolve_with_base(&store, &target(), 8, &format!("{base}{LIST_PATH}"), &cancel)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("cancelled resolution must fail"))?;
        assert!(format!("{error:#}").contains("cancelled"), "{error:#}");
        Ok(())
    }

    #[test]
    fn unsupported_targets_have_no_vendor_fallback() {
        assert!(!supports(&Target {
            os: OperatingSystem::Linux,
            arch: Architecture::X86_64,
            os_version: String::new(),
        }));
        assert!(supports(&Target {
            os: OperatingSystem::Windows,
            arch: Architecture::Aarch64,
            os_version: String::new(),
        }));
    }
}
