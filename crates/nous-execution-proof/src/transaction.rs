//! Transactional effect lifecycle built on canonical intents and receipts.

use crate::{EffectIntent, EffectReceipt};
use nous_types::error::{ErrorCode, NousError};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTransactionPhase {
    Planned,
    Authorized,
    Prepared,
    Verified,
    Committed,
    Compensated,
    Failed,
}

#[derive(Debug, Clone)]
pub struct EffectTransaction {
    pub transaction_id: String,
    pub idempotency_key: String,
    pub principal_id: String,
    pub capability: String,
    pub intent: EffectIntent,
    pub phase: EffectTransactionPhase,
    pub receipt: Option<EffectReceipt>,
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
                    transaction.phase = EffectTransactionPhase::Failed;
                }
                return Err(error);
            }
        };
        if !receipt.success || receipt.actual_target != intent.expected_target {
            let mut state = self.state.write().map_err(|_| state_error())?;
            if let Some(transaction) = state.transactions.get_mut(transaction_id) {
                transaction.phase = EffectTransactionPhase::Failed;
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
        transaction.phase = EffectTransactionPhase::Verified;
        Ok(receipt)
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
}
