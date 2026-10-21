//! Optional server preferences. Only a verified Quick Play launch connects;
//! saving an address performs no network request and creates no identity.
use anyhow::ensure;
use minecraft_manifest::model::{Argument, ArgumentValue, VersionManifest};
use serde::{Deserialize, Serialize};

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
        let address = self.address.trim();
        if address.is_empty() {
            ensure!(
                !self.connect_on_launch,
                "Enter a server address before enabling automatic connection"
            );
            return Ok(());
        }
        ensure!(
            address.len() <= 253
                && address.is_ascii()
                && !address
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || "/\\@?#".contains(c)),
            "Enter a server hostname or IP address, with an optional port"
        );
        let host = if let Some(tail) = address.strip_prefix('[') {
            let (host, suffix) = tail
                .split_once(']')
                .ok_or_else(|| anyhow::anyhow!("Close the IPv6 address with ]"))?;
            host.parse::<std::net::Ipv6Addr>()
                .map_err(|_| anyhow::anyhow!("Enter a valid IPv6 address"))?;
            if !suffix.is_empty() {
                validate_port(
                    suffix
                        .strip_prefix(':')
                        .ok_or_else(|| anyhow::anyhow!("Use a colon before the server port"))?,
                )?;
            }
            return Ok(());
        } else if address.matches(':').count() > 1 {
            address
                .parse::<std::net::Ipv6Addr>()
                .map_err(|_| anyhow::anyhow!("Use [IPv6]:port when specifying a port"))?;
            return Ok(());
        } else if let Some((host, port)) = address.split_once(':') {
            validate_port(port)?;
            host
        } else {
            address
        };
        ensure!(
            !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|label| !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')),
            "Enter a valid server hostname or IP address"
        );
        Ok(())
    }

    /// Compose only arguments the actual installed version declares support for.
    /// # Errors
    /// Rejects invalid preferences or a version without multiplayer Quick Play.
    pub fn launch_arguments(&self, manifest: &VersionManifest) -> anyhow::Result<Vec<String>> {
        self.validate()?;
        if !self.connect_on_launch {
            return Ok(Vec::new());
        }
        ensure!(
            supports_quick_play(manifest),
            "This installed version does not support multiplayer Quick Play; turn off automatic connection for this Profile"
        );
        Ok(vec![
            "--quickPlayMultiplayer".into(),
            self.address.trim().to_owned(),
        ])
    }
}
fn validate_port(port: &str) -> anyhow::Result<()> {
    ensure!(
        port.parse::<u16>().is_ok_and(|port| port > 0),
        "Server port must be between 1 and 65535"
    );
    Ok(())
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
    #[test]
    fn installed_official_manifest_controls_quick_play_and_saved_addresses_do_not_launch()
    -> anyhow::Result<()> {
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
        let mut server = ServerPreferences {
            address: "play.example.com:25566".into(),
            connect_on_launch: true,
        };
        assert_eq!(
            server.launch_arguments(&modern)?,
            ["--quickPlayMultiplayer", "play.example.com:25566"]
        );
        assert!(server.launch_arguments(&legacy).is_err());
        server.connect_on_launch = false;
        assert_eq!(server.launch_arguments(&modern)?, Vec::<String>::new());
        assert_eq!(server.launch_arguments(&legacy)?, Vec::<String>::new());
        let serialized = serde_json::to_vec(&server)?;
        let restored: ServerPreferences = serde_json::from_slice(&serialized)?;
        assert_eq!(restored, server);
        Ok(())
    }
}
