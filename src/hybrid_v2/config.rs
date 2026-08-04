//! `BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1` — Hybrid V2 worker
//! configuration + fail-closed validation.
//!
//! Every field is validated at config-parse time. Base mainnet
//! (`chain_id == 8453`) is refused unconditionally — the frozen posture
//! forbids Hybrid V2 experimental identity on the production Base chain.
//!
//! Design goals:
//! - Additive to `src/main.rs`: `HybridV2Config::disabled()` returns a
//!   benign no-op config so a build that never enables Hybrid V2 has
//!   nothing to configure.
//! - `HybridV2Config::from_env()` reads all env vars up-front and
//!   validates; a failing config aborts startup instead of running with
//!   silent defaults.
//! - No I/O in this module. Composition with the store + worker is
//!   done by the caller (main.rs) — this file is a pure value type.

use crate::error::{BackendError, Result};
use crate::hybrid_v2::manifest::{
    BASE_MAINNET_CHAIN_ID, BASE_SEPOLIA_CHAIN_ID, LOCAL_ANVIL_CHAIN_ID, LOCAL_HARDHAT_CHAIN_ID,
};
use std::env;

/// Hybrid V2 indexer worker + persistence configuration.
///
/// Layout mirrors what `main.rs` reads from the environment to build
/// the `IndexerRuntime` + `PostgresHybridV2ProjectionStore` +
/// worker task. Every field is validated by [`Self::validate`].
///
/// The `Debug` impl on this struct is manual and redacts `rpc_url` to
/// the URL host only — the raw URL (which may embed provider API keys
/// in the path) must never appear in logs, panic backtraces, or
/// bug reports.
#[derive(Clone, PartialEq, Eq)]
pub struct HybridV2Config {
    /// Master switch. When `false` no worker is spawned, no persistence
    /// is initialised, and the runtime is not attached to any store.
    pub enabled: bool,
    /// Deployment identifier assigned by `upsert_deployment`. Must be
    /// positive when `enabled = true`.
    pub deployment_id: i64,
    /// Chain id the manifest binds to; validated against the frozen
    /// posture (rejects Base mainnet unconditionally).
    pub chain_id: u64,
    /// Worker poll interval in milliseconds. Bounded to
    /// [100, 60_000].
    pub poll_interval_ms: u64,
    /// Number of confirmations before treating a block as finalised
    /// for read-side canonicality metadata. Bounded to [0, 1024].
    pub confirmation_depth: u64,
    /// Upper bound on the number of blocks pulled from the chain
    /// source per catch-up round. Bounded to [1, 4096].
    pub max_block_batch: u32,
    /// Optional starting block for a fresh deployment. `None` means
    /// "start from block 0" — the runtime will bootstrap or begin at
    /// the source's earliest available block.
    pub start_block: Option<u64>,
    /// Named cursor row inside `hybrid_v2_cursors`; default `"indexer"`.
    pub cursor_name: String,
    /// EVM JSON-RPC endpoint the live chain source polls. Required when
    /// `enabled = true`. MUST start with `http://` or `https://`. The
    /// full URL may embed a provider API key on the path — it must
    /// therefore never be logged verbatim (the `Debug` impl redacts
    /// this field to `<host>` only).
    pub rpc_url: Option<String>,
    /// Per-request timeout for RPC calls (ms). Bounded to [500, 60_000].
    /// Applied by the `reqwest::Client` builder.
    pub rpc_timeout_ms: u64,
    /// Number of additional retry attempts after the first request
    /// failure. Bounded to [0, 10].
    pub rpc_max_retries: u32,
    /// Base backoff (ms) between retry attempts. Bounded to [50, 10_000].
    /// Actual sleep = `rpc_retry_backoff_ms * 2^attempt` capped at 30s.
    pub rpc_retry_backoff_ms: u64,
    /// Maximum log entries returned per `eth_getLogs` call before the
    /// source rejects the response as un-bounded. Bounded to
    /// [1, 20_000]. Providers may still return fewer.
    pub rpc_max_logs_per_range: u32,
    /// Optional path to a JSON manifest file describing the deployment.
    /// When `enabled = true` and this is unset, startup fails — the
    /// manifest is authoritative for emitter addresses and event
    /// version; guessing it would let the indexer decode against the
    /// wrong contracts.
    pub manifest_path: Option<String>,
}

