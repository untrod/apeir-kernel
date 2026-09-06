//! NKI v1 method names - the complete set of kernel operations.

/// All NKI v1 methods.
pub struct NKIMethods;

impl NKIMethods {
    // Workload management
    pub const SUBMIT_WORKLOAD: &'static str = "SubmitWorkload";
    pub const SUBMIT_CONTINUITY_PLAN: &'static str = "SubmitContinuityPlan";
    pub const GET_WORKLOAD: &'static str = "GetWorkload";
    pub const LIST_WORKLOADS: &'static str = "ListWorkloads";
    pub const CANCEL_WORKLOAD: &'static str = "CancelWorkload";
    pub const PAUSE_WORKLOAD: &'static str = "PauseWorkload";
    pub const RESUME_WORKLOAD: &'static str = "ResumeWorkload";

    // Admission & resources
    pub const ADMIT_WORKLOAD: &'static str = "AdmitWorkload";
    pub const ADMIT_EXTENSION: &'static str = "AdmitExtension";
    pub const AUTHORIZE_EXTENSION: &'static str = "AuthorizeExtension";
    pub const AUTHORIZE_EXTENSION_EXECUTION: &'static str = "AuthorizeExtensionExecution";
    pub const REVOKE_EXTENSION: &'static str = "RevokeExtension";
    pub const RESERVE_RESOURCES: &'static str = "ReserveResources";
    pub const RELEASE_RESOURCES: &'static str = "ReleaseResources";
    pub const RENEW_LEASE: &'static str = "RenewLease";

    // Model management
    pub const REGISTER_MODEL: &'static str = "RegisterModel";
    pub const VALIDATE_MODEL: &'static str = "ValidateModel";
    pub const LOAD_MODEL: &'static str = "LoadModel";
    pub const UNLOAD_MODEL: &'static str = "UnloadModel";

    // Runtime Control Plane developer assets
    pub const PUT_CONTROL_ASSET: &'static str = "PutControlAsset";
    pub const GET_CONTROL_ASSET: &'static str = "GetControlAsset";
    pub const LIST_CONTROL_ASSETS: &'static str = "ListControlAssets";
    pub const DELETE_CONTROL_ASSET: &'static str = "DeleteControlAsset";

    // Engine management
    pub const REGISTER_ENGINE: &'static str = "RegisterEngine";
    pub const PROBE_ENGINE: &'static str = "ProbeEngine";
    pub const LIST_ENGINES: &'static str = "ListEngines";

    // Device management
    pub const REGISTER_DEVICE: &'static str = "RegisterDevice";
    pub const PROBE_DEVICE: &'static str = "ProbeDevice";
    pub const LIST_DEVICES: &'static str = "ListDevices";

    // State & recovery
    pub const CREATE_CHECKPOINT: &'static str = "CreateCheckpoint";
    pub const RESTORE_CHECKPOINT: &'static str = "RestoreCheckpoint";

    // Observability
    pub const WATCH_EVENTS: &'static str = "WatchEvents";
    pub const GET_TRACE: &'static str = "GetTrace";
    pub const GET_METRICS: &'static str = "GetMetrics";
    pub const EXPLAIN_EXECUTION: &'static str = "ExplainExecution";

    // Health
    pub const HEALTH_CHECK: &'static str = "HealthCheck";

    /// Check if a method name is valid.
    pub fn is_valid(method: &str) -> bool {
        matches!(
            method,
            Self::SUBMIT_WORKLOAD
                | Self::SUBMIT_CONTINUITY_PLAN
                | Self::GET_WORKLOAD
                | Self::LIST_WORKLOADS
                | Self::CANCEL_WORKLOAD
                | Self::PAUSE_WORKLOAD
                | Self::RESUME_WORKLOAD
                | Self::ADMIT_WORKLOAD
                | Self::ADMIT_EXTENSION
                | Self::AUTHORIZE_EXTENSION
                | Self::AUTHORIZE_EXTENSION_EXECUTION
                | Self::REVOKE_EXTENSION
                | Self::RESERVE_RESOURCES
                | Self::RELEASE_RESOURCES
                | Self::REGISTER_MODEL
                | Self::VALIDATE_MODEL
                | Self::LOAD_MODEL
                | Self::UNLOAD_MODEL
                | Self::PUT_CONTROL_ASSET
                | Self::GET_CONTROL_ASSET
                | Self::LIST_CONTROL_ASSETS
                | Self::DELETE_CONTROL_ASSET
                | Self::REGISTER_ENGINE
                | Self::PROBE_ENGINE
                | Self::LIST_ENGINES
                | Self::REGISTER_DEVICE
                | Self::PROBE_DEVICE
                | Self::LIST_DEVICES
                | Self::CREATE_CHECKPOINT
                | Self::RESTORE_CHECKPOINT
                | Self::WATCH_EVENTS
                | Self::GET_TRACE
                | Self::GET_METRICS
                | Self::EXPLAIN_EXECUTION
                | Self::HEALTH_CHECK
                | Self::RENEW_LEASE
        )
    }
}
