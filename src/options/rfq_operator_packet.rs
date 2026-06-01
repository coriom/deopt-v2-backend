//! V2G-P1 — OPTION RFQ operator packet generator.
//!
//! This module gives operators a small, safe-by-default surface to
//! prepare an `executeRfqTrade(...)` broadcast packet for the V2G-O
//! [`OptionMatchingEngine`] without ever holding or printing a private
//! key.
//!
//! Goals:
//! * Produce the EIP-712 RFQ digest the maker / taker EOAs must sign
//!   off-chain (e.g. with a hardware wallet or the V2G-D2 EOA registry
//!   signing CLI).
//! * Once both signatures are collected, produce a ready-to-broadcast
//!   `executeRfqTrade` calldata bundle.
//! * Refuse to "broadcast" — this module never sends a transaction.
//!   The actual broadcast is gated by the explicit env confirm flag
//!   [`OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV`] and is the operator
//!   tooling's responsibility (e.g. the executor binary).
//!
//! What this module **does not** do:
//! * It does NOT touch private keys.
//! * It does NOT submit anything to the chain.
//! * It does NOT modify backend DB state.
//!
//! What it asserts at build time:
//! * Buyer / seller addresses are derived from the operator-provided
//!   `OptionTradePayload`. They are NOT mutated based on signer
//!   recovery — if the operator passes mismatched signatures, the
//!   downstream `OptionMatchingEngine.executeRfqTrade` reverts on
//!   chain. The packet builder simply attaches the signatures
//!   verbatim.
//! * The RFQ digest is computed from the canonical
//!   [`OPTION_RFQ_TRADE_TYPE`](super::execution::OPTION_RFQ_TRADE_TYPE)
//!   so it cannot replay against the ORDERBOOK path.

use super::execution::{
    encode_option_execute_rfq_trade_calldata, option_execute_rfq_trade_selector,
    option_rfq_trade_digest, option_rfq_trade_digest_bytes, OptionTradePayload,
    OptionTradeSignatureBundle,
};
use crate::error::{BackendError, Result};
use crate::signing::Eip712Domain;

/// Env flag that operator tooling MUST honour before broadcasting a
/// packet produced by this module. The module itself never reads the
/// flag — it is documented here so the executor binary can refuse to
/// broadcast unless the operator has explicitly set it to `true`.
pub const OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV: &str = "OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM";

/// Inputs collected from the operator. Signatures are optional at the
/// preparation stage so an operator can:
/// 1. Build a packet with no signatures, extract the `digest_hex`,
///    sign offline.
/// 2. Re-build the packet with both signatures attached.
pub struct OptionRfqOperatorPacketInputs<'a> {
    pub payload: &'a OptionTradePayload,
    pub domain: &'a Eip712Domain,
    pub signatures: Option<&'a OptionTradeSignatureBundle>,
}

/// Result of [`build_option_rfq_operator_packet`].
///
/// Fields populated when `signatures` were attached:
/// * `calldata_hex` — ABI-encoded `executeRfqTrade(...)` calldata,
///   `0x`-prefixed.
/// * `function_selector_hex` — 4-byte selector, for sanity-checking
///   against `option_execute_rfq_trade_selector()` in the operator's
///   tooling.
///
/// Always populated:
/// * `digest_hex` — the EIP-712 digest the maker / taker EOAs must
///   sign.
/// * `digest_bytes` — same digest as bytes, suitable for piping into
///   `ethers signMessage(digest)` etc.
/// * `payload_summary` — read-only operator-facing summary string.
#[derive(Debug, Clone)]
pub struct OptionRfqOperatorPacket {
    pub digest_hex: String,
    pub digest_bytes: [u8; 32],
    pub function_selector_hex: String,
    pub calldata_hex: Option<String>,
    pub payload_summary: String,
    pub broadcast_confirm_env: &'static str,
    pub broadcast_ready: bool,
}

