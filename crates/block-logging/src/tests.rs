use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, ensure};
use serde_json::Value;
use tempfile::TempDir;

use crate::{
    export::export_in,
    files::{Limits, SegmentWriter, segments},
    filter, subscriber,
    worker::{self, Failure},
};

fn limits(file_bytes: u64, retention_bytes: u64) -> Limits {
    Limits {
        file_bytes,
        retention_bytes,
    }
}

fn records(directory: &Path) -> anyhow::Result<Vec<Value>> {
    let mut result = Vec::new();
    for segment in segments(directory)? {
        let contents = fs::read_to_string(segment.path)?;
        for line in contents.lines() {
            result.push(serde_json::from_str(line)?);
        }
    }
    Ok(result)
}

fn wait_until(mut predicate: impl FnMut() -> bool) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !predicate() {
        ensure!(Instant::now() < deadline, "timed out waiting for logging");
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[test]
fn json_events_have_utc_metadata_fields_and_spans() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let failure = Failure::default();
    let (writer, guard) = worker::start(
        temp.path().to_owned(),
        "launcher",
        limits(4096, 16384),
        failure.clone(),
    )?;
    tracing::subscriber::with_default(subscriber(writer, filter(None)), || {
        let span = tracing::info_span!(target: "block_launcher::session", "session", attempt = 2);
        let _entered = span.enter();
        tracing::info!(target: "block_launcher::session", phase = "prepare", "started");
    });
    drop(guard);
    let events = records(temp.path())?;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event["level"], "INFO");
    assert_eq!(event["target"], "block_launcher::session");
    assert_eq!(event["fields"]["phase"], "prepare");
    assert_eq!(event["fields"]["message"], "started");
    assert_eq!(event["span"]["name"], "session");
    assert_eq!(event["span"]["attempt"], 2);
    assert_eq!(event["spans"][0]["name"], "session");
    let timestamp = event["timestamp"].as_str().context("missing timestamp")?;
    let timestamp =
        time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)?;
    assert_eq!(timestamp.offset(), time::UtcOffset::UTC);
    assert_eq!(failure.message(), None);
    Ok(())
}

fn emit_application_events() {
    tracing::info!(target: "block_launcher::test", "app_info");
    tracing::debug!(target: "block_launcher::test", "app_debug");
}

fn emit_dependency_events() {
    tracing::info!(target: "dependency::test", "dependency_info");
    tracing::warn!(target: "dependency::test", "dependency_warn");
    tracing::debug!(target: "dependency::test", "dependency_debug");
}

fn emit_similar_target_events() {
    tracing::info!(target: "blocking", "blocking_info");
    tracing::info!(target: "block_buffer", "block_buffer_info");
}

fn filtered_messages(specification: Option<&str>) -> anyhow::Result<Vec<String>> {
    let temp = TempDir::new()?;
    let (writer, guard) = worker::start(
        temp.path().to_owned(),
        "cli",
        limits(4096, 16384),
        Failure::default(),
    )?;
    tracing::subscriber::with_default(subscriber(writer, filter(specification)), || {
        emit_application_events();
        emit_dependency_events();
        emit_similar_target_events();
    });
    drop(guard);
    records(temp.path())?
        .into_iter()
        .map(|event| {
            event["fields"]["message"]
                .as_str()
                .map(str::to_owned)
                .context("missing message")
        })
        .collect()
}

#[test]
fn default_and_explicit_filters_have_expected_scope() -> anyhow::Result<()> {
    assert_eq!(filtered_messages(None)?, ["app_info", "dependency_warn"]);
    assert_eq!(
        filtered_messages(Some("block_launcher=debug"))?,
        ["app_info", "app_debug", "dependency_warn"]
    );
    assert_eq!(
        filtered_messages(Some("debug"))?,
        [
            "app_info",
            "app_debug",
            "dependency_info",
            "dependency_warn",
            "dependency_debug",
            "blocking_info",
            "block_buffer_info"
        ]
    );
    Ok(())
}

#[test]
fn shutdown_drains_queued_events() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let failure = Failure::default();
    let (writer, guard) = worker::start(
        temp.path().to_owned(),
        "cli",
        limits(1024 * 1024, 2 * 1024 * 1024),
        failure.clone(),
    )?;
    tracing::subscriber::with_default(subscriber(writer, filter(None)), || {
        for sequence in 0..1000 {
            tracing::info!(target: "block_cli::test", sequence, "queued");
        }
    });
    drop(guard);
    let events = records(temp.path())?;
    assert_eq!(events.len(), 1000);
    assert_eq!(events[0]["fields"]["sequence"], 0);
    assert_eq!(events[999]["fields"]["sequence"], 999);
    assert_eq!(failure.message(), None);
    Ok(())
}

