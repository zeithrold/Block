use super::super::repair_existing;
use super::fixtures::{Fixture, write};
use crate::plan::ResourcePath;
use anyhow::Context as _;
use sha1::{Digest as _, Sha1};
use std::fs;

#[tokio::test]
async fn local_only_digest_receipt_rejects_changed_loader_and_repair_without_upstream_descriptor()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let metadata = fixture.resolve().await?;
    let receipt = metadata
        .existing
        .context("missing existing inputs")?
        .receipt;
    let receipt_path = fixture.context.store.root().join(format!(
        "installations/{}/{}/existing.json",
        fixture.instance.id, fixture.instance.target_id
    ));
    write(&receipt_path, &serde_json::to_vec(&receipt)?)?;
    let logical = "libraries/example/override/1/override-1.jar";
    fs::write(fixture.root().join(logical), b"changed loader")?;
    assert!(
        fixture
            .resolve()
            .await
            .err()
            .context("changed loader was accepted")?
            .to_string()
            .contains("corrupt")
    );
    let error = repair_existing(
        &fixture.instance,
        &fixture.instance.targets[0],
        &fixture.context,
    )
    .await
    .err()
    .context("unverified source repair was accepted")?;
    assert!(error.to_string().contains("no trustworthy"));
    assert_eq!(fs::read(fixture.root().join(logical))?, b"changed loader");
    Ok(())
}

#[tokio::test]
async fn explicit_repair_reuses_verified_cache_and_preserves_corrupt_client_bytes()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let client = b"client fixture";
    let sha1 = format!("{:x}", Sha1::digest(client));
    write(&fixture.context.store.object_path(&sha1)?, client)?;
    let source = fixture.root().join("versions/1.20.1/1.20.1.jar");
    fs::write(&source, b"corrupt client")?;
    let metadata = repair_existing(
        &fixture.instance,
        &fixture.instance.targets[0],
        &fixture.context,
    )
    .await?;
    let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    plan.verify_existing_resources()?;
    let replacement = metadata
        .existing
        .as_ref()
        .context("existing inputs missing")?
        .receipt
        .entries
        .iter()
        .find(|entry| entry.logical_path == "versions/1.20.1/1.20.1.jar")
        .context("repair receipt entry missing")?;
    assert!(matches!(replacement.path, ResourcePath::Cache { .. }));
    assert_eq!(
        fs::read(
            replacement
                .path
                .resolve(&fixture.context.store.root().canonicalize()?)?
        )?,
        client
    );
    assert_eq!(fs::read(source)?, b"corrupt client");
    let receipt_path = fixture.context.store.root().join(format!(
        "installations/{}/{}/existing.json",
        fixture.instance.id, fixture.instance.target_id
    ));
    write(
        &receipt_path,
        &serde_json::to_vec(
            &metadata
                .existing
                .context("existing inputs missing")?
                .receipt,
        )?,
    )?;
    let replay = fixture.resolve().await?;
    assert!(crate::plan::compile_existing(&fixture.instance, &replay)?.command.classpath.iter().any(|path| matches!(path, ResourcePath::Cache { relative } if relative.contains("/repairs/"))));
    Ok(())
}

#[tokio::test]
async fn repairing_one_asset_builds_a_complete_cache_assets_root_without_source_writes()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let asset = b"asset fixture";
    let hash = format!("{:x}", Sha1::digest(asset));
    write(&fixture.context.store.object_path(&hash)?, asset)?;
    let original = fixture
        .root()
        .join(format!("assets/objects/{}/{hash}", &hash[..2]));
    fs::write(&original, b"corrupt asset")?;
    let index = fs::read(fixture.root().join("assets/indexes/test.json"))?;
    let metadata = repair_existing(
        &fixture.instance,
        &fixture.instance.targets[0],
        &fixture.context,
    )
    .await?;
    let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    plan.verify_existing_resources()?;
    let root = plan
        .existing_asset_root
        .context("repaired asset root missing")?;
    assert!(root.starts_with(fixture.context.store.root().canonicalize()?));
    assert_eq!(fs::read(root.join("indexes/test.json"))?, index);
    assert_eq!(
        fs::read(root.join(format!("objects/{}/{hash}", &hash[..2])))?,
        asset
    );
    assert_eq!(fs::read(original)?, b"corrupt asset");
    assert_eq!(
        fs::read(fixture.root().join("assets/indexes/test.json"))?,
        index
    );
    Ok(())
}
