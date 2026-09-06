use crate::{
    CompatibilityReport, ModelCapabilityManifest, ModelCompatibilityRequirements, OperationRequest,
    RuntimeExecution,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverCandidate {
    pub request: OperationRequest,
    pub capabilities: ModelCapabilityManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuityStep {
    pub step_id: String,
    pub requirements: ModelCompatibilityRequirements,
    pub allow_degraded: bool,
    pub candidates: Vec<FailoverCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuityPlan {
    pub workload_id: String,
    pub process_identity: String,
    pub steps: Vec<ContinuityStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverRecord {
    pub step_id: String,
    pub from_provider: String,
    pub to_provider: String,
    pub failure_code: String,
    pub compatibility: CompatibilityReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuityExecution {
    pub workload_id: String,
    pub process_identity: String,
    pub completed_steps: Vec<RuntimeExecution>,
    pub failovers: Vec<FailoverRecord>,
}
