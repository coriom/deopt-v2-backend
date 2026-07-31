//! Read-only chain view + reconciliation.
//!
//! `ChainViewProvider` reports the on-chain canonical view of a bounded
//! set of state slices. `Reconciler` compares that against the projection
//! reducer's derived state and classifies the result.
//!
//! Frozen rules:
//! - No state-changing RPC. This is a strictly read-only interface.
//! - Reconciliation is bounded per batch (per-subKey/token pair count).
//! - Drift never auto-repairs projections — it fails readiness.

use crate::hybrid_v2::reducer::{ProjectionState, RecoveryStateProjection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bounded snapshot of on-chain state, all uint256 as decimal strings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainSnapshot {
    pub manifest_hash: String,
    pub subaccount_owners: BTreeMap<String, String>, // subKey → owner
    pub balances: BTreeMap<(String, String), String>, // (subKey, token) → amount
    pub reservations: BTreeMap<(String, String, String), String>,
    pub recovery_state: BTreeMap<String, String>,
}

pub trait ChainViewProvider: Send + Sync {
    fn snapshot_at(&self, block: u64) -> Option<ChainSnapshot>;
    fn head_block(&self) -> u64;
    fn is_available(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryChainViewProvider {
    pub head: u64,
    pub snapshots: BTreeMap<u64, ChainSnapshot>,
    pub available: bool,
}

impl InMemoryChainViewProvider {
    pub fn new() -> Self {
        Self {
            head: 0,
            snapshots: BTreeMap::new(),
            available: true,
        }
    }
    pub fn set_snapshot(&mut self, block: u64, snap: ChainSnapshot) -> &mut Self {
        self.head = self.head.max(block);
        self.snapshots.insert(block, snap);
        self
    }
    pub fn set_available(&mut self, v: bool) -> &mut Self {
        self.available = v;
        self
    }
}

impl ChainViewProvider for InMemoryChainViewProvider {
    fn snapshot_at(&self, block: u64) -> Option<ChainSnapshot> {
        self.snapshots.get(&block).cloned()
    }
    fn head_block(&self) -> u64 {
        self.head
    }
    fn is_available(&self) -> bool {
        self.available
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationResult {
    Converged { at_block: u64 },
    IndexerBehind { indexed: u64, observed: u64 },
    NonFinalDifference { block: u64 },
    ManifestMismatch { expected: String, actual: String },
    ProjectionDrift { detail: String },
    ProviderUnavailable,
    Unsupported { detail: String },
}

pub struct Reconciler {
    pub max_pairs_per_batch: usize,
}

impl Reconciler {
    pub fn new() -> Self {
        Self {
            max_pairs_per_batch: 4096,
        }
    }

    pub fn reconcile(
        &self,
        expected_manifest_hash: &str,
        indexed_block: u64,
        state: &ProjectionState,
        provider: &dyn ChainViewProvider,
    ) -> ReconciliationResult {
        if !provider.is_available() {
            return ReconciliationResult::ProviderUnavailable;
        }
        let observed = provider.head_block();
        if indexed_block < observed {
            return ReconciliationResult::IndexerBehind {
                indexed: indexed_block,
                observed,
            };
        }
        let Some(snap) = provider.snapshot_at(indexed_block) else {
            return ReconciliationResult::NonFinalDifference {
                block: indexed_block,
            };
        };
        if !snap
            .manifest_hash
            .eq_ignore_ascii_case(expected_manifest_hash)
        {
            return ReconciliationResult::ManifestMismatch {
                expected: expected_manifest_hash.to_string(),
                actual: snap.manifest_hash,
            };
        }
        // Balance drift.
        let mut compared = 0usize;
        for ((subkey, token), amount) in state.balances.iter() {
            if compared >= self.max_pairs_per_batch {
                break;
            }
            compared += 1;
            let chain_amount = snap
                .balances
                .get(&(subkey.clone(), token.clone()))
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            if amount != &chain_amount {
                return ReconciliationResult::ProjectionDrift {
                    detail: format!(
                        "balance drift on {}/{}: projection={} chain={}",
                        subkey, token, amount, chain_amount
                    ),
                };
            }
        }
        for ((subkey, token, engine), amount) in state.reservations.iter() {
            if compared >= self.max_pairs_per_batch {
                break;
            }
            compared += 1;
            let chain_amount = snap
                .reservations
                .get(&(subkey.clone(), token.clone(), engine.clone()))
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            if amount != &chain_amount {
                return ReconciliationResult::ProjectionDrift {
                    detail: format!(
                        "reservation drift on {}/{}/{}: projection={} chain={}",
                        subkey, token, engine, amount, chain_amount
                    ),
                };
            }
        }
        for (subkey, rec) in state.recovery_state.iter() {
            let chain = snap
                .recovery_state
                .get(subkey)
                .cloned()
                .unwrap_or_else(|| "NORMAL".to_string());
            if rec.as_str() != chain {
                return ReconciliationResult::ProjectionDrift {
                    detail: format!(
                        "recovery drift on {}: projection={} chain={}",
                        subkey,
                        rec.as_str(),
                        chain
                    ),
                };
            }
        }
        let _ = RecoveryStateProjection::Normal;
        ReconciliationResult::Converged {
            at_block: indexed_block,
        }
    }
}

impl Default for Reconciler {
    fn default() -> Self {
        Self::new()
    }
}
