//! OPTIONS-HYBRID-V2-ECONOMIC-EXECUTION-CORE-V1 — canonical identity
//! derivation for Options orders and fills.
//!
//! Purpose: give every Options order a cryptographically stable
//! canonical order hash, and give every Options fill exactly one
//! canonical Hybrid V2 execution ID. Both derivations are pure
//! functions of their inputs — no I/O, no randomness, no side
//! effects — so they survive restart, DB replay, and duplicate
//! delivery.
//!
//! Design rules (frozen):
//!
//!   * `derive_canonical_order_hash` binds every execution-relevant
//!     field of an Options order: deployment, chain, owner,
//!     subaccount, series, side, price, size, TIF, post-only, nonce,
//!     deadline. Two subaccounts of the same owner never collide.
//!     Same-signature intent submitted twice with different nonces
//!     produces different hashes; identical replay produces the
//!     same hash.
//!
//!   * `derive_canonical_execution_id_from_fill` reuses the existing
//!     Hybrid V2 execution identity derivation from
//!     `crate::hybrid_v2::execution::identity`. Bindings:
//!     `(deployment_id, chain_id, buyer_order_hash,
//!     seller_order_hash, fill_quantity_1e8)`. Same economic fill
//!     produces the same execution ID; different fill quantity
//!     produces a different execution ID.
//!
//!   * `OPTIONS_ORDER_HASH_V1` is the canonical preimage tag. Never
//!     change without a new version tag.
//!
//! This module intentionally does NOT modify the `OptionOrder` /
//! `OptionFill` structs; the derived identities are persisted to
//! `option_orders.canonical_order_hash` /
//! `option_fills.canonical_execution_id` (both additive, nullable,
//! immutable-once-set — migration `0053_options_canonical_execution_identity.sql`).

use crate::hybrid_v2::execution::identity::{derive_canonical_execution_id, CanonicalExecutionId};
use crate::options::types::OptionOrder;
use crate::types::{AccountId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};
use sha3::{Digest, Keccak256};

/// Canonical preimage tag for Options order hashes. Bumped only on
/// intentional versioned changes to the derivation preimage.
pub const OPTIONS_ORDER_HASH_TAG_V1: &[u8] = b"OPTIONS_ORDER_HASH_V1";

/// Structured input for canonical Options order hash derivation. Kept
/// distinct from `OptionOrder` so the hashing surface is explicit and
/// stable even if the persisted struct gains presentation-layer
/// fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionOrderHashInputs<'a> {
    pub deployment_id: i64,
    pub chain_id: u64,
    pub owner: &'a AccountId,
    pub subaccount_id: u32,
    pub series_id: &'a str,
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub time_in_force: TimeInForce,
    pub post_only: bool,
    pub nonce: Option<u64>,
    pub deadline_ms: Option<TimestampMs>,
}

impl<'a> OptionOrderHashInputs<'a> {
    /// Build hashing inputs from an `OptionOrder` plus deployment
    /// context. Called at insert time in the service layer.
    pub fn from_order(order: &'a OptionOrder, deployment_id: i64, chain_id: u64) -> Self {
        Self {
            deployment_id,
            chain_id,
            owner: &order.account,
            subaccount_id: order.subaccount_id,
            series_id: order.option_series_id.as_str(),
            side: order.side,
            price_1e8: order.price_1e8,
            size_1e8: order.size_1e8,
            time_in_force: order.time_in_force,
            post_only: order.post_only,
            nonce: order.nonce,
            deadline_ms: order.deadline_ms,
        }
    }
}

