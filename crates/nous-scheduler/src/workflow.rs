//! Agent Program Compiler and Workflow Scheduler.
//!
//! The compiler transforms agent and workflow definitions into
//! AgentExecutionGraphs that the kernel can execute.
//!
//! Compiler analysis phases:
//! 1. Parse agent/workflow definition
//! 2. Build dependency graph (data, control, tool)
//! 3. Identify parallel regions
//! 4. Detect cache reuse opportunities
//! 5. Compute critical path
//! 6. Insert verification nodes
//! 7. Insert checkpoint nodes
//! 8. Estimate resources per step
//! 9. Output AgentExecutionGraph

use nous_types::resource::ResourceVector;
use nous_types::workload::{
    EdgeCondition, ExecutionGraph, PhaseConfig, PhaseEdge, PhaseNode, PhaseType,
};
use std::collections::{HashMap, HashSet, VecDeque};

// -- Agent Program Definition --

/// A step in an agent program before compilation.
#[derive(Debug, Clone)]
pub struct AgentStep {
    pub id: String,
    pub description: String,
    pub step_type: AgentStepType,
    /// IDs of steps this step depends on.
    pub depends_on: Vec<String>,
    /// Estimated model calls needed.
    pub model_calls: u32,
    /// Whether this step requires a strong model.
    pub requires_strong_model: bool,
    /// Tools this step may call.
    pub tools: Vec<String>,
    /// Estimated input tokens.
    pub estimated_input_tokens: u64,
    /// Estimated output tokens.
    pub estimated_output_tokens: u64,
    /// Whether this step has side effects.
    pub has_side_effects: bool,
    /// Whether side effects are reversible.
    pub side_effects_reversible: bool,
    /// Required capabilities.
    pub required_capabilities: Vec<String>,
    /// Whether human approval is required before this step.
    pub requires_approval: bool,
    /// Whether output must be verified.
    pub requires_verification: bool,
    /// Maximum retries for this step.
    pub max_retries: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStepType {
    /// Think/plan - no tools, just reasoning.
    Think,
    /// Retrieve knowledge.
    Retrieve,
    /// Call a tool.
    ToolCall,
    /// Generate output.
    Generate,
    /// Verify output.
    Verify,
    /// Wait for human approval.
    Approval,
    /// Commit side effects.
    Commit,
    /// Compensate/rollback.
    Compensate,
}

/// A compiled agent execution graph ready for scheduling.
#[derive(Debug, Clone)]
pub struct AgentExecutionGraph {
    /// The execution graph (phases and edges).
    pub graph: ExecutionGraph,
    /// Critical path: node IDs in order of criticality.
    pub critical_path: Vec<String>,
    /// Nodes that can run in parallel.
    pub parallel_groups: Vec<Vec<String>>,
    /// Nodes with high KV cache reuse potential.
    pub cache_reuse_nodes: HashSet<String>,
    /// Total estimated resource cost.
    pub total_resources: ResourceVector,
    /// Total estimated wall-clock time on critical path.
    pub critical_path_duration_ms: u64,
    /// Number of verification boundaries.
    pub verification_boundaries: u32,
    /// Number of approval boundaries.
    pub approval_boundaries: u32,
    /// Checkpoint nodes.
    pub checkpoint_nodes: Vec<String>,
}

// -- Agent Program Compiler --

/// Compiles agent program definitions into executable graphs.
pub struct AgentProgramCompiler {
    /// Default verification model (if not specified per step).
    pub default_verifier_model: String,
    /// Whether to auto-insert checkpoints before risky steps.
    pub auto_checkpoint: bool,
    /// Whether to auto-verify outputs.
    pub auto_verify: bool,
    /// Minimum quality threshold for verification.
    pub quality_threshold: f64,
}

impl Default for AgentProgramCompiler {
    fn default() -> Self {
        Self {
            default_verifier_model: String::new(),
            auto_checkpoint: true,
            auto_verify: true,
            quality_threshold: 0.8,
        }
    }
}

impl AgentProgramCompiler {
    /// Compile a list of agent steps into an AgentExecutionGraph.
    pub fn compile(
        &self,
        steps: &[AgentStep],
        _goal: &str,
    ) -> Result<AgentExecutionGraph, CompileError> {
        // Phase 1: Validate
        self.validate_steps(steps)?;

        // Phase 2: Build dependency graph
        let deps = self.build_dependency_map(steps);

        // Phase 3: Topological sort
        let order = self.topological_sort(steps, &deps)?;

        // Phase 4: Build execution graph nodes
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut node_map: HashMap<String, String> = HashMap::new(); // step_id -> node_id

        for step_id in &order {
            let step = steps.iter().find(|s| &s.id == step_id).unwrap();
            let phase_nodes = self.step_to_phase_nodes(step);
            let mut prev_node_id: Option<String> = None;

            for pn in &phase_nodes {
                let node_id = pn.node_id.clone();

                // Connect to previous node in this step
                if let Some(ref prev) = prev_node_id {
                    edges.push(PhaseEdge {
                        from_node_id: prev.clone(),
                        to_node_id: node_id.clone(),
                        condition: Some(EdgeCondition::OnSuccess),
                    });
                }

                // Connect from dependency steps
                if prev_node_id.is_none() {
                    // First node in this step - connect from dependency steps' last nodes
                    for dep_id in &step.depends_on {
                        if let Some(dep_last_node) = node_map.get(dep_id) {
                            edges.push(PhaseEdge {
                                from_node_id: dep_last_node.clone(),
                                to_node_id: node_id.clone(),
                                condition: Some(EdgeCondition::OnSuccess),
                            });
                        }
                    }
                }

                prev_node_id = Some(node_id.clone());
                nodes.push(pn.clone());
            }

            // Remember the last node of this step
            if let Some(last) = prev_node_id {
                node_map.insert(step.id.clone(), last);
            }
        }

        // Phase 5: Identify parallel groups
        let parallel_groups = self.find_parallel_groups(&nodes, &edges);

        // Phase 6: Compute critical path
        let critical_path = self.compute_critical_path(&nodes, &edges);

        // Phase 7: Detect cache reuse
        let cache_reuse_nodes = self.detect_cache_reuse(&nodes, steps);

        // Phase 8: Estimate total resources
        let total_resources = self.estimate_resources(&nodes, steps);

        // Phase 9: Compute critical path duration
        let critical_path_duration_ms = self.estimate_duration(&critical_path, &nodes);

        // Count boundaries
        let verification_boundaries = nodes
            .iter()
            .filter(|n| n.phase_type == PhaseType::Verify)
            .count() as u32;
        let approval_boundaries = nodes
            .iter()
            .filter(|n| matches!(&n.config.quality_verifier_phase_id, Some(id) if id == "approval"))
            .count() as u32;
        let checkpoint_nodes = nodes
            .iter()
            .filter(|n| n.phase_type == PhaseType::Checkpoint)
            .map(|n| n.node_id.clone())
            .collect();

        let entry_node_id = nodes.first().map(|n| n.node_id.clone()).unwrap_or_default();

        Ok(AgentExecutionGraph {
            graph: ExecutionGraph {
                nodes,
                edges,
                entry_node_id,
            },
            critical_path,
            parallel_groups,
            cache_reuse_nodes,
            total_resources,
            critical_path_duration_ms,
            verification_boundaries,
            approval_boundaries,
            checkpoint_nodes,
        })
    }

