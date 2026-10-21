//! Microsoft desktop authentication and operating-system protected refresh tokens.
//!
//! Only account metadata is serializable. Sessions, authorization codes, PKCE
//! verifiers and credentials deliberately have no serialization implementation.

mod authlib;
mod callback;
mod credentials;
mod microsoft;
mod secret;

pub use authlib::{
    AuthlibAuth, AuthlibProfile, AuthlibProvider, PendingAuthlibSignIn, PreparedAuthlibSignIn,
    ProviderAddress, ProviderMetadata,
};
pub use callback::PendingSignIn;
pub use credentials::probe_secure_storage;
pub use microsoft::{MicrosoftAuth, PendingDeviceSignIn, PreparedMicrosoftSignIn, SignInStage};
pub use secret::SecretString;

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable public account identity; no credentials belong in a Profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct AccountRef {
    pub player_id: Uuid,
}

/// Explicit account authority; offline identities grant no online access.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountProvider {
    #[default]
    Microsoft,
    Offline,
    Authlib,
}

impl AccountProvider {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Microsoft => "Microsoft Account",
            Self::Offline => "Offline Mode",
            Self::Authlib => "Authlib-Injector",
        }
    }
}

/// Safe-to-persist metadata. Microsoft ownership is rechecked for each session.
/// Offline identities carry empty client and credential references.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Account {
    pub reference: AccountRef,
    pub provider: AccountProvider,
    pub player_name: String,
    pub client_id: String,
    pub credential_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authlib: Option<AuthlibAccount>,
}

/// Public provider/user linkage. Access and client tokens stay in secure storage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthlibAccount {
    pub provider_id: Uuid,
    pub login: String,
}

impl Account {
    /// Provider scope prevents equal character UUIDs on unrelated servers colliding.
    #[must_use]
    pub fn identity_key(&self) -> String {
        if self.provider == AccountProvider::Authlib
            && let Some(link) = &self.authlib
        {
            use sha2::{Digest as _, Sha256};
            return format!(
                "authlib:{}:{:x}:{}",
                link.provider_id,
                Sha256::digest(link.login.as_bytes()),
                self.reference.player_id
            );
        }
        self.reference.player_id.to_string()
    }
}

/// A verified, expiring Minecraft session. Never persist this object or its token.
#[derive(Debug)]
pub struct AuthenticatedSession {
    pub account: Account,
    pub expires_at: SystemTime,
    access_token: SecretString,
    user_properties: SecretString,
}

impl AuthenticatedSession {
    /// Read the bearer token only when composing the in-memory Java invocation.
    #[must_use]
    pub fn expose_access_token(&self) -> &str {
        self.access_token.expose()
    }

    #[must_use]
    pub fn expose_user_properties(&self) -> &str {
        self.user_properties.expose()
    }

    /// Allow one minute for preparing and starting the Java process.
    #[must_use]
    pub fn needs_refresh(&self) -> bool {
        self.expires_at
            .duration_since(SystemTime::now())
            .map_or(true, |remaining| remaining <= Duration::from_secs(60))
    }
}

/// Errors contain only controlled messages and status codes, never response bodies.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthError {
    #[error("Use a provider display name of 1–128 characters, without control characters.")]
    InvalidProviderName,
    #[error(
        "Enter an HTTP(S) API root or a valid Authlib quick link, without credentials, a query or a fragment."
    )]
    InvalidProviderAddress,
    #[error(
        "This provider uses HTTP. Confirm the warning for this address before connecting; HTTPS connections are never downgraded automatically."
    )]
    InsecureProvider,
    #[error(
        "The provider discovery redirects contain a loop or exceed five hops. Enter its API root directly."
    )]
    ProviderDiscoveryLoop,
    #[error(
        "The provider request timed out. Check its address and your connection, then try again."
    )]
    ProviderTimedOut,
    #[error("The provider returned invalid Yggdrasil metadata. Check its API root and public key.")]
    ProviderMetadata,
    #[error("The provider is limiting requests. Try again after {retry_after_seconds} seconds.")]
    ProviderThrottled { retry_after_seconds: u64 },
    #[error("The provider rejected the account or password. Check them and try again.")]
    ProviderCredentialsRejected,
    #[error("Provider authorization has expired or was revoked. Sign in to this provider again.")]
    ProviderSignInRequired,
    #[error(
        "This provider account has no playable characters. Create a character on its website, then sign in again."
    )]
    ProviderNoProfiles,
    #[error("Choose one of the characters returned by this provider before continuing.")]
    ProviderProfileSelectionRequired,
    #[error(
        "The provider identity, client token or selected character changed unexpectedly. Sign in again."
    )]
    ProviderIdentityChanged,
    #[error("This prepared provider sign-in expired. Sign in again before saving the account.")]
    ProviderPreparationExpired,
    #[error("Enter a provider account and password. The password is never saved.")]
    ProviderLoginRequired,
    #[error("Configure a public Microsoft application client ID before signing in.")]
    MissingClientId,
    #[error("The Microsoft application client ID is invalid.")]
    InvalidClientId,
    #[error("Sign-in was cancelled.")]
    Cancelled,
    #[error("Sign-in timed out. Try again and finish authorization in your browser.")]
    TimedOut,
    #[error("The Microsoft device code expired. Request a new code when you are ready to sign in.")]
    DeviceCodeExpired,
    #[error("Could not create the local browser callback listener.")]
    CallbackUnavailable,
    #[error("Could not open your default browser. Check your system browser settings.")]
    BrowserUnavailable,
    #[error("The browser callback was invalid. Start a new sign-in.")]
    InvalidCallback,
    #[error(
        "The secure credential store is unavailable or locked. Unlock the system keychain or configure a Linux Secret Service. Credentials were not saved to a file."
    )]
    CredentialStoreUnavailable,
    #[error("Saved credentials are missing. Sign in again.")]
    CredentialsMissing,
    #[error("This platform has no supported secure credential store.")]
    UnsupportedSecureStore,
    #[error("The saved account uses a different Microsoft application client ID. Sign in again.")]
    ClientIdChanged,
    #[error("A network request failed during {stage}. Check your connection and try again.")]
    Network { stage: &'static str },
    #[error("{stage} rejected the request (HTTP {status}).")]
    Http { stage: &'static str, status: u16 },
    #[error("Microsoft authorization has expired or was revoked. Sign in again.")]
    SignInRequired,
    #[error(
        "Microsoft application registration rejected public desktop authentication. Check public client flows, its desktop redirect URI and personal-account permissions."
    )]
    ApplicationRegistration,
    #[error("Create an Xbox profile for this Microsoft account, then sign in again.")]
    XboxProfileRequired,
    #[error("Xbox account permissions or family restrictions prevent Minecraft authentication.")]
    XboxAccountRestricted,
    #[error("This account does not have a playable Minecraft: Java Edition entitlement.")]
    NoJavaOwnership,
    #[error(
        "This account has no Minecraft: Java Edition player profile. Complete profile setup at minecraft.net."
    )]
    NoMinecraftProfile,
    #[error("The refreshed Minecraft identity did not match the saved account. Sign in again.")]
    IdentityChanged,
    #[error("{stage} returned an invalid response.")]
    InvalidResponse { stage: &'static str },
    #[error("The operating system could not generate authentication randomness.")]
    RandomnessUnavailable,
}

pub type Result<T> = std::result::Result<T, AuthError>;

const MICROSOFT_SCOPE: &str = "XboxLive.signin XboxLive.offline_access";
