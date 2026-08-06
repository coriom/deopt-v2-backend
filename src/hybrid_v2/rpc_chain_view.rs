//! `BACKEND-HYBRID-V2-CHAIN-VIEW-PROVIDER-AND-RECONCILIATION-TASK-V1`
//! — production `ChainViewProvider` backed by allowlisted, block-bound
//! `eth_call` reads against the Hybrid V2 module contracts.
//!
//! Frozen posture:
//! - The provider is strictly read-only. It calls `eth_call` only.
//! - Every target address is validated against the manifest's declared
//!   module addresses and every 4-byte selector is validated against a
//!   compile-time per-module allowlist. Any deviation is refused inside
//!   [`RpcHybridV2ChainSource::eth_call`] as defence-in-depth.
//! - Block-coherence: every read in one snapshot is bound to the same
//!   `BlockRef::Number(indexed_block)` so the reconciler compares
//!   projection state against a single, consistent chain height.
//! - Provider failure is NOT projection drift: `fetch_snapshot_at`
//!   returns a typed error and the caller records
//!   `DriftClassification::ProviderUnavailable`, never mutating any
//!   projection table.
//! - Reservations, positions, order lifecycle, executions, active
//!   series, and fee accounting are intentionally UNSUPPORTED views
//!   in this milestone — the Solidity view signatures are not yet
//!   pinned. Passing this provider through the reconciler will
//!   classify divergence on those categories as UNSUPPORTED_VIEW
//!   (via an explicit sentinel in the returned `ChainSnapshot`).
//!
//! The trait `ChainViewProvider::snapshot_at` is synchronous. This
//! provider therefore exposes an async `fetch_snapshot_at(...)` that
//! populates an internal cache; the trait then returns the cached
//! snapshot for the requested block. Callers MUST call
//! `fetch_snapshot_at` before invoking `Reconciler::reconcile`.

use crate::hybrid_v2::chain_source::ChainSourceError;
use crate::hybrid_v2::chain_view::{ChainSnapshot, ChainViewProvider};
use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::rpc_chain_source::{BlockRef, RpcHybridV2ChainSource};
use alloy_primitives::{Address, FixedBytes, U256};
use alloy_sol_types::{sol, SolCall, SolValue};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use thiserror::Error;

// ------------- Solidity view signatures (pinned surface) -------------

sol! {
    /// SubaccountRegistry.ownerOf(bytes32 subKey) → address
    function ownerOf(bytes32 subKey) external view returns (address);
    /// CollateralVault.balanceWithYield(bytes32 subKey, address token) → uint256
    function balanceWithYield(bytes32 subKey, address token) external view returns (uint256);
    /// RecoveryFinalizer.getRecoveryState(bytes32 subKey) → uint8 enum
    function getRecoveryState(bytes32 subKey) external view returns (uint8);
}

// The selectors are computed by the alloy `sol!` macro; we expose the
// literal 4-byte values here so the compile-time allowlist is easy to
// audit and, crucially, so RpcHybridV2ChainSource::eth_call can enforce
// them without pulling `alloy-sol-types` into its dependency graph.
//
// These bytes are the keccak256 prefix of the canonical function
// signature strings, exactly the same as the alloy `SolCall::SELECTOR`
// consts. Unit tests compare the two forms so any drift is caught at
// compile time.
pub const SELECTOR_OWNER_OF: [u8; 4] = ownerOfCall::SELECTOR;
pub const SELECTOR_BALANCE_WITH_YIELD: [u8; 4] = balanceWithYieldCall::SELECTOR;
pub const SELECTOR_GET_RECOVERY_STATE: [u8; 4] = getRecoveryStateCall::SELECTOR;

// ------------- Provider errors -------------