    /// Validate steps for consistency.
    fn validate_steps(&self, steps: &[AgentStep]) -> Result<(), CompileError> {
        let ids: HashSet<&str> = steps.iter().map(|s| s.id.as_str()).collect();

        for step in steps {
            // Check all dependencies exist
            for dep in &step.depends_on {
                if !ids.contains(dep.as_str()) {
                    return Err(CompileError::MissingDependency {
                        step: step.id.clone(),
                        missing_dep: dep.clone(),
                    });
                }
            }

            // Check no self-dependency
            if step.depends_on.contains(&step.id) {
                return Err(CompileError::CircularDependency {
                    step: step.id.clone(),
                });
            }
        }

        Ok(())
    }

    /// Build a map of step -> its direct dependencies.
    fn build_dependency_map(&self, steps: &[AgentStep]) -> HashMap<String, Vec<String>> {
        steps
            .iter()
            .map(|s| (s.id.clone(), s.depends_on.clone()))
            .collect()
    }

    /// Topological sort using Kahn's algorithm. Detects cycles.
    fn topological_sort(
        &self,
        steps: &[AgentStep],
        _deps: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<String>, CompileError> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();

        for step in steps {
            in_degree.entry(&step.id).or_insert(0);
            graph.entry(&step.id).or_default();
            for dep in &step.depends_on {
                graph.entry(dep).or_default().push(&step.id);
                *in_degree.entry(&step.id).or_insert(0) += 1;
            }
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut sorted = Vec::new();

        while let Some(node) = queue.pop_front() {
            sorted.push(node.to_string());
            if let Some(neighbors) = graph.get(node) {
                for &neighbor in neighbors {
                    if let Some(deg) = in_degree.get_mut(neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(neighbor);
                        }
                    }
                }
            }
        }

        if sorted.len() != steps.len() {
            return Err(CompileError::CircularDependency {
                step: "cycle detected in dependency graph".into(),
            });
        }

        Ok(sorted)
    }

