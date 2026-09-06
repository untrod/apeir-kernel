//! nous-runtime-client - Rust NKI client SDK.
//!
//! Provides a typed Rust client for communicating with nousd via NKI.
//! Used by system services, tools, and other Rust applications.

use nous_nki::envelope::{NKIErrorBody, NKIOutcome, NKIRequest, NKIResponse};
use nous_nki::methods::NKIMethods;
use nous_nki::transport::LengthPrefixedCodec;
use nous_types::{AssetSelector, ControlAsset, ListAssetsRequest, PutAssetRequest};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Connection to an APEIR Kernel daemon.
pub struct NkiConnection {
    stream: Box<dyn AsyncStream>,
}

impl NkiConnection {
    /// Connect to nousd via Unix Domain Socket.
    pub async fn connect(socket_path: impl Into<PathBuf>) -> Result<Self, ClientError> {
        let path = socket_path.into();
        #[cfg(unix)]
        {
            let stream = tokio::net::UnixStream::connect(&path).await.map_err(|e| {
                ClientError::ConnectionFailed(format!(
                    "Cannot connect to {}: {}",
                    path.display(),
                    e
                ))
            })?;
            return Ok(Self {
                stream: Box::new(stream),
            });
        }
        #[cfg(not(unix))]
        {
            Err(ClientError::Unsupported(format!(
                "Unix domain sockets are unavailable on this platform: {}",
                path.display()
            )))
        }
    }

