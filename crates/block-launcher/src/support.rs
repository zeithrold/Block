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

/// The last official Minecraft release whose client bundles LWJGL 2.
pub const LAST_LWJGL2_RELEASE: &str = "1.12.2";

/// Reason a LWJGL 2-era version is unavailable on native ARM64 hosts.
pub const LWJGL2_ARM64_REASON: &str = "official LWJGL 2 natives are x86/x64 only";

/// Release time of [`LAST_LWJGL2_RELEASE`] in the official version index. Used
/// as a fallback only when the catalog has not loaded the 1.12.2 entry yet.
const LAST_LWJGL2_RELEASE_TIME: &str = "2017-09-18T11:33:40+00:00";

/// Why a catalog version cannot run natively on the requested architecture.
///
/// LWJGL 2-era versions (1.12.2 and earlier, including their snapshots) have
/// official natives only for x86/x64. The official catalog's release times are
/// authoritative; the 1.12.2 entry is the boundary of the LWJGL 3 switch.
#[must_use]
pub fn legacy_lwjgl_reason(
    arch: Architecture,
    entry: &minecraft_manifest::model::VersionEntry,
    catalog: &[minecraft_manifest::model::VersionEntry],
) -> Option<&'static str> {
    if arch != Architecture::Aarch64 {
        return None;
    }
    let boundary = catalog
        .iter()
        .find(|entry| entry.id == LAST_LWJGL2_RELEASE)
        .map_or(LAST_LWJGL2_RELEASE_TIME, |entry| {
            entry.release_time.as_str()
        });
    // The official index uses one ISO-8601 format and offset for every entry, so
    // the lexicographic order matches chronological order.
    (entry.release_time.as_str() <= boundary).then_some(LWJGL2_ARM64_REASON)
}

/// Message shown when a specific version is refused on native ARM64.
#[must_use]
pub fn legacy_lwjgl_message(version: &str) -> String {
    format!("Minecraft {version} cannot run natively on ARM64: {LWJGL2_ARM64_REASON}")
}

/// The reviewed LWJGL release that ships official Linux ARM64 natives.
/// Manifests that select the older 3.2.x modules are replaced with this release,
/// the composition proven on Linux ARM64 1.16.5.
pub const REVIEWED_LINUX_ARM64_LWJGL: &str = "3.3.1";

/// The reviewed LWJGL release that ships official Windows ARM64 natives from
/// Maven Central (`natives-windows-arm64` classifiers). Manifests that select the
/// older 3.2.x modules are replaced with this release.
pub const REVIEWED_WINDOWS_ARM64_LWJGL: &str = "3.3.1";

/// The reviewed JNA release whose bundled natives include Linux and macOS ARM64.
pub const REVIEWED_ARM64_JNA: &str = "5.13.0";

/// The reviewed text2speech release that loads without an ARM64 native wrapper.
pub const REVIEWED_TEXT2SPEECH: &str = "1.12.4";

/// The official slf4j-api release the reviewed text2speech depends on. Manifests
/// that predate text2speech 1.12.4 do not list it, so the plan adds it alongside
/// the replacement. The artifact is the one the official 1.18.2 manifest pairs
/// with text2speech 1.12.4.
pub const REVIEWED_TEXT2SPEECH_SLF4J: &str = "1.8.0-beta4";

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
    if versions
        .iter()
        .all(|version| matches!(version.as_str(), "3.2.1" | "3.2.2"))
    {
        return Ok(LinuxArm64LwjglPlan::ReplaceWith(REVIEWED_LINUX_ARM64_LWJGL));
    }
    let selected = versions.iter().cloned().collect::<Vec<_>>().join(", ");
    Err(format!(
        "no reviewed Linux ARM64 composition exists for LWJGL {selected}; reviewed: {REVIEWED_LINUX_ARM64_LWJGL} or newer, and the 3.2.1/3.2.2 replacement with {REVIEWED_LINUX_ARM64_LWJGL}"
    ))
}

/// Whether this LWJGL 3 release publishes official Windows ARM64 natives.
#[must_use]
pub fn lwjgl_supports_windows_arm64(version: &str) -> bool {
    version_at_least(version, REVIEWED_WINDOWS_ARM64_LWJGL)
}

/// The reviewed Windows ARM64 composition for one manifest's selected LWJGL modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsArm64LwjglPlan {
    /// The selected modules already publish ARM64 natives at their own version.
    SameVersion,
    /// Replace every selected module with this reviewed LWJGL release.
    ReplaceWith(&'static str),
}

