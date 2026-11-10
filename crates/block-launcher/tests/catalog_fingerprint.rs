mod common;

use block_launcher::{
    catalog::{Catalog, InstanceConfiguration, TargetSource},
    game_import::GameDataEvidence,
    recipe::RuntimeSelection,
};
use std::fs;

#[test]
fn fingerprint_changes_for_each_launch_input_and_installed_manifest_content() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let catalog = Catalog::open(root.path())?;
    let instance = common::instance(&catalog)?;
    let snapshot = catalog.snapshot()?;
    let original = snapshot.launch_fingerprint(&instance.id)?;
    for change in 0..9 {
        let mut changed = snapshot.clone();
        let instance = &mut changed.instances[0];
        match change {
            0 => {
                instance.runtime = RuntimeSelection::Local {
                    java: std::env::current_exe()?,
                }
            }
            1 => instance.memory_mib = 4096,
            2 => instance.jvm_args.push("-Dfixture=true".into()),
            3 => instance.game_args.push("--demo".into()),
            4 => instance.default_account = Some("another-public-player-id".into()),
            5 => instance.game_directory = instance.resource_directory.join("another-game"),
            6 => instance.resource_directory = instance.resource_directory.join("another-source"),
            7 => instance.width = 1440,
            8 => {
                let mut target = instance.targets[0].clone();
                target.id = "other".into();
                instance.targets.push(target);
                instance.target_id = "other".into();
            }
            _ => unreachable!(),
        }
        let id = instance.id.clone();
        assert_ne!(changed.launch_fingerprint(&id)?, original, "{change}");
    }
    let manifest = instance.resource_directory.join("installed.json");
    fs::write(&manifest, b"{\"fixture\":1}")?;
    let mut target = instance.targets[0].clone();
    target.source = TargetSource::Managed {
        manifest: Some(manifest.clone()),
    };
    catalog.set_target(&instance.id, &target)?;
    let with_manifest = catalog.snapshot()?;
    let before = with_manifest.launch_fingerprint(&instance.id)?;
    fs::write(&manifest, b"{\"fixture\":2}")?;
    assert_ne!(with_manifest.launch_fingerprint(&instance.id)?, before);
    Ok(())
}

#[test]
fn existing_fingerprint_tracks_parent_content_and_rejects_unsafe_chains() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("minecraft");
    common::candidate(&source, "parent", GameDataEvidence::Unknown)?;
    let candidate = common::candidate(&source, "child", GameDataEvidence::Unknown)?;
    let child = source.join("versions/child/child.json");
    let parent = source.join("versions/parent/parent.json");
    fs::write(&child, br#"{"id":"child","inheritsFrom":"parent"}"#)?;
    let catalog = Catalog::open(root.path().join("block"))?;
    let instance = catalog.register_existing(
        &candidate,
        InstanceConfiguration {
            name: "Inherited".into(),
            game_directory: source.join("versions/child"),
            ..Default::default()
        },
    )?;
    let snapshot = catalog.snapshot()?;
    let before = snapshot.launch_fingerprint(&instance.id)?;
    fs::write(&parent, br#"{"id":"parent","mainClass":"Changed"}"#)?;
    assert_ne!(snapshot.launch_fingerprint(&instance.id)?, before);
    fs::write(&parent, br#"{"id":"parent","inheritsFrom":"child"}"#)?;
    assert!(snapshot.launch_fingerprint(&instance.id).is_err());
    fs::write(&parent, br#"{"id":"parent","inheritsFrom":"../outside"}"#)?;
    assert!(snapshot.launch_fingerprint(&instance.id).is_err());
    fs::write(&parent, vec![b' '; 8 * 1024 * 1024 + 1])?;
    assert!(snapshot.launch_fingerprint(&instance.id).is_err());
    Ok(())
}
