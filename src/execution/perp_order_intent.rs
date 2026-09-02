//! PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part D — Rust mirror of the
//! Solidity `PerpOrderIntent` EIP-712 struct.
//!
//! The type string + typehash below are byte-frozen against the on-chain
//! `PerpMatchingEngine.PERP_ORDER_INTENT_TYPEHASH`. Any drift here means
//! signatures the backend previews (or verifies against a signer) will
//! NOT verify on-chain — the wire-lock test `perp_order_intent_typehash_matches_locked_value`
//! is the earliest place the drift surfaces.
//!
//! Domain: reuses the same `PerpTradeDomain` (`DeOptV2-PerpMatchingEngine`,
//! version `"1"`) as the pre-existing `PerpTrade` type — both structs
//! are consumed by the same on-chain engine under the same domain.
//!
//! Scope:
//! * `PerpOrderIntent` — the Rust twin of the Solidity struct (fields in
//!   snake_case, same wire order).
//! * `perp_order_intent_hash` — keccak256 EIP-712 struct hash.
//! * `perp_order_intent_digest` — the full `\x19\x01 || domainSep || structHash`.
//! * `verify_perp_order_intent` — ecrecover + signer==trader assertion.
//! * `validate_shape` — pure-Rust side/bound consistency; mirrors the
//!   Solidity `IntentSideBoundInconsistent` semantics without touching
//!   nonce/deadline (those checks belong to the endpoint that also
//!   consults the process-wide nonce store).

use crate::error::{BackendError, Result};
use crate::execution::perp_trade::PerpTradeDomain;
use crate::signing::eip712::{keccak256, parse_evm_address, EIP712_DOMAIN_TYPE};
use crate::signing::recover_eip712_signer;
use crate::types::AccountId;
use alloy_primitives::B256;

/// PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 — canonical Solidity
/// `PerpOrderIntent` type string. 11 fields; field order matches the
/// Solidity struct verbatim. Byte-frozen against the Solidity source of
/// truth — the keccak256 of this string is
/// `PERP_ORDER_INTENT_TYPEHASH_HEX` below.
pub const PERP_ORDER_INTENT_TYPE: &str = "PerpOrderIntent(bytes32 intentId,address trader,uint32 subaccountId,uint256 marketId,uint8 side,uint128 size1e8,uint128 limitPrice1e8,uint128 maxExecPrice1e8,uint128 minExecPrice1e8,uint256 nonce,uint256 deadline)";

/// PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 — the on-chain
/// `PERP_ORDER_INTENT_TYPEHASH` locked by the Solidity side. Pinned by
/// `perp_order_intent_typehash_matches_locked_value` below.
pub const PERP_ORDER_INTENT_TYPEHASH_HEX: &str =
    "0xeeaf370e4195f568ccb783efe23803dd5bf3c859aef9d0c3e3f211c2da2d5d1c";

/// `PerpOrderIntent.side` sentinel: buy.
pub const PERP_ORDER_INTENT_SIDE_BUY: u8 = 0;
/// `PerpOrderIntent.side` sentinel: sell.
pub const PERP_ORDER_INTENT_SIDE_SELL: u8 = 1;

/// Rust mirror of the Solidity `PerpOrderIntent` struct.
///
/// Field names use snake_case (Rust convention); the on-wire EIP-712
/// field names live in [`PERP_ORDER_INTENT_TYPE`] and MUST remain
/// byte-identical to the Solidity struct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpOrderIntent {
    pub intent_id: B256,
    pub trader: AccountId,
    pub subaccount_id: u32,
    pub market_id: u128,
    /// `0` = buy, `1` = sell. Any other value is rejected by
    /// [`validate_shape`].
    pub side: u8,
    pub size_1e8: u128,
    /// `0` marks a market order (only bounds constrain). Non-zero =
    /// limit (also respect the matching bound).
    pub limit_price_1e8: u128,
    /// Buy-side execution ceiling. Must be non-zero for buy; must be
    /// zero for sell.
    pub max_exec_price_1e8: u128,
    /// Sell-side execution floor. Must be non-zero for sell; must be
    /// zero for buy.
    pub min_exec_price_1e8: u128,
    /// Per-trader monotonic nonce; verified via a nonce-consumed map
    /// at the endpoint.
    pub nonce: u128,
    /// Unix seconds; the endpoint compares against `now_ms() / 1000`.
    pub deadline: u128,
}

