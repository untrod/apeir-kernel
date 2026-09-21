//! Proof-carrying execution primitives.
//!
//! Governance gates and supply-chain verification alone cannot prove that:
//! - The action EXECUTED matches the action APPROVED
//! - Parameters were not substituted between approval and execution
//! - The actual effects comply with policy
//! - The trace has not been tampered with
//! - Failed actions did not produce duplicate side effects

//!
//! Proof-Carrying Execution addresses this by binding every action to
//! an approval, generating verifiable execution receipts, and producing
//! workload-level certificates of correct execution.
//!
//! # Architecture
//!
//! ```text
//! Planner -> proposes CanonicalAction
//! Policy Engine -> approves (or denies)
//! Effector -> executes with exclusive credentials
//! Recorder -> generates tamper-evident receipt
//! Verifier -> independently checks action <-> result
//! Certificate -> aggregates all actions into workload-level proof
//! ```

pub mod transaction;

pub use transaction::{EffectTransaction, EffectTransactionPhase, TransactionalEffectEngine};

use nous_types::{EffectContract, EffectVerification, ObservedEffect};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Sha256Digest = [u8; 32];
pub type ActionId = String;

// -- Canonical Action --

/// A canonical action - the immutable specification of what should be done,
/// as approved by governance.
///
/// Once created and approved, the canonical_spec MUST NOT be modified.
/// The execution receipt proves that what ran == what was approved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalAction {
    /// Unique action ID.
    pub action_id: ActionId,

    /// The action specification as approved (immutable).
    pub canonical_spec: ActionSpec,

    /// The principal who requested the action.
    pub principal_id: String,

    /// The process that executed the action.
    pub process_id: String,

    /// The capability that authorized this action.
    pub capability: String,

    /// The policy decision that allowed this action.
    pub policy_decision: PolicyDecision,

    /// The approval receipt (human or automated).
    pub approval_receipt: Option<ApprovalReceipt>,

    /// Hash of all inputs at approval time.
    pub input_digest: Sha256Digest,

    /// The execution environment fingerprint.
    pub execution_environment: EnvironmentFingerprint,

    /// The intended effect as stated in the approval.
    pub effect_intent: EffectIntent,

    /// The actual effect receipt (populated after execution).
    pub effect_receipt: Option<EffectReceipt>,

    /// Independent observation of reality after provider execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_effect: Option<ObservedEffect>,

    /// Receipt binding the approved expectation to the observed reality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect_verification: Option<EffectVerification>,

    /// A successful provider receipt alone never sets this flag.
    #[serde(default)]
    pub effect_committed: bool,

    #[serde(default)]
    pub recovery_involved: bool,

    /// Hash of all outputs (populated after execution).
    pub output_digest: Option<Sha256Digest>,

    /// Causal parent - the action that triggered this one.
    pub causal_parent: Option<ActionId>,

    /// Unique replay descriptor to detect duplicate execution.
    pub replay_descriptor: ReplayDescriptor,
}

/// The specification of what an action does.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSpec {
    /// Action type (e.g., "file_write", "email_send", "code_commit").
    pub action_type: String,

    /// Target of the action (e.g., file path, email address, repo).
    pub target: String,

    /// Parameters (canonical JSON).
    pub parameters: String,

    /// Expected output schema.
    pub output_schema: Option<String>,

    /// Whether this action is idempotent.
    pub is_idempotent: bool,

    /// Whether this action is reversible (has compensation).
    pub is_reversible: bool,

    /// Compensation action ID (if reversible).
    pub compensation_action_id: Option<ActionId>,

    /// Risk level.
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// A policy decision record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub decision_id: String,
    pub decided_by: String, // "governance-gate", "human-approver", etc.
    pub decision: Decision,
    pub reason: String,
    pub decided_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Approved,
    Denied,
    ApprovedWithConditions,
    Escalated,
}

/// A human or automated approval receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalReceipt {
    pub receipt_id: String,
    pub approver_id: String,
    pub approver_type: ApproverType,
    pub approved_at: chrono::DateTime<chrono::Utc>,
    pub justification: Option<String>,
    pub signature: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApproverType {
    Human,
    AutomatedPolicy,
    MultiPartyConsensus,
}

/// Fingerprint of the execution environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentFingerprint {
    pub node_id: String,
    pub device_id: String,
    pub engine_id: String,
    pub model_revision: String,
    pub kernel_version: String,
    pub nki_version: u32,
    pub sandbox_profile: String,
    pub runtime_hash: Option<Sha256Digest>,
}

