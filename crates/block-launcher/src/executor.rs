use std::{
    fmt, fs, io,
    io::Read as _,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, bail, ensure};
use block_download::Store;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio_util::sync::CancellationToken;

const RUN_ID_TOKEN: &str = "${run_id}";

use crate::{
    command::JavaCommand,
    plan::{ExecutionPlan, JavaExecutable, PrepareAction},
    platform::Architecture,
    session::SessionProvider,
};

#[derive(Debug, Clone)]
pub enum ExecutionEvent {
    Stage { name: String },
    Log { line: String, stderr: bool },
    Started { pid: u32 },
    Exited { code: Option<i32> },
    Terminated { outcome: crate::crash::ExitOutcome },
    CaptureFailed { message: String },
}

pub type ExecutionSink = Arc<dyn Fn(ExecutionEvent) + Send + Sync>;

/// Holds the instance's exclusive lock until preparation/launch finishes.
#[derive(Debug)]
pub struct PreparedInstance {
    pub plan: ExecutionPlan,
    pub root: PathBuf,
    pub run_id: String,
    _lock: fs::File,
}

#[derive(Clone)]
pub struct Executor {
    store: Store,
    events: ExecutionSink,
}

impl fmt::Debug for Executor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Executor")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl Executor {
    #[must_use]
    pub fn new(store: Store, events: ExecutionSink) -> Self {
        Self { store, events }
    }

    /// Execute a typed plan, preparing only verified objects and confined file paths.
    ///
    /// # Errors
    /// Fails on an incompatible target/JVM, unavailable resources, locks, unsafe paths, or cancellation.
    pub async fn prepare(
        &self,
        plan: ExecutionPlan,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PreparedInstance> {
        self.prepare_inner(plan, cancel, false, None).await
    }

    /// Explicitly repair declared immutable views, retaining corrupt bytes for diagnosis.
    /// # Errors
    /// Fails on locks, invalid metadata, unsafe paths or unavailable verified resources.
    pub async fn prepare_repair(
        &self,
        plan: ExecutionPlan,
        cancel: &CancellationToken,
    ) -> anyhow::Result<PreparedInstance> {
        self.prepare_inner(plan, cancel, true, None).await
    }

    /// Prepare an installed target using the supervisor's already allocated run ID.
    /// # Errors
    /// Rejects invalid IDs and the same integrity, compatibility and lock failures as preparation.
    pub async fn prepare_session(
        &self,
        plan: ExecutionPlan,
        cancel: &CancellationToken,
        run_id: &str,
    ) -> anyhow::Result<PreparedInstance> {
        ensure!(
            uuid::Uuid::parse_str(run_id)?.to_string() == run_id,
            "invalid run ID"
        );
        self.prepare_inner(plan, cancel, false, Some(run_id.to_owned()))
            .await
    }

    async fn prepare_inner(
        &self,
        plan: ExecutionPlan,
        cancel: &CancellationToken,
        repair: bool,
        run_id: Option<String>,
    ) -> anyhow::Result<PreparedInstance> {
        ensure!(
            plan.schema_version == 1,
            "unsupported execution-plan schema"
        );
        crate::support::plan_support(&plan).require()?;
        plan.target.validate_execution()?;
        let root = self.store.root().canonicalize()?;
        let instance = confined_path(&root, &format!("instances/{}", plan.instance))?;
        ensure_write_path(&root, &instance)?;
        fs::create_dir_all(&instance)?;
        ensure_write_path(&root, &instance.join("instance.lock"))?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(instance.join("instance.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("this instance is already preparing or running")?;
        if let JavaExecutable::Local { path } = &plan.command.java {
            probe_java(path, &plan).await?;
        }
        (self.events)(ExecutionEvent::Stage {
            name: "Downloading verified resources".into(),
        });
        self.store.fetch_all(&plan.resources, cancel).await?;
        ensure!(!cancel.is_cancelled(), "preparation cancelled");
        (self.events)(ExecutionEvent::Stage {
            name: "Materializing instance".into(),
        });
        let run_id = materialize_async(&plan, &root, &self.store, cancel, repair, run_id).await?;
        if let JavaExecutable::Managed { path } = &plan.command.java {
            probe_java(&confined_path(&root, path)?, &plan).await?;
        }
        let lock_path = instance.join("prepared.lock.json");
        let temporary = instance.join(format!("prepared-{run_id}.tmp"));
        fs::write(&temporary, serde_json::to_vec_pretty(&plan)?)?;
        // Removing an old marker is safe while holding the exclusive instance lock.
        if lock_path.exists() {
            fs::remove_file(&lock_path)?;
        }
        fs::rename(temporary, lock_path)?;
        (self.events)(ExecutionEvent::Stage {
            name: "Ready".into(),
        });
        Ok(PreparedInstance {
            plan,
            root,
            run_id,
            _lock: lock,
        })
    }

    /// Bind identity after preparation and supervise the actual game process.
    ///
    /// # Errors
    /// Fails on account/command binding, process/IO failures, or log-drain errors.
    pub async fn launch(
        &self,
        prepared: PreparedInstance,
        provider: &dyn SessionProvider,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Option<i32>> {
        let command =
            JavaCommand::bind(&prepared.plan, &prepared.root, &prepared.run_id, provider)?;
        let logs = confined_path(
            &prepared.root,
            &format!(
                "instances/{}/runs/{}/logs",
                prepared.plan.instance, prepared.run_id
            ),
        )?;
        ensure_write_path(&prepared.root, &logs)?;
        tokio::fs::create_dir_all(&logs).await?;
        supervise(command, logs, self.events.clone(), cancel).await
    }
}

async fn materialize_async(
    plan: &ExecutionPlan,
    root: &Path,
    store: &Store,
    cancel: &CancellationToken,
    repair: bool,
    run_id: Option<String>,
) -> anyhow::Result<String> {
    let run_id = run_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let materialization = (
        plan.clone(),
        root.to_owned(),
        run_id.clone(),
        store.clone(),
        cancel.clone(),
        repair,
    );
    tokio::task::spawn_blocking(move || {
        if materialization.5 {
            crate::repair::quarantine_corrupt_copies(
                &materialization.3,
                &materialization.0,
                &materialization.2,
                &materialization.4,
            )?;
        }
        materialize(
            &materialization.0,
            &materialization.1,
            &materialization.2,
            &materialization.3,
            &materialization.4,
        )
    })
    .await??;
    Ok(run_id)
}

pub(crate) async fn supervise(
    command: JavaCommand,
    logs: PathBuf,
    events: ExecutionSink,
    cancel: &CancellationToken,
) -> anyhow::Result<Option<i32>> {
    tokio::fs::write(
        logs.join("command.json"),
        serde_json::to_vec_pretty(&command.preview())?,
    )
    .await?;
    let mut process = command.process();
    process
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let mut child = process.spawn().context("failed to spawn Java")?;
    (events)(ExecutionEvent::Started {
        pid: child.id().context("Java process has no PID")?,
    });
    let stdout = child.stdout.take().context("missing game stdout")?;
    let stderr = child.stderr.take().context("missing game stderr")?;
    let mut out = tokio::spawn(drain(
        stdout,
        logs.join("stdout.log"),
        command.clone(),
        events.clone(),
        false,
    ));
    let mut err = tokio::spawn(drain(
        stderr,
        logs.join("stderr.log"),
        command,
        events.clone(),
        true,
    ));
    let (status, terminated) = tokio::select! {
        status = child.wait() => (status?, false),
        () = cancel.cancelled() => match child.try_wait()? {
            Some(status) => (status, false),
            None => {
                match child.kill().await {
                    Ok(()) => (child.wait().await?, true),
                    Err(error) => match child.try_wait()? {
                        Some(status) => (status, false),
                        None => return Err(error.into()),
                    },
                }
            }
        },
    };
    (events)(ExecutionEvent::Terminated {
        outcome: crate::crash::ExitOutcome::from_status(status, terminated),
    });
    let capture = tokio::time::timeout(Duration::from_secs(2), async {
        (&mut out).await??;
        (&mut err).await??;
        anyhow::Ok(())
    })
    .await;
    let failure = match capture {
        Ok(Ok(())) => None,
        Ok(Err(_)) => Some("Some game output could not be saved"),
        Err(_) => Some("Game output remained open after exit; capture was truncated"),
    };
    if let Some(message) = failure {
        out.abort();
        err.abort();
        (events)(ExecutionEvent::CaptureFailed {
            message: message.into(),
        });
    }
    (events)(ExecutionEvent::Exited {
        code: status.code(),
    });
    Ok(status.code())
}

/// Join an untrusted portable relative path, rejecting Windows escape syntax even on Unix.
///
/// # Errors
/// Fails on traversal, absolute paths, alternate separators, or drive/stream syntax.
pub fn confined_path(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    ensure!(
        !relative.is_empty() && !relative.contains(['\\', ':', '\0']),
        "unsafe relative path: {relative}"
    );
    let path = Path::new(relative);
    ensure!(
        path.components()
            .all(|part| matches!(part, Component::Normal(_))),
        "unsafe relative path: {relative}"
    );
    Ok(root.join(path))
}

pub(crate) fn ensure_write_path(root: &Path, path: &Path) -> anyhow::Result<()> {
    let relative = path.strip_prefix(root)?;
    let mut current = root.to_owned();
    for component in relative.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "refusing to write through symlink: {}",
                current.display()
            );
        }
    }
    Ok(())
}

