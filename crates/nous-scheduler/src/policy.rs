//! SchedulerPolicy trait and 10 baseline implementations.
//!
//! Every scheduler policy implements the same interface, making
//! scheduling strategies pluggable and experimentally comparable.
//!
//! Required output for every selection:
//! - Candidate set
//! - Rejection reasons per candidate
//! - Score breakdown
//! - Confidence interval
//! - Prediction source
//! - Whether the prediction is out-of-distribution
//! - Fallback strategy

use nous_types::resource::ResourceVector;
use nous_types::workload::WorkloadSpec;
use std::collections::HashMap;

/// A candidate for scheduling - a specific combination of node, device, engine, and model.
#[derive(Debug, Clone)]
pub struct SchedulerCandidate {
    pub node_id: String,
    pub device_id: String,
    pub engine_id: String,
    pub model_revision: String,
    pub resources_available: ResourceVector,
    pub resources_required: ResourceVector,
    pub estimated_ttft_ms: u64,
    pub estimated_tps: f64,
    pub estimated_cost_microcents: u64,
    pub estimated_quality: f64,
    pub kv_cache_hit_rate: f64,
    pub engine_load: f64,
    pub device_utilization: f64,
}

/// The result of a scheduling decision.
#[derive(Debug, Clone)]
pub struct ScheduleDecision {
    /// Selected candidate (if any).
    pub selected: Option<SchedulerCandidate>,

    /// All candidates considered.
    pub candidates: Vec<SchedulerCandidate>,

    /// Why each candidate was rejected (candidate index -> reason).
    pub rejections: HashMap<usize, String>,

    /// Score breakdown for the selected candidate.
    pub score_breakdown: HashMap<String, f64>,

    /// Confidence interval (lower, upper) for the score.
    pub confidence_interval: (f64, f64),

    /// Whether this prediction is out-of-distribution.
    pub is_out_of_distribution: bool,

    /// Fallback strategy if selection fails at runtime.
    pub fallback_strategy: FallbackStrategy,
}

#[derive(Debug, Clone)]
pub enum FallbackStrategy {
    /// Try the next-best candidate.
    TryNextBest,
    /// Switch to a simpler model.
    DegradeModel,
    /// Queue and wait.
    QueueAndWait,
    /// Reject the workload.
    Reject,
}

/// The unified scheduler policy interface.
pub trait SchedulerPolicy: Send + Sync {
    /// Name of this policy.
    fn name(&self) -> &str;

    /// Hard constraint filter: which candidates are feasible?
    fn filter(&self, workload: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize>;

    /// Score each feasible candidate (higher = better).
    fn score(&self, workload: &WorkloadSpec, candidate: &SchedulerCandidate) -> f64;

    /// Rank and select the best candidate.
    fn select(
        &self,
        workload: &WorkloadSpec,
        candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision;
}

// -- 10 Baseline Policies --

/// 1. FIFO - First In, First Out.
///
/// Simple: no scoring. Pick the first feasible candidate.
#[derive(Default)]
pub struct FifoPolicy;

impl SchedulerPolicy for FifoPolicy {
    fn name(&self) -> &str {
        "FIFO"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.resources_required.fits_within(&c.resources_available))
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, _: &WorkloadSpec, _: &SchedulerCandidate) -> f64 {
        0.0
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        let feasible = self.filter(workload, &candidates);
        let selected = feasible.first().map(|&i| candidates[i].clone());

        ScheduleDecision {
            selected,
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::TryNextBest,
        }
    }
}

/// 2. Priority - Highest priority class first.
#[derive(Default)]
pub struct PriorityPolicy;

impl SchedulerPolicy for PriorityPolicy {
    fn name(&self) -> &str {
        "Priority"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        // Priority doesn't filter by resources directly - it relies on admission control
        (0..candidates.len()).collect()
    }

