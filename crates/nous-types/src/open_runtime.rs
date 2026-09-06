//! Versioned extension contracts for the open runtime.
//!
//! These are declarative boundary types. They do not own execution or durable
//! state; the existing kernel, journal, scheduler, and provider host remain the
//! sole production authorities.

use crate::ResourceVector;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use thiserror::Error;

pub const OPEN_RUNTIME_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {contract} field '{field}': {message}")]
pub struct RuntimeContractError {
    pub contract: &'static str,
    pub field: &'static str,
    pub message: String,
}

pub trait ContractValidation {
    fn validate(&self) -> Result<(), RuntimeContractError>;
}

fn required(
    contract: &'static str,
    field: &'static str,
    value: &str,
) -> Result<(), RuntimeContractError> {
    if value.trim().is_empty() {
        return Err(RuntimeContractError {
            contract,
            field,
            message: "must not be empty".into(),
        });
    }
    Ok(())
}

fn schema(contract: &'static str, version: u32) -> Result<(), RuntimeContractError> {
    if version != OPEN_RUNTIME_SCHEMA_VERSION {
        return Err(RuntimeContractError {
            contract,
            field: "schema_version",
            message: format!("expected {OPEN_RUNTIME_SCHEMA_VERSION}, found {version}"),
        });
    }
    Ok(())
}

fn capability_name(value: &str) -> bool {
    let mut segments = value.split('.');
    let valid = |segment: &str| {
        !segment.is_empty()
            && segment.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '_' | '-')
            })
    };
    segments.by_ref().all(valid) && value.contains('.')
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityContract {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub constraints: BTreeMap<String, Value>,
    pub requirements: BTreeMap<String, Value>,
    pub metadata: BTreeMap<String, String>,
}

impl ContractValidation for CapabilityContract {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("CapabilityContract", self.schema_version)?;
        if !capability_name(&self.id) {
            return Err(RuntimeContractError {
                contract: "CapabilityContract",
                field: "id",
                message: "must be a namespaced identifier such as reasoning.generate".into(),
            });
        }
        required("CapabilityContract", "version", &self.version)
    }
}

macro_rules! simple_contract {
    ($name:ident, $label:literal, {$($field:ident: $type:ty),* $(,)?}) => {
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct $name {
            pub schema_version: u32,
            pub id: String,
            pub version: String,
            $(pub $field: $type,)*
            pub metadata: BTreeMap<String, String>,
        }

        impl ContractValidation for $name {
            fn validate(&self) -> Result<(), RuntimeContractError> {
                schema($label, self.schema_version)?;
                required($label, "id", &self.id)?;
                required($label, "version", &self.version)
            }
        }
    };
}

simple_contract!(TaskContract, "TaskContract", {
    capability: String,
    input: Value,
    context_id: String,
    policy_ids: Vec<String>
});
simple_contract!(ProviderContract, "ProviderContract", {
    manifest: String,
    capabilities: Vec<String>,
    lifecycle: Vec<String>
});
simple_contract!(ResourceContract, "ResourceContract", {
    requirements: ResourceVector,
    labels: BTreeMap<String, String>
});
simple_contract!(ContextContract, "ContextContract", {
    workspace: String,
    trace_id: String,
    values: BTreeMap<String, Value>
});
simple_contract!(ArtifactContract, "ArtifactContract", {
    media_type: String,
    location: String,
    sha256: String,
    creator: String
});
simple_contract!(PolicyContract, "PolicyContract", {
    rules: Vec<Value>,
    enforcement: String
});
simple_contract!(IdentityContract, "IdentityContract", {
    principal: String,
    roles: Vec<String>
});
simple_contract!(NodeContract, "NodeContract", {
    capabilities: Vec<String>,
    resources: ResourceVector,
    endpoint: String
});
simple_contract!(WorkflowContract, "WorkflowContract", {
    steps: Vec<String>,
    edges: Vec<[String; 2]>
});

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EventContract {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub event_type: String,
    pub source: String,
    pub timestamp: String,
    pub payload: Value,
    pub metadata: BTreeMap<String, Value>,
}

