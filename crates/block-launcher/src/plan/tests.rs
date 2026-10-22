use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use minecraft_manifest::{decode, resources::AssetObjects};

use super::{JavaExecutable, PrepareAction, plan_runtime};
use crate::{
    azul::{ArchiveKind, ZuluRuntime},
    platform::{Architecture, OperatingSystem, Target},
    recipe::{Compatibility, LaunchRecipe, ParsedRecipe, RecipeSource, RuntimeSelection},
    resolver::{ManagedRuntime, ResolvedMetadata},
    session::OfflineSession,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../minecraft-manifest/tests/fixtures")
}

fn recipe() -> LaunchRecipe {
    LaunchRecipe {
        schema_version: 1,
        instance: "instance".into(),
        source: RecipeSource::Mojang {
            version: "1.12.2".into(),
        },
        session: OfflineSession::Offline {
            username: "Player".into(),
        },
        runtime: RuntimeSelection::Managed,
        target: None,
        compatibility: Compatibility::Official,
        memory_mib: 1024,
        jvm_args: Vec::new(),
        features: BTreeMap::new(),
    }
}

#[test]
fn zulu_runtime_substitution_is_explicit_and_extracts_into_the_runtime_store() -> anyhow::Result<()>
{
    let manifest = decode(&std::fs::read(fixtures().join("1.12.2.json"))?)?;
    let sha256 = "c".repeat(64);
    let metadata = ResolvedMetadata {
        parsed: ParsedRecipe {
            manifest,
            manifest_sha1: "a".repeat(40),
        },
        target: Target {
            os: OperatingSystem::Linux,
            arch: Architecture::Aarch64,
            os_version: "6.1".into(),
        },
        assets: AssetObjects {
            objects: BTreeMap::new(),
            virtual_layout: None,
            map_to_resources: None,
        },
        runtime: Some(ManagedRuntime::Zulu {
            runtime: ZuluRuntime {
                java_version: "8u504".into(),
                package: "zulu8.96.0.205-ca-jre8.0.504-linux_aarch64.tar.gz".into(),
                download_url: "https://cdn.azul.com/zulu/bin/zulu8.tar.gz".into(),
                sha256: sha256.clone(),
                size: 123,
                archive: ArchiveKind::TarGz,
            },
            component: "jre-legacy".into(),
        }),
        library_overrides: BTreeMap::new(),
        classifier_overrides: BTreeMap::new(),
        compatibility: None,
    };
    let mut resources = Vec::new();
    let mut actions = Vec::new();
    let java = plan_runtime(&recipe(), &metadata, &mut resources, &mut actions)?;
    let expected_root = format!("runtimes/linux-arm64/{sha256}");
    assert!(
        resources.is_empty(),
        "the verified archive is already in the store"
    );
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        PrepareAction::ExtractArchive {
            sha256: action_sha256,
            directory,
            archive,
        } => {
            assert_eq!(action_sha256, &sha256);
            assert_eq!(directory, &expected_root);
            assert_eq!(*archive, ArchiveKind::TarGz);
        }
        other => anyhow::bail!("unexpected preparation action: {other:?}"),
    }
    match java {
        JavaExecutable::Managed { path } => {
            assert_eq!(path, format!("{expected_root}/bin/java"));
        }
        other => anyhow::bail!("expected a managed Java executable, got {other:?}"),
    }
    let note = metadata
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.substitution_note(&metadata.target))
        .ok_or_else(|| anyhow::anyhow!("missing runtime substitution note"))?;
    assert!(note.contains("Azul Zulu Java 8u504"), "{note}");
    assert!(note.contains(&sha256), "{note}");
    assert!(note.contains("jre-legacy"), "{note}");
    assert!(note.contains("api.azul.com"), "{note}");
    Ok(())
}