fn materialize(
    plan: &ExecutionPlan,
    root: &Path,
    run_id: &str,
    store: &Store,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let mut links = Vec::new();
    for action in &plan.actions {
        ensure!(!cancel.is_cancelled(), "preparation cancelled");
        match action {
            PrepareAction::Copy {
                sha1,
                path,
                executable,
            } => {
                let destination = confined_path(root, &path.replace(RUN_ID_TOKEN, run_id))?;
                ensure_write_path(root, &destination)?;
                fs::create_dir_all(
                    destination
                        .parent()
                        .context("copy destination has no parent")?,
                )?;
                copy_immutable(&store.object_path(sha1)?, &destination, *executable)?;
            }
            PrepareAction::Directory { path } => {
                let path = confined_path(root, &path.replace(RUN_ID_TOKEN, run_id))?;
                ensure_write_path(root, &path)?;
                fs::create_dir_all(path)?;
            }
            PrepareAction::Extract {
                sha1,
                directory,
                excludes,
            } => {
                let directory = confined_path(root, &directory.replace(RUN_ID_TOKEN, run_id))?;
                extract(root, &store.object_path(sha1)?, &directory, excludes)?;
            }
            PrepareAction::Link {
                path,
                target,
                scope,
            } => links.push((path, target, scope)),
        }
    }
    while !links.is_empty() {
        let mut deferred = Vec::new();
        let mut progress = false;
        for (path, target, scope) in links {
            let destination = confined_path(root, path)?;
            let parent = destination.parent().context("link has no parent")?;
            if parent.join(target).exists() {
                runtime_link(root, path, target, scope)?;
                progress = true;
            } else {
                deferred.push((path, target, scope));
            }
        }
        ensure!(progress, "runtime links are cyclic or have missing targets");
        links = deferred;
    }
    Ok(())
}

