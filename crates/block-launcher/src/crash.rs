//! Bounded, local crash explanations. Callers redact secrets before supplying text.

use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata},
    io::Read as _,
    path::{Path, PathBuf},
    process::ExitStatus,
    time::SystemTime,
};

use anyhow::{Context as _, ensure};

use serde::{Deserialize, Serialize};

const MAX_TAIL_BYTES: usize = 64 * 1024;
const MAX_EVIDENCE_BYTES: usize = 400;
const MAX_ATTACHMENT_BYTES: usize = 128 * 1024;
const MAX_ATTACHMENTS: usize = 8;
const MAX_DIRECTORY_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitKind {
    Normal,
    Stopped,
    StartFailed,
    Abnormal,
    MonitoringInterrupted,
}

/// Facts about termination, independent of any diagnosis inferred from logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitOutcome {
    pub kind: ExitKind,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub windows_status: Option<u32>,
}

impl ExitOutcome {
    #[must_use]
    pub const fn should_notify(&self) -> bool {
        matches!(self.kind, ExitKind::StartFailed | ExitKind::Abnormal)
    }

    /// Preserve native status. A stop request alone cannot explain an unrelated crash.
    /// Set `terminated_by_supervisor` only when the executor actually initiated termination.
    #[must_use]
    pub fn from_status(status: ExitStatus, terminated_by_supervisor: bool) -> Self {
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt as _;
            status.signal()
        };
        #[cfg(not(unix))]
        let signal = None;
        #[cfg(windows)]
        let windows_status = status
            .code()
            .map(|code| u32::from_ne_bytes(code.to_ne_bytes()));
        #[cfg(not(windows))]
        let windows_status = None;

        #[cfg(unix)]
        let stop_status = matches!(signal, Some(9 | 15));
        #[cfg(windows)]
        let stop_status = status.code() == Some(1);
        #[cfg(not(any(unix, windows)))]
        let stop_status = false;

