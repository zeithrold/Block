use std::{fs, path::Path};

use anyhow::ensure;
use minecraft_manifest::{
    decode,
    model::{
        Argument, ArgumentValue, LegacyVersionEntry, VersionEntry, VersionIndex, VersionManifest,
    },
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha1::{Digest as _, Sha1};

fn fixtures() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
}

fn assert_round_trip<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> anyhow::Result<T> {
    let parsed: T = decode(bytes)?;
    assert_eq!(
        serde_json::to_value(&parsed)?,
        serde_json::from_slice::<Value>(bytes)?
    );
    Ok(parsed)
}

#[test]
fn official_fixtures_match_their_hashes_and_round_trip() -> anyhow::Result<()> {
    let sources: Vec<VersionEntry> = decode(&fs::read(fixtures().join("sources.json"))?)?;
    for source in sources {
        let bytes = fs::read(fixtures().join(format!("{}.json", source.id)))?;
        assert_eq!(format!("{:x}", Sha1::digest(&bytes)), source.sha1);
        let manifest: VersionManifest = assert_round_trip(&bytes)?;
        assert_eq!(manifest.id, source.id);
        assert_eq!(manifest.kind, source.kind);
    }
    Ok(())
}

#[test]
fn both_index_generations_round_trip() -> anyhow::Result<()> {
    let v1: VersionIndex<LegacyVersionEntry> =
        assert_round_trip(&fs::read(fixtures().join("version_manifest.json"))?)?;
    let v2: VersionIndex =
        assert_round_trip(&fs::read(fixtures().join("version_manifest_v2.json"))?)?;
    assert_eq!(v1.latest, v2.latest);
    assert_eq!(v1.versions.len(), 2);
    assert_eq!(v2.versions.len(), 2);
    Ok(())
}

#[test]
fn legacy_and_modern_argument_generations_remain_distinct() -> anyhow::Result<()> {
    let legacy: VersionManifest = decode(&fs::read(fixtures().join("13w38c.json"))?)?;
    assert!(legacy.minecraft_arguments.is_some());
    assert!(legacy.arguments.is_none());
    assert!(legacy.java_version.is_none());
    assert!(legacy.compliance_level.is_none());
    assert!(legacy.logging.is_none());
    let modern: VersionManifest = decode(&fs::read(fixtures().join("26.4-snapshot-4.json"))?)?;
    let arguments = modern
        .arguments
        .ok_or_else(|| anyhow::anyhow!("missing modern arguments"))?;
    assert!(modern.minecraft_arguments.is_none());
    assert!(arguments.default_user_jvm.is_some());
    ensure!(arguments.game.iter().any(|argument| matches!(argument,
        Argument::Conditional(conditional) if matches!(&conditional.value, ArgumentValue::Single(_)))));
    ensure!(arguments.jvm.iter().any(|argument| matches!(argument,
        Argument::Conditional(conditional) if matches!(&conditional.value, ArgumentValue::Multiple(_)))));
    Ok(())
}

#[test]
fn unknown_root_and_nested_fields_are_rejected() -> anyhow::Result<()> {
    let original: Value = serde_json::from_slice(&fs::read(fixtures().join("26.3.json"))?)?;
    for pointer in ["", "/assetIndex", "/libraries/0/downloads/artifact"] {
        let mut modified = original.clone();
        let object = modified
            .pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow::anyhow!("fixture object is missing at {pointer}"))?;
        object.insert("unexpectedSchemaField".into(), json!(true));
        let error = decode::<VersionManifest>(&serde_json::to_vec(&modified)?)
            .err()
            .ok_or_else(|| anyhow::anyhow!("unknown field was silently accepted"))?;
        assert!(error.to_string().contains("unexpectedSchemaField"));
    }
    Ok(())
}

#[test]
fn conditional_arguments_do_not_swallow_unknown_fields_or_types() -> anyhow::Result<()> {
    let bytes = fs::read(fixtures().join("26.3.json"))?;
    for argument in [
        json!({"value": "--demo", "unexpected": true}),
        json!({"value": 42}),
    ] {
        let mut modified: Value = serde_json::from_slice(&bytes)?;
        modified["arguments"]["game"][0] = argument;
        assert!(decode::<VersionManifest>(&serde_json::to_vec(&modified)?).is_err());
    }
    Ok(())
}

#[test]
fn missing_required_fields_and_trailing_json_are_rejected() -> anyhow::Result<()> {
    let mut modified: Value = serde_json::from_slice(&fs::read(fixtures().join("26.3.json"))?)?;
    modified
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("expected object"))?
        .remove("libraries");
    assert!(decode::<VersionManifest>(&serde_json::to_vec(&modified)?).is_err());
    assert!(
        decode::<VersionIndex>(br#"{"latest":{"release":"x","snapshot":"x"},"versions":[]} {}"#)
            .is_err()
    );
    Ok(())
}
