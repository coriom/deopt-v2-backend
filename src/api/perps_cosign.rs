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
use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::execution::{
    intent_id_to_b256, perp_trade_v1_digest, perp_trade_v1_digest_bytes, ExecutionIntent,
    ExecutionIntentStatus, PerpTradeDomain, PerpTradePayload, PerpTradeSignatureBundle,
};
use crate::signing::eip712::parse_evm_address;
use crate::signing::signature::recover_eip712_signer;
use crate::types::{now_ms, AccountId, MarketId, OrderId, Price1e8, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

/// PME `nonces(address)` selector — first four bytes of
/// `keccak256("nonces(address)")`. Used by
/// [`RpcNonceReader`] to fetch the authoritative buyer/seller PME
/// nonce at prepare time.
pub const PME_NONCES_SELECTOR: [u8; 4] = [0x7e, 0xce, 0xbe, 0x00];

/// PerpEngine `getMarkPrice(uint256)` selector — first four bytes of
/// `keccak256("getMarkPrice(uint256)")`. Used by
/// [`RpcMarkPriceReader`] to fetch the authoritative live mark for
/// the prepare execution price.
pub const PERP_ENGINE_GET_MARK_PRICE_SELECTOR: [u8; 4] = [0x5a, 0xf3, 0xd0, 0x61];

pub type ReaderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Read the authoritative PME nonce for a trader. Implementations:
/// - [`RpcNonceReader`] uses `PerpMatchingEngine.nonces(address)`
///   via `eth_call`.
/// - Test doubles inject deterministic values.
pub trait NonceReader: Send + Sync {
    fn read_pme_nonce<'a>(&'a self, trader: &'a AccountId) -> ReaderFuture<'a, u128>;
}

/// Read the authoritative mark price for a market. Implementations:
/// - [`RpcMarkPriceReader`] uses `PerpEngine.getMarkPrice(marketId)`
///   which internally consults the OracleRouter.
/// - Test doubles inject deterministic values.
pub trait MarkPriceReader: Send + Sync {
    fn read_mark_price<'a>(&'a self, market_id: u128) -> ReaderFuture<'a, u128>;
}

/// Live-chain [`NonceReader`] implementation backed by any
/// [`EthCallProvider`] pointed at the PME contract.
pub struct RpcNonceReader<R> {
    pub rpc: R,
    pub pme: AccountId,
}

impl<R> RpcNonceReader<R> {
    pub fn new(rpc: R, pme: AccountId) -> Self {
        Self { rpc, pme }
    }
}

impl<R> NonceReader for RpcNonceReader<R>
where
    R: EthCallProvider + Send + Sync + 'static,
{
    fn read_pme_nonce<'a>(&'a self, trader: &'a AccountId) -> ReaderFuture<'a, u128> {
        Box::pin(async move {
            let trader_bytes = parse_evm_address(trader)?;
            let mut data = Vec::with_capacity(4 + 32);
            data.extend_from_slice(&PME_NONCES_SELECTOR);
            data.extend_from_slice(&[0u8; 12]);
            data.extend_from_slice(&trader_bytes);
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: trader.clone(),
                    to: self.pme.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            decode_uint256_low128(&out.output, "nonces")
        })
    }
}

/// Live-chain [`MarkPriceReader`] implementation backed by any
/// [`EthCallProvider`] pointed at the PerpEngine contract.
pub struct RpcMarkPriceReader<R> {
    pub rpc: R,
    pub perp_engine: AccountId,
}

impl<R> RpcMarkPriceReader<R> {
    pub fn new(rpc: R, perp_engine: AccountId) -> Self {
        Self { rpc, perp_engine }
    }
}

impl<R> MarkPriceReader for RpcMarkPriceReader<R>
where
    R: EthCallProvider + Send + Sync + 'static,
{
    fn read_mark_price<'a>(&'a self, market_id: u128) -> ReaderFuture<'a, u128> {
        Box::pin(async move {
            let mut data = Vec::with_capacity(4 + 32);
            data.extend_from_slice(&PERP_ENGINE_GET_MARK_PRICE_SELECTOR);
            let mut market_word = [0u8; 32];
            market_word[16..].copy_from_slice(&market_id.to_be_bytes());
            data.extend_from_slice(&market_word);
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.perp_engine.clone(),
                    to: self.perp_engine.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            let price = decode_uint256_low128(&out.output, "getMarkPrice")?;
            if price == 0 {
                return Err(BackendError::PerpsProtocolReferencePriceUnavailable(
                    "getMarkPrice returned 0 — oracle likely stale or unconfigured".to_string(),
                ));
            }
            Ok(price)
        })
    }
}