        let kind = if status.success() {
            ExitKind::Normal
        } else if terminated_by_supervisor && stop_status {
            ExitKind::Stopped
        } else {
            ExitKind::Abnormal
        };
        Self {
            kind,
            code: status.code(),
            signal,
            windows_status,
        }
    }

    #[must_use]
    pub const fn start_failed() -> Self {
        Self {
            kind: ExitKind::StartFailed,
            code: None,
            signal: None,
            windows_status: None,
        }
    }

    #[must_use]
    pub const fn monitoring_interrupted() -> Self {
        Self {
            kind: ExitKind::MonitoringInterrupted,
            code: None,
            signal: None,
            windows_status: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashContext {
    pub run_id: String,
    pub profile_id: String,
    pub profile_name: String,
    pub minecraft_version: String,
    pub loader: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisKind {
    JavaHeapMemory,
    NativeMemory,
    JavaVersion,
    ModDependencies,
    GraphicsInitialization,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Possible,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    pub kind: DiagnosisKind,
    pub confidence: Confidence,
    pub title: String,
    pub evidence: Vec<String>,
    pub suggested_action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    MinecraftCrashReport,
    JvmFatalLog,
}

/// Only sanitized text belongs here. Never include an absolute source path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticAttachment {
    pub name: String,
    pub kind: AttachmentKind,
    pub sanitized_text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashReport {
    pub schema_version: u32,
    pub context: CrashContext,
    pub outcome: ExitOutcome,
    pub summary: String,
    pub diagnoses: Vec<Diagnosis>,
    pub sanitized_tail: String,
    pub attachments: Vec<DiagnosticAttachment>,
}

impl CrashReport {
    /// Build a bounded explanation from already redacted output and attachments.
    ///
    /// Secret redaction must precede this call, including metadata and filenames.
    /// This module deliberately receives no account, command line, or game session.
    #[must_use]
    pub fn new(
        context: CrashContext,
        outcome: ExitOutcome,
        sanitized_tail: &str,
        mut attachments: Vec<DiagnosticAttachment>,
    ) -> Self {
        attachments.truncate(MAX_ATTACHMENTS);
        for attachment in &mut attachments {
            attachment.sanitized_text = sanitize_diagnostic_text(&attachment.sanitized_text);
            if attachment.sanitized_text.len() > MAX_ATTACHMENT_BYTES {
                attachment.sanitized_text =
                    prefix(&attachment.sanitized_text, MAX_ATTACHMENT_BYTES);
                attachment.truncated = true;
            }
            attachment.name = prefix(&sanitize_diagnostic_text(&attachment.name), 160);
        }
        let sanitized_tail = tail(&sanitize_diagnostic_text(sanitized_tail), MAX_TAIL_BYTES);
        let diagnoses = if matches!(outcome.kind, ExitKind::Abnormal | ExitKind::StartFailed) {
            diagnose(&sanitized_tail, &attachments)
        } else {
            Vec::new()
        };
        let summary = match outcome.kind {
            ExitKind::Normal => "Minecraft closed normally.",
            ExitKind::Stopped => "Minecraft was stopped.",
            ExitKind::StartFailed => "Minecraft could not start.",
            ExitKind::Abnormal => "Minecraft closed unexpectedly.",
            ExitKind::MonitoringInterrupted => {
                "Game monitoring was interrupted. Minecraft may still be running."
            }
        }
        .into();
        Self {
            schema_version: 1,
            context,
            outcome,
            summary,
            diagnoses,
            sanitized_tail,
            attachments,
        }
    }
}

/// Conservatively remove credential-bearing lines and JVM argument/environment sections.
///
/// Apply known-secret replacement first. This second defense does not attempt to
/// recognize arbitrary passwords printed without a label or other identifying context.
#[must_use]
pub fn sanitize_diagnostic_text(text: &str) -> String {
    let mut result = String::new();
    let mut sensitive_section = false;
    let mut omitted = false;
    for line in text.split_inclusive('\n') {
        let lower = line.trim().to_ascii_lowercase();
        let heading = sensitive_heading(&lower);
        if heading {
            sensitive_section = true;
        } else if lower.is_empty() || public_section_heading(&lower) {
            sensitive_section = false;
        }
        let credential = credential_line(&lower);
        if credential && lower.ends_with([':', '=']) {
            sensitive_section = true;
        }
        let failure_evidence = RULES
            .iter()
            .any(|rule| rule.3.iter().any(|needle| lower.contains(needle)));
        if credential || (sensitive_section && (heading || !failure_evidence)) {
            if !omitted {
                result.push_str("[Sensitive diagnostic content omitted]\n");
            }
            omitted = true;
        } else {
            result.push_str(line);
            omitted = false;
        }
    }
    result
}

fn sensitive_heading(lower: &str) -> bool {
    let lower = lower.trim_start_matches('#').trim_start();
    [
        "command line:",
        "commandline:",
        "command-line:",
        "vm arguments:",
        "jvm arguments:",
        "java arguments:",
        "environment variables:",
        "environmentvariables:",
        "environment:",
    ]
    .iter()
    .any(|heading| lower.starts_with(heading))
        || [
            "jvm_args:",
            "java_command:",
            "java command:",
            "java_class_path",
        ]
        .iter()
        .any(|heading| lower.starts_with(heading))
}

fn public_section_heading(lower: &str) -> bool {
    [
        "memory:",
        "heap:",
        "native memory tracking:",
        "signal handlers:",
        "dynamic libraries:",
        "---------------",
        "[global flags]",
    ]
    .iter()
    .any(|heading| lower.starts_with(heading))
}

fn credential_line(lower: &str) -> bool {
    [
        "token",
        "accesstoken",
        "refreshtoken",
        "idtoken",
        "password",
        "passwd",
        "secret",
        "clientsecret",
        "authorization",
        "cookie",
        "apikey",
        "api_key",
        "api-key",
        "bearer",
        "sessionid",
        "session_id",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
}

fn diagnose(tail: &str, attachments: &[DiagnosticAttachment]) -> Vec<Diagnosis> {
    let mut diagnoses = Vec::new();
    for (kind, title, action, needles) in RULES {
        let mut evidence = Vec::new();
        for line in std::iter::once(tail)
            .chain(
                attachments
                    .iter()
                    .map(|attachment| attachment.sanitized_text.as_str()),
            )
            .flat_map(str::lines)
        {
            let lower = line.to_ascii_lowercase();
            if needles.iter().any(|needle| lower.contains(needle)) {
                let snippet = prefix(line.trim(), MAX_EVIDENCE_BYTES);
                if !evidence.contains(&snippet) {
                    evidence.push(snippet);
                }
                if evidence.len() == 3 {
                    break;
                }
            }
        }
        if !evidence.is_empty() {
            diagnoses.push(Diagnosis {
                kind: *kind,
                confidence: Confidence::Possible,
                title: (*title).into(),
                evidence,
                suggested_action: (*action).into(),
            });
        }
    }
    if diagnoses.is_empty() {
        diagnoses.push(Diagnosis {
            kind: DiagnosisKind::Unknown,
            confidence: Confidence::Unknown,
            title: "The cause is not yet known".into(),
            evidence: Vec::new(),
            suggested_action: "Review the game logs or export diagnostics for help. An exit code alone does not identify the cause.".into(),
        });
    }
    diagnoses
}

type Rule = (
    DiagnosisKind,
    &'static str,
    &'static str,
    &'static [&'static str],
);

const RULES: &[Rule] = &[
    (
        DiagnosisKind::JavaHeapMemory,
        "Java heap memory may have been exhausted",
        "Review the profile's memory limit and available system memory. Reduce demanding mods or resource packs before increasing the heap limit.",
        &[
            "outofmemoryerror: java heap space",
            "outofmemoryerror: gc overhead limit exceeded",
        ],
    ),
    (
        DiagnosisKind::NativeMemory,
        "The Java process may have exhausted native memory",
        "Close other memory-heavy applications and review available system memory. A larger Java heap can leave less memory for the operating system and native libraries.",
        &[
            "native memory allocation (malloc) failed",
            "native memory allocation (mmap) failed",
            "insufficient memory for the java runtime environment",
            "outofmemoryerror: unable to create native thread",
            "outofmemoryerror: direct buffer memory",
        ],
    ),
    (
        DiagnosisKind::JavaVersion,
        "The selected Java version may be incompatible",
        "Use the Java version required by this Minecraft version and its mod loader. Review the reported class-file version before changing Java.",
        &[
            "unsupportedclassversionerror",
            "unsupported class file major version",
            "has been compiled by a more recent version of the java runtime",
            "unrecognized vm option",
        ],
    ),
    (
        DiagnosisKind::ModDependencies,
        "Mods may have missing or incompatible dependencies",
        "Review the loader's dependency details and use mods built for this Minecraft version and loader. Back up the instance before changing mods.",
        &[
            "incompatible mods found!",
            "modresolutionexception",
            "mod resolution encountered an incompatible mod set",
            "missing or unsupported mandatory dependencies",
            "missing mandatory dependencies",
            "mandatory dependencies are not loaded",
            "mod loading has failed",
            "modloadingexception",
        ],
    ),
    (
        DiagnosisKind::GraphicsInitialization,
        "Graphics initialization may have failed",
        "Review graphics-driver support and the required OpenGL version. In a virtual machine, check its graphics configuration and host driver.",
        &[
            "glfw error 65542",
            "glfw error 65543",
            "failed to create the glfw window",
            "failed to create window: opengl",
            "pixel format not accelerated",
            "no opengl context found",
            "could not create opengl context",
        ],
    ),
];

fn prefix(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].into()
}

fn tail(text: &str, max_bytes: usize) -> String {
    let mut start = text.len().saturating_sub(max_bytes);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportFingerprint {
    length: u64,
    created: Option<SystemTime>,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl ReportFingerprint {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Self {
            length: metadata.len(),
            created: metadata.created().ok(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        }
    }

    #[cfg(unix)]
    const fn same_identity(&self, other: &Self) -> bool {
        self.device == other.device && self.inode == other.inode
    }

    #[cfg(not(unix))]
    fn same_identity(&self, other: &Self) -> bool {
        self.created == other.created
    }
}

/// Files observed before Java starts; unchanged reports are never attributed to a new run.
#[derive(Debug, Clone)]
pub struct ReportBaseline {
    root: PathBuf,
    root_identity: ReportFingerprint,
    files: BTreeMap<PathBuf, ReportFingerprint>,
}

/// Snapshot only conventional immediate Minecraft/JVM report locations.
///
/// # Errors
/// Rejects a symlinked game directory or an unreadable report directory. Callers
/// should record collection failure rather than silently use an empty baseline.
pub fn snapshot_reports(game_directory: &Path) -> anyhow::Result<ReportBaseline> {
    ensure!(
        !fs::symlink_metadata(game_directory)?
            .file_type()
            .is_symlink(),
        "game directory must not be a symlink"
    );
    let root = game_directory.canonicalize()?;
    let root_identity = ReportFingerprint::from_metadata(&fs::metadata(&root)?);
    let mut files = BTreeMap::new();
    for (path, _) in candidates(&root)? {
        if let Ok(metadata) = safe_metadata(&root, &path) {
            files.insert(path, ReportFingerprint::from_metadata(&metadata));
        }
    }
    Ok(ReportBaseline {
        root,
        root_identity,
        files,
    })
}

/// Read a bounded selection of new or changed reports and redact before returning text.
///
/// Keep the game directory locked until collection finishes. Files and their immediate
/// parent directories must be ordinary files/directories inside the original game root.
/// These portable metadata/canonical-path checks reject symlinks, but are not an atomic
/// no-follow filesystem API against a concurrently malicious local process.
///
/// # Errors
/// Rejects a changed game root or an unreadable report directory. Individual unsafe,
/// replaced, deleted, or unchanged reports are skipped; raw contents are never returned.
pub fn collect_fresh_reports(
    game_directory: &Path,
    baseline: &ReportBaseline,
    redact: impl Fn(&str) -> String,
) -> anyhow::Result<Vec<DiagnosticAttachment>> {
    ensure!(
        !fs::symlink_metadata(game_directory)?
            .file_type()
            .is_symlink(),
        "game directory must not be a symlink"
    );
    ensure!(
        game_directory.canonicalize()? == baseline.root,
        "game directory changed during the session"
    );
    ensure!(
        baseline
            .root_identity
            .same_identity(&ReportFingerprint::from_metadata(&fs::metadata(
                &baseline.root
            )?)),
        "game directory was replaced during the session"
    );
    let mut fresh = candidates(&baseline.root)?
        .into_iter()
        .filter_map(|(path, kind)| {
            let metadata = safe_metadata(&baseline.root, &path).ok()?;
            let fingerprint = ReportFingerprint::from_metadata(&metadata);
            if baseline.files.get(&path) == Some(&fingerprint) {
                return None;
            }
            Some((path, kind, fingerprint))
        })
        .collect::<Vec<_>>();
    fresh.sort_by_key(|entry| std::cmp::Reverse(entry.2.modified));
    let mut attachments = Vec::new();
    for (path, kind, fingerprint) in fresh {
        if let Ok(attachment) = read_attachment(&baseline.root, &path, kind, &fingerprint, &redact)
        {
            attachments.push(attachment);
            if attachments.len() == MAX_ATTACHMENTS {
                break;
            }
        }
    }
    Ok(attachments)
}

fn candidates(root: &Path) -> anyhow::Result<Vec<(PathBuf, AttachmentKind)>> {
    let mut result = Vec::new();
    for (index, entry) in fs::read_dir(root)?.enumerate() {
        ensure!(
            index < MAX_DIRECTORY_ENTRIES,
            "too many game directory entries for bounded report collection"
        );
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("hs_err_pid") && name.ends_with(".log") {
            result.push((entry.path(), AttachmentKind::JvmFatalLog));
        }
    }
    let reports = root.join("crash-reports");
    match fs::symlink_metadata(&reports) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            for (index, entry) in fs::read_dir(reports)?.enumerate() {
                ensure!(
                    index < MAX_DIRECTORY_ENTRIES,
                    "too many crash-report directory entries for bounded collection"
                );
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("crash-") && name.ends_with(".txt") {
                    result.push((entry.path(), AttachmentKind::MinecraftCrashReport));
                }
            }
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect Minecraft crash-report directory"),
    }
    Ok(result)
}

fn safe_metadata(root: &Path, path: &Path) -> anyhow::Result<Metadata> {
    let parent = path.parent().context("report has no parent")?;
    let parent_metadata = fs::symlink_metadata(parent)?;
    ensure!(
        parent_metadata.is_dir() && !parent_metadata.file_type().is_symlink(),
        "report parent is not an ordinary directory"
    );
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "report is not an ordinary file"
    );
    ensure!(
        path.canonicalize()?.starts_with(root),
        "report escaped game directory"
    );
    Ok(metadata)
}

fn read_attachment(
    root: &Path,
    path: &Path,
    kind: AttachmentKind,
    fingerprint: &ReportFingerprint,
    redact: &impl Fn(&str) -> String,
) -> anyhow::Result<DiagnosticAttachment> {
    ensure!(
        ReportFingerprint::from_metadata(&safe_metadata(root, path)?) == *fingerprint,
        "report changed before opening"
    );
    let file = File::open(path)?;
    ensure!(
        ReportFingerprint::from_metadata(&file.metadata()?) == *fingerprint,
        "opened report does not match candidate"
    );
    let mut bytes = Vec::new();
    file.take((MAX_ATTACHMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        ReportFingerprint::from_metadata(&safe_metadata(root, path)?) == *fingerprint,
        "report changed while reading"
    );
    let truncated = bytes.len() > MAX_ATTACHMENT_BYTES;
    bytes.truncate(MAX_ATTACHMENT_BYTES);
    // Avoid handing a redactor half of a credential-bearing final line. Matching a
    // known full token cannot sanitize an arbitrarily clipped prefix of that token.
    if truncated {
        let last_complete_line = bytes.iter().rposition(|byte| *byte == b'\n');
        bytes.truncate(last_complete_line.map_or(0, |position| position + 1));
    }
    let sanitized_text = redact(&String::from_utf8_lossy(&bytes));
    let name = path
        .file_name()
        .context("report has no filename")?
        .to_string_lossy();
    Ok(DiagnosticAttachment {
        name: prefix(&redact(&name), 160),
        kind,
        truncated: truncated || sanitized_text.len() > MAX_ATTACHMENT_BYTES,
        sanitized_text: prefix(&sanitized_text, MAX_ATTACHMENT_BYTES),
    })
}
