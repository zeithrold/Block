use std::{fs, io, path::Path};

fn write_json(path: &Path, value: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "fixture JSON path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    fs::write(path, value)
}

#[path = "tests/configurations.rs"]
mod configurations;
#[path = "tests/discovery.rs"]
mod discovery;
#[path = "tests/metadata.rs"]
mod metadata;
