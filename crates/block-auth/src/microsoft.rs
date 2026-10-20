use std::time::{Duration, SystemTime};

use reqwest::{Client, RequestBuilder, StatusCode};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    Account, AccountProvider, AccountRef, AuthError, AuthenticatedSession,
    MICROSOFT_SCOPE as SCOPE, PendingSignIn, Result, SecretString, credentials,
};

const TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBOX: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const ENTITLEMENTS: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Public desktop Microsoft authentication. No client-secret setting exists.
#[derive(Clone)]
pub struct MicrosoftAuth {
    client_id: String,
    client: Client,
    test_base_url: Option<String>,
}

impl std::fmt::Debug for MicrosoftAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MicrosoftAuth")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl MicrosoftAuth {
    /// Configure a public desktop application registered for personal accounts.
    ///
    /// # Errors
    /// Fails if the client ID is missing/invalid or TLS client initialization fails.
    pub fn new(client_id: &str) -> Result<Self> {
        if client_id.trim().is_empty() {
            return Err(AuthError::MissingClientId);
        }
        let parsed = Uuid::parse_str(client_id.trim()).map_err(|_| AuthError::InvalidClientId)?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(45))
            .user_agent("Block/0.1 (desktop authentication)")
            .build()
            .map_err(|_| AuthError::Network {
                stage: "HTTP client initialization",
            })?;
        Ok(Self {
            client_id: parsed.to_string(),
            client,
            test_base_url: None,
        })
    }

    /// Bind a one-use localhost callback and prepare a PKCE URL for the system browser.
    /// Register `http://localhost/callback` as a Mobile/Desktop application redirect.
    /// Microsoft ignores the ephemeral port for localhost desktop redirects.
    ///
    /// # Errors
    /// Returns a secure-store, local callback or operating-system randomness error.
    pub async fn begin_sign_in(&self) -> Result<PendingSignIn> {
        credentials::probe_secure_storage().await?;
        PendingSignIn::new(&self.client_id).await
    }

    /// Finish browser authorization, verify Java ownership/profile and securely save
    /// the refresh token. No usable account is returned if secure storage fails.
    ///
    /// # Errors
    /// Returns explicit cancellation, authorization, ownership, network or secure-store errors.
    pub async fn complete_sign_in(
        &self,
        pending: PendingSignIn,
        cancellation: &CancellationToken,
    ) -> Result<AuthenticatedSession> {
        if pending.client_id != self.client_id {
            return Err(AuthError::ClientIdChanged);
        }
        let code = pending.receive_code(cancellation).await?;
        let microsoft: MicrosoftTokens = self
            .request(
                self.client.post(self.endpoint(TOKEN)).form(&[
                    ("client_id", self.client_id.as_str()),
                    ("grant_type", "authorization_code"),
                    ("code", code.expose()),
                    ("redirect_uri", pending.redirect_uri()),
                    ("code_verifier", pending.verifier.expose()),
                    ("scope", SCOPE),
                ]),
                "Microsoft authorization",
                cancellation,
            )
            .await?;
        let refresh = microsoft.refresh_token.ok_or(AuthError::SignInRequired)?;
        validate_secret(&refresh, "Microsoft authorization")?;
        let session = self
            .minecraft_session(&microsoft.access_token, cancellation)
            .await?;
        credentials::save(session.account.credential_ref.clone(), refresh).await?;
        Ok(session)
    }

    /// Recheck ownership/profile while obtaining a fresh session from secure storage.
    /// A rotated Microsoft refresh token is saved before subsequent network requests,
    /// so a transient Xbox/Minecraft outage does not discard the new credential.
    ///
    /// # Errors
    /// Missing/revoked credentials require sign-in; secure-store/network errors stay distinct.
    pub async fn refresh(
        &self,
        account: &Account,
        cancellation: &CancellationToken,
    ) -> Result<AuthenticatedSession> {
        self.validate_account(account)?;
        let saved = credentials::load(account).await?;
        let microsoft: MicrosoftTokens = self
            .request(
                self.client.post(self.endpoint(TOKEN)).form(&[
                    ("client_id", self.client_id.as_str()),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", saved.expose()),
                    ("scope", SCOPE),
                ]),
                "Microsoft refresh",
                cancellation,
            )
            .await?;
        if let Some(rotated) = microsoft.refresh_token {
            validate_secret(&rotated, "Microsoft refresh")?;
            credentials::save(account.credential_ref.clone(), rotated).await?;
        }
        let session = self
            .minecraft_session(&microsoft.access_token, cancellation)
            .await?;
        if session.account.reference != account.reference {
            return Err(AuthError::IdentityChanged);
        }
        Ok(session)
    }

    /// Delete this app's refresh credential. Remove metadata only after success.
    /// Already missing credentials count as signed out. Existing Java processes keep
    /// their in-memory session until exit; sign-out is not a remote token-revocation API.
    ///
    /// # Errors
    /// Returns a secure-store error if deletion cannot be confirmed.
    pub async fn sign_out(&self, account: &Account) -> Result<()> {
        if account.credential_ref != credential_ref(&account.client_id, account.reference.player_id)
        {
            return Err(AuthError::CredentialsMissing);
        }
        credentials::delete(account).await
    }

    fn endpoint(&self, official: &'static str) -> String {
        if let Some(base) = &self.test_base_url {
            return format!("{base}/{}", official.rsplit('/').next().unwrap_or_default());
        }
        official.to_owned()
    }

    fn validate_account(&self, account: &Account) -> Result<()> {
        if account.client_id != self.client_id {
            return Err(AuthError::ClientIdChanged);
        }
        if account.credential_ref != credential_ref(&self.client_id, account.reference.player_id) {
            return Err(AuthError::CredentialsMissing);
        }
        Ok(())
    }

    async fn minecraft_session(
        &self,
        microsoft: &SecretString,
        cancellation: &CancellationToken,
    ) -> Result<AuthenticatedSession> {
        validate_secret(microsoft, "Microsoft authorization")?;
        let user: XboxTokens = self.request(self.client.post(self.endpoint(XBOX)).header("x-xbl-contract-version", "1").json(&json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={}", microsoft.expose()) },
            "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT",
        })), "Xbox authentication", cancellation).await?;
        let user_hash = user.user_hash()?;
        let xsts: XboxTokens = self.request(self.client.post(self.endpoint(XSTS)).header("x-xbl-contract-version", "1").json(&json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [user.token.expose()] },
            "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT",
        })), "Xbox authorization", cancellation).await?;
        let xsts_hash = xsts.user_hash()?;
        if user_hash != xsts_hash {
            return Err(AuthError::IdentityChanged);
        }
        let minecraft: MinecraftTokens = self
            .request(
                self.client.post(self.endpoint(MINECRAFT)).json(&json!({
                    "identityToken": format!("XBL3.0 x={xsts_hash};{}", xsts.token.expose()),
                })),
                "Minecraft authentication",
                cancellation,
            )
            .await?;
        validate_secret(&minecraft.access_token, "Minecraft authentication")?;
        if !minecraft.token_type.eq_ignore_ascii_case("bearer")
            || minecraft.expires_in == 0
            || minecraft.expires_in > 86_400
        {
            return Err(AuthError::InvalidResponse {
                stage: "Minecraft authentication",
            });
        }
        let expires_at = SystemTime::now()
            .checked_add(Duration::from_secs(minecraft.expires_in))
            .ok_or(AuthError::InvalidResponse {
                stage: "Minecraft authentication",
            })?;
        let entitlements: Entitlements = self
            .request(
                self.client
                    .get(self.endpoint(ENTITLEMENTS))
                    .bearer_auth(minecraft.access_token.expose()),
                "Minecraft ownership",
                cancellation,
            )
            .await?;
        entitlements.validate()?;
        let profile: MinecraftProfile = self
            .request(
                self.client
                    .get(self.endpoint(PROFILE))
                    .bearer_auth(minecraft.access_token.expose()),
                "Minecraft profile",
                cancellation,
            )
            .await?;
        let player_id = profile.validate()?;
        let account = Account {
            reference: AccountRef { player_id },
            provider: AccountProvider::Microsoft,
            player_name: profile.name,
            client_id: self.client_id.clone(),
            credential_ref: credential_ref(&self.client_id, player_id),
        };
        Ok(AuthenticatedSession {
            account,
            expires_at,
            access_token: minecraft.access_token,
        })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        stage: &'static str,
        cancellation: &CancellationToken,
    ) -> Result<T> {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(AuthError::Cancelled),
            result = request_json(request, stage) => result,
        }
    }
}

