mod reality_service;

use nous_control_plane::{
    AssetKind, AssetSelector, ControlPlaneError, ControlPlaneStore, ListAssetsRequest,
    PutAssetRequest,
};
use nous_kernel_core::{
    digest_bytes, BootCore, ContinuityPlan, KernelError, KernelRuntime, ProcessProvider,
};
use nous_nki::envelope::{NKIErrorBody, NKIOutcome, NKIRequest, NKIResponse};
use nous_nki::methods::NKIMethods;
use nous_nki::transport::LengthPrefixedCodec;
use nous_state::{
    journal::{EntryType, JournalEntry},
    ExecutionProfileStore,
};
use nous_types::{
    ContractValidation, DeliverySemantics, ExtensionAdmissionDecision, ExtensionAdmissionRequest,
    ExtensionAuthorizationRequest, ExtensionExecutionPermit, ExtensionExecutionRequest,
    ExtensionRevocationRequest, OperationRequest, ProviderProbeRequest, SemanticExecutionSnapshot,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("inspect") {
        let journal = PathBuf::from(args.get(2).ok_or("journal path is required")?);
        let core = BootCore::open(journal)?;
        let corrupted = core.journal.verify_integrity()?;
        println!(
            "{}",
            serde_json::json!({
                "state": core.state(),
                "journal_sequence": core.journal.current_sequence()?,
                "corrupted_sequences": corrupted,
            })
        );
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("serve") {
        let journal = PathBuf::from(args.get(2).ok_or("journal path is required")?);
        let worker = PathBuf::from(args.get(3).ok_or("provider worker path is required")?);
        let address = args.get(4).map(String::as_str).unwrap_or("127.0.0.1:8771");
        let reality_config = args.get(5).map(PathBuf::from);
        return serve(journal, worker, address, reality_config).await;
    }
    if args.get(1).map(String::as_str) != Some("run-once") {
        eprintln!(
            "usage: apeird (or nousd) <serve JOURNAL WORKER [ADDRESS] [REALITY_SERVICE_CONFIG] | run-once JOURNAL WORKER INPUT | inspect JOURNAL>"
        );
        std::process::exit(2);
    }
    let journal = PathBuf::from(args.get(2).ok_or("journal path is required")?);
    let worker = PathBuf::from(args.get(3).ok_or("provider worker path is required")?);
    let input = args.get(4).ok_or("input is required")?.clone();
    let operation_id = format!("op-{}", digest_bytes(input.as_bytes()));

    let core = BootCore::open(journal)?;
    if core.state() != nous_kernel_core::KernelState::Ready {
        return Err("kernel did not reach READY".into());
    }
    let runtime = KernelRuntime::new(Arc::new(core), ProcessProvider::new(worker));
    runtime.recover().await?;
    let request = OperationRequest {
        operation_id: operation_id.clone(),
        workload_id: format!("workload-{operation_id}"),
        step_id: "step-1".into(),
        backend: "reference".into(),
        execution_domain: nous_types::ProviderRuntimeClass::Reference,
        model: String::new(),
        endpoint: String::new(),
        credential_env: String::new(),
        provider_entrypoint: String::new(),
        input,
        delivery: DeliverySemantics::Idempotent,
        snapshot: SemanticExecutionSnapshot {
            model_revision: "none".into(),
            provider_revision: "reference-v1".into(),
            prompt_revision: "prompt-v1".into(),
            tool_revision: "none".into(),
            knowledge_revision: "none".into(),
            policy_revision: "deterministic-v1".into(),
            capability_revision: "reference-v1".into(),
            context_revision: "context-v1".into(),
        },
        timeout_ms: 10_000,
        effect_contract: None,
    };
    let execution = runtime.execute(&request).await?;
    println!("{}", serde_json::to_string(&execution)?);
    runtime.shutdown(std::time::Duration::from_secs(5)).await?;
    Ok(())
}

async fn serve(
    journal: PathBuf,
    worker: PathBuf,
    address: &str,
    reality_config: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let session_token = std::env::var("NOUS_NKI_TOKEN").unwrap_or_default();
    if !session_token.is_empty() && session_token.len() < 32 {
        return Err("NOUS_NKI_TOKEN must contain at least 32 characters".into());
    }
    let session_token = Arc::new(session_token);
    let control_path = journal.with_extension("control.db");
    let reality = if let Some(path) = reality_config.as_deref() {
        Some(reality_service::load(path, &journal).await?)
    } else {
        None
    };
    let core = Arc::new(BootCore::open(journal)?);
    let control = Arc::new(ControlPlaneStore::open(control_path)?);
    let mut runtime = KernelRuntime::new(core.clone(), ProcessProvider::new(worker));
    if let Some(reality) = reality {
        runtime = runtime.with_reality_registry(reality.registry, reality.node_trust);
    }
    let runtime = Arc::new(runtime);
    runtime.recover().await?;
    let listener = TcpListener::bind(address).await?;
    if !listener.local_addr()?.ip().is_loopback() {
        return Err("NKI TCP listener must bind to a loopback address".into());
    }
    println!("READY {address}");
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let runtime = runtime.clone();
                let core = core.clone();
                let control = control.clone();
                let session_token = session_token.clone();
                tokio::spawn(async move {
                    if let Err(error) =
                        handle_connection(stream, runtime, core, control, session_token).await
                    {
                        eprintln!("NKI connection error: {error}");
                    }
                });
            }
            signal = tokio::signal::ctrl_c() => {
                signal?;
                runtime.shutdown(std::time::Duration::from_secs(5)).await?;
                break;
            }
        }
    }
    Ok(())
}