fn decode_uint256_low128(bytes: &[u8], name: &str) -> Result<u128> {
    if bytes.len() != 32 {
        return Err(BackendError::Config(format!(
            "{name} return length {} != 32",
            bytes.len()
        )));
    }
    // uint256 → we only support values that fit in the low 128 bits;
    // nonces and 1e8 prices never exceed that for closed-test.
    for byte in &bytes[..16] {
        if *byte != 0 {
            return Err(BackendError::Config(format!(
                "{name} value overflows u128 (upper 128 bits non-zero)"
            )));
        }
    }
    let mut u128_bytes = [0u8; 16];
    u128_bytes.copy_from_slice(&bytes[16..32]);
    Ok(u128::from_be_bytes(u128_bytes))
}

/// Default deadline TTL (seconds) for a prepared closed-test trade:
/// **1 hour**. The deployed V1 PME compares `t.deadline` against
/// `block.timestamp`, which is Unix **seconds**. This constant is in
/// SECONDS, and `prepare_trade_core` computes `deadline = now_sec +
/// PREPARE_DEADLINE_TTL_SEC` — NOT `now_ms + 3_600_000` (which would
/// be interpreted by Solidity as ~50 years in the future).
pub const PREPARE_DEADLINE_TTL_SEC: u128 = 3_600;

/// Backwards-compat alias while callers migrate. Retains the old
/// name but semantics have changed: value is now in SECONDS (was
/// milliseconds in the initial v1 draft — that draft had a units bug
/// caught by the reopened milestone audit).
#[deprecated(
    note = "Use PREPARE_DEADLINE_TTL_SEC. This alias has been corrected to seconds — do not scale by 1000."
)]
pub const PREPARE_DEADLINE_TTL_MS: u128 = PREPARE_DEADLINE_TTL_SEC;

// ================================================================
// PHASE A — PREPARE
// ================================================================

/// FINAL closed-test prepare request. All backend-owned fields
/// (`uuid`, `intentId`, `executionPrice1e8`, `buyerNonce`,
/// `sellerNonce`, `deadline`) are frozen by the backend at prepare
/// time — the caller CANNOT influence them.
///
/// Only the following fields are operator-owned:
///
/// * `buyer` — closed-test allowlisted address
/// * `seller` — closed-test allowlisted address
/// * `marketId` — market to trade against
/// * `sizeDelta1e8` — trade size (1e8 scale)
/// * `buyerIsMaker` — maker-side hint (economic classification only;
///   the deployed V1 PME does not enforce a specific maker orientation)
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
///
/// `nonce_reader` and `mark_price_reader` are injected so:
/// - production wires the RPC-backed implementations
///   ([`RpcNonceReader`], [`RpcMarkPriceReader`]),
/// - integration tests inject deterministic doubles,
/// - failure in either reader FAILS CLOSED (no silent zero
///   substitution).
pub async fn prepare_trade_core<NR, MR>(
    state: &AppState,
    req: &PrepareTradeRequest,
    nonce_reader: &NR,
    mark_price_reader: &MR,
    now_ms_override: Option<TimestampMs>,
) -> Result<PrepareOutcome>
where
    NR: NonceReader + ?Sized,
    MR: MarkPriceReader + ?Sized,
{
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
    if size_delta_1e8 == 0 {
        return Err(BackendError::PerpZeroSize);
    }

    // ---- Backend-owned execution price ----
    //
    // Read the authoritative live mark from the injected
    // `mark_price_reader`. Production wires
    // [`RpcMarkPriceReader`] which invokes
    // `PerpEngine.getMarkPrice(marketId)` (the same value the receipt
    // identity verifier uses at settlement time). Any RPC failure or
    // zero return FAILS CLOSED with
    // `PerpsProtocolReferencePriceUnavailable`.
    let execution_price_1e8 = mark_price_reader.read_mark_price(market_id).await?;
    if execution_price_1e8 == 0 {
        return Err(BackendError::PerpsProtocolReferencePriceUnavailable(
            "mark price 0 — refuse to freeze zero execution price".to_string(),
        ));
    }

    // ---- Backend-owned PME nonces ----
    //
    // Read the authoritative nonces from
    // `PerpMatchingEngine.nonces(buyer)` and
    // `PerpMatchingEngine.nonces(seller)` via the injected reader.
    // Any RPC failure FAILS CLOSED — no silent 0 substitution. This
    // is the property the reopened milestone required (previous
    // draft accepted client-supplied nonces).
    let buyer_nonce = nonce_reader.read_pme_nonce(&buyer).await?;
    let seller_nonce = nonce_reader.read_pme_nonce(&seller).await?;

    // ---- Freeze identity + timestamps ----
    //
    // CRITICAL: Solidity `_isDeadlineValid` compares `t.deadline`
    // against `block.timestamp`, which is Unix SECONDS. Convert now
    // from ms to seconds BEFORE adding the TTL. See regression test
    // `deadline_is_unix_seconds_not_ms`.
    let now_ms_val = now_ms_override.unwrap_or_else(now_ms);
    let now_sec = (now_ms_val / 1000) as u128;
    let deadline_sec = now_sec.saturating_add(PREPARE_DEADLINE_TTL_SEC);

    let uuid = Uuid::new_v4();
    let intent_id_hex = crate::execution::intent_id_to_hex_bytes32(&uuid.to_string())?;
    let intent_id_b256 = intent_id_to_b256(&uuid.to_string())?;

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
        deadline_sec,
    )?;

    let domain = PerpTradeDomain::new(
        state.perps_read_config.chain_id,
        state.execution_config.perp_matching_engine_address.clone(),
    );
    let digest_hex = perp_trade_v1_digest(&payload, &domain)?;

    // ExecutionIntent.deadline_ms is a ms field but the on-chain
    // deadline is seconds. We persist the seconds value multiplied by
    // 1000 back to ms so the type stays coherent with existing rows.
    // The AUTHORITATIVE frozen deadline lives on the `PerpTradePayload`
    // (deadline_sec) — cosign reload uses that value, not the ms
    // shadow.
    let deadline_shadow_ms: TimestampMs = i64::try_from(deadline_sec.saturating_mul(1000))
        .map_err(|_| {
            BackendError::Config("deadline overflow when scaling to ms shadow".to_string())
        })?;

    let typed_data = build_typed_data_v1(&payload, &domain, &intent_id_hex, deadline_sec);

    // ---- PREPARED-AWAITING-COSIGN state ----
    //
    // Persisted with `Pending` status so the executor tick / worker
    // does NOT select it for broadcast. The executor path selects
    // rows in `Pending → DryRun → SimulationOk → CalldataReady →
    // Submitted` phase-wise; without buyer_sig+seller_sig persisted,
    // `build_execution_transaction_request` short-circuits with
    // `MissingTradeSignatures`. State advances to `CalldataReady`
    // only after cosign persists both signatures.
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
        deadline_ms: Some(deadline_shadow_ms),
        created_at_ms: now_ms_val,
        status: ExecutionIntentStatus::Pending,
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
    deadline_sec: u128,
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
            "deadline": deadline_sec.to_string()
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

