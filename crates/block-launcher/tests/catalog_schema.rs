mod common;

use anyhow::Context as _;
use block_launcher::catalog::{Catalog, InstanceConfiguration};
use std::fs;

#[test]
fn unsupported_schema_fails_explicitly_and_preserves_the_entire_document() -> anyhow::Result<()> {
    for version in [1, 3] {
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let original = format!(
            "[accounts]\nselected = 'public-account-id'\n[catalog]\nschema_version = {version}\n[catalog.snapshot]\nprofiles = []\ninstances = []\n"
        );
        fs::write(common::path(root.path()), &original)?;
        let error = catalog
            .snapshot()
            .err()
            .context("unsupported catalog schema was accepted")?
            .to_string();
        assert!(error.contains("unsupported catalog schema"), "{error}");
        assert!(common::instance(&catalog).is_err());
        assert_eq!(fs::read_to_string(common::path(root.path()))?, original);
    }
    Ok(())
}

#[test]
fn flat_schema_roundtrips_and_atomic_edits_preserve_unrelated_sections() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    let mut document = common::load(root.path())?;
    document.insert(
        "accounts".into(),
        common::table(&[("selected", "public-account-id".into())]),
    );
    document.insert(
        "other_preferences".into(),
        common::table(&[("theme", "dark".into())]),
    );
    common::save(root.path(), &document)?;
    let mut configuration = InstanceConfiguration::from(&instance);
    configuration.name = "Renamed".into();
    configuration.default_account = Some("public-account-id".into());
    catalog.save_instance(&instance.id, configuration)?;
    let saved = common::load(root.path())?;
    assert_eq!(saved["catalog"]["schema_version"].as_integer(), Some(2));
    assert_eq!(saved["accounts"], document["accounts"]);
    assert_eq!(saved["other_preferences"], document["other_preferences"]);
    assert_eq!(saved.get("storage"), document.get("storage"));
    let snapshot = saved["catalog"]["snapshot"]
        .as_table()
        .context("persisted snapshot is not a table")?;
    assert_eq!(
        snapshot.keys().map(String::as_str).collect::<Vec<_>>(),
        ["instances"]
    );
    let reopened = Catalog::open(root.path())?.snapshot()?;
    let (saved, target) = reopened.selection(&instance.id)?;
    assert_eq!(saved.name, "Renamed");
    assert_eq!(saved.default_account.as_deref(), Some("public-account-id"));
    assert_eq!(target.id, "vanilla");
    let json = serde_json::to_value(&reopened)?;
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<block_launcher::catalog::Snapshot>(
            json.clone()
        )?)?,
        json
    );
    Ok(())
}

#[test]
fn duplicate_instances_missing_targets_and_removed_settings_are_rejected_without_reset()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    common::instance(&catalog)?;
    let original = common::load(root.path())?;
    for change in 0..4 {
        let mut document = original.clone();
        match change {
            0 => {
                let instances = common::snapshot_mut(&mut document)?
                    .get_mut("instances")
                    .and_then(toml::Value::as_array_mut)
                    .context("persisted instances are not an array")?;
                instances.push(instances[0].clone());
            }
            1 => {
                common::first_instance_mut(&mut document)?
                    .insert("target_id".into(), "missing-target".into());
            }
            2 => {
                common::snapshot_mut(&mut document)?
                    .insert("profiles".into(), toml::Value::Array(Vec::new()));
            }
            3 => {
                common::first_instance_mut(&mut document)?.remove("appearance");
            }
            _ => unreachable!(),
        }
        common::save(root.path(), &document)?;
        let before = fs::read(common::path(root.path()))?;
        assert!(catalog.snapshot().is_err());
        assert!(common::instance(&catalog).is_err());
        assert_eq!(fs::read(common::path(root.path()))?, before);
    }
    Ok(())
}
