//! Generic state machine with CAS transitions, history, and hooks.
//!
//! Used by the kernel to ensure every state transition is:
//! 1. Valid (the transition is in the transition table)
//! 2. Atomic (CAS with generation check)
//! 3. Journaled (every transition is appended to the journal)
//! 4. Fenced (fencing token prevents split-brain)

use crate::journal::{EntryType, Journal, JournalEntry};
use nous_types::error::{ErrorCode, NousError};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashSet;
use std::fmt::Display;
use std::hash::Hash;

type TransitionHook<S> = Box<dyn Fn(&S, &S) -> Result<(), NousError> + Send + Sync>;

/// A state machine for an object with typed states.
///
/// S = state type (enum with valid transitions)
/// The state machine enforces valid transitions and records history.
pub struct StateMachine<S> {
    /// Current state.
    current: S,

    /// Valid transitions: (from, to) pairs.
    transitions: HashSet<(S, S)>,

    /// Transition history (latest first).
    history: Vec<TransitionRecord<S>>,

    /// Current generation (for CAS).
    generation: u64,

    /// Fencing token (changes on every transition).
    fencing_token: String,

    /// Pre-transition hooks.
    before_hooks: Vec<TransitionHook<S>>,

    /// Post-transition hooks.
    after_hooks: Vec<TransitionHook<S>>,
}

impl<S> StateMachine<S>
where
    S: Clone + PartialEq + Eq + Hash + Display,
{
    /// Create a new state machine with an initial state.
    pub fn new(initial_state: S) -> Self {
        Self {
            current: initial_state,
            transitions: HashSet::new(),
            history: Vec::new(),
            generation: 1,
            fencing_token: uuid::Uuid::now_v7().to_string(),
            before_hooks: Vec::new(),
            after_hooks: Vec::new(),
        }
    }

    /// Register a valid transition.
    pub fn allow_transition(&mut self, from: S, to: S) -> &mut Self {
        self.transitions.insert((from, to));
        self
    }

    /// Register multiple valid transitions from a single source state.
    pub fn allow_transitions_from(&mut self, from: S, to_states: Vec<S>) -> &mut Self {
        for to in to_states {
            self.transitions.insert((from.clone(), to));
        }
        self
    }

    /// Add a pre-transition hook (runs before the state changes).
    pub fn before_transition<F>(&mut self, hook: F) -> &mut Self
    where
        F: Fn(&S, &S) -> Result<(), NousError> + Send + Sync + 'static,
    {
        self.before_hooks.push(Box::new(hook));
        self
    }

    /// Add a post-transition hook (runs after the state changes).
    pub fn after_transition<F>(&mut self, hook: F) -> &mut Self
    where
        F: Fn(&S, &S) -> Result<(), NousError> + Send + Sync + 'static,
    {
        self.after_hooks.push(Box::new(hook));
        self
    }

    /// Return the current state.
    pub fn current(&self) -> &S {
        &self.current
    }

    /// Return the current generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Return the current fencing token.
    pub fn fencing_token(&self) -> &str {
        &self.fencing_token
    }

    /// Attempt a transition from the current state to a target state.
    ///
    /// Returns Err if:
    /// - The transition is not in the valid set
    /// - A pre-transition hook rejects it
    /// - The expected_generation doesn't match
    pub fn transition(
        &mut self,
        to: S,
        expected_generation: u64,
    ) -> Result<TransitionResult<S>, NousError> {
        let from = self.current.clone();

        // Check generation (CAS)
        if expected_generation != self.generation {
            return Err(NousError::new(
                ErrorCode::WorkloadConflict,
                format!(
                    "Generation conflict: expected {}, current {}",
                    expected_generation, self.generation
                ),
            ));
        }

        // Check valid transition
        if from != to && !self.transitions.contains(&(from.clone(), to.clone())) {
            return Err(NousError::new(
                ErrorCode::FailedPrecondition,
                format!("Invalid transition: '{}' -> '{}' is not allowed", from, to),
            ));
        }

        // Run before hooks
        for hook in &self.before_hooks {
            hook(&from, &to)?;
        }

        let previous = from.clone();
        let prev_generation = self.generation;
        let prev_token = self.fencing_token.clone();

        // Execute transition
        self.current = to.clone();
        self.generation += 1;
        self.fencing_token = uuid::Uuid::now_v7().to_string();

        // Record history
        let record = TransitionRecord {
            from: previous.clone(),
            to: to.clone(),
            generation: self.generation,
            fencing_token: self.fencing_token.clone(),
            timestamp: chrono::Utc::now(),
        };
        self.history.push(record.clone());

        // Run after hooks
        for hook in &self.after_hooks {
            if let Err(error) = hook(&previous, &to) {
                self.current = previous;
                self.generation = prev_generation;
                self.fencing_token = prev_token;
                self.history.pop();
                return Err(error);
            }
        }

        Ok(TransitionResult {
            from: previous,
            to,
            previous_generation: prev_generation,
            new_generation: self.generation,
            previous_fencing_token: prev_token,
            new_fencing_token: self.fencing_token.clone(),
            record,
        })
    }

    /// Try to transition with a journal append.
    ///
    /// If the transition succeeds, the result is appended to the journal.
    #[allow(clippy::too_many_arguments)]
    pub async fn transition_with_journal<T: Serialize + DeserializeOwned>(
        &mut self,
        to: S,
        expected_generation: u64,
        journal: &Journal,
        object_type: &str,
        object_id: &str,
        workload_id: &str,
        actor: &str,
        idempotency_key: Option<&str>,
        payload: &T,
    ) -> Result<TransitionResult<S>, NousError> {
        let result = self.transition(to, expected_generation)?;

        // Journal the transition
        let payload_bytes = serde_json::to_vec(payload).map_err(|e| {
            NousError::new(
                ErrorCode::Internal,
                format!("Failed to serialize payload: {}", e),
            )
        })?;

        let entry = JournalEntry {
            sequence: 0, // Assigned by journal
            workload_id: workload_id.to_string(),
            entry_type: EntryType::Transition,
            object_type: object_type.to_string(),
            object_id: object_id.to_string(),
            previous_phase: Some(result.from.to_string()),
            new_phase: result.to.to_string(),
            generation: result.new_generation,
            payload: payload_bytes,
            fencing_token: result.new_fencing_token.clone(),
            actor: actor.to_string(),
            idempotency_key: idempotency_key.map(|s| s.to_string()),
            timestamp_us: 0,
            checksum: Vec::new(),
        };

        if let Err(error) = journal.append(entry) {
            self.current = result.from.clone();
            self.generation = result.previous_generation;
            self.fencing_token = result.previous_fencing_token.clone();
            self.history.pop();
            return Err(error);
        }

        Ok(result)
    }

    /// Get transition history.
    pub fn history(&self) -> &[TransitionRecord<S>] {
        &self.history
    }
}

