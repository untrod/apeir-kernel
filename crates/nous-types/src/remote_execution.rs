//! Signed remote execution facts supplied by Nous Node Protocol.

use crate::{DeliverySemantics, EffectContract, OperationRequest, TargetBinding};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const REMOTE_EXECUTION_RECEIPT_SCHEMA_VERSION: u32 = 1;
pub const NODE_PROTOCOL_V1: &str = "1.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedNodeEnvelope {
    pub created_at: String,
    pub expires_at: String,
    pub idempotency_key: String,
    pub message_id: String,
    pub message_type: String,
    pub payload: Value,
    pub protocol: String,
    pub protocol_version: String,
    pub reply_to: String,
    pub sequence: u64,
    pub source: String,
    pub target: String,
    pub signature: String,
}

impl SignedNodeEnvelope {
    pub fn unsigned_value(&self) -> Value {
        serde_json::json!({
            "created_at": self.created_at,
            "expires_at": self.expires_at,
            "idempotency_key": self.idempotency_key,
            "message_id": self.message_id,
            "message_type": self.message_type,
            "payload": self.payload,
            "protocol": self.protocol,
            "protocol_version": self.protocol_version,
            "reply_to": self.reply_to,
            "sequence": self.sequence,
            "source": self.source,
            "target": self.target,
        })
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, String> {
        crate::effect::canonical_json_bytes(&self.unsigned_value())
    }

    pub fn digest(&self) -> Result<String, String> {
        crate::effect::canonical_digest(self)
    }
}

/// Candidate remote execution fact. It becomes Kernel truth only after
/// signature, trust, intent, effect, target, and digest admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExecutionReceipt {
    pub schema_version: u32,
    pub operation_id: String,
    pub workload_id: String,
    pub node_id: String,
    pub executor_id: String,
    pub intent_id: String,
    pub effect_contract_digest: String,
    pub target_ref: String,
    pub target_binding_digest: String,
    pub request_digest: String,
    pub output_digest: String,
    pub provider_revision: String,
    pub delivery_semantics: DeliverySemantics,
    pub started_at: String,
    pub completed_at: String,
    pub node_protocol_version: String,
    pub signed_envelope_digest: String,
    pub signed_envelope: SignedNodeEnvelope,
}

