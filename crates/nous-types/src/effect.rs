//! Provider-neutral contracts for proving real-world effects.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const EFFECT_CONTRACT_SCHEMA_VERSION: u32 = 1;
pub const TARGET_BINDING_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerificationMode {
    None,
    Required,
    Independent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerificationOutcome {
    Match,
    Partial,
    Mismatch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectExpectation {
    pub schema: String,
    pub subject: String,
    pub expected_value: Value,
    #[serde(default)]
    pub evidence_requirement: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectContract {
    pub schema_version: u32,
    pub effect_id: String,
    pub target: String,
    pub expectation: EffectExpectation,
    pub verification: VerificationMode,
}

impl EffectContract {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != EFFECT_CONTRACT_SCHEMA_VERSION {
            return Err("effect contract schema_version must be 1".into());
        }
        if self.effect_id.is_empty()
            || self.target.is_empty()
            || self.expectation.schema.is_empty()
            || self.expectation.subject.is_empty()
        {
            return Err(
                "effect contract identities, target, schema, and subject are required".into(),
            );
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, String> {
        canonical_digest(self)
    }
}

/// Versioned, governed resolution of an intent-level target reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetBinding {
    pub schema_version: u32,
    pub target_ref: String,
    pub target_kind: String,
    pub node_id: String,
    pub adapter_id: String,
    pub adapter_revision: String,
    pub endpoint_binding: Value,
    #[serde(default)]
    pub allowed_effect_schemas: Vec<String>,
    pub revision: String,
}

impl TargetBinding {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != TARGET_BINDING_SCHEMA_VERSION {
            return Err("target binding schema_version must be 1".into());
        }
        if self.target_ref.is_empty()
            || self.target_kind.is_empty()
            || self.node_id.is_empty()
            || self.adapter_id.is_empty()
            || self.adapter_revision.is_empty()
            || self.revision.is_empty()
            || !self.endpoint_binding.is_object()
            || self.allowed_effect_schemas.is_empty()
            || self
                .allowed_effect_schemas
                .iter()
                .any(|schema| schema.is_empty())
        {
            return Err(
                "target binding identities, endpoint, effects, and revision are required".into(),
            );
        }
        Ok(())
    }

    pub fn admits(&self, contract: &EffectContract) -> Result<(), String> {
        self.validate()?;
        contract.validate()?;
        if contract.target != self.target_ref
            || !self
                .allowed_effect_schemas
                .iter()
                .any(|schema| schema == &contract.expectation.schema)
        {
            return Err("effect contract is outside the target binding".into());
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        canonical_digest(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub artifact_ref: String,
    /// Lowercase SHA-256 digest of the content stored by Artifact Runtime.
    pub digest: String,
}

impl EvidenceRef {
    pub fn validate(&self) -> Result<(), String> {
        if !is_sha256(&self.digest) || self.artifact_ref != format!("sha256:{}", self.digest) {
            return Err("evidence requires a content-addressed SHA-256 reference".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealityIdentity {
    pub identity: String,
    pub capability: String,
}

impl RealityIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if self.identity.is_empty() || self.capability.is_empty() {
            return Err("reality actor identity and capability are required".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedEffect {
    pub observation_id: String,
    pub effect_id: String,
    pub subject: String,
    pub schema: String,
    pub observed_value: Value,
    pub observer: RealityIdentity,
    pub observed_at: DateTime<Utc>,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    /// Digest of the observed value and its immutable evidence bindings.
    pub evidence_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_environment_digest: Option<String>,
}

impl ObservedEffect {
    pub fn compute_evidence_digest(
        observed_value: &Value,
        evidence_refs: &[EvidenceRef],
    ) -> Result<String, String> {
        canonical_digest(&(observed_value, evidence_refs))
    }

    pub fn validate(&self, contract: &EffectContract) -> Result<(), String> {
        self.observer.validate()?;
        if self.effect_id != contract.effect_id
            || self.subject != contract.expectation.subject
            || self.schema != contract.expectation.schema
        {
            return Err("observation is not bound to the requested effect contract".into());
        }
        for evidence in &self.evidence_refs {
            evidence.validate()?;
        }
        if !contract.expectation.evidence_requirement.is_empty() && self.evidence_refs.is_empty() {
            return Err("effect contract requires observation evidence".into());
        }
        let digest = Self::compute_evidence_digest(&self.observed_value, &self.evidence_refs)?;
        if digest != self.evidence_digest {
            return Err("observation evidence digest mismatch".into());
        }
        if self
            .execution_environment_digest
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        {
            return Err("execution environment digest must be lowercase SHA-256".into());
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, String> {
        canonical_digest(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectVerification {
    pub verification_id: String,
    pub effect_id: String,
    pub outcome: VerificationOutcome,
    pub effect_contract_digest: String,
    pub observation_digest: String,
    pub verifier: RealityIdentity,
    pub verification_policy_revision: String,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    pub verified_at: DateTime<Utc>,
}

impl EffectVerification {
    pub fn validate_bindings(
        &self,
        contract: &EffectContract,
        observation: &ObservedEffect,
    ) -> Result<(), String> {
        self.verifier.validate()?;
        if self.effect_id != contract.effect_id
            || self.effect_contract_digest != contract.digest()?
            || self.observation_digest != observation.digest()?
        {
            return Err("verification receipt binding mismatch".into());
        }
        if self.verification_policy_revision.is_empty() {
            return Err("verification policy revision is required".into());
        }
        for evidence in &self.evidence_refs {
            evidence.validate()?;
        }
        Ok(())
    }
}

pub fn canonical_digest<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> EffectContract {
        EffectContract {
            schema_version: 1,
            effect_id: "effect-1".into(),
            target: "service:test".into(),
            expectation: EffectExpectation {
                schema: "apeir.service-health/v1".into(),
                subject: "service:test".into(),
                expected_value: serde_json::json!({"status": 200}),
                evidence_requirement: vec!["http-response".into()],
            },
            verification: VerificationMode::Independent,
        }
    }

    #[test]
    fn tampered_observation_is_rejected() {
        let contract = contract();
        let value = serde_json::json!({"status": 200});
        let evidence_refs = vec![EvidenceRef {
            artifact_ref: format!("sha256:{}", "a".repeat(64)),
            digest: "a".repeat(64),
        }];
        let mut observation = ObservedEffect {
            observation_id: "observation-1".into(),
            effect_id: contract.effect_id.clone(),
            subject: contract.expectation.subject.clone(),
            schema: contract.expectation.schema.clone(),
            evidence_digest: ObservedEffect::compute_evidence_digest(&value, &evidence_refs)
                .unwrap(),
            observed_value: value,
            observer: RealityIdentity {
                identity: "probe".into(),
                capability: "http.observe".into(),
            },
            observed_at: Utc::now(),
            evidence_refs,
            execution_environment_digest: None,
        };
        observation.observed_value = serde_json::json!({"status": 502});
        assert_eq!(
            observation.validate(&contract).unwrap_err(),
            "observation evidence digest mismatch"
        );
    }
}
