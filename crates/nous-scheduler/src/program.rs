//! Program scheduler for durable processes and critical paths.
//!
//! ```text
//! Level 1: Program Scheduler
//!   Which AgentProcess gets to advance next?
//!
//! Level 2: Placement Scheduler <- EXISTING (placement.rs)
//!   Which Node/Device/Engine/Model for this Workload?
//!
//! Level 3: Workflow Scheduler  <- EXISTING (workflow.rs)
//!   Which phase in the ExecutionGraph is most urgent?
//!
//! Level 4: Phase Scheduler     <- EXISTING (phase.rs)
//!   What priority/preemption hint to send to the engine?
//! ```
//!
//! # Scheduling Objects
//!
//! The Program Scheduler operates on:
//! - **AgentProcess**: Full process state including status, budgets, children
//! - **Continuation**: Where the process will resume
//! - **Context Residency**: Which pages are in fast tiers (warm state)
//! - **Downstream Unlock**: How many other processes become runnable
//!
//! # Experimental Priority Formula (P_i)
//!
//! ```text
//! P_i = (C_i * U_i * S_i * W_i) / (T_i * R_i * D_i)
//!
//! Where:
//!   C_i = critical path importance
//!   U_i = unlocked work ratio
//!   S_i = SLO breach risk
//!   W_i = warm state benefit
//!   T_i = estimated remaining time
//!   R_i = resource cost
//!   D_i = prediction uncertainty
//! ```
//!
//! This is a RESEARCH hypothesis - it must be evaluated against all
//! existing baseline policies before being used in production.

use std::collections::HashMap;

/// A process-level scheduling candidate.
///
/// Unlike SchedulerCandidate (which is about WHERE to run a workload),
/// ProcessCandidate is about WHICH process to advance next.
#[derive(Debug, Clone)]
pub struct ProcessCandidate {
    /// The process ID.
    pub process_id: String,

    /// The process status (affects scheduling priority).
    pub status: ProcessSchedulingStatus,

    /// Critical path importance C_i (0.0 - 1.0).
    /// Higher = on critical path, completing this unlocks more work.
    pub critical_path_importance: f64,

    /// Unlocked work ratio U_i (0.0 - 1.0).
    /// How many other processes become runnable after this one advances.
    pub unlocked_work_ratio: f64,

    /// SLO breach risk S_i (>= 1.0).
    /// 1.0 = on track, 2.0 = deadline likely missed, 5.0 = already breached.
    pub slo_breach_risk: f64,

    /// Warm state benefit W_i (0.0 - 1.0).
    /// Higher = more context pages are resident in fast tiers.
    pub warm_state_benefit: f64,

    /// Estimated remaining time T_i (milliseconds).
    pub estimated_remaining_time_ms: u64,

    /// Resource cost R_i (normalized, 0.0 - 1.0).
    /// Higher = more expensive to run.
    pub resource_cost: f64,

    /// Prediction uncertainty D_i (>= 1.0).
    /// 1.0 = very certain, > 2.0 = highly uncertain.
    pub prediction_uncertainty: f64,

    /// Computed priority score (higher = run first).
    pub priority_score: f64,

    /// Whether this process is currently runnable.
    pub is_runnable: bool,

    /// Why the process is not runnable (if applicable).
    pub blocked_reason: Option<String>,

    /// The number of child processes waiting on this process.
    pub waiting_children: u32,

    /// Budget remaining ratio (1.0 = full budget, 0.0 = exhausted).
    pub budget_remaining_ratio: f64,
}

/// Simplified process status for scheduling decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSchedulingStatus {
    /// Process is ready and waiting for the scheduler.
    Runnable,
    /// Process is blocked waiting for a resource.
    Blocked,
    /// Process is suspended (paused or checkpointed).
    Suspended,
    /// Process is actively running (already has resources).
    Running,
    /// Process is in a terminal or transient state.
    Other,
}

/// The result of a program scheduling decision.
#[derive(Debug, Clone)]
pub struct ProgramScheduleDecision {
    /// The selected process to advance (if any).
    pub selected: Option<ProcessCandidate>,

    /// All candidates considered.
    pub candidates: Vec<ProcessCandidate>,

    /// How many processes are currently runnable.
    pub runnable_count: usize,