async fn handle_connection(
    mut stream: TcpStream,
    runtime: Arc<KernelRuntime<ProcessProvider>>,
    core: Arc<BootCore>,
    control: Arc<ControlPlaneStore>,
    session_token: Arc<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let mut length = [0_u8; 4];
        match stream.read_exact(&mut length).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        let body_length = u32::from_be_bytes(length) as usize;
        if body_length > 16 * 1024 * 1024 {
            return Err("NKI frame exceeds 16 MiB".into());
        }
        let mut body = vec![0_u8; body_length];
        stream.read_exact(&mut body).await?;
        let request: NKIRequest = serde_json::from_slice(&body)?;
        let outcome = if !(nous_nki::MIN_NKI_VERSION..=nous_nki::NKI_VERSION)
            .contains(&request.nki_version)
        {
            nki_error(
                "NKI_VERSION_UNSUPPORTED",
                "unsupported NKI version".into(),
                false,
            )
        } else if request.request_id.is_empty()
            || request.idempotency_key.is_empty()
            || request.principal_id.is_empty()
            || request.namespace.is_empty()
        {
            nki_error(
                "INVALID_ENVELOPE",
                "request_id, idempotency_key, principal_id, and namespace are required".into(),
                false,
            )
        } else if !session_token.is_empty()
            && !constant_time_equal(request.session_token.as_bytes(), session_token.as_bytes())
        {
            nki_error(
                "UNAUTHENTICATED",
                "local Runtime session is invalid".into(),
                false,
            )
        } else if request.deadline_us > 0
            && chrono::Utc::now().timestamp_micros() > request.deadline_us
        {
            nki_error("DEADLINE_EXCEEDED", "request deadline expired".into(), true)
        } else {
            dispatch_request(&request, &runtime, &core, &control).await?
        };
        let response = NKIResponse {
            request_id: request.request_id,
            nki_version: nous_nki::NKI_VERSION,
            server_timestamp_us: chrono::Utc::now().timestamp_micros(),
            trace_id: uuid::Uuid::now_v7().to_string(),
            outcome,
        };
        stream
            .write_all(&LengthPrefixedCodec::encode(&response)?)
            .await?;
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

async fn dispatch_request(
    request: &NKIRequest,
    runtime: &KernelRuntime<ProcessProvider>,
    core: &BootCore,
    control: &ControlPlaneStore,
) -> Result<NKIOutcome, Box<dyn std::error::Error>> {
    Ok(match request.method.as_str() {
        NKIMethods::HEALTH_CHECK => {
            let adapters = runtime.reality_adapters();
            let mut capabilities = vec![
                "nki.v1".to_string(),
                "nki.v2".to_string(),
                "nki.v3".to_string(),
                "provider.process".to_string(),
                "provider.external-process".to_string(),
                "durable.execution".to_string(),
                "mathematics.evaluate".to_string(),
                "fenced.lease".to_string(),
            ];
            if !adapters.is_empty() {
                capabilities.push("reality.effect".into());
                capabilities.push("artifact.evidence".into());
                for adapter in &adapters {
                    capabilities.push(format!("reality.observe.{}", adapter.effect_schema));
                    capabilities.push(format!("reality.verify.{}", adapter.effect_schema));
                }
                capabilities.sort();
                capabilities.dedup();
            }
            NKIOutcome::Success {
                payload: serde_json::json!({
                "state": core.state(),
                "runtime_version": env!("CARGO_PKG_VERSION"),
                "contracts": {
                    "foundation": 1,
                    "open_runtime": 1,
                    "provider_sdk": 1,
                },
                "journal_sequence": core.journal.current_sequence()?,
                "node": {
                    "identity": "local-kernel",
                    "connection": "CONNECTED",
                    "heartbeat_us": chrono::Utc::now().timestamp_micros(),
                    "capabilities": capabilities,
                    "reality_adapters": adapters,
                },
                }),
            }
        }
        NKIMethods::SUBMIT_WORKLOAD => {
            match serde_json::from_slice::<OperationRequest>(&request.payload) {
                Ok(mut operation) => {
                    if operation.effect_contract.is_some()
                        && request.nki_version < nous_nki::REALITY_EFFECT_NKI_VERSION
                    {
                        return Ok(nki_error(
                            "NKI_VERSION_UNSUPPORTED",
                            "reality effect contracts require NKI version 3".into(),
                            false,
                        ));
                    }
                    if request.deadline_us > 0 {
                        let remaining_us = request
                            .deadline_us
                            .saturating_sub(chrono::Utc::now().timestamp_micros());
                        let remaining_ms = u64::try_from(remaining_us.max(1))
                            .unwrap_or(1)
                            .saturating_add(999)
                            / 1_000;
                        operation.timeout_ms = operation.timeout_ms.max(1).min(remaining_ms);
                    }
                    match runtime.execute(&operation).await {
                        Ok(execution) => NKIOutcome::Success {
                            payload: serde_json::to_value(execution)?,
                        },
                        Err(error) => kernel_error(error),
                    }
                }
                Err(error) => nki_error("INVALID_OPERATION", error.to_string(), false),
            }
        }
        NKIMethods::ADMIT_EXTENSION => {
            match serde_json::from_slice::<ExtensionAdmissionRequest>(&request.payload) {
                Ok(extension) => match extension.validate() {
                    Ok(()) => {
                        let requested_capabilities = extension
                            .capability_requests
                            .iter()
                            .map(|item| item.capability.clone())
                            .collect::<Vec<_>>();
                        let scope_constraints = extension
                            .capability_requests
                            .iter()
                            .map(|item| (item.capability.clone(), item.scope.clone()))
                            .collect();
                        let mut executor_constraints = std::collections::BTreeMap::new();
                        executor_constraints.insert("authority".into(), "none".into());
                        executor_constraints.insert("network".into(), "deny_by_default".into());
                        if !extension.executor.is_empty() {
                            executor_constraints.insert("isolation".into(), "strict".into());
                            executor_constraints
                                .insert("executor".into(), extension.executor.clone());
                        }
                        let receipt_material = serde_json::to_vec(&serde_json::json!({
                            "extension": &extension,
                            "principal": &request.principal_id,
                            "policy": "extension-admission-v1",
                        }))?;
                        let receipt_digest = digest_bytes(&receipt_material);
                        let decision = ExtensionAdmissionDecision {
                            admitted: true,
                            extension_id: extension.extension_id.clone(),
                            normalized_digest: extension.content_digest.clone(),
                            requested_capabilities: requested_capabilities.clone(),
                            granted_capabilities: Vec::new(),
                            denied_capabilities: Vec::new(),
                            approval_required: !requested_capabilities.is_empty(),
                            executor_constraints,
                            scope_constraints,
                            policy_version: "extension-admission-v1".into(),
                            decision_reason: if requested_capabilities.is_empty() {
                                "admitted_without_runtime_authority".into()
                            } else {
                                "capability_approval_required".into()
                            },
                            receipt_id: format!("admission-{}", &receipt_digest[..24]),
                        };
                        core.journal.append(JournalEntry {
                            sequence: 0,
                            workload_id: format!("extension::{}", extension.extension_id),
                            entry_type: EntryType::Commit,
                            object_type: "ExtensionAdmissionDecision".into(),
                            object_id: decision.receipt_id.clone(),
                            previous_phase: None,
                            new_phase: if decision.approval_required {
                                "APPROVAL_REQUIRED".into()
                            } else {
                                "ADMITTED".into()
                            },
                            generation: 1,
                            payload: serde_json::to_vec(&decision)?,
                            fencing_token: extension.content_digest.clone(),
                            actor: request.principal_id.clone(),
                            idempotency_key: Some(format!(
                                "extension:{}:{}:admit:{}",
                                extension.extension_id,
                                extension.content_digest,
                                request.idempotency_key
                            )),
                            timestamp_us: 0,
                            checksum: Vec::new(),
                        })?;
                        NKIOutcome::Success {
                            payload: serde_json::to_value(decision)?,
                        }
                    }
                    Err(error) => nki_error("EXTENSION_ADMISSION_DENIED", error.to_string(), false),
                },
                Err(error) => nki_error("INVALID_EXTENSION_ADMISSION", error.to_string(), false),
            }
        }
        NKIMethods::AUTHORIZE_EXTENSION => {
            match serde_json::from_slice::<ExtensionAuthorizationRequest>(&request.payload) {
                Ok(authorization) => match authorization.validate() {
                    Ok(()) => {
                        let workload_id =
                            format!("extension::{}", authorization.admission.extension_id);
                        let prior = core
                            .journal
                            .read_workload(&workload_id)?
                            .into_iter()
                            .rev()
                            .find(|entry| {
                                entry.entry_type == EntryType::Commit
                                    && entry.object_type == "ExtensionAdmissionDecision"
                                    && entry.object_id == authorization.admission_receipt_id
                            })
                            .and_then(|entry| {
                                // Bind the full request to the persisted admission, not
                                // merely its caller-provided content digest. Reuse the
                                // original actor: admission and approval may be separate roles.
                                let material = serde_json::to_vec(&serde_json::json!({
                                    "extension": &authorization.admission,
                                    "principal": &entry.actor,
                                    "policy": "extension-admission-v1",
                                }))
                                .ok()?;
                                let digest = digest_bytes(&material);
                                if entry.object_id != format!("admission-{}", &digest[..24]) {
                                    return None;
                                }
                                serde_json::from_slice::<ExtensionAdmissionDecision>(&entry.payload)
                                    .ok()
                            });
                        match prior {
                            Some(prior)
                                if prior.extension_id == authorization.admission.extension_id
                                    && prior.normalized_digest
                                        == authorization.admission.content_digest =>
                            {
                                let requested = authorization
                                    .admission
                                    .capability_requests
                                    .iter()
                                    .map(|item| item.capability.clone())
                                    .collect::<Vec<_>>();
                                let approved = authorization.approved_capabilities.clone();
                                let remaining = requested
                                    .iter()
                                    .filter(|item| !approved.contains(item))
                                    .cloned()
                                    .collect::<Vec<_>>();
                                let receipt_material = serde_json::to_vec(&serde_json::json!({
                                    "admission_receipt": authorization.admission_receipt_id,
                                    "approval": authorization.approval_id,
                                    "approved": &approved,
                                    "digest": authorization.admission.content_digest,
                                }))?;
                                let digest = digest_bytes(&receipt_material);
                                let decision = ExtensionAdmissionDecision {
                                    admitted: true,
                                    extension_id: authorization.admission.extension_id.clone(),
                                    normalized_digest: authorization
                                        .admission
                                        .content_digest
                                        .clone(),
                                    requested_capabilities: requested,
                                    granted_capabilities: approved,
                                    denied_capabilities: Vec::new(),
                                    approval_required: !remaining.is_empty(),
                                    executor_constraints: prior.executor_constraints,
                                    scope_constraints: prior.scope_constraints,
                                    policy_version: "extension-authorization-v1".into(),
                                    decision_reason: if remaining.is_empty() {
                                        "approved_capabilities_bound".into()
                                    } else {
                                        "partial_approval_requires_review".into()
                                    },
                                    receipt_id: format!("authorization-{}", &digest[..24]),
                                };
                                core.journal.append(JournalEntry {
                                    sequence: 0,
                                    workload_id,
                                    entry_type: EntryType::Commit,
                                    object_type: "ExtensionAuthorizationDecision".into(),
                                    object_id: decision.receipt_id.clone(),
                                    previous_phase: Some("APPROVAL_REQUIRED".into()),
                                    new_phase: if decision.approval_required {
                                        "PARTIALLY_AUTHORIZED".into()
                                    } else {
                                        "AUTHORIZED".into()
                                    },
                                    generation: 2,
                                    payload: serde_json::to_vec(&decision)?,
                                    fencing_token: authorization.admission.content_digest.clone(),
                                    actor: request.principal_id.clone(),
                                    idempotency_key: Some(format!(
                                        "extension:{}:{}:authorize:{}",
                                        authorization.admission.extension_id,
                                        authorization.admission.content_digest,
                                        authorization.approval_id
                                    )),
                                    timestamp_us: 0,
                                    checksum: Vec::new(),
                                })?;
                                NKIOutcome::Success {
                                    payload: serde_json::to_value(decision)?,
                                }
                            }
                            _ => nki_error(
                                "EXTENSION_ADMISSION_NOT_FOUND",
                                "matching admission receipt and digest are required".into(),
                                false,
                            ),
                        }
                    }
                    Err(error) => {
                        nki_error("EXTENSION_AUTHORIZATION_DENIED", error.to_string(), false)
                    }
                },
                Err(error) => {
                    nki_error("INVALID_EXTENSION_AUTHORIZATION", error.to_string(), false)
                }
            }
        }
        NKIMethods::AUTHORIZE_EXTENSION_EXECUTION => {
            match serde_json::from_slice::<ExtensionExecutionRequest>(&request.payload) {
                Ok(execution) => match execution.validate() {
                    Ok(()) => {
                        let workload_id = format!("extension::{}", execution.extension_id);
                        let authorization = core
                            .journal
                            .read_workload(&workload_id)?
                            .into_iter()
                            .rev()
                            .find(|entry| {
                                entry.entry_type == EntryType::Commit
                                    && matches!(
                                        entry.object_type.as_str(),
                                        "ExtensionAuthorizationDecision"
                                            | "ExtensionRevocationDecision"
                                    )
                            })
                            .and_then(|entry| {
                                if entry.object_id != execution.authorization_receipt_id {
                                    return None;
                                }
                                serde_json::from_slice::<ExtensionAdmissionDecision>(&entry.payload)
                                    .ok()
                            });
                        match authorization {
                            Some(authorization)
                                if authorization.extension_id == execution.extension_id
                                    && authorization.normalized_digest == execution.content_digest
                                    && authorization
                                        .granted_capabilities
                                        .contains(&execution.capability) =>
                            {
                                let allowed_scope = authorization
                                    .scope_constraints
                                    .get(&execution.capability)
                                    .cloned()
                                    .unwrap_or_default();
                                let scope_allowed = execution
                                    .scope
                                    .iter()
                                    .all(|item| allowed_scope.contains(item));
                                let required_executor = authorization
                                    .executor_constraints
                                    .get("executor")
                                    .cloned()
                                    .unwrap_or_default();
                                if !scope_allowed
                                    || (!required_executor.is_empty()
                                        && execution.executor != required_executor)
                                {
                                    nki_error(
                                        "EXTENSION_EXECUTION_DENIED",
                                        "executor or scope exceeds the Kernel authorization".into(),
                                        false,
                                    )
                                } else {
                                    let material = serde_json::to_vec(&serde_json::json!({
                                        "execution": &execution,
                                        "principal": &request.principal_id,
                                        "idempotency_key": &request.idempotency_key,
                                    }))?;
                                    let digest = digest_bytes(&material);
                                    let permit = ExtensionExecutionPermit {
                                        allowed: true,
                                        operation_id: execution.operation_id.clone(),
                                        actor: request.principal_id.clone(),
                                        extension_id: execution.extension_id.clone(),
                                        content_digest: execution.content_digest.clone(),
                                        authorization_receipt_id: execution
                                            .authorization_receipt_id
                                            .clone(),
                                        capability: execution.capability.clone(),
                                        operation: execution.operation.clone(),
                                        parameter_hash: execution.parameter_hash.clone(),
                                        executor: execution.executor.clone(),
                                        policy_version: "extension-execution-v1".into(),
                                        decision_reason: "authorized_capability_bound".into(),
                                        issued_at_us: chrono::Utc::now().timestamp_micros(),
                                        permit_id: format!("extension-permit-{}", &digest[..24]),
                                    };
                                    core.journal.append(JournalEntry {
                                        sequence: 0,
                                        workload_id,
                                        entry_type: EntryType::Commit,
                                        object_type: "ExtensionExecutionPermit".into(),
                                        object_id: permit.permit_id.clone(),
                                        previous_phase: Some("AUTHORIZED".into()),
                                        new_phase: "EXECUTION_PERMITTED".into(),
                                        generation: 3,
                                        payload: serde_json::to_vec(&permit)?,
                                        fencing_token: execution.content_digest.clone(),
                                        actor: request.principal_id.clone(),
                                        idempotency_key: Some(format!(
                                            "extension:{}:{}:execute:{}",
                                            execution.extension_id,
                                            execution.content_digest,
                                            request.idempotency_key
                                        )),
                                        timestamp_us: 0,
                                        checksum: Vec::new(),
                                    })?;
                                    NKIOutcome::Success {
                                        payload: serde_json::to_value(permit)?,
                                    }
                                }
                            }
                            _ => nki_error(
                                "EXTENSION_EXECUTION_DENIED",
                                "matching authorization receipt, digest, and capability are required"
                                    .into(),
                                false,
                            ),
                        }
                    }
                    Err(error) => nki_error("EXTENSION_EXECUTION_DENIED", error.to_string(), false),
                },
                Err(error) => nki_error("INVALID_EXTENSION_EXECUTION", error.to_string(), false),
            }
        }
        NKIMethods::REVOKE_EXTENSION => {
            match serde_json::from_slice::<ExtensionRevocationRequest>(&request.payload) {
                Ok(revocation) => {
                    match revocation.validate() {
                        Ok(()) => {
                            let workload_id = format!("extension::{}", revocation.extension_id);
                            let entries = core.journal.read_workload(&workload_id)?;
                            let journal_idempotency_key = format!(
                                "extension:{}:{}:revoke:{}",
                                revocation.extension_id,
                                revocation.content_digest,
                                request.idempotency_key
                            );
                            let replay = entries.iter().rev().find(|entry| {
                                entry.entry_type == EntryType::Commit
                                    && entry.object_type == "ExtensionRevocationDecision"
                                    && entry.idempotency_key.as_deref()
                                        == Some(journal_idempotency_key.as_str())
                            });
                            if let Some(entry) = replay {
                                match serde_json::from_slice::<ExtensionAdmissionDecision>(
                                    &entry.payload,
                                ) {
                                    Ok(decision) => NKIOutcome::Success {
                                        payload: serde_json::to_value(decision)?,
                                    },
                                    Err(error) => nki_error(
                                        "EXTENSION_REVOCATION_STATE_INVALID",
                                        error.to_string(),
                                        false,
                                    ),
                                }
                            } else {
                                let prior = entries
                                    .into_iter()
                                    .rev()
                                    .find(|entry| {
                                        entry.entry_type == EntryType::Commit
                                            && matches!(
                                                entry.object_type.as_str(),
                                                "ExtensionAuthorizationDecision"
                                                    | "ExtensionRevocationDecision"
                                            )
                                    })
                                    .and_then(|entry| {
                                        if entry.object_type != "ExtensionAuthorizationDecision"
                                            || entry.object_id
                                                != revocation.authorization_receipt_id
                                        {
                                            return None;
                                        }
                                        serde_json::from_slice::<ExtensionAdmissionDecision>(
                                            &entry.payload,
                                        )
                                        .ok()
                                    });
                                match prior {
                            Some(prior)
                                if prior.extension_id == revocation.extension_id
                                    && prior.normalized_digest == revocation.content_digest =>
                            {
                                let material = serde_json::to_vec(&serde_json::json!({
                                    "authorization_receipt": revocation.authorization_receipt_id,
                                    "digest": revocation.content_digest,
                                    "principal": request.principal_id,
                                    "reason": revocation.reason,
                                }))?;
                                let digest = digest_bytes(&material);
                                let mut constraints = prior.executor_constraints;
                                constraints.insert("authority".into(), "none".into());
                                let decision = ExtensionAdmissionDecision {
                                    admitted: true,
                                    extension_id: revocation.extension_id.clone(),
                                    normalized_digest: revocation.content_digest.clone(),
                                    requested_capabilities: prior.requested_capabilities.clone(),
                                    granted_capabilities: Vec::new(),
                                    denied_capabilities: prior.granted_capabilities,
                                    approval_required: !prior.requested_capabilities.is_empty(),
                                    executor_constraints: constraints,
                                    scope_constraints: prior.scope_constraints,
                                    policy_version: "extension-revocation-v1".into(),
                                    decision_reason: "authority_revoked".into(),
                                    receipt_id: format!("revocation-{}", &digest[..24]),
                                };
                                core.journal.append(JournalEntry {
                                    sequence: 0,
                                    workload_id,
                                    entry_type: EntryType::Commit,
                                    object_type: "ExtensionRevocationDecision".into(),
                                    object_id: decision.receipt_id.clone(),
                                    previous_phase: Some("AUTHORIZED".into()),
                                    new_phase: "REVOKED".into(),
                                    generation: 4,
                                    payload: serde_json::to_vec(&decision)?,
                                    fencing_token: revocation.content_digest.clone(),
                                    actor: request.principal_id.clone(),
                                    idempotency_key: Some(journal_idempotency_key),
                                    timestamp_us: 0,
                                    checksum: Vec::new(),
                                })?;
                                NKIOutcome::Success {
                                    payload: serde_json::to_value(decision)?,
                                }
                            }
                            _ => nki_error(
                                "EXTENSION_AUTHORIZATION_NOT_FOUND",
                                "matching active authorization receipt and digest are required".into(),
                                false,
                            ),
                        }
                            }
                        }
                        Err(error) => {
                            nki_error("EXTENSION_REVOCATION_DENIED", error.to_string(), false)
                        }
                    }
                }
                Err(error) => nki_error("INVALID_EXTENSION_REVOCATION", error.to_string(), false),
            }
        }
        NKIMethods::SUBMIT_CONTINUITY_PLAN => {
            match serde_json::from_slice::<ContinuityPlan>(&request.payload) {
                Ok(plan) => match runtime.execute_continuously(&plan).await {
                    Ok(execution) => NKIOutcome::Success {
                        payload: serde_json::to_value(execution)?,
                    },
                    Err(error) => kernel_error(error),
                },
                Err(error) => nki_error("INVALID_CONTINUITY_PLAN", error.to_string(), false),
            }
        }
        NKIMethods::GET_WORKLOAD => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            match payload
                .get("workload_id")
                .and_then(serde_json::Value::as_str)
            {
                Some(workload_id) => {
                    let profile = ExecutionProfileStore::list(&core.journal)?
                        .into_iter()
                        .rev()
                        .find(|profile| profile.workload_id == workload_id);
                    match profile {
                        Some(profile) => NKIOutcome::Success {
                            payload: serde_json::to_value(profile)?,
                        },
                        None => nki_error("WORKLOAD_NOT_FOUND", workload_id.into(), false),
                    }
                }
                None => nki_error(
                    "INVALID_WORKLOAD_QUERY",
                    "workload_id is required".into(),
                    false,
                ),
            }
        }
        NKIMethods::LIST_WORKLOADS => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            let limit = payload
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(100)
                .min(1_000) as usize;
            let workloads = ExecutionProfileStore::list(&core.journal)?
                .into_iter()
                .rev()
                .take(limit)
                .collect::<Vec<_>>();
            NKIOutcome::Success {
                payload: serde_json::json!({"workloads": workloads}),
            }
        }
        NKIMethods::REGISTER_MODEL => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            let expected_generation = payload
                .get("expected_generation")
                .and_then(serde_json::Value::as_u64);
            let document = payload.get("model").cloned().unwrap_or(payload);
            match control.put(PutAssetRequest {
                schema_version: 1,
                kind: AssetKind::Model,
                document,
                expected_generation,
            }) {
                Ok(asset) => NKIOutcome::Success {
                    payload: serde_json::to_value(asset)?,
                },
                Err(error) => control_error(error),
            }
        }
        NKIMethods::VALIDATE_MODEL => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            let document = payload.get("model").unwrap_or(&payload);
            match control.validate(AssetKind::Model, document) {
                Ok((id, version, sha256)) => NKIOutcome::Success {
                    payload: serde_json::json!({"valid": true, "id": id, "version": version, "sha256": sha256}),
                },
                Err(error) => control_error(error),
            }
        }
        NKIMethods::PUT_CONTROL_ASSET => {
            match serde_json::from_slice::<PutAssetRequest>(&request.payload) {
                Ok(value) => match control.put(value) {
                    Ok(asset) => NKIOutcome::Success {
                        payload: serde_json::to_value(asset)?,
                    },
                    Err(error) => control_error(error),
                },
                Err(error) => nki_error("INVALID_CONTROL_ASSET", error.to_string(), false),
            }
        }
        NKIMethods::GET_CONTROL_ASSET => {
            match serde_json::from_slice::<AssetSelector>(&request.payload) {
                Ok(value) => match control.get(&value) {
                    Ok(asset) => NKIOutcome::Success {
                        payload: serde_json::to_value(asset)?,
                    },
                    Err(error) => control_error(error),
                },
                Err(error) => nki_error("INVALID_CONTROL_QUERY", error.to_string(), false),
            }
        }
        NKIMethods::LIST_CONTROL_ASSETS => {
            match serde_json::from_slice::<ListAssetsRequest>(&request.payload) {
                Ok(value) => match control.list(&value) {
                    Ok(assets) => NKIOutcome::Success {
                        payload: serde_json::json!({"assets": assets}),
                    },
                    Err(error) => control_error(error),
                },
                Err(error) => nki_error("INVALID_CONTROL_QUERY", error.to_string(), false),
            }
        }
        NKIMethods::DELETE_CONTROL_ASSET => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            let selector = payload
                .get("selector")
                .cloned()
                .and_then(|value| serde_json::from_value::<AssetSelector>(value).ok());
            let generation = payload
                .get("expected_generation")
                .and_then(serde_json::Value::as_u64);
            match (selector, generation) {
                (Some(selector), Some(generation)) => match control.delete(&selector, generation) {
                    Ok(()) => NKIOutcome::Success {
                        payload: serde_json::json!({"deleted": true, "kind": selector.kind, "id": selector.id}),
                    },
                    Err(error) => control_error(error),
                },
                _ => nki_error(
                    "INVALID_CONTROL_DELETE",
                    "selector and expected_generation are required".into(),
                    false,
                ),
            }
        }
        NKIMethods::GET_METRICS => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            let verify_integrity = payload
                .get("verify_integrity")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            NKIOutcome::Success {
                payload: serde_json::json!({
                "journal_sequence": core.journal.current_sequence()?,
                "corrupted_sequences": core.integrity_failures(verify_integrity)?,
                "integrity_verified_now": verify_integrity,
                "active_operations": runtime.active_count()?,
                "execution_profiles": core.journal.count_object_type("ExecutionProfile")?,
                }),
            }
        }
        NKIMethods::PROBE_ENGINE => {
            match serde_json::from_slice::<ProviderProbeRequest>(&request.payload) {
                Ok(probe) => match runtime.probe_provider(&probe).await {
                    Ok(report) => NKIOutcome::Success {
                        payload: serde_json::to_value(report)?,
                    },
                    Err(error) => kernel_error(error),
                },
                Err(error) => nki_error("INVALID_PROVIDER_PROBE", error.to_string(), false),
            }
        }
        NKIMethods::EXPLAIN_EXECUTION => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            NKIOutcome::Success {
                payload: execution_projection(core, runtime, &payload)?,
            }
        }
        NKIMethods::CANCEL_WORKLOAD => {
            let payload: serde_json::Value = serde_json::from_slice(&request.payload)?;
            match payload
                .get("workload_id")
                .and_then(serde_json::Value::as_str)
            {
                Some(workload_id) => match runtime.cancel(workload_id) {
                    Ok(operation_id) => NKIOutcome::Success {
                        payload: serde_json::json!({
                            "workload_id": workload_id,
                            "operation_id": operation_id,
                            "cancellation_requested": true,
                        }),
                    },
                    Err(error) => kernel_error(error),
                },
                None => nki_error(
                    "INVALID_CANCELLATION",
                    "workload_id is required".into(),
                    false,
                ),
            }
        }
        _ => nki_error("METHOD_NOT_SUPPORTED", request.method.clone(), false),
    })
}