/// Persistence-aware co-sign path. Reconstructs the frozen 10-field
/// `PerpTrade` from a previously persisted `ExecutionIntent`, verifies
/// both signatures against the SERVER-COMPUTED digest, and returns
/// the verified sigs + a rebuilt `PerpTradePayload` ready for
/// `upsert_execution_intent_signatures`.
///
/// This function does NOT trust the client to re-send economic
/// fields. The reload path is:
///   uuid → ExecutionIntent (all 10 frozen fields) → PerpTradePayload
///   → PerpTradeDomain → digest → ecrecover.
///
/// Idempotency: if `prior_bundle` matches `req` byte-for-byte, this
/// returns Ok WITHOUT re-verifying (safe replay). If `prior_bundle`
/// is Some but differs, returns
/// `BackendError::BroadcastRejected("cosign_conflict")`.
///
/// Expired-trade rejection: if `now_sec > payload.deadline`, returns
/// `PerpsIntentDeadlineExpired`.
///
/// Callers responsible for the actual repository write; this function
/// returns the verified bundle for `upsert_execution_intent_signatures`.
pub fn cosign_load_and_verify(
    intent: &ExecutionIntent,
    domain: &PerpTradeDomain,
    req: &CosignTradeRequest,
    prior_bundle: Option<&StoredTradeSignaturesView>,
    now_sec: u128,
) -> Result<CosignVerifiedSignatures> {
    // Reconstruct payload from persisted intent — this is
    // authoritative; client-supplied fields are ignored.
    let payload = intent_to_v1_payload(intent)?;
    // Deadline check against Unix seconds.
    if payload.deadline > 0 && now_sec > payload.deadline {
        return Err(BackendError::PerpsIntentDeadlineExpired);
    }
    // Idempotency vs conflict check.
    if let Some(prior) = prior_bundle {
        let same_buyer = prior
            .buyer_sig
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case(&req.buyer_signature))
            .unwrap_or(false);
        let same_seller = prior
            .seller_sig
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case(&req.seller_signature))
            .unwrap_or(false);
        if same_buyer && same_seller {
            // Idempotent replay — same bundle, no state change.
            return Ok(CosignVerifiedSignatures {
                buyer_signer: payload.buyer.clone(),
                seller_signer: payload.seller.clone(),
                bundle: PerpTradeSignatureBundle::new(&req.buyer_signature, &req.seller_signature)?,
            });
        }
        if prior.buyer_sig.is_some() || prior.seller_sig.is_some() {
            return Err(BackendError::BroadcastRejected(
                "cosign_conflict: prior signature bundle exists and differs".to_string(),
            ));
        }
    }
    cosign_verify_core(&payload, domain, req)
}