    /// Convert an agent step into one or more phase nodes.
    fn step_to_phase_nodes(&self, step: &AgentStep) -> Vec<PhaseNode> {
        let mut nodes = Vec::new();
        let sid = &step.id;

        match step.step_type {
            AgentStepType::Think => {
                nodes.push(PhaseNode {
                    node_id: format!("{}-think", sid),
                    phase_type: PhaseType::Plan,
                    config: PhaseConfig {
                        is_idempotent: true,
                        is_cancellable: true,
                        is_retryable: true,
                        estimated_duration_ms: step.estimated_output_tokens as u32 / 50 * 1000,
                        ..Default::default()
                    },
                    description: format!("Think: {}", step.description),
                });
            }
            AgentStepType::Retrieve => {
                nodes.push(PhaseNode {
                    node_id: format!("{}-retrieve", sid),
                    phase_type: PhaseType::Retrieve,
                    config: PhaseConfig {
                        is_idempotent: true,
                        is_cancellable: true,
                        is_retryable: true,
                        estimated_duration_ms: 500,
                        ..Default::default()
                    },
                    description: format!("Retrieve: {}", step.description),
                });
            }
            AgentStepType::ToolCall => {
                // Checkpoint before risky tool calls
                if self.auto_checkpoint && step.has_side_effects {
                    nodes.push(PhaseNode {
                        node_id: format!("{}-checkpoint", sid),
                        phase_type: PhaseType::Checkpoint,
                        config: PhaseConfig {
                            is_idempotent: true,
                            is_cancellable: false,
                            is_checkpointable: true,
                            ..Default::default()
                        },
                        description: format!("Checkpoint before: {}", step.description),
                    });
                }

                nodes.push(PhaseNode {
                    node_id: format!("{}-tool", sid),
                    phase_type: PhaseType::ToolExecute,
                    config: PhaseConfig {
                        has_side_effects: step.has_side_effects,
                        is_idempotent: !step.has_side_effects,
                        is_cancellable: true,
                        is_retryable: !step.has_side_effects,
                        timeout_seconds: 120,
                        compensation_phase_id: if step.side_effects_reversible {
                            Some(format!("{}-compensate", sid))
                        } else {
                            None
                        },
                        ..Default::default()
                    },
                    description: format!("Tool: {}", step.description),
                });

                // Add compensation node if reversible
                if step.side_effects_reversible {
                    nodes.push(PhaseNode {
                        node_id: format!("{}-compensate", sid),
                        phase_type: PhaseType::Compensate,
                        config: PhaseConfig {
                            is_idempotent: true,
                            is_cancellable: false,
                            ..Default::default()
                        },
                        description: format!("Compensate: {}", step.description),
                    });
                }
            }
            AgentStepType::Generate => {
                nodes.push(PhaseNode {
                    node_id: format!("{}-tokenize", sid),
                    phase_type: PhaseType::Tokenize,
                    config: PhaseConfig {
                        is_idempotent: true,
                        is_cancellable: true,
                        estimated_duration_ms: 10,
                        ..Default::default()
                    },
                    description: "Tokenize input".into(),
                });
                nodes.push(PhaseNode {
                    node_id: format!("{}-generate", sid),
                    phase_type: PhaseType::Decode,
                    config: PhaseConfig {
                        is_cancellable: true,
                        is_retryable: true,
                        estimated_duration_ms: step.estimated_output_tokens as u32 / 50 * 1000,
                        timeout_seconds: 300,
                        allowed_engines: if step.requires_strong_model {
                            vec![] // All engines - filtered at placement time
                        } else {
                            vec!["small-model".into()] // Prefer small models
                        },
                        ..Default::default()
                    },
                    description: format!("Generate: {}", step.description),
                });
            }
            AgentStepType::Verify => {
                nodes.push(PhaseNode {
                    node_id: format!("{}-verify", sid),
                    phase_type: PhaseType::Verify,
                    config: PhaseConfig {
                        is_idempotent: true,
                        is_cancellable: true,
                        is_retryable: true,
                        quality_verifier_phase_id: Some(format!("{}-verify-retry", sid)),
                        ..Default::default()
                    },
                    description: format!("Verify: {}", step.description),
                });
            }
            AgentStepType::Approval => {
                nodes.push(PhaseNode {
                    node_id: format!("{}-approval", sid),
                    phase_type: PhaseType::Validate, // Reused for human approval
                    config: PhaseConfig {
                        is_cancellable: true,
                        is_idempotent: true,
                        timeout_seconds: 0, // Wait indefinitely
                        ..Default::default()
                    },
                    description: format!("Await approval: {}", step.description),
                });
            }
            AgentStepType::Commit => {
                nodes.push(PhaseNode {
                    node_id: format!("{}-commit", sid),
                    phase_type: PhaseType::Commit,
                    config: PhaseConfig {
                        has_side_effects: true,
                        is_idempotent: true, // Commits should be idempotent
                        is_cancellable: false,
                        timeout_seconds: 60,
                        ..Default::default()
                    },
                    description: format!("Commit: {}", step.description),
                });
            }
            AgentStepType::Compensate => {
                nodes.push(PhaseNode {
                    node_id: format!("{}-compensate", sid),
                    phase_type: PhaseType::Compensate,
                    config: PhaseConfig {
                        has_side_effects: true,
                        is_idempotent: true,
                        is_cancellable: false,
                        ..Default::default()
                    },
                    description: format!("Compensate: {}", step.description),
                });
            }
        }

        // Auto-verify after generation steps
        if self.auto_verify && step.requires_verification {
            nodes.push(PhaseNode {
                node_id: format!("{}-auto-verify", sid),
                phase_type: PhaseType::Verify,
                config: PhaseConfig {
                    is_idempotent: true,
                    is_cancellable: true,
                    is_retryable: true,
                    quality_verifier_phase_id: Some(format!("{}-verifier", sid)),
                    ..Default::default()
                },
                description: format!("Auto-verify: {}", step.description),
            });
        }

        nodes
    }

