//! Explicitly configured, loopback-only reference adapter for service reality.
//! No NKI request may choose a probe destination or evidence directory.

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use nous_kernel_core::{
    digest_bytes, KernelError, NodeTrustResolver, OperationReceipt, RealityAdapterDescriptor,
    RealityAdapterRegistry, RealityObserver, RealityVerifier, ResolvedRealityAdapter,
    VerificationDecision,
};
use nous_types::{
    EffectContract, EvidenceRef, ObservedEffect, OperationRequest, RealityIdentity, TargetBinding,
    VerificationOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Command;

const SERVICE_SCHEMA: &str = "apeir.service-health/v1";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RealityConfig {
    schema_version: u32,
    targets: Vec<ServiceTargetConfig>,
    artifact_bridge: ArtifactBridgeConfig,
    #[serde(default)]
    trusted_nodes_path: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactBridgeConfig {
    program: String,
    root: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceTargetConfig {
    target_binding: TargetBinding,
    subject: String,
}

struct ServiceObserver {
    address: SocketAddr,
    target: String,
    subject: String,
    node_id: String,
    evidence: Arc<dyn EvidenceArtifactRuntime>,
}

struct ServiceVerifier {
    evidence: Arc<dyn EvidenceArtifactRuntime>,
}

#[derive(Serialize)]
struct EvidenceMetadata<'a> {
    operation_id: &'a str,
    intent_id: &'a str,
    effect_id: &'a str,
    effect_contract_digest: &'a str,
    target_ref: &'a str,
    observer_identity: &'a str,
    node_identity: &'a str,
    evidence_schema: &'a str,
    observed_at: &'a str,
}

#[async_trait]
trait EvidenceArtifactRuntime: Send + Sync {
    async fn put(
        &self,
        bytes: &[u8],
        media_type: &str,
        name: &str,
        metadata: &EvidenceMetadata<'_>,
    ) -> Result<EvidenceRef, KernelError>;

    async fn resolve(&self, reference: &EvidenceRef) -> Result<Vec<u8>, KernelError>;
}

struct ArtifactBridge {
    program: String,
    root: PathBuf,
}

struct ServiceRegistry {
    adapters: HashMap<String, ResolvedRealityAdapter>,
    descriptors: Vec<RealityAdapterDescriptor>,
}

struct FileNodeTrustResolver {
    path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedNodesFile {
    schema: String,
    nodes: HashMap<String, String>,
}

impl NodeTrustResolver for FileNodeTrustResolver {
    fn public_key_hex(&self, node_id: &str) -> Result<Option<String>, KernelError> {
        let bytes = fs::read(&self.path)
            .map_err(|error| evidence_error(format!("node trust registry unavailable: {error}")))?;
        let trust: TrustedNodesFile = serde_json::from_slice(&bytes)
            .map_err(|error| evidence_error(format!("node trust registry invalid: {error}")))?;
        if trust.schema != "nous.relay-trust/v1" {
            return Err(evidence_error("node trust registry schema is unsupported"));
        }
        let key = trust.nodes.get(node_id).cloned();
        if key.as_deref().is_some_and(|value| {
            value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(evidence_error("trusted node public key is invalid"));
        }
        Ok(key)
    }
}

pub struct ConfiguredReality {
    pub registry: Arc<dyn RealityAdapterRegistry>,
    pub node_trust: Option<Arc<dyn NodeTrustResolver>>,
}

impl RealityAdapterRegistry for ServiceRegistry {
    fn resolve(&self, contract: &EffectContract) -> Result<ResolvedRealityAdapter, KernelError> {
        let adapter = self
            .adapters
            .get(&contract.target)
            .ok_or_else(|| evidence_error("no registered reality adapter for effect target"))?;
        adapter.target.admits(contract).map_err(evidence_error)?;
        if adapter.descriptor.effect_schema != contract.expectation.schema {
            return Err(evidence_error(
                "registered adapter does not support effect schema",
            ));
        }
        Ok(adapter.clone())
    }

    fn descriptors(&self) -> Vec<RealityAdapterDescriptor> {
        self.descriptors.clone()
    }
}

pub async fn load(
    config_path: &Path,
    _journal: &Path,
) -> Result<ConfiguredReality, Box<dyn std::error::Error>> {
    let config: RealityConfig = serde_json::from_slice(&fs::read(config_path)?)?;
    if config.schema_version != 2 || config.targets.is_empty() {
        return Err("reality registry config requires version 2 and targets".into());
    }
    if config.artifact_bridge.program.is_empty() {
        return Err("artifact bridge program is required".into());
    }
    let artifact_root = if config.artifact_bridge.root.is_absolute() {
        config.artifact_bridge.root
    } else {
        config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(config.artifact_bridge.root)
    };
    let bridge = Arc::new(ArtifactBridge {
        program: config.artifact_bridge.program,
        root: artifact_root,
    });
    bridge.health().await?;
    let evidence: Arc<dyn EvidenceArtifactRuntime> = bridge;
    let mut adapters = HashMap::new();
    let mut descriptors = Vec::new();
    for configured in config.targets {
        let adapter = configure_target(configured, evidence.clone())?;
        let descriptor = adapter.descriptor.clone();
        let target_ref = adapter.target.target_ref.clone();
        if adapters.insert(target_ref, adapter).is_some() {
            return Err("duplicate reality target binding".into());
        }
        if !descriptors.contains(&descriptor) {
            descriptors.push(descriptor);
        }
    }
    let node_trust = config.trusted_nodes_path.map(|path| {
        let path = if path.is_absolute() {
            path
        } else {
            config_path.parent().unwrap_or(Path::new(".")).join(path)
        };
        Arc::new(FileNodeTrustResolver { path }) as Arc<dyn NodeTrustResolver>
    });
    Ok(ConfiguredReality {
        registry: Arc::new(ServiceRegistry {
            adapters,
            descriptors,
        }),
        node_trust,
    })
}

fn configure_target(
    configured: ServiceTargetConfig,
    evidence: Arc<dyn EvidenceArtifactRuntime>,
) -> Result<ResolvedRealityAdapter, Box<dyn std::error::Error>> {
    let binding = configured.target_binding;
    binding.validate()?;
    let address: SocketAddr = binding
        .endpoint_binding
        .get("address")
        .and_then(serde_json::Value::as_str)
        .ok_or("HTTP service target requires endpoint_binding.address")?
        .parse()?;
    if binding.target_kind != "http-service"
        || binding.adapter_id != "apeir.http-service/v1"
        || binding.adapter_revision != "1"
        || configured.subject.is_empty()
        || !address.ip().is_loopback()
        || address.port() == 0
    {
        return Err(
            "HTTP reality target requires the registered adapter, subject, and a loopback socket"
                .into(),
        );
    }
    let observer: Arc<dyn RealityObserver> = Arc::new(ServiceObserver {
        address,
        target: binding.target_ref.clone(),
        subject: configured.subject,
        node_id: binding.node_id.clone(),
        evidence: evidence.clone(),
    });
    let verifier: Arc<dyn RealityVerifier> = Arc::new(ServiceVerifier { evidence });
    let descriptor = RealityAdapterDescriptor {
        adapter_id: binding.adapter_id.clone(),
        adapter_revision: binding.adapter_revision.clone(),
        effect_schema: SERVICE_SCHEMA.into(),
        target_kind: binding.target_kind.clone(),
        evidence_schema: "application/vnd.apeir.effect.http-observation+json".into(),
        observer_identity: observer.identity(),
        verifier_identity: verifier.identity(),
        authority: "none".into(),
    };
    descriptor.validate()?;
    Ok(ResolvedRealityAdapter {
        descriptor,
        target: binding,
        observer,
        verifier,
    })
}

fn evidence_error(message: impl Into<String>) -> KernelError {
    KernelError::RealityVerification(message.into())
}

impl ArtifactBridge {
    async fn invoke(&self, request: serde_json::Value) -> Result<serde_json::Value, KernelError> {
        let mut child = Command::new(&self.program)
            .arg("-m")
            .arg("nous_runtime.artifact.bridge")
            .arg("--root")
            .arg(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| evidence_error(format!("artifact bridge unavailable: {error}")))?;
        let body =
            serde_json::to_vec(&request).map_err(|error| evidence_error(error.to_string()))?;
        child
            .stdin
            .take()
            .ok_or_else(|| evidence_error("artifact bridge stdin unavailable"))?
            .write_all(&body)
            .await
            .map_err(|error| evidence_error(error.to_string()))?;
        let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
            .await
            .map_err(|_| evidence_error("artifact bridge timed out"))?
            .map_err(|error| evidence_error(error.to_string()))?;
        let response: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                evidence_error(format!("artifact bridge returned malformed JSON: {error}"))
            })?;
        if !output.status.success()
            || response.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        {
            return Err(evidence_error(
                response
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("artifact bridge rejected request"),
            ));
        }
        Ok(response)
    }

    async fn health(&self) -> Result<(), KernelError> {
        let response = self.invoke(json!({"operation": "health"})).await?;
        if response.get("schema").and_then(serde_json::Value::as_str)
            != Some("nous.artifact-bridge/v1")
        {
            return Err(evidence_error("artifact bridge schema mismatch"));
        }
        Ok(())
    }
}

#[async_trait]
impl EvidenceArtifactRuntime for ArtifactBridge {
    async fn put(
        &self,
        bytes: &[u8],
        media_type: &str,
        name: &str,
        metadata: &EvidenceMetadata<'_>,
    ) -> Result<EvidenceRef, KernelError> {
        let response = self
            .invoke(json!({
                "operation": "put",
                "content_base64": BASE64.encode(bytes),
                "media_type": media_type,
                "name": name,
                "produced_by": metadata.observer_identity,
                "metadata": metadata,
            }))
            .await?;
        let artifact_ref = response
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| evidence_error("artifact bridge returned no digest"))?;
        let digest = artifact_ref
            .strip_prefix("sha256:")
            .ok_or_else(|| evidence_error("artifact bridge returned invalid digest"))?;
        let reference = EvidenceRef {
            artifact_ref: artifact_ref.into(),
            digest: digest.into(),
        };
        reference.validate().map_err(evidence_error)?;
        Ok(reference)
    }

    async fn resolve(&self, reference: &EvidenceRef) -> Result<Vec<u8>, KernelError> {
        reference.validate().map_err(evidence_error)?;
        let response = self
            .invoke(json!({"operation": "get", "digest": reference.artifact_ref}))
            .await?;
        if response.get("digest").and_then(serde_json::Value::as_str)
            != Some(reference.artifact_ref.as_str())
        {
            return Err(evidence_error("resolved evidence reference mismatch"));
        }
        let encoded = response
            .get("content_base64")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| evidence_error("artifact bridge returned no evidence bytes"))?;
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| evidence_error(error.to_string()))?;
        if bytes.len() > MAX_RESPONSE_BYTES || digest_bytes(&bytes) != reference.digest {
            return Err(evidence_error("resolved evidence digest mismatch"));
        }
        Ok(bytes)
    }
}

