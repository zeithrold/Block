//! Exercise the real executable without opening a desktop or accessing accounts.

use std::{path::Path, process::Command};

fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_block"));
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOCALAPPDATA", home.join("local"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env_remove("RUST_LOG");
    command
}

fn launcher_logs(home: &Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Logs/Block/launcher")
    } else if cfg!(windows) {
        home.join("local/Block/logs/launcher")
    } else {
        home.join("state/block/logs/launcher")
    }
}

#[test]
fn informational_command_does_not_create_logs() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let output = command(home.path()).arg("--build-info").output()?;
    assert!(output.status.success());
    let _: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(!launcher_logs(home.path()).exists());
    Ok(())
}

#[test]
fn failed_headless_operation_flushes_safe_structured_events() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let private_path = home.path().join("private-path-sentinel");
    let output = command(home.path())
        .args([
            "--supervise",
            "00000000-0000-4000-8000-000000000001",
            "--data-dir",
        ])
        .arg(&private_path)
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let mut contents = String::new();
    for entry in std::fs::read_dir(launcher_logs(home.path()))? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            contents.push_str(&std::fs::read_to_string(path)?);
        }
    }
    assert!(!contents.is_empty());
    assert!(contents.ends_with('\n'));
    let events = contents
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert!(contents.contains("application_started"));
    assert!(contents.contains("application_failed"));
    assert!(contents.contains("application_stopped"));
    let lifecycle = events
        .iter()
        .filter(|event| {
            matches!(
                event["fields"]["event"].as_str(),
                Some("application_started" | "application_failed" | "application_stopped")
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(lifecycle.len(), 3);
    assert_eq!(lifecycle[0]["fields"]["event"], "application_started");
    assert_eq!(lifecycle[1]["fields"]["event"], "application_failed");
    assert_eq!(lifecycle[1]["fields"]["error_code"], "operation_failed");
    assert_eq!(lifecycle[2]["fields"]["event"], "application_stopped");
    assert_eq!(lifecycle[2]["fields"]["failed"], true);
    assert!(!contents.contains("private-path-sentinel"));
    Ok(())
}

#[test]
fn logging_initialization_failure_does_not_replace_the_requested_operation() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let logs = launcher_logs(home.path());
    std::fs::create_dir_all(
        logs.parent()
            .ok_or_else(|| anyhow::anyhow!("missing log parent"))?,
    )?;
    std::fs::write(&logs, b"not a directory")?;
    let output = command(home.path())
        .args([
            "--supervise",
            "00000000-0000-4000-8000-000000000001",
            "--data-dir",
        ])
        .arg(home.path())
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("could not initialize local logging"));
    assert!(stderr.contains("Block could not complete this operation:"));
    assert_eq!(std::fs::read(logs)?, b"not a directory");
    Ok(())
}
