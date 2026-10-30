//! Fetch or audit all official version metadata without downloading game assets.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, bail, ensure};
use block_launcher::manifest::{
    decode,
    model::{LegacyVersionEntry, VersionEntry, VersionIndex, VersionManifest, VersionType},
};
use rayon::prelude::{IntoParallelRefIterator as _, ParallelIterator as _};
use reqwest::blocking::Client;
use serde::{Serialize, de::DeserializeOwned};
use sha1::{Digest as _, Sha1};

const BASE_URL: &str = "https://piston-meta.mojang.com/mc/game";

#[derive(Debug, Serialize)]
struct Report {
    audited_at_unix_seconds: u64,
    source: String,
    index_v1_sha1: String,
    index_v2_sha1: String,
    indexed_versions: usize,
    passed: usize,
    total_json_bytes: usize,
    by_type: BTreeMap<VersionType, usize>,
    legacy_arguments: usize,
    modern_arguments: usize,
    default_user_jvm: usize,
    without_java_version: usize,
    without_logging: usize,
    libraries: usize,
    failures: Vec<Failure>,
}

#[derive(Debug, Serialize)]
struct Failure {
    id: String,
    error: String,
}

fn main() -> anyhow::Result<()> {
    let mut args = env::args_os().skip(1);
    let command = args.next().unwrap_or_else(|| "verify".into());
    let root = args.next().map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../experiments/minecraft-manifests"),
        PathBuf::from,
    );
    ensure!(
        args.next().is_none(),
        "usage: manifest-audit [fetch|fetch-snapshot|verify] [experiment-directory]"
    );
    match command.to_str() {
        Some("fetch") => fetch(&root, false),
        Some("fetch-snapshot") => fetch(&root, true),
        Some("verify") => verify(&root),
        _ => bail!("usage: manifest-audit [fetch|fetch-snapshot|verify] [experiment-directory]"),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn version_path(root: &Path, entry: &VersionEntry) -> anyhow::Result<PathBuf> {
    ensure!(
        entry.sha1.len() == 40 && entry.sha1.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid SHA-1 for {}",
        entry.id
    );
    Ok(root
        .join("cache/versions")
        .join(format!("{}.json", entry.sha1)))
}

fn download(client: &Client, url: &str) -> anyhow::Result<Vec<u8>> {
    // Retry transient network failures; schema and integrity failures are never retried.
    for attempt in 0..3 {
        let result = client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .and_then(reqwest::blocking::Response::bytes);
        match result {
            Ok(bytes) => return Ok(bytes.to_vec()),
            Err(error) if attempt == 2 => return Err(error).with_context(|| url.to_owned()),
            Err(_) => thread::sleep(Duration::from_secs(attempt + 1)),
        }
    }
    bail!("download attempts exhausted for {url}")
}

fn fetch(root: &Path, snapshot: bool) -> anyhow::Result<()> {
    fs::create_dir_all(root.join("cache/versions"))?;
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("Block manifest parsing experiment")
        .build()?;
    let bytes = if snapshot {
        fs::read(root.join("snapshot/version_manifest_v2.json"))?
    } else {
        download(&client, &format!("{BASE_URL}/version_manifest_v2.json"))?
    };
    let index: VersionIndex = decode(&bytes)?;
    let legacy = if snapshot {
        fs::read(root.join("snapshot/version_manifest.json"))?
    } else {
        download(&client, &format!("{BASE_URL}/version_manifest.json"))?
    };
    let legacy_index: VersionIndex<LegacyVersionEntry> = decode(&legacy)?;
    validate_indexes(&index, &legacy_index)?;
    fs::write(root.join("cache/version_manifest_v2.json"), bytes)?;
    fs::write(root.join("cache/version_manifest.json"), legacy)?;
    let completed = AtomicUsize::new(0);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(8).build()?;
    let results: Vec<anyhow::Result<()>> = pool.install(|| {
        index
            .versions
            .par_iter()
            .map(|entry| {
                let path = version_path(root, entry)?;
                let cached = fs::read(&path).is_ok_and(|data| digest(&data) == entry.sha1);
                if !cached {
                    ensure!(
                        entry.url.starts_with("https://piston-meta.mojang.com/"),
                        "unexpected metadata host for {}",
                        entry.id
                    );
                    let data = download(&client, &entry.url)?;
                    ensure!(
                        digest(&data) == entry.sha1,
                        "SHA-1 mismatch for {}",
                        entry.id
                    );
                    fs::write(path, data)?;
                }
                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(100) || count == index.versions.len() {
                    eprintln!("{count}/{} verified downloads", index.versions.len());
                }
                Ok(())
            })
            .collect()
    });
    let failures: Vec<String> = results
        .into_iter()
        .filter_map(Result::err)
        .map(|error| format!("{error:#}"))
        .collect();
    ensure!(
        failures.is_empty(),
        "download failures: {}",
        failures.join("; ")
    );
    Ok(())
}

fn round_trip<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> anyhow::Result<T> {
    let parsed = decode::<T>(bytes)?;
    let original: serde_json::Value = serde_json::from_slice(bytes)?;
    ensure!(
        serde_json::to_value(&parsed)? == original,
        "typed serialization changed the JSON structure or a value"
    );
    Ok(parsed)
}

fn read_version(root: &Path, entry: &VersionEntry) -> anyhow::Result<(VersionManifest, usize)> {
    let bytes = fs::read(version_path(root, entry)?)?;
    ensure!(digest(&bytes) == entry.sha1, "SHA-1 mismatch");
    let version: VersionManifest = round_trip(&bytes)?;
    ensure!(
        version.id == entry.id,
        "version ID does not match the index"
    );
    ensure!(
        version.kind == entry.kind,
        "version type does not match the index"
    );
    ensure!(
        version.arguments.is_some() != version.minecraft_arguments.is_some(),
        "expected exactly one argument generation"
    );
    Ok((version, bytes.len()))
}

fn validate_indexes(
    index: &VersionIndex,
    legacy: &VersionIndex<LegacyVersionEntry>,
) -> anyhow::Result<()> {
    let ids: BTreeSet<_> = index.versions.iter().map(|entry| &entry.id).collect();
    let legacy_ids: BTreeSet<_> = legacy.versions.iter().map(|entry| &entry.id).collect();
    ensure!(
        ids.len() == index.versions.len(),
        "duplicate v2 version IDs"
    );
    ensure!(
        legacy_ids.len() == legacy.versions.len(),
        "duplicate v1 version IDs"
    );
    ensure!(
        ids == legacy_ids && index.latest == legacy.latest,
        "v1 and v2 indexes refer to different version sets; fetch a consistent snapshot"
    );
    ensure!(
        ids.contains(&index.latest.release) && ids.contains(&index.latest.snapshot),
        "latest versions are absent from the index"
    );
    ensure!(!ids.is_empty(), "the version index is empty");
    let entries: BTreeMap<_, _> = index
        .versions
        .iter()
        .map(|entry| (&entry.id, entry))
        .collect();
    for old in &legacy.versions {
        let new = entries.get(&old.id).context("v1 entry is absent from v2")?;
        ensure!(
            old.url == new.url
                && old.kind == new.kind
                && old.time == new.time
                && old.release_time == new.release_time,
            "index generations disagree on metadata for {}",
            old.id
        );
    }
    Ok(())
}

fn verify(root: &Path) -> anyhow::Result<()> {
    let v2 = fs::read(root.join("cache/version_manifest_v2.json"))?;
    let v1 = fs::read(root.join("cache/version_manifest.json"))?;
    let index: VersionIndex = round_trip(&v2)?;
    let legacy: VersionIndex<LegacyVersionEntry> = round_trip(&v1)?;
    validate_indexes(&index, &legacy)?;
    let mut report = Report {
        audited_at_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        source: format!("{BASE_URL}/version_manifest_v2.json"),
        index_v1_sha1: digest(&v1),
        index_v2_sha1: digest(&v2),
        indexed_versions: index.versions.len(),
        passed: 0,
        total_json_bytes: 0,
        by_type: BTreeMap::new(),
        legacy_arguments: 0,
        modern_arguments: 0,
        default_user_jvm: 0,
        without_java_version: 0,
        without_logging: 0,
        libraries: 0,
        failures: Vec::new(),
    };
    for entry in &index.versions {
        match read_version(root, entry) {
            Ok((version, size)) => record(&mut report, &version, size),
            Err(error) => report.failures.push(Failure {
                id: entry.id.clone(),
                error: format!("{error:#}"),
            }),
        }
    }
    let path = root.join("report.json");
    fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;
    println!(
        "{}/{} version documents passed; {} failures",
        report.passed,
        report.indexed_versions,
        report.failures.len()
    );
    println!("Both indexes passed strict parsing and lossless JSON round-trip checks.");
    println!("Report: {}", path.display());
    for failure in &report.failures {
        eprintln!("{}: {}", failure.id, failure.error);
    }
    ensure!(
        report.failures.is_empty(),
        "not all official version documents passed"
    );
    Ok(())
}

fn record(report: &mut Report, version: &VersionManifest, size: usize) {
    report.passed += 1;
    report.total_json_bytes += size;
    *report.by_type.entry(version.kind).or_default() += 1;
    report.legacy_arguments += usize::from(version.minecraft_arguments.is_some());
    report.modern_arguments += usize::from(version.arguments.is_some());
    report.default_user_jvm += usize::from(
        version
            .arguments
            .as_ref()
            .is_some_and(|arguments| arguments.default_user_jvm.is_some()),
    );
    report.without_java_version += usize::from(version.java_version.is_none());
    report.without_logging += usize::from(version.logging.is_none());
    report.libraries += version.libraries.len();
}
