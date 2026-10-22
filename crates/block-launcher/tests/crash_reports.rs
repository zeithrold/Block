use std::fs;

use block_launcher::crash::{
    AttachmentKind, Confidence, CrashContext, CrashReport, DiagnosisKind, DiagnosticAttachment,
    ExitKind, ExitOutcome, collect_fresh_reports, sanitize_diagnostic_text, snapshot_reports,
};

fn report(log: &str) -> CrashReport {
    CrashReport::new(
        CrashContext {
            run_id: "owned-test-run".into(),
            profile_id: "test-profile".into(),
            profile_name: "Test".into(),
            minecraft_version: "1.20.6".into(),
            loader: Some("NeoForge".into()),
        },
        ExitOutcome {
            kind: ExitKind::Abnormal,
            code: Some(1),
            signal: None,
            windows_status: None,
        },
        log,
        Vec::new(),
    )
}

#[test]
fn heap_and_native_memory_have_distinct_evidence_and_advice() {
    let heap = report("java.lang.OutOfMemoryError: Java heap space");
    assert_eq!(heap.diagnoses.len(), 1);
    assert_eq!(heap.diagnoses[0].kind, DiagnosisKind::JavaHeapMemory);
    assert_eq!(heap.diagnoses[0].evidence.len(), 1);
    let native = report("Native memory allocation (malloc) failed to allocate 32744 bytes");
    assert_eq!(native.diagnoses.len(), 1);
    assert_eq!(native.diagnoses[0].kind, DiagnosisKind::NativeMemory);
    assert!(native.diagnoses[0].suggested_action.contains("less memory"));
    assert_ne!(
        heap.diagnoses[0].suggested_action,
        native.diagnoses[0].suggested_action
    );
}

#[test]
fn explicit_failure_signatures_produce_bounded_evidence() {
    for (message, expected) in [
        (
            "java.lang.UnsupportedClassVersionError: Loader",
            DiagnosisKind::JavaVersion,
        ),
        ("Incompatible mods found!", DiagnosisKind::ModDependencies),
        (
            "GLFW error 65542: WGL: The driver does not appear to support OpenGL",
            DiagnosisKind::GraphicsInitialization,
        ),
    ] {
        let result = report(&format!("{message}{}", "x".repeat(2000)));
        assert_eq!(result.diagnoses[0].kind, expected);
        assert_eq!(result.diagnoses[0].confidence, Confidence::Possible);
        assert!(result.diagnoses[0].evidence[0].len() <= 400);
    }
}

#[test]
fn unknown_credentials_and_jvm_environment_are_omitted_before_export() -> anyhow::Result<()> {
    let text = "VM Arguments:\njvm_args: -Dcustom.value=UNKNOWN-JVM-CREDENTIAL\njava_command: Game --accessToken <redacted>\n\nEnvironment Variables:\nPRIVATE_CONFIGURATION=UNKNOWN-ENV-CREDENTIAL\nAZURE_CLIENT_SECRET=UNKNOWN-CLIENT-SECRET\n\nAuthorization: Bearer UNKNOWN-AUTHORIZATION\n{\"refresh_token\":\"UNKNOWN-REFRESH-CREDENTIAL\"}\nmyApiKey=UNKNOWN-API-CREDENTIAL\nCookie: sid=UNKNOWN-COOKIE\njava.lang.OutOfMemoryError: Java heap space\n";
    let sanitized = sanitize_diagnostic_text(text);
    assert!(sanitized.contains("OutOfMemoryError: Java heap space"));
    assert!(!sanitized.contains("UNKNOWN-"));
    let original = report("");
    let result = CrashReport::new(
        original.context,
        original.outcome,
        text,
        vec![DiagnosticAttachment {
            name: "password=UNKNOWN-FILENAME-CREDENTIAL".into(),
            kind: AttachmentKind::JvmFatalLog,
            sanitized_text: text.into(),
            truncated: false,
        }],
    );
    let multiline = "{\n\"refresh_token\":\n\"UNKNOWN-MULTILINE-CREDENTIAL\"\n}\n";
    assert!(!sanitize_diagnostic_text(multiline).contains("UNKNOWN-"));
    let serialized = serde_json::to_string(&result)?;
    assert!(!serialized.contains("UNKNOWN-"));
    assert_eq!(result.diagnoses[0].kind, DiagnosisKind::JavaHeapMemory);
    assert!(
        result
            .sanitized_tail
            .contains("Sensitive diagnostic content omitted")
    );
    Ok(())
}

