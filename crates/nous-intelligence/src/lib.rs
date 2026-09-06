//! Developer-facing intelligence contracts built on the APEIR runtime foundation.
//!
//! This crate is deliberately outside the kernel. It describes projects, model
//! graphs, datasets, experiments, and guarded model engineering operations while
//! leaving execution, scheduling, durable state, and artifacts with their
//! existing runtime authorities.

mod backend;
mod data;
mod graph;
mod model_engineering;
mod project;

pub use backend::{BackendKind, BackendSpec};
pub use data::{
    DatasetManifest, DatasetSplit, ExperimentDiff, ExperimentManifest, ExperimentStatus,
    MetricValue,
};
pub use graph::{
    GraphDiff, GraphEdge, GraphNode, IntelligenceGraph, NodeKind, SideEffectDeclaration,
};
pub use model_engineering::{
    assess_merge, ArchitectureMetadata, LicenseGrant, MergeAssessment, MergeRequest, MergeStrategy,
    ModelDiff, ModelExtension, ModelProfile,
};
pub use project::{
    analyze_project, MigrationProposal, ProjectAnalysis, ProjectManifest, ProjectReference,
};

use thiserror::Error;

pub const INTELLIGENCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IntelligenceError {
    #[error("invalid {object} field '{field}': {message}")]
    Invalid {
        object: &'static str,
        field: &'static str,
        message: String,
    },
    #[error("incompatible {object}: {message}")]
    Incompatible {
        object: &'static str,
        message: String,
    },
}

pub trait Validate {
    fn validate(&self) -> Result<(), IntelligenceError>;
}

pub(crate) fn invalid(
    object: &'static str,
    field: &'static str,
    message: impl Into<String>,
) -> IntelligenceError {
    IntelligenceError::Invalid {
        object,
        field,
        message: message.into(),
    }
}

pub(crate) fn require(
    object: &'static str,
    field: &'static str,
    value: &str,
) -> Result<(), IntelligenceError> {
    if value.trim().is_empty() {
        return Err(invalid(object, field, "must not be empty"));
    }
    Ok(())
}

pub(crate) fn schema(object: &'static str, version: u32) -> Result<(), IntelligenceError> {
    if version != INTELLIGENCE_SCHEMA_VERSION {
        return Err(invalid(
            object,
            "schema_version",
            format!("expected {INTELLIGENCE_SCHEMA_VERSION}, found {version}"),
        ));
    }
    Ok(())
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        && value != "."
        && value != ".."
}
