//! Per-connection session state for the public WebSocket API.
//!
//! Keeps the receive loop tiny by isolating:
//!   * subscription book-keeping (channel → subscription id, monotonic
//!     `seq`),
//!   * pending wallet challenges (nonce + expiry, address-scoped),
//!   * rate-limit window (messages per second).
//!
//! The session never stores wallet signatures, private keys, or any
//! secret. Challenges are random nonces; signing is performed by the
//! wallet, not the server.

use super::protocol::Channel;
use crate::types::{AccountId, TimestampMs};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct SubscriptionState {
    pub subscription_id: String,
    pub channel: Channel,
    pub created_at_ms: TimestampMs,
    pub next_seq: u64,
}

/// Wallet-challenge state held by the session between `auth.challenge`
/// and `auth.verify`. All fields except `nonce` are reflected back in
/// the challenge response so the wallet can sign the canonical message
/// without having to know any server-only state. Successful
/// verification consumes the challenge (single-use); a replay or an
/// expired nonce fails cleanly.
#[derive(Clone, Debug)]
pub struct PendingChallenge {
    pub nonce: String,
    pub address: AccountId,
    pub issued_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub chain_id: u64,
    pub domain: String,
}

#[derive(Clone, Debug)]
pub struct WsSession {
    pub connection_id: String,
    /// Bound only after `auth.verify` succeeds. The bound address is
    /// the canonical lower-cased 0x-prefixed EVM address recovered
    /// from the EIP-191 signature of the canonical challenge.
    pub account: Option<AccountId>,
    /// `subscription_id` → live subscription. We use BTreeMap for
    /// deterministic `subscriptions` ordering in tests.
    pub subscriptions: BTreeMap<String, SubscriptionState>,
    /// `address.lowercase()` → pending challenge. Cleared on
    /// verification, expiry, or duplicate `auth.challenge` for the
    /// same address.
    pub challenges: BTreeMap<String, PendingChallenge>,
    pub connected_at_ms: TimestampMs,
    pub last_message_at_ms: TimestampMs,
    pub window_started_at_ms: TimestampMs,
    pub messages_in_current_window: u32,
}

impl WsSession {
    pub fn new(connection_id: String, now_ms: TimestampMs) -> Self {
        Self {
            connection_id,
            account: None,
            subscriptions: BTreeMap::new(),
            challenges: BTreeMap::new(),
            connected_at_ms: now_ms,
            last_message_at_ms: now_ms,
            window_started_at_ms: now_ms,
            messages_in_current_window: 0,
        }
    }

    /// True iff the session is bound to a wallet via a successful
    /// `auth.verify`.
    pub fn is_authenticated(&self) -> bool {
        self.account.is_some()
    }

    pub fn subscription_for(&self, channel: Channel) -> Option<&SubscriptionState> {
        self.subscriptions.values().find(|s| s.channel == channel)
    }

    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    pub fn prune_expired_challenges(&mut self, now_ms: TimestampMs) {
        self.challenges.retain(|_, c| c.expires_at_ms > now_ms);
    }

    pub fn record_message(&mut self, now_ms: TimestampMs) {
        if now_ms.saturating_sub(self.window_started_at_ms) >= 1_000 {
            self.window_started_at_ms = now_ms;
            self.messages_in_current_window = 0;
        }
        self.messages_in_current_window = self.messages_in_current_window.saturating_add(1);
        self.last_message_at_ms = now_ms;
    }

    /// Returns true iff the new message must be rejected as rate-limited.
    pub fn is_rate_limited(&self, limit_per_sec: u32) -> bool {
        self.messages_in_current_window > limit_per_sec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_unauthenticated_and_empty() {
        let s = WsSession::new("conn_1".to_string(), 1_000);
        assert!(!s.is_authenticated());
        assert_eq!(s.subscription_count(), 0);
        assert!(s.challenges.is_empty());
    }

    #[test]
    fn record_message_increments_window_and_rolls_after_1s() {
        let mut s = WsSession::new("c".to_string(), 0);
        s.record_message(100);
        s.record_message(200);
        s.record_message(300);
        assert_eq!(s.messages_in_current_window, 3);
        assert!(!s.is_rate_limited(30));
        // Cross the window boundary.
        s.record_message(1_500);
        assert_eq!(s.messages_in_current_window, 1);
    }

    #[test]
    fn rate_limit_triggers_when_window_exceeds_limit() {
        let mut s = WsSession::new("c".to_string(), 0);
        for i in 0..32u32 {
            s.record_message(i as i64);
        }
        assert!(s.is_rate_limited(30));
    }

    #[test]
    fn prune_expired_challenges_drops_old_only() {
        let mut s = WsSession::new("c".to_string(), 0);
        s.challenges.insert(
            "0xa".to_string(),
            PendingChallenge {
                nonce: "n1".to_string(),
                address: AccountId::new("0xa".to_string()),
                issued_at_ms: 0,
                expires_at_ms: 100,
                chain_id: 31337,
                domain: "deopt-v2-public-ws".to_string(),
            },
        );
        s.challenges.insert(
            "0xb".to_string(),
            PendingChallenge {
                nonce: "n2".to_string(),
                address: AccountId::new("0xb".to_string()),
                issued_at_ms: 0,
                expires_at_ms: 1_000,
                chain_id: 31337,
                domain: "deopt-v2-public-ws".to_string(),
            },
        );
        s.prune_expired_challenges(500);
        assert!(s.challenges.contains_key("0xb"));
        assert!(!s.challenges.contains_key("0xa"));
    }
}