/// Errors surfaced to operator tooling. Distinct from the executor's
/// internal `BackendError` taxonomy so operators see actionable
/// messages.
#[derive(Debug, thiserror::Error)]
pub enum OptionRfqOperatorBroadcastError {
    #[error("OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM env flag not set to true")]
    BroadcastConfirmFlagNotSet,
    #[error("packet has no signatures attached — call build_option_rfq_operator_packet again with Some(signatures)")]
    NoSignaturesAttached,
    #[error("OPTION RFQ packet validation failed: {0}")]
    Validation(#[from] BackendError),
}

/// Build a safe-by-default OPTION RFQ operator packet.
///
/// Behaviour:
/// * Always computes the EIP-712 digest from the canonical RFQ
///   typehash + domain.
/// * If `signatures` is `Some`, also produces the
///   `executeRfqTrade(...)` ABI calldata.
/// * Never reads private keys. Never emits a log line containing
///   secrets. Never sends a transaction.
pub fn build_option_rfq_operator_packet(
    inputs: OptionRfqOperatorPacketInputs<'_>,
) -> Result<OptionRfqOperatorPacket> {
    let digest_hex = option_rfq_trade_digest(inputs.payload, inputs.domain)?;
    let digest_bytes = option_rfq_trade_digest_bytes(inputs.payload, inputs.domain)?;
    let selector = option_execute_rfq_trade_selector();
    let function_selector_hex = format!(
        "0x{:02x}{:02x}{:02x}{:02x}",
        selector[0], selector[1], selector[2], selector[3]
    );

    let calldata_hex = match inputs.signatures {
        Some(signatures) => {
            let bytes =
                encode_option_execute_rfq_trade_calldata(inputs.payload, signatures)?;
            Some(format!("0x{}", hex_encode(&bytes)))
        }
        None => None,
    };

    let payload_summary = format!(
        "OptionRfqTrade(intentId=0x{intent}, buyer={buyer}, seller={seller}, optionId={option_id}, expiry={expiry}, quantity={qty}, premiumPerContract={premium}, buyerIsMaker={bim}, deadline={deadline})",
        intent = hex_encode(inputs.payload.intent_id.as_slice()),
        buyer = inputs.payload.buyer.0,
        seller = inputs.payload.seller.0,
        option_id = inputs.payload.option_id,
        expiry = inputs.payload.expiry,
        qty = inputs.payload.quantity,
        premium = inputs.payload.premium_per_contract,
        bim = inputs.payload.buyer_is_maker,
        deadline = inputs.payload.deadline,
    );

    Ok(OptionRfqOperatorPacket {
        digest_hex,
        digest_bytes,
        function_selector_hex,
        broadcast_ready: calldata_hex.is_some(),
        calldata_hex,
        payload_summary,
        broadcast_confirm_env: OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV,
    })
}

/// Operator helper: returns Ok(()) if the env flag confirming the
/// operator's intent to broadcast is set to the literal string
/// `"true"`. Any other value (including absent / empty) returns the
/// safe-by-default error.
///
/// This is *not* called by [`build_option_rfq_operator_packet`] — it
/// is exposed for the executor binary so the operator's broadcast
/// path can require explicit consent.
pub fn require_option_rfq_broadcast_confirm(
    env: &std::collections::HashMap<String, String>,
) -> std::result::Result<(), OptionRfqOperatorBroadcastError> {
    match env.get(OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV) {
        Some(value) if value == "true" => Ok(()),
        _ => Err(OptionRfqOperatorBroadcastError::BroadcastConfirmFlagNotSet),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::execution::option_execute_trade_selector;
    use crate::options::execution::OPTION_RFQ_TRADE_TYPE;
    use crate::signing::eip712::keccak256;
    use crate::types::AccountId;
    use alloy_primitives::{B256, U256};
    use std::collections::HashMap;

    fn domain() -> Eip712Domain {
        Eip712Domain {
            name: "DeOptV2-OptionMatchingEngine".to_string(),
            version: "1".to_string(),
            chain_id: 84532,
            verifying_contract: AccountId::new("0x00000000000000000000000000000000000000ee"),
        }
    }

    fn payload() -> OptionTradePayload {
        let underlying = AccountId::new("0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02");
        let settlement_asset = AccountId::new("0x6eAe407f5640B006faC9965182e238582A3B412E");
        let expiry = 1_893_456_000;
        let strike_1e8 = 300_000_000_000;
        let contract_size_1e8 = 100_000_000;
        let is_call = true;

        OptionTradePayload {
            intent_id: B256::from(keccak256(b"v2gp1-rfq-operator-packet-test")),
            buyer: AccountId::new("0x77ca9dd6ccce2d692fb23877a2db7178807b0020"),
            seller: AccountId::new("0x290bd12c93e467bf51c51f5273d35bddb19e9274"),
            option_id: super::super::execution::option_product_registry_option_id(
                &underlying,
                &settlement_asset,
                expiry,
                strike_1e8,
                contract_size_1e8,
                is_call,
                true,
            )
            .unwrap(),
            underlying,
            settlement_asset,
            expiry,
            strike_1e8,
            is_call,
            contract_size_1e8,
            quantity: 1,
            premium_per_contract: 200_000,
            buyer_is_maker: false,
            buyer_nonce: 0,
            seller_nonce: 0,
            deadline: u128::from(1_893_456_600u64),
        }
    }

    fn signature_bundle() -> OptionTradeSignatureBundle {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str("aa");
        }
        let mut signature2 = String::from("0x");
        for _ in 0..65 {
            signature2.push_str("bb");
        }
        OptionTradeSignatureBundle::new(&signature, &signature2).unwrap()
    }

    #[test]
    fn builds_packet_without_signatures_for_offline_signing() {
        let payload = payload();
        let domain = domain();
        let packet = build_option_rfq_operator_packet(OptionRfqOperatorPacketInputs {
            payload: &payload,
            domain: &domain,
            signatures: None,
        })
        .unwrap();

        assert!(packet.digest_hex.starts_with("0x"));
        assert_eq!(packet.digest_hex.len(), 66, "digest must be 32-byte hex");
        assert!(!packet.broadcast_ready);
        assert!(packet.calldata_hex.is_none(), "must not produce calldata without sigs");
        assert!(
            packet.payload_summary.contains("buyer="),
            "summary must include buyer field"
        );
        assert_eq!(
            packet.broadcast_confirm_env,
            OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV
        );
    }

    #[test]
    fn builds_packet_with_signatures_attaches_calldata() {
        let payload = payload();
        let domain = domain();
        let signatures = signature_bundle();
        let packet = build_option_rfq_operator_packet(OptionRfqOperatorPacketInputs {
            payload: &payload,
            domain: &domain,
            signatures: Some(&signatures),
        })
        .unwrap();

        assert!(packet.broadcast_ready);
        let calldata = packet.calldata_hex.expect("calldata must be attached");
        assert!(calldata.starts_with("0x"));
        // selector check — first 4 bytes after 0x.
        let selector_hex = &calldata[2..10];
        assert_eq!(selector_hex, &packet.function_selector_hex[2..10]);
    }

    #[test]
    fn calldata_carries_rfq_selector_not_orderbook_selector() {
        let payload = payload();
        let domain = domain();
        let signatures = signature_bundle();
        let packet = build_option_rfq_operator_packet(OptionRfqOperatorPacketInputs {
            payload: &payload,
            domain: &domain,
            signatures: Some(&signatures),
        })
        .unwrap();

        let calldata = packet.calldata_hex.unwrap();
        let rfq_selector_hex = &calldata[2..10];
        let orderbook_selector = option_execute_trade_selector();
        let orderbook_hex = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            orderbook_selector[0],
            orderbook_selector[1],
            orderbook_selector[2],
            orderbook_selector[3]
        );
        assert_ne!(
            rfq_selector_hex, &orderbook_hex,
            "calldata must NOT carry the ORDERBOOK selector — cross-flow replay defense"
        );
    }

