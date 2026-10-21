use std::{
    collections::BTreeMap,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
};

use anyhow::Context as _;
use block_download::Store;
use tokio_util::sync::CancellationToken;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::{copy_immutable, extract, materialize, runtime_link};
use crate::{
    plan::{CommandTemplate, ExecutionPlan, JavaExecutable, PrepareAction},
    platform::{Architecture, OperatingSystem, Target},
};

fn plan(actions: Vec<PrepareAction>) -> ExecutionPlan {
    ExecutionPlan {
        schema_version: 1,
        instance: "executor-test".into(),
        version: "test".into(),
        manifest_sha1: String::new(),
        target: Target {
            os: OperatingSystem::Linux,
            arch: Architecture::X86_64,
            os_version: String::new(),
        },
        resources: Vec::new(),
        actions,
        command: CommandTemplate {
            java: JavaExecutable::Managed {
                path: "runtime/java".into(),
            },
            java_major: 21,
            main_class: "test.Main".into(),
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            classpath: Vec::new(),
            variables: BTreeMap::new(),
            path_variables: BTreeMap::new(),
            cwd: "game".into(),
        },
    }
}

fn output_command(root: &Path) -> anyhow::Result<crate::command::JavaCommand> {
    crate::command::JavaCommand::bind(
        &plan(Vec::new()),
        root,
        "capture-test",
        &crate::session::GameSession {
            username: "Player".into(),
            uuid: "123456781234123412341234567890ab".into(),
            access_token: "synthetic-capture-token".into(),
            user_type: "msa".into(),
            xuid: String::new(),
            client_id: String::new(),
            user_properties: "{}".into(),
        },
    )
}

