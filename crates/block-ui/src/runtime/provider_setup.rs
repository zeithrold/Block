//! Network discovery returns a draft; only explicit save operations write the catalog.
use super::*;
use block_auth::AuthlibProvider;

impl Engine {
    pub(crate) fn detect_auth_provider(
        &self,
        address: String,
        allow_http: bool,
        cancel: CancellationToken,
    ) {
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let provider = AuthlibProvider::detect(&address, allow_http, &cancel).await?;
                let icon = match ImageFetch::new() {
                    Ok(fetch) => fetch
                        .icon(&provider.metadata.icon_url, &provider.api_url, &cancel)
                        .await
                        .ok(),
                    Err(_) => None,
                };
                ensure!(!cancel.is_cancelled(), "Provider setup was cancelled.");
                sender
                    .send(Message::ProviderDetected(Box::new(provider), icon))
                    .await
                    .map_err(|_| anyhow::anyhow!("Provider drawer closed"))?;
                Ok(msg!("accounts.auth.provider_add.detected"))
            }
            .await;
            finish(&sender, result).await;
        });
    }
    pub(crate) fn save_detected_provider(
        &self,
        provider: AuthlibProvider,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                ensure!(!cancel.is_cancelled(), "Provider setup was cancelled.");
                commit(&root, None, provider, &sender).await
            }
            .await;
            finish(&sender, result).await;
        });
    }
    pub(crate) fn save_provider(
        &self,
        original: Option<AuthlibProvider>,
        name: String,
        address: String,
        allow_http: bool,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let address = block_auth::ProviderAddress::parse(&address)?;
                let provider = if let Some(provider) = &original
                    && address.url == provider.api_url
                    && (!address.insecure || allow_http)
                {
                    let mut provider = provider.clone();
                    provider.name = name;
                    provider.validate()?;
                    provider
                } else {
                    AuthlibProvider::discover(
                        original.as_ref().map_or_else(uuid::Uuid::new_v4, |p| p.id),
                        &name,
                        &address.url,
                        allow_http,
                        &cancel,
                    )
                    .await?
                };
                ensure!(!cancel.is_cancelled(), "Provider setup was cancelled.");
                commit(&root, original.as_ref(), provider, &sender).await
            }
            .await;
            finish(&sender, result).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    #[test]
    fn redirected_provider_is_only_saved_after_confirmation_even_if_server_goes_offline()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, runtime, mut events) = super::super::tests::fixture_engine(root.path())?;
        let document = crate::providers::fixture_provider(false)?.metadata_json;
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let origin = format!("http://{}", listener.local_addr()?);
            let fixture = async {
                for (path, response) in [
                    ("/landing/", "302 Found\r\nLocation: /api/\r\nContent-Length: 0\r\n\r\n".into()),
                    ("/api/", format!("200 OK\r\nContent-Length: {}\r\n\r\n{document}", document.len())),
                ] {
                    let (mut socket, _) = listener.accept().await?;
                    let mut request = String::new();
                    let mut reader = BufReader::new(&mut socket);
                    reader.read_line(&mut request).await?;
                    assert!(request.starts_with(&format!("GET {path} ")));
                    loop {
                        request.clear();
                        if reader.read_line(&mut request).await? == 0 || request == "\r\n" { break; }
                    }
                    socket.write_all(format!("HTTP/1.1 {}", response.replacen("\r\n", "\r\nConnection: close\r\n", 1)).as_bytes()).await?;
                }
                drop(listener);
                Ok::<_, anyhow::Error>(())
            };
            let registration = async {
                engine.detect_auth_provider(format!("{origin}/landing/"), true, CancellationToken::new());
                let mut candidate = match events.recv().await {
                    Some(Message::ProviderDetected(p, None)) => p,
                    Some(Message::Finished(Err(error))) => anyhow::bail!("Detection failed: {error}"),
                    _ => anyhow::bail!("Expected a detected candidate with fallback icon"),
                };
                assert!(matches!(events.recv().await, Some(Message::Finished(Ok(_)))));
                assert!(load_providers(root.path())?.is_empty());
                assert_eq!(candidate.name, "Fixture Community");
                assert_eq!(candidate.api_url, format!("{origin}/api/"));
                candidate.name = "Local alias".into();
                engine.save_detected_provider((*candidate).clone(), CancellationToken::new());
                assert!(matches!(events.recv().await, Some(Message::Providers(Ok(_)))));
                assert!(matches!(events.recv().await, Some(Message::ProviderSaved(id)) if id == candidate.id.to_string()));
                assert!(matches!(events.recv().await, Some(Message::Finished(Ok(_)))));
                assert_eq!(load_providers(root.path())?, vec![*candidate]);
                Ok::<_, anyhow::Error>(())
            };
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::try_join!(fixture, registration)
            }).await??;
            Ok(())
        })
    }
}

async fn commit(
    root: &Path,
    original: Option<&AuthlibProvider>,
    provider: AuthlibProvider,
    sender: &EventSender,
) -> anyhow::Result<crate::i18n::UiText> {
    persist_provider(root, original, &provider)?;
    let _ = sender
        .unscoped()
        .send(Message::Providers(Ok(load_providers(root)?)))
        .await;
    let _ = sender
        .send(Message::ProviderSaved(provider.id.to_string()))
        .await;
    Ok(msg!("completion.provider_saved", name = provider.name))
}