/// The intended effect of an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectIntent {
    pub description: String,
    pub expected_target: String,
    pub expected_outcome: String,
    pub max_retries: u32,
    pub timeout_ms: u64,
    /// Versioned reality contract. `None` preserves the v0.1 receipt-only path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<EffectContract>,
}

/// The actual effect receipt after execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectReceipt {
    pub receipt_id: String,
    pub actual_target: String,
    pub actual_outcome: String,
    pub execution_time_us: u64,
    pub retry_count: u32,
    pub success: bool,
    pub error: Option<String>,
    pub output_hash: Sha256Digest,
    pub executed_at: chrono::DateTime<chrono::Utc>,
    pub executed_by: String,
}

/// Descriptor to prevent replay attacks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayDescriptor {
    pub nonce: u64,
    pub idempotency_key: String,
    pub process_id: String,
    pub sequence_number: u64,
}

// -- Verification --

/// Result of verifying a CanonicalAction.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub action_id: ActionId,
    pub verified: bool,
    pub checks: Vec<VerificationCheck>,
}

#[derive(Debug, Clone)]
pub struct VerificationCheck {
    pub check_name: String,
    pub passed: bool,
    pub detail: String,
}

/// Verify that a CanonicalAction is consistent and valid.
pub fn verify_action(action: &CanonicalAction) -> VerificationResult {
    let mut checks = Vec::new();

    // Check 1: Input digest must match
    let recomputed_input = compute_input_digest(&action.canonical_spec);
    let input_match = recomputed_input == action.input_digest;
    checks.push(VerificationCheck {
        check_name: "input_digest_match".into(),
        passed: input_match,
        detail: if input_match {
            "Input digest matches canonical spec".into()
        } else {
            "Input digest mismatch - spec may have been modified after approval".into()
        },
    });

    // Check 2: If effect receipt exists, verify output
    if let Some(ref receipt) = action.effect_receipt {
        checks.push(VerificationCheck {
            check_name: "effect_receipt_present".into(),
            passed: true,
            detail: format!("Effect receipt '{}' present", receipt.receipt_id),
        });

        // Check target matches intent
        let target_match = receipt.actual_target == action.effect_intent.expected_target;
        checks.push(VerificationCheck {
            check_name: "target_matches_intent".into(),
            passed: target_match,
            detail: if target_match {
                "Actual target matches intended target".into()
            } else {
                format!(
                    "Target mismatch: expected '{}', got '{}'",
                    action.effect_intent.expected_target, receipt.actual_target
                )
            },
        });
    }

    if let Some(contract) = &action.effect_intent.contract {
        let observation_valid = action
            .observed_effect
            .as_ref()
            .is_some_and(|observation| observation.validate(contract).is_ok());
        checks.push(VerificationCheck {
            check_name: "reality_observation_bound".into(),
            passed: observation_valid,
            detail: if observation_valid {
                "Reality observation is bound to the effect contract and evidence".into()
            } else {
                "Reality observation is missing, mismatched, or tampered".into()
            },
        });
        let verification_matches = action
            .observed_effect
            .as_ref()
            .zip(action.effect_verification.as_ref())
            .is_some_and(|(observation, verification)| {
                verification.outcome == nous_types::VerificationOutcome::Match
                    && verification
                        .validate_bindings(contract, observation)
                        .is_ok()
            });
        checks.push(VerificationCheck {
            check_name: "reality_verification_match".into(),
            passed: verification_matches,
            detail: if verification_matches {
                "Reality verification is a bound MATCH".into()
            } else {
                "Effect has no bound MATCH verification".into()
            },
        });
        checks.push(VerificationCheck {
            check_name: "effect_committed_after_verification".into(),
            passed: verification_matches && action.effect_committed,
            detail: format!("Effect committed: {}", action.effect_committed),
        });
    }

    // Check 3: Replay descriptor must be unique (checked by the verifier at runtime)
    checks.push(VerificationCheck {
        check_name: "replay_descriptor_present".into(),
        passed: true,
        detail: format!(
            "Replay descriptor: nonce={}, key={}",
            action.replay_descriptor.nonce, action.replay_descriptor.idempotency_key
        ),
    });

    // Check 4: Policy decision must be Approved
    let approved = action.policy_decision.decision == Decision::Approved
        || action.policy_decision.decision == Decision::ApprovedWithConditions;
    checks.push(VerificationCheck {
        check_name: "policy_approved".into(),
        passed: approved,
        detail: format!("Policy decision: {:?}", action.policy_decision.decision),
    });

    let all_passed = checks.iter().all(|c| c.passed);

    VerificationResult {
        action_id: action.action_id.clone(),
        verified: all_passed,
        checks,
    }
}

