use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, ensure};
use block_download::{Resource, Store};
use futures_util::future::BoxFuture;
use minecraft_manifest::{
    decode,
    model::{Download, VersionIndex, VersionManifest},
    resources::{AssetObjects, RuntimeCatalog, RuntimeManifest},
};
use sha1::{Digest as _, Sha1};
use tokio_util::sync::CancellationToken;

use crate::{
    platform::{Architecture, OperatingSystem, Target},
    recipe::{
        Compatibility, LaunchRecipe, ParsedRecipe, RecipeParser, RecipeSource, RuntimeSelection,
    },
};

const VERSION_INDEX: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const JAVA_INDEX: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Debug, Clone)]
pub struct ResolveContext {
    pub store: Store,
    pub cancel: CancellationToken,
}

#[derive(Debug)]
pub struct MojangParser;

#[derive(Debug)]
pub struct LocalManifestParser;

impl RecipeParser for MojangParser {
    fn resolve<'a>(
        &'a self,
        source: &'a RecipeSource,
        context: &'a ResolveContext,
    ) -> BoxFuture<'a, anyhow::Result<ParsedRecipe>> {
        Box::pin(async move {
            let RecipeSource::Mojang { version } = source else {
                anyhow::bail!("Mojang parser requires a Mojang source");
            };
            let bytes = context
                .store
                .metadata("version_manifest_v2.json", VERSION_INDEX)
                .await?;
            let index: VersionIndex = decode(&bytes)?;
            let entry = index
                .versions
                .iter()
                .find(|entry| entry.id == *version)
                .with_context(|| format!("version not found: {version}"))?;
            // The v2 index supplies the digest but not the document size.
            let bytes = context
                .store
                .metadata(&format!("version-{}.json", entry.sha1), &entry.url)
                .await?;
            ensure!(
                format!("{:x}", Sha1::digest(&bytes)) == entry.sha1,
                "version metadata SHA-1 mismatch"
            );
            Ok(ParsedRecipe {
                manifest: decode(&bytes)?,
                manifest_sha1: entry.sha1.clone(),
            })
        })
    }
}

impl RecipeParser for LocalManifestParser {
    fn resolve<'a>(
        &'a self,
        source: &'a RecipeSource,
        _context: &'a ResolveContext,
    ) -> BoxFuture<'a, anyhow::Result<ParsedRecipe>> {
        Box::pin(async move {
            let RecipeSource::LocalManifest { path } = source else {
                anyhow::bail!("local parser requires a local manifest");
            };
            let bytes = tokio::fs::read(path).await?;
            Ok(ParsedRecipe {
                manifest: decode(&bytes)?,
                manifest_sha1: format!("{:x}", Sha1::digest(&bytes)),
            })
        })
    }
}

