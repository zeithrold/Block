use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, ensure};
use block_download::{Resource, Store};
use futures_util::future::BoxFuture;
use sha1::{Digest as _, Sha1};
use tokio_util::sync::CancellationToken;

use crate::{
    manifest::{
        decode,
        model::{Download, VersionIndex, VersionManifest},
        resources::{AssetObjects, RuntimeCatalog, RuntimeManifest},
    },
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
    /// Artifacts added beyond the manifest, such as a dependency the reviewed
    /// replacement needs but the older manifest never listed.
    pub companion_libraries: Vec<LibraryOverride>,
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
    let (library_overrides, classifier_overrides, companion_libraries, compatibility) =
        compatibility_overrides(recipe, &parsed, &target, context).await?;
    Ok(ResolvedMetadata {
        parsed,
        target,
        assets,
        runtime,
        library_overrides,
        companion_libraries,
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
    Vec<LibraryOverride>,
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
            } else if target.os == OperatingSystem::Windows && target.arch == Architecture::Aarch64
            {
                windows_arm64_overrides(recipe, parsed, target, context).await
            } else {
                Ok((BTreeMap::new(), BTreeMap::new(), Vec::new(), None))
            }
        }
        Compatibility::LinuxArm64Lwjgl3 => {
            linux_arm64_overrides(recipe, parsed, target, context).await
        }
        Compatibility::WindowsArm64Lwjgl3 => {
            windows_arm64_overrides(recipe, parsed, target, context).await
        }
        Compatibility::MacosJna513 => Ok((
            macos_jna_overrides(recipe, parsed, target, context).await?,
            BTreeMap::new(),
            Vec::new(),
            Some(format!(
                "macOS ARM64: the affected JNA pair is replaced with the official JNA {} release from Maven Central",
                crate::support::REVIEWED_ARM64_JNA
            )),
        )),
    }
}

/// The host facts that select one ARM64 LWJGL substitution.
///
/// Linux and Windows ARM64 need the same decision: swap each x64 native
/// classifier a manifest selected for the ARM64 classifier this host loads, and
/// move classifier-era manifests to the reviewed composition. Only the names and
/// the reviewed-composition planner differ, so both paths share this description
/// and the helpers below.
#[derive(Debug)]
struct Arm64LwjglHost {
    /// Compatibility rule that selects this substitution.
    rule: &'static str,
    /// Host name used in diagnostics and in the recorded note.
    label: &'static str,
    os: OperatingSystem,
    /// Key this host uses in a manifest's `natives` map.
    natives_key: &'static str,
    /// Classifier a manifest selects for this host's x64 natives.
    x64_classifier: &'static str,
    /// Classifier this host loads natively.
    arm64_classifier: &'static str,
    /// Reviewed composition for the selected LWJGL versions.
    plan: fn(&BTreeSet<String>) -> Result<Option<&'static str>, String>,
}

const LINUX_ARM64: Arm64LwjglHost = Arm64LwjglHost {
    rule: "linux_arm64_lwjgl3",
    label: "Linux",
    os: OperatingSystem::Linux,
    natives_key: "linux",
    x64_classifier: "natives-linux",
    arm64_classifier: "natives-linux-arm64",
    plan: linux_arm64_plan,
};

const WINDOWS_ARM64: Arm64LwjglHost = Arm64LwjglHost {
    rule: "windows_arm64_lwjgl3",
    label: "Windows",
    os: OperatingSystem::Windows,
    natives_key: "windows",
    x64_classifier: "natives-windows",
    arm64_classifier: "natives-windows-arm64",
    plan: windows_arm64_plan,
};

fn linux_arm64_plan(versions: &BTreeSet<String>) -> Result<Option<&'static str>, String> {
    crate::support::linux_arm64_lwjgl_plan(versions).map(|plan| match plan {
        crate::support::LinuxArm64LwjglPlan::SameVersion => None,
        crate::support::LinuxArm64LwjglPlan::ReplaceWith(version) => Some(version),
    })
}

fn windows_arm64_plan(versions: &BTreeSet<String>) -> Result<Option<&'static str>, String> {
    crate::support::windows_arm64_lwjgl_plan(versions).map(|plan| match plan {
        crate::support::WindowsArm64LwjglPlan::SameVersion => None,
        crate::support::WindowsArm64LwjglPlan::ReplaceWith(version) => Some(version),
    })
}

impl Arm64LwjglHost {
    /// Require the target this substitution covers.
    fn require(&self, target: &Target) -> anyhow::Result<()> {
        ensure!(
            target.os == self.os && target.arch == Architecture::Aarch64,
            "{} requires a {} ARM64 target",
            self.rule,
            self.label
        );
        Ok(())
    }
}

