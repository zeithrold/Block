use std::{fmt, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::Zeroizing;

use crate::{AuthError, Result, SecretString};

use crate::MICROSOFT_SCOPE as SCOPE;
const AUTHORIZE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const CALLBACK_SECONDS: u64 = 600;
const CALLBACK_PATH: &str = "/block/callback";

/// One-use browser authorization, bound to a loopback listener and PKCE verifier.
/// Dropping it closes the listener. Debug never reveals the URL, state or verifier.
pub struct PendingSignIn {
    listener: TcpListener,
    pub(super) client_id: String,
    authorization_url: String,
    redirect_uri: String,
    state: SecretString,
    pub(super) verifier: SecretString,
    deadline: Instant,
}

impl fmt::Debug for PendingSignIn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingSignIn")
            .field("authorization", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl PendingSignIn {
    /// Pass directly to the system browser. This URL contains ephemeral OAuth state.
    #[must_use]
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    /// The local callback URI (contains no state, code or credentials).
    #[must_use]
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub(super) async fn new(client_id: &str) -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| AuthError::CallbackUnavailable)?;
        let port = listener
            .local_addr()
            .map_err(|_| AuthError::CallbackUnavailable)?
            .port();
        let redirect_uri = format!("http://localhost:{port}{CALLBACK_PATH}");
        let verifier = random_secret()?;
        let state = random_secret()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose().as_bytes()));
        let mut url = Url::parse(AUTHORIZE).map_err(|_| AuthError::InvalidCallback)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", client_id),
            ("response_type", "code"),
            ("response_mode", "query"),
            ("redirect_uri", &redirect_uri),
            ("scope", SCOPE),
            ("state", state.expose()),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ]);
        Ok(Self {
            listener,
            client_id: client_id.to_owned(),
            authorization_url: url.into(),
            redirect_uri,
            state,
            verifier,
            deadline: Instant::now() + Duration::from_secs(CALLBACK_SECONDS),
        })
    }

    pub(super) async fn receive_code(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<SecretString> {
        loop {
            let accepted = tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(AuthError::Cancelled),
                () = tokio::time::sleep_until(self.deadline) => return Err(AuthError::TimedOut),
                result = self.listener.accept() => result.map_err(|_| AuthError::CallbackUnavailable)?,
            };
            if !accepted.1.ip().is_loopback() {
                continue;
            }
            let mut socket = accepted.0;
            let received = tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(AuthError::Cancelled),
                result = timeout(Duration::from_secs(3), read_request(&mut socket)) => result,
            };
            let result = received
                .ok()
                .and_then(std::result::Result::ok)
                .and_then(|request| parse_callback(&request, self.state.expose()).ok());
            let status = if result.is_some() {
                "200 OK"
            } else {
                "400 Bad Request"
            };
            let text = if result.is_some() {
                "Authorization received. Return to Block to finish signing in."
            } else {
                "Invalid authorization response. Return to the Microsoft sign-in tab."
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n<!doctype html><title>Block sign-in</title><p>{text}</p><script>history.replaceState(null,'','/complete')</script>"
            );
            let _ = timeout(
                Duration::from_secs(1),
                socket.write_all(response.as_bytes()),
            )
            .await;
            if let Some(result) = result {
                return result;
            }
        }
    }
}

fn random_secret() -> Result<SecretString> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| AuthError::RandomnessUnavailable)?;
    Ok(SecretString::new(URL_SAFE_NO_PAD.encode(bytes.as_slice())))
}

async fn read_request(socket: &mut TcpStream) -> std::io::Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::new());
    let mut chunk = [0_u8; 1024];
    while bytes.len() < 8192 {
        let count = socket.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|value| value == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(bytes.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| std::io::Error::other("Invalid callback request"))
}

