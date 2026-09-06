//! Model - ModelPackage and related types.

use crate::meta::{Condition, ObjectMeta};
use crate::resource::ResourceVector;
use crate::traits::KernelObject;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPackage {
    pub meta: ObjectMeta,
    pub spec: ModelPackageSpec,
    pub status: ModelPackageStatus,
}

impl KernelObject for ModelPackage {
    type Spec = ModelPackageSpec;
    type Status = ModelPackageStatus;
    fn meta(&self) -> &ObjectMeta {
        &self.meta
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.meta
    }
    fn spec(&self) -> &Self::Spec {
        &self.spec
    }
    fn status(&self) -> &Self::Status {
        &self.status
    }
    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelPackageSpec {
    pub family: String,
    pub architecture: String,
    pub total_parameters: u64,
    pub activated_parameters: u64,
    pub layer_count: u32,
    pub hidden_size: u32,
    pub attention_type: String,
    pub attention_heads: u32,
    pub kv_heads: u32,
    pub head_dimension: u32,
    pub expert_count: u32,
    pub active_experts: u32,
    pub shared_experts: u32,
    pub context_length: u64,
    pub rope_parameters: String,
    pub tokenizer: String,
    pub modalities: Vec<String>,
    pub vision_encoder: String,
    pub audio_encoder: String,
    pub weight_format: String,
    pub weight_precision: String,
    pub activation_precision: String,
    pub quantization: String,
    pub tensor_shapes: Vec<String>,
    pub kv_bytes_per_token: u64,
    pub supported_engines: Vec<String>,
    pub supported_devices: Vec<String>,
    pub required_operators: Vec<String>,
    pub parallelism_support: Vec<String>,
    pub memory_requirements: ResourceVector,
    pub license: String,
    pub source: String,
    pub sha256: Vec<u8>,
    pub signature: Vec<u8>,
    pub sbom: String,
    pub security_status: String,
    pub benchmark_profiles: Vec<BenchmarkProfile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelPackageStatus {
    pub phase: ModelLifecyclePhase,
    pub loaded_on_engines: Vec<String>,
    pub loaded_on_devices: Vec<String>,
    pub validated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ModelLifecyclePhase {
    #[default]
    Unknown = 0,
    Importing = 1,
    Imported = 2,
    Validating = 3,
    Validated = 4,
    Loading = 5,
    Loaded = 6,
    Active = 7,
    Unloading = 8,
    Unloaded = 9,
    Removing = 10,
    Removed = 11,
    Failed = 12,
    Quarantined = 13,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchmarkProfile {
    pub benchmark_name: String,
    pub hardware_config: String,
    pub engine: String,
    pub tokens_per_second: f64,
    pub ttft_ms: u64,
    pub tpot_ms: u64,
    pub memory_bytes: u64,
    pub quality_score: f64,
    pub run_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoadOptions {
    pub quantization_override: Option<String>,
    pub max_context_override: Option<u64>,
    pub kv_cache_bytes_override: Option<u64>,
    pub force_reload: bool,
}
