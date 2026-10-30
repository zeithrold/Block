//! Atomic read-modify-write access to the single `block.toml` data-root document.
//!
//! This crate keeps its durable configuration in exactly one TOML file in the
//! data root. Each component owns one top-level section: `[storage]` (library,
//! cache and relocation generation) and `[catalog]` (catalog schema version and
//! installation snapshot). Every write reads the complete document first and
//! only replaces the owned section, so sections owned by other components are
//! preserved verbatim. The old `storage.json` and `catalog.json` documents are
//! never read or written again.
//!
//! The transient `storage-migration.json` journal is deliberately *not* stored
//! here. It is crash-recovery state for an in-flight relocation rather than user
//! configuration, so it stays a small disposable JSON document that is removed
//! when the migration is resolved.
//!
//! Callers of [`update`] must already hold the data-root lease
//! ([`crate::storage::acquire`]). The lease serializes read-modify-write cycles
//! across processes, so two writers cannot drop each other's sections. Every
//! writer in this crate holds it: catalog transactions acquire it for writes,
//! and storage relocation holds the exclusive lease.

use std::{io::Write as _, path::Path};

use anyhow::Context as _;

/// The single configuration document inside the data root.
pub(crate) const FILE: &str = "block.toml";

/// Top-level section holding this crate's library/cache selection.
pub(crate) const STORAGE_SECTION: &str = "storage";

/// Top-level section holding the catalog schema version and snapshot.
pub(crate) const CATALOG_SECTION: &str = "catalog";

/// Read the complete document. A missing file is an empty document.
///
/// Malformed content is reported without touching the file, so a user can fix
/// it by hand instead of losing unrelated sections.
pub(crate) fn load(root: &Path) -> anyhow::Result<toml::Table> {
    match std::fs::read(root.join(FILE)) {
        Ok(bytes) => {
            let text = std::str::from_utf8(&bytes)
                .context("Configuration is invalid; it was preserved")?;
            toml::from_str(text).context("Configuration is invalid; it was preserved")
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(error) => Err(error.into()),
    }
}

/// Encode and atomically replace the document in `root`.
///
/// The temporary file lives in `root` so the final rename stays on the same
/// filesystem, and it is synced before the rename so a crash cannot leave a
/// truncated document behind.
pub(crate) fn write(root: &Path, table: &toml::Table) -> anyhow::Result<()> {
    let text = toml::to_string_pretty(table).context("Configuration could not be encoded")?;
    std::fs::create_dir_all(root)?;
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(root.join(FILE))?;
    Ok(())
}

/// Atomically apply `change` to the complete document.
///
/// Unknown sections survive because the whole document is loaded first and only
/// written back after the change.
pub(crate) fn update(root: &Path, change: impl FnOnce(&mut toml::Table)) -> anyhow::Result<()> {
    let mut table = load(root)?;
    change(&mut table);
    write(root, &table)
}

/// Convert a JSON value into a TOML value, dropping `null` object members.
///
/// TOML has no null. Serde writes absent or `None` fields as `null`, so those
/// members are omitted instead of failing the write; reading the document back
/// leaves the field at its serde default (`None`).
pub(crate) fn from_json(value: &serde_json::Value) -> anyhow::Result<toml::Value> {
    use serde_json::Value as Json;

    Ok(match value {
        Json::Null => anyhow::bail!("Configuration cannot represent null values"),
        Json::Bool(value) => toml::Value::Boolean(*value),
        Json::Number(number) => match (number.as_i64(), number.as_u64()) {
            (Some(value), _) => toml::Value::Integer(value),
            (None, Some(value)) => toml::Value::Integer(
                i64::try_from(value).context("Configuration number is out of range")?,
            ),
            (None, None) => {
                toml::Value::Float(number.as_f64().context("Configuration number is invalid")?)
            }
        },
        Json::String(value) => toml::Value::String(value.clone()),
        Json::Array(values) => toml::Value::Array(
            values
                .iter()
                .map(from_json)
                .collect::<anyhow::Result<_>>()?,
        ),
        Json::Object(values) => toml::Value::Table(
            values
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| Ok((key.clone(), from_json(value)?)))
                .collect::<anyhow::Result<_>>()?,
        ),
    })
}

