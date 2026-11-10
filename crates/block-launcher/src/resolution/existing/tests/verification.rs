use std::{fs, sync::Arc};

use super::fixtures::Fixture;
use crate::{
    CancellationToken,
    catalog::{GameDirectoryGuard, InstallStatus, Instance},
    executor::{ExecutionEvent, ResourceLease},
    library::Library,
    session::{GameSession, OfflineSession, SessionProvider},
};
use anyhow::Context as _;

pub(super) fn status(library: &Library, instance: &Instance) -> anyhow::Result<InstallStatus> {
    Ok(library
        .catalog()
        .snapshot()?
        .selection(&instance.id)?
        .1
        .status
        .clone())
}

pub(super) fn session() -> OfflineSession {
    OfflineSession::Offline {
        username: "Player".into(),
    }
}

#[tokio::test]
async fn source_damage_records_failed_without_modifying_source_bytes() -> anyhow::Result<()> {
    for relative in [
        "versions/1.20.1/1.20.1.json",
        "versions/1.20.1/1.20.1.jar",
        "library",
        "assets/indexes/test.json",
        "asset-object",
    ] {
        for missing in [false, true] {
            let mut fixture = Fixture::new()?;
            let (library, instance) = fixture.registered(Arc::new(|_| {}))?;
            // Select a fixture library independently of this test host's classifier.
            let relative = match relative {
                "library" => format!(
                    "libraries/{}",
                    fixture.manifest["libraries"][0]["downloads"]["artifact"]["path"]
                        .as_str()
                        .context("library path")?
                ),
                "asset-object" => {
                    let index: serde_json::Value = serde_json::from_slice(&fs::read(
                        fixture.root().join("assets/indexes/test.json"),
                    )?)?;
                    let hash = index["objects"]["minecraft/sound.ogg"]["hash"]
                        .as_str()
                        .context("fixture asset hash")?;
                    format!("assets/objects/{}/{hash}", &hash[..2])
                }
                relative => relative.to_owned(),
            };
            let path = fixture.root().join(&relative);
            if missing {
                fs::remove_file(&path)?;
            } else {
                fs::write(&path, b"corrupt source fixture")?;
            }
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
                .context("damaged source accepted")?;
            assert!(
                error
                    .downcast_ref::<crate::catalog::ExistingSourceFailure>()
                    .is_some(),
                "{error:#}"
            );
            let InstallStatus::Failed { message } = status(&library, &instance)? else {
                anyhow::bail!("source damage did not record Failed: {error:#}");
            };
            assert!(
                message.contains(if missing {
                    "missing"
                } else if relative.ends_with(".json") && relative.starts_with("versions/") {
                    "invalid"
                } else {
                    "corrupt"
                }),
                "{message}"
            );
            if !missing {
                assert_eq!(fs::read(path)?, b"corrupt source fixture");
            } else {
                assert_eq!(
                    fs::metadata(path)
                        .err()
                        .context("missing source was silently recreated")?
                        .kind(),
                    std::io::ErrorKind::NotFound
                );
            }
        }
    }
    Ok(())
}

struct Expired;
impl SessionProvider for Expired {
    fn session(&self) -> anyhow::Result<GameSession> {
        anyhow::bail!("synthetic authentication expired")
    }
}

#[tokio::test]
async fn cancellation_authentication_java_and_busy_failures_preserve_pending() -> anyhow::Result<()>
{
    for case in 0..8 {
        let mut fixture = Fixture::new()?;
        let (library, instance) = fixture.registered(Arc::new(|_| {}))?;
        let cancel = CancellationToken::new();
        let mut directory = None;
        let mut resources = None;
        match case {
            0 => cancel.cancel(),
            1 => {}
            2 => {
                let crate::recipe::RuntimeSelection::Local { java } = &instance.runtime else {
                    anyhow::bail!("expected local fixture Java");
                };
                fs::write(java, b"#!/bin/sh\nexit 9\n")?;
            }
            3 => directory = Some(GameDirectoryGuard::for_instance(&instance)?),
            4 => resources = Some(ResourceLease::exclusive(library.catalog().root())?),
            5 => {
                let receipt = library.catalog().root().join(format!(
                    "installations/{}/{}/existing.json",
                    instance.id, instance.target_id
                ));
                super::fixtures::write(&receipt, b"invalid Block cache receipt")?;
            }
            6 => {
                fixture.manifest["mainClass"] = "example.Unsupported".into();
                fixture.save_manifest()?;
            }
            7 => {
                let mut receipt = fixture
                    .resolve()
                    .await?
                    .existing
                    .context("existing fixture")?
                    .receipt;
                let client = receipt
                    .entries
                    .iter_mut()
                    .find(|entry| entry.logical_path.ends_with("1.20.1.jar"))
                    .context("client receipt entry")?;
                client.path = crate::plan::ResourcePath::Cache {
                    relative: "objects/missing-repaired-client".into(),
                };
                let path = library.catalog().root().join(format!(
                    "installations/{}/{}/existing.json",
                    instance.id, instance.target_id
                ));
                super::fixtures::write(&path, &serde_json::to_vec(&receipt)?)?;
            }
            _ => unreachable!(),
        }
        let offline = session();
        let provider: &dyn SessionProvider = if case == 1 { &Expired } else { &offline };
        let error = library
            .prepare_launch(&instance.id, provider, &cancel, None, None)
            .await
            .err()
            .context("excluded failure unexpectedly succeeded")?;
        assert!(
            error
                .downcast_ref::<crate::catalog::ExistingSourceFailure>()
                .is_none_or(|failure| !failure.path.starts_with(fixture.root())),
            "{error:#}"
        );
        assert_eq!(
            status(&library, &instance)?,
            InstallStatus::Pending,
            "case {case}"
        );
        drop((directory, resources));
    }
    Ok(())
}

#[tokio::test]
async fn stale_fingerprint_is_not_source_damage_and_cancelled_observation_is_not_published()
-> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let cancel = CancellationToken::new();
    let observed = cancel.clone();
    let (library, instance) = fixture.registered(Arc::new(move |event| {
        if matches!(event, ExecutionEvent::Stage { name } if name == "Ready") {
            observed.cancel();
        }
    }))?;
    let error = library
        .prepare_launch(
            &instance.id,
            &session(),
            &CancellationToken::new(),
            None,
            Some("stale-valid-request"),
        )
        .await
        .err()
        .context("stale fingerprint accepted")?;
    assert!(error.to_string().contains("Launch settings changed"));
    assert_eq!(status(&library, &instance)?, InstallStatus::Pending);
    drop(
        library
            .prepare_launch(&instance.id, &session(), &cancel, None, None)
            .await?,
    );
    assert!(cancel.is_cancelled());
    assert_eq!(status(&library, &instance)?, InstallStatus::Pending);
    Ok(())
}
