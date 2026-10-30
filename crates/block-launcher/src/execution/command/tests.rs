use super::windows_java_path;

#[test]
fn external_properties_bind_only_at_runtime_and_remain_redacted() -> anyhow::Result<()> {
    use crate::{
        command::JavaCommand,
        plan::{CommandTemplate, ExecutionPlan, JavaExecutable},
        platform::{Architecture, OperatingSystem, Target},
        session::GameSession,
    };
    use base64::Engine as _;
    use sha2::{Digest as _, Sha256};
    use std::collections::BTreeMap;
    let properties = r#"{"preferredLanguage":["zh_CN"],"private":["SYNTHETIC_PROPERTY"]}"#;
    let session = GameSession {
        username: "山与海".into(),
        uuid: "123456781234123412341234567890ab".into(),
        access_token: "EXTERNAL_TOKEN".into(),
        user_type: "mojang".into(),
        xuid: String::new(),
        client_id: String::new(),
        user_properties: properties.into(),
        authlib: None,
    };
    let plan = ExecutionPlan {
        schema_version: 1,
        instance: "fixture".into(),
        version: "fixture".into(),
        manifest_sha1: String::new(),
        target: Target {
            os: OperatingSystem::Linux,
            arch: Architecture::X86_64,
            os_version: String::new(),
        },
        resources: Vec::new(),
        actions: Vec::new(),
        compatibility: None,
        runtime_substitution: None,
        command: CommandTemplate {
            java: JavaExecutable::Managed {
                path: "runtime/bin/java".into(),
            },
            java_major: 8,
            main_class: "fixture.Main".into(),
            jvm_args: Vec::new(),
            game_args: vec![
                "${auth_player_name}".into(),
                "${user_properties}".into(),
                "${auth_session}".into(),
                "${user_type}".into(),
            ],
            classpath: Vec::new(),
            variables: BTreeMap::from([("user_properties".into(), "{}".into())]),
            path_variables: BTreeMap::new(),
            cwd: "game".into(),
        },
    };
    let root = tempfile::Builder::new()
        .prefix("block agent path with spaces ")
        .tempdir()?;
    let command = JavaCommand::bind(&plan, root.path(), "run", &session)?;
    assert!(command.arguments.iter().any(|arg| arg.value == properties));
    assert!(
        command
            .arguments
            .iter()
            .any(|arg| arg.value == "EXTERNAL_TOKEN" && arg.secret)
    );
    assert!(command.arguments.iter().any(|arg| arg.value == "山与海"));
    assert!(command.preview().arguments.contains(&"mojang".into()));
    assert!(!serde_json::to_string(&plan)?.contains("SYNTHETIC_PROPERTY"));
    assert!(!serde_json::to_string(&command.preview())?.contains("SYNTHETIC_PROPERTY"));
    assert!(!serde_json::to_string(&command.preview())?.contains("EXTERNAL_TOKEN"));
    assert!(!format!("{command:?} {session:?}").contains("SYNTHETIC_PROPERTY"));
    assert_eq!(
        command.redact(&format!("properties={properties}")),
        "properties=<redacted>"
    );
    let jar = root.path().join("verified agent.jar");
    std::fs::write(&jar, b"verified fixture agent")?;
    let mut injected = session.clone();
    injected.authlib = Some(crate::session::AuthlibLaunch {
        api_url: "https://provider.example.test/api/".into(),
        metadata_json: r#"{"skinDomains":["skins.example.test"]}"#.into(),
        agent: jar.clone(),
        sha256: format!("{:x}", Sha256::digest(b"verified fixture agent")),
    });
    let with_agent = JavaCommand::bind(&plan, root.path(), "run", &injected)?;
    assert_eq!(
        with_agent.arguments[0].value,
        format!(
            "-javaagent:{}=https://provider.example.test/api/",
            jar.display()
        )
    );
    let prefetched = with_agent.arguments[1]
        .value
        .strip_prefix("-Dauthlibinjector.yggdrasil.prefetched=")
        .ok_or_else(|| anyhow::anyhow!("Missing Authlib prefetch argument"))?;
    assert_eq!(
        base64::engine::general_purpose::STANDARD.decode(prefetched)?,
        br#"{"skinDomains":["skins.example.test"]}"#
    );
    assert_eq!(with_agent.arguments[2].value, "fixture.Main");
    assert!(!serde_json::to_string(&plan)?.contains("provider.example.test"));
    std::fs::write(jar, b"tampered fixture agent")?;
    assert!(JavaCommand::bind(&plan, root.path(), "run", &injected).is_err());
    for invalid in ["null", r#"{"x":"not an array"}"#, r#"{"x":[12]}"#] {
        let mut invalid_session = session.clone();
        invalid_session.user_properties = invalid.into();
        assert!(JavaCommand::bind(&plan, root.path(), "run", &invalid_session).is_err());
    }
    let mut opaque = session.clone();
    opaque.access_token = "0".into();
    let opaque = JavaCommand::bind(&plan, root.path(), "run", &opaque)?;
    assert_eq!(opaque.redact("token=0"), "token=<redacted>");
    let mut offline = session;
    offline.user_type = "msa".into();
    assert!(offline.validate().is_err());
    Ok(())
}

