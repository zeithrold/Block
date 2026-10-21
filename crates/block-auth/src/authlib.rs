//! User-configured Yggdrasil providers. Discovery sends no account credentials.

use std::{
    collections::HashSet,
    time::{Duration, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use crate::{AuthError, Result};

const QUICK_LINK: &str = "authlib-injector:yggdrasil-server:";
const MAX_METADATA: usize = 512 * 1024;
const DISCOVERY_STAGE: &str = "Provider discovery";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAddress {
    pub url: String,
    pub suggested_name: String,
    pub insecure: bool,
}

impl ProviderAddress {
    /// Normalize a user address without making a connection.
    /// # Errors
    /// Rejects malformed links, credentials and non-HTTP API roots.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() || input.len() > 8192 {
            return Err(AuthError::InvalidProviderAddress);
        }
        let decoded;
        let address = if let Some(encoded) = input.strip_prefix(QUICK_LINK) {
            for (i, byte) in encoded.bytes().enumerate() {
                if byte == b'%'
                    && !encoded
                        .as_bytes()
                        .get(i + 1..i + 3)
                        .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
                {
                    return Err(AuthError::InvalidProviderAddress);
                }
            }
            decoded = percent_encoding::percent_decode_str(encoded)
                .decode_utf8()
                .map_err(|_| AuthError::InvalidProviderAddress)?;
            if decoded.chars().any(char::is_control) {
                return Err(AuthError::InvalidProviderAddress);
            }
            decoded.trim()
        } else if input.starts_with("authlib-injector:") {
            return Err(AuthError::InvalidProviderAddress);
        } else {
            input
        };
        let explicit = if address.contains("://") {
            address.to_owned()
        } else {
            format!("https://{address}")
        };
        let url = canonical_url(&explicit)?;
        Ok(Self {
            suggested_name: url
                .host_str()
                .ok_or(AuthError::InvalidProviderAddress)?
                .to_owned(),
            insecure: url.scheme() == "http",
            url: url.to_string(),
        })
    }
}

fn canonical_url(input: &str) -> Result<Url> {
    if input.len() > 2048
        || input
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
        || input.contains('\\')
    {
        return Err(AuthError::InvalidProviderAddress);
    }
    let mut url = Url::parse(input).map_err(|_| AuthError::InvalidProviderAddress)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port() == Some(0)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AuthError::InvalidProviderAddress);
    }
    let path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

fn discovery_target(current: &Url, target: &str) -> Result<Url> {
    let next = current
        .join(target)
        .map_err(|_| AuthError::InvalidProviderAddress)?;
    let next = canonical_url(next.as_str())?;
    if current.scheme() == "https" && next.scheme() == "http" {
        return Err(AuthError::InsecureProvider);
    }
    Ok(next)
}

/// Public metadata is cached with its source endpoint; no login fields belong here.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderMetadata {
    pub server_name: Option<String>,
    pub icon_url: String,
    pub skin_domains: Vec<String>,
    pub signature_public_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AuthlibProvider {
    pub id: Uuid,
    pub name: String,
    pub api_url: String,
    pub allow_http: bool,
    pub metadata: ProviderMetadata,
    pub metadata_json: String,
}

impl AuthlibProvider {
    /// Discover the real API and retain bounded metadata after explicit HTTP consent.
    /// # Errors
    /// Reports cancellation, timeouts, redirect/downgrade failures and invalid metadata.
    pub async fn discover(
        id: Uuid,
        name: &str,
        address: &str,
        allow_http: bool,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        validate_name(name)?;
        if id.is_nil() {
            return Err(AuthError::ProviderMetadata);
        }
        let address = ProviderAddress::parse(address)?;
        // Consent applies only to a URL the user actually entered as HTTP.
        let allow_http = allow_http && address.insecure;
        if address.insecure && !allow_http {
            return Err(AuthError::InsecureProvider);
        }
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(AuthError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(45), discover_inner(id, name, &address.url, allow_http)) =>
                result.map_err(|_| AuthError::ProviderTimedOut)?,
        }
    }

    /// Validate cached public fields before using a persisted endpoint.
    /// # Errors
    /// Rejects malformed or inconsistent cache entries instead of silently rewriting them.
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        let url = canonical_url(&self.api_url)?;
        if self.id.is_nil() || url.as_str() != self.api_url {
            return Err(AuthError::ProviderMetadata);
        }
        if url.scheme() == "http" && !self.allow_http {
            return Err(AuthError::InsecureProvider);
        }
        if metadata(&self.metadata_json, &url)? != self.metadata {
            return Err(AuthError::ProviderMetadata);
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.chars().count() > 128
        || name.chars().any(char::is_control)
    {
        return Err(AuthError::InvalidProviderName);
    }
    Ok(())
}

