//! Compile-time source identity for native verification and packaged artifacts.

/// Report immutable build identity without reading the runtime environment.
/// Unstamped manual builds explicitly report null source identity fields.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Build identity is exposed only to the parent CLI module."
)]
pub(super) fn current() -> serde_json::Value {
    serde_json::json!({
        "product_id": "one.ztd.block",
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "source_sha256": option_env!("BLOCK_SOURCE_SHA256"),
        "source_commit": option_env!("BLOCK_SOURCE_COMMIT"),
    })
}
