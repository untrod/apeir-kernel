//! Recovery integration tests.
//!
//! These tests verify the recovery subsystem can handle real crash scenarios.
//! Run with: cargo test -p nous-state -- recovery

#[cfg(test)]
mod recovery_integration_tests {
    use crate::journal::{EntryType, Journal, JournalEntry};
    use crate::recovery;

    fn make_workload_entry(
        workload_id: &str,
        phase: &str,
        generation: u64,
        key: &str,
    ) -> JournalEntry {
        JournalEntry {
            sequence: 0,
            workload_id: workload_id.to_string(),
            entry_type: EntryType::Transition,
            object_type: "Workload".into(),
            object_id: workload_id.to_string(),
            previous_phase: if generation > 1 {
                Some("CREATED".into())
            } else {
                None
            },
            new_phase: phase.to_string(),
            generation,
            payload: b"{}".to_vec(),
            fencing_token: format!("token-{}", generation),
            actor: "test".into(),
            idempotency_key: Some(key.to_string()),
            timestamp_us: 0,
            checksum: vec![],
        }
    }

    // -- Scenario 1: Kernel crash after admission --

    #[test]
    fn test_recovery_after_admission_crash() {
        let journal = Journal::open_in_memory().unwrap();

        // Simulate: workload submitted, admitted, then kernel crashes
        journal
            .append(make_workload_entry("wl-1", "CREATED", 1, "ik-wl1-intent"))
            .unwrap();
        journal
            .append(make_workload_entry(
                "wl-1",
                "VALIDATING",
                2,
                "ik-wl1-CREATED-to-VALIDATING",
            ))
            .unwrap();
        journal
            .append(make_workload_entry(
                "wl-1",
                "VALIDATED",
                3,
                "ik-wl1-VALIDATING-to-VALIDATED",
            ))
            .unwrap();
        journal
            .append(make_workload_entry(
                "wl-1",
                "ADMITTED",
                4,
                "ik-wl1-VALIDATED-to-ADMITTED",
            ))
            .unwrap();
        // CRASH - kernel never placed or executed

        // Recover
        let (state, actions) = recovery::recover(&journal).unwrap();

        // Should find wl-1 as interrupted (ADMITTED is not terminal)
        assert!(state.interrupted_workloads.contains(&"wl-1".to_string()));
        // Should be in the active workloads list
        assert!(state.active_workloads.contains(&"wl-1".to_string()));
        // Should generate a Resume action
        let has_resume = actions
            .iter()
            .any(|a| matches!(a, recovery::RecoveryAction::Resume { .. }));
        assert!(
            has_resume,
            "Recovery should generate Resume action for ADMITTED workload"
        );
    }

    // -- Scenario 2: Kernel crash after lease grant --

    #[test]
    fn test_recovery_after_lease_grant_crash() {
        let journal = Journal::open_in_memory().unwrap();

        // Simulate: lease granted, placement done, then crash
        for (i, phase) in [
            "CREATED",
            "VALIDATING",
            "VALIDATED",
            "ADMITTED",
            "PLACED",
            "PREPARING",
        ]
        .iter()
        .enumerate()
        {
            let gen = (i + 1) as u64;
            let prev = if i > 0 {
                ["CREATED", "VALIDATING", "VALIDATED", "ADMITTED", "PLACED"][i - 1]
            } else {
                "CREATED"
            };
            journal
                .append(make_workload_entry(
                    "wl-2",
                    phase,
                    gen,
                    &format!(
                        "ik-wl2-{}-to-{}",
                        if i == 0 { "CREATED" } else { prev },
                        phase
                    ),
                ))
                .unwrap();
        }
        // CRASH

        let (state, actions) = recovery::recover(&journal).unwrap();
        assert!(state.interrupted_workloads.contains(&"wl-2".to_string()));

        // Should generate Resume (PREPARING is active, not terminal)
        let has_resume = actions
            .iter()
            .any(|a| matches!(a, recovery::RecoveryAction::Resume { .. }));
        assert!(has_resume);
    }

    // -- Scenario 3: Workload completed successfully - no recovery needed --

