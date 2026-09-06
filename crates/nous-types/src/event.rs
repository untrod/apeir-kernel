//! Event - journaled event for observability and replay.

use serde::{Deserialize, Serialize};

/// A single event in a workload's journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub workload_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub actor: String,
    pub payload: Vec<u8>,
    pub phase: String,
    pub trace_id: String,
}
