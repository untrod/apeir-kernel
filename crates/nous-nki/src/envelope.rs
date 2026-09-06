//! NKI request/response envelope types.
//!
//! Payload bytes are base64-encoded on the wire for cross-language compatibility.

use serde::{Deserialize, Serialize};

/// Every NKI request is wrapped in this envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NKIRequest {
    pub request_id: String,
    pub idempotency_key: String,
    pub nki_version: u32,
    pub principal_id: String,
    /// Optional local-session bearer. Never include this field in logs or traces.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_token: String,
    pub namespace: String,
    pub deadline_us: i64,
    pub traceparent: String,
    pub tracestate: String,
    pub feature_flags: Vec<String>,
    pub method: String, // Method name, e.g. "SubmitWorkload"
    #[serde(
        serialize_with = "base64_serialize",
        deserialize_with = "base64_deserialize"
    )]
    pub payload: Vec<u8>, // Method-specific payload bytes (base64 on wire)
}

fn base64_serialize<S: serde::Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
    if data.is_empty() {
        s.serialize_str("")
    } else {
        use base64::Engine;
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(data))
    }
}

fn base64_deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s: String = String::deserialize(d)?;
    if s.is_empty() {
        return Ok(Vec::new());
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(&s)
        .map_err(serde::de::Error::custom)
}

/// Every NKI response is wrapped in this envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NKIResponse {
    pub request_id: String,
    pub nki_version: u32,
    pub server_timestamp_us: i64,
    pub trace_id: String,
    #[serde(flatten)]
    pub outcome: NKIOutcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum NKIOutcome {
    #[serde(rename = "success")]
    Success { payload: serde_json::Value },
    #[serde(rename = "error")]
    Error { error: NKIErrorBody },
}

impl<'de> Deserialize<'de> for NKIOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("NKI outcome must be an object"))?;
        let status = object
            .remove("status")
            .and_then(|status| status.as_str().map(str::to_owned));

        match status.as_deref() {
            Some("success") => object
                .remove("payload")
                .map(|payload| Self::Success { payload })
                .ok_or_else(|| serde::de::Error::missing_field("payload")),
            Some("error") => {
                let error = object
                    .remove("error")
                    .ok_or_else(|| serde::de::Error::missing_field("error"))?;
                serde_json::from_value(error)
                    .map(|error| Self::Error { error })
                    .map_err(serde::de::Error::custom)
            }
            Some(other) => Err(serde::de::Error::custom(format!(
                "unknown NKI outcome status '{other}'"
            ))),
            None if object.contains_key("error") => {
                let error = object
                    .remove("error")
                    .ok_or_else(|| serde::de::Error::missing_field("error"))?;
                serde_json::from_value(error)
                    .map(|error| Self::Error { error })
                    .map_err(serde::de::Error::custom)
            }
            None if object.contains_key("payload") => {
                let payload = object
                    .remove("payload")
                    .ok_or_else(|| serde::de::Error::missing_field("payload"))?;
                Ok(Self::Success { payload })
            }
            None => Err(serde::de::Error::custom(
                "NKI outcome requires status, payload, or error",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NKIErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub failed_phase: String,
    #[serde(default)]
    pub cause: String,
    pub retryable: bool,
    #[serde(default)]
    pub recommended_delay_ms: u32,
}

impl std::fmt::Display for NKIErrorBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}
