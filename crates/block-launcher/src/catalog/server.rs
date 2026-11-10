//! Optional server preferences. Only a verified automatic connection launches;
//! saving an address performs no network request and creates no identity.
use anyhow::ensure;
use serde::{Deserialize, Serialize};

use crate::manifest::model::{Argument, ArgumentValue, VersionManifest, VersionType};

/// How an installed version's client accepts an automatic server connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomaticConnection {
    /// The official manifest declares `--quickPlayMultiplayer` (1.20 and newer).
    QuickPlay,
    /// The client accepts the legacy `--server`/`--port` game arguments.
    Legacy,
    /// Official Alpha and Beta builds predate both forms, so nothing is offered.
    Unsupported,
}

/// Which form the installed version really accepts.
///
/// Quick Play is keyed on the official version data: the manifest must declare
/// `--quickPlayMultiplayer`. Release and snapshot clients that predate that
/// declaration accept the legacy `--server`/`--port` game arguments; the
/// `old_alpha`/`old_beta` documents describe the pre-release client, which has
/// no launcher server option, so automatic connection stays unavailable there.
/// Real join tests exercised the legacy form on 1.12.2 and Quick Play on 1.20.6.
#[must_use]
pub fn automatic_connection(manifest: &VersionManifest) -> AutomaticConnection {
    if supports_quick_play(manifest) {
        AutomaticConnection::QuickPlay
    } else if matches!(manifest.kind, VersionType::OldAlpha | VersionType::OldBeta) {
        AutomaticConnection::Unsupported
    } else {
        AutomaticConnection::Legacy
    }
}

/// A validated address split into the host and port a client can launch with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAddress {
    /// Hostname or IP literal without brackets.
    pub host: String,
    /// Explicit or defaulted multiplayer port.
    pub port: u16,
}

impl ServerAddress {
    /// # Errors
    /// Rejects invalid host/port syntax, whitespace, URLs and unterminated IPv6.
    pub fn parse(address: &str) -> anyhow::Result<Option<Self>> {
        let address = address.trim();
        if address.is_empty() {
            return Ok(None);
        }
        ensure!(
            address.len() <= 253
                && address.is_ascii()
                && !address
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || "/\\@?#".contains(c)),
            "Enter a server hostname or IP address, with an optional port"
        );
        if let Some(tail) = address.strip_prefix('[') {
            let (host, suffix) = tail
                .split_once(']')
                .ok_or_else(|| anyhow::anyhow!("Close the IPv6 address with ]"))?;
            host.parse::<std::net::Ipv6Addr>()
                .map_err(|_| anyhow::anyhow!("Enter a valid IPv6 address"))?;
            let port = if suffix.is_empty() {
                25565
            } else {
                parse_port(
                    suffix
                        .strip_prefix(':')
                        .ok_or_else(|| anyhow::anyhow!("Use a colon before the server port"))?,
                )?
            };
            return Ok(Some(Self {
                host: host.to_owned(),
                port,
            }));
        }
        if address.matches(':').count() > 1 {
            address
                .parse::<std::net::Ipv6Addr>()
                .map_err(|_| anyhow::anyhow!("Use [IPv6]:port when specifying a port"))?;
            return Ok(Some(Self {
                host: address.to_owned(),
                port: 25565,
            }));
        }
        let (host, port) = match address.split_once(':') {
            Some((host, port)) => (host, parse_port(port)?),
            None => (address, 25565),
        };
        ensure!(
            valid_hostname(host),
            "Enter a valid server hostname or IP address"
        );
        Ok(Some(Self {
            host: host.to_owned(),
            port,
        }))
    }
}

fn valid_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

/// A remembered multiplayer address and an explicit connect-on-launch choice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerPreferences {
    /// ASCII hostname or IP literal, with an optional port.
    pub address: String,
    /// Connect using the installed version's real Quick Play support.
    pub connect_on_launch: bool,
}

impl ServerPreferences {
    /// # Errors
    /// Rejects invalid host/port syntax, whitespace, URLs and an empty enabled address.
    pub fn validate(&self) -> anyhow::Result<()> {
        if ServerAddress::parse(&self.address)?.is_none() {
            ensure!(
                !self.connect_on_launch,
                "Enter a server address before enabling automatic connection"
            );
        }
        Ok(())
    }

    /// Compose only the arguments the actual installed version accepts.
    /// # Errors
    /// Rejects invalid preferences or a version without any automatic connection.
    pub fn launch_arguments(&self, manifest: &VersionManifest) -> anyhow::Result<Vec<String>> {
        self.validate()?;
        if !self.connect_on_launch {
            return Ok(Vec::new());
        }
        let Some(address) = ServerAddress::parse(&self.address)? else {
            anyhow::bail!("Enter a server address before enabling automatic connection");
        };
        match automatic_connection(manifest) {
            AutomaticConnection::QuickPlay => Ok(vec![
                "--quickPlayMultiplayer".into(),
                self.address.trim().to_owned(),
            ]),
            AutomaticConnection::Legacy => Ok(vec![
                "--server".into(),
                address.host,
                "--port".into(),
                address.port.to_string(),
            ]),
            AutomaticConnection::Unsupported => anyhow::bail!(
                "This version's client does not accept an automatic server connection; turn off automatic connection for this Instance"
            ),
        }
    }
}
fn parse_port(port: &str) -> anyhow::Result<u16> {
    port.parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| anyhow::anyhow!("Server port must be between 1 and 65535"))
}
/// Read the real game argument contract rather than guessing a snapshot version.
#[must_use]
pub fn supports_quick_play(manifest: &VersionManifest) -> bool {
    manifest.arguments.as_ref().is_some_and(|arguments| {
        arguments.game.iter().any(|argument| match argument {
            Argument::Literal(value) => value == "--quickPlayMultiplayer",
            Argument::Conditional(argument) => match &argument.value {
                ArgumentValue::Single(value) => value == "--quickPlayMultiplayer",
                ArgumentValue::Multiple(values) => {
                    values.iter().any(|value| value == "--quickPlayMultiplayer")
                }
            },
        })
    })
}

#[cfg(test)]
mod tests;
