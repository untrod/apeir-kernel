//! Device - hardware device registration and lifecycle.

use crate::meta::{Condition, ObjectMeta};
use crate::resource::ResourceVector;
use crate::traits::KernelObject;
use crate::workload::DeviceType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub meta: ObjectMeta,
    pub spec: DeviceSpec,
    pub status: DeviceStatus,
}

impl KernelObject for Device {
    type Spec = DeviceSpec;
    type Status = DeviceStatus;
    fn meta(&self) -> &ObjectMeta {
        &self.meta
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.meta
    }
    fn spec(&self) -> &Self::Spec {
        &self.spec
    }
    fn status(&self) -> &Self::Status {
        &self.status
    }
    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceSpec {
    pub device_type: DeviceType,
    pub vendor: String,
    pub model: String,
    pub driver_version: String,
    pub total_resources: ResourceVector,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub phase: DevicePhase,
    pub available: ResourceVector,
    pub temperature_celsius: f64,
    /// Power consumption in milliwatts (consistent with ResourceVector).
    pub power_milliwatts: u64,
    pub utilization_percent: f64,
    pub active_engines: Vec<String>,
    pub topology: Option<TopologyLink>,
    pub last_health_check: Option<chrono::DateTime<chrono::Utc>>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DevicePhase {
    #[default]
    Unknown = 0,
    Discovered = 1,
    Probed = 2,
    Bound = 3,
    Initialized = 4,
    Ready = 5,
    Allocating = 6,
    Busy = 7,
    Draining = 8,
    Suspended = 9,
    Resetting = 10,
    Failed = 11,
    Unbound = 12,
}

/// A link between two devices in the topology graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TopologyLink {
    pub from_device_id: String,
    pub to_device_id: String,
    pub link_type: String,
    pub bandwidth_bps: u64,
    pub latency_ns: u64,
}
