use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

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
const DEVICE_CODE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const XBOX: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const ENTITLEMENTS: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Observed work stages contain no URLs, account identifiers or credentials.
/// A stage begins immediately before the real operation; it is not a percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignInStage {
    /// Check the operating system credential store before authorization.
    SecureStorage,
    /// Request the short-lived code the user enters on Microsoft's website.
    DeviceAuthorization,
    /// Wait for the one-use localhost PKCE callback.
    BrowserAuthorization,
    /// Exchange the returned authorization code.
    MicrosoftAuthorization,
    /// Authenticate the Microsoft session with Xbox Live.
    XboxAuthentication,
    /// Authorize access to Minecraft services.
    XboxAuthorization,
    /// Obtain a Minecraft session.
    MinecraftAuthentication,
    /// Verify Java Edition entitlement.
    Ownership,
    /// Resolve and validate the player profile.
    PlayerProfile,
    /// Save the refresh credential in the operating system store.
    SaveCredential,
    /// Save public account metadata after the credential has been saved.
    SaveMetadata,
}

/// Public desktop Microsoft authentication. No client-secret setting exists.
#[derive(Clone)]
pub struct MicrosoftAuth {
    client_id: String,
    client: Client,
    test_base_url: Option<String>,
    stage_observer: Option<Arc<dyn Fn(SignInStage) + Send + Sync>>,
}

/// An authenticated identity awaiting the user's final save confirmation.
/// Refresh credentials remain memory-only and are zeroized if setup is abandoned.
#[derive(Debug)]
pub struct PreparedMicrosoftSignIn {
    session: AuthenticatedSession,
    refresh_token: SecretString,
}

impl PreparedMicrosoftSignIn {
    #[must_use]
    pub const fn account(&self) -> &Account {
        &self.session.account
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.session.needs_refresh()
    }
}

/// Transient device authorization state. The polling secret is never displayed,
/// serialized or included in formatting; the user code is exposed explicitly.
pub struct PendingDeviceSignIn {
    client_id: String,
    device_code: SecretString,
    user_code: String,
    verification_uri: String,
    interval: Duration,
    expires_at: tokio::time::Instant,
}

impl std::fmt::Debug for PendingDeviceSignIn {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PendingDeviceSignIn([REDACTED])")
    }
}

impl PendingDeviceSignIn {
    #[must_use]
    pub fn user_code(&self) -> &str {
        &self.user_code
    }

    #[must_use]
    pub fn verification_uri(&self) -> &str {
        &self.verification_uri
    }

    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.expires_at
            .saturating_duration_since(tokio::time::Instant::now())
    }
}

