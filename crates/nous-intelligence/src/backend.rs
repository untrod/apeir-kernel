use crate::{invalid, require, IntelligenceError, Validate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    RemoteApi,
    Python,
    Native,
    Onnx,
    Pytorch,
    Container,
    Wasm,
    Device,
    Human,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendSpec {
    pub kind: BackendKind,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub entrypoint: String,
    #[serde(default)]
    pub configuration: BTreeMap<String, String>,
}

impl Validate for BackendSpec {
    fn validate(&self) -> Result<(), IntelligenceError> {
        if matches!(self.kind, BackendKind::RemoteApi | BackendKind::Device) {
            require("BackendSpec", "provider", &self.provider)?;
        }
        if matches!(
            self.kind,
            BackendKind::Python
                | BackendKind::Native
                | BackendKind::Onnx
                | BackendKind::Pytorch
                | BackendKind::Container
                | BackendKind::Wasm
                | BackendKind::Custom
        ) {
            require("BackendSpec", "entrypoint", &self.entrypoint)?;
        }
        if self.entrypoint.contains("..") {
            return Err(invalid(
                "BackendSpec",
                "entrypoint",
                "parent traversal is not allowed",
            ));
        }
        Ok(())
    }
}
