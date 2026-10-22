use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, ensure};
use serde::{Serialize, de::DeserializeOwned};

use super::{RunRecord, validate_id};
use crate::{crash::CrashReport, executor::ensure_write_path};

const MAX_RECORD_BYTES: u64 = 256 * 1024;
// Bounded decoded text may expand sixfold when JSON escapes control characters.
const MAX_REPORT_BYTES: u64 = 8 * 1024 * 1024;

/// Locate a run inside this library without accepting path aliases from metadata.
/// # Errors
/// Rejects noncanonical IDs and symlinked session paths.
pub fn report_directory(root: &Path, run_id: &str) -> anyhow::Result<PathBuf> {
    validate_id(run_id)?;
    let root = root.canonicalize()?;
    let path = root.join("sessions").join(run_id);
    ensure_write_path(&root, &path)?;
    Ok(path)
}

pub(super) fn create_directory(root: &Path, run_id: &str) -> anyhow::Result<PathBuf> {
    let path = report_directory(root, run_id)?;
    fs::create_dir_all(path.parent().context("missing sessions parent")?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        fs::DirBuilder::new().mode(0o700).create(&path)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(&path)?;
    Ok(path)
}

fn read_json<T: DeserializeOwned>(path: &Path, limit: u64) -> anyhow::Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= limit,
        "invalid session document"
    );
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        u64::try_from(bytes.len())? <= limit,
        "session document is too large"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

pub(super) fn write_json(path: &Path, document: &impl Serialize) -> anyhow::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "invalid session document destination"
        );
    }
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("missing document parent")?)?;
    serde_json::to_writer_pretty(file.as_file_mut(), document)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    persist_with_retry(file, path)
}

/// Replace the destination file, retrying transient Windows sharing denials.
///
/// The launcher polls the session document while the supervisor rewrites it,
/// and antivirus/indexer handles can briefly deny the atomic replace even
/// though the destination is owned by this process. Retrying a bounded number
/// of times keeps that transient denial from losing a record update.
fn persist_with_retry(mut file: tempfile::NamedTempFile, path: &Path) -> anyhow::Result<()> {
    let mut delay = Duration::from_millis(20);
    let mut attempt = 0;
    loop {
        match file.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) if attempt < 5 && retryable_persist_error(&error.error) => {
                file = error.file;
                attempt += 1;
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(200));
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        }
    }
}

fn retryable_persist_error(error: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        // ERROR_ACCESS_DENIED and ERROR_SHARING_VIOLATION are transient while
        // another process has the destination open for inspection.
        matches!(error.raw_os_error(), Some(5) | Some(32))
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}

pub(super) fn write_record(root: &Path, record: &RunRecord) -> anyhow::Result<()> {
    write_json(
        &report_directory(root, &record.context.run_id)?.join("session.json"),
        record,
    )
}

pub(super) fn write_report(root: &Path, report: &CrashReport) -> anyhow::Result<()> {
    write_json(
        &report_directory(root, &report.context.run_id)?.join("crash.json"),
        report,
    )
}

/// Read a secret-free run record.
/// # Errors
/// Rejects malformed, oversized, mismatched or symlinked documents.
pub fn read_run(root: &Path, run_id: &str) -> anyhow::Result<RunRecord> {
    let record: RunRecord = read_json(
        &report_directory(root, run_id)?.join("session.json"),
        MAX_RECORD_BYTES,
    )?;
    ensure!(
        record.schema_version == 1 && record.context.run_id == run_id,
        "session document identity mismatch"
    );
    Ok(record)
}

/// Read the sanitized report produced by the session owner.
/// # Errors
/// Rejects malformed, oversized, mismatched or symlinked reports.
pub fn read_crash(root: &Path, run_id: &str) -> anyhow::Result<CrashReport> {
    let report: CrashReport = read_json(
        &report_directory(root, run_id)?.join("crash.json"),
        MAX_REPORT_BYTES,
    )?;
    ensure!(
        report.schema_version == 1 && report.context.run_id == run_id,
        "crash report identity mismatch"
    );
    Ok(report)
}

