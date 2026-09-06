//! Resource model - ResourceVector, ResourceLease, ResourceDomain.
//!
//! The resource model is the foundation for admission control,
//! scheduling, and fair sharing. Every workload must acquire a
//! ResourceLease before execution.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Sub};

/// A vector of resource dimensions that can be allocated, reserved, and consumed.
///
/// All fields are in their natural units (bytes, microseconds, milliwatts, etc.)
/// and use u64 for compatibility with common hardware counters.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceVector {
    /// CPU cores in millicores (1000 = 1 core).
    #[serde(default)]
    pub cpu_cores_millis: u64,

    /// CPU time budget in microseconds.
    #[serde(default)]
    pub cpu_time_us: u64,

    /// Normal RAM in bytes.
    #[serde(default)]
    pub ram_bytes: u64,

    /// Pinned (page-locked) RAM in bytes.
    #[serde(default)]
    pub pinned_ram_bytes: u64,

    /// GPU/NPU device memory (VRAM) in bytes.
    #[serde(default)]
    pub device_memory_bytes: u64,

    /// KV cache allocation in bytes.
    #[serde(default)]
    pub kv_cache_bytes: u64,

    /// Storage (disk) in bytes.
    #[serde(default)]
    pub storage_bytes: u64,

    /// Memory bandwidth in bytes per second.
    #[serde(default)]
    pub memory_bandwidth_bps: u64,

    /// Interconnect bandwidth in bytes per second.
    #[serde(default)]
    pub interconnect_bandwidth_bps: u64,

    /// Network bandwidth in bytes per second.
    #[serde(default)]
    pub network_bandwidth_bps: u64,

    /// Power consumption in milliwatts.
    #[serde(default)]
    pub power_milliwatts: u64,

    /// Thermal budget in millidegrees Celsius.
    #[serde(default)]
    pub thermal_budget_millic: u64,

    /// Wall-clock time budget in microseconds.
    #[serde(default)]
    pub time_budget_us: u64,
}

impl ResourceVector {
    /// Create a zero vector.
    pub fn zero() -> Self {
        Self::default()
    }

    /// Check if this vector fits within `capacity` in all dimensions.
    pub fn fits_within(&self, capacity: &ResourceVector) -> bool {
        self.cpu_cores_millis <= capacity.cpu_cores_millis
            && self.cpu_time_us <= capacity.cpu_time_us
            && self.ram_bytes <= capacity.ram_bytes
            && self.pinned_ram_bytes <= capacity.pinned_ram_bytes
            && self.device_memory_bytes <= capacity.device_memory_bytes
            && self.kv_cache_bytes <= capacity.kv_cache_bytes
            && self.storage_bytes <= capacity.storage_bytes
            && self.memory_bandwidth_bps <= capacity.memory_bandwidth_bps
            && self.interconnect_bandwidth_bps <= capacity.interconnect_bandwidth_bps
            && self.network_bandwidth_bps <= capacity.network_bandwidth_bps
            && self.power_milliwatts <= capacity.power_milliwatts
            && self.thermal_budget_millic <= capacity.thermal_budget_millic
            && self.time_budget_us <= capacity.time_budget_us
    }

    /// Check if any dimension is non-zero.
    pub fn is_zero(&self) -> bool {
        *self == ResourceVector::zero()
    }

    /// Create a ResourceVector representing the maximum of self and other in each dimension.
    pub fn max_per_dimension(&self, other: &ResourceVector) -> ResourceVector {
        ResourceVector {
            cpu_cores_millis: self.cpu_cores_millis.max(other.cpu_cores_millis),
            cpu_time_us: self.cpu_time_us.max(other.cpu_time_us),
            ram_bytes: self.ram_bytes.max(other.ram_bytes),
            pinned_ram_bytes: self.pinned_ram_bytes.max(other.pinned_ram_bytes),
            device_memory_bytes: self.device_memory_bytes.max(other.device_memory_bytes),
            kv_cache_bytes: self.kv_cache_bytes.max(other.kv_cache_bytes),
            storage_bytes: self.storage_bytes.max(other.storage_bytes),
            memory_bandwidth_bps: self.memory_bandwidth_bps.max(other.memory_bandwidth_bps),
            interconnect_bandwidth_bps: self
                .interconnect_bandwidth_bps
                .max(other.interconnect_bandwidth_bps),
            network_bandwidth_bps: self.network_bandwidth_bps.max(other.network_bandwidth_bps),
            power_milliwatts: self.power_milliwatts.max(other.power_milliwatts),
            thermal_budget_millic: self.thermal_budget_millic.max(other.thermal_budget_millic),
            time_budget_us: self.time_budget_us.max(other.time_budget_us),
        }
    }
}

impl Add for ResourceVector {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            cpu_cores_millis: self.cpu_cores_millis.saturating_add(rhs.cpu_cores_millis),
            cpu_time_us: self.cpu_time_us.saturating_add(rhs.cpu_time_us),
            ram_bytes: self.ram_bytes.saturating_add(rhs.ram_bytes),
            pinned_ram_bytes: self.pinned_ram_bytes.saturating_add(rhs.pinned_ram_bytes),
            device_memory_bytes: self
                .device_memory_bytes
                .saturating_add(rhs.device_memory_bytes),
            kv_cache_bytes: self.kv_cache_bytes.saturating_add(rhs.kv_cache_bytes),
            storage_bytes: self.storage_bytes.saturating_add(rhs.storage_bytes),
            memory_bandwidth_bps: self
                .memory_bandwidth_bps
                .saturating_add(rhs.memory_bandwidth_bps),
            interconnect_bandwidth_bps: self
                .interconnect_bandwidth_bps
                .saturating_add(rhs.interconnect_bandwidth_bps),
            network_bandwidth_bps: self
                .network_bandwidth_bps
                .saturating_add(rhs.network_bandwidth_bps),
            power_milliwatts: self.power_milliwatts.saturating_add(rhs.power_milliwatts),
            thermal_budget_millic: self
                .thermal_budget_millic
                .saturating_add(rhs.thermal_budget_millic),
            time_budget_us: self.time_budget_us.saturating_add(rhs.time_budget_us),
        }
    }
}

