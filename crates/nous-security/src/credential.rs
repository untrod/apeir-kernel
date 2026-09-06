//! Credential broker with reference-only storage and scoped, short-lived leases.

use nous_types::error::{ErrorCode, NousError};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Clone)]
pub struct CredentialReference {
    pub credential_id: String,
    pub environment_variable: String,
    pub allowed_principals: Vec<String>,
    pub allowed_capabilities: Vec<String>,
}

/// Secret material is confined to a lease and erased when the lease is dropped.
pub struct CredentialLease {
    pub lease_id: String,
    pub credential_id: String,
    pub principal_id: String,
    pub capability: String,
    pub expires_at_us: i64,
    secret: Vec<u8>,
}

impl CredentialLease {
    pub fn with_secret<T>(&self, operation: impl FnOnce(&[u8]) -> T) -> Result<T, NousError> {
        if chrono::Utc::now().timestamp_micros() >= self.expires_at_us {
            return Err(NousError::new(
                ErrorCode::Unauthenticated,
                "Credential lease expired",
            ));
        }
        Ok(operation(&self.secret))
    }
}

impl Drop for CredentialLease {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

#[derive(Default)]
pub struct CredentialBroker {
    references: RwLock<HashMap<String, CredentialReference>>,
}

impl CredentialBroker {
    pub fn register(&self, reference: CredentialReference) -> Result<(), NousError> {
        if reference.environment_variable.trim().is_empty() {
            return Err(NousError::new(
                ErrorCode::InvalidRequest,
                "Credential reference requires an environment variable",
            ));
        }
        self.references
            .write()
            .map_err(|_| {
                NousError::new(ErrorCode::Internal, "credential reference lock is poisoned")
            })?
            .insert(reference.credential_id.clone(), reference);
        Ok(())
    }

    pub fn lease(
        &self,
        credential_id: &str,
        principal_id: &str,
        capability: &str,
        ttl_seconds: u32,
    ) -> Result<CredentialLease, NousError> {
        let references = self.references.read().map_err(|_| {
            NousError::new(ErrorCode::Internal, "credential reference lock is poisoned")
        })?;
        let reference = references
            .get(credential_id)
            .ok_or_else(|| NousError::new(ErrorCode::NotFound, "Credential reference not found"))?;
        if !reference
            .allowed_principals
            .iter()
            .any(|allowed| allowed == principal_id)
        {
            return Err(NousError::new(
                ErrorCode::PermissionDenied,
                "Principal is not allowed to lease this credential",
            ));
        }
        if !reference
            .allowed_capabilities
            .iter()
            .any(|allowed| allowed == capability)
        {
            return Err(NousError::new(
                ErrorCode::CapabilityDenied,
                "Credential is not valid for the requested capability",
            ));
        }
        let secret = std::env::var_os(&reference.environment_variable)
            .ok_or_else(|| {
                NousError::new(
                    ErrorCode::Unauthenticated,
                    "Credential source is unavailable",
                )
            })?
            .to_string_lossy()
            .as_bytes()
            .to_vec();
        Ok(CredentialLease {
            lease_id: uuid::Uuid::now_v7().to_string(),
            credential_id: credential_id.into(),
            principal_id: principal_id.into(),
            capability: capability.into(),
            expires_at_us: chrono::Utc::now().timestamp_micros()
                + i64::from(ttl_seconds.max(1)) * 1_000_000,
            secret,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_enforces_scope_without_persisting_values() {
        let variable = format!("NOUS_TEST_CREDENTIAL_{}", std::process::id());
        std::env::set_var(&variable, "test-only-secret");
        let broker = CredentialBroker::default();
        broker
            .register(CredentialReference {
                credential_id: "model-provider".into(),
                environment_variable: variable.clone(),
                allowed_principals: vec!["runtime".into()],
                allowed_capabilities: vec!["model.infer".into()],
            })
            .unwrap();
        let lease = broker
            .lease("model-provider", "runtime", "model.infer", 1)
            .unwrap();
        assert_eq!(lease.with_secret(|secret| secret.len()).unwrap(), 16);
        assert!(broker
            .lease("model-provider", "other", "model.infer", 1)
            .is_err());
        std::env::remove_var(variable);
    }
}