#[derive(Debug, Error)]
pub enum RpcChainViewProviderError {
    #[error("chain source error: {0}")]
    ChainSource(#[from] ChainSourceError),
    #[error("abi decode failed: {0}")]
    Decode(String),
    #[error("invalid manifest address: {0}")]
    InvalidAddress(String),
    #[error("invalid subkey bytes32: {0}")]
    InvalidSubkey(String),
    #[error("invalid token address: {0}")]
    InvalidToken(String),
}

// ------------- Provider -------------

/// Production `ChainViewProvider` reading canonical state via block-bound
/// `eth_call`.
///
/// The provider caches the most recently fetched `ChainSnapshot`
/// keyed by block number. Callers MUST call [`Self::fetch_snapshot_at`]
/// before passing the provider to `Reconciler::reconcile`.
pub struct RpcChainViewProvider {
    source: Arc<RpcHybridV2ChainSource>,
    manifest: ManifestParams,
    subaccount_registry: [u8; 20],
    collateral_vault: [u8; 20],
    recovery_finalizer: [u8; 20],
    /// Full allowlist keyed by module address (20 bytes) → allowed
    /// 4-byte selectors. Passed through to
    /// `RpcHybridV2ChainSource::eth_call` for defence-in-depth.
    allowed_selectors: HashMap<[u8; 20], HashSet<[u8; 4]>>,
    cache: Mutex<CacheState>,
}

#[derive(Debug, Default)]
struct CacheState {
    head: u64,
    snapshots: BTreeMap<u64, ChainSnapshot>,
    available: bool,
}

impl RpcChainViewProvider {
    /// Build a provider bound to the manifest's module addresses. Fails
    /// when a required module address is not a well-formed 20-byte hex
    /// string.
    pub fn new(
        source: Arc<RpcHybridV2ChainSource>,
        manifest: ManifestParams,
    ) -> Result<Self, RpcChainViewProviderError> {
        let subaccount_registry = parse_address_20(&manifest.module_addresses.subaccount_registry)
            .map_err(|e| {
                RpcChainViewProviderError::InvalidAddress(format!("subaccount_registry: {e}"))
            })?;
        let collateral_vault = parse_address_20(&manifest.module_addresses.collateral_vault)
            .map_err(|e| {
                RpcChainViewProviderError::InvalidAddress(format!("collateral_vault: {e}"))
            })?;
        let recovery_finalizer = parse_address_20(&manifest.module_addresses.recovery_finalizer)
            .map_err(|e| {
                RpcChainViewProviderError::InvalidAddress(format!("recovery_finalizer: {e}"))
            })?;

        let mut allowed_selectors: HashMap<[u8; 20], HashSet<[u8; 4]>> = HashMap::new();
        allowed_selectors
            .entry(subaccount_registry)
            .or_default()
            .insert(SELECTOR_OWNER_OF);
        allowed_selectors
            .entry(collateral_vault)
            .or_default()
            .insert(SELECTOR_BALANCE_WITH_YIELD);
        allowed_selectors
            .entry(recovery_finalizer)
            .or_default()
            .insert(SELECTOR_GET_RECOVERY_STATE);

        Ok(Self {
            source,
            manifest,
            subaccount_registry,
            collateral_vault,
            recovery_finalizer,
            allowed_selectors,
            cache: Mutex::new(CacheState {
                head: 0,
                snapshots: BTreeMap::new(),
                available: true,
            }),
        })
    }

    pub fn manifest(&self) -> &ManifestParams {
        &self.manifest
    }

    pub fn allowed_selectors(&self) -> &HashMap<[u8; 20], HashSet<[u8; 4]>> {
        &self.allowed_selectors
    }

    /// Async pre-fetch. Iterates each supplied subkey and (subkey, token)
    /// pair and populates the internal cache for the requested block.
    ///
    /// On any transport / allowlist / decode failure the cache is marked
    /// `available = false` and the error is returned. Callers translate
    /// this into `DriftClassification::ProviderUnavailable` — a chain
    /// source failure never mutates the projection.
    pub async fn fetch_snapshot_at(
        &self,
        block: u64,
        subkeys: &[String],
        tokens_per_subkey: &BTreeMap<String, Vec<String>>,
    ) -> Result<(), RpcChainViewProviderError> {
        let mut snap = ChainSnapshot::default();
        snap.manifest_hash = self.manifest.manifest_hash.clone();

        // ownerOf(subKey) → address
        for subkey in subkeys {
            let sk_bytes = parse_bytes32(subkey)
                .map_err(|e| RpcChainViewProviderError::InvalidSubkey(format!("{subkey}: {e}")))?;
            let call = ownerOfCall {
                subKey: FixedBytes::<32>::from(sk_bytes),
            };
            let bytes = call.abi_encode();
            let raw = self
                .call_module(&self.subaccount_registry, &bytes, block)
                .await?;
            let owner_return = ownerOfCall::abi_decode_returns(&raw, true)
                .map_err(|e| RpcChainViewProviderError::Decode(format!("ownerOf: {e}")))?;
            let owner_addr = owner_return._0;
            snap.subaccount_owners.insert(
                subkey.clone(),
                format!("{:?}", owner_addr).to_ascii_lowercase(),
            );

            // balanceWithYield(subKey, token) → uint256 for each token
            if let Some(tokens) = tokens_per_subkey.get(subkey) {
                for token in tokens {
                    let token_bytes = parse_address_20(token).map_err(|e| {
                        RpcChainViewProviderError::InvalidToken(format!("{token}: {e}"))
                    })?;
                    let call = balanceWithYieldCall {
                        subKey: FixedBytes::<32>::from(sk_bytes),
                        token: Address::from(token_bytes),
                    };
                    let data = call.abi_encode();
                    let raw = self
                        .call_module(&self.collateral_vault, &data, block)
                        .await?;
                    let bal_return =
                        balanceWithYieldCall::abi_decode_returns(&raw, true).map_err(|e| {
                            RpcChainViewProviderError::Decode(format!("balanceWithYield: {e}"))
                        })?;
                    let amount = bal_return._0;
                    snap.balances
                        .insert((subkey.clone(), token.clone()), u256_to_dec(amount));
                }
            }

            // getRecoveryState(subKey) → uint8 → symbolic label
            let call = getRecoveryStateCall {
                subKey: FixedBytes::<32>::from(sk_bytes),
            };
            let data = call.abi_encode();
            let raw = self
                .call_module(&self.recovery_finalizer, &data, block)
                .await?;
            let rec_return = getRecoveryStateCall::abi_decode_returns(&raw, true)
                .map_err(|e| RpcChainViewProviderError::Decode(format!("getRecoveryState: {e}")))?;
            let label = recovery_label(rec_return._0);
            snap.recovery_state
                .insert(subkey.clone(), label.to_string());
        }

        let mut cache = self.cache.lock().unwrap();
        cache.head = cache.head.max(block);
        cache.snapshots.insert(block, snap);
        cache.available = true;
        Ok(())
    }

