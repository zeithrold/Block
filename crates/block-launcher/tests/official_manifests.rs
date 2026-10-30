//! Offline regression coverage for the complete captured Mojang corpus.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context as _, ensure};
use block_launcher::manifest::{
    decode,
    model::{LegacyVersionEntry, VersionEntry, VersionIndex, VersionManifest, VersionType},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha1::{Digest as _, Sha1};

const VERSION_COUNT: usize = 919;

fn corpus() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/manifests/official"
    ))
}

fn read_index<E: DeserializeOwned + Serialize>(name: &str) -> anyhow::Result<VersionIndex<E>> {
    let path = corpus().join(name);
    let bytes = fs::read(&path).with_context(|| format!("missing fixture: {}", path.display()))?;
    lossless_decode(&bytes)
}

fn lossless_decode<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> anyhow::Result<T> {
    let parsed: T = decode(bytes)?;
    let original: Value = serde_json::from_slice(bytes)?;
    ensure!(
        serde_json::to_value(&parsed)? == original,
        "typed serialization changed JSON keys or values"
    );
    Ok(parsed)
}

fn read_version(entry: &VersionEntry) -> anyhow::Result<(VersionManifest, usize)> {
    ensure!(
        entry.sha1.len() == 40 && entry.sha1.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid SHA-1 in the captured index"
    );
    let path = corpus()
        .join("versions")
        .join(format!("{}.json", entry.sha1));
    let bytes = fs::read(&path).with_context(|| format!("missing fixture: {}", path.display()))?;
    ensure!(
        format!("{:x}", Sha1::digest(&bytes)) == entry.sha1,
        "fixture SHA-1 does not match the official index"
    );
    let version: VersionManifest = lossless_decode(&bytes)?;
    ensure!(version.id == entry.id, "version ID differs from the index");
    ensure!(
        version.kind == entry.kind,
        "version type differs from the index"
    );
    ensure!(
        version.arguments.is_some() != version.minecraft_arguments.is_some(),
        "expected exactly one argument generation"
    );
    Ok((version, bytes.len()))
}

#[test]
fn complete_official_indexes_are_lossless_and_consistent() -> anyhow::Result<()> {
    let index: VersionIndex = read_index("version_manifest_v2.json")?;
    let legacy: VersionIndex<LegacyVersionEntry> = read_index("version_manifest.json")?;
    assert_eq!(index.versions.len(), VERSION_COUNT);
    assert_eq!(legacy.versions.len(), VERSION_COUNT);
    assert_eq!(index.latest, legacy.latest);
    let entries: BTreeMap<_, _> = index
        .versions
        .iter()
        .map(|entry| (&entry.id, entry))
        .collect();
    let old_ids: BTreeSet<_> = legacy.versions.iter().map(|entry| &entry.id).collect();
    assert_eq!(entries.len(), VERSION_COUNT, "duplicate v2 version IDs");
    assert_eq!(old_ids.len(), VERSION_COUNT, "duplicate v1 version IDs");
    assert!(entries.contains_key(&index.latest.release));
    assert!(entries.contains_key(&index.latest.snapshot));
    for old in legacy.versions {
        let new = entries
            .get(&old.id)
            .with_context(|| format!("{} is absent from v2", old.id))?;
        assert_eq!(old.kind, new.kind, "{}: type differs", old.id);
        assert_eq!(old.url, new.url, "{}: URL differs", old.id);
        assert_eq!(old.time, new.time, "{}: time differs", old.id);
        assert_eq!(
            old.release_time, new.release_time,
            "{}: release time differs",
            old.id
        );
    }
    Ok(())
}

#[test]
fn every_official_version_is_typed_and_lossless() -> anyhow::Result<()> {
    let index: VersionIndex = read_index("version_manifest_v2.json")?;
    assert_eq!(index.versions.len(), VERSION_COUNT);
    let mut failures = Vec::new();
    let mut passed = 0;
    let mut bytes = 0;
    let mut libraries = 0;
    let mut legacy_arguments = 0;
    let mut default_user_jvm = 0;
    let mut by_type = BTreeMap::new();
    for entry in &index.versions {
        match read_version(entry) {
            Ok((version, size)) => {
                passed += 1;
                bytes += size;
                libraries += version.libraries.len();
                legacy_arguments += usize::from(version.minecraft_arguments.is_some());
                default_user_jvm += usize::from(
                    version
                        .arguments
                        .as_ref()
                        .is_some_and(|arguments| arguments.default_user_jvm.is_some()),
                );
                *by_type.entry(version.kind).or_insert(0) += 1;
            }
            Err(error) => failures.push(format!("{}: {error:#}", entry.id)),
        }
    }
    assert!(
        failures.is_empty(),
        "{} version fixtures failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(passed, VERSION_COUNT);
    assert_eq!(bytes, 23_772_351);
    assert_eq!(libraries, 53_896);
    assert_eq!(legacy_arguments, 349);
    assert_eq!(default_user_jvm, 60);
    assert_eq!(
        by_type,
        BTreeMap::from([
            (VersionType::Release, 103),
            (VersionType::Snapshot, 755),
            (VersionType::OldBeta, 26),
            (VersionType::OldAlpha, 35),
        ])
    );
    Ok(())
}

#[test]
fn official_fixture_directory_matches_the_index_exactly() -> anyhow::Result<()> {
    let index: VersionIndex = read_index("version_manifest_v2.json")?;
    let expected: BTreeSet<_> = index
        .versions
        .iter()
        .map(|entry| format!("{}.json", entry.sha1).into())
        .collect();
    let actual = fs::read_dir(corpus().join("versions"))?
        .map(|entry| entry.map(|file| file.file_name()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    assert_eq!(expected.len(), VERSION_COUNT);
    assert_eq!(actual, expected, "missing or unindexed version fixtures");
    Ok(())
}