#[tokio::test]
async fn output_capture_redacts_chunk_boundaries_and_discards_entire_oversized_lines()
-> anyhow::Result<()> {
    use std::{sync::Mutex, time::Duration};
    use tokio::io::AsyncWriteExt as _;
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("stdout.log");
    let command = output_command(temporary.path())?;
    let mut bytes =
        b"first synthetic-capture-token\nPRIVATE-PREFIX synthetic-capture-token ".to_vec();
    bytes.extend(vec![b'x'; super::MAX_OUTPUT_LINE_BYTES]);
    bytes.extend_from_slice(b" synthetic-capture-token\nlast synthetic-capture-token\xff");
    let (mut writer, reader) = tokio::io::duplex(13);
    let producer = tokio::spawn(async move {
        for chunk in bytes.chunks(7) {
            writer.write_all(chunk).await?;
        }
        writer.shutdown().await
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    tokio::time::timeout(
        Duration::from_secs(5),
        super::drain(
            reader,
            path.clone(),
            command,
            Arc::new(move |event| {
                if let Ok(mut sink) = sink.lock() {
                    sink.push(event);
                }
            }),
            false,
        ),
    )
    .await??;
    producer.await??;
    let log = fs::read_to_string(path)?;
    assert!(log.contains("first <redacted>\n"));
    assert!(log.contains("last <redacted>\u{fffd}\n"));
    assert!(log.contains("oversized game output line was discarded"));
    assert!(!log.contains("PRIVATE-PREFIX"));
    assert!(!log.contains("synthetic-capture-token"));
    let events = events
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock poisoned"))?;
    assert_eq!(events.len(), 3);
    assert!(!format!("{events:?}").contains("synthetic-capture-token"));
    assert!(!format!("{events:?}").contains("PRIVATE-PREFIX"));
    drop(events);
    Ok(())
}

#[tokio::test]
async fn failed_log_creation_keeps_draining_and_redacting_without_exposing_paths()
-> anyhow::Result<()> {
    use std::sync::Mutex;
    let temporary = tempfile::tempdir()?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    super::drain(
        std::io::Cursor::new(b"first synthetic-capture-token\nlast synthetic-capture-token"),
        temporary.path().join("PRIVATE-PATH/missing/stdout.log"),
        output_command(temporary.path())?,
        Arc::new(move |event| {
            if let Ok(mut sink) = sink.lock() {
                sink.push(event);
            }
        }),
        false,
    )
    .await?;
    let events = events
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock poisoned"))?;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, super::ExecutionEvent::CaptureFailed { .. }))
            .count(),
        1
    );
    assert!(
        matches!(events.last(), Some(super::ExecutionEvent::Log { line, .. }) if line == "last <redacted>")
    );
    let debug = format!("{events:?}");
    drop(events);
    assert!(!debug.contains("synthetic-capture-token"));
    assert!(!debug.contains("PRIVATE-PATH"));
    Ok(())
}

#[tokio::test]
async fn log_quota_caps_saved_bytes_but_keeps_consuming_and_emitting_final_output()
-> anyhow::Result<()> {
    use std::{
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::io::AsyncWriteExt as _;
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("stdout.log");
    let (mut writer, reader) = tokio::io::duplex(4096);
    let producer = tokio::spawn(async move {
        let mut line = vec![b'x'; super::MAX_OUTPUT_LINE_BYTES - 1];
        line.push(b'\n');
        for _ in 0..256 {
            writer.write_all(&line).await?;
        }
        writer
            .write_all(b"AFTER_QUOTA synthetic-capture-token\nFINAL_LINE\n")
            .await?;
        writer.shutdown().await
    });
    let failures = Arc::new(AtomicUsize::new(0));
    let failure_sink = failures.clone();
    let last_line = Arc::new(Mutex::new(String::new()));
    let last_sink = last_line.clone();
    tokio::time::timeout(
        Duration::from_secs(10),
        super::drain(
            reader,
            path.clone(),
            output_command(temporary.path())?,
            Arc::new(move |event| match event {
                super::ExecutionEvent::CaptureFailed { .. } => {
                    failure_sink.fetch_add(1, Ordering::SeqCst);
                }
                super::ExecutionEvent::Log { line, .. } => {
                    if let Ok(mut last) = last_sink.lock() {
                        *last = line;
                    }
                }
                _ => {}
            }),
            false,
        ),
    )
    .await??;
    producer.await??;
    assert_eq!(
        fs::metadata(path)?.len(),
        super::MAX_OUTPUT_FILE_BYTES as u64
    );
    assert_eq!(failures.load(Ordering::SeqCst), 1);
    assert_eq!(
        *last_line
            .lock()
            .map_err(|_| anyhow::anyhow!("capture lock poisoned"))?,
        "FINAL_LINE"
    );
    Ok(())
}

#[tokio::test]
async fn failed_log_write_is_reported_once_and_live_output_continues() -> anyhow::Result<()> {
    use std::sync::Mutex;
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("readonly.log");
    fs::write(&path, b"previous data")?;
    let mut capture = super::OutputCapture {
        file: Some(tokio::fs::File::from_std(fs::File::open(&path)?)),
        written: 0,
        quota_reached: false,
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let sink: super::ExecutionSink = Arc::new(move |event| {
        if let Ok(mut sink) = sink.lock() {
            sink.push(event);
        }
    });
    let command = output_command(temporary.path())?;
    for line in [
        b"first synthetic-capture-token".as_slice(),
        b"last synthetic-capture-token",
    ] {
        super::emit_output(line, &command, &mut capture, &sink, false).await;
    }
    capture.finish(&sink, false).await;
    let events = events
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock poisoned"))?;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, super::ExecutionEvent::CaptureFailed { .. }))
            .count(),
        1
    );
    assert!(
        matches!(events.last(), Some(super::ExecutionEvent::Log { line, .. }) if line == "last <redacted>")
    );
    assert!(!format!("{events:?}").contains("synthetic-capture-token"));
    drop(events);
    assert_eq!(fs::read(&path)?, b"previous data");
    Ok(())
}

fn archive(root: &Path, entries: &[(&str, &[u8])]) -> anyhow::Result<PathBuf> {
    let path = root.join("natives.zip");
    let mut writer = ZipWriter::new(fs::File::create(&path)?);
    for (name, bytes) in entries {
        writer.start_file(*name, SimpleFileOptions::default())?;
        writer.write_all(bytes)?;
    }
    writer.finish()?;
    Ok(path)
}

#[test]
fn concurrent_shared_runtime_publication_never_replaces_an_open_binary() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("object");
    let destination = directory.path().join("java");
    let bytes = vec![0x5a; 512 * 1024];
    fs::write(&source, &bytes)?;
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let barrier = barrier.clone();
            let source = &source;
            let destination = &destination;
            tasks.push(scope.spawn(move || {
                barrier.wait();
                copy_immutable(source, destination, true)
            }));
        }
        for task in tasks {
            task.join()
                .map_err(|_| anyhow::anyhow!("publication worker failed"))??;
        }
        anyhow::Ok(())
    })?;
    let mut running_binary = fs::File::open(&destination)?;
    // A subsequent preparer must preserve the published binary even if the
    // object pathname changed while this process still has the view open.
    // Integrity validation rejects the changed source without replacing the binary.
    fs::write(&source, vec![0xa5; bytes.len()])?;
    assert!(copy_immutable(&source, &destination, true).is_err());
    let mut opened_bytes = Vec::new();
    running_binary.read_to_end(&mut opened_bytes)?;
    assert_eq!(opened_bytes, bytes);
    assert_eq!(fs::read(&destination)?, bytes);
    assert_eq!(fs::read_dir(directory.path())?.count(), 2);
    Ok(())
}

