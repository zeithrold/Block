use super::{
    fixtures::Fixture,
    verification::{session, status},
};
use crate::{catalog::InstallStatus, session::SessionProvider as _};
use anyhow::Context as _;
use std::sync::Arc;

#[tokio::test]
async fn fingerprint_failure_before_supervisor_records_missing_parent_without_creating_a_session()
-> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let (library, instance) = fixture.registered(Arc::new(|_| {}))?;
    fixture.manifest["inheritsFrom"] = "missing-parent".into();
    fixture.save_manifest()?;
    let error = crate::session_host::spawn_session(
        library.catalog().root(),
        &fixture.context.store.root().join("unused-supervisor"),
        &instance.id,
        &session().session()?,
        std::time::SystemTime::now() + std::time::Duration::from_secs(600),
    )
    .await
    .err()
    .context("missing parent was accepted before supervisor startup")?;
    assert!(
        error
            .downcast_ref::<crate::catalog::ExistingSourceFailure>()
            .is_some(),
        "{error:#}"
    );
    let InstallStatus::Failed { message } = status(&library, &instance)? else {
        anyhow::bail!("missing parent did not record Failed");
    };
    assert!(
        format!("{error:#}").contains("missing-parent.json"),
        "{error:#}"
    );
    assert!(
        message.contains("metadata") && message.contains("missing"),
        "{message}"
    );
    assert!(!message.contains("missing-parent"));
    assert!(crate::session_host::list_runs(library.catalog().root())?.is_empty());
    Ok(())
}
