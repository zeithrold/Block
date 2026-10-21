use std::{fs, path::Path, sync::Arc, time::Duration};

use block_launcher::{
    CancellationToken, DownloadOptions, ProxyMode, catalog::InstallStatus, library::Library,
    recipe::RuntimeSelection, session::OfflineSession,
};

#[test]
fn custom_download_policy_preserves_existing_catalog_and_default_constructor() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let original = Library::new(root.path().to_owned(), Arc::new(|_| {}), Arc::new(|_| {}))?;
    assert_eq!(original.download_options(), &DownloadOptions::default());
    let (instance, profile) =
        original
            .catalog()
            .create_instance("Existing world".into(), "1.20.6".into(), None, None)?;
    let options = DownloadOptions {
        max_concurrent: 2,
        max_retries: 0,
        bytes_per_second: Some(4096),
        proxy: ProxyMode::Direct,
    };
    let configured = Library::with_options(
        root.path().to_owned(),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
        options.clone(),
    )?;
    assert_eq!(configured.download_options(), &options);
    let state = configured.catalog().snapshot()?;
    assert_eq!(state.selection(&profile.id)?.0.id, instance.id);
    assert_eq!(state.selection(&profile.id)?.1.name, profile.name);
    Ok(())
}

#[test]
fn invalid_download_policy_is_rejected_before_creating_a_catalog() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("not-created");
    let options = DownloadOptions {
        max_concurrent: 0,
        ..DownloadOptions::default()
    };
    assert!(
        Library::with_options(root.clone(), Arc::new(|_| {}), Arc::new(|_| {}), options).is_err()
    );
    assert!(!root.exists());
    Ok(())
}

#[tokio::test]
async fn launch_preparation_keeps_missing_resources_offline_with_an_explicit_proxy()
-> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let root = tempfile::tempdir()?;
    let options = DownloadOptions {
        max_retries: 0,
        bytes_per_second: Some(1),
        proxy: ProxyMode::Http { url: proxy.clone() },
        ..DownloadOptions::default()
    };
    let library = Library::with_options(
        root.path().to_owned(),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
        options,
    )?;
    let (instance, mut profile) = library.catalog().create_instance(
        "Offline integrity check".into(),
        "26.3".into(),
        None,
        None,
    )?;
    profile.runtime = RuntimeSelection::Local {
        java: std::env::current_exe()?,
    };
    library.catalog().save_profile(profile.clone())?;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../minecraft-manifest/tests/fixtures/26.3.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(fixture)?)?;
    manifest["assetIndex"]["url"] = serde_json::json!(format!("{proxy}/missing-asset-index"));
    let manifest_path = root.path().join("installed.json");
    fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
    let mut target = instance.targets[0].clone();
    target.manifest = Some(manifest_path);
    target.status = InstallStatus::Ready;
    library.catalog().set_target(&instance.id, &target)?;
    let session = OfflineSession::Offline {
        username: "Player".into(),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        library.prepare_launch(&profile.id, &session, &CancellationToken::new(), None, None),
    )
    .await?;
    let Err(error) = result else {
        anyhow::bail!("missing resources unexpectedly prepared successfully")
    };
    assert!(format!("{error:#}").contains("missing or corrupt offline resource"));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}
