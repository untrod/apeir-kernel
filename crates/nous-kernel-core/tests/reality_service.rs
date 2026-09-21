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
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

struct ServiceProcess {
    child: Child,
    port: u16,
}

impl ServiceProcess {
    fn start(status: u16, version: &str, directory: &std::path::Path) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mode = directory.join("service-mode.json");
        fs::write(
            &mode,
            json!({"status": status, "version": version}).to_string(),
        )
        .unwrap();
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/service_health.py");
        let child = Command::new("python")
            .arg(fixture)
            .arg(port.to_string())
            .arg(mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("native Python is required for the process E2E fixture");
        for _ in 0..100 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Self { child, port };
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("service fixture did not start");
    }
}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct SuccessProvider;

#[async_trait]
impl Provider for SuccessProvider {
    async fn execute(
        &self,
        request: &OperationRequest,
        _cancellation: CancellationToken,
    ) -> Result<OperationReceipt, KernelError> {
        Ok(OperationReceipt {
            operation_id: request.operation_id.clone(),
            input_digest: request.input_digest(),
            output_digest: digest_bytes(b"exit=0"),
            snapshot_digest: digest_json(&request.snapshot)?,
            provider_revision: request.snapshot.provider_revision.clone(),
            result: "exit=0".into(),
            completed_at_us: chrono::Utc::now().timestamp_micros(),
        })
    }
}

struct HttpObserver {
    port: u16,
    evidence_dir: PathBuf,
    tamper_evidence: bool,
}

fn store_evidence(directory: &std::path::Path, content: &[u8]) -> EvidenceRef {
    let digest = digest_bytes(content);
    fs::write(directory.join(&digest), content).unwrap();
    EvidenceRef {
        artifact_ref: format!("sha256:{digest}"),
        digest,
    }
}

fn get(port: u16, path: &str) -> (u16, String, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8(raw.clone()).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, body.to_string(), raw)
}

#[async_trait]
impl RealityObserver for HttpObserver {
    fn identity(&self) -> RealityIdentity {
        RealityIdentity {
            identity: "http-probe".into(),
            capability: "service.health.observe".into(),
        }
    }

    async fn observe(
        &self,
        contract: &EffectContract,
        _request: &OperationRequest,
        _receipt: &OperationReceipt,
    ) -> Result<ObservedEffect, KernelError> {
        let (status, _, health_raw) = get(self.port, "/health");
        let (_, version_body, version_raw) = get(self.port, "/version");
        let version: serde_json::Value = serde_json::from_str(&version_body).unwrap();
        let value = json!({"http_status": status, "version": version["version"]});
        let evidence_refs = vec![
            store_evidence(&self.evidence_dir, &health_raw),
            store_evidence(&self.evidence_dir, &version_raw),
        ];
        if self.tamper_evidence {
            fs::write(
                self.evidence_dir.join(&evidence_refs[0].digest),
                b"tampered",
            )
            .unwrap();
        }
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
            execution_environment_digest: Some(digest_bytes(b"process-e2e-host")),
        })
    }
}

struct ExactVerifier {
    evidence_dir: PathBuf,
}

#[async_trait]
impl RealityVerifier for ExactVerifier {
    fn identity(&self) -> RealityIdentity {
        RealityIdentity {
            identity: "exact-service-verifier".into(),
            capability: "service.health.verify".into(),
        }
    }

    fn policy_revision(&self) -> String {
        "exact-json-v1".into()
    }