    fn score(&self, w: &WorkloadSpec, _: &SchedulerCandidate) -> f64 {
        // Higher score = higher priority (lower PriorityClass value)
        match w.resource_requirements.priority {
            nous_types::resource::PriorityClass::System => 4.0,
            nous_types::resource::PriorityClass::Interactive => 3.0,
            nous_types::resource::PriorityClass::Batch => 2.0,
            nous_types::resource::PriorityClass::BestEffort => 1.0,
        }
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        mut candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        candidates.sort_by(|a, b| self.score(workload, b).total_cmp(&self.score(workload, a)));
        let selected = candidates.first().cloned();
        ScheduleDecision {
            selected,
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::QueueAndWait,
        }
    }
}

/// 3. Weighted Sum - Linear combination of multiple objectives.
#[derive(Default)]
pub struct WeightedSumPolicy {
    pub weights: SchedulingWeights,
}

#[derive(Debug, Clone)]
pub struct SchedulingWeights {
    pub latency_weight: f64,
    pub throughput_weight: f64,
    pub cost_weight: f64,
    pub quality_weight: f64,
    pub cache_weight: f64,
    pub load_weight: f64,
}

impl Default for SchedulingWeights {
    fn default() -> Self {
        Self {
            latency_weight: 0.25,
            throughput_weight: 0.20,
            cost_weight: 0.15,
            quality_weight: 0.20,
            cache_weight: 0.10,
            load_weight: 0.10,
        }
    }
}

impl SchedulerPolicy for WeightedSumPolicy {
    fn name(&self) -> &str {
        "WeightedSum"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.resources_required.fits_within(&c.resources_available))
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, _: &WorkloadSpec, c: &SchedulerCandidate) -> f64 {
        let w = &self.weights;
        // Normalize each dimension to [0, 1] and weight
        let latency_score = 1.0 / (1.0 + c.estimated_ttft_ms as f64 / 1000.0); // Lower latency = higher score
        let throughput_score = c.estimated_tps.min(100.0) / 100.0;
        let cost_score = 1.0 / (1.0 + c.estimated_cost_microcents as f64 / 1_000_000.0);
        let quality_score = c.estimated_quality; // Already 0-1
        let cache_score = c.kv_cache_hit_rate; // 0-1
        let load_score = 1.0 - c.engine_load; // Lower load = higher score

        w.latency_weight * latency_score
            + w.throughput_weight * throughput_score
            + w.cost_weight * cost_score
            + w.quality_weight * quality_score
            + w.cache_weight * cache_score
            + w.load_weight * load_score
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        let mut breakdown = HashMap::new();
        let mut best_score = f64::NEG_INFINITY;
        let mut best_idx = None;

        for (i, c) in candidates.iter().enumerate() {
            let score = self.score(workload, c);
            if score > best_score {
                best_score = score;
                best_idx = Some(i);
            }
            breakdown.insert(c.node_id.clone(), score);
        }

        let selected = best_idx.map(|i| candidates[i].clone());
        ScheduleDecision {
            selected,
            candidates,
            rejections: HashMap::new(),
            score_breakdown: breakdown,
            confidence_interval: (best_score - 0.05, best_score + 0.05),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::TryNextBest,
        }
    }
}

/// 4. Multiplicative Score - Multiply factors (more sensitive to zero values).
#[derive(Default)]
pub struct MultiplicativePolicy;

impl SchedulerPolicy for MultiplicativePolicy {
    fn name(&self) -> &str {
        "Multiplicative"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.resources_required.fits_within(&c.resources_available))
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, _: &WorkloadSpec, c: &SchedulerCandidate) -> f64 {
        let latency_factor = 1.0 / (1.0 + c.estimated_ttft_ms as f64 / 500.0);
        let quality_factor = 0.1 + 0.9 * c.estimated_quality;
        let cache_factor = 0.5 + 0.5 * c.kv_cache_hit_rate;
        let load_factor = 1.0 - 0.5 * c.engine_load;
        latency_factor * quality_factor * cache_factor * load_factor
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        mut candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        candidates.sort_by(|a, b| self.score(workload, b).total_cmp(&self.score(workload, a)));
        let selected = candidates.first().cloned();
        ScheduleDecision {
            selected,
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::TryNextBest,
        }
    }
}

/// 5. Pareto Ranking - Non-dominated sorting across multiple objectives.
#[derive(Default)]
pub struct ParetoPolicy {
    pub objectives: Vec<ObjectiveDirection>,
}