impl ContractValidation for EventContract {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("EventContract", self.schema_version)?;
        required("EventContract", "id", &self.id)?;
        required("EventContract", "version", &self.version)?;
        required("EventContract", "event_type", &self.event_type)?;
        required("EventContract", "source", &self.source)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Foundation,
    Vision,
    Speech,
    Embedding,
    Mathematical,
    Statistical,
    Optimization,
    Control,
    Simulation,
    Analytical,
    #[default]
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelBackend {
    pub kind: String,
    pub entrypoint: String,
    pub runtime: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeploymentSpec {
    pub mode: String,
    pub target: String,
    pub replicas: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvaluationSpec {
    pub dataset: String,
    pub metrics: Vec<String>,
    pub thresholds: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelSpec {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub kind: ModelKind,
    pub version: String,
    pub capabilities: Vec<String>,
    pub input_schema: Value,
    pub output_schema: Value,
    pub backend: ModelBackend,
    pub resource_requirements: ResourceVector,
    pub deployment: DeploymentSpec,
    pub evaluation: EvaluationSpec,
    pub artifacts: Vec<ArtifactContract>,
    pub provenance: BTreeMap<String, String>,
    pub compatibility: BTreeMap<String, String>,
    pub security: BTreeMap<String, String>,
}

impl ContractValidation for ModelSpec {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("ModelSpec", self.schema_version)?;
        required("ModelSpec", "id", &self.id)?;
        required("ModelSpec", "name", &self.name)?;
        required("ModelSpec", "version", &self.version)?;
        required("ModelSpec", "backend.kind", &self.backend.kind)?;
        if !self.backend.entrypoint.is_empty() {
            portable_reference("ModelSpec", "backend.entrypoint", &self.backend.entrypoint)?;
        }
        if self.capabilities.is_empty()
            || self.capabilities.iter().any(|item| !capability_name(item))
        {
            return Err(RuntimeContractError {
                contract: "ModelSpec",
                field: "capabilities",
                message: "must contain valid namespaced capability identifiers".into(),
            });
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    pub backend: String,
    pub runtime_class: String,
    pub entrypoint: String,
    pub model: String,
    pub endpoint: String,
    pub credential_env: String,
    pub capabilities: Vec<String>,
    pub lifecycle: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}

impl ContractValidation for ProviderManifest {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("ProviderManifest", self.schema_version)?;
        required("ProviderManifest", "name", &self.name)?;
        required("ProviderManifest", "version", &self.version)?;
        required("ProviderManifest", "backend", &self.backend)?;
        if self.capabilities.iter().any(|item| !capability_name(item)) {
            return Err(RuntimeContractError {
                contract: "ProviderManifest",
                field: "capabilities",
                message: "contains an invalid capability identifier".into(),
            });
        }
        if !self.credential_env.is_empty()
            && (!self
                .credential_env
                .chars()
                .next()
                .is_some_and(|character| character == '_' || character.is_ascii_uppercase())
                || !self.credential_env.chars().all(|character| {
                    character == '_' || character.is_ascii_uppercase() || character.is_ascii_digit()
                }))
        {
            return Err(RuntimeContractError {
                contract: "ProviderManifest",
                field: "credential_env",
                message: "must be an environment-variable name, not a secret".into(),
            });
        }
        if !self.endpoint.is_empty()
            && (self.endpoint.contains('@')
                || self.endpoint.contains('?')
                || self.endpoint.contains('#'))
        {
            return Err(RuntimeContractError {
                contract: "ProviderManifest",
                field: "endpoint",
                message: "must not contain credentials, query parameters, or fragments".into(),
            });
        }
        if self.backend == "external-process" {
            required("ProviderManifest", "entrypoint", &self.entrypoint)?;
            portable_reference("ProviderManifest", "entrypoint", &self.entrypoint)?;
            let required_lifecycle = [
                "probe",
                "metadata",
                "capabilities",
                "health",
                "execute",
                "cancel",
                "shutdown",
            ];
            if required_lifecycle
                .iter()
                .any(|operation| !self.lifecycle.iter().any(|item| item == operation))
            {
                return Err(RuntimeContractError {
                    contract: "ProviderManifest",
                    field: "lifecycle",
                    message:
                        "external providers must declare the complete Provider SDK v1 lifecycle"
                            .into(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PackManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    pub description: String,
    pub models: Vec<String>,
    pub providers: Vec<String>,
    pub workflows: Vec<String>,
    pub policies: Vec<String>,
    pub resources: Vec<String>,
    pub compatibility: BTreeMap<String, String>,
    pub license: String,
}

/// Kernel-facing projection of an imported ecosystem extension.
///
/// Vendor instructions, prompts, templates, and transport configuration are
/// intentionally absent.  The Kernel admits only an immutable identity,
/// requested capabilities, and an executor class.  A valid request still does
/// not constitute a grant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionCapabilityRequest {
    pub capability: String,
    pub access: String,
    pub scope: Vec<String>,
    pub reason: String,
    pub required: bool,
}

impl ExtensionCapabilityRequest {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        if !capability_name(&self.capability) {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "capability_requests.capability",
                message: "must be a namespaced capability identifier".into(),
            });
        }
        if !matches!(
            self.access.as_str(),
            "read" | "write" | "execute" | "connect" | "use"
        ) {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "capability_requests.access",
                message: "must be read, write, execute, connect, or use".into(),
            });
        }
        if self
            .scope
            .iter()
            .any(|item| item.is_empty() || item.contains('\0'))
        {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "capability_requests.scope",
                message: "must contain non-empty, NUL-free values".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionAdmissionRequest {
    pub schema_version: u32,
    pub extension_id: String,
    pub version: String,
    pub content_digest: String,
    pub source_format: String,
    pub compatibility_level: u8,
    pub kinds: Vec<String>,
    pub capability_requests: Vec<ExtensionCapabilityRequest>,
    pub executor: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionAdmissionDecision {
    pub admitted: bool,
    pub extension_id: String,
    pub normalized_digest: String,
    pub requested_capabilities: Vec<String>,
    pub granted_capabilities: Vec<String>,
    pub denied_capabilities: Vec<String>,
    pub approval_required: bool,
    pub executor_constraints: BTreeMap<String, String>,
    pub scope_constraints: BTreeMap<String, Vec<String>>,
    pub policy_version: String,
    pub decision_reason: String,
    pub receipt_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionAuthorizationRequest {
    pub admission: ExtensionAdmissionRequest,
    pub admission_receipt_id: String,
    pub approval_id: String,
    pub approved_capabilities: Vec<String>,
}

/// A single extension operation requesting an execution permit from Kernel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionExecutionRequest {
    pub schema_version: u32,
    pub operation_id: String,
    pub extension_id: String,
    pub content_digest: String,
    pub authorization_receipt_id: String,
    pub capability: String,
    pub operation: String,
    pub parameter_hash: String,
    pub executor: String,
    pub scope: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionExecutionPermit {
    pub allowed: bool,
    pub operation_id: String,
    pub actor: String,
    pub extension_id: String,
    pub content_digest: String,
    pub authorization_receipt_id: String,
    pub capability: String,
    pub operation: String,
    pub parameter_hash: String,
    pub executor: String,
    pub policy_version: String,
    pub decision_reason: String,
    pub issued_at_us: i64,
    pub permit_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionRevocationRequest {
    pub schema_version: u32,
    pub extension_id: String,
    pub content_digest: String,
    pub authorization_receipt_id: String,
    pub reason: String,
}

impl ContractValidation for ExtensionRevocationRequest {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("ExtensionRevocationRequest", self.schema_version)?;
        required(
            "ExtensionRevocationRequest",
            "extension_id",
            &self.extension_id,
        )?;
        required(
            "ExtensionRevocationRequest",
            "authorization_receipt_id",
            &self.authorization_receipt_id,
        )?;
        let valid_digest = self.content_digest.len() == 71
            && self.content_digest.starts_with("sha256:")
            && self.content_digest[7..]
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase());
        if !valid_digest {
            return Err(RuntimeContractError {
                contract: "ExtensionRevocationRequest",
                field: "content_digest",
                message: "must be a lowercase sha256 digest".into(),
            });
        }
        Ok(())
    }
}

impl ContractValidation for ExtensionExecutionRequest {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("ExtensionExecutionRequest", self.schema_version)?;
        required(
            "ExtensionExecutionRequest",
            "operation_id",
            &self.operation_id,
        )?;
        required(
            "ExtensionExecutionRequest",
            "extension_id",
            &self.extension_id,
        )?;
        required(
            "ExtensionExecutionRequest",
            "authorization_receipt_id",
            &self.authorization_receipt_id,
        )?;
        required("ExtensionExecutionRequest", "operation", &self.operation)?;
        if !capability_name(&self.capability) {
            return Err(RuntimeContractError {
                contract: "ExtensionExecutionRequest",
                field: "capability",
                message: "must be a namespaced capability identifier".into(),
            });
        }
        for (field, digest) in [
            ("content_digest", self.content_digest.as_str()),
            ("parameter_hash", self.parameter_hash.as_str()),
        ] {
            let valid = digest.len() == 71
                && digest.starts_with("sha256:")
                && digest[7..].chars().all(|character| {
                    character.is_ascii_hexdigit() && !character.is_ascii_uppercase()
                });
            if !valid {
                return Err(RuntimeContractError {
                    contract: "ExtensionExecutionRequest",
                    field,
                    message: "must be a lowercase sha256 digest".into(),
                });
            }
        }
        if self
            .scope
            .iter()
            .any(|item| item.is_empty() || item.contains('\0'))
        {
            return Err(RuntimeContractError {
                contract: "ExtensionExecutionRequest",
                field: "scope",
                message: "must contain non-empty, NUL-free values".into(),
            });
        }
        Ok(())
    }
}

impl ContractValidation for ExtensionAuthorizationRequest {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        self.admission.validate()?;
        required(
            "ExtensionAuthorizationRequest",
            "admission_receipt_id",
            &self.admission_receipt_id,
        )?;
        required(
            "ExtensionAuthorizationRequest",
            "approval_id",
            &self.approval_id,
        )?;
        let requested = self
            .admission
            .capability_requests
            .iter()
            .map(|item| item.capability.as_str())
            .collect::<BTreeSet<_>>();
        let approved = self
            .approved_capabilities
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if approved.len() != self.approved_capabilities.len() || !approved.is_subset(&requested) {
            return Err(RuntimeContractError {
                contract: "ExtensionAuthorizationRequest",
                field: "approved_capabilities",
                message: "must be a unique subset of requested capabilities".into(),
            });
        }
        Ok(())
    }
}

impl ContractValidation for ExtensionAdmissionRequest {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("ExtensionAdmissionRequest", self.schema_version)?;
        required(
            "ExtensionAdmissionRequest",
            "extension_id",
            &self.extension_id,
        )?;
        required("ExtensionAdmissionRequest", "version", &self.version)?;
        required(
            "ExtensionAdmissionRequest",
            "source_format",
            &self.source_format,
        )?;
        if self.compatibility_level > 5 {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "compatibility_level",
                message: "must be between C0 and C5".into(),
            });
        }
        let valid_digest = self.content_digest.len() == 71
            && self.content_digest.starts_with("sha256:")
            && self.content_digest[7..]
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase());
        if !valid_digest {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "content_digest",
                message: "must be a lowercase sha256 digest".into(),
            });
        }
        if self.metadata.get("authority").map(String::as_str) != Some("none") {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "metadata.authority",
                message: "must be none at admission".into(),
            });
        }
        if self
            .metadata
            .get("supply_chain_verified")
            .map(String::as_str)
            != Some("true")
        {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "metadata.supply_chain_verified",
                message: "must confirm local evidence verification".into(),
            });
        }
        for field in [
            "normalized_ir_digest",
            "sbom_digest",
            "provenance_digest",
            "signature_digest",
        ] {
            let value = self.metadata.get(field).map(String::as_str).unwrap_or("");
            let valid = value.len() == 71
                && value.starts_with("sha256:")
                && value[7..].chars().all(|character| {
                    character.is_ascii_hexdigit() && !character.is_ascii_uppercase()
                });
            if !valid {
                return Err(RuntimeContractError {
                    contract: "ExtensionAdmissionRequest",
                    field: "metadata.supply_chain_digest",
                    message: format!("{field} must be a lowercase sha256 digest"),
                });
            }
        }
        if !matches!(
            self.metadata.get("signature_status").map(String::as_str),
            Some("Unsigned" | "Verified")
        ) {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "metadata.signature_status",
                message: "must be Unsigned or cryptographically Verified".into(),
            });
        }
        let allowed_kinds = [
            "skill", "tool", "plugin", "provider", "agent", "template", "pack",
        ];
        if self.kinds.is_empty()
            || self
                .kinds
                .iter()
                .any(|kind| !allowed_kinds.contains(&kind.as_str()))
        {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "kinds",
                message: "must contain known extension kinds".into(),
            });
        }
        let mut unique = BTreeSet::new();
        if self.kinds.iter().any(|kind| !unique.insert(kind)) {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "kinds",
                message: "must not contain duplicates".into(),
            });
        }
        for request in &self.capability_requests {
            request.validate()?;
        }
        if self.kinds.iter().any(|kind| kind == "plugin") && self.executor.is_empty() {
            return Err(RuntimeContractError {
                contract: "ExtensionAdmissionRequest",
                field: "executor",
                message: "code-bearing plugins must select a governed executor class".into(),
            });
        }
        Ok(())
    }
}

