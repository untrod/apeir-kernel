//! nous-resource - Resource admission control and lease management.
//!
//! The admission pipeline:
//! 1. Estimate - compute the ResourceVector needed by a workload
//! 2. Feasibility Check - can the system ever satisfy this?
//! 3. Admission - is there enough available capacity?
//! 4. Reservation - hold resources for this workload
//! 5. Placement - assign to a specific node/device
//! 6. Lease - grant a time-bound lease
//! 7. Execution - workload runs under lease
//!
//! Principle: NO LEASE -> NO EXECUTION.

pub mod admission;
pub mod domain;
pub mod lease;

pub use lease::{FencedLease, LeaseManager};

use nous_types::error::NousError;
use nous_types::resource::ResourceVector;

/// Result for resource operations.
pub type ResourceResult<T> = Result<T, NousError>;

/// Estimate the resources required by a workload.
///
/// This is a simple baseline estimator. Policy plugins can provide
/// more sophisticated estimation (model-specific, history-based, etc.).
pub fn estimate_resources(
    model_params: u64,
    context_length: u64,
    max_output_tokens: u64,
    quantization_bits: u8,
) -> ResourceVector {
    let bytes_per_param = quantization_bits as u64 / 8;
    let weight_bytes = model_params * bytes_per_param;

    // Rough KV cache estimate: 2 * layers * hidden_dim * context * bytes_per_elem
    // Simplified: ~ (kv_bytes_per_token) * context_length
    let kv_bytes_per_token = if quantization_bits <= 4 { 512 } else { 1024 };
    let kv_cache_bytes = kv_bytes_per_token * context_length;

    // Output buffer
    let output_buffer = max_output_tokens * kv_bytes_per_token;

    ResourceVector {
        device_memory_bytes: weight_bytes + kv_cache_bytes + output_buffer,
        ram_bytes: weight_bytes, // At minimum, weights must fit in RAM
        kv_cache_bytes,
        ..ResourceVector::default()
    }
}
