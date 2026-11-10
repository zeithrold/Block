mod common;
use block_launcher::catalog::Catalog;
use std::fs;

#[test]
fn invalid_stored_memory_is_rejected_without_migration_or_rewrite() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    common::instance(&catalog)?;
    let valid = common::load(root.path())?;
    for memory in [256, 65536] {
        let mut document = valid.clone();
        common::first_instance_mut(&mut document)?
            .insert("memory_mib".into(), toml::Value::Integer(memory));
        common::save(root.path(), &document)?;
        let before = fs::read(common::path(root.path()))?;
        assert!(catalog.snapshot().is_err());
        assert!(common::instance(&catalog).is_err());
        assert_eq!(fs::read(common::path(root.path()))?, before);
    }
    Ok(())
}

#[test]
fn instance_validation_accepts_only_the_one_to_thirty_two_gb_bounds() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    for (memory, accepted) in [(1023, false), (1024, true), (32768, true), (32769, false)] {
        let mut candidate = instance.clone();
        candidate.memory_mib = memory;
        assert_eq!(candidate.validate().is_ok(), accepted, "memory {memory}");
    }
    Ok(())
}