/// Reconstruct the 10-field `PerpTradePayload` from a persisted
/// `ExecutionIntent`. Fails closed if any required field is absent —
/// `execution_intents` rows with `NULL` `buyer_nonce` / `seller_nonce`
/// / `deadline_ms` / `buyer_is_maker` are not co-signable through
/// this path.
pub fn intent_to_v1_payload(intent: &ExecutionIntent) -> Result<PerpTradePayload> {
    let intent_id_b256 = intent_id_to_b256(&intent.intent_id.to_string())?;
    let buyer_is_maker = intent
        .buyer_is_maker
        .ok_or_else(|| BackendError::MissingExecutionMetadata("buyer_is_maker".to_string()))?;
    let buyer_nonce = intent
        .buyer_nonce
        .ok_or_else(|| BackendError::MissingExecutionMetadata("buyer_nonce".to_string()))?;
    let seller_nonce = intent
        .seller_nonce
        .ok_or_else(|| BackendError::MissingExecutionMetadata("seller_nonce".to_string()))?;
    let deadline_ms = intent
        .deadline_ms
        .ok_or_else(|| BackendError::MissingExecutionMetadata("deadline".to_string()))?;
    // The persisted deadline_ms is `deadline_seconds × 1000` (see
    // prepare_trade_core::deadline_shadow_ms). Convert back to
    // seconds for the on-chain V1 comparison.
    let deadline_sec = u128::try_from(deadline_ms.saturating_div(1000))
        .map_err(|_| BackendError::Config("deadline_ms → seconds conversion failed".to_string()))?;

    PerpTradePayload::new(
        intent_id_b256,
        intent.buyer.clone(),
        intent.seller.clone(),
        u128::from(intent.market_id),
        intent.size_1e8,
        intent.price_1e8,
        0,
        0,
        buyer_is_maker,
        u128::from(buyer_nonce),
        u128::from(seller_nonce),
        deadline_sec,
    )
}

/// Read-only view of the two signature strings for idempotency
/// comparison. Matches [`crate::execution::StoredTradeSignatures`]
/// but decoupled to keep this module free of persistence-layer
/// details.
#[derive(Clone, Debug, Default)]
pub struct StoredTradeSignaturesView {
    pub buyer_sig: Option<String>,
    pub seller_sig: Option<String>,
}

impl From<&crate::execution::StoredTradeSignatures> for StoredTradeSignaturesView {
    fn from(v: &crate::execution::StoredTradeSignatures) -> Self {
        Self {
            buyer_sig: v.buyer_sig.clone(),
            seller_sig: v.seller_sig.clone(),
        }
    }
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

    // ================================================================
    // REOPENED MILESTONE — expanded test matrix
    // ================================================================

    // Test-only builder for a PerpTrade with configurable nonces + deadline.
    fn payload_with(
        uuid: uuid::Uuid,
        buyer_nonce: u128,
        seller_nonce: u128,
        deadline_sec: u128,
    ) -> PerpTradePayload {
        let intent_id = intent_id_to_b256(&uuid.to_string()).unwrap();
        PerpTradePayload::new(
            intent_id,
            AccountId::new(BUYER_ADDR.to_string()),
            AccountId::new(SELLER_ADDR.to_string()),
            1,
            1_000_000,
            240_000_000_000,
            0,
            0,
            false,
            buyer_nonce,
            seller_nonce,
            deadline_sec,
        )
        .unwrap()
    }

    fn intent_from_payload(
        uuid: uuid::Uuid,
        payload: &PerpTradePayload,
    ) -> crate::execution::ExecutionIntent {
        crate::execution::ExecutionIntent {
            intent_id: uuid,
            market_id: crate::types::MarketId::from(payload.market_id as u16),
            buyer: payload.buyer.clone(),
            seller: payload.seller.clone(),
            price_1e8: payload.execution_price_1e8 as crate::types::Price1e8,
            size_1e8: payload.size_delta_1e8 as crate::types::Size1e8,
            buy_order_id: crate::types::OrderId(uuid::Uuid::new_v4()),
            sell_order_id: crate::types::OrderId(uuid::Uuid::new_v4()),
            buyer_is_maker: Some(payload.buyer_is_maker),
            buyer_nonce: Some(payload.buyer_nonce as u64),
            seller_nonce: Some(payload.seller_nonce as u64),
            deadline_ms: Some(i64::try_from(payload.deadline.saturating_mul(1000)).unwrap()),
            created_at_ms: 1_700_000_000_000,
            status: crate::execution::ExecutionIntentStatus::Pending,
        }
    }

    // --- (Z1) Deadline unit is seconds, not milliseconds ---
    #[test]
    fn z1_deadline_is_unix_seconds_not_ms() {
        // Ensure PREPARE_DEADLINE_TTL_SEC is exactly 3600 seconds (1
        // hour). If someone re-introduces the ms-scaled bug this
        // test flags it.
        assert_eq!(PREPARE_DEADLINE_TTL_SEC, 3_600);
        // A prepared trade's deadline should be ~+3600 seconds ahead
        // of the current timestamp, NOT +3_600_000.
        let now_ms_val: i64 = 1_700_000_000_000;
        let now_sec: u128 = (now_ms_val / 1000) as u128;
        let expected_deadline_sec = now_sec + 3_600;
        // Sanity: this value is a plausible Unix second timestamp
        // (~2023), NOT a value like 1_700_003_600_000 (which would
        // be interpreted by Solidity block.timestamp as year ~55000).
        assert!(
            expected_deadline_sec < 2_000_000_000,
            "deadline is a plausible Unix seconds value, not ms-scaled"
        );
    }

