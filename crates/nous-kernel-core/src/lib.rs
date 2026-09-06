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
use nous_state::journal::{EntryType, Journal, JournalEntry};
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
}

impl KernelError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Journal(_) => "JOURNAL_ERROR",
            Self::Provider(_) => "PROVIDER_ERROR",
            Self::Serialization(_) => "SERIALIZATION_ERROR",
            Self::UnsafeRecovery(_) => "UNSAFE_RECOVERY",
            Self::Cancelled(_) => "OPERATION_CANCELLED",
            Self::DeadlineExceeded(_) => "DEADLINE_EXCEEDED",
            Self::Admission(_) => "ADMISSION_DENIED",
            Self::Scheduling(_) => "NO_FEASIBLE_PLACEMENT",
            Self::Resource(_) => "RESOURCE_ERROR",
            Self::ShuttingDown => "KERNEL_SHUTTING_DOWN",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::Journal(_) | Self::Serialization(_) => "state",
            Self::Provider(_) => "provider",
            Self::UnsafeRecovery(_)
            | Self::Cancelled(_)
            | Self::DeadlineExceeded(_)
            | Self::ShuttingDown => "execution",
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
            | Self::Cancelled(_)
            | Self::DeadlineExceeded(_)
            | Self::ShuttingDown => "nous-kernel-core",
        }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
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
                return Err(KernelError::Cancelled(operation_id.into()));
            }
            _ = tokio::time::sleep(timeout) => {
                wait_task.abort();
                let _ = wait_task.await;
                return Err(KernelError::DeadlineExceeded(operation_id.into()));
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
                    return Err(KernelError::Provider(format!(
                        "{}: {}",
                        response.error_code, response.error_message
                    )));
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
            return Err(KernelError::Provider(format!(
                "{}: {}",
                response.error_code, response.error_message
            )));
        }
        Ok(response)
    }
}

fn external_provider_request(command: &ProviderCommand) -> Result<serde_json::Value, KernelError> {
    match command {
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
            Ok(ProviderResponse {
                ok: true,
                receipt: Some(OperationReceipt {
                    operation_id: request.operation_id.clone(),
                    input_digest: request.input_digest(),
                    output_digest: digest_bytes(result.as_bytes()),
                    snapshot_digest: digest_json(&request.snapshot)?,
                    provider_revision: request.snapshot.provider_revision.clone(),
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
}

impl<P: Provider> DurableExecutor<P> {
    pub(crate) fn new(journal: Arc<Journal>, provider: P) -> Self {
        Self { journal, provider }
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

        fault::trigger(FaultPoint::EffectAfterIntent, &request.operation_id);

        if cancellation.is_cancelled() {
            return Err(KernelError::Cancelled(request.operation_id.clone()));
        }

        let receipt = match self.received_receipt(request)? {
            Some(receipt) => receipt,
            None => {
                fault::trigger(FaultPoint::EffectBeforeExecute, &request.operation_id);
                let receipt = self.provider.execute(request, cancellation).await?;
                fault::trigger(FaultPoint::EffectAfterExecute, &request.operation_id);
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

        fault::trigger(FaultPoint::EffectAfterReceipt, &request.operation_id);

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

    pub(crate) fn pending_operations(&self) -> Result<Vec<OperationRequest>, KernelError> {
        let entries = self
            .journal
            .read_from(0)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        let mut pending = HashMap::new();
        for entry in entries {
            if entry.entry_type == EntryType::Intent && entry.object_type == "Operation" {
                if let Ok(request) = serde_json::from_slice::<OperationRequest>(&entry.payload) {
                    pending.insert(request.operation_id.clone(), request);
                }
            } else if (entry.entry_type == EntryType::Commit && entry.object_type == "StepCommit")
                || (entry.entry_type == EntryType::Abort && entry.object_type == "Operation")
            {
                pending.remove(&entry.object_id);
            }
        }
        Ok(pending.into_values().collect())
    }

    #[cfg(test)]
    pub(crate) async fn recover(&self) -> Result<Vec<OperationReceipt>, KernelError> {
        let mut receipts = Vec::new();
        for request in self.pending_operations()? {
            match request.delivery {
                DeliverySemantics::Idempotent | DeliverySemantics::Reconcilable => {
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
        let entries = self
            .journal
            .read_workload(&request.workload_id)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        Ok(entries
            .iter()
            .rev()
            .find(|entry| {
                entry.entry_type == EntryType::Commit
                    && entry.object_type == "StepCommit"
                    && entry.object_id == request.operation_id
            })
            .and_then(|entry| serde_json::from_slice(&entry.payload).ok()))
    }

    fn received_receipt(
        &self,
        request: &OperationRequest,
    ) -> Result<Option<OperationReceipt>, KernelError> {
        let entries = self
            .journal
            .read_workload(&request.workload_id)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        Ok(entries
            .iter()
            .rev()
            .find(|entry| {
                entry.entry_type == EntryType::Observation
                    && entry.object_type == "OperationReceipt"
                    && entry.object_id == request.operation_id
            })
            .and_then(|entry| serde_json::from_slice(&entry.payload).ok()))
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

#[cfg(test)]
mod tests {
    use super::*;

    struct DigestProvider;

    struct RejectProvider;

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
        }
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
        let journal = Arc::new(Journal::open_in_memory().unwrap());
        let executor = DurableExecutor::new(journal, DigestProvider);
        let mut unsafe_request = request();
        unsafe_request.delivery = DeliverySemantics::Unknown;
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
