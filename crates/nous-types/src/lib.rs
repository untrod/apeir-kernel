//! nous-types - Core type system for the APEIR AI Kernel
//!
//! This crate defines all kernel object types, enums, and identifiers
//! with zero external dependencies beyond serde and uuid.
//!
//! Design principles:
//! 1. Spec/Status split - every object separates desired state from observed state
//! 2. Generation-based CAS - every mutable object has a monotonically increasing generation
//! 3. UUID v7 identity - time-ordered, globally unique identifiers
//! 4. Conditions-based health - status is reported through typed conditions
//! 5. No I/O - types are pure data; persistence lives in nous-state

pub mod audit;
pub mod checkpoint;
pub mod contracts;
pub mod control_api;
pub mod device;
pub mod effect;
pub mod engine;
pub mod error;
pub mod event;
pub mod meta;
pub mod model;
pub mod node;
pub mod open_runtime;
pub mod principal;
pub mod profile;
pub mod resource;
pub mod runtime_api;
pub mod traits;
pub mod workload;

// Re-export commonly used types
pub use audit::AuditRecord;
pub use checkpoint::{Checkpoint, CheckpointRef};
pub use contracts::{ContractDescriptor, ContractId, FOUNDATION_CONTRACTS};
pub use control_api::{
    AssetKind, AssetSelector, ControlAsset, ListAssetsRequest, PutAssetRequest,
    CONTROL_API_SCHEMA_VERSION,
};
pub use device::{Device, DevicePhase, DeviceSpec, DeviceStatus, TopologyLink};
pub use effect::{
    EffectContract, EffectExpectation, EffectVerification, EvidenceRef, ObservedEffect,
    RealityIdentity, VerificationMode, VerificationOutcome, EFFECT_CONTRACT_SCHEMA_VERSION,
};
pub use engine::{Engine, EnginePhase, EngineSpec, EngineStatus};
pub use error::{ErrorCode, NousError, RetryHint, RetryStrategy};
pub use event::Event;
pub use meta::{AuditMetadata, Condition, ConditionStatus, HealthStatus, ObjectMeta};
pub use model::{
    BenchmarkProfile, LoadOptions, ModelLifecyclePhase, ModelPackage, ModelPackageSpec,
    ModelPackageStatus,
};
pub use node::{Node, NodePhase, NodeSpec, NodeStatus};
pub use open_runtime::{
    ArtifactContract, CapabilityContract, CapabilityGraph, ContextContract, ContractValidation,
    DeploymentSpec, EvaluationSpec, EventContract, ExtensionAdmissionDecision,
    ExtensionAdmissionRequest, ExtensionAuthorizationRequest, ExtensionCapabilityRequest,
    ExtensionExecutionPermit, ExtensionExecutionRequest, ExtensionRevocationRequest,
    IdentityContract, ModelBackend, ModelKind, ModelSpec, NodeContract, PackManifest,
    PolicyContract, ProviderContract, ProviderManifest, ResourceContract, ResourceGraph,
    RuntimeContractError, TaskContract, WorkflowContract,
};
pub use principal::{CapabilityGrant, Principal, PrincipalSpec, PrincipalStatus, PrincipalType};
pub use profile::{RuntimeProfile, RuntimeProfileCapabilities};
pub use resource::{
    PreemptionPolicy, PriorityClass, ResourceDomain, ResourceLease, ResourceLimits, ResourceVector,
};
pub use runtime_api::{
    DeliverySemantics, ModelInvocationInput, ModelInvocationOutput, OperationRequest,
    ProviderProbeRequest, ProviderRuntimeClass, RecoveryStrategy, SemanticExecutionSnapshot,
};
pub use traits::KernelObject;
pub use workload::{
    CancellationPolicy, CapabilityRequirements, CheckpointPolicy, CheckpointStrategy, CostBudget,
    CostModel, DataClassification, DeviceRequirements, DeviceType, EdgeCondition, EnergyBudget,
    ExecutionGraph, FallbackPolicy, FallbackStep, LatencySLO, Modality, ModelRequirements,
    OutputContract, OutputFormat, PhaseConfig, PhaseEdge, PhaseExecution, PhaseExecutionStatus,
    PhaseNode, PhaseType, QualityEscalation, QualityRequirements, QuantizationTolerance,
    ResourceRequirements, RetryPolicy, RoutingPreference, Run, RunOutcome, RunPhase,
    SchedulingClass, SecurityRequirements, Workload, WorkloadPhase, WorkloadSpec, WorkloadStatus,
    WorkloadType,
};

/// Kernel version string
pub const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// NKI v1 schema version
pub const NKI_SCHEMA_VERSION: u32 = 1;
