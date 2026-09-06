//! Runtime deployment profiles share one contract and execution model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeProfile {
    Core,
    Edge,
    Micro,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProfileCapabilities {
    pub durable_journal: bool,
    pub distributed_scheduling: bool,
    pub transactional_effects: bool,
    pub governed_learning: bool,
    pub dynamic_allocation: bool,
}

impl RuntimeProfile {
    pub const fn capabilities(self) -> RuntimeProfileCapabilities {
        match self {
            Self::Core => RuntimeProfileCapabilities {
                durable_journal: true,
                distributed_scheduling: true,
                transactional_effects: true,
                governed_learning: true,
                dynamic_allocation: true,
            },
            Self::Edge => RuntimeProfileCapabilities {
                durable_journal: true,
                distributed_scheduling: false,
                transactional_effects: true,
                governed_learning: false,
                dynamic_allocation: true,
            },
            Self::Micro => RuntimeProfileCapabilities {
                durable_journal: false,
                distributed_scheduling: false,
                transactional_effects: true,
                governed_learning: false,
                dynamic_allocation: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_and_effects_exist_in_every_profile() {
        for profile in [
            RuntimeProfile::Core,
            RuntimeProfile::Edge,
            RuntimeProfile::Micro,
        ] {
            assert!(profile.capabilities().transactional_effects);
        }
        assert!(!RuntimeProfile::Micro.capabilities().dynamic_allocation);
    }
}
