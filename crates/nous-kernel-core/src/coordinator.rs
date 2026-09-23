use crate::{
    assess_compatibility, BootCore, CancellationToken, CompatibilityReport, ContinuityExecution,
    ContinuityPlan, DurableExecutor, FailoverRecord, KernelError, KernelState, ModelCompatibility,
    NodeTrustResolver, OperationReceipt, OperationRequest, Provider, ProviderProbeReport,
    ProviderProbeRequest, RealityAdapterRegistry, RealityObserver, RealityVerifier,
};
use nous_resource::admission::{AdmissionController, AdmissionDecision};
use nous_resource::{FencedLease, LeaseManager};
use nous_scheduler::placement::{DevicePlacementInfo, EnginePlacementInfo, NodeInfo};
use nous_scheduler::{DecisionTrace, SchedulerCore, SchedulerCoreConfig};
use nous_state::journal::EntryType;
use nous_state::{ExecutionProfile, ExecutionProfileStore};
use nous_types::resource::{ResourceDomain, ResourceVector};
use nous_types::workload::WorkloadSpec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeExecution {
    pub workload_id: String,
    #[serde(flatten)]
    pub receipt: OperationReceipt,
    pub decision_trace: DecisionTrace,
}

pub struct KernelRuntime<P: Provider> {
    core: Arc<BootCore>,
    executor: DurableExecutor<P>,
    admission: AdmissionController,
    scheduler: SchedulerCore,
    leases: LeaseManager,
    active: Mutex<HashMap<String, ActiveOperation>>,
}

struct ActiveOperation {
    operation_id: String,
    request: OperationRequest,
    cancellation: CancellationToken,
}

