//! Stable error codes for the APEIR AI Kernel.
//!
//! Every error in the kernel maps to one of these codes.
//! All errors carry a retry hint so clients know whether (and how) to retry.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Kernel error - wraps an error code with context.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct NousError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub failed_phase: String,
    #[serde(default)]
    pub cause: String,
    pub retry: RetryHint,
}

impl NousError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            failed_phase: String::new(),
            cause: String::new(),
            retry: RetryHint::for_code(code),
        }
    }

    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.failed_phase = phase.into();
        self
    }

    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.cause = cause.into();
        self
    }

    pub fn not_implemented(feature: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::NotImplemented,
            format!("NOT_IMPLEMENTED: {}", feature.into()),
        )
    }

    pub fn not_supported(feature: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::NotSupported,
            format!("NOT_SUPPORTED: {}", feature.into()),
        )
    }

    pub fn backend_unavailable(backend: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::BackendUnavailable,
            format!("BACKEND_UNAVAILABLE: {}", backend.into()),
        )
    }

    pub fn incompatible(detail: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::BackendIncompatible,
            format!("INCOMPATIBLE: {}", detail.into()),
        )
    }
}

/// Stable error codes for all kernel operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    // -- Generic (matching gRPC codes) --
    Unknown = 0,
    InvalidRequest = 1,
    NotFound = 2,
    AlreadyExists = 3,
    PermissionDenied = 4,
    Unauthenticated = 5,
    ResourceExhausted = 6,
    FailedPrecondition = 7,
    Aborted = 8,
    OutOfRange = 9,
    NotImplemented = 10,
    Internal = 11,
    Unavailable = 12,
    DataLoss = 13,
    DeadlineExceeded = 14,

    // -- Workload --
    WorkloadRejected = 20,
    WorkloadCancelled = 21,
    WorkloadLost = 22,
    WorkloadQuarantined = 23,
    WorkloadConflict = 24,

    // -- Resource --
    ResourceInsufficient = 30,
    LeaseExpired = 31,
    LeaseConflict = 32,

    // -- Model --
    ModelNotFound = 40,
    ModelIncompatible = 41,
    ModelNotLoaded = 42,
    ModelValidationFailed = 43,
    ModelSecurityBlocked = 44,

    // -- Engine --
    EngineNotFound = 50,
    EngineIncompatible = 51,
    EngineUnhealthy = 52,
    EngineTimeout = 53,

    // -- Device --
    DeviceNotFound = 60,
    DeviceIncompatible = 61,
    DeviceUnhealthy = 62,
    DeviceOutOfMemory = 63,

    // -- Backend --
    BackendUnavailable = 70,
    BackendIncompatible = 71,

    // -- Capability --
    NotSupported = 80,

    // -- Security --
    SecurityPolicy = 90,
    CapabilityDenied = 91,
    IsolationFailed = 92,
}

impl ErrorCode {
    /// Is this error retryable?
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::Unavailable
                | ErrorCode::DeadlineExceeded
                | ErrorCode::ResourceExhausted
                | ErrorCode::Aborted
                | ErrorCode::LeaseExpired
                | ErrorCode::EngineUnhealthy
                | ErrorCode::EngineTimeout
                | ErrorCode::DeviceUnhealthy
                | ErrorCode::BackendUnavailable
        )
    }

    /// Is this a terminal error (retry won't help)?
    pub fn is_terminal(self) -> bool {
        !self.is_retryable()
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

/// Hint to the client about whether and how to retry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RetryHint {
    pub retryable: bool,
    pub recommended_delay_ms: u32,
    pub max_retries: u32,
    pub strategy: RetryStrategy,
}

impl RetryHint {
    pub fn for_code(code: ErrorCode) -> Self {
        if code.is_retryable() {
            Self {
                retryable: true,
                recommended_delay_ms: 100,
                max_retries: 3,
                strategy: RetryStrategy::Backoff,
            }
        } else {
            Self {
                retryable: false,
                recommended_delay_ms: 0,
                max_retries: 0,
                strategy: RetryStrategy::Never,
            }
        }
    }

    pub fn never() -> Self {
        Self {
            retryable: false,
            recommended_delay_ms: 0,
            max_retries: 0,
            strategy: RetryStrategy::Never,
        }
    }
}

/// Strategy for retrying a failed request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryStrategy {
    /// Safe to retry immediately.
    Immediate = 0,
    /// Use exponential backoff.
    Backoff = 1,
    /// Wait for server-specified duration.
    After = 2,
    /// Use a new idempotency key.
    NewIdempotency = 3,
    /// Do not retry; request was processed.
    Never = 4,
}