fn copy_immutable(source: &Path, destination: &Path, executable: bool) -> anyhow::Result<()> {
    let expected_size = fs::metadata(source)?.len();
    if destination.exists() {
        ensure!(
            fs::metadata(destination)?.len() == expected_size
                && identical_files(source, destination)?,
            "immutable view is corrupt; choose Repair before playing: {}",
            destination.display()
        );
        return Ok(());
    }
    let directory = destination.parent().context("view file has no parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    io::copy(&mut fs::File::open(source)?, temporary.as_file_mut())?;
    set_executable(temporary.path(), executable)?;
    // Windows readonly attributes also prevent deleting a losing publication's
    // temporary file. The no-clobber operation provides the publication guarantee.
    #[cfg(unix)]
    {
        let mut permissions = temporary.as_file().metadata()?.permissions();
        permissions.set_readonly(true);
        temporary.as_file().set_permissions(permissions)?;
    }
    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            ensure!(
                fs::metadata(destination)?.len() == expected_size
                    && identical_files(source, destination)?,
                "concurrent immutable view is corrupt; choose Repair"
            );
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn identical_files(left: &Path, right: &Path) -> anyhow::Result<bool> {
    let mut left = io::BufReader::new(fs::File::open(left)?);
    let mut right = io::BufReader::new(fs::File::open(right)?);
    let mut a = vec![0_u8; 65536];
    let mut b = vec![0_u8; 65536];
    loop {
        let count = left.read(&mut a)?;
        let mut other = 0;
        while other < count {
            let read = right.read(&mut b[other..count])?;
            if read == 0 {
                return Ok(false);
            }
            other += read;
        }
        if a[..count] != b[..count] {
            return Ok(false);
        }
        if count == 0 {
            return Ok(right.read(&mut b[..1])? == 0);
        }
    }
}

fn extract(
    root: &Path,
    source: &Path,
    directory: &Path,
    excludes: &[String],
) -> anyhow::Result<()> {
    ensure_write_path(root, directory)?;
    fs::create_dir_all(directory)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(source)?)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if excludes
            .iter()
            .any(|prefix| entry.name().starts_with(prefix))
        {
            continue;
        }
        let relative = entry
            .enclosed_name()
            .context("native archive contains an escape path")?;
        let destination = confined_path(
            directory,
            relative.to_str().context("native filename is not UTF-8")?,
        )?;
        ensure_write_path(root, &destination)?;
        ensure!(
            entry
                .unix_mode()
                .is_none_or(|mode| mode & 0o170_000 != 0o120_000),
            "native archive contains a symlink"
        );
        if entry.is_dir() {
            fs::create_dir_all(destination)?;
        } else {
            fs::create_dir_all(destination.parent().context("native file has no parent")?)?;
            io::copy(&mut entry, &mut fs::File::create(destination)?)?;
        }
    }
    Ok(())
}

