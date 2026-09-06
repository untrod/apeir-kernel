//! NKI transport layer - Unix Domain Socket, Named Pipe, and QUIC.
//!
//! The transport layer handles framing, serialization, and connection
//! lifecycle. The same NKI envelope flows over all transports.

use crate::envelope::{NKIRequest, NKIResponse};

/// Transport abstraction for NKI communication.
#[async_trait::async_trait]
pub trait NKITransport: Send + Sync {
    /// Send a request and receive a response.
    async fn request(&self, request: NKIRequest) -> Result<NKIResponse, TransportError>;

    /// Check if the transport is healthy.
    async fn health_check(&self) -> Result<bool, TransportError>;
}

/// Errors that can occur at the transport layer.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Request timed out after {0}ms")]
    Timeout(u64),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Transport closed")]
    Closed,

    #[error("Unauthenticated")]
    Unauthenticated,

    #[error("Unsupported transport: {0}")]
    Unsupported(String),
}

/// Framing: 4-byte big-endian length prefix + JSON bytes.
pub struct LengthPrefixedCodec;

impl LengthPrefixedCodec {
    /// Encode a message with a 4-byte big-endian length prefix.
    pub fn encode<T: serde::Serialize>(msg: &T) -> Result<Vec<u8>, TransportError> {
        let body =
            serde_json::to_vec(msg).map_err(|e| TransportError::Serialization(e.to_string()))?;
        let len = body.len() as u32;
        let mut framed = Vec::with_capacity(4 + body.len());
        framed.extend_from_slice(&len.to_be_bytes());
        framed.extend_from_slice(&body);
        Ok(framed)
    }

    /// Encode raw bytes with a 4-byte big-endian length prefix.
    pub fn encode_raw(bytes: &[u8]) -> Vec<u8> {
        let len = bytes.len() as u32;
        let mut framed = Vec::with_capacity(4 + bytes.len());
        framed.extend_from_slice(&len.to_be_bytes());
        framed.extend_from_slice(bytes);
        framed
    }

    /// Decode a message from a 4-byte big-endian length prefix + JSON bytes.
    pub fn decode<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T, TransportError> {
        if data.len() < 4 {
            return Err(TransportError::Serialization("Message too short".into()));
        }
        let len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if data.len() < 4 + len {
            return Err(TransportError::Serialization("Message truncated".into()));
        }
        serde_json::from_slice(&data[4..4 + len])
            .map_err(|e| TransportError::Serialization(e.to_string()))
    }
}