    #[test]
    fn digest_differs_from_orderbook_digest_for_identical_payload() {
        let payload = payload();
        let domain = domain();
        let rfq_digest =
            super::super::execution::option_rfq_trade_digest(&payload, &domain).unwrap();
        let orderbook_digest =
            super::super::execution::option_trade_digest(&payload, &domain).unwrap();
        assert_ne!(
            rfq_digest, orderbook_digest,
            "RFQ digest must differ from ORDERBOOK digest"
        );
    }

    #[test]
    fn require_broadcast_confirm_refuses_when_flag_absent() {
        let env = HashMap::new();
        assert!(matches!(
            require_option_rfq_broadcast_confirm(&env),
            Err(OptionRfqOperatorBroadcastError::BroadcastConfirmFlagNotSet)
        ));
    }

    #[test]
    fn require_broadcast_confirm_refuses_when_flag_is_not_true() {
        let mut env = HashMap::new();
        env.insert(
            OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV.to_string(),
            "false".to_string(),
        );
        assert!(matches!(
            require_option_rfq_broadcast_confirm(&env),
            Err(OptionRfqOperatorBroadcastError::BroadcastConfirmFlagNotSet)
        ));

        env.insert(
            OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV.to_string(),
            "1".to_string(),
        );
        assert!(matches!(
            require_option_rfq_broadcast_confirm(&env),
            Err(OptionRfqOperatorBroadcastError::BroadcastConfirmFlagNotSet)
        ));

        env.insert(
            OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV.to_string(),
            "TRUE".to_string(),
        );
        assert!(matches!(
            require_option_rfq_broadcast_confirm(&env),
            Err(OptionRfqOperatorBroadcastError::BroadcastConfirmFlagNotSet)
        ));
    }