fn parse_response(raw: &[u8]) -> Result<(u16, serde_json::Value), KernelError> {
    let response = std::str::from_utf8(raw).map_err(|error| evidence_error(error.to_string()))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| evidence_error("malformed HTTP response"))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| evidence_error("HTTP response has no status"))?;
    let value = serde_json::from_str(body).map_err(|error| evidence_error(error.to_string()))?;
    Ok((status, value))
}

async fn probe(
    address: SocketAddr,
    path: &str,
) -> Result<(u16, serde_json::Value, Vec<u8>), KernelError> {
    let operation = async {
        let mut stream = TcpStream::connect(address)
            .await
            .map_err(|error| evidence_error(error.to_string()))?;
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .map_err(|error| evidence_error(error.to_string()))?;
        let mut raw = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream
                .read(&mut chunk)
                .await
                .map_err(|error| evidence_error(error.to_string()))?;
            if read == 0 {
                break;
            }
            if raw.len() + read > MAX_RESPONSE_BYTES {
                return Err(evidence_error("service probe response exceeds 64 KiB"));
            }
            raw.extend_from_slice(&chunk[..read]);
        }
        let (status, body) = parse_response(&raw)?;
        Ok((status, body, raw))
    };
    tokio::time::timeout(Duration::from_secs(3), operation)
        .await
        .map_err(|_| evidence_error("service probe timed out"))?
}

