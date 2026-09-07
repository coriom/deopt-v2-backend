//! DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1 — chain runtime
//! foundation.
//!
//! Thin, config-only scaffold that names the target per-chain runtime
//! model without yet materialising the full multi-chain plumbing.
//! Callers reach for [`ChainRuntimeHandle`] to obtain the canonical
//! chain identity for the currently instantiated runtime; the returned
//! handle is guaranteed to reference a `ChainConfig` from
//! [`crate::config::chains`] so a future multi-chain runtime can key
//! per-chain plumbing off the same identity.
//!
//! # Target model (for reference; not fully realised in V1)
//!
//! ```text
//! ChainRuntime
//!   ├── ChainConfig (identity + settlement asset + finality)
//!   ├── RpcReader (per-chain HTTP JSON-RPC client)
//!   ├── DeploymentAddresses (per-chain contract set)
//!   ├── IndexerState (per-chain cursor + reorg tracker)
//!   ├── OracleReader (per-chain price aggregator adapters)
//!   └── ExecutorContext (per-chain signer / broadcast queue)
//! ```
//!
//! # V1 posture
//!
//! Exactly one runtime is instantiated per binary and it always
//! targets `BASE_SEPOLIA`. Other chains are represented in the
//! `ChainConfig` registry so refusal gates can name them, but no
//! runtime is created for them. Adding a second chain later is
//! therefore a wiring change (instantiate a second runtime, key
//! per-chain state off `chain_id`), not a protocol rewrite.
//!
//! # Economic isolation
//!
//! Even in a future multi-runtime binary each `ChainRuntime` MUST own
//! its own economic state exclusively:
//!
//! * subaccount balances are keyed by `(chain_id, owner, subaccount_id)`
//! * positions never migrate between chains
//! * collateral on Chain A cannot back a position on Chain B
//!
//! These rules are architectural — enforced by never introducing a
//! bridge, a shared margin ledger, or a cross-chain nonce coordinator.

use crate::config::chains::{find_chain, ChainConfig, BASE_SEPOLIA_CHAIN_ID};

/// Handle onto a runtime's canonical chain identity. Cheap to clone
/// (borrows the static `ChainConfig`) and safe to hand to worker
/// threads.
#[derive(Clone, Copy, Debug)]
pub struct ChainRuntimeHandle {
    chain: &'static ChainConfig,
}

impl ChainRuntimeHandle {
    /// Construct a handle for the given chain id. Returns `None` if
    /// the id is not in the `ChainConfig` registry — callers should
    /// treat that as a hard configuration error.
    pub fn for_chain(chain_id: u64) -> Option<Self> {
        find_chain(chain_id).map(|chain| Self { chain })
    }

    /// Construct a handle for the V1 default chain (Base Sepolia).
    /// This is the canonical constructor for the current single-chain
    /// runtime posture.
    pub fn v1_default() -> Self {
        Self::for_chain(BASE_SEPOLIA_CHAIN_ID)
            .expect("BASE_SEPOLIA must be present in chain registry")
    }

    #[inline]
    pub fn chain(&self) -> &'static ChainConfig {
        self.chain
    }

    #[inline]
    pub fn chain_id(&self) -> u64 {
        self.chain.chain_id
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.chain.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_default_targets_base_sepolia() {
        let h = ChainRuntimeHandle::v1_default();
        assert_eq!(h.chain_id(), BASE_SEPOLIA_CHAIN_ID);
        assert!(h.is_enabled(), "the sole V1 runtime must target an enabled chain");
        assert!(h.chain().is_base_sepolia());
    }

    #[test]
    fn unknown_chain_returns_none() {
        assert!(ChainRuntimeHandle::for_chain(999_999).is_none());
    }

    #[test]
    fn base_mainnet_handle_would_be_disabled() {
        // Base mainnet is in the registry but disabled — the handle
        // constructor still succeeds (so callers can *name* it in
        // refusal messages) but `is_enabled()` reports false.
        let h = ChainRuntimeHandle::for_chain(crate::config::chains::BASE_MAINNET_CHAIN_ID)
            .expect("base mainnet must be present as a refused entry");
        assert!(!h.is_enabled());
    }
}
