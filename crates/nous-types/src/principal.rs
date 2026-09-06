//! Principal - authenticated identity and capability grants.

use crate::meta::ObjectMeta;
use crate::resource::ResourceLimits;
use crate::traits::KernelObject;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub meta: ObjectMeta,
    pub spec: PrincipalSpec,
    pub status: PrincipalStatus,
}

impl KernelObject for Principal {
    type Spec = PrincipalSpec;
    type Status = PrincipalStatus;
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
pub struct PrincipalSpec {
    pub principal_type: PrincipalType,
    pub display_name: String,
    pub public_key: Vec<u8>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PrincipalType {
    #[default]
    Unspecified = 0,
    User = 1,
    Service = 2,
    Node = 3,
    System = 4,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrincipalStatus {
    pub authenticated: bool,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub active_leases: Vec<String>,
}

/// A capability grant - what a principal is allowed to do.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityGrant {
    pub grant_id: String,
    pub principal_id: String,
    pub capability: String,
    pub paths: Vec<String>,
    pub hosts: Vec<String>,
    pub limits: Option<ResourceLimits>,
    pub requires_approval: bool,
    pub data_classification: String,
    pub granted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl CapabilityGrant {
    pub fn is_valid(&self) -> bool {
        if let Some(expires) = self.expires_at {
            chrono::Utc::now() < expires
        } else {
            true
        }
    }
}
