//! Workload admission control.
//!
//! The admission controller decides whether a workload can be accepted
//! based on available resources, security constraints, and policy.

use nous_types::resource::{ResourceDomain, ResourceVector};
use nous_types::workload::{SecurityRequirements, WorkloadSpec};

/// The result of an admission decision.
#[derive(Debug)]
pub enum AdmissionDecision {
    /// Workload is admitted.
    Admitted,
    /// Workload is rejected with a reason.
    Rejected {
        reason: String,
        missing_requirements: Vec<String>,
    },
    /// Workload requires more resources than the system can ever provide.
    Infeasible {
        reason: String,
        required: ResourceVector,
        capacity: ResourceVector,
    },
}

/// An admission controller checks workloads against resource and security constraints.
pub struct AdmissionController {
    /// The system resource domain (root of hierarchy).
    system_domain: ResourceDomain,
}

impl AdmissionController {
    pub fn new(system_domain: ResourceDomain) -> Self {
        Self { system_domain }
    }

    /// Check if a workload can be admitted.
    ///
    /// Checks in order:
    /// 0. Deadline - has the deadline already passed?
    /// 1. Feasibility - can the system ever satisfy this?
    /// 2. Resource availability - is there enough free capacity?
    /// 3. Security - does the workload meet security requirements?
    pub fn admit(&self, spec: &WorkloadSpec) -> AdmissionDecision {
        let required = &spec.resource_requirements.minimum;

        // 0. Deadline check - reject workloads with past deadlines
        if spec.deadline_us > 0 {
            let now_us = chrono::Utc::now().timestamp_micros();
            if spec.deadline_us < now_us {
                return AdmissionDecision::Rejected {
                    reason: format!(
                        "Deadline expired: deadline={}us, now={}us ({}us past)",
                        spec.deadline_us,
                        now_us,
                        now_us - spec.deadline_us
                    ),
                    missing_requirements: vec!["valid_deadline".to_string()],
                };
            }
        }

        // 1. Feasibility check
        if !required.fits_within(&self.system_domain.capacity) {
            return AdmissionDecision::Infeasible {
                reason: "Required resources exceed system capacity".to_string(),
                required: *required,
                capacity: self.system_domain.capacity,
            };
        }

        // 2. Resource availability
        if !self.system_domain.can_fit(required) {
            let available = self.system_domain.available();
            let mut missing = Vec::new();

            if required.ram_bytes > available.ram_bytes {
                missing.push(format!(
                    "RAM: need {} bytes, have {} bytes",
                    required.ram_bytes, available.ram_bytes
                ));
            }
            if required.device_memory_bytes > available.device_memory_bytes {
                missing.push(format!(
                    "VRAM: need {} bytes, have {} bytes",
                    required.device_memory_bytes, available.device_memory_bytes
                ));
            }
            if required.kv_cache_bytes > available.kv_cache_bytes {
                missing.push(format!(
                    "KV Cache: need {} bytes, have {} bytes",
                    required.kv_cache_bytes, available.kv_cache_bytes
                ));
            }

            return AdmissionDecision::Rejected {
                reason: "Insufficient resources".to_string(),
                missing_requirements: missing,
            };
        }

        // 3. Security check (basic)
        if let Err(missing) = Self::validate_security(&spec.security_requirements) {
            return AdmissionDecision::Rejected {
                reason: "Security requirements not met".to_string(),
                missing_requirements: missing,
            };
        }

        AdmissionDecision::Admitted
    }

    /// Validate security requirements.
    fn validate_security(sec: &SecurityRequirements) -> Result<(), Vec<String>> {
        let mut issues = Vec::new();

        // If the workload has side effects, it must be explicitly allowed
        if sec.allow_side_effects && sec.isolation_profile.is_empty() {
            // OK - explicitly allowed
        }

        // If network access is requested, allowed hosts should be specified
        if sec.allow_network_access && sec.allowed_hosts.is_empty() {
            issues.push("Network access allowed but no allowed_hosts specified".to_string());
        }

        // If file access is requested, allowed paths should be specified
        if sec.allow_file_access && sec.allowed_paths.is_empty() {
            issues.push("File access allowed but no allowed_paths specified".to_string());
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_system_domain() -> ResourceDomain {
        ResourceDomain {
            domain_id: "system".to_string(),
            parent_domain_id: None,
            capacity: ResourceVector {
                ram_bytes: 16 * 1024 * 1024 * 1024,          // 16 GB
                device_memory_bytes: 8 * 1024 * 1024 * 1024, // 8 GB
                kv_cache_bytes: 4 * 1024 * 1024 * 1024,      // 4 GB
                cpu_cores_millis: 8000,                      // 8 cores
                ..ResourceVector::default()
            },
            allocated: ResourceVector::default(),
            reserved: ResourceVector::default(),
            hard_limits: Default::default(),
            soft_limits: Default::default(),
        }
    }

    #[test]
    fn test_admit_feasible_workload() {
        let controller = AdmissionController::new(make_system_domain());
        let spec = WorkloadSpec {
            resource_requirements: nous_types::workload::ResourceRequirements {
                minimum: ResourceVector {
                    ram_bytes: 4 * 1024 * 1024 * 1024,
                    device_memory_bytes: 2 * 1024 * 1024 * 1024,
                    ..ResourceVector::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        match controller.admit(&spec) {
            AdmissionDecision::Admitted => {} // expected
            other => panic!("Expected Admitted, got {:?}", other),
        }
    }

    #[test]
    fn test_reject_infeasible_workload() {
        let controller = AdmissionController::new(make_system_domain());
        let spec = WorkloadSpec {
            resource_requirements: nous_types::workload::ResourceRequirements {
                minimum: ResourceVector {
                    ram_bytes: 128 * 1024 * 1024 * 1024, // 128 GB - more than system has
                    ..ResourceVector::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        match controller.admit(&spec) {
            AdmissionDecision::Infeasible { .. } => {} // expected
            other => panic!("Expected Infeasible, got {:?}", other),
        }
    }
}
