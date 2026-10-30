use std::{fs, path::Path};

use block_launcher::manifest::{
    decode,
    resources::{AssetObjects, RuntimeCatalog, RuntimeManifest},
};
use serde::{Serialize, de::DeserializeOwned};

fn round_trip<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> anyhow::Result<T> {
    let parsed = decode::<T>(bytes)?;
    assert_eq!(
        serde_json::to_value(&parsed)?,
        serde_json::from_slice::<serde_json::Value>(bytes)?
    );
    Ok(parsed)
}

#[test]
fn all_real_asset_indexes_and_runtime_manifests_are_typed_losslessly() -> anyhow::Result<()> {
    let root = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/manifests/resources"
    ));
    let mut assets = 0;
    let mut runtimes = 0;
    let mut legacy = 0;
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid fixture filename"))?;
        let bytes = fs::read(&path)?;
        if name.starts_with("asset-") {
            let asset: AssetObjects = round_trip(&bytes)?;
            legacy += usize::from(
                asset.virtual_layout == Some(true) || asset.map_to_resources == Some(true),
            );
            assets += 1;
        } else if name.starts_with("runtime-") {
            let _: RuntimeManifest = round_trip(&bytes)?;
            runtimes += 1;
        } else {
            assert_eq!(name, "java-runtimes.json");
            let _: RuntimeCatalog = round_trip(&bytes)?;
        }
    }
    assert_eq!((assets, runtimes, legacy), (58, 29, 2));
    Ok(())
}
