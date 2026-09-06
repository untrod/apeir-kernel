//! Fencing tokens - prevent split-brain in distributed state management.
//!
//! A fencing token is a monotonically increasing identifier attached to every
//! state transition. When a lease or ownership changes hands, the new owner
//! receives a higher fencing token. The previous owner's token becomes invalid.
//!
//! Any operation using a stale fencing token is rejected.

use std::sync::atomic::{AtomicU64, Ordering};

/// A fencing token generator - produces monotonically increasing tokens.
///
/// Thread-safe. Used to ensure that only the current owner of a lease
/// or state can perform operations.
pub struct FencingToken {
    value: AtomicU64,
    node_id: String,
}

impl FencingToken {
    /// Create a new fencing token generator for a node.
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            value: AtomicU64::new(0),
            node_id: node_id.into(),
        }
    }

    /// Generate the next fencing token.
    pub fn next(&self) -> String {
        let v = self.value.fetch_add(1, Ordering::SeqCst) + 1;
        format!("{}:{}", self.node_id, v)
    }

    /// Get the current token value without incrementing.
    pub fn current_value(&self) -> u64 {
        self.value.load(Ordering::SeqCst)
    }

    /// Parse a fencing token into (node_id, value).
    pub fn parse(token: &str) -> Option<(&str, u64)> {
        let (node_id, value_str) = token.split_once(':')?;
        let value = value_str.parse::<u64>().ok()?;
        Some((node_id, value))
    }

    /// Validate that the provided token is exactly the current token.
    pub fn is_token_valid(current_token: &str, provided_token: &str) -> bool {
        match (Self::parse(current_token), Self::parse(provided_token)) {
            (Some((cur_node, cur_val)), Some((prov_node, prov_val))) => {
                // Tokens must be from the same node
                cur_node == prov_node && prov_val == cur_val
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_monotonic() {
        let ft = FencingToken::new("node-1");
        let t1 = ft.next();
        let t2 = ft.next();
        let t3 = ft.next();

        let (_, v1) = FencingToken::parse(&t1).unwrap();
        let (_, v2) = FencingToken::parse(&t2).unwrap();
        let (_, v3) = FencingToken::parse(&t3).unwrap();

        assert!(v1 < v2);
        assert!(v2 < v3);
    }

    #[test]
    fn test_token_validation() {
        assert!(FencingToken::is_token_valid("node-1:5", "node-1:5"));
        assert!(!FencingToken::is_token_valid("node-1:5", "node-1:7"));
        assert!(!FencingToken::is_token_valid("node-1:5", "node-1:3"));
        assert!(!FencingToken::is_token_valid("node-1:5", "node-2:5"));
    }

    #[test]
    fn test_parse_invalid() {
        assert!(FencingToken::parse("invalid").is_none());
        assert!(FencingToken::parse("node:notanumber").is_none());
    }
}