    /// Connect via TCP (for remote nodes).
    pub async fn connect_tcp(addr: &str) -> Result<Self, ClientError> {
        let tcp = tokio::time::timeout(DEFAULT_TIMEOUT, tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(|e| {
                ClientError::ConnectionFailed(format!("Cannot connect to {}: {}", addr, e))
            })?;
        Ok(Self {
            stream: Box::new(tcp),
        })
    }

    /// Send a request and receive the response payload.
    pub async fn request(
        &mut self,
        method: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        tokio::time::timeout(DEFAULT_TIMEOUT, self.request_inner(method, payload))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn request_inner(
        &mut self,
        method: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let request_id = uuid::Uuid::now_v7().to_string();
        let deadline_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| ClientError::Protocol(error.to_string()))?
            .as_micros()
            .saturating_add(DEFAULT_TIMEOUT.as_micros())
            .min(i64::MAX as u128) as i64;
        let request = NKIRequest {
            request_id: request_id.clone(),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            nki_version: nous_nki::NKI_VERSION,
            principal_id: "rust-sdk".into(),
            session_token: std::env::var("NOUS_NKI_TOKEN").unwrap_or_default(),
            namespace: "default".into(),
            deadline_us,
            traceparent: String::new(),
            tracestate: String::new(),
            feature_flags: vec![],
            method: method.to_string(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        };

        let body =
            serde_json::to_vec(&request).map_err(|e| ClientError::Serialization(e.to_string()))?;
        if body.len() > MAX_FRAME_BYTES {
            return Err(ClientError::Protocol("request exceeds 16 MiB".into()));
        }
        let framed = LengthPrefixedCodec::encode_raw(&body);

        self.stream
            .write_all(&framed)
            .await
            .map_err(|e| ClientError::Io(e.to_string()))?;
        self.stream
            .flush()
            .await
            .map_err(|e| ClientError::Io(e.to_string()))?;

        let mut len_buf = [0u8; 4];
        self.stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| ClientError::Io(e.to_string()))?;
        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len > MAX_FRAME_BYTES {
            return Err(ClientError::Protocol("response exceeds 16 MiB".into()));
        }
        let mut msg_buf = vec![0u8; msg_len];
        self.stream
            .read_exact(&mut msg_buf)
            .await
            .map_err(|e| ClientError::Io(e.to_string()))?;

        let response: NKIResponse = serde_json::from_slice(&msg_buf)
            .map_err(|e| ClientError::Serialization(e.to_string()))?;
        if response.request_id != request_id {
            return Err(ClientError::Protocol(
                "response request_id does not match request".into(),
            ));
        }
        if response.nki_version != nous_nki::NKI_VERSION {
            return Err(ClientError::Protocol(
                "response NKI version is unsupported".into(),
            ));
        }

        match response.outcome {
            NKIOutcome::Success { payload } => Ok(payload),
            NKIOutcome::Error { error } => Err(ClientError::NkiError(error)),
        }
    }

    // -- Convenience methods --

    pub async fn submit_workload(
        &mut self,
        spec: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(NKIMethods::SUBMIT_WORKLOAD, spec).await
    }

    pub async fn get_workload(
        &mut self,
        workload_id: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(
            NKIMethods::GET_WORKLOAD,
            serde_json::json!({"workload_id": workload_id}),
        )
        .await
    }

    pub async fn list_workloads(&mut self, limit: usize) -> Result<serde_json::Value, ClientError> {
        self.request(
            NKIMethods::LIST_WORKLOADS,
            serde_json::json!({"limit": limit}),
        )
        .await
    }

    pub async fn cancel_workload(
        &mut self,
        workload_id: &str,
        reason: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(
            NKIMethods::CANCEL_WORKLOAD,
            serde_json::json!({"workload_id": workload_id, "reason": reason, "force": false}),
        )
        .await
    }

    pub async fn health_check(&mut self, deep: bool) -> Result<serde_json::Value, ClientError> {
        self.request(NKIMethods::HEALTH_CHECK, serde_json::json!({"deep": deep}))
            .await
    }

    pub async fn register_model(
        &mut self,
        model: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(
            NKIMethods::REGISTER_MODEL,
            serde_json::json!({"model": model}),
        )
        .await
    }

    pub async fn put_control_asset(
        &mut self,
        request: &PutAssetRequest,
    ) -> Result<ControlAsset, ClientError> {
        let payload = self
            .request(
                NKIMethods::PUT_CONTROL_ASSET,
                serde_json::to_value(request)
                    .map_err(|error| ClientError::Serialization(error.to_string()))?,
            )
            .await?;
        serde_json::from_value(payload)
            .map_err(|error| ClientError::Serialization(error.to_string()))
    }

    pub async fn get_control_asset(
        &mut self,
        selector: &AssetSelector,
    ) -> Result<ControlAsset, ClientError> {
        let payload = self
            .request(
                NKIMethods::GET_CONTROL_ASSET,
                serde_json::to_value(selector)
                    .map_err(|error| ClientError::Serialization(error.to_string()))?,
            )
            .await?;
        serde_json::from_value(payload)
            .map_err(|error| ClientError::Serialization(error.to_string()))
    }

    pub async fn list_control_assets(
        &mut self,
        request: &ListAssetsRequest,
    ) -> Result<Vec<ControlAsset>, ClientError> {
        let payload = self
            .request(
                NKIMethods::LIST_CONTROL_ASSETS,
                serde_json::to_value(request)
                    .map_err(|error| ClientError::Serialization(error.to_string()))?,
            )
            .await?;
        serde_json::from_value(payload.get("assets").cloned().unwrap_or_default())
            .map_err(|error| ClientError::Serialization(error.to_string()))
    }

    pub async fn delete_control_asset(
        &mut self,
        selector: &AssetSelector,
        expected_generation: u64,
    ) -> Result<(), ClientError> {
        self.request(
            NKIMethods::DELETE_CONTROL_ASSET,
            serde_json::json!({
                "selector": selector,
                "expected_generation": expected_generation,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn list_engines(&mut self) -> Result<serde_json::Value, ClientError> {
        self.request(
            NKIMethods::LIST_ENGINES,
            serde_json::json!({"namespace": "default"}),
        )
        .await
    }

    pub async fn list_devices(&mut self) -> Result<serde_json::Value, ClientError> {
        self.request(
            NKIMethods::LIST_DEVICES,
            serde_json::json!({"namespace": "default"}),
        )
        .await
    }

    pub async fn create_checkpoint(
        &mut self,
        workload_id: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(
            NKIMethods::CREATE_CHECKPOINT,
            serde_json::json!({"workload_id": workload_id}),
        )
        .await
    }
}

/// Client SDK errors.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("NKI error: {0}")]
    NkiError(NKIErrorBody),
    #[error("Unsupported: {0}")]
    Unsupported(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Timeout")]
    Timeout,
}

/// High-level typed NKI client.
pub struct NkiClient {
    conn: NkiConnection,
}

impl NkiClient {
    pub async fn connect(socket: impl Into<PathBuf>) -> Result<Self, ClientError> {
        Ok(Self {
            conn: NkiConnection::connect(socket).await?,
        })
    }

    pub async fn connect_tcp(address: &str) -> Result<Self, ClientError> {
        Ok(Self {
            conn: NkiConnection::connect_tcp(address).await?,
        })
    }

    pub async fn submit_workload(
        &mut self,
        spec: serde_json::Value,
    ) -> Result<String, ClientError> {
        let resp = self.conn.submit_workload(spec).await?;
        resp["workload_id"]
            .as_str()
            .or_else(|| resp["operation_id"].as_str())
            .map(str::to_owned)
            .ok_or_else(|| ClientError::Protocol("submission response has no workload ID".into()))
    }

    pub async fn health(&mut self) -> Result<bool, ClientError> {
        let resp = self.conn.health_check(false).await?;
        Ok(resp["healthy"]
            .as_bool()
            .unwrap_or_else(|| matches!(resp["state"].as_str(), Some("READY") | Some("DEGRADED"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_error_display() {
        let err = ClientError::ConnectionFailed("test".into());
        assert!(err.to_string().contains("test"));
    }

    #[test]
    fn test_nki_request_construction() {
        let req = NKIRequest {
            request_id: "rid".into(),
            idempotency_key: "ik".into(),
            nki_version: 1,
            principal_id: "p".into(),
            session_token: String::new(),
            namespace: "ns".into(),
            deadline_us: 0,
            traceparent: String::new(),
            tracestate: String::new(),
            feature_flags: vec![],
            method: "HealthCheck".into(),
            payload: b"{}".to_vec(),
        };
        assert_eq!(req.method, "HealthCheck");
        assert_eq!(req.nki_version, 1);
    }
}
