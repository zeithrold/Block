use std::fmt;

use anyhow::ensure;
use md5::{Digest as _, Md5};
use serde::{Deserialize, Serialize};

/// Offline identity for an owned game installation; it grants no online-service access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfflineSession {
    Offline { username: String },
}

/// Auth adapters supply identity at command binding time, after resource preparation.
pub trait SessionProvider: Send + Sync {
    /// Obtain a currently usable game identity.
    ///
    /// # Errors
    /// Returns validation/authentication errors, including expired online sessions.
    fn session(&self) -> anyhow::Result<GameSession>;
}

/// Intentionally not serializable: secrets never belong to recipes or execution plans.
#[derive(Clone)]
pub struct GameSession {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
    pub xuid: String,
    pub client_id: String,
    pub user_properties: String,
    pub authlib: Option<AuthlibLaunch>,
}

/// Public provider configuration carried only with the transient launch session.
/// The Java command rechecks the agent digest immediately before binding it.
#[derive(Clone, Debug)]
pub struct AuthlibLaunch {
    pub api_url: String,
    pub metadata_json: String,
    pub agent: std::path::PathBuf,
    pub sha256: String,
}

impl AuthlibLaunch {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        let url = reqwest::Url::parse(&self.api_url)?;
        ensure!(
            matches!(url.scheme(), "https" | "http")
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.as_str() == self.api_url
                && url.path().ends_with('/'),
            "invalid Authlib API root"
        );
        ensure!(
            self.metadata_json.len() <= 512 * 1024
                && serde_json::from_str::<serde_json::Value>(&self.metadata_json)?.is_object(),
            "invalid prefetched Authlib metadata"
        );
        ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid Authlib agent digest"
        );
        ensure!(
            self.agent.is_absolute()
                && self
                    .agent
                    .to_str()
                    .is_some_and(|path| !path.contains('=') && !path.chars().any(char::is_control)),
            "Authlib agent requires an absolute UTF-8 path without '=' or control characters"
        );
        Ok(())
    }
}

impl fmt::Debug for GameSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameSession")
            .field("username", &self.username)
            .field("uuid", &self.uuid)
            .finish_non_exhaustive()
    }
}

impl OfflineSession {
    /// Validate a Java player name.
    ///
    /// # Errors
    /// Rejects names the game cannot use as an offline profile.
    pub fn validate(&self) -> anyhow::Result<()> {
        let Self::Offline { username } = self;
        ensure!(
            !username.is_empty()
                && username.len() <= 16
                && username
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "Offline username must be 1–16 letters, digits or underscores."
        );
        Ok(())
    }
}

impl SessionProvider for OfflineSession {
    fn session(&self) -> anyhow::Result<GameSession> {
        self.validate()?;
        let Self::Offline { username } = self;
        // Java UUID.nameUUIDFromBytes uses this exact MD5 input and UUID v3 bits.
        let mut bytes: [u8; 16] =
            Md5::digest(format!("OfflinePlayer:{username}").as_bytes()).into();
        bytes[6] = (bytes[6] & 0x0f) | 0x30;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(GameSession {
            username: username.clone(),
            uuid: uuid::Uuid::from_bytes(bytes).simple().to_string(),
            access_token: "0".into(),
            user_type: "msa".into(),
            xuid: "0".into(),
            client_id: String::new(),
            user_properties: "{}".into(),
            authlib: None,
        })
    }
}

impl GameSession {
    /// Validate the final identity independently of installation metadata.
    /// # Errors
    /// Rejects unusable or inconsistent player/session fields.
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(authlib) = &self.authlib {
            ensure!(
                self.user_type == "mojang",
                "Authlib injection requires an external account"
            );
            authlib.validate()?;
        }
        if self.user_type == "mojang" {
            ensure!(
                !self.username.is_empty()
                    && self.username.encode_utf16().count() <= 16
                    && !self.username.chars().any(char::is_control),
                "invalid external player name"
            );
        } else {
            OfflineSession::Offline {
                username: self.username.clone(),
            }
            .validate()?;
        }
        ensure!(
            self.user_properties.len() <= 64 * 1024,
            "user properties are too large"
        );
        let properties: serde_json::Value = serde_json::from_str(&self.user_properties)?;
        ensure!(
            properties
                .as_object()
                .is_some_and(|values| values.len() <= 64
                    && values.iter().all(|(name, value)| !name.is_empty()
                        && name.len() <= 128
                        && value.as_array().is_some_and(|items| items
                            .iter()
                            .all(|item| item.as_str().is_some_and(|text| text.len() <= 4096))))),
            "invalid user properties"
        );
        ensure!(
            self.uuid.len() == 32 && self.uuid.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid player UUID"
        );
        ensure!(
            !self.access_token.is_empty() && !self.access_token.chars().any(char::is_control),
            "invalid game access token"
        );
        ensure!(
            self.user_type == "msa" || self.user_type == "mojang",
            "unsupported game identity type"
        );
        Ok(())
    }
}

impl SessionProvider for GameSession {
    fn session(&self) -> anyhow::Result<GameSession> {
        self.validate()?;
        Ok(self.clone())
    }
}
