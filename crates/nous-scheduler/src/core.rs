//! Cross-layer scheduler coordinator.
//!
//! Placement, program, workflow, and phase scheduling are execution scopes of
//! this core. Foundation policy sets replace independent Scheduler 1/2/3 modes.

use crate::placement::{ExpectedCompletionTime, NodeInfo, PlacementDecision, PlacementScheduler};
use crate::policy::{ContextualBanditPolicy, CriticalPathPolicy, WeightedSumPolicy};
use crate::program::{ProcessCandidate, ProgramScheduleDecision, ProgramScheduler};
use nous_types::error::{ErrorCode, NousError};
use nous_types::resource::ResourceVector;
use nous_types::workload::RoutingPreference;
use nous_types::workload::WorkloadSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerPolicySet {
    Deterministic,
    StateAware,
    Adaptive,
}

#[derive(Debug, Clone, Copy)]
pub struct SchedulerCoreConfig {
    pub policy_set: SchedulerPolicySet,
    pub learning_authorized: bool,
}

impl Default for SchedulerCoreConfig {
    fn default() -> Self {
        Self {
            policy_set: SchedulerPolicySet::Deterministic,
            learning_authorized: false,
        }
    }
}

pub struct SchedulerCore {
    config: SchedulerCoreConfig,
}

/// Stable explanation emitted by the single scheduler authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionTrace {
    pub policy: String,
    pub candidates: Vec<String>,
    pub rejections: Vec<CandidateRejection>,
    pub selected: Option<String>,
    pub selected_provider: Option<String>,
    pub selected_model: Option<String>,
    pub fallback_candidate: Option<String>,
    pub capability_match: String,
    pub reason: String,
    pub expected_completion_ms: Option<u64>,
    pub expected_completion: Option<ExpectedCompletionTime>,
    pub resource_snapshot: ResourceVector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRejection {
    pub candidate: String,
    pub reason: String,
}

impl SchedulerCore {
    pub fn new(config: SchedulerCoreConfig) -> Self {
        Self { config }
    }

    pub fn effective_policy_set(&self) -> SchedulerPolicySet {
        if self.config.policy_set == SchedulerPolicySet::Adaptive
            && !self.config.learning_authorized
        {
            SchedulerPolicySet::Deterministic
        } else {
            self.config.policy_set
        }
    }

    pub fn place(&self, workload: &WorkloadSpec, nodes: &[NodeInfo]) -> Option<PlacementDecision> {
        let scheduler = match self.effective_policy_set() {
            SchedulerPolicySet::Deterministic => {
                PlacementScheduler::new(Box::new(WeightedSumPolicy::default()))
            }
            SchedulerPolicySet::StateAware => PlacementScheduler::new(Box::new(CriticalPathPolicy)),
            SchedulerPolicySet::Adaptive => {
                PlacementScheduler::new(Box::new(ContextualBanditPolicy::default()))
            }
        };
        scheduler.place(workload, nodes)
    }