    /// Find groups of nodes that can execute in parallel.
    fn find_parallel_groups(&self, nodes: &[PhaseNode], edges: &[PhaseEdge]) -> Vec<Vec<String>> {
        // Build adjacency: node -> nodes it blocks
        let mut blocks: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut blocked_by: HashMap<&str, Vec<&str>> = HashMap::new();

        for node in nodes {
            blocks.entry(&node.node_id).or_default();
            blocked_by.entry(&node.node_id).or_default();
        }

        for edge in edges {
            blocks
                .entry(&edge.from_node_id)
                .or_default()
                .push(&edge.to_node_id);
            blocked_by
                .entry(&edge.to_node_id)
                .or_default()
                .push(&edge.from_node_id);
        }

        // Simple wave-front: nodes with zero remaining dependencies form a parallel group
        let mut remaining_deps: HashMap<String, usize> = nodes
            .iter()
            .map(|n| {
                let count = blocked_by
                    .get(n.node_id.as_str())
                    .map(|v| v.len())
                    .unwrap_or(0);
                (n.node_id.clone(), count)
            })
            .collect();

        let mut groups = Vec::new();

        while !remaining_deps.is_empty() {
            let ready: Vec<String> = remaining_deps
                .iter()
                .filter(|(_, &count)| count == 0)
                .map(|(id, _)| id.clone())
                .collect();

            if ready.is_empty() {
                break; // Cycle (shouldn't happen after topological sort)
            }

            // Remove ready nodes
            for id in &ready {
                remaining_deps.remove(id);
                // Decrement dependency count for nodes blocked by this one
                if let Some(blocked) = blocks.get(id.as_str()) {
                    for &target in blocked {
                        if let Some(count) = remaining_deps.get_mut(target) {
                            *count = count.saturating_sub(1);
                        }
                    }
                }
            }

            groups.push(ready);
        }

        groups
    }

