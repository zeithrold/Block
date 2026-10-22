//! Optional server preferences. Only a verified automatic connection launches;
//! saving an address performs no network request and creates no identity.
use anyhow::ensure;
use minecraft_manifest::model::{Argument, ArgumentValue, VersionManifest, VersionType};
use serde::{Deserialize, Serialize};

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
                "This version's client does not accept an automatic server connection; turn off automatic connection for this Profile"
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
mod tests {
    use super::ServerPreferences;
    #[test]
    fn server_preferences_validate_without_connecting_or_accepting_urls() {
        for address in [
            "play.example.com",
            "localhost:25565",
            "127.0.0.1",
            "[::1]:25566",
            "2001:db8::1",
        ] {
            assert!(
                ServerPreferences {
                    address: address.into(),
                    connect_on_launch: true
                }
                .validate()
                .is_ok(),
                "{address}"
            );
        }
        for address in [
            "",
            "https://example.com",
            "example.com/path",
            "host:0",
            "host:65536",
            "user@host",
            "host other",
            "[::1",
        ] {
            assert!(
                ServerPreferences {
                    address: address.into(),
                    connect_on_launch: true
                }
                .validate()
                .is_err(),
                "{address}"
            );
        }
        assert!(ServerPreferences::default().validate().is_ok());
    }
    use super::{AutomaticConnection, ServerAddress, automatic_connection};
    use minecraft_manifest::model::VersionType;

    fn official_manifests() -> anyhow::Result<(
        minecraft_manifest::model::VersionManifest,
        minecraft_manifest::model::VersionManifest,
    )> {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../minecraft-manifest/tests/fixtures");
        let index: minecraft_manifest::model::VersionIndex = minecraft_manifest::decode(
            &std::fs::read(fixtures.join("official/version_manifest_v2.json"))?,
        )?;
        let entry = index
            .versions
            .iter()
            .find(|entry| entry.id == "1.20.6")
            .ok_or_else(|| anyhow::anyhow!("official fixture missing"))?;
        let modern = minecraft_manifest::decode(&std::fs::read(
            fixtures
                .join("official/versions")
                .join(format!("{}.json", entry.sha1)),
        )?)?;
        let legacy = minecraft_manifest::decode(&std::fs::read(fixtures.join("1.12.2.json"))?)?;
        Ok((modern, legacy))
    }

    #[test]
    fn official_manifest_selects_quick_play_or_legacy_server_arguments() -> anyhow::Result<()> {
        let (modern, legacy) = official_manifests()?;
        assert_eq!(
            automatic_connection(&modern),
            AutomaticConnection::QuickPlay
        );
        assert_eq!(automatic_connection(&legacy), AutomaticConnection::Legacy);
        // Pre-release documents keep automatic connection unavailable.
        let mut beta = legacy.clone();
        beta.kind = VersionType::OldBeta;
        assert_eq!(
            automatic_connection(&beta),
            AutomaticConnection::Unsupported
        );
        let mut alpha = legacy.clone();
        alpha.kind = VersionType::OldAlpha;
        assert_eq!(
            automatic_connection(&alpha),
            AutomaticConnection::Unsupported
        );

        let mut server = ServerPreferences {
            address: "play.example.com:25566".into(),
            connect_on_launch: true,
        };
        assert_eq!(
            server.launch_arguments(&modern)?,
            ["--quickPlayMultiplayer", "play.example.com:25566"]
        );
        assert_eq!(
            server.launch_arguments(&legacy)?,
            ["--server", "play.example.com", "--port", "25566"]
        );
        assert!(server.launch_arguments(&beta).is_err());
        server.connect_on_launch = false;
        assert_eq!(server.launch_arguments(&modern)?, Vec::<String>::new());
        assert_eq!(server.launch_arguments(&legacy)?, Vec::<String>::new());
        let serialized = serde_json::to_vec(&server)?;
        let restored: ServerPreferences = serde_json::from_slice(&serialized)?;
        assert_eq!(restored, server);
        Ok(())
    }

    #[test]
    fn legacy_arguments_default_the_port_and_keep_ipv6_literals() -> anyhow::Result<()> {
        let (modern, legacy) = official_manifests()?;
        for (address, expected) in [
            ("play.example.com", ["play.example.com", "25565"]),
            ("127.0.0.1:25570", ["127.0.0.1", "25570"]),
            ("[::1]:25566", ["::1", "25566"]),
            ("[2001:db8::2]", ["2001:db8::2", "25565"]),
            ("2001:db8::1", ["2001:db8::1", "25565"]),
        ] {
            let server = ServerPreferences {
                address: address.into(),
                connect_on_launch: true,
            };
            assert_eq!(
                server.launch_arguments(&legacy)?,
                [
                    "--server".to_owned(),
                    expected[0].to_owned(),
                    "--port".to_owned(),
                    expected[1].to_owned()
                ],
                "{address}"
            );
            // Quick Play keeps the user's written address verbatim.
            assert_eq!(
                server.launch_arguments(&modern)?,
                ["--quickPlayMultiplayer", address.trim()]
            );
        }
        for address in ["host:0", "host:65536", "host:", "[::1]:0", "a b:c"] {
            let server = ServerPreferences {
                address: address.into(),
                connect_on_launch: true,
            };
            assert!(server.launch_arguments(&legacy).is_err(), "{address}");
        }
        assert_eq!(
            ServerAddress::parse(" play.example.com:25565 ")?
                .map(|address| (address.host, address.port)),
            Some(("play.example.com".to_owned(), 25565))
        );
        assert!(ServerAddress::parse("host:0").is_err());
        assert!(ServerAddress::parse("")?.is_none());
        Ok(())
    }
}
