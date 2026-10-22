//! User-configured Yggdrasil providers. Discovery sends no account credentials.

use std::{
    collections::HashSet,
    time::{Duration, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    Account, AccountProvider, AccountRef, AuthError, AuthenticatedSession, AuthlibAccount, Result,
    SecretString, credentials,
};

const QUICK_LINK: &str = "authlib-injector:yggdrasil-server:";
const MAX_METADATA: usize = 512 * 1024;
const DISCOVERY_STAGE: &str = "Provider discovery";
const AUTH_STAGE: &str = "External authentication";
const PREPARATION_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AuthlibProfile {
    pub id: Uuid,
    pub name: String,
}

impl AuthlibProfile {
    fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || self.name.encode_utf16().count() > 16
            || self.name.chars().any(char::is_control)
        {
            return Err(AuthError::InvalidResponse { stage: AUTH_STAGE });
        }
        Ok(())
    }
}

/// A provider response waiting for character selection. Secrets remain transient.
pub struct PendingAuthlibSignIn {
    provider_id: Uuid,
    api_url: String,
    login: String,
    reply: AuthlibReply,
    expires_at: tokio::time::Instant,
}

impl std::fmt::Debug for PendingAuthlibSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingAuthlibSignIn([REDACTED])")
    }
}

impl PendingAuthlibSignIn {
    #[must_use]
    pub fn profiles(&self) -> &[AuthlibProfile] {
        &self.reply.available_profiles
    }

    #[must_use]
    pub fn selected_profile(&self) -> Option<&AuthlibProfile> {
        self.reply.selected_profile.as_ref()
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        tokio::time::Instant::now() >= self.expires_at
    }
}

/// A verified character awaiting Finish; dropping it writes no credentials.
pub struct PreparedAuthlibSignIn {
    session: AuthenticatedSession,
    tokens: TokenBundle,
    api_url: String,
    expires_at: tokio::time::Instant,
}

impl std::fmt::Debug for PreparedAuthlibSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreparedAuthlibSignIn([REDACTED])")
    }
}

impl PreparedAuthlibSignIn {
    #[must_use]
    pub const fn account(&self) -> &Account {
        &self.session.account
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        tokio::time::Instant::now() >= self.expires_at
    }
}

#[derive(Clone, Debug)]
pub struct AuthlibAuth {
    provider: AuthlibProvider,
    client: Client,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthlibReply {
    access_token: SecretString,
    client_token: SecretString,
    #[serde(default)]
    available_profiles: Vec<AuthlibProfile>,
    selected_profile: Option<AuthlibProfile>,
    user: Option<AuthlibUser>,
}

#[derive(Clone, Deserialize, Serialize)]
struct AuthlibUser {
    id: Option<Uuid>,
    #[serde(default)]
    properties: Vec<UserProperty>,
}

#[derive(Clone, Deserialize, Serialize)]
struct UserProperty {
    name: String,
    value: String,
}

// Only the encoded SecretString is passed to the OS store. No serializable
// session/token type is exported, and account metadata contains neither token.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenBundle {
    schema_version: u32,
    account_key: String,
    api_url: String,
    access_token: SecretString,
    client_token: SecretString,
    user: Option<AuthlibUser>,
}

impl AuthlibAuth {
    /// Bind authentication to an already discovered and explicitly accepted API.
    /// POST requests never follow redirects or ALI to another endpoint.
    pub fn new(provider: &AuthlibProvider) -> Result<Self> {
        provider.validate()?;
        let builder = Client::builder();
        #[cfg(test)]
        let builder = builder.no_proxy();
        let client = builder
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("Block/0.1 (external authentication)")
            .build()
            .map_err(auth_network)?;
        Ok(Self {
            provider: provider.clone(),
            client,
        })
    }

    /// Authenticate without persisting a password or either token.
    pub async fn authenticate(
        &self,
        login: &str,
        password: SecretString,
        cancel: &CancellationToken,
    ) -> Result<PendingAuthlibSignIn> {
        validate_login(login)?;
        if password.expose().is_empty() || password.expose().len() > 4096 {
            return Err(AuthError::ProviderLoginRequired);
        }
        tokio::select! {biased; ()=cancel.cancelled()=>return Err(AuthError::Cancelled), result=credentials::probe_authlib_storage()=>result?,}
        let client_token = Uuid::new_v4().simple().to_string();
        let reply = self.token_request("authenticate",&serde_json::json!({"username":login,"password":password.expose(),"clientToken":client_token,"requestUser":true,"agent":{"name":"Minecraft","version":1}}),cancel).await?;
        if reply.client_token.expose() != client_token {
            return Err(AuthError::ProviderIdentityChanged);
        }
        if reply.selected_profile.is_none() && reply.available_profiles.is_empty() {
            return Err(AuthError::ProviderNoProfiles);
        }
        Ok(PendingAuthlibSignIn {
            provider_id: self.provider.id,
            api_url: self.provider.api_url.clone(),
            login: login.to_owned(),
            reply,
            expires_at: tokio::time::Instant::now() + PREPARATION_TTL,
        })
    }

    /// A profile assignment is a real refresh, never a local token relabeling.
    pub async fn prepare_sign_in(
        &self,
        pending: &PendingAuthlibSignIn,
        selection: Option<Uuid>,
        cancel: &CancellationToken,
    ) -> Result<PreparedAuthlibSignIn> {
        if cancel.is_cancelled() {
            return Err(AuthError::Cancelled);
        }
        if pending.is_expired() {
            return Err(AuthError::ProviderPreparationExpired);
        }
        if pending.provider_id != self.provider.id || pending.api_url != self.provider.api_url {
            return Err(AuthError::ProviderIdentityChanged);
        }
        let reply = if let Some(bound) = &pending.reply.selected_profile {
            if selection.is_some_and(|id| id != bound.id) {
                return Err(AuthError::ProviderIdentityChanged);
            }
            AuthlibReply {
                access_token: SecretString::new(pending.reply.access_token.expose().to_owned()),
                client_token: SecretString::new(pending.reply.client_token.expose().to_owned()),
                available_profiles: pending.reply.available_profiles.clone(),
                selected_profile: Some(bound.clone()),
                user: pending.reply.user.clone(),
            }
        } else {
            let id = selection
                .or_else(|| {
                    (pending.reply.available_profiles.len() == 1)
                        .then(|| pending.reply.available_profiles[0].id)
                })
                .ok_or(AuthError::ProviderProfileSelectionRequired)?;
            let profile = pending
                .reply
                .available_profiles
                .iter()
                .find(|profile| profile.id == id)
                .ok_or(AuthError::ProviderIdentityChanged)?;
            let reply = self.token_request("refresh",&serde_json::json!({"accessToken":pending.reply.access_token.expose(),"clientToken":pending.reply.client_token.expose(),"requestUser":true,"selectedProfile":{"id":profile.id.simple().to_string(),"name":profile.name}}),cancel).await?;
            if reply.client_token.expose() != pending.reply.client_token.expose()
                || reply.selected_profile.as_ref().is_none_or(|p| p.id != id)
            {
                return Err(AuthError::ProviderIdentityChanged);
            }
            reply
        };
        if cancel.is_cancelled() {
            return Err(AuthError::Cancelled);
        }
        if pending.is_expired() {
            return Err(AuthError::ProviderPreparationExpired);
        }
        let profile = reply
            .selected_profile
            .as_ref()
            .ok_or(AuthError::ProviderProfileSelectionRequired)?;
        let mut account = Account {
            reference: AccountRef {
                player_id: profile.id,
            },
            provider: AccountProvider::Authlib,
            player_name: profile.name.clone(),
            client_id: String::new(),
            credential_ref: String::new(),
            authlib: Some(AuthlibAccount {
                provider_id: self.provider.id,
                login: pending.login.clone(),
            }),
        };
        account.credential_ref = account.identity_key();
        let tokens = TokenBundle {
            schema_version: 1,
            account_key: account.identity_key(),
            api_url: self.provider.api_url.clone(),
            access_token: reply.access_token,
            client_token: reply.client_token,
            user: reply.user,
        };
        let session = session_from_tokens(account, &tokens)?;
        Ok(PreparedAuthlibSignIn {
            session,
            tokens,
            api_url: self.provider.api_url.clone(),
            expires_at: pending.expires_at,
        })
    }