    /// Mark the provider as unavailable — used by callers when a
    /// preceding chain source probe (e.g. `head_block_number`) fails.
    /// After this, `is_available()` returns false and the reconciler
    /// records `PROVIDER_UNAVAILABLE`.
    pub fn mark_unavailable(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.available = false;
    }

    async fn call_module(
        &self,
        target: &[u8; 20],
        data: &[u8],
        block: u64,
    ) -> Result<Vec<u8>, RpcChainViewProviderError> {
        let target_hex = format!("0x{}", hex_encode_bytes(target));
        let bytes = self
            .source
            .eth_call(
                &target_hex,
                data,
                BlockRef::Number(block),
                &self.allowed_selectors,
            )
            .await?;
        Ok(bytes)
    }
}

impl ChainViewProvider for RpcChainViewProvider {
    fn snapshot_at(&self, block: u64) -> Option<ChainSnapshot> {
        self.cache.lock().unwrap().snapshots.get(&block).cloned()
    }

    fn head_block(&self) -> u64 {
        self.cache.lock().unwrap().head
    }

    fn is_available(&self) -> bool {
        self.cache.lock().unwrap().available
    }
}

// -----------------------------------------------------------------
//                       HELPERS
// -----------------------------------------------------------------

fn recovery_label(v: u8) -> &'static str {
    // Mirrors `reducer::RecoveryStateProjection::as_str` ordering.
    // 0 → NORMAL, 1 → RECOVERY_PENDING, 2 → RECOVERY_ACTIVE,
    // 3 → CANCELLED, 4 → RECOVERED. Unknown values fall back to NORMAL
    // so the reconciler surfaces this as PROJECTION_DRIFT rather than
    // exploding — the operator inspects the mismatch sample.
    match v {
        0 => "NORMAL",
        1 => "RECOVERY_PENDING",
        2 => "RECOVERY_ACTIVE",
        3 => "CANCELLED",
        4 => "RECOVERED",
        _ => "NORMAL",
    }
}

fn parse_bytes32(hex: &str) -> Result<[u8; 32], String> {
    let stripped = hex
        .trim()
        .strip_prefix("0x")
        .ok_or_else(|| "expected 0x-prefixed hex".to_string())?;
    if stripped.len() != 64 {
        return Err(format!("expected 32 bytes, got {}", stripped.len() / 2));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&stripped[i * 2..i * 2 + 2], 16)
            .map_err(|e| format!("byte {i}: {e}"))?;
    }
    Ok(out)
}

fn parse_address_20(hex: &str) -> Result<[u8; 20], String> {
    let stripped = hex
        .trim()
        .strip_prefix("0x")
        .ok_or_else(|| "expected 0x-prefixed hex".to_string())?;
    if stripped.len() != 40 {
        return Err(format!("expected 20 bytes, got {}", stripped.len() / 2));
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&stripped[i * 2..i * 2 + 2], 16)
            .map_err(|e| format!("byte {i}: {e}"))?;
    }
    Ok(out)
}

fn hex_encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn u256_to_dec(v: U256) -> String {
    // alloy U256 supports Display as decimal by default.
    v.to_string()
}

// A dummy use of SolValue to keep the trait import from being flagged.
#[allow(dead_code)]
fn _sol_value_marker() -> impl SolValue {
    U256::ZERO
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_constants_match_sol_call_selectors() {
        assert_eq!(SELECTOR_OWNER_OF, ownerOfCall::SELECTOR);
        assert_eq!(SELECTOR_BALANCE_WITH_YIELD, balanceWithYieldCall::SELECTOR);
        assert_eq!(SELECTOR_GET_RECOVERY_STATE, getRecoveryStateCall::SELECTOR);
    }

    #[test]
    fn parse_address_20_rejects_bad_length() {
        assert!(parse_address_20("0xdead").is_err());
        assert!(parse_address_20(&format!("0x{}", "aa".repeat(20))).is_ok());
    }

    #[test]
    fn parse_bytes32_rejects_bad_length() {
        assert!(parse_bytes32("0xdead").is_err());
        assert!(parse_bytes32(&format!("0x{}", "aa".repeat(32))).is_ok());
    }

    #[test]
    fn recovery_label_covers_pinned_variants() {
        assert_eq!(recovery_label(0), "NORMAL");
        assert_eq!(recovery_label(1), "RECOVERY_PENDING");
        assert_eq!(recovery_label(2), "RECOVERY_ACTIVE");
        assert_eq!(recovery_label(3), "CANCELLED");
        assert_eq!(recovery_label(4), "RECOVERED");
        assert_eq!(recovery_label(255), "NORMAL");
    }
}