fn credential_ref(client_id: &str, player_id: Uuid) -> String {
    format!("{client_id}:{}", player_id.simple())
}

fn validate_secret(secret: &SecretString, stage: &'static str) -> Result<()> {
    if secret.expose().is_empty() {
        Err(AuthError::InvalidResponse { stage })
    } else {
        Ok(())
    }
}

async fn request_json<T: DeserializeOwned>(
    request: RequestBuilder,
    stage: &'static str,
) -> Result<T> {
    let mut response = request
        .send()
        .await
        .map_err(|_| AuthError::Network { stage })?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(AuthError::InvalidResponse { stage });
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AuthError::Network { stage })?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(AuthError::InvalidResponse { stage });
        }
        body.extend_from_slice(&chunk);
    }
    decode_response(status, &body, stage)
}

fn decode_response<T: DeserializeOwned>(
    status: StatusCode,
    body: &[u8],
    stage: &'static str,
) -> Result<T> {
    if !status.is_success() {
        let code = serde_json::from_slice::<ServiceError>(body).ok();
        return Err(classify_error(status, stage, code.as_ref()));
    }
    serde_json::from_slice(body).map_err(|_| AuthError::InvalidResponse { stage })
}

fn classify_error(
    status: StatusCode,
    stage: &'static str,
    error: Option<&ServiceError>,
) -> AuthError {
    match error.and_then(|value| value.error.as_deref()) {
        Some("invalid_grant" | "interaction_required" | "login_required") => {
            return AuthError::SignInRequired;
        }
        Some("invalid_client" | "unauthorized_client") => {
            return AuthError::ApplicationRegistration;
        }
        Some("access_denied") => return AuthError::Cancelled,
        _ => {}
    }
    match error.and_then(|value| value.xerr) {
        Some(2_148_916_233) => return AuthError::XboxProfileRequired,
        Some(2_148_916_235..=2_148_916_238) => return AuthError::XboxAccountRestricted,
        _ => {}
    }
    if stage == "Minecraft profile" && status == StatusCode::NOT_FOUND {
        return AuthError::NoMinecraftProfile;
    }
    AuthError::Http {
        stage,
        status: status.as_u16(),
    }
}