/// Convert a TOML value back into JSON. TOML datetimes become strings.
pub(crate) fn to_json(value: &toml::Value) -> serde_json::Value {
    use serde_json::Value as Json;

    match value {
        toml::Value::String(value) => Json::String(value.clone()),
        toml::Value::Integer(value) => Json::Number((*value).into()),
        toml::Value::Float(value) => {
            serde_json::Number::from_f64(*value).map_or(Json::Null, Json::Number)
        }
        toml::Value::Boolean(value) => Json::Bool(*value),
        toml::Value::Datetime(value) => Json::String(value.to_string()),
        toml::Value::Array(values) => Json::Array(values.iter().map(to_json).collect()),
        toml::Value::Table(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{FILE, from_json, load, to_json, update};

    #[test]
    fn missing_document_loads_empty_and_malformed_content_is_preserved() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        assert!(load(root.path())?.is_empty());

        std::fs::write(root.path().join(FILE), b"not = [valid")?;
        let original = std::fs::read(root.path().join(FILE))?;
        assert!(load(root.path()).is_err());
        assert_eq!(std::fs::read(root.path().join(FILE))?, original);
        Ok(())
    }

    #[test]
    fn update_preserves_unknown_sections_and_omits_null_fields() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::write(
            root.path().join(FILE),
            "[other]\nkept = true\n\n[storage]\ncache = \"previous\"\n",
        )?;
        let storage = from_json(&serde_json::json!({
            "library": null,
            "cache": "/synthetic/cache",
            "generation": "7d6bd6d2-0a3e-4a3e-9d55-4c5d5f7d6a1e",
        }))?;
        update(root.path(), |table| {
            table.insert("storage".into(), storage);
        })?;

        let table = load(root.path())?;
        assert_eq!(
            table
                .get("other")
                .and_then(|value| value.get("kept"))
                .and_then(toml::Value::as_bool),
            Some(true)
        );
        let storage = table
            .get("storage")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| anyhow::anyhow!("storage section missing"))?;
        assert!(!storage.contains_key("library"));
        assert_eq!(
            storage.get("cache").and_then(toml::Value::as_str),
            Some("/synthetic/cache")
        );
        Ok(())
    }

    #[test]
    fn json_and_toml_values_round_trip_through_the_document() -> anyhow::Result<()> {
        let value = serde_json::json!({
            "schema_version": 1,
            "snapshot": {
                "profile_mode": "bundled",
                "instances": [{
                    "id": "7d6bd6d2-0a3e-4a3e-9d55-4c5d5f7d6a1e",
                    "name": "Fixture",
                    "directory": "/synthetic/game",
                    "targets": [{
                        "id": "vanilla",
                        "kind": "vanilla",
                        "loader_version": null,
                        "manifest": null,
                        "status": {"state": "ready"},
                        "optifine": null,
                        "optifine_mod": null,
                    }],
                    "settings": null,
                }],
                "profiles": [],
            },
        });
        let toml_value = from_json(&value)?;
        let text = toml::to_string_pretty(&toml_value)?;
        let parsed: toml::Table = toml::from_str(&text)?;
        let json = to_json(&toml::Value::Table(parsed));
        assert_eq!(json["schema_version"], value["schema_version"]);
        assert_eq!(json["snapshot"]["profile_mode"], "bundled");
        assert_eq!(
            json["snapshot"]["instances"][0]["directory"],
            "/synthetic/game"
        );
        assert_eq!(json["snapshot"]["profiles"], serde_json::json!([]));
        Ok(())
    }
}
