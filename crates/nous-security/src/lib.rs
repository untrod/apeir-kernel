//! nous-security - Authentication, Capability Manifest, default-deny enforcement.
//!
//! The security kernel enforces:
//! 1. Authentication - every request must have a valid Principal
//! 2. Capability verification - explicit grants required for side effects
//! 3. Default-deny - all 9 conditions must be met before execution
//! 4. Process isolation - third-party code never linked into nousd

pub mod credential;

pub use credential::{CredentialBroker, CredentialLease, CredentialReference};

use nous_types::principal::{CapabilityGrant, Principal};
use nous_types::workload::DataClassification;
use nous_types::{
    error::{ErrorCode, NousError},
    KernelObject,
};
use std::collections::HashMap;
use std::sync::RwLock;

// -- Security Gate --

/// The 9 conditions that must ALL be met before executing a workload with side effects.
#[derive(Debug)]
pub struct ExecutionGate {
    /// Is the principal authenticated?
    pub principal_authenticated: bool,
    /// Does the principal have the required capability grant?
    pub capability_granted: bool,
    /// Has the WorkloadSpec been validated?
    pub workload_validated: bool,
    /// Is there an active ResourceLease?
    pub lease_active: bool,
    /// Is the deadline still in the future?
    pub deadline_valid: bool,
    /// Is there a cancellation policy?
    pub cancellation_policy_set: bool,
    /// Has governance approved (if required)?
    pub governance_approved: bool,
    /// Is an isolation profile configured?
    pub isolation_configured: bool,
    /// Is there an audit ID for this execution?
    pub audit_id_set: bool,

    /// If any condition is false, the reason why.
    pub failure_reasons: Vec<String>,
}

impl ExecutionGate {
    /// Create a new gate - all conditions are initially false (denied).
    pub fn new() -> Self {
        Self {
            principal_authenticated: false,
            capability_granted: false,
            workload_validated: false,
            lease_active: false,
            deadline_valid: false,
            cancellation_policy_set: false,
            governance_approved: false,
            isolation_configured: false,
            audit_id_set: false,
            failure_reasons: Vec::new(),
        }
    }

    /// Check if all gates are open.
    pub fn is_open(&self) -> bool {
        self.principal_authenticated
            && self.capability_granted
            && self.workload_validated
            && self.lease_active
            && self.deadline_valid
            && self.cancellation_policy_set
            && self.governance_approved
            && self.isolation_configured
            && self.audit_id_set
    }

    /// Check with side-effect requirements.
    ///
    /// For workloads without side effects, fewer gates are required.
    pub fn is_open_for(&self, has_side_effects: bool) -> bool {
        if has_side_effects {
            self.is_open()
        } else {
            // Non-side-effect workloads only need auth + validation + lease
            self.principal_authenticated && self.workload_validated && self.lease_active
        }
    }

    /// Human-readable summary of gate state.
    pub fn summary(&self) -> String {
        let gates = [
            ("Principal", self.principal_authenticated),
            ("Capability", self.capability_granted),
            ("Workload", self.workload_validated),
            ("Lease", self.lease_active),
            ("Deadline", self.deadline_valid),
            ("Cancellation", self.cancellation_policy_set),
            ("Governance", self.governance_approved),
            ("Isolation", self.isolation_configured),
            ("Audit", self.audit_id_set),
        ];

        let status: Vec<String> = gates
            .iter()
            .map(|(name, open)| format!("{}: {}", name, if *open { "pass" } else { "fail" }))
            .collect();
        status.join(" | ")
    }
}

impl Default for ExecutionGate {
    fn default() -> Self {
        Self::new()
    }
}

// -- Capability Manifest --

/// A capability that a workload wants to exercise.
///
/// Every capability with side effects must be explicitly declared.
/// The kernel verifies the principal has a matching CapabilityGrant.
#[derive(Debug, Clone, Default)]
pub struct CapabilityManifest {
    /// What this capability can read (files, databases, APIs).
    pub reads: Vec<String>,
    /// What this capability can write (files, databases, APIs).
    pub writes: Vec<String>,
    /// What devices this capability needs.
    pub devices: Vec<String>,
    /// What network destinations this capability may contact.
    pub network_destinations: Vec<String>,
    /// What credentials this capability requires.
    pub credentials: Vec<String>,
    /// What side effects this capability produces.
    pub side_effects: Vec<String>,
    /// Whether these side effects are reversible.
    pub reversible: bool,
    /// Data classification of information handled.
    pub data_classification: DataClassification,
    /// Whether human approval is required.
    pub requires_approval: bool,
    /// Resource limits for this capability.
    pub resource_limits: Option<nous_types::resource::ResourceLimits>,
}