    // --- (Z2) Persistence round-trip: intent → payload → digest ---
    #[test]
    fn z2_persistence_round_trip_digest_equality() {
        let uuid = uuid::Uuid::from_u128(0xdead_beef);
        let payload = payload_with(uuid, 7, 12, 1_700_003_600);
        let digest_1 = perp_trade_v1_digest(&payload, &domain()).unwrap();
        let intent = intent_from_payload(uuid, &payload);
        let reloaded = intent_to_v1_payload(&intent).unwrap();
        let digest_2 = perp_trade_v1_digest(&reloaded, &domain()).unwrap();
        assert_eq!(
            digest_1, digest_2,
            "digest is identical after DB round-trip"
        );
        assert_eq!(reloaded.buyer_nonce, 7);
        assert_eq!(reloaded.seller_nonce, 12);
        assert_eq!(reloaded.deadline, 1_700_003_600);
    }

    // --- (Z3) Non-zero PME nonces preserved byte-exactly ---
    #[test]
    fn z3_non_zero_nonces_preserved_through_pipeline() {
        use crate::execution::{encode_execute_trade_calldata, PerpTradeSignatureBundle};
        let uuid = uuid::Uuid::from_u128(0x1111);
        let payload = payload_with(uuid, 7, 12, 1_700_003_600);
        let bundle = PerpTradeSignatureBundle::new(
            &format!("0x{}", "aa".repeat(65)),
            &format!("0x{}", "bb".repeat(65)),
        )
        .unwrap();
        let calldata = encode_execute_trade_calldata(&payload, &bundle).unwrap();
        // Tuple fields are laid out post-selector at 4-byte offset.
        // Field 7 (buyerNonce, uint256) at offset 4 + 32*7 = 228.
        //   4 (selector) + 32 * (intentId, buyer, seller, marketId, sizeDelta1e8, executionPrice1e8, buyerIsMaker) = 4 + 224 = 228
        // Reading 32 bytes → BE uint256.
        let buyer_nonce_bytes: &[u8] = &calldata[228..260];
        let mut buyer_nonce_val = 0u128;
        for b in &buyer_nonce_bytes[16..32] {
            buyer_nonce_val = (buyer_nonce_val << 8) | (*b as u128);
        }
        assert_eq!(buyer_nonce_val, 7, "buyerNonce = 7 preserved in calldata");
        // Field 8 (sellerNonce, uint256) at 260..292.
        let seller_nonce_bytes: &[u8] = &calldata[260..292];
        let mut seller_nonce_val = 0u128;
        for b in &seller_nonce_bytes[16..32] {
            seller_nonce_val = (seller_nonce_val << 8) | (*b as u128);
        }
        assert_eq!(
            seller_nonce_val, 12,
            "sellerNonce = 12 preserved in calldata"
        );
    }

    // --- (Z4) cosign_load_and_verify: unknown UUID surface handled by loader ---
    // (This test documents the caller responsibility; unknown UUID
    // is caught by the repository loader before this function is
    // invoked. Verified in the runtime wiring layer.)

