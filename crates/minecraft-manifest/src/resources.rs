//! Asset-object indexes and managed Java runtime metadata.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::Download;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetObjects {
    pub objects: BTreeMap<String, AssetObject>,
    #[serde(rename = "virtual", skip_serializing_if = "Option::is_none")]
    pub virtual_layout: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_to_resources: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

pub type RuntimeCatalog = BTreeMap<String, BTreeMap<String, Vec<RuntimeRelease>>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRelease {
    pub availability: RuntimeAvailability,
    pub manifest: Download,
    pub version: RuntimeVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAvailability {
    pub group: u32,
    pub progress: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeVersion {
    pub name: String,
    pub released: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub files: BTreeMap<String, RuntimeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum RuntimeFile {
    File {
        downloads: RuntimeDownloads,
        executable: bool,
    },
    Directory,
    Link {
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDownloads {
    pub raw: Download,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lzma: Option<Download>,
}