impl std::fmt::Debug for HybridV2Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact `rpc_url` to just the host — a raw URL may embed the
        // provider API key on the path, which must NEVER appear in
        // logs, panics, or bug reports.
        let redacted_rpc = self
            .rpc_url
            .as_deref()
            .map(redact_rpc_url)
            .unwrap_or_else(|| "<unset>".to_string());
        f.debug_struct("HybridV2Config")
            .field("enabled", &self.enabled)
            .field("deployment_id", &self.deployment_id)
            .field("chain_id", &self.chain_id)
            .field("poll_interval_ms", &self.poll_interval_ms)
            .field("confirmation_depth", &self.confirmation_depth)
            .field("max_block_batch", &self.max_block_batch)
            .field("start_block", &self.start_block)
            .field("cursor_name", &self.cursor_name)
            .field("rpc_url", &redacted_rpc)
            .field("rpc_timeout_ms", &self.rpc_timeout_ms)
            .field("rpc_max_retries", &self.rpc_max_retries)
            .field("rpc_retry_backoff_ms", &self.rpc_retry_backoff_ms)
            .field("rpc_max_logs_per_range", &self.rpc_max_logs_per_range)
            .field(
                "manifest_path",
                &self.manifest_path.as_deref().map(|_| "<set>"),
            )
            .finish()
    }
}

/// Reduce an RPC URL to a redacted form suitable for logs. Strips
/// query, path, and credentials — retains only scheme + host + port.
/// Returns `<opaque>` for anything that doesn't parse.
pub(crate) fn redact_rpc_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => {
            let scheme = u.scheme();
            let host = u.host_str().unwrap_or("<opaque>");
            match u.port() {
                Some(p) => format!("{}://{}:{}", scheme, host, p),
                None => format!("{}://{}", scheme, host),
            }
        }
        Err(_) => "<opaque>".to_string(),
    }
}