/// One selected Maven artifact, split into the parts a substitution needs.
#[derive(Debug, Clone)]
struct SelectedArtifact {
    /// Library name the override map is keyed on.
    name: String,
    module: String,
    version: String,
    classifier: Option<String>,
}

impl SelectedArtifact {
    /// Filename this artifact becomes on an ARM64 host, or `None` when it is
    /// already ARM64, unrelated to this host, or a core jar that only changes
    /// with the reviewed composition.
    fn arm64_filename(&self, host: &Arm64LwjglHost, reviewed: Option<&str>) -> Option<String> {
        let version = reviewed.unwrap_or(&self.version);
        match self.classifier.as_deref() {
            Some(classifier) if classifier == host.x64_classifier => Some(format!(
                "{}-{version}-{}.jar",
                self.module, host.arm64_classifier
            )),
            None if reviewed.is_some() => Some(format!("{}-{version}.jar", self.module)),
            _ => None,
        }
    }
}

/// The LWJGL artifacts one manifest selected for an ARM64 host.
#[derive(Debug, Default)]
struct Arm64LwjglSelection {
    /// Selected `org.lwjgl:` artifacts, with their classifiers.
    artifacts: Vec<SelectedArtifact>,
    /// Native artifacts the manifest declares through its `natives` map.
    natives: Vec<SelectedArtifact>,
    versions: BTreeSet<String>,
}

/// Collect the LWJGL artifacts one manifest selects for this host.
fn select_arm64_lwjgl(
    recipe: &LaunchRecipe,
    parsed: &ParsedRecipe,
    target: &Target,
    host: &Arm64LwjglHost,
) -> anyhow::Result<Arm64LwjglSelection> {
    let mut selection = Arm64LwjglSelection::default();
    for library in &parsed.manifest.libraries {
        if !crate::rules::allowed(library.rules.as_deref(), target, &recipe.features)? {
            continue;
        }
        if library.name.starts_with("org.lwjgl.lwjgl:") {
            anyhow::bail!(
                "this version uses LWJGL 2, whose official natives are x86/x64 only; {label} ARM64 support covers LWJGL 3-era versions",
                label = host.label
            );
        }
        let Some(rest) = library.name.strip_prefix("org.lwjgl:") else {
            continue;
        };
        let mut parts = rest.split(':');
        let (Some(module), Some(version)) = (parts.next(), parts.next()) else {
            continue;
        };
        let artifact = SelectedArtifact {
            name: library.name.clone(),
            module: module.to_owned(),
            version: version.to_owned(),
            classifier: parts.next().map(str::to_owned),
        };
        selection.versions.insert(artifact.version.clone());
        if let Some(native) = library
            .natives
            .as_ref()
            .and_then(|natives| natives.get(host.natives_key))
        {
            selection.natives.push(SelectedArtifact {
                classifier: Some(native.clone()),
                ..artifact.clone()
            });
        }
        selection.artifacts.push(artifact);
    }
    Ok(selection)
}

/// The overrides one ARM64 substitution produces, and the facts its note needs.
#[derive(Debug, Default)]
struct Arm64LwjglPlan {
    /// Replacements keyed by the library that selects the artifact.
    library_overrides: BTreeMap<String, LibraryOverride>,
    /// Replacement native classifiers for `natives`-map manifests.
    classifier_overrides: BTreeMap<String, LibraryOverride>,
    /// Release the plan moved the modules to, when it moved them.
    reviewed_module: Option<String>,
    /// LWJGL versions the manifest selected, for the recorded note.
    versions: BTreeSet<String>,
    /// Reviewed release the modules moved to, when the manifest's own release
    /// predates this host's natives.
    reviewed: Option<&'static str>,
}

impl Arm64LwjglPlan {
    /// Describe what the substitution changed, and why, for the saved plan and
    /// the launch log.
    fn note(&self, host: &Arm64LwjglHost) -> String {
        let lwjgl = self.reviewed_module.as_deref().unwrap_or_default();
        let arm64 = host.arm64_classifier;
        let label = host.label;
        if self.reviewed.is_some() {
            let selected = self.versions.iter().cloned().collect::<Vec<_>>().join("/");
            format!(
                "{label} ARM64: the manifest's LWJGL {selected} is replaced with the reviewed {lwjgl} composition and its official {arm64} classifiers from Maven Central"
            )
        } else if self.library_overrides.is_empty() && self.classifier_overrides.is_empty() {
            format!("{label} ARM64: LWJGL {lwjgl} already selects official {arm64} artifacts")
        } else {
            format!(
                "{label} ARM64: LWJGL {lwjgl} keeps its version and uses the official {arm64} classifiers from Maven Central"
            )
        }
    }
}

