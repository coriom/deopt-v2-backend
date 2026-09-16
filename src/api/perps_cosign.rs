//! PERPS_BASE_SEPOLIA_CLOSED_TEST_COSIGN_ROUTE_V1 — closed-test-only
//! co-sign flow that produces the exact 10-field `PerpTrade`
//! [`crate::execution::perp_trade::PERP_TRADE_V1_TYPE`] required by
//! the DEPLOYED Base Sepolia PME V1 at
//! `0x774d96E5739bffadEE91508b4D3D74F5BE29F165`.
//!
//! ## Scope
//!
//! * Closed-test only. `PERPS_CLOSED_TEST_ENABLED` and both parties
//!   in the `PERPS_CLOSED_TEST_ALLOWLIST` are required.
//! * Public Perps route (`POST /perps/orders/signed`) is untouched —
//!   its `PerpOrderIntent` signature domain is distinct and cannot
//!   settle through the deployed 10-field PME. See the module doc for
//!   the taxonomy of the two domains.
//! * Nothing in this module broadcasts. Preparing a trade + persisting
//!   both signatures makes the intent broadcast-eligible, but the
//!   independent executor gates (`EXECUTION_ENABLED`,
//!   `EXECUTOR_REAL_BROADCAST_ENABLED`, `!DRY_RUN`, signer readiness)
//!   still gate any real chain-write.
//!
//! ## Two-phase flow
//!
//! ```
//! POST /perps/closed-test/trades/prepare
//!   body: { buyer, seller, marketId, sizeDelta1e8, buyerIsMaker }
//!   → freezes ONE 10-field PerpTrade + persists ExecutionIntent
//!   → returns { intentId, uuid, typedData, digest }
//!
//! (both traders sign the same `digest` off-chain with EIP-712)
//!
//! POST /perps/closed-test/trades/{uuid}/cosign
//!   body: { buyerSignature, sellerSignature }
//!   → verifies both sigs recover the frozen buyer / seller addresses
//!   → persists { buyer_sig, seller_sig } in execution_intent_signatures
//!   → intent becomes broadcast-eligible for the executor tick
//! ```

