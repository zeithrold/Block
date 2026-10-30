//! Small native Java discovery for hosts without an official Mojang runtime.
use anyhow::Context as _;
use std::path::{Path, PathBuf};

/// Locate an already installed matching Java; never install packages or change alternatives.
/// # Errors
/// Reports that matching native Java is needed rather than using the wrong major silently.
pub fn system_java(major: u32) -> anyhow::Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        candidates.push(PathBuf::from(home).join("bin/java"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|directory| directory.join("java")));
    }
    if let Ok(entries) = std::fs::read_dir("/usr/lib/jvm") {
        for entry in entries.flatten() {
            candidates.push(entry.path().join("bin/java"));
        }
    }
    candidates.into_iter().filter_map(|path|path.canonicalize().ok()).find(|path|release_major(path)==Some(major)).with_context(||format!("No installed native Java{major} was detected. Install Java{major} or select its executable in this Profile. Block did not change system Java settings."))
}

fn release_major(java: &Path) -> Option<u32> {
    let release = java.parent()?.parent()?.join("release");
    let content = std::fs::read_to_string(release).ok()?;
    let version = content
        .lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))?
        .trim_matches('"');
    parse_major(version)
}

fn parse_major(version: &str) -> Option<u32> {
    let mut parts = version.split(['.', '-', '_', '+']);
    let first = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_major;
    #[test]
    fn native_java_release_versions_preserve_legacy_and_modern_majors() {
        assert_eq!(parse_major("1.8.0_202"), Some(8));
        assert_eq!(parse_major("21.0.6+7"), Some(21));
        assert_eq!(parse_major("25-ea"), Some(25));
        assert_eq!(parse_major("unknown"), None);
    }
}
