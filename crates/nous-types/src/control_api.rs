//! Transport-neutral Runtime Control API contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONTROL_API_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Project,
    Model,
    Graph,
    Dataset,
    Experiment,
    Artifact,
}

impl AssetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Model => "model",
            Self::Graph => "graph",
            Self::Dataset => "dataset",
            Self::Experiment => "experiment",
            Self::Artifact => "artifact",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutAssetRequest {
    pub schema_version: u32,
    pub kind: AssetKind,
    pub document: Value,
    #[serde(default)]
    pub expected_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSelector {
    pub schema_version: u32,
    pub kind: AssetKind,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListAssetsRequest {
    pub schema_version: u32,
    pub kind: AssetKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlAsset {
    pub schema_version: u32,
    pub kind: AssetKind,
    pub id: String,
    pub version: String,
    pub generation: u64,
    pub sha256: String,
    pub document: Value,
    pub created_at_us: i64,
    pub updated_at_us: i64,
}