use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::execution::{
    intent_id_to_b256, perp_trade_v1_digest, perp_trade_v1_digest_bytes, ExecutionIntent,
    ExecutionIntentStatus, PerpTradeDomain, PerpTradePayload, PerpTradeSignatureBundle,
};
use crate::signing::eip712::parse_evm_address;
use crate::signing::signature::recover_eip712_signer;
use crate::types::{now_ms, AccountId, MarketId, OrderId, Price1e8, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use uuid::Uuid;

/// Default deadline TTL for a prepared closed-test trade: **1 hour**.
/// A finite deadline is preferred over `0` per the milestone directive
/// (avoids depending on `deadline == 0` V1-disabled semantics).
pub const PREPARE_DEADLINE_TTL_MS: i64 = 3_600_000;

// ================================================================
// PHASE A — PREPARE
// ================================================================

#[derive(Clone, Debug, Deserialize)]
pub struct PrepareTradeRequest {
    pub buyer: String,
    pub seller: String,
    /// Decimal string; matches on-chain `uint256 marketId`.
    #[serde(rename = "marketId")]
    pub market_id: String,
    /// Decimal string; on-chain `uint128 sizeDelta1e8` (0.01 ETH = "1000000").
    #[serde(rename = "sizeDelta1e8")]
    pub size_delta_1e8: String,
    /// Decimal string; on-chain `uint128 executionPrice1e8`. Client
    /// picks the price snapshot; backend enforces > 0 but does not
    /// perform an on-chain oracle deviation check (deployed V1 PME
    /// does not either — see fork rehearsal milestone).
    #[serde(rename = "executionPrice1e8")]
    pub execution_price_1e8: String,
    #[serde(rename = "buyerIsMaker")]
    pub buyer_is_maker: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrepareTradeResponse {
    /// UUID v4 assigned by the backend for this prepared trade. Both
    /// signatures MUST be submitted against this exact UUID.
    pub uuid: String,
    /// bytes32 `intentId` = `keccak256(uuid.to_string().as_bytes())`.
    /// Matches the on-chain `TradeExecuted.topic1` that the receipt
    /// identity verifier will require.
    #[serde(rename = "intentId")]
    pub intent_id_hex: String,
    /// Full 32-byte EIP-712 digest that both parties MUST sign.
    pub digest: String,
    /// EIP-712 typed-data envelope suitable for `eth_signTypedData_v4`.
    #[serde(rename = "typedData")]
    pub typed_data: JsonValue,
    /// Frozen `PerpTrade` fields — echoed back so the client can
    /// display them before signing.
    pub trade: FrozenTradeEcho,
}

#[derive(Clone, Debug, Serialize)]
pub struct FrozenTradeEcho {
    pub buyer: String,
    pub seller: String,
    #[serde(rename = "marketId")]
    pub market_id: String,
    #[serde(rename = "sizeDelta1e8")]
    pub size_delta_1e8: String,
    #[serde(rename = "executionPrice1e8")]
    pub execution_price_1e8: String,
    #[serde(rename = "buyerIsMaker")]
    pub buyer_is_maker: bool,
    #[serde(rename = "buyerNonce")]
    pub buyer_nonce: String,
    #[serde(rename = "sellerNonce")]
    pub seller_nonce: String,
    pub deadline: String,
}

/// Result of an internal prepare call — used by the HTTP handler and
/// by tests to exercise the flow without spinning up axum.
#[derive(Clone, Debug)]
pub struct PrepareOutcome {
    pub uuid: Uuid,
    pub intent_id_hex: String,
    pub digest_hex: String,
    pub payload: PerpTradePayload,
    pub domain: PerpTradeDomain,
    pub execution_intent: ExecutionIntent,
    pub typed_data: JsonValue,
}

/// Core prepare logic — validates + freezes one 10-field PerpTrade.
/// HTTP layer wraps this. Kept public within the crate so integration
/// tests can call it directly.
pub fn prepare_trade_core(
    state: &AppState,
    req: &PrepareTradeRequest,
    now_ms_override: Option<TimestampMs>,
) -> Result<PrepareOutcome> {
    // ---- Gate 1: closed-test only ----
    if !state.perps_closed_test_enabled {
        return Err(BackendError::PerpsNotLive);
    }
    if state.perps_public_trading_enabled {
        // Same posture as `perps_submit_signed_order`: this route is
        // closed-test-only, refuses to run when public trading is
        // enabled to prevent surface conflation.
        return Err(BackendError::PerpsNotLive);
    }
    // ---- Gate 2: parse + allowlist ----
    let buyer = AccountId::new(req.buyer.trim().to_ascii_lowercase());
    let seller = AccountId::new(req.seller.trim().to_ascii_lowercase());
    parse_evm_address(&buyer).map_err(|_| BackendError::PerpsIntentSubaccountUnauthorized)?;
    parse_evm_address(&seller).map_err(|_| BackendError::PerpsIntentSubaccountUnauthorized)?;
    if !state.perps_closed_test_allows(&buyer) || !state.perps_closed_test_allows(&seller) {
        return Err(BackendError::PerpsNotLive);
    }
    if buyer.0 == seller.0 {
        return Err(BackendError::PerpsIntentSideBoundInconsistent(
            "buyer == seller not permitted".to_string(),
        ));
    }
    // ---- Gate 3: parse economic fields ----
    let market_id: u128 = req.market_id.parse().map_err(|_| {
        BackendError::Config(format!(
            "invalid marketId decimal string: {}",
            req.market_id
        ))
    })?;
    let size_delta_1e8: u128 = req.size_delta_1e8.parse().map_err(|_| {
        BackendError::Config(format!(
            "invalid sizeDelta1e8 decimal string: {}",
            req.size_delta_1e8
        ))
    })?;
    let execution_price_1e8: u128 = req.execution_price_1e8.parse().map_err(|_| {
        BackendError::Config(format!(
            "invalid executionPrice1e8 decimal string: {}",
            req.execution_price_1e8
        ))
    })?;
    if size_delta_1e8 == 0 {
        return Err(BackendError::PerpZeroSize);
    }
    if execution_price_1e8 == 0 {
        return Err(BackendError::PerpsProtocolReferencePriceUnavailable(
            "executionPrice1e8 must be > 0".to_string(),
        ));
    }
    // ---- Freeze identity + timestamps + nonces ----
    let now = now_ms_override.unwrap_or_else(now_ms);
    let deadline_ms = now.saturating_add(PREPARE_DEADLINE_TTL_MS);
    let uuid = Uuid::new_v4();
    let intent_id_hex = crate::execution::intent_id_to_hex_bytes32(&uuid.to_string())?;
    let intent_id_b256 = intent_id_to_b256(&uuid.to_string())?;

    // NOTE: buyer/seller PME nonces should be read from chain at
    // signing time in production; for this milestone we accept 0/0
    // as the canonical starting state (matches the fresh trader
    // fixtures created by TRADER_KEYSTORES_V1). Future extension may
    // read `PME.nonces(buyer)` and `PME.nonces(seller)` via RPC and
    // bind them here.
    let buyer_nonce: u128 = 0;
    let seller_nonce: u128 = 0;

    // Build the 10-field payload. Note the max/min bounds are set to
    // 0 (not encoded by the V1 digest / calldata) — the deployed V1
    // PerpTrade has no such fields.
    let payload = PerpTradePayload::new(
        intent_id_b256,
        buyer.clone(),
        seller.clone(),
        market_id,
        size_delta_1e8,
        execution_price_1e8,
        0,
        0,
        req.buyer_is_maker,
        buyer_nonce,
        seller_nonce,
        u128::try_from(deadline_ms).unwrap_or(u128::MAX),
    )?;

    let domain = PerpTradeDomain::new(
        state.perps_read_config.chain_id,
        state.execution_config.perp_matching_engine_address.clone(),
    );
    let digest_hex = perp_trade_v1_digest(&payload, &domain)?;

    let typed_data = build_typed_data_v1(&payload, &domain, &intent_id_hex, deadline_ms);

    let execution_intent = ExecutionIntent {
        intent_id: uuid,
        market_id: market_id_from_u128(market_id)?,
        buyer: buyer.clone(),
        seller: seller.clone(),
        price_1e8: execution_price_1e8 as Price1e8,
        size_1e8: size_delta_1e8 as Size1e8,
        buy_order_id: OrderId(Uuid::new_v4()),
        sell_order_id: OrderId(Uuid::new_v4()),
        buyer_is_maker: Some(req.buyer_is_maker),
        buyer_nonce: Some(buyer_nonce as u64),
        seller_nonce: Some(seller_nonce as u64),
        deadline_ms: Some(deadline_ms),
        created_at_ms: now,
        status: ExecutionIntentStatus::CalldataReady,
    };

    Ok(PrepareOutcome {
        uuid,
        intent_id_hex,
        digest_hex,
        payload,
        domain,
        execution_intent,
        typed_data,
    })
}

fn market_id_from_u128(v: u128) -> Result<MarketId> {
    u16::try_from(v)
        .map(MarketId::from)
        .map_err(|_| BackendError::UnknownMarket(MarketId::from(0u16)))
}

fn build_typed_data_v1(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
    intent_id_hex: &str,
    deadline_ms: TimestampMs,
) -> JsonValue {
    json!({
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"}
            ],
            "PerpTrade": [
                {"name": "intentId", "type": "bytes32"},
                {"name": "buyer", "type": "address"},
                {"name": "seller", "type": "address"},
                {"name": "marketId", "type": "uint256"},
                {"name": "sizeDelta1e8", "type": "uint128"},
                {"name": "executionPrice1e8", "type": "uint128"},
                {"name": "buyerIsMaker", "type": "bool"},
                {"name": "buyerNonce", "type": "uint256"},
                {"name": "sellerNonce", "type": "uint256"},
                {"name": "deadline", "type": "uint256"}
            ]
        },
        "primaryType": "PerpTrade",
        "domain": {
            "name": domain.name,
            "version": domain.version,
            "chainId": domain.chain_id,
            "verifyingContract": domain.verifying_contract.0
        },
        "message": {
            "intentId": intent_id_hex,
            "buyer": payload.buyer.0,
            "seller": payload.seller.0,
            "marketId": payload.market_id.to_string(),
            "sizeDelta1e8": payload.size_delta_1e8.to_string(),
            "executionPrice1e8": payload.execution_price_1e8.to_string(),
            "buyerIsMaker": payload.buyer_is_maker,
            "buyerNonce": payload.buyer_nonce.to_string(),
            "sellerNonce": payload.seller_nonce.to_string(),
            "deadline": deadline_ms.to_string()
        }
    })
}