#[test]
fn periodic_and_error_flush_publish_active_records() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let (writer, guard) = worker::start(
        temp.path().to_owned(),
        "launcher",
        limits(4096, 16384),
        Failure::default(),
    )?;
    tracing::subscriber::with_default(subscriber(writer, filter(None)), || {
        tracing::info!(target: "block_app::test", "periodic");
        wait_until(|| {
            records(temp.path()).is_ok_and(|events| {
                events
                    .iter()
                    .any(|event| event["fields"]["message"] == "periodic")
            })
        })?;
        let started = Instant::now();
        tracing::error!(target: "block_app::test", "prompt");
        wait_until(|| {
            records(temp.path()).is_ok_and(|events| {
                events
                    .iter()
                    .any(|event| event["fields"]["message"] == "prompt")
            })
        })?;
        ensure!(
            started.elapsed() < Duration::from_millis(800),
            "ERROR record waited for the periodic flush"
        );
        Ok::<_, anyhow::Error>(())
    })?;
    drop(guard);
    Ok(())
}

#[test]
fn rotation_keeps_complete_records_and_respects_file_limit() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let (writer, guard) = worker::start(
        temp.path().to_owned(),
        "supervisor",
        limits(300, 100_000),
        Failure::default(),
    )?;
    tracing::subscriber::with_default(subscriber(writer, filter(None)), || {
        for sequence in 0..10 {
            tracing::info!(target: "block_launcher::test", sequence, "rotate");
        }
    });
    drop(guard);
    let files = segments(temp.path())?;
    assert!(files.len() > 1);
    assert!(files.iter().all(|segment| segment.size <= 300));
    assert_eq!(records(temp.path())?.len(), 10);
    Ok(())
}

#[test]
fn shutdown_prunes_inactive_files_but_preserves_active_files() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let mut active = SegmentWriter::new(temp.path().to_owned(), "launcher", limits(1024, 150))?;
    active.write_record(format!("{{\"padding\":\"{}\"}}\n", "a".repeat(70)).as_bytes())?;
    active.flush()?;
    let active_path = segments(temp.path())?.remove(0).path;

    let (writer, guard) = worker::start(
        temp.path().to_owned(),
        "cli",
        limits(1024, 150),
        Failure::default(),
    )?;
    tracing::subscriber::with_default(subscriber(writer, filter(None)), || {
        tracing::info!(target: "block_cli::test", "close");
    });
    drop(guard);

    let remaining = segments(temp.path())?;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].path, active_path);
    assert!(active_path.exists());
    Ok(())
}

#[test]
fn initialization_failure_is_public_and_safe() {
    assert!(crate::init("not-a-role").is_err());
    assert_eq!(
        crate::failure().as_deref(),
        Some("Could not initialize local logging.")
    );
}

#[test]
fn public_init_installs_and_drains_the_subscriber_in_a_process() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", "tests::public_api_helper", "--nocapture"])
        .env("BLOCK_LOG_TEST_API_ROOT", temp.path())
        .env_remove("RUST_LOG")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "macos")]
    command.env("HOME", temp.path());
    #[cfg(target_os = "windows")]
    command.env("LOCALAPPDATA", temp.path());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    command.env("XDG_STATE_HOME", temp.path());
    assert!(command.status()?.success());

    #[cfg(target_os = "macos")]
    let directory = temp.path().join("Library/Logs/Block/launcher");
    #[cfg(target_os = "windows")]
    let directory = temp.path().join("Block/logs/launcher");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let directory = temp.path().join("block/logs/launcher");
    let events = records(&directory)?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["fields"]["message"], "public_init");
    Ok(())
}

#[test]
fn public_api_helper() -> anyhow::Result<()> {
    if std::env::var_os("BLOCK_LOG_TEST_API_ROOT").is_none() {
        return Ok(());
    }
    let guard = crate::init("cli")?;
    tracing::info!(target: "block_cli::test", "public_init");
    drop(guard);
    assert_eq!(crate::failure(), None);
    Ok(())
}

