use super::windows_java_path;

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