impl RemoteExecutionReceipt {
    pub fn validate_bindings(
        &self,
        request: &OperationRequest,
        contract: &EffectContract,
        target: &TargetBinding,
    ) -> Result<(), String> {
        if self.schema_version != REMOTE_EXECUTION_RECEIPT_SCHEMA_VERSION {
            return Err("remote execution receipt schema_version must be 1".into());
        }
        target.admits(contract)?;
        if self.operation_id != request.operation_id
            || self.workload_id != request.workload_id
            || self.intent_id != request.operation_id
            || self.effect_contract_digest != contract.digest()?
            || self.target_ref != contract.target
            || self.target_binding_digest != target.digest()?
            || self.request_digest != request.input_digest()
            || self.provider_revision != request.snapshot.provider_revision
            || self.delivery_semantics != request.delivery
            || self.node_id != target.node_id
            || self.node_protocol_version != NODE_PROTOCOL_V1
            || self.signed_envelope.source != self.node_id
            || self.signed_envelope.message_type != "WORKLOAD_STATUS"
            || self.signed_envelope.protocol != "nous-node"
            || self.signed_envelope.protocol_version != self.node_protocol_version
            || self.signed_envelope.idempotency_key != self.operation_id
            || self.signed_envelope_digest != self.signed_envelope.digest()?
        {
            return Err("remote execution receipt binding mismatch".into());
        }
        let payload = self
            .signed_envelope
            .payload
            .as_object()
            .ok_or_else(|| "remote execution envelope payload must be an object".to_string())?;
        if payload.get("workload_id").and_then(Value::as_str) != Some(self.operation_id.as_str()) {
            return Err("signed node payload does not bind the remote receipt".into());
        }
        if payload.get("state").and_then(Value::as_str) != Some("COMPLETED") {
            return Err("signed node payload is not a completed execution fact".into());
        }
        let binding = payload
            .get("binding")
            .and_then(Value::as_object)
            .ok_or_else(|| "signed node payload has no effect binding".to_string())?;
        let node_receipt = payload
            .get("receipt")
            .and_then(Value::as_object)
            .ok_or_else(|| "signed node payload has no operation receipt".to_string())?;
        if binding.get("intent_id").and_then(Value::as_str) != Some(self.intent_id.as_str())
            || binding
                .get("effect_contract_digest")
                .and_then(Value::as_str)
                != Some(self.effect_contract_digest.as_str())
            || binding.get("target_ref").and_then(Value::as_str) != Some(self.target_ref.as_str())
            || binding.get("target_binding_digest").and_then(Value::as_str)
                != Some(self.target_binding_digest.as_str())
            || binding.get("workload_id").and_then(Value::as_str) != Some(self.workload_id.as_str())
            || binding.get("request_digest").and_then(Value::as_str)
                != Some(self.request_digest.as_str())
            || binding.get("provider_revision").and_then(Value::as_str)
                != Some(self.provider_revision.as_str())
            || node_receipt.get("operation_id").and_then(Value::as_str)
                != Some(self.operation_id.as_str())
            || node_receipt.get("node_id").and_then(Value::as_str) != Some(self.node_id.as_str())
            || node_receipt.get("executor").and_then(Value::as_str)
                != Some(self.executor_id.as_str())
            || node_receipt.get("effect_digest").and_then(Value::as_str)
                != Some(self.output_digest.as_str())
        {
            return Err("signed node receipt fields do not match remote receipt".into());
        }
        if self.executor_id.is_empty()
            || self.output_digest.len() != 64
            || self.started_at.is_empty()
            || self.completed_at.is_empty()
        {
            return Err("remote execution receipt identities and digests are required".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EffectExpectation, SemanticExecutionSnapshot, VerificationMode,
        EFFECT_CONTRACT_SCHEMA_VERSION, TARGET_BINDING_SCHEMA_VERSION,
    };

    fn request() -> OperationRequest {
        OperationRequest {
            operation_id: "operation-1".into(),
            workload_id: "workload-1".into(),
            step_id: "step-1".into(),
            backend: "remote-node".into(),
            execution_domain: Default::default(),
            model: "provider".into(),
            endpoint: String::new(),
            credential_env: String::new(),
            provider_entrypoint: String::new(),
            input: "set-version-B".into(),
            delivery: DeliverySemantics::AtMostOnce,
            snapshot: SemanticExecutionSnapshot {
                model_revision: "model-1".into(),
                provider_revision: "provider-1".into(),
                prompt_revision: "prompt-1".into(),
                tool_revision: "tool-1".into(),
                knowledge_revision: "knowledge-1".into(),
                policy_revision: "policy-1".into(),
                capability_revision: "capability-1".into(),
                context_revision: "context-1".into(),
            },
            timeout_ms: 1_000,
            effect_contract: None,
        }
    }

    fn contract() -> EffectContract {
        EffectContract {
            schema_version: EFFECT_CONTRACT_SCHEMA_VERSION,
            effect_id: "effect-1".into(),
            target: "node://arm64-lab/service/test-api".into(),
            expectation: EffectExpectation {
                schema: "apeir.service-health/v1".into(),
                subject: "service:test-api".into(),
                expected_value: serde_json::json!({"version": "B"}),
                evidence_requirement: vec!["http-version".into()],
            },
            verification: VerificationMode::Independent,
        }
    }

    fn target() -> TargetBinding {
        TargetBinding {
            schema_version: TARGET_BINDING_SCHEMA_VERSION,
            target_ref: "node://arm64-lab/service/test-api".into(),
            target_kind: "http-service".into(),
            node_id: "node-arm64".into(),
            adapter_id: "apeir.http-service/v1".into(),
            adapter_revision: "1".into(),
            endpoint_binding: serde_json::json!({"address": "127.0.0.1:9010"}),
            allowed_effect_schemas: vec!["apeir.service-health/v1".into()],
            revision: "target-1".into(),
        }
    }

    fn receipt(
        request: &OperationRequest,
        contract: &EffectContract,
        target: &TargetBinding,
    ) -> RemoteExecutionReceipt {
        let output_digest = "1".repeat(64);
        let payload = serde_json::json!({
            "workload_id": request.operation_id,
            "request_digest": request.input_digest(),
            "state": "COMPLETED",
            "binding": {
                "intent_id": request.operation_id,
                "effect_contract_digest": contract.digest().unwrap(),
                "target_ref": target.target_ref,
                "target_binding_digest": target.digest().unwrap(),
                "workload_id": request.workload_id,
                "request_digest": request.input_digest(),
                "provider_revision": request.snapshot.provider_revision,
            },
            "receipt": {
                "operation_id": request.operation_id,
                "node_id": target.node_id,
                "executor": "nous-node/bounded-handler",
                "request_digest": request.input_digest(),
                "effect_digest": output_digest,
            }
        });
        let envelope = SignedNodeEnvelope {
            created_at: "2026-09-23T00:00:00Z".into(),
            expires_at: String::new(),
            idempotency_key: request.operation_id.clone(),
            message_id: "message-1".into(),
            message_type: "WORKLOAD_STATUS".into(),
            payload,
            protocol: "nous-node".into(),
            protocol_version: NODE_PROTOCOL_V1.into(),
            reply_to: "dispatch-1".into(),
            sequence: 2,
            source: target.node_id.clone(),
            target: "control_plane".into(),
            signature: "00".repeat(64),
        };
        RemoteExecutionReceipt {
            schema_version: REMOTE_EXECUTION_RECEIPT_SCHEMA_VERSION,
            operation_id: request.operation_id.clone(),
            workload_id: request.workload_id.clone(),
            node_id: target.node_id.clone(),
            executor_id: "nous-node/bounded-handler".into(),
            intent_id: request.operation_id.clone(),
            effect_contract_digest: contract.digest().unwrap(),
            target_ref: target.target_ref.clone(),
            target_binding_digest: target.digest().unwrap(),
            request_digest: request.input_digest(),
            output_digest,
            provider_revision: request.snapshot.provider_revision.clone(),
            delivery_semantics: request.delivery,
            started_at: "2026-09-23T00:00:00Z".into(),
            completed_at: "2026-09-23T00:00:01Z".into(),
            node_protocol_version: NODE_PROTOCOL_V1.into(),
            signed_envelope_digest: envelope.digest().unwrap(),
            signed_envelope: envelope,
        }
    }

    #[test]
    fn target_binding_is_canonical_and_governs_contract() {
        let target = target();
        target.admits(&contract()).unwrap();
        assert_eq!(
            target.digest().unwrap(),
            "2dcedb460f2f71ee32f2c8c49061d57f5d0c4c5f201b17edc3321f4e357f40b3"
        );
        assert_eq!(
            contract().digest().unwrap(),
            "a1dba61f6b64a4d503e24c4f4c450edea15c4980b8470f163aa11385a287c6aa"
        );
        let mut changed = target.clone();
        changed.revision = "target-2".into();
        assert_ne!(target.digest().unwrap(), changed.digest().unwrap());
    }

    #[test]
    fn remote_receipt_rejects_effect_and_target_changes() {
        let request = request();
        let contract = contract();
        let target = target();
        let receipt = receipt(&request, &contract, &target);
        receipt
            .validate_bindings(&request, &contract, &target)
            .unwrap();

        let mut wrong = receipt.clone();
        wrong.effect_contract_digest = "0".repeat(64);
        assert!(wrong
            .validate_bindings(&request, &contract, &target)
            .is_err());
        let mut changed_target = target.clone();
        changed_target.revision = "target-2".into();
        assert!(receipt
            .validate_bindings(&request, &contract, &changed_target)
            .is_err());
    }
}
