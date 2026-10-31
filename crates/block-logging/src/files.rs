use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write as _},
    path::{Path, PathBuf},
    time::SystemTime,
};

use uuid::Uuid;

pub(crate) const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub file_bytes: u64,
    pub retention_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 10 * MIB,
            retention_bytes: 100 * MIB,
        }
    }
}

pub(crate) fn valid_role(role: &str) -> bool {
    matches!(role, "launcher" | "cli" | "supervisor" | "crash-help")
}

fn file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

pub(crate) fn lock_directory(directory: &Path) -> io::Result<File> {
    let lock = file_options()
        .create(true)
        .truncate(false)
        .open(directory.join(".retention.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    Ok(lock)
}

#[derive(Debug)]
pub(crate) struct Segment {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    modified: SystemTime,
}

fn owned_name(name: &str) -> bool {
    let mut parts = name.split('.');
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some(role), Some(pid), Some(id), Some(index), Some("jsonl"), None) => {
            valid_role(role)
                && pid.parse::<u32>().is_ok()
                && Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
                && index.parse::<u64>().is_ok()
        }
        _ => false,
    }
}

pub(crate) fn segments(directory: &Path) -> io::Result<Vec<Segment>> {
    let mut segments = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| owned_name(name)) else {
            continue;
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        // Windows directory enumeration can retain a stale size while the
        // writer is open. Query the file so active segments count in snapshots.
        let metadata = fs::metadata(entry.path())?;
        segments.push(Segment {
            path: entry.path(),
            name: name.to_owned(),
            size: metadata.len(),
            modified: metadata.modified()?,
        });
    }
    segments.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(segments)
}

// The caller holds the directory lock. Each active segment has a locked
// sidecar, allowing readers to access its data even on Windows.
fn prune(directory: &Path, limit: u64) -> io::Result<()> {
    let segments = segments(directory)?;
    let mut total = segments
        .iter()
        .fold(0_u64, |sum, segment| sum.saturating_add(segment.size));
    for segment in segments {
        if total <= limit {
            break;
        }
        let lock_path = directory.join(format!("{}.lock", segment.name));
        let lock = file_options()
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => {
                match fs::remove_file(&segment.path) {
                    Ok(()) => total = total.saturating_sub(segment.size),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                drop(lock);
                fs::remove_file(lock_path)?;
            }
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct SegmentWriter {
    directory: PathBuf,
    role: String,
    process_id: u32,
    process_uuid: Uuid,
    next_index: u64,
    current: BufWriter<File>,
    current_lock: File,
    size: u64,
    limits: Limits,
}

impl SegmentWriter {
    pub(crate) fn new(directory: PathBuf, role: &str, limits: Limits) -> io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let _lock = lock_directory(&directory)?;
        let process_id = std::process::id();
        let process_uuid = Uuid::new_v4();
        let (current, current_lock) = new_segment(&directory, role, process_id, process_uuid, 0)?;
        prune(&directory, limits.retention_bytes)?;
        Ok(Self {
            directory,
            role: role.to_owned(),
            process_id,
            process_uuid,
            next_index: 1,
            current: BufWriter::new(current),
            current_lock,
            size: 0,
            limits,
        })
    }

    pub(crate) fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        let length = u64::try_from(record.len())
            .map_err(|_| io::Error::other("logging record is too large"))?;
        if length > self.limits.file_bytes {
            return Err(io::Error::other("logging record is too large"));
        }
        if self.size > 0 && self.size + length > self.limits.file_bytes {
            self.rotate()?;
        }
        self.current.write_all(record)?;
        self.size += length;
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.current.flush()
    }

    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.flush()?;
        let directory = self.directory.clone();
        let limit = self.limits.retention_bytes;
        drop(self);
        let _lock = lock_directory(&directory)?;
        prune(&directory, limit)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.flush()?;
        let _lock = lock_directory(&self.directory)?;
        let (file, lock) = new_segment(
            &self.directory,
            &self.role,
            self.process_id,
            self.process_uuid,
            self.next_index,
        )?;
        self.next_index += 1;
        let old = std::mem::replace(&mut self.current, BufWriter::new(file));
        let old_lock = std::mem::replace(&mut self.current_lock, lock);
        self.size = 0;
        drop(old);
        drop(old_lock);
        prune(&self.directory, self.limits.retention_bytes)
    }
}

fn new_segment(
    directory: &Path,
    role: &str,
    process_id: u32,
    process_uuid: Uuid,
    index: u64,
) -> io::Result<(File, File)> {
    let name = format!("{role}.{process_id}.{process_uuid}.{index:06}.jsonl");
    let lock = file_options()
        .create_new(true)
        .open(directory.join(format!("{name}.lock")))?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    let file = file_options().create_new(true).open(directory.join(name))?;
    Ok((file, lock))
}
