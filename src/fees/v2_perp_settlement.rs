//! Canonical replacement-PERP fee accounting. Local orderbook fills are
//! projections; only a canonical D6 trade plus its D2 charge may enter this
//! ledger. This records an already charged on-chain fee; it never transfers it.
use crate::db::PgRepository;
use crate::error::{BackendError, Result};
use crate::execution::perp_trade::PerpsProtocolVersion;
use crate::fees::d2_onchain::{
    read_account_tier_at_block, verify_d2_fee_policy, VerifiedD2FeePolicy, BASE_SEPOLIA_CHAIN_ID,
    D2_ADDRESS, TIMELOCK_ADDRESS,
};
use crate::indexer::decoder::{decode_trade_executed_log_for_emitter, trade_executed_topic0};
use crate::indexer::{EthLog, IndexedPerpTrade};
use alloy_primitives::keccak256;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

pub const D4_ENGINE: &str = "0xd0901de8f6de72aecc716ca1465c4cf58a0b99fb";
pub const D6_MATCHER: &str = "0xeea4e5d891b90d3d99e9c4e2c61069911477110e";
pub const D1_REGISTRY: &str = "0x6b3d846536116082dc4e7227861c341bb85ee963";
pub const MUSDC: &str = "0x6eae407f5640b006fac9965182e238582a3b412e";
const MUSDC_DECIMALS: u32 = 6;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalV2PerpFee {
    pub chain_id: u64,
    pub tx_hash: String,
    pub trade_log_index: u64,
    pub fee_log_index: Option<u64>,
    pub block_number: u64,
    pub block_hash: String,
    pub market_id: String,
    pub payer: String,
    pub recipient: String,
    pub settlement_asset: String,
    pub maker_tier: u8,
    pub taker_tier: u8,
    pub basis_native: u128,
    pub amount_native: u128,
}

/// Minimal durable classification. Signed hashes live in VerifiedIntentFill;
/// a trace digest commits to nested calldata without retaining signatures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FillProvenance {
    pub status: &'static str,
    pub trace_call_hash: Option<String>,
}

impl FillProvenance {
    pub fn signed_direct() -> Self { Self { status: "signed_direct_verified", trace_call_hash: None } }
    pub fn signed_trace(hash: String) -> Self { Self { status: "signed_trace_verified", trace_call_hash: Some(hash) } }
    pub fn trace_unavailable() -> Self { Self { status: "wallet_only_trace_unavailable", trace_call_hash: None } }
    pub fn no_intent() -> Self { Self { status: "wallet_only_no_intent", trace_call_hash: None } }
}

/// A settled wallet-level fact. D6's `TradeExecuted` event carries wallets,
/// but no subaccount identifier. In particular, the local default account
/// is never evidence of canonical subaccount ownership.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CanonicalV2PerpHistoryRow {
    pub chain_id: u64,
    pub tx_hash: String,
    pub trade_log_index: u64,
    pub fee_log_index: Option<u64>,
    pub block_number: u64,
    pub block_hash: String,
    pub market_id: String,
    pub wallet: String,
    pub counterparty: String,
    pub role: &'static str,
    pub side: &'static str,
    pub size_delta_1e8: String,
    pub execution_price_1e8: String,
    pub fee_native: String,
    pub fee_payer: String,
    pub fee_recipient: String,
    pub subaccount_id: Option<u32>,
    /// EIP-712 signed order-origin label. It is not a canonical D4 account.
    pub signed_subaccount_id: Option<u32>,
    pub attribution: &'static str,
    pub settlement: &'static str,
    /// Authentication performed at ingestion. This is not a claim that the
    /// historical block is still canonical at the moment of this API read.
    pub signed_origin_provenance: String,
    pub canonicality_status: &'static str,
}

/// The five hashes are public execution receipts, supplied only after their
/// independent S2 postflights. Missing or malformed values keep V2 closed.
pub fn execution_hashes_from_env() -> Result<[String; 5]> {
    let value = std::env::var("D2_PERP_TIER_S2_TX_HASHES").map_err(|_| {
        BackendError::Config("D2 canonical execution receipts are not configured".into())
    })?;
    let hashes: Vec<_> = value
        .split(',')
        .map(|part| part.trim().to_string())
        .collect();
    let array: [String; 5] = hashes.try_into().map_err(|_| {
        BackendError::Config("D2 requires exactly five S2 execution receipts".into())
    })?;
    if array.iter().any(|hash| !is_hash(hash)) {
        return Err(BackendError::Config(
            "D2 S2 receipt hash is malformed".into(),
        ));
    }
    Ok(array)
}

