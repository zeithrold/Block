mod common;

use block_launcher::catalog::{Catalog, ProfileConfiguration};
use std::path::Path;

fn set_stored_memory(root: &Path, memory_mib: u32) -> anyhow::Result<()> {
    let mut document = common::load(root)?;
    common::first_profile_mut(&mut document)?.insert(
        "memory_mib".into(),
        toml::Value::Integer(i64::from(memory_mib)),
    );
    common::save(root, &document)
}

#[test]
fn out_of_range_stored_profile_memory_clamps_on_load_and_on_next_write() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (_instance, profile) = catalog.create_instance_configured(
        "Memory game".into(),
        "1.20.6".into(),
        None,
        None,
        ProfileConfiguration {
            name: "Memory profile".into(),
            ..ProfileConfiguration::default()
        },
    )?;
    for (stored, expected) in [(256, 1024), (65536, 32768), (4096, 4096)] {
        set_stored_memory(root.path(), stored)?;
        let state = Catalog::open(root.path())?.snapshot()?;
        assert_eq!(state.profiles[0].memory_mib, expected);
    }
    // The clamped snapshot is valid and can be written back through the normal API.
    set_stored_memory(root.path(), 256)?;
    let state = Catalog::open(root.path())?.snapshot()?;
    let mut profile = state
        .profiles
        .iter()
        .find(|candidate| candidate.id == profile.id)
        .ok_or_else(|| anyhow::anyhow!("profile missing"))?
        .clone();
    profile.memory_mib = state.profiles[0].memory_mib;
    catalog.save_profile(profile)?;
    let document = common::load(root.path())?;
    assert_eq!(
        document["catalog"]["snapshot"]["profiles"][0]["memory_mib"].as_integer(),
        Some(1024)
    );
    Ok(())
}

#[test]
fn profile_validation_accepts_only_the_one_to_thirty_two_gb_bounds() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let (_instance, profile) = catalog.create_instance_configured(
        "Bounds game".into(),
        "1.20.6".into(),
        None,
        None,
        ProfileConfiguration {
            name: "Bounds profile".into(),
            ..ProfileConfiguration::default()
        },
    )?;
    for (memory, accepted) in [(1023, false), (1024, true), (32768, true), (32769, false)] {
        let mut candidate = profile.clone();
        candidate.memory_mib = memory;
        assert_eq!(candidate.validate().is_ok(), accepted, "memory {memory}");
    }
    Ok(())
}
