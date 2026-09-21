//! Transactional effect lifecycle built on canonical intents and receipts.

use crate::{EffectIntent, EffectReceipt};
use nous_types::error::{ErrorCode, NousError};
use nous_types::{
    EffectVerification, EvidenceRef, ObservedEffect, RealityIdentity, VerificationMode,
    VerificationOutcome,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectTransactionPhase {
    Planned,
    Authorized,
    Prepared,
    Executed,
    Observing,
    Verified,
    Committed,
    ExecutionFailed,
    ObservationFailed,
    VerificationMismatch,
    Unknown,
    Compensating,
    Compensated,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectTransaction {
    pub transaction_id: String,
    pub idempotency_key: String,
    pub principal_id: String,
    pub capability: String,
    pub intent: EffectIntent,
    pub phase: EffectTransactionPhase,
    pub receipt: Option<EffectReceipt>,
    pub observation: Option<ObservedEffect>,
    pub verification: Option<EffectVerification>,
    pub authorization_ref: Option<String>,
}

#[derive(Default)]
struct EffectState {
    transactions: HashMap<String, EffectTransaction>,
    idempotency_index: HashMap<String, String>,
}

#[derive(Default)]
pub struct TransactionalEffectEngine {
    state: RwLock<EffectState>,
}

fn state_error() -> NousError {
    NousError::new(
        ErrorCode::Internal,
        "Effect transaction state lock is poisoned",
    )
}

impl TransactionalEffectEngine {
    pub fn plan(
        &self,
        principal_id: impl Into<String>,
        capability: impl Into<String>,
        idempotency_key: impl Into<String>,
        intent: EffectIntent,
    ) -> Result<EffectTransaction, NousError> {
        let idempotency_key = idempotency_key.into();
        let mut state = self.state.write().map_err(|_| state_error())?;
        if let Some(existing_id) = state.idempotency_index.get(&idempotency_key) {
            return state.transactions.get(existing_id).cloned().ok_or_else(|| {
                NousError::new(
                    ErrorCode::DataLoss,
                    "Effect idempotency index is inconsistent",
                )
            });
        }
        let transaction = EffectTransaction {
            transaction_id: uuid::Uuid::now_v7().to_string(),
            idempotency_key: idempotency_key.clone(),
            principal_id: principal_id.into(),
            capability: capability.into(),
            intent,
            phase: EffectTransactionPhase::Planned,
            receipt: None,
            observation: None,
            verification: None,
            authorization_ref: None,
        };
        state
            .idempotency_index
            .insert(idempotency_key, transaction.transaction_id.clone());
        state
            .transactions
            .insert(transaction.transaction_id.clone(), transaction.clone());
        Ok(transaction)
    }

    pub fn authorize(
        &self,
        transaction_id: &str,
        authorization_ref: impl Into<String>,
    ) -> Result<(), NousError> {
        let mut state = self.state.write().map_err(|_| state_error())?;
        let transaction = state
            .transactions
            .get_mut(transaction_id)
            .ok_or_else(|| NousError::new(ErrorCode::NotFound, "Effect transaction not found"))?;
        if transaction.phase != EffectTransactionPhase::Planned {
            return Err(NousError::new(
                ErrorCode::FailedPrecondition,
                "Effect is not awaiting authorization",
            ));
        }
        transaction.authorization_ref = Some(authorization_ref.into());
        transaction.phase = EffectTransactionPhase::Authorized;
        Ok(())
    }

    pub fn execute(
        &self,
        transaction_id: &str,
        effector: impl FnOnce(&EffectIntent) -> Result<EffectReceipt, NousError>,
    ) -> Result<EffectReceipt, NousError> {
        let intent = {
            let mut state = self.state.write().map_err(|_| state_error())?;
            let transaction = state.transactions.get_mut(transaction_id).ok_or_else(|| {
                NousError::new(ErrorCode::NotFound, "Effect transaction not found")
            })?;
            if transaction.phase == EffectTransactionPhase::Verified
                || transaction.phase == EffectTransactionPhase::Committed
            {
                return transaction.receipt.clone().ok_or_else(|| {
                    NousError::new(ErrorCode::DataLoss, "Verified effect has no receipt")
                });
            }
            if transaction.phase != EffectTransactionPhase::Authorized {
                return Err(NousError::new(
                    ErrorCode::PermissionDenied,
                    "Effect has not been authorized",
                ));
            }
            transaction.phase = EffectTransactionPhase::Prepared;
            transaction.intent.clone()
        };

        let receipt = match effector(&intent) {
            Ok(receipt) => receipt,
            Err(error) => {
                let mut state = self.state.write().map_err(|_| state_error())?;
                if let Some(transaction) = state.transactions.get_mut(transaction_id) {
                    transaction.phase = EffectTransactionPhase::ExecutionFailed;
                }
                return Err(error);
            }
        };
        if !receipt.success || receipt.actual_target != intent.expected_target {
            let mut state = self.state.write().map_err(|_| state_error())?;
            if let Some(transaction) = state.transactions.get_mut(transaction_id) {
                transaction.phase = EffectTransactionPhase::ExecutionFailed;
                transaction.receipt = Some(receipt);
            }
            return Err(NousError::new(
                ErrorCode::SecurityPolicy,
                "Effect receipt does not match the authorized intent",
            ));
        }
        let mut state = self.state.write().map_err(|_| state_error())?;
        let transaction = state.transactions.get_mut(transaction_id).ok_or_else(|| {
            NousError::new(
                ErrorCode::DataLoss,
                "Effect transaction disappeared while executing",
            )
        })?;
        transaction.receipt = Some(receipt.clone());
        transaction.phase = if transaction.intent.contract.is_some() {
            EffectTransactionPhase::Executed
        } else {
            // Compatibility: v0.1 effects had no reality contract.
            EffectTransactionPhase::Verified
        };
        Ok(receipt)
    }

    pub fn observe(
        &self,
        transaction_id: &str,
        observer: impl FnOnce(
            &nous_types::EffectContract,
            &EffectReceipt,
        ) -> Result<ObservedEffect, NousError>,
    ) -> Result<ObservedEffect, NousError> {
        let (contract, receipt) = {
            let mut state = self.state.write().map_err(|_| state_error())?;
            let transaction = state.transactions.get_mut(transaction_id).ok_or_else(|| {
                NousError::new(ErrorCode::NotFound, "Effect transaction not found")
            })?;
            if transaction.phase != EffectTransactionPhase::Executed {
                return Err(NousError::new(
                    ErrorCode::FailedPrecondition,
                    "Only an executed effect can be observed",
                ));
            }
            transaction.phase = EffectTransactionPhase::Observing;
            (
                transaction.intent.contract.clone().ok_or_else(|| {
                    NousError::new(
                        ErrorCode::FailedPrecondition,
                        "Effect has no reality contract",
                    )
                })?,
                transaction.receipt.clone().ok_or_else(|| {
                    NousError::new(ErrorCode::DataLoss, "Executed effect has no receipt")
                })?,
            )
        };
        let observation = match observer(&contract, &receipt) {
            Ok(observation) => observation,
            Err(error) => {
                if let Ok(mut state) = self.state.write() {
                    if let Some(transaction) = state.transactions.get_mut(transaction_id) {
                        transaction.phase = EffectTransactionPhase::ObservationFailed;
                    }
                }
                return Err(error);
            }
        };
        observation.validate(&contract).map_err(|message| {
            NousError::new(
                ErrorCode::DataLoss,
                format!("Invalid observation: {message}"),
            )
        })?;
        let mut state = self.state.write().map_err(|_| state_error())?;
        let transaction = state.transactions.get_mut(transaction_id).ok_or_else(|| {
            NousError::new(
                ErrorCode::DataLoss,
                "Effect transaction disappeared while observing",
            )
        })?;
        transaction.observation = Some(observation.clone());
        transaction.phase = EffectTransactionPhase::Observing;
        Ok(observation)
    }

    pub fn verify(
        &self,
        transaction_id: &str,
        verifier: RealityIdentity,
        verification_policy_revision: impl Into<String>,
        evidence_refs: Vec<EvidenceRef>,
        evaluate: impl FnOnce(&nous_types::EffectContract, &ObservedEffect) -> VerificationOutcome,
    ) -> Result<EffectVerification, NousError> {
        let mut state = self.state.write().map_err(|_| state_error())?;
        let transaction = state
            .transactions
            .get_mut(transaction_id)
            .ok_or_else(|| NousError::new(ErrorCode::NotFound, "Effect transaction not found"))?;
        if transaction.phase != EffectTransactionPhase::Observing {
            return Err(NousError::new(
                ErrorCode::FailedPrecondition,
                "Effect has no completed observation",
            ));
        }
        let contract = transaction.intent.contract.as_ref().ok_or_else(|| {
            NousError::new(
                ErrorCode::FailedPrecondition,
                "Effect has no reality contract",
            )
        })?;
        let observation = transaction
            .observation
            .as_ref()
            .ok_or_else(|| NousError::new(ErrorCode::DataLoss, "Observed effect is missing"))?;
        let executed_by = transaction
            .receipt
            .as_ref()
            .map(|receipt| receipt.executed_by.as_str())
            .unwrap_or_default();
        if contract.verification == VerificationMode::Independent
            && verifier.identity == executed_by
        {
            transaction.phase = EffectTransactionPhase::Unknown;
            return Err(NousError::new(
                ErrorCode::PermissionDenied,
                "Independent verifier must differ from the effect executor",
            ));
        }
        let verification = EffectVerification {
            verification_id: uuid::Uuid::now_v7().to_string(),
            effect_id: contract.effect_id.clone(),
            outcome: evaluate(contract, observation),
            effect_contract_digest: contract
                .digest()
                .map_err(|message| NousError::new(ErrorCode::DataLoss, message))?,
            observation_digest: observation
                .digest()
                .map_err(|message| NousError::new(ErrorCode::DataLoss, message))?,
            verifier,
            verification_policy_revision: verification_policy_revision.into(),
            evidence_refs,
            verified_at: chrono::Utc::now(),
        };
        verification
            .validate_bindings(contract, observation)
            .map_err(|message| NousError::new(ErrorCode::DataLoss, message))?;
        transaction.phase = match verification.outcome {
            VerificationOutcome::Match => EffectTransactionPhase::Verified,
            VerificationOutcome::Mismatch => EffectTransactionPhase::VerificationMismatch,
            VerificationOutcome::Partial | VerificationOutcome::Unknown => {
                EffectTransactionPhase::Unknown
            }
        };
        transaction.verification = Some(verification.clone());
        Ok(verification)
    }

    pub fn commit(&self, transaction_id: &str) -> Result<(), NousError> {
        let mut state = self.state.write().map_err(|_| state_error())?;
        let transaction = state
            .transactions
            .get_mut(transaction_id)
            .ok_or_else(|| NousError::new(ErrorCode::NotFound, "Effect transaction not found"))?;
        if transaction.phase != EffectTransactionPhase::Verified {
            return Err(NousError::new(
                ErrorCode::FailedPrecondition,
                "Only a verified effect can commit",
            ));
        }
        transaction.phase = EffectTransactionPhase::Committed;
        Ok(())
    }

    pub fn get(&self, transaction_id: &str) -> Option<EffectTransaction> {
        self.state
            .read()
            .ok()?
            .transactions
            .get(transaction_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent() -> EffectIntent {
        EffectIntent {
            description: "write".into(),
            expected_target: "device:1".into(),
            expected_outcome: "ok".into(),
            max_retries: 0,
            timeout_ms: 100,
            contract: None,
        }
    }

    fn receipt() -> EffectReceipt {
        EffectReceipt {
            receipt_id: "r1".into(),
            actual_target: "device:1".into(),
            actual_outcome: "ok".into(),
            execution_time_us: 1,
            retry_count: 0,
            success: true,
            error: None,
            output_hash: [0; 32],
            executed_at: chrono::Utc::now(),
            executed_by: "test".into(),
        }
    }

    fn reality_intent() -> EffectIntent {
        let mut intent = intent();
        intent.contract = Some(nous_types::EffectContract {
            schema_version: 1,
            effect_id: "effect-1".into(),
            target: "device:1".into(),
            expectation: nous_types::EffectExpectation {
                schema: "apeir.device-state/v1".into(),
                subject: "device:1".into(),
                expected_value: serde_json::json!({"state": "ready"}),
                evidence_requirement: vec!["probe".into()],
            },
            verification: VerificationMode::Independent,
        });
        intent
    }

    fn observation(value: serde_json::Value) -> ObservedEffect {
        let evidence_refs = vec![EvidenceRef {
            artifact_ref: format!("sha256:{}", "a".repeat(64)),
            digest: "a".repeat(64),
        }];
        ObservedEffect {
            observation_id: "observation-1".into(),
            effect_id: "effect-1".into(),
            subject: "device:1".into(),
            schema: "apeir.device-state/v1".into(),
            evidence_digest: ObservedEffect::compute_evidence_digest(&value, &evidence_refs)
                .unwrap(),
            observed_value: value,
            observer: RealityIdentity {
                identity: "probe".into(),
                capability: "device.observe".into(),
            },
            observed_at: chrono::Utc::now(),
            evidence_refs,
            execution_environment_digest: None,
        }
    }

    #[test]
    fn effect_requires_authorization_and_verification() {
        let engine = TransactionalEffectEngine::default();
        let transaction = engine.plan("p", "device.write", "key", intent()).unwrap();
        assert!(engine
            .execute(&transaction.transaction_id, |_| Ok(receipt()))
            .is_err());
        engine
            .authorize(&transaction.transaction_id, "approval:1")
            .unwrap();
        engine
            .execute(&transaction.transaction_id, |_| Ok(receipt()))
            .unwrap();
        engine.commit(&transaction.transaction_id).unwrap();
        assert_eq!(
            engine.get(&transaction.transaction_id).unwrap().phase,
            EffectTransactionPhase::Committed
        );
    }

    #[test]
    fn idempotent_plan_returns_existing_transaction() {
        let engine = TransactionalEffectEngine::default();
        let first = engine.plan("p", "device.write", "key", intent()).unwrap();
        let second = engine.plan("p", "device.write", "key", intent()).unwrap();
        assert_eq!(first.transaction_id, second.transaction_id);
    }

    #[test]
    fn concurrent_plan_has_one_transaction_owner() {
        let engine = std::sync::Arc::new(TransactionalEffectEngine::default());
        let mut workers = Vec::new();
        for _ in 0..16 {
            let engine = engine.clone();
            workers.push(std::thread::spawn(move || {
                engine
                    .plan("p", "device.write", "shared-key", intent())
                    .map(|transaction| transaction.transaction_id)
            }));
        }
        let mut ids = std::collections::HashSet::new();
        for worker in workers {
            ids.insert(worker.join().unwrap().unwrap());
        }
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn reality_mismatch_cannot_commit() {
        let engine = TransactionalEffectEngine::default();
        let transaction = engine
            .plan("p", "device.write", "reality-key", reality_intent())
            .unwrap();
        engine
            .authorize(&transaction.transaction_id, "approval:1")
            .unwrap();
        engine
            .execute(&transaction.transaction_id, |_| Ok(receipt()))
            .unwrap();
        assert_eq!(
            engine.get(&transaction.transaction_id).unwrap().phase,
            EffectTransactionPhase::Executed
        );
        engine
            .observe(&transaction.transaction_id, |_, _| {
                Ok(observation(serde_json::json!({"state": "broken"})))
            })
            .unwrap();
        let verification = engine
            .verify(
                &transaction.transaction_id,
                RealityIdentity {
                    identity: "verifier".into(),
                    capability: "device.verify".into(),
                },
                "exact-v1",
                vec![],
                |contract, observed| {
                    if contract.expectation.expected_value == observed.observed_value {
                        VerificationOutcome::Match
                    } else {
                        VerificationOutcome::Mismatch
                    }
                },
            )
            .unwrap();
        assert_eq!(verification.outcome, VerificationOutcome::Mismatch);
        assert!(engine.commit(&transaction.transaction_id).is_err());
        assert_eq!(
            engine.get(&transaction.transaction_id).unwrap().phase,
            EffectTransactionPhase::VerificationMismatch
        );
    }
}