#[derive(Deserialize)]
struct ServiceError {
    error: Option<String>,
    #[serde(rename = "XErr")]
    xerr: Option<u64>,
}

#[derive(Deserialize)]
struct MicrosoftTokens {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
}

#[derive(Deserialize)]
struct XboxTokens {
    #[serde(rename = "Token")]
    token: SecretString,
    #[serde(rename = "DisplayClaims")]
    claims: XboxClaims,
}

#[derive(Deserialize)]
struct XboxClaims {
    xui: Vec<XboxUser>,
}
#[derive(Deserialize)]
struct XboxUser {
    uhs: String,
}

impl XboxTokens {
    fn user_hash(&self) -> Result<&str> {
        validate_secret(&self.token, "Xbox authorization")?;
        let user = self
            .claims
            .xui
            .first()
            .filter(|_| self.claims.xui.len() == 1)
            .ok_or(AuthError::InvalidResponse {
                stage: "Xbox authorization",
            })?;
        if user.uhs.is_empty() || !user.uhs.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(AuthError::InvalidResponse {
                stage: "Xbox authorization",
            });
        }
        Ok(&user.uhs)
    }
}

#[derive(Deserialize)]
struct MinecraftTokens {
    access_token: SecretString,
    token_type: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct Entitlements {
    items: Vec<Entitlement>,
}
#[derive(Deserialize)]
struct Entitlement {
    name: String,
}

impl Entitlements {
    fn validate(&self) -> Result<()> {
        if self.items.iter().any(|item| item.name == "game_minecraft") {
            Ok(())
        } else {
            Err(AuthError::NoJavaOwnership)
        }
    }
}

#[derive(Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}
impl MinecraftProfile {
    fn validate(&self) -> Result<Uuid> {
        if self.id.len() != 32
            || self.name.is_empty()
            || self.name.len() > 16
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(AuthError::InvalidResponse {
                stage: "Minecraft profile",
            });
        }
        Uuid::parse_str(&self.id).map_err(|_| AuthError::InvalidResponse {
            stage: "Minecraft profile",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_never_echo_provider_descriptions_or_unrecognized_codes() {
        let body = br#"{"error":"invalid_grant","error_description":"SYNTHETIC_PRIVATE_TOKEN","access_token":"SYNTHETIC_PRIVATE_TOKEN"}"#;
        let result =
            decode_response::<MicrosoftTokens>(StatusCode::BAD_REQUEST, body, "Microsoft refresh");
        assert_eq!(result.err(), Some(AuthError::SignInRequired));
        let error = classify_error(
            StatusCode::FORBIDDEN,
            "Minecraft authentication",
            Some(&ServiceError {
                error: Some("SYNTHETIC_PRIVATE_TOKEN".into()),
                xerr: None,
            }),
        );
        assert!(!format!("{error:?} {error}").contains("SYNTHETIC_PRIVATE_TOKEN"));
        assert_eq!(
            error,
            AuthError::Http {
                stage: "Minecraft authentication",
                status: 403
            }
        );
    }

    #[test]
    fn access_token_does_not_substitute_for_java_entitlement() -> Result<()> {
        for body in [
            br#"{"items":[]}"#.as_slice(),
            br#"{"items":[{"name":"product_bedrock"}]}"#.as_slice(),
        ] {
            let result: Entitlements =
                decode_response(StatusCode::OK, body, "Minecraft ownership")?;
            assert_eq!(result.validate(), Err(AuthError::NoJavaOwnership));
        }
        let owned: Entitlements = decode_response(
            StatusCode::OK,
            br#"{"items":[{"name":"game_minecraft"}]}"#,
            "Minecraft ownership",
        )?;
        owned.validate()
    }

    #[test]
    fn validates_profile_and_xbox_claims() -> Result<()> {
        let profile: MinecraftProfile = decode_response(
            StatusCode::OK,
            br#"{"id":"00112233445566778899aabbccddeeff","name":"Test_Player"}"#,
            "Minecraft profile",
        )?;
        assert_eq!(
            profile.validate()?.simple().to_string(),
            "00112233445566778899aabbccddeeff"
        );
        assert!(
            MinecraftProfile {
                id: "not-uuid".into(),
                name: "../escape".into()
            }
            .validate()
            .is_err()
        );
        let xbox: XboxTokens = decode_response(
            StatusCode::OK,
            br#"{"Token":"synthetic","DisplayClaims":{"xui":[{"uhs":"123456"}]}}"#,
            "Xbox authorization",
        )?;
        assert_eq!(xbox.user_hash()?, "123456");
        Ok(())
    }

    #[test]
    fn missing_and_malformed_response_fields_fail_closed() {
        for body in [
            b"{}".as_slice(),
            b"not json".as_slice(),
            br#"{"access_token":123}"#.as_slice(),
        ] {
            assert_eq!(
                decode_response::<MicrosoftTokens>(StatusCode::OK, body, "Microsoft authorization")
                    .err(),
                Some(AuthError::InvalidResponse {
                    stage: "Microsoft authorization"
                })
            );
        }
    }

    async fn scripted_server(
        bodies: Vec<(u16, &'static str)>,
    ) -> std::result::Result<
        (
            String,
            tokio::task::JoinHandle<std::io::Result<Vec<String>>>,
        ),
        std::io::Error,
    > {
        use tokio::{
            io::{AsyncReadExt as _, AsyncWriteExt as _},
            net::TcpListener,
        };
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let base = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in bodies {
                let (mut stream, _) = listener.accept().await?;
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let count = stream.read(&mut chunk).await?;
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or_default();
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(
                    String::from_utf8(bytes)
                        .map_err(|_| std::io::Error::other("Invalid synthetic request"))?,
                );
                let response = format!(
                    "HTTP/1.1 {status} Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await?;
            }
            Ok(requests)
        });
        Ok((base, task))
    }

    #[tokio::test]
    async fn complete_http_chain_checks_identity_entitlement_and_expiring_session()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (base, server) = scripted_server(vec![
            (
                200,
                r#"{"Token":"synthetic-xbox","DisplayClaims":{"xui":[{"uhs":"123"}]}}"#,
            ),
            (
                200,
                r#"{"Token":"synthetic-xsts","DisplayClaims":{"xui":[{"uhs":"123"}]}}"#,
            ),
            (
                200,
                r#"{"access_token":"synthetic-minecraft","token_type":"Bearer","expires_in":3600}"#,
            ),
            (
                200,
                r#"{"items":[{"name":"game_minecraft"},{"name":"product_minecraft"}]}"#,
            ),
            (
                200,
                r#"{"id":"00112233445566778899aabbccddeeff","name":"SyntheticUser"}"#,
            ),
        ])
        .await?;
        let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        auth.test_base_url = Some(base);
        let session = auth
            .minecraft_session(
                &SecretString::new("synthetic-microsoft".into()),
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!(session.account.player_name, "SyntheticUser");
        assert_eq!(session.expose_access_token(), "synthetic-minecraft");
        assert!(!session.needs_refresh());
        assert!(!format!("{session:?}").contains("synthetic-minecraft"));
        let metadata = serde_json::to_string(&session.account)?;
        for secret in [
            "synthetic-microsoft",
            "synthetic-xbox",
            "synthetic-xsts",
            "synthetic-minecraft",
        ] {
            assert!(!metadata.contains(secret));
        }
        let requests = server.await??;
        assert_eq!(requests.len(), 5);
        assert!(requests[0].contains("d=synthetic-microsoft"));
        assert!(requests[1].contains("synthetic-xbox"));
        assert!(requests[2].contains("XBL3.0 x=123;synthetic-xsts"));
        assert!(requests[3].contains("Bearer synthetic-minecraft"));
        assert!(requests[4].contains("Bearer synthetic-minecraft"));
        Ok(())
    }

    #[tokio::test]
    async fn http_chain_stops_before_profile_if_ownership_is_missing()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (base, server) = scripted_server(vec![
            (
                200,
                r#"{"Token":"synthetic-xbox","DisplayClaims":{"xui":[{"uhs":"123"}]}}"#,
            ),
            (
                200,
                r#"{"Token":"synthetic-xsts","DisplayClaims":{"xui":[{"uhs":"123"}]}}"#,
            ),
            (
                200,
                r#"{"access_token":"synthetic-minecraft","token_type":"Bearer","expires_in":3600}"#,
            ),
            (200, r#"{"items":[]}"#),
        ])
        .await?;
        let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        auth.test_base_url = Some(base);
        let result = auth
            .minecraft_session(
                &SecretString::new("synthetic".into()),
                &CancellationToken::new(),
            )
            .await;
        assert_eq!(result.err(), Some(AuthError::NoJavaOwnership));
        assert_eq!(server.await??.len(), 4);
        Ok(())
    }

    #[tokio::test]
    async fn http_error_is_sanitized_and_precancelled_request_is_not_sent()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (base, server) = scripted_server(vec![(
            401,
            r#"{"XErr":2148916233,"Message":"synthetic-private-response"}"#,
        )])
        .await?;
        let auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        let result = auth
            .request::<XboxTokens>(
                auth.client.get(&base),
                "Xbox authorization",
                &CancellationToken::new(),
            )
            .await;
        let error = result.err().ok_or("Expected synthetic error")?;
        assert_eq!(error, AuthError::XboxProfileRequired);
        assert!(!format!("{error:?} {error}").contains("synthetic-private-response"));
        assert_eq!(server.await??.len(), 1);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = auth
            .request::<MicrosoftTokens>(
                auth.client.get("http://127.0.0.1:1/unused"),
                "Microsoft authorization",
                &cancel,
            )
            .await;
        assert_eq!(result.err(), Some(AuthError::Cancelled));
        Ok(())
    }

    #[tokio::test]
    async fn xbox_identity_mismatch_fails_before_minecraft_exchange()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (base, server) = scripted_server(vec![
            (
                200,
                r#"{"Token":"synthetic-xbox","DisplayClaims":{"xui":[{"uhs":"123"}]}}"#,
            ),
            (
                200,
                r#"{"Token":"synthetic-xsts","DisplayClaims":{"xui":[{"uhs":"456"}]}}"#,
            ),
        ])
        .await?;
        let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        auth.test_base_url = Some(base);
        assert_eq!(
            auth.minecraft_session(
                &SecretString::new("synthetic".into()),
                &CancellationToken::new()
            )
            .await
            .err(),
            Some(AuthError::IdentityChanged)
        );
        assert_eq!(server.await??.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn callback_cannot_be_completed_using_another_client_registration() -> Result<()> {
        let pending = PendingSignIn::new("00000000-0000-0000-0000-000000000001").await?;
        let auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000002")?;
        assert_eq!(
            auth.complete_sign_in(pending, &CancellationToken::new())
                .await
                .err(),
            Some(AuthError::ClientIdChanged)
        );
        Ok(())
    }
}