#[test]
fn defensive_sanitization_retains_plain_memory_and_version_evidence() {
    let text = "Native memory allocation (malloc) failed to allocate 64 bytes\njava.lang.UnsupportedClassVersionError: Main\n";
    assert_eq!(sanitize_diagnostic_text(text), text);
    let text = "Environment Variables:\nPATH=/private/example\nMemory:\nNative memory allocation (malloc) failed to allocate 64 bytes\n";
    let sanitized = sanitize_diagnostic_text(text);
    assert!(!sanitized.contains("/private/example"));
    assert!(sanitized.contains("Native memory allocation (malloc) failed"));
    let text = "Command Line: -Dcustom=UNKNOWN-VALUE OutOfMemoryError: Java heap space\n";
    assert!(!sanitize_diagnostic_text(text).contains("UNKNOWN-VALUE"));
}

#[test]
fn exit_code_one_and_error_severity_do_not_invent_a_cause() {
    for log in ["", "[Render thread/ERROR]: Failed to load optional texture"] {
        let result = report(log);
        assert_eq!(result.diagnoses.len(), 1);
        assert_eq!(result.diagnoses[0].kind, DiagnosisKind::Unknown);
        assert_eq!(result.diagnoses[0].evidence, Vec::<String>::new());
    }
    let mut normal = report("java.lang.OutOfMemoryError: Java heap space");
    normal.outcome.kind = ExitKind::Normal;
    let result = CrashReport::new(
        normal.context,
        normal.outcome,
        &normal.sanitized_tail,
        Vec::new(),
    );
    assert!(result.diagnoses.is_empty());
    assert!(!result.outcome.should_notify());
    assert!(!ExitOutcome::monitoring_interrupted().should_notify());
    assert!(ExitOutcome::start_failed().should_notify());
}

#[test]
fn old_reports_are_excluded_and_new_reports_are_redacted_before_serialization() -> anyhow::Result<()>
{
    let directory = tempfile::tempdir()?;
    let reports = directory.path().join("crash-reports");
    fs::create_dir(&reports)?;
    fs::write(
        reports.join("crash-old-client.txt"),
        b"Old graphics failure",
    )?;
    let baseline = snapshot_reports(directory.path())?;
    fs::write(
        reports.join("crash-new-client.txt"),
        b"java.lang.OutOfMemoryError: Java heap space\naccessToken=PRIVATE-TEST-TOKEN",
    )?;
    fs::write(
        directory.path().join("hs_err_pid123.log"),
        b"JVM fatal error\xff",
    )?;
    fs::write(
        directory.path().join("unrelated-private.txt"),
        b"never collect",
    )?;
    let attachments = collect_fresh_reports(directory.path(), &baseline, |text| {
        text.replace("PRIVATE-TEST-TOKEN", "[REDACTED]")
    })?;
    assert_eq!(attachments.len(), 2);
    assert!(serde_json::to_string(&attachments)?.contains("[REDACTED]"));
    assert!(
        attachments
            .iter()
            .any(|attachment| attachment.kind == AttachmentKind::JvmFatalLog)
    );
    assert!(
        attachments
            .iter()
            .all(|attachment| !attachment.name.contains("old"))
    );
    let mut result = report("");
    result = CrashReport::new(result.context, result.outcome, "", attachments);
    assert_eq!(result.diagnoses[0].kind, DiagnosisKind::JavaHeapMemory);
    let serialized = serde_json::to_string(&result)?;
    assert!(!serialized.contains("PRIVATE-TEST-TOKEN"));
    assert!(!serialized.contains("never collect"));
    assert!(serialized.contains("Sensitive diagnostic content omitted"));
    Ok(())
}

#[test]
fn changed_existing_report_is_collected_and_large_utf8_is_bounded() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("hs_err_pid123.log");
    fs::write(&path, b"old")?;
    let baseline = snapshot_reports(directory.path())?;
    fs::write(path, "界".repeat(100_000))?;
    let attachments = collect_fresh_reports(directory.path(), &baseline, str::to_owned)?;
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].truncated);
    assert!(attachments[0].sanitized_text.len() <= 128 * 1024);
    let tail = report(&"界".repeat(100_000));
    assert!(tail.sanitized_tail.len() <= 64 * 1024);
    Ok(())
}

#[test]
fn truncated_raw_line_is_discarded_before_secret_matching() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let baseline = snapshot_reports(directory.path())?;
    let secret = "PRIVATE-TOKEN-THAT-CROSSES-THE-READ-LIMIT";
    let mut contents = "JVM fatal error\n".to_owned();
    contents.push_str(&"x".repeat(128 * 1024 - contents.len() - 8));
    contents.push_str(secret);
    fs::write(directory.path().join("hs_err_pid5.log"), contents)?;
    let attachments = collect_fresh_reports(directory.path(), &baseline, |text| {
        text.replace(secret, "[REDACTED]")
    })?;
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].truncated);
    assert_eq!(attachments[0].sanitized_text, "JVM fatal error\n");
    Ok(())
}