    /// Place a workload and retain a deterministic, serializable explanation.
    pub fn place_with_trace(
        &self,
        workload: &WorkloadSpec,
        nodes: &[NodeInfo],
    ) -> (Option<PlacementDecision>, DecisionTrace) {
        let decision = self.place(workload, nodes);
        let mut candidates = Vec::new();
        let mut rejections = Vec::new();
        for node in nodes {
            if !node.connection_state.schedulable() {
                rejections.push(CandidateRejection {
                    candidate: node.node_id.clone(),
                    reason: format!("node is {:?}", node.connection_state).to_lowercase(),
                });
                continue;
            }
            if workload.model_requirements.routing == RoutingPreference::LocalOnly && !node.is_local
            {
                rejections.push(CandidateRejection {
                    candidate: node.node_id.clone(),
                    reason: "privacy requires local execution".into(),
                });
                continue;
            }
            if node.devices.is_empty() {
                rejections.push(CandidateRejection {
                    candidate: node.node_id.clone(),
                    reason: "no device is available".into(),
                });
                continue;
            }
            if node.engines.is_empty() {
                rejections.push(CandidateRejection {
                    candidate: node.node_id.clone(),
                    reason: "no provider engine is available".into(),
                });
                continue;
            }
            if node.engines.iter().all(|engine| !engine.healthy) {
                rejections.push(CandidateRejection {
                    candidate: node.node_id.clone(),
                    reason: "all provider engines are unhealthy".into(),
                });
                continue;
            }
            if !workload
                .capability_requirements
                .required_capabilities
                .iter()
                .all(|required| {
                    node.engines.iter().any(|engine| {
                        engine.healthy && engine.capabilities.iter().any(|item| item == required)
                    })
                })
            {
                rejections.push(CandidateRejection {
                    candidate: node.node_id.clone(),
                    reason: "required provider capability is unavailable".into(),
                });
                continue;
            }
            if !workload
                .resource_requirements
                .minimum
                .fits_within(&node.allocatable)
            {
                rejections.push(CandidateRejection {
                    candidate: node.node_id.clone(),
                    reason: "minimum resources exceed allocatable resources".into(),
                });
                continue;
            }
            candidates.push(node.node_id.clone());
        }
        let selected = decision.as_ref().map(|item| item.node_id.clone());
        let selected_provider = decision.as_ref().map(|item| item.engine_id.clone());
        let selected_model = decision.as_ref().map(|item| item.model_revision.clone());
        let fallback_candidate = candidates
            .iter()
            .find(|candidate| Some(candidate.as_str()) != selected.as_deref())
            .cloned();
        let reason = decision
            .as_ref()
            .map(|item| item.reasoning.clone())
            .unwrap_or_else(|| "no feasible candidate passed hard constraints".into());
        let resource_snapshot = decision
            .as_ref()
            .and_then(|item| nodes.iter().find(|node| node.node_id == item.node_id))
            .map(|node| node.allocatable)
            .unwrap_or_default();
        let expected_completion_ms = decision.as_ref().map(|item| item.expected_completion_ms);
        let expected_completion = decision
            .as_ref()
            .map(|item| item.expected_completion.clone());
        let capability_match = if decision.is_some() {
            "all hard capability constraints satisfied"
        } else {
            "no candidate satisfied every hard capability constraint"
        }
        .into();
        let policy = format!("{:?}", self.effective_policy_set());
        (
            decision,
            DecisionTrace {
                policy,
                candidates,
                rejections,
                selected,
                selected_provider,
                selected_model,
                fallback_candidate,
                capability_match,
                reason,
                expected_completion_ms,
                expected_completion,
                resource_snapshot,
            },
        )
    }

    pub fn schedule_processes(
        &self,
        candidates: &[ProcessCandidate],
    ) -> Result<ProgramScheduleDecision, NousError> {
        let policy_name = match self.effective_policy_set() {
            SchedulerPolicySet::Deterministic => "FIFO",
            SchedulerPolicySet::StateAware | SchedulerPolicySet::Adaptive => "StateAware",
        };
        ProgramScheduler::with_all_baselines()
            .schedule_with(policy_name, candidates)
            .map_err(|error| {
                NousError::new(
                    ErrorCode::Internal,
                    format!("built-in scheduler policy '{policy_name}' is unavailable: {error}"),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_policy_falls_back_without_safety_authorization() {
        let core = SchedulerCore::new(SchedulerCoreConfig {
            policy_set: SchedulerPolicySet::Adaptive,
            learning_authorized: false,
        });
        assert_eq!(
            core.effective_policy_set(),
            SchedulerPolicySet::Deterministic
        );
    }

    #[test]
    fn authorized_adaptive_policy_is_visible() {
        let core = SchedulerCore::new(SchedulerCoreConfig {
            policy_set: SchedulerPolicySet::Adaptive,
            learning_authorized: true,
        });
        assert_eq!(core.effective_policy_set(), SchedulerPolicySet::Adaptive);
    }

    #[test]
    fn decision_trace_records_hard_rejection() {
        let core = SchedulerCore::new(SchedulerCoreConfig::default());
        let workload = WorkloadSpec::default();
        let node = NodeInfo {
            node_id: "node-1".into(),
            hostname: "localhost".into(),
            arch: "arm64".into(),
            os: "windows".into(),
            capacity: ResourceVector::default(),
            allocatable: ResourceVector::default(),
            devices: vec![],
            engines: vec![],
            network_latency_us: 0,
            is_local: true,
            connection_state: crate::NodeConnectionState::Connected,
        };
        let (decision, trace) = core.place_with_trace(&workload, &[node]);
        assert!(decision.is_none());
        assert_eq!(trace.rejections.len(), 1);
        assert_eq!(trace.rejections[0].reason, "no device is available");
    }

    #[test]
    fn offline_node_is_removed_before_soft_ranking() {
        let core = SchedulerCore::new(SchedulerCoreConfig::default());
        let workload = WorkloadSpec::default();
        let node = NodeInfo {
            node_id: "edge-offline".into(),
            hostname: "edge.local".into(),
            arch: "arm64".into(),
            os: "linux".into(),
            capacity: ResourceVector::default(),
            allocatable: ResourceVector::default(),
            devices: vec![],
            engines: vec![],
            network_latency_us: 0,
            is_local: true,
            connection_state: crate::NodeConnectionState::Offline,
        };
        let (decision, trace) = core.place_with_trace(&workload, &[node]);
        assert!(decision.is_none());
        assert_eq!(trace.rejections[0].reason, "node is offline");
    }
}
