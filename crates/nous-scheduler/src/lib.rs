//! nous-scheduler - Multi-level scheduling for the APEIR AI Kernel.
//!
//! Execution scopes coordinated by one scheduler authority:
//! 1. Program Scheduler - AgentProcess, Critical Path, State-Aware
//! 2. Global Placement Scheduler - Node, Device, Engine, Model Revision
//! 3. Workflow Scheduler - DAG, Agent Program, Critical Path
//! 4. Phase Scheduler - Encoder/Prefill/Decode co-location vs disaggregation
//!
//! All schedulers implement the SchedulerPolicy trait for pluggability.

pub mod core;
pub mod node_transport;
pub mod phase;
pub mod placement;
pub mod policy;
pub mod program;
pub mod workflow;

#[cfg(test)]
mod integration_tests;

pub use core::{
    CandidateRejection, DecisionTrace, SchedulerCore, SchedulerCoreConfig, SchedulerPolicySet,
};
pub use node_transport::{NodeAdvertisement, NodeConnectionState};
pub use placement::{
    DevicePlacementInfo, EnginePlacementInfo, ExpectedCompletionTime, KVCachePlacement, NodeInfo,
    PlacementDecision, PlacementScheduler,
};
pub use policy::{
    CacheAwarePolicy, ContextualBanditPolicy, CriticalPathPolicy, DeadlineAwarePolicy,
    FallbackStrategy, FifoPolicy, MultiplicativePolicy, ParetoPolicy, PolicyRegistry,
    PriorityPolicy, ScheduleDecision, SchedulerCandidate, SchedulerPolicy, SchedulingWeights,
    WeightedSumPolicy,
};
pub use program::{
    build_candidates, ProcessCandidate, ProcessSchedulingInfo, ProcessSchedulingStatus,
    ProgramCriticalPathPolicy, ProgramFifoPolicy, ProgramPolicyRegistry, ProgramPriorityPolicy,
    ProgramSRTFPolicy, ProgramScheduleDecision, ProgramScheduler, ProgramSchedulingPolicy,
    StateAwarePolicy,
};