/// Called after a confirmed replacement trade. D2 policy proof and account
/// tiers are read outside the database transaction. The receipt is verified
/// against the policy's pinned safe block, then the trade and fee are persisted
/// together. A reorg after commit is detected on replay and requires explicit
/// reconciliation; settled economic records are never silently reversed.
pub async fn reconcile_confirmed_v2_trade(
    repository: &PgRepository,
    rpc_url: &str,
    execution_hashes: &[String; 5],
    trade: &IndexedPerpTrade,
    now_seconds: u64,
) -> Result<bool> {
    let policy = verify_d2_fee_policy(rpc_url, now_seconds, 300, execution_hashes)
        .await
        .map_err(fee_error)?;
    verify_market_and_decimals_at_trade(rpc_url, trade).await?;
    let (maker, taker) = roles(trade)?;
    let trade_block_hash = trade
        .block_hash
        .as_deref()
        .ok_or_else(|| fee_error("indexed trade block hash missing".into()))?;
    let maker_tier =
        read_account_tier_at_block(rpc_url, trade.block_number, trade_block_hash, maker)
            .await
            .map_err(fee_error)?;
    let taker_tier =
        read_account_tier_at_block(rpc_url, trade.block_number, trade_block_hash, taker)
            .await
            .map_err(fee_error)?;
    let receipt = rpc(rpc_url, "eth_getTransactionReceipt", json!([trade.tx_hash])).await?;
    let fee = validate_receipt(
        trade,
        &receipt,
        &policy,
        maker_tier,
        taker_tier,
        (crate::types::now_ms() as u64) / 1000,
    )?;
    let block = rpc(
        rpc_url,
        "eth_getBlockByNumber",
        json!([format!("0x{:x}", fee.block_number), false]),
    )
    .await?;
    if !block
        .get("hash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(&fee.block_hash))
    {
        return Err(fee_error("V2 fee receipt block is noncanonical".into()));
    }
    repository.persist_canonical_v2_perp_fee(trade, &fee).await
}

/// Read-only RPC reconciliation for a direct or wrapped D6 intent transaction.
/// A wrapper is signed-origin verified only with a matching callTracer proof;
/// explicit method-not-found retains a wallet fee with unverified signed
/// origin. Neither path supplies a canonical economic subaccount.
pub async fn reconcile_confirmed_v2_intent_tx(
    repository: &PgRepository,
    rpc_url: &str,
    execution_hashes: &[String; 5],
    tx_hash: &str,
    now_seconds: u64,
) -> Result<bool> {
    if !is_hash(tx_hash) {
        return Err(fee_error("invalid intent transaction hash".into()));
    }
    let tx = rpc_nullable(rpc_url, "eth_getTransactionByHash", json!([tx_hash])).await?;
    let receipt = rpc_nullable(rpc_url, "eth_getTransactionReceipt", json!([tx_hash])).await?;
    if tx.is_null() || receipt.is_null() {
        repository.mark_canonical_v2_provenance_divergent(tx_hash).await?;
        return Err(fee_error("intent receipt missing; persisted provenance requires reconciliation".into()));
    }
    let block_number = quantity(receipt.get("blockNumber"))?;
    let block = rpc(rpc_url, "eth_getBlockByNumber", json!([format!("0x{block_number:x}"), false])).await?;
    let block_hash = block.get("hash").and_then(Value::as_str)
        .ok_or_else(|| fee_error("canonical block hash missing".into()))?;
    if !receipt.get("blockHash").and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(block_hash)) {
        repository.mark_canonical_v2_provenance_divergent(tx_hash).await?;
        return Err(fee_error("intent receipt block is noncanonical".into()));
    }
    repository.ensure_canonical_v2_provenance_block(tx_hash, block_hash).await?;
    let policy = verify_d2_fee_policy(rpc_url, now_seconds, 300, execution_hashes)
        .await.map_err(fee_error)?;
    let timestamp = quantity(block.get("timestamp"))?;
    let logs = receipt.get("logs").and_then(Value::as_array)
        .ok_or_else(|| fee_error("intent receipt logs missing".into()))?;
    let intent_topic = topic(crate::fees::v2_intent_receipt::INTENT_EVENT_SIGNATURE);
    let candidates: Vec<_> = logs.iter().filter(|log| {
        log.get("address").and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(D6_MATCHER))
            && log.get("topics").and_then(Value::as_array).and_then(|v| v.first()).and_then(Value::as_str)
                .is_some_and(|v| v.eq_ignore_ascii_case(&intent_topic))
    }).collect();
    if candidates.is_empty() || candidates.len() > 64 {
        return Err(fee_error("intent receipt fill count missing or exceeds bound".into()));
    }
    let direct = tx.get("to").and_then(Value::as_str)
        .is_some_and(|to| to.eq_ignore_ascii_case(D6_MATCHER));
    if direct && candidates.len() != 1 {
        return Err(fee_error("direct D6 call emitted multiple intent fills".into()));
    }
    let trace_proofs = if direct { None } else {
        match optional_transaction_trace(rpc_url, tx_hash).await? {
            Some(trace) => {
                let events: Vec<EthLog> = candidates.iter().map(|candidate|
                    serde_json::from_value((*candidate).clone())
                        .map_err(|_| fee_error("malformed intent event".into()))
                ).collect::<Result<_>>()?;
                Some(crate::fees::v2_intent_receipt::prove_traced_wrapper_intent_fills(
                    &tx, &receipt, &trace, &events, timestamp)?)
            }
            None => None,
        }
    };
    let mut verified = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        let event: EthLog = serde_json::from_value((*candidate).clone())
            .map_err(|_| fee_error("malformed intent event".into()))?;
        let (trade, fee, signed, provenance) = if direct {
            let proof = crate::fees::v2_intent_receipt::prove_direct_intent_fill(&tx, &receipt, &event, timestamp)?;
            let (maker, taker) = roles(&proof.trade)?;
            let maker_tier = read_account_tier_at_block(rpc_url, block_number, block_hash, maker).await.map_err(fee_error)?;
            let taker_tier = read_account_tier_at_block(rpc_url, block_number, block_hash, taker).await.map_err(fee_error)?;
            let fee = validate_intent_receipt(&proof, &receipt, &policy, maker_tier, taker_tier, now_seconds)?;
            (proof.trade.clone(), fee, Some(proof), FillProvenance::signed_direct())
        } else if let Some(ref trace_proofs) = trace_proofs {
            let (proof, call_hash) = &trace_proofs[index];
            verify_d4_matcher_at_block(rpc_url, block_hash).await?;
            let (maker, taker) = roles(&proof.trade)?;
            let maker_tier = read_account_tier_at_block(rpc_url, block_number, block_hash, maker).await.map_err(fee_error)?;
            let taker_tier = read_account_tier_at_block(rpc_url, block_number, block_hash, taker).await.map_err(fee_error)?;
            let fee = validate_intent_receipt(proof, &receipt, &policy, maker_tier, taker_tier, now_seconds)?;
            (proof.trade.clone(), fee, Some(proof.clone()), FillProvenance::signed_trace(call_hash.clone()))
        } else {
            let proof = crate::fees::v2_intent_receipt::prove_wallet_intent_fill(&tx, &receipt, &event, timestamp)?;
            verify_d4_matcher_at_block(rpc_url, block_hash).await?;
            let (maker, taker) = roles(&proof.trade)?;
            let maker_tier = read_account_tier_at_block(rpc_url, block_number, block_hash, maker).await.map_err(fee_error)?;
            let taker_tier = read_account_tier_at_block(rpc_url, block_number, block_hash, taker).await.map_err(fee_error)?;
            let fee = validate_wallet_intent_receipt(&proof, &receipt, &policy, maker_tier, taker_tier, now_seconds)?;
            (proof.trade, fee, None, FillProvenance::trace_unavailable())
        };
        verify_market_and_decimals_at_trade(rpc_url, &trade).await?;
        verified.push((trade, fee, signed, provenance));
    }
    let canonical_again = rpc(rpc_url, "eth_getBlockByNumber",
        json!([format!("0x{block_number:x}"), false])).await?;
    if !canonical_again.get("hash").and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(block_hash)) {
        repository.mark_canonical_v2_provenance_divergent(tx_hash).await?;
        return Err(fee_error("intent receipt reorg detected before persistence".into()));
    }
    // One receipt, one SQL transaction. A later fill's failure rolls back
    // all earlier fills and fees from this receipt.
    match repository.persist_canonical_v2_intent_batch_with_provenance(&verified).await {
        Err(error) => {
            if let BackendError::Persistence(message) = &error {
                if message.contains("stored fill provenance conflicts")
                    || message.contains("stored signed-intent proof diverged")
                    || message.contains("stored V2 trade diverged")
                    || message.contains("stored D2 fee diverged") {
                    repository.mark_canonical_v2_provenance_conflicting(tx_hash).await?;
                }
            }
            Err(error)
        }
        result => result,
    }
}