    #[test]
    fn test_no_recovery_for_succeeded_workload() {
        let journal = Journal::open_in_memory().unwrap();

        for (i, phase) in [
            "CREATED",
            "VALIDATING",
            "VALIDATED",
            "ADMITTED",
            "PLACED",
            "PREPARING",
            "RUNNING",
            "SUCCEEDED",
        ]
        .iter()
        .enumerate()
        {
            let gen = (i + 1) as u64;
            let prev = if i > 0 {
                [
                    "CREATED",
                    "VALIDATING",
                    "VALIDATED",
                    "ADMITTED",
                    "PLACED",
                    "PREPARING",
                    "RUNNING",
                ][i - 1]
            } else {
                "CREATED"
            };
            journal
                .append(make_workload_entry(
                    "wl-3",
                    phase,
                    gen,
                    &format!("ik-wl3-{}-to-{}", prev, phase),
                ))
                .unwrap();
        }

        let (state, actions) = recovery::recover(&journal).unwrap();
        // SUCCEEDED is terminal - should NOT be in interrupted list
        assert!(!state.interrupted_workloads.contains(&"wl-3".to_string()));
        // Should have zero actions for this workload
        let wl3_actions: Vec<_> = actions
            .iter()
            .filter(|a| match a {
                recovery::RecoveryAction::Resume { workload_id, .. } => workload_id == "wl-3",
                recovery::RecoveryAction::Restart { workload_id } => workload_id == "wl-3",
                recovery::RecoveryAction::MarkLost { workload_id } => workload_id == "wl-3",
                recovery::RecoveryAction::MarkFailed { workload_id, .. } => workload_id == "wl-3",
            })
            .collect();
        assert!(
            wl3_actions.is_empty(),
            "Terminal workload should have no recovery actions"
        );
    }

    // -- Scenario 4: Duplicate idempotency keys --

    #[test]
    fn test_duplicate_idempotency_key_handling() {
        let journal = Journal::open_in_memory().unwrap();

        // First submission
        let seq1 = journal
            .append(make_workload_entry("wl-4", "CREATED", 1, "dup-key-1"))
            .unwrap();

        // Retry with same key - should be idempotent
        let seq2 = journal
            .append(make_workload_entry("wl-4", "CREATED", 1, "dup-key-1"))
            .unwrap();

        assert_eq!(
            seq1, seq2,
            "Duplicate idempotency key must return same sequence"
        );
        // Only one entry should exist
        let entries = journal.read_workload("wl-4").unwrap();
        assert_eq!(entries.len(), 1);
    }

    // -- Scenario 5: Cancel during execution --

    #[test]
    fn test_recovery_after_cancel_during_execution() {
        let journal = Journal::open_in_memory().unwrap();

        // Workload progressed to RUNNING, then cancelled
        for (i, phase) in [
            "CREATED",
            "VALIDATING",
            "VALIDATED",
            "ADMITTED",
            "PLACED",
            "PREPARING",
            "RUNNING",
            "CANCELLED",
        ]
        .iter()
        .enumerate()
        {
            let gen = (i + 1) as u64;
            let prev = if i > 0 {
                [
                    "CREATED",
                    "VALIDATING",
                    "VALIDATED",
                    "ADMITTED",
                    "PLACED",
                    "PREPARING",
                    "RUNNING",
                ][i - 1]
            } else {
                "CREATED"
            };
            journal
                .append(make_workload_entry(
                    "wl-5",
                    phase,
                    gen,
                    &format!("ik-wl5-{}-to-{}", prev, phase),
                ))
                .unwrap();
        }

        let (state, _actions) = recovery::recover(&journal).unwrap();
        // CANCELLED is terminal
        assert!(!state.interrupted_workloads.contains(&"wl-5".to_string()));
    }

    // -- Scenario 6: Journal replay produces correct sequence numbers --

    #[test]
    fn test_journal_replay_sequence_monotonic() {
        let journal = Journal::open_in_memory().unwrap();

        for i in 0..20 {
            journal
                .append(make_workload_entry(
                    &format!("wl-{}", i),
                    "CREATED",
                    1,
                    &format!("key-{}", i),
                ))
                .unwrap();
        }

        let entries = journal.read_from(0).unwrap();
        assert_eq!(entries.len(), 20);

        // Verify monotonic sequences
        for i in 1..entries.len() {
            assert!(
                entries[i].sequence > entries[i - 1].sequence,
                "Sequence must be strictly increasing: {} <= {}",
                entries[i].sequence,
                entries[i - 1].sequence
            );
        }
    }

    // -- Scenario 7: Replay produces same state as original --

    #[test]
    fn test_journal_replay_deterministic() {
        let journal = Journal::open_in_memory().unwrap();

        // Write a complex sequence
        let workloads = ["wl-a", "wl-b", "wl-c"];
        let phases = ["CREATED", "VALIDATING", "VALIDATED", "ADMITTED"];

        for wl in &workloads {
            for (i, phase) in phases.iter().enumerate() {
                let prev = if i > 0 { phases[i - 1] } else { "CREATED" };
                journal
                    .append(make_workload_entry(
                        wl,
                        phase,
                        (i + 1) as u64,
                        &format!("{}-{}-to-{}", wl, prev, phase),
                    ))
                    .unwrap();
            }
        }

        // First replay
        let entries1 = journal.read_from(0).unwrap();

        // Second replay should be identical
        let entries2 = journal.read_from(0).unwrap();

        assert_eq!(entries1.len(), entries2.len());
        for i in 0..entries1.len() {
            assert_eq!(entries1[i].sequence, entries2[i].sequence);
            assert_eq!(entries1[i].workload_id, entries2[i].workload_id);
            assert_eq!(entries1[i].new_phase, entries2[i].new_phase);
            assert_eq!(entries1[i].generation, entries2[i].generation);
        }
    }
}