impl ContractValidation for PackManifest {
    fn validate(&self) -> Result<(), RuntimeContractError> {
        schema("PackManifest", self.schema_version)?;
        required("PackManifest", "name", &self.name)?;
        required("PackManifest", "version", &self.version)?;
        required("PackManifest", "license", &self.license)?;
        for reference in self
            .models
            .iter()
            .chain(&self.providers)
            .chain(&self.workflows)
            .chain(&self.policies)
            .chain(&self.resources)
        {
            portable_reference("PackManifest", "references", reference)?;
        }
        Ok(())
    }
}

fn portable_reference(
    contract: &'static str,
    field: &'static str,
    value: &str,
) -> Result<(), RuntimeContractError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(RuntimeContractError {
            contract,
            field,
            message: "must be a portable relative path without parent traversal".into(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGraph {
    pub providers: BTreeMap<String, BTreeSet<String>>,
}

impl CapabilityGraph {
    pub fn advertise(&mut self, provider: impl Into<String>, capability: impl Into<String>) {
        self.providers
            .entry(capability.into())
            .or_default()
            .insert(provider.into());
    }

    pub fn providers_for(&self, capability: &str) -> Vec<&str> {
        self.providers
            .get(capability)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceGraph {
    pub capacity: BTreeMap<String, ResourceVector>,
}

impl ResourceGraph {
    pub fn feasible_nodes(&self, required: &ResourceVector) -> Vec<&str> {
        self.capacity
            .iter()
            .filter(|(_, capacity)| required.fits_within(capacity))
            .map(|(node, _)| node.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension_supply_chain_metadata(seed: char) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("authority".into(), "none".into()),
            ("supply_chain_verified".into(), "true".into()),
            (
                "normalized_ir_digest".into(),
                format!("sha256:{}", seed.to_string().repeat(64)),
            ),
            (
                "sbom_digest".into(),
                format!("sha256:{}", seed.to_string().repeat(64)),
            ),
            (
                "provenance_digest".into(),
                format!("sha256:{}", seed.to_string().repeat(64)),
            ),
            ("signature_status".into(), "Unsigned".into()),
            (
                "signature_digest".into(),
                format!("sha256:{}", seed.to_string().repeat(64)),
            ),
        ])
    }

    #[test]
    fn capability_requires_namespace() {
        let invalid = CapabilityContract {
            schema_version: 1,
            id: "chat".into(),
            version: "1.0.0".into(),
            ..Default::default()
        };
        assert_eq!(invalid.validate().unwrap_err().field, "id");
    }

    #[test]
    fn model_spec_rejects_unversioned_or_unknown_fields_through_serde() {
        let value = serde_json::json!({
            "schema_version": 1,
            "id": "linear-v1",
            "name": "Linear reference",
            "kind": "mathematical",
            "version": "1.0.0",
            "capabilities": ["mathematics.evaluate"],
            "backend": {"kind": "reference-math", "entrypoint": "", "runtime": "native"}
        });
        let model: ModelSpec = serde_json::from_value(value).unwrap();
        assert!(model.validate().is_ok());
        let unknown = serde_json::json!({"schema_version": 1, "unknown": true});
        assert!(serde_json::from_value::<ModelSpec>(unknown).is_err());
    }

    #[test]
    fn provider_manifest_accepts_references_but_not_secret_values() {
        let mut manifest = ProviderManifest {
            schema_version: 1,
            name: "remote".into(),
            version: "1.0.0".into(),
            backend: "openai-compatible".into(),
            credential_env: "DEEPSEEK_API_KEY".into(),
            capabilities: vec!["reasoning.generate".into()],
            ..Default::default()
        };
        assert!(manifest.validate().is_ok());
        manifest.credential_env = "secret-value".into();
        assert_eq!(manifest.validate().unwrap_err().field, "credential_env");
    }

    #[test]
    fn external_provider_requires_complete_lifecycle_and_portable_entrypoint() {
        let mut manifest = ProviderManifest {
            schema_version: 1,
            name: "echo".into(),
            version: "1.0.0".into(),
            backend: "external-process".into(),
            entrypoint: "src/provider.py".into(),
            lifecycle: vec![
                "probe",
                "metadata",
                "capabilities",
                "health",
                "execute",
                "cancel",
                "shutdown",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            ..Default::default()
        };
        assert!(manifest.validate().is_ok());
        manifest.entrypoint = "../provider.py".into();
        assert_eq!(manifest.validate().unwrap_err().field, "entrypoint");
    }

    #[test]
    fn pack_rejects_host_specific_references() {
        let pack = PackManifest {
            schema_version: 1,
            name: "sample".into(),
            version: "1.0.0".into(),
            license: "Apache-2.0".into(),
            models: vec!["../private/model.yaml".into()],
            ..Default::default()
        };
        assert_eq!(pack.validate().unwrap_err().field, "references");
    }

    #[test]
    fn extension_admission_is_a_strict_kernel_projection() {
        let request = ExtensionAdmissionRequest {
            schema_version: 1,
            extension_id: "skills/code-helper".into(),
            version: "1.2.3".into(),
            content_digest: format!("sha256:{}", "a".repeat(64)),
            source_format: "agent-skill".into(),
            compatibility_level: 2,
            kinds: vec!["skill".into()],
            capability_requests: vec![ExtensionCapabilityRequest {
                capability: "filesystem.read".into(),
                access: "read".into(),
                scope: vec!["references/guide.md".into()],
                reason: "read packaged reference".into(),
                required: true,
            }],
            metadata: extension_supply_chain_metadata('a'),
            ..Default::default()
        };
        assert!(request.validate().is_ok());

        let mut invalid = request.clone();
        invalid.content_digest = "sha256:secret".into();
        assert_eq!(invalid.validate().unwrap_err().field, "content_digest");

        let mut missing_evidence = request.clone();
        missing_evidence.metadata.remove("sbom_digest");
        assert_eq!(
            missing_evidence.validate().unwrap_err().field,
            "metadata.supply_chain_digest"
        );

        let mut invalid_signature = request.clone();
        invalid_signature
            .metadata
            .insert("signature_status".into(), "Invalid".into());
        assert_eq!(
            invalid_signature.validate().unwrap_err().field,
            "metadata.signature_status"
        );

        let unknown = serde_json::json!({
            "schema_version": 1,
            "extension_id": "skills/code-helper",
            "version": "1.2.3",
            "content_digest": format!("sha256:{}", "a".repeat(64)),
            "source_format": "agent-skill",
            "compatibility_level": 2,
            "kinds": ["skill"],
            "instructions": "must never cross the Kernel boundary"
        });
        assert!(serde_json::from_value::<ExtensionAdmissionRequest>(unknown).is_err());
    }

    #[test]
    fn code_bearing_extension_requires_governed_executor_class() {
        let request = ExtensionAdmissionRequest {
            schema_version: 1,
            extension_id: "plugins/example".into(),
            version: "1.0.0".into(),
            content_digest: format!("sha256:{}", "b".repeat(64)),
            source_format: "mcp-config".into(),
            compatibility_level: 2,
            kinds: vec!["plugin".into()],
            metadata: extension_supply_chain_metadata('b'),
            ..Default::default()
        };
        assert_eq!(request.validate().unwrap_err().field, "executor");
    }

    #[test]
    fn extension_authorization_cannot_expand_original_request() {
        let request = ExtensionAuthorizationRequest {
            admission: ExtensionAdmissionRequest {
                schema_version: 1,
                extension_id: "skills/example".into(),
                version: "1.0.0".into(),
                content_digest: format!("sha256:{}", "c".repeat(64)),
                source_format: "agent-skill".into(),
                compatibility_level: 2,
                kinds: vec!["skill".into()],
                capability_requests: vec![ExtensionCapabilityRequest {
                    capability: "filesystem.read".into(),
                    access: "read".into(),
                    required: true,
                    ..Default::default()
                }],
                metadata: extension_supply_chain_metadata('c'),
                ..Default::default()
            },
            admission_receipt_id: "admission-1".into(),
            approval_id: "approval-1".into(),
            approved_capabilities: vec!["filesystem.write".into()],
        };
        assert_eq!(
            request.validate().unwrap_err().field,
            "approved_capabilities"
        );
    }

    #[test]
    fn extension_execution_requires_receipt_and_canonical_hashes() {
        let request = ExtensionExecutionRequest {
            schema_version: 1,
            operation_id: "extension-operation-fixture".into(),
            extension_id: "plugins/example".into(),
            content_digest: format!("sha256:{}", "a".repeat(64)),
            authorization_receipt_id: "authorization-fixture".into(),
            capability: "process.execute".into(),
            operation: "example.echo".into(),
            parameter_hash: format!("sha256:{}", "b".repeat(64)),
            executor: "python".into(),
            scope: Vec::new(),
        };
        assert!(request.validate().is_ok());

        let mut invalid = request;
        invalid.parameter_hash = "not-a-digest".into();
        assert_eq!(invalid.validate().unwrap_err().field, "parameter_hash");
    }

    #[test]
    fn extension_revocation_is_bound_to_authorization_and_digest() {
        let request = ExtensionRevocationRequest {
            schema_version: 1,
            extension_id: "plugins/example".into(),
            content_digest: format!("sha256:{}", "d".repeat(64)),
            authorization_receipt_id: "authorization-fixture".into(),
            reason: "user requested".into(),
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn graphs_answer_discovery_and_hard_resource_queries() {
        let mut capabilities = CapabilityGraph::default();
        capabilities.advertise("edge-a", "vision.detect");
        capabilities.advertise("edge-b", "vision.detect");
        assert_eq!(
            capabilities.providers_for("vision.detect"),
            ["edge-a", "edge-b"]
        );

        let mut resources = ResourceGraph::default();
        resources.capacity.insert(
            "small".into(),
            ResourceVector {
                ram_bytes: 128,
                ..Default::default()
            },
        );
        resources.capacity.insert(
            "large".into(),
            ResourceVector {
                ram_bytes: 1024,
                ..Default::default()
            },
        );
        assert_eq!(
            resources.feasible_nodes(&ResourceVector {
                ram_bytes: 512,
                ..Default::default()
            }),
            ["large"]
        );
    }
}