impl<P: Provider> KernelRuntime<P> {
    pub fn new(core: Arc<BootCore>, provider: P) -> Self {
        let capacity = ResourceVector {
            cpu_cores_millis: 4_000,
            ram_bytes: 8 * 1024 * 1024 * 1024,
            network_bandwidth_bps: u64::MAX,
            ..ResourceVector::default()
        };
        let admission = AdmissionController::new(ResourceDomain {
            domain_id: "local-kernel".into(),
            parent_domain_id: None,
            capacity,
            allocated: ResourceVector::default(),
            reserved: ResourceVector::default(),
            hard_limits: Default::default(),
            soft_limits: Default::default(),
        });
        Self {
            executor: DurableExecutor::new(core.journal.clone(), provider),
            core,
            admission,
            scheduler: SchedulerCore::new(SchedulerCoreConfig::default()),
            leases: LeaseManager::new(30),
            active: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_reality_verification(
        mut self,
        observer: Arc<dyn RealityObserver>,
        verifier: Arc<dyn RealityVerifier>,
    ) -> Self {
        self.executor.configure_reality_registry(
            Arc::new(crate::FixedRealityRegistry { observer, verifier }),
            None,
        );
        self
    }

    pub fn with_reality_registry(
        mut self,
        registry: Arc<dyn RealityAdapterRegistry>,
        node_trust: Option<Arc<dyn NodeTrustResolver>>,
    ) -> Self {
        self.executor
            .configure_reality_registry(registry, node_trust);
        self
    }

    pub fn reality_adapters(&self) -> Vec<crate::RealityAdapterDescriptor> {
        self.executor.reality_descriptors()
    }

    pub async fn execute(
        &self,
        request: &OperationRequest,
    ) -> Result<RuntimeExecution, KernelError> {
        self.execute_internal(request, false).await
    }

    async fn execute_internal(
        &self,
        request: &OperationRequest,
        recovered: bool,
    ) -> Result<RuntimeExecution, KernelError> {
        let admitted_started = Instant::now();
        request
            .validate_sensitive_references()
            .map_err(KernelError::Admission)?;
        if self.core.state() == KernelState::ShuttingDown {
            return Err(KernelError::ShuttingDown);
        }
        self.executor.preflight_reality(request)?;
        self.executor.validate_intent(request)?;
        if let Some(receipt) = self.executor.committed_receipt(request)? {
            return Ok(RuntimeExecution {
                workload_id: request.workload_id.clone(),
                receipt,
                decision_trace: self
                    .latest_decision_trace(&request.operation_id)?
                    .unwrap_or_else(replay_trace),
            });
        }

        let workload = workload_for(request);
        match self.admission.admit(&workload) {
            AdmissionDecision::Admitted => {}
            AdmissionDecision::Rejected { reason, .. }
            | AdmissionDecision::Infeasible { reason, .. } => {
                return Err(KernelError::Admission(reason));
            }
        }

        let node = reference_node(request);
        let (placement, decision_trace) = self.scheduler.place_with_trace(&workload, &[node]);
        let placement = placement.ok_or_else(|| {
            KernelError::Scheduling("no provider placement passed hard constraints".into())
        })?;
        self.executor.append(
            request,
            EntryType::Observation,
            "SchedulerDecision",
            "SELECTED",
            &decision_trace,
            Some(format!("operation:{}:schedule", request.operation_id)),
        )?;

        let fenced_lease = self
            .leases
            .grant_fenced(
                &request.workload_id,
                "kernel-runtime",
                placement.resources_reserved,
                &placement.node_id,
                &placement.device_id,
                true,
            )
            .map_err(|error| KernelError::Resource(error.to_string()))?;
        if let Err(error) = self.executor.append(
            request,
            EntryType::Commit,
            "ResourceLease",
            "ACTIVE",
            &fenced_lease,
            Some(format!(
                "operation:{}:lease:{}:grant",
                request.operation_id, fenced_lease.lease.lease_id
            )),
        ) {
            let _ = self.leases.release(&fenced_lease.lease.lease_id);
            return Err(error);
        }
        if !self
            .leases
            .authorize(&fenced_lease.lease.lease_id, &fenced_lease.fencing_token)
        {
            let _ = self.leases.release(&fenced_lease.lease.lease_id);
            return Err(KernelError::Resource(
                "new resource lease failed fencing validation".into(),
            ));
        }

        let cancellation = CancellationToken::default();
        {
            let mut active = match self.active.lock() {
                Ok(active) => active,
                Err(_) => {
                    let _ = self.leases.release(&fenced_lease.lease.lease_id);
                    return Err(KernelError::Resource(
                        "active operation lock is poisoned".into(),
                    ));
                }
            };
            active.insert(
                request.workload_id.clone(),
                ActiveOperation {
                    operation_id: request.operation_id.clone(),
                    request: request.clone(),
                    cancellation: cancellation.clone(),
                },
            );
        }

        let execution_started = Instant::now();
        let queue_latency_ms =
            u64::try_from(admitted_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let execution = self
            .executor
            .execute_cancellable(request, cancellation)
            .await;
        let mut bookkeeping_error = None;
        let profile = ExecutionProfile {
            operation_id: request.operation_id.clone(),
            workload_id: request.workload_id.clone(),
            task_type: "operation".into(),
            task_class: format!("{:?}", workload.scheduling_class).to_lowercase(),
            model: request.model.clone(),
            provider: request.snapshot.provider_revision.clone(),
            execution_domain: format!("{:?}", request.execution_domain).to_lowercase(),
            device: placement.device_id.clone(),
            latency_ms: u64::try_from(execution_started.elapsed().as_millis()).unwrap_or(u64::MAX),
            queue_latency_ms,
            state_transfer_ms: None,
            recovery_latency_ms: recovered.then_some(queue_latency_ms),
            cost_microcents: None,
            failure_code: execution.as_ref().err().map(|error| error.code().into()),
            retry_count: 0,
            recovered,
            quality_signal: None,
            recorded_at_us: 0,
        };
        if let Err(error) = ExecutionProfileStore::record(&self.core.journal, profile) {
            bookkeeping_error = Some(KernelError::Journal(error.to_string()));
        }
        match self.active.lock() {
            Ok(mut active) => {
                active.remove(&request.workload_id);
            }
            Err(_) => {
                bookkeeping_error = Some(KernelError::Resource(
                    "active operation lock is poisoned".into(),
                ));
            }
        }
        if let Err(error) = &execution {
            if let Err(error) = self.executor.append(
                request,
                EntryType::Abort,
                "Operation",
                error.code(),
                &serde_json::json!({
                    "code": error.code(),
                    "category": error.category(),
                    "retryable": error.retryable(),
                    "source": error.source_component(),
                }),
                Some(format!(
                    "operation:{}:abort:{}",
                    request.operation_id, request.snapshot.provider_revision
                )),
            ) {
                bookkeeping_error = Some(error);
            }
            if let Err(error) = self.executor.append(
                request,
                EntryType::Observation,
                "OperationFailure",
                error.code(),
                &serde_json::json!({
                    "code": error.code(),
                    "category": error.category(),
                    "retryable": error.retryable(),
                    "source": error.source_component(),
                }),
                Some(format!("operation:{}:failure", request.operation_id)),
            ) {
                bookkeeping_error = Some(error);
            }
        }

        let (entry_type, phase) = match &execution {
            Ok(_) => (EntryType::Commit, "COMPLETED"),
            Err(KernelError::Cancelled(_)) => (EntryType::Commit, "CANCELLED"),
            Err(_) => (EntryType::Transition, "FAILED"),
        };
        if let Err(error) = self.executor.append(
            request,
            entry_type,
            "Workload",
            phase,
            &serde_json::json!({"workload_id": request.workload_id, "phase": phase}),
            Some(format!(
                "operation:{}:workload:{phase}",
                request.operation_id
            )),
        ) {
            bookkeeping_error.get_or_insert(error);
        }

        if let Err(error) = self
            .leases
            .release(&fenced_lease.lease.lease_id)
            .map_err(|error| KernelError::Resource(error.to_string()))
        {
            bookkeeping_error.get_or_insert(error);
        }
        crate::fault::trigger(
            crate::FaultPoint::ResourceAfterRelease,
            &request.operation_id,
        );
        if let Err(error) = self.executor.append(
            request,
            EntryType::Commit,
            "ResourceLease",
            "RELEASED",
            &serde_json::json!({"lease_id": fenced_lease.lease.lease_id}),
            Some(format!(
                "operation:{}:lease:{}:release",
                request.operation_id, fenced_lease.lease.lease_id
            )),
        ) {
            bookkeeping_error.get_or_insert(error);
        }

        if let Some(error) = bookkeeping_error {
            return Err(error);
        }

        Ok(RuntimeExecution {
            workload_id: request.workload_id.clone(),
            receipt: execution?,
            decision_trace,
        })
    }

    /// Execute a durable multi-step plan. Every candidate for a logical step
    /// shares the same operation ID, so provider replacement cannot create a
    /// second committed effect. Completed steps are replayed from the journal.
    pub async fn execute_continuously(
        &self,
        plan: &ContinuityPlan,
    ) -> Result<ContinuityExecution, KernelError> {
        if plan.workload_id.is_empty() || plan.process_identity.is_empty() {
            return Err(KernelError::Admission(
                "continuity plan requires workload and process identities".into(),
            ));
        }
        let mut completed_steps = Vec::new();
        let mut failovers = Vec::new();
        for step in &plan.steps {
            if step.candidates.is_empty() {
                return Err(KernelError::Scheduling(format!(
                    "step '{}' has no provider candidate",
                    step.step_id
                )));
            }
            let operation_id = step.candidates[0].request.operation_id.clone();
            if let Some(receipt) = self
                .executor
                .committed_receipt(&step.candidates[0].request)?
            {
                completed_steps.push(RuntimeExecution {
                    workload_id: plan.workload_id.clone(),
                    receipt,
                    decision_trace: replay_trace(),
                });
                continue;
            }
            let mut last_failure: Option<(String, KernelError)> = None;
            let mut last_rejection: Option<KernelError> = None;
            let mut last_outcome_was_rejection = false;
            let mut completed = None;
            for candidate in &step.candidates {
                self.record_candidate_outcome(
                    &candidate.request,
                    &candidate.capabilities.provider,
                    "EVALUATING",
                    "candidate entered compatibility pipeline",
                )?;
                validate_continuity_candidate(
                    plan,
                    &step.step_id,
                    &operation_id,
                    &candidate.request,
                    &candidate.capabilities,
                )?;
                let effective_capabilities = if candidate.capabilities.runtime_class
                    == crate::ProviderRuntimeClass::Reference
                {
                    candidate.capabilities.clone()
                } else {
                    let probe = ProviderProbeRequest {
                        provider: candidate.capabilities.provider.clone(),
                        backend: candidate.request.backend.clone(),
                        execution_domain: candidate.request.execution_domain,
                        model: candidate.request.model.clone(),
                        endpoint: candidate.request.endpoint.clone(),
                        credential_env: candidate.request.credential_env.clone(),
                        provider_entrypoint: candidate.request.provider_entrypoint.clone(),
                        timeout_ms: candidate.request.timeout_ms,
                    };
                    match self.probe_provider(&probe).await {
                        Ok(report) => report.capabilities,
                        Err(error) if error.retryable() => {
                            self.record_candidate_outcome(
                                &candidate.request,
                                &candidate.capabilities.provider,
                                "UNAVAILABLE",
                                error.code(),
                            )?;
                            last_failure = Some((candidate.capabilities.provider.clone(), error));
                            last_outcome_was_rejection = false;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                };
                let mut compatibility =
                    assess_compatibility(&step.requirements, &effective_capabilities);
                if compatibility.classification == ModelCompatibility::Incompatible
                    || (compatibility.classification == ModelCompatibility::Degraded
                        && !step.allow_degraded)
                {
                    self.record_candidate_outcome(
                        &candidate.request,
                        &effective_capabilities.provider,
                        "INCOMPATIBLE",
                        &compatibility.reasons.join(", "),
                    )?;
                    last_rejection = Some(KernelError::Scheduling(format!(
                        "provider '{}' failed compatibility gate: {}",
                        effective_capabilities.provider,
                        compatibility.reasons.join(", ")
                    )));
                    last_outcome_was_rejection = true;
                    continue;
                }
                if last_failure.is_some()
                    && compatibility.classification == ModelCompatibility::Compatible
                {
                    compatibility.classification = ModelCompatibility::Rebindable;
                    compatibility.reasons.push(
                        "provider rebind required; kernel state remains authoritative".into(),
                    );
                }
                if let Some((failed_provider, error)) = &last_failure {
                    self.record_rebind(&candidate.request, failed_provider, error, &compatibility)?;
                    failovers.push(FailoverRecord {
                        step_id: step.step_id.clone(),
                        from_provider: failed_provider.clone(),
                        to_provider: effective_capabilities.provider.clone(),
                        failure_code: error.code().into(),
                        compatibility: compatibility.clone(),
                    });
                }
                match self.execute(&candidate.request).await {
                    Ok(execution) => {
                        completed = Some(execution);
                        break;
                    }
                    Err(error) if error.retryable() => {
                        self.record_candidate_outcome(
                            &candidate.request,
                            &candidate.capabilities.provider,
                            "EXECUTION_FAILED",
                            error.code(),
                        )?;
                        last_failure = Some((candidate.capabilities.provider.clone(), error));
                        last_outcome_was_rejection = false;
                    }
                    Err(error) => return Err(error),
                }
            }
            match completed {
                Some(execution) => completed_steps.push(execution),
                None => {
                    let error = if last_outcome_was_rejection {
                        last_rejection.or_else(|| last_failure.map(|(_, error)| error))
                    } else {
                        last_failure.map(|(_, error)| error).or(last_rejection)
                    };
                    return Err(error.unwrap_or_else(|| {
                        KernelError::Scheduling(format!(
                            "step '{}' has no compatible provider after evaluating {} candidates",
                            step.step_id,
                            step.candidates.len()
                        ))
                    }));
                }
            }
        }
        Ok(ContinuityExecution {
            workload_id: plan.workload_id.clone(),
            process_identity: plan.process_identity.clone(),
            completed_steps,
            failovers,
        })
    }

    fn record_rebind(
        &self,
        request: &OperationRequest,
        from_provider: &str,
        failure: &KernelError,
        compatibility: &CompatibilityReport,
    ) -> Result<(), KernelError> {
        self.executor.append(
            request,
            EntryType::Intent,
            "Operation",
            "REBOUND",
            request,
            Some(format!(
                "operation:{}:rebind:{}",
                request.operation_id, request.snapshot.provider_revision
            )),
        )?;
        self.executor.append(
            request,
            EntryType::Observation,
            "ProviderRebind",
            "SELECTED",
            &serde_json::json!({
                "from_provider": from_provider,
                "to_provider": request.snapshot.provider_revision,
                "failure_code": failure.code(),
                "compatibility": compatibility,
            }),
            Some(format!(
                "operation:{}:provider-rebind:{}",
                request.operation_id, request.snapshot.provider_revision
            )),
        )?;
        Ok(())
    }

    fn record_candidate_outcome(
        &self,
        request: &OperationRequest,
        provider: &str,
        phase: &str,
        reason: &str,
    ) -> Result<(), KernelError> {
        self.executor.append(
            request,
            EntryType::Observation,
            "ProviderCandidate",
            phase,
            &serde_json::json!({
                "provider": provider,
                "reason": reason,
            }),
            Some(format!(
                "operation:{}:candidate:{}:{phase}",
                request.operation_id, provider
            )),
        )?;
        Ok(())
    }

    pub fn cancel(&self, workload_id: &str) -> Result<String, KernelError> {
        let (operation_id, request, cancellation) = {
            let active = self
                .active
                .lock()
                .map_err(|_| KernelError::Resource("active operation lock is poisoned".into()))?;
            let operation = active
                .get(workload_id)
                .ok_or_else(|| KernelError::Cancelled(format!("{workload_id} is not active")))?;
            (
                operation.operation_id.clone(),
                operation.request.clone(),
                operation.cancellation.clone(),
            )
        };
        self.executor.append(
            &request,
            EntryType::Transition,
            "Workload",
            "CANCELLATION_REQUESTED",
            &serde_json::json!({
                "workload_id": workload_id,
                "operation_id": operation_id,
            }),
            Some(format!("operation:{}:cancellation-requested", operation_id)),
        )?;
        cancellation.cancel();
        Ok(operation_id)
    }

    pub async fn recover(&self) -> Result<Vec<RuntimeExecution>, KernelError> {
        let mut recovered = Vec::new();
        self.revoke_orphaned_leases()?;
        for request in self.executor.pending_operations()? {
            if self.executor.received_receipt(&request)?.is_some() {
                // Physical execution is already durable. Resume observation and
                // verification; never call the provider a second time.
                recovered.push(self.execute_internal(&request, true).await?);
                continue;
            }
            match request.delivery.recovery_strategy() {
                nous_types::RecoveryStrategy::Replay => {
                    recovered.push(self.execute_internal(&request, true).await?);
                }
                _ => {
                    self.executor.record_recovery_required(
                        &request,
                        "no durable receipt exists and delivery semantics forbid replay",
                    )?;
                }
            }
        }
        Ok(recovered)
    }

    fn revoke_orphaned_leases(&self) -> Result<(), KernelError> {
        let entries = self
            .core
            .journal
            .read_from(0)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        let mut requests = HashMap::<String, OperationRequest>::new();
        let mut active = HashMap::<String, (String, FencedLease)>::new();
        for entry in entries {
            if entry.object_type == "Operation" && entry.entry_type == EntryType::Intent {
                if let Ok(request) = serde_json::from_slice::<OperationRequest>(&entry.payload) {
                    requests.insert(request.workload_id.clone(), request);
                }
                continue;
            }
            if entry.object_type != "ResourceLease" {
                continue;
            }
            if entry.new_phase == "ACTIVE" {
                if let Ok(lease) = serde_json::from_slice::<FencedLease>(&entry.payload) {
                    active.insert(
                        lease.lease.lease_id.clone(),
                        (entry.workload_id.clone(), lease),
                    );
                }
            } else if matches!(entry.new_phase.as_str(), "RELEASED" | "REVOKED" | "EXPIRED") {
                if let Some(lease_id) = serde_json::from_slice::<serde_json::Value>(&entry.payload)
                    .ok()
                    .and_then(|payload| payload["lease_id"].as_str().map(str::to_owned))
                {
                    active.remove(&lease_id);
                }
            }
        }
        for (lease_id, (workload_id, lease)) in active {
            let request = requests.get(&workload_id).ok_or_else(|| {
                KernelError::Journal(format!(
                    "orphaned lease {lease_id} has no durable operation intent"
                ))
            })?;
            self.executor.append(
                request,
                EntryType::Commit,
                "ResourceLease",
                "REVOKED",
                &serde_json::json!({
                    "lease_id": lease_id,
                    "fencing_token": lease.fencing_token,
                    "reason": "runtime recovery invalidated an orphaned lease",
                }),
                Some(format!(
                    "operation:{}:lease:{}:recovery-revoke",
                    request.operation_id, lease.lease.lease_id
                )),
            )?;
        }
        Ok(())
    }

    pub async fn shutdown(&self, timeout: Duration) -> Result<(), KernelError> {
        self.core.shutdown();
        {
            let active = self
                .active
                .lock()
                .map_err(|_| KernelError::Resource("active operation lock is poisoned".into()))?;
            for operation in active.values() {
                operation.cancellation.cancel();
            }
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let empty = self
                .active
                .lock()
                .map_err(|_| KernelError::Resource("active operation lock is poisoned".into()))?
                .is_empty();
            if empty {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(KernelError::ShuttingDown);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub fn active_count(&self) -> Result<usize, KernelError> {
        Ok(self
            .active
            .lock()
            .map_err(|_| KernelError::Resource("active operation lock is poisoned".into()))?
            .len())
    }

    pub async fn probe_provider(
        &self,
        request: &ProviderProbeRequest,
    ) -> Result<ProviderProbeReport, KernelError> {
        request
            .validate_sensitive_references()
            .map_err(KernelError::Admission)?;
        self.executor.provider.probe(request).await
    }

    pub fn latest_decision_trace(
        &self,
        operation_id: &str,
    ) -> Result<Option<DecisionTrace>, KernelError> {
        let entries = self
            .core
            .journal
            .read_from(0)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        Ok(entries
            .iter()
            .rev()
            .find(|entry| {
                entry.object_type == "SchedulerDecision" && entry.object_id == operation_id
            })
            .and_then(|entry| serde_json::from_slice(&entry.payload).ok()))
    }

    pub fn latest_decision_trace_any(&self) -> Result<Option<DecisionTrace>, KernelError> {
        let entries = self
            .core
            .journal
            .read_from(0)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        Ok(entries
            .iter()
            .rev()
            .find(|entry| entry.object_type == "SchedulerDecision")
            .and_then(|entry| serde_json::from_slice(&entry.payload).ok()))
    }

    pub fn latest_decision_trace_for_workload(
        &self,
        workload_id: &str,
    ) -> Result<Option<DecisionTrace>, KernelError> {
        let entries = self
            .core
            .journal
            .read_workload(workload_id)
            .map_err(|error| KernelError::Journal(error.to_string()))?;
        Ok(entries
            .iter()
            .rev()
            .find(|entry| entry.object_type == "SchedulerDecision")
            .and_then(|entry| serde_json::from_slice(&entry.payload).ok()))
    }
}

fn validate_continuity_candidate(
    plan: &ContinuityPlan,
    step_id: &str,
    operation_id: &str,
    request: &OperationRequest,
    capabilities: &crate::ModelCapabilityManifest,
) -> Result<(), KernelError> {
    if request.workload_id != plan.workload_id
        || request.step_id != step_id
        || request.operation_id != operation_id
    {
        return Err(KernelError::Admission(
            "failover candidates must preserve workload, step, and operation identities".into(),
        ));
    }
    if capabilities.backend != request.backend
        || (!request.model.is_empty() && capabilities.model != request.model)
    {
        return Err(KernelError::Admission(
            "candidate request does not match its capability manifest".into(),
        ));
    }
    Ok(())
}

fn workload_for(request: &OperationRequest) -> WorkloadSpec {
    let mut workload = WorkloadSpec {
        workload_id: request.workload_id.clone(),
        idempotency_key: request.operation_id.clone(),
        principal_id: "kernel-runtime".into(),
        namespace: "default".into(),
        goal: request.input.clone(),
        ..WorkloadSpec::default()
    };
    workload.resource_requirements.minimum = ResourceVector {
        cpu_cores_millis: 1,
        ram_bytes: 1024 * 1024,
        ..ResourceVector::default()
    };
    if !request.model.is_empty() {
        workload
            .model_requirements
            .allowed_model_families
            .push(request.model.clone());
    }
    workload
}

fn reference_node(request: &OperationRequest) -> NodeInfo {
    let capacity = ResourceVector {
        cpu_cores_millis: 4_000,
        ram_bytes: 8 * 1024 * 1024 * 1024,
        network_bandwidth_bps: u64::MAX,
        ..ResourceVector::default()
    };
    NodeInfo {
        node_id: "local-reference".into(),
        hostname: "localhost".into(),
        arch: std::env::consts::ARCH.into(),
        os: std::env::consts::OS.into(),
        capacity,
        allocatable: capacity,
        devices: vec![DevicePlacementInfo {
            device_id: "host-cpu".into(),
            device_type: "cpu".into(),
            vendor: "host".into(),
            total_vram: 0,
            free_vram: 0,
            utilization: 0.0,
            temperature_celsius: 0.0,
            has_kv_cache_for_model: false,
        }],
        engines: vec![EnginePlacementInfo {
            engine_id: request.backend.clone(),
            engine_type: "provider-worker".into(),
            loaded_models: vec![if request.model.is_empty() {
                request.backend.clone()
            } else {
                request.model.clone()
            }],
            active_requests: 0,
            max_batch_size: 1,
            supports_streaming: false,
            capabilities: vec!["chat".into()],
            healthy: true,
            queue_depth: 0,
            model_load_ms: 0,
            state_transfer_ms: 0,
            estimated_execution_ms: 1,
            estimated_tps: 1.0,
            kv_cache_hit_rate: 0.0,
        }],
        network_latency_us: 0,
        is_local: true,
        connection_state: nous_scheduler::NodeConnectionState::Connected,
    }
}

fn replay_trace() -> DecisionTrace {
    DecisionTrace {
        policy: "JournalReplay".into(),
        candidates: vec![],
        rejections: vec![],
        selected: None,
        selected_provider: None,
        selected_model: None,
        fallback_candidate: None,
        capability_match: "replayed from durable committed receipt".into(),
        reason: "returned a previously committed operation receipt".into(),
        expected_completion_ms: None,
        expected_completion: None,
        resource_snapshot: ResourceVector::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CapabilitySupport, ContinuityStep, DeliverySemantics, FailoverCandidate,
        ModelCapabilityManifest, ModelCompatibilityRequirements, OperationReceipt,
        ProviderRuntimeClass, SemanticExecutionSnapshot,
    };
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct BlockingProvider;

    #[async_trait]
    impl Provider for BlockingProvider {
        async fn execute(
            &self,
            request: &OperationRequest,
            cancellation: CancellationToken,
        ) -> Result<OperationReceipt, KernelError> {
            cancellation.cancelled().await;
            Err(KernelError::Cancelled(request.operation_id.clone()))
        }
    }

    fn request() -> OperationRequest {
        OperationRequest {
            operation_id: "cancel-op".into(),
            workload_id: "cancel-workload".into(),
            step_id: "step-1".into(),
            backend: "reference-delay".into(),
            execution_domain: crate::ProviderRuntimeClass::Reference,
            model: "10000".into(),
            endpoint: String::new(),
            credential_env: String::new(),
            provider_entrypoint: String::new(),
            input: "cancel me".into(),
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
            timeout_ms: 20_000,
            effect_contract: None,
        }
    }

    #[tokio::test]
    async fn cancellation_propagates_and_releases_lease() {
        let directory = tempfile::tempdir().unwrap();
        let core = Arc::new(BootCore::open(directory.path().join("journal.db")).unwrap());
        let runtime = Arc::new(KernelRuntime::new(core.clone(), BlockingProvider));
        let executing = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.execute(&request()).await })
        };
        for _ in 0..100 {
            if runtime.active_count().unwrap() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        runtime.cancel("cancel-workload").unwrap();
        let error = executing.await.unwrap().unwrap_err();
        assert!(matches!(error, KernelError::Cancelled(_)));
        assert_eq!(runtime.active_count().unwrap(), 0);
        let entries = core.journal.read_workload("cancel-workload").unwrap();
        assert!(entries
            .iter()
            .any(|entry| entry.object_type == "Workload" && entry.new_phase == "CANCELLED"));
        assert!(entries
            .iter()
            .any(|entry| entry.object_type == "ResourceLease" && entry.new_phase == "RELEASED"));
    }

    struct SwitchingProvider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Provider for SwitchingProvider {
        async fn execute(
            &self,
            request: &OperationRequest,
            _cancellation: CancellationToken,
        ) -> Result<OperationReceipt, KernelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if request.backend == "provider-a-fails" {
                return Err(KernelError::Provider("forced provider loss".into()));
            }
            Ok(OperationReceipt {
                operation_id: request.operation_id.clone(),
                input_digest: request.input_digest(),
                output_digest: crate::digest_bytes(request.backend.as_bytes()),
                snapshot_digest: crate::digest_json(&request.snapshot)?,
                provider_revision: request.snapshot.provider_revision.clone(),
                executor_identity: None,
                remote_execution: None,
                result: request.backend.clone(),
                completed_at_us: chrono::Utc::now().timestamp_micros(),
            })
        }
    }

    fn continuity_request(step: &str, backend: &str, provider: &str) -> OperationRequest {
        OperationRequest {
            operation_id: format!("continuity-{step}"),
            workload_id: "continuity-workload".into(),
            step_id: step.into(),
            backend: backend.into(),
            execution_domain: crate::ProviderRuntimeClass::Reference,
            model: provider.into(),
            endpoint: String::new(),
            credential_env: String::new(),
            provider_entrypoint: String::new(),
            input: format!("input-{step}"),
            delivery: DeliverySemantics::Idempotent,
            snapshot: SemanticExecutionSnapshot {
                model_revision: provider.into(),
                provider_revision: provider.into(),
                prompt_revision: "prompt-v1".into(),
                tool_revision: "none".into(),
                knowledge_revision: "state-v1".into(),
                policy_revision: "deterministic-v1".into(),
                capability_revision: "capability-v1".into(),
                context_revision: "context-v1".into(),
            },
            timeout_ms: 1_000,
            effect_contract: None,
        }
    }

    fn capabilities(provider: &str, backend: &str) -> ModelCapabilityManifest {
        ModelCapabilityManifest {
            schema_version: 1,
            provider: provider.into(),
            backend: backend.into(),
            model: provider.into(),
            runtime_class: ProviderRuntimeClass::Reference,
            chat: CapabilitySupport::Supported,
            streaming: CapabilitySupport::Unknown,
            tools: CapabilitySupport::Supported,
            structured_output: CapabilitySupport::Supported,
            vision: CapabilitySupport::Unsupported,
            embedding: CapabilitySupport::Unsupported,
            reasoning: CapabilitySupport::Supported,
            local_execution: CapabilitySupport::Supported,
            remote_execution: CapabilitySupport::Unsupported,
            context_length: Some(32_768),
            privacy_local: true,
            privacy: crate::ModelPrivacy::LocalOnly,
            estimated_latency_ms: Some(1),
            estimated_cost_microcents: Some(0),
            probed_at_us: 0,
        }
    }

    #[tokio::test]
    async fn failover_preserves_logical_identity_and_committed_steps() {
        let directory = tempfile::tempdir().unwrap();
        let core = Arc::new(BootCore::open(directory.path().join("journal.db")).unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = KernelRuntime::new(
            core.clone(),
            SwitchingProvider {
                calls: calls.clone(),
            },
        );
        let requirements = ModelCompatibilityRequirements {
            chat: true,
            tools: true,
            local_only: true,
            ..Default::default()
        };
        let plan = ContinuityPlan {
            workload_id: "continuity-workload".into(),
            process_identity: "agent-process-1".into(),
            steps: vec![
                ContinuityStep {
                    step_id: "step-1".into(),
                    requirements: requirements.clone(),
                    allow_degraded: false,
                    candidates: vec![FailoverCandidate {
                        request: continuity_request("step-1", "provider-a", "provider-a"),
                        capabilities: capabilities("provider-a", "provider-a"),
                    }],
                },
                ContinuityStep {
                    step_id: "step-2".into(),
                    requirements,
                    allow_degraded: false,
                    candidates: vec![
                        FailoverCandidate {
                            request: continuity_request("step-2", "provider-a-fails", "provider-a"),
                            capabilities: capabilities("provider-a", "provider-a-fails"),
                        },
                        FailoverCandidate {
                            request: continuity_request("step-2", "provider-b", "provider-b"),
                            capabilities: capabilities("provider-b", "provider-b"),
                        },
                    ],
                },
            ],
        };
        let first = runtime.execute_continuously(&plan).await.unwrap();
        assert_eq!(first.workload_id, "continuity-workload");
        assert_eq!(first.process_identity, "agent-process-1");
        assert_eq!(first.completed_steps.len(), 2);
        assert_eq!(first.failovers.len(), 1);
        assert_eq!(first.failovers[0].from_provider, "provider-a");
        assert_eq!(first.failovers[0].to_provider, "provider-b");
        assert_eq!(
            first.failovers[0].compatibility.classification,
            ModelCompatibility::Rebindable
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        let replay = runtime.execute_continuously(&plan).await.unwrap();
        assert_eq!(replay.completed_steps.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let entries = core.journal.read_workload("continuity-workload").unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.object_type == "StepCommit")
                .count(),
            2
        );
        assert!(entries
            .iter()
            .any(|entry| entry.object_type == "ProviderRebind"));
    }

    struct FailingProvider;

    #[async_trait]
    impl Provider for FailingProvider {
        async fn execute(
            &self,
            _request: &OperationRequest,
            _cancellation: CancellationToken,
        ) -> Result<OperationReceipt, KernelError> {
            Err(KernelError::Provider("observed failure".into()))
        }
    }

    struct PanicProvider;

    #[async_trait]
    impl Provider for PanicProvider {
        async fn execute(
            &self,
            _request: &OperationRequest,
            _cancellation: CancellationToken,
        ) -> Result<OperationReceipt, KernelError> {
            panic!("terminal failed operations must not be recovered")
        }
    }

    #[tokio::test]
    async fn observed_provider_failure_is_not_reexecuted_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.db");
        {
            let core = Arc::new(BootCore::open(&path).unwrap());
            let runtime = KernelRuntime::new(core, FailingProvider);
            let error = runtime.execute(&request()).await.unwrap_err();
            assert!(matches!(error, KernelError::Provider(_)));
        }
        let core = Arc::new(BootCore::open(&path).unwrap());
        let runtime = KernelRuntime::new(core, PanicProvider);
        assert!(runtime.recover().await.unwrap().is_empty());
    }

    struct ProbeRejectProvider;

    #[async_trait]
    impl Provider for ProbeRejectProvider {
        async fn execute(
            &self,
            _request: &OperationRequest,
            _cancellation: CancellationToken,
        ) -> Result<OperationReceipt, KernelError> {
            panic!("unverified supplied capabilities must not reach execution")
        }

        async fn probe(
            &self,
            request: &ProviderProbeRequest,
        ) -> Result<ProviderProbeReport, KernelError> {
            let mut actual = capabilities(&request.provider, &request.backend);
            actual.runtime_class = ProviderRuntimeClass::Local;
            actual.tools = CapabilitySupport::Unsupported;
            Ok(ProviderProbeReport {
                reachable: true,
                latency_ms: 1,
                available_models: vec![request.model.clone()],
                capabilities: actual,
                warnings: vec![],
            })
        }
    }

    #[tokio::test]
    async fn non_reference_capabilities_are_reprobed_before_scheduling() {
        let directory = tempfile::tempdir().unwrap();
        let core = Arc::new(BootCore::open(directory.path().join("journal.db")).unwrap());
        let runtime = KernelRuntime::new(core, ProbeRejectProvider);
        let mut supplied = capabilities("provider-x", "provider-x");
        supplied.runtime_class = ProviderRuntimeClass::Local;
        supplied.tools = CapabilitySupport::Supported;
        let plan = ContinuityPlan {
            workload_id: "continuity-workload".into(),
            process_identity: "agent-process-1".into(),
            steps: vec![ContinuityStep {
                step_id: "step-1".into(),
                requirements: ModelCompatibilityRequirements {
                    tools: true,
                    ..Default::default()
                },
                allow_degraded: false,
                candidates: vec![FailoverCandidate {
                    request: continuity_request("step-1", "provider-x", "provider-x"),
                    capabilities: supplied,
                }],
            }],
        };
        assert!(matches!(
            runtime.execute_continuously(&plan).await,
            Err(KernelError::Scheduling(_))
        ));
    }

    #[tokio::test]
    async fn recovered_execution_is_identified_in_profile_store() {
        let directory = tempfile::tempdir().unwrap();
        let core = Arc::new(BootCore::open(directory.path().join("journal.db")).unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = KernelRuntime::new(
            core.clone(),
            SwitchingProvider {
                calls: calls.clone(),
            },
        );
        let mut pending = continuity_request("step-1", "provider-ok", "provider-ok");
        pending.workload_id = "recovery-workload".into();
        pending.operation_id = "recovery-operation".into();
        runtime
            .executor
            .append(
                &pending,
                EntryType::Intent,
                "Operation",
                "PENDING",
                &pending,
                Some("operation:recovery-operation:intent".into()),
            )
            .unwrap();
        let recovered = runtime.recover().await.unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let profiles = ExecutionProfileStore::list(&core.journal).unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].recovered);
    }
}
