//! Object metadata - shared by all kernel objects.
//!
//! Every kernel object carries ObjectMeta which provides:
//! - Globally unique identity (UUID v7)
//! - Schema versioning
//! - Namespace isolation
//! - Optimistic concurrency (generation + resource_version)
//! - Health and condition reporting

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Metadata shared by every kernel object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMeta {
    /// Globally unique identifier (UUID v7 - time-ordered).
    pub uid: String,

    /// Schema version for this object type.
    pub schema_version: u32,

    /// Human-readable name (unique within namespace).
    pub name: String,

    /// Namespace for multi-tenancy.
    pub namespace: String,

    /// Principal who owns this object.
    pub owner_principal: String,

    /// Creation timestamp.
    pub created_at: DateTime<Utc>,

    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,

    /// Monotonically increasing generation (for CAS updates).
    pub generation: u64,

    /// Opaque resource version (for efficient watch resumption).
    pub resource_version: String,

    /// Labels for indexing and selection.
    #[serde(default)]
    pub labels: HashMap<String, String>,

    /// Annotations for non-identifying metadata.
    #[serde(default)]
    pub annotations: HashMap<String, String>,

    /// Current lifecycle phase (object-type-specific).
    pub phase: String,

    /// Human-readable reason for current phase.
    #[serde(default)]
    pub phase_reason: String,

    /// Aggregate health status.
    #[serde(default)]
    pub health: HealthStatus,

    /// Typed conditions for detailed status.
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// W3C TraceContext trace ID.
    #[serde(default)]
    pub trace_id: String,

    /// Audit trail metadata.
    #[serde(default)]
    pub audit: Option<AuditMetadata>,
}

impl ObjectMeta {
    /// Create a new ObjectMeta with a fresh UUID v7 and current timestamp.
    pub fn new(name: String, namespace: String, owner_principal: String) -> Self {
        let now = Utc::now();
        Self {
            uid: Uuid::now_v7().to_string(),
            schema_version: 1,
            name,
            namespace,
            owner_principal,
            created_at: now,
            updated_at: now,
            generation: 1,
            resource_version: "1".to_string(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            phase: "CREATED".to_string(),
            phase_reason: String::new(),
            health: HealthStatus::Unknown,
            conditions: Vec::new(),
            trace_id: String::new(),
            audit: None,
        }
    }

    /// Bump generation and update timestamp.
    pub fn bump_generation(&mut self) {
        self.generation += 1;
        self.updated_at = Utc::now();
        self.resource_version = self.generation.to_string();
    }

    /// Set the current phase and record the reason.
    pub fn set_phase(&mut self, phase: impl Into<String>, reason: impl Into<String>) {
        self.phase = phase.into();
        self.phase_reason = reason.into();
        self.bump_generation();
    }

    /// Add or update a condition.
    pub fn set_condition(&mut self, condition: Condition) {
        self.conditions
            .retain(|c| c.condition_type != condition.condition_type);
        self.conditions.push(condition);
        self.bump_generation();
    }
}

/// A typed condition reporting the status of a specific aspect of an object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    /// Unique condition type (e.g., "Ready", "MemoryPressure", "LeaseExpired").
    #[serde(rename = "type")]
    pub condition_type: String,

    /// Status of this condition.
    pub status: ConditionStatus,

    /// Machine-readable reason for the last transition.
    pub reason: String,

    /// Human-readable message.
    pub message: String,

    /// Last time this condition transitioned.
    pub last_transition_time: DateTime<Utc>,

    /// Last time this condition was observed.
    pub observed_time: DateTime<Utc>,
}

impl Condition {
    pub fn new_true(
        condition_type: impl Into<String>,
        reason: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            condition_type: condition_type.into(),
            status: ConditionStatus::True,
            reason: reason.into(),
            message: message.into(),
            last_transition_time: now,
            observed_time: now,
        }
    }

    pub fn new_false(
        condition_type: impl Into<String>,
        reason: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            condition_type: condition_type.into(),
            status: ConditionStatus::False,
            reason: reason.into(),
            message: message.into(),
            last_transition_time: now,
            observed_time: now,
        }
    }
}

/// Condition status - True, False, or Unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConditionStatus {
    #[serde(rename = "True")]
    True,
    #[serde(rename = "False")]
    False,
    #[default]
    #[serde(rename = "Unknown")]
    Unknown,
}

/// Aggregate health status for an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HealthStatus {
    #[default]
    Unknown,
    Healthy,
    Degraded,
    Unhealthy,
    Critical,
}

/// Audit trail metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditMetadata {
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub updated_by: String,
    #[serde(default)]
    pub approved_by: String,
    #[serde(default)]
    pub deployment: String,
}
