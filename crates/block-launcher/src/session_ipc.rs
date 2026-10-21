//! Private, per-run local sockets for the detached game supervisor.
#![allow(
    clippy::redundant_pub_crate,
    reason = "IPC helpers must remain crate-private under unreachable_pub."
)]

use anyhow::{Context as _, ensure};
use interprocess::local_socket::{
    GenericFilePath, ListenerOptions, Name, ToFsName as _,
    tokio::{Listener, Stream},
    traits::tokio::Stream as _,
};
use uuid::Uuid;

fn validate_run_id(run_id: &str) -> anyhow::Result<()> {
    let parsed = Uuid::parse_str(run_id).context("invalid session run ID")?;
    ensure!(
        parsed.to_string() == run_id,
        "session run ID must be canonical"
    );
    Ok(())
}

#[cfg(unix)]
fn directory(run_id: &str) -> anyhow::Result<std::path::PathBuf> {
    validate_run_id(run_id)?;
    Ok(std::path::Path::new("/tmp").join(format!("block-session-{run_id}")))
}

fn endpoint(run_id: &str) -> anyhow::Result<Name<'static>> {
    validate_run_id(run_id)?;
    #[cfg(unix)]
    let path = directory(run_id)?.join("control.sock");
    #[cfg(windows)]
    let path = std::path::PathBuf::from(format!(r"\\.\pipe\Block-session-{run_id}"));
    Ok(path.to_fs_name::<GenericFilePath>()?.into_owned())
}

#[cfg(unix)]
fn private_directory(run_id: &str) -> anyhow::Result<std::fs::Metadata> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let path = directory(run_id)?;
    let metadata = std::fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "session socket directory is not a plain directory"
    );
    ensure!(
        metadata.permissions().mode() & 0o777 == 0o700,
        "session socket directory is not private"
    );
    ensure!(
        metadata.uid() == rustix::process::geteuid().as_raw(),
        "session socket directory belongs to another user"
    );
    let canonical = std::fs::canonicalize(&path)?;
    let parent = std::fs::canonicalize("/tmp")?;
    ensure!(
        canonical.parent() == Some(parent.as_path()),
        "session socket directory is outside its runtime parent"
    );
    Ok(metadata)
}

/// Bind once without replacing an existing endpoint or run directory.
pub(crate) fn listen(run_id: &str) -> anyhow::Result<Listener> {
    let options = ListenerOptions::new().name(endpoint(run_id)?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        let path = directory(run_id)?;
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        // The enclosing directory supplies privacy on macOS as well as Linux.
        // Setting the socket mode before bind is unsupported on some Unices.
        let result = options.create_tokio();
        if result.is_err() {
            // Remove only the empty directory created by this invocation.
            let _ = std::fs::remove_dir(path);
        }
        Ok(result?)
    }
    #[cfg(windows)]
    {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt as _, security_descriptor::SecurityDescriptor,
        };

        let security =
            SecurityDescriptor::deserialize(widestring::u16cstr!("D:P(A;;GA;;;OW)(A;;GA;;;SY)"))?;
        Ok(options.security_descriptor(security).create_tokio()?)
    }
}

/// Connect only to the private endpoint associated with the requested run.
pub(crate) async fn connect(run_id: &str) -> anyhow::Result<Stream> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt as _;

        private_directory(run_id)?;
        let metadata = std::fs::symlink_metadata(directory(run_id)?.join("control.sock"))?;
        ensure!(
            metadata.file_type().is_socket(),
            "session endpoint is not a socket"
        );
    }
    let stream = Stream::connect(endpoint(run_id)?).await?;
    validate_peer(run_id, &stream)?;
    Ok(stream)
}

/// Accept handlers must call this before reading commands from a new client.
pub(crate) fn validate_peer(run_id: &str, stream: &Stream) -> anyhow::Result<()> {
    validate_run_id(run_id)?;
    #[cfg(unix)]
    {
        use interprocess::local_socket::traits::StreamCommon as _;
        use std::os::unix::fs::MetadataExt as _;

        let owner = private_directory(run_id)?.uid();
        ensure!(
            stream.peer_creds()?.euid() == Some(owner),
            "session socket peer has a different owner"
        );
    }
    #[cfg(windows)]
    let _ = stream;
    Ok(())
}

