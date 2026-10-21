use std::{
    fs::OpenOptions,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context as _, ensure};
use interprocess::local_socket::{tokio::Listener, traits::tokio::Listener as _};
use tokio_util::sync::CancellationToken;

use super::{RunRecord, RunState, now_ms, storage, wire};
use crate::{
    catalog::GameDirectoryGuard,
    crash::{self, CrashReport, ExitKind, ExitOutcome},
    executor::{ExecutionEvent, ExecutionSink},
    launch::LaunchExecutor,
    library::Library,
    session_ipc,
};

type Shared = Arc<Mutex<RunRecord>>;

fn snapshot(state: &Shared) -> anyhow::Result<RunRecord> {
    state
        .lock()
        .map(|record| record.clone())
        .map_err(|_| anyhow::anyhow!("session state unavailable"))
}

fn warning(state: &Shared, message: &str) {
    if let Ok(mut record) = state.lock()
        && !record.capture_warnings.iter().any(|value| value == message)
    {
        if record.capture_warnings.len() == 8 {
            record.capture_warnings.remove(0);
        }
        record
            .capture_warnings
            .push(message.chars().take(512).collect());
        record.revision = record.revision.saturating_add(1);
    }
}

fn events(state: &Shared) -> ExecutionSink {
    let shared = state.clone();
    Arc::new(move |event| {
        if let Ok(mut record) = shared.lock() {
            match event {
                ExecutionEvent::Stage { name } => {
                    record.state = RunState::Preparing;
                    record.status = name;
                }
                ExecutionEvent::Started { pid } => {
                    record.java_pid = Some(pid);
                    record.state = RunState::Running;
                    record.status = format!("Running · Minecraft PID {pid}");
                }
                ExecutionEvent::Terminated { outcome } => record.outcome = Some(outcome),
                ExecutionEvent::CaptureFailed { message } => {
                    if record.capture_warnings.len() < 8 {
                        record.capture_warnings.push(message);
                    }
                }
                ExecutionEvent::Log { line, .. } => {
                    if record.recent_output.len() == 48 {
                        record.recent_output.remove(0);
                    }
                    record.recent_output.push(line.chars().take(384).collect());
                }
                ExecutionEvent::Exited { .. } => {}
            }
            record.revision = record.revision.saturating_add(1);
        }
    })
}

async fn serve(
    listener: Listener,
    run_id: String,
    state: Shared,
    stop: CancellationToken,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let permits = Arc::new(tokio::sync::Semaphore::new(16));
    loop {
        let mut connection = tokio::select! {
            result = listener.accept() => result?,
            () = shutdown.cancelled() => break,
        };
        session_ipc::validate_peer(&run_id, &connection)?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let id = run_id.clone();
        let state = state.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let request: wire::Request = wire::receive(&mut connection).await?;
                ensure!(
                    request.schema_version == 1 && request.run_id == id,
                    "invalid session request"
                );
                if request.stop && snapshot(&state)?.is_active() {
                    stop.cancel();
                }
                wire::send(&mut connection, &snapshot(&state)?).await
            })
            .await;
        });
    }
    Ok(())
}

const fn stopped() -> ExitOutcome {
    ExitOutcome {
        kind: ExitKind::Stopped,
        code: None,
        signal: None,
        windows_status: None,
    }
}

