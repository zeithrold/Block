use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

use crate::files::{Limits, SegmentWriter};

const QUEUE_CAPACITY: usize = 4096;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub(crate) enum Problem {
    Initialization = 1,
    Storage = 2,
    Loss = 4,
}

impl Problem {
    fn message(self) -> &'static str {
        match self {
            Self::Initialization => "Could not initialize local logging.",
            Self::Storage => "Local logging stopped because storage failed.",
            Self::Loss => "Some local log events could not be recorded.",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Failure(Arc<AtomicU8>);

impl Failure {
    pub(crate) fn report(&self, problem: Problem) {
        let bit = problem as u8;
        self.0.fetch_or(bit, Ordering::Relaxed);
    }

    pub(crate) fn message(&self) -> Option<&'static str> {
        let problems = self.0.load(Ordering::Relaxed);
        if problems & Problem::Storage as u8 != 0 {
            Some(Problem::Storage.message())
        } else if problems & Problem::Initialization as u8 != 0 {
            Some(Problem::Initialization.message())
        } else if problems & Problem::Loss as u8 != 0 {
            Some(Problem::Loss.message())
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct Record {
    bytes: Vec<u8>,
    is_error: bool,
}

#[derive(Debug)]
struct Queue {
    sender: Mutex<Option<SyncSender<Record>>>,
    dropped: AtomicU64,
    failure: Failure,
    max_record_bytes: u64,
}

impl Queue {
    fn send(&self, bytes: &[u8], is_error: bool) {
        let result = if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > self.max_record_bytes {
            Some(false)
        } else {
            let record = Record {
                bytes: bytes.to_vec(),
                is_error,
            };
            let sender = self
                .sender
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            sender.as_ref().map(|sender| match sender.try_send(record) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => false,
                Err(TrySendError::Disconnected(_)) => {
                    self.failure.report(Problem::Storage);
                    false
                }
            })
        };
        if result == Some(false) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.failure.report(Problem::Loss);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EventWriterFactory(Arc<Queue>);

#[derive(Debug)]
pub(crate) struct EventWriter {
    queue: Arc<Queue>,
    is_error: bool,
}

impl io::Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.queue.send(bytes, self.is_error);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for EventWriterFactory {
    type Writer = EventWriter;

    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            queue: self.0.clone(),
            is_error: false,
        }
    }

    fn make_writer_for(&'a self, metadata: &Metadata<'_>) -> Self::Writer {
        EventWriter {
            queue: self.0.clone(),
            is_error: *metadata.level() == Level::ERROR,
        }
    }
}

/// Owns the background writer. Dropping it stops acceptance, drains queued
/// events, flushes the file, and joins the writer thread.
#[must_use]
#[derive(Debug)]
pub struct LogGuard {
    queue: Arc<Queue>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        self.queue
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if self
            .thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            self.queue.failure.report(Problem::Storage);
        }
    }
}

pub(crate) fn start(
    directory: PathBuf,
    role: &str,
    limits: Limits,
    failure: Failure,
) -> io::Result<(EventWriterFactory, LogGuard)> {
    let sink = SegmentWriter::new(directory, role, limits)?;
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let queue = Arc::new(Queue {
        sender: Mutex::new(Some(sender)),
        dropped: AtomicU64::new(0),
        failure,
        max_record_bytes: limits.file_bytes,
    });
    let worker_queue = queue.clone();
    let thread = thread::Builder::new()
        .name("block-logging".to_owned())
        .spawn(move || {
            if run(sink, receiver, &worker_queue).is_err() {
                worker_queue.failure.report(Problem::Storage);
            }
        })?;
    Ok((
        EventWriterFactory(queue.clone()),
        LogGuard {
            queue,
            thread: Some(thread),
        },
    ))
}

fn run(mut sink: SegmentWriter, receiver: Receiver<Record>, queue: &Queue) -> io::Result<()> {
    let mut deadline = Instant::now() + FLUSH_INTERVAL;
    let mut reported_drops = 0;
    loop {
        let timeout = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(timeout) {
            Ok(record) => {
                sink.write_record(&record.bytes)?;
                if record.is_error {
                    sink.flush()?;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                write_drop_summary(&mut sink, queue, &mut reported_drops)?;
                return sink.finish();
            }
        }
        if Instant::now() >= deadline {
            write_drop_summary(&mut sink, queue, &mut reported_drops)?;
            sink.flush()?;
            deadline = Instant::now() + FLUSH_INTERVAL;
        }
    }
}

fn write_drop_summary(
    sink: &mut SegmentWriter,
    queue: &Queue,
    reported_drops: &mut u64,
) -> io::Result<()> {
    let total = queue.dropped.load(Ordering::Relaxed);
    if total == *reported_drops {
        return Ok(());
    }
    let timestamp = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(io::Error::other)?;
    let mut record = serde_json::to_vec(&serde_json::json!({
        "timestamp": timestamp,
        "level": "WARN",
        "target": "block_logging",
        "fields": {
            "event": "log_events_dropped",
            "dropped_events": total.saturating_sub(*reported_drops),
            "total_dropped_events": total,
        }
    }))
    .map_err(io::Error::other)?;
    record.push(b'\n');
    sink.write_record(&record)?;
    *reported_drops = total;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex, atomic::AtomicU64, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::ensure;
    use tempfile::TempDir;

    use super::{Failure, Problem, Queue, run};
    use crate::files::{Limits, SegmentWriter, segments};

    #[test]
    fn full_queue_drops_without_blocking_and_reports_a_summary() -> anyhow::Result<()> {
        let temp = TempDir::new()?;
        let limits = Limits {
            file_bytes: 1024,
            retention_bytes: 4096,
        };
        let sink = SegmentWriter::new(temp.path().to_owned(), "cli", limits)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let failure = Failure::default();
        let queue = Arc::new(Queue {
            sender: Mutex::new(Some(sender)),
            dropped: AtomicU64::new(0),
            failure: failure.clone(),
            max_record_bytes: limits.file_bytes,
        });
        queue.send(b"{\"queued\":true}\n", false);
        queue.send(b"{\"lost\":true}\n", true);
        assert_eq!(
            failure.message(),
            Some("Some local log events could not be recorded.")
        );
        let worker_queue = queue.clone();
        let handle = thread::spawn(move || run(sink, receiver, &worker_queue));
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let contents = std::fs::read_to_string(&segments(temp.path())?[0].path)?;
            if contents.contains("log_events_dropped") {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "drop summary was not flushed while the queue was open"
            );
            thread::sleep(Duration::from_millis(10));
        }
        queue
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("worker panicked"))??;
        let contents = std::fs::read_to_string(&segments(temp.path())?[0].path)?;
        let events = contents
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["queued"], true);
        assert_eq!(events[1]["fields"]["event"], "log_events_dropped");
        assert_eq!(events[1]["fields"]["dropped_events"], 1);
        assert_eq!(events[1]["fields"]["total_dropped_events"], 1);
        Ok(())
    }

    #[test]
    fn storage_failure_is_not_hidden_by_an_earlier_loss() {
        let failure = Failure::default();
        failure.report(Problem::Loss);
        failure.report(Problem::Storage);
        assert_eq!(
            failure.message(),
            Some("Local logging stopped because storage failed.")
        );
    }

    #[cfg(unix)]
    #[test]
    fn full_stderr_pipe_does_not_block_producers_or_shutdown() -> anyhow::Result<()> {
        use std::process::{Child, Command, Stdio};

        struct KillOnDrop(Child);
        impl Drop for KillOnDrop {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "worker::tests::full_stderr_pipe_helper",
                "--nocapture",
            ])
            .env("BLOCK_LOG_TEST_FULL_STDERR", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut child = KillOnDrop(child);
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = child.0.try_wait()? {
                ensure!(status.success(), "stderr backpressure helper failed");
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "producer or shutdown blocked on stderr"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn full_stderr_pipe_helper() -> anyhow::Result<()> {
        use std::{
            io::Write as _,
            sync::atomic::{AtomicBool, Ordering},
        };

        if std::env::var_os("BLOCK_LOG_TEST_FULL_STDERR").is_none() {
            return Ok(());
        }
        let writes = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(AtomicBool::new(false));
        let writes_in_thread = writes.clone();
        let pending_in_thread = pending.clone();
        thread::spawn(move || {
            loop {
                pending_in_thread.store(true, Ordering::SeqCst);
                if std::io::stderr().write_all(&[b'x'; 8192]).is_err() {
                    break;
                }
                writes_in_thread.fetch_add(1, Ordering::SeqCst);
                pending_in_thread.store(false, Ordering::SeqCst);
            }
        });
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut previous = 0;
        let mut stable_since = Instant::now();
        loop {
            let current = writes.load(Ordering::SeqCst);
            if current != previous || !pending.load(Ordering::SeqCst) {
                previous = current;
                stable_since = Instant::now();
            } else if current > 0 && stable_since.elapsed() >= Duration::from_millis(150) {
                break;
            }
            ensure!(Instant::now() < deadline, "stderr pipe did not fill");
            thread::sleep(Duration::from_millis(10));
        }

        let (sender, receiver) = mpsc::sync_channel(1);
        let queue = Queue {
            sender: Mutex::new(Some(sender)),
            dropped: AtomicU64::new(0),
            failure: Failure::default(),
            max_record_bytes: 128,
        };
        queue.send(b"{\"queued\":true}\n", false);
        queue.send(b"{\"dropped\":true}\n", true);
        assert_eq!(queue.failure.message(), Some(Problem::Loss.message()));
        drop(receiver);
        queue.send(b"{\"disconnected\":true}\n", true);
        assert_eq!(queue.failure.message(), Some(Problem::Storage.message()));

        let temp = TempDir::new()?;
        let directory = temp.path().join("logs");
        let failure = Failure::default();
        let limits = Limits {
            file_bytes: 12,
            retention_bytes: 100,
        };
        let (writer, guard) = super::start(directory.clone(), "launcher", limits, failure.clone())?;
        writer.0.send(b"{\"n\":1}\n", true);
        let deadline = Instant::now() + Duration::from_secs(4);
        while !segments(&directory)?.iter().any(|segment| segment.size > 0) {
            ensure!(Instant::now() < deadline, "first record was not flushed");
            thread::sleep(Duration::from_millis(10));
        }
        std::fs::rename(&directory, temp.path().join("moved"))?;
        std::fs::write(&directory, b"not a directory")?;
        writer.0.send(b"{\"n\":2}\n", true);
        drop(guard);
        assert_eq!(failure.message(), Some(Problem::Storage.message()));
        Ok(())
    }
}