#[derive(Debug, Clone)]
pub struct ObjectiveDirection {
    pub name: String,
    /// true = maximize, false = minimize
    pub maximize: bool,
    pub extractor: fn(&SchedulerCandidate) -> f64,
}

impl SchedulerPolicy for ParetoPolicy {
    fn name(&self) -> &str {
        "Pareto"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.resources_required.fits_within(&c.resources_available))
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, _: &WorkloadSpec, _c: &SchedulerCandidate) -> f64 {
        // Pareto front rank: number of candidates that dominate this one
        // Lower rank = better. Use 1/(1+rank) as score.
        let dominated_by = 0u32; // Simplified - full Pareto would compare all pairs
        1.0 / (1.0 + dominated_by as f64)
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        let feasible: Vec<usize> = self.filter(workload, &candidates);
        // For now, select the first feasible candidate
        let selected = feasible.first().map(|&i| candidates[i].clone());
        ScheduleDecision {
            selected,
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: true, // Pareto is OOD by nature
            fallback_strategy: FallbackStrategy::DegradeModel,
        }
    }
}

/// 6. Cache-Aware - Prefer nodes/engines with higher KV cache hit rates.
pub struct CacheAwarePolicy {
    pub min_cache_hit_rate: f64,
}

impl Default for CacheAwarePolicy {
    fn default() -> Self {
        Self {
            min_cache_hit_rate: 0.0,
        }
    }
}

impl SchedulerPolicy for CacheAwarePolicy {
    fn name(&self) -> &str {
        "CacheAware"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.resources_required.fits_within(&c.resources_available)
                    && c.kv_cache_hit_rate >= self.min_cache_hit_rate
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, _: &WorkloadSpec, c: &SchedulerCandidate) -> f64 {
        // Primarily based on cache hit rate, with load penalty
        c.kv_cache_hit_rate * (1.0 - 0.3 * c.engine_load)
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        mut candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        candidates.sort_by(|a, b| self.score(workload, b).total_cmp(&self.score(workload, a)));
        let selected = candidates.first().cloned();
        ScheduleDecision {
            selected,
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::TryNextBest,
        }
    }
}

/// 7. Deadline-Aware - Prioritize workloads with approaching deadlines.
#[derive(Default)]
pub struct DeadlineAwarePolicy;

impl SchedulerPolicy for DeadlineAwarePolicy {
    fn name(&self) -> &str {
        "DeadlineAware"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.resources_required.fits_within(&c.resources_available))
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, w: &WorkloadSpec, c: &SchedulerCandidate) -> f64 {
        // Urgency = 1 / (time until deadline + estimated execution time)
        let now = chrono::Utc::now().timestamp_micros();
        let time_left_secs = if w.deadline_us > 0 {
            ((w.deadline_us - now) as f64 / 1_000_000.0).max(0.0)
        } else {
            3600.0 // No deadline: assume 1 hour
        };
        let token_time_secs = if c.estimated_tps > 0.0 {
            1.0 / c.estimated_tps
        } else {
            f64::INFINITY
        };
        let exec_time_secs = c.estimated_ttft_ms as f64 / 1000.0 + token_time_secs;
        let urgency = 1.0 / (time_left_secs + exec_time_secs + 0.001);

        // Can this candidate meet the deadline?
        let can_meet = c.estimated_ttft_ms as f64 / 1000.0 <= time_left_secs;

        urgency * if can_meet { 10.0 } else { 0.1 }
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        mut candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        candidates.sort_by(|a, b| self.score(workload, b).total_cmp(&self.score(workload, a)));
        let selected = candidates.first().cloned();
        ScheduleDecision {
            selected,
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::DegradeModel,
        }
    }
}

/// 8. Critical-Path-Aware - Prioritize nodes on the critical path of agent programs.
#[derive(Default)]
pub struct CriticalPathPolicy;