async fn discover_inner(
    id: Uuid,
    name: &str,
    address: &str,
    allow_http: bool,
) -> Result<AuthlibProvider> {
    let builder = Client::builder();
    #[cfg(test)]
    let builder = builder.no_proxy();
    let client = builder
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent("Block/0.1 (provider discovery)")
        .build()
        .map_err(provider_network)?;
    let mut url = canonical_url(address)?;
    let mut visited = HashSet::new();
    for _ in 0..=5 {
        if !visited.insert(url.to_string()) {
            return Err(AuthError::ProviderDiscoveryLoop);
        }
        if url.scheme() == "http" && !allow_http {
            return Err(AuthError::InsecureProvider);
        }
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(provider_network)?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok());
            let seconds = retry_after
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    retry_after
                        .and_then(|s| httpdate::parse_http_date(s).ok())
                        .and_then(|when| when.duration_since(SystemTime::now()).ok())
                        .map(|d| d.as_secs())
                })
                .unwrap_or(30)
                .clamp(1, 86400);
            return Err(AuthError::ProviderThrottled {
                retry_after_seconds: seconds,
            });
        }
        let target = if response.status().is_redirection() {
            Some(
                response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or(AuthError::ProviderMetadata)?,
            )
        } else {
            response.headers().get("X-Authlib-Injector-API-Location")
        };
        if let Some(target) = target {
            let target = target
                .to_str()
                .map_err(|_| AuthError::InvalidProviderAddress)?;
            let next = discovery_target(&url, target)?;
            if next != url {
                url = next;
                continue;
            }
        }
        if !response.status().is_success() {
            return Err(AuthError::Http {
                stage: DISCOVERY_STAGE,
                status: response.status().as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_METADATA as u64)
        {
            return Err(AuthError::ProviderMetadata);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(provider_network)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_METADATA {
                return Err(AuthError::ProviderMetadata);
            }
            bytes.extend_from_slice(&chunk);
        }
        let json = String::from_utf8(bytes).map_err(|_| AuthError::ProviderMetadata)?;
        let provider = AuthlibProvider {
            id,
            name: name.to_owned(),
            api_url: url.to_string(),
            allow_http: url.scheme() == "http" && allow_http,
            metadata: metadata(&json, &url)?,
            metadata_json: json,
        };
        provider.validate()?;
        return Ok(provider);
    }
    Err(AuthError::ProviderDiscoveryLoop)
}

fn provider_network(error: reqwest::Error) -> AuthError {
    if error.is_timeout() {
        AuthError::ProviderTimedOut
    } else {
        AuthError::Network {
            stage: DISCOVERY_STAGE,
        }
    }
}

