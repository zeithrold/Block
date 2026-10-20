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
            "offline username must contain 1..16 ASCII letters, digits or underscores"
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
        })
    }
}
