//! Node - compute node in the APEIR cluster.

use crate::meta::{Condition, ObjectMeta};
use crate::resource::ResourceVector;
use crate::traits::KernelObject;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub meta: ObjectMeta,
    pub spec: NodeSpec,
    pub status: NodeStatus,
}

impl KernelObject for Node {
    type Spec = NodeSpec;
    type Status = NodeStatus;
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
pub struct NodeSpec {
    pub hostname: String,
    pub arch: String,
    pub os: String,
    pub capacity: ResourceVector,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeStatus {
    pub phase: NodePhase,
    pub allocatable: ResourceVector,
    pub devices: Vec<String>,
    pub engines: Vec<String>,
    pub active_workloads: Vec<String>,
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
    pub kernel_version: String,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NodePhase {
    #[default]
    Unknown = 0,
    Starting = 1,
    Ready = 2,
    Busy = 3,
    Draining = 4,
    Disconnected = 5,
    Failed = 6,
}
