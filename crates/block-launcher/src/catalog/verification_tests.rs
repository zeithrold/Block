use super::{Catalog, InstallStatus, Instance, TargetSource};
use anyhow::Context as _;
use std::fs;

fn existing(catalog: &Catalog) -> anyhow::Result<Instance> {
    catalog.create_instance("Verification".into(), "1.20.1".into(), None, None)?;
    catalog.transaction(true, |state| {
        state.instances[0].ownership = super::InstanceOwnership::Registered;
        state.instances[0].targets[0].source = TargetSource::Existing {
            version_id: "1.20.1".into(),
        };
        Ok(state.instances[0].clone())
    })
}

#[test]
fn verification_rejects_superseded_observations_without_rewriting_bytes() -> anyhow::Result<()> {
    for change in 0..5 {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let expected = existing(&catalog)?;
        catalog.transaction(true, |state| {
            match change {
                0 => state.instances.clear(),
                1 => state.instances[0].memory_mib = 4096,
                2 => {
                    state.instances[0].targets[0].source = TargetSource::Existing {
                        version_id: "other-version".into(),
                    }
                }
                3 => state.instances[0].targets[0].status = InstallStatus::Ready,
                4 => {
                    let mut target = state.instances[0].targets[0].clone();
                    target.id = "other".into();
                    state.instances[0].targets.push(target);
                    state.instances[0].target_id = "other".into();
                }
                _ => unreachable!(),
            }
            Ok(())
        })?;
        let path = root.path().join(crate::config::FILE);
        let before = fs::read(&path)?;
        let error = catalog
            .record_existing_verification(&expected, InstallStatus::Ready)
            .err()
            .context("superseded observation was accepted")?;
        assert!(error.to_string().contains("superseded"), "{error:#}");
        assert_eq!(fs::read(path)?, before, "change {change}");
    }
    Ok(())
}

#[test]
fn verification_changes_only_selected_status_and_preserves_unrelated_sections() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let expected = existing(&catalog)?;
    let mut document = crate::config::load(root.path())?;
    document.insert(
        "accounts".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "selected".into(),
            "public-account".into(),
        )])),
    );
    document.insert(
        "preferences".into(),
        toml::Value::Table(toml::Table::from_iter([("theme".into(), "dark".into())])),
    );
    crate::config::write(root.path(), &document)?;
    catalog.record_existing_verification(&expected, InstallStatus::Ready)?;
    let saved_document = crate::config::load(root.path())?;
    for key in ["accounts", "preferences", "storage"] {
        assert_eq!(saved_document.get(key), document.get(key));
    }
    let mut saved = catalog.snapshot()?.selection(&expected.id)?.0.clone();
    assert_eq!(saved.targets[0].status, InstallStatus::Ready);
    saved.targets[0].status = InstallStatus::Pending;
    assert_eq!(
        serde_json::to_value(saved)?,
        serde_json::to_value(expected)?
    );
    Ok(())
}