    /// How many processes are blocked and on what.
    pub blocked_by_resource: HashMap<String, usize>,

    /// Score breakdown for the selected candidate.
    pub score_breakdown: HashMap<String, f64>,

    /// The policy used for this decision.
    pub policy_name: String,

    /// When the decision was made.
    pub decided_at: chrono::DateTime<chrono::Utc>,
}

// -- Program Scheduler --

/// The Program Scheduler - Level 1 of the scheduling hierarchy.
///
/// It decides which AgentProcess to advance next, given:
/// - Process state (status, budgets, children)
/// - Critical path information from the hypergraph
/// - Context residency from the context VM
/// - Historical execution profiles from the system twin
pub struct ProgramScheduler {
    /// The scheduling policy to use.
    policy: Box<dyn ProgramSchedulingPolicy>,

    /// Historical scheduling decisions (for learning policies).
    history: Vec<ProgramScheduleDecision>,

    /// Statistics.
    decisions_made: u64,
}

/// The trait for program-level scheduling policies.
pub trait ProgramSchedulingPolicy: Send + Sync {
    /// Name of this policy.
    fn name(&self) -> &str;

    /// Score a process candidate (higher = more urgent).
    fn score(&self, candidate: &ProcessCandidate) -> f64;

    /// Select the best candidate from the runnable set.
    fn select(&self, candidates: &[ProcessCandidate]) -> ProgramScheduleDecision;
}

// -- Baseline Policies --

/// 1. FIFO - First created, first served.
pub struct ProgramFifoPolicy;

impl ProgramSchedulingPolicy for ProgramFifoPolicy {
    fn name(&self) -> &str {
        "ProgramFIFO"
    }

    fn score(&self, _: &ProcessCandidate) -> f64 {
        0.0
    }

    fn select(&self, candidates: &[ProcessCandidate]) -> ProgramScheduleDecision {
        let runnable: Vec<_> = candidates.iter().filter(|c| c.is_runnable).collect();
        let selected = runnable.first().map(|c| (*c).clone());

        let mut blocked_by_resource = HashMap::new();
        for c in candidates.iter().filter(|c| !c.is_runnable) {
            if let Some(ref reason) = c.blocked_reason {
                *blocked_by_resource.entry(reason.clone()).or_insert(0) += 1;
            }
        }

        ProgramScheduleDecision {
            selected,
            candidates: candidates.to_vec(),
            runnable_count: runnable.len(),
            blocked_by_resource,
            score_breakdown: HashMap::new(),
            policy_name: self.name().to_string(),
            decided_at: chrono::Utc::now(),
        }
    }
}

/// 2. Priority - Highest priority class first (inherits from Workload priority).
pub struct ProgramPriorityPolicy;

impl ProgramSchedulingPolicy for ProgramPriorityPolicy {
    fn name(&self) -> &str {
        "ProgramPriority"
    }

    fn score(&self, candidate: &ProcessCandidate) -> f64 {
        // Priority encoded in budget_remaining_ratio and slo_breach_risk
        candidate.slo_breach_risk * 0.7 + (1.0 - candidate.budget_remaining_ratio) * 0.3
    }