/// Returns the runtime-computed EIP-712 typehash for `PerpOrderIntent`.
/// Prefer the constant [`PERP_ORDER_INTENT_TYPEHASH_HEX`] when you need
/// the hex form; this helper exists so the unit test
/// [`tests::perp_order_intent_typehash_matches_locked_value`] can pin
/// the runtime keccak against the pinned constant byte-for-byte.
pub fn perp_order_intent_typehash() -> [u8; 32] {
    keccak256(PERP_ORDER_INTENT_TYPE.as_bytes())
}

/// Compute the EIP-712 struct hash (`keccak256(typeHash || encoded fields)`)
/// for the intent. Field order matches [`PERP_ORDER_INTENT_TYPE`] verbatim.
pub fn perp_order_intent_hash(intent: &PerpOrderIntent) -> Result<[u8; 32]> {
    let trader = parse_evm_address(&intent.trader)?;
    let mut encoded = Vec::with_capacity(32 * 12);
    encoded.extend_from_slice(&perp_order_intent_typehash());
    encoded.extend_from_slice(intent.intent_id.as_slice());
    encoded.extend_from_slice(&encode_address(&trader));
    encoded.extend_from_slice(&encode_u32(intent.subaccount_id));
    encoded.extend_from_slice(&encode_u128(intent.market_id));
    encoded.extend_from_slice(&encode_u8(intent.side));
    encoded.extend_from_slice(&encode_u128(intent.size_1e8));
    encoded.extend_from_slice(&encode_u128(intent.limit_price_1e8));
    encoded.extend_from_slice(&encode_u128(intent.max_exec_price_1e8));
    encoded.extend_from_slice(&encode_u128(intent.min_exec_price_1e8));
    encoded.extend_from_slice(&encode_u128(intent.nonce));
    encoded.extend_from_slice(&encode_u128(intent.deadline));
    Ok(keccak256(&encoded))
}

/// Compute the full EIP-712 v4 digest (`0x19 0x01 || domainSeparator || structHash`).
pub fn perp_order_intent_digest(
    intent: &PerpOrderIntent,
    domain: &PerpTradeDomain,
) -> Result<[u8; 32]> {
    let domain_separator = domain_separator(domain)?;
    let struct_hash = perp_order_intent_hash(intent)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&struct_hash);
    Ok(keccak256(&encoded))
}

/// Verify the trader's signature over the EIP-712 digest and return the
/// recovered address. Returns:
///
/// * [`BackendError::PerpsIntentSignatureInvalid`] on any recovery /
///   parse failure — the caller does not need to know which sub-step
///   failed, so we collapse the space to a single fail-closed error.
/// * [`BackendError::PerpsIntentTraderMismatch`] when the signature
///   recovers to an address DIFFERENT from `intent.trader`.
///
/// The comparison is case-insensitive (mirrors AccountId semantics).
pub fn verify_perp_order_intent(
    intent: &PerpOrderIntent,
    domain: &PerpTradeDomain,
    signature: &str,
) -> Result<AccountId> {
    let digest = perp_order_intent_digest(intent, domain)
        .map_err(|_| BackendError::PerpsIntentSignatureInvalid)?;
    let recovered = recover_eip712_signer(&digest, signature)
        .map_err(|_| BackendError::PerpsIntentSignatureInvalid)?;
    if recovered.0.to_lowercase() != intent.trader.0.to_lowercase() {
        return Err(BackendError::PerpsIntentTraderMismatch);
    }
    Ok(recovered)
}

