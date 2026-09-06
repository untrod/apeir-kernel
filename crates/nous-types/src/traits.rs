//! Core trait for kernel objects - the Spec/Status pattern.
//!
//! Every kernel object implements the KernelObject trait, which enforces
//! the separation between desired state (Spec) and observed state (Status).

use crate::meta::ObjectMeta;

/// Every kernel object splits into:
/// - Spec: the user's or system's desired state (immutable after creation)
/// - Status: the kernel's observed actual state (mutable, updated by kernel)
///
/// Rules:
/// - Spec is set at creation and immutable thereafter (except via explicit update with generation check)
/// - Status is updated ONLY by the kernel, never by external clients
/// - Status updates go through the state machine and journal
pub trait KernelObject {
    /// The desired-state type for this object.
    type Spec;

    /// The observed-state type for this object.
    type Status;

    /// Object metadata (identity, versioning, health).
    fn meta(&self) -> &ObjectMeta;

    /// Mutable metadata reference (kernel-only).
    fn meta_mut(&mut self) -> &mut ObjectMeta;

    /// The desired state (immutable after creation).
    fn spec(&self) -> &Self::Spec;

    /// The observed state (kernel-updated).
    fn status(&self) -> &Self::Status;

    /// Mutable status reference (kernel-only).
    fn status_mut(&mut self) -> &mut Self::Status;

    /// Globally unique identifier.
    fn uid(&self) -> &str {
        &self.meta().uid
    }

    /// Current generation for CAS updates.
    fn generation(&self) -> u64 {
        self.meta().generation
    }

    /// Namespace this object belongs to.
    fn namespace(&self) -> &str {
        &self.meta().namespace
    }

    /// Current lifecycle phase.
    fn phase(&self) -> &str {
        &self.meta().phase
    }

    /// Update the observed status (kernel-only path).
    /// Increments the object's generation.
    fn update_status(&mut self, status: Self::Status) {
        *self.status_mut() = status;
        self.meta_mut().bump_generation();
    }
}