    /// Final confirmation validates the prepared token and then writes the OS store.
    pub async fn save_sign_in(
        &self,
        prepared: PreparedAuthlibSignIn,
        cancel: &CancellationToken,
    ) -> Result<AuthenticatedSession> {
        if prepared.is_expired() {
            return Err(AuthError::ProviderPreparationExpired);
        }
        self.validate_account(&prepared.session.account)?;
        if prepared.api_url != self.provider.api_url {
            return Err(AuthError::ProviderIdentityChanged);
        }
        if !self.validate_tokens(&prepared.tokens, cancel).await? {
            return Err(AuthError::ProviderSignInRequired);
        }
        if prepared.is_expired() {
            return Err(AuthError::ProviderPreparationExpired);
        }
        if cancel.is_cancelled() {
            return Err(AuthError::Cancelled);
        }
        credentials::save(
            prepared.session.account.credential_ref.clone(),
            prepared.tokens.encode()?,
        )
        .await?;
        Ok(prepared.session)
    }

    /// Validate before launch; refresh only a provider-declared invalid token.
    pub async fn refresh(
        &self,
        account: &Account,
        cancel: &CancellationToken,
    ) -> Result<AuthenticatedSession> {
        self.validate_account(account)?;
        let mut tokens = self.load_tokens(account).await?;
        if !self.validate_tokens(&tokens, cancel).await? {
            let reply = self.token_request("refresh",&serde_json::json!({"accessToken":tokens.access_token.expose(),"clientToken":tokens.client_token.expose(),"requestUser":true}),cancel).await?;
            let profile = reply
                .selected_profile
                .as_ref()
                .ok_or(AuthError::ProviderIdentityChanged)?;
            if profile.id != account.reference.player_id
                || reply.client_token.expose() != tokens.client_token.expose()
            {
                return Err(AuthError::ProviderIdentityChanged);
            }
            tokens.access_token = reply.access_token;
            tokens.client_token = reply.client_token;
            tokens.user = reply.user;
            let mut account = account.clone();
            account.player_name = profile.name.clone();
            if cancel.is_cancelled() {
                return Err(AuthError::Cancelled);
            }
            credentials::save(account.credential_ref.clone(), tokens.encode()?).await?;
            return session_from_tokens(account, &tokens);
        }
        session_from_tokens(account.clone(), &tokens)
    }

    /// Revoke only this saved token. Local credential deletion is a separate action.
    pub async fn invalidate(&self, account: &Account, cancel: &CancellationToken) -> Result<()> {
        self.validate_account(account)?;
        let tokens = self.load_tokens(account).await?;
        let (status,_) = self.request("invalidate",&serde_json::json!({"accessToken":tokens.access_token.expose(),"clientToken":tokens.client_token.expose()}),cancel).await?;
        expect_status(status, StatusCode::NO_CONTENT, "invalidate")
    }

    /// Revoke all provider sessions only with a freshly supplied password.
    pub async fn signout(
        &self,
        login: &str,
        password: SecretString,
        cancel: &CancellationToken,
    ) -> Result<()> {
        validate_login(login)?;
        if password.expose().is_empty() || password.expose().len() > 4096 {
            return Err(AuthError::ProviderLoginRequired);
        }
        let (status, _) = self
            .request(
                "signout",
                &serde_json::json!({"username":login,"password":password.expose()}),
                cancel,
            )
            .await?;
        expect_status(status, StatusCode::NO_CONTENT, "signout")
    }

    /// Remove this app's local external credential, even if the server is unavailable.
    pub async fn forget(account: &Account) -> Result<()> {
        validate_external_account(account)?;
        credentials::delete(account).await
    }

    fn validate_account(&self, account: &Account) -> Result<()> {
        validate_external_account(account)?;
        if account
            .authlib
            .as_ref()
            .is_none_or(|link| link.provider_id != self.provider.id)
        {
            return Err(AuthError::ProviderIdentityChanged);
        }
        Ok(())
    }

    async fn load_tokens(&self, account: &Account) -> Result<TokenBundle> {
        let raw = credentials::load(account).await?;
        if raw.expose().len() > MAX_METADATA {
            return Err(AuthError::CredentialsMissing);
        }
        let tokens: TokenBundle =
            serde_json::from_str(raw.expose()).map_err(|_| AuthError::CredentialsMissing)?;
        if tokens.schema_version != 1
            || tokens.account_key != account.identity_key()
            || tokens.api_url != self.provider.api_url
        {
            return Err(AuthError::ProviderIdentityChanged);
        }
        validate_auth_token(&tokens.access_token)?;
        validate_auth_token(&tokens.client_token)?;
        legacy_properties(tokens.user.as_ref())?;
        Ok(tokens)
    }

    async fn validate_tokens(
        &self,
        tokens: &TokenBundle,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let (status,_) = self.request("validate",&serde_json::json!({"accessToken":tokens.access_token.expose(),"clientToken":tokens.client_token.expose()}),cancel).await?;
        if status == StatusCode::FORBIDDEN {
            return Ok(false);
        }
        expect_status(status, StatusCode::NO_CONTENT, "validate")?;
        Ok(true)
    }

    async fn token_request(
        &self,
        operation: &'static str,
        body: &serde_json::Value,
        cancel: &CancellationToken,
    ) -> Result<AuthlibReply> {
        let (status, bytes) = self.request(operation, body, cancel).await?;
        expect_status(status, StatusCode::OK, operation)?;
        let reply: AuthlibReply = serde_json::from_slice(&bytes)
            .map_err(|_| AuthError::InvalidResponse { stage: AUTH_STAGE })?;
        validate_auth_token(&reply.access_token)?;
        validate_auth_token(&reply.client_token)?;
        if reply.available_profiles.len() > 256 {
            return Err(AuthError::InvalidResponse { stage: AUTH_STAGE });
        }
        let mut seen = HashSet::new();
        for profile in &reply.available_profiles {
            profile.validate()?;
            if !seen.insert(profile.id) {
                return Err(AuthError::InvalidResponse { stage: AUTH_STAGE });
            }
        }
        if let Some(profile) = &reply.selected_profile {
            profile.validate()?;
            if !reply.available_profiles.is_empty()
                && !reply.available_profiles.iter().any(|p| p == profile)
            {
                return Err(AuthError::ProviderIdentityChanged);
            }
        }
        legacy_properties(reply.user.as_ref())?;
        Ok(reply)
    }

    async fn request(
        &self,
        operation: &'static str,
        body: &serde_json::Value,
        cancel: &CancellationToken,
    ) -> Result<(StatusCode, Zeroizing<Vec<u8>>)> {
        let url = canonical_url(&self.provider.api_url)?
            .join(&format!("authserver/{operation}"))
            .map_err(|_| AuthError::InvalidProviderAddress)?;
        tokio::select! {
            biased;
            ()=cancel.cancelled()=>Err(AuthError::Cancelled),
            result=async {
                let mut response = self.client.post(url).json(body).send().await.map_err(auth_network)?;
                let status=response.status();
                if status==StatusCode::TOO_MANY_REQUESTS {
                    let value=response.headers().get(reqwest::header::RETRY_AFTER).and_then(|v|v.to_str().ok());
                    let seconds=value.and_then(|s|s.parse().ok()).or_else(||value.and_then(|s|httpdate::parse_http_date(s).ok()).and_then(|when|when.duration_since(SystemTime::now()).ok()).map(|d|d.as_secs())).unwrap_or(30).clamp(1,86400);
                    return Err(AuthError::ProviderThrottled {retry_after_seconds:seconds});
                }
                if response.content_length().is_some_and(|length|length>MAX_METADATA as u64) {return Err(AuthError::InvalidResponse {stage:AUTH_STAGE});}
                let mut bytes=Zeroizing::new(Vec::new());
                while let Some(chunk)=response.chunk().await.map_err(auth_network)? {
                    if bytes.len().saturating_add(chunk.len())>MAX_METADATA {return Err(AuthError::InvalidResponse {stage:AUTH_STAGE});}
                    bytes.extend_from_slice(&chunk);
                }
                Ok((status,bytes))
            }=>result,
        }
    }
}