#[test]
fn corrupt_existing_view_is_preserved_and_reported() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("object");
    let destination = directory.path().join("view");
    fs::write(&source, b"verified object")?;
    fs::write(&destination, b"bad")?;
    let error = copy_immutable(&source, &destination, false)
        .err()
        .context("a corrupt view was accepted")?;
    assert!(error.to_string().contains("immutable view is corrupt"));
    assert_eq!(fs::read(destination)?, b"bad");
    Ok(())
}

#[test]
fn native_extraction_honors_exclusions_and_rejects_portable_escape_paths() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = archive(
        directory.path(),
        &[
            ("META-INF/signature", b"ignored"),
            ("lib/native", b"library"),
        ],
    )?;
    let destination = directory.path().join("run/natives");
    extract(
        directory.path(),
        &source,
        &destination,
        &["META-INF/".into()],
    )?;
    assert_eq!(fs::read(destination.join("lib/native"))?, b"library");
    assert!(!destination.join("META-INF").exists());
    for name in [
        "../escape",
        "nested/../../escape",
        "C:escape",
        "nested\\escape",
    ] {
        let source = archive(directory.path(), &[(name, b"must not escape")])?;
        assert!(extract(directory.path(), &source, &destination, &[]).is_err());
    }
    assert!(!directory.path().join("escape").exists());
    Ok(())
}

#[test]
fn native_archives_cannot_publish_symlinks() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("natives.zip");
    let mut writer = ZipWriter::new(fs::File::create(&source)?);
    writer.add_symlink("native", "../outside", SimpleFileOptions::default())?;
    writer.finish()?;
    let destination = directory.path().join("natives");
    let error = extract(directory.path(), &source, &destination, &[])
        .err()
        .context("archive symlink was accepted")?;
    assert!(error.to_string().contains("archive contains a symlink"));
    assert!(!destination.join("native").exists());
    Ok(())
}

#[test]
fn runtime_aliases_can_precede_their_targets_in_the_manifest() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    fs::create_dir(root.join("runtime"))?;
    fs::write(root.join("runtime/java-real"), b"runtime binary")?;
    let actions = [
        ("runtime/java", "java-alias"),
        ("runtime/java-alias", "java-real"),
    ]
    .map(|(path, target)| PrepareAction::Link {
        path: path.into(),
        target: target.into(),
        scope: "runtime".into(),
    });
    let plan = plan(actions.to_vec());
    let store = Store::new(root.clone(), true, Arc::new(|_| {}))?;
    materialize(&plan, &root, "run", &store, &CancellationToken::new())?;
    materialize(&plan, &root, "run", &store, &CancellationToken::new())?;
    assert_eq!(fs::read(root.join("runtime/java"))?, b"runtime binary");
    Ok(())
}