    /// Compute the critical path (longest chain of dependencies).
    fn compute_critical_path(&self, nodes: &[PhaseNode], edges: &[PhaseEdge]) -> Vec<String> {
        // Build adjacency and compute longest path using DP on DAG
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut in_degree: HashMap<&str, usize> = HashMap::new();

        for node in nodes {
            adj.entry(&node.node_id).or_default();
            in_degree.entry(&node.node_id).or_insert(0);
        }

        for edge in edges {
            adj.entry(&edge.from_node_id)
                .or_default()
                .push(&edge.to_node_id);
            *in_degree.entry(&edge.to_node_id).or_insert(0) += 1;
        }

        // Topological order
        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut order = Vec::new();

        while let Some(node) = queue.pop_front() {
            order.push(node);
            if let Some(neighbors) = adj.get(node) {
                for &n in neighbors {
                    if let Some(deg) = in_degree.get_mut(n) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(n);
                        }
                    }
                }
            }
        }

        // DP: longest path to each node
        let mut longest_to: HashMap<&str, u64> = HashMap::new();
        let mut predecessor: HashMap<&str, Option<&str>> = HashMap::new();
        let mut max_len = 0u64;
        let mut max_node = order.first().copied().unwrap_or("");

        for &node in &order {
            let node_len = nodes
                .iter()
                .find(|n| n.node_id == node)
                .map(|n| n.config.estimated_duration_ms as u64)
                .unwrap_or(1);

            let mut best = node_len;
            let mut best_pred = None;

            // Check all incoming edges
            for edge in edges.iter().filter(|e| e.to_node_id == node) {
                if let Some(&pred_len) = longest_to.get(edge.from_node_id.as_str()) {
                    let candidate = pred_len + node_len;
                    if candidate > best {
                        best = candidate;
                        best_pred = Some(edge.from_node_id.as_str());
                    }
                }
            }

            longest_to.insert(node, best);
            predecessor.insert(node, best_pred);

            if best > max_len {
                max_len = best;
                max_node = node;
            }
        }