impl TokenBundle {
    fn encode(&self) -> Result<SecretString> {
        serde_json::to_string(&serde_json::json!({"schema_version":self.schema_version,"account_key":self.account_key,"api_url":self.api_url,"access_token":self.access_token.expose(),"client_token":self.client_token.expose(),"user":self.user}))
            .map(SecretString::new).map_err(|_|AuthError::InvalidResponse {stage:AUTH_STAGE})
    }
}

fn validate_login(login: &str) -> Result<()> {
    if login.trim() != login
        || login.is_empty()
        || login.chars().count() > 128
        || login.chars().any(char::is_control)
    {
        return Err(AuthError::ProviderLoginRequired);
    }
    Ok(())
}

fn validate_auth_token(token: &SecretString) -> Result<()> {
    if token.expose().is_empty()
        || token.expose().len() > 32768
        || token.expose().chars().any(char::is_control)
    {
        return Err(AuthError::InvalidResponse { stage: AUTH_STAGE });
    }
    Ok(())
}

fn validate_external_account(account: &Account) -> Result<()> {
    if account.provider != AccountProvider::Authlib
        || !account.client_id.is_empty()
        || account.credential_ref != account.identity_key()
    {
        return Err(AuthError::ProviderIdentityChanged);
    }
    let link = account
        .authlib
        .as_ref()
        .ok_or(AuthError::ProviderIdentityChanged)?;
    if link.provider_id.is_nil() {
        return Err(AuthError::ProviderIdentityChanged);
    }
    validate_login(&link.login)?;
    AuthlibProfile {
        id: account.reference.player_id,
        name: account.player_name.clone(),
    }
    .validate()
}

fn legacy_properties(user: Option<&AuthlibUser>) -> Result<SecretString> {
    let mut values = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    if let Some(user) = user {
        if user.properties.len() > 64 {
            return Err(AuthError::InvalidResponse { stage: AUTH_STAGE });
        }
        for property in &user.properties {
            if property.name.is_empty() || property.name.len() > 128 || property.value.len() > 4096
            {
                return Err(AuthError::InvalidResponse { stage: AUTH_STAGE });
            }
            values
                .entry(&property.name)
                .or_default()
                .push(&property.value);
        }
    }
    let json = serde_json::to_string(&values)
        .map_err(|_| AuthError::InvalidResponse { stage: AUTH_STAGE })?;
    if json.len() > 64 * 1024 {
        return Err(AuthError::InvalidResponse { stage: AUTH_STAGE });
    }
    Ok(SecretString::new(json))
}

fn session_from_tokens(account: Account, tokens: &TokenBundle) -> Result<AuthenticatedSession> {
    Ok(AuthenticatedSession {
        account,
        access_token: SecretString::new(tokens.access_token.expose().to_owned()),
        user_properties: legacy_properties(tokens.user.as_ref())?,
        // Yggdrasil does not report expiry. Bound bootstrap freshness locally;
        // every subsequent launch still validates against the provider.
        expires_at: SystemTime::now() + PREPARATION_TTL,
    })
}

fn expect_status(status: StatusCode, expected: StatusCode, operation: &str) -> Result<()> {
    if status == expected {
        return Ok(());
    }
    if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
        return Err(if matches!(operation, "authenticate" | "signout") {
            AuthError::ProviderCredentialsRejected
        } else {
            AuthError::ProviderSignInRequired
        });
    }
    Err(AuthError::Http {
        stage: AUTH_STAGE,
        status: status.as_u16(),
    })
}

fn auth_network(error: reqwest::Error) -> AuthError {
    if error.is_timeout() {
        AuthError::ProviderTimedOut
    } else {
        AuthError::Network { stage: AUTH_STAGE }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAddress {
    pub url: String,
    pub suggested_name: String,
    pub insecure: bool,
}

impl ProviderAddress {
    /// Normalize a user address without making a connection.
    /// # Errors
    /// Rejects malformed links, credentials and non-HTTP API roots.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() || input.len() > 8192 {
            return Err(AuthError::InvalidProviderAddress);
        }
        let decoded;
        let address = if let Some(encoded) = input.strip_prefix(QUICK_LINK) {
            for (i, byte) in encoded.bytes().enumerate() {
                if byte == b'%'
                    && !encoded
                        .as_bytes()
                        .get(i + 1..i + 3)
                        .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
                {
                    return Err(AuthError::InvalidProviderAddress);
                }
            }
            decoded = percent_encoding::percent_decode_str(encoded)
                .decode_utf8()
                .map_err(|_| AuthError::InvalidProviderAddress)?;
            if decoded.chars().any(char::is_control) {
                return Err(AuthError::InvalidProviderAddress);
            }
            decoded.trim()
        } else if input.starts_with("authlib-injector:") {
            return Err(AuthError::InvalidProviderAddress);
        } else {
            input
        };
        let explicit = if address.contains("://") {
            address.to_owned()
        } else {
            format!("https://{address}")
        };
        let url = canonical_url(&explicit)?;
        Ok(Self {
            suggested_name: url
                .host_str()
                .ok_or(AuthError::InvalidProviderAddress)?
                .to_owned(),
            insecure: url.scheme() == "http",
            url: url.to_string(),
        })
    }
}

fn canonical_url(input: &str) -> Result<Url> {
    if input.len() > 2048
        || input
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
        || input.contains('\\')
    {
        return Err(AuthError::InvalidProviderAddress);
    }
    let mut url = Url::parse(input).map_err(|_| AuthError::InvalidProviderAddress)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port() == Some(0)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AuthError::InvalidProviderAddress);
    }
    let path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

fn discovery_target(current: &Url, target: &str) -> Result<Url> {
    let next = current
        .join(target)
        .map_err(|_| AuthError::InvalidProviderAddress)?;
    let next = canonical_url(next.as_str())?;
    if current.scheme() == "https" && next.scheme() == "http" {
        return Err(AuthError::InsecureProvider);
    }
    Ok(next)
}

/// Public metadata is cached with its source endpoint; no login fields belong here.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderMetadata {
    pub server_name: Option<String>,
    pub icon_url: String,
    pub skin_domains: Vec<String>,
    pub signature_public_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AuthlibProvider {
    pub id: Uuid,
    pub name: String,
    pub api_url: String,
    pub allow_http: bool,
    pub metadata: ProviderMetadata,
    pub metadata_json: String,
}

impl AuthlibProvider {
    /// Discover the real API and retain bounded metadata after explicit HTTP consent.
    /// # Errors
    /// Reports cancellation, timeouts, redirect/downgrade failures and invalid metadata.
    pub async fn discover(
        id: Uuid,
        name: &str,
        address: &str,
        allow_http: bool,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        validate_name(name)?;
        if id.is_nil() {
            return Err(AuthError::ProviderMetadata);
        }
        let address = ProviderAddress::parse(address)?;
        // Consent applies only to a URL the user actually entered as HTTP.
        let allow_http = allow_http && address.insecure;
        if address.insecure && !allow_http {
            return Err(AuthError::InsecureProvider);
        }
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(AuthError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(45), discover_inner(id, name, &address.url, allow_http)) =>
                result.map_err(|_| AuthError::ProviderTimedOut)?,
        }
    }

    /// Validate cached public fields before using a persisted endpoint.
    /// # Errors
    /// Rejects malformed or inconsistent cache entries instead of silently rewriting them.
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        let url = canonical_url(&self.api_url)?;
        if self.id.is_nil() || url.as_str() != self.api_url {
            return Err(AuthError::ProviderMetadata);
        }
        if url.scheme() == "http" && !self.allow_http {
            return Err(AuthError::InsecureProvider);
        }
        if metadata(&self.metadata_json, &url)? != self.metadata {
            return Err(AuthError::ProviderMetadata);
        }
        Ok(())
    }
}

/// Read the account's textures property and return a skin only when its host is
/// declared in the provider metadata. Malformed or undeclared textures are silent
/// failures: callers fall back to initials instead of showing anything else.
#[must_use]
pub fn skin_texture(
    session: &AuthenticatedSession,
    provider: &AuthlibProvider,
) -> Option<crate::SkinTexture> {
    let properties: std::collections::BTreeMap<String, Vec<String>> =
        serde_json::from_str(session.expose_user_properties()).ok()?;
    let encoded = properties.get("textures")?.last()?;
    let decoded = STANDARD.decode(encoded).ok()?;
    if decoded.len() > 16 * 1024 {
        return None;
    }
    let document: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    let skin = document.pointer("/textures/SKIN")?;
    let url = skin.get("url")?.as_str()?;
    if !skin_url_declared(url, &provider.metadata.skin_domains) {
        return None;
    }
    let variant = if skin
        .pointer("/metadata/model")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|model| model.eq_ignore_ascii_case("slim"))
    {
        crate::SkinVariant::Slim
    } else {
        crate::SkinVariant::Classic
    };
    Some(crate::SkinTexture {
        url: url.to_owned(),
        variant,
    })
}