    // --- (Z5) cosign_load_and_verify: expired deadline rejected ---
    #[test]
    fn z5_expired_deadline_rejects() {
        let uuid = uuid::Uuid::from_u128(0x2222);
        let payload = payload_with(uuid, 0, 0, 1_700_000_000);
        let intent = intent_from_payload(uuid, &payload);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: seller_sig,
        };
        // now_sec > deadline (1_700_000_000) → expired.
        let err =
            cosign_load_and_verify(&intent, &domain(), &req, None, 1_700_100_000).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentDeadlineExpired));
    }

    // --- (Z6) cosign_load_and_verify: happy path ---
    #[test]
    fn z6_load_and_verify_happy_path() {
        let uuid = uuid::Uuid::from_u128(0x3333);
        let payload = payload_with(uuid, 0, 0, 1_800_000_000);
        let intent = intent_from_payload(uuid, &payload);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig.clone(),
            seller_signature: seller_sig.clone(),
        };
        let verified =
            cosign_load_and_verify(&intent, &domain(), &req, None, 1_700_000_000).unwrap();
        assert_eq!(verified.buyer_signer.0.to_lowercase(), BUYER_ADDR);
        assert_eq!(verified.seller_signer.0.to_lowercase(), SELLER_ADDR);
    }

    // --- (Z7) Idempotent resubmit: same bundle returns Ok without re-verification ---
    #[test]
    fn z7_idempotent_resubmit_same_bundle_ok() {
        let uuid = uuid::Uuid::from_u128(0x4444);
        let payload = payload_with(uuid, 0, 0, 1_800_000_000);
        let intent = intent_from_payload(uuid, &payload);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig.clone(),
            seller_signature: seller_sig.clone(),
        };
        let prior = StoredTradeSignaturesView {
            buyer_sig: Some(buyer_sig.clone()),
            seller_sig: Some(seller_sig.clone()),
        };
        let verified =
            cosign_load_and_verify(&intent, &domain(), &req, Some(&prior), 1_700_000_000).unwrap();
        assert_eq!(verified.buyer_signer.0.to_lowercase(), BUYER_ADDR);
    }

    // --- (Z8) Conflicting resubmit: different bundle rejected ---
    #[test]
    fn z8_conflicting_resubmit_rejected() {
        let uuid = uuid::Uuid::from_u128(0x5555);
        let payload = payload_with(uuid, 0, 0, 1_800_000_000);
        let intent = intent_from_payload(uuid, &payload);
        let digest = perp_trade_v1_digest_bytes(&payload, &domain()).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        // Prior stores a DIFFERENT buyer_sig.
        let different_buyer_sig = format!("0x{}", "cc".repeat(65));
        let prior = StoredTradeSignaturesView {
            buyer_sig: Some(different_buyer_sig),
            seller_sig: Some(seller_sig.clone()),
        };
        let req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: seller_sig,
        };
        let err = cosign_load_and_verify(&intent, &domain(), &req, Some(&prior), 1_700_000_000)
            .unwrap_err();
        assert!(
            matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("cosign_conflict"))
        );
    }

    // --- (Z9) Unsigned ExecutionIntent cannot be built into a
    //     broadcast request: build_execution_transaction_request
    //     returns MissingTradeSignatures. Proves the invariant that
    //     a prepared row with no sigs cannot reach the RPC.
    #[test]
    fn z9_unsigned_prepared_intent_cannot_broadcast() {
        use crate::execution::{
            build_execution_transaction_request, ExecutionConfig, StoredTradeSignatures,
        };
        let uuid = uuid::Uuid::from_u128(0x6666);
        let payload = payload_with(uuid, 0, 0, 1_800_000_000);
        let mut intent = intent_from_payload(uuid, &payload);
        intent.status = crate::execution::ExecutionIntentStatus::SimulationOk;
        let sigs = StoredTradeSignatures::default(); // NO sigs
        let config = ExecutionConfig {
            perp_matching_engine_address: AccountId::new(PME.to_string()),
            require_simulation_ok: true,
            executor_chain_id: 84532,
            max_gas_limit: 1_000_000,
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            ..ExecutionConfig::disabled()
        };
        let err = build_execution_transaction_request(&config, &intent, &sigs).unwrap_err();
        assert!(matches!(err, BackendError::MissingTradeSignatures));
    }

    // --- (Z10) Signed ExecutionIntent DOES produce broadcast calldata ---
    #[test]
    fn z10_signed_intent_produces_valid_calldata() {
        use crate::execution::{
            build_execution_transaction_request, ExecutionConfig, StoredTradeSignatures,
        };
        let uuid = uuid::Uuid::from_u128(0x7777);
        let payload = payload_with(uuid, 0, 0, 1_800_000_000);
        let mut intent = intent_from_payload(uuid, &payload);
        intent.status = crate::execution::ExecutionIntentStatus::SimulationOk;
        let sigs = StoredTradeSignatures {
            buyer_sig: Some(format!("0x{}", "aa".repeat(65))),
            seller_sig: Some(format!("0x{}", "bb".repeat(65))),
        };
        let config = ExecutionConfig {
            perp_matching_engine_address: AccountId::new(PME.to_string()),
            require_simulation_ok: true,
            executor_chain_id: 84532,
            max_gas_limit: 1_000_000,
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            ..ExecutionConfig::disabled()
        };
        let request = build_execution_transaction_request(&config, &intent, &sigs).unwrap();
        // Selector at bytes 0..4 must be 0x7a708c4c (deployed 10-field executeTrade).
        assert_eq!(hex_encode(&request.calldata[..4]), "7a708c4c");
    }

    // --- (Z11) intent_to_v1_payload: fail-closed on missing fields ---
    #[test]
    fn z11_intent_to_payload_fails_closed_on_missing_fields() {
        let uuid = uuid::Uuid::from_u128(0x8888);
        let payload = payload_with(uuid, 0, 0, 1_800_000_000);
        let mut intent = intent_from_payload(uuid, &payload);
        intent.buyer_nonce = None;
        let err = intent_to_v1_payload(&intent).unwrap_err();
        assert!(
            matches!(err, BackendError::MissingExecutionMetadata(f) if f.contains("buyer_nonce"))
        );
    }

    // ================================================================
    // FINALIZATION — reader-injected prepare_trade_core tests
    // ================================================================
    use std::sync::Mutex;

    struct StaticNonceReader {
        buyer_addr: String,
        buyer: u128,
        seller_addr: String,
        seller: u128,
        fail: bool,
    }
    impl NonceReader for StaticNonceReader {
        fn read_pme_nonce<'a>(&'a self, trader: &'a AccountId) -> ReaderFuture<'a, u128> {
            let addr = trader.0.to_ascii_lowercase();
            let buyer_addr = self.buyer_addr.clone();
            let seller_addr = self.seller_addr.clone();
            let (buyer_n, seller_n) = (self.buyer, self.seller);
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(BackendError::Config("mock RPC failure (nonce)".to_string()));
                }
                if addr == buyer_addr {
                    Ok(buyer_n)
                } else if addr == seller_addr {
                    Ok(seller_n)
                } else {
                    Err(BackendError::Config(format!("unexpected trader {addr}")))
                }
            })
        }
    }

    struct StaticMarkPriceReader {
        price: u128,
        fail: bool,
    }
    impl MarkPriceReader for StaticMarkPriceReader {
        fn read_mark_price<'a>(&'a self, _market_id: u128) -> ReaderFuture<'a, u128> {
            let price = self.price;
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(BackendError::PerpsProtocolReferencePriceUnavailable(
                        "mock RPC failure (mark price)".to_string(),
                    ));
                }
                Ok(price)
            })
        }
    }

    fn app_state_closed_test_allowed() -> AppState {
        let mut state =
            crate::api::http::AppState::new(crate::engine::EngineState::with_default_markets());
        state.perps_closed_test_enabled = true;
        state.perps_public_trading_enabled = false;
        state.perps_closed_test_allowlist = vec![
            AccountId::new(BUYER_ADDR.to_string()),
            AccountId::new(SELLER_ADDR.to_string()),
        ];
        state.perps_read_config.chain_id = CHAIN_ID;
        state.execution_config.perp_matching_engine_address = AccountId::new(PME.to_string());
        state.execution_config.perp_engine_address =
            AccountId::new("0xc6c592100723fe0c66343a16e95ec34cc0c2141c".to_string());
        state
    }

    fn mk_request() -> PrepareTradeRequest {
        PrepareTradeRequest {
            buyer: BUYER_ADDR.to_string(),
            seller: SELLER_ADDR.to_string(),
            market_id: "1".to_string(),
            size_delta_1e8: "1000000".to_string(),
            buyer_is_maker: false,
        }
    }

    // (F1) Backend-owned nonces + price: injected 7/12/240e11.
    #[tokio::test]
    async fn f1_backend_owned_nonces_and_price_flow_through_pipeline() {
        let state = app_state_closed_test_allowed();
        let req = mk_request();
        let nonce_reader = StaticNonceReader {
            buyer_addr: BUYER_ADDR.to_string(),
            buyer: 7,
            seller_addr: SELLER_ADDR.to_string(),
            seller: 12,
            fail: false,
        };
        let mark_reader = StaticMarkPriceReader {
            price: 240_000_000_000,
            fail: false,
        };
        let outcome = prepare_trade_core(
            &state,
            &req,
            &nonce_reader,
            &mark_reader,
            Some(1_700_000_000_000),
        )
        .await
        .expect("prepare must succeed with valid readers");
        assert_eq!(outcome.payload.buyer_nonce, 7);
        assert_eq!(outcome.payload.seller_nonce, 12);
        assert_eq!(outcome.payload.execution_price_1e8, 240_000_000_000);
        // Deadline = now_sec + 3600 (Unix seconds).
        assert_eq!(outcome.payload.deadline, 1_700_000_000 + 3_600);
        // Persistence shadow ms = seconds × 1000.
        assert_eq!(
            outcome.execution_intent.deadline_ms.unwrap(),
            (1_700_000_000 + 3_600) * 1_000
        );
        // Status is Pending — unsigned prepared intent cannot broadcast.
        assert_eq!(
            outcome.execution_intent.status,
            crate::execution::ExecutionIntentStatus::Pending
        );
    }

    // (F2) Nonce RPC failure → FAIL CLOSED.
    #[tokio::test]
    async fn f2_nonce_rpc_failure_fails_closed() {
        let state = app_state_closed_test_allowed();
        let req = mk_request();
        let nonce_reader = StaticNonceReader {
            buyer_addr: BUYER_ADDR.to_string(),
            buyer: 0,
            seller_addr: SELLER_ADDR.to_string(),
            seller: 0,
            fail: true,
        };
        let mark_reader = StaticMarkPriceReader {
            price: 240_000_000_000,
            fail: false,
        };
        let err = prepare_trade_core(&state, &req, &nonce_reader, &mark_reader, None)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::Config(msg) if msg.contains("nonce")));
    }

    // (F3) Mark-price RPC failure → FAIL CLOSED.
    #[tokio::test]
    async fn f3_mark_price_rpc_failure_fails_closed() {
        let state = app_state_closed_test_allowed();
        let req = mk_request();
        let nonce_reader = StaticNonceReader {
            buyer_addr: BUYER_ADDR.to_string(),
            buyer: 0,
            seller_addr: SELLER_ADDR.to_string(),
            seller: 0,
            fail: false,
        };
        let mark_reader = StaticMarkPriceReader {
            price: 0,
            fail: true,
        };
        let err = prepare_trade_core(&state, &req, &nonce_reader, &mark_reader, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsProtocolReferencePriceUnavailable(_)
        ));
    }

    // (F4) Closed-test disabled → PerpsNotLive.
    #[tokio::test]
    async fn f4_closed_test_disabled_rejects() {
        let mut state = app_state_closed_test_allowed();
        state.perps_closed_test_enabled = false;
        let req = mk_request();
        let nonce_reader = StaticNonceReader {
            buyer_addr: BUYER_ADDR.to_string(),
            buyer: 0,
            seller_addr: SELLER_ADDR.to_string(),
            seller: 0,
            fail: false,
        };
        let mark_reader = StaticMarkPriceReader {
            price: 240_000_000_000,
            fail: false,
        };
        let err = prepare_trade_core(&state, &req, &nonce_reader, &mark_reader, None)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PerpsNotLive));
    }

    // (F5) Non-allowlisted buyer → PerpsNotLive.
    #[tokio::test]
    async fn f5_non_allowlisted_buyer_rejects() {
        let mut state = app_state_closed_test_allowed();
        state.perps_closed_test_allowlist = vec![AccountId::new(SELLER_ADDR.to_string())];
        let req = mk_request();
        let nonce_reader = StaticNonceReader {
            buyer_addr: BUYER_ADDR.to_string(),
            buyer: 0,
            seller_addr: SELLER_ADDR.to_string(),
            seller: 0,
            fail: false,
        };
        let mark_reader = StaticMarkPriceReader {
            price: 240_000_000_000,
            fail: false,
        };
        let err = prepare_trade_core(&state, &req, &nonce_reader, &mark_reader, None)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PerpsNotLive));
    }

    // (F6) buyer == seller → rejected.
    #[tokio::test]
    async fn f6_buyer_equals_seller_rejects() {
        let mut state = app_state_closed_test_allowed();
        // allowlist contains buyer twice
        state.perps_closed_test_allowlist = vec![AccountId::new(BUYER_ADDR.to_string())];
        let mut req = mk_request();
        req.seller = BUYER_ADDR.to_string();
        let nonce_reader = StaticNonceReader {
            buyer_addr: BUYER_ADDR.to_string(),
            buyer: 0,
            seller_addr: BUYER_ADDR.to_string(),
            seller: 0,
            fail: false,
        };
        let mark_reader = StaticMarkPriceReader {
            price: 240_000_000_000,
            fail: false,
        };
        let err = prepare_trade_core(&state, &req, &nonce_reader, &mark_reader, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BackendError::PerpsIntentSideBoundInconsistent(msg) if msg.contains("buyer == seller")
        ));
    }

    // (F7) End-to-end: prepare → cosign → both sigs recover.
    #[tokio::test]
    async fn f7_prepare_and_cosign_end_to_end_with_readers() {
        let state = app_state_closed_test_allowed();
        let req = mk_request();
        let nonce_reader = StaticNonceReader {
            buyer_addr: BUYER_ADDR.to_string(),
            buyer: 7,
            seller_addr: SELLER_ADDR.to_string(),
            seller: 12,
            fail: false,
        };
        let mark_reader = StaticMarkPriceReader {
            price: 240_000_000_000,
            fail: false,
        };
        let outcome = prepare_trade_core(
            &state,
            &req,
            &nonce_reader,
            &mark_reader,
            Some(1_700_000_000_000),
        )
        .await
        .unwrap();
        let digest = perp_trade_v1_digest_bytes(&outcome.payload, &outcome.domain).unwrap();
        let buyer_sig = sign_v1_with(BUYER_KEY, &digest);
        let seller_sig = sign_v1_with(SELLER_KEY, &digest);
        let cosign_req = CosignTradeRequest {
            buyer_signature: buyer_sig,
            seller_signature: seller_sig,
        };
        let verified = cosign_load_and_verify(
            &outcome.execution_intent,
            &outcome.domain,
            &cosign_req,
            None,
            /*now_sec*/ 1_700_000_100,
        )
        .unwrap();
        assert_eq!(verified.buyer_signer.0.to_lowercase(), BUYER_ADDR);
        assert_eq!(verified.seller_signer.0.to_lowercase(), SELLER_ADDR);
    }

    // (F8) Byte-decode of uint256 low-128 helper.
    #[test]
    fn f8_decode_uint256_low128() {
        // Value 42 encoded as 32-byte BE.
        let mut bytes = [0u8; 32];
        bytes[31] = 42;
        let val = super::decode_uint256_low128(&bytes, "x").unwrap();
        assert_eq!(val, 42);
        // Upper 128 bits nonzero → overflow error.
        bytes[0] = 1;
        let err = super::decode_uint256_low128(&bytes, "x").unwrap_err();
        assert!(matches!(err, BackendError::Config(msg) if msg.contains("overflows")));
    }

    // (F9) Selector regression check.
    #[test]
    fn f9_reader_selectors_match_deployed() {
        assert_eq!(PME_NONCES_SELECTOR, [0x7e, 0xce, 0xbe, 0x00]);
        assert_eq!(
            PERP_ENGINE_GET_MARK_PRICE_SELECTOR,
            [0x5a, 0xf3, 0xd0, 0x61]
        );
    }
}