#[test]
fn cyclic_or_missing_runtime_aliases_fail_without_hanging() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    fs::create_dir(root.join("runtime"))?;
    let store = Store::new(root.clone(), true, Arc::new(|_| {}))?;
    for target in ["java", "missing"] {
        let plan = plan(vec![PrepareAction::Link {
            path: "runtime/java".into(),
            target: target.into(),
            scope: "runtime".into(),
        }]);
        let error = materialize(&plan, &root, "run", &store, &CancellationToken::new())
            .err()
            .context("invalid runtime alias was accepted")?;
        assert!(error.to_string().contains("cyclic or have missing targets"));
    }
    Ok(())
}

#[test]
fn runtime_aliases_cannot_escape_their_installation() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    fs::create_dir(directory.path().join("runtime"))?;
    fs::write(directory.path().join("outside"), b"outside runtime")?;
    assert!(runtime_link(directory.path(), "runtime/java", "../outside", "runtime").is_err());
    assert!(!directory.path().join("runtime/java").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn even_empty_archives_cannot_create_directories_through_symlinks() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    std::os::unix::fs::symlink(outside.path(), root.join("linked"))?;
    let source = archive(&root, &[])?;
    assert!(extract(&root, &source, &root.join("linked/created"), &[]).is_err());
    assert!(!outside.path().join("created").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn runtime_aliases_cannot_resolve_through_an_external_symlink() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    fs::create_dir(directory.path().join("runtime"))?;
    fs::write(outside.path().join("java"), b"outside binary")?;
    std::os::unix::fs::symlink(outside.path(), directory.path().join("runtime/escape"))?;
    assert!(runtime_link(directory.path(), "runtime/java", "escape/java", "runtime").is_err());
    assert!(!directory.path().join("runtime/java").exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn preparation_rejects_instance_symlinks_before_creating_a_lock() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    fs::create_dir(root.join("instances"))?;
    std::os::unix::fs::symlink(outside.path(), root.join("instances/executor-test"))?;
    let store = Store::new(root, true, Arc::new(|_| {}))?;
    let executor = super::Executor::new(store, Arc::new(|_| {}));
    let mut plan = plan(Vec::new());
    plan.target = Target::host()?;
    assert!(
        executor
            .prepare(plan, &CancellationToken::new())
            .await
            .is_err()
    );
    assert!(!outside.path().join("instance.lock").exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn launcher_drains_both_pipes_and_reaps_a_cancelled_process() -> anyhow::Result<()> {
    use std::{sync::Mutex, time::Duration};

    use crate::session::OfflineSession;

    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    fs::create_dir(root.join("game"))?;
    let executable = root.join("fake-java");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s\\n' \"$1\" \"$2\"\nprintf '%s\\n' 'ready' >&2\nexec sleep 60\n",
    )?;
    super::set_executable(&executable, true)?;
    let mut plan = plan(Vec::new());
    plan.command.java = JavaExecutable::Local { path: executable };
    plan.command.game_args = vec!["one argument with spaces".into()];
    let prepared = super::PreparedInstance {
        plan,
        root: root.clone(),
        run_id: "cancel-test".into(),
        _lock: fs::File::create(root.join("lock"))?,
    };
    let cancel = CancellationToken::new();
    let cancellation = cancel.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let store = Store::new(root.clone(), true, Arc::new(|_| {}))?;
    let executor = super::Executor::new(
        store,
        Arc::new(move |event| {
            if matches!(&event, super::ExecutionEvent::Log { line, stderr: true } if line == "ready")
            {
                cancellation.cancel();
            }
            if let Ok(mut captured) = captured.lock() {
                captured.push(event);
            }
        }),
    );
    let session = OfflineSession::Offline {
        username: "Player".into(),
    };
    let code = tokio::time::timeout(
        Duration::from_secs(5),
        executor.launch(prepared, &session, &cancel),
    )
    .await??;
    assert_eq!(code, None);
    let logs = root.join("instances/executor-test/runs/cancel-test/logs");
    assert_eq!(
        fs::read_to_string(logs.join("stdout.log"))?,
        "test.Main\none argument with spaces\n"
    );
    assert_eq!(fs::read_to_string(logs.join("stderr.log"))?, "ready\n");
    let events = events
        .lock()
        .map_err(|_| anyhow::anyhow!("event capture mutex was poisoned"))?;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, super::ExecutionEvent::Started { .. }))
    );
    assert!(matches!(
        events.last(),
        Some(super::ExecutionEvent::Exited { code: None })
    ));
    drop(events);
    Ok(())
}

