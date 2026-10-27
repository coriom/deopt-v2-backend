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
            && topic0.eq_ignore_ascii_case(&trade_executed_topic0())
        {
            let parsed: EthLog = serde_json::from_value(log.clone())
                .map_err(|_| fee_error("malformed D6 TradeExecuted log".into()))?;
            let parsed = decode_trade_executed_log_for_emitter(&parsed, PerpsProtocolVersion::V2)?;
            if parsed.tx_hash.eq_ignore_ascii_case(&trade.tx_hash)
                && parsed.log_index == trade.log_index
                && parsed.block_number == trade.block_number
                && parsed.block_hash.as_deref().is_some_and(|hash| {
                    trade
                        .block_hash
                        .as_deref()
                        .is_some_and(|stored| hash.eq_ignore_ascii_case(stored))
                })
                && parsed.onchain_intent_id == trade.onchain_intent_id
                && parsed.market_id == trade.market_id
                && parsed.size_delta_1e8 == trade.size_delta_1e8
                && parsed.execution_price_1e8 == trade.execution_price_1e8
                && parsed.buyer_is_maker == trade.buyer_is_maker
                && parsed.buyer_nonce == trade.buyer_nonce
                && parsed.seller_nonce == trade.seller_nonce
                && parsed.buyer.eq_ignore_ascii_case(&trade.buyer)
                && parsed.seller.eq_ignore_ascii_case(&trade.seller)
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
                || quantity(log.get("logIndex"))? >= trade.log_index
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
            fee_log = Some((
                quantity(log.get("logIndex"))?,
                word_u128(&data[5])?,
                word_u128(&data[6])?,
            ));
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
    response
        .get("result")
        .filter(|value| !value.is_null())
        .cloned()
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
                    word(price), word(1), word(1), word(2)),
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
    }
}