/// Decide the reviewed Windows ARM64 substitution for a manifest's LWJGL modules.
///
/// # Errors
/// Fails with a human-readable reason when no reviewed composition exists.
pub fn windows_arm64_lwjgl_plan(
    versions: &BTreeSet<String>,
) -> Result<WindowsArm64LwjglPlan, String> {
    if versions.is_empty() {
        return Err("the manifest selects no LWJGL 3 modules".into());
    }
    if versions
        .iter()
        .all(|version| lwjgl_supports_windows_arm64(version))
    {
        return Ok(WindowsArm64LwjglPlan::SameVersion);
    }
    if versions
        .iter()
        .all(|version| matches!(version.as_str(), "3.2.1" | "3.2.2"))
    {
        return Ok(WindowsArm64LwjglPlan::ReplaceWith(
            REVIEWED_WINDOWS_ARM64_LWJGL,
        ));
    }
    let selected = versions.iter().cloned().collect::<Vec<_>>().join(", ");
    Err(format!(
        "no reviewed Windows ARM64 composition exists for LWJGL {selected}; reviewed: {REVIEWED_WINDOWS_ARM64_LWJGL} or newer, and the 3.2.1/3.2.2 replacement with {REVIEWED_WINDOWS_ARM64_LWJGL}"
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
        (OperatingSystem::Windows, Architecture::Aarch64) => {
            windows_arm64_support(manifest, target, features)
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
        for resource in &plan.resources {
            if plan_filename(&resource.url).ends_with("-natives-linux-arm64.jar") {
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
    if plan.target.os == OperatingSystem::Windows {
        // The Windows plan carries the substituted ARM64 classifiers explicitly.
        let mut legacy = false;
        let mut arm64_native = false;
        for path in &plan.command.classpath {
            let filename = plan_filename(path);
            if filename.starts_with("lwjgl-2.") || filename.starts_with("lwjgl_util-2.") {
                legacy = true;
            }
            if filename.ends_with("-natives-windows-arm64.jar") {
                arm64_native = true;
            }
        }
        for resource in &plan.resources {
            if plan_filename(&resource.url).ends_with("-natives-windows-arm64.jar") {
                arm64_native = true;
            }
        }
        let reason = if legacy {
            Some(NativeSupportReason::LegacyLwjgl)
        } else if !arm64_native {
            Some(NativeSupportReason::MissingArm64Artifact {
                library: "Windows ARM64 natives".into(),
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
        && matches!(
            target.os,
            OperatingSystem::Macos | OperatingSystem::Linux | OperatingSystem::Windows
        )
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

/// Windows ARM64 selects its substitution explicitly in the plan; this policy only
/// rejects versions for which no reviewed official composition exists.
fn windows_arm64_support(
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
        windows_arm64_lwjgl_plan(&versions).err().map(|_| {
            NativeSupportReason::MissingArm64Artifact {
                library: format!(
                    "LWJGL {}",
                    versions.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            }
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
        LAST_LWJGL2_RELEASE, LWJGL2_ARM64_REASON, LinuxArm64LwjglPlan, REVIEWED_LINUX_ARM64_LWJGL,
        legacy_lwjgl_message, legacy_lwjgl_reason, linux_arm64_lwjgl_plan,
        lwjgl_supports_linux_arm64,
    };
    use crate::platform::Architecture;
    use minecraft_manifest::model::{VersionEntry, VersionType};
    use std::collections::BTreeSet;

    fn catalog_entry(id: &str, release_time: &str) -> VersionEntry {
        VersionEntry {
            id: id.into(),
            kind: VersionType::Release,
            url: format!("https://example.invalid/{id}.json"),
            time: release_time.into(),
            release_time: release_time.into(),
            sha1: "a".repeat(40),
            compliance_level: 1,
        }
    }

    fn catalog() -> Vec<VersionEntry> {
        vec![
            catalog_entry("1.21.1", "2024-08-08T12:21:54+00:00"),
            catalog_entry("1.18.2", "2022-02-28T11:22:16+00:00"),
            catalog_entry("1.16.5", "2021-01-14T16:05:29+00:00"),
            catalog_entry("1.13", "2018-07-18T12:57:53+00:00"),
            catalog_entry(LAST_LWJGL2_RELEASE, "2017-09-18T11:33:40+00:00"),
            catalog_entry("1.8.9", "2015-12-09T09:19:57+00:00"),
            catalog_entry("17w43a", "2017-10-25T14:13:24+00:00"),
        ]
    }

    fn entry(catalog: &[VersionEntry], id: &str) -> anyhow::Result<VersionEntry> {
        catalog
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing catalog entry {id}"))
    }

    #[test]
    fn arm64_marks_lwjgl2_versions_unsupported_and_keeps_modern_ones() -> anyhow::Result<()> {
        let catalog = catalog();
        for legacy in [LAST_LWJGL2_RELEASE, "1.8.9"] {
            assert_eq!(
                legacy_lwjgl_reason(Architecture::Aarch64, &entry(&catalog, legacy)?, &catalog),
                Some(LWJGL2_ARM64_REASON),
                "{legacy}"
            );
        }
        for modern in ["1.13", "1.16.5", "1.18.2", "1.21.1", "17w43a"] {
            assert_eq!(
                legacy_lwjgl_reason(Architecture::Aarch64, &entry(&catalog, modern)?, &catalog),
                None,
                "{modern}"
            );
        }
        assert!(
            legacy_lwjgl_message("1.12.2").contains(LWJGL2_ARM64_REASON),
            "the refusal must keep the published reason"
        );
        Ok(())
    }

    #[test]
    fn x64_keeps_lwjgl2_versions_supported() -> anyhow::Result<()> {
        for arch in [Architecture::X86_64, Architecture::X86] {
            let catalog = catalog();
            for id in [LAST_LWJGL2_RELEASE, "1.8.9"] {
                assert_eq!(
                    legacy_lwjgl_reason(arch, &entry(&catalog, id)?, &catalog),
                    None,
                    "{id}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn missing_catalog_boundary_uses_the_recorded_release_time() {
        let catalog = vec![catalog_entry("1.8.9", "2015-12-09T09:19:57+00:00")];
        let entry = catalog[0].clone();
        assert_eq!(
            legacy_lwjgl_reason(Architecture::Aarch64, &entry, &catalog),
            Some(LWJGL2_ARM64_REASON)
        );
    }

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
        // The proven 1.16.5 composition replaces the classifier-era release.
        assert_eq!(
            linux_arm64_lwjgl_plan(&versions(&["3.2.1", "3.2.2"])),
            Ok(LinuxArm64LwjglPlan::ReplaceWith(REVIEWED_LINUX_ARM64_LWJGL))
        );
        assert_eq!(
            linux_arm64_lwjgl_plan(&versions(&["3.2.2"])),
            Ok(LinuxArm64LwjglPlan::ReplaceWith(REVIEWED_LINUX_ARM64_LWJGL))
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