#[test]
fn replacing_the_game_root_invalidates_the_baseline() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let game = directory.path().join("game");
    fs::create_dir(&game)?;
    let baseline = snapshot_reports(&game)?;
    fs::rename(&game, directory.path().join("previous-game"))?;
    fs::create_dir(&game)?;
    fs::write(
        game.join("hs_err_pid6.log"),
        b"unrelated replacement directory",
    )?;
    assert!(collect_fresh_reports(&game, &baseline, str::to_owned).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_reports_directories_and_game_roots_are_rejected() -> anyhow::Result<()> {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let secret = outside.path().join("private.txt");
    fs::write(&secret, b"private contents")?;
    let baseline = snapshot_reports(directory.path())?;
    symlink(&secret, directory.path().join("hs_err_pid1.log"))?;
    symlink(outside.path(), directory.path().join("crash-reports"))?;
    fs::write(
        outside.path().join("crash-outside.txt"),
        b"private contents",
    )?;
    assert!(collect_fresh_reports(directory.path(), &baseline, str::to_owned)?.is_empty());
    let linked_game = outside.path().join("linked-game");
    symlink(directory.path(), &linked_game)?;
    assert!(snapshot_reports(&linked_game).is_err());
    assert!(collect_fresh_reports(&linked_game, &baseline, str::to_owned).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn signal_and_nonzero_status_are_preserved() {
    use std::{os::unix::process::ExitStatusExt as _, process::ExitStatus};
    let killed = ExitOutcome::from_status(ExitStatus::from_raw(9), false);
    assert_eq!(killed.kind, ExitKind::Abnormal);
    assert_eq!(killed.signal, Some(9));
    assert_eq!(killed.code, None);
    let stopped = ExitOutcome::from_status(ExitStatus::from_raw(15), true);
    assert_eq!(stopped.kind, ExitKind::Stopped);
    let normal = ExitOutcome::from_status(ExitStatus::from_raw(0), false);
    assert_eq!(normal.kind, ExitKind::Normal);
    let failed = ExitOutcome::from_status(ExitStatus::from_raw(256), false);
    assert_eq!(failed.kind, ExitKind::Abnormal);
    assert_eq!(failed.code, Some(1));
    let raced_failure = ExitOutcome::from_status(ExitStatus::from_raw(256), true);
    assert_eq!(raced_failure.kind, ExitKind::Abnormal);
    let raced_signal = ExitOutcome::from_status(ExitStatus::from_raw(11), true);
    assert_eq!(raced_signal.kind, ExitKind::Abnormal);
}

#[cfg(windows)]
#[test]
fn windows_exception_status_is_preserved_without_sign_loss() {
    use std::{os::windows::process::ExitStatusExt as _, process::ExitStatus};
    let crashed = ExitOutcome::from_status(ExitStatus::from_raw(0xc000_0005), false);
    assert_eq!(crashed.kind, ExitKind::Abnormal);
    assert_eq!(crashed.windows_status, Some(0xc000_0005));
    assert!(crashed.should_notify());
    let raced_crash = ExitOutcome::from_status(ExitStatus::from_raw(0xc000_0005), true);
    assert_eq!(raced_crash.kind, ExitKind::Abnormal);
    let stopped = ExitOutcome::from_status(ExitStatus::from_raw(1), true);
    assert_eq!(stopped.kind, ExitKind::Stopped);
}

#[test]
fn reserved_game_directory_becomes_an_actionable_session_diagnosis() {
    let mut start_failed = report("");
    start_failed.outcome.kind = ExitKind::StartFailed;
    start_failed.outcome.code = None;
    let result = CrashReport::new(
        start_failed.context,
        start_failed.outcome,
        "this game directory has a reserved session; reconnect to its supervisor or confirm the previous game has ended before recovering it",
        Vec::new(),
    );
    assert_eq!(result.diagnoses.len(), 1);
    assert_eq!(result.diagnoses[0].kind, DiagnosisKind::SessionReservation);
    assert_eq!(result.diagnoses[0].confidence, Confidence::Possible);
    assert!(result.diagnoses[0].evidence[0].contains("reserved session"));
    assert!(
        result.diagnoses[0]
            .suggested_action
            .contains("Confirm the previous Minecraft window")
    );
    assert!(
        !result.diagnoses[0]
            .suggested_action
            .contains("export diagnostics")
    );
}