fn skin_url_declared(url: &str, domains: &[String]) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    domains.iter().any(|domain| domain_declares(domain, &host))
}

fn domain_declares(declared: &str, host: &str) -> bool {
    let declared = declared.trim_end_matches('.').to_ascii_lowercase();
    if let Some(suffix) = declared.strip_prefix("*.") {
        return host == suffix || host.ends_with(&format!(".{suffix}"));
    }
    if let Some(suffix) = declared.strip_prefix('.') {
        return host == suffix || host.ends_with(&format!(".{suffix}"));
    }
    host == declared
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.chars().count() > 128
        || name.chars().any(char::is_control)
    {
        return Err(AuthError::InvalidProviderName);
    }
    Ok(())
}

async fn discover_inner(
    id: Uuid,
    name: &str,
    address: &str,
    allow_http: bool,
) -> Result<AuthlibProvider> {
    let builder = Client::builder();
    #[cfg(test)]
    let builder = builder.no_proxy();
    let client = builder
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent("Block/0.1 (provider discovery)")
        .build()
        .map_err(provider_network)?;
    let mut url = canonical_url(address)?;
    let mut visited = HashSet::new();
    for _ in 0..=5 {
        if !visited.insert(url.to_string()) {
            return Err(AuthError::ProviderDiscoveryLoop);
        }
        if url.scheme() == "http" && !allow_http {
            return Err(AuthError::InsecureProvider);
        }
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(provider_network)?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok());
            let seconds = retry_after
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    retry_after
                        .and_then(|s| httpdate::parse_http_date(s).ok())
                        .and_then(|when| when.duration_since(SystemTime::now()).ok())
                        .map(|d| d.as_secs())
                })
                .unwrap_or(30)
                .clamp(1, 86400);
            return Err(AuthError::ProviderThrottled {
                retry_after_seconds: seconds,
            });
        }
        let target = if response.status().is_redirection() {
            Some(
                response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or(AuthError::ProviderMetadata)?,
            )
        } else {
            response.headers().get("X-Authlib-Injector-API-Location")
        };
        if let Some(target) = target {
            let target = target
                .to_str()
                .map_err(|_| AuthError::InvalidProviderAddress)?;
            let next = discovery_target(&url, target)?;
            if next != url {
                url = next;
                continue;
            }
        }
        if !response.status().is_success() {
            return Err(AuthError::Http {
                stage: DISCOVERY_STAGE,
                status: response.status().as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_METADATA as u64)
        {
            return Err(AuthError::ProviderMetadata);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(provider_network)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_METADATA {
                return Err(AuthError::ProviderMetadata);
            }
            bytes.extend_from_slice(&chunk);
        }
        let json = String::from_utf8(bytes).map_err(|_| AuthError::ProviderMetadata)?;
        let provider = AuthlibProvider {
            id,
            name: name.to_owned(),
            api_url: url.to_string(),
            allow_http: url.scheme() == "http" && allow_http,
            metadata: metadata(&json, &url)?,
            metadata_json: json,
        };
        provider.validate()?;
        return Ok(provider);
    }
    Err(AuthError::ProviderDiscoveryLoop)
}

fn provider_network(error: reqwest::Error) -> AuthError {
    if error.is_timeout() {
        AuthError::ProviderTimedOut
    } else {
        AuthError::Network {
            stage: DISCOVERY_STAGE,
        }
    }
}

