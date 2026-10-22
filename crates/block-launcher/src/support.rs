//! Native architecture policy shared by catalog views, planners and saved-plan execution.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

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
        let platform = match self.target.os {
            OperatingSystem::Macos => "macOS",
            OperatingSystem::Linux => "Linux",
            OperatingSystem::Windows => "Windows",
        };
        let architecture = match self.target.arch {
            Architecture::Aarch64 => "ARM64",
            Architecture::X86_64 => "x64",
            Architecture::X86 => "x86",
        };
        write!(
            f,
            "Minecraft {} is unavailable for native {platform} {architecture}: ",
            self.version
        )?;
        match &self.reason {
            NativeSupportReason::LegacyLwjgl => {
                f.write_str("this version uses LWJGL 2, whose official natives are x86/x64 only")
            }
            NativeSupportReason::MissingArm64Artifact { library } => {
                write!(f, "no reviewed ARM64 artifact is available for {library}")
            }
            NativeSupportReason::MissingCoreArtifact => {
                f.write_str("the selected artifacts do not include an ARM64-capable LWJGL core")
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

/// The reviewed LWJGL release that ships official Linux ARM64 natives.
/// Manifests that select the older 3.2.x modules are replaced with this release,
/// the composition proven on Linux ARM64 1.16.5.
pub const REVIEWED_LINUX_ARM64_LWJGL: &str = "3.3.1";

/// The reviewed JNA release whose bundled natives include Linux and macOS ARM64.
pub const REVIEWED_ARM64_JNA: &str = "5.13.0";

/// Whether this LWJGL 3 release publishes official Linux ARM64 natives.
#[must_use]
pub fn lwjgl_supports_linux_arm64(version: &str) -> bool {
    version_at_least(version, REVIEWED_LINUX_ARM64_LWJGL)
}

/// The reviewed Linux ARM64 composition for one manifest's selected LWJGL modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxArm64LwjglPlan {
    /// The selected modules already publish ARM64 natives at their own version.
    SameVersion,
    /// Replace every selected module with this reviewed LWJGL release.
    ReplaceWith(&'static str),
}

/// Decide the reviewed substitution for the versions selected by a manifest.
///
/// # Errors
/// Fails with a human-readable reason when no reviewed composition exists.
pub fn linux_arm64_lwjgl_plan(versions: &BTreeSet<String>) -> Result<LinuxArm64LwjglPlan, String> {
    if versions.is_empty() {
        return Err("the manifest selects no LWJGL 3 modules".into());
    }
    if versions
        .iter()
        .all(|version| lwjgl_supports_linux_arm64(version))
    {
        return Ok(LinuxArm64LwjglPlan::SameVersion);
    }
    let selected = versions.iter().cloned().collect::<Vec<_>>().join(", ");
    Err(format!(
        "no reviewed Linux ARM64 composition exists for LWJGL {selected}; reviewed: {REVIEWED_LINUX_ARM64_LWJGL} or newer, whose official natives-linux-arm64 classifiers are substituted explicitly"
    ))
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
    match (target.os, target.arch) {
        (OperatingSystem::Macos, Architecture::Aarch64) => {
            macos_arm64_support(manifest, target, features)
        }
        (OperatingSystem::Linux, Architecture::Aarch64) => {
            linux_arm64_support(manifest, target, features)
        }
        _ => Ok(NativeSupport::Supported),
    }
}

fn macos_arm64_support(
    manifest: &VersionManifest,
    target: &Target,
    features: &BTreeMap<String, bool>,
) -> anyhow::Result<NativeSupport> {
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
    if plan.target.os == OperatingSystem::Linux {
        // The Linux plan carries the substituted ARM64 classifiers explicitly.
        let mut evidence = NativeEvidence::default();
        let mut arm64_native = false;
        for path in &plan.command.classpath {
            let filename = plan_filename(path);
            if filename.starts_with("lwjgl-2.") || filename.starts_with("lwjgl_util-2.") {
                evidence.legacy = true;
            }
            if filename.ends_with("-natives-linux-arm64.jar") {
                arm64_native = true;
            }
        }
        let reason = if evidence.legacy {
            Some(NativeSupportReason::LegacyLwjgl)
        } else if !arm64_native {
            Some(NativeSupportReason::MissingArm64Artifact {
                library: "Linux ARM64 natives".into(),
            })
        } else {
            None
        };
        return reason.map_or(NativeSupport::Supported, |reason| {
            NativeSupport::Unsupported(UnsupportedNativeSupport {
                version: plan.version.clone(),
                target: plan.target.clone(),
                reason,
            })
        });
    }
    let mut evidence = NativeEvidence::default();
    for path in &plan.command.classpath {
        evidence.record(path);
    }
    evidence.decision(&plan.version, &plan.target)
}

fn plan_filename(path: &str) -> &str {
    let filename = path.rsplit('/').next().unwrap_or(path);
    // Materialized plan filenames have a content-address prefix; manifest paths do not.
    filename
        .get(41..)
        .filter(|_| {
            filename.as_bytes().get(40) == Some(&b'-')
                && filename.as_bytes()[..40].iter().all(u8::is_ascii_hexdigit)
        })
        .unwrap_or(filename)
}

/// Whether this target always needs an architecture review before planning.
const fn needs_arm64_policy(target: &Target) -> bool {
    matches!(target.arch, Architecture::Aarch64)
        && matches!(target.os, OperatingSystem::Macos | OperatingSystem::Linux)
}

/// Linux ARM64 selects its substitution explicitly in the plan; this policy only
/// rejects versions for which no reviewed official composition exists.
fn linux_arm64_support(
    manifest: &VersionManifest,
    target: &Target,
    features: &BTreeMap<String, bool>,
) -> anyhow::Result<NativeSupport> {
    let mut legacy = false;
    let mut versions = BTreeSet::new();
    for library in &manifest.libraries {
        if !rules::allowed(library.rules.as_deref(), target, features)? {
            continue;
        }
        if library.name.starts_with("org.lwjgl.lwjgl:") {
            legacy = true;
        }
        if let Some(rest) = library.name.strip_prefix("org.lwjgl:")
            && let Some((_, version)) = rest.split_once(':')
            && let Some(version) = version.split(':').next()
        {
            versions.insert(version.to_owned());
        }
    }
    let reason = if legacy {
        Some(NativeSupportReason::LegacyLwjgl)
    } else if versions.is_empty() {
        Some(NativeSupportReason::MissingCoreArtifact)
    } else {
        linux_arm64_lwjgl_plan(&versions)
            .err()
            .map(|_| NativeSupportReason::MissingArm64Artifact {
                library: format!(
                    "LWJGL {}",
                    versions.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            })
    };
    Ok(reason.map_or(NativeSupport::Supported, |reason| {
        NativeSupport::Unsupported(UnsupportedNativeSupport {
            version: manifest.id.clone(),
            target: target.clone(),
            reason,
        })
    }))
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

#[derive(Debug, Default)]
struct NativeEvidence {
    modules: BTreeMap<String, bool>,
    legacy: bool,
}

impl NativeEvidence {
    fn record(&mut self, path: &str) {
        let filename = plan_filename(path);
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

#[cfg(test)]
mod tests {
    use super::{
        LinuxArm64LwjglPlan, REVIEWED_LINUX_ARM64_LWJGL, linux_arm64_lwjgl_plan,
        lwjgl_supports_linux_arm64,
    };
    use std::collections::BTreeSet;

    fn versions(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn reviewed_linux_arm64_composition_follows_the_manifest_lwjgl_version() {
        assert!(lwjgl_supports_linux_arm64(REVIEWED_LINUX_ARM64_LWJGL));
        assert!(lwjgl_supports_linux_arm64("3.3.3"));
        assert!(lwjgl_supports_linux_arm64("3.4.3"));
        assert!(!lwjgl_supports_linux_arm64("3.2.2"));
        assert_eq!(
            linux_arm64_lwjgl_plan(&versions(&["3.3.1"])),
            Ok(LinuxArm64LwjglPlan::SameVersion)
        );
        assert_eq!(
            linux_arm64_lwjgl_plan(&versions(&["3.3.1", "3.3.3"])),
            Ok(LinuxArm64LwjglPlan::SameVersion)
        );
        // The proven 1.16.5 composition is not wired into planning yet.
        let error = linux_arm64_lwjgl_plan(&versions(&["3.2.1", "3.2.2"]))
            .err()
            .map(|reason| reason.to_owned());
        assert!(
            error
                .as_ref()
                .is_some_and(|reason| reason.contains("3.2.1, 3.2.2")),
            "{error:?}"
        );
        let error = linux_arm64_lwjgl_plan(&versions(&["3.1.6"]))
            .err()
            .map(|reason| reason.to_owned());
        assert!(
            error
                .as_ref()
                .is_some_and(|reason| reason.contains("reviewed: 3.3.1")),
            "{error:?}"
        );
        assert!(linux_arm64_lwjgl_plan(&BTreeSet::new()).is_err());
    }
}
