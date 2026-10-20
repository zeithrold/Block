use std::{fs, path::Path, process::Command};

use minecraft_manifest::{decode, model::VersionIndex};
use serde_json::Value;

fn sample_corpus() -> anyhow::Result<tempfile::TempDir> {
    let root = tempfile::tempdir()?;
    let fixtures = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    fs::create_dir_all(root.path().join("cache/versions"))?;
    for name in ["version_manifest.json", "version_manifest_v2.json"] {
        fs::copy(fixtures.join(name), root.path().join("cache").join(name))?;
    }
    let index: VersionIndex = decode(&fs::read(fixtures.join("version_manifest_v2.json"))?)?;
    for entry in index.versions {
        fs::copy(
            fixtures.join(format!("{}.json", entry.id)),
            root.path()
                .join("cache/versions")
                .join(format!("{}.json", entry.sha1)),
        )?;
    }
    Ok(root)
}

#[test]
fn audit_verifies_every_index_entry_and_writes_a_report() -> anyhow::Result<()> {
    let root = sample_corpus()?;
    let output = Command::new(env!("CARGO_BIN_EXE_manifest-audit"))
        .arg("verify")
        .arg(root.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(root.path().join("report.json"))?)?;
    assert_eq!(report["passed"], 2);
    assert_eq!(report["indexed_versions"], 2);
    assert_eq!(report["failures"], serde_json::json!([]));
    Ok(())
}

#[test]
fn corrupt_and_missing_documents_are_reported_without_skipping_entries() -> anyhow::Result<()> {
    let root = sample_corpus()?;
    let index: VersionIndex = decode(&fs::read(
        root.path().join("cache/version_manifest_v2.json"),
    )?)?;
    for (position, entry) in index.versions.iter().enumerate() {
        let path = root
            .path()
            .join("cache/versions")
            .join(format!("{}.json", entry.sha1));
        if position == 0 {
            fs::write(path, b"{}")?;
        } else {
            fs::remove_file(path)?;
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_manifest-audit"))
        .arg("verify")
        .arg(root.path())
        .output()?;
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&fs::read(root.path().join("report.json"))?)?;
    assert_eq!(report["passed"], 0);
    let failures = report["failures"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing failure list"))?;
    assert_eq!(failures.len(), 2);
    assert!(failures.iter().any(|failure| {
        failure["error"]
            .as_str()
            .is_some_and(|error| error.contains("SHA-1 mismatch"))
    }));
    Ok(())
}
