//! Stable Runtime API request types shared by clients, daemon, and providers.
//!
//! These transport contracts live below the runtime implementation so the CLI
//! and SDK never need to depend on kernel internals.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRuntimeClass {
    Remote,
    Local,
    Edge,
    Reference,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeliverySemantics {
    AtMostOnce,
    AtLeastOnce,
    Idempotent,
    Compensatable,
    ExternalCommit,
    Reconcilable,
    Unknown,
}

/// Provider-neutral model input carried inside `OperationRequest.input`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInvocationInput {
    pub schema_version: u32,
    #[serde(default = "default_model_capability")]
    pub capability: String,
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub response_format: Option<Value>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub temperature: Option<f64>,
}

impl ModelInvocationInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("model invocation schema_version must be 1".into());
        }
        if self.messages.is_empty() && self.input.is_null() {
            return Err("model invocation requires messages or input".into());
        }
        if self.max_output_tokens == Some(0) {
            return Err("max_output_tokens must be positive".into());
        }
        if self
            .temperature
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        {
            return Err("temperature must be between 0 and 2".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInvocationOutput {
    pub schema_version: u32,
    pub content: Value,
    #[serde(default)]
    pub usage: Value,
    #[serde(default)]
    pub finish_reason: String,
    #[serde(default)]
    pub metadata: Value,
}

fn default_model_capability() -> String {
    "chat".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticExecutionSnapshot {
    pub model_revision: String,
    pub provider_revision: String,
    pub prompt_revision: String,
    pub tool_revision: String,
    pub knowledge_revision: String,
    pub policy_revision: String,
    pub capability_revision: String,
    pub context_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRequest {
    pub operation_id: String,
    pub workload_id: String,
    pub step_id: String,
    pub backend: String,
    #[serde(default)]
    pub execution_domain: ProviderRuntimeClass,
    pub model: String,
    pub endpoint: String,
    pub credential_env: String,
    #[serde(default)]
    pub provider_entrypoint: String,
    pub input: String,
    pub delivery: DeliverySemantics,
    pub snapshot: SemanticExecutionSnapshot,
    pub timeout_ms: u64,
}

impl OperationRequest {
    pub fn input_digest(&self) -> String {
        hex_digest(self.input.as_bytes())
    }

    pub fn validate_sensitive_references(&self) -> Result<(), String> {
        validate_credential_reference(&self.credential_env)?;
        validate_endpoint_reference(&self.endpoint)?;
        validate_provider_entrypoint(&self.provider_entrypoint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProbeRequest {
    pub provider: String,
    pub backend: String,
    #[serde(default)]
    pub execution_domain: ProviderRuntimeClass,
    pub model: String,
    pub endpoint: String,
    pub credential_env: String,
    #[serde(default)]
    pub provider_entrypoint: String,
    pub timeout_ms: u64,
}

impl ProviderProbeRequest {
    pub fn validate_sensitive_references(&self) -> Result<(), String> {
        validate_credential_reference(&self.credential_env)?;
        validate_endpoint_reference(&self.endpoint)?;
        validate_provider_entrypoint(&self.provider_entrypoint)
    }
}

pub fn validate_credential_reference(reference: &str) -> Result<(), String> {
    if reference.is_empty() {
        return Ok(());
    }
    let mut characters = reference.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !valid_start
        || reference.len() > 128
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(
            "credential must be an environment-variable name, not a credential value".into(),
        );
    }
    Ok(())
}

pub fn validate_endpoint_reference(endpoint: &str) -> Result<(), String> {
    if endpoint.is_empty() {
        return Ok(());
    }
    if endpoint.contains('?') || endpoint.contains('#') {
        return Err("endpoint must not contain query parameters or fragments".into());
    }
    let authority = endpoint
        .split_once("://")
        .map(|(_, value)| value.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err("endpoint is invalid or contains user information".into());
    }
    Ok(())
}

pub fn validate_provider_entrypoint(entrypoint: &str) -> Result<(), String> {
    if entrypoint.is_empty() {
        return Ok(());
    }
    if Path::new(entrypoint)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("provider entrypoint must not traverse parent directories".into());
    }
    if entrypoint.contains('\0') {
        return Err("provider entrypoint contains a null byte".into());
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_values_are_rejected_at_the_public_runtime_boundary() {
        let request = ProviderProbeRequest {
            provider: "remote".into(),
            backend: "remote-api".into(),
            execution_domain: ProviderRuntimeClass::Remote,
            model: "model".into(),
            endpoint: "https://user:secret@example.invalid/v1".into(),
            credential_env: "sk-value".into(),
            provider_entrypoint: String::new(),
            timeout_ms: 1,
        };
        assert!(request.validate_sensitive_references().is_err());
    }

    #[test]
    fn model_invocation_contract_is_bounded_and_versioned() {
        let input = ModelInvocationInput {
            schema_version: 1,
            capability: "chat".into(),
            messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
            input: Value::Null,
            tools: Vec::new(),
            response_format: None,
            max_output_tokens: Some(64),
            temperature: Some(0.5),
        };
        assert!(input.validate().is_ok());
        let mut invalid = input;
        invalid.temperature = Some(3.0);
        assert!(invalid.validate().is_err());
    }
}
