use super::{Instance, InstanceOwnership, LoaderKind, Snapshot, TargetSource};
use crate::recipe::RuntimeSelection;
use anyhow::ensure;
use std::path::Path;

impl Instance {
    /// # Errors
    /// Invalid names, Java paths, memory bounds or process arguments are rejected.
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_id(&self.id)?;
        validate_directory(&self.resource_directory)?;
        validate_directory(&self.game_directory)?;
        self.validate_targets()?;
        super::instance_settings::validate_window(self.width, self.height)?;
        self.server.validate()?;
        if self.server.connect_on_launch {
            ensure!(
                !self.game_args.iter().any(|argument| [
                    "--quickPlayMultiplayer",
                    "--quickPlaySingleplayer",
                    "--quickPlayRealms",
                    "--server",
                    "--port"
                ]
                .iter()
                .any(|flag| argument == flag || argument.starts_with(&format!("{flag}=")))),
                "Automatic connection manages server launch arguments; remove conflicting extra game arguments"
            );
        }
        validate_name(&self.name)?;
        ensure!(
            (crate::MIN_MEMORY_MIB..=crate::MAX_MEMORY_MIB).contains(&self.memory_mib),
            "Memory must be between 1 and 32 GB"
        );
        ensure!(
            self.jvm_args
                .iter()
                .chain(&self.game_args)
                .all(|a| !a.contains('\0')),
            "arguments cannot contain NUL bytes"
        );
        let reserved = [
            "--accessToken",
            "--uuid",
            "--username",
            "--userType",
            "--xuid",
            "--clientId",
            "--gameDir",
            "--assetsDir",
            "--assetIndex",
            "--version",
            "--versionType",
        ];
        ensure!(
            !self.game_args.iter().any(|arg| reserved
                .iter()
                .any(|flag| arg == flag || arg.starts_with(&format!("{flag}=")))),
            "identity and installation arguments are managed by Block"
        );
        ensure!(
            !self.jvm_args.iter().any(|arg| matches!(
                arg.as_str(),
                "-cp" | "-classpath" | "--class-path"
            ) || ["-Xmx", "-Xms", "--class-path="]
                .iter()
                .any(|prefix| arg.starts_with(prefix))),
            "classpath and memory are managed by Block"
        );
        if let RuntimeSelection::Local { java } = &self.runtime {
            ensure!(java.is_absolute(), "custom Java path must be absolute");
            ensure!(
                !java.to_string_lossy().contains('\0'),
                "Java path cannot contain NUL bytes"
            );
        }
        Ok(())
    }
}

impl Instance {
    fn validate_targets(&self) -> anyhow::Result<()> {
        ensure!(
            !self.minecraft_version.is_empty()
                && self.minecraft_version.len() <= 80
                && !self.minecraft_version.chars().any(char::is_control),
            "invalid Minecraft version"
        );
        let mut targets = std::collections::BTreeSet::new();
        for target in &self.targets {
            ensure!(
                !target.id.is_empty()
                    && target.id.len() <= 80
                    && target
                        .id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
                "invalid launch target ID"
            );
            ensure!(targets.insert(&target.id), "duplicate launch target ID");
            match (&self.ownership, &target.source) {
                (InstanceOwnership::Managed, TargetSource::Managed { manifest }) => {
                    if let Some(path) = manifest {
                        validate_directory(path)?;
                    }
                }
                (InstanceOwnership::Registered, TargetSource::Existing { version_id }) => {
                    validate_version_id(version_id)?
                }
                _ => anyhow::bail!("target source does not match instance ownership"),
            }
            if let Some(selected) = &target.optifine {
                selected.validate_for(
                    &self.minecraft_version,
                    &target.kind,
                    target.loader_version.as_deref(),
                )?;
            }
            ensure!(
                target.optifine_mod.is_none()
                    || target.optifine.is_some() && target.kind == LoaderKind::Forge,
                "Unexpected OptiFine mod output"
            );
        }
        ensure!(
            targets.contains(&self.target_id),
            "launch target does not belong to this instance"
        );
        if let Some(pack) = &self.pack {
            ensure!(
                self.ownership == InstanceOwnership::Managed,
                "registered instances cannot own pack data"
            );
            ensure!(
                targets.contains(&pack.target_id),
                "pack launch target is missing"
            );
        }
        Ok(())
    }
}

pub(super) fn validate_snapshot(state: &Snapshot) -> anyhow::Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    for instance in &state.instances {
        instance.validate()?;
        ensure!(ids.insert(&instance.id), "duplicate instance ID");
    }
    Ok(())
}

pub(super) fn validate_id(value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid catalog identifier"
    );
    Ok(())
}

/// Validate a display name used by an Instance.
///
/// # Errors
/// Rejects empty names, control characters and names over 160 UTF-8 bytes.
pub fn validate_name(value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 160 && !value.chars().any(char::is_control),
        "name must not be empty, exceed 160 UTF-8 bytes, or contain control characters"
    );
    Ok(())
}

/// Installation-root validation does not silently reinterpret relative UI input.
pub(super) fn validate_directory(path: &Path) -> anyhow::Result<()> {
    ensure!(
        !path.to_string_lossy().contains('\0'),
        "directory contains NUL bytes"
    );
    ensure!(
        path.is_absolute(),
        "installation directory must be absolute"
    );
    ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "installation directory contains parent traversal"
    );
    Ok(())
}

pub(super) fn validate_version_id(id: &str) -> anyhow::Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 256
            && !matches!(id, "latest-release" | "latest-snapshot")
            && Path::new(id)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            && !id.chars().any(|c| c == '/' || c == '\\' || c.is_control()),
        "invalid concrete version ID"
    );
    Ok(())
}