        // Reconstruct path backwards
        let mut path = Vec::new();
        let mut current = Some(max_node);
        while let Some(node) = current {
            path.push(node.to_string());
            current = predecessor.get(node).and_then(|&p| p);
        }
        path.reverse();

        path
    }

    /// Detect nodes with high KV cache reuse potential.
    fn detect_cache_reuse(&self, nodes: &[PhaseNode], steps: &[AgentStep]) -> HashSet<String> {
        let mut reuse_nodes = HashSet::new();

        // Steps that use the same model and similar prompts can reuse KV cache
        for i in 0..steps.len() {
            for j in (i + 1)..steps.len() {
                let si = &steps[i];
                let sj = &steps[j];

                // If steps use the same model and similar input length, cache may be reusable
                if si.requires_strong_model == sj.requires_strong_model
                    && (si.estimated_input_tokens as i64 - sj.estimated_input_tokens as i64).abs()
                        < si.estimated_input_tokens as i64 / 2
                {
                    // Mark the second step's generate node as cache-reusable
                    let node_id = format!("{}-generate", sj.id);
                    if nodes.iter().any(|n| n.node_id == node_id) {
                        reuse_nodes.insert(node_id);
                    }
                }
            }
        }

        reuse_nodes
    }

    /// Estimate total resources for the execution graph.
    fn estimate_resources(&self, _nodes: &[PhaseNode], steps: &[AgentStep]) -> ResourceVector {
        let total_tokens: u64 = steps
            .iter()
            .map(|s| s.estimated_input_tokens + s.estimated_output_tokens)
            .sum();

        let model_calls: u64 = steps.iter().map(|s| s.model_calls as u64).sum();
        let kv_bytes: u64 = total_tokens * 1024; // ~1KB per token for KV cache

        ResourceVector {
            kv_cache_bytes: kv_bytes,
            device_memory_bytes: kv_bytes + 2 * 1024 * 1024 * 1024, // KV + 2GB model
            ram_bytes: 4 * 1024 * 1024 * 1024,                      // 4GB baseline
            cpu_time_us: model_calls * 500_000,                     // 500ms per model call
            ..ResourceVector::default()
        }
    }

    /// Estimate the critical path duration.
    fn estimate_duration(&self, critical_path: &[String], nodes: &[PhaseNode]) -> u64 {
        critical_path
            .iter()
            .filter_map(|id| nodes.iter().find(|n| &n.node_id == id))
            .map(|n| n.config.estimated_duration_ms as u64)
            .sum()
    }
}

// -- Compile Error --

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("Missing dependency: step '{step}' depends on non-existent step '{missing_dep}'")]
    MissingDependency { step: String, missing_dep: String },

    #[error("Circular dependency detected at step '{step}'")]
    CircularDependency { step: String },

    #[error("Invalid step type for step '{step}': {reason}")]
    InvalidStep { step: String, reason: String },
}

// -- Critical Path Scheduler --

/// Schedules agent execution with critical-path awareness.
///
/// Prioritizes:
/// 1. Nodes on the critical path
/// 2. Nodes that unlock many downstream tasks
/// 3. Nodes with high KV cache reuse
/// 4. Nodes whose failure has high compensation cost
pub struct CriticalPathScheduler {
    compiler: AgentProgramCompiler,
}

impl Default for CriticalPathScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl CriticalPathScheduler {
    pub fn new() -> Self {
        Self {
            compiler: AgentProgramCompiler::default(),
        }
    }