#[test]
fn windows_java_paths_preserve_drive_unc_and_unicode_names() {
    for (input, expected) in [
        (
            r"\\?\C:\Users\Player\client.jar",
            r"C:\Users\Player\client.jar",
        ),
        (
            r"\\?\D:\Games with spaces\游戏\java.exe",
            r"D:\Games with spaces\游戏\java.exe",
        ),
        (
            r"\\?\UNC\server\share\client.jar",
            r"\\server\share\client.jar",
        ),
        (r"C:\Games\client.jar", r"C:\Games\client.jar"),
        (r"\\server\share\client.jar", r"\\server\share\client.jar"),
        ("/home/player/game", "/home/player/game"),
        (r"\\.\device", r"\\.\device"),
    ] {
        assert_eq!(windows_java_path(input), expected);
    }
}

#[cfg(windows)]
#[test]
fn binding_normalizes_every_java_path_but_preserves_canonical_storage() -> anyhow::Result<()> {
    use std::collections::BTreeMap;

    use crate::{
        command::JavaCommand,
        plan::{CommandTemplate, ExecutionPlan, JavaExecutable},
        platform::{Architecture, OperatingSystem, Target},
        session::OfflineSession,
    };

    let temporary = tempfile::tempdir()?;
    let root = temporary.path().canonicalize()?;
    assert!(root.to_string_lossy().starts_with(r"\\?\"));
    let plan = ExecutionPlan {
        schema_version: 1,
        instance: "windows-test".into(),
        version: "test".into(),
        manifest_sha1: String::new(),
        target: Target {
            os: OperatingSystem::Windows,
            arch: Architecture::X86_64,
            os_version: "10.0.19044".into(),
        },
        resources: Vec::new(),
        actions: Vec::new(),
        compatibility: None,
        runtime_substitution: None,
        command: CommandTemplate {
            java: JavaExecutable::Managed {
                path: "runtime/bin/java.exe".into(),
            },
            java_major: 21,
            main_class: "test.Main".into(),
            jvm_args: vec![
                "-cp".into(),
                "${classpath}".into(),
                "-Djava.library.path=${natives_directory}".into(),
            ],
            game_args: vec!["--gameDir".into(), "${game_directory}".into()],
            classpath: vec!["libraries/one.jar".into(), "clients/client.jar".into()],
            variables: BTreeMap::new(),
            path_variables: BTreeMap::from([
                ("natives_directory".into(), "runs/${run_id}/natives".into()),
                ("game_directory".into(), "instances/game with spaces".into()),
            ]),
            cwd: "instances/game with spaces".into(),
        },
    };
    let command = JavaCommand::bind(
        &plan,
        &root,
        "run-1",
        &OfflineSession::Offline {
            username: "Player".into(),
        },
    )?;
    let preview = command.preview();
    let regular = windows_java_path(
        root.to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 temporary path"))?,
    );
    assert_eq!(
        preview.program,
        std::path::Path::new(&regular).join("runtime/bin/java.exe")
    );
    assert_eq!(
        preview.cwd,
        std::path::Path::new(&regular).join("instances/game with spaces")
    );
    assert!(
        preview
            .arguments
            .iter()
            .all(|argument| !argument.contains(r"\\?\"))
    );
    assert!(preview.arguments[1].contains(';'));
    assert_eq!(
        preview.arguments[2],
        format!(
            "-Djava.library.path={}",
            std::path::Path::new(&regular)
                .join("runs")
                .join("run-1")
                .join("natives")
                .display()
        )
    );
    assert!(
        preview
            .arguments
            .last()
            .is_some_and(|argument| std::path::Path::new(argument) == preview.cwd)
    );
    assert!(root.to_string_lossy().starts_with(r"\\?\"));
    Ok(())
}