/// Describe one Maven artifact as a library override.
async fn library_override(
    store: &Store,
    url: &str,
    filename: String,
) -> anyhow::Result<LibraryOverride> {
    Ok(LibraryOverride {
        resource: store.describe_maven(url).await?,
        filename,
    })
}

/// Describe the LWJGL artifact a substitution replaces.
async fn lwjgl_override(
    context: &ResolveContext,
    artifact: &SelectedArtifact,
    version: &str,
    filename: String,
) -> anyhow::Result<LibraryOverride> {
    let url = format!(
        "https://repo.maven.apache.org/maven2/org/lwjgl/{module}/{version}/{filename}",
        module = artifact.module
    );
    library_override(&context.store, &url, filename).await
}

/// Plan the replacements an ARM64 host needs for one manifest.
async fn plan_arm64_lwjgl(
    recipe: &LaunchRecipe,
    parsed: &ParsedRecipe,
    target: &Target,
    context: &ResolveContext,
    host: &Arm64LwjglHost,
) -> anyhow::Result<Arm64LwjglPlan> {
    host.require(target)?;
    let selection = select_arm64_lwjgl(recipe, parsed, target, host)?;
    let reviewed = (host.plan)(&selection.versions).map_err(anyhow::Error::msg)?;
    let mut plan = Arm64LwjglPlan {
        versions: selection.versions,
        reviewed,
        ..Arm64LwjglPlan::default()
    };
    for artifact in &selection.artifacts {
        let Some(filename) = artifact.arm64_filename(host, reviewed) else {
            continue;
        };
        let version = reviewed.unwrap_or(&artifact.version);
        plan.reviewed_module
            .get_or_insert_with(|| version.to_owned());
        let entry = lwjgl_override(context, artifact, version, filename).await?;
        plan.library_overrides.insert(artifact.name.clone(), entry);
    }
    for native in &selection.natives {
        let classifier = native.classifier.as_deref().unwrap_or_default();
        ensure!(
            classifier == host.x64_classifier,
            "unexpected {label} native classifier {classifier} for {name}",
            label = host.label,
            name = native.name
        );
        let version = reviewed.unwrap_or(&native.version);
        let filename = format!("{}-{version}-{}.jar", native.module, host.arm64_classifier);
        plan.reviewed_module
            .get_or_insert_with(|| version.to_owned());
        let entry = lwjgl_override(context, native, version, filename).await?;
        plan.classifier_overrides.insert(native.name.clone(), entry);
    }
    // A manifest may already select official ARM64 natives (for example a
    // synthetic fixture or a future official update); nothing needs replacing.
    let already_arm64 = selection
        .artifacts
        .iter()
        .any(|artifact| artifact.classifier.as_deref() == Some(host.arm64_classifier));
    ensure!(
        !plan.library_overrides.is_empty()
            || !plan.classifier_overrides.is_empty()
            || already_arm64,
        "this version selects no LWJGL 3 {label} artifacts to substitute",
        label = host.label
    );
    Ok(plan)
}

/// The Linux ARM64 companion artifacts one manifest declares.
#[derive(Debug, Default)]
struct LinuxArm64Companions {
    /// Replacements keyed by the library that selects them.
    overrides: BTreeMap<String, LibraryOverride>,
    /// Artifacts added beyond the manifest.
    libraries: Vec<LibraryOverride>,
    /// Human-readable list for the recorded note.
    phrases: Vec<String>,
    /// The manifest already lists slf4j-api.
    has_slf4j: bool,
}