fn parse_callback(request: &str, expected_state: &str) -> Result<Result<SecretString>> {
    let first = request.lines().next().ok_or(AuthError::InvalidCallback)?;
    let mut parts = first.split_whitespace();
    if parts.next() != Some("GET") {
        return Err(AuthError::InvalidCallback);
    }
    let target = parts.next().ok_or(AuthError::InvalidCallback)?;
    if !target.starts_with(&format!("{CALLBACK_PATH}?")) {
        return Err(AuthError::InvalidCallback);
    }
    let url =
        Url::parse(&format!("http://localhost{target}")).map_err(|_| AuthError::InvalidCallback)?;
    let fields: Vec<_> = url.query_pairs().collect();
    let unique = |key: &str| {
        let mut values = fields
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.as_ref());
        let value = values.next();
        if values.next().is_some() { None } else { value }
    };
    if !unique("state")
        .is_some_and(|state| bool::from(state.as_bytes().ct_eq(expected_state.as_bytes())))
    {
        return Err(AuthError::InvalidCallback);
    }
    if unique("error").is_some() {
        return Ok(Err(AuthError::Cancelled));
    }
    let code = unique("code")
        .filter(|value| !value.is_empty())
        .ok_or(AuthError::InvalidCallback)?;
    Ok(Ok(SecretString::new(code.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_rejects_state_substitution_duplicates_and_wrong_path() {
        for request in [
            "GET /block/callback?state=wrong&code=test HTTP/1.1",
            "GET /block/callback?state=expected&state=expected&code=test HTTP/1.1",
            "GET /wrong?state=expected&code=test HTTP/1.1",
            "GET /callback?state=expected&code=test HTTP/1.1",
            "GET /block/callback?state=expected&code=one&code=two HTTP/1.1",
        ] {
            assert_eq!(
                parse_callback(request, "expected").err(),
                Some(AuthError::InvalidCallback)
            );
        }
    }

    #[test]
    fn callback_accepts_only_matching_state_and_handles_denial() -> Result<()> {
        let code = parse_callback(
            "GET /block/callback?state=expected&code=test-code HTTP/1.1",
            "expected",
        )??;
        assert_eq!(code.expose(), "test-code");
        assert_eq!(
            parse_callback(
                "GET /block/callback?state=expected&error=access_denied HTTP/1.1",
                "expected"
            )?
            .err(),
            Some(AuthError::Cancelled)
        );
        Ok(())
    }

    #[tokio::test]
    async fn pkce_uses_sha256_and_redacts_debug() -> Result<()> {
        let pending = PendingSignIn::new("00000000-0000-0000-0000-000000000001").await?;
        let url =
            Url::parse(pending.authorization_url()).map_err(|_| AuthError::InvalidCallback)?;
        let redirect =
            Url::parse(pending.redirect_uri()).map_err(|_| AuthError::InvalidCallback)?;
        assert_eq!(redirect.path(), "/block/callback");
        let fields: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(
            fields.get("redirect_uri").map(AsRef::as_ref),
            Some(pending.redirect_uri())
        );
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(pending.verifier.expose()));
        assert_eq!(
            fields.get("code_challenge").map(AsRef::as_ref),
            Some(challenge.as_str())
        );
        let debug = format!("{pending:?}");
        assert!(!debug.contains(pending.state.expose()));
        assert!(!debug.contains(pending.verifier.expose()));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            pending.receive_code(&cancel).await.err(),
            Some(AuthError::Cancelled)
        );
        Ok(())
    }

    #[tokio::test]
    async fn listener_ignores_invalid_state_then_consumes_matching_callback()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let pending = PendingSignIn::new("00000000-0000-0000-0000-000000000001").await?;
        let port = pending.listener.local_addr()?.port();
        let state = pending.state.expose().to_owned();
        let receiver =
            tokio::spawn(async move { pending.receive_code(&CancellationToken::new()).await });
        let client = reqwest::Client::builder().no_proxy().build()?;
        let legacy = client
            .get(format!("http://127.0.0.1:{port}/callback"))
            .query(&[("state", state.as_str()), ("code", "synthetic-legacy")])
            .send()
            .await?;
        assert_eq!(legacy.status(), reqwest::StatusCode::BAD_REQUEST);
        let invalid = client
            .get(format!(
                "http://127.0.0.1:{port}/block/callback?state=wrong&code=synthetic"
            ))
            .send()
            .await?;
        assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
        let accepted = client
            .get(format!("http://127.0.0.1:{port}/block/callback"))
            .query(&[("state", state.as_str()), ("code", "synthetic-code")])
            .send()
            .await?;
        assert_eq!(accepted.status(), reqwest::StatusCode::OK);
        assert_eq!(
            accepted
                .headers()
                .get("referrer-policy")
                .and_then(|value| value.to_str().ok()),
            Some("no-referrer")
        );
        assert_eq!(receiver.await??.expose(), "synthetic-code");
        Ok(())
    }
}
