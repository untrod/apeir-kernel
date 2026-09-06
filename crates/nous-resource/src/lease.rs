//! Lease manager - grants, renews, and expires resource leases.
//!
//! Every executing workload must hold a valid ResourceLease.
//! Leases have a finite duration and must be renewed for long-running workloads.
//! Expired leases cause the workload to be paused or cancelled.

use nous_state::fencing::FencingToken;
use nous_types::error::{ErrorCode, NousError};
use nous_types::resource::{ResourceLease, ResourceVector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

fn lock_or_error<'a, T>(mutex: &'a Mutex<T>, name: &str) -> Result<MutexGuard<'a, T>, NousError> {
    mutex
        .lock()
        .map_err(|_| NousError::new(ErrorCode::Internal, format!("{name} lock is poisoned")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FencedLease {
    pub lease: ResourceLease,
    pub fencing_token: String,
    pub resource_key: String,
}

/// Manages resource leases for active workloads.
pub struct LeaseManager {
    /// Active leases: lease_id -> ResourceLease
    leases: Mutex<HashMap<String, ResourceLease>>,

    lease_tokens: Mutex<HashMap<String, String>>,

    resource_tokens: Mutex<HashMap<String, String>>,

    fencing: FencingToken,

    /// Default lease duration in seconds.
    default_duration: chrono::Duration,
}

impl LeaseManager {
    pub fn new(default_duration_secs: i64) -> Self {
        Self::new_with_duration(chrono::Duration::seconds(default_duration_secs))
    }

    pub fn new_with_duration(default_duration: chrono::Duration) -> Self {
        Self {
            leases: Mutex::new(HashMap::new()),
            lease_tokens: Mutex::new(HashMap::new()),
            resource_tokens: Mutex::new(HashMap::new()),
            fencing: FencingToken::new(format!("lease-{}", uuid::Uuid::now_v7())),
            default_duration,
        }
    }

    pub fn grant_fenced(
        &self,
        workload_id: &str,
        principal_id: &str,
        resources: ResourceVector,
        node_id: &str,
        device_id: &str,
        renewable: bool,
    ) -> Result<FencedLease, NousError> {
        let lease = self.grant_internal(
            workload_id,
            principal_id,
            resources,
            node_id,
            device_id,
            renewable,
        )?;
        let resource_key = Self::resource_key(node_id, device_id);
        let fencing_token = self.fencing.next();
        lock_or_error(&self.lease_tokens, "lease token")?
            .insert(lease.lease_id.clone(), fencing_token.clone());
        lock_or_error(&self.resource_tokens, "resource token")?
            .insert(resource_key.clone(), fencing_token.clone());
        Ok(FencedLease {
            lease,
            fencing_token,
            resource_key,
        })
    }

    fn grant_internal(
        &self,
        workload_id: &str,
        principal_id: &str,
        resources: ResourceVector,
        node_id: &str,
        device_id: &str,
        renewable: bool,
    ) -> Result<ResourceLease, NousError> {
        // Check for existing valid lease - prevent double-booking
        {
            let leases = lock_or_error(&self.leases, "lease")?;
            if leases
                .values()
                .any(|l| l.workload_id == workload_id && l.is_valid())
            {
                return Err(NousError::new(
                    ErrorCode::AlreadyExists,
                    format!("Workload '{}' already has an active lease", workload_id),
                ));
            }
        }

        let now = chrono::Utc::now();
        let expires = now + self.default_duration;
        let lease_id = uuid::Uuid::now_v7().to_string();

        let lease = ResourceLease {
            lease_id: lease_id.clone(),
            workload_id: workload_id.to_string(),
            principal_id: principal_id.to_string(),
            reserved: resources,
            node_id: node_id.to_string(),
            device_id: device_id.to_string(),
            granted_at: now,
            expires_at: expires,
            generation: 1,
            renewable,
        };

        let mut leases = lock_or_error(&self.leases, "lease")?;

        // Double-check after acquiring write lock (no race with concurrent grant)
        if leases
            .values()
            .any(|l| l.workload_id == workload_id && l.is_valid())
        {
            return Err(NousError::new(
                ErrorCode::AlreadyExists,
                format!("Workload '{}' already has an active lease", workload_id),
            ));
        }

        leases.insert(lease_id, lease.clone());

        tracing::info!(
            lease_id = %lease.lease_id,
            workload_id = %lease.workload_id,
            expires_at = %lease.expires_at,
            "Lease granted"
        );

        Ok(lease)
    }

    /// Renew an existing lease (extend its expiry).
    pub fn renew(
        &self,
        lease_id: &str,
        expected_generation: u64,
    ) -> Result<ResourceLease, NousError> {
        let mut leases = lock_or_error(&self.leases, "lease")?;
        let lease = leases.get_mut(lease_id).ok_or_else(|| {
            NousError::new(
                ErrorCode::NotFound,
                format!("Lease not found: {}", lease_id),
            )
        })?;

        if lease.generation != expected_generation {
            return Err(NousError::new(
                ErrorCode::LeaseConflict,
                format!(
                    "Lease generation conflict: expected {}, current {}",
                    expected_generation, lease.generation
                ),
            ));
        }

        if !lease.is_valid() {
            return Err(NousError::new(
                ErrorCode::LeaseExpired,
                "Cannot renew expired lease".to_string(),
            ));
        }

        if !lease.renewable {
            return Err(NousError::new(
                ErrorCode::FailedPrecondition,
                "Lease is not renewable".to_string(),
            ));
        }

        let now = chrono::Utc::now();
        lease.expires_at = now + self.default_duration;
        lease.generation += 1;

        tracing::info!(
            lease_id = %lease_id,
            new_generation = lease.generation,
            new_expiry = %lease.expires_at,
            "Lease renewed"
        );

        Ok(lease.clone())
    }

    /// Release a lease (free the reserved resources).
    pub fn release(&self, lease_id: &str) -> Result<(), NousError> {
        let mut leases = lock_or_error(&self.leases, "lease")?;
        if leases.remove(lease_id).is_some() {
            lock_or_error(&self.lease_tokens, "lease token")?.remove(lease_id);
            tracing::info!(lease_id = %lease_id, "Lease released");
            Ok(())
        } else {
            Err(NousError::new(
                ErrorCode::NotFound,
                format!("Lease not found: {}", lease_id),
            ))
        }
    }

    pub fn revoke(&self, lease_id: &str) -> Result<(), NousError> {
        self.release(lease_id)
    }

    pub fn authorize(&self, lease_id: &str, provided_token: &str) -> bool {
        let Ok(leases) = self.leases.lock() else {
            return false;
        };
        let Some(lease) = leases.get(lease_id) else {
            return false;
        };
        if !lease.is_valid() {
            return false;
        }
        let Ok(lease_tokens) = self.lease_tokens.lock() else {
            return false;
        };
        let Some(assigned_token) = lease_tokens.get(lease_id) else {
            return false;
        };
        let Ok(resource_tokens) = self.resource_tokens.lock() else {
            return false;
        };
        let resource_key = Self::resource_key(&lease.node_id, &lease.device_id);
        resource_tokens.get(&resource_key).is_some_and(|current| {
            assigned_token == provided_token
                && FencingToken::is_token_valid(current, provided_token)
        })
    }

    /// Get a lease by ID.
    pub fn get(&self, lease_id: &str) -> Option<ResourceLease> {
        let leases = self.leases.lock().ok()?;
        leases.get(lease_id).cloned()
    }

    /// Get the lease for a workload.
    pub fn get_for_workload(&self, workload_id: &str) -> Option<ResourceLease> {
        let leases = self.leases.lock().ok()?;
        leases
            .values()
            .find(|l| l.workload_id == workload_id)
            .cloned()
    }

    /// Check if a workload has a valid lease.
    pub fn has_valid_lease(&self, workload_id: &str) -> bool {
        self.get_for_workload(workload_id)
            .map(|l| l.is_valid())
            .unwrap_or(false)
    }

    /// Expire all leases past their expiry time.
    /// Returns the list of workload IDs whose leases expired.
    pub fn expire_stale(&self) -> Vec<String> {
        self.try_expire_stale().unwrap_or_default()
    }

    /// Fallible form of [`Self::expire_stale`] for runtime control paths.
    pub fn try_expire_stale(&self) -> Result<Vec<String>, NousError> {
        let mut leases = lock_or_error(&self.leases, "lease")?;
        let mut expired = Vec::new();
        let mut expired_lease_ids = Vec::new();

        leases.retain(|lease_id, lease| {
            if lease.is_expired() {
                tracing::warn!(
                    lease_id = %lease_id,
                    workload_id = %lease.workload_id,
                    "Lease expired"
                );
                expired.push(lease.workload_id.clone());
                expired_lease_ids.push(lease_id.clone());
                false
            } else {
                true
            }
        });

        drop(leases);
        let mut tokens = lock_or_error(&self.lease_tokens, "lease token")?;
        for lease_id in expired_lease_ids {
            tokens.remove(&lease_id);
        }

        Ok(expired)
    }

    /// Count active (non-expired) leases.
    pub fn active_count(&self) -> usize {
        self.try_active_count().unwrap_or_default()
    }

    /// Fallible form of [`Self::active_count`] for runtime control paths.
    pub fn try_active_count(&self) -> Result<usize, NousError> {
        let leases = lock_or_error(&self.leases, "lease")?;
        Ok(leases.values().filter(|l| l.is_valid()).count())
    }

    fn resource_key(node_id: &str, device_id: &str) -> String {
        format!("{node_id}:{device_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grant_and_get() {
        let mgr = LeaseManager::new(60);
        let lease = mgr
            .grant_fenced(
                "w-1",
                "user-1",
                ResourceVector::default(),
                "node-1",
                "",
                true,
            )
            .unwrap()
            .lease;
        assert!(lease.is_valid());
        assert_eq!(mgr.active_count(), 1);
    }

    #[test]
    fn test_release() {
        let mgr = LeaseManager::new(60);
        let lease = mgr
            .grant_fenced(
                "w-1",
                "user-1",
                ResourceVector::default(),
                "node-1",
                "",
                true,
            )
            .unwrap()
            .lease;
        mgr.release(&lease.lease_id).unwrap();
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn test_renew() {
        let mgr = LeaseManager::new(60);
        let lease = mgr
            .grant_fenced(
                "w-1",
                "user-1",
                ResourceVector::default(),
                "node-1",
                "",
                true,
            )
            .unwrap()
            .lease;
        let renewed = mgr.renew(&lease.lease_id, 1).unwrap();
        assert_eq!(renewed.generation, 2);
    }

    #[test]
    fn test_cannot_renew_expired() {
        let mgr = LeaseManager::new(-1); // Expires immediately
        let lease = mgr
            .grant_fenced(
                "w-1",
                "user-1",
                ResourceVector::default(),
                "node-1",
                "",
                true,
            )
            .unwrap()
            .lease;
        let err = mgr.renew(&lease.lease_id, 1).unwrap_err();
        assert_eq!(err.code, ErrorCode::LeaseExpired);
    }

    #[test]
    fn expired_owner_cannot_use_stale_fencing_token() {
        let manager = LeaseManager::new_with_duration(chrono::Duration::milliseconds(1));
        let first = manager
            .grant_fenced(
                "workload-a",
                "runtime",
                ResourceVector::default(),
                "node-1",
                "cpu-0",
                true,
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(manager.expire_stale(), vec!["workload-a"]);
        let second = manager
            .grant_fenced(
                "workload-b",
                "runtime",
                ResourceVector::default(),
                "node-1",
                "cpu-0",
                true,
            )
            .unwrap();

        assert!(!manager.authorize(&first.lease.lease_id, &first.fencing_token));
        assert!(manager.authorize(&second.lease.lease_id, &second.fencing_token));
        assert_ne!(first.fencing_token, second.fencing_token);
    }
}
