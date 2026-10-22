use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, ensure};
use block_download::{Resource, Store};
use futures_util::future::BoxFuture;
use minecraft_manifest::{
    decode,
    model::{Download, VersionIndex},
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
    pub runtime: Option<(String, RuntimeManifest)>,
    pub library_overrides: BTreeMap<String, LibraryOverride>,
    /// Explicit description of an applied compatibility substitution, carried
    /// into the saved plan and the launch log so it is never silent.
    pub compatibility: Option<String>,
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
        let component = parsed
            .manifest
            .java_version
            .as_ref()
            .map_or("jre-legacy", |java| java.component.as_str());
        let bytes = context
            .store
            .metadata("java-runtimes.json", JAVA_INDEX)
            .await?;
        let catalog: RuntimeCatalog = decode(&bytes)?;
        let release = catalog.get(target.runtime_platform()).and_then(|components| components.get(component))
            .and_then(|releases| releases.iter().find(|release| release.availability.progress > 0))
            .with_context(|| format!("no official {component} runtime for {}; select an explicit local Java matching this target", target.runtime_platform()))?;
        let path = context
            .store
            .fetch(&download_resource(&release.manifest), &context.cancel)
            .await?;
        Some((
            release.manifest.sha1.clone(),
            decode(&tokio::fs::read(path).await?)?,
        ))
    } else {
        None
    };
    let (library_overrides, compatibility) =
        compatibility_overrides(recipe, &parsed, &target, context).await?;
    Ok(ResolvedMetadata {
        parsed,
        target,
        assets,
        runtime,
        library_overrides,
        compatibility,
    })
}

type AppliedOverrides = (BTreeMap<String, LibraryOverride>, Option<String>);

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
                Ok((BTreeMap::new(), None))
            }
        }
        Compatibility::LinuxArm64Lwjgl3 => {
            linux_arm64_overrides(recipe, parsed, target, context).await
        }
        Compatibility::MacosJna513 => Ok((
            macos_jna_overrides(recipe, parsed, target, context).await?,
            Some(format!(
                "macOS ARM64: the affected JNA pair is replaced with the official JNA {} release from Maven Central",
                crate::support::REVIEWED_ARM64_JNA
            )),
        )),
    }
}

/// Replace the LWJGL and JNA artifacts a Linux ARM64 host cannot load.
///
/// The decision is keyed on the manifest's own LWJGL version, not on Minecraft
/// versions: manifests whose modules are all 3.2.1/3.2.2 (the composition proven
/// on 1.16.5) move to the reviewed 3.3.1 release, while 3.3.1 and newer keep
/// their version and only swap the Linux classifiers for the published ARM64
/// ones. JNA below the reviewed release is replaced as a companion because the
/// bundled natives predate ARM64. Every artifact URL is checked through the
/// official Maven Central checksum before it can be planned.
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
    let mut modules = Vec::new();
    let mut versions = BTreeSet::new();
    let mut jna = Vec::new();
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
            modules.push((library.name.clone(), parts));
        } else if let Some(rest) = library.name.strip_prefix("net.java.dev.jna:") {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() == 2 {
                jna.push((
                    library.name.clone(),
                    parts[0].to_owned(),
                    parts[1].to_owned(),
                ));
            }
        }
    }
    let plan = crate::support::linux_arm64_lwjgl_plan(&versions)
        .map_err(|reason| anyhow::anyhow!(reason))?;
    let reviewed = match plan {
        crate::support::LinuxArm64LwjglPlan::SameVersion => None,
        crate::support::LinuxArm64LwjglPlan::ReplaceWith(version) => Some(version),
    };
    let mut replacements = BTreeMap::new();
    let mut reviewed_modules = None;
    for (name, parts) in modules {
        let module = parts[0];
        let manifest_version = parts[1];
        let classifier = parts.get(2).copied();
        let version = reviewed.unwrap_or(manifest_version);
        if let Some(classifier) = classifier {
            if classifier == "natives-linux-arm64" {
                continue;
            }
            if classifier != "natives-linux" {
                continue;
            }
        } else if reviewed.is_none() {
            // Core artifacts stay at their own version unless the composition
            // replaces the whole reviewed release.
            continue;
        }
        let mut filename = format!("{module}-{version}");
        if classifier.is_some() {
            filename.push_str("-natives-linux-arm64");
        }
        filename.push_str(".jar");
        let url =
            format!("https://repo.maven.apache.org/maven2/org/lwjgl/{module}/{version}/{filename}");
        reviewed_modules.get_or_insert(version);
        replacements.insert(
            name,
            LibraryOverride {
                resource: context.store.describe_maven(&url).await?,
                filename,
            },
        );
    }
    ensure!(
        !replacements.is_empty(),
        "this version selects no LWJGL 3 Linux artifacts to substitute"
    );
    let mut jna_replaced = false;
    for (name, artifact, manifest_version) in jna {
        if !version_at_least(&manifest_version, crate::support::REVIEWED_ARM64_JNA) {
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
            replacements.insert(
                name,
                LibraryOverride {
                    resource: context.store.describe_maven(&url).await?,
                    filename,
                },
            );
            jna_replaced = true;
        }
    }
    let lwjgl = reviewed_modules.unwrap_or_default();
    let note = if reviewed.is_some() {
        format!(
            "Linux ARM64: the manifest's LWJGL {} is replaced with the reviewed {lwjgl} composition and its official natives-linux-arm64 classifiers from Maven Central",
            versions.iter().cloned().collect::<Vec<_>>().join("/")
        )
    } else {
        format!(
            "Linux ARM64: LWJGL {lwjgl} keeps its version and uses the official natives-linux-arm64 classifiers from Maven Central"
        )
    };
    let note = if jna_replaced {
        format!(
            "{note}; JNA is replaced with the official {} release",
            crate::support::REVIEWED_ARM64_JNA
        )
    } else {
        note
    };
    Ok((replacements, Some(note)))
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
