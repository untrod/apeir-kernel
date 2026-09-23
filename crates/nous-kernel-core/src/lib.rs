//! Durable execution core and independent provider-process protocol.

mod cancellation;
mod continuity;
mod coordinator;
mod fault;
mod provider;

pub use cancellation::CancellationToken;
pub use continuity::{
    ContinuityExecution, ContinuityPlan, ContinuityStep, FailoverCandidate, FailoverRecord,
};
pub use coordinator::{KernelRuntime, RuntimeExecution};
pub use fault::FaultPoint;
pub use nous_types::{
    DeliverySemantics, OperationRequest, ProviderProbeRequest, ProviderRuntimeClass,
    SemanticExecutionSnapshot,
};
pub use provider::{
    assess_compatibility, CapabilitySupport, CompatibilityReport, ModelCapability,
    ModelCapabilityManifest, ModelCompatibility, ModelCompatibilityRequirements, ModelInstance,
    ModelManifest, ModelPrivacy, ModelProvider, ProviderProbeReport,
};

use async_trait::async_trait;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use nous_state::journal::{EntryType, Journal, JournalEntry};
use nous_types::{
    EffectContract, EffectVerification, EvidenceRef, ObservedEffect, RealityIdentity,
    RemoteExecutionReceipt, TargetBinding, VerificationMode, VerificationOutcome,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KernelState {
    Starting,
    Recovering,
    Ready,
    Degraded,
    Failed,
    ShuttingDown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationReceipt {
    pub operation_id: String,
    pub input_digest: String,
    pub output_digest: String,
    pub snapshot_digest: String,
    pub provider_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_identity: Option<String>,
    /// Signed remote fact admitted by Kernel policy. Absence denotes a local provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_execution: Option<RemoteExecutionReceipt>,
    pub result: String,
    pub completed_at_us: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCommand {
    Health,
    Probe { request: ProviderProbeRequest },
    Execute { request: Box<OperationRequest> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub ok: bool,
    pub receipt: Option<OperationReceipt>,
    #[serde(default)]
    pub probe: Option<ProviderProbeReport>,
    pub error_code: String,
    pub error_message: String,
}

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("journal error: {0}")]
    Journal(String),
    #[error("provider process error: {0}")]
    Provider(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("unsafe recovery for operation {0}")]
    UnsafeRecovery(String),
    #[error("manual recovery required for operation {0}")]
    RecoveryRequired(String),
    #[error("operation cancelled: {0}")]
    Cancelled(String),
    #[error("operation deadline exceeded: {0}")]
    DeadlineExceeded(String),
    #[error("admission denied: {0}")]
    Admission(String),
    #[error("scheduling failed: {0}")]
    Scheduling(String),
    #[error("resource error: {0}")]
    Resource(String),
    #[error("kernel is shutting down")]
    ShuttingDown,
    #[error("reality verification failed: {0}")]
    RealityVerification(String),
}

impl KernelError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Journal(_) => "JOURNAL_ERROR",
            Self::Provider(_) => "PROVIDER_ERROR",
            Self::Serialization(_) => "SERIALIZATION_ERROR",
            Self::UnsafeRecovery(_) => "UNSAFE_RECOVERY",
            Self::RecoveryRequired(_) => "RECOVERY_REQUIRED",
            Self::Cancelled(_) => "OPERATION_CANCELLED",
            Self::DeadlineExceeded(_) => "DEADLINE_EXCEEDED",
            Self::Admission(_) => "ADMISSION_DENIED",
            Self::Scheduling(_) => "NO_FEASIBLE_PLACEMENT",
            Self::Resource(_) => "RESOURCE_ERROR",
            Self::ShuttingDown => "KERNEL_SHUTTING_DOWN",
            Self::RealityVerification(_) => "REALITY_VERIFICATION_FAILED",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::Journal(_) | Self::Serialization(_) => "state",
            Self::Provider(_) => "provider",
            Self::UnsafeRecovery(_)
            | Self::RecoveryRequired(_)
            | Self::Cancelled(_)
            | Self::DeadlineExceeded(_)
            | Self::ShuttingDown
            | Self::RealityVerification(_) => "execution",
            Self::Admission(_) | Self::Resource(_) => "resource",
            Self::Scheduling(_) => "scheduler",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Provider(_) | Self::Resource(_) | Self::DeadlineExceeded(_)
        )
    }

    pub fn source_component(&self) -> &'static str {
        match self {
            Self::Journal(_) | Self::Serialization(_) => "nous-state",
            Self::Provider(_) => "provider-runtime",
            Self::Scheduling(_) => "nous-scheduler",
            Self::Admission(_) | Self::Resource(_) => "nous-resource",
            Self::UnsafeRecovery(_)
            | Self::RecoveryRequired(_)
            | Self::Cancelled(_)
            | Self::DeadlineExceeded(_)
            | Self::ShuttingDown
            | Self::RealityVerification(_) => "nous-kernel-core",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerificationDecision {
    pub outcome: VerificationOutcome,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[async_trait]
pub trait RealityObserver: Send + Sync {
    fn identity(&self) -> RealityIdentity;

    /// Reject contracts outside this observer's configured scope before any
    /// provider side effect is attempted.
    fn admit_contract(&self, _contract: &EffectContract) -> Result<(), KernelError> {
        Ok(())
    }

    async fn observe(
        &self,
        contract: &EffectContract,
        request: &OperationRequest,
        receipt: &OperationReceipt,
    ) -> Result<ObservedEffect, KernelError>;
}

#[async_trait]
pub trait RealityVerifier: Send + Sync {
    fn identity(&self) -> RealityIdentity;
    fn policy_revision(&self) -> String;

    /// Resolve the immutable Artifact Runtime references and hash their bytes.
    /// A verifier that cannot resolve evidence must fail closed.
    async fn verify_evidence(&self, evidence_refs: &[EvidenceRef]) -> Result<(), KernelError>;

    async fn evaluate(
        &self,
        contract: &EffectContract,
        observation: &ObservedEffect,
    ) -> Result<VerificationDecision, KernelError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealityAdapterDescriptor {
    pub adapter_id: String,
    pub adapter_revision: String,
    pub effect_schema: String,
    pub target_kind: String,
    pub evidence_schema: String,
    pub observer_identity: RealityIdentity,
    pub verifier_identity: RealityIdentity,
    pub authority: String,
}

impl RealityAdapterDescriptor {
    pub fn validate(&self) -> Result<(), KernelError> {
        self.observer_identity
            .validate()
            .map_err(KernelError::RealityVerification)?;
        self.verifier_identity
            .validate()
            .map_err(KernelError::RealityVerification)?;
        if self.adapter_id.is_empty()
            || self.adapter_revision.is_empty()
            || self.effect_schema.is_empty()
            || self.target_kind.is_empty()
            || self.evidence_schema.is_empty()
            || self.authority != "none"
        {
            return Err(KernelError::RealityVerification(
                "reality adapter descriptor is incomplete or claims authority".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct ResolvedRealityAdapter {
    pub descriptor: RealityAdapterDescriptor,
    pub target: TargetBinding,
    pub observer: Arc<dyn RealityObserver>,
    pub verifier: Arc<dyn RealityVerifier>,
}

pub trait RealityAdapterRegistry: Send + Sync {
    fn resolve(&self, contract: &EffectContract) -> Result<ResolvedRealityAdapter, KernelError>;
    fn descriptors(&self) -> Vec<RealityAdapterDescriptor>;
}

pub trait NodeTrustResolver: Send + Sync {
    fn public_key_hex(&self, node_id: &str) -> Result<Option<String>, KernelError>;
}

#[derive(Clone)]
struct RealityRuntime {
    registry: Arc<dyn RealityAdapterRegistry>,
    node_trust: Option<Arc<dyn NodeTrustResolver>>,
}

struct FixedRealityRegistry {
    observer: Arc<dyn RealityObserver>,
    verifier: Arc<dyn RealityVerifier>,
}

impl RealityAdapterRegistry for FixedRealityRegistry {
    fn resolve(&self, contract: &EffectContract) -> Result<ResolvedRealityAdapter, KernelError> {
        self.observer.admit_contract(contract)?;
        let descriptor = RealityAdapterDescriptor {
            adapter_id: "apeir.fixed-reference/v1".into(),
            adapter_revision: "1".into(),
            effect_schema: contract.expectation.schema.clone(),
            target_kind: "reference".into(),
            evidence_schema: contract.expectation.schema.clone(),
            observer_identity: self.observer.identity(),
            verifier_identity: self.verifier.identity(),
            authority: "none".into(),
        };
        descriptor.validate()?;
        Ok(ResolvedRealityAdapter {
            descriptor,
            target: TargetBinding {
                schema_version: nous_types::TARGET_BINDING_SCHEMA_VERSION,
                target_ref: contract.target.clone(),
                target_kind: "reference".into(),
                node_id: "local-kernel".into(),
                adapter_id: "apeir.fixed-reference/v1".into(),
                adapter_revision: "1".into(),
                endpoint_binding: serde_json::json!({}),
                allowed_effect_schemas: vec![contract.expectation.schema.clone()],
                revision: "reference-only".into(),
            },
            observer: self.observer.clone(),
            verifier: self.verifier.clone(),
        })
    }

    fn descriptors(&self) -> Vec<RealityAdapterDescriptor> {
        Vec::new()
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Kernel-owned executor identity, independent of the provider's result.
    fn executor_identity(&self) -> String {
        std::any::type_name::<Self>().to_owned()
    }

    async fn execute(
        &self,
        request: &OperationRequest,
        cancellation: CancellationToken,
    ) -> Result<OperationReceipt, KernelError>;

    async fn probe(
        &self,
        _request: &ProviderProbeRequest,
    ) -> Result<ProviderProbeReport, KernelError> {
        Err(KernelError::Provider(
            "provider capability probe is unsupported".into(),
        ))
    }
}

pub struct ProcessProvider {
    program: PathBuf,
}

impl ProcessProvider {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    async fn invoke(
        &self,
        command_body: ProviderCommand,
        credential_env: &str,
        timeout_ms: u64,
        cancellation: CancellationToken,
        operation_id: &str,
    ) -> Result<ProviderResponse, KernelError> {
        let uncertain_remote_effect = matches!(
            &command_body,
            ProviderCommand::Execute { request }
                if request.execution_domain == ProviderRuntimeClass::Remote
                    && request.effect_contract.is_some()
        );
        let external_entrypoint = match &command_body {
            ProviderCommand::Execute { request } if request.backend == "external-process" => {
                Some(request.provider_entrypoint.as_str())
            }
            ProviderCommand::Probe { request } if request.backend == "external-process" => {
                Some(request.provider_entrypoint.as_str())
            }
            _ => None,
        };
        let mut command = match external_entrypoint {
            Some(entrypoint) if entrypoint.ends_with(".py") => {
                let mut command = Command::new("python");
                command.arg(entrypoint);
                command
            }
            Some(entrypoint) => Command::new(entrypoint),
            None => {
                let mut command = Command::new(&self.program);
                command.arg("--stdio-once");
                command
            }
        };
        command
            .env_clear()
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Keep the provider environment deliberately small, but retain the
        // Windows user-runtime locations needed by executable aliases such as
        // the Python install manager. Without LOCALAPPDATA, `python.exe` may
        // bootstrap a second interpreter beneath the daemon working directory
        // and exhaust the provider deadline before the script starts.
        for name in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "HOME",
            "TMP",
            "TEMP",
            "LOCALAPPDATA",
            "APPDATA",
            "USERPROFILE",
            "APEIR_RELAY_STATE_DIR",
            "APEIR_REALITY_CONFIG",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        if !credential_env.is_empty() {
            nous_types::runtime_api::validate_credential_reference(credential_env)
                .map_err(KernelError::Admission)?;
            let allowed = std::env::var("NOUS_ALLOWED_CREDENTIALS").unwrap_or_default();
            let permitted = allowed
                .split(',')
                .map(str::trim)
                .any(|name| name == credential_env);
            if !permitted {
                return Err(KernelError::Provider(
                    "credential reference is not authorized for provider use".into(),
                ));
            }
            let value = std::env::var_os(credential_env).ok_or_else(|| {
                KernelError::Provider("credential reference is unavailable".into())
            })?;
            command.env(credential_env, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| KernelError::Provider(error.to_string()))?;
        let body = if external_entrypoint.is_some() {
            serde_json::to_vec(&external_provider_request(&command_body)?)
        } else {
            serde_json::to_vec(&command_body)
        }
        .map_err(|error| KernelError::Serialization(error.to_string()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| KernelError::Provider("provider stdin unavailable".into()))?;
        stdin
            .write_all(&body)
            .await
            .map_err(|error| KernelError::Provider(error.to_string()))?;
        drop(stdin);

        let timeout = Duration::from_millis(timeout_ms.max(1));
        let mut wait_task = tokio::spawn(child.wait_with_output());
        let output = tokio::select! {
            result = &mut wait_task => result
                .map_err(|error| KernelError::Provider(error.to_string()))?
                .map_err(|error| KernelError::Provider(error.to_string()))?,
            _ = cancellation.cancelled() => {
                wait_task.abort();
                let _ = wait_task.await;
                return Err(if uncertain_remote_effect {
                    KernelError::RecoveryRequired(operation_id.into())
                } else {
                    KernelError::Cancelled(operation_id.into())
                });
            }
            _ = tokio::time::sleep(timeout) => {
                wait_task.abort();
                let _ = wait_task.await;
                return Err(if uncertain_remote_effect {
                    KernelError::RecoveryRequired(operation_id.into())
                } else {
                    KernelError::DeadlineExceeded(operation_id.into())
                });
            }
        };
        let response = if external_entrypoint.is_some() {
            normalize_external_provider_response(&command_body, &output.stdout)
        } else {
            serde_json::from_slice::<ProviderResponse>(&output.stdout)
                .map_err(|error| KernelError::Serialization(error.to_string()))
        };
        if !output.status.success() {
            if let Ok(response) = &response {
                if !response.ok {
                    return Err(provider_response_error(response, operation_id));
                }
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(KernelError::Provider(if stderr.is_empty() {
                format!("provider process exited with status {}", output.status)
            } else {
                stderr
            }));
        }
        let response = response?;
        if !response.ok {
            return Err(provider_response_error(&response, operation_id));
        }
        Ok(response)
    }
}

fn provider_response_error(response: &ProviderResponse, operation_id: &str) -> KernelError {
    if response.error_code == "NOUS_NODE_UNCERTAIN_EFFECT"
        || response.error_code == "RECOVERY_REQUIRED"
    {
        KernelError::RecoveryRequired(operation_id.into())
    } else {
        KernelError::Provider(format!(
            "{}: {}",
            response.error_code, response.error_message
        ))
    }
}

fn external_provider_request(command: &ProviderCommand) -> Result<serde_json::Value, KernelError> {
    match command {
        ProviderCommand::Execute { request }
            if request.execution_domain == ProviderRuntimeClass::Remote =>
        {
            Ok(serde_json::json!({
                "schema_version": 2,
                "type": "execute",
                "request": request,
            }))
        }
        ProviderCommand::Execute { request } => Ok(serde_json::json!({
            "schema_version": 1,
            "type": "execute",
            "operation_id": request.operation_id,
            "input": request.input,
            "model": request.model,
        })),
        ProviderCommand::Probe { .. } => {
            Ok(serde_json::json!({"schema_version": 1, "type": "probe"}))
        }
        ProviderCommand::Health => Ok(serde_json::json!({"schema_version": 1, "type": "health"})),
    }
}

fn normalize_external_provider_response(
    command: &ProviderCommand,
    output: &[u8],
) -> Result<ProviderResponse, KernelError> {
    let body: serde_json::Value = serde_json::from_slice(output)
        .map_err(|error| KernelError::Provider(format!("malformed provider response: {error}")))?;
    if body.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Ok(ProviderResponse {
            ok: false,
            receipt: None,
            probe: None,
            error_code: body
                .pointer("/error/code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("PROVIDER_EXECUTION_FAILED")
                .into(),
            error_message: body
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("external provider rejected the request")
                .into(),
        });
    }
    match command {
        ProviderCommand::Execute { request } => {
            let result = body
                .get("output")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    KernelError::Provider("external provider returned no string output".into())
                })?
                .to_owned();
            let remote_execution = body
                .get("remote_execution_receipt")
                .cloned()
                .map(serde_json::from_value::<RemoteExecutionReceipt>)
                .transpose()
                .map_err(|error| {
                    KernelError::Provider(format!("malformed remote execution receipt: {error}"))
                })?;
            if request.execution_domain == ProviderRuntimeClass::Remote
                && remote_execution.is_none()
            {
                return Err(KernelError::Provider(
                    "remote provider returned no signed execution receipt".into(),
                ));
            }
            let output_digest = remote_execution
                .as_ref()
                .map(|receipt| receipt.output_digest.clone())
                .unwrap_or_else(|| digest_bytes(result.as_bytes()));
            Ok(ProviderResponse {
                ok: true,
                receipt: Some(OperationReceipt {
                    operation_id: request.operation_id.clone(),
                    input_digest: request.input_digest(),
                    output_digest,
                    snapshot_digest: digest_json(&request.snapshot)?,
                    provider_revision: request.snapshot.provider_revision.clone(),
                    executor_identity: None,
                    remote_execution,
                    result,
                    completed_at_us: chrono::Utc::now().timestamp_micros(),
                }),
                probe: None,
                error_code: String::new(),
                error_message: String::new(),
            })
        }
        ProviderCommand::Probe { request } => Ok(ProviderResponse {
            ok: true,
            receipt: None,
            probe: Some(ProviderProbeReport {
                reachable: true,
                latency_ms: 0,
                available_models: Vec::new(),
                capabilities: ModelCapabilityManifest {
                    schema_version: 1,
                    provider: request.provider.clone(),
                    backend: request.backend.clone(),
                    model: request.model.clone(),
                    runtime_class: request.execution_domain,
                    chat: CapabilitySupport::Unknown,
                    streaming: CapabilitySupport::Unknown,
                    tools: CapabilitySupport::Unknown,
                    structured_output: CapabilitySupport::Unknown,
                    vision: CapabilitySupport::Unknown,
                    embedding: CapabilitySupport::Unknown,
                    reasoning: CapabilitySupport::Unknown,
                    local_execution: CapabilitySupport::Supported,
                    remote_execution: CapabilitySupport::Unsupported,
                    context_length: None,
                    privacy_local: true,
                    privacy: ModelPrivacy::LocalOnly,
                    estimated_latency_ms: None,
                    estimated_cost_microcents: None,
                    probed_at_us: chrono::Utc::now().timestamp_micros(),
                },
                warnings: vec!["generic capabilities are declared by ProviderManifest v1".into()],
            }),
            error_code: String::new(),
            error_message: String::new(),
        }),
        ProviderCommand::Health => Ok(ProviderResponse {
            ok: true,
            receipt: None,
            probe: None,
            error_code: String::new(),
            error_message: String::new(),
        }),
    }
}

#[async_trait]
impl Provider for ProcessProvider {
    fn executor_identity(&self) -> String {
        "apeir.process-provider".into()
    }

    async fn execute(
        &self,
        request: &OperationRequest,
        cancellation: CancellationToken,
    ) -> Result<OperationReceipt, KernelError> {
        self.invoke(
            ProviderCommand::Execute {
                request: Box::new(request.clone()),
            },
            &request.credential_env,
            request.timeout_ms,
            cancellation,
            &request.operation_id,
        )
        .await?
        .receipt
        .ok_or_else(|| KernelError::Provider("provider returned no receipt".into()))
    }

    async fn probe(
        &self,
        request: &ProviderProbeRequest,
    ) -> Result<ProviderProbeReport, KernelError> {
        self.invoke(
            ProviderCommand::Probe {
                request: request.clone(),
            },
            &request.credential_env,
            request.timeout_ms,
            CancellationToken::default(),
            "provider-probe",
        )
        .await?
        .probe
        .ok_or_else(|| KernelError::Provider("provider returned no capability report".into()))
    }
}

pub struct BootCore {
    state: RwLock<KernelState>,
    integrity_failures: RwLock<Vec<u64>>,
    pub journal: Arc<Journal>,
}

impl BootCore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, KernelError> {
        let core = Self {
            state: RwLock::new(KernelState::Starting),
            integrity_failures: RwLock::new(Vec::new()),
            journal: Arc::new(
                Journal::open(path).map_err(|error| KernelError::Journal(error.to_string()))?,
            ),
        };
        core.set_state(KernelState::Recovering)?;
        let corrupted = core
            .journal
            .verify_integrity()
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        *core
            .integrity_failures
            .write()
            .map_err(|_| KernelError::Journal("integrity cache lock is poisoned".into()))? =
            corrupted.clone();
        core.set_state(if corrupted.is_empty() {
            KernelState::Ready
        } else {
            KernelState::Degraded
        })?;
        Ok(core)
    }

    pub fn state(&self) -> KernelState {
        self.state
            .read()
            .map(|state| *state)
            .unwrap_or(KernelState::Failed)
    }

    pub fn integrity_failures(&self, verify: bool) -> Result<Vec<u64>, KernelError> {
        if verify {
            let corrupted = self
                .journal
                .verify_integrity()
                .map_err(|error| KernelError::Journal(error.to_string()))?;
            *self
                .integrity_failures
                .write()
                .map_err(|_| KernelError::Journal("integrity cache lock is poisoned".into()))? =
                corrupted.clone();
            return Ok(corrupted);
        }
        self.integrity_failures
            .read()
            .map(|failures| failures.clone())
            .map_err(|_| KernelError::Journal("integrity cache lock is poisoned".into()))
    }

    pub fn shutdown(&self) {
        let _ = self.set_state(KernelState::ShuttingDown);
    }

    fn set_state(&self, state: KernelState) -> Result<(), KernelError> {
        *self
            .state
            .write()
            .map_err(|_| KernelError::Journal("kernel state lock is poisoned".into()))? = state;
        Ok(())
    }
}

pub(crate) struct DurableExecutor<P: Provider> {
    journal: Arc<Journal>,
    pub(crate) provider: P,
    reality: Option<RealityRuntime>,
}

impl<P: Provider> DurableExecutor<P> {
    pub(crate) fn new(journal: Arc<Journal>, provider: P) -> Self {
        Self {
            journal,
            provider,
            reality: None,
        }
    }

    pub(crate) fn configure_reality_registry(
        &mut self,
        registry: Arc<dyn RealityAdapterRegistry>,
        node_trust: Option<Arc<dyn NodeTrustResolver>>,
    ) {
        self.reality = Some(RealityRuntime {
            registry,
            node_trust,
        });
    }

    pub(crate) fn reality_descriptors(&self) -> Vec<RealityAdapterDescriptor> {
        self.reality
            .as_ref()
            .map(|runtime| runtime.registry.descriptors())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn configure_reality(
        &mut self,
        observer: Arc<dyn RealityObserver>,
        verifier: Arc<dyn RealityVerifier>,
    ) {
        self.configure_reality_registry(
            Arc::new(FixedRealityRegistry { observer, verifier }),
            None,
        );
    }

    #[cfg(test)]
    pub(crate) async fn execute(
        &self,
        request: &OperationRequest,
    ) -> Result<OperationReceipt, KernelError> {
        self.execute_cancellable(request, CancellationToken::default())
            .await
    }

    pub(crate) async fn execute_cancellable(
        &self,
        request: &OperationRequest,
        cancellation: CancellationToken,
    ) -> Result<OperationReceipt, KernelError> {
        self.preflight_reality(request)?;
        self.validate_intent(request)?;
        if let Some(receipt) = self.committed_receipt(request)? {
            return Ok(receipt);
        }
        self.append(
            request,
            EntryType::Intent,
            "Operation",
            "PENDING",
            request,
            Some(format!("operation:{}:intent", request.operation_id)),
        )?;

        if let Some(contract) = &request.effect_contract {
            let reality = self.reality.as_ref().ok_or_else(|| {
                KernelError::RealityVerification(
                    "effect contract requires a configured reality registry".into(),
                )
            })?;
            let adapter = reality.registry.resolve(contract)?;
            self.append(
                request,
                EntryType::Observation,
                "TargetBinding",
                "RESOLVED",
                &adapter.target,
                Some(format!("operation:{}:target-binding", request.operation_id)),
            )?;
        }

        fault::trigger(FaultPoint::EffectAfterIntent, &request.operation_id);

        if cancellation.is_cancelled() {
            return Err(KernelError::Cancelled(request.operation_id.clone()));
        }

        let receipt = match self.received_receipt(request)? {
            Some(receipt) => receipt,
            None => {
                if request.effect_contract.is_some() {
                    self.append(
                        request,
                        EntryType::Transition,
                        "EffectState",
                        "EXECUTION_DISPATCHED",
                        &serde_json::json!({"operation_id": request.operation_id}),
                        Some(format!(
                            "operation:{}:execution-dispatched",
                            request.operation_id
                        )),
                    )?;
                }
                fault::trigger(FaultPoint::EffectBeforeExecute, &request.operation_id);
                let mut receipt = match self.provider.execute(request, cancellation).await {
                    Ok(receipt) => receipt,
                    Err(KernelError::RecoveryRequired(operation_id)) => {
                        self.record_recovery_required(
                            request,
                            "remote execution outcome is uncertain",
                        )?;
                        return Err(KernelError::RecoveryRequired(operation_id));
                    }
                    Err(error) => return Err(error),
                };
                if request.effect_contract.is_some() {
                    receipt.executor_identity = Some(self.provider.executor_identity());
                }
                fault::trigger(FaultPoint::EffectAfterExecute, &request.operation_id);
                self.validate_receipt(request, &receipt)?;
                self.append(
                    request,
                    EntryType::Observation,
                    "OperationReceipt",
                    "RECEIVED",
                    &receipt,
                    Some(format!("operation:{}:receipt", request.operation_id)),
                )?;
                receipt
            }
        };

        self.validate_receipt(request, &receipt)?;

        fault::trigger(FaultPoint::EffectAfterReceipt, &request.operation_id);

        if let Some(contract) = &request.effect_contract {
            self.verify_reality(request, &receipt, contract).await?;
        }

        self.append(
            request,
            EntryType::Commit,
            "StepCommit",
            "COMMITTED",
            &receipt,
            Some(format!("operation:{}:commit", request.operation_id)),
        )?;
        fault::trigger(FaultPoint::EffectAfterCommit, &request.operation_id);
        Ok(receipt)
    }

    fn validate_receipt(
        &self,
        request: &OperationRequest,
        receipt: &OperationReceipt,
    ) -> Result<(), KernelError> {
        if receipt.operation_id != request.operation_id
            || receipt.input_digest != request.input_digest()
            || receipt.snapshot_digest != digest_json(&request.snapshot)?
            || receipt.provider_revision != request.snapshot.provider_revision
            || (request.effect_contract.is_some()
                && receipt
                    .executor_identity
                    .as_deref()
                    .is_none_or(str::is_empty))
        {
            return Err(KernelError::UnsafeRecovery(request.operation_id.clone()));
        }
        if request.effect_contract.is_some()
            && request.execution_domain == ProviderRuntimeClass::Remote
            && receipt.remote_execution.is_none()
        {
            return Err(KernelError::RealityVerification(
                "remote effect execution requires a signed remote receipt".into(),
            ));
        }
        if let Some(remote) = &receipt.remote_execution {
            if receipt.output_digest != remote.output_digest {
                return Err(KernelError::RealityVerification(
                    "kernel and remote receipt output digests differ".into(),
                ));
            }
            let contract = request.effect_contract.as_ref().ok_or_else(|| {
                KernelError::RealityVerification(
                    "remote receipt requires an effect contract".into(),
                )
            })?;
            let reality = self.reality.as_ref().ok_or_else(|| {
                KernelError::RealityVerification(
                    "remote receipt requires a configured reality registry".into(),
                )
            })?;
            let adapter = reality.registry.resolve(contract)?;
            remote
                .validate_bindings(request, contract, &adapter.target)
                .map_err(KernelError::RealityVerification)?;
            if contract.verification == VerificationMode::Independent
                && adapter.verifier.identity().identity == remote.executor_id
            {
                return Err(KernelError::RealityVerification(
                    "independent verifier identity equals remote executor identity".into(),
                ));
            }
            let trust = reality.node_trust.as_ref().ok_or_else(|| {
                KernelError::RealityVerification(
                    "remote receipt requires a node trust resolver".into(),
                )
            })?;
            let public_key = trust.public_key_hex(&remote.node_id)?.ok_or_else(|| {
                KernelError::RealityVerification("remote node identity is not trusted".into())
            })?;
            verify_node_signature(remote, &public_key)?;
        }
        Ok(())
    }

    pub(crate) fn preflight_reality(&self, request: &OperationRequest) -> Result<(), KernelError> {
        if let Some(contract) = &request.effect_contract {
            contract
                .validate()
                .map_err(KernelError::RealityVerification)?;
            if contract.verification == VerificationMode::None {
                return Err(KernelError::RealityVerification(
                    "verification NONE cannot commit an effectful operation".into(),
                ));
            }
            let reality = self.reality.as_ref().ok_or_else(|| {
                KernelError::RealityVerification(
                    "effect contract requires an explicitly configured observer and verifier"
                        .into(),
                )
            })?;
            let adapter = reality.registry.resolve(contract)?;
            adapter.descriptor.validate()?;
            adapter
                .target
                .admits(contract)
                .map_err(KernelError::RealityVerification)?;
            adapter
                .observer
                .identity()
                .validate()
                .map_err(KernelError::RealityVerification)?;
            adapter.observer.admit_contract(contract)?;
            adapter
                .verifier
                .identity()
                .validate()
                .map_err(KernelError::RealityVerification)?;
            if adapter.descriptor.observer_identity != adapter.observer.identity()
                || adapter.descriptor.verifier_identity != adapter.verifier.identity()
                || adapter.descriptor.effect_schema != contract.expectation.schema
                || adapter.descriptor.adapter_id != adapter.target.adapter_id
                || adapter.descriptor.adapter_revision != adapter.target.adapter_revision
            {
                return Err(KernelError::RealityVerification(
                    "reality registry descriptor or target binding mismatch".into(),
                ));
            }
            if contract.verification == VerificationMode::Independent
                && adapter.verifier.identity().identity == self.provider.executor_identity()
            {
                return Err(KernelError::RealityVerification(
                    "independent verifier identity equals kernel-owned executor identity".into(),
                ));
            }
        }
        Ok(())
    }

    async fn verify_reality(
        &self,
        request: &OperationRequest,
        receipt: &OperationReceipt,
        contract: &EffectContract,
    ) -> Result<EffectVerification, KernelError> {
        contract
            .validate()
            .map_err(KernelError::RealityVerification)?;
        if contract.verification == VerificationMode::None {
            return Err(KernelError::RealityVerification(
                "verification NONE cannot commit an effectful operation".into(),
            ));
        }
        let reality = self.reality.as_ref().ok_or_else(|| {
            KernelError::RealityVerification(
                "effect contract requires an explicitly configured observer and verifier".into(),
            )
        })?;
        let adapter = reality.registry.resolve(contract)?;

        self.append(
            request,
            EntryType::Transition,
            "EffectState",
            "EXECUTED",
            &serde_json::json!({"effect_id": contract.effect_id}),
            Some(format!(
                "operation:{}:effect:executed",
                request.operation_id
            )),
        )?;

        let observation = if let Some(observation) = self.received_observation(request)? {
            if observation.observer != adapter.observer.identity() {
                return Err(KernelError::RealityVerification(
                    "durable observer identity differs from configured observer".into(),
                ));
            }
            observation
                .validate(contract)
                .map_err(KernelError::RealityVerification)?;
            observation
        } else {
            let observation = match adapter.observer.observe(contract, request, receipt).await {
                Ok(observation) => observation,
                Err(error) => {
                    self.append(
                        request,
                        EntryType::Transition,
                        "EffectState",
                        "OBSERVATION_FAILED",
                        &serde_json::json!({"effect_id": contract.effect_id, "error": error.code()}),
                        Some(format!(
                            "operation:{}:effect:observation-failed",
                            request.operation_id
                        )),
                    )?;
                    return Err(error);
                }
            };
            if observation.observer != adapter.observer.identity() {
                return Err(KernelError::RealityVerification(
                    "observer receipt identity differs from configured observer".into(),
                ));
            }
            observation
                .validate(contract)
                .map_err(KernelError::RealityVerification)?;
            self.append(
                request,
                EntryType::Observation,
                "ObservedEffect",
                "OBSERVED",
                &observation,
                Some(format!("operation:{}:observation", request.operation_id)),
            )?;
            observation
        };
        fault::trigger(FaultPoint::EffectAfterObservation, &request.operation_id);

        adapter
            .verifier
            .verify_evidence(&observation.evidence_refs)
            .await?;

        let verifier = adapter.verifier.identity();
        if contract.verification == VerificationMode::Independent
            && (verifier.identity == receipt.provider_revision
                || receipt.executor_identity.as_deref() == Some(verifier.identity.as_str()))
        {
            return Err(KernelError::RealityVerification(
                "independent verifier identity equals provider executor identity".into(),
            ));
        }
        let verification = if let Some(verification) = self.received_verification(request)? {
            if verification.verifier != verifier
                || verification.verification_policy_revision != adapter.verifier.policy_revision()
            {
                return Err(KernelError::RealityVerification(
                    "durable verifier identity or policy differs from configured verifier".into(),
                ));
            }
            verification
                .validate_bindings(contract, &observation)
                .map_err(KernelError::RealityVerification)?;
            verification
        } else {
            let decision = adapter.verifier.evaluate(contract, &observation).await?;
            let verification = EffectVerification {
                verification_id: uuid::Uuid::now_v7().to_string(),
                effect_id: contract.effect_id.clone(),
                outcome: decision.outcome,
                effect_contract_digest: contract
                    .digest()
                    .map_err(KernelError::RealityVerification)?,
                observation_digest: observation
                    .digest()
                    .map_err(KernelError::RealityVerification)?,
                verifier: verifier.clone(),
                verification_policy_revision: adapter.verifier.policy_revision(),
                evidence_refs: decision.evidence_refs,
                verified_at: chrono::Utc::now(),
            };
            verification
                .validate_bindings(contract, &observation)
                .map_err(KernelError::RealityVerification)?;
            let phase = match verification.outcome {
                VerificationOutcome::Match => "VERIFIED",
                VerificationOutcome::Partial => "PARTIAL",
                VerificationOutcome::Mismatch => "VERIFICATION_MISMATCH",
                VerificationOutcome::Unknown => "UNKNOWN",
            };
            self.append(
                request,
                EntryType::Observation,
                "EffectVerification",
                phase,
                &verification,
                Some(format!("operation:{}:verification", request.operation_id)),
            )?;
            verification
        };
        adapter
            .verifier
            .verify_evidence(&verification.evidence_refs)
            .await?;
        if verification.verifier != verifier {
            return Err(KernelError::RealityVerification(
                "verification receipt identity differs from configured verifier".into(),
            ));
        }
        if verification.outcome != VerificationOutcome::Match {
            return Err(KernelError::RealityVerification(format!(
                "effect {} verification outcome is {:?}",
                contract.effect_id, verification.outcome
            )));
        }
        Ok(verification)
    }

    pub(crate) fn pending_operations(&self) -> Result<Vec<OperationRequest>, KernelError> {
        let entries = self
            .journal
            .read_from(0)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        let mut pending = HashMap::new();
        for entry in entries {
            if entry.entry_type == EntryType::Intent && entry.object_type == "Operation" {
                let request = serde_json::from_slice::<OperationRequest>(&entry.payload)
                    .map_err(|error| KernelError::Serialization(error.to_string()))?;
                pending.insert(request.operation_id.clone(), request);
            } else if (entry.entry_type == EntryType::Commit && entry.object_type == "StepCommit")
                || (entry.entry_type == EntryType::Abort && entry.object_type == "Operation")
            {
                pending.remove(&entry.object_id);
            }
        }
        Ok(pending.into_values().collect())
    }

    pub(crate) fn record_recovery_required(
        &self,
        request: &OperationRequest,
        reason: &str,
    ) -> Result<(), KernelError> {
        self.append(
            request,
            EntryType::Transition,
            "EffectState",
            "RECOVERY_REQUIRED",
            &serde_json::json!({
                "operation_id": request.operation_id,
                "effect_id": request.effect_contract.as_ref().map(|effect| &effect.effect_id),
                "reason": reason,
            }),
            Some(format!(
                "operation:{}:recovery-required",
                request.operation_id
            )),
        )
    }

    #[cfg(test)]
    pub(crate) async fn recover(&self) -> Result<Vec<OperationReceipt>, KernelError> {
        let mut receipts = Vec::new();
        for request in self.pending_operations()? {
            match request.delivery.recovery_strategy() {
                nous_types::RecoveryStrategy::Replay => {
                    receipts.push(self.execute(&request).await?);
                }
                _ => return Err(KernelError::UnsafeRecovery(request.operation_id)),
            }
        }
        Ok(receipts)
    }

    pub(crate) fn committed_receipt(
        &self,
        request: &OperationRequest,
    ) -> Result<Option<OperationReceipt>, KernelError> {
        self.received_fact(request, EntryType::Commit, "StepCommit")
    }

    pub(crate) fn received_receipt(
        &self,
        request: &OperationRequest,
    ) -> Result<Option<OperationReceipt>, KernelError> {
        self.received_fact(request, EntryType::Observation, "OperationReceipt")
    }

    pub(crate) fn validate_intent(&self, request: &OperationRequest) -> Result<(), KernelError> {
        let existing: Option<OperationRequest> =
            self.received_fact(request, EntryType::Intent, "Operation")?;
        if let Some(existing) = existing {
            // The NKI envelope may tighten the per-attempt deadline. It is not
            // part of the durable effect identity; every other field is.
            let mut candidate = request.clone();
            candidate.timeout_ms = existing.timeout_ms;
            if existing != candidate {
                return Err(KernelError::UnsafeRecovery(request.operation_id.clone()));
            }
        }
        Ok(())
    }

    fn received_observation(
        &self,
        request: &OperationRequest,
    ) -> Result<Option<ObservedEffect>, KernelError> {
        self.received_fact(request, EntryType::Observation, "ObservedEffect")
    }

    fn received_verification(
        &self,
        request: &OperationRequest,
    ) -> Result<Option<EffectVerification>, KernelError> {
        self.received_fact(request, EntryType::Observation, "EffectVerification")
    }

    fn received_fact<T: serde::de::DeserializeOwned>(
        &self,
        request: &OperationRequest,
        entry_type: EntryType,
        object_type: &str,
    ) -> Result<Option<T>, KernelError> {
        let entries = self
            .journal
            .read_workload(&request.workload_id)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        entries
            .iter()
            .rev()
            .find(|entry| {
                entry.entry_type == entry_type
                    && entry.object_type == object_type
                    && entry.object_id == request.operation_id
            })
            .map(|entry| {
                serde_json::from_slice(&entry.payload)
                    .map_err(|error| KernelError::Serialization(error.to_string()))
            })
            .transpose()
    }

    pub(crate) fn append<T: Serialize>(
        &self,
        request: &OperationRequest,
        entry_type: EntryType,
        object_type: &str,
        phase: &str,
        payload: &T,
        idempotency_key: Option<String>,
    ) -> Result<(), KernelError> {
        let payload = serde_json::to_vec(payload)
            .map_err(|error| KernelError::Serialization(error.to_string()))?;
        self.journal
            .append(JournalEntry {
                sequence: 0,
                workload_id: request.workload_id.clone(),
                entry_type,
                object_type: object_type.into(),
                object_id: request.operation_id.clone(),
                previous_phase: None,
                new_phase: phase.into(),
                generation: 1,
                payload,
                fencing_token: request.operation_id.clone(),
                actor: "nousd".into(),
                idempotency_key,
                timestamp_us: 0,
                checksum: Vec::new(),
            })
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        Ok(())
    }
}

pub fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn digest_json<T: Serialize>(value: &T) -> Result<String, KernelError> {
    let body =
        serde_json::to_vec(value).map_err(|error| KernelError::Serialization(error.to_string()))?;
    Ok(digest_bytes(&body))
}

fn verify_node_signature(
    receipt: &RemoteExecutionReceipt,
    public_key_hex: &str,
) -> Result<(), KernelError> {
    let public_key: [u8; 32] = decode_hex(public_key_hex)?
        .try_into()
        .map_err(|_| KernelError::RealityVerification("node public key must be 32 bytes".into()))?;
    let signature: [u8; 64] = decode_hex(&receipt.signed_envelope.signature)?
        .try_into()
        .map_err(|_| KernelError::RealityVerification("node signature must be 64 bytes".into()))?;
    let verifier = VerifyingKey::from_bytes(&public_key)
        .map_err(|_| KernelError::RealityVerification("node public key is invalid".into()))?;
    verifier
        .verify(
            &receipt
                .signed_envelope
                .signing_bytes()
                .map_err(KernelError::RealityVerification)?,
            &Signature::from_bytes(&signature),
        )
        .map_err(|_| KernelError::RealityVerification("node signature is invalid".into()))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, KernelError> {
    if !value.len().is_multiple_of(2) {
        return Err(KernelError::RealityVerification(
            "hex value has an invalid length".into(),
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| {
                KernelError::RealityVerification("hex value contains invalid UTF-8".into())
            })?;
            u8::from_str_radix(text, 16)
                .map_err(|_| KernelError::RealityVerification("hex value is invalid".into()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_types::SignedNodeEnvelope;

    struct DigestProvider;

    struct RejectProvider;

    struct TestObserver {
        value: serde_json::Value,
        tamper_digest: bool,
    }

    struct TestVerifier {
        identity: String,
    }

    #[test]
    fn verifies_python_node_protocol_ed25519_fixture() {
        let envelope: SignedNodeEnvelope = serde_json::from_str(
            r#"{"created_at":"2026-09-23T00:00:00Z","expires_at":"","idempotency_key":"operation-1","message_id":"message-fixture","message_type":"WORKLOAD_STATUS","payload":{"binding":{"intent_id":"operation-1"},"request_digest":"ab","state":"COMPLETED","workload_id":"operation-1"},"protocol":"nous-node","protocol_version":"1.0","reply_to":"dispatch-fixture","sequence":7,"signature":"17cf3591d2d721a09d871f97a8c1f6228f2dc804533972658ec73f74699bc0fa0300de4eea9bd2a2cc18dd692abec3f21c0ecfa76c37db133442ad27ae4d3e02","source":"node-fixture","target":"control_plane"}"#,
        )
        .unwrap();
        let receipt = RemoteExecutionReceipt {
            schema_version: 1,
            operation_id: "operation-1".into(),
            workload_id: "workload-1".into(),
            node_id: "node-fixture".into(),
            executor_id: "fixture".into(),
            intent_id: "operation-1".into(),
            effect_contract_digest: "0".repeat(64),
            target_ref: "node://fixture/service".into(),
            target_binding_digest: "1".repeat(64),
            request_digest: "2".repeat(64),
            output_digest: "3".repeat(64),
            provider_revision: "fixture".into(),
            delivery_semantics: DeliverySemantics::AtMostOnce,
            started_at: "2026-09-23T00:00:00Z".into(),
            completed_at: "2026-09-23T00:00:01Z".into(),
            node_protocol_version: "1.0".into(),
            signed_envelope_digest: envelope.digest().unwrap(),
            signed_envelope: envelope,
        };
        verify_node_signature(
            &receipt,
            "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8",
        )
        .unwrap();
        let mut tampered = receipt;
        tampered.signed_envelope.payload["state"] = serde_json::json!("FAILED");
        assert!(verify_node_signature(
            &tampered,
            "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8",
        )
        .is_err());
    }

    #[test]
    fn remote_provider_uncertainty_requires_manual_recovery() {
        let response = ProviderResponse {
            ok: false,
            receipt: None,
            probe: None,
            error_code: "NOUS_NODE_UNCERTAIN_EFFECT".into(),
            error_message: "outcome unknown".into(),
        };
        assert!(matches!(
            provider_response_error(&response, "operation-1"),
            KernelError::RecoveryRequired(operation) if operation == "operation-1"
        ));
    }

    #[test]
    fn remote_external_provider_requires_signed_receipt() {
        let mut request = request();
        request.execution_domain = ProviderRuntimeClass::Remote;
        request.backend = "external-process".into();
        let command = ProviderCommand::Execute {
            request: Box::new(request.clone()),
        };
        assert_eq!(
            external_provider_request(&command).unwrap()["schema_version"],
            serde_json::json!(2)
        );
        let response = serde_json::to_vec(&serde_json::json!({
            "ok": true,
            "output": "{}",
        }))
        .unwrap();
        assert!(normalize_external_provider_response(&command, &response)
            .unwrap_err()
            .to_string()
            .contains("no signed execution receipt"));
    }

    #[async_trait]
    impl RealityObserver for TestObserver {
        fn identity(&self) -> RealityIdentity {
            RealityIdentity {
                identity: "health-observer".into(),
                capability: "service.health.observe".into(),
            }
        }

        async fn observe(
            &self,
            contract: &EffectContract,
            _request: &OperationRequest,
            _receipt: &OperationReceipt,
        ) -> Result<ObservedEffect, KernelError> {
            let evidence_refs = vec![EvidenceRef {
                artifact_ref: format!("sha256:{}", digest_bytes(b"health-response")),
                digest: digest_bytes(b"health-response"),
            }];
            let mut evidence_digest =
                ObservedEffect::compute_evidence_digest(&self.value, &evidence_refs)
                    .map_err(KernelError::RealityVerification)?;
            if self.tamper_digest {
                evidence_digest = "0".repeat(64);
            }
            Ok(ObservedEffect {
                observation_id: uuid::Uuid::now_v7().to_string(),
                effect_id: contract.effect_id.clone(),
                subject: contract.expectation.subject.clone(),
                schema: contract.expectation.schema.clone(),
                observed_value: self.value.clone(),
                observer: self.identity(),
                observed_at: chrono::Utc::now(),
                evidence_refs,
                evidence_digest,
                execution_environment_digest: Some(digest_bytes(b"arm64-host-profile")),
            })
        }
    }

    #[async_trait]
    impl RealityVerifier for TestVerifier {
        fn identity(&self) -> RealityIdentity {
            RealityIdentity {
                identity: self.identity.clone(),
                capability: "service.health.verify".into(),
            }
        }

        fn policy_revision(&self) -> String {
            "service-health-policy-v1".into()
        }

        async fn verify_evidence(&self, evidence_refs: &[EvidenceRef]) -> Result<(), KernelError> {
            if evidence_refs.len() == 1
                && evidence_refs[0].artifact_ref
                    == format!("sha256:{}", digest_bytes(b"health-response"))
                && evidence_refs[0].digest == digest_bytes(b"health-response")
            {
                Ok(())
            } else {
                Err(KernelError::RealityVerification(
                    "observation evidence is unavailable or mismatched".into(),
                ))
            }
        }

        async fn evaluate(
            &self,
            contract: &EffectContract,
            observation: &ObservedEffect,
        ) -> Result<VerificationDecision, KernelError> {
            Ok(VerificationDecision {
                outcome: if contract.expectation.expected_value == observation.observed_value {
                    VerificationOutcome::Match
                } else {
                    VerificationOutcome::Mismatch
                },
                evidence_refs: observation.evidence_refs.clone(),
            })
        }
    }

    #[async_trait]
    impl Provider for DigestProvider {
        async fn execute(
            &self,
            request: &OperationRequest,
            _cancellation: CancellationToken,
        ) -> Result<OperationReceipt, KernelError> {
            Ok(OperationReceipt {
                operation_id: request.operation_id.clone(),
                input_digest: request.input_digest(),
                output_digest: digest_bytes(request.input.as_bytes()),
                snapshot_digest: digest_json(&request.snapshot)?,
                provider_revision: request.snapshot.provider_revision.clone(),
                executor_identity: None,
                remote_execution: None,
                result: request.input.clone(),
                completed_at_us: chrono::Utc::now().timestamp_micros(),
            })
        }
    }

    #[async_trait]
    impl Provider for RejectProvider {
        async fn execute(
            &self,
            _request: &OperationRequest,
            _cancellation: CancellationToken,
        ) -> Result<OperationReceipt, KernelError> {
            panic!("provider must not be called when a durable receipt exists")
        }
    }

    fn request() -> OperationRequest {
        OperationRequest {
            operation_id: "op-1".into(),
            workload_id: "workload-1".into(),
            step_id: "step-1".into(),
            backend: "digest".into(),
            execution_domain: ProviderRuntimeClass::Reference,
            model: String::new(),
            endpoint: String::new(),
            credential_env: String::new(),
            provider_entrypoint: String::new(),
            input: "hello".into(),
            delivery: DeliverySemantics::Idempotent,
            snapshot: SemanticExecutionSnapshot {
                model_revision: "none".into(),
                provider_revision: "digest-v1".into(),
                prompt_revision: "prompt-v1".into(),
                tool_revision: "none".into(),
                knowledge_revision: "none".into(),
                policy_revision: "policy-v1".into(),
                capability_revision: "cap-v1".into(),
                context_revision: "context-v1".into(),
            },
            timeout_ms: 5_000,
            effect_contract: None,
        }
    }

    fn reality_request(expected_status: u16) -> OperationRequest {
        let mut request = request();
        request.effect_contract = Some(EffectContract {
            schema_version: 1,
            effect_id: "effect-service-restart".into(),
            target: "service:test".into(),
            expectation: nous_types::EffectExpectation {
                schema: "apeir.service-health/v1".into(),
                subject: "service:test".into(),
                expected_value: serde_json::json!({"http_status": expected_status, "version": "v2"}),
                evidence_requirement: vec!["http-response".into()],
            },
            verification: VerificationMode::Independent,
        });
        request
    }

    fn reality_executor<P: Provider>(
        journal: Arc<Journal>,
        provider: P,
        value: serde_json::Value,
        tamper_digest: bool,
        verifier_identity: &str,
    ) -> DurableExecutor<P> {
        let mut executor = DurableExecutor::new(journal, provider);
        executor.configure_reality(
            Arc::new(TestObserver {
                value,
                tamper_digest,
            }),
            Arc::new(TestVerifier {
                identity: verifier_identity.into(),
            }),
        );
        executor
    }

    #[tokio::test]
    async fn commits_once_and_replays_receipt() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let executor = DurableExecutor::new(journal.clone(), DigestProvider);
        let first = executor.execute(&request()).await.unwrap();
        let second = executor.execute(&request()).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(journal.read_workload("workload-1").unwrap().len(), 3);
    }

    #[tokio::test]
    async fn refuses_unsafe_pending_operation_recovery() {
        for delivery in [DeliverySemantics::AtMostOnce, DeliverySemantics::Unknown] {
            let journal = Arc::new(Journal::open_in_memory().unwrap());
            let executor = DurableExecutor::new(journal, DigestProvider);
            let mut unsafe_request = request();
            unsafe_request.delivery = delivery;
            executor
                .append(
                    &unsafe_request,
                    EntryType::Intent,
                    "Operation",
                    "PENDING",
                    &unsafe_request,
                    Some("operation:op-1:intent".into()),
                )
                .unwrap();

            let error = executor.recover().await.unwrap_err();
            assert!(matches!(error, KernelError::UnsafeRecovery(operation) if operation == "op-1"));
        }
    }

    #[tokio::test]
    async fn recovery_commits_received_receipt_without_reexecution() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let request = request();
        let receipt = OperationReceipt {
            operation_id: request.operation_id.clone(),
            input_digest: request.input_digest(),
            output_digest: digest_bytes(request.input.as_bytes()),
            snapshot_digest: digest_json(&request.snapshot).unwrap(),
            provider_revision: request.snapshot.provider_revision.clone(),
            executor_identity: None,
            remote_execution: None,
            result: "durable-result".into(),
            completed_at_us: 1,
        };
        let writer = DurableExecutor::new(journal.clone(), DigestProvider);
        writer
            .append(
                &request,
                EntryType::Intent,
                "Operation",
                "PENDING",
                &request,
                Some("operation:op-1:intent".into()),
            )
            .unwrap();
        writer
            .append(
                &request,
                EntryType::Observation,
                "OperationReceipt",
                "RECEIVED",
                &receipt,
                Some("operation:op-1:receipt".into()),
            )
            .unwrap();

        let recovery = DurableExecutor::new(journal.clone(), RejectProvider);
        let recovered = recovery.recover().await.unwrap();
        assert_eq!(recovered, vec![receipt]);
        assert_eq!(journal.read_workload("workload-1").unwrap().len(), 3);
    }

    #[tokio::test]
    async fn provider_success_does_not_commit_reality_mismatch() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let executor = reality_executor(
            journal.clone(),
            DigestProvider,
            serde_json::json!({"http_status": 502, "version": "v2"}),
            false,
            "health-verifier",
        );
        let error = executor.execute(&reality_request(200)).await.unwrap_err();
        assert!(matches!(error, KernelError::RealityVerification(_)));
        let entries = journal.read_workload("workload-1").unwrap();
        assert!(entries.iter().any(|entry| {
            entry.object_type == "EffectVerification" && entry.new_phase == "VERIFICATION_MISMATCH"
        }));
        assert!(!entries
            .iter()
            .any(|entry| entry.object_type == "StepCommit"));
    }

    #[tokio::test]
    async fn matching_reality_is_required_before_commit() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let executor = reality_executor(
            journal.clone(),
            DigestProvider,
            serde_json::json!({"http_status": 200, "version": "v2"}),
            false,
            "health-verifier",
        );
        executor.execute(&reality_request(200)).await.unwrap();
        let entries = journal.read_workload("workload-1").unwrap();
        let verification = entries
            .iter()
            .position(|entry| entry.object_type == "EffectVerification")
            .unwrap();
        let commit = entries
            .iter()
            .position(|entry| entry.object_type == "StepCommit")
            .unwrap();
        assert!(verification < commit);
    }

    #[tokio::test]
    async fn recovery_after_execution_resumes_verification_without_provider_replay() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let request = reality_request(200);
        let receipt = OperationReceipt {
            operation_id: request.operation_id.clone(),
            input_digest: request.input_digest(),
            output_digest: digest_bytes(request.input.as_bytes()),
            snapshot_digest: digest_json(&request.snapshot).unwrap(),
            provider_revision: request.snapshot.provider_revision.clone(),
            executor_identity: Some(std::any::type_name::<DigestProvider>().into()),
            remote_execution: None,
            result: "provider-success".into(),
            completed_at_us: 1,
        };
        let writer = DurableExecutor::new(journal.clone(), DigestProvider);
        writer
            .append(
                &request,
                EntryType::Intent,
                "Operation",
                "PENDING",
                &request,
                Some("reality:intent".into()),
            )
            .unwrap();
        writer
            .append(
                &request,
                EntryType::Observation,
                "OperationReceipt",
                "RECEIVED",
                &receipt,
                Some("reality:receipt".into()),
            )
            .unwrap();
        let recovery = reality_executor(
            journal.clone(),
            RejectProvider,
            serde_json::json!({"http_status": 200, "version": "v2"}),
            false,
            "health-verifier",
        );
        assert_eq!(recovery.recover().await.unwrap(), vec![receipt]);
        assert!(journal
            .read_workload("workload-1")
            .unwrap()
            .iter()
            .any(|entry| entry.object_type == "StepCommit"));
    }

    #[tokio::test]
    async fn evidence_tampering_prevents_verified_commit() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let executor = reality_executor(
            journal.clone(),
            DigestProvider,
            serde_json::json!({"http_status": 200, "version": "v2"}),
            true,
            "health-verifier",
        );
        assert!(executor.execute(&reality_request(200)).await.is_err());
        assert!(!journal
            .read_workload("workload-1")
            .unwrap()
            .iter()
            .any(|entry| entry.object_type == "StepCommit"));
    }

    #[tokio::test]
    async fn independent_verifier_cannot_be_the_executor() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let executor = reality_executor(
            journal.clone(),
            DigestProvider,
            serde_json::json!({"http_status": 200, "version": "v2"}),
            false,
            "digest-v1",
        );
        let error = executor.execute(&reality_request(200)).await.unwrap_err();
        assert!(error.to_string().contains("independent verifier"));
        assert!(!journal
            .read_workload("workload-1")
            .unwrap()
            .iter()
            .any(|entry| entry.object_type == "StepCommit"));
    }

    #[tokio::test]
    async fn kernel_owned_executor_identity_rejects_same_verifier_before_execution() {
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let executor = reality_executor(
            journal.clone(),
            DigestProvider,
            serde_json::json!({"http_status": 200, "version": "v2"}),
            false,
            std::any::type_name::<DigestProvider>(),
        );
        let error = executor.execute(&reality_request(200)).await.unwrap_err();
        assert!(error.to_string().contains("kernel-owned executor identity"));
        assert!(journal.read_workload("workload-1").unwrap().is_empty());
    }

    #[tokio::test]
    async fn provider_rejects_unapproved_credential_reference() {
        let mut request = request();
        request.credential_env = "UNAPPROVED_TEST_CREDENTIAL".into();
        let provider = ProcessProvider::new("provider-does-not-need-to-exist");

        let error = provider
            .execute(&request, CancellationToken::default())
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("credential reference is not authorized"));
    }

    #[test]
    fn credential_values_are_rejected_before_journaling() {
        let mut request = request();
        request.credential_env = "not-an-environment-name".into();
        assert!(request.validate_sensitive_references().is_err());
    }

    #[test]
    fn endpoint_query_credentials_are_rejected_before_journaling() {
        let mut request = request();
        request.endpoint = "https://provider.invalid/v1?api_key=secret".into();
        assert!(request.validate_sensitive_references().is_err());
    }
}