fn execution_projection(
    core: &BootCore,
    runtime: &KernelRuntime<ProcessProvider>,
    selector: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let operation_id = selector
        .get("operation_id")
        .and_then(serde_json::Value::as_str);
    let workload_id = selector
        .get("workload_id")
        .and_then(serde_json::Value::as_str);
    let entries = core.journal.read_from(0)?;
    let selected = entries
        .into_iter()
        .filter(|entry| {
            operation_id.is_some_and(|value| entry.object_id == value)
                || workload_id.is_some_and(|value| entry.workload_id == value)
        })
        .collect::<Vec<_>>();
    let mut path = vec!["NKI".to_string()];
    let mut facts = Vec::new();
    let mut recovery_state = "NOT_STARTED".to_string();
    for entry in selected {
        let payload = serde_json::from_slice::<serde_json::Value>(&entry.payload)
            .unwrap_or_else(|_| serde_json::json!({"unavailable": true}));
        let projected = match entry.object_type.as_str() {
            "Operation" => serde_json::from_value::<OperationRequest>(payload)
                .map(|request| {
                    serde_json::json!({
                        "operation_id": request.operation_id,
                        "workload_id": request.workload_id,
                        "backend": request.backend,
                        "delivery_semantics": request.delivery,
                        "input_digest": request.input_digest(),
                        "effect_contract": request.effect_contract,
                    })
                })
                .unwrap_or_else(|_| serde_json::json!({"unavailable": true})),
            "OperationReceipt" | "StepCommit" => {
                serde_json::from_value::<nous_kernel_core::OperationReceipt>(payload)
                    .map(|receipt| {
                        serde_json::json!({
                            "operation_id": receipt.operation_id,
                            "input_digest": receipt.input_digest,
                            "output_digest": receipt.output_digest,
                            "snapshot_digest": receipt.snapshot_digest,
                            "provider_revision": receipt.provider_revision,
                            "executor_identity": receipt.executor_identity,
                            "remote_execution": receipt.remote_execution,
                            "completed_at_us": receipt.completed_at_us,
                        })
                    })
                    .unwrap_or_else(|_| serde_json::json!({"unavailable": true}))
            }
            "TargetBinding" => serde_json::from_value::<nous_types::TargetBinding>(payload)
                .map(|binding| {
                    serde_json::json!({
                        "target_ref": binding.target_ref,
                        "target_kind": binding.target_kind,
                        "node_id": binding.node_id,
                        "adapter_id": binding.adapter_id,
                        "adapter_revision": binding.adapter_revision,
                        "revision": binding.revision,
                        "digest": binding.digest().ok(),
                    })
                })
                .unwrap_or_else(|_| serde_json::json!({"unavailable": true})),
            _ => payload,
        };
        match (entry.object_type.as_str(), entry.new_phase.as_str()) {
            ("Operation", _) => push_stage(&mut path, "durable_intent"),
            ("TargetBinding", _) => push_stage(&mut path, "target_binding"),
            ("EffectState", "EXECUTION_DISPATCHED") => push_stage(&mut path, "provider"),
            ("OperationReceipt", _) => push_stage(&mut path, "receipt"),
            ("ObservedEffect", _) => {
                push_stage(&mut path, "observation");
                push_stage(&mut path, "evidence");
            }
            ("EffectVerification", _) => push_stage(&mut path, "verification"),
            ("StepCommit", _) => push_stage(&mut path, "step_commit"),
            _ => {}
        }
        if entry.new_phase == "RECOVERY_REQUIRED" {
            recovery_state = "RECOVERY_REQUIRED".into();
        } else if entry.object_type == "StepCommit" {
            recovery_state = "COMMITTED".into();
        } else if entry.object_type == "EffectVerification" {
            recovery_state = entry.new_phase.clone();
        } else if entry.object_type == "ObservedEffect" {
            recovery_state = "OBSERVED".into();
        } else if entry.object_type == "OperationReceipt" {
            recovery_state = "EXECUTED".into();
        } else if entry.object_type == "Operation" {
            recovery_state = "INTENT_ACCEPTED".into();
        }
        facts.push(serde_json::json!({
            "sequence": entry.sequence,
            "object_type": entry.object_type,
            "object_id": entry.object_id,
            "phase": entry.new_phase,
            "timestamp_us": entry.timestamp_us,
            "fact": projected,
        }));
    }
    let decision_trace = match workload_id {
        Some(workload_id) => runtime.latest_decision_trace_for_workload(workload_id)?,
        None => runtime.latest_decision_trace_any()?,
    };
    Ok(serde_json::json!({
        "operation_id": operation_id,
        "workload_id": workload_id,
        "execution_path": path,
        "facts": facts,
        "recovery_state": recovery_state,
        "decision_trace": decision_trace,
        "journal_sequence": core.journal.current_sequence()?,
    }))
}

