//! nous-state - Append-only state journal with CAS transitions.
//!
//! The journal is the single source of truth for all kernel state.
//! Every state transition is recorded as an append-only entry.
//! On crash recovery, the journal is replayed to reconstruct state.
//!
//! Design:
//! 1. Append-only - entries are never modified or deleted (only truncated by retention)
//! 2. CAS transitions - every state change includes the expected previous generation
//! 3. Fencing - generation tokens prevent split-brain
//! 4. Replayable - kernel restart replays journal to recover state
//! 5. Idempotent - replaying the same entry is safe

pub mod execution_profile;
pub mod fencing;
pub mod journal;
pub mod recovery;
pub mod semantic;
pub mod state_machine;

pub use execution_profile::{ExecutionProfile, ExecutionProfileStore, ModelExecutionProfile};
pub use semantic::{SemanticStateFabric, SemanticStateRecord, StateRepresentation};

#[cfg(test)]
mod recovery_tests;

use nous_types::error::NousError;

/// Result type for state operations.
pub type StateResult<T> = Result<T, NousError>;
