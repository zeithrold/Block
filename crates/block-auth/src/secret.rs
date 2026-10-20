use std::fmt;

use serde::{Deserialize, Deserializer};
use zeroize::Zeroizing;

/// Memory-only secret with redacted formatting and zeroization on drop.
/// Deliberately does not implement Clone, Display or Serialize.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// Explicitly expose a secret for an authentication request or Java argv.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}
