//! Hierarchical resource domains.
//!
//! Domains form a tree: System -> Tenant -> Workspace -> Agent -> Workload.
//! Each domain has capacity, allocations, reservations, and limits.
//! Child domains are limited by their parent's available capacity.

use nous_types::resource::{ResourceDomain, ResourceVector};
use nous_types::{ErrorCode, NousError};
use std::collections::HashMap;
use std::sync::Mutex;

/// Manages the hierarchy of resource domains.
pub struct DomainHierarchy {
    domains: Mutex<HashMap<String, ResourceDomain>>,
}

impl DomainHierarchy {
    pub fn new(root_domain: ResourceDomain) -> Self {
        let mut domains = HashMap::new();
        domains.insert(root_domain.domain_id.clone(), root_domain);
        Self {
            domains: Mutex::new(domains),
        }
    }

    /// Register a new domain.
    pub fn register(&self, domain: ResourceDomain) -> Result<(), NousError> {
        let mut domains = self
            .domains
            .lock()
            .map_err(|_| NousError::new(ErrorCode::Internal, "resource domain lock is poisoned"))?;

        // Validate parent exists
        if let Some(ref parent_id) = domain.parent_domain_id {
            if !domains.contains_key(parent_id) {
                return Err(NousError::new(
                    ErrorCode::NotFound,
                    format!("Parent domain '{parent_id}' not found"),
                ));
            }
        }

        domains.insert(domain.domain_id.clone(), domain);
        Ok(())
    }

    /// Allocate resources in a domain.
    pub fn allocate(&self, domain_id: &str, request: &ResourceVector) -> Result<(), NousError> {
        let mut domains = self
            .domains
            .lock()
            .map_err(|_| NousError::new(ErrorCode::Internal, "resource domain lock is poisoned"))?;
        let domain = domains.get_mut(domain_id).ok_or_else(|| {
            NousError::new(
                ErrorCode::NotFound,
                format!("Domain '{domain_id}' not found"),
            )
        })?;

        if !domain.can_fit(request) {
            return Err(NousError::new(
                ErrorCode::ResourceInsufficient,
                format!(
                    "Insufficient resources in domain '{domain_id}': available={:?}, requested={request:?}",
                    domain.available()
                ),
            ));
        }

        domain.allocated = domain.allocated + *request;
        Ok(())
    }

    /// Free allocated resources in a domain.
    pub fn free(&self, domain_id: &str, freed: &ResourceVector) -> Result<(), NousError> {
        let mut domains = self
            .domains
            .lock()
            .map_err(|_| NousError::new(ErrorCode::Internal, "resource domain lock is poisoned"))?;
        let domain = domains.get_mut(domain_id).ok_or_else(|| {
            NousError::new(
                ErrorCode::NotFound,
                format!("Domain '{domain_id}' not found"),
            )
        })?;

        domain.allocated = domain.allocated - *freed;
        Ok(())
    }

    /// Get available resources in a domain.
    pub fn available(&self, domain_id: &str) -> Option<ResourceVector> {
        let domains = self.domains.lock().ok()?;
        domains.get(domain_id).map(|d| d.available())
    }

    /// Check if a domain is under memory pressure (>80% allocated).
    pub fn is_under_pressure(&self, domain_id: &str) -> bool {
        let Ok(domains) = self.domains.lock() else {
            return true;
        };
        if let Some(domain) = domains.get(domain_id) {
            if domain.capacity.ram_bytes > 0 {
                let used = domain.allocated.ram_bytes + domain.reserved.ram_bytes;
                used as f64 / domain.capacity.ram_bytes as f64 > 0.8
            } else {
                false
            }
        } else {
            false
        }
    }
}
