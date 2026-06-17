//! Public WebSocket configuration.
//!
//! All knobs are env-overridable via `PUBLIC_WS_*`; defaults are
//! conservative and safe for local + public-testnet beta. Missing env
//! vars never break startup — the defaults below are used instead.
//!
//! No secrets, no RPC URLs, no DB URLs are surfaced here.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicWsConfig {
    pub enabled: bool,
    pub path: String,
    pub max_connections: usize,
    pub max_subscriptions_per_connection: usize,
    pub max_frame_bytes: usize,
    pub client_rate_limit_per_sec: u32,
    pub heartbeat_interval_ms: u64,
    pub snapshot_interval_ms: u64,
    /// TTL applied to `auth.challenge` nonces; the wallet must call
    /// `auth.verify` before this many ms have elapsed.
    pub challenge_ttl_ms: u64,
}

impl PublicWsConfig {
    /// Conservative defaults safe for local + public testnet beta.
    pub fn default_testnet() -> Self {
        Self {
            enabled: true,
            path: "/ws".to_string(),
            max_connections: 1_000,
            max_subscriptions_per_connection: 64,
            max_frame_bytes: 64 * 1024,
            client_rate_limit_per_sec: 30,
            heartbeat_interval_ms: 15_000,
            snapshot_interval_ms: 5_000,
            challenge_ttl_ms: 60_000,
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default_testnet()
        }
    }
}

impl Default for PublicWsConfig {
    fn default() -> Self {
        Self::default_testnet()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative_and_self_consistent() {
        let c = PublicWsConfig::default_testnet();
        assert!(c.enabled);
        assert_eq!(c.path, "/ws");
        assert!(c.max_connections >= 1);
        assert!(c.max_subscriptions_per_connection >= 1);
        // Frame cap must comfortably exceed the largest expected
        // JSON-RPC subscribe payload but stay well below 1 MiB to keep
        // bandwidth predictable.
        assert!(c.max_frame_bytes >= 4 * 1024);
        assert!(c.max_frame_bytes <= 1024 * 1024);
        assert!(c.client_rate_limit_per_sec >= 1);
        assert!(c.heartbeat_interval_ms >= 1_000);
        assert!(c.snapshot_interval_ms >= 1_000);
        assert!(c.challenge_ttl_ms >= 10_000);
    }

    #[test]
    fn disabled_keeps_the_other_knobs() {
        let c = PublicWsConfig::disabled();
        assert!(!c.enabled);
        assert_eq!(c.path, "/ws");
    }
}