/// Enforce the pure-Rust side/bound consistency semantics that mirror
/// the Solidity `IntentSideBoundInconsistent` checks. Nonce, deadline,
/// and replay validation are the endpoint's responsibility — those
/// require external state and are deliberately left off this function.
///
/// Semantics:
/// * `side` MUST be either [`PERP_ORDER_INTENT_SIDE_BUY`] or
///   [`PERP_ORDER_INTENT_SIDE_SELL`]; any other value → error.
/// * `size_1e8 != 0`.
/// * `trader` is a well-formed EVM address.
/// * Buy → `max_exec_price_1e8 > 0`, `min_exec_price_1e8 == 0`.
/// * Sell → `min_exec_price_1e8 > 0`, `max_exec_price_1e8 == 0`.
/// * Limit orders (`limit_price_1e8 != 0`) may also set the matching
///   bound; the bound must be at least as generous as the limit
///   (buy → `max >= limit`, sell → `min <= limit`). This mirrors the
///   `SubmitPerpOrderInput` guard so both surfaces stay consistent.
pub fn validate_shape(intent: &PerpOrderIntent) -> Result<()> {
    // Well-formed trader.
    parse_evm_address(&intent.trader)?;
    // Size must be non-zero (mirrors `PerpZeroSize` on the internal
    // submit path).
    if intent.size_1e8 == 0 {
        return Err(BackendError::PerpZeroSize);
    }
    match intent.side {
        PERP_ORDER_INTENT_SIDE_BUY => {
            if intent.max_exec_price_1e8 == 0 {
                return Err(BackendError::PerpsIntentSideBoundInconsistent(
                    "buy intent requires max_exec_price_1e8 > 0".to_string(),
                ));
            }
            if intent.min_exec_price_1e8 != 0 {
                return Err(BackendError::PerpsIntentSideBoundInconsistent(
                    "buy intent must set min_exec_price_1e8 == 0".to_string(),
                ));
            }
            if intent.limit_price_1e8 != 0
                && intent.max_exec_price_1e8 < intent.limit_price_1e8
            {
                return Err(BackendError::PerpsIntentSideBoundInconsistent(format!(
                    "buy max_exec_price_1e8 {} below limit_price_1e8 {}",
                    intent.max_exec_price_1e8, intent.limit_price_1e8
                )));
            }
        }
        PERP_ORDER_INTENT_SIDE_SELL => {
            if intent.min_exec_price_1e8 == 0 {
                return Err(BackendError::PerpsIntentSideBoundInconsistent(
                    "sell intent requires min_exec_price_1e8 > 0".to_string(),
                ));
            }
            if intent.max_exec_price_1e8 != 0 {
                return Err(BackendError::PerpsIntentSideBoundInconsistent(
                    "sell intent must set max_exec_price_1e8 == 0".to_string(),
                ));
            }
            if intent.limit_price_1e8 != 0
                && intent.min_exec_price_1e8 > intent.limit_price_1e8
            {
                return Err(BackendError::PerpsIntentSideBoundInconsistent(format!(
                    "sell min_exec_price_1e8 {} above limit_price_1e8 {}",
                    intent.min_exec_price_1e8, intent.limit_price_1e8
                )));
            }
        }
        other => {
            return Err(BackendError::PerpsIntentSideBoundInconsistent(format!(
                "invalid side sentinel {other} (expected 0=buy or 1=sell)"
            )));
        }
    }
    Ok(())
}

fn domain_separator(domain: &PerpTradeDomain) -> Result<[u8; 32]> {
    let verifying_contract = parse_evm_address(&domain.verifying_contract)?;
    let mut encoded = Vec::with_capacity(160);
    encoded.extend_from_slice(&keccak256(EIP712_DOMAIN_TYPE.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.name.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.version.as_bytes()));
    encoded.extend_from_slice(&encode_u64(domain.chain_id));
    encoded.extend_from_slice(&encode_address(&verifying_contract));
    Ok(keccak256(&encoded))
}

fn encode_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn encode_u8(value: u8) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = value;
    word
}