#[test]
fn same_size_view_corruption_is_detected_without_changing_either_file() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("verified");
    let view = root.path().join("view");
    fs::write(&source, b"original")?;
    fs::write(&view, b"tampered")?;
    assert!(copy_immutable(&source, &view, false).is_err());
    assert_eq!(fs::read(&source)?, b"original");
    assert_eq!(fs::read(&view)?, b"tampered");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn resolved_launch_owns_directory_until_exit_and_redacts_actual_process_logs()
-> anyhow::Result<()> {
    use crate::{
        catalog::GameDirectoryGuard,
        launch::{LaunchExecutor, LaunchPlan},
        session::GameSession,
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let store_directory = tempfile::tempdir()?;
    let root = store_directory.path().canonicalize()?;
    let mutable = tempfile::tempdir()?;
    let game = mutable.path().canonicalize()?;
    let java = root.join("fake-java");
    fs::write(
        &java,
        "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$BLOCK_TEST_RENDERER\" \"$2\"\nexit 0\n",
    )?;
    super::set_executable(&java, true)?;
    let mut plan = plan(Vec::new());
    plan.command.java = JavaExecutable::Local { path: java };
    plan.command.game_args = vec!["${auth_access_token}".into()];
    let prepared = super::PreparedInstance {
        plan,
        root: root.clone(),
        run_id: "resolved-test".into(),
        _lock: fs::File::create(root.join("lock"))?,
    };
    let session = GameSession {
        username: "Player".into(),
        uuid: "123456781234123412341234567890ab".into(),
        access_token: "synthetic-sensitive-access-token".into(),
        user_type: "msa".into(),
        xuid: String::new(),
        client_id: String::new(),
        user_properties: "{}".into(),
    };
    let directory = GameDirectoryGuard::acquire(&game)?;
    let invocation = LaunchPlan::resolve(
        prepared,
        &session,
        directory,
        BTreeMap::from([("BLOCK_TEST_RENDERER".into(), "software-test".into())]),
    )?;
    assert_eq!(invocation.preview().cwd, game);
    assert!(!format!("{invocation:?}").contains("synthetic-sensitive-access-token"));
    let lock_was_held = Arc::new(AtomicBool::new(false));
    let observed = lock_was_held.clone();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    let locked_directory = game.clone();
    let status = LaunchExecutor::run(
        invocation,
        Arc::new(move |event| {
            if matches!(event, super::ExecutionEvent::Started { .. }) {
                observed.store(
                    GameDirectoryGuard::acquire(&locked_directory).is_err(),
                    Ordering::SeqCst,
                );
            }
            if let Ok(mut events) = sink.lock() {
                events.push(event);
            }
        }),
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(status, Some(0));
    assert!(lock_was_held.load(Ordering::SeqCst));
    assert!(GameDirectoryGuard::acquire(&game).is_ok());
    let log = fs::read_to_string(game.join(".block-runs/resolved-test/stdout.log"))?;
    assert!(
        log.contains(
            game.to_str()
                .ok_or_else(|| anyhow::anyhow!("nonUTF8 path"))?
        )
    );
    assert!(log.contains("software-test"));
    assert!(log.contains("<redacted>"));
    assert!(!log.contains("synthetic-sensitive-access-token"));
    Ok(())
}