/// Canonical Options order hash. `0x`-prefixed lowercase 32-byte
/// keccak256. Persisted on `option_orders.canonical_order_hash`.
pub fn derive_canonical_order_hash(inputs: &OptionOrderHashInputs<'_>) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(OPTIONS_ORDER_HASH_TAG_V1);
    hasher.update((inputs.deployment_id as i64).to_be_bytes());
    hasher.update(inputs.chain_id.to_be_bytes());
    hasher.update(&owner_address_bytes(inputs.owner));
    hasher.update(inputs.subaccount_id.to_be_bytes());
    // Series id is variable-length UTF-8; length-prefix so
    // "ETH-01" + "-30000-C" cannot collide with "ETH-01-30000-C"
    // etc. via ambiguous concatenation.
    let series_bytes = inputs.series_id.as_bytes();
    hasher.update((series_bytes.len() as u32).to_be_bytes());
    hasher.update(series_bytes);
    hasher.update([side_discriminant(inputs.side)]);
    hasher.update(inputs.price_1e8.to_be_bytes());
    hasher.update(inputs.size_1e8.to_be_bytes());
    hasher.update([tif_discriminant(inputs.time_in_force)]);
    hasher.update([u8::from(inputs.post_only)]);
    hasher.update([u8::from(inputs.nonce.is_some())]);
    hasher.update(inputs.nonce.unwrap_or(0).to_be_bytes());
    hasher.update([u8::from(inputs.deadline_ms.is_some())]);
    hasher.update((inputs.deadline_ms.unwrap_or(0) as i64).to_be_bytes());
    let digest = hasher.finalize();
    to_hex_prefixed(&digest)
}

/// Convenience: wraps `crate::hybrid_v2::execution::identity::
/// derive_canonical_execution_id` for callers on the Options side
/// that hold canonical order hashes but do not need to depend on the
/// full Hybrid V2 execution module import path.
pub fn derive_canonical_execution_id_from_fill(
    deployment_id: i64,
    chain_id: u64,
    buyer_order_hash: &str,
    seller_order_hash: &str,
    fill_quantity_1e8: Size1e8,
) -> CanonicalExecutionId {
    derive_canonical_execution_id(
        deployment_id,
        chain_id,
        buyer_order_hash,
        seller_order_hash,
        fill_quantity_1e8,
    )
}

fn side_discriminant(side: Side) -> u8 {
    match side {
        Side::Buy => 1,
        Side::Sell => 2,
    }
}

fn tif_discriminant(tif: TimeInForce) -> u8 {
    match tif {
        TimeInForce::Gtc => 1,
        TimeInForce::Ioc => 2,
        TimeInForce::Fok => 3,
    }
}

fn owner_address_bytes(owner: &AccountId) -> [u8; 20] {
    // Owner address is 0x-prefixed 20-byte hex; normalise to raw
    // bytes for the preimage so upper/lower case never affects the
    // hash. Malformed input degrades to the zero address rather than
    // panicking; the service layer validates address shape upstream
    // via `validate_account`.
    let normalised = owner.0.trim_start_matches("0x");
    let mut out = [0u8; 20];
    if normalised.len() == 40 {
        for (i, chunk) in normalised.as_bytes().chunks(2).enumerate() {
            if i >= 20 {
                break;
            }
            if let Ok(byte) = u8::from_str_radix(std::str::from_utf8(chunk).unwrap_or("00"), 16) {
                out[i] = byte;
            }
        }
    }
    out
}