/// Remove this run's socket and empty private directory after host shutdown.
pub(crate) fn cleanup(run_id: &str) -> anyhow::Result<()> {
    validate_run_id(run_id)?;
    #[cfg(unix)]
    {
        use std::{io::ErrorKind, os::unix::fs::FileTypeExt as _};

        let path = directory(run_id)?;
        match private_directory(run_id) {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
            {
                return Ok(());
            }
            result => {
                result?;
            }
        }
        let socket = path.join("control.sock");
        match std::fs::symlink_metadata(&socket) {
            Ok(metadata) => {
                ensure!(
                    metadata.file_type().is_socket(),
                    "refusing to remove a non-socket endpoint"
                );
                std::fs::remove_file(socket)?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        std::fs::remove_dir(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{cleanup, connect, listen, validate_run_id};
    use interprocess::local_socket::traits::tokio::Listener as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use uuid::Uuid;

    #[test]
    fn run_ids_reject_paths_and_noncanonical_spellings() {
        for value in [
            "",
            "../control",
            "00000000000000000000000000000000",
            "A96E17B2-F164-46CE-B71A-73596D5FAB80",
        ] {
            assert!(validate_run_id(value).is_err());
        }
        assert!(validate_run_id(&Uuid::new_v4().to_string()).is_ok());
    }

    #[tokio::test]
    async fn endpoint_reconnects_without_replacing_the_listener() -> anyhow::Result<()> {
        let run_id = Uuid::new_v4().to_string();
        let listener = listen(&run_id)?;
        assert!(listen(&run_id).is_err());
        for _ in 0..2 {
            let (accepted, connected) =
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    tokio::join!(listener.accept(), connect(&run_id))
                })
                .await?;
            let mut accepted = accepted?;
            let mut connected = connected?;
            super::validate_peer(&run_id, &accepted)?;
            connected.write_all(b"stop").await?;
            let mut message = [0; 4];
            accepted.read_exact(&mut message).await?;
            assert_eq!(&message, b"stop");
        }
        drop(listener);
        cleanup(&run_id)?;
        cleanup(&run_id)?;
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn client_refuses_a_directory_readable_by_other_users() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let run_id = Uuid::new_v4().to_string();
        let listener = listen(&run_id)?;
        let directory = super::directory(&run_id)?;
        let permissions = std::fs::metadata(&directory)?.permissions();
        assert_eq!(permissions.mode() & 0o777, 0o700);
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))?;
        assert!(connect(&run_id).await.is_err());
        assert!(cleanup(&run_id).is_err());
        std::fs::set_permissions(directory, permissions)?;
        drop(listener);
        cleanup(&run_id)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_rejects_symlinks_and_keeps_unrelated_files() -> anyhow::Result<()> {
        use std::os::unix::fs::{DirBuilderExt as _, symlink};

        let run_id = Uuid::new_v4().to_string();
        let directory = super::directory(&run_id)?;
        let target = tempfile::tempdir()?;
        symlink(target.path(), &directory)?;
        assert!(cleanup(&run_id).is_err());
        std::fs::remove_file(&directory)?;

        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        std::fs::write(directory.join("control.sock"), b"not a socket")?;
        assert!(cleanup(&run_id).is_err());
        assert_eq!(
            std::fs::read(directory.join("control.sock"))?,
            b"not a socket"
        );
        std::fs::remove_file(directory.join("control.sock"))?;
        std::fs::write(directory.join("keep"), b"unrelated")?;
        assert!(cleanup(&run_id).is_err());
        assert_eq!(std::fs::read(directory.join("keep"))?, b"unrelated");
        std::fs::remove_file(directory.join("keep"))?;
        cleanup(&run_id)?;
        Ok(())
    }
}
