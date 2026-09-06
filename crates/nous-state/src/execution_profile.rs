use crate::journal::{EntryType, Journal, JournalEntry};
use nous_types::error::{ErrorCode, NousError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionProfile {
    pub operation_id: String,
    pub workload_id: String,
    pub task_type: String,
    pub task_class: String,
    pub model: String,
    pub provider: String,
    pub execution_domain: String,
    pub device: String,
    pub latency_ms: u64,
    pub queue_latency_ms: u64,
    pub state_transfer_ms: Option<u64>,
    pub recovery_latency_ms: Option<u64>,
    pub cost_microcents: Option<u64>,
    pub failure_code: Option<String>,
    pub retry_count: u32,
    pub recovered: bool,
    pub quality_signal: Option<f64>,
    pub recorded_at_us: i64,
}

impl Default for ExecutionProfile {
    fn default() -> Self {
        Self {
            operation_id: String::new(),
            workload_id: String::new(),
            task_type: String::new(),
            task_class: String::new(),
            model: String::new(),
            provider: String::new(),
            execution_domain: "unknown".into(),
            device: String::new(),
            latency_ms: 0,
            queue_latency_ms: 0,
            state_transfer_ms: None,
            recovery_latency_ms: None,
            cost_microcents: None,
            failure_code: None,
            retry_count: 0,
            recovered: false,
            quality_signal: None,
            recorded_at_us: 0,
        }
    }
}

pub type ModelExecutionProfile = ExecutionProfile;

/// Journal-backed query facade. Profiles are evidence only and cannot mutate
/// scheduler or safety policy.
pub struct ExecutionProfileStore;

impl ExecutionProfileStore {
    pub fn record(journal: &Journal, mut profile: ExecutionProfile) -> Result<u64, NousError> {
        profile.recorded_at_us = chrono::Utc::now().timestamp_micros();
        let payload = serde_json::to_vec(&profile).map_err(|error| {
            NousError::new(
                ErrorCode::Internal,
                format!("Cannot encode execution profile: {error}"),
            )
        })?;
        let outcome = if profile.failure_code.is_some() {
            "FAILED"
        } else {
            "COMPLETED"
        };
        journal.append(JournalEntry {
            sequence: 0,
            workload_id: profile.workload_id.clone(),
            entry_type: EntryType::Observation,
            object_type: "ExecutionProfile".into(),
            object_id: format!("{}:{}:{outcome}", profile.operation_id, profile.provider),
            previous_phase: None,
            new_phase: outcome.into(),
            generation: 1,
            payload,
            fencing_token: String::new(),
            actor: "kernel-runtime".into(),
            idempotency_key: Some(format!(
                "operation:{}:profile:{}:{outcome}",
                profile.operation_id, profile.provider
            )),
            timestamp_us: profile.recorded_at_us,
            checksum: Vec::new(),
        })
    }

    pub fn list(journal: &Journal) -> Result<Vec<ExecutionProfile>, NousError> {
        journal
            .read_from(0)?
            .into_iter()
            .filter(|entry| entry.object_type == "ExecutionProfile")
            .map(|entry| {
                serde_json::from_slice(&entry.payload).map_err(|error| {
                    NousError::new(
                        ErrorCode::DataLoss,
                        format!("Invalid execution profile: {error}"),
                    )
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_durable_and_idempotent() {
        let journal = Journal::open_in_memory().unwrap();
        let profile = ExecutionProfile {
            operation_id: "operation-1".into(),
            workload_id: "workload-1".into(),
            task_type: "chat".into(),
            task_class: "interactive".into(),
            model: "model-a".into(),
            provider: "provider-a".into(),
            execution_domain: "local".into(),
            device: "cpu".into(),
            latency_ms: 10,
            queue_latency_ms: 1,
            state_transfer_ms: Some(0),
            recovery_latency_ms: None,
            cost_microcents: Some(0),
            failure_code: None,
            retry_count: 0,
            recovered: false,
            quality_signal: None,
            recorded_at_us: 0,
        };
        ExecutionProfileStore::record(&journal, profile.clone()).unwrap();
        ExecutionProfileStore::record(&journal, profile).unwrap();
        let profiles = ExecutionProfileStore::list(&journal).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].operation_id, "operation-1");
    }
}
