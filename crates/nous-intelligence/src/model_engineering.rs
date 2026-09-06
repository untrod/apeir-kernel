use crate::{invalid, require, schema, IntelligenceError, Validate};
use nous_types::{ContractValidation, ModelKind, ModelSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureMetadata {
    pub architecture_id: String,
    #[serde(default)]
    pub layers: Option<u32>,
    #[serde(default)]
    pub hidden_dimension: Option<u64>,
    #[serde(default)]
    pub attention: String,
    #[serde(default)]
    pub experts: Option<u32>,
    #[serde(default)]
    pub activation: String,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub tokenizer_digest: String,
    #[serde(default)]
    pub tensor_shapes_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelExtension {
    None,
    Llm {
        context_length: u64,
        tokenizer: String,
        precision: String,
        #[serde(default)]
        generation: BTreeMap<String, Value>,
    },
    Vision {
        input_shape: Vec<u64>,
        #[serde(default)]
        classes: Vec<String>,
        #[serde(default)]
        preprocessing: Vec<String>,
        #[serde(default)]
        postprocessing: Vec<String>,
    },
    Mathematical {
        solver: String,
        #[serde(default)]
        variables: Vec<String>,
        #[serde(default)]
        parameters: BTreeMap<String, Value>,
        #[serde(default)]
        initial_conditions: BTreeMap<String, f64>,
        tolerance: f64,
    },
    Control {
        sampling_period_seconds: f64,
        state_dimension: u64,
        control_dimension: u64,
        #[serde(default)]
        stability: BTreeMap<String, String>,
    },
    Custom {
        name: String,
        #[serde(default)]
        values: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseGrant {
    pub id: String,
    pub source: String,
    pub derivative_works: Option<bool>,
    pub redistribution: Option<bool>,
    pub commercial_use: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfile {
    pub schema_version: u32,
    pub model: ModelSpec,
    pub architecture: ArchitectureMetadata,
    pub extension: ModelExtension,
    pub license: LicenseGrant,
    #[serde(default)]
    pub adapters: Vec<String>,
    #[serde(default)]
    pub quantization: String,
    #[serde(default)]
    pub dataset_lineage: Vec<String>,
    #[serde(default)]
    pub status: String,
}

impl Validate for ModelProfile {
    fn validate(&self) -> Result<(), IntelligenceError> {
        schema("ModelProfile", self.schema_version)?;
        self.model
            .validate()
            .map_err(|error| invalid("ModelProfile", "model", error.to_string()))?;
        require(
            "ModelProfile",
            "architecture.architecture_id",
            &self.architecture.architecture_id,
        )?;
        require("ModelProfile", "license.id", &self.license.id)?;
        require("ModelProfile", "license.source", &self.license.source)?;
        match (&self.model.kind, &self.extension) {
            (ModelKind::Vision, ModelExtension::Vision { input_shape, .. })
                if input_shape.is_empty() =>
            {
                Err(invalid(
                    "ModelProfile",
                    "extension.input_shape",
                    "vision input shape must not be empty",
                ))
            }
            (ModelKind::Vision, ModelExtension::Vision { .. })
            | (ModelKind::Foundation, ModelExtension::Llm { .. })
            | (
                ModelKind::Mathematical | ModelKind::Statistical | ModelKind::Optimization,
                ModelExtension::Mathematical { .. },
            )
            | (ModelKind::Control, ModelExtension::Control { .. })
            | (_, ModelExtension::Custom { .. } | ModelExtension::None) => Ok(()),
            _ => Err(invalid(
                "ModelProfile",
                "extension",
                "extension type is incompatible with ModelSpec kind",
            )),
        }?;
        match &self.extension {
            ModelExtension::Llm {
                context_length,
                tokenizer,
                precision,
                ..
            } => {
                if *context_length == 0 {
                    return Err(invalid(
                        "ModelProfile",
                        "extension.context_length",
                        "must be positive",
                    ));
                }
                require("ModelProfile", "extension.tokenizer", tokenizer)?;
                require("ModelProfile", "extension.precision", precision)?;
            }
            ModelExtension::Mathematical {
                solver, tolerance, ..
            } => {
                require("ModelProfile", "extension.solver", solver)?;
                if !tolerance.is_finite() || *tolerance <= 0.0 {
                    return Err(invalid(
                        "ModelProfile",
                        "extension.tolerance",
                        "must be finite and positive",
                    ));
                }
            }
            ModelExtension::Control {
                sampling_period_seconds,
                ..
            } if !sampling_period_seconds.is_finite() || *sampling_period_seconds <= 0.0 => {
                return Err(invalid(
                    "ModelProfile",
                    "extension.sampling_period_seconds",
                    "must be finite and positive",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDiff {
    pub identity_changed: bool,
    pub version_changed: bool,
    pub kind_changed: bool,
    pub architecture_changed: bool,
    pub tokenizer_changed: bool,
    pub backend_changed: bool,
    pub capability_changes: [Vec<String>; 2],
    pub artifact_changes: [Vec<String>; 2],
    pub adapter_changes: [Vec<String>; 2],
    pub quantization_changed: bool,
    pub evaluation_changed: bool,
    pub deployment_changed: bool,
    pub license_changed: bool,
}

impl ModelProfile {
    pub fn diff(&self, other: &Self) -> ModelDiff {
        ModelDiff {
            identity_changed: self.model.id != other.model.id,
            version_changed: self.model.version != other.model.version,
            kind_changed: self.model.kind != other.model.kind,
            architecture_changed: self.architecture != other.architecture,
            tokenizer_changed: self.architecture.tokenizer_digest
                != other.architecture.tokenizer_digest,
            backend_changed: self.model.backend != other.model.backend,
            capability_changes: string_set_diff(
                &self.model.capabilities,
                &other.model.capabilities,
            ),
            artifact_changes: string_set_diff(
                &self
                    .model
                    .artifacts
                    .iter()
                    .map(|artifact| artifact.sha256.clone())
                    .collect::<Vec<_>>(),
                &other
                    .model
                    .artifacts
                    .iter()
                    .map(|artifact| artifact.sha256.clone())
                    .collect::<Vec<_>>(),
            ),
            adapter_changes: string_set_diff(&self.adapters, &other.adapters),
            quantization_changed: self.quantization != other.quantization,
            evaluation_changed: self.model.evaluation != other.model.evaluation,
            deployment_changed: self.model.deployment != other.model.deployment,
            license_changed: self.license != other.license,
        }
    }
}

fn string_set_diff(left: &[String], right: &[String]) -> [Vec<String>; 2] {
    let left = left.iter().cloned().collect::<BTreeSet<_>>();
    let right = right.iter().cloned().collect::<BTreeSet<_>>();
    [
        left.difference(&right).cloned().collect(),
        right.difference(&left).cloned().collect(),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    Linear,
    Slerp,
    Ties,
    Dare,
    AdapterMerge,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeRequest {
    pub schema_version: u32,
    pub output_id: String,
    pub strategy: MergeStrategy,
    pub publish: bool,
    pub sources: Vec<ModelProfile>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeAssessment {
    pub compatible: bool,
    pub strategy: MergeStrategy,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn assess_merge(request: &MergeRequest) -> Result<MergeAssessment, IntelligenceError> {
    schema("MergeRequest", request.schema_version)?;
    require("MergeRequest", "output_id", &request.output_id)?;
    if request.sources.len() < 2 {
        return Err(invalid(
            "MergeRequest",
            "sources",
            "at least two source models are required",
        ));
    }
    for source in &request.sources {
        source.validate()?;
    }

    let baseline = &request.sources[0];
    let mut reasons = Vec::new();
    let mut warnings = Vec::new();
    for source in &request.sources {
        if source.model.kind != baseline.model.kind {
            reasons.push(format!("{} has a different model kind", source.model.id));
        }
        if source.architecture.architecture_id != baseline.architecture.architecture_id {
            reasons.push(format!("{} has a different architecture", source.model.id));
        }
        if source.architecture.tokenizer_digest != baseline.architecture.tokenizer_digest {
            reasons.push(format!("{} has an incompatible tokenizer", source.model.id));
        }
        if request.strategy != MergeStrategy::AdapterMerge
            && source.architecture.tensor_shapes_digest
                != baseline.architecture.tensor_shapes_digest
        {
            reasons.push(format!(
                "{} has incompatible tensor shapes",
                source.model.id
            ));
        }
        if source.license.derivative_works != Some(true) {
            reasons.push(format!(
                "{} license does not explicitly permit derivative works",
                source.model.id
            ));
        }
        if request.publish && source.license.redistribution != Some(true) {
            reasons.push(format!(
                "{} license does not explicitly permit redistribution",
                source.model.id
            ));
        }
        if source.license.commercial_use.is_none() {
            warnings.push(format!(
                "{} commercial-use permission is unknown",
                source.model.id
            ));
        }
    }
    reasons.sort();
    reasons.dedup();
    warnings.sort();
    warnings.dedup();
    Ok(MergeAssessment {
        compatible: reasons.is_empty(),
        strategy: request.strategy,
        reasons,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_types::{DeploymentSpec, EvaluationSpec, ModelBackend, ResourceVector};

    fn profile(id: &str) -> ModelProfile {
        ModelProfile {
            schema_version: 1,
            model: ModelSpec {
                schema_version: 1,
                id: id.into(),
                name: id.into(),
                kind: ModelKind::Foundation,
                version: "1.0.0".into(),
                capabilities: vec!["reasoning.generate".into()],
                input_schema: Value::Null,
                output_schema: Value::Null,
                backend: ModelBackend {
                    kind: "remote-api".into(),
                    entrypoint: String::new(),
                    runtime: "provider".into(),
                },
                resource_requirements: ResourceVector::default(),
                deployment: DeploymentSpec::default(),
                evaluation: EvaluationSpec::default(),
                artifacts: vec![],
                provenance: BTreeMap::new(),
                compatibility: BTreeMap::new(),
                security: BTreeMap::new(),
            },
            architecture: ArchitectureMetadata {
                architecture_id: "reference-transformer".into(),
                layers: Some(2),
                hidden_dimension: Some(16),
                attention: "mha".into(),
                experts: None,
                activation: "gelu".into(),
                context_length: Some(128),
                tokenizer_digest: "tokenizer-a".into(),
                tensor_shapes_digest: "shapes-a".into(),
            },
            extension: ModelExtension::Llm {
                context_length: 128,
                tokenizer: "reference".into(),
                precision: "fp32".into(),
                generation: BTreeMap::new(),
            },
            license: LicenseGrant {
                id: "Apache-2.0".into(),
                source: "LICENSE".into(),
                derivative_works: Some(true),
                redistribution: Some(true),
                commercial_use: Some(true),
            },
            adapters: vec![],
            quantization: String::new(),
            dataset_lineage: vec![],
            status: "validated".into(),
        }
    }

    #[test]
    fn merge_guard_accepts_compatible_sources_and_blocks_unknown_rights() {
        let mut request = MergeRequest {
            schema_version: 1,
            output_id: "merged".into(),
            strategy: MergeStrategy::Linear,
            publish: true,
            sources: vec![profile("a"), profile("b")],
            parameters: BTreeMap::new(),
        };
        assert!(assess_merge(&request).unwrap().compatible);
        request.sources[1].license.redistribution = None;
        let assessment = assess_merge(&request).unwrap();
        assert!(!assessment.compatible);
        assert!(assessment.reasons[0].contains("redistribution"));
    }

    #[test]
    fn model_diff_reports_architecture_and_adapter_changes() {
        let left = profile("a");
        let mut right = left.clone();
        right.architecture.layers = Some(3);
        right.adapters.push("adapter-1".into());
        let diff = left.diff(&right);
        assert!(diff.architecture_changed);
        assert_eq!(diff.adapter_changes[1], vec!["adapter-1"]);
    }
}
