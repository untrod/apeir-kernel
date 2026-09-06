//! Runtime Control Plane asset catalog.
//!
//! This crate is the sole state owner for developer assets exposed through the
//! Runtime API. It does not execute workloads and cannot mutate kernel state.
//! The kernel journal remains the authority for execution and recovery.

use nous_intelligence::{
    DatasetManifest, ExperimentManifest, IntelligenceGraph, ModelProfile, ProjectManifest,
    Validate as IntelligenceValidate,
};
use nous_types::{ArtifactContract, ContractValidation, ModelSpec};
pub use nous_types::{
    AssetKind, AssetSelector, ControlAsset, ListAssetsRequest, PutAssetRequest,
    CONTROL_API_SCHEMA_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("invalid control request: {0}")]
    Invalid(String),
    #[error("control asset not found: {kind}/{id}")]
    NotFound { kind: &'static str, id: String },
    #[error("control asset conflict: {0}")]
    Conflict(String),
    #[error("control-plane storage failure: {0}")]
    Storage(String),
}

pub struct ControlPlaneStore {
    connection: Mutex<Connection>,
}

impl ControlPlaneStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ControlPlaneError> {
        let connection = Connection::open(path).map_err(storage)?;
        Self::initialize(connection)
    }

    pub fn open_in_memory() -> Result<Self, ControlPlaneError> {
        let connection = Connection::open_in_memory().map_err(storage)?;
        Self::initialize(connection)
    }

    fn initialize(connection: Connection) -> Result<Self, ControlPlaneError> {
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS control_assets (
                    kind TEXT NOT NULL,
                    id TEXT NOT NULL,
                    version TEXT NOT NULL,
                    generation INTEGER NOT NULL,
                    sha256 TEXT NOT NULL,
                    document TEXT NOT NULL,
                    created_at_us INTEGER NOT NULL,
                    updated_at_us INTEGER NOT NULL,
                    PRIMARY KEY(kind, id)
                );
                CREATE INDEX IF NOT EXISTS idx_control_assets_kind
                    ON control_assets(kind, id);",
            )
            .map_err(storage)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn put(&self, request: PutAssetRequest) -> Result<ControlAsset, ControlPlaneError> {
        check_schema(request.schema_version)?;
        reject_embedded_secrets(&request.document)?;
        let (id, version) = validate_document(request.kind, &request.document)?;
        let encoded = serde_json::to_string(&request.document)
            .map_err(|error| ControlPlaneError::Invalid(error.to_string()))?;
        let sha256 = hex_digest(encoded.as_bytes());
        let now = chrono::Utc::now().timestamp_micros();
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage)?;
        let existing = select_asset(&transaction, request.kind, &id)?;
        let asset = match existing {
            Some(existing) if existing.sha256 == sha256 => existing,
            Some(existing) => {
                if request.expected_generation != Some(existing.generation) {
                    return Err(ControlPlaneError::Conflict(format!(
                        "{} generation is {}; expected_generation is required for update",
                        id, existing.generation
                    )));
                }
                let generation = existing.generation.saturating_add(1);
                transaction
                    .execute(
                        "UPDATE control_assets SET version=?1, generation=?2, sha256=?3,
                         document=?4, updated_at_us=?5 WHERE kind=?6 AND id=?7",
                        params![
                            version,
                            generation,
                            sha256,
                            encoded,
                            now,
                            request.kind.as_str(),
                            id
                        ],
                    )
                    .map_err(storage)?;
                ControlAsset {
                    schema_version: CONTROL_API_SCHEMA_VERSION,
                    kind: request.kind,
                    id,
                    version,
                    generation,
                    sha256,
                    document: request.document,
                    created_at_us: existing.created_at_us,
                    updated_at_us: now,
                }
            }
            None => {
                if request.expected_generation.is_some() {
                    return Err(ControlPlaneError::Conflict(
                        "expected_generation cannot be set when creating an asset".into(),
                    ));
                }
                transaction
                    .execute(
                        "INSERT INTO control_assets
                         (kind,id,version,generation,sha256,document,created_at_us,updated_at_us)
                         VALUES (?1,?2,?3,1,?4,?5,?6,?6)",
                        params![request.kind.as_str(), id, version, sha256, encoded, now],
                    )
                    .map_err(storage)?;
                ControlAsset {
                    schema_version: CONTROL_API_SCHEMA_VERSION,
                    kind: request.kind,
                    id,
                    version,
                    generation: 1,
                    sha256,
                    document: request.document,
                    created_at_us: now,
                    updated_at_us: now,
                }
            }
        };
        transaction.commit().map_err(storage)?;
        Ok(asset)
    }

    pub fn validate(
        &self,
        kind: AssetKind,
        document: &Value,
    ) -> Result<(String, String, String), ControlPlaneError> {
        reject_embedded_secrets(document)?;
        let (id, version) = validate_document(kind, document)?;
        let encoded = serde_json::to_vec(document)
            .map_err(|error| ControlPlaneError::Invalid(error.to_string()))?;
        Ok((id, version, hex_digest(&encoded)))
    }

    pub fn get(&self, selector: &AssetSelector) -> Result<ControlAsset, ControlPlaneError> {
        check_schema(selector.schema_version)?;
        required("id", &selector.id)?;
        select_asset(&*self.lock()?, selector.kind, &selector.id)?.ok_or_else(|| {
            ControlPlaneError::NotFound {
                kind: selector.kind.as_str(),
                id: selector.id.clone(),
            }
        })
    }

    pub fn list(
        &self,
        request: &ListAssetsRequest,
    ) -> Result<Vec<ControlAsset>, ControlPlaneError> {
        check_schema(request.schema_version)?;
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT kind,id,version,generation,sha256,document,created_at_us,updated_at_us
                 FROM control_assets WHERE kind=?1 ORDER BY id",
            )
            .map_err(storage)?;
        let assets = statement
            .query_map([request.kind.as_str()], row_to_asset)
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        Ok(assets)
    }

    pub fn delete(
        &self,
        selector: &AssetSelector,
        expected_generation: u64,
    ) -> Result<(), ControlPlaneError> {
        let asset = self.get(selector)?;
        if asset.generation != expected_generation {
            return Err(ControlPlaneError::Conflict(format!(
                "{} generation is {}, not {}",
                asset.id, asset.generation, expected_generation
            )));
        }
        let deleted = self
            .lock()?
            .execute(
                "DELETE FROM control_assets WHERE kind=?1 AND id=?2 AND generation=?3",
                params![selector.kind.as_str(), selector.id, expected_generation],
            )
            .map_err(storage)?;
        if deleted != 1 {
            return Err(ControlPlaneError::Conflict(
                "asset changed during deletion".into(),
            ));
        }
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ControlPlaneError> {
        self.connection
            .lock()
            .map_err(|_| ControlPlaneError::Storage("catalog lock is poisoned".into()))
    }
}

