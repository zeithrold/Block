use super::{fixtures::Fixture, verification::session};
use crate::{
    CancellationToken,
    catalog::{Catalog, GameDirectoryGuard, InstallStatus, Instance, InstanceConfiguration},
    executor::ExecutionEvent,
};
use anyhow::Context as _;
use std::sync::{Arc, OnceLock, mpsc};

#[tokio::test]
async fn superseded_publication_keeps_captured_plan_and_game_lock() -> anyhow::Result<()> {
    for delete in [false, true] {
        let mut fixture = Fixture::new()?;
        let captured: Arc<OnceLock<(Catalog, Instance)>> = Arc::default();
        let callback = captured.clone();
        let (actions, results) = mpsc::channel::<anyhow::Result<()>>();
        let (logs, messages) = mpsc::channel();
        let (library, instance) = fixture.registered(Arc::new(move |event| match event {
            ExecutionEvent::Stage { name } if name == "Ready" => {
                let result = (|| {
                    let (catalog, instance) =
                        callback.get().context("captured Instance missing")?;
                    if delete {
                        catalog.delete_instance(&instance.id)?;
                    } else {
                        let mut configuration = InstanceConfiguration::from(instance);
                        configuration.memory_mib = 4096;
                        catalog.save_instance(&instance.id, configuration)?;
                    }
                    Ok(())
                })();
                assert!(actions.send(result).is_ok());
            }
            ExecutionEvent::Log { line, .. } => assert!(logs.send(line).is_ok()),
            _ => {}
        }))?;
        assert!(
            captured
                .set((library.catalog().clone(), instance.clone()))
                .is_ok()
        );
        let plan = library
            .prepare_launch(
                &instance.id,
                &session(),
                &CancellationToken::new(),
                None,
                None,
            )
            .await?;
        results.try_recv()??;
        assert_eq!(plan.game_directory(), fixture.root());
        assert!(GameDirectoryGuard::for_instance(&instance).is_err());
        assert!(
            messages
                .try_iter()
                .any(|line| line.contains("verification status could not be saved"))
        );
        let snapshot = library.catalog().snapshot()?;
        if delete {
            assert!(snapshot.instances.is_empty());
        } else {
            let (saved, selected) = snapshot.selection(&instance.id)?;
            assert_eq!(saved.memory_mib, 4096);
            assert_eq!(selected.status, InstallStatus::Pending);
        }
        drop(plan);
    }
    Ok(())
}

#[tokio::test]
async fn failed_publication_preserves_the_original_source_error() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let captured: Arc<OnceLock<(Catalog, Instance)>> = Arc::default();
    let callback = captured.clone();
    let (actions, results) = mpsc::channel::<anyhow::Result<()>>();
    let (logs, messages) = mpsc::channel();
    let client = fixture.root().join("versions/1.20.1/1.20.1.jar");
    let changed = client.clone();
    let (library, instance) = fixture.registered(Arc::new(move |event| match event {
        ExecutionEvent::Stage { name } if name == "Ready" => {
            let result = (|| {
                let (catalog, instance) = callback.get().context("captured Instance missing")?;
                std::fs::write(&changed, b"corruption after preflight")?;
                catalog.delete_instance(&instance.id)?;
                Ok(())
            })();
            assert!(actions.send(result).is_ok());
        }
        ExecutionEvent::Log { line, .. } => assert!(logs.send(line).is_ok()),
        _ => {}
    }))?;
    assert!(
        captured
            .set((library.catalog().clone(), instance.clone()))
            .is_ok()
    );
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
        .context("changed source accepted")?;
    results.try_recv()??;
    assert!(
        error
            .downcast_ref::<crate::catalog::ExistingSourceFailure>()
            .is_some(),
        "{error:#}"
    );
    assert!(error.to_string().contains("corrupt"), "{error:#}");
    assert!(!error.to_string().contains("superseded"), "{error:#}");
    assert!(
        messages
            .try_iter()
            .any(|line| line.contains("verification status could not be saved"))
    );
    assert!(library.catalog().snapshot()?.instances.is_empty());
    assert_eq!(std::fs::read(client)?, b"corruption after preflight");
    Ok(())
}