    /// Schedule an agent program for execution.
    pub fn schedule(
        &self,
        steps: &[AgentStep],
        goal: &str,
    ) -> Result<ScheduledProgram, CompileError> {
        let graph = self.compiler.compile(steps, goal)?;

        // Prioritize nodes: critical path first, then by fan-out, then by cache reuse
        let mut priority_order: Vec<String> = graph.critical_path.clone();

        // Add non-critical-path nodes sorted by fan-out (nodes that unlock many others)
        let fan_out: HashMap<&str, usize> =
            graph.graph.edges.iter().fold(HashMap::new(), |mut acc, e| {
                *acc.entry(&e.from_node_id).or_insert(0) += 1;
                acc
            });

        let mut remaining: Vec<&str> = graph
            .graph
            .nodes
            .iter()
            .map(|n| n.node_id.as_str())
            .filter(|id| !priority_order.contains(&id.to_string()))
            .collect();

        remaining.sort_by_key(|id| {
            let fan = fan_out.get(id).copied().unwrap_or(0);
            let cache = if graph.cache_reuse_nodes.contains(*id) {
                1
            } else {
                0
            };
            // Higher fan-out + cache reuse = higher priority (sort descending)
            std::cmp::Reverse(fan * 10 + cache)
        });

        priority_order.extend(remaining.into_iter().map(|s| s.to_string()));

        Ok(ScheduledProgram {
            graph,
            execution_order: priority_order,
        })
    }
}

