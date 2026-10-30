//! Single durable document for user preferences, accounts and providers.
//!
//! All global user configuration lives in exactly one TOML document in the
//! data root:
//!
//! ```toml
//! [preferences]
//! appearance = "Dark"
//! reduce_motion = false
//!
//! [[accounts]]
//! provider = "Offline"
//! player_name = "Player"
//!
//! [[providers]]
//! id = "..."
//! name = "Community server"
//! ```
//!
//! The previous `preferences.json`, `accounts.json` and `providers.json`
//! documents are deliberately never read or migrated again. Every write goes
//! through [`update`], which loads the complete document, applies the caller's
//! change and atomically replaces the file. A preferences write therefore
//! cannot erase saved accounts or providers, and unknown top-level sections
//! written by another component survive untouched.
//!
//! TOML has no `null`, while the persisted structs contain `Option` fields.
//! Values are converted through `serde_json` and `null` object members are
//! dropped, so `None` fields serialize as absent keys and deserialize back to
//! their serde default.

use std::path::Path;

use anyhow::Context as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::appearance::Preferences;

/// The single global configuration document inside the data root.
pub(super) const FILE: &str = "block-global.toml";

/// Upper bound for the complete document. Its only purpose is to stop a
/// corrupted or hostile file from exhausting memory on load.
const MAX_BYTES: usize = 16 * 1024 * 1024;

const PREFERENCES_SECTION: &str = "preferences";
const ACCOUNTS_SECTION: &str = "accounts";
const PROVIDERS_SECTION: &str = "providers";

/// Serializes in-process read-modify-write cycles so a preferences save on the
/// UI thread cannot interleave with an account/provider save on a runtime
/// worker. Preferences saves additionally hold the data-root storage lease,
/// which serializes them with other components' writes across processes.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The complete global configuration document.
///
/// Every section is optional in the file and falls back to its serde default,
/// so documents written by older or newer versions still load.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct GlobalConfig {
    pub preferences: Preferences,
    pub accounts: Vec<block_auth::Account>,
    pub providers: Vec<block_auth::AuthlibProvider>,
}

/// Read the document, ignoring missing or malformed content.
///
/// This is the recovery path for display-only consumers: a damaged file must
/// not prevent the UI from rendering. Callers that must not lose user data
/// (writers) use [`read`] or [`update`] instead.
pub(super) fn load(root: &Path) -> GlobalConfig {
    let Some(table) = document(root).ok().flatten() else {
        return GlobalConfig::default();
    };
    GlobalConfig {
        preferences: table
            .get(PREFERENCES_SECTION)
            .and_then(|value| decode::<Preferences>(value).ok())
            .unwrap_or_default(),
        accounts: table
            .get(ACCOUNTS_SECTION)
            .and_then(|value| decode(value).ok())
            .unwrap_or_default(),
        providers: table
            .get(PROVIDERS_SECTION)
            .and_then(|value| decode(value).ok())
            .unwrap_or_default(),
    }
}

/// Read the complete document without losing errors.
///
/// `Ok(None)` means the file does not exist yet. A malformed document is
/// reported as an error (and preserved untouched), so callers can surface the
/// same failure they always did instead of silently dropping user data.
pub(super) fn read(root: &Path) -> anyhow::Result<Option<GlobalConfig>> {
    document(root)?.as_ref().map(config_from_table).transpose()
}

/// Atomically apply `change` to the complete document.
///
/// The mutation is applied in memory to a configuration parsed from the
/// current document and only the three owned sections are written back; every
/// other top-level section is preserved verbatim. In-process writers are
/// serialized by a lock so a preferences save on the UI thread cannot
/// interleave with an account/provider save on a runtime worker.
pub(super) fn update(root: &Path, change: impl FnOnce(&mut GlobalConfig)) -> anyhow::Result<()> {
    let _writer = WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut table = document(root)?.unwrap_or_default();
    let mut config = config_from_table(&table)?;
    change(&mut config);
    table.insert(
        PREFERENCES_SECTION.into(),
        encode(&config.preferences).context("Preferences could not be encoded")?,
    );
    table.insert(
        ACCOUNTS_SECTION.into(),
        encode(&config.accounts).context("Account metadata could not be encoded")?,
    );
    table.insert(
        PROVIDERS_SECTION.into(),
        encode(&config.providers).context("Provider metadata could not be encoded")?,
    );
    write_document(root, &table)
}