fn runtime_link(root: &Path, path: &str, target: &str, scope: &str) -> anyhow::Result<()> {
    let path = confined_path(root, path)?;
    let scope = confined_path(root, scope)?;
    ensure!(
        !Path::new(target).is_absolute() && !target.contains(['\\', ':']),
        "unsafe runtime link target"
    );
    let mut resolved = path.parent().context("link has no parent")?.to_path_buf();
    for component in Path::new(target).components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::ParentDir => {
                ensure!(resolved.pop(), "runtime link escaped root");
            }
            Component::CurDir => {}
            _ => bail!("invalid runtime link"),
        }
    }
    ensure!(
        resolved.starts_with(&scope),
        "runtime link escapes its installation"
    );
    ensure!(
        resolved.canonicalize()?.starts_with(scope.canonicalize()?),
        "runtime link resolves outside its installation"
    );
    ensure_write_path(root, path.parent().context("link has no parent")?)?;
    fs::create_dir_all(path.parent().context("link has no parent")?)?;
    if let Ok(previous) = fs::read_link(&path) {
        ensure!(
            previous == Path::new(target),
            "runtime link changed unexpectedly"
        );
        return Ok(());
    }
    #[cfg(unix)]
    if let Err(error) = std::os::unix::fs::symlink(target, &path) {
        ensure!(
            error.kind() == io::ErrorKind::AlreadyExists
                && fs::read_link(&path)? == Path::new(target),
            "could not publish runtime link: {error}"
        );
    }
    #[cfg(windows)]
    {
        ensure!(
            resolved.is_file(),
            "runtime directory links require a supported Windows runtime"
        );
        copy_immutable(&resolved, &path, false)?;
    }
    Ok(())
}

fn set_executable(path: &Path, executable: bool) -> anyhow::Result<()> {
    ensure!(
        fs::metadata(path)?.is_file(),
        "executable target must be a regular file"
    );
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = (path, executable);
    Ok(())
}

async fn probe_java(path: &Path, plan: &ExecutionPlan) -> anyhow::Result<()> {
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(path)
            .args(["-XshowSettings:properties", "-version"])
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    ensure!(
        output.status.success(),
        "Java probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let properties = String::from_utf8_lossy(&output.stderr);
    let property = |key: &str| {
        properties
            .lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix(key))
            .map(str::trim)
    };
    let version = property("java.version =").context("Java did not report its version")?;
    let major = version
        .strip_prefix("1.")
        .unwrap_or(version)
        .split(['.', '-', '_'])
        .next()
        .context("invalid Java version")?
        .parse::<u32>()?;
    ensure!(
        major == plan.command.java_major,
        "this version requires Java {}, selected Java is {version}",
        plan.command.java_major
    );
    let arch = property("os.arch =").context("Java did not report its architecture")?;
    let matches = match plan.target.arch {
        Architecture::Aarch64 => arch == "aarch64" || arch == "arm64",
        Architecture::X86_64 => arch == "amd64" || arch == "x86_64",
        Architecture::X86 => arch == "x86" || arch == "i386",
    };
    ensure!(
        matches,
        "Java architecture {arch} does not match {:?}",
        plan.target.arch
    );
    Ok(())
}

