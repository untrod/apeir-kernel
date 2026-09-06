//! Engine - inference engine registration and lifecycle.

use crate::meta::{Condition, ObjectMeta};
use crate::resource::ResourceVector;
use crate::traits::KernelObject;
use crate::workload::{DeviceRequirements, PerformanceMetrics};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engine {
    pub meta: ObjectMeta,
    pub spec: EngineSpec,
    pub status: EngineStatus,
}

impl KernelObject for Engine {
    type Spec = EngineSpec;
    type Status = EngineStatus;
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
pub struct EngineSpec {
    pub engine_type: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub supported_architectures: Vec<String>,
    pub supported_quantizations: Vec<String>,
    pub supports_streaming: bool,
    pub supports_batching: bool,
    pub supports_pause: bool,
    pub supports_snapshot: bool,
    pub device_requirements: Option<DeviceRequirements>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineStatus {
    pub phase: EnginePhase,
    pub process_id: String,
    pub loaded_models: Vec<String>,
    pub allocated: ResourceVector,
    pub metrics: Option<PerformanceMetrics>,
    pub last_health_check: Option<chrono::DateTime<chrono::Utc>>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EnginePhase {
    #[default]
    Unknown = 0,
    Starting = 1,
    Ready = 2,
    LoadingModel = 3,
    Running = 4,
    Draining = 5,
    Stopping = 6,
    Stopped = 7,
    Unhealthy = 8,
}