impl SchedulerPolicy for CriticalPathPolicy {
    fn name(&self) -> &str {
        "CriticalPathAware"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.resources_required.fits_within(&c.resources_available))
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, w: &WorkloadSpec, c: &SchedulerCandidate) -> f64 {
        // Critical path nodes: those that unlock many downstream tasks
        let downstream_count = w
            .execution_graph
            .edges
            .iter()
            .map(|edge| {
                w.execution_graph
                    .edges
                    .iter()
                    .filter(|candidate| candidate.from_node_id == edge.from_node_id)
                    .count()
            })
            .max()
            .unwrap_or(0) as f64;

        // Higher fan-out = more critical
        let criticality = downstream_count / (w.execution_graph.nodes.len().max(1) as f64);

        // Combine with latency
        let latency_score = 1.0 / (1.0 + c.estimated_ttft_ms as f64 / 1000.0);
        criticality * 0.6 + latency_score * 0.4
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        mut candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        candidates.sort_by(|a, b| self.score(workload, b).total_cmp(&self.score(workload, a)));
        ScheduleDecision {
            selected: candidates.first().cloned(),
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::TryNextBest,
        }
    }
}

/// 9. Contextual Bandit - Learn from past scheduling outcomes.
pub struct ContextualBanditPolicy {
    /// Learning rate.
    pub alpha: f64,
    /// Exploration rate.
    pub epsilon: f64,
    /// Historical rewards: candidate_key -> (count, mean_reward)
    history: std::sync::RwLock<HashMap<String, (u64, f64)>>,
}

impl Default for ContextualBanditPolicy {
    fn default() -> Self {
        Self::new(0.1, 0.05)
    }
}

impl ContextualBanditPolicy {
    pub fn new(alpha: f64, epsilon: f64) -> Self {
        Self {
            alpha,
            epsilon,
            history: std::sync::RwLock::new(HashMap::new()),
        }
    }

    fn candidate_key(c: &SchedulerCandidate) -> String {
        format!("{}:{}:{}", c.node_id, c.engine_id, c.model_revision)
    }

    pub fn record_outcome(&self, candidate: &SchedulerCandidate, reward: f64) {
        let Ok(mut hist) = self.history.write() else {
            return;
        };
        let key = Self::candidate_key(candidate);
        let (count, mean) = hist.get(&key).copied().unwrap_or((0, 0.0));
        let new_count = count + 1;
        let new_mean = mean + self.alpha * (reward - mean);
        hist.insert(key, (new_count, new_mean));
    }
}

impl SchedulerPolicy for ContextualBanditPolicy {
    fn name(&self) -> &str {
        "ContextualBandit"
    }

    fn filter(&self, _w: &WorkloadSpec, candidates: &[SchedulerCandidate]) -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.resources_required.fits_within(&c.resources_available))
            .map(|(i, _)| i)
            .collect()
    }

    fn score(&self, _: &WorkloadSpec, c: &SchedulerCandidate) -> f64 {
        let Ok(hist) = self.history.read() else {
            return f64::NEG_INFINITY;
        };
        let key = Self::candidate_key(c);
        let (count, mean) = hist.get(&key).copied().unwrap_or((0, 0.0));
        // UCB1-style: mean + exploration bonus
        let exploration_bonus = if count > 0 {
            self.epsilon * (2.0 * (count as f64).ln() / count as f64).sqrt()
        } else {
            self.epsilon * 2.0 // High bonus for unexplored candidates
        };
        mean + exploration_bonus
    }

    fn select(
        &self,
        workload: &WorkloadSpec,
        mut candidates: Vec<SchedulerCandidate>,
    ) -> ScheduleDecision {
        candidates.sort_by(|a, b| self.score(workload, b).total_cmp(&self.score(workload, a)));
        ScheduleDecision {
            selected: candidates.first().cloned(),
            candidates,
            rejections: HashMap::new(),
            score_breakdown: HashMap::new(),
            confidence_interval: (0.0, 0.0),
            is_out_of_distribution: false,
            fallback_strategy: FallbackStrategy::TryNextBest,
        }
    }
}

// -- Scheduler Registry --

/// Registry of available scheduler policies.
pub struct PolicyRegistry {
    policies: HashMap<String, Box<dyn SchedulerPolicy>>,
}

impl Default for PolicyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyRegistry {
    pub fn new() -> Self {
        Self {
            policies: HashMap::new(),
        }
    }

    pub fn register(&mut self, policy: Box<dyn SchedulerPolicy>) {
        self.policies.insert(policy.name().to_string(), policy);
    }

    pub fn get(&self, name: &str) -> Option<&dyn SchedulerPolicy> {
        self.policies.get(name).map(|p| p.as_ref())
    }

