//! Process helper for crash-after-execute recovery qualification.

use async_trait::async_trait;
use nous_kernel_core::{
    digest_bytes, digest_json, BootCore, CancellationToken, KernelError, KernelRuntime,
    OperationReceipt, Provider, RealityObserver, RealityVerifier, VerificationDecision,
};
use nous_types::{
    DeliverySemantics, EffectContract, EffectExpectation, EvidenceRef, ObservedEffect,
    OperationRequest, ProviderRuntimeClass, RealityIdentity, SemanticExecutionSnapshot,
    VerificationMode, VerificationOutcome,
};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

struct CounterProvider(PathBuf);

#[async_trait]
impl Provider for CounterProvider {
    async fn execute(
        &self,
        request: &OperationRequest,
        _cancellation: CancellationToken,
    ) -> Result<OperationReceipt, KernelError> {
        let count = std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        std::fs::write(&self.0, count.to_string())
            .map_err(|error| KernelError::Provider(error.to_string()))?;
        Ok(OperationReceipt {
            operation_id: request.operation_id.clone(),
            input_digest: request.input_digest(),
            output_digest: digest_bytes(b"exit=0"),
            snapshot_digest: digest_json(&request.snapshot)?,
            provider_revision: request.snapshot.provider_revision.clone(),
            executor_identity: None,
            remote_execution: None,
            result: "exit=0".into(),
            completed_at_us: chrono::Utc::now().timestamp_micros(),
        })
    }
}

struct StaticObserver;

#[async_trait]
impl RealityObserver for StaticObserver {
    fn identity(&self) -> RealityIdentity {
        RealityIdentity {
            identity: "recovery-observer".into(),
            capability: "service.health.observe".into(),
        }
    }

    async fn observe(
        &self,
        contract: &EffectContract,
        _request: &OperationRequest,
        _receipt: &OperationReceipt,
    ) -> Result<ObservedEffect, KernelError> {
        let value = json!({"http_status": 200, "version": "v2"});
        let evidence_refs = vec![EvidenceRef {
            artifact_ref: format!("sha256:{}", digest_bytes(b"recovery-health")),
            digest: digest_bytes(b"recovery-health"),
        }];
        Ok(ObservedEffect {
            observation_id: uuid::Uuid::now_v7().to_string(),
            effect_id: contract.effect_id.clone(),
            subject: contract.expectation.subject.clone(),
            schema: contract.expectation.schema.clone(),
            evidence_digest: ObservedEffect::compute_evidence_digest(&value, &evidence_refs)
                .map_err(KernelError::RealityVerification)?,
            observed_value: value,
            observer: self.identity(),
            observed_at: chrono::Utc::now(),
            evidence_refs,
            execution_environment_digest: Some(digest_bytes(b"fault-worker-host")),
        })
    }
}

struct StaticVerifier;

#[async_trait]
impl RealityVerifier for StaticVerifier {
    fn identity(&self) -> RealityIdentity {
        RealityIdentity {
            identity: "recovery-verifier".into(),
            capability: "service.health.verify".into(),
        }
    }

    fn policy_revision(&self) -> String {
        "exact-json-v1".into()
    }

    async fn verify_evidence(&self, evidence_refs: &[EvidenceRef]) -> Result<(), KernelError> {
        if evidence_refs.len() == 1
            && evidence_refs[0].artifact_ref
                == format!("sha256:{}", digest_bytes(b"recovery-health"))
            && evidence_refs[0].digest == digest_bytes(b"recovery-health")
        {
            Ok(())
        } else {
            Err(KernelError::RealityVerification(
                "recovery evidence is unavailable or mismatched".into(),
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

fn request() -> OperationRequest {
    OperationRequest {
        operation_id: "crash-after-execute".into(),
        workload_id: "workload-crash-after-execute".into(),
        step_id: "restart-service".into(),
        backend: "fault-worker".into(),
        execution_domain: ProviderRuntimeClass::Reference,
        model: String::new(),
        endpoint: String::new(),
        credential_env: String::new(),
        provider_entrypoint: String::new(),
        input: "restart service".into(),
        delivery: DeliverySemantics::AtMostOnce,
        snapshot: SemanticExecutionSnapshot {
            model_revision: "none".into(),
            provider_revision: "counter-provider".into(),
            prompt_revision: "none".into(),
            tool_revision: "service-control-v1".into(),
            knowledge_revision: "none".into(),
            policy_revision: "fault-policy-v1".into(),
            capability_revision: "service.restart-v1".into(),
            context_revision: "fault-context-v1".into(),
        },
        timeout_ms: 5_000,
        effect_contract: Some(EffectContract {
            schema_version: 1,
            effect_id: "effect-crash-after-execute".into(),
            target: "service:test".into(),
            expectation: EffectExpectation {
                schema: "apeir.service-health/v1".into(),
                subject: "service:test".into(),
                expected_value: json!({"http_status": 200, "version": "v2"}),
                evidence_requirement: vec!["health".into()],
            },
            verification: VerificationMode::Independent,
        }),
    }
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let mode = arguments.get(1).expect("mode is required");
    let journal = arguments.get(2).expect("journal path is required");
    let counter = arguments.get(3).expect("counter path is required");
    let core = Arc::new(BootCore::open(journal).expect("open journal"));
    let runtime = KernelRuntime::new(core, CounterProvider(counter.into()))
        .with_reality_verification(Arc::new(StaticObserver), Arc::new(StaticVerifier));
    match mode.as_str() {
        "execute" => {
            runtime.execute(&request()).await.expect("execute effect");
        }
        "recover" => {
            let recovered = runtime.recover().await.expect("recover effect");
            assert_eq!(recovered.len(), 1);
        }
        _ => panic!("unknown mode"),
    }
}
