use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::ensure;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::session::{GameSession, SessionProvider};

const MAX_FRAME: u32 = 256 * 1024;

// This private wire representation is used only on the inherited bootstrap pipe.
// Keeping GameSession itself non-serializable prevents accidental disk persistence.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bootstrap {
    pub schema_version: u32,
    pub profile_id: String,
    pub profile_fingerprint: String,
    pub expires_at: u64,
    username: String,
    uuid: String,
    access_token: String,
    user_type: String,
    xuid: String,
    client_id: String,
}

impl fmt::Debug for Bootstrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bootstrap")
            .field("profile_id", &self.profile_id)
            .finish_non_exhaustive()
    }
}

impl Bootstrap {
    pub(super) fn new(
        profile_id: &str,
        game: &GameSession,
        expires_at: SystemTime,
    ) -> anyhow::Result<Self> {
        game.validate()?;
        Ok(Self {
            schema_version: 1,
            profile_id: profile_id.into(),
            profile_fingerprint: String::new(),
            expires_at: expires_at.duration_since(UNIX_EPOCH)?.as_secs(),
            username: game.username.clone(),
            uuid: game.uuid.clone(),
            access_token: game.access_token.clone(),
            user_type: game.user_type.clone(),
            xuid: game.xuid.clone(),
            client_id: game.client_id.clone(),
        })
    }

    pub(super) fn redact(&self, text: &str) -> String {
        if self.access_token.len() > 1 {
            text.replace(&self.access_token, "<redacted>")
        } else {
            text.to_owned()
        }
    }
}

