use std::{
    fs::File,
    io::{self, Read as _, Seek as _, SeekFrom, Write as _},
    path::Path,
};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::files::{self, MIB};

const EXPORT_BYTES: u64 = 100 * MIB;

/// Export a bounded snapshot of launcher JSONL segments to a new ZIP file.
///
/// The archive contains only recognized launcher diagnostic segments, named by
/// basename, and at most 100 MiB of uncompressed, complete newline-terminated
/// records. Active segments are included through their current on-disk length.
/// The destination must not already exist.
///
/// # Errors
/// Returns an error if the snapshot cannot be read or the new archive cannot
/// be created. An existing destination is never overwritten.
pub fn export_launcher_logs(destination: &Path) -> anyhow::Result<()> {
    export_in(
        &crate::log_directory()?.join("launcher"),
        destination,
        EXPORT_BYTES,
    )
}

fn snapshot(directory: &Path) -> io::Result<Vec<files::Segment>> {
    if !directory.try_exists()? {
        return Ok(Vec::new());
    }
    let _lock = files::lock_directory(directory)?;
    let mut entries = files::segments(directory)?;
    entries.reverse();
    Ok(entries)
}

pub(crate) fn export_in(
    directory: &Path,
    destination: &Path,
    max_bytes: u64,
) -> anyhow::Result<()> {
    let entries = snapshot(directory)?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = ZipWriter::new(temporary.as_file_mut());
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o600);
        let mut remaining = max_bytes;
        for entry in entries {
            if remaining == 0 {
                break;
            }
            // Open each segment under the retention lock, but do not hold one
            // descriptor per historical file or block rotation while copying.
            let mut file = {
                let _lock = files::lock_directory(directory)?;
                match File::open(&entry.path) {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                }
            };
            let length = complete_prefix(&mut file, entry.size.min(remaining))?;
            if length == 0 {
                continue;
            }
            zip.start_file(&entry.name, options)?;
            file.seek(SeekFrom::Start(0))?;
            let copied = io::copy(&mut file.take(length), &mut zip)?;
            if copied != length {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            remaining -= length;
        }
        zip.finish()?.flush()?;
    }
    temporary.persist_noclobber(destination)?;
    Ok(())
}

// Find the last complete record at or before the captured file length without
// allocating a buffer proportional to a potentially incomplete final record.
fn complete_prefix(file: &mut File, mut end: u64) -> io::Result<u64> {
    let mut buffer = [0_u8; 8192];
    while end > 0 {
        let count = usize::try_from(end.min(buffer.len() as u64))
            .map_err(|_| io::Error::other("invalid snapshot length"))?;
        let start = end - count as u64;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..count])?;
        if let Some(position) = buffer[..count].iter().rposition(|byte| *byte == b'\n') {
            return Ok(start + position as u64 + 1);
        }
        end = start;
    }
    Ok(0)
}