impl CapabilityManifest {
    /// Validate this manifest against a set of grants.
    pub fn validate(&self, grants: &[CapabilityGrant]) -> Result<(), Vec<String>> {
        let mut issues = Vec::new();

        // For each read path, check if any grant allows it
        for read in &self.reads {
            let allowed = grants
                .iter()
                .any(|g| g.capability == "read" && g.paths.iter().any(|p| read.starts_with(p)));
            if !allowed {
                issues.push(format!("Read access not granted for: {}", read));
            }
        }

        // For each write path
        for write in &self.writes {
            let allowed = grants
                .iter()
                .any(|g| g.capability == "write" && g.paths.iter().any(|p| write.starts_with(p)));
            if !allowed {
                issues.push(format!("Write access not granted for: {}", write));
            }
        }

        // For each network destination
        for host in &self.network_destinations {
            let allowed = grants.iter().any(|g| {
                g.capability == "network"
                    && g.hosts
                        .iter()
                        .any(|h| host.contains(h.as_str()) || h.contains(host.as_str()))
            });
            if !allowed {
                issues.push(format!("Network access not granted for: {}", host));
            }
        }

        // Side effects require explicit grant
        if !self.side_effects.is_empty() {
            let has_side_effect_grant = grants.iter().any(|g| g.capability == "side_effect");
            if !has_side_effect_grant {
                issues.push("Side effects not explicitly granted".to_string());
            }
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}

// -- Security Policy Engine --

/// The security policy engine enforces default-deny for all side-effect operations.
pub struct SecurityEngine {
    /// Registered principals.
    principals: RwLock<HashMap<String, Principal>>,
    /// Active capability grants.
    grants: RwLock<HashMap<String, Vec<CapabilityGrant>>>,
}

impl SecurityEngine {
    pub fn new() -> Self {
        Self {
            principals: RwLock::new(HashMap::new()),
            grants: RwLock::new(HashMap::new()),
        }
    }

    /// Register a principal.
    pub fn register_principal(&self, principal: Principal) -> Result<(), NousError> {
        self.principals
            .write()
            .map_err(|_| NousError::new(ErrorCode::Internal, "principal state lock is poisoned"))?
            .insert(principal.uid().to_string(), principal);
        Ok(())
    }

    /// Grant a capability to a principal.
    pub fn grant(&self, grant: CapabilityGrant) -> Result<(), NousError> {
        self.grants
            .write()
            .map_err(|_| NousError::new(ErrorCode::Internal, "grant state lock is poisoned"))?
            .entry(grant.principal_id.clone())
            .or_default()
            .push(grant);
        Ok(())
    }

    /// Check if a principal has a specific capability.
    pub fn check_capability(&self, principal_id: &str, capability: &str) -> bool {
        self.grants
            .read()
            .map(|grants| {
                grants.get(principal_id).is_some_and(|grants| {
                    grants
                        .iter()
                        .any(|g| g.capability == capability && g.is_valid())
                })
            })
            .unwrap_or(false)
    }

    /// Get all valid grants for a principal.
    pub fn get_grants(&self, principal_id: &str) -> Vec<CapabilityGrant> {
        self.grants
            .read()
            .map(|state| {
                state
                    .get(principal_id)
                    .map(|grants| grants.iter().filter(|g| g.is_valid()).cloned().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    /// Build an ExecutionGate for a workload, filling in all conditions.
    pub fn evaluate(
        &self,
        principal_id: &str,
        manifest: &CapabilityManifest,
        has_lease: bool,
        deadline_us: i64,
        has_cancellation_policy: bool,
        isolation_profile: &str,
    ) -> ExecutionGate {
        let mut gate = ExecutionGate::new();

        // 1. Principal authenticated?
        gate.principal_authenticated = self
            .principals
            .read()
            .map(|principals| principals.contains_key(principal_id))
            .unwrap_or(false);

        // 2. Capability granted?
        let grants = self.get_grants(principal_id);
        gate.capability_granted = manifest.validate(&grants).is_ok();

        // 3. Workload validated? (Set by caller after spec validation)
        gate.workload_validated = false; // Caller must set

        // 4. Lease active?
        gate.lease_active = has_lease;

        // 5. Deadline valid?
        gate.deadline_valid =
            deadline_us == 0 || deadline_us > chrono::Utc::now().timestamp_micros();

        // 6. Cancellation policy set?
        gate.cancellation_policy_set = has_cancellation_policy;

        // 7. Governance approved?
        gate.governance_approved = !manifest.requires_approval; // If no approval needed, auto-approved

        // 8. Isolation configured?
        gate.isolation_configured = !isolation_profile.is_empty();

        // 9. Audit ID set?
        gate.audit_id_set = true; // Generated by kernel

        if !gate.principal_authenticated {
            gate.failure_reasons
                .push("Principal not authenticated".into());
        }
        if !gate.capability_granted {
            gate.failure_reasons.push("Capability not granted".into());
        }
        if !gate.lease_active {
            gate.failure_reasons.push("No active resource lease".into());
        }
        if !gate.deadline_valid {
            gate.failure_reasons.push("Deadline has passed".into());
        }
        if !gate.isolation_configured {
            gate.failure_reasons
                .push("No isolation profile configured".into());
        }

        gate
    }
}

impl Default for SecurityEngine {
    fn default() -> Self {
        Self::new()
    }
}

// -- Isolation Profiles --

/// Platform-specific process isolation configuration.
#[derive(Debug, Clone)]
pub struct IsolationProfile {
    pub name: String,
    pub linux_config: Option<LinuxIsolation>,
    pub windows_config: Option<WindowsIsolation>,
}

/// Linux isolation using cgroups, namespaces, seccomp.
#[derive(Debug, Clone)]
pub struct LinuxIsolation {
    pub cgroup_profile: String,
    pub namespaces: Vec<String>,
    pub seccomp_profile: String,
    pub readonly_rootfs: bool,
    pub min_uid: u32,
    pub network_egress_filter: Vec<String>,
}

/// Windows isolation using Job Objects, AppContainer.
#[derive(Debug, Clone)]
pub struct WindowsIsolation {
    pub job_object_name: String,
    pub app_container_sid: String,
    pub restricted_token: bool,
    pub named_pipe_acl: Vec<String>,
    pub filesystem_acl: Vec<String>,
    pub firewall_rules: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_deny_all_closed() {
        let gate = ExecutionGate::new();
        assert!(!gate.is_open());
        assert!(gate.failure_reasons.is_empty()); // No failures yet, just not evaluated
    }

    #[test]
    fn test_capability_manifest_validation() {
        let manifest = CapabilityManifest {
            reads: vec!["/data/".into()],
            writes: vec!["/output/".into()],
            network_destinations: vec!["api.example.com".into()],
            side_effects: vec!["file_write".into()],
            ..Default::default()
        };

        let grants = vec![
            CapabilityGrant {
                grant_id: "g1".into(),
                principal_id: "u1".into(),
                capability: "read".into(),
                paths: vec!["/data/".into()],
                ..Default::default()
            },
            CapabilityGrant {
                grant_id: "g2".into(),
                principal_id: "u1".into(),
                capability: "write".into(),
                paths: vec!["/output/".into()],
                ..Default::default()
            },
            CapabilityGrant {
                grant_id: "g3".into(),
                principal_id: "u1".into(),
                capability: "network".into(),
                hosts: vec!["api.example.com".into()],
                ..Default::default()
            },
        ];

        let result = manifest.validate(&grants);
        // Missing side_effect grant
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("Side effects")));
    }

    #[test]
    fn test_security_engine_grant_check() {
        let engine = SecurityEngine::new();
        engine
            .grant(CapabilityGrant {
                grant_id: "g1".into(),
                principal_id: "u1".into(),
                capability: "shell.execute".into(),
                ..Default::default()
            })
            .unwrap();

        assert!(engine.check_capability("u1", "shell.execute"));
        assert!(!engine.check_capability("u1", "file.delete"));
        assert!(!engine.check_capability("u2", "shell.execute"));
    }
}
