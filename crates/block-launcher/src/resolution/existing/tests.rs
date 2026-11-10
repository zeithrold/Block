#[cfg(unix)]
#[path = "tests/fingerprint.rs"]
mod fingerprint;
#[path = "tests/fixtures.rs"]
mod fixtures;
#[cfg(unix)]
#[path = "tests/publication.rs"]
mod publication;
#[path = "tests/repair.rs"]
mod repair;
#[path = "tests/resolution.rs"]
mod resolution;
#[path = "tests/resources.rs"]
mod resources;
#[cfg(unix)]
#[path = "tests/security.rs"]
mod security;
#[cfg(unix)]
#[path = "tests/verification.rs"]
mod verification;