async fn execute(
    root: &Path,
    bootstrap: &wire::Bootstrap,
    state: &Shared,
    stop: &CancellationToken,
) -> anyhow::Result<ExitOutcome> {
    let record = snapshot(state)?;
    let library = Library::new(root.to_owned(), Arc::new(|_| {}), events(state))?;
    let plan = library
        .prepare_launch(
            &bootstrap.profile_id,
            bootstrap,
            stop,
            Some(&record.context.run_id),
            Some(&bootstrap.profile_fingerprint),
        )
        .await?;
    let baseline = crash::snapshot_reports(plan.game_directory()).map_or_else(
        |_| {
            warning(
                state,
                "Existing crash reports could not be indexed; report attachments are unavailable",
            );
            None
        },
        Some,
    );
    if stop.is_cancelled() {
        return Ok(stopped());
    }
    let result = LaunchExecutor::run_borrowed(&plan, events(state), stop).await;
    let observed = snapshot(state)?;
    let outcome = observed.outcome.unwrap_or_else(|| {
        if observed.java_pid.is_some() {
            ExitOutcome::monitoring_interrupted()
        } else if stop.is_cancelled() {
            stopped()
        } else {
            ExitOutcome::start_failed()
        }
    });
    if let Err(error) = result {
        let text = plan.redact(&format!("{error:#}"));
        warning(state, &text);
    }
    if outcome.should_notify() {
        let attachments = baseline
            .map(|baseline| {
                crash::collect_fresh_reports(plan.game_directory(), &baseline, |text| {
                    plan.redact(text)
                })
            })
            .transpose();
        let attachments = attachments.map_or_else(
            |_| {
                warning(
                    state,
                    "Some crash report attachments could not be collected",
                );
                Vec::new()
            },
            Option::unwrap_or_default,
        );
        let record = snapshot(state)?;
        let mut tail = record.recent_output.join("\n");
        for value in &record.capture_warnings {
            tail.push('\n');
            tail.push_str(value);
        }
        let report = CrashReport::new(record.context, outcome.clone(), &tail, attachments);
        if storage::write_report(root, &report).is_err() {
            warning(
                state,
                "The crash report could not be saved; check free disk space and folder permissions",
            );
        }
    }
    // The borrowed plan owns both locks throughout bounded report collection.
    Ok(outcome)
}

fn finish_record(root: &Path, state: &Shared, outcome: ExitOutcome) -> anyhow::Result<()> {
    let mut record = snapshot(state)?;
    let mut release_failed = false;
    if outcome.kind == ExitKind::MonitoringInterrupted {
        record.state = RunState::MonitoringInterrupted;
    } else {
        release_failed = match GameDirectoryGuard::session_reservation_owner(&record.game_directory)
        {
            Ok(Some(owner)) if owner == record.context.run_id => {
                GameDirectoryGuard::clear_session_reservation(
                    &record.game_directory,
                    &record.context.run_id,
                )
                .is_err()
            }
            // A failed second start never owned the existing game's reservation.
            Ok(_) => false,
            Err(_) => true,
        };
        record.state = RunState::Finished;
        record.finished_at_unix_ms = Some(now_ms());
    }
    record.status = match outcome.kind {
        ExitKind::Normal => "Exited normally · ready to play again",
        ExitKind::Stopped => "Stopped",
        ExitKind::Abnormal => "Minecraft closed unexpectedly · crash help is available",
        ExitKind::StartFailed => "Minecraft could not start · launch help is available",
        ExitKind::MonitoringInterrupted => "Monitoring was interrupted; the game may still be running. Close it normally, then verify it has ended before manually recovering its directory reservation.",
    }.into();
    if release_failed {
        record.status = "Minecraft ended, but its directory reservation could not be released. Verify the session has ended before manual recovery.".into();
        if record.capture_warnings.len() < 8 {
            record.capture_warnings.push(record.status.clone());
        }
    }
    record.outcome = Some(outcome);
    record.revision = record.revision.saturating_add(1);
    *state
        .lock()
        .map_err(|_| anyhow::anyhow!("session state unavailable"))? = record.clone();
    storage::write_record(root, &record)
}

