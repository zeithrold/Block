//! Instance defaults and explicit Profile inheritance, preserving legacy launches.

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};

use super::{Instance, Profile};
use crate::recipe::RuntimeSelection;

/// Persisted defaults belong to an Instance; its data directory never moves here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstanceSettings {
    pub runtime: RuntimeSelection,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
}

impl Default for InstanceSettings {
    fn default() -> Self {
        Self {
            runtime: RuntimeSelection::Managed,
            width: 1280,
            height: 720,
            fullscreen: false,
        }
    }
}

impl InstanceSettings {
    /// # Errors
    /// Rejects invalid window bounds and relative or NUL-containing Java paths.
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            (320..=16384).contains(&self.width),
            "Window width must be between 320 and 16384 pixels"
        );
        ensure!(
            (240..=16384).contains(&self.height),
            "Window height must be between 240 and 16384 pixels"
        );
        if let RuntimeSelection::Local { java } = &self.runtime {
            ensure!(java.is_absolute(), "Custom Java path must be absolute");
            ensure!(
                !java.to_string_lossy().contains('\0'),
                "Java path cannot contain NUL bytes"
            );
        }
        Ok(())
    }

    /// Seed a new Instance from the actual configured initial Profile.
    /// # Errors
    /// Invalid owned window arguments are rejected before creating directories.
    pub fn from_profile(profile: &Profile) -> anyhow::Result<Self> {
        let mut settings = Self {
            runtime: profile.runtime.clone(),
            ..Self::default()
        };
        let mut arguments = profile.game_args.iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--width" => {
                    settings.width = arguments
                        .next()
                        .context("Window width argument is missing")?
                        .parse()?;
                }
                "--height" => {
                    settings.height = arguments
                        .next()
                        .context("Window height argument is missing")?
                        .parse()?;
                }
                "--fullscreen" => settings.fullscreen = true,
                _ if argument.starts_with("--width=") => settings.width = argument[8..].parse()?,
                _ if argument.starts_with("--height=") => {
                    settings.height = argument[9..].parse()?;
                }
                _ if argument.starts_with("--fullscreen=") => {
                    settings.fullscreen = argument[13..].parse()?;
                }
                _ => {}
            }
        }
        settings.validate()?;
        Ok(settings)
    }
}

/// Explicit switches avoid changing the meaning of a legacy managed Profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileInheritance {
    pub runtime: bool,
    pub window: bool,
}

impl ProfileInheritance {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "Serde skip_serializing_if requires a borrowed value."
    )]
    pub(super) const fn is_legacy(&self) -> bool {
        !self.runtime && !self.window
    }
}

/// Validated metadata submitted by the Instance editor in one catalog commit.
#[derive(Debug, Clone)]
pub struct InstanceConfiguration {
    pub name: String,
    pub settings: InstanceSettings,
}

impl Profile {
    /// A Profile's explicit custom Java always wins, including migrated catalogs.
    #[must_use]
    pub fn effective_runtime(&self, instance: &Instance) -> RuntimeSelection {
        if self.inheritance.runtime
            && matches!(self.runtime, RuntimeSelection::Managed)
            && let Some(settings) = &instance.settings
        {
            return settings.runtime.clone();
        }
        self.runtime.clone()
    }

    /// Resolve only the owned window flags; preserve every unrelated argument.
    #[must_use]
    pub fn effective_game_args(&self, instance: &Instance) -> Vec<String> {
        let Some(settings) = instance
            .settings
            .as_ref()
            .filter(|_| self.inheritance.window)
        else {
            return self.game_args.clone();
        };
        let mut arguments = self.game_args.iter();
        let mut result = Vec::new();
        while let Some(argument) = arguments.next() {
            if matches!(argument.as_str(), "--width" | "--height") {
                arguments.next();
            } else if argument != "--fullscreen"
                && !["--width=", "--height=", "--fullscreen="]
                    .iter()
                    .any(|prefix| argument.starts_with(prefix))
            {
                result.push(argument.clone());
            }
        }
        result.extend([
            "--width".into(),
            settings.width.to_string(),
            "--height".into(),
            settings.height.to_string(),
        ]);
        if settings.fullscreen {
            result.push("--fullscreen".into());
        }
        result
    }
}
