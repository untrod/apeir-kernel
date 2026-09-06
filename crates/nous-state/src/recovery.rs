//! Crash recovery - reconstruct kernel state from the journal.
//!
//! On startup after a crash, the kernel:
//! 1. Opens the journal
//! 2. Replays all entries to reconstruct state
//! 3. Re-acquires leases that were active
//! 4. Marks lost workloads (no lease, no heartbeat) as LOST
//! 5. Resumes running workloads that still have valid leases

#[cfg(test)]
use crate::journal::JournalEntry;
use crate::journal::{EntryType, Journal};
use nous_types::error::NousError;
use std::collections::HashMap;

/// The result of replaying the journal during recovery.
#[derive(Debug, Default)]
pub struct RecoveryState {
    /// Reconstructed workload states: workload_id -> current_phase
    pub workload_phases: HashMap<String, String>,

    /// Reconstructed workload generations: workload_id -> generation
    pub workload_generations: HashMap<String, u64>,

    /// Active (non-terminal) workloads at time of crash.
    pub active_workloads: Vec<String>,

    /// Workloads that were RUNNING or PREPARING at time of crash.
    pub interrupted_workloads: Vec<String>,

    /// Workloads with valid leases at time of crash.
    pub leased_workloads: Vec<String>,

    /// Total entries replayed.
    pub entries_replayed: u64,

    /// Highest sequence number replayed.
    pub max_sequence: u64,
}

/// Replay the journal to reconstruct kernel state.
///
/// This is idempotent - replaying the same journal twice produces the same result.
pub fn replay_journal(journal: &Journal) -> Result<RecoveryState, NousError> {
    let entries = journal.read_from(0)?;
    let mut state = RecoveryState::default();

    for entry in &entries {
        state.entries_replayed += 1;
        state.max_sequence = entry.sequence;

        if entry.object_type == "Workload" {
            // Track current phase
            state
                .workload_phases
                .insert(entry.object_id.clone(), entry.new_phase.clone());
            state
                .workload_generations
                .insert(entry.object_id.clone(), entry.generation);

            // Track active workloads
            let is_terminal = matches!(
                entry.new_phase.as_str(),
                "REJECTED" | "SUCCEEDED" | "FAILED" | "CANCELLED" | "LOST" | "QUARANTINED"
            );

            if is_terminal {
                state.active_workloads.retain(|id| id != &entry.object_id);
                state
                    .interrupted_workloads
                    .retain(|id| id != &entry.object_id);
                state.leased_workloads.retain(|id| id != &entry.object_id);
            } else {
                if !state.active_workloads.contains(&entry.object_id) {
                    state.active_workloads.push(entry.object_id.clone());
                }
            }

            // Track interrupted workloads (were running at crash)
            let is_active_phase = matches!(
                entry.new_phase.as_str(),
                "ADMITTED"
                    | "PLACED"
                    | "PREPARING"
                    | "RUNNING"
                    | "QUIESCING"
                    | "CHECKPOINTING"
                    | "RECOVERING"
            );

            if is_active_phase && !state.interrupted_workloads.contains(&entry.object_id) {
                state.interrupted_workloads.push(entry.object_id.clone());
            }

            // Track leased workloads
            if entry.entry_type == EntryType::Commit
                && !state.leased_workloads.contains(&entry.object_id)
            {
                state.leased_workloads.push(entry.object_id.clone());
            }
        }
    }

    Ok(state)
}

/// Determine what to do with interrupted workloads after recovery.
#[derive(Debug)]
pub enum RecoveryAction {
    /// Resume from the last checkpoint.
    Resume {
        workload_id: String,
        last_phase: String,
    },
    /// Restart from scratch (no valid checkpoint).
    Restart { workload_id: String },
    /// Mark as lost (no lease, no heartbeat).
    MarkLost { workload_id: String },
    /// Mark as failed (recovery not possible).
    MarkFailed { workload_id: String, reason: String },
}