/// The only accepted downgrade is explicit JSON-RPC method-not-found. A
/// malformed, mismatched or failed trace is a hard error, not wallet proof.
async fn optional_transaction_trace(rpc_url: &str, tx_hash: &str) -> Result<Option<Value>> {
    let response: Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(8)).build()
        .map_err(|_| fee_error("trace RPC client unavailable".into()))?
        .post(rpc_url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"debug_traceTransaction",
            "params":[tx_hash,{"tracer":"callTracer"}]}))
        .send().await.map_err(|_| fee_error("trace RPC request failed".into()))?
        .error_for_status().map_err(|_| fee_error("trace RPC HTTP failure".into()))?
        .json().await.map_err(|_| fee_error("trace RPC response malformed".into()))?;
    if response.get("error").and_then(|error| error.get("code"))
        .and_then(Value::as_i64) == Some(-32601) {
        return Ok(None);
    }
    if response.get("error").is_some() || response.get("result").map_or(true, Value::is_null) {
        return Err(fee_error("trace RPC result unavailable or inconsistent".into()));
    }
    Ok(Some(response["result"].clone()))
}

async fn verify_d4_matcher_at_block(rpc_url: &str, block_hash: &str) -> Result<()> {
    let result = rpc(rpc_url, "eth_call", json!([
        {"to": D4_ENGINE, "data": format!("0x{}", hex::encode(&keccak256("matchingEngine()".as_bytes())[..4]))},
        {"blockHash": block_hash}
    ])).await?;
    let words = abi_result_words(&result, 1)?;
    if !word_address(&words[0])?.eq_ignore_ascii_case(D6_MATCHER) {
        return Err(fee_error("D4 matching engine was not canonical D6 at fill block".into()));
    }
    Ok(())
}

async fn verify_market_and_decimals_at_trade(
    rpc_url: &str,
    trade: &IndexedPerpTrade,
) -> Result<()> {
    let block_hash = trade
        .block_hash
        .as_deref()
        .ok_or_else(|| fee_error("indexed trade block hash missing".into()))?;
    let market_id = trade
        .market_id
        .parse::<u128>()
        .map_err(|_| fee_error("invalid market id".into()))?;
    let mut data = keccak256("getMarket(uint256)".as_bytes())[..4].to_vec();
    data.extend([0u8; 16]);
    data.extend(market_id.to_be_bytes());
    let market = rpc(
        rpc_url,
        "eth_call",
        json!([
            {"to": D1_REGISTRY, "data": format!("0x{}", hex::encode(data))},
            {"blockHash": block_hash}
        ]),
    )
    .await?;
    let words = abi_result_words(&market, 7)?;
    if !word_address(&words[1])?.eq_ignore_ascii_case(MUSDC) || word_u128(&words[4])? != 1 {
        return Err(fee_error(
            "trade market absent or settlement asset differs".into(),
        ));
    }
    let decimals = rpc(rpc_url, "eth_call", json!([
        {"to": MUSDC, "data": format!("0x{}", hex::encode(&keccak256("decimals()".as_bytes())[..4]))},
        {"blockHash": block_hash}
    ])).await?;
    if abi_result_words(&decimals, 1)?
        .first()
        .map(word_u128)
        .transpose()?
        != Some(MUSDC_DECIMALS as u128)
    {
        return Err(fee_error(
            "settlement token decimals differ from reviewed mUSDC".into(),
        ));
    }
    Ok(())
}

fn abi_result_words(result: &Value, count: usize) -> Result<Vec<[u8; 32]>> {
    let bytes = hex::decode(
        result
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .ok_or_else(|| fee_error("missing ABI result".into()))?,
    )
    .map_err(|_| fee_error("malformed ABI result".into()))?;
    if bytes.len() != count * 32 {
        return Err(fee_error("ABI result length mismatch".into()));
    }
    bytes
        .chunks_exact(32)
        .map(|chunk| {
            chunk
                .try_into()
                .map_err(|_| fee_error("invalid ABI word".into()))
        })
        .collect()
}

pub fn validate_receipt(
    trade: &IndexedPerpTrade,
    receipt: &Value,
    policy: &VerifiedD2FeePolicy,
    maker_tier: u8,
    taker_tier: u8,
    now_seconds: u64,
) -> Result<CanonicalV2PerpFee> {
    validate_receipt_mode(trade, receipt, policy, maker_tier, taker_tier, now_seconds, None)
}

/// Use only after `prove_direct_intent_fill` has verified the transaction
/// input, both signatures and the exact D6 event. Never synthesize a legacy
/// `TradeExecuted` event to enter the canonical fee ledger.
pub fn validate_intent_receipt(
    proof: &crate::fees::v2_intent_receipt::VerifiedIntentFill,
    receipt: &Value,
    policy: &VerifiedD2FeePolicy,
    maker_tier: u8,
    taker_tier: u8,
    now_seconds: u64,
) -> Result<CanonicalV2PerpFee> {
    validate_receipt_mode(&proof.trade, receipt, policy, maker_tier, taker_tier, now_seconds,
        Some((&proof.buyer_intent_hash, &proof.seller_intent_hash, &proof.event_data)))
}

pub fn validate_wallet_intent_receipt(
    proof: &crate::fees::v2_intent_receipt::VerifiedWalletIntentFill,
    receipt: &Value,
    policy: &VerifiedD2FeePolicy,
    maker_tier: u8,
    taker_tier: u8,
    now_seconds: u64,
) -> Result<CanonicalV2PerpFee> {
    validate_receipt_mode(&proof.trade, receipt, policy, maker_tier, taker_tier, now_seconds,
        Some((&proof.buyer_intent_hash, &proof.seller_intent_hash, &proof.event_data)))
}