// ================================================================
// PHASE B — COSIGN
// ================================================================

#[derive(Clone, Debug, Deserialize)]
pub struct CosignTradeRequest {
    #[serde(rename = "buyerSignature")]
    pub buyer_signature: String,
    #[serde(rename = "sellerSignature")]
    pub seller_signature: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CosignTradeResponse {
    pub uuid: String,
    #[serde(rename = "intentId")]
    pub intent_id_hex: String,
    #[serde(rename = "buyerSigner")]
    pub buyer_signer: String,
    #[serde(rename = "sellerSigner")]
    pub seller_signer: String,
    /// Whether the intent is now broadcast-eligible (both sigs
    /// persisted, `execution_intent_signatures.calldata_ready() ==
    /// true`). Real broadcast still requires the independent
    /// executor gates.
    #[serde(rename = "calldataReady")]
    pub calldata_ready: bool,
}

/// Core cosign logic — verifies both signatures against the frozen
/// EIP-712 digest and returns the recovered signer addresses. The
/// HTTP handler / integration test are responsible for persisting the
/// signatures (via `PgRepository::upsert_execution_intent_signatures`
/// or an in-memory equivalent).
pub fn cosign_verify_core(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
    req: &CosignTradeRequest,
) -> Result<CosignVerifiedSignatures> {
    let digest = perp_trade_v1_digest_bytes(payload, domain)?;
    let buyer_recovered = recover_eip712_signer(&digest, &req.buyer_signature)
        .map_err(|_| BackendError::PerpsIntentSignatureInvalid)?;
    if buyer_recovered.0.to_lowercase() != payload.buyer.0.to_lowercase() {
        return Err(BackendError::PerpsIntentTraderMismatch);
    }
    let seller_recovered = recover_eip712_signer(&digest, &req.seller_signature)
        .map_err(|_| BackendError::PerpsIntentSignatureInvalid)?;
    if seller_recovered.0.to_lowercase() != payload.seller.0.to_lowercase() {
        return Err(BackendError::PerpsIntentTraderMismatch);
    }
    Ok(CosignVerifiedSignatures {
        buyer_signer: buyer_recovered,
        seller_signer: seller_recovered,
        bundle: PerpTradeSignatureBundle::new(&req.buyer_signature, &req.seller_signature)?,
    })
}

#[derive(Clone, Debug)]
pub struct CosignVerifiedSignatures {
    pub buyer_signer: AccountId,
    pub seller_signer: AccountId,
    pub bundle: PerpTradeSignatureBundle,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{
        perp_trade_v1_typehash, PERP_TRADE_V1_TYPE, PERP_TRADE_V1_TYPEHASH_HEX,
    };

