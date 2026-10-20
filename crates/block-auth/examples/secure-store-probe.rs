//! Explicit native-store diagnostic. Never accesses an existing account credential.

#[tokio::main]
async fn main() -> block_auth::Result<()> {
    block_auth::probe_secure_storage().await?;
    println!("PASS: synthetic credential write, read and deletion in the native secure store.");
    Ok(())
}
