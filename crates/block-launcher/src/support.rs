//! Native architecture policy shared by catalog views, planners and saved-plan execution.

use std::{collections::BTreeMap, error::Error, fmt};

use minecraft_manifest::model::VersionManifest;

use crate::{
    plan::ExecutionPlan,
    platform::{Architecture, OperatingSystem, Target},
    rules,
};

/// An explicit catalog decision; unsupported entries can remain visible with a reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeSupport {
    Supported,
    Unsupported(UnsupportedNativeSupport),
}

/// A native support failure, preserved as a typed error through resolution and execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedNativeSupport {
    pub version: String,
    pub target: Target,
    pub reason: NativeSupportReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeSupportReason {
    LegacyLwjgl,
    MissingArm64Artifact { library: String },
    MissingCoreArtifact,
    MissingWindowArtifact,
}

impl fmt::Display for UnsupportedNativeSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Minecraft {} is unavailable for native macOS ARM64: ",
            self.version
        )?;
        match &self.reason {
            NativeSupportReason::LegacyLwjgl => {
                f.write_str("the selected legacy LWJGL natives do not declare ARM64 support")
            }
            NativeSupportReason::MissingArm64Artifact { library } => {
                write!(f, "{library} has no selected natives-macos-arm64 artifact")
            }
            NativeSupportReason::MissingCoreArtifact => {
                f.write_str("the selected artifacts do not include native ARM64 LWJGL core")
            }
            NativeSupportReason::MissingWindowArtifact => f.write_str(
                "the selected artifacts do not include a native ARM64 GLFW or SDL window backend",
            ),
        }
    }
}

impl Error for UnsupportedNativeSupport {}

impl NativeSupport {
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    #[must_use]
    pub const fn reason(&self) -> Option<&UnsupportedNativeSupport> {
        match self {
            Self::Supported => None,
            Self::Unsupported(reason) => Some(reason),
        }
    }

    /// Enforce the same decision used to explain availability in the UI.
    ///
    /// # Errors
    /// Returns the typed native support failure for unsupported metadata.
    pub fn require(self) -> Result<(), UnsupportedNativeSupport> {
        match self {
            Self::Supported => Ok(()),
            Self::Unsupported(reason) => Err(reason),
        }
    }
}

/// Inspect selected native artifacts without downloading game assets or runtimes.
///
/// This policy establishes native architecture availability, not compatibility of arbitrary mods.
/// Modern manifests may intentionally select both x64 and ARM64 jars; matching ARM64 artifacts
/// must exist for every selected LWJGL module/version. No version-number cutoff is used.
///
/// # Errors
/// Fails if a manifest contains invalid selection rules.
pub fn manifest_support(
    manifest: &VersionManifest,
    target: &Target,
    features: &BTreeMap<String, bool>,
) -> anyhow::Result<NativeSupport> {
    if !needs_arm64_policy(target) {
        return Ok(NativeSupport::Supported);
    }
    let mut evidence = NativeEvidence::default();
    for library in &manifest.libraries {
        if !rules::allowed(library.rules.as_deref(), target, features)? {
            continue;
        }
        if library.name.starts_with("org.lwjgl.lwjgl:") {
            evidence.legacy = true;
        }
        if !library.name.starts_with("org.lwjgl:") {
            continue;
        }
        if let Some(classifier) = library.natives.as_ref().and_then(|items| items.get("osx")) {
            let classifier = classifier.replace("${arch}", target.bits());
            let artifact = library
                .downloads
                .classifiers
                .as_ref()
                .and_then(|items| items.get(&classifier));
            if let Some(artifact) = artifact {
                evidence.record(&artifact.path);
            }
        }
        if let Some(artifact) = &library.downloads.artifact {
            evidence.record(&artifact.path);
        }
    }
    Ok(evidence.decision(&manifest.id, target))
}

/// Recheck the architecture evidence embedded in a serialized command plan.
///
/// Plans are executable configuration, not an untrusted security boundary. This guard prevents
/// accidentally replaying older/x64-only plans as native ARM64 without changing their schema.
#[must_use]
pub fn plan_support(plan: &ExecutionPlan) -> NativeSupport {
    if !needs_arm64_policy(&plan.target) {
        return NativeSupport::Supported;
    }
    let mut evidence = NativeEvidence::default();
    for path in &plan.command.classpath {
        evidence.record(path);
    }
    evidence.decision(&plan.version, &plan.target)
}

const fn needs_arm64_policy(target: &Target) -> bool {
    matches!(target.os, OperatingSystem::Macos) && matches!(target.arch, Architecture::Aarch64)
}

#[derive(Debug, Default)]
struct NativeEvidence {
    modules: BTreeMap<String, bool>,
    legacy: bool,
}

impl NativeEvidence {
    fn record(&mut self, path: &str) {
        let filename = path.rsplit('/').next().unwrap_or(path);
        // Materialized plan filenames have a content-address prefix; manifest paths do not.
        let filename = filename
            .get(41..)
            .filter(|_| {
                filename.as_bytes().get(40) == Some(&b'-')
                    && filename.as_bytes()[..40].iter().all(u8::is_ascii_hexdigit)
            })
            .unwrap_or(filename);
        if filename.starts_with("lwjgl-2.") || filename.starts_with("lwjgl_util-2.") {
            self.legacy = true;
        }
        if !filename.starts_with("lwjgl-") {
            return;
        }
        let Some((module_version, classifier)) = filename.rsplit_once("-natives-macos") else {
            return;
        };
        let arm64 = classifier == "-arm64.jar";
        self.modules
            .entry(module_version.to_owned())
            .and_modify(|found| *found |= arm64)
            .or_insert(arm64);
    }

    fn decision(self, version: &str, target: &Target) -> NativeSupport {
        let reason = if self.legacy {
            Some(NativeSupportReason::LegacyLwjgl)
        } else if let Some((library, _)) = self.modules.iter().find(|(_, arm64)| !**arm64) {
            Some(NativeSupportReason::MissingArm64Artifact {
                library: library.clone(),
            })
        } else if !self
            .modules
            .keys()
            .any(|name| name.strip_prefix("lwjgl-").is_some_and(starts_with_digit))
        {
            Some(NativeSupportReason::MissingCoreArtifact)
        } else if !self
            .modules
            .keys()
            .any(|name| name.starts_with("lwjgl-glfw-") || name.starts_with("lwjgl-sdl-"))
        {
            Some(NativeSupportReason::MissingWindowArtifact)
        } else {
            None
        };
        reason.map_or(NativeSupport::Supported, |reason| {
            NativeSupport::Unsupported(UnsupportedNativeSupport {
                version: version.to_owned(),
                target: target.clone(),
                reason,
            })
        })
    }
}

fn starts_with_digit(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_digit)
}