impl HybridV2Config {
    /// Return a benign, disabled configuration. `chain_id` defaults to
    /// Base Sepolia (a permitted test network); every other field takes
    /// a defensive value so a caller that forgets to check `enabled`
    /// before wiring the config still fails validation.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            deployment_id: 0,
            chain_id: BASE_SEPOLIA_CHAIN_ID,
            poll_interval_ms: 1_000,
            confirmation_depth: 12,
            max_block_batch: 128,
            start_block: None,
            cursor_name: "indexer".to_string(),
            rpc_url: None,
            rpc_timeout_ms: 10_000,
            rpc_max_retries: 3,
            rpc_retry_backoff_ms: 250,
            rpc_max_logs_per_range: 2_000,
            manifest_path: None,
        }
    }

    /// Build a [`HybridV2Config`] from process environment variables.
    ///
    /// Recognised env vars:
    /// - `HYBRID_V2_ENABLED`     — `"1"`/`"true"` (case-insensitive)
    /// - `HYBRID_V2_DEPLOYMENT_ID` — positive i64
    /// - `HYBRID_V2_CHAIN_ID`      — u64 (Base mainnet refused)
    /// - `HYBRID_V2_POLL_INTERVAL_MS`     — u64 in [100, 60_000]
    /// - `HYBRID_V2_CONFIRMATION_DEPTH`   — u64 in [0, 1024]
    /// - `HYBRID_V2_MAX_BLOCK_BATCH`      — u32 in [1, 4096]
    /// - `HYBRID_V2_START_BLOCK`          — u64 (optional)
    /// - `HYBRID_V2_CURSOR_NAME`          — non-empty string
    ///
    /// When `HYBRID_V2_ENABLED` is not set or false, returns
    /// `disabled()` immediately without validating the other fields.
    pub fn from_env() -> Result<Self> {
        let enabled = parse_bool_env("HYBRID_V2_ENABLED")?.unwrap_or(false);
        if !enabled {
            return Ok(Self::disabled());
        }
        let deployment_id: i64 = parse_env("HYBRID_V2_DEPLOYMENT_ID")?
            .ok_or_else(|| cfg_err("HYBRID_V2_ENABLED=1 but HYBRID_V2_DEPLOYMENT_ID unset"))?;
        let chain_id: u64 = parse_env("HYBRID_V2_CHAIN_ID")?
            .ok_or_else(|| cfg_err("HYBRID_V2_ENABLED=1 but HYBRID_V2_CHAIN_ID unset"))?;
        let poll_interval_ms: u64 = parse_env("HYBRID_V2_POLL_INTERVAL_MS")?.unwrap_or(1_000);
        let confirmation_depth: u64 = parse_env("HYBRID_V2_CONFIRMATION_DEPTH")?.unwrap_or(12);
        let max_block_batch: u32 = parse_env("HYBRID_V2_MAX_BLOCK_BATCH")?.unwrap_or(128);
        let start_block: Option<u64> = parse_env("HYBRID_V2_START_BLOCK")?;
        let cursor_name = env::var("HYBRID_V2_CURSOR_NAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "indexer".to_string());
        let rpc_url = env::var("HYBRID_V2_RPC_URL").ok().filter(|s| !s.is_empty());
        let rpc_timeout_ms: u64 = parse_env("HYBRID_V2_RPC_TIMEOUT_MS")?.unwrap_or(10_000);
        let rpc_max_retries: u32 = parse_env("HYBRID_V2_RPC_MAX_RETRIES")?.unwrap_or(3);
        let rpc_retry_backoff_ms: u64 = parse_env("HYBRID_V2_RPC_RETRY_BACKOFF_MS")?.unwrap_or(250);
        let rpc_max_logs_per_range: u32 =
            parse_env("HYBRID_V2_RPC_MAX_LOGS_PER_RANGE")?.unwrap_or(2_000);
        let manifest_path = env::var("HYBRID_V2_MANIFEST_PATH")
            .ok()
            .filter(|s| !s.is_empty());
        let cfg = Self {
            enabled: true,
            deployment_id,
            chain_id,
            poll_interval_ms,
            confirmation_depth,
            max_block_batch,
            start_block,
            cursor_name,
            rpc_url,
            rpc_timeout_ms,
            rpc_max_retries,
            rpc_retry_backoff_ms,
            rpc_max_logs_per_range,
            manifest_path,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Fail-closed validation. Refuses Base mainnet unconditionally
    /// and enforces bounds on every other numeric field.
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            // Nothing to validate for a disabled config; the caller
            // must not wire it into the runtime.
            return Ok(());
        }
        if self.chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(cfg_err(format!(
                "Base mainnet forbidden: HYBRID_V2_CHAIN_ID={} is not authorised for Hybrid V2",
                self.chain_id
            )));
        }
        // Only known networks are accepted. This mirrors the
        // NetworkPolicy layer's guardrail so misconfigurations surface
        // at boot rather than at first block.
        let permitted = matches!(
            self.chain_id,
            BASE_SEPOLIA_CHAIN_ID | LOCAL_ANVIL_CHAIN_ID | LOCAL_HARDHAT_CHAIN_ID
        );
        if !permitted {
            return Err(cfg_err(format!(
                "unsupported HYBRID_V2_CHAIN_ID={}: only Base Sepolia + local dev chains \
                 are recognised by this build",
                self.chain_id
            )));
        }
        if self.deployment_id <= 0 {
            return Err(cfg_err(format!(
                "HYBRID_V2_DEPLOYMENT_ID must be > 0, got {}",
                self.deployment_id
            )));
        }
        if !(100..=60_000).contains(&self.poll_interval_ms) {
            return Err(cfg_err(format!(
                "HYBRID_V2_POLL_INTERVAL_MS must be within [100, 60000], got {}",
                self.poll_interval_ms
            )));
        }
        if self.confirmation_depth > 1024 {
            return Err(cfg_err(format!(
                "HYBRID_V2_CONFIRMATION_DEPTH must be <= 1024, got {}",
                self.confirmation_depth
            )));
        }
        if !(1..=4096).contains(&self.max_block_batch) {
            return Err(cfg_err(format!(
                "HYBRID_V2_MAX_BLOCK_BATCH must be within [1, 4096], got {}",
                self.max_block_batch
            )));
        }
        if self.cursor_name.is_empty() {
            return Err(cfg_err("HYBRID_V2_CURSOR_NAME must be non-empty"));
        }
        // rpc_url is required when enabled=true, and must be http(s).
        match self.rpc_url.as_deref() {
            None => {
                return Err(cfg_err(
                    "HYBRID_V2_ENABLED=1 but HYBRID_V2_RPC_URL unset — the live \
                     JSON-RPC chain source cannot start without an endpoint",
                ));
            }
            Some(url) if !(url.starts_with("http://") || url.starts_with("https://")) => {
                return Err(cfg_err(
                    "HYBRID_V2_RPC_URL must start with http:// or https:// \
                     (redacted; only http/https reqwest transports are supported)",
                ));
            }
            Some(_) => {}
        }
        if !(500..=60_000).contains(&self.rpc_timeout_ms) {
            return Err(cfg_err(format!(
                "HYBRID_V2_RPC_TIMEOUT_MS must be within [500, 60000], got {}",
                self.rpc_timeout_ms
            )));
        }
        if self.rpc_max_retries > 10 {
            return Err(cfg_err(format!(
                "HYBRID_V2_RPC_MAX_RETRIES must be <= 10, got {}",
                self.rpc_max_retries
            )));
        }
        if !(50..=10_000).contains(&self.rpc_retry_backoff_ms) {
            return Err(cfg_err(format!(
                "HYBRID_V2_RPC_RETRY_BACKOFF_MS must be within [50, 10000], got {}",
                self.rpc_retry_backoff_ms
            )));
        }
        if !(1..=20_000).contains(&self.rpc_max_logs_per_range) {
            return Err(cfg_err(format!(
                "HYBRID_V2_RPC_MAX_LOGS_PER_RANGE must be within [1, 20000], got {}",
                self.rpc_max_logs_per_range
            )));
        }
        // Manifest path is required when enabled=true — it carries the
        // canonical emitter set and event version the runtime binds to.
        if self.manifest_path.as_deref().unwrap_or_default().is_empty() {
            return Err(cfg_err(
                "HYBRID_V2_ENABLED=1 but HYBRID_V2_MANIFEST_PATH unset — the \
                 canonical manifest is required to bind emitter addresses",
            ));
        }
        Ok(())
    }
}

