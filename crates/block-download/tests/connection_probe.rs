use block_download::{DownloadOptions, ProxyMode, Store};
use std::{
    io::{BufRead as _, BufReader, Read as _, Write as _},
    net::Shutdown,
    sync::Arc,
};

#[tokio::test]
async fn connection_probe_uses_the_configured_proxy_and_reports_http_failure_without_cache()
-> anyhow::Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let worker = std::thread::spawn(move || -> anyhow::Result<()> {
        for status in ["200 OK", "503 Service Unavailable"] {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            // Darwin accepts inherit the nonblocking listener's descriptor
            // flag. The bounded accept loop must not make HTTP reads race the
            // arrival of the next packet (or EOF after a half-close).
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
            // TCP can split headers over several reads. Consume the complete
            // request before closing, otherwise unread bytes can reset the
            // connection and discard our response on macOS.
            let mut request = String::new();
            let mut reader = BufReader::new(&stream);
            while !request.ends_with("\r\n\r\n") {
                anyhow::ensure!(
                    reader.read_line(&mut request)? > 0,
                    "incomplete proxy request"
                );
                anyhow::ensure!(request.len() < 32 * 1024, "oversized proxy request");
            }
            assert!(request.starts_with("HEAD http://example.invalid/metadata HTTP/1.1\r\n"));
            drop(reader);
            stream.write_all(
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )?;
            // Finish the response in TCP order and let the client close its
            // side, rather than racing a full socket drop with its header read.
            stream.shutdown(Shutdown::Write)?;
            let mut trailing = [0; 1024];
            while stream.read(&mut trailing)? != 0 {}
        }
        Ok(())
    });
    let root = tempfile::tempdir()?;
    let store = Store::with_options(
        root.path().to_owned(),
        false,
        Arc::new(|_| {}),
        DownloadOptions {
            proxy: ProxyMode::Http { url: proxy },
            ..DownloadOptions::default()
        },
    )?;
    assert_eq!(
        store
            .probe_connection("http://example.invalid/metadata")
            .await?,
        200
    );
    let error = store
        .probe_connection("http://example.invalid/metadata")
        .await
        .err();
    assert!(
        error
            .as_ref()
            .is_some_and(|error| error.to_string().contains("503")),
        "{error:?}"
    );
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("proxy fixture worker failed"))??;
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    Ok(())
}
