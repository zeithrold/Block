use std::process::Command;

use anyhow::{bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingSystem {
    Macos,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    X86,
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub os: OperatingSystem,
    pub arch: Architecture,
    pub os_version: String,
}

impl Target {
    /// Detect the host independently of recipe/Java process overrides.
    ///
    /// # Errors
    /// Fails if the host OS or architecture has no implementation.
    pub fn host() -> anyhow::Result<Self> {
        let os = match std::env::consts::OS {
            "macos" => OperatingSystem::Macos,
            "windows" => OperatingSystem::Windows,
            "linux" => OperatingSystem::Linux,
            other => bail!("unsupported OS: {other}"),
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" => Architecture::Aarch64,
            "x86_64" => Architecture::X86_64,
            "x86" => Architecture::X86,
            other => bail!("unsupported architecture: {other}"),
        };
        let mut command = match os {
            OperatingSystem::Macos => {
                let mut c = Command::new("sw_vers");
                c.arg("-productVersion");
                c
            }
            OperatingSystem::Linux => {
                let mut c = Command::new("uname");
                c.arg("-r");
                c
            }
            OperatingSystem::Windows => {
                let mut c = Command::new("powershell");
                c.args([
                    "-NoProfile",
                    "-Command",
                    "[System.Environment]::OSVersion.Version.ToString()",
                ]);
                c
            }
        };
        let version = command
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map_or_else(String::new, |output| {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            });
        Ok(Self {
            os,
            arch,
            os_version: version,
        })
    }

    #[must_use]
    pub const fn manifest_os(&self) -> &str {
        match self.os {
            OperatingSystem::Macos => "osx",
            OperatingSystem::Windows => "windows",
            OperatingSystem::Linux => "linux",
        }
    }

    #[must_use]
    pub const fn bits(&self) -> &str {
        match self.arch {
            Architecture::X86 => "32",
            Architecture::X86_64 | Architecture::Aarch64 => "64",
        }
    }

    #[must_use]
    pub const fn runtime_platform(&self) -> &str {
        match (self.os, self.arch) {
            (OperatingSystem::Macos, Architecture::Aarch64) => "mac-os-arm64",
            (OperatingSystem::Macos, _) => "mac-os",
            (OperatingSystem::Windows, Architecture::X86) => "windows-x86",
            (OperatingSystem::Windows, Architecture::X86_64) => "windows-x64",
            (OperatingSystem::Windows, Architecture::Aarch64) => "windows-arm64",
            (OperatingSystem::Linux, Architecture::X86) => "linux-i386",
            (OperatingSystem::Linux, Architecture::X86_64) => "linux",
            (OperatingSystem::Linux, Architecture::Aarch64) => "linux-arm64",
        }
    }

    /// Require a host-compatible execution target. Cross-target planning is allowed.
    ///
    /// # Errors
    /// Fails if executing this target would require an unconfigured emulator.
    pub fn validate_execution(&self) -> anyhow::Result<()> {
        let host = Self::host()?;
        ensure!(
            host.os == self.os && host.arch == self.arch,
            "this plan targets {:?}/{:?}; host is {:?}/{:?}. Configure a matching JVM and execution environment",
            self.os,
            self.arch,
            host.os,
            host.arch
        );
        Ok(())
    }
}
