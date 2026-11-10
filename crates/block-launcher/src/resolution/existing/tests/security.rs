use super::{fixtures::Fixture, verification::session};
use crate::{CancellationToken, catalog::InstallStatus, executor::ExecutionEvent};
use anyhow::Context as _;
use std::sync::{Arc, mpsc};

#[tokio::test]
async fn persisted_failed_status_excludes_arbitrary_source_metadata_credentials()
-> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let (logs, messages) = mpsc::channel();
    let (library, instance) = fixture.registered(Arc::new(move |event| {
        if let ExecutionEvent::Log { line, .. } = event {
            assert!(logs.send(line).is_ok());
        }
    }))?;
    let private = "https://alice:unlabelled-value@example.invalid/?q=another-unlabelled-value";
    fixture.manifest["assetIndex"]["size"] = private.into();
    fixture.save_manifest()?;
    let error = library
        .prepare_launch(
            &instance.id,
            &session(),
            &CancellationToken::new(),
            None,
            None,
        )
        .await
        .err()
        .context("malformed metadata accepted")?;
    assert!(
        format!("{error:#}").contains("unlabelled-value"),
        "{error:#}"
    );
    let snapshot = library.catalog().snapshot()?;
    let InstallStatus::Failed { message } = &snapshot.selection(&instance.id)?.1.status else {
        anyhow::bail!("malformed source did not record Failed");
    };
    let document = std::fs::read_to_string(library.catalog().root().join("block.toml"))?;
    assert!(!document.contains("unlabelled-value"));
    assert!(!document.contains("alice"));
    assert!(
        message.contains("metadata") && message.contains("invalid"),
        "{message}"
    );
    assert!(
        messages
            .try_iter()
            .all(|line| !line.contains("unlabelled-value"))
    );
    Ok(())
}
