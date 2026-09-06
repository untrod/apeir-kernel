//! Minimal node connectivity state used by the placement authority.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeConnectionState {
    #[default]
    Connected,
    Degraded,
    Offline,
    Reconnecting,
}

impl NodeConnectionState {
    pub fn schedulable(self) -> bool {
        matches!(self, Self::Connected | Self::Degraded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAdvertisement {
    pub node_id: String,
    pub capabilities: Vec<String>,
    pub state: NodeConnectionState,
    pub last_heartbeat_us: i64,
    pub heartbeat_timeout_us: i64,
}

impl NodeAdvertisement {
    pub fn connect(
        node_id: impl Into<String>,
        capabilities: Vec<String>,
        now_us: i64,
        heartbeat_timeout_us: i64,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            capabilities,
            state: NodeConnectionState::Connected,
            last_heartbeat_us: now_us,
            heartbeat_timeout_us: heartbeat_timeout_us.max(1),
        }
    }

    pub fn heartbeat(&mut self, now_us: i64, capabilities: Vec<String>) {
        self.last_heartbeat_us = now_us;
        self.capabilities = capabilities;
        self.state = NodeConnectionState::Connected;
    }

    pub fn disconnect(&mut self) {
        self.state = NodeConnectionState::Offline;
    }

    pub fn begin_reconnect(&mut self) {
        self.state = NodeConnectionState::Reconnecting;
    }

    pub fn evaluate_health(&mut self, now_us: i64) -> NodeConnectionState {
        if self.state == NodeConnectionState::Connected
            && now_us.saturating_sub(self.last_heartbeat_us) > self.heartbeat_timeout_us
        {
            self.state = NodeConnectionState::Degraded;
        } else if self.state == NodeConnectionState::Degraded
            && now_us.saturating_sub(self.last_heartbeat_us)
                > self.heartbeat_timeout_us.saturating_mul(2)
        {
            self.state = NodeConnectionState::Offline;
        }
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_heartbeats_remove_node_from_scheduling() {
        let mut node = NodeAdvertisement::connect("edge-1", vec!["vision".into()], 100, 10);
        assert_eq!(node.evaluate_health(111), NodeConnectionState::Degraded);
        assert!(node.state.schedulable());
        assert_eq!(node.evaluate_health(121), NodeConnectionState::Offline);
        assert!(!node.state.schedulable());
    }

    #[test]
    fn reconnect_requires_fresh_advertisement() {
        let mut node = NodeAdvertisement::connect("edge-1", vec!["vision".into()], 100, 10);
        node.disconnect();
        node.begin_reconnect();
        assert!(!node.state.schedulable());
        node.heartbeat(200, vec!["vision".into(), "onnx".into()]);
        assert_eq!(node.state, NodeConnectionState::Connected);
        assert_eq!(node.capabilities, vec!["vision", "onnx"]);
    }
}
