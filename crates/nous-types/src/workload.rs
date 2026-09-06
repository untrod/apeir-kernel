//! Workload - the central kernel object representing a unit of AI work.
//!
//! A Workload is the unified representation of any AI task:
//! chat, completion, embedding, agent program, workflow, tool execution, etc.
//!
//! Every workload:
//! 1. Has an immutable WorkloadSpec (what to do)
//! 2. Has a mutable WorkloadStatus (what's happening)
//! 3. Progresses through a 17-state lifecycle
//! 4. Contains an ExecutionGraph describing the phases to execute

use crate::meta::{Condition, ObjectMeta};
use crate::resource::{PreemptionPolicy, PriorityClass, ResourceLease, ResourceVector};
use crate::traits::KernelObject;
use serde::{Deserialize, Serialize};

// Workload

/// A workload is the central unit of work in the APEIR Kernel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workload {
    pub meta: ObjectMeta,
    pub spec: WorkloadSpec,
    pub status: WorkloadStatus,
}

impl KernelObject for Workload {
    type Spec = WorkloadSpec;
    type Status = WorkloadStatus;

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

// WorkloadSpec (immutable after creation)

/// The desired state of a workload - set by the client, immutable after creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadSpec {
    /// Client-assigned workload ID (UUID v7).
    #[serde(default)]
    pub workload_id: String,

    /// Idempotency key for safe retry.
    #[serde(default)]
    pub idempotency_key: String,

    /// Schema version for this WorkloadSpec.
    #[serde(default)]
    pub schema_version: u32,

    /// Authenticated principal.
    #[serde(default)]
    pub principal_id: String,

    /// Target namespace.
    #[serde(default)]
    pub namespace: String,

    /// Human-readable goal (for debugging and audit).
    #[serde(default)]
    pub goal: String,

    /// Type of workload.
    #[serde(default)]
    pub workload_type: WorkloadType,

    /// The execution graph - what phases to run.
    #[serde(default)]
    pub execution_graph: ExecutionGraph,

    /// Model requirements and constraints.
    #[serde(default)]
    pub model_requirements: ModelRequirements,

    /// Required capabilities.
    #[serde(default)]
    pub capability_requirements: CapabilityRequirements,

    /// Device requirements.
    #[serde(default)]
    pub device_requirements: DeviceRequirements,

    /// Quality thresholds.
    #[serde(default)]
    pub quality_requirements: QualityRequirements,

    /// Security constraints.
    #[serde(default)]
    pub security_requirements: SecurityRequirements,

    /// Resource requirements.
    #[serde(default)]
    pub resource_requirements: ResourceRequirements,

    /// Coarse scheduling service class. Hard constraints are evaluated before
    /// this class influences ordering or completion-time estimates.
    #[serde(default)]
    pub scheduling_class: SchedulingClass,

    /// Latency service-level objectives.
    #[serde(default)]
    pub latency_slo: LatencySLO,

    /// Absolute deadline (Unix microseconds).
    #[serde(default)]
    pub deadline_us: i64,

    /// Energy budget.
    #[serde(default)]
    pub energy_budget: EnergyBudget,

    /// Cost budget.
    #[serde(default)]
    pub cost_budget: CostBudget,

    /// When and how to checkpoint.
    #[serde(default)]
    pub checkpoint_policy: CheckpointPolicy,

    /// How cancellation is handled.
    #[serde(default)]
    pub cancellation_policy: CancellationPolicy,

    /// How failures are retried.
    #[serde(default)]
    pub retry_policy: RetryPolicy,

    /// Fallback strategy on degradation.
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,

    /// Expected output format and constraints.
    #[serde(default)]
    pub output_contract: OutputContract,
}

impl WorkloadSpec {
    /// Create a minimal WorkloadSpec for a simple chat request.
    pub fn new_chat(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            workload_type: WorkloadType::Chat,
            ..Default::default()
        }
    }

    /// Create a minimal WorkloadSpec for an agent program.
    pub fn new_agent(goal: impl Into<String>, graph: ExecutionGraph) -> Self {
        Self {
            goal: goal.into(),
            workload_type: WorkloadType::AgentProgram,
            execution_graph: graph,
            ..Default::default()
        }
    }
}

