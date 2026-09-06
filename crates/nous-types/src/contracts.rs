//! Stable external contracts for the APEIR Foundation.

use serde::{Deserialize, Serialize};

/// Identifiers for the ten public contracts frozen by the Foundation profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContractId {
    /// Nous Execution Contract.
    NEC,
    /// Nous State Ownership contract.
    NSO,
    /// Nous Transactional Effects contract.
    NTE,
    /// Nous Resource Protocol.
    NRP,
    /// Nous Provider ABI.
    NPA,
    /// Nous Kernel Interface.
    NKI,
    /// Nous Micro Protocol.
    NMP,
    /// Nous Credential Contract.
    NCT,
    /// Nous Learning Contract.
    NLC,
    /// Nous Safety Envelope.
    NSE,
}

/// Machine-readable contract metadata exposed by all runtime profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractDescriptor {
    pub id: ContractId,
    pub name: &'static str,
    pub version: u16,
    pub authority: &'static str,
}

/// Foundation v1 contract registry. Additive changes retain the version;
/// breaking changes require a new contract version.
pub const FOUNDATION_CONTRACTS: [ContractDescriptor; 10] = [
    ContractDescriptor {
        id: ContractId::NEC,
        name: "Nous Execution Contract",
        version: 1,
        authority: "nousd",
    },
    ContractDescriptor {
        id: ContractId::NSO,
        name: "Nous State Ownership",
        version: 1,
        authority: "nous-state journal",
    },
    ContractDescriptor {
        id: ContractId::NTE,
        name: "Nous Transactional Effects",
        version: 1,
        authority: "effect engine",
    },
    ContractDescriptor {
        id: ContractId::NRP,
        name: "Nous Resource Protocol",
        version: 1,
        authority: "resource and scheduler core",
    },
    ContractDescriptor {
        id: ContractId::NPA,
        name: "Nous Provider ABI",
        version: 1,
        authority: "provider host",
    },
    ContractDescriptor {
        id: ContractId::NKI,
        name: "Nous Kernel Interface",
        version: 1,
        authority: "nousd NKI server",
    },
    ContractDescriptor {
        id: ContractId::NMP,
        name: "Nous Micro Protocol",
        version: 1,
        authority: "Nous Micro runtime",
    },
    ContractDescriptor {
        id: ContractId::NCT,
        name: "Nous Credential Contract",
        version: 1,
        authority: "credential broker",
    },
    ContractDescriptor {
        id: ContractId::NLC,
        name: "Nous Learning Contract",
        version: 1,
        authority: "learning governance",
    },
    ContractDescriptor {
        id: ContractId::NSE,
        name: "Nous Safety Envelope",
        version: 1,
        authority: "safety gate",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn foundation_contract_ids_are_unique() {
        let ids: HashSet<_> = FOUNDATION_CONTRACTS.iter().map(|item| item.id).collect();
        assert_eq!(ids.len(), FOUNDATION_CONTRACTS.len());
        assert!(FOUNDATION_CONTRACTS.iter().all(|item| item.version == 1));
    }
}
