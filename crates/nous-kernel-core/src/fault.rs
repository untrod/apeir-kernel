use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaultPoint {
    EffectAfterIntent,
    EffectBeforeExecute,
    EffectAfterExecute,
    EffectAfterReceipt,
    EffectAfterObservation,
    EffectAfterCommit,
    ResourceAfterRelease,
}

impl FaultPoint {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EffectAfterIntent => "effect.after_intent",
            Self::EffectBeforeExecute => "effect.before_execute",
            Self::EffectAfterExecute => "effect.after_execute",
            Self::EffectAfterReceipt => "effect.after_receipt",
            Self::EffectAfterObservation => "effect.after_observation",
            Self::EffectAfterCommit => "effect.after_commit",
            Self::ResourceAfterRelease => "resource.after_release",
        }
    }
}

pub(crate) fn trigger(point: FaultPoint, operation_id: &str) {
    #[cfg(feature = "fault-injection")]
    {
        let selected = std::env::var("NOUS_FAULT_POINT").unwrap_or_default();
        let selected_operation = std::env::var("NOUS_FAULT_OPERATION").unwrap_or_default();
        if selected == point.as_str()
            && (selected_operation.is_empty() || selected_operation == operation_id)
        {
            std::process::abort();
        }
    }
    #[cfg(not(feature = "fault-injection"))]
    {
        let _ = (point, operation_id);
    }
}