fn validate_receipt_mode(
    trade: &IndexedPerpTrade,
    receipt: &Value,
    policy: &VerifiedD2FeePolicy,
    maker_tier: u8,
    taker_tier: u8,
    now_seconds: u64,
    intent_proof: Option<(&str, &str, &str)>,
) -> Result<CanonicalV2PerpFee> {
    policy
        .snapshot()
        .validate(now_seconds, 300)
        .map_err(fee_error)?;
    if maker_tier >= 5 || taker_tier >= 5 {
        return Err(fee_error("D2 account tier out of range".into()));
    }
    if trade.protocol_version != PerpsProtocolVersion::V2
        || !trade.emitter_address.eq_ignore_ascii_case(D6_MATCHER)
        || !is_hash(&trade.tx_hash)
        || !trade.block_hash.as_deref().is_some_and(is_hash)
    {
        return Err(fee_error("trade is not a canonical D6 V2 event".into()));
    }
    if !receipt
        .get("transactionHash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(&trade.tx_hash))
        || quantity(receipt.get("status"))? != 1
        || quantity(receipt.get("blockNumber"))? != trade.block_number
        || !receipt
            .get("blockHash")
            .and_then(Value::as_str)
            .is_some_and(|hash| {
                trade
                    .block_hash
                    .as_deref()
                    .is_some_and(|expected| hash.eq_ignore_ascii_case(expected))
            })
        || trade.block_number > policy.snapshot().block_number
        || trade.block_number <= policy.latest_execution_block()
    {
        return Err(fee_error(
            "V2 fee receipt identity or status mismatch".into(),
        ));
    }
    let logs = receipt
        .get("logs")
        .and_then(Value::as_array)
        .ok_or_else(|| fee_error("V2 receipt logs missing".into()))?;
    let mut trade_matches = 0;
    let expected_topic = intent_proof.map_or_else(trade_executed_topic0, |_| {
        topic(crate::fees::v2_intent_receipt::INTENT_EVENT_SIGNATURE)
    });
    let mut prior_trade_log_index: Option<u64> = None;
    for log in logs {
        let emitter = log.get("address").and_then(Value::as_str).unwrap_or("");
        let topic0 = log
            .get("topics")
            .and_then(Value::as_array)
            .and_then(|topics| topics.first())
            .and_then(Value::as_str)
            .unwrap_or("");
        if emitter.eq_ignore_ascii_case(D6_MATCHER)
            && (topic0.eq_ignore_ascii_case(&expected_topic)
                || topic0.eq_ignore_ascii_case(&trade_executed_topic0()))
        {
            let index = quantity(log.get("logIndex"))?;
            if index < trade.log_index {
                prior_trade_log_index =
                    Some(prior_trade_log_index.map_or(index, |old| old.max(index)));
            }
        }
    }
    let mut fee_log: Option<(u64, u128, u128)> = None;
    let charge_topic = topic(
        "FeeChargedV2(address,address,address,address,uint8,uint8,bool,int32,uint256,uint256)",
    );
    let rebate_topic =
        topic("FeeRebatedV2(address,address,address,address,uint8,uint8,int32,uint256,uint256)");
    let (maker, taker) = roles(trade)?;
    for log in logs {
        if log.get("removed").and_then(Value::as_bool) == Some(true) {
            return Err(fee_error("removed V2 receipt log".into()));
        }
        let emitter = log.get("address").and_then(Value::as_str).unwrap_or("");
        let topic0 = log
            .get("topics")
            .and_then(Value::as_array)
            .and_then(|topics| topics.first())
            .and_then(Value::as_str)
            .unwrap_or("");
        if emitter.eq_ignore_ascii_case(D6_MATCHER)
            && topic0.eq_ignore_ascii_case(&expected_topic)
        {
            let index = quantity(log.get("logIndex"))?;
            if index < trade.log_index {
                continue;
            }
            if index > trade.log_index {
                continue;
            }
            let parsed: EthLog = serde_json::from_value(log.clone())
                .map_err(|_| fee_error("malformed D6 trade log".into()))?;
            let valid_event = if let Some((buyer_hash, seller_hash, event_data)) = intent_proof {
                parsed.topics.len() == 3
                    && parsed.topics[1].eq_ignore_ascii_case(buyer_hash)
                    && parsed.topics[2].eq_ignore_ascii_case(seller_hash)
                    && parsed.data.eq_ignore_ascii_case(event_data)
            } else {
                let decoded = decode_trade_executed_log_for_emitter(&parsed, PerpsProtocolVersion::V2)?;
                decoded.onchain_intent_id == trade.onchain_intent_id
                    && decoded.market_id == trade.market_id
                    && decoded.size_delta_1e8 == trade.size_delta_1e8
                    && decoded.execution_price_1e8 == trade.execution_price_1e8
                    && decoded.buyer_is_maker == trade.buyer_is_maker
                    && decoded.buyer_nonce == trade.buyer_nonce
                    && decoded.seller_nonce == trade.seller_nonce
                    && decoded.buyer.eq_ignore_ascii_case(&trade.buyer)
                    && decoded.seller.eq_ignore_ascii_case(&trade.seller)
            };
            if parsed.transaction_hash.as_deref().is_some_and(|hash| hash.eq_ignore_ascii_case(&trade.tx_hash))
                && parsed.log_index.as_deref().and_then(|value| crate::indexer::decoder::parse_hex_quantity(value).ok()) == Some(trade.log_index)
                && parsed.block_number.as_deref().and_then(|value| crate::indexer::decoder::parse_hex_quantity(value).ok()) == Some(trade.block_number)
                && parsed.block_hash.as_deref().is_some_and(|hash| {
                    trade
                        .block_hash
                        .as_deref()
                        .is_some_and(|stored| hash.eq_ignore_ascii_case(stored))
                })
                && valid_event
            {
                trade_matches += 1;
            } else {
                return Err(fee_error("D6 trade log differs from indexed fill".into()));
            }
        }
        if emitter.eq_ignore_ascii_case(D2_ADDRESS) && topic0.eq_ignore_ascii_case(&rebate_topic) {
            return Err(fee_error("unexpected D2 rebate in V2 PERP receipt".into()));
        }
        if emitter.eq_ignore_ascii_case(D2_ADDRESS) && topic0.eq_ignore_ascii_case(&charge_topic) {
            let index = quantity(log.get("logIndex"))?;
            if index >= trade.log_index {
                continue;
            }
            // A batch can contain several D6 fills and D2 charges. Bind
            // only the charge since the preceding D6 TradeExecuted log.
            if prior_trade_log_index.is_some_and(|previous| index <= previous) {
                continue;
            }
            if fee_log.is_some() {
                return Err(fee_error(
                    "multiple D2 charges in one V2 trade receipt".into(),
                ));
            }
            if !log
                .get("transactionHash")
                .and_then(Value::as_str)
                .is_some_and(|hash| hash.eq_ignore_ascii_case(&trade.tx_hash))
                || !log
                    .get("blockHash")
                    .and_then(Value::as_str)
                    .is_some_and(|hash| {
                        trade
                            .block_hash
                            .as_deref()
                            .is_some_and(|expected| hash.eq_ignore_ascii_case(expected))
                    })
                || quantity(log.get("blockNumber"))? != trade.block_number
                || index >= trade.log_index
            {
                return Err(fee_error(
                    "D2 charge log identity or ordering mismatch".into(),
                ));
            }
            let topics = log
                .get("topics")
                .and_then(Value::as_array)
                .ok_or_else(|| fee_error("D2 charge topics missing".into()))?;
            if topics.len() != 4
                || !topic_address(&topics[1])?.eq_ignore_ascii_case(D4_ENGINE)
                || !topic_address(&topics[2])?.eq_ignore_ascii_case(taker)
                || !topic_address(&topics[3])?.eq_ignore_ascii_case(TIMELOCK_ADDRESS)
            {
                return Err(fee_error("D2 charge roles or recipient mismatch".into()));
            }
            let data = data_words(log, 7)?;
            if !word_address(&data[0])?.eq_ignore_ascii_case(MUSDC)
                || word_u128(&data[1])? != 1
                || word_u128(&data[2])? != 0
                || word_u128(&data[3])? != 0
                || word_i32(&data[4])? != 200
            {
                return Err(fee_error("D2 charge asset, flow or rate mismatch".into()));
            }
            fee_log = Some((index, word_u128(&data[5])?, word_u128(&data[6])?));
        }
    }
    if trade_matches != 1 {
        return Err(fee_error("exact D6 TradeExecuted event absent".into()));
    }
    let size = trade
        .size_delta_1e8
        .parse::<u128>()
        .map_err(|_| fee_error("invalid trade size".into()))?;
    let price = trade
        .execution_price_1e8
        .parse::<u128>()
        .map_err(|_| fee_error("invalid trade price".into()))?;
    if size == 0 || price == 0 || maker.eq_ignore_ascii_case(taker) {
        return Err(fee_error("zero-notional or self trade".into()));
    }
    // D4 floors at each step: size*price/1e8, then value*10^dec/1e8.
    // A changed Vault decimal configuration causes the receipt comparison to
    // fail rather than silently changing the fee unit.
    let notional_1e8 = size
        .checked_mul(price)
        .ok_or_else(|| fee_error("notional overflow".into()))?
        / 100_000_000;
    let basis_native = notional_1e8
        .checked_mul(10u128.pow(MUSDC_DECIMALS))
        .ok_or_else(|| fee_error("native notional overflow".into()))?
        / 100_000_000;
    let expected_fee = policy
        .taker_fee(basis_native, now_seconds, 300)
        .map_err(fee_error)?;
    let fee_log_index = match fee_log {
        Some((index, basis, amount))
            if expected_fee > 0 && basis == basis_native && amount == expected_fee =>
        {
            Some(index)
        }
        None if expected_fee == 0 => None,
        _ => return Err(fee_error("D2 canonical taker fee or basis mismatch".into())),
    };
    Ok(CanonicalV2PerpFee {
        chain_id: BASE_SEPOLIA_CHAIN_ID,
        tx_hash: trade.tx_hash.to_ascii_lowercase(),
        trade_log_index: trade.log_index,
        fee_log_index,
        block_number: trade.block_number,
        block_hash: trade
            .block_hash
            .as_ref()
            .ok_or_else(|| fee_error("indexed trade block hash missing".into()))?
            .to_ascii_lowercase(),
        market_id: trade.market_id.clone(),
        payer: taker.to_ascii_lowercase(),
        recipient: TIMELOCK_ADDRESS.into(),
        settlement_asset: MUSDC.into(),
        maker_tier,
        taker_tier,
        basis_native,
        amount_native: expected_fee,
    })
}

fn roles(trade: &IndexedPerpTrade) -> Result<(&str, &str)> {
    if trade.buyer.eq_ignore_ascii_case(&trade.seller) {
        return Err(fee_error("self trade cannot accrue D2 fee".into()));
    }
    Ok(if trade.buyer_is_maker {
        (&trade.buyer, &trade.seller)
    } else {
        (&trade.seller, &trade.buyer)
    })
}
fn topic(signature: &str) -> String {
    format!("0x{}", hex::encode(keccak256(signature.as_bytes())))
}
fn fee_error(message: String) -> BackendError {
    BackendError::Config(format!("canonical D2 fee: {message}"))
}
fn is_hash(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..].bytes().all(|c| c.is_ascii_hexdigit())
}
fn quantity(value: Option<&Value>) -> Result<u64> {
    let text = value
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix("0x"))
        .ok_or_else(|| fee_error("missing hex quantity".into()))?;
    u64::from_str_radix(text, 16).map_err(|_| fee_error("invalid hex quantity".into()))
}
fn topic_address(value: &Value) -> Result<String> {
    let bytes = fixed_32(value.as_str().unwrap_or(""))?;
    word_address(&bytes)
}
fn word_address(word: &[u8; 32]) -> Result<String> {
    if word[..12].iter().any(|byte| *byte != 0) {
        return Err(fee_error("noncanonical ABI address".into()));
    }
    Ok(format!("0x{}", hex::encode(&word[12..])))
}
fn fixed_32(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(
        value
            .strip_prefix("0x")
            .ok_or_else(|| fee_error("invalid ABI hex".into()))?,
    )
    .map_err(|_| fee_error("invalid ABI hex".into()))?;
    bytes
        .try_into()
        .map_err(|_| fee_error("invalid ABI word length".into()))
}
fn data_words(log: &Value, count: usize) -> Result<Vec<[u8; 32]>> {
    let data = log
        .get("data")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix("0x"))
        .ok_or_else(|| fee_error("D2 charge data absent".into()))?;
    let bytes = hex::decode(data).map_err(|_| fee_error("D2 charge data malformed".into()))?;
    if bytes.len() != count * 32 {
        return Err(fee_error("D2 charge data length mismatch".into()));
    }
    bytes
        .chunks_exact(32)
        .map(|word| {
            word.try_into()
                .map_err(|_| fee_error("bad ABI word".into()))
        })
        .collect()
}
fn word_u128(word: &[u8; 32]) -> Result<u128> {
    if word[..16].iter().any(|byte| *byte != 0) {
        return Err(fee_error("ABI uint exceeds u128".into()));
    }
    Ok(u128::from_be_bytes(
        word[16..]
            .try_into()
            .map_err(|_| fee_error("bad ABI uint".into()))?,
    ))
}
fn word_i32(word: &[u8; 32]) -> Result<i32> {
    let value = i32::from_be_bytes(
        word[28..]
            .try_into()
            .map_err(|_| fee_error("bad ABI int".into()))?,
    );
    if word[..28]
        .iter()
        .any(|byte| *byte != if value < 0 { 0xff } else { 0 })
    {
        return Err(fee_error("bad ABI int sign extension".into()));
    }
    Ok(value)
}
async fn rpc(url: &str, method: &str, params: Value) -> Result<Value> {
    let result = rpc_nullable(url, method, params).await?;
    if result.is_null() {
        return Err(fee_error(format!("{method} result absent")));
    }
    Ok(result)
}

