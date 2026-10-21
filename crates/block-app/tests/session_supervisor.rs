//! Process-level tests use a compiled fixture, not a game installation or account.

use std::{
    fs::{self, File, OpenOptions},
    io::Read as _,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context as _, ensure};
use block_download::Store;
use block_launcher::{
    catalog::{Catalog, GameDirectoryGuard, InstallStatus, Instance, Profile},
    crash::{DiagnosisKind, ExitKind},
    recipe::{Compatibility, RuntimeSelection},
    session::GameSession,
    session_host::{self, RunRecord, RunState},
};
use serde_json::json;
use sha1::{Digest as _, Sha1};

const TOKEN: &str = "synthetic-fixture-access-token-never-persist-this";
const EXE: &str = env!("CARGO_BIN_EXE_block");

struct Fixture {
    temporary: Option<tempfile::TempDir>,
    root: PathBuf,
    tools: PathBuf,
    instance: Instance,
    profile: Profile,
    game: PathBuf,
}

impl Fixture {
    fn new(mode: &str) -> anyhow::Result<Self> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().canonicalize()?;
        let tools = root.join("fixture");
        fs::create_dir(&tools)?;
        let executable = tools.join(format!("fake-java{}", std::env::consts::EXE_SUFFIX));
        let output = Command::new("rustc")
            .args(["--edition=2024", "--crate-name=block_fixture_java"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_java.rs"))
            .arg("-o")
            .arg(&executable)
            .output()?;
        ensure!(
            output.status.success(),
            "fixture compiler failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(tools.join("mode"), mode)?;
        if mode == "probe-fail" {
            fs::write(tools.join("probe-hold"), b"wait")?;
        }
        let catalog = Catalog::open(&root)?;
        let (instance, mut profile) =
            catalog.create_instance("Supervisor fixture".into(), "1.20.6".into(), None, None)?;
        profile.runtime = RuntimeSelection::Local { java: executable };
        // The synthetic child loads no native code. Real ARM64 policy is tested separately.
        profile.compatibility = Compatibility::Official;
        catalog.save_profile(profile.clone())?;
        let store = Store::new(root.clone(), true, Arc::new(|_| {}))?;
        let assets = object(&store, br#"{"objects":{}}"#)?;
        let client = object(&store, b"fixture client bytes, not a real jar")?;
        let libraries = ["lwjgl-3.3.3-natives-macos-arm64.jar", "lwjgl-glfw-3.3.3-natives-macos-arm64.jar"]
            .into_iter().map(|filename| {
                let mut artifact = client.clone();
                artifact["path"] = json!(filename);
                json!({"name":format!("org.lwjgl:{filename}:fixture"),"downloads":{"artifact":artifact}})
            }).collect::<Vec<_>>();
        let mut asset_index = assets;
        asset_index["id"] = json!("fixture");
        asset_index["totalSize"] = json!(0);
        let manifest = root.join("fixture-version.json");
        fs::write(
            &manifest,
            serde_json::to_vec(&json!({
                "id":"1.20.6", "type":"release", "time":"2024-04-29T00:00:00Z",
                "releaseTime":"2024-04-29T00:00:00Z", "mainClass":"fixture.Main",
                "minimumLauncherVersion":21, "assets":"fixture", "assetIndex":asset_index,
                "downloads":{"client":client}, "libraries":libraries,
                "javaVersion":{"component":"java-runtime-delta","majorVersion":21},
                "arguments":{"game":["--accessToken","${auth_access_token}","--username","${auth_player_name}"],"jvm":[]}
            }))?,
        )?;
        let mut target = instance.targets[0].clone();
        target.manifest = Some(manifest);
        target.status = InstallStatus::Ready;
        catalog.set_target(&instance.id, &target)?;
        let game = profile.game_directory(&instance)?;
        fs::create_dir_all(game.join("crash-reports"))?;
        fs::write(
            game.join("crash-reports/crash-fixture-old-client.txt"),
            b"old-report-must-not-be-attributed-to-this-run",
        )?;
        Ok(Self {
            temporary: Some(temporary),
            root,
            tools,
            instance,
            profile,
            game,
        })
    }

    async fn start(&self) -> anyhow::Result<RunRecord> {
        self.start_with(Path::new(EXE)).await
    }

    async fn start_with(&self, executable: &Path) -> anyhow::Result<RunRecord> {
        session_host::spawn_session(
            &self.root,
            executable,
            &self.profile.id,
            &GameSession {
                username: "FixturePlayer".into(),
                uuid: "0123456789abcdef0123456789abcdef".into(),
                access_token: TOKEN.into(),
                user_type: "msa".into(),
                xuid: "fixture-xuid".into(),
                client_id: String::new(),
                user_properties: "{}".into(),
                authlib: None,
            },
            SystemTime::now() + Duration::from_secs(600),
        )
        .await
    }

    fn release(&self) -> anyhow::Result<()> {
        fs::write(self.tools.join("release"), b"finish")?;
        Ok(())
    }

    fn presenter(&self, run_id: &str) -> anyhow::Result<File> {
        let path = session_host::report_directory(&self.root, run_id)?.join("presentation.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        fs2::FileExt::try_lock_exclusive(&file)?;
        Ok(file)
    }

    fn complete(mut self) {
        // Successful tests waited for their own supervisor and Java process to finish.
        self.temporary.take();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(temporary) = self.temporary.take() {
            // On failure, let the owned fixture exit normally and preserve its evidence.
            // Do not delete its executable, gate or diagnostics while it is still alive.
            let _ = fs::write(self.tools.join("mode"), "normal");
            let _ = fs::write(self.tools.join("release"), b"test cleanup");
            let _ = temporary.keep();
        }
    }
}

fn object(store: &Store, bytes: &[u8]) -> anyhow::Result<serde_json::Value> {
    let digest = format!("{:x}", Sha1::digest(bytes));
    let path = store.object_path(&digest)?;
    fs::create_dir_all(path.parent().context("object parent missing")?)?;
    fs::write(path, bytes)?;
    Ok(json!({"sha1":digest,"size":bytes.len(),"url":"https://example.invalid/fixture"}))
}

async fn wait_for(root: &Path, run_id: &str, desired: RunState) -> anyhow::Result<RunRecord> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let record = session_host::status(root, run_id).await?;
        if record.state == desired {
            return Ok(record);
        }
        ensure!(
            record.state != RunState::Finished,
            "session finished before {desired:?}: {}",
            record.status
        );
        if Instant::now() >= deadline {
            let directory = session_host::report_directory(root, run_id)?;
            let persisted = session_host::read_run(root, run_id)?;
            let mut diagnostic = Vec::new();
            if let Ok(file) = File::open(directory.join("supervisor.log")) {
                file.take(16 * 1024).read_to_end(&mut diagnostic)?;
            }
            anyhow::bail!(
                "session did not reach {desired:?}: {}; persisted={:?}, outcome={:?}, warnings={:?}, fixture_exited={}, supervisor_diagnostic={}",
                record.status,
                persisted.state,
                persisted.outcome,
                persisted.capture_warnings,
                root.join("fixture/java.exited").exists(),
                String::from_utf8_lossy(&diagnostic).replace(TOKEN, "<redacted>")
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_host_exit(root: &Path, run_id: &str) -> anyhow::Result<()> {
    let path = session_host::report_directory(root, run_id)?.join("supervisor.lock");
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(()),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(error) => return Err(error.into()),
        }
        ensure!(Instant::now() < deadline, "fixture supervisor did not exit");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn detached_game_reconnects_stops_and_can_restart_normally() -> anyhow::Result<()> {
    let fixture = Fixture::new("normal")?;
    let record = fixture.start().await?;
    let id = record.context.run_id.clone();
    let running = wait_for(&fixture.root, &id, RunState::Running).await?;
    let pid = running.java_pid.context("fixture Java PID missing")?;
    drop(record);
    drop(running);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let reconnected = session_host::status(&fixture.root, &id).await?;
    assert_eq!(reconnected.state, RunState::Running);
    assert_eq!(reconnected.java_pid, Some(pid));
    assert_eq!(
        fs::read_to_string(fixture.tools.join("java.pid"))?,
        pid.to_string()
    );
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_err());
    session_host::stop(&fixture.root, &id).await?;
    let stopped = wait_for(&fixture.root, &id, RunState::Finished).await?;
    assert_eq!(
        stopped.outcome.context("stop outcome missing")?.kind,
        ExitKind::Stopped
    );
    assert!(session_host::read_crash(&fixture.root, &id).is_err());
    wait_host_exit(&fixture.root, &id).await?;
    assert!(!fixture.game.join(".block-session.json").exists());

    let restarted = fixture.start().await?;
    let id = restarted.context.run_id;
    wait_for(&fixture.root, &id, RunState::Running).await?;
    fixture.release()?;
    let normal = wait_for(&fixture.root, &id, RunState::Finished).await?;
    let outcome = normal.outcome.context("normal outcome missing")?;
    assert_eq!(outcome.kind, ExitKind::Normal);
    assert_eq!(outcome.code, Some(0));
    assert!(session_host::read_crash(&fixture.root, &id).is_err());
    wait_host_exit(&fixture.root, &id).await?;
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_ok());
    fixture.complete();
    Ok(())
}

#[tokio::test]
async fn abnormal_exit_collects_only_fresh_sanitized_evidence_and_retains_unread_help()
-> anyhow::Result<()> {
    let fixture = Fixture::new("crash")?;
    let record = fixture.start().await?;
    let id = record.context.run_id;
    wait_for(&fixture.root, &id, RunState::Running).await?;
    let presentation = fixture.presenter(&id)?;
    fixture.release()?;
    let finished = wait_for(&fixture.root, &id, RunState::Finished).await?;
    let outcome = finished.outcome.context("crash outcome missing")?;
    assert_eq!(outcome.kind, ExitKind::Abnormal);
    assert_eq!(outcome.code, Some(17));
    let report = session_host::read_crash(&fixture.root, &id)?;
    assert!(
        report
            .diagnoses
            .iter()
            .any(|value| value.kind == DiagnosisKind::JavaHeapMemory)
    );
    assert_eq!(report.attachments.len(), 2);
    assert!(
        report
            .attachments
            .iter()
            .all(|attachment| !attachment.name.contains("old"))
    );
    let directory = session_host::report_directory(&fixture.root, &id)?;
    assert!(!directory.join("seen.json").exists());
    assert!(session_host::claim_help(&fixture.root, &id)?.is_none());
    wait_host_exit(&fixture.root, &id).await?;
    for path in [
        directory.join("session.json"),
        directory.join("crash.json"),
        directory.join("supervisor.log"),
        fixture
            .game
            .join(".block-runs")
            .join(&id)
            .join("stdout.log"),
        fixture
            .game
            .join(".block-runs")
            .join(&id)
            .join("stderr.log"),
    ] {
        assert!(!fs::read_to_string(path)?.contains(TOKEN));
    }
    assert!(!serde_json::to_string(&report)?.contains(TOKEN));
    assert!(!fixture.game.join(".block-session.json").exists());
    drop(presentation);
    assert!(session_host::claim_help(&fixture.root, &id)?.is_some());
    assert!(!directory.join("seen.json").exists());
    fixture.complete();
    Ok(())
}

#[tokio::test]
async fn rejected_second_launch_preserves_the_other_live_sessions_reservation() -> anyhow::Result<()>
{
    let fixture = Fixture::new("normal")?;
    let first = fixture.start().await?.context.run_id;
    wait_for(&fixture.root, &first, RunState::Running).await?;
    let second = fixture.start().await?.context.run_id;
    let presentation = fixture.presenter(&second)?;
    let rejected = wait_for(&fixture.root, &second, RunState::Finished).await?;
    assert_eq!(
        rejected.outcome.context("outcome missing")?.kind,
        ExitKind::StartFailed
    );
    assert!(!rejected.status.contains("manual recovery"));
    let reservation: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.game.join(".block-session.json"))?)?;
    assert_eq!(reservation["run_id"], first);
    assert_eq!(
        session_host::status(&fixture.root, &first).await?.state,
        RunState::Running
    );
    wait_host_exit(&fixture.root, &second).await?;
    fixture.release()?;
    wait_for(&fixture.root, &first, RunState::Finished).await?;
    wait_host_exit(&fixture.root, &first).await?;
    drop(presentation);
    fixture.complete();
    Ok(())
}

#[tokio::test]
async fn terminal_exit_reports_a_reservation_that_could_not_be_released() -> anyhow::Result<()> {
    let fixture = Fixture::new("normal")?;
    let record = fixture.start().await?;
    let id = record.context.run_id;
    wait_for(&fixture.root, &id, RunState::Running).await?;
    let marker = fixture.game.join(".block-session.json");
    let original = fs::read(&marker)?;
    fs::write(&marker, b"invalid reservation intentionally preserved")?;
    fixture.release()?;
    let finished = wait_for(&fixture.root, &id, RunState::Finished).await?;
    assert_eq!(
        finished.outcome.context("outcome missing")?.kind,
        ExitKind::Normal
    );
    assert!(finished.status.contains("manual recovery"));
    assert!(!finished.status.contains("ready to play"));
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_err());
    wait_host_exit(&fixture.root, &id).await?;
    fs::write(marker, original)?;
    GameDirectoryGuard::clear_session_reservation(&fixture.game, &id)?;
    fixture.complete();
    Ok(())
}

#[tokio::test]
async fn missing_supervisor_finishes_without_starting_java_or_leaking_the_token()
-> anyhow::Result<()> {
    let fixture = Fixture::new("normal")?;
    let record = fixture
        .start_with(&fixture.tools.join("missing-supervisor"))
        .await?;
    assert_eq!(record.state, RunState::Finished);
    assert!(record.supervisor_pid.is_none());
    assert!(record.java_pid.is_none());
    assert_eq!(
        record.outcome.context("start failure missing")?.kind,
        ExitKind::StartFailed
    );
    let id = record.context.run_id;
    let report = session_host::read_crash(&fixture.root, &id)?;
    assert!(report.attachments.is_empty());
    assert!(!serde_json::to_string(&report)?.contains(TOKEN));
    assert!(!fixture.tools.join("java.pid").exists());
    assert!(!fixture.game.join(".block-session.json").exists());
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_ok());
    fixture.complete();
    Ok(())
}