/// A record of a state transition.
#[derive(Debug, Clone)]
pub struct TransitionRecord<S> {
    pub from: S,
    pub to: S,
    pub generation: u64,
    pub fencing_token: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// The result of a successful transition.
#[derive(Debug, Clone)]
pub struct TransitionResult<S> {
    pub from: S,
    pub to: S,
    pub previous_generation: u64,
    pub new_generation: u64,
    pub previous_fencing_token: String,
    pub new_fencing_token: String,
    pub record: TransitionRecord<S>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    enum TestState {
        Created,
        Running,
        Done,
    }

    impl Display for TestState {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:?}", self)
        }
    }

    #[test]
    fn test_valid_transition() {
        let mut sm = StateMachine::new(TestState::Created);
        sm.allow_transition(TestState::Created, TestState::Running);
        sm.allow_transition(TestState::Running, TestState::Done);

        let result = sm.transition(TestState::Running, 1).unwrap();
        assert_eq!(result.to, TestState::Running);
        assert_eq!(result.new_generation, 2);
        assert_eq!(*sm.current(), TestState::Running);
    }

    #[test]
    fn test_invalid_transition() {
        let mut sm = StateMachine::new(TestState::Created);
        sm.allow_transition(TestState::Created, TestState::Running);

        let err = sm.transition(TestState::Done, 1).unwrap_err();
        assert_eq!(err.code, ErrorCode::FailedPrecondition);
    }

    #[test]
    fn test_generation_conflict() {
        let mut sm = StateMachine::new(TestState::Created);
        sm.allow_transition(TestState::Created, TestState::Running);

        let err = sm.transition(TestState::Running, 999).unwrap_err();
        assert_eq!(err.code, ErrorCode::WorkloadConflict);
    }

    #[test]
    fn test_noop_transition_allowed() {
        let mut sm = StateMachine::new(TestState::Created);
        // Same-state transition should always work
        let result = sm.transition(TestState::Created, 1).unwrap();
        assert_eq!(result.to, TestState::Created);
    }

    #[test]
    fn test_fencing_token_changes() {
        let mut sm = StateMachine::new(TestState::Created);
        sm.allow_transition(TestState::Created, TestState::Running);

        let old_token = sm.fencing_token().to_string();
        sm.transition(TestState::Running, 1).unwrap();
        assert_ne!(sm.fencing_token(), &old_token);
    }

    #[test]
    fn rejected_after_hook_restores_state_and_generation() {
        let mut sm = StateMachine::new(TestState::Created);
        sm.allow_transition(TestState::Created, TestState::Running)
            .after_transition(|_, _| {
                Err(NousError::new(
                    ErrorCode::FailedPrecondition,
                    "post-transition validation failed",
                ))
            });
        assert!(sm.transition(TestState::Running, 1).is_err());
        assert_eq!(sm.current(), &TestState::Created);
        assert_eq!(sm.generation(), 1);
        assert!(sm.history().is_empty());
    }
}