#[async_trait]
impl RealityObserver for ServiceObserver {
    fn identity(&self) -> RealityIdentity {
        RealityIdentity {
            identity: "apeir.http-service-observer".into(),
            capability: "service.health.observe".into(),
        }
    }

    fn admit_contract(&self, contract: &EffectContract) -> Result<(), KernelError> {
        if contract.target != self.target
            || contract.expectation.subject != self.subject
            || contract.expectation.schema != SERVICE_SCHEMA
        {
            return Err(evidence_error(
                "effect contract is outside the configured service scope",
            ));
        }
        Ok(())
    }

    async fn observe(
        &self,
        contract: &EffectContract,
        request: &OperationRequest,
        _receipt: &OperationReceipt,
    ) -> Result<ObservedEffect, KernelError> {
        self.admit_contract(contract)?;
        let (status, _, health_raw) = probe(self.address, "/health").await?;
        let (_, version_body, version_raw) = probe(self.address, "/version").await?;
        let version = version_body["version"]
            .as_str()
            .ok_or_else(|| evidence_error("service version response has no version"))?;
        let value = json!({"http_status": status, "version": version});
        let observed_at = chrono::Utc::now();
        let observed_at_text = observed_at.to_rfc3339();
        let contract_digest = contract.digest().map_err(evidence_error)?;
        let identity = self.identity();
        let metadata = EvidenceMetadata {
            operation_id: &request.operation_id,
            intent_id: &request.operation_id,
            effect_id: &contract.effect_id,
            effect_contract_digest: &contract_digest,
            target_ref: &contract.target,
            observer_identity: &identity.identity,
            node_identity: &self.node_id,
            evidence_schema: "apeir.http-observation/v1",
            observed_at: &observed_at_text,
        };
        let health_evidence = serde_json::to_vec(&json!({
            "request_target": contract.target,
            "method": "GET",
            "path": "/health",
            "response_status": status,
            "body_digest": digest_bytes(&health_raw),
            "observed_at": observed_at_text,
            "observer_identity": identity.identity,
        }))
        .map_err(|error| evidence_error(error.to_string()))?;
        let version_evidence = serde_json::to_vec(&json!({
            "request_target": contract.target,
            "method": "GET",
            "path": "/version",
            "service_version": version,
            "body_digest": digest_bytes(&version_raw),
            "observed_at": observed_at_text,
            "observer_identity": identity.identity,
        }))
        .map_err(|error| evidence_error(error.to_string()))?;
        let evidence_refs = vec![
            self.evidence
                .put(
                    &health_evidence,
                    "application/vnd.apeir.effect.http-observation+json",
                    "http-health-observation.json",
                    &metadata,
                )
                .await?,
            self.evidence
                .put(
                    &version_evidence,
                    "application/vnd.apeir.effect.http-observation+json",
                    "http-version-observation.json",
                    &metadata,
                )
                .await?,
        ];
        Ok(ObservedEffect {
            observation_id: uuid::Uuid::now_v7().to_string(),
            effect_id: contract.effect_id.clone(),
            subject: contract.expectation.subject.clone(),
            schema: contract.expectation.schema.clone(),
            observed_value: value.clone(),
            observer: identity,
            observed_at,
            evidence_digest: ObservedEffect::compute_evidence_digest(&value, &evidence_refs)
                .map_err(evidence_error)?,
            evidence_refs,
            execution_environment_digest: None,
        })
    }
}

