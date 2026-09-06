//! Journal-backed semantic state with explicit ownership and CAS updates.

use crate::journal::{EntryType, Journal, JournalEntry};
use nous_types::error::{ErrorCode, NousError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StateRepresentation {
    #[default]
    Raw,
    Structured,
    Summary,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticStateRecord {
    pub state_id: String,
    pub namespace: String,
    pub semantic_type: String,
    #[serde(default)]
    pub representation: StateRepresentation,
    pub owner: String,
    #[serde(default)]
    pub source_model_revision: String,
    pub generation: u64,
    pub value: serde_json::Value,
    pub evidence_refs: Vec<String>,
    pub updated_at_us: i64,
}

/// Materialized view of semantic state. The journal remains authoritative.
pub struct SemanticStateFabric {
    records: RwLock<HashMap<String, SemanticStateRecord>>,
}

impl Default for SemanticStateFabric {
    fn default() -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
        }
    }
}

impl SemanticStateFabric {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, state_id: &str) -> Option<SemanticStateRecord> {
        self.records.read().ok()?.get(state_id).cloned()
    }

    pub fn put(
        &self,
        journal: &Journal,
        mut record: SemanticStateRecord,
        expected_generation: u64,
        actor: &str,
        idempotency_key: &str,
    ) -> Result<u64, NousError> {
        let mut records = self
            .records
            .write()
            .map_err(|_| NousError::new(ErrorCode::Internal, "semantic state lock is poisoned"))?;
        let current_generation = records
            .get(&record.state_id)
            .map(|current| current.generation)
            .unwrap_or(0);
        if current_generation != expected_generation {
            return Err(NousError::new(
                ErrorCode::WorkloadConflict,
                format!(
                    "Semantic state generation conflict: expected {expected_generation}, current {current_generation}"
                ),
            ));
        }

        record.generation = current_generation + 1;
        record.updated_at_us = chrono::Utc::now().timestamp_micros();
        let payload = serde_json::to_vec(&record).map_err(|error| {
            NousError::new(
                ErrorCode::Internal,
                format!("Cannot encode semantic state: {error}"),
            )
        })?;
        let sequence = journal.append(JournalEntry {
            sequence: 0,
            workload_id: record.state_id.clone(),
            entry_type: EntryType::Commit,
            object_type: "SemanticState".into(),
            object_id: record.state_id.clone(),
            previous_phase: Some(
                if current_generation == 0 {
                    "ABSENT"
                } else {
                    "ACTIVE"
                }
                .into(),
            ),
            new_phase: "ACTIVE".into(),
            generation: record.generation,
            payload,
            fencing_token: format!("semantic:{}:{}", record.state_id, record.generation),
            actor: actor.into(),
            idempotency_key: Some(idempotency_key.into()),
            timestamp_us: record.updated_at_us,
            checksum: Vec::new(),
        })?;
        records.insert(record.state_id.clone(), record);
        Ok(sequence)
    }

    pub fn replay(journal: &Journal) -> Result<Self, NousError> {
        let fabric = Self::new();
        for entry in journal.read_from(0)? {
            if entry.object_type == "SemanticState" && entry.entry_type == EntryType::Commit {
                let record: SemanticStateRecord =
                    serde_json::from_slice(&entry.payload).map_err(|error| {
                        NousError::new(
                            ErrorCode::DataLoss,
                            format!("Invalid semantic state journal entry: {error}"),
                        )
                    })?;
                fabric
                    .records
                    .write()
                    .map_err(|_| {
                        NousError::new(ErrorCode::Internal, "semantic state lock is poisoned")
                    })?
                    .insert(record.state_id.clone(), record);
            }
        }
        Ok(fabric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(value: i32) -> SemanticStateRecord {
        SemanticStateRecord {
            state_id: "machine:1".into(),
            namespace: "factory".into(),
            semantic_type: "MachineState".into(),
            representation: StateRepresentation::Structured,
            owner: "controller".into(),
            source_model_revision: "model-a".into(),
            generation: 0,
            value: serde_json::json!({"temperature": value}),
            evidence_refs: vec!["evidence:1".into()],
            updated_at_us: 0,
        }
    }

    #[test]
    fn state_updates_are_journaled_and_replayable() {
        let journal = Journal::open_in_memory().unwrap();
        let fabric = SemanticStateFabric::new();
        fabric
            .put(&journal, record(42), 0, "test", "semantic-1")
            .unwrap();
        assert_eq!(fabric.get("machine:1").unwrap().generation, 1);
        let replayed = SemanticStateFabric::replay(&journal).unwrap();
        assert_eq!(replayed.get("machine:1").unwrap().value["temperature"], 42);
    }

    #[test]
    fn stale_generation_is_rejected() {
        let journal = Journal::open_in_memory().unwrap();
        let fabric = SemanticStateFabric::new();
        fabric
            .put(&journal, record(42), 0, "test", "semantic-1")
            .unwrap();
        assert_eq!(
            fabric
                .put(&journal, record(43), 0, "test", "semantic-2")
                .unwrap_err()
                .code,
            ErrorCode::WorkloadConflict
        );
    }

    #[test]
    fn concurrent_compare_and_swap_has_one_winner() {
        let journal = std::sync::Arc::new(Journal::open_in_memory().unwrap());
        let fabric = std::sync::Arc::new(SemanticStateFabric::new());
        let mut workers = Vec::new();
        for index in 0..16 {
            let journal = journal.clone();
            let fabric = fabric.clone();
            workers.push(std::thread::spawn(move || {
                fabric.put(
                    &journal,
                    record(index),
                    0,
                    "test",
                    &format!("semantic-{index}"),
                )
            }));
        }
        let successes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(Result::is_ok)
            .count();
        assert_eq!(successes, 1);
        assert_eq!(fabric.get("machine:1").unwrap().generation, 1);
    }

    #[test]
    fn state_is_model_neutral_across_replay() {
        let journal = Journal::open_in_memory().unwrap();
        let first = SemanticStateFabric::new();
        first
            .put(&journal, record(42), 0, "model-a", "semantic-model-a")
            .unwrap();

        let second = SemanticStateFabric::replay(&journal).unwrap();
        let mut migrated = second.get("machine:1").unwrap();
        assert_eq!(migrated.source_model_revision, "model-a");
        assert_eq!(migrated.value["temperature"], 42);
        migrated.source_model_revision = "model-b".into();
        migrated.value = serde_json::json!({"temperature": 43, "continued_by": "model-b"});
        second
            .put(&journal, migrated, 1, "model-b", "semantic-model-b")
            .unwrap();
        let final_state = SemanticStateFabric::replay(&journal)
            .unwrap()
            .get("machine:1")
            .unwrap();
        assert_eq!(final_state.generation, 2);
        assert_eq!(final_state.source_model_revision, "model-b");
        assert_eq!(final_state.value["continued_by"], "model-b");
    }
}