/// Replace the companions a Linux ARM64 host cannot load.
///
/// JNA and text2speech below the reviewed releases bundle natives that predate
/// ARM64. The reviewed text2speech also needs an slf4j-api these manifests never
/// list, so it joins the classpath as a companion library.
async fn linux_arm64_companions(
    recipe: &LaunchRecipe,
    parsed: &ParsedRecipe,
    target: &Target,
    context: &ResolveContext,
) -> anyhow::Result<LinuxArm64Companions> {
    let mut companions = LinuxArm64Companions::default();
    let mut jna = Vec::new();
    let mut text2speech = Vec::new();
    for library in &parsed.manifest.libraries {
        if !crate::rules::allowed(library.rules.as_deref(), target, &recipe.features)? {
            continue;
        }
        let mut parts = library.name.split(':');
        match (parts.next(), parts.next()) {
            (Some("net.java.dev.jna"), Some(module)) => {
                if let (Some(version), None) = (parts.next(), parts.next()) {
                    jna.push(SelectedArtifact {
                        name: library.name.clone(),
                        module: module.to_owned(),
                        version: version.to_owned(),
                        classifier: None,
                    });
                }
            }
            (Some("com.mojang"), Some("text2speech")) => {
                let version = parts.next().unwrap_or_default();
                text2speech.push(SelectedArtifact {
                    name: library.name.clone(),
                    module: "text2speech".to_owned(),
                    version: version.to_owned(),
                    classifier: None,
                });
            }
            (Some("org.slf4j"), Some("slf4j-api")) => companions.has_slf4j = true,
            _ => {}
        }
    }
    for artifact in jna {
        if version_at_least(&artifact.version, crate::support::REVIEWED_ARM64_JNA) {
            continue;
        }
        // The `platform` artifact is published as the `jna-platform` module.
        let module = if artifact.module == "platform" {
            "jna-platform"
        } else {
            artifact.module.as_str()
        };
        let version = crate::support::REVIEWED_ARM64_JNA;
        let filename = format!("{module}-{version}.jar");
        let url = format!(
            "https://repo.maven.apache.org/maven2/net/java/dev/jna/{module}/{version}/{filename}"
        );
        let entry = library_override(&context.store, &url, filename).await?;
        companions.overrides.insert(artifact.name, entry);
        companions.phrases.push(format!("JNA {version}"));
    }
    let mut replaced_text2speech = false;
    for artifact in text2speech {
        if version_at_least(&artifact.version, crate::support::REVIEWED_TEXT2SPEECH) {
            continue;
        }
        replaced_text2speech = true;
        let version = crate::support::REVIEWED_TEXT2SPEECH;
        let filename = format!("text2speech-{version}.jar");
        let url =
            format!("https://libraries.minecraft.net/com/mojang/text2speech/{version}/{filename}");
        let entry = library_override(&context.store, &url, filename).await?;
        companions.overrides.insert(artifact.name, entry);
        companions.phrases.push(format!("text2speech {version}"));
    }
    if replaced_text2speech && !companions.has_slf4j {
        let version = crate::support::REVIEWED_TEXT2SPEECH_SLF4J;
        let filename = format!("slf4j-api-{version}.jar");
        let url =
            format!("https://libraries.minecraft.net/org/slf4j/slf4j-api/{version}/{filename}");
        companions
            .libraries
            .push(library_override(&context.store, &url, filename).await?);
        companions
            .phrases
            .push(format!("slf4j-api {version} (text2speech dependency)"));
    }
    Ok(companions)
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
    let host = &LINUX_ARM64;
    let mut plan = plan_arm64_lwjgl(recipe, parsed, target, context, host).await?;
    let companions = linux_arm64_companions(recipe, parsed, target, context).await?;
    plan.library_overrides.extend(companions.overrides);
    let note = plan.note(host);
    let note = if companions.phrases.is_empty() {
        note
    } else {
        format!(
            "{note}; companions replaced with official releases: {}",
            companions.phrases.join(", ")
        )
    };
    Ok((
        plan.library_overrides,
        plan.classifier_overrides,
        companions.libraries,
        Some(format!(
            "{note}; narration has no native ARM64 flite wrapper, so it stays unavailable"
        )),
    ))
}

/// Replace the LWJGL native artifacts a Windows ARM64 host cannot load.
///
/// The decision is keyed on the manifest's own LWJGL modules, not on Minecraft
/// versions: 3.3.1 and newer keep their release and swap every selected
/// `natives-windows` classifier for the published `natives-windows-arm64` one,
/// while classifier-era 3.2.x manifests move to the reviewed 3.3.1 composition
/// whose natives are the first official Windows ARM64 publication. Every
/// artifact URL is checked through the published Maven metadata before it can
/// be planned, and x64 Windows targets never enter this path.
async fn windows_arm64_overrides(
    recipe: &LaunchRecipe,
    parsed: &ParsedRecipe,
    target: &Target,
    context: &ResolveContext,
) -> anyhow::Result<AppliedOverrides> {
    let host = &WINDOWS_ARM64;
    let plan = plan_arm64_lwjgl(recipe, parsed, target, context, host).await?;
    let note = plan.note(host);
    Ok((
        plan.library_overrides,
        plan.classifier_overrides,
        Vec::new(),
        Some(note),
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
            library_override(&context.store, &url, filename).await?,
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
