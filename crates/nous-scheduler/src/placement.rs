//! Global Placement Scheduler.
//!
//! Decides: Node, Device, Engine, Model Revision, Parallelism strategy,
//! Resource Lease placement, and Data/KV cache location.
//!
//! This is the top-level scheduler that decides WHERE a workload runs.

use crate::policy::{SchedulerCandidate, SchedulerPolicy};
use crate::NodeConnectionState;
use nous_types::resource::ResourceVector;
use nous_types::workload::{RoutingPreference, WorkloadSpec};
use serde::{Deserialize, Serialize};

/// Information about a candidate node for placement.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub node_id: String,
    pub hostname: String,
    pub arch: String,
    pub os: String,
    pub capacity: ResourceVector,
    pub allocatable: ResourceVector,
    pub devices: Vec<DevicePlacementInfo>,
    pub engines: Vec<EnginePlacementInfo>,
    pub network_latency_us: u64,
    pub is_local: bool,
    pub connection_state: NodeConnectionState,
}

/// Device information for placement decisions.
#[derive(Debug, Clone)]
pub struct DevicePlacementInfo {
    pub device_id: String,
    pub device_type: String,
    pub vendor: String,
    pub total_vram: u64,
    pub free_vram: u64,
    pub utilization: f64,
    pub temperature_celsius: f64,
    pub has_kv_cache_for_model: bool,
}

/// Engine information for placement decisions.
#[derive(Debug, Clone)]
pub struct EnginePlacementInfo {
    pub engine_id: String,
    pub engine_type: String,
    pub loaded_models: Vec<String>,
    pub active_requests: u32,
    pub max_batch_size: u32,
    pub supports_streaming: bool,
    pub capabilities: Vec<String>,
    pub healthy: bool,
    pub queue_depth: u32,
    pub model_load_ms: u64,
    pub state_transfer_ms: u64,
    pub estimated_execution_ms: u64,
    pub estimated_tps: f64,
    pub kv_cache_hit_rate: f64,
}

