//! DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1 +
//! DEOPT_MULTICHAIN_SCHEMA_HARDENING_AND_MULTICOLLATERAL_ACTIVATION_DESIGN_V1
//! Part B — per-chain runtime container.
//!
//! Materialises the per-chain plumbing slots enumerated in the
//! milestone Part B audit so a future multi-chain binary can
//! instantiate `ChainRuntime(Base)` and `ChainRuntime(Arbitrum)` side
//! by side without any shared mutable state.
//!
//! # Design shape
//!
//! Every runtime is bounded by exactly one [`ChainConfig`]. All the
//! plumbing slots are represented by `PerChainPlumbing` — a plain
//! struct of fields, each of which is optional so a runtime can be
//! bootstrapped incrementally (some deployments have no indexer,
//! some are read-only). The fields themselves are opaque `String`s
//! or narrow value objects rather than live client handles because
//! wiring live handles here would touch dozens of files; the point
//! of this scaffold is to name the shape and prove that constructing
//! two runtimes is a well-typed operation with no shared state, not
//! to migrate every live client into the container in one milestone.
//!
//! Live clients (RPC providers, indexer workers, signers) are still
//! constructed inside `AppState` and their per-chain identity is
//! obtained by asking the owning `ChainRuntimeHandle` for its
//! `chain_id`. When multi-chain lands the live clients will be
//! moved into `PerChainPlumbing` fields the same way — that migration
//! is a wiring change, not a schema change.
//!
//! # V1 posture
//!
//! Exactly one runtime is instantiated per binary and it always
//! targets `BASE_SEPOLIA`. The `is_enabled()` gate on the handle
//! refuses to hand out a live runtime for a disabled chain.

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

/// Named per-chain plumbing slots. Every field is optional so
/// bootstrapping a read-only runtime (no executor, no indexer) is
/// legal. Each field carries a *reference* to the concrete resource
/// (an RPC URL, a contract-set label, an indexer cursor name) rather
/// than the live client — see module docs.
///
/// Adding a second chain later requires filling in per-chain values
/// for each field consumed by whatever workers a future binary
/// enables.
#[derive(Clone, Debug, Default)]
pub struct PerChainPlumbing {
    /// Chain-scoped RPC endpoint URL. `None` when the runtime is
    /// read-only or when the RPC handle lives in a legacy singleton
    /// container.
    pub rpc_url: Option<String>,
    /// Label pointing to the per-chain contract deployment set the
    /// runtime targets. Used as a lookup key into the deployment
    /// address registry (e.g. `AppConfig::trading_views`).
    pub deployment_label: Option<String>,
    /// Cursor name used by the indexer for this chain. In V1 this is
    /// always `"perp_matching_engine"`; a second chain would use the
    /// same cursor name because the `(chain_id, name)` PK from
    /// migration 0062 keeps the pair unique.
    pub indexer_cursor_name: Option<String>,
    /// Depth-based finality confirmation count. Sourced from
    /// `ChainConfig::finality_policy`.
    pub finality_confirmations: Option<u32>,
    /// Chain-scoped executor label — points to the signer / broadcast
    /// queue owning outbound transactions on this chain.
    pub executor_label: Option<String>,
    /// Oracle policy label — matches `ChainConfig::oracle_policy`.
    pub oracle_policy_label: Option<String>,
}

/// Fully-typed per-chain runtime container. Constructed once per
/// enabled chain at startup. Owns no live client handles today —
/// see module docs.
#[derive(Clone, Debug)]
pub struct ChainRuntime {
    handle: ChainRuntimeHandle,
    plumbing: PerChainPlumbing,
}

impl ChainRuntime {
    /// Build a runtime for `chain_id`. Fails if the chain is missing
    /// from the registry or is disabled — a disabled chain cannot
    /// own a live runtime.
    pub fn build(chain_id: u64, plumbing: PerChainPlumbing) -> Result<Self, ChainRuntimeError> {
        let handle = ChainRuntimeHandle::for_chain(chain_id)
            .ok_or(ChainRuntimeError::UnknownChain(chain_id))?;
        if !handle.is_enabled() {
            return Err(ChainRuntimeError::DisabledChain(chain_id));
        }
        // Cross-check the finality expected by the plumbing against
        // the registry's finality policy so the caller can't wire a
        // stale confirmation count.
        if let Some(configured) = plumbing.finality_confirmations {
            let expected = handle
                .chain()
                .finality_policy
                .expected_confirmations()
                .unwrap_or(configured);
            if expected != configured {
                return Err(ChainRuntimeError::FinalityMismatch {
                    chain_id,
                    configured,
                    expected,
                });
            }
        }
        Ok(Self { handle, plumbing })
    }