fn metadata(json: &str, url: &Url) -> Result<ProviderMetadata> {
    if json.len() > MAX_METADATA {
        return Err(AuthError::ProviderMetadata);
    }
    let object: serde_json::Value =
        serde_json::from_str(json).map_err(|_| AuthError::ProviderMetadata)?;
    if !object.is_object() {
        return Err(AuthError::ProviderMetadata);
    }
    let key = object
        .get("signaturePublickey")
        .and_then(serde_json::Value::as_str)
        .ok_or(AuthError::ProviderMetadata)?;
    let der = key
        .trim_end_matches(['\r', '\n'])
        .strip_prefix("-----BEGIN PUBLIC KEY-----")
        .and_then(|s| s.strip_suffix("-----END PUBLIC KEY-----"))
        .ok_or(AuthError::ProviderMetadata)?
        .replace(['\r', '\n'], "");
    let decoded = STANDARD
        .decode(der)
        .map_err(|_| AuthError::ProviderMetadata)?;
    if key.len() > 32768 || decoded.len() < 16 || decoded.first() != Some(&0x30) {
        return Err(AuthError::ProviderMetadata);
    }
    let domains = match object.get("skinDomains") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or(AuthError::ProviderMetadata)?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or(AuthError::ProviderMetadata)
            })
            .collect::<Result<Vec<_>>>()?,
    };
    if domains.len() > 256
        || domains.iter().any(|d| {
            d.is_empty()
                || d.len() > 512
                || d.chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control() || ":/\\?#@".contains(ch))
        })
    {
        return Err(AuthError::ProviderMetadata);
    }
    let name = match object.get("meta").and_then(|m| m.get("serverName")) {
        None => None,
        Some(value) => {
            let name = value.as_str().ok_or(AuthError::ProviderMetadata)?;
            validate_name(name).map_err(|_| AuthError::ProviderMetadata)?;
            Some(name.to_owned())
        }
    };
    Ok(ProviderMetadata {
        server_name: name,
        icon_url: url
            .join("/favicon.ico")
            .map_err(|_| AuthError::ProviderMetadata)?
            .to_string(),
        skin_domains: domains,
        signature_public_key: key.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[test]
    fn discovery_never_downgrades_https_even_after_http_consent() -> Result<()> {
        let secure = Url::parse("https://community.example.test/api/")
            .map_err(|_| AuthError::InvalidProviderAddress)?;
        assert_eq!(
            discovery_target(&secure, "http://community.example.test/api/"),
            Err(AuthError::InsecureProvider)
        );
        assert_eq!(
            discovery_target(&secure, "../other/")?.as_str(),
            "https://community.example.test/other/"
        );
        let insecure = Url::parse("http://community.example.test/")
            .map_err(|_| AuthError::InvalidProviderAddress)?;
        assert_eq!(discovery_target(&insecure, secure.as_str())?, secure);
        Ok(())
    }

    #[tokio::test]
    async fn discovery_rejects_oversized_responses_and_inconsistent_metadata()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await?;
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await?;
                if count == 0 {
                    return Err(std::io::Error::other(
                        "Fixture request ended before headers",
                    ));
                }
                request.extend_from_slice(&buffer[..count]);
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 Fixture\r\nContent-Length: 600000\r\nConnection: close\r\n\r\n",
                )
                .await
        });
        assert_eq!(discover(&url).await, Err(AuthError::ProviderMetadata));
        served.await??;
        let root = canonical_url("https://example.test/api/")?;
        for replacement in [
            serde_json::json!({"skinDomains":["https://secret.test/"]}),
            serde_json::json!({"signaturePublickey":"not a PEM key"}),
            serde_json::json!({"meta":{"serverName":"bad\nname"}}),
        ] {
            let mut value: serde_json::Value = serde_json::from_str(&document())?;
            for (key, value_new) in replacement.as_object().ok_or("fixture object")? {
                value[key] = value_new.clone();
            }
            assert!(metadata(&value.to_string(), &root).is_err());
        }
        Ok(())
    }

    #[test]
    fn existing_accounts_keep_their_identity_and_external_profiles_use_provider_and_login_scope()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut account: crate::Account = serde_json::from_value(
            serde_json::json!({"reference":{"player_id":"00000000-0000-4000-8000-000000000001"},"provider":"microsoft","player_name":"Player","client_id":"public","credential_ref":"fixture"}),
        )?;
        assert!(account.authlib.is_none());
        assert_eq!(
            account.identity_key(),
            account.reference.player_id.to_string()
        );
        assert!(serde_json::to_value(&account)?.get("authlib").is_none());
        account.provider = crate::AccountProvider::Authlib;
        account.authlib = Some(crate::AuthlibAccount {
            provider_id: Uuid::from_u128(1),
            login: "FirstUser".into(),
        });
        let first = account.identity_key();
        if let Some(link) = &mut account.authlib {
            link.provider_id = Uuid::from_u128(2);
        }
        assert_ne!(first, account.identity_key());
        if let Some(link) = &mut account.authlib {
            link.provider_id = Uuid::from_u128(1);
            link.login = "SecondUser".into();
        }
        assert_ne!(first, account.identity_key());
        Ok(())
    }

    fn document() -> String {
        serde_json::json!({
            "meta": {"serverName": "Fixture Community"},
            "skinDomains": ["textures.example.test", ".example.test", "*.skins.test"],
            "signaturePublickey": "-----BEGIN PUBLIC KEY-----\nMBkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDAgAA\n-----END PUBLIC KEY-----"
        }).to_string()
    }

    async fn server(
        steps: Vec<(u16, String, String)>,
    ) -> std::result::Result<
        (
            String,
            tokio::task::JoinHandle<std::io::Result<Vec<String>>>,
        ),
        Box<dyn std::error::Error>,
    > {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, headers, body) in steps {
                let (mut stream, _) = listener.accept().await?;
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut buffer = [0; 2048];
                    let count = stream.read(&mut buffer).await?;
                    if count == 0 {
                        return Err(std::io::Error::other("Fixture request ended early"));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                requests.push(String::from_utf8(bytes).map_err(std::io::Error::other)?);
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await?;
            }
            Ok(requests)
        });
        Ok((url, task))
    }

    async fn discover(url: &str) -> Result<AuthlibProvider> {
        AuthlibProvider::discover(
            Uuid::new_v4(),
            "My community",
            url,
            true,
            &CancellationToken::new(),
        )
        .await
    }

    #[test]
    fn addresses_and_quick_links_normalize_without_dropping_security_fields() -> Result<()> {
        for (input, expected) in [
            ("Example.COM/api///", "https://example.com/api/"),
            ("localhost:25565/api", "https://localhost:25565/api/"),
            ("https://example.com:443/api/", "https://example.com/api/"),
            (
                "authlib-injector:yggdrasil-server:https%3A%2F%2Fexample.com%2Fapi",
                "https://example.com/api/",
            ),
        ] {
            assert_eq!(ProviderAddress::parse(input)?.url, expected);
        }
        assert!(ProviderAddress::parse("http://localhost:8080/api")?.insecure);
        for input in [
            "",
            "https://user:password@example.com",
            "https://example.com/?token=private",
            "https://example.com/#private",
            "file:///tmp/provider",
            "ftp://example.com",
            "authlib-injector:invalid:example.com",
            "authlib-injector:yggdrasil-server:%GG",
            "authlib-injector:yggdrasil-server:%FF",
            "authlib-injector:yggdrasil-server:https%3A%2F%2Fexample.com%0A",
            "https://example.com/line\nbreak",
            "https://example.com\\api",
            "http://localhost:0/",
        ] {
            assert!(ProviderAddress::parse(input).is_err(), "{input:?}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn discovery_follows_redirects_and_relative_ali_then_caches_validated_metadata()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let json = document();
        let (url, task) = server(vec![
            (302, "Location: /landing/\r\n".into(), String::new()),
            (
                200,
                "X-Authlib-Injector-API-Location: ../api/\r\n".into(),
                "Not metadata".into(),
            ),
            (
                200,
                "X-Authlib-Injector-API-Location: ./\r\n".into(),
                json.clone(),
            ),
        ])
        .await?;
        let provider = discover(&url).await?;
        assert_eq!(provider.api_url, format!("{url}api/"));
        assert_eq!(provider.name, "My community");
        assert_eq!(
            provider.metadata.server_name.as_deref(),
            Some("Fixture Community")
        );
        assert_eq!(provider.metadata.icon_url, format!("{url}favicon.ico"));
        assert_eq!(provider.metadata.skin_domains.len(), 3);
        assert_eq!(provider.metadata_json, json);
        let cached: AuthlibProvider = serde_json::from_str(&serde_json::to_string(&provider)?)?;
        cached.validate()?;
        let requests = task.await??;
        assert_eq!(requests.len(), 3);
        for (request, path) in requests.iter().zip(["/", "/landing/", "/api/"]) {
            assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
            assert!(!request.to_lowercase().contains("authorization:"));
        }
        let mut tampered = cached;
        tampered.metadata.skin_domains.push("forged.test".into());
        assert!(tampered.validate().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn discovery_accepts_absolute_ali_on_a_different_server_without_credentials()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (api, api_task) = server(vec![(200, String::new(), document())]).await?;
        let (landing, landing_task) = server(vec![(
            404,
            format!("X-Authlib-Injector-API-Location: {api}\r\n"),
            String::new(),
        )])
        .await?;
        assert_eq!(discover(&landing).await?.api_url, api);
        assert_eq!(api_task.await??.len(), 1);
        landing_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn discovery_rejects_http_before_io_and_cancel_interrupts_a_pending_response()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        assert_eq!(
            AuthlibProvider::discover(
                Uuid::new_v4(),
                "Fixture",
                &url,
                false,
                &CancellationToken::new()
            )
            .await,
            Err(AuthError::InsecureProvider)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        let cancel = CancellationToken::new();
        let running = cancel.clone();
        let task = tokio::spawn(async move {
            AuthlibProvider::discover(Uuid::new_v4(), "Fixture", &url, true, &running).await
        });
        let (_held, _) = listener.accept().await?;
        cancel.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), task).await??,
            Err(AuthError::Cancelled)
        );
        Ok(())
    }

    #[tokio::test]
    async fn discovery_bounds_loops_and_rejects_untrusted_locations()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (url, task) = server(vec![
            (
                200,
                "X-Authlib-Injector-API-Location: /next/\r\n".into(),
                String::new(),
            ),
            (302, "Location: /\r\n".into(), String::new()),
        ])
        .await?;
        assert_eq!(discover(&url).await, Err(AuthError::ProviderDiscoveryLoop));
        task.await??;
        let (url, task) = server(
            (0..6)
                .map(|i| (302, format!("Location: /hop{}/\r\n", i + 1), String::new()))
                .collect(),
        )
        .await?;
        assert_eq!(discover(&url).await, Err(AuthError::ProviderDiscoveryLoop));
        assert_eq!(task.await??.len(), 6);
        for location in [
            "https://private:password@example.test/",
            "file:///private",
            "https://example.test/?private=token",
        ] {
            let (url, task) = server(vec![(
                302,
                format!("Location: {location}\r\n"),
                String::new(),
            )])
            .await?;
            assert_eq!(discover(&url).await, Err(AuthError::InvalidProviderAddress));
            task.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn discovery_maps_throttling_and_errors_without_echoing_provider_bodies()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (url, task) = server(vec![(
            429,
            "Retry-After: 17\r\n".into(),
            "PRIVATE_BODY".into(),
        )])
        .await?;
        assert_eq!(
            discover(&url).await,
            Err(AuthError::ProviderThrottled {
                retry_after_seconds: 17
            })
        );
        task.await??;
        let when = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(120));
        let (url, task) = server(vec![(
            429,
            format!("Retry-After: {when}\r\n"),
            "PRIVATE_BODY".into(),
        )])
        .await?;
        assert!(matches!(
            discover(&url).await,
            Err(AuthError::ProviderThrottled {
                retry_after_seconds: 110..=120
            })
        ));
        task.await??;
        for (status, body) in [
            (503, "PRIVATE_BODY".into()),
            (200, "<html>PRIVATE_BODY</html>".into()),
            (200, "{}".into()),
        ] {
            let (url, task) = server(vec![(status, String::new(), body)]).await?;
            let error = discover(&url)
                .await
                .err()
                .ok_or("Expected controlled error")?;
            assert!(!format!("{error:?} {error}").contains("PRIVATE_BODY"));
            task.await??;
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_times_out_a_stalled_provider_without_retrying()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        // Keep simulated time explicit while the operating system handles sockets.
        let runnable = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let task = tokio::spawn(async move { discover(&url).await });
        let (_held, _) = listener.accept().await?;
        tokio::time::advance(Duration::from_secs(21)).await;
        assert_eq!(task.await?, Err(AuthError::ProviderTimedOut));
        let next = tokio::time::timeout(Duration::from_millis(10), listener.accept());
        tokio::time::advance(Duration::from_millis(11)).await;
        assert!(next.await.is_err());
        runnable.abort();
        Ok(())
    }
}
