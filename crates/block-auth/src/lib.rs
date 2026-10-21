//! Microsoft desktop authentication and operating-system protected refresh tokens.
//!
//! Only account metadata is serializable. Sessions, authorization codes, PKCE
//! verifiers and credentials deliberately have no serialization implementation.

mod callback;
mod credentials;
mod microsoft;
mod secret;

pub use callback::PendingSignIn;
pub use credentials::probe_secure_storage;
pub use microsoft::{MicrosoftAuth, SignInStage};
pub use secret::SecretString;

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable public account identity; no credentials belong in a Profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct AccountRef {
    pub player_id: Uuid,
}

/// Authentication authority, intentionally limited to the implemented provider.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountProvider {
    Microsoft,
}

/// Safe-to-persist player metadata. Ownership is rechecked for every new session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Account {
    pub reference: AccountRef,
    pub provider: AccountProvider,
    pub player_name: String,
    pub client_id: String,
    pub credential_ref: String,
}

/// A verified, expiring Minecraft session. Never persist this object or its token.
#[derive(Debug)]
pub struct AuthenticatedSession {
    pub account: Account,
    pub expires_at: SystemTime,
    access_token: SecretString,
}

impl AuthenticatedSession {
    /// Read the bearer token only when composing the in-memory Java invocation.
    #[must_use]
    pub fn expose_access_token(&self) -> &str {
        self.access_token.expose()
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
    #[error("Configure a public Microsoft application client ID before signing in.")]
    MissingClientId,
    #[error("The Microsoft application client ID is invalid.")]
    InvalidClientId,
    #[error("Sign-in was cancelled.")]
    Cancelled,
    #[error("Sign-in timed out. Try again and finish authorization in your browser.")]
    TimedOut,
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
        "Microsoft application registration rejected public desktop authentication. Check its desktop redirect URI and personal-account permissions."
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

const MICROSOFT_SCOPE: &str = "XboxLive.SignIn XboxLive.offline_access";