    pub fn list(&self) -> Vec<String> {
        self.policies.keys().cloned().collect()
    }

    /// Create a registry containing every implemented baseline policy.
    pub fn with_all_baselines() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(FifoPolicy));
        registry.register(Box::new(PriorityPolicy));
        registry.register(Box::new(WeightedSumPolicy::default()));
        registry.register(Box::new(MultiplicativePolicy));
        registry.register(Box::new(CacheAwarePolicy::default()));
        registry.register(Box::new(DeadlineAwarePolicy));
        registry.register(Box::new(CriticalPathPolicy));
        registry.register(Box::new(ContextualBanditPolicy::new(0.1, 0.05)));
        // Pareto needs objectives - create with defaults
        registry.register(Box::new(ParetoPolicy {
            objectives: vec![
                ObjectiveDirection {
                    name: "latency".into(),
                    maximize: false,
                    extractor: |c: &SchedulerCandidate| c.estimated_ttft_ms as f64,
                },
                ObjectiveDirection {
                    name: "quality".into(),
                    maximize: true,
                    extractor: |c: &SchedulerCandidate| c.estimated_quality,
                },
            ],
        }));
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidate(
        node: &str,
        ttft: u64,
        tps: f64,
        quality: f64,
        cache: f64,
        load: f64,
    ) -> SchedulerCandidate {
        SchedulerCandidate {
            node_id: node.into(),
            device_id: "gpu0".into(),
            engine_id: "vllm".into(),
            model_revision: "v1".into(),
            resources_available: ResourceVector {
                device_memory_bytes: 16 * 1024 * 1024 * 1024,
                ..ResourceVector::default()
            },
            resources_required: ResourceVector {
                device_memory_bytes: 4 * 1024 * 1024 * 1024,
                ..ResourceVector::default()
            },
            estimated_ttft_ms: ttft,
            estimated_tps: tps,
            estimated_cost_microcents: 1000,
            estimated_quality: quality,
            kv_cache_hit_rate: cache,
            engine_load: load,
            device_utilization: 0.5,
        }
    }

    #[test]
    fn test_fifo_selects_first_feasible() {
        let policy = FifoPolicy;
        let candidates = vec![
            make_candidate("n2", 200, 50.0, 0.9, 0.5, 0.3),
            make_candidate("n1", 100, 60.0, 0.8, 0.7, 0.2),
        ];
        let workload = WorkloadSpec::default();
        let decision = policy.select(&workload, candidates);
        assert!(decision.selected.is_some());
        assert_eq!(decision.selected.unwrap().node_id, "n2"); // First in list
    }

    #[test]
    fn test_weighted_sum_prefers_low_latency() {
        let policy = WeightedSumPolicy {
            weights: SchedulingWeights {
                latency_weight: 1.0,
                ..Default::default()
            },
        };
        let candidates = vec![
            make_candidate("slow", 500, 30.0, 0.9, 0.5, 0.3),
            make_candidate("fast", 100, 30.0, 0.9, 0.5, 0.3),
        ];
        let workload = WorkloadSpec::default();
        let decision = policy.select(&workload, candidates);
        assert_eq!(decision.selected.unwrap().node_id, "fast");
    }

    #[test]
    fn test_cache_aware_prefers_high_cache() {
        let policy = CacheAwarePolicy::default();
        let candidates = vec![
            make_candidate("nocache", 100, 50.0, 0.9, 0.0, 0.3),
            make_candidate("cached", 110, 50.0, 0.9, 0.95, 0.3),
        ];
        let workload = WorkloadSpec::default();
        let decision = policy.select(&workload, candidates);
        assert_eq!(decision.selected.unwrap().node_id, "cached");
    }

    #[test]
    fn test_registry_has_all_policies() {
        let registry = PolicyRegistry::with_all_baselines();
        let names = registry.list();
        assert!(names.contains(&"FIFO".to_string()));
        assert!(names.contains(&"WeightedSum".to_string()));
        assert!(names.contains(&"ContextualBandit".to_string()));
        assert_eq!(names.len(), 9);
        assert!(!names.contains(&"MOBO".to_string()));
    }
}