#[derive(Deserialize)]
struct DeviceAuthorization {
    device_code: SecretString,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

enum DevicePoll {
    Authorized(MicrosoftTokens),
    Pending,
    SlowDown(Duration),
    ConnectionTimeout,
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
            stage_observer: None,
        })
    }

    /// Observe actual authentication work without exposing secret request data.
    #[must_use]
    pub fn with_stage_observer(mut self, observer: Arc<dyn Fn(SignInStage) + Send + Sync>) -> Self {
        self.stage_observer = Some(observer);
        self
    }

    fn stage(&self, stage: SignInStage) {
        if let Some(observer) = &self.stage_observer {
            observer(stage);
        }
    }

    /// Bind a one-use localhost callback and prepare a PKCE URL for the system browser.
    /// Register `http://localhost/block/callback` as a Mobile/Desktop application redirect.
    /// Microsoft ignores the ephemeral port for localhost desktop redirects.
    ///
    /// # Errors
    /// Returns a secure-store, local callback or operating-system randomness error.
    pub async fn begin_sign_in(&self) -> Result<PendingSignIn> {
        self.stage(SignInStage::SecureStorage);
        credentials::probe_secure_storage().await?;
        PendingSignIn::new(&self.client_id).await
    }

    /// Finish browser authorization and verify Java ownership/profile. The result
    /// is not persisted until the user confirms it with `save_sign_in`.
    ///
    /// # Errors
    /// Returns explicit cancellation, authorization, ownership, network or secure-store errors.
    pub async fn complete_sign_in(
        &self,
        pending: PendingSignIn,
        cancellation: &CancellationToken,
    ) -> Result<PreparedMicrosoftSignIn> {
        if pending.client_id != self.client_id {
            return Err(AuthError::ClientIdChanged);
        }
        self.stage(SignInStage::BrowserAuthorization);
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
        Ok(PreparedMicrosoftSignIn {
            session,
            refresh_token: refresh,
        })
    }

    /// Request a new device code only after the user chooses this sign-in method.
    /// # Errors
    /// Returns secure-store, cancellation, network or malformed-response errors.
    pub async fn begin_device_sign_in(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<PendingDeviceSignIn> {
        self.stage(SignInStage::SecureStorage);
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(AuthError::Cancelled),
            result = credentials::probe_secure_storage() => result?,
        }
        self.request_device_code(cancellation).await
    }

    async fn request_device_code(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<PendingDeviceSignIn> {
        self.stage(SignInStage::DeviceAuthorization);
        let started = tokio::time::Instant::now();
        let response: DeviceAuthorization = self
            .request(
                self.client
                    .post(self.endpoint(DEVICE_CODE))
                    .form(&[("client_id", self.client_id.as_str()), ("scope", SCOPE)]),
                "Microsoft device authorization",
                cancellation,
            )
            .await?;
        validate_secret(&response.device_code, "Microsoft device authorization")?;
        let invalid = || AuthError::InvalidResponse {
            stage: "Microsoft device authorization",
        };
        let interval = response.interval.unwrap_or(5);
        let uri = url::Url::parse(&response.verification_uri).map_err(|_| invalid())?;
        if !(1..=3600).contains(&response.expires_in)
            || !(1..=3600).contains(&interval)
            || response.user_code.is_empty()
            || response.user_code.len() > 64
            || !response
                .user_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || response.verification_uri.len() > 2048
            || uri.scheme() != "https"
            || !matches!(
                uri.host_str(),
                Some(
                    "microsoft.com"
                        | "www.microsoft.com"
                        | "login.microsoftonline.com"
                        | "login.live.com"
                )
            )
            || !uri.username().is_empty()
            || uri.password().is_some()
            || uri.port().is_some()
            || uri.fragment().is_some()
        {
            return Err(invalid());
        }
        let expires_at = started
            .checked_add(Duration::from_secs(response.expires_in))
            .ok_or_else(invalid)?;
        if expires_at <= tokio::time::Instant::now() {
            return Err(AuthError::DeviceCodeExpired);
        }
        Ok(PendingDeviceSignIn {
            client_id: self.client_id.clone(),
            device_code: response.device_code,
            user_code: response.user_code,
            verification_uri: uri.to_string(),
            interval: Duration::from_secs(interval),
            expires_at,
        })
    }

    /// Poll at the provider interval, honoring slow_down, throttling and expiry,
    /// then reuse the same Xbox/Minecraft ownership chain as browser PKCE.
    /// # Errors
    /// Returns explicit cancellation/expiry, registration, ownership or network errors.
    pub async fn complete_device_sign_in(
        &self,
        pending: PendingDeviceSignIn,
        cancellation: &CancellationToken,
    ) -> Result<PreparedMicrosoftSignIn> {
        let tokens = self.poll_device_token(pending, cancellation).await?;
        self.stage(SignInStage::MicrosoftAuthorization);
        let refresh_token = tokens.refresh_token.ok_or(AuthError::SignInRequired)?;
        validate_secret(&refresh_token, "Microsoft authorization")?;
        let session = self
            .minecraft_session(&tokens.access_token, cancellation)
            .await?;
        Ok(PreparedMicrosoftSignIn {
            session,
            refresh_token,
        })
    }

    async fn poll_device_token(
        &self,
        pending: PendingDeviceSignIn,
        cancellation: &CancellationToken,
    ) -> Result<MicrosoftTokens> {
        if pending.client_id != self.client_id {
            return Err(AuthError::ClientIdChanged);
        }
        let mut interval = pending.interval;
        loop {
            let now = tokio::time::Instant::now();
            if now >= pending.expires_at {
                return Err(AuthError::DeviceCodeExpired);
            }
            let wake = now
                .checked_add(interval)
                .unwrap_or(pending.expires_at)
                .min(pending.expires_at);
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(AuthError::Cancelled),
                () = tokio::time::sleep_until(wake) => (),
            }
            if tokio::time::Instant::now() >= pending.expires_at {
                return Err(AuthError::DeviceCodeExpired);
            }
            let polled = tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(AuthError::Cancelled),
                () = tokio::time::sleep_until(pending.expires_at) => return Err(AuthError::DeviceCodeExpired),
                result = self.device_token_request(&pending) => result?,
            };
            match polled {
                DevicePoll::Authorized(tokens) => return Ok(tokens),
                DevicePoll::Pending => (),
                DevicePoll::SlowDown(retry_after) => {
                    interval = interval
                        .saturating_add(Duration::from_secs(5))
                        .max(retry_after)
                }
                DevicePoll::ConnectionTimeout => interval = interval.saturating_mul(2),
            }
        }
    }

    async fn device_token_request(&self, pending: &PendingDeviceSignIn) -> Result<DevicePoll> {
        let stage = "Microsoft device authorization";
        let mut response = match self
            .client
            .post(self.endpoint(TOKEN))
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", pending.device_code.expose()),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return Ok(DevicePoll::ConnectionTimeout),
            Err(_) => return Err(AuthError::Network { stage }),
        };
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .map(Duration::from_secs)
                    .or_else(|| {
                        httpdate::parse_http_date(value)
                            .ok()
                            .map(|date| date.duration_since(SystemTime::now()).unwrap_or_default())
                    })
            })
            .unwrap_or_default();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(AuthError::InvalidResponse { stage });
        }
        let mut body = Zeroizing::new(Vec::new());
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                        return Err(AuthError::InvalidResponse { stage });
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) if error.is_timeout() => return Ok(DevicePoll::ConnectionTimeout),
                Err(_) => return Err(AuthError::Network { stage }),
            }
        }
        if status.is_success() {
            let tokens: MicrosoftTokens = decode_response(status, &body, stage)?;
            validate_secret(&tokens.access_token, stage)?;
            return Ok(DevicePoll::Authorized(tokens));
        }
        let error = serde_json::from_slice::<ServiceError>(&body).ok();
        match error.as_ref().and_then(|value| value.error.as_deref()) {
            Some("authorization_pending") if status == StatusCode::BAD_REQUEST => {
                Ok(DevicePoll::Pending)
            }
            Some("slow_down") => Ok(DevicePoll::SlowDown(retry_after)),
            Some("authorization_declined" | "access_denied") => Err(AuthError::Cancelled),
            Some("expired_token") => Err(AuthError::DeviceCodeExpired),
            Some("bad_verification_code") => Err(AuthError::InvalidResponse { stage }),
            _ if status == StatusCode::TOO_MANY_REQUESTS => Ok(DevicePoll::SlowDown(retry_after)),
            _ => Err(classify_error(status, stage, error.as_ref())),
        }
    }

    /// Persist the prepared refresh token only after final user confirmation.
    /// # Errors
    /// Rejects expired preparation, client changes or unavailable secure storage.
    pub async fn save_sign_in(
        &self,
        prepared: PreparedMicrosoftSignIn,
    ) -> Result<AuthenticatedSession> {
        self.validate_account(&prepared.session.account)?;
        if prepared.is_expired() {
            return Err(AuthError::SignInRequired);
        }
        self.stage(SignInStage::SaveCredential);
        credentials::save(
            prepared.session.account.credential_ref.clone(),
            prepared.refresh_token,
        )
        .await?;
        Ok(prepared.session)
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
        if cancellation.is_cancelled() {
            return Err(AuthError::Cancelled);
        }
        let observed = match stage {
            "Microsoft authorization" => Some(SignInStage::MicrosoftAuthorization),
            "Xbox authentication" => Some(SignInStage::XboxAuthentication),
            "Xbox authorization" => Some(SignInStage::XboxAuthorization),
            "Minecraft authentication" => Some(SignInStage::MinecraftAuthentication),
            "Minecraft ownership" => Some(SignInStage::Ownership),
            "Minecraft profile" => Some(SignInStage::PlayerProfile),
            _ => None,
        };
        if let Some(observed) = observed {
            self.stage(observed);
        }
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
                let throttling = if status == 429 {
                    "Retry-After: 12\r\n"
                } else {
                    ""
                };
                let response = format!(
                    "HTTP/1.1 {status} Synthetic\r\n{throttling}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
        let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = stages.clone();
        auth = auth.with_stage_observer(Arc::new(move |stage| {
            observed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(stage);
        }));
        let session = auth
            .minecraft_session(
                &SecretString::new("synthetic-microsoft".into()),
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!(
            *stages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [
                SignInStage::XboxAuthentication,
                SignInStage::XboxAuthorization,
                SignInStage::MinecraftAuthentication,
                SignInStage::Ownership,
                SignInStage::PlayerProfile,
            ]
        );
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

    // Keep the paused runtime runnable while loopback sockets do real I/O.
    // Otherwise Tokio may jump directly to reqwest's timeout before the OS
    // delivers the next packet, rather than advancing just the polling clock.
    async fn device_fixture_io<F: Future>(request: F) -> F::Output {
        let mut request = std::pin::pin!(request);
        loop {
            tokio::select! {
                biased;
                result = request.as_mut() => return result,
                () = tokio::task::yield_now() => (),
            }
            tokio::time::advance(Duration::from_millis(5)).await;
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    const DEVICE: &str = r#"{"device_code":"synthetic-private-device","user_code":"ABCD-EFGH","verification_uri":"https://microsoft.com/devicelogin","expires_in":900,"interval":1}"#;

    #[tokio::test(start_paused = true)]
    async fn device_pending_slow_down_and_http_throttling_observe_real_poll_intervals()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (base, server) = scripted_server(vec![
            (200, DEVICE),
            (400, r#"{"error":"authorization_pending"}"#),
            (400, r#"{"error":"slow_down"}"#),
            (429, r#"{"error":"throttled"}"#),
            (
                200,
                r#"{"access_token":"synthetic-ms","refresh_token":"synthetic-refresh"}"#,
            ),
        ])
        .await?;
        let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        auth.test_base_url = Some(base);
        let cancel = CancellationToken::new();
        let pending = device_fixture_io(auth.request_device_code(&cancel)).await?;
        assert_eq!(pending.user_code(), "ABCD-EFGH");
        assert_eq!(
            pending.verification_uri(),
            "https://microsoft.com/devicelogin"
        );
        assert!(!format!("{pending:?}").contains("ABCD"));
        assert!(!format!("{pending:?}").contains("synthetic-private-device"));
        let started = tokio::time::Instant::now();
        let tokens = device_fixture_io(auth.poll_device_token(pending, &cancel)).await?;
        assert_eq!(tokens.access_token.expose(), "synthetic-ms");
        assert!(started.elapsed() >= Duration::from_secs(20));
        let requests = server.await??;
        assert_eq!(requests.len(), 5);
        assert!(requests[0].starts_with("POST /devicecode "));
        assert!(requests[0].contains("client_id=00000000-0000-0000-0000-000000000001"));
        assert!(requests[0].contains("scope="));
        for request in &requests[1..] {
            assert!(request.starts_with("POST /token "));
            assert!(
                request
                    .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code")
            );
            assert!(request.contains("device_code=synthetic-private-device"));
            assert!(!request.contains("client_secret"));
            assert!(!request.contains("user_code="));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn device_expiry_stops_before_the_default_five_second_first_poll()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (base, server) = scripted_server(vec![(200, r#"{"device_code":"synthetic","user_code":"TEST-CODE","verification_uri":"https://microsoft.com/devicelogin","expires_in":3}"#)]).await?;
        let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        auth.test_base_url = Some(base);
        let cancel = CancellationToken::new();
        let pending = device_fixture_io(auth.request_device_code(&cancel)).await?;
        assert_eq!(pending.interval, Duration::from_secs(5));
        assert_eq!(
            device_fixture_io(auth.poll_device_token(pending, &cancel))
                .await
                .err(),
            Some(AuthError::DeviceCodeExpired)
        );
        assert_eq!(server.await??.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn device_cancellation_and_client_changes_never_poll_or_save_credentials()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for change_client in [false, true] {
            let (base, server) = scripted_server(vec![(200, DEVICE)]).await?;
            let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
            auth.test_base_url = Some(base);
            let cancel = CancellationToken::new();
            let pending = auth.request_device_code(&cancel).await?;
            let expected = if change_client {
                auth.client_id = "00000000-0000-0000-0000-000000000002".into();
                AuthError::ClientIdChanged
            } else {
                cancel.cancel();
                AuthError::Cancelled
            };
            assert_eq!(
                auth.complete_device_sign_in(pending, &cancel).await.err(),
                Some(expected)
            );
            assert_eq!(server.await??.len(), 1);
        }
        let auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            auth.begin_device_sign_in(&cancel).await.err(),
            Some(AuthError::Cancelled)
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn device_provider_failures_stop_polling_without_echoing_response_bodies()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for (status, response, expected) in [
            (
                400,
                r#"{"error":"authorization_declined","error_description":"PRIVATE_DESCRIPTION"}"#,
                AuthError::Cancelled,
            ),
            (
                400,
                r#"{"error":"expired_token","error_description":"PRIVATE_DESCRIPTION"}"#,
                AuthError::DeviceCodeExpired,
            ),
            (
                400,
                r#"{"error":"invalid_client","error_description":"PRIVATE_DESCRIPTION"}"#,
                AuthError::ApplicationRegistration,
            ),
            (
                400,
                r#"{"error":"bad_verification_code","error_description":"PRIVATE_DESCRIPTION"}"#,
                AuthError::InvalidResponse {
                    stage: "Microsoft device authorization",
                },
            ),
            (
                503,
                r#"{"error":"authorization_pending","error_description":"PRIVATE_DESCRIPTION"}"#,
                AuthError::Http {
                    stage: "Microsoft device authorization",
                    status: 503,
                },
            ),
        ] {
            let (base, server) = scripted_server(vec![(200, DEVICE), (status, response)]).await?;
            let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
            auth.test_base_url = Some(base);
            let cancel = CancellationToken::new();
            let pending = device_fixture_io(auth.request_device_code(&cancel)).await?;
            let error = device_fixture_io(auth.poll_device_token(pending, &cancel))
                .await
                .err();
            assert_eq!(error, Some(expected));
            assert!(!format!("{error:?}").contains("PRIVATE_DESCRIPTION"));
            assert_eq!(server.await??.len(), 2);
        }
        Ok(())
    }

    #[tokio::test]
    async fn device_preparation_rejects_untrusted_links_and_invalid_expiry_or_interval()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for response in [
            r#"{"device_code":"synthetic","user_code":"TEST","verification_uri":"http://microsoft.com/devicelogin","expires_in":900}"#,
            r#"{"device_code":"synthetic","user_code":"TEST","verification_uri":"https://microsoft.com.evil.invalid/devicelogin","expires_in":900}"#,
            r#"{"device_code":"synthetic","user_code":"TEST","verification_uri":"https://microsoft.com/devicelogin","expires_in":0}"#,
            r#"{"device_code":"synthetic","user_code":"TEST","verification_uri":"https://microsoft.com/devicelogin","expires_in":900,"interval":0}"#,
            r#"{"device_code":"","user_code":"TEST","verification_uri":"https://microsoft.com/devicelogin","expires_in":900}"#,
        ] {
            let (base, server) = scripted_server(vec![(200, response)]).await?;
            let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
            auth.test_base_url = Some(base);
            assert_eq!(
                auth.request_device_code(&CancellationToken::new())
                    .await
                    .err(),
                Some(AuthError::InvalidResponse {
                    stage: "Microsoft device authorization"
                })
            );
            assert_eq!(server.await??.len(), 1);
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn device_approval_reuses_the_real_ownership_chain_and_only_prepares_the_identity()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (base, server) = scripted_server(vec![
            (200, DEVICE),
            (
                200,
                r#"{"access_token":"synthetic-ms","refresh_token":"synthetic-refresh"}"#,
            ),
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
            (200, r#"{"items":[{"name":"game_minecraft"}]}"#),
            (
                200,
                r#"{"id":"00112233445566778899aabbccddeeff","name":"SyntheticUser"}"#,
            ),
        ])
        .await?;
        let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
        auth.test_base_url = Some(base);
        let cancel = CancellationToken::new();
        let pending = device_fixture_io(auth.request_device_code(&cancel)).await?;
        let prepared = device_fixture_io(auth.complete_device_sign_in(pending, &cancel)).await?;
        assert_eq!(prepared.account().player_name, "SyntheticUser");
        assert!(!prepared.is_expired());
        let formatted = format!("{prepared:?}");
        assert!(!formatted.contains("synthetic-refresh"));
        assert!(!formatted.contains("synthetic-minecraft"));
        let requests = server.await??;
        assert_eq!(requests.len(), 7);
        assert!(requests[5].starts_with("GET /mcstore "));
        assert!(requests[6].starts_with("GET /profile "));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn device_cancellation_and_expiry_interrupt_an_inflight_token_request()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        device_fixture_io(async {
            for cancelled in [true, false] {
                let (base, server) = scripted_server(vec![(200, DEVICE)]).await?;
                let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
                auth.test_base_url = Some(base);
                let cancel = CancellationToken::new();
                let mut pending = auth.request_device_code(&cancel).await?;
                assert_eq!(server.await??.len(), 1);
                let listener =
                    tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
                auth.test_base_url = Some(format!("http://{}", listener.local_addr()?));
                pending.expires_at = tokio::time::Instant::now() + Duration::from_secs(3);
                let active_cancel = cancel.clone();
                let polled =
                    tokio::spawn(
                        async move { auth.poll_device_token(pending, &active_cancel).await },
                    );
                let (_held_connection, _) = listener.accept().await?;
                if cancelled {
                    cancel.cancel();
                }
                assert_eq!(
                    polled.await?.err(),
                    Some(if cancelled {
                        AuthError::Cancelled
                    } else {
                        AuthError::DeviceCodeExpired
                    })
                );
                assert!(
                    tokio::time::timeout(Duration::from_millis(1), listener.accept())
                        .await
                        .is_err()
                );
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn device_connection_timeout_backs_off_instead_of_repeating_at_the_old_interval()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        device_fixture_io(async {
            let (base, server) = scripted_server(vec![(200, DEVICE)]).await?;
            let mut auth = MicrosoftAuth::new("00000000-0000-0000-0000-000000000001")?;
            auth.test_base_url = Some(base);
            let cancel = CancellationToken::new();
            let mut pending = auth.request_device_code(&cancel).await?;
            assert_eq!(server.await??.len(), 1);
            let listener =
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
            auth.test_base_url = Some(format!("http://{}", listener.local_addr()?));
            auth.client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(1))
                .build()?;
            pending.expires_at = tokio::time::Instant::now() + Duration::from_millis(3500);
            let polled =
                tokio::spawn(async move { auth.poll_device_token(pending, &cancel).await });
            let (_held_connection, _) = listener.accept().await?;
            assert_eq!(polled.await?.err(), Some(AuthError::DeviceCodeExpired));
            // Without timeout backoff a second connection would be queued at
            // three seconds, before this grant expires at three and a half.
            assert!(
                tokio::time::timeout(Duration::from_millis(1), listener.accept())
                    .await
                    .is_err()
            );
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await
    }
}
