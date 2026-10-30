use std::process::Command;

#[test]
fn failed_plan_keeps_stdout_clean_and_flushes_operation_logs() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_block-launch"))
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.path().join("local"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env_remove("RUST_LOG")
        .arg("--data-dir")
        .arg(home.path().join("data"))
        .args(["--cache-only", "plan", "--recipe"])
        .arg(home.path().join("private-recipe-sentinel.json"))
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let directory = if cfg!(target_os = "macos") {
        home.path().join("Library/Logs/Block/launcher")
    } else if cfg!(windows) {
        home.path().join("local/Block/logs/launcher")
    } else {
        home.path().join("state/block/logs/launcher")
    };
    let mut contents = String::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            contents.push_str(&std::fs::read_to_string(path)?);
        }
    }
    assert!(!contents.is_empty());
    for line in contents.lines() {
        let _: serde_json::Value = serde_json::from_str(line)?;
    }
    assert!(contents.contains("operation_started"));
    assert!(contents.contains("operation_finished"));
    assert!(contents.contains("application_stopped"));
    assert!(!contents.contains("private-recipe-sentinel"));
    Ok(())
}