async fn show_help(root: &Path, run_id: &str) -> anyhow::Result<()> {
    let directory = storage::report_directory(root, run_id)?;
    if !directory.join("crash.json").exists() {
        return Ok(());
    }
    for _ in 0..10 {
        if directory.join("seen.json").exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .arg("--crash")
        .arg(run_id)
        .arg("--data-dir")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("MICROSOFT_CLIENT_SECRET")
        .kill_on_drop(false);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000 | 0x0000_0200);
    for _ in 0..2 {
        let mut child = command
            .spawn()
            .context("could not open the crash help window")?;
        let mut exited = false;
        for _ in 0..20 {
            if directory.join("seen.json").exists() {
                return Ok(());
            }
            if child.try_wait()?.is_some() {
                exited = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !exited {
            return Ok(());
        }
        // A closing launcher may briefly retain its presentation lock before rendering.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}

fn launch_error(
    root: &Path,
    state: &Shared,
    bootstrap: Option<&wire::Bootstrap>,
    error: &anyhow::Error,
    stop: &CancellationToken,
) -> anyhow::Result<ExitOutcome> {
    let outcome = if stop.is_cancelled() {
        stopped()
    } else {
        ExitOutcome::start_failed()
    };
    let record = snapshot(state)?;
    let text = bootstrap.map_or_else(
        || "The supervisor could not accept the launch request".into(),
        |bootstrap| bootstrap.redact(&format!("{error:#}")),
    );
    if outcome.should_notify()
        && storage::write_report(
            root,
            &CrashReport::new(record.context, outcome.clone(), &text, Vec::new()),
        )
        .is_err()
    {
        warning(
            state,
            "Launch help could not be saved; check free disk space and folder permissions",
        );
    }
    Ok(outcome)
}

/// Run the headless mode of the same executable, owning exactly one game session.
/// # Errors
/// Returns invalid bootstrap, local IPC or persistence failures without logging credentials.
pub async fn run_session_host(root: &Path, run_id: &str) -> anyhow::Result<()> {
    let root = root.canonicalize()?;
    let directory = storage::report_directory(&root, run_id)?;
    let lock_path = directory.join("supervisor.lock");
    crate::executor::ensure_write_path(&root, &lock_path)?;
    let ownership = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&ownership)
        .context("this session already has a supervisor")?;
    let mut record = storage::read_run(&root, run_id)?;
    ensure!(
        record.supervisor_pid.is_none() && record.is_active(),
        "session has already been started"
    );
    let listener = session_ipc::listen(run_id)?;
    record.supervisor_pid = Some(std::process::id());
    record.state = RunState::Preparing;
    record.revision = record.revision.saturating_add(1);
    storage::write_record(&root, &record)?;
    let shared = Arc::new(Mutex::new(record));
    let stop = CancellationToken::new();
    let shutdown = CancellationToken::new();
    let server = tokio::spawn(serve(
        listener,
        run_id.into(),
        shared.clone(),
        stop.clone(),
        shutdown.clone(),
    ));
    let bootstrap = tokio::time::timeout(
        Duration::from_secs(5),
        wire::receive::<wire::Bootstrap>(&mut tokio::io::stdin()),
    )
    .await;
    let outcome = match bootstrap {
        Ok(Ok(bootstrap)) if bootstrap.profile_id == snapshot(&shared)?.context.profile_id => {
            let work = execute(&root, &bootstrap, &shared, &stop);
            tokio::pin!(work);
            loop {
                tokio::select! {
                    result = &mut work => break match result {
                        Ok(outcome) => outcome,
                        Err(error) => launch_error(&root, &shared, Some(&bootstrap), &error, &stop)?,
                    },
                    () = tokio::time::sleep(Duration::from_millis(300)) => {
                        if storage::write_record(&root, &snapshot(&shared)?).is_err() { warning(&shared, "Session status could not be saved"); }
                    }
                }
            }
        }
        _ => launch_error(
            &root,
            &shared,
            None,
            &anyhow::anyhow!("invalid bootstrap"),
            &stop,
        )?,
    };
    let notify = outcome.should_notify();
    finish_record(&root, &shared, outcome)?;
    if notify {
        let _ = show_help(&root, run_id).await;
    }
    shutdown.cancel();
    let _ = server.await;
    session_ipc::cleanup(run_id)?;
    drop(ownership);
    Ok(())
}