#[async_trait]
impl RealityVerifier for ServiceVerifier {
    fn identity(&self) -> RealityIdentity {
        RealityIdentity {
            identity: "apeir.exact-service-verifier".into(),
            capability: "service.health.verify".into(),
        }
    }

    fn policy_revision(&self) -> String {
        "exact-service-health-v1".into()
    }

    async fn verify_evidence(&self, evidence_refs: &[EvidenceRef]) -> Result<(), KernelError> {
        if evidence_refs.len() != 2 {
            return Err(evidence_error(
                "service observation requires health and version evidence",
            ));
        }
        for evidence in evidence_refs {
            evidence.validate().map_err(evidence_error)?;
            self.evidence.resolve(evidence).await?;
        }
        Ok(())
    }

    async fn evaluate(
        &self,
        contract: &EffectContract,
        observation: &ObservedEffect,
    ) -> Result<VerificationDecision, KernelError> {
        self.verify_evidence(&observation.evidence_refs).await?;
        let health: serde_json::Value =
            serde_json::from_slice(&self.evidence.resolve(&observation.evidence_refs[0]).await?)
                .map_err(|error| evidence_error(error.to_string()))?;
        let version_body: serde_json::Value =
            serde_json::from_slice(&self.evidence.resolve(&observation.evidence_refs[1]).await?)
                .map_err(|error| evidence_error(error.to_string()))?;
        let status = health["response_status"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| evidence_error("service health evidence has no status"))?;
        let version = version_body["service_version"]
            .as_str()
            .ok_or_else(|| evidence_error("service version evidence has no version"))?;
        let independently_observed = json!({"http_status": status, "version": version});
        Ok(VerificationDecision {
            outcome: if contract.expectation.expected_value == independently_observed
                && observation.observed_value == independently_observed
            {
                VerificationOutcome::Match
            } else {
                VerificationOutcome::Mismatch
            },
            evidence_refs: observation.evidence_refs.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_types::{EffectExpectation, VerificationMode};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryEvidence {
        values: Mutex<HashMap<String, Vec<u8>>>,
    }

    #[async_trait]
    impl EvidenceArtifactRuntime for MemoryEvidence {
        async fn put(
            &self,
            bytes: &[u8],
            _media_type: &str,
            _name: &str,
            _metadata: &EvidenceMetadata<'_>,
        ) -> Result<EvidenceRef, KernelError> {
            let digest = digest_bytes(bytes);
            self.values
                .lock()
                .unwrap()
                .insert(digest.clone(), bytes.to_vec());
            Ok(EvidenceRef {
                artifact_ref: format!("sha256:{digest}"),
                digest,
            })
        }

        async fn resolve(&self, reference: &EvidenceRef) -> Result<Vec<u8>, KernelError> {
            let bytes = self
                .values
                .lock()
                .unwrap()
                .get(&reference.digest)
                .cloned()
                .ok_or_else(|| evidence_error("evidence artifact not found"))?;
            if digest_bytes(&bytes) != reference.digest {
                return Err(evidence_error("evidence artifact digest mismatch"));
            }
            Ok(bytes)
        }
    }

    #[tokio::test]
    async fn verifier_reconstructs_reality_from_evidence_not_observer_claim() {
        let evidence = Arc::new(MemoryEvidence::default());
        let metadata = EvidenceMetadata {
            operation_id: "operation-test",
            intent_id: "operation-test",
            effect_id: "effect-test",
            effect_contract_digest: "0",
            target_ref: "service:test",
            observer_identity: "apeir.http-service-observer",
            node_identity: "local-node",
            evidence_schema: "apeir.http-observation/v1",
            observed_at: "2026-09-23T00:00:00Z",
        };
        let refs = vec![
            evidence
                .put(
                    br#"{"response_status":502}"#,
                    "application/json",
                    "health.json",
                    &metadata,
                )
                .await
                .unwrap(),
            evidence
                .put(
                    br#"{"service_version":"v2"}"#,
                    "application/json",
                    "version.json",
                    &metadata,
                )
                .await
                .unwrap(),
        ];
        let claimed = json!({"http_status": 200, "version": "v2"});
        let contract = EffectContract {
            schema_version: 1,
            effect_id: "effect-test".into(),
            target: "service:test".into(),
            expectation: EffectExpectation {
                schema: SERVICE_SCHEMA.into(),
                subject: "service:test".into(),
                expected_value: claimed.clone(),
                evidence_requirement: vec!["http-health".into(), "http-version".into()],
            },
            verification: VerificationMode::Independent,
        };
        let observation = ObservedEffect {
            observation_id: "observation-test".into(),
            effect_id: contract.effect_id.clone(),
            subject: contract.expectation.subject.clone(),
            schema: contract.expectation.schema.clone(),
            evidence_digest: ObservedEffect::compute_evidence_digest(&claimed, &refs).unwrap(),
            observed_value: claimed,
            observer: RealityIdentity {
                identity: "apeir.http-service-observer".into(),
                capability: "service.health.observe".into(),
            },
            observed_at: chrono::Utc::now(),
            evidence_refs: refs,
            execution_environment_digest: None,
        };
        let verifier = ServiceVerifier { evidence };
        let decision = verifier.evaluate(&contract, &observation).await.unwrap();
        assert_eq!(decision.outcome, VerificationOutcome::Mismatch);
    }

    #[test]
    fn non_loopback_probe_config_is_rejected() {
        let configured: ServiceTargetConfig = serde_json::from_value(json!({
            "subject": "service:test",
            "target_binding": {
                "schema_version": 1,
                "target_ref": "node://local/service/test",
                "target_kind": "http-service",
                "node_id": "local-node",
                "adapter_id": "apeir.http-service/v1",
                "adapter_revision": "1",
                "endpoint_binding": {"address": "8.8.8.8:80"},
                "allowed_effect_schemas": [SERVICE_SCHEMA],
                "revision": "target-1"
            }
        }))
        .unwrap();
        assert!(configure_target(configured, Arc::new(MemoryEvidence::default())).is_err());
    }

    #[test]
    fn registry_resolves_only_governed_target_and_schema() {
        let configured: ServiceTargetConfig = serde_json::from_value(json!({
            "subject": "service:test",
            "target_binding": {
                "schema_version": 1,
                "target_ref": "node://local/service/test",
                "target_kind": "http-service",
                "node_id": "local-node",
                "adapter_id": "apeir.http-service/v1",
                "adapter_revision": "1",
                "endpoint_binding": {"address": "127.0.0.1:9010"},
                "allowed_effect_schemas": [SERVICE_SCHEMA],
                "revision": "target-1"
            }
        }))
        .unwrap();
        let adapter = configure_target(configured, Arc::new(MemoryEvidence::default())).unwrap();
        let descriptor = adapter.descriptor.clone();
        let registry = ServiceRegistry {
            adapters: HashMap::from([(adapter.target.target_ref.clone(), adapter)]),
            descriptors: vec![descriptor],
        };
        let contract = EffectContract {
            schema_version: 1,
            effect_id: "effect-test".into(),
            target: "node://local/service/test".into(),
            expectation: EffectExpectation {
                schema: SERVICE_SCHEMA.into(),
                subject: "service:test".into(),
                expected_value: json!({"http_status": 200, "version": "B"}),
                evidence_requirement: vec!["http-health".into()],
            },
            verification: VerificationMode::Independent,
        };
        let resolved = registry.resolve(&contract).unwrap();
        assert_eq!(resolved.descriptor.adapter_id, "apeir.http-service/v1");
        assert_eq!(registry.descriptors().len(), 1);

        let mut unregistered = contract;
        unregistered.target = "node://local/service/other".into();
        assert!(registry.resolve(&unregistered).is_err());
    }
}