fn config_from_table(table: &toml::Table) -> anyhow::Result<GlobalConfig> {
    Ok(GlobalConfig {
        preferences: table
            .get(PREFERENCES_SECTION)
            .map(decode::<Preferences>)
            .transpose()
            .context("Saved preferences could not be read")?
            .unwrap_or_default(),
        accounts: table
            .get(ACCOUNTS_SECTION)
            .map(decode)
            .transpose()
            .context("Saved account metadata could not be read")?
            .unwrap_or_default(),
        providers: table
            .get(PROVIDERS_SECTION)
            .map(decode)
            .transpose()
            .context("Saved provider metadata could not be read")?
            .unwrap_or_default(),
    })
}

fn document(root: &Path) -> anyhow::Result<Option<toml::Table>> {
    let bytes = match std::fs::read(root.join(FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("The global configuration could not be opened"),
    };
    anyhow::ensure!(
        bytes.len() <= MAX_BYTES,
        "The global configuration is too large"
    );
    let text =
        std::str::from_utf8(&bytes).context("The global configuration is not valid UTF-8")?;
    let table = toml::from_str(text).context("The global configuration could not be parsed")?;
    Ok(Some(table))
}

fn write_document(root: &Path, table: &toml::Table) -> anyhow::Result<()> {
    use std::io::Write as _;

    let text =
        toml::to_string_pretty(table).context("The global configuration could not be encoded")?;
    std::fs::create_dir_all(root)?;
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(root.join(FILE))?;
    Ok(())
}

/// Encode a persisted value into a TOML value, omitting `None` object members.
fn encode<T: Serialize>(value: &T) -> anyhow::Result<toml::Value> {
    json_to_toml(serde_json::to_value(value)?)
}

/// Decode a TOML value back into a persisted struct.
fn decode<T: DeserializeOwned>(value: &toml::Value) -> anyhow::Result<T> {
    serde_json::from_value(toml_to_json(value)).map_err(Into::into)
}

/// TOML cannot represent `null`; object members whose value is `null` are
/// omitted so `None` fields deserialize back to their default.
fn json_to_toml(value: serde_json::Value) -> anyhow::Result<toml::Value> {
    use serde_json::Value as Json;

    Ok(match value {
        Json::Null => anyhow::bail!("The global configuration cannot represent null values"),
        Json::Bool(value) => toml::Value::Boolean(value),
        Json::Number(number) => match (number.as_i64(), number.as_u64()) {
            (Some(value), _) => toml::Value::Integer(value),
            (None, Some(value)) => toml::Value::Integer(
                i64::try_from(value).context("A configuration number is out of range")?,
            ),
            (None, None) => toml::Value::Float(
                number
                    .as_f64()
                    .context("A configuration number is invalid")?,
            ),
        },
        Json::String(value) => toml::Value::String(value),
        Json::Array(values) => toml::Value::Array(
            values
                .into_iter()
                .map(json_to_toml)
                .collect::<anyhow::Result<_>>()?,
        ),
        Json::Object(values) => toml::Value::Table(
            values
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| Ok((key, json_to_toml(value)?)))
                .collect::<anyhow::Result<_>>()?,
        ),
    })
}