/// A scheduled agent program ready for execution.
#[derive(Debug, Clone)]
pub struct ScheduledProgram {
    pub graph: AgentExecutionGraph,
    /// Priority-ordered list of node IDs for execution.
    pub execution_order: Vec<String>,
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str, deps: &[&str], step_type: AgentStepType) -> AgentStep {
        AgentStep {
            id: id.to_string(),
            description: format!("Step {}", id),
            step_type,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            model_calls: 1,
            requires_strong_model: false,
            tools: vec![],
            estimated_input_tokens: 1000,
            estimated_output_tokens: 500,
            has_side_effects: step_type == AgentStepType::ToolCall,
            side_effects_reversible: false,
            required_capabilities: vec![],
            requires_approval: false,
            requires_verification: step_type == AgentStepType::Generate,
            max_retries: 3,
        }
    }

    #[test]
    fn test_topological_sort_linear() {
        let compiler = AgentProgramCompiler::default();
        let steps = vec![
            make_step("a", &[], AgentStepType::Think),
            make_step("b", &["a"], AgentStepType::Generate),
            make_step("c", &["b"], AgentStepType::Verify),
        ];
        let deps = compiler.build_dependency_map(&steps);
        let order = compiler.topological_sort(&steps, &deps).unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_topological_sort_diamond() {
        let compiler = AgentProgramCompiler::default();
        let steps = vec![
            make_step("a", &[], AgentStepType::Think),
            make_step("b", &["a"], AgentStepType::Retrieve),
            make_step("c", &["a"], AgentStepType::ToolCall),
            make_step("d", &["b", "c"], AgentStepType::Generate),
        ];
        let deps = compiler.build_dependency_map(&steps);
        let order = compiler.topological_sort(&steps, &deps).unwrap();
        // a must come first, d must come last
        assert_eq!(order[0], "a");
        assert_eq!(order[3], "d");
        // b and c can be in either order
        assert!(order.contains(&"b".to_string()));
        assert!(order.contains(&"c".to_string()));
    }

    #[test]
    fn test_detect_cycle() {
        let compiler = AgentProgramCompiler::default();
        let steps = vec![
            make_step("a", &["b"], AgentStepType::Think),
            make_step("b", &["a"], AgentStepType::Think),
        ];
        let deps = compiler.build_dependency_map(&steps);
        let result = compiler.topological_sort(&steps, &deps);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cycle"));
    }

    #[test]
    fn test_compile_simple_program() {
        let compiler = AgentProgramCompiler::default();
        let steps = vec![
            make_step("think", &[], AgentStepType::Think),
            make_step("generate", &["think"], AgentStepType::Generate),
            make_step("verify", &["generate"], AgentStepType::Verify),
        ];
        let result = compiler.compile(&steps, "Test program");
        assert!(result.is_ok());
        let graph = result.unwrap();
        assert!(graph.graph.nodes.len() >= 5); // At minimum the phase nodes
        assert!(!graph.critical_path.is_empty());
    }

    #[test]
    fn test_compile_with_tool_call_adds_checkpoint() {
        let compiler = AgentProgramCompiler {
            auto_checkpoint: true,
            ..AgentProgramCompiler::default()
        };

        let steps = vec![
            make_step("think", &[], AgentStepType::Think),
            make_step("tool", &["think"], AgentStepType::ToolCall),
            make_step("generate", &["tool"], AgentStepType::Generate),
        ];
        let result = compiler.compile(&steps, "Test");
        assert!(result.is_ok());
        let graph = result.unwrap();
        // Should have checkpoint nodes
        assert!(!graph.checkpoint_nodes.is_empty());
    }

    #[test]
    fn test_parallel_group_detection() {
        let compiler = AgentProgramCompiler::default();
        let nodes = vec![
            PhaseNode {
                node_id: "a".into(),
                phase_type: PhaseType::Plan,
                config: Default::default(),
                description: "".into(),
            },
            PhaseNode {
                node_id: "b".into(),
                phase_type: PhaseType::Decode,
                config: Default::default(),
                description: "".into(),
            },
            PhaseNode {
                node_id: "c".into(),
                phase_type: PhaseType::Decode,
                config: Default::default(),
                description: "".into(),
            },
            PhaseNode {
                node_id: "d".into(),
                phase_type: PhaseType::Verify,
                config: Default::default(),
                description: "".into(),
            },
        ];
        let edges = vec![
            PhaseEdge {
                from_node_id: "a".into(),
                to_node_id: "b".into(),
                condition: None,
            },
            PhaseEdge {
                from_node_id: "a".into(),
                to_node_id: "c".into(),
                condition: None,
            },
            PhaseEdge {
                from_node_id: "b".into(),
                to_node_id: "d".into(),
                condition: None,
            },
            PhaseEdge {
                from_node_id: "c".into(),
                to_node_id: "d".into(),
                condition: None,
            },
        ];
        let groups = compiler.find_parallel_groups(&nodes, &edges);
        // First group: a (no dependencies)
        assert!(groups[0].contains(&"a".to_string()));
        // Second group: b and c (both depend only on a)
        assert!(groups[1].contains(&"b".to_string()) && groups[1].contains(&"c".to_string()));
    }

    #[test]
    fn test_cache_reuse_detection() {
        let compiler = AgentProgramCompiler::default();
        let steps = vec![
            AgentStep {
                id: "s1".into(),
                description: "".into(),
                step_type: AgentStepType::Generate,
                depends_on: vec![],
                model_calls: 1,
                requires_strong_model: false,
                tools: vec![],
                estimated_input_tokens: 1000,
                estimated_output_tokens: 500,
                has_side_effects: false,
                side_effects_reversible: false,
                required_capabilities: vec![],
                requires_approval: false,
                requires_verification: true,
                max_retries: 3,
            },
            AgentStep {
                id: "s2".into(),
                description: "".into(),
                step_type: AgentStepType::Generate,
                depends_on: vec!["s1".into()],
                model_calls: 1,
                requires_strong_model: false,
                tools: vec![],
                estimated_input_tokens: 1200,
                estimated_output_tokens: 600,
                has_side_effects: false,
                side_effects_reversible: false,
                required_capabilities: vec![],
                requires_approval: false,
                requires_verification: true,
                max_retries: 3,
            },
        ];
        let nodes = vec![
            PhaseNode {
                node_id: "s1-generate".into(),
                phase_type: PhaseType::Decode,
                config: Default::default(),
                description: "".into(),
            },
            PhaseNode {
                node_id: "s2-generate".into(),
                phase_type: PhaseType::Decode,
                config: Default::default(),
                description: "".into(),
            },
        ];
        let cache_nodes = compiler.detect_cache_reuse(&nodes, &steps);
        // s2 should be detected as cache-reusable (similar input length, same model class)
        assert!(cache_nodes.contains("s2-generate"));
    }
}