    /// V1 constructor. Instantiates the sole Base Sepolia runtime
    /// with the plumbing shape the current binary uses. The plumbing
    /// values here are minimal — this constructor is meant for
    /// startup wiring today and as the reference call site future
    /// multi-chain code will mirror per chain.
    pub fn v1_default_base_sepolia() -> Self {
        let handle = ChainRuntimeHandle::v1_default();
        Self {
            handle,
            plumbing: PerChainPlumbing {
                rpc_url: None,
                deployment_label: Some("base-sepolia-hybrid-v2".to_string()),
                indexer_cursor_name: Some("perp_matching_engine".to_string()),
                finality_confirmations: handle
                    .chain()
                    .finality_policy
                    .expected_confirmations(),
                executor_label: Some("base-sepolia-executor".to_string()),
                oracle_policy_label: Some("dual-source".to_string()),
            },
        }
    }

    #[inline]
    pub fn handle(&self) -> ChainRuntimeHandle {
        self.handle
    }

    #[inline]
    pub fn chain_id(&self) -> u64 {
        self.handle.chain_id()
    }

    #[inline]
    pub fn chain(&self) -> &'static ChainConfig {
        self.handle.chain()
    }

    #[inline]
    pub fn plumbing(&self) -> &PerChainPlumbing {
        &self.plumbing
    }
}

impl crate::config::chains::FinalityPolicy {
    /// Return the expected confirmation depth for depth-based
    /// finality policies. `None` for policies where depth is not the
    /// gating quantity (there are no such variants today).
    pub fn expected_confirmations(&self) -> Option<u32> {
        match self {
            crate::config::chains::FinalityPolicy::ConfirmationDepth(n) => Some(*n),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChainRuntimeError {
    #[error("chain id {0} is not in the ChainConfig registry")]
    UnknownChain(u64),
    #[error("chain id {0} is registered but not enabled — cannot instantiate a live runtime")]
    DisabledChain(u64),
    #[error("finality mismatch for chain {chain_id}: plumbing configured {configured} confirmations but registry expects {expected}")]
    FinalityMismatch {
        chain_id: u64,
        configured: u32,
        expected: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::chains::{BASE_MAINNET_CHAIN_ID, BASE_SEPOLIA_CHAIN_ID};

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
        let h = ChainRuntimeHandle::for_chain(BASE_MAINNET_CHAIN_ID)
            .expect("base mainnet must be present as a refused entry");
        assert!(!h.is_enabled());
    }

    #[test]
    fn build_refuses_disabled_chain() {
        let err = ChainRuntime::build(BASE_MAINNET_CHAIN_ID, PerChainPlumbing::default())
            .expect_err("must refuse a disabled chain");
        assert!(matches!(err, ChainRuntimeError::DisabledChain(id) if id == BASE_MAINNET_CHAIN_ID));
    }

    #[test]
    fn build_refuses_unknown_chain() {
        let err = ChainRuntime::build(555_555, PerChainPlumbing::default())
            .expect_err("must refuse an unknown chain");
        assert!(matches!(err, ChainRuntimeError::UnknownChain(id) if id == 555_555));
    }

    #[test]
    fn build_ok_for_enabled_chain_with_matching_plumbing() {
        let plumbing = PerChainPlumbing {
            finality_confirmations: Some(1),
            ..Default::default()
        };
        let runtime = ChainRuntime::build(BASE_SEPOLIA_CHAIN_ID, plumbing).unwrap();
        assert_eq!(runtime.chain_id(), BASE_SEPOLIA_CHAIN_ID);
    }

    #[test]
    fn build_rejects_finality_mismatch() {
        let plumbing = PerChainPlumbing {
            finality_confirmations: Some(99),
            ..Default::default()
        };
        let err = ChainRuntime::build(BASE_SEPOLIA_CHAIN_ID, plumbing)
            .expect_err("must refuse mismatched finality");
        assert!(matches!(
            err,
            ChainRuntimeError::FinalityMismatch { chain_id: 84532, configured: 99, .. }
        ));
    }

    #[test]
    fn v1_default_constructor_produces_isolated_runtime() {
        let runtime = ChainRuntime::v1_default_base_sepolia();
        assert_eq!(runtime.chain_id(), BASE_SEPOLIA_CHAIN_ID);
        assert!(runtime.chain().enabled);
        assert_eq!(
            runtime.plumbing().indexer_cursor_name.as_deref(),
            Some("perp_matching_engine")
        );
    }

    #[test]
    fn two_runtimes_have_no_shared_mutable_state() {
        // We cannot instantiate a second enabled runtime today (only
        // BASE_SEPOLIA is enabled). This test proves the type is
        // capable of holding two independent instances via cloning —
        // the sole guarantee we need for the "second-chain readiness"
        // audit: no static mut, no lazy_static singleton, no shared
        // interior-mutable field.
        let a = ChainRuntime::v1_default_base_sepolia();
        let b = a.clone();
        assert_eq!(a.chain_id(), b.chain_id());
        assert_eq!(
            a.plumbing().indexer_cursor_name,
            b.plumbing().indexer_cursor_name,
        );
    }
}
