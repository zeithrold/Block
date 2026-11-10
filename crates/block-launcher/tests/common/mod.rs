//! Shared fixtures for integration tests that exercise the single `block.toml`
//! configuration document in the data root.

#![allow(
    dead_code,
    reason = "Each integration test target uses a different subset of these helpers."
)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context as _;

/// The single configuration document in the data root.
pub(crate) const FILE: &str = "block.toml";

/// Path of the configuration document below `root`.
pub(crate) fn path(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// Read and decode the complete document.
pub(crate) fn load(root: &Path) -> anyhow::Result<toml::Table> {
    Ok(toml::from_str(&fs::read_to_string(path(root))?)?)
}

/// Encode and write the complete document.
pub(crate) fn save(root: &Path, document: &toml::Table) -> anyhow::Result<()> {
    fs::write(path(root), toml::to_string_pretty(document)?)?;
    Ok(())
}

/// The `[catalog.snapshot]` section as a mutable table.
pub(crate) fn snapshot_mut(document: &mut toml::Table) -> anyhow::Result<&mut toml::Table> {
    document
        .get_mut("catalog")
        .and_then(toml::Value::as_table_mut)
        .and_then(|catalog| catalog.get_mut("snapshot"))
        .and_then(toml::Value::as_table_mut)
        .context("catalog snapshot section missing")
}

/// First stored Instance as a mutable table.
pub(crate) fn first_instance_mut(document: &mut toml::Table) -> anyhow::Result<&mut toml::Table> {
    snapshot_mut(document)?
        .get_mut("instances")
        .and_then(toml::Value::as_array_mut)
        .and_then(|instances| instances.first_mut())
        .and_then(toml::Value::as_table_mut)
        .context("first instance missing")
}

/// A single managed Instance with the catalog's normal creation defaults.
pub(crate) fn instance(
    catalog: &block_launcher::catalog::Catalog,
) -> anyhow::Result<block_launcher::catalog::Instance> {
    catalog.create_instance("Game".into(), "1.20.6".into(), None, None)
}

/// An importable local Vanilla installation with explicit launch-data evidence.
pub(crate) fn candidate(
    resource_directory: &Path,
    version: &str,
    data: block_launcher::game_import::GameDataEvidence,
) -> anyhow::Result<block_launcher::game_import::GameCandidate> {
    use block_launcher::game_import::{DetectedLoader, GameCandidate, LoaderKind, SourceKind};
    let directory = resource_directory.join("versions").join(version);
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join(format!("{version}.json")),
        serde_json::to_vec(&serde_json::json!({"id": version}))?,
    )?;
    Ok(GameCandidate {
        id: version.into(),
        source: SourceKind::OfficialLauncher,
        source_root: resource_directory.into(),
        resource_directory: resource_directory.into(),
        name: version.into(),
        version_id: Some(version.into()),
        data,
        configured_version: Some(version.into()),
        minecraft_version: Some(version.into()),
        loader: Some(DetectedLoader {
            kind: LoaderKind::Vanilla,
            version: None,
        }),
        latest_release: None,
        latest_snapshot: None,
        importable: true,
        unsupported_reason: None,
        metadata_warning: None,
    })
}

/// Build a TOML table from literal key/value pairs.
pub(crate) fn table(entries: &[(&str, toml::Value)]) -> toml::Value {
    toml::Value::Table(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}