#[cfg(unix)]
#[test]
fn writer_failure_becomes_observable() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let directory = temp.path().join("launcher");
    let failure = Failure::default();
    let (writer, guard) = worker::start(
        directory.clone(),
        "launcher",
        limits(160, 2048),
        failure.clone(),
    )?;
    tracing::subscriber::with_default(subscriber(writer, filter(None)), || {
        tracing::error!(target: "block_app::test", "first");
        wait_until(|| records(&directory).is_ok_and(|events| !events.is_empty()))?;
        fs::rename(&directory, temp.path().join("moved"))?;
        fs::write(&directory, b"not a directory")?;
        tracing::error!(target: "block_app::test", "second");
        wait_until(|| failure.message().is_some())?;
        Ok::<_, anyhow::Error>(())
    })?;
    drop(guard);
    assert_eq!(
        failure.message(),
        Some("Local logging stopped because storage failed.")
    );
    Ok(())
}

#[test]
fn export_is_bounded_complete_owned_and_never_overwrites() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let directory = temp.path().join("logs");
    let mut writer = SegmentWriter::new(directory.clone(), "launcher", limits(4096, 16384))?;
    writer.write_record(b"{\"n\":1}\n{\"n\":2}\npartial")?;
    writer.flush()?;
    let original = segments(&directory)?.remove(0).path;
    fs::write(directory.join("unrelated.jsonl"), b"{\"other\":true}\n")?;
    fs::write(
        directory.join("launcher.invalid.jsonl"),
        b"{\"other\":true}\n",
    )?;
    fs::create_dir(directory.join("game"))?;
    fs::write(directory.join("game").join("latest.log"), b"game output")?;

    let archive = temp.path().join("logs.zip");
    export_in(&directory, &archive, 10)?;
    let mut zip = zip::ZipArchive::new(File::open(&archive)?)?;
    assert_eq!(zip.len(), 1);
    let mut entry = zip.by_index(0)?;
    assert_eq!(
        entry.name(),
        original
            .file_name()
            .context("no filename")?
            .to_str()
            .context("non-UTF8 filename")?
    );
    let mut data = String::new();
    entry.read_to_string(&mut data)?;
    assert_eq!(data, "{\"n\":1}\n");
    drop(entry);
    drop(zip);

    let before = fs::read(&original)?;
    assert!(export_in(&directory, &original, 4096).is_err());
    assert_eq!(fs::read(&original)?, before);
    assert!(export_in(&directory, &archive, 4096).is_err());
    Ok(())
}

struct ChildCleanup(Child);

impl Drop for ChildCleanup {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn retention_does_not_delete_another_process_active_file() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let directory = temp.path().join("logs");
    let ready = temp.path().join("ready");
    let child = Command::new(std::env::current_exe()?)
        .args(["--exact", "tests::hold_active_segment", "--nocapture"])
        .env("BLOCK_LOG_TEST_DIRECTORY", &directory)
        .env("BLOCK_LOG_TEST_READY", &ready)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut child = ChildCleanup(child);
    wait_until(|| ready.exists())?;
    let active = PathBuf::from(fs::read_to_string(&ready)?);
    assert!(active.exists());

    let mut other = SegmentWriter::new(directory.clone(), "cli", limits(1024, 100))?;
    other.write_record(format!("{{\"padding\":\"{}\"}}\n", "a".repeat(110)).as_bytes())?;
    other.flush()?;
    let inactive = segments(&directory)?
        .into_iter()
        .find(|segment| segment.path != active)
        .context("missing second segment")?
        .path;
    drop(other);

    let trigger = SegmentWriter::new(directory.clone(), "launcher", limits(1024, 100))?;
    assert!(
        active.exists(),
        "another process's active segment was deleted"
    );
    assert!(!inactive.exists(), "inactive segment was not pruned");
    drop(trigger);
    writeln!(
        child.0.stdin.as_mut().context("missing child stdin")?,
        "continue"
    )?;
    assert!(child.0.wait()?.success());
    assert!(fs::read_to_string(&active)?.contains("\"after\":true"));
    Ok(())
}

#[test]
fn hold_active_segment() -> anyhow::Result<()> {
    let Some(directory) = std::env::var_os("BLOCK_LOG_TEST_DIRECTORY") else {
        return Ok(());
    };
    let directory = PathBuf::from(directory);
    let ready = PathBuf::from(std::env::var_os("BLOCK_LOG_TEST_READY").context("missing ready")?);
    let mut writer = SegmentWriter::new(directory.clone(), "launcher", limits(1024, 100))?;
    writer.write_record(format!("{{\"padding\":\"{}\"}}\n", "b".repeat(110)).as_bytes())?;
    writer.flush()?;
    let path = segments(&directory)?.remove(0).path;
    fs::write(ready, path.to_string_lossy().as_bytes())?;
    let mut signal = String::new();
    std::io::stdin().read_line(&mut signal)?;
    writer.write_record(b"{\"after\":true}\n")?;
    writer.flush()?;
    Ok(())
}
