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

/// First stored Profile as a mutable table.
pub(crate) fn first_profile_mut(document: &mut toml::Table) -> anyhow::Result<&mut toml::Table> {
    snapshot_mut(document)?
        .get_mut("profiles")
        .and_then(toml::Value::as_array_mut)
        .and_then(|profiles| profiles.first_mut())
        .and_then(toml::Value::as_table_mut)
        .context("first profile missing")
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