/// Convert a TOML value back into JSON. TOML datetimes become strings.
fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    use serde_json::Value as Json;

    match value {
        toml::Value::String(value) => Json::String(value.clone()),
        toml::Value::Integer(value) => Json::Number((*value).into()),
        toml::Value::Float(value) => {
            serde_json::Number::from_f64(*value).map_or(Json::Null, Json::Number)
        }
        toml::Value::Boolean(value) => Json::Bool(*value),
        toml::Value::Datetime(value) => Json::String(value.to_string()),
        toml::Value::Array(values) => Json::Array(values.iter().map(toml_to_json).collect()),
        toml::Value::Table(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), toml_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use block_auth::{
        Account, AccountProvider, AccountRef, AuthlibAccount, AuthlibProvider, ProviderMetadata,
    };

    use super::{
        ACCOUNTS_SECTION, FILE, GlobalConfig, PREFERENCES_SECTION, PROVIDERS_SECTION, encode, load,
        read, update,
    };
    use crate::appearance::{Appearance, Preferences};

    fn fixture_account(authlib: Option<AuthlibAccount>) -> Account {
        Account {
            reference: AccountRef {
                player_id: "00000000-0000-4000-8000-000000000001"
                    .parse()
                    .unwrap_or_default(),
            },
            provider: if authlib.is_some() {
                AccountProvider::Authlib
            } else {
                AccountProvider::Offline
            },
            player_name: "Player".into(),
            client_id: String::new(),
            credential_ref: String::new(),
            authlib,
        }
    }

    fn fixture_provider() -> anyhow::Result<AuthlibProvider> {
        let mut provider = crate::providers::fixture_provider(false)?;
        // A missing server name exercises the `Option::None` path.
        provider.metadata = ProviderMetadata {
            server_name: None,
            ..provider.metadata
        };
        Ok(provider)
    }

    #[test]
    fn missing_and_malformed_documents_are_ignored_by_the_lenient_loader() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        assert_eq!(load(root.path()), GlobalConfig::default());
        assert_eq!(read(root.path())?, None);

        std::fs::write(root.path().join(FILE), b"not = [valid")?;
        let original = std::fs::read(root.path().join(FILE))?;
        assert_eq!(load(root.path()), GlobalConfig::default());
        assert!(read(root.path()).is_err());
        assert_eq!(std::fs::read(root.path().join(FILE))?, original);
        Ok(())
    }

    #[test]
    fn round_trip_omits_none_preserves_unknown_sections_and_writes_only_owned_sections()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::write(
            root.path().join(FILE),
            "[other]\nkept = true\n\n[preferences]\nappearance = \"Light\"\n",
        )?;

        let provider = fixture_provider()?;
        let options = block_launcher::DownloadOptions {
            bytes_per_second: None,
            ..Default::default()
        };
        let preferences = Preferences {
            appearance: Appearance::Dark,
            reduce_motion: true,
            onboarding_complete: true,
            downloads: options,
            ..Preferences::default()
        };
        let account = fixture_account(None);
        let expected = GlobalConfig {
            preferences: preferences.clone(),
            accounts: vec![account.clone()],
            providers: vec![provider.clone()],
        };
        update(root.path(), |config| *config = expected.clone())?;

        // Reading the same document back preserves every value, including the
        // `None` fields that were omitted from the TOML text.
        let loaded = read(root.path())?.unwrap_or_default();
        assert_eq!(loaded, expected);
        assert_eq!(load(root.path()), expected);

        let text = std::fs::read_to_string(root.path().join(FILE))?;
        assert!(text.contains("[other]"), "{text}");
        assert!(text.contains("[preferences]"), "{text}");
        assert!(text.contains("[[accounts]]"), "{text}");
        assert!(text.contains("[[providers]]"), "{text}");
        assert!(!text.contains("authlib"), "{text}");
        assert!(!text.contains("bytes_per_second"), "{text}");
        assert!(!text.contains("server_name"), "{text}");

        // A preferences-only write keeps accounts, providers and unknown
        // top-level sections.
        let mut expected_after_preferences = preferences.clone();
        expected_after_preferences.reduce_motion = false;
        update(root.path(), |config| {
            config.preferences.reduce_motion = false;
        })?;
        let loaded = read(root.path())?.unwrap_or_default();
        assert_eq!(loaded.accounts, vec![account]);
        assert_eq!(loaded.providers, vec![provider]);
        assert!(!loaded.preferences.reduce_motion);

        // A providers-only write keeps preferences and accounts.
        update(root.path(), |config| config.providers.clear())?;
        let loaded = read(root.path())?.unwrap_or_default();
        assert_eq!(loaded.preferences, expected_after_preferences);
        assert_eq!(loaded.accounts, vec![fixture_account(None)]);
        assert!(loaded.providers.is_empty());

        let table: toml::Table = toml::from_str(&std::fs::read_to_string(root.path().join(FILE))?)?;
        assert_eq!(
            table
                .get("other")
                .and_then(|value| value.get("kept"))
                .and_then(toml::Value::as_bool),
            Some(true)
        );
        assert!(table.contains_key(PREFERENCES_SECTION));
        assert!(table.contains_key(ACCOUNTS_SECTION));
        assert!(table.contains_key(PROVIDERS_SECTION));
        Ok(())
    }

    #[test]
    fn encoding_strips_nested_nulls_from_persisted_values() -> anyhow::Result<()> {
        let account = fixture_account(None);
        let value = encode(&account)?;
        let table = value
            .as_table()
            .ok_or_else(|| anyhow::anyhow!("account should encode as a table"))?;
        assert!(!table.contains_key("authlib"));
        assert_eq!(
            table.get("player_name").and_then(toml::Value::as_str),
            Some("Player")
        );
        Ok(())
    }
}