#[cfg(test)]
mod tests;

async fn drain(
    reader: impl tokio::io::AsyncRead + Unpin,
    path: PathBuf,
    command: JavaCommand,
    events: ExecutionSink,
    stderr: bool,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut capture = OutputCapture::create(path, &events, stderr).await;
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let buffer = reader
            .fill_buf()
            .await
            .map_err(|_| anyhow::anyhow!("Game output could not be read"))?;
        if buffer.is_empty() {
            if !oversized && !line.is_empty() {
                emit_output(&line, &command, &mut capture, &events, stderr).await;
            }
            break;
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let bytes = newline.unwrap_or(buffer.len());
        let newly_oversized =
            !oversized && bytes > MAX_OUTPUT_LINE_BYTES.saturating_sub(line.len());
        if newly_oversized {
            line.clear();
            oversized = true;
        } else if !oversized {
            line.extend_from_slice(&buffer[..bytes]);
        }
        reader.consume(newline.map_or(bytes, |position| position + 1));
        if newly_oversized {
            emit_output(
                b"[Block] An oversized game output line was discarded.",
                &command,
                &mut capture,
                &events,
                stderr,
            )
            .await;
        }
        if newline.is_some() {
            if !oversized {
                emit_output(&line, &command, &mut capture, &events, stderr).await;
            }
            line.clear();
            oversized = false;
        }
    }
    capture.finish(&events, stderr).await;
    Ok(())
}

const MAX_OUTPUT_LINE_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_FILE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
struct OutputCapture {
    file: Option<tokio::fs::File>,
    written: usize,
    quota_reached: bool,
}

impl OutputCapture {
    async fn create(path: PathBuf, events: &ExecutionSink, stderr: bool) -> Self {
        let file = tokio::fs::File::create(path).await.ok();
        if file.is_none() {
            capture_failure(events, stderr);
        }
        Self {
            file,
            written: 0,
            quota_reached: false,
        }
    }

    async fn write(&mut self, line: &str, events: &ExecutionSink, stderr: bool) {
        if self.quota_reached {
            return;
        }
        let Some(file) = &mut self.file else { return };
        let bytes = line.len().saturating_add(1);
        if bytes > MAX_OUTPUT_FILE_BYTES.saturating_sub(self.written) {
            self.quota_reached = true;
            let stream = if stderr {
                "Standard error"
            } else {
                "Standard output"
            };
            (events)(ExecutionEvent::CaptureFailed {
                message: format!(
                    "{stream} reached its 16 MiB capture limit; live monitoring continues."
                ),
            });
            return;
        }
        let result = async {
            file.write_all(line.as_bytes()).await?;
            file.write_all(b"\n").await
        }
        .await;
        if result.is_err() {
            self.file = None;
            capture_failure(events, stderr);
        } else {
            self.written += bytes;
        }
    }

    async fn finish(&mut self, events: &ExecutionSink, stderr: bool) {
        if let Some(file) = &mut self.file
            && file.flush().await.is_err()
        {
            self.file = None;
            capture_failure(events, stderr);
        }
    }
}

async fn emit_output(
    bytes: &[u8],
    command: &JavaCommand,
    capture: &mut OutputCapture,
    events: &ExecutionSink,
    stderr: bool,
) {
    let line = command.redact(&String::from_utf8_lossy(bytes));
    capture.write(&line, events, stderr).await;
    (events)(ExecutionEvent::Log { line, stderr });
}

fn capture_failure(events: &ExecutionSink, stderr: bool) {
    let stream = if stderr {
        "Standard error"
    } else {
        "Standard output"
    };
    (events)(ExecutionEvent::CaptureFailed {
        message: format!("{stream} could not be saved; live monitoring continues."),
    });
}
