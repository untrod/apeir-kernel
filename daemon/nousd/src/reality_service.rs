//! Explicitly configured, loopback-only reference adapter for service reality.
//! No NKI request may choose a probe destination or evidence directory.

use async_trait::async_trait;
use nous_kernel_core::{
    digest_bytes, KernelError, OperationReceipt, RealityObserver, RealityVerifier,
    VerificationDecision,
};
use nous_types::{
    EffectContract, EvidenceRef, ObservedEffect, OperationRequest, RealityIdentity,
    VerificationOutcome,
};
use serde::Deserialize;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SERVICE_SCHEMA: &str = "apeir.service-health/v1";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceConfig {
    schema_version: u32,
    target: String,
    subject: String,
    service_address: String,
}

struct ServiceObserver {
    address: SocketAddr,
    target: String,
    subject: String,
    evidence_dir: Arc<PathBuf>,
}

struct ServiceVerifier {
    evidence_dir: Arc<PathBuf>,
}

pub type ConfiguredReality = (Arc<dyn RealityObserver>, Arc<dyn RealityVerifier>);

pub fn load(
    config_path: &Path,
    journal: &Path,
) -> Result<ConfiguredReality, Box<dyn std::error::Error>> {
    let config: ServiceConfig = serde_json::from_slice(&fs::read(config_path)?)?;
    let address: SocketAddr = config.service_address.parse()?;
    if config.schema_version != 1
        || config.target.is_empty()
        || config.subject.is_empty()
        || !address.ip().is_loopback()
        || address.port() == 0
    {
        return Err(
            "reality service config requires version 1, identities, and a loopback socket".into(),
        );
    }
    let evidence_dir = Arc::new(journal.with_extension("evidence"));
    fs::create_dir_all(evidence_dir.as_ref())?;
    Ok((
        Arc::new(ServiceObserver {
            address,
            target: config.target,
            subject: config.subject,
            evidence_dir: evidence_dir.clone(),
        }),
        Arc::new(ServiceVerifier { evidence_dir }),
    ))
}

fn evidence_error(message: impl Into<String>) -> KernelError {
    KernelError::RealityVerification(message.into())
}

fn store_evidence(directory: &Path, bytes: &[u8]) -> Result<EvidenceRef, KernelError> {
    let digest = digest_bytes(bytes);
    let path = directory.join(&digest);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| evidence_error(error.to_string()))?;
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let existing = fs::read(&path).map_err(|error| evidence_error(error.to_string()))?;
            if digest_bytes(&existing) != digest {
                return Err(evidence_error(
                    "existing content-addressed evidence is corrupt",
                ));
            }
        }
        Err(error) => return Err(evidence_error(error.to_string())),
    }
    Ok(EvidenceRef {
        artifact_ref: format!("sha256:{digest}"),
        digest,
    })
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
        _request: &OperationRequest,
        _receipt: &OperationReceipt,
    ) -> Result<ObservedEffect, KernelError> {
        self.admit_contract(contract)?;
        let (status, _, health_raw) = probe(self.address, "/health").await?;
        let (_, version_body, version_raw) = probe(self.address, "/version").await?;
        let version = version_body["version"]
            .as_str()
            .ok_or_else(|| evidence_error("service version response has no version"))?;
        let value = json!({"http_status": status, "version": version});
        let evidence_refs = vec![
            store_evidence(&self.evidence_dir, &health_raw)?,
            store_evidence(&self.evidence_dir, &version_raw)?,
        ];
        Ok(ObservedEffect {
            observation_id: uuid::Uuid::now_v7().to_string(),
            effect_id: contract.effect_id.clone(),
            subject: contract.expectation.subject.clone(),
            schema: contract.expectation.schema.clone(),
            observed_value: value.clone(),
            observer: self.identity(),
            observed_at: chrono::Utc::now(),
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
            let path = self.evidence_dir.join(&evidence.digest);
            if fs::metadata(&path)
                .map_err(|error| evidence_error(error.to_string()))?
                .len()
                > MAX_RESPONSE_BYTES as u64
            {
                return Err(evidence_error("service evidence exceeds 64 KiB"));
            }
            let bytes = fs::read(path).map_err(|error| evidence_error(error.to_string()))?;
            if digest_bytes(&bytes) != evidence.digest {
                return Err(evidence_error("service evidence digest mismatch"));
            }
        }
        Ok(())
    }

    async fn evaluate(
        &self,
        contract: &EffectContract,
        observation: &ObservedEffect,
    ) -> Result<VerificationDecision, KernelError> {
        self.verify_evidence(&observation.evidence_refs).await?;
        let health_raw = fs::read(self.evidence_dir.join(&observation.evidence_refs[0].digest))
            .map_err(|error| evidence_error(error.to_string()))?;
        let version_raw = fs::read(self.evidence_dir.join(&observation.evidence_refs[1].digest))
            .map_err(|error| evidence_error(error.to_string()))?;
        let (status, _) = parse_response(&health_raw)?;
        let (_, version_body) = parse_response(&version_raw)?;
        let version = version_body["version"]
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

    #[tokio::test]
    async fn verifier_reconstructs_reality_from_evidence_not_observer_claim() {
        let directory = tempfile::tempdir().unwrap();
        let refs = vec![
            store_evidence(
                directory.path(),
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\n\r\n{\"status\":502}",
            )
            .unwrap(),
            store_evidence(
                directory.path(),
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"version\":\"v2\"}",
            )
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
        let verifier = ServiceVerifier {
            evidence_dir: Arc::new(directory.path().to_path_buf()),
        };
        let decision = verifier.evaluate(&contract, &observation).await.unwrap();
        assert_eq!(decision.outcome, VerificationOutcome::Mismatch);
    }

    #[test]
    fn non_loopback_probe_config_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.json");
        fs::write(
            &config,
            json!({
                "schema_version": 1,
                "target": "service:test",
                "subject": "service:test",
                "service_address": "8.8.8.8:80"
            })
            .to_string(),
        )
        .unwrap();
        assert!(load(&config, &directory.path().join("journal.db")).is_err());
    }
}