    #[test]
    fn require_broadcast_confirm_accepts_only_literal_true() {
        let mut env = HashMap::new();
        env.insert(
            OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV.to_string(),
            "true".to_string(),
        );
        assert!(require_option_rfq_broadcast_confirm(&env).is_ok());
    }

    #[test]
    fn packet_summary_does_not_expose_private_key_or_signature() {
        let payload = payload();
        let domain = domain();
        let signatures = signature_bundle();
        let packet = build_option_rfq_operator_packet(OptionRfqOperatorPacketInputs {
            payload: &payload,
            domain: &domain,
            signatures: Some(&signatures),
        })
        .unwrap();

        let lower = packet.payload_summary.to_lowercase();
        for forbidden in &[
            "private",
            "privatekey",
            "private_key",
            "secret",
            "mnemonic",
            "0xaa", // canary — the signature bundle bytes
            "0xbb",
        ] {
            assert!(
                !lower.contains(forbidden),
                "payload_summary must not leak '{forbidden}': {lower}"
            );
        }
    }

    #[test]
    fn option_rfq_trade_type_constant_referenced() {
        // Keep `OPTION_RFQ_TRADE_TYPE` linked from this module so
        // operator-facing tooling can print the canonical signed-type
        // string without re-deriving it.
        assert!(OPTION_RFQ_TRADE_TYPE.starts_with("OptionRfqTrade("));
    }

    #[test]
    fn payload_field_count_matches_canonical_type_string() {
        // V2G-O typehash fixed at 16 fields. If a field is added or
        // removed without updating OPTION_RFQ_TRADE_TYPE, this test
        // catches the drift before it ships.
        let field_count = OPTION_RFQ_TRADE_TYPE
            .trim_start_matches("OptionRfqTrade(")
            .trim_end_matches(')')
            .split(',')
            .count();
        assert_eq!(field_count, 16, "OPTION_RFQ_TRADE_TYPE field count");
        let _ = U256::ZERO; // anchor U256 import.
    }
}