fn push_stage(path: &mut Vec<String>, stage: &str) {
    if path.last().is_none_or(|existing| existing != stage) {
        path.push(stage.into());
    }
}

fn nki_error(code: &str, message: String, retryable: bool) -> NKIOutcome {
    NKIOutcome::Error {
        error: NKIErrorBody {
            code: code.into(),
            message,
            failed_phase: "EXECUTE".into(),
            cause: String::new(),
            retryable,
            recommended_delay_ms: if retryable { 250 } else { 0 },
        },
    }
}

fn kernel_error(error: KernelError) -> NKIOutcome {
    NKIOutcome::Error {
        error: NKIErrorBody {
            code: error.code().into(),
            message: error.to_string(),
            failed_phase: error.category().into(),
            cause: error.source_component().into(),
            retryable: error.retryable(),
            recommended_delay_ms: if error.retryable() { 250 } else { 0 },
        },
    }
}

fn control_error(error: ControlPlaneError) -> NKIOutcome {
    let code = match &error {
        ControlPlaneError::Invalid(_) => "CONTROL_INVALID",
        ControlPlaneError::NotFound { .. } => "CONTROL_NOT_FOUND",
        ControlPlaneError::Conflict(_) => "CONTROL_CONFLICT",
        ControlPlaneError::Storage(_) => "CONTROL_STORAGE",
    };
    nki_error(code, error.to_string(), false)
}