impl SessionProvider for Bootstrap {
    fn session(&self) -> anyhow::Result<GameSession> {
        ensure!(self.schema_version == 1, "unsupported bootstrap protocol");
        let expiry = UNIX_EPOCH.checked_add(Duration::from_secs(self.expires_at));
        ensure!(
            expiry
                .and_then(|time| time.duration_since(SystemTime::now()).ok())
                .is_some_and(|remaining| remaining >= Duration::from_secs(30)),
            "The account session expired while preparing. Sign in and click Play again."
        );
        let game = GameSession {
            username: self.username.clone(),
            uuid: self.uuid.clone(),
            access_token: self.access_token.clone(),
            user_type: self.user_type.clone(),
            xuid: self.xuid.clone(),
            client_id: self.client_id.clone(),
        };
        game.validate()?;
        Ok(game)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub schema_version: u32,
    pub run_id: String,
    pub stop: bool,
}

pub(super) async fn send(
    stream: &mut (impl AsyncWrite + Unpin + Send),
    value: &(impl Serialize + Sync),
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= usize::try_from(MAX_FRAME)?,
        "session message is too large"
    );
    stream.write_u32(u32::try_from(bytes.len())?).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

pub(super) async fn receive<T: DeserializeOwned + Send>(
    stream: &mut (impl AsyncRead + Unpin + Send),
) -> anyhow::Result<T> {
    let length = stream.read_u32().await?;
    ensure!(
        length > 0 && length <= MAX_FRAME,
        "invalid session message length"
    );
    let mut bytes = vec![0; usize::try_from(length)?];
    stream.read_exact(&mut bytes).await?;
    // Never attach serde's error text: it may quote a secret-bearing bootstrap value.
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid session message"))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use anyhow::Context as _;
    use serde_json::json;

    use super::{Bootstrap, MAX_FRAME, Request, receive, send};
    use crate::session::{GameSession, SessionProvider as _};

    const TOKEN: &str = "synthetic-wire-token-not-for-diagnostics";

    fn bootstrap() -> anyhow::Result<Bootstrap> {
        Bootstrap::new(
            "fixture-profile",
            &GameSession {
                username: "FixturePlayer".into(),
                uuid: "0123456789abcdef0123456789abcdef".into(),
                access_token: TOKEN.into(),
                user_type: "msa".into(),
                xuid: "synthetic-xuid".into(),
                client_id: "synthetic-client-id".into(),
            },
            SystemTime::now() + Duration::from_secs(600),
        )
    }

    fn frame(payload: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut bytes = u32::try_from(payload.len())?.to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        Ok(bytes)
    }

    #[tokio::test]
    async fn authorized_bootstrap_round_trips_only_in_the_private_wire() -> anyhow::Result<()> {
        let original = bootstrap()?;
        let mut bytes = Vec::new();
        send(&mut bytes, &original).await?;
        let decoded: Bootstrap = receive(&mut bytes.as_slice()).await?;
        let session = decoded.session()?;
        assert_eq!(session.access_token, TOKEN);
        assert_eq!(session.username, "FixturePlayer");
        assert_eq!(decoded.profile_id, original.profile_id);

        let debug = format!("{decoded:?}");
        assert!(debug.contains("fixture-profile"));
        for value in [
            TOKEN,
            session.username.as_str(),
            session.uuid.as_str(),
            session.xuid.as_str(),
            session.client_id.as_str(),
        ] {
            assert!(!debug.contains(value));
        }
        assert_eq!(
            decoded.redact(&format!("first={TOKEN}, second={TOKEN}")),
            "first=<redacted>, second=<redacted>"
        );
        Ok(())
    }

    #[tokio::test]
    async fn malformed_messages_never_quote_their_secret_bearing_json() -> anyhow::Result<()> {
        let values = [
            serde_json::to_vec(TOKEN)?,
            serde_json::to_vec(
                &json!({"schema_version":1,"run_id":"fixture","stop":false,(TOKEN):"unexpected field"}),
            )?,
            format!("{{\"access_token\":\"{TOKEN}\",").into_bytes(),
        ];
        for payload in values {
            assert!(String::from_utf8_lossy(&payload).contains(TOKEN));
            let encoded = frame(&payload)?;
            let error = receive::<Request>(&mut encoded.as_slice())
                .await
                .err()
                .context("malformed JSON was accepted")?;
            assert_eq!(error.to_string(), "invalid session message");
            assert!(!format!("{error:#}").contains(TOKEN));
            assert!(!format!("{error:?}").contains(TOKEN));
        }
        Ok(())
    }

    #[tokio::test]
    async fn invalid_lengths_are_rejected_before_reading_the_payload() -> anyhow::Result<()> {
        for length in [0, MAX_FRAME + 1, u32::MAX] {
            let mut bytes = length.to_be_bytes().to_vec();
            bytes.extend_from_slice(TOKEN.as_bytes());
            let mut input = bytes.as_slice();
            let error = receive::<Request>(&mut input)
                .await
                .err()
                .context("invalid length was accepted")?;
            assert_eq!(error.to_string(), "invalid session message length");
            assert_eq!(input, TOKEN.as_bytes());
            assert!(!format!("{error:#}").contains(TOKEN));
        }
        Ok(())
    }

    #[tokio::test]
    async fn truncated_headers_and_payloads_return_without_secret_context() -> anyhow::Result<()> {
        let mut partial_body = 100_u32.to_be_bytes().to_vec();
        partial_body.extend_from_slice(TOKEN.as_bytes());
        for bytes in [vec![0, 0], partial_body] {
            let error = receive::<Request>(&mut bytes.as_slice())
                .await
                .err()
                .context("truncated frame was accepted")?;
            assert!(!format!("{error:#}").contains(TOKEN));
        }
        Ok(())
    }

    #[tokio::test]
    async fn maximum_frame_round_trips_and_one_extra_byte_writes_nothing() -> anyhow::Result<()> {
        // JSON adds two quotes around this ASCII string.
        let payload = "x".repeat(usize::try_from(MAX_FRAME)? - 2);
        let mut encoded = Vec::new();
        send(&mut encoded, &payload).await?;
        assert_eq!(&encoded[..4], &MAX_FRAME.to_be_bytes());
        let decoded: String = receive(&mut encoded.as_slice()).await?;
        assert_eq!(decoded, payload);

        let oversized = format!("{payload}x");
        let mut destination = Vec::new();
        let error = send(&mut destination, &oversized)
            .await
            .err()
            .context("oversized frame was sent")?;
        assert_eq!(error.to_string(), "session message is too large");
        assert_eq!(destination, Vec::<u8>::new());
        Ok(())
    }

    #[test]
    fn expired_short_lived_and_unrepresentable_sessions_are_rejected() -> anyhow::Result<()> {
        let mut value = bootstrap()?;
        assert!(value.session().is_ok());
        for expires_at in [
            1,
            (SystemTime::now() + Duration::from_secs(29))
                .duration_since(UNIX_EPOCH)?
                .as_secs(),
            u64::MAX,
        ] {
            value.expires_at = expires_at;
            let error = value
                .session()
                .err()
                .context("expired or invalid session was accepted")?;
            assert!(error.to_string().contains("session expired"));
            assert!(!format!("{error:#}").contains(TOKEN));
        }
        Ok(())
    }

    #[test]
    fn unsupported_protocol_and_invalid_identity_fail_without_quoting_credentials()
    -> anyhow::Result<()> {
        let mut value = bootstrap()?;
        value.schema_version = 2;
        assert_eq!(
            value
                .session()
                .err()
                .context("unsupported protocol was accepted")?
                .to_string(),
            "unsupported bootstrap protocol"
        );
        value.schema_version = 1;
        value.access_token = format!("{TOKEN}\n");
        let error = value
            .session()
            .err()
            .context("invalid identity was accepted")?;
        assert!(!format!("{error:#}").contains(TOKEN));
        Ok(())
    }
}
