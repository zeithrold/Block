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
use crate::manifest::{
    decode,
    model::{VersionIndex, VersionManifest, VersionType},
};

fn official_manifests() -> anyhow::Result<(VersionManifest, VersionManifest)> {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifests");
    let index: VersionIndex = decode(&std::fs::read(
        fixtures.join("official/version_manifest_v2.json"),
    )?)?;
    let entry = index
        .versions
        .iter()
        .find(|entry| entry.id == "1.20.6")
        .ok_or_else(|| anyhow::anyhow!("official fixture missing"))?;
    let modern = decode(&std::fs::read(
        fixtures
            .join("official/versions")
            .join(format!("{}.json", entry.sha1)),
    )?)?;
    let legacy = decode(&std::fs::read(fixtures.join("1.12.2.json"))?)?;
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
