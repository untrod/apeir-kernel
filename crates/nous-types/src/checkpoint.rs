//! Checkpoint - workload state snapshots for recovery.

use serde::{Deserialize, Serialize};

/// A workload state checkpoint for recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub workload_id: String,
    pub sequence: u64,
    pub state_snapshot: Vec<u8>,
    pub node_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub sha256: Vec<u8>,
}

/// A lightweight reference to a checkpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub checkpoint_id: String,
    pub sequence: u64,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}
