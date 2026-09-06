pub use nous_types::ProviderRuntimeClass;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilitySupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ModelPrivacy {
    LocalOnly,
    ProviderManaged,
    PrivateEndpoint,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    Chat,
    Reasoning,
    Tools,
    StructuredOutput,
    Vision,
    Embedding,
    Streaming,
    LocalExecution,
    RemoteExecution,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCapabilityManifest {
    pub schema_version: u32,
    pub provider: String,
    pub backend: String,
    pub model: String,
    pub runtime_class: ProviderRuntimeClass,
    pub chat: CapabilitySupport,
    pub streaming: CapabilitySupport,
    pub tools: CapabilitySupport,
    pub structured_output: CapabilitySupport,
    pub vision: CapabilitySupport,
    pub embedding: CapabilitySupport,
    pub reasoning: CapabilitySupport,
    pub local_execution: CapabilitySupport,
    pub remote_execution: CapabilitySupport,
    pub context_length: Option<u64>,
    pub privacy_local: bool,
    pub privacy: ModelPrivacy,
    pub estimated_latency_ms: Option<u64>,
    pub estimated_cost_microcents: Option<u64>,
    pub probed_at_us: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub manifest_id: String,
    pub revision: String,
    pub capabilities: ModelCapabilityManifest,
    pub labels: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProvider {
    pub provider_id: String,
    pub backend: String,
    pub runtime_class: ProviderRuntimeClass,
    pub endpoint: String,
    pub credential_env: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInstance {
    pub instance_id: String,
    pub manifest_id: String,
    pub provider_id: String,
    pub model: String,
    pub runtime_class: ProviderRuntimeClass,
    pub healthy: bool,
    pub loaded_at_us: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProbeReport {
    pub reachable: bool,
    pub latency_ms: u64,
    pub available_models: Vec<String>,
    pub capabilities: ModelCapabilityManifest,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCompatibilityRequirements {
    pub chat: bool,
    pub streaming: bool,
    pub tools: bool,
    pub structured_output: bool,
    pub vision: bool,
    pub embedding: bool,
    pub reasoning: bool,
    pub min_context_length: u64,
    pub local_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ModelCompatibility {
    Compatible,
    Rebindable,
    Degraded,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    pub classification: ModelCompatibility,
    pub reasons: Vec<String>,
}

pub fn assess_compatibility(
    requirements: &ModelCompatibilityRequirements,
    manifest: &ModelCapabilityManifest,
) -> CompatibilityReport {
    let mut incompatible = Vec::new();
    let mut unknown = Vec::new();
    let checks = [
        (requirements.chat, manifest.chat, "chat"),
        (requirements.streaming, manifest.streaming, "streaming"),
        (requirements.tools, manifest.tools, "tools"),
        (
            requirements.structured_output,
            manifest.structured_output,
            "structured_output",
        ),
        (requirements.vision, manifest.vision, "vision"),
        (requirements.embedding, manifest.embedding, "embedding"),
        (requirements.reasoning, manifest.reasoning, "reasoning"),
    ];
    for (required, support, name) in checks {
        if !required {
            continue;
        }
        match support {
            CapabilitySupport::Supported => {}
            CapabilitySupport::Unsupported => {
                incompatible.push(format!("missing {name} capability"))
            }
            CapabilitySupport::Unknown => unknown.push(format!("{name} capability is unverified")),
        }
    }
    if requirements.local_only
        && (!manifest.privacy_local || manifest.local_execution != CapabilitySupport::Supported)
    {
        incompatible.push("privacy policy requires local execution".into());
    }
    match manifest.context_length {
        Some(length) if length < requirements.min_context_length => incompatible.push(format!(
            "context length {length} is below required {}",
            requirements.min_context_length
        )),
        None if requirements.min_context_length > 0 => {
            unknown.push("context length is unverified".into())
        }
        _ => {}
    }

    let classification = if !incompatible.is_empty() {
        ModelCompatibility::Incompatible
    } else if !unknown.is_empty() {
        ModelCompatibility::Degraded
    } else {
        ModelCompatibility::Compatible
    };
    incompatible.extend(unknown);
    CompatibilityReport {
        classification,
        reasons: incompatible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ModelCapabilityManifest {
        ModelCapabilityManifest {
            schema_version: 1,
            provider: "local".into(),
            backend: "ollama".into(),
            model: "model-a".into(),
            runtime_class: ProviderRuntimeClass::Local,
            chat: CapabilitySupport::Supported,
            streaming: CapabilitySupport::Supported,
            tools: CapabilitySupport::Unsupported,
            structured_output: CapabilitySupport::Unknown,
            vision: CapabilitySupport::Unsupported,
            embedding: CapabilitySupport::Unsupported,
            reasoning: CapabilitySupport::Unknown,
            local_execution: CapabilitySupport::Supported,
            remote_execution: CapabilitySupport::Unsupported,
            context_length: Some(32_768),
            privacy_local: true,
            privacy: ModelPrivacy::LocalOnly,
            estimated_latency_ms: Some(10),
            estimated_cost_microcents: Some(0),
            probed_at_us: 0,
        }
    }

    #[test]
    fn hard_capability_mismatch_is_incompatible() {
        let report = assess_compatibility(
            &ModelCompatibilityRequirements {
                tools: true,
                ..Default::default()
            },
            &manifest(),
        );
        assert_eq!(report.classification, ModelCompatibility::Incompatible);
    }

    #[test]
    fn unknown_capability_is_explicitly_degraded() {
        let report = assess_compatibility(
            &ModelCompatibilityRequirements {
                structured_output: true,
                ..Default::default()
            },
            &manifest(),
        );
        assert_eq!(report.classification, ModelCompatibility::Degraded);
    }

    #[test]
    fn local_manifest_satisfies_local_privacy() {
        let report = assess_compatibility(
            &ModelCompatibilityRequirements {
                chat: true,
                local_only: true,
                min_context_length: 8_192,
                ..Default::default()
            },
            &manifest(),
        );
        assert_eq!(report.classification, ModelCompatibility::Compatible);
    }
}
