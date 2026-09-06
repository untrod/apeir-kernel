//! Audit - security and governance audit records.

use serde::{Deserialize, Serialize};

/// An audit record for a security or governance decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub audit_id: String,
    pub workload_id: String,
    pub principal_id: String,
    pub action: String,
    pub decision: String,
    pub reason: String,
    pub context: Vec<u8>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