/// The result of global placement.
#[derive(Debug, Clone)]
pub struct PlacementDecision {
    pub node_id: String,
    pub device_id: String,
    pub engine_id: String,
    pub model_revision: String,
    /// Whether to use tensor parallelism.
    pub tensor_parallelism: u32,
    /// Whether to use pipeline parallelism.
    pub pipeline_parallelism: u32,
    /// KV cache placement: local, remote, or shared.
    pub kv_cache_placement: KVCachePlacement,
    /// Resource lease details.
    pub resources_reserved: ResourceVector,
    /// Queue + model load + state transfer + execution estimate.
    pub expected_completion_ms: u64,
    pub expected_completion: ExpectedCompletionTime,
    /// Reasoning for the placement.
    pub reasoning: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedCompletionTime {
    pub queue_delay_ms: u64,
    pub model_load_ms: u64,
    pub state_transfer_ms: u64,
    pub execution_estimate_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KVCachePlacement {
    /// KV cache on the same device as the model.
    Local,
    /// KV cache on a separate device (PD disaggregation).
    Remote,
    /// KV cache shared across workloads (prefix cache).
    Shared,
    /// No KV cache (stateless inference).
    None,
}

/// The Global Placement Scheduler decides where workloads execute.
pub struct PlacementScheduler {
    policy: Box<dyn SchedulerPolicy>,
}

impl PlacementScheduler {
    pub fn new(policy: Box<dyn SchedulerPolicy>) -> Self {
        Self { policy }
    }

    /// Place a workload onto the best available node/device/engine.
    pub fn place(&self, workload: &WorkloadSpec, nodes: &[NodeInfo]) -> Option<PlacementDecision> {
        // Step 1: Build candidate set
        let mut candidates = Vec::new();

        for node in nodes {
            if !node.connection_state.schedulable() {
                continue;
            }
            if workload.model_requirements.routing == RoutingPreference::LocalOnly && !node.is_local
            {
                continue;
            }
            if !workload
                .resource_requirements
                .minimum
                .fits_within(&node.allocatable)
            {
                continue;
            }
            for device in &node.devices {
                for engine in &node.engines {
                    if !engine.healthy {
                        continue;
                    }
                    if !workload
                        .capability_requirements
                        .required_capabilities
                        .iter()
                        .all(|required| engine.capabilities.iter().any(|item| item == required))
                    {
                        continue;
                    }
                    // Check if any loaded model matches the workload requirements
                    let matching_model = engine.loaded_models.iter().find(|m| {
                        workload
                            .model_requirements
                            .allowed_model_families
                            .iter()
                            .any(|f| m.contains(f.as_str()))
                    });

                    // Check device compatibility
                    let device_ok = workload.device_requirements.allowed_device_types.is_empty()
                        || workload
                            .device_requirements
                            .allowed_device_types
                            .iter()
                            .any(|dt| {
                                format!("{:?}", dt).to_lowercase()
                                    == device.device_type.to_lowercase()
                            });

                    if !device_ok {
                        continue;
                    }

                    // Check VRAM
                    let required_vram = workload.resource_requirements.minimum.device_memory_bytes;
                    if required_vram > device.free_vram {
                        continue;
                    }

                    let candidate = SchedulerCandidate {
                        node_id: node.node_id.clone(),
                        device_id: device.device_id.clone(),
                        engine_id: engine.engine_id.clone(),
                        model_revision: matching_model.cloned().unwrap_or_default(),
                        resources_available: ResourceVector {
                            device_memory_bytes: device.free_vram,
                            ram_bytes: node.allocatable.ram_bytes,
                            kv_cache_bytes: node.allocatable.kv_cache_bytes,
                            cpu_cores_millis: node.allocatable.cpu_cores_millis,
                            ..ResourceVector::default()
                        },
                        resources_required: workload.resource_requirements.minimum,
                        estimated_ttft_ms: if device.device_type == "cpu" {
                            2000
                        } else {
                            200
                        },
                        estimated_tps: engine.estimated_tps,
                        estimated_cost_microcents: if node.is_local { 0 } else { 1000 },
                        estimated_quality: if matching_model.is_some() { 0.9 } else { 0.7 },
                        kv_cache_hit_rate: engine.kv_cache_hit_rate,
                        engine_load: engine.active_requests as f64
                            / engine.max_batch_size.max(1) as f64,
                        device_utilization: device.utilization,
                    };

                    candidates.push(candidate);
                }
            }
        }

        // Step 2: Run policy
        let decision = self.policy.select(workload, candidates);

        // Step 3: Convert to placement decision
        decision.selected.map(|c| {
            let engine = nodes
                .iter()
                .find(|node| node.node_id == c.node_id)
                .and_then(|node| {
                    node.engines
                        .iter()
                        .find(|engine| engine.engine_id == c.engine_id)
                });
            let expected_completion = engine
                .map(|engine| {
                    let queue_delay_ms =
                        u64::from(engine.queue_depth).saturating_mul(engine.estimated_execution_ms);
                    let model_load_ms = if engine
                        .loaded_models
                        .iter()
                        .any(|model| model == &c.model_revision)
                    {
                        0
                    } else {
                        engine.model_load_ms
                    };
                    let total_ms = queue_delay_ms
                        .saturating_add(model_load_ms)
                        .saturating_add(engine.state_transfer_ms)
                        .saturating_add(engine.estimated_execution_ms);
                    ExpectedCompletionTime {
                        queue_delay_ms,
                        model_load_ms,
                        state_transfer_ms: engine.state_transfer_ms,
                        execution_estimate_ms: engine.estimated_execution_ms,
                        total_ms,
                    }
                })
                .unwrap_or(ExpectedCompletionTime {
                    total_ms: u64::MAX,
                    ..Default::default()
                });
            PlacementDecision {
                node_id: c.node_id.clone(),
                device_id: c.device_id.clone(),
                engine_id: c.engine_id.clone(),
                model_revision: c.model_revision.clone(),
                tensor_parallelism: 1,
                pipeline_parallelism: 1,
                kv_cache_placement: if c.kv_cache_hit_rate > 0.5 {
                    KVCachePlacement::Shared
                } else {
                    KVCachePlacement::Local
                },
                resources_reserved: c.resources_required,
                expected_completion_ms: expected_completion.total_ms,
                expected_completion,
                reasoning: format!(
                    "Selected {} on {} (score: {:?}, kv_hit: {:.2}, load: {:.2})",
                    c.engine_id,
                    c.device_id,
                    decision.score_breakdown.get(&c.node_id),
                    c.kv_cache_hit_rate,
                    c.engine_load,
                ),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_node(id: &str, free_vram: u64, engine_tps: f64, kv_hit: f64) -> NodeInfo {
        NodeInfo {
            node_id: id.to_string(),
            hostname: format!("{}.local", id),
            arch: "x86_64".into(),
            os: "linux".into(),
            capacity: ResourceVector {
                device_memory_bytes: 16 * 1024 * 1024 * 1024,
                ram_bytes: 32 * 1024 * 1024 * 1024,
                ..Default::default()
            },
            allocatable: ResourceVector {
                device_memory_bytes: free_vram,
                ram_bytes: 16 * 1024 * 1024 * 1024,
                ..Default::default()
            },
            devices: vec![DevicePlacementInfo {
                device_id: format!("{}-gpu0", id),
                device_type: "cuda".into(),
                vendor: "NVIDIA".into(),
                total_vram: 16 * 1024 * 1024 * 1024,
                free_vram,
                utilization: 0.3,
                temperature_celsius: 65.0,
                has_kv_cache_for_model: kv_hit > 0.5,
            }],
            engines: vec![EnginePlacementInfo {
                engine_id: format!("{}-vllm", id),
                engine_type: "vLLM".into(),
                loaded_models: vec!["test-model".into()],
                active_requests: 2,
                max_batch_size: 32,
                supports_streaming: true,
                capabilities: vec!["chat".into(), "tools".into()],
                healthy: true,
                queue_depth: 2,
                model_load_ms: 500,
                state_transfer_ms: 20,
                estimated_execution_ms: 100,
                estimated_tps: engine_tps,
                kv_cache_hit_rate: kv_hit,
            }],
            network_latency_us: 100,
            is_local: true,
            connection_state: NodeConnectionState::Connected,
        }
    }

    #[test]
    fn test_placement_prefers_local_node() {
        use crate::policy::WeightedSumPolicy;
        let scheduler = PlacementScheduler::new(Box::new(WeightedSumPolicy::default()));
        let nodes = vec![make_test_node("local", 12 * 1024 * 1024 * 1024, 60.0, 0.8)];
        let workload = WorkloadSpec::default();
        let decision = scheduler.place(&workload, &nodes);
        assert!(decision.is_some());
        assert_eq!(decision.unwrap().node_id, "local");
    }

    #[test]
    fn test_placement_rejects_insufficient_vram() {
        use crate::policy::WeightedSumPolicy;
        let scheduler = PlacementScheduler::new(Box::new(WeightedSumPolicy::default()));
        let nodes = vec![
            make_test_node("small", 512 * 1024 * 1024, 60.0, 0.8), // Only 512MB free
        ];
        let workload = WorkloadSpec {
            resource_requirements: nous_types::workload::ResourceRequirements {
                minimum: ResourceVector {
                    device_memory_bytes: 8 * 1024 * 1024 * 1024, // Needs 8GB
                    ..ResourceVector::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let decision = scheduler.place(&workload, &nodes);
        assert!(decision.is_none());
    }
}