#[derive(Debug, Clone)]
pub struct LibraryOverride {
    pub resource: Resource,
    pub filename: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedMetadata {
    pub parsed: ParsedRecipe,
    pub target: Target,
    pub assets: AssetObjects,
    pub runtime: Option<ManagedRuntime>,
    pub library_overrides: BTreeMap<String, LibraryOverride>,
    /// Replacement native classifiers keyed by the library that selects them.
    /// Classifier-era manifests declare natives through `natives`/`classifiers`
    /// instead of separate artifacts, so they need their own override map.
    pub classifier_overrides: BTreeMap<String, LibraryOverride>,
    /// Explicit description of an applied compatibility substitution, carried
    /// into the saved plan and the launch log so it is never silent.
    pub compatibility: Option<String>,
}

/// Managed runtime selected during resolution.
#[derive(Debug, Clone)]
pub enum ManagedRuntime {
    /// Official Mojang runtime files described by the launcher catalog.
    Mojang {
        digest: String,
        manifest: RuntimeManifest,
    },
    /// Official vendor runtime substituted where Mojang's catalog has no entry.
    Zulu {
        runtime: crate::azul::ZuluRuntime,
        component: String,
    },
}

impl ManagedRuntime {
    /// Explicit substitution text for the saved plan and the launch log.
    #[must_use]
    pub fn substitution_note(&self, target: &Target) -> Option<String> {
        match self {
            Self::Mojang { .. } => None,
            Self::Zulu { runtime, component } => Some(runtime.note(target, component)),
        }
    }
}

/// Resolve typed metadata before compiling a deterministic resource/command plan.
///
/// # Errors
/// Fails for invalid recipes, unavailable metadata/runtimes, or unsupported compatibility.
pub async fn resolve(
    recipe: &LaunchRecipe,
    context: &ResolveContext,
) -> anyhow::Result<ResolvedMetadata> {
    tokio::select! {
        biased;
        () = context.cancel.cancelled() => anyhow::bail!("resolution cancelled"),
        result = resolve_inner(recipe, context) => result,
    }
}

async fn resolve_inner(
    recipe: &LaunchRecipe,
    context: &ResolveContext,
) -> anyhow::Result<ResolvedMetadata> {
    recipe.validate()?;
    let parsed = match &recipe.source {
        RecipeSource::Mojang { .. } => MojangParser.resolve(&recipe.source, context).await?,
        RecipeSource::LocalManifest { .. } => {
            LocalManifestParser.resolve(&recipe.source, context).await?
        }
    };
    let target = recipe.target.clone().map_or_else(Target::host, Ok)?;
    crate::support::manifest_support(&parsed.manifest, &target, &recipe.features)?.require()?;
    let asset = &parsed.manifest.asset_index;
    let resource = Resource {
        sha1: asset.sha1.clone(),
        size: asset.size,
        url: asset.url.clone(),
    };
    let bytes = context.store.fetch(&resource, &context.cancel).await?;
    let assets = decode(&tokio::fs::read(bytes).await?)?;
    let runtime = if matches!(recipe.runtime, RuntimeSelection::Managed) {
        Some(resolve_managed_runtime(&parsed.manifest, &target, context).await?)
    } else {
        None
    };
    let (library_overrides, classifier_overrides, compatibility) =
        compatibility_overrides(recipe, &parsed, &target, context).await?;
    Ok(ResolvedMetadata {
        parsed,
        target,
        assets,
        runtime,
        library_overrides,
        classifier_overrides,
        compatibility,
    })
}

async fn resolve_managed_runtime(
    manifest: &VersionManifest,
    target: &Target,
    context: &ResolveContext,
) -> anyhow::Result<ManagedRuntime> {
    let component = manifest
        .java_version
        .as_ref()
        .map_or("jre-legacy", |java| java.component.as_str());
    let bytes = context
        .store
        .metadata("java-runtimes.json", JAVA_INDEX)
        .await?;
    let catalog: RuntimeCatalog = decode(&bytes)?;
    let release = catalog
        .get(target.runtime_platform())
        .and_then(|components| components.get(component))
        .and_then(|releases| {
            releases
                .iter()
                .find(|release| release.availability.progress > 0)
        });
    if let Some(release) = release {
        let path = context
            .store
            .fetch(&download_resource(&release.manifest), &context.cancel)
            .await?;
        return Ok(ManagedRuntime::Mojang {
            digest: release.manifest.sha1.clone(),
            manifest: decode(&tokio::fs::read(path).await?)?,
        });
    }
    if let Some(java) = &manifest.java_version
        && crate::azul::supports(target)
    {
        let runtime =
            crate::azul::resolve(&context.store, target, java.major_version, &context.cancel)
                .await?;
        crate::azul::download(&context.store, &runtime, &context.cancel).await?;
        return Ok(ManagedRuntime::Zulu {
            runtime,
            component: component.to_owned(),
        });
    }
    anyhow::bail!(
        "no official {component} runtime for {}; select an explicit local Java matching this target",
        target.runtime_platform()
    )
}

type AppliedOverrides = (
    BTreeMap<String, LibraryOverride>,
    BTreeMap<String, LibraryOverride>,
    Option<String>,
);

async fn compatibility_overrides(
    recipe: &LaunchRecipe,
    parsed: &ParsedRecipe,
    target: &Target,
    context: &ResolveContext,
) -> anyhow::Result<AppliedOverrides> {
    match recipe.compatibility {
        Compatibility::Official => {
            if target.os == OperatingSystem::Linux && target.arch == Architecture::Aarch64 {
                linux_arm64_overrides(recipe, parsed, target, context).await
            } else {
                Ok((BTreeMap::new(), BTreeMap::new(), None))
            }
        }
        Compatibility::LinuxArm64Lwjgl3 => {
            linux_arm64_overrides(recipe, parsed, target, context).await
        }
        Compatibility::MacosJna513 => Ok((
            macos_jna_overrides(recipe, parsed, target, context).await?,
            BTreeMap::new(),
            Some(format!(
                "macOS ARM64: the affected JNA pair is replaced with the official JNA {} release from Maven Central",
                crate::support::REVIEWED_ARM64_JNA
            )),
        )),
    }
}

/// Replace the LWJGL and companion artifacts a Linux ARM64 host cannot load.
///
/// The decision is keyed on the manifest's own LWJGL modules, not on Minecraft
/// versions: 3.3.1 and newer keep their release and swap each selected Linux
/// classifier for the published ARM64 one, while classifier-era 3.2.x manifests
/// move to the reviewed 3.3.1 composition proven on 1.16.5. JNA and
/// text2speech below the reviewed releases are replaced as companions because
/// their bundled natives predate ARM64. Every artifact URL is checked through
/// the official checksum file before it can be planned.
async fn linux_arm64_overrides(
    recipe: &LaunchRecipe,
    parsed: &ParsedRecipe,
    target: &Target,
    context: &ResolveContext,
) -> anyhow::Result<AppliedOverrides> {
    ensure!(
        target.os == OperatingSystem::Linux && target.arch == Architecture::Aarch64,
        "linux_arm64_lwjgl3 requires a Linux ARM64 target"
    );
    let mut artifacts = Vec::new();
    let mut classifiers = Vec::new();
    let mut versions = BTreeSet::new();
    let mut arm64_selected = false;
    let mut jna = Vec::new();
    let mut text2speech = Vec::new();
    for library in &parsed.manifest.libraries {
        if !crate::rules::allowed(library.rules.as_deref(), target, &recipe.features)? {
            continue;
        }
        if library.name.starts_with("org.lwjgl.lwjgl:") {
            anyhow::bail!(
                "this version uses LWJGL 2, whose official natives are x86/x64 only; Linux ARM64 support covers LWJGL 3-era versions"
            );
        }
        if let Some(rest) = library.name.strip_prefix("org.lwjgl:") {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() < 2 {
                continue;
            }
            versions.insert(parts[1].to_owned());
            if parts
                .get(2)
                .is_some_and(|classifier| *classifier == "natives-linux-arm64")
            {
                arm64_selected = true;
            }
            artifacts.push((
                library.name.clone(),
                parts[0].to_owned(),
                parts[1].to_owned(),
                parts.get(2).map(|classifier| (*classifier).to_owned()),
            ));
            if let Some(classifier) = library
                .natives
                .as_ref()
                .and_then(|natives| natives.get("linux"))
            {
                classifiers.push((
                    library.name.clone(),
                    parts[0].to_owned(),
                    parts[1].to_owned(),
                    classifier.clone(),
                ));
            }
        } else if let Some(rest) = library.name.strip_prefix("net.java.dev.jna:") {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() == 2 {
                jna.push((
                    library.name.clone(),
                    parts[0].to_owned(),
                    parts[1].to_owned(),
                ));
            }
        } else if let Some(rest) = library.name.strip_prefix("com.mojang:text2speech:") {
            let version = rest.split(':').next().unwrap_or_default();
            text2speech.push((library.name.clone(), version.to_owned()));
        }
    }
    let plan = crate::support::linux_arm64_lwjgl_plan(&versions)
        .map_err(|reason| anyhow::anyhow!(reason))?;
    let reviewed = match plan {
        crate::support::LinuxArm64LwjglPlan::SameVersion => None,
        crate::support::LinuxArm64LwjglPlan::ReplaceWith(version) => Some(version),
    };
    let mut artifact_overrides = BTreeMap::new();
    let mut classifier_overrides = BTreeMap::new();
    let mut reviewed_module = None;
    for (name, module, manifest_version, classifier) in artifacts {
        let version = reviewed.unwrap_or(&manifest_version);
        let filename = match classifier.as_deref() {
            Some("natives-linux-arm64") => continue,
            Some("natives-linux") => {
                reviewed_module.get_or_insert(version.to_owned());
                format!("{module}-{version}-natives-linux-arm64.jar")
            }
            Some(_) => continue,
            None if reviewed.is_some() => {
                reviewed_module.get_or_insert(version.to_owned());
                format!("{module}-{version}.jar")
            }
            None => continue,
        };
        let url =
            format!("https://repo.maven.apache.org/maven2/org/lwjgl/{module}/{version}/{filename}");
        artifact_overrides.insert(
            name,
            LibraryOverride {
                resource: context.store.describe_maven(&url).await?,
                filename,
            },
        );
    }
    for (name, module, manifest_version, classifier) in classifiers {
        ensure!(
            classifier == "natives-linux",
            "unexpected Linux native classifier {classifier} for {name}"
        );
        let version = reviewed.unwrap_or(&manifest_version);
        let filename = format!("{module}-{version}-natives-linux-arm64.jar");
        let url =
            format!("https://repo.maven.apache.org/maven2/org/lwjgl/{module}/{version}/{filename}");
        reviewed_module.get_or_insert(version.to_owned());
        classifier_overrides.insert(
            name,
            LibraryOverride {
                resource: context.store.describe_maven(&url).await?,
                filename,
            },
        );
    }
    // A manifest may already select official ARM64 natives (for example a
    // synthetic fixture or a future official update); nothing needs replacing.
    ensure!(
        !artifact_overrides.is_empty() || !classifier_overrides.is_empty() || arm64_selected,
        "this version selects no LWJGL 3 Linux artifacts to substitute"
    );
    let mut companions = Vec::new();
    for (name, artifact, manifest_version) in jna {
        if version_at_least(&manifest_version, crate::support::REVIEWED_ARM64_JNA) {
            continue;
        }
        let target_artifact = if artifact == "platform" {
            "jna-platform"
        } else {
            artifact.as_str()
        };
        let filename = format!(
            "{target_artifact}-{}.jar",
            crate::support::REVIEWED_ARM64_JNA
        );
        let url = format!(
            "https://repo.maven.apache.org/maven2/net/java/dev/jna/{target_artifact}/{}/{filename}",
            crate::support::REVIEWED_ARM64_JNA
        );
        artifact_overrides.insert(
            name,
            LibraryOverride {
                resource: context.store.describe_maven(&url).await?,
                filename,
            },
        );
        companions.push(format!("JNA {}", crate::support::REVIEWED_ARM64_JNA));
    }
    for (name, manifest_version) in text2speech {
        if version_at_least(&manifest_version, crate::support::REVIEWED_TEXT2SPEECH) {
            continue;
        }
        let filename = format!("text2speech-{}.jar", crate::support::REVIEWED_TEXT2SPEECH);
        let url = format!(
            "https://libraries.minecraft.net/com/mojang/text2speech/{}/{filename}",
            crate::support::REVIEWED_TEXT2SPEECH
        );
        artifact_overrides.insert(
            name,
            LibraryOverride {
                resource: context.store.describe_maven(&url).await?,
                filename,
            },
        );
        companions.push(format!(
            "text2speech {}",
            crate::support::REVIEWED_TEXT2SPEECH
        ));
    }
    let lwjgl = reviewed_module.unwrap_or_default();
    let note = if reviewed.is_some() {
        format!(
            "Linux ARM64: the manifest's LWJGL {} is replaced with the reviewed {lwjgl} composition and its official natives-linux-arm64 classifiers from Maven Central",
            versions.iter().cloned().collect::<Vec<_>>().join("/")
        )
    } else if artifact_overrides.is_empty() && classifier_overrides.is_empty() {
        format!("Linux ARM64: LWJGL {lwjgl} already selects official natives-linux-arm64 artifacts")
    } else {
        format!(
            "Linux ARM64: LWJGL {lwjgl} keeps its version and uses the official natives-linux-arm64 classifiers from Maven Central"
        )
    };
    let note = if companions.is_empty() {
        note
    } else {
        format!(
            "{note}; companions replaced with official releases: {}",
            companions.join(", ")
        )
    };
    Ok((
        artifact_overrides,
        classifier_overrides,
        Some(format!(
            "{note}; narration has no native ARM64 flite wrapper, so it stays unavailable"
        )),
    ))
}

fn version_at_least(version: &str, floor: &str) -> bool {
    fn parts(version: &str) -> [u32; 3] {
        let mut parts = version.split('.').map(|part| {
            part.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|digits| digits.parse().ok())
                .unwrap_or(0)
        });
        [
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        ]
    }
    parts(version) >= parts(floor)
}

