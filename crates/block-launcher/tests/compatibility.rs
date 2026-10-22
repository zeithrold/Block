use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context as _;
use block_download::{Resource, Store};
use block_launcher::{
    CancellationToken,
    command::JavaCommand,
    plan::compile,
    platform::{Architecture, OperatingSystem, Target},
    recipe::{Compatibility, LaunchRecipe, RecipeSource, RuntimeSelection},
    resolver::{ResolveContext, resolve},
    session::OfflineSession,
};
use minecraft_manifest::{
    decode,
    model::{VersionIndex, VersionManifest},
};
use sha1::{Digest as _, Sha1};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../minecraft-manifest/tests/fixtures")
}

fn manifest(version: &str) -> anyhow::Result<(PathBuf, VersionManifest)> {
    let directory = fixtures().join("official");
    let index: VersionIndex = decode(&fs::read(directory.join("version_manifest_v2.json"))?)?;
    let entry = index
        .versions
        .iter()
        .find(|entry| entry.id == version)
        .context("fixture version is missing")?;
    let path = directory
        .join("versions")
        .join(format!("{}.json", entry.sha1));
    Ok((path.clone(), decode(&fs::read(path)?)?))
}

fn recipe(path: PathBuf) -> anyhow::Result<LaunchRecipe> {
    Ok(LaunchRecipe {
        schema_version: 1,
        instance: "compatibility-test".into(),
        source: RecipeSource::LocalManifest { path },
        session: OfflineSession::Offline {
            username: "Player".into(),
        },
        runtime: RuntimeSelection::Local {
            java: std::env::current_exe()?,
        },
        target: Some(Target {
            os: OperatingSystem::Macos,
            arch: Architecture::Aarch64,
            os_version: "15.0".into(),
        }),
        compatibility: Compatibility::MacosJna513,
        memory_mib: 2048,
        jvm_args: Vec::new(),
        features: BTreeMap::new(),
    })
}

fn offline_context(root: &Path, manifest: &VersionManifest) -> anyhow::Result<ResolveContext> {
    let store = Store::new(root.into(), true, Arc::new(|_| {}))?;
    let asset = &manifest.asset_index;
    let suffix = format!("-{}.json", asset.sha1);
    let fixture = fs::read_dir(fixtures().join("resources"))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|entry| entry.file_name().to_string_lossy().ends_with(&suffix))
        .context("asset metadata fixture is missing")?
        .path();
    let destination = store.object_path(&asset.sha1)?;
    fs::create_dir_all(destination.parent().context("asset object has no parent")?)?;
    fs::copy(fixture, destination)?;
    let metadata = root.join("metadata");
    fs::create_dir_all(&metadata)?;
    // Published Maven descriptors; the regression remains entirely offline.
    for (artifact, sha1, size) in [
        ("jna", "1200e7ebeedbe0d10062093f32925a912020e747", 1_879_325),
        (
            "jna-platform",
            "88e9a306715e9379f3122415ef4ae759a352640d",
            1_363_209,
        ),
    ] {
        let url = format!(
            "https://repo.maven.apache.org/maven2/net/java/dev/jna/{artifact}/5.13.0/{artifact}-5.13.0.jar"
        );
        let key = format!("{:x}", Sha1::digest(url.as_bytes()));
        fs::write(
            metadata.join(format!("maven-{key}.json")),
            serde_json::to_vec(&Resource {
                sha1: sha1.into(),
                size,
                url,
            })?,
        )?;
    }
    Ok(ResolveContext {
        store,
        cancel: CancellationToken::new(),
    })
}

