use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use super::{JavaExecutable, PrepareAction, plan_runtime};
use crate::{
    azul::{ArchiveKind, ZuluRuntime},
    manifest::{decode, resources::AssetObjects},
    platform::{Architecture, OperatingSystem, Target},
    recipe::{Compatibility, LaunchRecipe, ParsedRecipe, RecipeSource, RuntimeSelection},
    resolver::{ManagedRuntime, ResolvedMetadata},
    session::OfflineSession,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifests")
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
        companion_libraries: Vec::new(),
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

#[test]
fn companion_libraries_are_added_to_the_verified_classpath() -> anyhow::Result<()> {
    // The companion mechanism is architecture-independent; this plans a modern
    // supported manifest on x64 and checks the dependency still reaches the
    // classpath and the verified resource list.
    let manifest = decode(&std::fs::read(fixtures().join("26.3.json"))?)?;
    let mut metadata = ResolvedMetadata {
        parsed: ParsedRecipe {
            manifest,
            manifest_sha1: "a".repeat(40),
        },
        target: Target {
            os: OperatingSystem::Linux,
            arch: Architecture::X86_64,
            os_version: "6.1".into(),
        },
        assets: AssetObjects {
            objects: BTreeMap::new(),
            virtual_layout: None,
            map_to_resources: None,
        },
        runtime: None,
        library_overrides: BTreeMap::new(),
        companion_libraries: Vec::new(),
        classifier_overrides: BTreeMap::new(),
        compatibility: None,
    };
    // A companion dependency the reviewed replacement needs but the older
    // manifest never listed, for example slf4j-api for text2speech 1.12.4.
    metadata.companion_libraries.push(crate::resolver::LibraryOverride {
        resource: block_download::Resource {
            sha1: "d".repeat(40),
            size: 69_908,
            url: "https://libraries.minecraft.net/org/slf4j/slf4j-api/1.8.0-beta4/slf4j-api-1.8.0-beta4.jar".into(),
        },
        filename: "slf4j-api-1.8.0-beta4.jar".into(),
    });
    let mut recipe = recipe();
    recipe.runtime = RuntimeSelection::Local {
        java: PathBuf::from("/usr/bin/java"),
    };
    let mut plan = super::compile(&recipe, &metadata)?;
    let companion = format!(
        "views/libraries/{}/d{}-slf4j-api-1.8.0-beta4.jar",
        "a".repeat(40),
        "d".repeat(39)
    );
    assert!(
        plan.command
            .classpath
            .iter()
            .any(|entry| entry == &companion),
        "companion classpath entry missing: {:?}",
        plan.command.classpath
    );
    assert!(
        plan.resources
            .iter()
            .any(|resource| resource.url.contains("slf4j-api-1.8.0-beta4.jar")),
        "companion resource missing"
    );
    plan.resources.clear();
    Ok(())
}