#[tokio::test]
async fn java_probe_failure_is_a_launch_error_without_old_crash_attachments() -> anyhow::Result<()>
{
    let fixture = Fixture::new("probe-fail")?;
    let record = fixture.start().await?;
    let id = record.context.run_id;
    let presentation = fixture.presenter(&id)?;
    fixture.release()?;
    let finished = wait_for(&fixture.root, &id, RunState::Finished).await?;
    assert!(finished.java_pid.is_none());
    assert_eq!(
        finished
            .outcome
            .context("launch error outcome missing")?
            .kind,
        ExitKind::StartFailed
    );
    let report = session_host::read_crash(&fixture.root, &id)?;
    assert!(report.attachments.is_empty());
    assert!(
        report.sanitized_tail.contains("requires Java 21"),
        "unexpected launch diagnostic: {}",
        report.sanitized_tail
    );
    assert!(!serde_json::to_string(&report)?.contains(TOKEN));
    wait_host_exit(&fixture.root, &id).await?;
    assert!(!fixture.game.join(".block-session.json").exists());
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_ok());
    drop(presentation);
    fixture.complete();
    Ok(())
}

#[tokio::test]
async fn launch_report_write_failure_still_finalizes_the_known_terminal_session()
-> anyhow::Result<()> {
    let fixture = Fixture::new("probe-fail")?;
    let record = fixture.start().await?;
    let id = record.context.run_id;
    let directory = session_host::report_directory(&fixture.root, &id)?;
    fs::create_dir(directory.join("crash.json"))?;
    fixture.release()?;
    let finished = wait_for(&fixture.root, &id, RunState::Finished).await?;
    assert!(finished.java_pid.is_none());
    assert_eq!(
        finished.outcome.context("outcome missing")?.kind,
        ExitKind::StartFailed
    );
    assert!(
        finished
            .capture_warnings
            .iter()
            .any(|text| text.contains("could not be saved"))
    );
    wait_host_exit(&fixture.root, &id).await?;
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_ok());
    fixture.complete();
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn abrupt_monitor_death_keeps_the_live_game_reserved() -> anyhow::Result<()> {
    let fixture = Fixture::new("normal")?;
    let record = fixture.start().await?;
    let id = record.context.run_id;
    let running = wait_for(&fixture.root, &id, RunState::Running).await?;
    let host = running.supervisor_pid.context("supervisor PID missing")?;
    let java = running.java_pid.context("fixture PID missing")?;
    // Both identities come from the live private endpoint of this newly created run.
    assert_eq!(
        session_host::status(&fixture.root, &id)
            .await?
            .supervisor_pid,
        Some(host)
    );
    ensure!(
        Command::new("/bin/kill")
            .args(["-KILL", &host.to_string()])
            .status()?
            .success(),
        "could not terminate the owned fixture supervisor"
    );
    wait_host_exit(&fixture.root, &id).await?;
    let interrupted = session_host::status(&fixture.root, &id).await?;
    assert_eq!(interrupted.state, RunState::MonitoringInterrupted);
    assert!(
        Command::new("/bin/kill")
            .args(["-0", &java.to_string()])
            .status()?
            .success()
    );
    assert!(fixture.game.join(".block-session.json").exists());
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_err());
    assert!(session_host::stop(&fixture.root, &id).await.is_err());
    fixture.release()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.tools.join("java.exited").exists() {
        ensure!(Instant::now() < deadline, "orphaned fixture did not finish");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    while Command::new("/bin/kill")
        .args(["-0", &java.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?
        .success()
    {
        ensure!(Instant::now() < deadline, "orphaned fixture has not exited");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    GameDirectoryGuard::clear_session_reservation(&fixture.game, &id)?;
    assert!(GameDirectoryGuard::for_profile(&fixture.instance, &fixture.profile).is_ok());
    fixture.complete();
    Ok(())
}