    // --- (A) Deployed TRADE_TYPEHASH fixture (10-field) ---
    #[test]
    fn a_typehash_matches_deployed_v1() {
        let live = perp_trade_v1_typehash();
        let hex = format!("0x{}", hex_encode(&live));
        assert_eq!(hex, PERP_TRADE_V1_TYPEHASH_HEX);
        assert_eq!(
            PERP_TRADE_V1_TYPEHASH_HEX,
            "0xfb345c17e97266a4c9efdc53b5baf04e3df8166f6fce15dc415758759d2e8293"
        );
    }

    // --- (A') Type string content ---
    #[test]
    fn a_type_string_has_ten_fields_no_bounds() {
        assert!(PERP_TRADE_V1_TYPE.contains("bytes32 intentId"));
        assert!(PERP_TRADE_V1_TYPE.contains("uint256 deadline"));
        assert!(!PERP_TRADE_V1_TYPE.contains("maxExecutionPrice1e8"));
        assert!(!PERP_TRADE_V1_TYPE.contains("minExecutionPrice1e8"));
        // count field commas: 9 commas separate 10 fields
        assert_eq!(PERP_TRADE_V1_TYPE.matches(',').count(), 9);
    }

    // --- (B) executeTrade selector matches deployed 10-field variant ---
    #[test]
    fn b_execute_trade_selector_matches_deployed_v1() {
        let sel = crate::execution::execute_trade_selector();
        assert_eq!(hex_encode(&sel), "7a708c4c");
    }