/// Compute recovery actions for interrupted workloads.
pub fn compute_recovery_actions(state: &RecoveryState) -> Vec<RecoveryAction> {
    let mut actions = Vec::new();

    for workload_id in &state.interrupted_workloads {
        let current_phase = state
            .workload_phases
            .get(workload_id)
            .map(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        // Admission and preparation are idempotent recovery points. Running work
        // requires an explicit committed lease before it can be resumed safely.
        let resumable_pre_execution = matches!(current_phase, "ADMITTED" | "PLACED" | "PREPARING");
        if state.leased_workloads.contains(workload_id) || resumable_pre_execution {
            actions.push(RecoveryAction::Resume {
                workload_id: workload_id.clone(),
                last_phase: current_phase.to_string(),
            });
        } else {
            // No lease - can't guarantee resources, mark lost
            actions.push(RecoveryAction::MarkLost {
                workload_id: workload_id.clone(),
            });
        }
    }

    actions
}

/// Replay and log recovery summary.
pub fn recover(journal: &Journal) -> Result<(RecoveryState, Vec<RecoveryAction>), NousError> {
    let state = replay_journal(journal)?;
    let actions = compute_recovery_actions(&state);

    tracing::info!(
        entries_replayed = state.entries_replayed,
        active_workloads = state.active_workloads.len(),
        interrupted = state.interrupted_workloads.len(),
        leased = state.leased_workloads.len(),
        actions = actions.len(),
        "Journal recovery complete"
    );

    for action in &actions {
        match action {
            RecoveryAction::Resume {
                workload_id,
                last_phase,
            } => {
                tracing::info!(%workload_id, %last_phase, "Resuming workload");
            }
            RecoveryAction::Restart { workload_id } => {
                tracing::info!(%workload_id, "Restarting workload from scratch");
            }
            RecoveryAction::MarkLost { workload_id } => {
                tracing::warn!(%workload_id, "Marking workload as LOST (no valid lease)");
            }
            RecoveryAction::MarkFailed {
                workload_id,
                reason,
            } => {
                tracing::error!(%workload_id, %reason, "Marking workload as FAILED");
            }
        }
    }

    Ok((state, actions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;

    fn create_test_entry(
        sequence: u64,
        workload_id: &str,
        new_phase: &str,
        entry_type: EntryType,
        generation: u64,
    ) -> JournalEntry {
        JournalEntry {
            sequence,
            workload_id: workload_id.to_string(),
            entry_type,
            object_type: "Workload".to_string(),
            object_id: workload_id.to_string(),
            previous_phase: Some("CREATED".to_string()),
            new_phase: new_phase.to_string(),
            generation,
            payload: b"{}".to_vec(),
            fencing_token: format!("node-1:{}", sequence),
            actor: "test".to_string(),
            idempotency_key: None,
            timestamp_us: 0,
            checksum: Vec::new(),
        }
    }

    #[test]
    fn test_replay_reconstructs_phase() {
        let journal = Journal::open_in_memory().unwrap();
        journal
            .append(create_test_entry(1, "w-1", "CREATED", EntryType::Intent, 1))
            .unwrap();
        journal
            .append(create_test_entry(
                2,
                "w-1",
                "VALIDATED",
                EntryType::Transition,
                2,
            ))
            .unwrap();
        journal
            .append(create_test_entry(
                3,
                "w-1",
                "RUNNING",
                EntryType::Transition,
                3,
            ))
            .unwrap();

        let state = replay_journal(&journal).unwrap();
        assert_eq!(state.workload_phases.get("w-1").unwrap(), "RUNNING");
        assert_eq!(state.entries_replayed, 3);
        assert!(state.interrupted_workloads.contains(&"w-1".to_string()));
    }

    #[test]
    fn test_replay_terminal_removes_from_active() {
        let journal = Journal::open_in_memory().unwrap();
        journal
            .append(create_test_entry(
                1,
                "w-1",
                "RUNNING",
                EntryType::Transition,
                1,
            ))
            .unwrap();
        journal
            .append(create_test_entry(
                2,
                "w-1",
                "SUCCEEDED",
                EntryType::Transition,
                2,
            ))
            .unwrap();

        let state = replay_journal(&journal).unwrap();
        assert_eq!(state.workload_phases.get("w-1").unwrap(), "SUCCEEDED");
        assert!(!state.active_workloads.contains(&"w-1".to_string()));
        assert!(!state.interrupted_workloads.contains(&"w-1".to_string()));
    }

    #[test]
    fn test_recovery_actions_for_interrupted() {
        let journal = Journal::open_in_memory().unwrap();
        journal
            .append(create_test_entry(
                1,
                "w-leased",
                "RUNNING",
                EntryType::Commit,
                3,
            ))
            .unwrap();
        journal
            .append(create_test_entry(
                2,
                "w-nolease",
                "RUNNING",
                EntryType::Transition,
                2,
            ))
            .unwrap();

        let state = replay_journal(&journal).unwrap();
        let actions = compute_recovery_actions(&state);

        // w-leased has a commit entry -> Resume
        assert!(actions.iter().any(
            |a| matches!(a, RecoveryAction::Resume { workload_id, .. } if workload_id == "w-leased")
        ));

        // w-nolease has no commit -> MarkLost
        assert!(actions.iter().any(
            |a| matches!(a, RecoveryAction::MarkLost { workload_id } if workload_id == "w-nolease")
        ));
    }
}