#[tokio::test]
async fn explicit_macos_profile_replaces_both_jna_artifacts_without_changing_manifest()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (path, original) = manifest("1.20.1")?;
    let context = offline_context(directory.path(), &original)?;
    let mut recipe = recipe(path)?;
    recipe.compatibility = Compatibility::Official;
    let official = resolve(&recipe, &context).await?;
    let official_plan = compile(&recipe, &official)?;
    assert!(official.library_overrides.is_empty());
    assert!(
        official_plan
            .command
            .classpath
            .iter()
            .any(|path| path.ends_with("jna-5.12.1.jar"))
    );
    recipe.compatibility = Compatibility::MacosJna513;
    let patched = resolve(&recipe, &context).await?;
    assert_eq!(patched.library_overrides.len(), 2);
    assert_eq!(patched.parsed.manifest_sha1, official.parsed.manifest_sha1);
    assert_eq!(
        serde_json::to_value(&patched.parsed.manifest)?,
        serde_json::to_value(&original)?
    );
    let patched_plan = compile(&recipe, &patched)?;
    assert_eq!(patched_plan.resources.len(), official_plan.resources.len());
    for artifact in ["jna", "jna-platform"] {
        let key = format!("net.java.dev.jna:{artifact}:5.12.1");
        let replacement = patched
            .library_overrides
            .get(&key)
            .context("JNA replacement is missing")?;
        assert!(patched_plan.resources.contains(&replacement.resource));
        assert!(
            patched_plan
                .command
                .classpath
                .iter()
                .any(|path| path.ends_with(&format!("{artifact}-5.13.0.jar")))
        );
    }
    let serialized = serde_json::to_string(&patched_plan)?;
    assert!(!serialized.contains("5.12.1"));
    let saved = serde_json::from_str(&serialized)?;
    let command = JavaCommand::bind(&saved, directory.path(), "run", &recipe.session)?;
    let arguments = command.preview().arguments;
    assert!(
        arguments
            .iter()
            .any(|arg| arg.contains("jna-5.13.0.jar") && arg.contains("jna-platform-5.13.0.jar"))
    );
    assert!(arguments.iter().all(|arg| !arg.contains("5.12.1")));
    Ok(())
}

#[tokio::test]
async fn macos_jna_profile_rejects_other_platforms_and_never_downgrades_new_jna()
-> anyhow::Result<()> {
    for (version, os, expected) in [
        ("1.20.1", OperatingSystem::Linux, "requires a macOS target"),
        (
            "1.20.6",
            OperatingSystem::Macos,
            "only supports the affected JNA",
        ),
    ] {
        let directory = tempfile::tempdir()?;
        let (path, manifest) = manifest(version)?;
        let context = offline_context(directory.path(), &manifest)?;
        let mut recipe = recipe(path)?;
        recipe.target.as_mut().context("missing target")?.os = os;
        let error = resolve(&recipe, &context)
            .await
            .err()
            .context("inapplicable compatibility profile was accepted")?;
        assert!(error.to_string().contains(expected), "{error:#}");
    }
    Ok(())
}