impl Default for WorkloadSpec {
    fn default() -> Self {
        Self {
            workload_id: String::new(),
            idempotency_key: String::new(),
            schema_version: 1,
            principal_id: String::new(),
            namespace: String::new(),
            goal: String::new(),
            workload_type: WorkloadType::Unspecified,
            execution_graph: ExecutionGraph::default(),
            model_requirements: ModelRequirements::default(),
            capability_requirements: CapabilityRequirements::default(),
            device_requirements: DeviceRequirements::default(),
            quality_requirements: QualityRequirements::default(),
            security_requirements: SecurityRequirements::default(),
            resource_requirements: ResourceRequirements::default(),
            scheduling_class: SchedulingClass::default(),
            latency_slo: LatencySLO::default(),
            deadline_us: 0,
            energy_budget: EnergyBudget::default(),
            cost_budget: CostBudget::default(),
            checkpoint_policy: CheckpointPolicy::default(),
            cancellation_policy: CancellationPolicy::default(),
            retry_policy: RetryPolicy::default(),
            fallback_policy: FallbackPolicy::default(),
            output_contract: OutputContract::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SchedulingClass {
    System,
    Deadline,
    #[default]
    Interactive,
    Batch,
    Background,
    Maintenance,
}

// WorkloadStatus (kernel-observed, mutable)

/// The observed state of a workload - updated only by the kernel.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkloadStatus {
    /// Current phase in the workload lifecycle.
    #[serde(default)]
    pub phase: WorkloadPhase,

    /// Index into the ExecutionGraph's phase list.
    #[serde(default)]
    pub current_phase_index: u32,

    /// Node assigned to execute this workload.
    #[serde(default)]
    pub node_id: String,

    /// Engine assigned to execute this workload.
    #[serde(default)]
    pub engine_id: String,

    /// Specific model revision assigned.
    #[serde(default)]
    pub model_revision: String,

    /// Active resource lease.
    pub lease: Option<ResourceLease>,

    /// Execution history (runs).
    #[serde(default)]
    pub runs: Vec<Run>,

    /// Latest checkpoint reference.
    pub latest_checkpoint: Option<crate::checkpoint::CheckpointRef>,

    /// Active conditions.
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// When execution started.
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,

    /// When execution completed (success or failure).
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,

    /// Performance metrics across all runs.
    pub metrics: Option<PerformanceMetrics>,

    /// Error information if failed.
    pub error: Option<ErrorInfo>,
}

// Workload Type

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WorkloadType {
    #[default]
    Unspecified = 0,
    Chat = 1,
    Completion = 2,
    StructuredOutput = 3,
    Embedding = 4,
    Rerank = 5,
    Vision = 6,
    Speech = 7,
    ModelInference = 8,
    AgentProgram = 9,
    Workflow = 10,
    ToolExecution = 11,
    Retrieval = 12,
    Evaluation = 13,
    Batch = 14,
    DeviceOperation = 15,
    ModelImport = 16,
    ModelCompile = 17,
    Benchmark = 18,
    KernelCommand = 19,
}

impl std::fmt::Display for WorkloadType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkloadType::Unspecified => write!(f, "Unspecified"),
            WorkloadType::Chat => write!(f, "Chat"),
            WorkloadType::Completion => write!(f, "Completion"),
            WorkloadType::StructuredOutput => write!(f, "StructuredOutput"),
            WorkloadType::Embedding => write!(f, "Embedding"),
            WorkloadType::Rerank => write!(f, "Rerank"),
            WorkloadType::Vision => write!(f, "Vision"),
            WorkloadType::Speech => write!(f, "Speech"),
            WorkloadType::ModelInference => write!(f, "ModelInference"),
            WorkloadType::AgentProgram => write!(f, "AgentProgram"),
            WorkloadType::Workflow => write!(f, "Workflow"),
            WorkloadType::ToolExecution => write!(f, "ToolExecution"),
            WorkloadType::Retrieval => write!(f, "Retrieval"),
            WorkloadType::Evaluation => write!(f, "Evaluation"),
            WorkloadType::Batch => write!(f, "Batch"),
            WorkloadType::DeviceOperation => write!(f, "DeviceOperation"),
            WorkloadType::ModelImport => write!(f, "ModelImport"),
            WorkloadType::ModelCompile => write!(f, "ModelCompile"),
            WorkloadType::Benchmark => write!(f, "Benchmark"),
            WorkloadType::KernelCommand => write!(f, "KernelCommand"),
        }
    }
}