/// Enumerate recent session metadata; never silently ignore an unbounded directory.
/// # Errors
/// Fails on unreadable metadata or an excessive number of session entries.
pub fn list_runs(root: &Path) -> anyhow::Result<Vec<RunRecord>> {
    let root = root.canonicalize()?;
    let parent = root.join("sessions");
    ensure_write_path(&root, &parent)?;
    if !parent.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for (index, entry) in fs::read_dir(parent)?.enumerate() {
        ensure!(
            index < 4096,
            "too many session records; archive older diagnostics before continuing"
        );
        let entry = entry?;
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_id(&id).is_err() || !entry.file_type()?.is_dir() {
            continue;
        }
        // A partial document may exist while a new supervisor is bootstrapping.
        if let Ok(record) = read_run(&root, &id) {
            records.push(record);
        }
    }
    records.sort_by_key(|record| std::cmp::Reverse(record.started_at_unix_ms));
    Ok(records)
}

/// Claim presentation for an unread report. Hold the file until the help view closes.
/// # Errors
/// Fails on malformed paths or filesystem errors; another presenter returns `None`.
pub fn claim_help(root: &Path, run_id: &str) -> anyhow::Result<Option<File>> {
    let directory = report_directory(root, run_id)?;
    if directory.join("seen.json").exists() || !directory.join("crash.json").exists() {
        return Ok(None);
    }
    let path = directory.join("presentation.lock");
    ensure_write_path(&root.canonicalize()?, &path)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) if !directory.join("seen.json").exists() => Ok(Some(file)),
        Ok(()) => Ok(None),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Acknowledge a help view only after the view was successfully constructed.
/// # Errors
/// Returns filesystem failures without removing the report.
pub fn mark_seen(root: &Path, run_id: &str) -> anyhow::Result<()> {
    write_json(
        &report_directory(root, run_id)?.join("seen.json"),
        &serde_json::json!({"schema_version":1,"run_id":run_id}),
    )
}

#[cfg(test)]
mod tests {
    use crate::crash::{
        AttachmentKind, CrashContext, CrashReport, DiagnosticAttachment, ExitOutcome,
    };

    #[test]
    fn transient_windows_persist_errors_are_retried() {
        #[cfg(windows)]
        {
            assert!(super::retryable_persist_error(
                &std::io::Error::from_raw_os_error(5)
            ));
            assert!(super::retryable_persist_error(
                &std::io::Error::from_raw_os_error(32)
            ));
            assert!(!super::retryable_persist_error(
                &std::io::Error::from_raw_os_error(2)
            ));
        }
        #[cfg(not(windows))]
        {
            assert!(!super::retryable_persist_error(
                &std::io::Error::from_raw_os_error(5)
            ));
        }
    }

    #[test]
    fn bounded_control_character_reports_remain_readable_after_json_expansion() -> anyhow::Result<()>
    {
        let root = tempfile::tempdir()?;
        let id = uuid::Uuid::new_v4().to_string();
        super::create_directory(root.path(), &id)?;
        let report = CrashReport::new(
            CrashContext {
                run_id: id.clone(),
                profile_id: "fixture".into(),
                profile_name: "Fixture".into(),
                minecraft_version: "1.20.6".into(),
                loader: None,
            },
            ExitOutcome::start_failed(),
            &"\0".repeat(64 * 1024),
            (0..8)
                .map(|index| DiagnosticAttachment {
                    name: format!("crash-{index}.txt"),
                    kind: AttachmentKind::MinecraftCrashReport,
                    sanitized_text: "\0".repeat(128 * 1024),
                    truncated: false,
                })
                .collect(),
        );
        super::write_report(root.path(), &report)?;
        let read = super::read_crash(root.path(), &id)?;
        assert_eq!(read.attachments.len(), 8);
        assert_eq!(
            read.attachments[7].sanitized_text,
            report.attachments[7].sanitized_text
        );
        assert_eq!(read.sanitized_tail, report.sanitized_tail);
        Ok(())
    }
}
