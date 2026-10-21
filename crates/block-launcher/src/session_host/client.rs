use std::{
    path::Path,
    process::Stdio,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, ensure};
use tokio::io::AsyncWriteExt as _;

use super::{RunRecord, RunState, now_ms, storage, wire};
use crate::{
    catalog::Catalog,
    crash::{CrashContext, CrashReport, ExitOutcome},
    session::{GameSession, SessionProvider as _},
    session_ipc,
};

fn starting_record(root: &Path, profile_id: &str) -> anyhow::Result<(Catalog, RunRecord, String)> {
    let catalog = Catalog::open(root)?;
    let snapshot = catalog.snapshot()?;
    let (instance, profile, target) = snapshot.selection(profile_id)?;
    let fingerprint = snapshot.launch_fingerprint(profile_id)?;
    let id = uuid::Uuid::new_v4().to_string();
    storage::create_directory(catalog.root(), &id)?;
    let record = RunRecord {
        schema_version: 1,
        context: CrashContext {
            run_id: id,
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            minecraft_version: instance.minecraft_version.clone(),
            loader: target.loader_version.clone(),
        },
        instance_id: instance.id.clone(),
        game_directory: profile.game_directory(instance)?,
        supervisor_pid: None,
        java_pid: None,
        state: RunState::Starting,
        outcome: None,
        status: "Starting session supervisor".into(),
        revision: 0,
        recent_output: Vec::new(),
        capture_warnings: Vec::new(),
        started_at_unix_ms: now_ms(),
        finished_at_unix_ms: None,
    };
    storage::write_record(catalog.root(), &record)?;
    Ok((catalog, record, fingerprint))
}

fn failed_start(root: &Path, mut record: RunRecord, message: &str) -> anyhow::Result<RunRecord> {
    let outcome = ExitOutcome::start_failed();
    let report = CrashReport::new(record.context.clone(), outcome.clone(), message, Vec::new());
    storage::write_report(root, &report)?;
    record.outcome = Some(outcome);
    record.state = RunState::Finished;
    record.status = "The session supervisor could not start · launch help is available".into();
    record.finished_at_unix_ms = Some(now_ms());
    record.revision = record.revision.saturating_add(1);
    storage::write_record(root, &record)?;
    Ok(record)
}

/// Hand an authorized game session to a detached owner using only inherited stdin.
/// # Errors
/// Returns validation, spawning or bootstrap failures. No refresh credential is transferred.
pub async fn spawn_session(
    root: &Path,
    executable: &Path,
    profile_id: &str,
    game: &GameSession,
    expires_at: SystemTime,
) -> anyhow::Result<RunRecord> {
    let mut bootstrap = wire::Bootstrap::new(profile_id, game, expires_at)?;
    bootstrap.session()?;
    let (catalog, record, fingerprint) = starting_record(root, profile_id)?;
    bootstrap.profile_fingerprint = fingerprint;
    let id = record.context.run_id.clone();
    // Detached owners have no terminal. Keep startup/finalization errors with
    // their run so a lost monitor does not discard the only useful diagnostic.
    let diagnostic = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(storage::report_directory(catalog.root(), &id)?.join("supervisor.log"))?;
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--supervise")
        .arg(&id)
        .arg("--data-dir")
        .arg(catalog.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(diagnostic))
        .env_remove("MICROSOFT_CLIENT_SECRET")
        .kill_on_drop(false);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000 | 0x0000_0200);
    let Ok(mut child) = command.spawn() else {
        return failed_start(
            catalog.root(),
            record,
            "The session supervisor could not be started. Check the Block application installation.",
        );
    };
    let mut input = child
        .stdin
        .take()
        .context("supervisor bootstrap pipe missing")?;
    tokio::time::timeout(Duration::from_secs(5), wire::send(&mut input, &bootstrap))
        .await
        .context("session bootstrap timed out")??;
    input.shutdown().await?;
    drop(input);
    // Do not tie this child or its cancellation to the lifetime of the launcher UI.
    for _ in 0..50 {
        let latest = storage::read_run(catalog.root(), &id)?;
        if latest.supervisor_pid.is_some() || !latest.is_active() {
            return Ok(latest);
        }
        if child.try_wait()?.is_some() {
            return failed_start(
                catalog.root(),
                latest,
                "The supervisor exited before accepting the launch request. No game was started.",
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!(
        "session supervisor did not acknowledge the launch request; inspect session history before retrying"
    )
}

async fn request(root: &Path, run_id: &str, stop: bool) -> anyhow::Result<RunRecord> {
    let record = storage::read_run(root, run_id)?;
    if !record.is_active() {
        return Ok(record);
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut stream = session_ipc::connect(run_id).await?;
        wire::send(
            &mut stream,
            &wire::Request {
                schema_version: 1,
                run_id: run_id.into(),
                stop,
            },
        )
        .await?;
        let response: RunRecord = wire::receive(&mut stream).await?;
        ensure!(
            response.schema_version == 1 && response.context.run_id == run_id,
            "session response identity mismatch"
        );
        Ok(response)
    })
    .await
    .context("session monitor is not responding")?
}

/// Read a live session. A lost monitor never implies that Java has stopped.
/// # Errors
/// Fails on invalid persisted metadata; unreachable monitors return an interrupted state.
pub async fn status(root: &Path, run_id: &str) -> anyhow::Result<RunRecord> {
    request(root, run_id, false).await.or_else(|_| {
            let mut record = storage::read_run(root, run_id)?;
            if record.is_active() {
                record.state = RunState::MonitoringInterrupted;
                record.status = "Session monitor unavailable; the game may still be running. Close it normally, then verify it has ended before manually recovering its directory reservation.".into();
            }
            Ok(record)
    })
}

/// Stop only the run accepted by its live private supervisor endpoint.
/// # Errors
/// An unreachable monitor is not replaced with an unsafe PID-based kill.
pub async fn stop(root: &Path, run_id: &str) -> anyhow::Result<RunRecord> {
    request(root, run_id, true).await.context(
        "Cannot contact the session monitor. Close the game normally before starting again.",
    )
}