// Workload Lifecycle (17 phases)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WorkloadPhase {
    #[default]
    Created = 0,
    Validating = 1,
    Validated = 2,
    Rejected = 3,
    Admitted = 4,
    Placed = 5,
    Preparing = 6,
    Running = 7,
    Quiescing = 8,
    Checkpointing = 9,
    Checkpointed = 10,
    Recovering = 11,
    Succeeded = 12,
    Failed = 13,
    Cancelled = 14,
    Lost = 15,
    Quarantined = 16,
}

impl WorkloadPhase {
    /// Check if this is a terminal phase (no further transitions).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WorkloadPhase::Rejected
                | WorkloadPhase::Succeeded
                | WorkloadPhase::Failed
                | WorkloadPhase::Cancelled
                | WorkloadPhase::Lost
                | WorkloadPhase::Quarantined
        )
    }

    /// Check if this is an active phase (workload is consuming resources).
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            WorkloadPhase::Preparing
                | WorkloadPhase::Running
                | WorkloadPhase::Quiescing
                | WorkloadPhase::Checkpointing
                | WorkloadPhase::Recovering
        )
    }

    /// Validate a transition from the current phase to a target phase.
    pub fn can_transition_to(&self, target: WorkloadPhase) -> bool {
        use WorkloadPhase::*;
        if *self == target {
            return true;
        }
        matches!(
            (*self, target),
            // Created can go to Validating or Cancelled
            (Created, Validating) | (Created, Cancelled) |
            // Validating can go to Validated or Rejected
            (Validating, Validated) | (Validating, Rejected) |
            // Validated can go to Admitted or Rejected
            (Validated, Admitted) | (Validated, Rejected) |
            // Admitted can go to Placed or Cancelled
            (Admitted, Placed) | (Admitted, Cancelled) |
            // Placed can go to Preparing or Cancelled
            (Placed, Preparing) | (Placed, Cancelled) |
            // Preparing can go to Running, Failed, or Cancelled
            (Preparing, Running) | (Preparing, Failed) | (Preparing, Cancelled) |
            // Running can go to Quiescing, Checkpointing, Succeeded, Failed, Cancelled, Lost
            (Running, Quiescing) | (Running, Checkpointing) | (Running, Succeeded)
                | (Running, Failed) | (Running, Cancelled) | (Running, Lost) |
            // Quiescing can go to Checkpointing, Succeeded, Failed, Cancelled
            (Quiescing, Checkpointing) | (Quiescing, Succeeded) | (Quiescing, Failed) | (Quiescing, Cancelled) |
            // Checkpointing can go to Checkpointed or Failed
            (Checkpointing, Checkpointed) | (Checkpointing, Failed) |
            // Checkpointed can go to Running, Recovering, or Cancelled
            (Checkpointed, Running) | (Checkpointed, Recovering) | (Checkpointed, Cancelled) |
            // Recovering can go to Running, Failed, or Cancelled
            (Recovering, Running) | (Recovering, Failed) | (Recovering, Cancelled) |
            // Succeeded: terminal
            // Failed can go to Recovering or Quarantined
            (Failed, Recovering) | (Failed, Quarantined) |
            // Cancelled: terminal
            // Lost can go to Recovering
            (Lost, Recovering) // Quarantined: terminal
        )
    }
}

// Execution Graph

/// A directed graph of phases to execute.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionGraph {
    pub nodes: Vec<PhaseNode>,
    pub edges: Vec<PhaseEdge>,
    #[serde(default)]
    pub entry_node_id: String,
}

/// A single node (phase) in the execution graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseNode {
    pub node_id: String,
    pub phase_type: PhaseType,
    #[serde(default)]
    pub config: PhaseConfig,
    #[serde(default)]
    pub description: String,
}

/// Types of phases in an execution graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PhaseType {
    #[default]
    Unspecified = 0,
    Receive = 1,
    Validate = 2,
    Authenticate = 3,
    ResolveContext = 4,
    Retrieve = 5,
    Plan = 6,
    Admit = 7,
    Place = 8,
    Prepare = 9,
    Tokenize = 10,
    Encode = 11,
    Prefill = 12,
    Decode = 13,
    ToolExecute = 14,
    Observe = 15,
    Verify = 16,
    Replan = 17,
    Finalize = 18,
    Commit = 19,
    Compensate = 20,
    Checkpoint = 21,
}

/// An edge between two phases, with an optional condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseEdge {
    pub from_node_id: String,
    pub to_node_id: String,
    #[serde(default)]
    pub condition: Option<EdgeCondition>,
}

/// Condition for traversing an edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdgeCondition {
    Always,
    OnSuccess,
    OnFailure,
    OnOutput { field_path: String, pattern: String },
    OnQuality { threshold: f64 },
    OnApproval { approver_role: String },
    OnRetryExhausted { max_retries: u32 },
}