/// Seed published Maven descriptors so substitution tests stay offline.
fn seed_maven(root: &Path, artifacts: &[(&str, &str)]) -> anyhow::Result<()> {
    let metadata = root.join("metadata");
    fs::create_dir_all(&metadata)?;
    for (url, sha1) in artifacts {
        let key = format!("{:x}", Sha1::digest(url.as_bytes()));
        fs::write(
            metadata.join(format!("maven-{key}.json")),
            serde_json::to_vec(&Resource {
                sha1: (*sha1).into(),
                size: 1,
                url: (*url).into(),
            })?,
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn linux_arm64_substitutes_published_arm64_natives_and_records_the_composition()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (path, original) = manifest("1.20.6")?;
    let context = offline_context(directory.path(), &original)?;
    let published: [(&str, &str); 8] = [
        ("lwjgl", "f35d8b6ffe1ac1e3a5eb1d4e33de80f044ad5fd8"),
        ("lwjgl-freetype", "498965aac06c4a0d42df1fbef6bacd05bde7f974"),
        ("lwjgl-glfw", "492a0f11f85b85899a6568f07511160c1b87cd38"),
        ("lwjgl-jemalloc", "eff8b86798191192fe2cba2dc2776109f30c239d"),
        ("lwjgl-openal", "ad8f302118a65bb8d615f8a2a680db58fb8f835e"),
        ("lwjgl-opengl", "2096f6b94b2d68745d858fbfe53aacf5f0c8074c"),
        ("lwjgl-stb", "ddc177afc2be1ee8d93684b11363b80589a13fe1"),
        ("lwjgl-tinyfd", "2823a8c955c758d0954d282888075019ef99cec7"),
    ];
    let urls: Vec<(String, &str)> = published
        .iter()
        .map(|(module, sha1)| {
            (
                format!(
                    "https://repo.maven.apache.org/maven2/org/lwjgl/{module}/3.3.3/{module}-3.3.3-natives-linux-arm64.jar"
                ),
                *sha1,
            )
        })
        .collect();
    let artifacts: Vec<(&str, &str)> = urls
        .iter()
        .map(|(url, sha1)| (url.as_str(), *sha1))
        .collect();
    seed_maven(directory.path(), &artifacts)?;

    let mut recipe = recipe(path)?;
    recipe.compatibility = Compatibility::Official;
    recipe.target = Some(Target {
        os: OperatingSystem::Linux,
        arch: Architecture::Aarch64,
        os_version: "6.1".into(),
    });
    let resolved = resolve(&recipe, &context).await?;
    assert_eq!(resolved.library_overrides.len(), 8);
    let note = resolved
        .compatibility
        .clone()
        .ok_or_else(|| anyhow::anyhow!("substitution was not recorded"))?;
    assert!(note.contains("natives-linux-arm64"), "{note}");
    let plan = compile(&recipe, &resolved)?;
    assert_eq!(plan.compatibility.as_deref(), Some(note.as_str()));
    for (module, _) in published {
        assert!(
            plan.command
                .classpath
                .iter()
                .any(|path| path.ends_with(&format!("{module}-3.3.3-natives-linux-arm64.jar"))),
            "{module} ARM64 native missing from the plan"
        );
    }
    assert!(
        !plan
            .command
            .classpath
            .iter()
            .any(|path| path.ends_with("-natives-linux.jar")),
        "the x64 Linux natives must not survive a reviewed ARM64 plan"
    );

    // The same manifest stays official on x64 Linux: no substitution, no note.
    recipe.target = Some(Target {
        os: OperatingSystem::Linux,
        arch: Architecture::X86_64,
        os_version: "6.1".into(),
    });
    let official = resolve(&recipe, &context).await?;
    assert!(official.library_overrides.is_empty());
    assert!(official.compatibility.is_none());
    Ok(())
}

#[tokio::test]
async fn classifier_era_manifest_is_replaced_with_the_reviewed_linux_arm64_composition()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (path, original) = manifest("1.16.5")?;
    let context = offline_context(directory.path(), &original)?;
    let modules = [
        "lwjgl",
        "lwjgl-glfw",
        "lwjgl-jemalloc",
        "lwjgl-openal",
        "lwjgl-opengl",
        "lwjgl-stb",
        "lwjgl-tinyfd",
    ];
    let mut urls = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        urls.push(format!(
            "https://repo.maven.apache.org/maven2/org/lwjgl/{module}/3.3.1/{module}-3.3.1.jar"
        ));
        urls.push(format!(
            "https://repo.maven.apache.org/maven2/org/lwjgl/{module}/3.3.1/{module}-3.3.1-natives-linux-arm64.jar"
        ));
        let _ = index;
    }
    for artifact in ["jna", "jna-platform"] {
        urls.push(format!(
            "https://repo.maven.apache.org/maven2/net/java/dev/jna/{artifact}/5.13.0/{artifact}-5.13.0.jar"
        ));
    }
    urls.push(
        "https://libraries.minecraft.net/com/mojang/text2speech/1.12.4/text2speech-1.12.4.jar"
            .into(),
    );
    // text2speech 1.12.4 needs slf4j-api, which the 1.16.5 manifest never lists.
    urls.push(
        "https://libraries.minecraft.net/org/slf4j/slf4j-api/1.8.0-beta4/slf4j-api-1.8.0-beta4.jar"
            .into(),
    );
    // Planning only needs published descriptors; digests are synthetic offline fixtures.
    let synthetic: Vec<String> = (1..=urls.len())
        .map(|index| format!("{index:040x}"))
        .collect();
    let artifacts: Vec<(&str, &str)> = urls
        .iter()
        .zip(&synthetic)
        .map(|(url, sha1)| (url.as_str(), sha1.as_str()))
        .collect();
    seed_maven(directory.path(), &artifacts)?;

    let mut recipe = recipe(path)?;
    recipe.compatibility = Compatibility::Official;
    recipe.target = Some(Target {
        os: OperatingSystem::Linux,
        arch: Architecture::Aarch64,
        os_version: "6.1".into(),
    });
    let resolved = resolve(&recipe, &context).await?;
    // Seven core replacements, the JNA pair and text2speech as companions.
    assert_eq!(resolved.library_overrides.len(), 10);
    // Every selected LWJGL module also receives an ARM64 native classifier.
    assert_eq!(resolved.classifier_overrides.len(), 7);
    let note = resolved
        .compatibility
        .clone()
        .ok_or_else(|| anyhow::anyhow!("substitution was not recorded"))?;
    assert!(note.contains("reviewed 3.3.1 composition"), "{note}");
    assert!(note.contains("JNA 5.13.0"), "{note}");
    assert!(note.contains("text2speech 1.12.4"), "{note}");
    assert!(note.contains("slf4j-api 1.8.0-beta4"), "{note}");
    assert!(note.contains("narration"), "{note}");
    let plan = compile(&recipe, &resolved)?;
    assert_eq!(plan.compatibility.as_deref(), Some(note.as_str()));
    for module in modules {
        assert!(
            plan.command
                .classpath
                .iter()
                .any(|path| path.ends_with(&format!("{module}-3.3.1.jar"))),
            "{module} 3.3.1 core jar missing"
        );
        assert!(
            plan.resources.iter().any(|resource| resource
                .url
                .ends_with(&format!("{module}-3.3.1-natives-linux-arm64.jar"))),
            "{module} ARM64 native missing"
        );
        assert!(
            !plan.resources.iter().any(|resource| resource
                .url
                .ends_with(&format!("{module}-3.2.2-natives-linux.jar"))),
            "{module} x64 native survived the reviewed composition"
        );
    }
    assert!(
        plan.command
            .classpath
            .iter()
            .any(|path| path.ends_with("jna-5.13.0.jar"))
    );
    assert!(
        plan.command
            .classpath
            .iter()
            .any(|path| path.ends_with("text2speech-1.12.4.jar"))
    );
    assert!(
        plan.command
            .classpath
            .iter()
            .any(|path| path.ends_with("slf4j-api-1.8.0-beta4.jar")),
        "text2speech dependency must join the classpath"
    );
    assert!(
        plan.resources
            .iter()
            .any(|resource| resource.url.ends_with("slf4j-api-1.8.0-beta4.jar")),
        "text2speech dependency must be verified as a resource"
    );
    Ok(())
}

#[tokio::test]
async fn cancellation_interrupts_a_stalled_metadata_response() -> anyhow::Result<()> {
    use std::time::Duration;

    let directory = tempfile::tempdir()?;
    let (_, mut document) = manifest("1.20.1")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    document.asset_index.url = format!("http://{}/stalled-index", listener.local_addr()?);
    let path = directory.path().join("version.json");
    fs::write(&path, serde_json::to_vec(&document)?)?;
    let mut recipe = recipe(path)?;
    recipe.compatibility = Compatibility::Official;
    let context = ResolveContext {
        store: Store::new(directory.path().join("data"), false, Arc::new(|_| {}))?,
        cancel: CancellationToken::new(),
    };
    let cancel = context.cancel.clone();
    let (accepted, connection) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await?;
        let _ = accepted.send(());
        std::future::pending::<()>().await;
        drop(socket);
        anyhow::Ok(())
    });
    let resolution = tokio::spawn(async move { resolve(&recipe, &context).await });
    tokio::time::timeout(Duration::from_secs(5), connection).await??;
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), resolution).await??;
    server.abort();
    let error = result
        .err()
        .context("stalled resolution ignored cancellation")?;
    assert!(error.to_string().contains("resolution cancelled"));
    Ok(())
}