    async fn verify_evidence(&self, evidence_refs: &[EvidenceRef]) -> Result<(), KernelError> {
        if evidence_refs.is_empty() {
            return Err(KernelError::RealityVerification("missing evidence".into()));
        }
        for evidence in evidence_refs {
            if evidence.artifact_ref != format!("sha256:{}", evidence.digest) {
                return Err(KernelError::RealityVerification(
                    "evidence reference is not content-addressed".into(),
                ));
            }
            let content = fs::read(self.evidence_dir.join(&evidence.digest))
                .map_err(|error| KernelError::RealityVerification(error.to_string()))?;
            if digest_bytes(&content) != evidence.digest {
                return Err(KernelError::RealityVerification(
                    "evidence content digest mismatch".into(),
                ));
            }
        }
        Ok(())
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

fn request(id: &str) -> OperationRequest {
    OperationRequest {
        operation_id: id.into(),
        workload_id: format!("workload-{id}"),
        step_id: "restart-service".into(),
        backend: "test-service".into(),
        execution_domain: ProviderRuntimeClass::Reference,
        model: String::new(),
        endpoint: String::new(),
        credential_env: String::new(),
        provider_entrypoint: String::new(),
        input: "restart service".into(),
        delivery: DeliverySemantics::AtMostOnce,
        snapshot: SemanticExecutionSnapshot {
            model_revision: "none".into(),
            provider_revision: "service-provider".into(),
            prompt_revision: "none".into(),
            tool_revision: "service-control-v1".into(),
            knowledge_revision: "none".into(),
            policy_revision: "test-policy-v1".into(),
            capability_revision: "service.restart-v1".into(),
            context_revision: "test-context-v1".into(),
        },
        timeout_ms: 5_000,
        effect_contract: Some(EffectContract {
            schema_version: 1,
            effect_id: format!("effect-{id}"),
            target: "service:test".into(),
            expectation: EffectExpectation {
                schema: "apeir.service-health/v1".into(),
                subject: "service:test".into(),
                expected_value: json!({"http_status": 200, "version": "v2"}),
                evidence_requirement: vec!["http-health".into(), "http-version".into()],
            },
            verification: VerificationMode::Independent,
        }),
    }
}

#[tokio::test]
async fn false_success_is_not_committed() {
    let directory = tempfile::tempdir().unwrap();
    let service = ServiceProcess::start(502, "v2", directory.path());
    let evidence_dir = directory.path().to_path_buf();
    let core = Arc::new(BootCore::open(directory.path().join("false-success.db")).unwrap());
    let runtime = KernelRuntime::new(core.clone(), SuccessProvider).with_reality_verification(
        Arc::new(HttpObserver {
            port: service.port,
            evidence_dir: evidence_dir.clone(),
            tamper_evidence: false,
        }),
        Arc::new(ExactVerifier { evidence_dir }),
    );
    let error = runtime
        .execute(&request("false-success"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Mismatch"));
    let entries = core
        .journal
        .read_workload("workload-false-success")
        .unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry.new_phase == "VERIFICATION_MISMATCH"));
    assert!(!entries
        .iter()
        .any(|entry| entry.object_type == "StepCommit"));
}

#[tokio::test]
async fn successful_provider_with_wrong_service_version_is_not_committed() {
    let directory = tempfile::tempdir().unwrap();
    let service = ServiceProcess::start(200, "v1", directory.path());
    let evidence_dir = directory.path().to_path_buf();
    let core = Arc::new(BootCore::open(directory.path().join("wrong-version.db")).unwrap());
    let runtime = KernelRuntime::new(core.clone(), SuccessProvider).with_reality_verification(
        Arc::new(HttpObserver {
            port: service.port,
            evidence_dir: evidence_dir.clone(),
            tamper_evidence: false,
        }),
        Arc::new(ExactVerifier { evidence_dir }),
    );
    let error = runtime
        .execute(&request("wrong-version"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Mismatch"));
    let entries = core
        .journal
        .read_workload("workload-wrong-version")
        .unwrap();
    assert!(!entries
        .iter()
        .any(|entry| entry.object_type == "StepCommit"));
}

#[tokio::test]
async fn matching_service_reality_commits_with_evidence_chain() {
    let directory = tempfile::tempdir().unwrap();
    let service = ServiceProcess::start(200, "v2", directory.path());
    let evidence_dir = directory.path().to_path_buf();
    let core = Arc::new(BootCore::open(directory.path().join("success.db")).unwrap());
    let runtime = KernelRuntime::new(core.clone(), SuccessProvider).with_reality_verification(
        Arc::new(HttpObserver {
            port: service.port,
            evidence_dir: evidence_dir.clone(),
            tamper_evidence: false,
        }),
        Arc::new(ExactVerifier { evidence_dir }),
    );
    runtime.execute(&request("success")).await.unwrap();
    let entries = core.journal.read_workload("workload-success").unwrap();
    let objects: Vec<_> = entries
        .iter()
        .map(|entry| entry.object_type.as_str())
        .collect();
    assert!(objects.windows(4).any(|window| {
        window
            == [
                "OperationReceipt",
                "EffectState",
                "ObservedEffect",
                "EffectVerification",
            ]
    }));
    assert!(entries
        .iter()
        .any(|entry| entry.object_type == "StepCommit"));
}

#[tokio::test]
async fn tampered_evidence_bytes_block_verified_commit() {
    let directory = tempfile::tempdir().unwrap();
    let service = ServiceProcess::start(200, "v2", directory.path());
    let evidence_dir = directory.path().to_path_buf();
    let core = Arc::new(BootCore::open(directory.path().join("tamper.db")).unwrap());
    let runtime = KernelRuntime::new(core.clone(), SuccessProvider).with_reality_verification(
        Arc::new(HttpObserver {
            port: service.port,
            evidence_dir: evidence_dir.clone(),
            tamper_evidence: true,
        }),
        Arc::new(ExactVerifier { evidence_dir }),
    );
    let error = runtime.execute(&request("tamper")).await.unwrap_err();
    assert!(error
        .to_string()
        .contains("evidence content digest mismatch"));
    let entries = core.journal.read_workload("workload-tamper").unwrap();
    assert!(!entries
        .iter()
        .any(|entry| entry.object_type == "StepCommit"));
}

#[cfg(feature = "fault-injection")]
#[test]
fn crash_after_execute_recovers_without_replaying_provider() {
    let directory = tempfile::tempdir().unwrap();
    let journal = directory.path().join("crash.db");
    let counter = directory.path().join("provider-count.txt");
    let worker = env!("CARGO_BIN_EXE_reality_fault_worker");
    let crashed = Command::new(worker)
        .arg("execute")
        .arg(&journal)
        .arg(&counter)
        .env("NOUS_FAULT_POINT", "effect.after_receipt")
        .env("NOUS_FAULT_OPERATION", "crash-after-execute")
        .status()
        .unwrap();
    assert!(!crashed.success());
    assert_eq!(fs::read_to_string(&counter).unwrap(), "1");

    let recovered = Command::new(worker)
        .arg("recover")
        .arg(&journal)
        .arg(&counter)
        .status()
        .unwrap();
    assert!(recovered.success());
    assert_eq!(fs::read_to_string(&counter).unwrap(), "1");
}