fn to_hex_prefixed(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Price1e8, Size1e8};

    fn base_inputs<'a>(owner: &'a AccountId, series: &'a str) -> OptionOrderHashInputs<'a> {
        OptionOrderHashInputs {
            deployment_id: 1,
            chain_id: 84532,
            owner,
            subaccount_id: 1,
            series_id: series,
            side: Side::Buy,
            price_1e8: 1_000_000_000 as Price1e8,
            size_1e8: 100_000_000 as Size1e8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            nonce: Some(42),
            deadline_ms: Some(1_700_000_000_000),
        }
    }

    #[test]
    fn order_hash_is_deterministic() {
        let owner = AccountId::new("0x00000000000000000000000000000000000000a1");
        let inputs = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        let a = derive_canonical_order_hash(&inputs);
        let b = derive_canonical_order_hash(&inputs);
        assert_eq!(a, b, "same inputs must produce same hash");
        assert!(a.starts_with("0x"));
        assert_eq!(a.len(), 66, "0x + 64 hex chars");
    }

    #[test]
    fn different_subaccount_of_same_owner_produces_distinct_hash() {
        let owner = AccountId::new("0x00000000000000000000000000000000000000a1");
        let mut a = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        let mut b = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        a.subaccount_id = 1;
        b.subaccount_id = 2;
        assert_ne!(
            derive_canonical_order_hash(&a),
            derive_canonical_order_hash(&b),
            "subaccount id must be bound in the hash",
        );
    }

    #[test]
    fn different_chain_id_produces_distinct_hash() {
        let owner = AccountId::new("0x00000000000000000000000000000000000000a1");
        let mut a = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        let mut b = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        a.chain_id = 84532;
        b.chain_id = 8453; // never used in tests; illustrative
        assert_ne!(
            derive_canonical_order_hash(&a),
            derive_canonical_order_hash(&b),
        );
    }

    #[test]
    fn different_deployment_id_produces_distinct_hash() {
        let owner = AccountId::new("0x00000000000000000000000000000000000000a1");
        let mut a = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        let mut b = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        a.deployment_id = 1;
        b.deployment_id = 2;
        assert_ne!(
            derive_canonical_order_hash(&a),
            derive_canonical_order_hash(&b),
        );
    }

    #[test]
    fn different_nonce_produces_distinct_hash() {
        let owner = AccountId::new("0x00000000000000000000000000000000000000a1");
        let mut a = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        let mut b = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        a.nonce = Some(1);
        b.nonce = Some(2);
        assert_ne!(
            derive_canonical_order_hash(&a),
            derive_canonical_order_hash(&b),
        );
    }

    #[test]
    fn none_and_some_zero_nonce_are_distinct() {
        let owner = AccountId::new("0x00000000000000000000000000000000000000a1");
        let mut a = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        let mut b = base_inputs(&owner, "ETH-30JAN2026-3000-C");
        a.nonce = None;
        b.nonce = Some(0);
        assert_ne!(
            derive_canonical_order_hash(&a),
            derive_canonical_order_hash(&b),
            "presence bit distinguishes None from Some(0)",
        );
    }

    #[test]
    fn series_id_length_prefix_prevents_boundary_collision() {
        let owner = AccountId::new("0x00000000000000000000000000000000000000a1");
        let mut a = base_inputs(&owner, "ETH-01");
        let mut b = base_inputs(&owner, "ETH-0130000-C");
        // Same collapsed byte sequence if concatenation were unsafe;
        // length-prefix keeps them apart.
        a.series_id = "ETH-01";
        b.series_id = "ETH-0130000-C";
        assert_ne!(
            derive_canonical_order_hash(&a),
            derive_canonical_order_hash(&b),
        );
    }

    #[test]
    fn owner_address_casing_normalized() {
        let lower = AccountId::new("0x00000000000000000000000000000000000000a1");
        let mixed = AccountId::new("0x00000000000000000000000000000000000000A1");
        let a = base_inputs(&lower, "ETH-30JAN2026-3000-C");
        let b = base_inputs(&mixed, "ETH-30JAN2026-3000-C");
        assert_eq!(
            derive_canonical_order_hash(&a),
            derive_canonical_order_hash(&b),
            "hash must be case-invariant on owner hex",
        );
    }

    #[test]
    fn execution_id_bound_to_both_order_hashes() {
        let buyer_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let seller_hash = "0x2222222222222222222222222222222222222222222222222222222222222222";
        let a =
            derive_canonical_execution_id_from_fill(1, 84532, buyer_hash, seller_hash, 100_000_000);
        let b =
            derive_canonical_execution_id_from_fill(1, 84532, buyer_hash, seller_hash, 100_000_000);
        assert_eq!(a, b, "same inputs → same execution id");
    }

    #[test]
    fn execution_id_distinct_for_distinct_fill_quantity() {
        let buyer_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let seller_hash = "0x2222222222222222222222222222222222222222222222222222222222222222";
        let a =
            derive_canonical_execution_id_from_fill(1, 84532, buyer_hash, seller_hash, 100_000_000);
        let b =
            derive_canonical_execution_id_from_fill(1, 84532, buyer_hash, seller_hash, 200_000_000);
        assert_ne!(a, b, "distinct fill quantity → distinct exec id");
    }

    #[test]
    fn execution_id_swapping_buyer_seller_changes_id() {
        let buyer_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let seller_hash = "0x2222222222222222222222222222222222222222222222222222222222222222";
        let a =
            derive_canonical_execution_id_from_fill(1, 84532, buyer_hash, seller_hash, 100_000_000);
        let b =
            derive_canonical_execution_id_from_fill(1, 84532, seller_hash, buyer_hash, 100_000_000);
        assert_ne!(a, b, "buyer/seller ordering is bound in the id");
    }
}