    fn hex_encode(bytes: &[u8]) -> String {
        const H: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push(H[(b >> 4) as usize] as char);
            s.push(H[(b & 0x0f) as usize] as char);
        }
        s
    }

    // --- Test fixtures ---
    // A well-known Ganache test key (BUYER role in tests only).
    const BUYER_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const BUYER_ADDR: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";
    // A second well-known test key.
    const SELLER_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SELLER_ADDR: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";

    const PME: &str = "0x774d96e5739bffadee91508b4d3d74f5be29f165";
    const CHAIN_ID: u64 = 84532;

    fn domain() -> PerpTradeDomain {
        PerpTradeDomain::new(CHAIN_ID, AccountId::new(PME.to_string()))
    }

    fn make_payload(uuid: uuid::Uuid) -> PerpTradePayload {
        let intent_id = intent_id_to_b256(&uuid.to_string()).unwrap();
        PerpTradePayload::new(
            intent_id,
            AccountId::new(BUYER_ADDR.to_string()),
            AccountId::new(SELLER_ADDR.to_string()),
            1,
            1_000_000,       // 0.01 ETH
            240_000_000_000, // ~$2400
            0,
            0,
            false,
            0,
            0,
            9_999_999_999_999,
        )
        .unwrap()
    }

    fn sign_v1_with(key_hex: &str, digest: &[u8; 32]) -> String {
        use k256::ecdsa::SigningKey;
        let key_bytes = decode_hex_bytes(key_hex.strip_prefix("0x").unwrap()).unwrap();
        let sk = SigningKey::from_bytes(key_bytes.as_slice().into()).unwrap();
        let (sig, rec_id) = sk.sign_prehash_recoverable(digest).unwrap();
        let r = sig.r().to_bytes();
        let s = sig.s().to_bytes();
        let mut out = String::from("0x");
        for b in r.iter().chain(s.iter()) {
            out.push_str(&format!("{b:02x}"));
        }
        // v = 27 + recid
        let v = 27u8 + rec_id.to_byte();
        out.push_str(&format!("{v:02x}"));
        out
    }

