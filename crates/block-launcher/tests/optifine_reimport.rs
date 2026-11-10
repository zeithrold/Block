use anyhow::Context as _;
use block_launcher::{
    Resource,
    catalog::{Catalog, Instance, InstanceConfiguration, LoaderKind},
    install::optifine::OptiFineJar,
};

fn jar(minecraft: &str) -> OptiFineJar {
    OptiFineJar {
        minecraft: minecraft.into(),
        version: format!("{minecraft}_HD_U_I6"),
        launchwrapper: "2.3".into(),
        forge: None,
        resource: Resource {
            sha1: "a".repeat(40),
            size: 123,
            url: "block-import:synthetic".into(),
            sha512: None,
            alternate_urls: Vec::new(),
        },
    }
}

fn configuration(instance: &Instance, name: &str) -> InstanceConfiguration {
    InstanceConfiguration {
        name: name.into(),
        ..InstanceConfiguration::from(instance)
    }
}

#[test]
fn reimport_replaces_and_clears_optifine_in_one_commit() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;

    catalog.save_instance_with_optifine(
        &instance.id,
        configuration(&instance, "Renamed"),
        Some(jar("1.20.6")),
    )?;
    let state = catalog.snapshot()?;
    let target = &state.instances[0].targets[0];
    assert_eq!(state.instances[0].name, "Renamed");
    assert_eq!(
        target
            .optifine
            .as_ref()
            .map(|selected| selected.version.as_str()),
        Some("1.20.6_HD_U_I6")
    );
    assert_eq!(target.optifine_mod, None);

    catalog.save_instance_with_optifine(&instance.id, configuration(&instance, "Renamed"), None)?;
    let state = catalog.snapshot()?;
    assert_eq!(state.instances[0].targets[0].optifine, None);
    assert_eq!(state.instances[0].targets[0].optifine_mod, None);
    Ok(())
}

#[test]
fn reimport_rejects_incompatible_game_or_loader_without_writing() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = catalog.create_instance("Game".into(), "1.20.6".into(), None, None)?;

    let error = catalog
        .save_instance_with_optifine(
            &instance.id,
            configuration(&instance, "Should not persist"),
            Some(jar("1.21.1")),
        )
        .err()
        .context("wrong game must be rejected")?;
    assert!(error.to_string().contains("1.21.1"), "{error}");
    let state = catalog.snapshot()?;
    assert_eq!(state.instances[0].name, "Game");
    assert_eq!(state.instances[0].targets[0].optifine, None);

    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let fabric = catalog.create_instance_with_loader(
        "1.20.6".into(),
        None,
        Some((LoaderKind::Fabric, "0.16.0".into())),
        InstanceConfiguration {
            name: "Fabric game".into(),
            target_id: "fabric-0.16.0".into(),
            ..Default::default()
        },
    )?;
    let error = catalog
        .save_instance_with_optifine(
            &fabric.id,
            configuration(&fabric, "Should not persist"),
            Some(jar("1.20.6")),
        )
        .err()
        .context("unsupported loader must be rejected")?;
    assert!(error.to_string().contains("Fabric"), "{error}");

    let state = catalog.snapshot()?;
    assert_eq!(state.instances[0].name, "Fabric game");
    assert_eq!(
        state.instances[0]
            .targets
            .iter()
            .find(|target| target.kind == LoaderKind::Fabric)
            .and_then(|target| target.optifine.clone()),
        None
    );
    Ok(())
}
