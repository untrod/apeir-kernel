use crate::{invalid, require, schema, valid_identifier, IntelligenceError, Validate};
use nous_types::{ArtifactContract, ContractValidation, ResourceVector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetSplit {
    pub name: String,
    pub artifact: ArtifactContract,
    #[serde(default)]
    pub schema: Value,
    #[serde(default)]
    pub rows: Option<u64>,
}

impl Validate for DatasetSplit {
    fn validate(&self) -> Result<(), IntelligenceError> {
        require("DatasetSplit", "name", &self.name)?;
        self.artifact
            .validate()
            .map_err(|error| invalid("DatasetSplit", "artifact", error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub license: String,
    #[serde(default)]
    pub splits: Vec<DatasetSplit>,
    #[serde(default)]
    pub transforms: Vec<String>,
    #[serde(default)]
    pub provenance: BTreeMap<String, String>,
    #[serde(default)]
    pub quality: BTreeMap<String, f64>,
}

impl Validate for DatasetManifest {
    fn validate(&self) -> Result<(), IntelligenceError> {
        schema("DatasetManifest", self.schema_version)?;
        if !valid_identifier(&self.id) {
            return Err(invalid("DatasetManifest", "id", "invalid identifier"));
        }
        require("DatasetManifest", "version", &self.version)?;
        require("DatasetManifest", "license", &self.license)?;
        if self.splits.is_empty() {
            return Err(invalid(
                "DatasetManifest",
                "splits",
                "must contain at least one split",
            ));
        }
        let mut names = BTreeSet::new();
        for split in &self.splits {
            split.validate()?;
            if !names.insert(&split.name) {
                return Err(invalid(
                    "DatasetManifest",
                    "splits",
                    format!("duplicate split '{}'", split.name),
                ));
            }
        }
        if self.quality.values().any(|value| !value.is_finite()) {
            return Err(invalid(
                "DatasetManifest",
                "quality",
                "quality values must be finite",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    Number(f64),
    Text(String),
    Boolean(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    Created,
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub model: String,
    pub dataset: String,
    pub code_revision: String,
    pub runtime_version: String,
    pub seed: u64,
    pub status: ExperimentStatus,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    #[serde(default)]
    pub resources: ResourceVector,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub metrics: BTreeMap<String, MetricValue>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactContract>,
}

impl Validate for ExperimentManifest {
    fn validate(&self) -> Result<(), IntelligenceError> {
        schema("ExperimentManifest", self.schema_version)?;
        if !valid_identifier(&self.id) {
            return Err(invalid("ExperimentManifest", "id", "invalid identifier"));
        }
        for (field, value) in [
            ("version", self.version.as_str()),
            ("model", self.model.as_str()),
            ("dataset", self.dataset.as_str()),
            ("code_revision", self.code_revision.as_str()),
            ("runtime_version", self.runtime_version.as_str()),
        ] {
            require("ExperimentManifest", field, value)?;
        }
        if self
            .metrics
            .values()
            .any(|metric| matches!(metric, MetricValue::Number(value) if !value.is_finite()))
        {
            return Err(invalid(
                "ExperimentManifest",
                "metrics",
                "numeric metrics must be finite",
            ));
        }
        for artifact in &self.artifacts {
            artifact
                .validate()
                .map_err(|error| invalid("ExperimentManifest", "artifacts", error.to_string()))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentDiff {
    pub model_changed: bool,
    pub dataset_changed: bool,
    pub code_changed: bool,
    pub seed_changed: bool,
    pub parameter_changes: BTreeMap<String, [Option<Value>; 2]>,
    pub metric_changes: BTreeMap<String, [Option<MetricValue>; 2]>,
}

impl ExperimentManifest {
    pub fn diff(&self, other: &Self) -> ExperimentDiff {
        ExperimentDiff {
            model_changed: self.model != other.model,
            dataset_changed: self.dataset != other.dataset,
            code_changed: self.code_revision != other.code_revision,
            seed_changed: self.seed != other.seed,
            parameter_changes: map_diff(&self.parameters, &other.parameters),
            metric_changes: map_diff(&self.metrics, &other.metrics),
        }
    }
}

fn map_diff<T: Clone + PartialEq>(
    left: &BTreeMap<String, T>,
    right: &BTreeMap<String, T>,
) -> BTreeMap<String, [Option<T>; 2]> {
    left.keys()
        .chain(right.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|key| {
            let before = left.get(key).cloned();
            let after = right.get(key).cloned();
            (before != after).then(|| (key.clone(), [before, after]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiment_diff_reports_reproducibility_changes() {
        let left = ExperimentManifest {
            schema_version: 1,
            id: "exp".into(),
            version: "1".into(),
            model: "m1".into(),
            dataset: "d1".into(),
            code_revision: "abc".into(),
            runtime_version: "0.1.0".into(),
            seed: 42,
            status: ExperimentStatus::Completed,
            parameters: BTreeMap::from([("lr".into(), serde_json::json!(0.1))]),
            resources: ResourceVector::default(),
            environment: BTreeMap::new(),
            metrics: BTreeMap::new(),
            artifacts: vec![],
        };
        left.validate().unwrap();
        let mut right = left.clone();
        right.seed = 7;
        right.parameters.insert("lr".into(), serde_json::json!(0.2));
        let diff = left.diff(&right);
        assert!(diff.seed_changed);
        assert!(diff.parameter_changes.contains_key("lr"));
    }
}