async fn macos_jna_overrides(
    recipe: &LaunchRecipe,
    parsed: &ParsedRecipe,
    target: &Target,
    context: &ResolveContext,
) -> anyhow::Result<BTreeMap<String, LibraryOverride>> {
    ensure!(
        target.os == OperatingSystem::Macos,
        "macos_jna_5_13 requires a macOS target"
    );
    let mut originals = BTreeMap::new();
    for library in &parsed.manifest.libraries {
        if !crate::rules::allowed(library.rules.as_deref(), target, &recipe.features)? {
            continue;
        }
        let mut parts = library.name.split(':');
        if let (
            Some("net.java.dev.jna"),
            Some(artifact @ ("jna" | "jna-platform")),
            Some(version),
            None,
        ) = (parts.next(), parts.next(), parts.next(), parts.next())
        {
            ensure!(
                ["5.8.0", "5.9.0", "5.10.0", "5.11.0", "5.12.0", "5.12.1"].contains(&version),
                "macos_jna_5_13 only supports the affected JNA 5.8.0 through 5.12.1 releases"
            );
            ensure!(
                originals
                    .insert(artifact, (&library.name, version))
                    .is_none(),
                "multiple selected versions of {artifact}"
            );
        }
    }
    ensure!(
        originals.len() == 2
            && originals.get("jna").map(|(_, version)| version)
                == originals.get("jna-platform").map(|(_, version)| version),
        "macos_jna_5_13 requires a matching affected JNA core/platform pair"
    );
    let mut replacements = BTreeMap::new();
    for (artifact, (original, _)) in originals {
        let filename = format!("{artifact}-5.13.0.jar");
        let url = format!(
            "https://repo.maven.apache.org/maven2/net/java/dev/jna/{artifact}/5.13.0/{filename}"
        );
        replacements.insert(
            original.clone(),
            LibraryOverride {
                resource: context.store.describe_maven(&url).await?,
                filename,
            },
        );
    }
    Ok(replacements)
}

#[must_use]
pub fn download_resource(download: &Download) -> Resource {
    Resource {
        sha1: download.sha1.clone(),
        size: download.size,
        url: download.url.clone(),
    }
}