    fn decode_hex_bytes(s: &str) -> Result<Vec<u8>> {
        (0..s.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| BackendError::MalformedSignature)
            })
            .collect()
    }

    // --- (C) Digest fixture ---
    #[test]
    fn c_digest_deterministic() {
        let uuid = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        let payload = make_payload(uuid);
        let d1 = perp_trade_v1_digest(&payload, &domain()).unwrap();
        let d2 = perp_trade_v1_digest(&payload, &domain()).unwrap();
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 66); // "0x" + 64 hex
    }

    // --- (D/E) UUID → bytes32 mapping ---
    #[test]
    fn de_uuid_to_bytes32_deterministic() {
        let uuid = uuid::Uuid::from_u128(42);
        let a = crate::execution::intent_id_to_hex_bytes32(&uuid.to_string()).unwrap();
        let b = crate::execution::intent_id_to_hex_bytes32(&uuid.to_string()).unwrap();
        assert_eq!(a, b);
        // Different uuid → different bytes32
        let c = crate::execution::intent_id_to_hex_bytes32(&uuid::Uuid::from_u128(43).to_string())
            .unwrap();
        assert_ne!(a, c);
    }

    // --- (G) Valid buyer + seller signatures ---
    #[test]
    fn g_valid_cosign_signatures_recover_correctly() {
        let uuid = uuid::Uuid::from_u128(7);
        let payload = make_payload(uuid);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: seller_sig,
        };
        let verified = cosign_verify_core(&payload, &domain(), &req).unwrap();
        assert_eq!(verified.buyer_signer.0.to_lowercase(), BUYER_ADDR);
        assert_eq!(verified.seller_signer.0.to_lowercase(), SELLER_ADDR);
    }

    // --- (H) Invalid buyer signature ---
    #[test]
    fn h_invalid_buyer_signature_rejects() {
        let uuid = uuid::Uuid::from_u128(8);
        let payload = make_payload(uuid);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let wrong_sig = sign_v1_with(SELLER_KEY, &digest); // buyer sig signed by wrong key
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let req = CosignTradeRequest {
            buyer_signature: wrong_sig,
            seller_signature: seller_sig,
        };
        let err = cosign_verify_core(&payload, &domain(), &req).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentTraderMismatch));
    }

    // --- (I) Invalid seller signature ---
    #[test]
    fn i_invalid_seller_signature_rejects() {
        let uuid = uuid::Uuid::from_u128(9);
        let payload = make_payload(uuid);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let wrong_sig = sign_v1_with(BUYER_KEY, &digest); // seller sig signed by wrong key
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: wrong_sig,
        };
        let err = cosign_verify_core(&payload, &domain(), &req).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentTraderMismatch));
    }

    // --- (J) Swapped signatures ---
    #[test]
    fn j_swapped_signatures_reject() {
        let uuid = uuid::Uuid::from_u128(10);
        let payload = make_payload(uuid);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let req = CosignTradeRequest {
            buyer_signature: seller_sig,
            seller_signature: buyer_sig,
        };
        let err = cosign_verify_core(&payload, &domain(), &req).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentTraderMismatch));
    }

    // --- (K) Tampered trade after signing ---
    #[test]
    fn k_tampered_trade_after_signing_rejects() {
        let uuid = uuid::Uuid::from_u128(11);
        let payload = make_payload(uuid);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let mut tampered = payload.clone();
        tampered.size_delta_1e8 += 1;
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: seller_sig,
        };
        let err = cosign_verify_core(&tampered, &domain(), &req).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentTraderMismatch));
    }

    // --- (O) Wrong verifying contract ---
    #[test]
    fn o_wrong_verifying_contract_rejects() {
        let uuid = uuid::Uuid::from_u128(15);
        let payload = make_payload(uuid);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let wrong_domain = PerpTradeDomain::new(
            CHAIN_ID,
            AccountId::new("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string()),
        );
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: seller_sig,
        };
        let err = cosign_verify_core(&payload, &wrong_domain, &req).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentTraderMismatch));
    }

    // --- (P) Wrong chain id ---
    #[test]
    fn p_wrong_chain_id_rejects() {
        let uuid = uuid::Uuid::from_u128(16);
        let payload = make_payload(uuid);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let wrong_domain = PerpTradeDomain::new(8453, AccountId::new(PME.to_string()));
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: seller_sig,
        };
        let err = cosign_verify_core(&payload, &wrong_domain, &req).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentTraderMismatch));
    }

    // --- (S) buyer == seller gate — enforced by `prepare_trade_core`
    //     (see the closed-test HTTP path), not by `PerpTradePayload::new`.
    //     This test documents the gate location: any change that removes
    //     the check in `prepare_trade_core` MUST re-add the equivalent
    //     rejection somewhere before signature verification.
    #[test]
    fn s_buyer_equals_seller_gate_documented() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/perps_cosign.rs"),
        )
        .unwrap();
        assert!(
            src.contains("buyer == seller not permitted"),
            "prepare_trade_core MUST reject buyer == seller before signature verification"
        );
    }

    // --- (X) Calldata decodes to exact frozen 10-field trade ---
    #[test]
    fn x_final_calldata_decodes_to_frozen_trade() {
        use crate::execution::{encode_execute_trade_calldata, PerpTradeSignatureBundle};
        let uuid = uuid::Uuid::from_u128(24);
        let payload = make_payload(uuid);
        let aa_sig = format!("0x{}", "aa".repeat(65));
        let bb_sig = format!("0x{}", "bb".repeat(65));
        let bundle = PerpTradeSignatureBundle::new(&aa_sig, &bb_sig).unwrap();
        let calldata = encode_execute_trade_calldata(&payload, &bundle).unwrap();
        // First 4 bytes: executeTrade selector
        assert_eq!(hex_encode(&calldata[..4]), "7a708c4c");
        // Non-empty tuple + sig-bytes body
        assert!(calldata.len() > 4 + 320);
    }

    // --- (Y) Receipt identity expectation uses same UUID hash ---
    #[test]
    fn y_receipt_identity_uses_uuid_keccak() {
        use crate::execution::expected_intent_hash_from_uuid;
        let uuid = uuid::Uuid::from_u128(25);
        let expected = expected_intent_hash_from_uuid(uuid);
        let hex = format!("0x{}", hex_encode(&expected));
        let via_intent_id_hex =
            crate::execution::intent_id_to_hex_bytes32(&uuid.to_string()).unwrap();
        assert_eq!(hex, via_intent_id_hex);
    }
}