fn encode_u32(value: u32) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[28..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u128(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::perp_trade::intent_id_to_b256;
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature, SigningKey};

    fn domain() -> PerpTradeDomain {
        PerpTradeDomain::new(
            84532,
            AccountId::new("0x0000000000000000000000000000000000000009"),
        )
    }

    fn signing_key() -> SigningKey {
        // Deterministic dev key — 32 non-zero bytes.
        SigningKey::from_bytes(&[0x11; 32].into()).unwrap()
    }

    fn signer_address(key: &SigningKey) -> AccountId {
        let verifying = key.verifying_key();
        let encoded = verifying.to_encoded_point(false);
        let hash = keccak256(&encoded.as_bytes()[1..]);
        let mut hex = String::from("0x");
        for byte in &hash[12..] {
            hex.push_str(&format!("{byte:02x}"));
        }
        AccountId::new(hex)
    }

    fn sign_digest(key: &SigningKey, digest: &[u8; 32]) -> String {
        let (sig, recovery): (Signature, RecoveryId) = key.sign_prehash(digest).unwrap();
        let mut bytes = [0u8; 65];
        bytes[..64].copy_from_slice(&sig.to_bytes());
        bytes[64] = recovery.to_byte();
        let mut hex = String::from("0x");
        for byte in bytes {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }

    fn buy_intent(trader: AccountId) -> PerpOrderIntent {
        PerpOrderIntent {
            intent_id: intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            trader,
            subaccount_id: 1,
            market_id: 1,
            side: PERP_ORDER_INTENT_SIDE_BUY,
            size_1e8: 100_000_000,
            limit_price_1e8: 0,
            max_exec_price_1e8: 320_000_000_000,
            min_exec_price_1e8: 0,
            nonce: 42,
            deadline: 9_999_999_999,
        }
    }

    fn sell_intent(trader: AccountId) -> PerpOrderIntent {
        PerpOrderIntent {
            intent_id: intent_id_to_b256("00000000-0000-0000-0000-000000000002").unwrap(),
            trader,
            subaccount_id: 1,
            market_id: 1,
            side: PERP_ORDER_INTENT_SIDE_SELL,
            size_1e8: 100_000_000,
            limit_price_1e8: 0,
            max_exec_price_1e8: 0,
            min_exec_price_1e8: 280_000_000_000,
            nonce: 43,
            deadline: 9_999_999_999,
        }
    }

    /// PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 — wire-lock. Freezes
    /// the byte value of [`PERP_ORDER_INTENT_TYPEHASH_HEX`] against the
    /// Solidity source of truth (`PerpMatchingEngine.PERP_ORDER_INTENT_TYPEHASH`).
    /// Any drift here means backend-generated signature previews will
    /// NOT verify on-chain. This test is the earliest place the ripple
    /// surfaces.
    #[test]
    fn perp_order_intent_typehash_matches_locked_value() {
        assert_eq!(
            PERP_ORDER_INTENT_TYPEHASH_HEX,
            "0xeeaf370e4195f568ccb783efe23803dd5bf3c859aef9d0c3e3f211c2da2d5d1c",
        );
        let computed = perp_order_intent_typehash();
        let mut hex = String::from("0x");
        for byte in computed {
            hex.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(hex.as_str(), PERP_ORDER_INTENT_TYPEHASH_HEX);
    }

    #[test]
    fn digest_is_deterministic_and_eip712_shape() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let intent = buy_intent(trader);
        let d = domain();
        let first = perp_order_intent_digest(&intent, &d).unwrap();
        let second = perp_order_intent_digest(&intent, &d).unwrap();
        assert_eq!(first, second);
        // 32 bytes.
        assert_eq!(first.len(), 32);
    }

    #[test]
    fn digest_changes_when_any_field_flips() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let d = domain();
        let base = perp_order_intent_digest(&buy_intent(trader.clone()), &d).unwrap();
        let mut different = buy_intent(trader);
        different.nonce += 1;
        let alt = perp_order_intent_digest(&different, &d).unwrap();
        assert_ne!(base, alt);
    }

    #[test]
    fn verify_perp_order_intent_recovers_trader() {
        let key = signing_key();
        let trader = signer_address(&key);
        let intent = buy_intent(trader.clone());
        let d = domain();
        let digest = perp_order_intent_digest(&intent, &d).unwrap();
        let sig = sign_digest(&key, &digest);
        let recovered = verify_perp_order_intent(&intent, &d, &sig).unwrap();
        assert_eq!(recovered.0.to_lowercase(), trader.0.to_lowercase());
    }

    #[test]
    fn verify_perp_order_intent_rejects_wrong_signer() {
        let key = signing_key();
        let bad_trader = AccountId::new("0x000000000000000000000000000000000000dead");
        let intent = buy_intent(bad_trader);
        let d = domain();
        let digest = perp_order_intent_digest(&intent, &d).unwrap();
        let sig = sign_digest(&key, &digest);
        let err = verify_perp_order_intent(&intent, &d, &sig).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentTraderMismatch));
    }

    #[test]
    fn verify_perp_order_intent_rejects_tampered_intent() {
        let key = signing_key();
        let trader = signer_address(&key);
        let intent = buy_intent(trader);
        let d = domain();
        let digest = perp_order_intent_digest(&intent, &d).unwrap();
        let sig = sign_digest(&key, &digest);
        // Flip the size: signature no longer matches the digest of the
        // tampered intent. Since we recover from the *tampered* digest,
        // the recovered address won't be the trader, so this surfaces
        // as `PerpsIntentTraderMismatch` (not signature-invalid).
        let mut tampered = intent;
        tampered.size_1e8 += 1;
        let err = verify_perp_order_intent(&tampered, &d, &sig).unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsIntentTraderMismatch | BackendError::PerpsIntentSignatureInvalid
        ));
    }

    #[test]
    fn verify_perp_order_intent_rejects_malformed_signature() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let intent = buy_intent(trader);
        let d = domain();
        let err = verify_perp_order_intent(&intent, &d, "0xdead").unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentSignatureInvalid));
    }

    #[test]
    fn validate_shape_accepts_valid_buy_and_sell() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        validate_shape(&buy_intent(trader.clone())).unwrap();
        validate_shape(&sell_intent(trader)).unwrap();
    }

    #[test]
    fn validate_shape_rejects_buy_without_max_bound() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let mut intent = buy_intent(trader);
        intent.max_exec_price_1e8 = 0;
        let err = validate_shape(&intent).unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsIntentSideBoundInconsistent(_)
        ));
    }

    #[test]
    fn validate_shape_rejects_buy_setting_min_bound() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let mut intent = buy_intent(trader);
        intent.min_exec_price_1e8 = 100;
        let err = validate_shape(&intent).unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsIntentSideBoundInconsistent(_)
        ));
    }

    #[test]
    fn validate_shape_rejects_sell_setting_max_bound() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let mut intent = sell_intent(trader);
        intent.max_exec_price_1e8 = 100;
        let err = validate_shape(&intent).unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsIntentSideBoundInconsistent(_)
        ));
    }

    #[test]
    fn validate_shape_rejects_unknown_side() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let mut intent = buy_intent(trader);
        intent.side = 7;
        let err = validate_shape(&intent).unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsIntentSideBoundInconsistent(_)
        ));
    }

    #[test]
    fn validate_shape_rejects_zero_size() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let mut intent = buy_intent(trader);
        intent.size_1e8 = 0;
        let err = validate_shape(&intent).unwrap_err();
        assert!(matches!(err, BackendError::PerpZeroSize));
    }

    #[test]
    fn validate_shape_rejects_buy_limit_above_max_bound() {
        let trader = AccountId::new("0x0000000000000000000000000000000000000001");
        let mut intent = buy_intent(trader);
        intent.limit_price_1e8 = 330_000_000_000; // > max 320_000_000_000
        let err = validate_shape(&intent).unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsIntentSideBoundInconsistent(_)
        ));
    }
}
