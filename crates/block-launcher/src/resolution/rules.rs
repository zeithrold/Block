use std::collections::BTreeMap;

use crate::{
    manifest::model::{OsName, Rule, RuleAction},
    platform::{Architecture, OperatingSystem, Target},
};

/// Evaluate an ordered Mojang rule list against an explicit game-process target.
///
/// # Errors
/// Fails for invalid version regexes or malformed numeric range components.
pub fn allowed(
    rules: Option<&[Rule]>,
    target: &Target,
    features: &BTreeMap<String, bool>,
) -> anyhow::Result<bool> {
    let Some(rules) = rules.filter(|rules| !rules.is_empty()) else {
        return Ok(true);
    };
    let mut result = false;
    for rule in rules {
        if matches(rule, target, features)? {
            result = rule.action == RuleAction::Allow;
        }
    }
    Ok(result)
}

fn matches(
    rule: &Rule,
    target: &Target,
    features: &BTreeMap<String, bool>,
) -> anyhow::Result<bool> {
    if let Some(expected) = &rule.features
        && expected
            .iter()
            .any(|(key, value)| features.get(key).copied().unwrap_or(false) != *value)
    {
        return Ok(false);
    }
    if let Some(os) = &rule.os {
        let name = match target.os {
            OperatingSystem::Macos => OsName::Osx,
            OperatingSystem::Windows => OsName::Windows,
            OperatingSystem::Linux => OsName::Linux,
        };
        if os.name.is_some_and(|expected| expected != name) {
            return Ok(false);
        }
        if let Some(arch) = &os.arch {
            let matches = match target.arch {
                Architecture::X86 => arch == "x86" || arch == "i386",
                Architecture::X86_64 => arch == "x86_64" || arch == "amd64",
                Architecture::Aarch64 => arch == "aarch64" || arch == "arm64",
            };
            if !matches {
                return Ok(false);
            }
        }
        if let Some(version) = &os.version
            && !regex::Regex::new(version)?.is_match(&target.os_version)
        {
            return Ok(false);
        }
        if let Some(range) = &os.version_range {
            let current = numeric_version(&target.os_version)?;
            if let Some(min) = &range.min
                && current < numeric_version(min)?
            {
                return Ok(false);
            }
            if let Some(max) = &range.max
                && current >= numeric_version(max)?
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn numeric_version(version: &str) -> anyhow::Result<Vec<u64>> {
    let mut parts = version
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()?;
    while parts.len() < 4 {
        parts.push(0);
    }
    Ok(parts)
}