    fn select(&self, candidates: &[ProcessCandidate]) -> ProgramScheduleDecision {
        let mut runnable: Vec<_> = candidates
            .iter()
            .filter(|c| c.is_runnable)
            .cloned()
            .collect();
        runnable.sort_by(|a, b| {
            self.score(b)
                .partial_cmp(&self.score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut blocked_by_resource = HashMap::new();
        for c in candidates.iter().filter(|c| !c.is_runnable) {
            if let Some(ref reason) = c.blocked_reason {
                *blocked_by_resource.entry(reason.clone()).or_insert(0) += 1;
            }
        }

        ProgramScheduleDecision {
            selected: runnable.first().cloned(),
            candidates: candidates.to_vec(),
            runnable_count: runnable.len(),
            blocked_by_resource,
            score_breakdown: HashMap::new(),
            policy_name: self.name().to_string(),
            decided_at: chrono::Utc::now(),
        }
    }
}

/// 3. Shortest Remaining Time - Prefer processes that will finish soonest.
pub struct ProgramSRTFPolicy;

impl ProgramSchedulingPolicy for ProgramSRTFPolicy {
    fn name(&self) -> &str {
        "ProgramSRTF"
    }

    fn score(&self, candidate: &ProcessCandidate) -> f64 {
        if candidate.estimated_remaining_time_ms == 0 {
            return f64::MAX; // Almost done, run immediately
        }
        1.0 / (candidate.estimated_remaining_time_ms as f64).max(1.0)
    }

    fn select(&self, candidates: &[ProcessCandidate]) -> ProgramScheduleDecision {
        let mut runnable: Vec<_> = candidates
            .iter()
            .filter(|c| c.is_runnable)
            .cloned()
            .collect();
        runnable.sort_by(|a, b| {
            self.score(b)
                .partial_cmp(&self.score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut blocked_by_resource = HashMap::new();
        for c in candidates.iter().filter(|c| !c.is_runnable) {
            if let Some(ref reason) = c.blocked_reason {
                *blocked_by_resource.entry(reason.clone()).or_insert(0) += 1;
            }
        }

        ProgramScheduleDecision {
            selected: runnable.first().cloned(),
            candidates: candidates.to_vec(),
            runnable_count: runnable.len(),
            blocked_by_resource,
            score_breakdown: HashMap::new(),
            policy_name: self.name().to_string(),
            decided_at: chrono::Utc::now(),
        }
    }
}

/// 4. Critical-Path-Aware - Prioritize processes on the critical path.
pub struct ProgramCriticalPathPolicy;

impl ProgramSchedulingPolicy for ProgramCriticalPathPolicy {
    fn name(&self) -> &str {
        "ProgramCriticalPath"
    }

    fn score(&self, candidate: &ProcessCandidate) -> f64 {
        // Heavy weight on critical path and unlocked work
        candidate.critical_path_importance * 0.5
            + candidate.unlocked_work_ratio * 0.3
            + candidate.slo_breach_risk * 0.2
    }

    fn select(&self, candidates: &[ProcessCandidate]) -> ProgramScheduleDecision {
        let mut runnable: Vec<_> = candidates
            .iter()
            .filter(|c| c.is_runnable)
            .cloned()
            .collect();
        runnable.sort_by(|a, b| {
            self.score(b)
                .partial_cmp(&self.score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut blocked_by_resource = HashMap::new();
        for c in candidates.iter().filter(|c| !c.is_runnable) {
            if let Some(ref reason) = c.blocked_reason {
                *blocked_by_resource.entry(reason.clone()).or_insert(0) += 1;
            }
        }

        let score_breakdown = runnable
            .first()
            .map(|c| {
                let mut map = HashMap::new();
                map.insert("critical_path".into(), c.critical_path_importance);
                map.insert("unlocked_work".into(), c.unlocked_work_ratio);
                map.insert("slo_risk".into(), c.slo_breach_risk);
                map.insert("warm_state".into(), c.warm_state_benefit);
                map
            })
            .unwrap_or_default();

        ProgramScheduleDecision {
            selected: runnable.first().cloned(),
            candidates: candidates.to_vec(),
            runnable_count: runnable.len(),
            blocked_by_resource,
            score_breakdown,
            policy_name: self.name().to_string(),
            decided_at: chrono::Utc::now(),
        }
    }
}

/// 5. State-aware priority formula.
///
/// This policy evaluates the full P_i priority formula:
///
/// ```text
/// P_i = (C_i * U_i * S_i * W_i) / (T_i * R_i * D_i)
/// ```
///
/// IMPORTANT: This is a RESEARCH policy. It must be experimentally
/// compared against all baselines before being promoted to default.
pub struct StateAwarePolicy {
    /// Weight for critical path importance.
    pub weight_critical: f64,
    /// Weight for unlocked work.
    pub weight_unlocked: f64,
    /// Weight for SLO risk.
    pub weight_slo: f64,
    /// Weight for warm state benefit.
    pub weight_warm: f64,
}

impl Default for StateAwarePolicy {
    fn default() -> Self {
        Self {
            weight_critical: 1.0,
            weight_unlocked: 0.8,
            weight_slo: 1.2,
            weight_warm: 0.6,
        }
    }
}

impl ProgramSchedulingPolicy for StateAwarePolicy {
    fn name(&self) -> &str {
        "StateAware (Experimental P_i)"
    }

    fn score(&self, candidate: &ProcessCandidate) -> f64 {
        // Numerator: benefits of running this process
        let c = candidate.critical_path_importance * self.weight_critical;
        let u = candidate.unlocked_work_ratio * self.weight_unlocked;
        let s = candidate.slo_breach_risk * self.weight_slo;
        let w = candidate.warm_state_benefit * self.weight_warm;

        let numerator = (c * u * s * w).max(0.001);

        // Denominator: costs and risks
        let t = (candidate.estimated_remaining_time_ms as f64).max(1.0);
        let r = (candidate.resource_cost).max(0.001);
        let d = (candidate.prediction_uncertainty).max(1.0);

        let denominator = (t * r * d).max(0.001);

        numerator / denominator
    }

    fn select(&self, candidates: &[ProcessCandidate]) -> ProgramScheduleDecision {
        let mut runnable: Vec<_> = candidates
            .iter()
            .filter(|c| c.is_runnable)
            .cloned()
            .collect();

        // Compute scores
        for c in &mut runnable {
            c.priority_score = self.score(c);
        }

        runnable.sort_by(|a, b| {
            b.priority_score
                .partial_cmp(&a.priority_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut blocked_by_resource = HashMap::new();
        for c in candidates.iter().filter(|c| !c.is_runnable) {
            if let Some(ref reason) = c.blocked_reason {
                *blocked_by_resource.entry(reason.clone()).or_insert(0) += 1;
            }
        }

        let score_breakdown = runnable
            .first()
            .map(|c| {
                let mut map = HashMap::new();
                map.insert("priority_score".into(), c.priority_score);
                map.insert(
                    "C_critical_path".into(),
                    c.critical_path_importance * self.weight_critical,
                );
                map.insert(
                    "U_unlocked".into(),
                    c.unlocked_work_ratio * self.weight_unlocked,
                );
                map.insert("S_slo_risk".into(), c.slo_breach_risk * self.weight_slo);
                map.insert(
                    "W_warm_state".into(),
                    c.warm_state_benefit * self.weight_warm,
                );
                map.insert("T_time".into(), c.estimated_remaining_time_ms as f64);
                map.insert("R_cost".into(), c.resource_cost);
                map.insert("D_uncertainty".into(), c.prediction_uncertainty);
                map
            })
            .unwrap_or_default();

        ProgramScheduleDecision {
            selected: runnable.first().cloned(),
            candidates: candidates.to_vec(),
            runnable_count: runnable.len(),
            blocked_by_resource,
            score_breakdown,
            policy_name: self.name().to_string(),
            decided_at: chrono::Utc::now(),
        }
    }
}

// -- Program Scheduler Implementation --

impl ProgramScheduler {
    /// Create a new program scheduler with the given policy.
    pub fn new(policy: Box<dyn ProgramSchedulingPolicy>) -> Self {
        Self {
            policy,
            history: Vec::new(),
            decisions_made: 0,
        }
    }

    /// Create a scheduler with the experimental StateAware (P_i) policy.
    pub fn with_state_aware() -> Self {
        Self::new(Box::new(StateAwarePolicy::default()))
    }

    /// Create a scheduler with all baseline policies in a registry.
    pub fn with_all_baselines() -> ProgramPolicyRegistry {
        let mut registry = ProgramPolicyRegistry::new();
        registry.register(Box::new(ProgramFifoPolicy));
        registry.register(Box::new(ProgramPriorityPolicy));
        registry.register(Box::new(ProgramSRTFPolicy));
        registry.register(Box::new(ProgramCriticalPathPolicy));
        registry.register(Box::new(StateAwarePolicy::default()));
        registry
    }

    /// Schedule the next process to advance.
    pub fn schedule(&mut self, candidates: &[ProcessCandidate]) -> ProgramScheduleDecision {
        let decision = self.policy.select(candidates);
        self.history.push(decision.clone());
        self.decisions_made += 1;
        decision
    }

    /// Get the number of decisions made.
    pub fn decisions_made(&self) -> u64 {
        self.decisions_made
    }

    /// Get the scheduling history.
    pub fn history(&self) -> &[ProgramScheduleDecision] {
        &self.history
    }
}

// -- Policy Registry --

/// Registry of program-level scheduling policies.
pub struct ProgramPolicyRegistry {
    policies: HashMap<String, Box<dyn ProgramSchedulingPolicy>>,
}

impl Default for ProgramPolicyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgramPolicyRegistry {
    pub fn new() -> Self {
        Self {
            policies: HashMap::new(),
        }
    }

    pub fn register(&mut self, policy: Box<dyn ProgramSchedulingPolicy>) {
        self.policies.insert(policy.name().to_string(), policy);
    }

    pub fn get(&self, name: &str) -> Option<&dyn ProgramSchedulingPolicy> {
        self.policies.get(name).map(|p| p.as_ref())
    }

    pub fn list(&self) -> Vec<String> {
        self.policies.keys().cloned().collect()
    }

    /// Schedule using a specific policy by name.
    pub fn schedule_with(
        &self,
        policy_name: &str,
        candidates: &[ProcessCandidate],
    ) -> Result<ProgramScheduleDecision, String> {
        let policy = self
            .policies
            .get(policy_name)
            .ok_or_else(|| format!("Policy '{}' not found", policy_name))?;
        Ok(policy.select(candidates))
    }
}

// -- Candidate Builder --

/// Build ProcessCandidate objects from process state for scheduling.
pub fn build_candidates(process_states: &[ProcessSchedulingInfo]) -> Vec<ProcessCandidate> {
    process_states
        .iter()
        .map(|info| {
            let is_runnable = matches!(info.status, ProcessSchedulingStatus::Runnable);
            let priority_score = 0.0; // Computed by the policy

            ProcessCandidate {
                process_id: info.process_id.clone(),
                status: info.status,
                critical_path_importance: info.critical_path_importance,
                unlocked_work_ratio: info.unlocked_work_ratio,
                slo_breach_risk: info.slo_breach_risk,
                warm_state_benefit: info.context_residency_ratio,
                estimated_remaining_time_ms: info.estimated_remaining_time_ms,
                resource_cost: info.normalized_resource_cost,
                prediction_uncertainty: info.prediction_uncertainty,
                priority_score,
                is_runnable,
                blocked_reason: info.blocked_reason.clone(),
                waiting_children: info.waiting_children,
                budget_remaining_ratio: info.budget_remaining_ratio,
            }
        })
        .collect()
}

/// Scheduling-relevant process information (fed into the Program Scheduler).
#[derive(Debug, Clone)]
pub struct ProcessSchedulingInfo {
    pub process_id: String,
    pub status: ProcessSchedulingStatus,
    pub critical_path_importance: f64,
    pub unlocked_work_ratio: f64,
    pub slo_breach_risk: f64,
    pub context_residency_ratio: f64,
    pub estimated_remaining_time_ms: u64,
    pub normalized_resource_cost: f64,
    pub prediction_uncertainty: f64,
    pub blocked_reason: Option<String>,
    pub waiting_children: u32,
    pub budget_remaining_ratio: f64,
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    fn make_runnable(
        id: &str,
        criticality: f64,
        unlocked: f64,
        warm: f64,
        remaining_ms: u64,
    ) -> ProcessCandidate {
        ProcessCandidate {
            process_id: id.into(),
            status: ProcessSchedulingStatus::Runnable,
            critical_path_importance: criticality,
            unlocked_work_ratio: unlocked,
            slo_breach_risk: 1.0,
            warm_state_benefit: warm,
            estimated_remaining_time_ms: remaining_ms,
            resource_cost: 0.5,
            prediction_uncertainty: 1.0,
            priority_score: 0.0,
            is_runnable: true,
            blocked_reason: None,
            waiting_children: 0,
            budget_remaining_ratio: 1.0,
        }
    }

    fn make_blocked(id: &str, reason: &str) -> ProcessCandidate {
        ProcessCandidate {
            process_id: id.into(),
            status: ProcessSchedulingStatus::Blocked,
            critical_path_importance: 0.5,
            unlocked_work_ratio: 0.3,
            slo_breach_risk: 1.0,
            warm_state_benefit: 0.5,
            estimated_remaining_time_ms: 5000,
            resource_cost: 0.3,
            prediction_uncertainty: 1.0,
            priority_score: 0.0,
            is_runnable: false,
            blocked_reason: Some(reason.into()),
            waiting_children: 0,
            budget_remaining_ratio: 1.0,
        }
    }

    #[test]
    fn test_fifo_selects_first_runnable() {
        let policy = ProgramFifoPolicy;
        let candidates = vec![
            make_runnable("proc-2", 0.5, 0.3, 0.5, 5000),
            make_runnable("proc-1", 0.9, 0.8, 0.9, 1000),
        ];
        let decision = policy.select(&candidates);
        assert!(decision.selected.is_some());
        assert_eq!(decision.selected.unwrap().process_id, "proc-2"); // First in list
    }

    #[test]
    fn test_srtf_prefers_shortest() {
        let policy = ProgramSRTFPolicy;
        let candidates = vec![
            make_runnable("long", 0.5, 0.3, 0.5, 10000),
            make_runnable("short", 0.5, 0.3, 0.5, 100),
        ];
        let decision = policy.select(&candidates);
        assert_eq!(decision.selected.unwrap().process_id, "short");
    }

    #[test]
    fn test_critical_path_prefers_important() {
        let policy = ProgramCriticalPathPolicy;
        let candidates = vec![
            make_runnable("normal", 0.2, 0.1, 0.5, 1000),
            make_runnable("critical", 0.95, 0.9, 0.5, 1000),
        ];
        let decision = policy.select(&candidates);
        assert_eq!(decision.selected.unwrap().process_id, "critical");
    }

    #[test]
    fn test_state_aware_pi_formula() {
        let policy = StateAwarePolicy::default();
        let candidates = vec![
            // High criticality, high warm state, short time -> high priority
            make_runnable("hot-critical", 0.95, 0.9, 0.95, 100),
            // Low criticality, cold state, long time -> low priority
            make_runnable("cold-normal", 0.1, 0.1, 0.1, 10000),
        ];
        let decision = policy.select(&candidates);
        assert_eq!(decision.selected.unwrap().process_id, "hot-critical");
        assert!(!decision.score_breakdown.is_empty());
    }

    #[test]
    fn test_blocked_processes_tracked() {
        let policy = ProgramFifoPolicy;
        let candidates = vec![
            make_runnable("r1", 0.5, 0.3, 0.5, 1000),
            make_blocked("b1", "waiting_model"),
            make_blocked("b2", "waiting_model"),
            make_blocked("b3", "waiting_approval"),
        ];
        let decision = policy.select(&candidates);
        assert_eq!(decision.runnable_count, 1);
        assert_eq!(decision.blocked_by_resource.get("waiting_model"), Some(&2));
        assert_eq!(
            decision.blocked_by_resource.get("waiting_approval"),
            Some(&1)
        );
    }

    #[test]
    fn test_policy_registry() {
        let registry = ProgramScheduler::with_all_baselines();
        let names = registry.list();
        assert!(names.contains(&"ProgramFIFO".to_string()));
        assert!(names.contains(&"ProgramCriticalPath".to_string()));
        assert!(names.contains(&"StateAware (Experimental P_i)".to_string()));
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn test_state_aware_penalizes_uncertainty() {
        let policy = StateAwarePolicy::default();

        let mut certain = make_runnable("certain", 0.8, 0.7, 0.8, 1000);
        certain.prediction_uncertainty = 1.0; // Very certain

        let mut uncertain = make_runnable("uncertain", 0.8, 0.7, 0.8, 1000);
        uncertain.prediction_uncertainty = 5.0; // Highly uncertain

        let certain_score = policy.score(&certain);
        let uncertain_score = policy.score(&uncertain);

        // Same properties but more uncertainty -> lower score
        assert!(certain_score > uncertain_score);
    }

    #[test]
    fn test_warm_state_boosts_priority() {
        let policy = StateAwarePolicy::default();

        let warm = make_runnable("warm", 0.8, 0.7, 0.95, 1000);
        let cold = make_runnable("cold", 0.8, 0.7, 0.05, 1000);

        let warm_score = policy.score(&warm);
        let cold_score = policy.score(&cold);

        assert!(warm_score > cold_score);
    }
}