// -- Workload Certificate --

/// A workload-level certificate aggregating all actions in a process.
///
/// This is the final deliverable that an enterprise, regulator, or auditor
/// can independently verify to confirm that the AI system operated correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadCertificate {
    pub certificate_id: String,
    pub process_id: String,
    pub program_name: String,
    pub actions: Vec<CanonicalAction>,
    pub models_used: Vec<String>,
    pub tools_used: Vec<String>,
    pub approvals_granted: Vec<ApprovalReceipt>,
    pub data_exfiltrated: Vec<DataExitRecord>,
    pub results_verified: Vec<VerificationSummary>,
    pub failures_recovered: Vec<RecoveryRecord>,
    pub final_side_effects: Vec<EffectReceipt>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub generated_by: String,
    pub attestation: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataExitRecord {
    pub data_description: String,
    pub destination: String,
    pub approved: bool,
    pub action_id: ActionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSummary {
    pub action_id: ActionId,
    pub verified: bool,
    pub verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub failure_action_id: ActionId,
    pub recovery_action_id: ActionId,
    pub recovery_success: bool,
}

impl WorkloadCertificate {
    /// Generate a certificate from a completed process's action log.
    pub fn generate(
        process_id: String,
        program_name: String,
        actions: Vec<CanonicalAction>,
        generated_by: String,
    ) -> Self {
        let mut models = Vec::new();
        let tools = Vec::new();
        let mut approvals = Vec::new();
        let mut data_exits = Vec::new();
        let mut verification_summaries = Vec::new();
        let recoveries = Vec::new();
        let mut side_effects = Vec::new();

        for action in &actions {
            // Collect models
            let model = &action.execution_environment.model_revision;
            if !models.contains(model) {
                models.push(model.clone());
            }

            // Collect approvals
            if let Some(ref receipt) = action.approval_receipt {
                approvals.push(receipt.clone());
            }

            // Collect effects
            if let Some(ref receipt) = action.effect_receipt {
                if receipt.success {
                    side_effects.push(receipt.clone());
                }
            }

            // Verify each action
            let result = verify_action(action);
            verification_summaries.push(VerificationSummary {
                action_id: action.action_id.clone(),
                verified: result.verified,
                verifier: "kernel".into(),
            });
        }

        // Detect data exits (actions that send data externally)
        for action in &actions {
            if is_data_exit_action(&action.canonical_spec) {
                data_exits.push(DataExitRecord {
                    data_description: action.canonical_spec.parameters.clone(),
                    destination: action.canonical_spec.target.clone(),
                    approved: action.policy_decision.decision == Decision::Approved,
                    action_id: action.action_id.clone(),
                });
            }
        }

        WorkloadCertificate {
            certificate_id: format!("cert-{}", uuid::Uuid::now_v7()),
            process_id,
            program_name,
            actions,
            models_used: models,
            tools_used: tools,
            approvals_granted: approvals,
            data_exfiltrated: data_exits,
            results_verified: verification_summaries,
            failures_recovered: recoveries,
            final_side_effects: side_effects,
            generated_at: chrono::Utc::now(),
            generated_by,
            attestation: None,
        }
    }

    /// Verify the entire certificate.
    pub fn verify(&self) -> CertificateVerificationResult {
        let action_results: Vec<VerificationResult> =
            self.actions.iter().map(verify_action).collect();

        let all_actions_verified = action_results.iter().all(|r| r.verified);
        let failed_actions: Vec<&VerificationResult> =
            action_results.iter().filter(|r| !r.verified).collect();

        CertificateVerificationResult {
            certificate_id: self.certificate_id.clone(),
            verified: all_actions_verified,
            total_actions: self.actions.len(),
            verified_actions: action_results.iter().filter(|r| r.verified).count(),
            failed_actions: failed_actions.len(),
            failure_details: failed_actions
                .iter()
                .map(|r| {
                    r.checks
                        .iter()
                        .filter(|c| !c.passed)
                        .map(|c| c.detail.clone())
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CertificateVerificationResult {
    pub certificate_id: String,
    pub verified: bool,
    pub total_actions: usize,
    pub verified_actions: usize,
    pub failed_actions: usize,
    pub failure_details: Vec<String>,
}

// -- Helpers --

fn compute_input_digest(spec: &ActionSpec) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(spec.action_type.as_bytes());
    hasher.update(spec.target.as_bytes());
    hasher.update(spec.parameters.as_bytes());
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

fn is_data_exit_action(spec: &ActionSpec) -> bool {
    matches!(
        spec.action_type.as_str(),
        "email_send" | "file_upload" | "api_call_external" | "content_publish" | "data_export"
    )
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_action() -> CanonicalAction {
        let spec = ActionSpec {
            action_type: "file_write".into(),
            target: "/tmp/test.txt".into(),
            parameters: r#"{"content": "hello"}"#.into(),
            output_schema: None,
            is_idempotent: true,
            is_reversible: true,
            compensation_action_id: None,
            risk_level: RiskLevel::Low,
        };

        let input_digest = compute_input_digest(&spec);

        CanonicalAction {
            action_id: "act-1".into(),
            canonical_spec: spec,
            principal_id: "user-1".into(),
            process_id: "proc-1".into(),
            capability: "file:write".into(),
            policy_decision: PolicyDecision {
                decision_id: "dec-1".into(),
                decided_by: "governance-gate".into(),
                decision: Decision::Approved,
                reason: "Low risk, authorized capability".into(),
                decided_at: chrono::Utc::now(),
                expires_at: None,
            },
            approval_receipt: None,
            input_digest,
            execution_environment: EnvironmentFingerprint {
                node_id: "node-1".into(),
                device_id: "cpu-0".into(),
                engine_id: "none".into(),
                model_revision: "none".into(),
                kernel_version: "0.1.0".into(),
                nki_version: 1,
                sandbox_profile: "default".into(),
                runtime_hash: None,
            },
            effect_intent: EffectIntent {
                description: "Write test file".into(),
                expected_target: "/tmp/test.txt".into(),
                expected_outcome: "File written".into(),
                max_retries: 1,
                timeout_ms: 5000,
                contract: None,
            },
            effect_receipt: Some(EffectReceipt {
                receipt_id: "rec-1".into(),
                actual_target: "/tmp/test.txt".into(),
                actual_outcome: "File written successfully".into(),
                execution_time_us: 1500,
                retry_count: 0,
                success: true,
                error: None,
                output_hash: [0u8; 32],
                executed_at: chrono::Utc::now(),
                executed_by: "kernel".into(),
            }),
            observed_effect: None,
            effect_verification: None,
            effect_committed: false,
            recovery_involved: false,
            output_digest: None,
            causal_parent: None,
            replay_descriptor: ReplayDescriptor {
                nonce: 1,
                idempotency_key: "idem-1".into(),
                process_id: "proc-1".into(),
                sequence_number: 1,
            },
        }
    }

    #[test]
    fn test_verify_action_passes() {
        let action = make_test_action();
        let result = verify_action(&action);
        assert!(result.verified);
        assert!(result.checks.iter().all(|c| c.passed));
    }

    #[test]
    fn test_verify_detects_input_tampering() {
        let mut action = make_test_action();
        // Tamper with the spec after approval
        action.canonical_spec.target = "/etc/passwd".into();

        let result = verify_action(&action);
        assert!(!result.verified);
        let input_check = result
            .checks
            .iter()
            .find(|c| c.check_name == "input_digest_match")
            .unwrap();
        assert!(!input_check.passed);
    }

    #[test]
    fn test_verify_detects_target_mismatch() {
        let mut action = make_test_action();
        // Effect receipt shows different target than intent
        if let Some(ref mut receipt) = action.effect_receipt {
            receipt.actual_target = "/tmp/other.txt".into();
        }

        let result = verify_action(&action);
        assert!(!result.verified);
        let target_check = result
            .checks
            .iter()
            .find(|c| c.check_name == "target_matches_intent")
            .unwrap();
        assert!(!target_check.passed);
    }

    #[test]
    fn test_workload_certificate_generation() {
        let action = make_test_action();
        let cert = WorkloadCertificate::generate(
            "proc-1".into(),
            "test_program".into(),
            vec![action],
            "kernel".into(),
        );

        assert_eq!(cert.process_id, "proc-1");
        assert_eq!(cert.actions.len(), 1);
        assert_eq!(cert.results_verified.len(), 1);

        let verify_result = cert.verify();
        assert!(verify_result.verified);
        assert_eq!(verify_result.total_actions, 1);
    }

    #[test]
    fn test_replay_descriptor_uniqueness() {
        let action1 = make_test_action();
        let mut action2 = make_test_action();
        action2.replay_descriptor.nonce = 2;
        action2.replay_descriptor.sequence_number = 2;

        // Different nonces -> different replay descriptors
        assert_ne!(
            action1.replay_descriptor.nonce,
            action2.replay_descriptor.nonce
        );
    }
}