/// Configuration for a single phase node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhaseConfig {
    pub input_artifacts: Vec<String>,
    pub output_artifacts: Vec<String>,
    pub allowed_devices: Vec<String>,
    pub allowed_engines: Vec<String>,
    pub estimated_resources: Option<ResourceVector>,
    pub is_idempotent: bool,
    pub is_cancellable: bool,
    pub is_retryable: bool,
    pub is_migratable: bool,
    pub is_checkpointable: bool,
    /// Explicit opt-in for speculative execution of an otherwise effectful phase.
    #[serde(default)]
    pub is_speculatable: bool,
    pub has_side_effects: bool,
    pub compensation_phase_id: Option<String>,
    pub timeout_seconds: u32,
    pub estimated_duration_ms: u32,
    pub quality_verifier_phase_id: Option<String>,
}

// Run tracking

/// A single execution run within a workload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub workload_id: String,
    pub run_number: u32,
    pub phase: RunPhase,
    pub phases: Vec<PhaseExecution>,
    pub consumed: ResourceVector,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub outcome: RunOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RunPhase {
    #[default]
    Created = 0,
    Preparing = 1,
    Executing = 2,
    Verifying = 3,
    Finalizing = 4,
    Completed = 5,
    Failed = 6,
    Cancelled = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RunOutcome {
    #[default]
    Unknown = 0,
    Success = 1,
    PartialSuccess = 2,
    Failure = 3,
    Cancelled = 4,
    TimedOut = 5,
}

/// Execution record for a single phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseExecution {
    pub phase_node_id: String,
    pub phase_type: PhaseType,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: PhaseExecutionStatus,
    pub engine_id: String,
    pub device_id: String,
    pub metrics: Option<PerformanceMetrics>,
    pub error: Option<ErrorInfo>,
    pub output_artifact_ref: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PhaseExecutionStatus {
    #[default]
    Pending = 0,
    Running = 1,
    Completed = 2,
    Failed = 3,
    Skipped = 4,
    TimedOut = 5,
}

// Requirements

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRequirements {
    pub allowed_model_families: Vec<String>,
    pub allowed_architectures: Vec<String>,
    pub excluded_models: Vec<String>,
    pub preferred_model: String,
    pub max_tokens_per_request: u64,
    pub min_context_length: u64,
    pub max_context_length: u64,
    pub min_quality_score: f64,
    pub thinking_required: bool,
    pub min_thinking_budget: u32,
    pub input_modalities: Vec<Modality>,
    pub output_modalities: Vec<Modality>,
    pub quantization: QuantizationTolerance,
    pub max_acceptable_perplexity_increase: f64,
    pub routing: RoutingPreference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Modality {
    #[default]
    Unspecified = 0,
    Text = 1,
    Image = 2,
    Audio = 3,
    Video = 4,
    Code = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum QuantizationTolerance {
    #[default]
    FullPrecisionOnly = 0,
    Int8Ok = 1,
    Int4Ok = 2,
    Any = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RoutingPreference {
    #[default]
    Any = 0,
    LocalOnly = 1,
    LocalPreferred = 2,
    LowestLatency = 3,
    LowestCost = 4,
    HighestQuality = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DeviceType {
    #[default]
    Unspecified = 0,
    Cpu = 1,
    Cuda = 2,
    Rocm = 3,
    Vulkan = 4,
    OpenVino = 5,
    Metal = 6,
    Sycl = 7,
    Cann = 8,
    Qnn = 9,
    Dsp = 10,
    Fpga = 11,
    Npu = 12,
    Remote = 13,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityRequirements {
    pub required_capabilities: Vec<String>,
    pub forbidden_capabilities: Vec<String>,
    pub minimum_capability_level: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceRequirements {
    pub allowed_device_types: Vec<DeviceType>,
    pub min_vram_bytes: u64,
    pub require_unified_memory: bool,
    pub required_extensions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityRequirements {
    pub minimum_quality: f64,
    pub uncertainty_limit: f64,
    pub verification_required: bool,
    pub verifier_model: String,
    pub evidence_required: bool,
    pub human_review_required: bool,
    pub escalation: Option<QualityEscalation>,
    pub required_benchmarks: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityEscalation {
    pub enabled: bool,
    pub max_retries: u32,
    pub fallback_model: String,
    pub escalation_verifier: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityRequirements {
    pub data_classification: DataClassification,
    pub allow_network_access: bool,
    pub allowed_hosts: Vec<String>,
    pub allow_file_access: bool,
    pub allowed_paths: Vec<String>,
    pub allow_side_effects: bool,
    pub reversible_side_effects_only: bool,
    pub require_approval: bool,
    pub isolation_profile: String,
    pub audit_all_output: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DataClassification {
    #[default]
    Public = 0,
    Internal = 1,
    Confidential = 2,
    Restricted = 3,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub minimum: ResourceVector,
    pub maximum: ResourceVector,
    pub preferred: ResourceVector,
    pub priority: PriorityClass,
    pub preemption: PreemptionPolicy,
}

// Budgets, SLOs, Policies

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencySLO {
    pub max_ttft_ms: u64,
    pub max_tpot_ms: u64,
    pub max_total_ms: u64,
    pub max_queue_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnergyBudget {
    pub max_joules: u64,
    pub prefer_efficiency: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBudget {
    pub max_microcents: u64,
    pub currency: String,
    pub cost_model: CostModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CostModel {
    #[default]
    PerToken = 0,
    PerRequest = 1,
    PerSecond = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointPolicy {
    pub enabled: bool,
    pub strategy: CheckpointStrategy,
    pub min_interval_seconds: u32,
    pub max_checkpoints: u32,
    pub retention_seconds: u32,
}

impl Default for CheckpointPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            strategy: CheckpointStrategy::None,
            min_interval_seconds: 60,
            max_checkpoints: 5,
            retention_seconds: 3600,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CheckpointStrategy {
    #[default]
    None = 0,
    BeforeRiskyPhase = 1,
    Periodic = 2,
    AfterEveryPhase = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancellationPolicy {
    pub cancellable: bool,
    pub graceful: bool,
    pub grace_period_seconds: u32,
    pub uncancellable_phases: Vec<String>,
}

impl Default for CancellationPolicy {
    fn default() -> Self {
        Self {
            cancellable: true,
            graceful: true,
            grace_period_seconds: 30,
            uncancellable_phases: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_backoff_ms: u32,
    pub max_backoff_ms: u32,
    pub backoff_multiplier: f64,
    pub retryable_errors: Vec<String>,
    pub retry_on_timeout: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 100,
            max_backoff_ms: 10000,
            backoff_multiplier: 2.0,
            retryable_errors: vec![],
            retry_on_timeout: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FallbackPolicy {
    pub enabled: bool,
    pub steps: Vec<FallbackStep>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FallbackStep {
    pub condition: String,
    pub action: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContract {
    pub format: OutputFormat,
    pub json_schema: String,
    pub max_output_tokens: u32,
    pub include_reasoning: bool,
    pub include_evidence: bool,
    pub include_tool_traces: bool,
    pub include_metrics: bool,
    pub redact_patterns: Vec<String>,
}

impl Default for OutputContract {
    fn default() -> Self {
        Self {
            format: OutputFormat::Text,
            json_schema: String::new(),
            max_output_tokens: 4096,
            include_reasoning: false,
            include_evidence: false,
            include_tool_traces: false,
            include_metrics: false,
            redact_patterns: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputFormat {
    #[default]
    Text = 0,
    Json = 1,
    JsonSchema = 2,
    Stream = 3,
}

// Observability

/// Performance metrics for a workload or phase execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub queue_time_us: u64,
    pub admission_time_us: u64,
    pub ttft_us: u64,
    pub tpot_us: u64,
    pub total_time_us: u64,
    pub tokens_per_second: f64,
    pub prefill_tokens: u64,
    pub decode_tokens: u64,
    pub total_tokens: u64,
    pub peak_ram_bytes: u64,
    pub peak_vram_bytes: u64,
    pub kv_cache_used_bytes: u64,
    pub vram_fragmentation: f64,
    pub kv_hit_rate: f64,
    pub prefix_hit_rate: f64,
    pub cache_transfer_bytes: u64,
    pub quality_score: f64,
    pub fallback_rate: f64,
    pub verification_attempts: u32,
    pub energy_joules: f64,
    pub thermal_throttled: bool,
    pub retry_count: u32,
    pub tool_failures: u32,
    pub failure_reason: String,
}

/// Error information attached to a failed workload or phase.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub error_code: String,
    pub message: String,
    pub failed_phase: String,
    pub cause: String,
    pub stack: Vec<String>,
    pub retryable: bool,
}