fn metadata(json: &str, url: &Url) -> Result<ProviderMetadata> {
    if json.len() > MAX_METADATA {
        return Err(AuthError::ProviderMetadata);
    }
    let object: serde_json::Value =
        serde_json::from_str(json).map_err(|_| AuthError::ProviderMetadata)?;
    if !object.is_object() {
        return Err(AuthError::ProviderMetadata);
    }
    let key = object
        .get("signaturePublickey")
        .and_then(serde_json::Value::as_str)
        .ok_or(AuthError::ProviderMetadata)?;
    let der = key
        .trim_end_matches(['\r', '\n'])
        .strip_prefix("-----BEGIN PUBLIC KEY-----")
        .and_then(|s| s.strip_suffix("-----END PUBLIC KEY-----"))
        .ok_or(AuthError::ProviderMetadata)?
        .replace(['\r', '\n'], "");
    let decoded = STANDARD
        .decode(der)
        .map_err(|_| AuthError::ProviderMetadata)?;
    if key.len() > 32768 || decoded.len() < 16 || decoded.first() != Some(&0x30) {
        return Err(AuthError::ProviderMetadata);
    }
    let domains = match object.get("skinDomains") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or(AuthError::ProviderMetadata)?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or(AuthError::ProviderMetadata)
            })
            .collect::<Result<Vec<_>>>()?,
    };
    if domains.len() > 256
        || domains.iter().any(|d| {
            d.is_empty()
                || d.len() > 512
                || d.chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control() || ":/\\?#@".contains(ch))
        })
    {
        return Err(AuthError::ProviderMetadata);
    }
    let name = match object.get("meta").and_then(|m| m.get("serverName")) {
        None => None,
        Some(value) => {
            let name = value.as_str().ok_or(AuthError::ProviderMetadata)?;
            validate_name(name).map_err(|_| AuthError::ProviderMetadata)?;
            Some(name.to_owned())
        }
    };
    Ok(ProviderMetadata {
        server_name: name,
        icon_url: url
            .join("/favicon.ico")
            .map_err(|_| AuthError::ProviderMetadata)?
            .to_string(),
        skin_domains: domains,
        signature_public_key: key.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn fixture_provider(domains: &[&str]) -> AuthlibProvider {
        AuthlibProvider {
            id: Uuid::from_u128(11),
            name: "Fixture".into(),
            api_url: "https://api.example.test/".into(),
            allow_http: false,
            metadata: ProviderMetadata {
                server_name: None,
                icon_url: "https://api.example.test/favicon.ico".into(),
                skin_domains: domains.iter().map(|domain| (*domain).to_owned()).collect(),
                signature_public_key: String::new(),
            },
            metadata_json: "{}".into(),
        }
    }

    fn textures_session(encoded: &str) -> AuthenticatedSession {
        AuthenticatedSession::for_test(
            Account {
                reference: AccountRef {
                    player_id: Uuid::from_u128(7),
                },
                provider: AccountProvider::Authlib,
                player_name: "Player".into(),
                client_id: String::new(),
                credential_ref: String::new(),
                authlib: Some(AuthlibAccount {
                    provider_id: Uuid::from_u128(11),
                    login: "player@example.test".into(),
                }),
            },
            "synthetic-access",
            &serde_json::json!({ "textures": [encoded] }).to_string(),
        )
    }

    fn textures_value(url: &str, slim: bool) -> String {
        let document = serde_json::json!({
            "textures": {
                "SKIN": {
                    "url": url,
                    "metadata": if slim { serde_json::json!({"model":"slim"}) } else { serde_json::json!({}) }
                }
            }
        });
        STANDARD.encode(serde_json::to_vec(&document).unwrap_or_default())
    }

    #[test]
    fn textures_property_yields_the_declared_skin_and_variant()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let provider = fixture_provider(&["textures.example.test"]);
        let encoded = textures_value("https://textures.example.test/skin/abc", true);
        let skin = skin_texture(&textures_session(&encoded), &provider).ok_or("declared skin")?;
        assert_eq!(skin.url, "https://textures.example.test/skin/abc");
        assert_eq!(skin.variant, crate::SkinVariant::Slim);
        let classic = textures_value("https://textures.example.test/skin/classic", false);
        assert_eq!(
            skin_texture(&textures_session(&classic), &provider)
                .ok_or("declared skin")?
                .variant,
            crate::SkinVariant::Classic
        );
        Ok(())
    }

    #[test]
    fn skin_hosts_must_match_the_declared_domain_forms() {
        let exact = fixture_provider(&["textures.example.test"]);
        assert!(
            skin_texture(
                &textures_session(&textures_value("https://textures.example.test/a", false)),
                &exact
            )
            .is_some()
        );
        assert!(
            skin_texture(
                &textures_session(&textures_value(
                    "https://sub.textures.example.test/a",
                    false
                )),
                &exact
            )
            .is_none(),
            "an exact declaration must not accept subdomains"
        );
        assert!(
            skin_texture(
                &textures_session(&textures_value("https://evil.example.net/a", false)),
                &exact
            )
            .is_none()
        );
        let suffix = fixture_provider(&[".example.test"]);
        assert!(
            skin_texture(
                &textures_session(&textures_value("https://skins.example.test/a", false)),
                &suffix
            )
            .is_some()
        );
        let wildcard = fixture_provider(&["*.skins.test"]);
        assert!(
            skin_texture(
                &textures_session(&textures_value("https://textures.skins.test/a", false)),
                &wildcard
            )
            .is_some()
        );
        assert!(
            skin_texture(
                &textures_session(&textures_value("https://skins.test/a", false)),
                &wildcard
            )
            .is_some()
        );
        for rejected in [
            "http://example.net/@textures.example.test/a",
            "https://user:pass@textures.example.test/a",
            "ftp://textures.example.test/a",
        ] {
            assert!(
                skin_texture(&textures_session(&textures_value(rejected, false)), &exact).is_none(),
                "{rejected}"
            );
        }
    }

    #[test]
    fn malformed_textures_fall_back_silently() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let provider = fixture_provider(&["textures.example.test"]);
        for encoded in [
            "not base64".to_owned(),
            STANDARD.encode(b"not json"),
            STANDARD.encode(serde_json::to_vec(&serde_json::json!({"textures": {}}))?),
            STANDARD.encode(serde_json::to_vec(
                &serde_json::json!({"textures": {"SKIN": {}}}),
            )?),
        ] {
            assert!(skin_texture(&textures_session(&encoded), &provider).is_none());
        }
        assert!(skin_texture(&textures_session(""), &provider).is_none());
        Ok(())
    }

    #[test]
    fn discovery_never_downgrades_https_even_after_http_consent() -> Result<()> {
        let secure = Url::parse("https://community.example.test/api/")
            .map_err(|_| AuthError::InvalidProviderAddress)?;
        assert_eq!(
            discovery_target(&secure, "http://community.example.test/api/"),
            Err(AuthError::InsecureProvider)
        );
        assert_eq!(
            discovery_target(&secure, "../other/")?.as_str(),
            "https://community.example.test/other/"
        );
        let insecure = Url::parse("http://community.example.test/")
            .map_err(|_| AuthError::InvalidProviderAddress)?;
        assert_eq!(discovery_target(&insecure, secure.as_str())?, secure);
        Ok(())
    }

    #[tokio::test]
    async fn discovery_rejects_oversized_responses_and_inconsistent_metadata()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await?;
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await?;
                if count == 0 {
                    return Err(std::io::Error::other(
                        "Fixture request ended before headers",
                    ));
                }
                request.extend_from_slice(&buffer[..count]);
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 Fixture\r\nContent-Length: 600000\r\nConnection: close\r\n\r\n",
                )
                .await
        });
        assert_eq!(discover(&url).await, Err(AuthError::ProviderMetadata));
        served.await??;
        let root = canonical_url("https://example.test/api/")?;
        for replacement in [
            serde_json::json!({"skinDomains":["https://secret.test/"]}),
            serde_json::json!({"signaturePublickey":"not a PEM key"}),
            serde_json::json!({"meta":{"serverName":"bad\nname"}}),
        ] {
            let mut value: serde_json::Value = serde_json::from_str(&document())?;
            for (key, value_new) in replacement.as_object().ok_or("fixture object")? {
                value[key] = value_new.clone();
            }
            assert!(metadata(&value.to_string(), &root).is_err());
        }
        Ok(())
    }

    #[test]
    fn existing_accounts_keep_their_identity_and_external_profiles_use_provider_and_login_scope()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut account: Account = serde_json::from_value(
            serde_json::json!({"reference":{"player_id":"00000000-0000-4000-8000-000000000001"},"provider":"microsoft","player_name":"Player","client_id":"public","credential_ref":"fixture"}),
        )?;
        assert!(account.authlib.is_none());
        assert_eq!(
            account.identity_key(),
            account.reference.player_id.to_string()
        );
        assert!(serde_json::to_value(&account)?.get("authlib").is_none());
        account.provider = AccountProvider::Authlib;
        account.authlib = Some(AuthlibAccount {
            provider_id: Uuid::from_u128(1),
            login: "FirstUser".into(),
        });
        let first = account.identity_key();
        if let Some(link) = &mut account.authlib {
            link.provider_id = Uuid::from_u128(2);
        }
        assert_ne!(first, account.identity_key());
        if let Some(link) = &mut account.authlib {
            link.provider_id = Uuid::from_u128(1);
            link.login = "SecondUser".into();
        }
        assert_ne!(first, account.identity_key());
        Ok(())
    }

    fn document() -> String {
        serde_json::json!({
            "meta": {"serverName": "Fixture Community"},
            "skinDomains": ["textures.example.test", ".example.test", "*.skins.test"],
            "signaturePublickey": "-----BEGIN PUBLIC KEY-----\nMBkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDAgAA\n-----END PUBLIC KEY-----"
        }).to_string()
    }

    async fn server(
        steps: Vec<(u16, String, String)>,
    ) -> std::result::Result<
        (
            String,
            tokio::task::JoinHandle<std::io::Result<Vec<String>>>,
        ),
        Box<dyn std::error::Error>,
    > {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, headers, body) in steps {
                let (mut stream, _) = listener.accept().await?;
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut buffer = [0; 2048];
                    let count = stream.read(&mut buffer).await?;
                    if count == 0 {
                        return Err(std::io::Error::other("Fixture request ended early"));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let header_end = bytes
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .ok_or_else(|| std::io::Error::other("Missing request headers"))?
                    + 4;
                let length = String::from_utf8_lossy(&bytes[..header_end])
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse::<usize>())
                    .transpose()
                    .map_err(std::io::Error::other)?
                    .unwrap_or(0);
                if length > MAX_METADATA {
                    return Err(std::io::Error::other("Oversized fixture request"));
                }
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 2048];
                    let count = stream.read(&mut buffer).await?;
                    if count == 0 {
                        return Err(std::io::Error::other("Fixture body ended early"));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let request = String::from_utf8(bytes).map_err(std::io::Error::other)?;
                let body = if body.contains("__CLIENT_TOKEN__") {
                    let value: serde_json::Value = serde_json::from_str(&request[header_end..])
                        .map_err(std::io::Error::other)?;
                    let token = value
                        .get("clientToken")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| std::io::Error::other("Missing client token"))?;
                    body.replace("__CLIENT_TOKEN__", token)
                } else {
                    body
                };
                requests.push(request);
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await?;
            }
            Ok(requests)
        });
        Ok((url, task))
    }

    async fn discover(url: &str) -> Result<AuthlibProvider> {
        AuthlibProvider::discover(
            Uuid::new_v4(),
            "My community",
            url,
            true,
            &CancellationToken::new(),
        )
        .await
    }

    fn auth_fixture(url: &str) -> std::result::Result<AuthlibAuth, Box<dyn std::error::Error>> {
        let json = document();
        Ok(AuthlibAuth::new(&AuthlibProvider {
            id: Uuid::new_v4(),
            name: "Protocol fixture".into(),
            api_url: url.into(),
            allow_http: true,
            metadata: metadata(&json, &Url::parse(url)?)?,
            metadata_json: json,
        })?)
    }

    fn profile(id: u128, name: &str) -> serde_json::Value {
        serde_json::json!({"id":Uuid::from_u128(id).simple().to_string(),"name":name})
    }

    fn token_reply(
        token: &str,
        selected: serde_json::Value,
        profiles: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({"accessToken":token,"clientToken":"__CLIENT_TOKEN__","availableProfiles":profiles,"selectedProfile":selected,
            "user":{"id":Uuid::from_u128(1234).simple().to_string(),"properties":[{"name":"preferredLanguage","value":"zh_CN"}]}})
    }

    fn json_step(value: &serde_json::Value) -> (u16, String, String) {
        (
            200,
            "Content-Type: application/json\r\n".into(),
            value.to_string(),
        )
    }

    fn request_json(
        request: &str,
    ) -> std::result::Result<serde_json::Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(
            request
                .split_once("\r\n\r\n")
                .ok_or("Missing fixture body")?
                .1,
        )?)
    }

    #[tokio::test]
    async fn yggdrasil_selects_a_character_then_saves_validates_refreshes_and_revokes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let first = profile(1, "First");
        let chosen = profile(2, "Second");
        let renamed = profile(2, "Renamed");
        let initial = token_reply(
            "AUTHENTICATE_SECRET",
            serde_json::Value::Null,
            serde_json::json!([first, chosen]),
        );
        let assigned = token_reply(
            "ASSIGNED_SECRET",
            chosen.clone(),
            serde_json::json!([chosen]),
        );
        let mut rotated = token_reply("ROTATED_SECRET", renamed, serde_json::json!([]));
        rotated["user"]["id"] = serde_json::json!(Uuid::from_u128(789).simple().to_string());
        rotated["user"]["properties"] =
            serde_json::json!([{"name":"preferredLanguage","value":"en"}]);
        let empty = || (204, String::new(), String::new());
        let (url, task) = server(vec![
            json_step(&initial),
            json_step(&assigned),
            empty(),
            empty(),
            (403, String::new(), "PRIVATE_BODY".into()),
            json_step(&rotated),
            empty(),
            empty(),
        ])
        .await?;
        let auth = auth_fixture(&url)?;
        let cancel = CancellationToken::new();
        let pending = auth
            .authenticate(
                "fixture@example.test",
                SecretString::new("FIRST_PASSWORD".into()),
                &cancel,
            )
            .await?;
        assert_eq!(pending.profiles().len(), 2);
        assert!(pending.selected_profile().is_none());
        assert!(matches!(
            auth.prepare_sign_in(&pending, None, &cancel).await,
            Err(AuthError::ProviderProfileSelectionRequired)
        ));
        let prepared = auth
            .prepare_sign_in(&pending, Some(Uuid::from_u128(2)), &cancel)
            .await?;
        let account = prepared.account().clone();
        assert_eq!(account.provider, AccountProvider::Authlib);
        assert_eq!(account.reference.player_id, Uuid::from_u128(2));
        assert_eq!(account.credential_ref, account.identity_key());
        assert!(matches!(
            credentials::load(&account).await,
            Err(AuthError::CredentialsMissing)
        ));
        let public = serde_json::to_string(&account)?;
        let debug = format!("{pending:?} {prepared:?}");
        for secret in [
            "AUTHENTICATE_SECRET",
            "ASSIGNED_SECRET",
            "FIRST_PASSWORD",
            "zh_CN",
        ] {
            assert!(!public.contains(secret));
            assert!(!debug.contains(secret));
        }
        let saved = auth.save_sign_in(prepared, &cancel).await?;
        assert_eq!(saved.expose_access_token(), "ASSIGNED_SECRET");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(saved.expose_user_properties())?,
            serde_json::json!({"preferredLanguage":["zh_CN"]})
        );
        assert!(
            !credentials::load(&account)
                .await?
                .expose()
                .contains("FIRST_PASSWORD")
        );
        assert_eq!(
            auth.refresh(&account, &cancel).await?.expose_access_token(),
            "ASSIGNED_SECRET"
        );
        let refreshed = auth.refresh(&account, &cancel).await?;
        assert_eq!(refreshed.expose_access_token(), "ROTATED_SECRET");
        assert_eq!(refreshed.account.player_name, "Renamed");
        assert_eq!(refreshed.account.identity_key(), account.identity_key());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(refreshed.expose_user_properties())?,
            serde_json::json!({"preferredLanguage":["en"]})
        );
        let stored = credentials::load(&account).await?;
        assert!(stored.expose().contains("ROTATED_SECRET"));
        assert!(!stored.expose().contains("ASSIGNED_SECRET"));
        let stored_user: serde_json::Value = serde_json::from_str(stored.expose())?;
        assert_eq!(stored_user["user"]["id"], Uuid::from_u128(789).to_string());
        auth.invalidate(&refreshed.account, &cancel).await?;
        auth.signout(
            "fixture@example.test",
            SecretString::new("FRESH_PASSWORD".into()),
            &cancel,
        )
        .await?;
        AuthlibAuth::forget(&account).await?;
        assert!(matches!(
            auth.refresh(&account, &cancel).await,
            Err(AuthError::CredentialsMissing)
        ));
        let requests = task.await??;
        let paths = [
            "authenticate",
            "refresh",
            "validate",
            "validate",
            "validate",
            "refresh",
            "invalidate",
            "signout",
        ];
        assert_eq!(requests.len(), paths.len());
        let bodies = requests
            .iter()
            .map(|r| request_json(r))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let client = bodies[0]["clientToken"]
            .as_str()
            .ok_or("No generated client token")?;
        assert_eq!(client.len(), 32);
        for (i, (request, path)) in requests.iter().zip(paths).enumerate() {
            assert!(request.starts_with(&format!("POST /authserver/{path} HTTP/1.1\r\n")));
            assert!(!request.to_lowercase().contains("authorization:"));
            if i < 7 {
                assert_eq!(bodies[i]["clientToken"], client);
            }
            if !matches!(i, 0 | 7) {
                assert!(bodies[i].get("password").is_none());
            }
        }
        assert_eq!(
            bodies[0]["agent"],
            serde_json::json!({"name":"Minecraft","version":1})
        );
        for i in [0, 1, 5] {
            assert_eq!(bodies[i]["requestUser"], true);
        }
        assert_eq!(bodies[1]["selectedProfile"], profile(2, "Second"));
        assert!(bodies[5].get("selectedProfile").is_none());
        assert_eq!(bodies[6]["accessToken"], "ROTATED_SECRET");
        assert_eq!(bodies[7]["password"], "FRESH_PASSWORD");
        Ok(())
    }

    #[tokio::test]
    async fn yggdrasil_bound_profile_skips_assignment_and_expired_or_foreign_drafts_never_save()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let selected = profile(4, "山与海");
        let (url, task) = server(vec![json_step(&token_reply(
            "BOUND_SECRET",
            selected.clone(),
            serde_json::json!([selected]),
        ))])
        .await?;
        let auth = auth_fixture(&url)?;
        let cancel = CancellationToken::new();
        let mut pending = auth
            .authenticate("user", SecretString::new("PASSWORD".into()), &cancel)
            .await?;
        assert_eq!(
            pending.selected_profile().map(|p| p.name.as_str()),
            Some("山与海")
        );
        assert!(matches!(
            auth.prepare_sign_in(&pending, Some(Uuid::from_u128(9)), &cancel)
                .await,
            Err(AuthError::ProviderIdentityChanged)
        ));
        let foreign = auth_fixture(&url)?;
        assert!(matches!(
            foreign.prepare_sign_in(&pending, None, &cancel).await,
            Err(AuthError::ProviderIdentityChanged)
        ));
        let mut changed = auth.provider.clone();
        changed.api_url = format!("{url}other/");
        changed.metadata = metadata(&changed.metadata_json, &Url::parse(&changed.api_url)?)?;
        assert!(matches!(
            AuthlibAuth::new(&changed)?
                .prepare_sign_in(&pending, None, &cancel)
                .await,
            Err(AuthError::ProviderIdentityChanged)
        ));
        let mut prepared = auth.prepare_sign_in(&pending, None, &cancel).await?;
        let account = prepared.account().clone();
        assert_eq!(account.player_name, "山与海");
        prepared.expires_at = tokio::time::Instant::now();
        assert!(matches!(
            auth.save_sign_in(prepared, &cancel).await,
            Err(AuthError::ProviderPreparationExpired)
        ));
        assert!(matches!(
            credentials::load(&account).await,
            Err(AuthError::CredentialsMissing)
        ));
        pending.expires_at = tokio::time::Instant::now();
        assert!(matches!(
            auth.prepare_sign_in(&pending, None, &cancel).await,
            Err(AuthError::ProviderPreparationExpired)
        ));
        assert_eq!(task.await??.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn yggdrasil_rejects_invalid_profiles_tokens_and_client_identity_without_exposing_bodies()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let selected = profile(7, "Player");
        let valid = token_reply(
            "PRIVATE_TOKEN",
            selected.clone(),
            serde_json::json!([selected]),
        );
        let mut cases = Vec::new();
        for (key, value) in [
            ("availableProfiles", serde_json::json!([selected, selected])),
            ("selectedProfile", profile(8, "Alien")),
            ("selectedProfile", profile(7, &"x".repeat(17))),
            ("accessToken", serde_json::json!("")),
            ("clientToken", serde_json::json!("WRONG_CLIENT")),
            (
                "user",
                serde_json::json!({"properties":[{"name":"oversized","value":"x".repeat(4097)}]}),
            ),
        ] {
            let mut value_body = valid.clone();
            value_body[key] = value;
            cases.push(value_body);
        }
        for body in cases {
            let (url, task) = server(vec![json_step(&body)]).await?;
            let error = auth_fixture(&url)?
                .authenticate(
                    "user",
                    SecretString::new("PRIVATE_PASSWORD".into()),
                    &CancellationToken::new(),
                )
                .await
                .err()
                .ok_or("Accepted invalid provider response")?;
            for secret in ["PRIVATE_TOKEN", "PRIVATE_PASSWORD", "WRONG_CLIENT"] {
                assert!(!format!("{error:?} {error}").contains(secret));
            }
            task.await??;
        }
        let (url, task) = server(vec![json_step(&token_reply(
            "EMPTY_PROFILE_SECRET",
            serde_json::Value::Null,
            serde_json::json!([]),
        ))])
        .await?;
        assert!(matches!(
            auth_fixture(&url)?
                .authenticate(
                    "user",
                    SecretString::new("PASSWORD".into()),
                    &CancellationToken::new()
                )
                .await,
            Err(AuthError::ProviderNoProfiles)
        ));
        task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn yggdrasil_never_forwards_passwords_to_redirects_or_ali_and_maps_throttling()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let trap = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let target = format!("http://{}/", trap.local_addr()?);
        let valid = token_reply("BOUND_SECRET", profile(1, "Player"), serde_json::json!([]));
        let when = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(120));
        for (status, headers, body, expected) in [
            (
                302,
                format!("Location: {target}\r\n"),
                "PRIVATE_BODY".into(),
                Some(AuthError::Http {
                    stage: AUTH_STAGE,
                    status: 302,
                }),
            ),
            (
                403,
                String::new(),
                "PRIVATE_BODY".into(),
                Some(AuthError::ProviderCredentialsRejected),
            ),
            (
                429,
                "Retry-After: 19\r\n".into(),
                "PRIVATE_BODY".into(),
                Some(AuthError::ProviderThrottled {
                    retry_after_seconds: 19,
                }),
            ),
            (
                200,
                format!("X-Authlib-Injector-API-Location: {target}\r\n"),
                valid.to_string(),
                None,
            ),
        ] {
            let (url, task) = server(vec![(status, headers, body)]).await?;
            let result = auth_fixture(&url)?
                .authenticate(
                    "user",
                    SecretString::new("PRIVATE_PASSWORD".into()),
                    &CancellationToken::new(),
                )
                .await;
            assert_eq!(result.err(), expected);
            let requests = task.await??;
            assert_eq!(requests.len(), 1);
            assert!(requests[0].contains("PRIVATE_PASSWORD"));
        }
        let (url, task) = server(vec![(
            429,
            format!("Retry-After: {when}\r\n"),
            String::new(),
        )])
        .await?;
        assert!(matches!(
            auth_fixture(&url)?
                .authenticate(
                    "user",
                    SecretString::new("PASSWORD".into()),
                    &CancellationToken::new()
                )
                .await,
            Err(AuthError::ProviderThrottled {
                retry_after_seconds: 110..=120
            })
        ));
        task.await??;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), trap.accept())
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn yggdrasil_changed_or_revoked_refresh_preserves_local_tokens_and_never_reauthenticates()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for changed_client in [false, true] {
            let valid = token_reply(
                "ORIGINAL_SECRET",
                profile(1, "Player"),
                serde_json::json!([]),
            );
            let mut changed = token_reply(
                "NEW_SECRET",
                profile(if changed_client { 1 } else { 2 }, "Player"),
                serde_json::json!([]),
            );
            if changed_client {
                changed["clientToken"] = serde_json::json!("UNEXPECTED_CLIENT");
            }
            let (url, task) = server(vec![
                json_step(&valid),
                (204, String::new(), String::new()),
                (403, String::new(), String::new()),
                json_step(&changed),
                (403, String::new(), String::new()),
                (403, String::new(), "PRIVATE_BODY".into()),
            ])
            .await?;
            let auth = auth_fixture(&url)?;
            let cancel = CancellationToken::new();
            let pending = auth
                .authenticate("user", SecretString::new("PASSWORD".into()), &cancel)
                .await?;
            let prepared = auth.prepare_sign_in(&pending, None, &cancel).await?;
            let account = auth.save_sign_in(prepared, &cancel).await?.account;
            let original = credentials::load(&account).await?;
            assert!(matches!(
                auth.refresh(&account, &cancel).await,
                Err(AuthError::ProviderIdentityChanged)
            ));
            assert_eq!(
                credentials::load(&account).await?.expose(),
                original.expose()
            );
            assert!(matches!(
                auth.refresh(&account, &cancel).await,
                Err(AuthError::ProviderSignInRequired)
            ));
            assert_eq!(
                credentials::load(&account).await?.expose(),
                original.expose()
            );
            let mut retargeted = auth.provider.clone();
            retargeted.api_url = format!("{url}elsewhere/");
            retargeted.metadata =
                metadata(&retargeted.metadata_json, &Url::parse(&retargeted.api_url)?)?;
            assert!(matches!(
                AuthlibAuth::new(&retargeted)?
                    .refresh(&account, &cancel)
                    .await,
                Err(AuthError::ProviderIdentityChanged)
            ));
            AuthlibAuth::forget(&account).await?;
            let requests = task.await??;
            assert_eq!(requests.len(), 6);
            assert!(
                requests
                    .iter()
                    .skip(1)
                    .all(|request| !request.contains("PASSWORD")
                        && !request.contains("/authenticate"))
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn yggdrasil_cancellation_interrupts_io_and_prevents_password_submission_when_already_cancelled()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let auth = auth_fixture(&url)?;
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            auth.authenticate("user", SecretString::new("PASSWORD".into()), &cancel)
                .await,
            Err(AuthError::Cancelled)
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        let cancel = CancellationToken::new();
        let running = cancel.clone();
        let task = tokio::spawn(async move {
            auth.authenticate("user", SecretString::new("PASSWORD".into()), &running)
                .await
        });
        let (_held, _) = listener.accept().await?;
        cancel.cancel();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), task).await??,
            Err(AuthError::Cancelled)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn yggdrasil_credentials_are_scoped_to_provider_login_character_and_api()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (url, task) = server(vec![
            json_step(&token_reply(
                "SCOPED_SECRET",
                profile(1, "Player"),
                serde_json::json!([]),
            )),
            (204, String::new(), String::new()),
        ])
        .await?;
        let auth = auth_fixture(&url)?;
        let cancel = CancellationToken::new();
        let pending = auth
            .authenticate("FirstLogin", SecretString::new("PASSWORD".into()), &cancel)
            .await?;
        let prepared = auth.prepare_sign_in(&pending, None, &cancel).await?;
        let account = auth.save_sign_in(prepared, &cancel).await?.account;
        let stored = credentials::load(&account).await?;
        for field in ["account_key", "api_url", "schema_version"] {
            let mut altered: serde_json::Value = serde_json::from_str(stored.expose())?;
            altered[field] = if field == "schema_version" {
                serde_json::json!(2)
            } else {
                serde_json::json!("https://other.example.test/")
            };
            credentials::save(
                account.credential_ref.clone(),
                SecretString::new(altered.to_string()),
            )
            .await?;
            assert!(matches!(
                auth.refresh(&account, &cancel).await,
                Err(AuthError::ProviderIdentityChanged)
            ));
        }
        credentials::save(account.credential_ref.clone(), stored).await?;
        for field in ["provider", "login", "character"] {
            let mut other = account.clone();
            match field {
                "provider" => {
                    if let Some(link) = &mut other.authlib {
                        link.provider_id = Uuid::new_v4();
                    }
                }
                "login" => {
                    if let Some(link) = &mut other.authlib {
                        link.login = "SecondLogin".into();
                    }
                }
                _ => other.reference.player_id = Uuid::from_u128(2),
            }
            // A copied reference must never load the original account's credential.
            assert!(matches!(
                auth.refresh(&other, &cancel).await,
                Err(AuthError::ProviderIdentityChanged)
            ));
            other.credential_ref = other.identity_key();
            assert!(matches!(
                auth.refresh(&other, &cancel).await,
                Err(AuthError::CredentialsMissing | AuthError::ProviderIdentityChanged)
            ));
        }
        assert!(
            credentials::load(&account)
                .await?
                .expose()
                .contains("SCOPED_SECRET")
        );
        AuthlibAuth::forget(&account).await?;
        assert_eq!(task.await??.len(), 2);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn yggdrasil_request_timeout_has_no_automatic_password_retry()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let runnable = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let auth = auth_fixture(&format!("http://{}/", listener.local_addr()?))?;
        let task = tokio::spawn(async move {
            auth.request(
                "authenticate",
                &serde_json::json!({"password":"PASSWORD"}),
                &CancellationToken::new(),
            )
            .await
        });
        let (_held, _) = listener.accept().await?;
        tokio::time::advance(Duration::from_secs(31)).await;
        assert_eq!(task.await?.err(), Some(AuthError::ProviderTimedOut));
        let next = tokio::time::timeout(Duration::from_millis(10), listener.accept());
        tokio::time::advance(Duration::from_millis(11)).await;
        assert!(next.await.is_err());
        runnable.abort();
        Ok(())
    }

    #[test]
    fn addresses_and_quick_links_normalize_without_dropping_security_fields() -> Result<()> {
        for (input, expected) in [
            ("Example.COM/api///", "https://example.com/api/"),
            ("localhost:25565/api", "https://localhost:25565/api/"),
            ("https://example.com:443/api/", "https://example.com/api/"),
            (
                "authlib-injector:yggdrasil-server:https%3A%2F%2Fexample.com%2Fapi",
                "https://example.com/api/",
            ),
        ] {
            assert_eq!(ProviderAddress::parse(input)?.url, expected);
        }
        assert!(ProviderAddress::parse("http://localhost:8080/api")?.insecure);
        for input in [
            "",
            "https://user:password@example.com",
            "https://example.com/?token=private",
            "https://example.com/#private",
            "file:///tmp/provider",
            "ftp://example.com",
            "authlib-injector:invalid:example.com",
            "authlib-injector:yggdrasil-server:%GG",
            "authlib-injector:yggdrasil-server:%FF",
            "authlib-injector:yggdrasil-server:https%3A%2F%2Fexample.com%0A",
            "https://example.com/line\nbreak",
            "https://example.com\\api",
            "http://localhost:0/",
        ] {
            assert!(ProviderAddress::parse(input).is_err(), "{input:?}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn discovery_follows_redirects_and_relative_ali_then_caches_validated_metadata()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let json = document();
        let (url, task) = server(vec![
            (302, "Location: /landing/\r\n".into(), String::new()),
            (
                200,
                "X-Authlib-Injector-API-Location: ../api/\r\n".into(),
                "Not metadata".into(),
            ),
            (
                200,
                "X-Authlib-Injector-API-Location: ./\r\n".into(),
                json.clone(),
            ),
        ])
        .await?;
        let provider = discover(&url).await?;
        assert_eq!(provider.api_url, format!("{url}api/"));
        assert_eq!(provider.name, "My community");
        assert_eq!(
            provider.metadata.server_name.as_deref(),
            Some("Fixture Community")
        );
        assert_eq!(provider.metadata.icon_url, format!("{url}favicon.ico"));
        assert_eq!(provider.metadata.skin_domains.len(), 3);
        assert_eq!(provider.metadata_json, json);
        let cached: AuthlibProvider = serde_json::from_str(&serde_json::to_string(&provider)?)?;
        cached.validate()?;
        let requests = task.await??;
        assert_eq!(requests.len(), 3);
        for (request, path) in requests.iter().zip(["/", "/landing/", "/api/"]) {
            assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
            assert!(!request.to_lowercase().contains("authorization:"));
        }
        let mut tampered = cached;
        tampered.metadata.skin_domains.push("forged.test".into());
        assert!(tampered.validate().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn discovery_accepts_absolute_ali_on_a_different_server_without_credentials()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (api, api_task) = server(vec![(200, String::new(), document())]).await?;
        let (landing, landing_task) = server(vec![(
            404,
            format!("X-Authlib-Injector-API-Location: {api}\r\n"),
            String::new(),
        )])
        .await?;
        assert_eq!(discover(&landing).await?.api_url, api);
        assert_eq!(api_task.await??.len(), 1);
        landing_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn discovery_rejects_http_before_io_and_cancel_interrupts_a_pending_response()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        assert_eq!(
            AuthlibProvider::discover(
                Uuid::new_v4(),
                "Fixture",
                &url,
                false,
                &CancellationToken::new()
            )
            .await,
            Err(AuthError::InsecureProvider)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        let cancel = CancellationToken::new();
        let running = cancel.clone();
        let task = tokio::spawn(async move {
            AuthlibProvider::discover(Uuid::new_v4(), "Fixture", &url, true, &running).await
        });
        let (_held, _) = listener.accept().await?;
        cancel.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), task).await??,
            Err(AuthError::Cancelled)
        );
        Ok(())
    }

    #[tokio::test]
    async fn discovery_bounds_loops_and_rejects_untrusted_locations()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (url, task) = server(vec![
            (
                200,
                "X-Authlib-Injector-API-Location: /next/\r\n".into(),
                String::new(),
            ),
            (302, "Location: /\r\n".into(), String::new()),
        ])
        .await?;
        assert_eq!(discover(&url).await, Err(AuthError::ProviderDiscoveryLoop));
        task.await??;
        let (url, task) = server(
            (0..6)
                .map(|i| (302, format!("Location: /hop{}/\r\n", i + 1), String::new()))
                .collect(),
        )
        .await?;
        assert_eq!(discover(&url).await, Err(AuthError::ProviderDiscoveryLoop));
        assert_eq!(task.await??.len(), 6);
        for location in [
            "https://private:password@example.test/",
            "file:///private",
            "https://example.test/?private=token",
        ] {
            let (url, task) = server(vec![(
                302,
                format!("Location: {location}\r\n"),
                String::new(),
            )])
            .await?;
            assert_eq!(discover(&url).await, Err(AuthError::InvalidProviderAddress));
            task.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn discovery_maps_throttling_and_errors_without_echoing_provider_bodies()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (url, task) = server(vec![(
            429,
            "Retry-After: 17\r\n".into(),
            "PRIVATE_BODY".into(),
        )])
        .await?;
        assert_eq!(
            discover(&url).await,
            Err(AuthError::ProviderThrottled {
                retry_after_seconds: 17
            })
        );
        task.await??;
        let when = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(120));
        let (url, task) = server(vec![(
            429,
            format!("Retry-After: {when}\r\n"),
            "PRIVATE_BODY".into(),
        )])
        .await?;
        assert!(matches!(
            discover(&url).await,
            Err(AuthError::ProviderThrottled {
                retry_after_seconds: 110..=120
            })
        ));
        task.await??;
        for (status, body) in [
            (503, "PRIVATE_BODY".into()),
            (200, "<html>PRIVATE_BODY</html>".into()),
            (200, "{}".into()),
        ] {
            let (url, task) = server(vec![(status, String::new(), body)]).await?;
            let error = discover(&url)
                .await
                .err()
                .ok_or("Expected controlled error")?;
            assert!(!format!("{error:?} {error}").contains("PRIVATE_BODY"));
            task.await??;
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_times_out_a_stalled_provider_without_retrying()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        // Keep simulated time explicit while the operating system handles sockets.
        let runnable = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let task = tokio::spawn(async move { discover(&url).await });
        let (_held, _) = listener.accept().await?;
        tokio::time::advance(Duration::from_secs(21)).await;
        assert_eq!(task.await?, Err(AuthError::ProviderTimedOut));
        let next = tokio::time::timeout(Duration::from_millis(10), listener.accept());
        tokio::time::advance(Duration::from_millis(11)).await;
        assert!(next.await.is_err());
        runnable.abort();
        Ok(())
    }
}