fn cfg_err(msg: impl Into<String>) -> BackendError {
    BackendError::Config(msg.into())
}

fn parse_bool_env(name: &str) -> Result<Option<bool>> {
    match env::var(name) {
        Err(_) => Ok(None),
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "" => Ok(None),
            "1" | "true" | "yes" | "on" => Ok(Some(true)),
            "0" | "false" | "no" | "off" => Ok(Some(false)),
            other => Err(cfg_err(format!(
                "{name} must be a boolean (1/true/0/false), got {other:?}"
            ))),
        },
    }
}

fn parse_env<T>(name: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Err(_) => Ok(None),
        Ok(v) if v.is_empty() => Ok(None),
        Ok(v) => v
            .parse::<T>()
            .map(Some)
            .map_err(|e| cfg_err(format!("{name}={v:?} is not parsable: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config_base() -> HybridV2Config {
        let mut cfg = HybridV2Config::disabled();
        cfg.enabled = true;
        cfg.deployment_id = 1;
        cfg.chain_id = BASE_SEPOLIA_CHAIN_ID;
        cfg.rpc_url = Some("https://sepolia.base.org/some/api-key".to_string());
        cfg.manifest_path = Some("/tmp/manifest.json".to_string());
        cfg
    }

    #[test]
    fn disabled_config_skips_validation() {
        let cfg = HybridV2Config::disabled();
        assert!(!cfg.enabled);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn base_mainnet_refused_unconditionally() {
        let mut cfg = enabled_config_base();
        cfg.chain_id = BASE_MAINNET_CHAIN_ID;
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("Base mainnet forbidden"), "err={err}");
    }

    #[test]
    fn unsupported_chain_id_refused() {
        let mut cfg = enabled_config_base();
        cfg.chain_id = 999_999;
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("unsupported"), "err={err}");
    }

    #[test]
    fn poll_interval_bounds_enforced() {
        let mut cfg = enabled_config_base();
        cfg.poll_interval_ms = 50;
        assert!(cfg.validate().is_err());
        cfg.poll_interval_ms = 60_001;
        assert!(cfg.validate().is_err());
        cfg.poll_interval_ms = 1_000;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn deployment_id_must_be_positive() {
        let mut cfg = enabled_config_base();
        cfg.deployment_id = 0;
        assert!(cfg.validate().is_err());
        cfg.deployment_id = -3;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn cursor_name_must_be_nonempty() {
        let mut cfg = enabled_config_base();
        cfg.cursor_name = String::new();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn max_block_batch_bounds_enforced() {
        let mut cfg = enabled_config_base();
        cfg.max_block_batch = 0;
        assert!(cfg.validate().is_err());
        cfg.max_block_batch = 5000;
        assert!(cfg.validate().is_err());
        cfg.max_block_batch = 128;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rpc_url_required_when_enabled() {
        let mut cfg = enabled_config_base();
        cfg.rpc_url = None;
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("HYBRID_V2_RPC_URL unset"), "err={err}");
    }

    #[test]
    fn rpc_url_must_be_http_or_https() {
        let mut cfg = enabled_config_base();
        cfg.rpc_url = Some("ws://foo/bar".to_string());
        assert!(cfg.validate().is_err());
        cfg.rpc_url = Some("http://foo/bar".to_string());
        assert!(cfg.validate().is_ok());
        cfg.rpc_url = Some("https://foo/bar".to_string());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rpc_timeout_bounds_enforced() {
        let mut cfg = enabled_config_base();
        cfg.rpc_timeout_ms = 400;
        assert!(cfg.validate().is_err());
        cfg.rpc_timeout_ms = 70_000;
        assert!(cfg.validate().is_err());
        cfg.rpc_timeout_ms = 10_000;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rpc_max_retries_bounds_enforced() {
        let mut cfg = enabled_config_base();
        cfg.rpc_max_retries = 11;
        assert!(cfg.validate().is_err());
        cfg.rpc_max_retries = 3;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rpc_retry_backoff_bounds_enforced() {
        let mut cfg = enabled_config_base();
        cfg.rpc_retry_backoff_ms = 20;
        assert!(cfg.validate().is_err());
        cfg.rpc_retry_backoff_ms = 20_000;
        assert!(cfg.validate().is_err());
        cfg.rpc_retry_backoff_ms = 250;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rpc_max_logs_per_range_bounds_enforced() {
        let mut cfg = enabled_config_base();
        cfg.rpc_max_logs_per_range = 0;
        assert!(cfg.validate().is_err());
        cfg.rpc_max_logs_per_range = 20_001;
        assert!(cfg.validate().is_err());
        cfg.rpc_max_logs_per_range = 2_000;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn manifest_path_required_when_enabled() {
        let mut cfg = enabled_config_base();
        cfg.manifest_path = None;
        assert!(cfg.validate().is_err());
        cfg.manifest_path = Some(String::new());
        assert!(cfg.validate().is_err());
        cfg.manifest_path = Some("/x".to_string());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn debug_impl_never_leaks_rpc_url_path_or_key() {
        let mut cfg = enabled_config_base();
        cfg.rpc_url = Some("https://mainnet.example.com/v3/SUPER_SECRET_API_KEY".to_string());
        let dbg = format!("{:?}", cfg);
        assert!(
            !dbg.contains("SUPER_SECRET_API_KEY"),
            "Debug impl leaks credentials: {dbg}"
        );
        assert!(
            !dbg.contains("/v3/"),
            "Debug impl leaks the URL path: {dbg}"
        );
        assert!(dbg.contains("mainnet.example.com"));
    }

    #[test]
    fn redact_rpc_url_strips_path_and_query() {
        let out = redact_rpc_url("https://mainnet.example.com/v3/abc?token=xyz");
        assert_eq!(out, "https://mainnet.example.com");
        let out_port = redact_rpc_url("http://127.0.0.1:8545/foo");
        assert_eq!(out_port, "http://127.0.0.1:8545");
    }

    #[test]
    fn redact_rpc_url_on_garbage_returns_opaque() {
        assert_eq!(redact_rpc_url("not-a-url"), "<opaque>");
    }
}
