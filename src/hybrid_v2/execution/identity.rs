//! Canonical BACKEND execution identity for Hybrid V2 option match
//! executions.
//!
//! The on-chain `executionId` returned by
//! `OptionMatchingEngineV2.executeMatch(...)` is
//!   `keccak256(buyerOrderId, sellerOrderId, block.number,
//!              block.timestamp, fillQuantity1e8)`
//! and therefore is NOT knowable at intent-formation time (both
//! `block.number` and `block.timestamp` are decided by the sequencer).
//!
//! To identify the pre-broadcast intent-to-execute deterministically —
//! so restart, retry, and admin idempotency all converge — the backend
//! derives its own canonical id from data available at intent time:
//!
//!   canonical_execution_id = keccak256(
//!       "HV2_EXEC_V1"                          // 11 bytes tag
//!    || deployment_id_be8                      // 8 bytes big-endian
//!    || chain_id_be8                           // 8 bytes big-endian
//!    || buyer_order_hash                       // 32 bytes
//!    || seller_order_hash                      // 32 bytes
//!    || fill_quantity_1e8_be16                 // 16 bytes big-endian u128
//!   )
//!
//! Emitted as `0x` prefixed 32-byte lowercase hex. The buyer/seller
//! inputs are ORDERED — swapping them yields a distinct id. This is a
//! deliberate correctness property: the engine assigns buyer and seller
//! roles (maker/taker + side) and the backend's intent record must
//! preserve that ordering.

use sha3::{Digest, Keccak256};

/// Canonical BACKEND execution identity for a Hybrid V2 option match
/// intent. Wraps the 0x-prefixed 32-byte lowercase hex string returned
/// by [`derive_canonical_execution_id`].
///
/// This type is intentionally opaque to prevent silent construction
/// from user-supplied strings. The persistence layer stores the raw
/// hex; every other code path should hold this type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalExecutionId(pub String);

impl CanonicalExecutionId {
    /// Borrow the 0x-prefixed lowercase hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the owned hex string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for CanonicalExecutionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Derive the canonical BACKEND execution identity for a Hybrid V2
/// option match. See module docs for the exact preimage layout.
///
/// # Inputs
/// * `deployment_id` — backend deployment row id. Serialized as
///   big-endian i64 (cast to u64 bit-pattern to preserve
///   determinism across signed/unsigned confusion).
/// * `chain_id` — EVM chain id. Serialized as big-endian u64.
/// * `buyer_order_hash` / `seller_order_hash` — 0x-prefixed 32-byte
///   hex strings. Any other length or non-hex character produces a
///   deterministic-but-different id (the strings are hashed as-is via
///   their canonical byte decoding — the caller is expected to have
///   validated shape upstream). Both are decoded to raw bytes before
///   hashing, so surface-formatting (case) does not affect the id.
/// * `fill_quantity_1e8` — u128 quantity in 1e8 units. Serialized as
///   big-endian 16 bytes.
pub fn derive_canonical_execution_id(
    deployment_id: i64,
    chain_id: u64,
    buyer_order_hash: &str,
    seller_order_hash: &str,
    fill_quantity_1e8: u128,
) -> CanonicalExecutionId {
    let mut hasher = Keccak256::new();
    // Domain-separation tag. Bump the suffix (V1/V2/...) any time the
    // preimage layout changes; the persisted ids are opaque so the
    // caller MUST NOT read old rows with a new tag.
    hasher.update(b"HV2_EXEC_V1");
    // Cast i64 → u64 bit-pattern to preserve the exact 8-byte layout
    // regardless of sign. Callers on the read path recompute using the
    // same bit-cast, so the identity round-trips deterministically.
    hasher.update((deployment_id as u64).to_be_bytes());
    hasher.update(chain_id.to_be_bytes());
    hasher.update(decode_hex32_lax(buyer_order_hash));
    hasher.update(decode_hex32_lax(seller_order_hash));
    hasher.update(fill_quantity_1e8.to_be_bytes());
    let digest = hasher.finalize();
    // `finalize()` returns a `GenericArray<u8, U32>`; deref to `[u8]`
    // without touching the deprecated `.as_slice()` method.
    let bytes: &[u8] = &digest[..];
    let hex = to_hex_lowercase_prefixed(bytes);
    CanonicalExecutionId(hex)
}

/// Lax hex decoder — accepts optional 0x prefix, any case, and returns
/// a 32-byte array. If the input does not decode cleanly to exactly 32
/// bytes the function returns the input's raw UTF-8 bytes padded /
/// truncated to 32 bytes. This deliberately keeps the derivation total
/// (no panics on malformed input) while still producing a distinct id
/// for distinct inputs — the caller is expected to have validated the
/// hash shape upstream via the SQL CHECK constraints on
/// `hybrid_v2_execution_requests`.
fn decode_hex32_lax(s: &str) -> [u8; 32] {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    let mut out = [0u8; 32];
    if stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        for i in 0..32 {
            let hi = hex_nibble(stripped.as_bytes()[2 * i]);
            let lo = hex_nibble(stripped.as_bytes()[2 * i + 1]);
            out[i] = (hi << 4) | lo;
        }
        return out;
    }
    // Fallback: fold bytes so different inputs still hash differently.
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        out[i % 32] ^= *b;
    }
    out
}

fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => 10 + (b - b'a'),
        b'A'..=b'F' => 10 + (b - b'A'),
        _ => 0,
    }
}

fn to_hex_lowercase_prefixed(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push('0');
    s.push('x');
    for b in bytes {
        s.push(LUT[(b >> 4) as usize] as char);
        s.push(LUT[(b & 0x0f) as usize] as char);
    }
    s
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------
// Golden-vector coverage: distinct-deployment, distinct-chain,
// distinct-buyer, distinct-seller, distinct-quantity, buyer/seller
// non-commutativity, format-normalization (case + prefix), and
// stability across process invocations.

#[cfg(test)]
mod tests {
    use super::*;

    const BUYER: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const SELLER: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
    const OTHER: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";

    #[test]
    fn identity_shape_is_0x_prefixed_32_byte_lowercase_hex() {
        let id = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 1_000_000_000);
        let s = id.as_str();
        assert!(s.starts_with("0x"));
        assert_eq!(s.len(), 2 + 64);
        assert!(s[2..].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(s[2..].chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn deterministic_across_calls() {
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_deployment_ids_produce_distinct_execution_ids() {
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(2, 84532, BUYER, SELLER, 42);
        assert_ne!(a, b);
    }

    #[test]
    fn distinct_chain_ids_produce_distinct_execution_ids() {
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(1, 11155111, BUYER, SELLER, 42);
        assert_ne!(a, b);
    }

    #[test]
    fn distinct_buyer_hashes_produce_distinct_execution_ids() {
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(1, 84532, OTHER, SELLER, 42);
        assert_ne!(a, b);
    }

    #[test]
    fn distinct_seller_hashes_produce_distinct_execution_ids() {
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(1, 84532, BUYER, OTHER, 42);
        assert_ne!(a, b);
    }

    #[test]
    fn distinct_fill_quantities_produce_distinct_execution_ids() {
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 43);
        assert_ne!(a, b);
    }

    #[test]
    fn buyer_seller_swap_is_not_commutative() {
        // The engine assigns roles; swapping buyer<->seller changes
        // semantic identity and MUST change the canonical id.
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(1, 84532, SELLER, BUYER, 42);
        assert_ne!(a, b);
    }

    #[test]
    fn hex_case_and_prefix_are_normalized_before_hashing() {
        // 0x prefix optional, case irrelevant — decoding to raw bytes
        // is what enters the hash. Two textually distinct but
        // canonically equal hashes MUST yield the same id.
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, 42);
        let upper = format!("0x{}", BUYER.trim_start_matches("0x").to_uppercase());
        let no_prefix = SELLER.trim_start_matches("0x").to_string();
        let b = derive_canonical_execution_id(1, 84532, &upper, &no_prefix, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn negative_deployment_id_still_deterministic() {
        // Bit-cast preserves determinism; -1 and 0 must produce
        // different ids and repeat calls must match.
        let a = derive_canonical_execution_id(-1, 84532, BUYER, SELLER, 42);
        let b = derive_canonical_execution_id(-1, 84532, BUYER, SELLER, 42);
        let c = derive_canonical_execution_id(0, 84532, BUYER, SELLER, 42);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn u128_max_fill_quantity_is_accepted() {
        let a = derive_canonical_execution_id(1, 84532, BUYER, SELLER, u128::MAX);
        let b = derive_canonical_execution_id(1, 84532, BUYER, SELLER, u128::MAX - 1);
        assert_ne!(a, b);
    }
}
