//! Strict, lossless structural models for official Minecraft Java version metadata.
//!
//! Fixed-shape objects reject unknown fields. Maps are reserved for named feature
//! flags, native classifiers, and OS-to-classifier mappings. Timestamp strings
//! preserve their original spelling; parsing does not interpret launch rules.

pub mod model;

use serde::de::DeserializeOwned;

/// Decode one complete JSON document, including a check for trailing content.
///
/// # Errors
/// Returns a field-path diagnostic on a schema mismatch or malformed JSON.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<T> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let result = serde_path_to_error::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(result)
}
