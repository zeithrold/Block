//! Ephemeral Instance editing values and the owned window argument contract.
use super::{DataPolicy, Instance, InstanceAppearance, ServerPreferences};
use crate::recipe::RuntimeSelection;
use anyhow::ensure;
use std::path::PathBuf;

/// Editable values committed directly onto an Instance.
/// An empty game_directory is a create-only request for the managed root.
#[derive(Debug, Clone)]
pub struct InstanceConfiguration {
    pub name: String,
    pub target_id: String,
    pub runtime: RuntimeSelection,
    pub memory_mib: u32,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub default_account: Option<String>,
    pub appearance: InstanceAppearance,
    pub server: ServerPreferences,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub data_policy: DataPolicy,
    pub game_directory: PathBuf,
}

impl Default for InstanceConfiguration {
    fn default() -> Self {
        Self {
            name: String::new(),
            target_id: "vanilla".into(),
            runtime: RuntimeSelection::Managed,
            memory_mib: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            default_account: None,
            appearance: InstanceAppearance::default(),
            server: ServerPreferences::default(),
            width: 1280,
            height: 720,
            fullscreen: false,
            data_policy: DataPolicy::Isolated,
            game_directory: PathBuf::new(),
        }
    }
}

impl From<&Instance> for InstanceConfiguration {
    fn from(instance: &Instance) -> Self {
        Self {
            name: instance.name.clone(),
            target_id: instance.target_id.clone(),
            runtime: instance.runtime.clone(),
            memory_mib: instance.memory_mib,
            jvm_args: instance.jvm_args.clone(),
            game_args: instance.game_args.clone(),
            default_account: instance.default_account.clone(),
            appearance: instance.appearance,
            server: instance.server.clone(),
            width: instance.width,
            height: instance.height,
            fullscreen: instance.fullscreen,
            data_policy: instance.data_policy,
            game_directory: instance.game_directory.clone(),
        }
    }
}

impl InstanceConfiguration {
    pub(super) fn apply(self, instance: &mut Instance) {
        instance.name = self.name;
        instance.target_id = self.target_id;
        instance.runtime = self.runtime;
        instance.memory_mib = self.memory_mib;
        instance.jvm_args = self.jvm_args;
        instance.game_args = self.game_args;
        instance.default_account = self.default_account;
        instance.appearance = self.appearance;
        instance.server = self.server;
        instance.width = self.width;
        instance.height = self.height;
        instance.fullscreen = self.fullscreen;
        instance.data_policy = self.data_policy;
        instance.game_directory = self.game_directory;
    }
}

pub(super) fn validate_window(width: u32, height: u32) -> anyhow::Result<()> {
    ensure!(
        (320..=16384).contains(&width),
        "Window width must be between 320 and 16384 pixels"
    );
    ensure!(
        (240..=16384).contains(&height),
        "Window height must be between 240 and 16384 pixels"
    );
    Ok(())
}

impl Instance {
    /// Normalize owned window flags while preserving unrelated extra arguments.
    #[must_use]
    pub fn launch_game_args(&self) -> Vec<String> {
        let mut arguments = self.game_args.iter().peekable();
        let mut result = Vec::new();
        while let Some(argument) = arguments.next() {
            if matches!(argument.as_str(), "--width" | "--height") {
                if arguments
                    .peek()
                    .is_some_and(|value| !value.starts_with("--"))
                {
                    arguments.next();
                }
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
            self.width.to_string(),
            "--height".into(),
            self.height.to_string(),
        ]);
        if self.fullscreen {
            result.push("--fullscreen".into());
        }
        result
    }
}