fn select_asset(
    connection: &Connection,
    kind: AssetKind,
    id: &str,
) -> Result<Option<ControlAsset>, ControlPlaneError> {
    connection
        .query_row(
            "SELECT kind,id,version,generation,sha256,document,created_at_us,updated_at_us
             FROM control_assets WHERE kind=?1 AND id=?2",
            params![kind.as_str(), id],
            row_to_asset,
        )
        .optional()
        .map_err(storage)
}

fn row_to_asset(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlAsset> {
    let kind: String = row.get(0)?;
    let document: String = row.get(5)?;
    Ok(ControlAsset {
        schema_version: CONTROL_API_SCHEMA_VERSION,
        kind: serde_json::from_value(Value::String(kind)).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        id: row.get(1)?,
        version: row.get(2)?,
        generation: row.get(3)?,
        sha256: row.get(4)?,
        document: serde_json::from_str(&document).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        created_at_us: row.get(6)?,
        updated_at_us: row.get(7)?,
    })
}

fn validate_document(
    kind: AssetKind,
    document: &Value,
) -> Result<(String, String), ControlPlaneError> {
    match kind {
        AssetKind::Project => {
            let value: ProjectManifest = decode(document)?;
            value.validate().map_err(invalid_contract)?;
            Ok((value.id, value.version))
        }
        AssetKind::Model => {
            if let Ok(value) = serde_json::from_value::<ModelProfile>(document.clone()) {
                value.validate().map_err(invalid_contract)?;
                return Ok((value.model.id, value.model.version));
            }
            let value: ModelSpec = decode(document)?;
            value.validate().map_err(invalid_contract)?;
            Ok((value.id, value.version))
        }
        AssetKind::Graph => {
            let value: IntelligenceGraph = decode(document)?;
            value.validate().map_err(invalid_contract)?;
            Ok((value.id, value.version))
        }
        AssetKind::Dataset => {
            let value: DatasetManifest = decode(document)?;
            value.validate().map_err(invalid_contract)?;
            Ok((value.id, value.version))
        }
        AssetKind::Experiment => {
            let value: ExperimentManifest = decode(document)?;
            value.validate().map_err(invalid_contract)?;
            Ok((value.id, value.version))
        }
        AssetKind::Artifact => {
            let value: ArtifactContract = decode(document)?;
            value.validate().map_err(invalid_contract)?;
            Ok((value.id, value.version))
        }
    }
}

fn decode<T: for<'de> Deserialize<'de>>(document: &Value) -> Result<T, ControlPlaneError> {
    serde_json::from_value(document.clone())
        .map_err(|error| ControlPlaneError::Invalid(error.to_string()))
}

fn invalid_contract(error: impl std::fmt::Display) -> ControlPlaneError {
    ControlPlaneError::Invalid(error.to_string())
}

fn reject_embedded_secrets(value: &Value) -> Result<(), ControlPlaneError> {
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                if matches!(
                    normalized.as_str(),
                    "api_key" | "token" | "secret" | "password" | "access_token" | "session_token"
                ) && !value.is_null()
                    && value.as_str() != Some("")
                {
                    return Err(ControlPlaneError::Invalid(format!(
                        "embedded secret field '{key}' is forbidden; use an environment reference"
                    )));
                }
                reject_embedded_secrets(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_embedded_secrets(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn required(field: &str, value: &str) -> Result<(), ControlPlaneError> {
    if value.trim().is_empty() {
        return Err(ControlPlaneError::Invalid(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn check_schema(version: u32) -> Result<(), ControlPlaneError> {
    if version != CONTROL_API_SCHEMA_VERSION {
        return Err(ControlPlaneError::Invalid(format!(
            "expected schema version {CONTROL_API_SCHEMA_VERSION}, found {version}"
        )));
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn storage(error: rusqlite::Error) -> ControlPlaneError {
    ControlPlaneError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(name: &str) -> Value {
        json!({
            "schema_version": 1,
            "id": name,
            "name": name,
            "version": "1.0.0",
            "description": "test",
            "license": "Apache-2.0",
            "models": [], "algorithms": [], "datasets": [], "providers": [],
            "workflows": [], "policies": [], "experiments": [], "deployments": [],
            "secret_refs": {}, "metadata": {}
        })
    }

    #[test]
    fn catalog_is_idempotent_and_uses_generation_cas() {
        let store = ControlPlaneStore::open_in_memory().unwrap();
        let created = store
            .put(PutAssetRequest {
                schema_version: 1,
                kind: AssetKind::Project,
                document: project("p"),
                expected_generation: None,
            })
            .unwrap();
        assert_eq!(created.generation, 1);
        let replay = store
            .put(PutAssetRequest {
                schema_version: 1,
                kind: AssetKind::Project,
                document: project("p"),
                expected_generation: None,
            })
            .unwrap();
        assert_eq!(replay.generation, 1);
        let mut changed = project("p");
        changed["description"] = json!("changed");
        assert!(matches!(
            store.put(PutAssetRequest {
                schema_version: 1,
                kind: AssetKind::Project,
                document: changed.clone(),
                expected_generation: None,
            }),
            Err(ControlPlaneError::Conflict(_))
        ));
        let updated = store
            .put(PutAssetRequest {
                schema_version: 1,
                kind: AssetKind::Project,
                document: changed,
                expected_generation: Some(1),
            })
            .unwrap();
        assert_eq!(updated.generation, 2);
    }

    #[test]
    fn catalog_persists_and_lists_typed_assets() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("control.db");
        ControlPlaneStore::open(&path)
            .unwrap()
            .put(PutAssetRequest {
                schema_version: 1,
                kind: AssetKind::Project,
                document: project("p"),
                expected_generation: None,
            })
            .unwrap();
        let reopened = ControlPlaneStore::open(&path).unwrap();
        let assets = reopened
            .list(&ListAssetsRequest {
                schema_version: 1,
                kind: AssetKind::Project,
            })
            .unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].id, "p");
    }

    #[test]
    fn catalog_rejects_embedded_credentials() {
        let store = ControlPlaneStore::open_in_memory().unwrap();
        let mut document = project("p");
        document["metadata"]["api_key"] = json!("not-allowed");
        assert!(matches!(
            store.put(PutAssetRequest {
                schema_version: 1,
                kind: AssetKind::Project,
                document,
                expected_generation: None,
            }),
            Err(ControlPlaneError::Invalid(_))
        ));
    }

    #[test]
    fn catalog_validates_every_public_asset_kind() {
        let store = ControlPlaneStore::open_in_memory().unwrap();
        let yaml_assets = [
            (
                AssetKind::Project,
                include_str!("../../../examples/intelligence-platform/project.yaml"),
            ),
            (
                AssetKind::Model,
                include_str!("../../../examples/intelligence-platform/models/linear.profile.yaml"),
            ),
            (
                AssetKind::Graph,
                include_str!("../../../examples/intelligence-platform/workflows/math.graph.yaml"),
            ),
            (
                AssetKind::Dataset,
                include_str!(
                    "../../../examples/intelligence-platform/datasets/reference.dataset.yaml"
                ),
            ),
            (
                AssetKind::Experiment,
                include_str!(
                    "../../../examples/intelligence-platform/experiments/baseline.experiment.yaml"
                ),
            ),
        ];
        for (kind, yaml) in yaml_assets {
            let document: Value = serde_yaml::from_str(yaml).unwrap();
            store
                .put(PutAssetRequest {
                    schema_version: CONTROL_API_SCHEMA_VERSION,
                    kind,
                    document,
                    expected_generation: None,
                })
                .unwrap();
        }
        store
            .put(PutAssetRequest {
                schema_version: CONTROL_API_SCHEMA_VERSION,
                kind: AssetKind::Artifact,
                document: json!({
                    "schema_version": 1,
                    "id": "report",
                    "version": "1.0.0",
                    "media_type": "text/markdown",
                    "location": "artifacts/report.md",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "creator": "test",
                    "metadata": {}
                }),
                expected_generation: None,
            })
            .unwrap();
    }
}