/// A disappeared receipt is a valid RPC response during a reorg. The caller
/// must see null so it can invalidate persisted provenance before returning.
async fn rpc_nullable(url: &str, method: &str, params: Value) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| fee_error("RPC client unavailable".into()))?;
    let response: Value = client
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .map_err(|_| fee_error("RPC request failed".into()))?
        .error_for_status()
        .map_err(|_| fee_error("RPC HTTP failure".into()))?
        .json()
        .await
        .map_err(|_| fee_error("RPC response malformed".into()))?;
    if response.get("error").is_some() {
        return Err(fee_error(format!("{method} RPC error")));
    }
    response.get("result").cloned()
        .ok_or_else(|| fee_error(format!("{method} result absent")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fees::d2_onchain::{approved_taker_fee, D2Snapshot};

    const BUYER: &str = "0x1111111111111111111111111111111111111111";
    const SELLER: &str = "0x2222222222222222222222222222222222222222";
    fn hash(byte: &str) -> String {
        format!("0x{}", byte.repeat(32))
    }
    fn word(value: u128) -> String {
        format!("{value:064x}")
    }
    fn address_word(value: &str) -> String {
        format!("{}{}", "0".repeat(24), &value[2..])
    }
    fn trade() -> IndexedPerpTrade {
        let tx = hash("bb");
        IndexedPerpTrade {
            event_id: format!("{tx}:3"),
            tx_hash: tx,
            log_index: 3,
            block_number: 100,
            block_hash: Some(hash("aa")),
            onchain_intent_id: Some(hash("cc")),
            buyer: BUYER.into(),
            seller: SELLER.into(),
            market_id: "1".into(),
            size_delta_1e8: "100000000".into(),
            execution_price_1e8: "10000000000".into(),
            buyer_is_maker: true,
            buyer_nonce: "1".into(),
            seller_nonce: "2".into(),
            created_at_ms: 1000,
            protocol_version: PerpsProtocolVersion::V2,
            emitter_address: D6_MATCHER.into(),
        }
    }
    fn policy() -> VerifiedD2FeePolicy {
        VerifiedD2FeePolicy::synthetic_for_tests(D2Snapshot {
            chain_id: BASE_SEPOLIA_CHAIN_ID,
            block_number: 200,
            block_hash: hash("dd"),
            block_timestamp: 1000,
            contract: D2_ADDRESS.into(),
            owner: TIMELOCK_ADDRESS.into(),
            recipient: TIMELOCK_ADDRESS.into(),
            rebate_funding_account: "0x0000000000000000000000000000000000000000".into(),
            protocol_fee_vault: "0x0000000000000000000000000000000000000000".into(),
            profiles: [(0, 200); 5],
            rfq_discounts: [(0, 0); 5],
        })
    }
    fn receipt(trade: &IndexedPerpTrade) -> Value {
        let size = trade.size_delta_1e8.parse::<u128>().unwrap();
        let price = trade.execution_price_1e8.parse::<u128>().unwrap();
        let buyer_nonce = trade.buyer_nonce.parse::<u128>().unwrap();
        let seller_nonce = trade.seller_nonce.parse::<u128>().unwrap();
        let basis = (size * price / 100_000_000) * 1_000_000 / 100_000_000;
        let amount = approved_taker_fee(basis).unwrap();
        json!({
            "transactionHash": trade.tx_hash, "status": "0x1", "blockNumber": "0x64",
            "blockHash": trade.block_hash, "logs": [
                {"address": D2_ADDRESS, "topics": [
                    topic("FeeChargedV2(address,address,address,address,uint8,uint8,bool,int32,uint256,uint256)"),
                    format!("0x{}", address_word(D4_ENGINE)),
                    format!("0x{}", address_word(SELLER)),
                    format!("0x{}", address_word(TIMELOCK_ADDRESS))],
                 "data": format!("0x{}{}{}{}{}{}{}", address_word(MUSDC), word(1), word(0),
                    word(0), word(200), word(basis), word(amount)),
                 "logIndex": "0x2", "transactionHash": trade.tx_hash,
                 "blockNumber": "0x64", "blockHash": trade.block_hash},
                {"address": D6_MATCHER, "topics": [trade_executed_topic0(), trade.onchain_intent_id,
                    format!("0x{}", address_word(BUYER)),
                    format!("0x{}", address_word(SELLER))],
                 "data": format!("0x{}{}{}{}{}{}", word(1), word(size),
                    word(price), word(1), word(buyer_nonce), word(seller_nonce)),
                 "logIndex": "0x3", "transactionHash": trade.tx_hash,
                 "blockNumber": "0x64", "blockHash": trade.block_hash}
            ]
        })
    }

    #[test]
    fn exact_canonical_charge_and_all_account_tiers() {
        let trade = trade();
        for tier in 0..5 {
            let fee =
                validate_receipt(&trade, &receipt(&trade), &policy(), tier, tier, 1010).unwrap();
            assert_eq!(fee.basis_native, 100_000_000);
            assert_eq!(fee.amount_native, 20_000);
            assert_eq!(fee.fee_log_index, Some(2));
            assert_eq!(fee.payer, SELLER);
        }
    }

    #[test]
    fn registry_market_tuple_and_decimal_abi_are_exact() {
        let market = json!(format!(
            "0x{}{}{}{}{}{}{}",
            address_word(BUYER),
            address_word(MUSDC),
            address_word(SELLER),
            word(0),
            word(1),
            word(1),
            word(0)
        ));
        let words = abi_result_words(&market, 7).unwrap();
        assert_eq!(word_address(&words[1]).unwrap(), MUSDC);
        assert_eq!(word_u128(&words[4]).unwrap(), 1);
        assert_eq!(
            word_u128(&abi_result_words(&json!(format!("0x{}", word(6))), 1).unwrap()[0]).unwrap(),
            6
        );
        assert!(abi_result_words(&json!("0x00"), 7).is_err());
    }

    #[test]
    fn partial_fills_round_independently_and_zero_fees_need_no_charge_log() {
        let mut partial = trade();
        partial.size_delta_1e8 = "25000000".into();
        let fee = validate_receipt(&partial, &receipt(&partial), &policy(), 1, 4, 1010).unwrap();
        assert_eq!((fee.basis_native, fee.amount_native), (25_000_000, 5_000));
        let mut tiny = trade();
        tiny.size_delta_1e8 = "1".into();
        tiny.execution_price_1e8 = "100000000".into();
        let mut zero_receipt = receipt(&tiny);
        zero_receipt["logs"].as_array_mut().unwrap().remove(0);
        let fee = validate_receipt(&tiny, &zero_receipt, &policy(), 0, 0, 1010).unwrap();
        assert_eq!(
            (fee.basis_native, fee.amount_native, fee.fee_log_index),
            (0, 0, None)
        );
    }

    #[test]
    fn malformed_or_drifted_receipts_fail_closed() {
        let trade = trade();
        let original = receipt(&trade);
        let mut failed = original.clone();
        failed["status"] = json!("0x0");
        assert!(validate_receipt(&trade, &failed, &policy(), 0, 0, 1010).is_err());
        let mut failed = original.clone();
        failed["blockHash"] = json!(hash("ef"));
        assert!(validate_receipt(&trade, &failed, &policy(), 0, 0, 1010).is_err());
        let mut failed = original.clone();
        failed["logs"][0]["data"] = json!(format!(
            "0x{}{}{}{}{}{}{}",
            address_word(MUSDC),
            word(1),
            word(0),
            word(0),
            word(200),
            word(100_000_000),
            word(20_001)
        ));
        assert!(validate_receipt(&trade, &failed, &policy(), 0, 0, 1010).is_err());
        let mut failed = original.clone();
        failed["logs"][0]["topics"][3] = json!(format!("0x{}", address_word(BUYER)));
        assert!(validate_receipt(&trade, &failed, &policy(), 0, 0, 1010).is_err());
        let mut failed = original.clone();
        failed["logs"]
            .as_array_mut()
            .unwrap()
            .push(original["logs"][0].clone());
        assert!(validate_receipt(&trade, &failed, &policy(), 0, 0, 1010).is_err());
        let mut failed = original.clone();
        failed["logs"].as_array_mut().unwrap().pop();
        assert!(validate_receipt(&trade, &failed, &policy(), 0, 0, 1010).is_err());
        assert!(validate_receipt(&trade, &original, &policy(), 5, 0, 1010).is_err());
        assert!(validate_receipt(&trade, &original, &policy(), 0, 0, 1400).is_err());
        let mut bad_trade = trade.clone();
        bad_trade.protocol_version = PerpsProtocolVersion::V1;
        assert!(validate_receipt(&bad_trade, &original, &policy(), 0, 0, 1010).is_err());
        let mut missing_block_hash = trade.clone();
        missing_block_hash.block_hash = None;
        assert!(validate_receipt(&missing_block_hash, &original, &policy(), 0, 0, 1010).is_err());
        let mut pre_policy_trade = trade.clone();
        pre_policy_trade.block_number = 50;
        let mut pre_policy_receipt = receipt(&pre_policy_trade);
        pre_policy_receipt["blockNumber"] = json!("0x32");
        for log in pre_policy_receipt["logs"].as_array_mut().unwrap() {
            log["blockNumber"] = json!("0x32");
        }
        assert!(validate_receipt(
            &pre_policy_trade,
            &pre_policy_receipt,
            &policy(),
            0,
            0,
            1010
        )
        .is_err());
    }

    #[test]
    fn batched_receipt_binds_each_charge_to_its_own_trade() {
        let first = trade();
        let mut second = first.clone();
        second.log_index = 6;
        second.event_id = format!("{}:6", second.tx_hash);
        second.onchain_intent_id = Some(hash("de"));
        second.buyer_nonce = "3".into();
        second.seller_nonce = "4".into();
        let mut batch = receipt(&first);
        let mut other = receipt(&second);
        other["logs"][0]["logIndex"] = json!("0x5");
        other["logs"][1]["logIndex"] = json!("0x6");
        batch["logs"]
            .as_array_mut()
            .unwrap()
            .extend(other["logs"].as_array().unwrap().clone());
        let first_fee = validate_receipt(&first, &batch, &policy(), 0, 0, 1010).unwrap();
        let second_fee = validate_receipt(&second, &batch, &policy(), 0, 0, 1010).unwrap();
        assert_eq!(first_fee.fee_log_index, Some(2));
        assert_eq!(second_fee.fee_log_index, Some(5));
        let mut duplicate = batch.clone();
        duplicate["logs"]
            .as_array_mut()
            .unwrap()
            .insert(2, batch["logs"][0].clone());
        assert!(validate_receipt(&first, &duplicate, &policy(), 0, 0, 1010).is_err());
    }

    #[test]
    fn intent_hash_event_alone_never_becomes_wallet_or_subaccount_fee_history() {
        let trade = trade();
        let mut intent_only = receipt(&trade);
        intent_only["logs"][1]["topics"] = json!([
            topic("TradeExecutedFromIntents(bytes32,bytes32,uint256,uint128,uint128,uint256)"),
            hash("cd"),
            hash("ef")
        ]);
        assert!(validate_receipt(&trade, &intent_only, &policy(), 0, 0, 1010).is_err());
    }

    #[tokio::test]
    async fn postgres_atomic_retry_reorg_and_divergence() {
        let Ok(url) = std::env::var("D2_ATOMIC_PG_TEST_URL") else {
            return;
        };
        assert!(
            url.ends_with("/deopt_fee_atomic_test"),
            "dedicated disposable DB required"
        );
        let repository = PgRepository::connect(&url).await.unwrap();
        sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
            .execute(repository.pool())
            .await
            .unwrap();
        sqlx::query("CREATE SCHEMA public")
            .execute(repository.pool())
            .await
            .unwrap();
        repository.run_migrations().await.unwrap();
        let trade = trade();
        let fee = validate_receipt(&trade, &receipt(&trade), &policy(), 0, 4, 1010).unwrap();
        assert!(repository
            .persist_canonical_v2_perp_fee(&trade, &fee)
            .await
            .unwrap());
        assert!(!repository
            .persist_canonical_v2_perp_fee(&trade, &fee)
            .await
            .unwrap());
        let restarted = PgRepository::connect(&url).await.unwrap();
        assert!(!restarted
            .persist_canonical_v2_perp_fee(&trade, &fee)
            .await
            .unwrap());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM canonical_v2_perp_fees")
            .fetch_one(repository.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
        let buyer_history = repository
            .list_canonical_v2_perp_history(BUYER, 10)
            .await
            .unwrap();
        let seller_history = repository
            .list_canonical_v2_perp_history(SELLER, 10)
            .await
            .unwrap();
        assert_eq!(buyer_history.len(), 1);
        assert_eq!(seller_history.len(), 1);
        assert_eq!(buyer_history[0].role, "maker");
        assert_eq!(buyer_history[0].fee_native, "0");
        assert_eq!(seller_history[0].role, "taker");
        assert_eq!(seller_history[0].fee_native, "20000");
        assert_eq!(seller_history[0].subaccount_id, None);
        assert_eq!(
            seller_history[0].attribution,
            "wallet_only_unresolved_subaccount"
        );
        for id in [1_i32, 2_i32] {
            sqlx::query("INSERT INTO subaccounts (owner_address, subaccount_id, created_at_ms, updated_at_ms) VALUES ($1,$2,0,0)")
                .bind(BUYER)
                .bind(id)
                .execute(repository.pool())
                .await
                .unwrap();
        }
        let after_local_accounts = repository
            .list_canonical_v2_perp_history(BUYER, 10)
            .await
            .unwrap();
        assert_eq!(after_local_accounts.len(), 1);
        assert_eq!(after_local_accounts[0].subaccount_id, None);
        use tower::ServiceExt;
        let mut state =
            crate::api::AppState::new(crate::engine::EngineState::with_default_markets());
        state.repository = Some(repository.clone());
        let response = crate::api::router(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/accounts/{BUYER}/perps/canonical-fees"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let api: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(api["identity_scope"], "wallet");
        assert_eq!(api["subaccount_attribution"], "unresolved");
        assert_eq!(api["fees"].as_array().unwrap().len(), 1);
        assert!(api["fees"][0]["subaccount_id"].is_null());
        assert_eq!(api["fees"][0]["fee_native"], "0");
        assert!(repository
            .list_canonical_v2_perp_history("not-a-wallet", 10)
            .await
            .is_err());
        assert!(repository
            .list_canonical_v2_perp_history("0x3333333333333333333333333333333333333333", 10)
            .await
            .unwrap()
            .is_empty());
        let legacy: i64 = sqlx::query_scalar("SELECT count(*) FROM fee_events")
            .fetch_one(repository.pool())
            .await
            .unwrap();
        assert_eq!(legacy, 0);
        let mut drifted = fee.clone();
        drifted.amount_native += 1;
        assert!(repository
            .persist_canonical_v2_perp_fee(&trade, &drifted)
            .await
            .is_err());
        let mut reorg = trade.clone();
        reorg.block_hash = Some(hash("ee"));
        let mut reorg_fee = fee.clone();
        reorg_fee.block_hash = hash("ee");
        assert!(repository
            .persist_canonical_v2_perp_fee(&reorg, &reorg_fee)
            .await
            .is_err());
        let count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM canonical_v2_perp_fees")
            .fetch_one(repository.pool())
            .await
            .unwrap();
        assert_eq!(count_after, 1);
        let mut other = trade.clone();
        other.tx_hash = hash("ab");
        other.event_id = format!("{}:3", other.tx_hash);
        other.onchain_intent_id = Some(hash("ac"));
        let other_fee = validate_receipt(&other, &receipt(&other), &policy(), 0, 2, 1010).unwrap();
        let (first, second) = tokio::join!(
            repository.persist_canonical_v2_perp_fee(&other, &other_fee),
            repository.persist_canonical_v2_perp_fee(&other, &other_fee)
        );
        assert_eq!(
            [first.unwrap(), second.unwrap()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            1
        );
        let mut conflicting_trade = other.clone();
        conflicting_trade.log_index = 4;
        conflicting_trade.event_id = format!("{}:4", conflicting_trade.tx_hash);
        let mut conflicting_fee = other_fee.clone();
        conflicting_fee.trade_log_index = 4;
        assert!(repository
            .persist_canonical_v2_perp_fee(&conflicting_trade, &conflicting_fee)
            .await
            .is_err());
        let orphan: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM indexed_perp_trades WHERE tx_hash=$1 AND log_index=4",
        )
        .bind(&other.tx_hash)
        .fetch_one(repository.pool())
        .await
        .unwrap();
        assert_eq!(orphan, 0);
        let mut second_in_same_tx = trade.clone();
        second_in_same_tx.log_index = 6;
        second_in_same_tx.event_id = format!("{}:6", second_in_same_tx.tx_hash);
        second_in_same_tx.onchain_intent_id = Some(hash("de"));
        second_in_same_tx.buyer_nonce = "3".into();
        second_in_same_tx.seller_nonce = "4".into();
        let mut batch = receipt(&trade);
        let mut second_receipt = receipt(&second_in_same_tx);
        second_receipt["logs"][0]["logIndex"] = json!("0x5");
        second_receipt["logs"][1]["logIndex"] = json!("0x6");
        batch["logs"]
            .as_array_mut()
            .unwrap()
            .extend(second_receipt["logs"].as_array().unwrap().clone());
        let second_fee =
            validate_receipt(&second_in_same_tx, &batch, &policy(), 0, 0, 1010).unwrap();
        assert_eq!(second_fee.fee_log_index, Some(5));
        assert!(repository
            .persist_canonical_v2_perp_fee(&second_in_same_tx, &second_fee)
            .await
            .unwrap());
        assert!(!repository
            .persist_canonical_v2_perp_fee(&second_in_same_tx, &second_fee)
            .await
            .unwrap());
        let same_tx_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM canonical_v2_perp_fees WHERE tx_hash=$1")
                .bind(&trade.tx_hash)
                .fetch_one(repository.pool())
                .await
                .unwrap();
        assert_eq!(same_tx_count, 2);
    }
}