impl Sub for ResourceVector {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self {
            cpu_cores_millis: self.cpu_cores_millis.saturating_sub(rhs.cpu_cores_millis),
            cpu_time_us: self.cpu_time_us.saturating_sub(rhs.cpu_time_us),
            ram_bytes: self.ram_bytes.saturating_sub(rhs.ram_bytes),
            pinned_ram_bytes: self.pinned_ram_bytes.saturating_sub(rhs.pinned_ram_bytes),
            device_memory_bytes: self
                .device_memory_bytes
                .saturating_sub(rhs.device_memory_bytes),
            kv_cache_bytes: self.kv_cache_bytes.saturating_sub(rhs.kv_cache_bytes),
            storage_bytes: self.storage_bytes.saturating_sub(rhs.storage_bytes),
            memory_bandwidth_bps: self
                .memory_bandwidth_bps
                .saturating_sub(rhs.memory_bandwidth_bps),
            interconnect_bandwidth_bps: self
                .interconnect_bandwidth_bps
                .saturating_sub(rhs.interconnect_bandwidth_bps),
            network_bandwidth_bps: self
                .network_bandwidth_bps
                .saturating_sub(rhs.network_bandwidth_bps),
            power_milliwatts: self.power_milliwatts.saturating_sub(rhs.power_milliwatts),
            thermal_budget_millic: self
                .thermal_budget_millic
                .saturating_sub(rhs.thermal_budget_millic),
            time_budget_us: self.time_budget_us.saturating_sub(rhs.time_budget_us),
        }
    }
}

/// Resource limits for a domain or capability grant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_cpu_cores_millis: Option<u64>,
    pub max_ram_bytes: Option<u64>,
    pub max_device_memory_bytes: Option<u64>,
    pub max_kv_cache_bytes: Option<u64>,
    pub max_storage_bytes: Option<u64>,
    pub max_power_milliwatts: Option<u64>,
    pub max_concurrent_workloads: Option<u64>,
}

/// A hierarchical resource domain (System -> Tenant -> Workspace -> Agent -> Workload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDomain {
    pub domain_id: String,
    pub parent_domain_id: Option<String>,
    pub capacity: ResourceVector,
    pub allocated: ResourceVector,
    pub reserved: ResourceVector,
    pub hard_limits: ResourceLimits,
    pub soft_limits: ResourceLimits,
}

impl ResourceDomain {
    /// Check if there is enough available capacity to satisfy a request.
    pub fn can_fit(&self, request: &ResourceVector) -> bool {
        let available = self.capacity - self.allocated - self.reserved;
        request.fits_within(&available)
    }

    /// Available resources (capacity minus allocated and reserved).
    pub fn available(&self) -> ResourceVector {
        self.capacity - self.allocated - self.reserved
    }
}

/// A time-bound resource reservation for a specific workload.
///
/// Leases are the kernel's mechanism for enforcing resource isolation.
/// No workload may execute without a valid, unexpired lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLease {
    /// Unique lease identifier (UUID v7).
    pub lease_id: String,

    /// The workload this lease is for.
    pub workload_id: String,

    /// The principal who holds this lease.
    pub principal_id: String,

    /// Resources reserved by this lease.
    pub reserved: ResourceVector,

    /// Node where resources are reserved.
    #[serde(default)]
    pub node_id: String,

    /// Specific device where resources are reserved.
    #[serde(default)]
    pub device_id: String,

    /// When the lease was granted.
    pub granted_at: chrono::DateTime<chrono::Utc>,

    /// When the lease expires.
    pub expires_at: chrono::DateTime<chrono::Utc>,

    /// Lease generation (increments on renewal).
    pub generation: u64,

    /// Whether the lease can be renewed.
    pub renewable: bool,
}

impl ResourceLease {
    /// Check if the lease is currently valid (not expired).
    pub fn is_valid(&self) -> bool {
        chrono::Utc::now() < self.expires_at
    }

    /// Check if the lease has expired.
    pub fn is_expired(&self) -> bool {
        !self.is_valid()
    }

    /// Time remaining before expiry.
    pub fn remaining(&self) -> chrono::Duration {
        let now = chrono::Utc::now();
        if now < self.expires_at {
            self.expires_at - now
        } else {
            chrono::Duration::zero()
        }
    }
}

/// Priority class for workload scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum PriorityClass {
    /// Kernel operations (highest priority).
    System = 0,
    /// User-facing interactive requests.
    #[default]
    Interactive = 1,
    /// Batch/background jobs.
    Batch = 2,
    /// Best-effort (lowest priority, preemptible).
    BestEffort = 3,
}

/// Preemption policy for workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PreemptionPolicy {
    /// Never preempt this workload.
    #[default]
    Never = 0,
    /// Preempt if a higher-priority workload needs resources.
    IfLowerPriority = 1,
    /// Always allow preemption.
    Always = 2,
}
