//! nous-nki - NKI v1 protocol: request/response types, transport, client/server.
//!
//! NKI is the sole entry point to the APEIR Kernel. All external communication
//! (desktop, CLI, SDK, web) must go through NKI.

pub mod envelope;
pub mod methods;
pub mod transport;

#[cfg(test)]
mod golden_tests;

use uuid::Uuid;

/// NKI v1alpha2 schema version.
/// Breaking change from v1alpha1: tagged enum (status: "success" | "error").
/// v1alpha1 clients remain compatible due to #[serde(flatten)] - error field still at top level.
/// See docs/compatibility/BREAKING_CHANGES.md
pub const NKI_VERSION: u32 = 2;

/// Minimum supported NKI version (v1alpha1).
/// Requests with nki_version < MIN_NKI_VERSION are rejected.
pub const MIN_NKI_VERSION: u32 = 1;

/// Standard NKI request ID.
pub fn new_request_id() -> String {
    Uuid::now_v7().to_string()
}

/// Standard NKI idempotency key.
pub fn new_idempotency_key() -> String {
    Uuid::now_v7().to_string()
}
