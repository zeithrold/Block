use uuid::Uuid;

use crate::{Account, AuthError, Result, SecretString};

const SERVICE: &str = "dev.block.launcher.microsoft";
const AUTHLIB_SERVICE: &str = "dev.block.launcher.authlib";

fn service(reference: &str) -> &'static str {
    if reference.starts_with("authlib:") {
        AUTHLIB_SERVICE
    } else {
        SERVICE
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[cfg(not(test))]
fn entry(reference: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(service(reference), reference)
        .map_err(|_| AuthError::CredentialStoreUnavailable)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const fn map_store_error(error: &keyring::Error) -> AuthError {
    if matches!(error, keyring::Error::NoEntry) {
        AuthError::CredentialsMissing
    } else {
        AuthError::CredentialStoreUnavailable
    }
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "Credential operations must remain crate-private under unreachable_pub."
)]
pub(crate) async fn load(account: &Account) -> Result<SecretString> {
    let reference = account.credential_ref.clone();
    tokio::task::spawn_blocking(move || load_blocking(&reference))
        .await
        .map_err(|_| AuthError::CredentialStoreUnavailable)?
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn load_blocking(reference: &str) -> Result<SecretString> {
    #[cfg(test)]
    {
        fixture_store()
            .lock()
            .map_err(|_| AuthError::CredentialStoreUnavailable)?
            .get(&(service(reference), reference.to_owned()))
            .map(|secret| SecretString::new(secret.expose().to_owned()))
            .ok_or(AuthError::CredentialsMissing)
    }
    #[cfg(not(test))]
    entry(reference)?
        .get_password()
        .map(SecretString::new)
        .map_err(|error| map_store_error(&error))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn load_blocking(_reference: &str) -> Result<SecretString> {
    Err(AuthError::UnsupportedSecureStore)
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "Credential operations must remain crate-private under unreachable_pub."
)]
pub(crate) async fn save(reference: String, secret: SecretString) -> Result<()> {
    tokio::task::spawn_blocking(move || save_blocking(&reference, &secret))
        .await
        .map_err(|_| AuthError::CredentialStoreUnavailable)?
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn save_blocking(reference: &str, secret: &SecretString) -> Result<()> {
    #[cfg(test)]
    {
        fixture_store()
            .lock()
            .map_err(|_| AuthError::CredentialStoreUnavailable)?
            .insert(
                (service(reference), reference.to_owned()),
                SecretString::new(secret.expose().to_owned()),
            );
        Ok(())
    }
    #[cfg(not(test))]
    entry(reference)?
        .set_password(secret.expose())
        .map_err(|error| map_store_error(&error))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn save_blocking(_reference: &str, _secret: &SecretString) -> Result<()> {
    Err(AuthError::UnsupportedSecureStore)
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "Credential operations must remain crate-private under unreachable_pub."
)]
pub(crate) async fn delete(account: &Account) -> Result<()> {
    let reference = account.credential_ref.clone();
    tokio::task::spawn_blocking(move || delete_blocking(&reference))
        .await
        .map_err(|_| AuthError::CredentialStoreUnavailable)?
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn delete_blocking(reference: &str) -> Result<()> {
    #[cfg(test)]
    {
        fixture_store()
            .lock()
            .map_err(|_| AuthError::CredentialStoreUnavailable)?
            .remove(&(service(reference), reference.to_owned()));
        Ok(())
    }
    #[cfg(not(test))]
    match entry(reference)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(AuthError::CredentialStoreUnavailable),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn delete_blocking(_reference: &str) -> Result<()> {
    Err(AuthError::UnsupportedSecureStore)
}

/// Verify write/read/delete of a fresh, synthetic entry in this app's secure store.
/// Does not enumerate or read existing accounts, and never writes plaintext files.
///
/// # Errors
/// Returns an explicit error if the backend is missing, locked or inaccessible.
pub async fn probe_secure_storage() -> Result<()> {
    probe_storage(false).await
}

pub(crate) async fn probe_authlib_storage() -> Result<()> {
    probe_storage(true).await
}

async fn probe_storage(authlib: bool) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let reference = format!(
            "{}storage-check-{}",
            if authlib { "authlib:" } else { "" },
            Uuid::new_v4()
        );
        let value = SecretString::new(Uuid::new_v4().to_string());
        save_blocking(&reference, &value)?;
        let read = load_blocking(&reference);
        let cleanup = delete_blocking(&reference);
        let matches = read.is_ok_and(|loaded| loaded.expose() == value.expose());
        cleanup?;
        if matches {
            Ok(())
        } else {
            Err(AuthError::CredentialStoreUnavailable)
        }
    })
    .await
    .map_err(|_| AuthError::CredentialStoreUnavailable)?
}

#[cfg(test)]
fn fixture_store()
-> &'static std::sync::Mutex<std::collections::HashMap<(&'static str, String), SecretString>> {
    static STORE: std::sync::LazyLock<
        std::sync::Mutex<std::collections::HashMap<(&'static str, String), SecretString>>,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    &STORE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_credentials_are_distinct_from_locked_backend() {
        assert_eq!(
            map_store_error(&keyring::Error::NoEntry),
            AuthError::CredentialsMissing
        );
        assert_eq!(
            map_store_error(&keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::other("SYNTHETIC_SECRET")
            ))),
            AuthError::CredentialStoreUnavailable
        );
    }
}
