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
#[derive(Debug, Clone, PartialEq, Eq)]
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
        let cfg = Self {
            enabled: true,
            deployment_id,
            chain_id,
            poll_interval_ms,
            confirmation_depth,
            max_block_batch,
            start_block,
            cursor_name,
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

    #[test]
    fn disabled_config_skips_validation() {
        let cfg = HybridV2Config::disabled();
        assert!(!cfg.enabled);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn base_mainnet_refused_unconditionally() {
        let mut cfg = HybridV2Config::disabled();
        cfg.enabled = true;
        cfg.deployment_id = 1;
        cfg.chain_id = BASE_MAINNET_CHAIN_ID;
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("Base mainnet forbidden"), "err={err}");
    }

    #[test]
    fn unsupported_chain_id_refused() {
        let mut cfg = HybridV2Config::disabled();
        cfg.enabled = true;
        cfg.deployment_id = 1;
        cfg.chain_id = 999_999;
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("unsupported"), "err={err}");
    }

    #[test]
    fn poll_interval_bounds_enforced() {
        let mut cfg = HybridV2Config::disabled();
        cfg.enabled = true;
        cfg.deployment_id = 1;
        cfg.chain_id = BASE_SEPOLIA_CHAIN_ID;
        cfg.poll_interval_ms = 50;
        assert!(cfg.validate().is_err());
        cfg.poll_interval_ms = 60_001;
        assert!(cfg.validate().is_err());
        cfg.poll_interval_ms = 1_000;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn deployment_id_must_be_positive() {
        let mut cfg = HybridV2Config::disabled();
        cfg.enabled = true;
        cfg.chain_id = BASE_SEPOLIA_CHAIN_ID;
        cfg.deployment_id = 0;
        assert!(cfg.validate().is_err());
        cfg.deployment_id = -3;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn cursor_name_must_be_nonempty() {
        let mut cfg = HybridV2Config::disabled();
        cfg.enabled = true;
        cfg.chain_id = BASE_SEPOLIA_CHAIN_ID;
        cfg.deployment_id = 1;
        cfg.cursor_name = String::new();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn max_block_batch_bounds_enforced() {
        let mut cfg = HybridV2Config::disabled();
        cfg.enabled = true;
        cfg.chain_id = BASE_SEPOLIA_CHAIN_ID;
        cfg.deployment_id = 1;
        cfg.max_block_batch = 0;
        assert!(cfg.validate().is_err());
        cfg.max_block_batch = 5000;
        assert!(cfg.validate().is_err());
        cfg.max_block_batch = 128;
        assert!(cfg.validate().is_ok());
    }
}
