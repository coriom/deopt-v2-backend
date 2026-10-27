//! Proof for a direct D6 `executeTradeFromIntents` transaction.
//!
//! The D6 event contains intent digests, not wallets or subaccounts. Those
//! identities are recovered from the transaction input only after both EIP-712
//! signatures and the event digests agree. The signed subaccount numbers are
//! order-origin metadata; D4 settles by wallet and does not maintain separate
//! on-chain subaccount balances.
use crate::error::{BackendError, Result};
use crate::execution::perp_order_intent::{
    perp_order_intent_digest, validate_shape, verify_perp_order_intent, PerpOrderIntent,
};
use crate::execution::perp_trade::{PerpTradeDomain, PerpsProtocolVersion};
use crate::fees::v2_perp_settlement::D6_MATCHER;
use crate::indexer::decoder::parse_hex_quantity;
use crate::indexer::{EthLog, IndexedPerpTrade};
use crate::signing::eip712::keccak256;
use crate::types::{now_ms, AccountId};
use alloy_primitives::U256;
use alloy_sol_types::{sol, SolCall};
use serde_json::Value;

sol! {
    struct SignedIntent {
        bytes32 intentId;
        address trader;
        uint32 subaccountId;
        uint256 marketId;
        uint8 side;
        uint128 size1e8;
        uint128 limitPrice1e8;
        uint128 maxExecPrice1e8;
        uint128 minExecPrice1e8;
        uint256 nonce;
        uint256 deadline;
    }
    struct IntentTrade {
        uint256 marketId;
        uint32 buyerSubaccountId;
        uint32 sellerSubaccountId;
        uint128 size1e8;
        uint128 executionPrice1e8;
        bool buyerIsMaker;
        bytes32 buyerIntentId;
        bytes32 sellerIntentId;
    }
    function executeTradeFromIntents(
        SignedIntent buyerIntent, bytes buyerSig,
        SignedIntent sellerIntent, bytes sellerSig,
        IntentTrade trade
    );
}

pub const INTENT_EVENT_SIGNATURE: &str =
    "TradeExecutedFromIntents(bytes32,bytes32,uint256,uint128,uint128,uint256)";

#[derive(Clone, Debug)]
pub struct VerifiedIntentFill {
    pub trade: IndexedPerpTrade,
    pub buyer_intent_hash: String,
    pub seller_intent_hash: String,
    pub event_data: String,
    /// Signed order-origin labels. Not canonical economic subaccount balances.
    pub buyer_signed_subaccount_id: u32,
    pub seller_signed_subaccount_id: u32,
}

fn invalid(reason: &str) -> BackendError {
    BackendError::Indexer(format!("D6 signed-intent receipt: {reason}"))
}

fn uint(value: U256, field: &str) -> Result<u128> {
    u128::try_from(value).map_err(|_| BackendError::Indexer(format!("{field} exceeds u128")))
}

fn intent(value: SignedIntent) -> Result<PerpOrderIntent> {
    Ok(PerpOrderIntent {
        intent_id: value.intentId,
        trader: AccountId(format!("{:#x}", value.trader)),
        subaccount_id: value.subaccountId,
        market_id: uint(value.marketId, "marketId")?,
        side: value.side,
        size_1e8: value.size1e8,
        limit_price_1e8: value.limitPrice1e8,
        max_exec_price_1e8: value.maxExecPrice1e8,
        min_exec_price_1e8: value.minExecPrice1e8,
        nonce: uint(value.nonce, "nonce")?,
        deadline: uint(value.deadline, "deadline")?,
    })
}

fn word(data: &[u8], index: usize) -> Result<U256> {
    let start = index * 32;
    let end = start + 32;
    let bytes = data
        .get(start..end)
        .ok_or_else(|| invalid("event data truncated"))?;
    Ok(U256::from_be_slice(bytes))
}

fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(field))
}

fn digest_hex(digest: [u8; 32]) -> String {
    format!("0x{}", hex::encode(digest))
}

/// Verify a *direct* D6 call and one exact event. A wrapper or a receipt with
/// multiple intent events requires independently authenticated call traces;
/// no event is attributed by order or by matching a convenient first log.
pub fn prove_direct_intent_fill(
    transaction: &Value,
    receipt: &Value,
    event: &EthLog,
    block_timestamp: u64,
) -> Result<VerifiedIntentFill> {
    if parse_hex_quantity(required(transaction, "chainId")?)? != 84532
        || parse_hex_quantity(required(transaction, "value")?)? != 0
        || !required(transaction, "to")?.eq_ignore_ascii_case(D6_MATCHER)
        || !event.address.eq_ignore_ascii_case(D6_MATCHER)
        || !required(transaction, "hash")?
            .eq_ignore_ascii_case(required(receipt, "transactionHash")?)
        || !required(transaction, "blockHash")?
            .eq_ignore_ascii_case(required(receipt, "blockHash")?)
        || parse_hex_quantity(required(transaction, "blockNumber")?)?
            != parse_hex_quantity(required(receipt, "blockNumber")?)?
        || !required(transaction, "hash")?.eq_ignore_ascii_case(
            event
                .transaction_hash
                .as_deref()
                .ok_or_else(|| invalid("event transactionHash"))?,
        )
        || parse_hex_quantity(required(receipt, "status")?)? != 1
        || !required(receipt, "blockHash")?.eq_ignore_ascii_case(
            event
                .block_hash
                .as_deref()
                .ok_or_else(|| invalid("event blockHash"))?,
        )
        || parse_hex_quantity(required(receipt, "blockNumber")?)?
            != parse_hex_quantity(
                event
                    .block_number
                    .as_deref()
                    .ok_or_else(|| invalid("event blockNumber"))?,
            )?
    {
        return Err(invalid(
            "chain, emitter or canonical receipt identity mismatch",
        ));
    }
    let input = hex::decode(
        required(transaction, "input")?
            .strip_prefix("0x")
            .ok_or_else(|| invalid("input hex"))?,
    )
    .map_err(|_| invalid("input hex"))?;
    let call = executeTradeFromIntentsCall::abi_decode(&input, true)
        .map_err(|_| invalid("calldata is not canonical executeTradeFromIntents"))?;
    if call.abi_encode() != input {
        return Err(invalid("noncanonical calldata or trailing bytes"));
    }
    let buyer = intent(call.buyerIntent)?;
    let seller = intent(call.sellerIntent)?;
    let trade = call.trade;
    let domain = PerpTradeDomain::new_v2(84532, AccountId(D6_MATCHER.into()));
    validate_shape(&buyer)?;
    validate_shape(&seller)?;
    verify_perp_order_intent(
        &buyer,
        &domain,
        &format!("0x{}", hex::encode(call.buyerSig)),
    )?;
    verify_perp_order_intent(
        &seller,
        &domain,
        &format!("0x{}", hex::encode(call.sellerSig)),
    )?;
    if buyer.side != 0
        || seller.side != 1
        || buyer.trader.0.eq_ignore_ascii_case(&seller.trader.0)
        || buyer.market_id != seller.market_id
        || buyer.market_id != uint(trade.marketId, "trade.marketId")?
        || buyer.subaccount_id != trade.buyerSubaccountId
        || seller.subaccount_id != trade.sellerSubaccountId
        || buyer.intent_id != trade.buyerIntentId
        || seller.intent_id != trade.sellerIntentId
        || trade.size1e8 == 0
        || trade.executionPrice1e8 == 0
        || trade.size1e8 > buyer.size_1e8
        || trade.size1e8 > seller.size_1e8
        || block_timestamp as u128 > buyer.deadline
        || block_timestamp as u128 > seller.deadline
        || trade.executionPrice1e8 > buyer.max_exec_price_1e8
        || trade.executionPrice1e8 < seller.min_exec_price_1e8
        || (buyer.limit_price_1e8 != 0 && trade.executionPrice1e8 > buyer.limit_price_1e8)
        || (seller.limit_price_1e8 != 0 && trade.executionPrice1e8 < seller.limit_price_1e8)
    {
        return Err(invalid("signed intent and trade arguments disagree"));
    }
    let buyer_hash = digest_hex(perp_order_intent_digest(&buyer, &domain)?);
    let seller_hash = digest_hex(perp_order_intent_digest(&seller, &domain)?);
    if event.topics.len() != 3
        || !event.topics[0]
            .eq_ignore_ascii_case(&digest_hex(keccak256(INTENT_EVENT_SIGNATURE.as_bytes())))
        || !event.topics[1].eq_ignore_ascii_case(&buyer_hash)
        || !event.topics[2].eq_ignore_ascii_case(&seller_hash)
    {
        return Err(invalid("event intent hashes differ from signed calldata"));
    }
    let event_data = hex::decode(
        event
            .data
            .strip_prefix("0x")
            .ok_or_else(|| invalid("event data"))?,
    )
    .map_err(|_| invalid("event data"))?;
    if event_data.len() != 4 * 32
        || word(&event_data, 0)? != trade.marketId
        || word(&event_data, 1)? != U256::from(trade.size1e8)
        || word(&event_data, 2)? != U256::from(trade.executionPrice1e8)
        || word(&event_data, 3)? != U256::from(block_timestamp)
    {
        return Err(invalid(
            "event market, fill size, price or timestamp differs",
        ));
    }
    let event_index = parse_hex_quantity(
        event
            .log_index
            .as_deref()
            .ok_or_else(|| invalid("event logIndex"))?,
    )?;
    let transaction_hash = required(transaction, "hash")?;
    let receipt_block_hash = required(receipt, "blockHash")?;
    let event_occurrences = receipt
        .get("logs")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("receipt logs"))?
        .iter()
        .filter(|log| {
            log.get("address")
                .and_then(Value::as_str)
                .is_some_and(|address| address.eq_ignore_ascii_case(D6_MATCHER))
                && log
                    .get("topics")
                    .and_then(Value::as_array)
                    .and_then(|topics| topics.first())
                    .and_then(Value::as_str)
                    .is_some_and(|topic| topic.eq_ignore_ascii_case(&event.topics[0]))
        })
        .count();
    if event_occurrences != 1
        || !receipt["logs"].as_array().unwrap().iter().any(|log| {
            log.get("address")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(D6_MATCHER))
                && log
                    .get("transactionHash")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.eq_ignore_ascii_case(transaction_hash))
                && log
                    .get("blockHash")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.eq_ignore_ascii_case(receipt_block_hash))
                && log
                    .get("logIndex")
                    .and_then(Value::as_str)
                    .and_then(|value| parse_hex_quantity(value).ok())
                    == Some(event_index)
                && log
                    .get("data")
                    .and_then(Value::as_str)
                    .is_some_and(|v| v.eq_ignore_ascii_case(&event.data))
                && log
                    .get("topics")
                    .and_then(Value::as_array)
                    .is_some_and(|topics| {
                        topics.len() == 3
                            && topics[0]
                                .as_str()
                                .is_some_and(|v| v.eq_ignore_ascii_case(&event.topics[0]))
                            && topics[1]
                                .as_str()
                                .is_some_and(|v| v.eq_ignore_ascii_case(&buyer_hash))
                            && topics[2]
                                .as_str()
                                .is_some_and(|v| v.eq_ignore_ascii_case(&seller_hash))
                    })
        })
    {
        return Err(invalid("missing or ambiguous intent event in receipt"));
    }
    let tx_hash = required(transaction, "hash")?.to_ascii_lowercase();
    let block_number = parse_hex_quantity(required(receipt, "blockNumber")?)?;
    Ok(VerifiedIntentFill {
        trade: IndexedPerpTrade {
            event_id: format!("{tx_hash}:{event_index}"),
            tx_hash,
            log_index: event_index,
            block_number,
            block_hash: Some(required(receipt, "blockHash")?.to_ascii_lowercase()),
            onchain_intent_id: None,
            buyer: buyer.trader.0.to_ascii_lowercase(),
            seller: seller.trader.0.to_ascii_lowercase(),
            market_id: buyer.market_id.to_string(),
            size_delta_1e8: trade.size1e8.to_string(),
            execution_price_1e8: trade.executionPrice1e8.to_string(),
            buyer_is_maker: trade.buyerIsMaker,
            buyer_nonce: buyer.nonce.to_string(),
            seller_nonce: seller.nonce.to_string(),
            created_at_ms: now_ms(),
            protocol_version: PerpsProtocolVersion::V2,
            emitter_address: D6_MATCHER.into(),
        },
        buyer_intent_hash: buyer_hash,
        seller_intent_hash: seller_hash,
        event_data: event.data.to_ascii_lowercase(),
        buyer_signed_subaccount_id: buyer.subaccount_id,
        seller_signed_subaccount_id: seller.subaccount_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, B256};
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature, SigningKey};
    use serde_json::json;

    fn actor() -> (SigningKey, Address) {
        // Ephemeral test signer: no reusable private key in source or evidence.
        let key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let encoded = key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&encoded.as_bytes()[1..]);
        (key, Address::from_slice(&hash[12..]))
    }

    fn signed(value: &SignedIntent, key: &SigningKey) -> Bytes {
        let intent = intent(value.clone()).unwrap();
        let domain = PerpTradeDomain::new_v2(84532, AccountId(D6_MATCHER.into()));
        let hash = perp_order_intent_digest(&intent, &domain).unwrap();
        let (signature, recovery): (Signature, RecoveryId) = key.sign_prehash(&hash).unwrap();
        let mut out = signature.to_bytes().to_vec();
        out.push(recovery.to_byte());
        Bytes::from(out)
    }

    fn fixture() -> (Value, Value, EthLog) {
        let (buyer_key, buyer_address) = actor();
        let (seller_key, seller_address) = actor();
        let buyer = SignedIntent {
            intentId: B256::repeat_byte(1),
            trader: buyer_address,
            subaccountId: 7,
            marketId: U256::from(1),
            side: 0,
            size1e8: 200_000_000,
            limitPrice1e8: 0,
            maxExecPrice1e8: 320_000_000_000,
            minExecPrice1e8: 0,
            nonce: U256::from(11),
            deadline: U256::from(2000),
        };
        let seller = SignedIntent {
            intentId: B256::repeat_byte(2),
            trader: seller_address,
            subaccountId: 9,
            marketId: U256::from(1),
            side: 1,
            size1e8: 100_000_000,
            limitPrice1e8: 0,
            maxExecPrice1e8: 0,
            minExecPrice1e8: 280_000_000_000,
            nonce: U256::from(12),
            deadline: U256::from(2000),
        };
        let buyer_hash = digest_hex(
            perp_order_intent_digest(
                &intent(buyer.clone()).unwrap(),
                &PerpTradeDomain::new_v2(84532, AccountId(D6_MATCHER.into())),
            )
            .unwrap(),
        );
        let seller_hash = digest_hex(
            perp_order_intent_digest(
                &intent(seller.clone()).unwrap(),
                &PerpTradeDomain::new_v2(84532, AccountId(D6_MATCHER.into())),
            )
            .unwrap(),
        );
        let call = executeTradeFromIntentsCall {
            buyerSig: signed(&buyer, &buyer_key),
            sellerSig: signed(&seller, &seller_key),
            buyerIntent: buyer,
            sellerIntent: seller,
            trade: IntentTrade {
                marketId: U256::from(1),
                buyerSubaccountId: 7,
                sellerSubaccountId: 9,
                size1e8: 50_000_000,
                executionPrice1e8: 300_000_000_000,
                buyerIsMaker: true,
                buyerIntentId: B256::repeat_byte(1),
                sellerIntentId: B256::repeat_byte(2),
            },
        };
        let mut data = Vec::new();
        for number in [1u128, 50_000_000, 300_000_000_000, 1000] {
            data.extend_from_slice(&U256::from(number).to_be_bytes::<32>());
        }
        let tx_hash = format!("0x{}", "11".repeat(32));
        let block_hash = format!("0x{}", "22".repeat(32));
        let event = EthLog {
            address: D6_MATCHER.into(),
            topics: vec![
                digest_hex(keccak256(INTENT_EVENT_SIGNATURE.as_bytes())),
                buyer_hash,
                seller_hash,
            ],
            data: format!("0x{}", hex::encode(data)),
            block_number: Some("0x64".into()),
            block_hash: Some(block_hash.clone()),
            transaction_hash: Some(tx_hash.clone()),
            log_index: Some("0x2".into()),
        };
        let tx = json!({"chainId":"0x14a34","to":D6_MATCHER,"hash":tx_hash,
            "blockHash":block_hash,"blockNumber":"0x64","value":"0x0",
            "input":format!("0x{}", hex::encode(call.abi_encode()))});
        let receipt = json!({"transactionHash":tx_hash,"status":"0x1",
            "blockNumber":"0x64","blockHash":block_hash,"logs":[event]});
        (tx, receipt, event)
    }

    #[test]
    fn frozen_d6_abi_selector_and_event_topic_match() {
        // Selector independently derived from the frozen Solidity ABI.
        assert_eq!(
            executeTradeFromIntentsCall::SELECTOR,
            [0xb6, 0x94, 0x59, 0xb0]
        );
        assert_eq!(
            keccak256(INTENT_EVENT_SIGNATURE.as_bytes()).as_slice(),
            &crate::execution::broadcast_policy::PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0,
        );
    }

    #[test]
    fn signed_direct_receipt_proves_wallets_and_origin_ids() {
        let (tx, receipt, event) = fixture();
        let proof = prove_direct_intent_fill(&tx, &receipt, &event, 1000).unwrap();
        assert_eq!(proof.trade.size_delta_1e8, "50000000");
        assert_eq!(proof.buyer_signed_subaccount_id, 7);
        assert_eq!(proof.seller_signed_subaccount_id, 9);
        assert_ne!(proof.trade.buyer, proof.trade.seller);
    }

    #[test]
    fn rejects_tampering_and_ambiguous_receipt() {
        let (tx, receipt, event) = fixture();
        let mut tampered = tx.clone();
        tampered["chainId"] = json!("0x1");
        assert!(prove_direct_intent_fill(&tampered, &receipt, &event, 1000).is_err());
        let mut tampered_event = event.clone();
        tampered_event.topics[1] = format!("0x{}", "00".repeat(32));
        assert!(prove_direct_intent_fill(&tx, &receipt, &tampered_event, 1000).is_err());
        let mut ambiguous = receipt.clone();
        ambiguous["logs"]
            .as_array_mut()
            .unwrap()
            .push(json!(event.clone()));
        assert!(prove_direct_intent_fill(&tx, &ambiguous, &event, 1000).is_err());
        let mut wrong_log_identity = receipt.clone();
        wrong_log_identity["logs"][0]["transactionHash"] = json!(format!("0x{}", "ff".repeat(32)));
        assert!(prove_direct_intent_fill(&tx, &wrong_log_identity, &event, 1000).is_err());
        assert!(prove_direct_intent_fill(&tx, &receipt, &event, 2001).is_err());
        let encoded =
            hex::decode(tx["input"].as_str().unwrap().strip_prefix("0x").unwrap()).unwrap();
        let mut call = executeTradeFromIntentsCall::abi_decode(&encoded, true).unwrap();
        call.buyerSig = Bytes::from(vec![0u8; 65]);
        let mut bad_signature = tx.clone();
        bad_signature["input"] = json!(format!("0x{}", hex::encode(call.abi_encode())));
        assert!(prove_direct_intent_fill(&bad_signature, &receipt, &event, 1000).is_err());
        let mut call = executeTradeFromIntentsCall::abi_decode(&encoded, true).unwrap();
        call.trade.buyerSubaccountId += 1;
        let mut wrong_subaccount_echo = tx.clone();
        wrong_subaccount_echo["input"] = json!(format!("0x{}", hex::encode(call.abi_encode())));
        assert!(prove_direct_intent_fill(&wrong_subaccount_echo, &receipt, &event, 1000).is_err());
    }

    #[test]
    fn direct_intent_receipt_binds_one_d2_charge_without_legacy_event() {
        use crate::fees::d2_onchain::{
            D2Snapshot, VerifiedD2FeePolicy, D2_ADDRESS, TIMELOCK_ADDRESS,
        };
        use crate::fees::v2_perp_settlement::{validate_intent_receipt, D4_ENGINE, MUSDC};
        let (tx, mut receipt, mut event) = fixture();
        event.log_index = Some("0x3".into());
        receipt["logs"][0]["logIndex"] = json!("0x3");
        let proof = prove_direct_intent_fill(&tx, &receipt, &event, 1000).unwrap();
        let address_word = |value: &str| format!("{}{}", "0".repeat(24), &value[2..]);
        let word = |value: u128| format!("{value:064x}");
        let charge = json!({
            "address":D2_ADDRESS,
            "topics":[
                digest_hex(keccak256("FeeChargedV2(address,address,address,address,uint8,uint8,bool,int32,uint256,uint256)".as_bytes())),
                format!("0x{}", address_word(D4_ENGINE)),
                format!("0x{}", address_word(&proof.trade.seller)),
                format!("0x{}", address_word(TIMELOCK_ADDRESS))
            ],
            "data":format!("0x{}{}{}{}{}{}{}", address_word(MUSDC), word(1), word(0),
                word(0), word(200), word(1_500_000_000), word(300_000)),
            "transactionHash":proof.trade.tx_hash, "blockHash":proof.trade.block_hash,
            "blockNumber":"0x64", "logIndex":"0x2"
        });
        receipt["logs"].as_array_mut().unwrap().insert(0, charge);
        let policy = VerifiedD2FeePolicy::synthetic_for_tests(D2Snapshot {
            chain_id: 84532,
            block_number: 200,
            block_hash: format!("0x{}", "33".repeat(32)),
            block_timestamp: 1000,
            contract: D2_ADDRESS.into(),
            owner: TIMELOCK_ADDRESS.into(),
            recipient: TIMELOCK_ADDRESS.into(),
            rebate_funding_account: "0x0000000000000000000000000000000000000000".into(),
            protocol_fee_vault: "0x0000000000000000000000000000000000000000".into(),
            profiles: [(0, 200); 5],
            rfq_discounts: [(0, 0); 5],
        });
        let fee = validate_intent_receipt(&proof, &receipt, &policy, 0, 4, 1010).unwrap();
        assert_eq!(fee.amount_native, 300_000);
        assert_eq!(fee.fee_log_index, Some(2));
        let mut wrong = receipt.clone();
        wrong["logs"][0]["data"] = json!(format!(
            "0x{}{}{}{}{}{}{}",
            address_word(MUSDC),
            word(1),
            word(0),
            word(0),
            word(200),
            word(1_500_000_000),
            word(300_001)
        ));
        assert!(validate_intent_receipt(&proof, &wrong, &policy, 0, 4, 1010).is_err());
    }

    #[tokio::test]
    async fn postgres_signed_origin_and_fee_are_atomic_and_idempotent() {
        use crate::db::PgRepository;
        use crate::fees::d2_onchain::TIMELOCK_ADDRESS;
        use crate::fees::v2_perp_settlement::{CanonicalV2PerpFee, MUSDC};
        let Ok(url) = std::env::var("D2_INTENT_PG_TEST_URL") else {
            return;
        };
        assert!(
            url.ends_with("/deopt_intent_test"),
            "disposable test database required"
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
        let (tx, receipt, event) = fixture();
        let proof = prove_direct_intent_fill(&tx, &receipt, &event, 1000).unwrap();
        let fee = CanonicalV2PerpFee {
            chain_id: 84532,
            tx_hash: proof.trade.tx_hash.clone(),
            trade_log_index: proof.trade.log_index,
            fee_log_index: Some(1),
            block_number: 100,
            block_hash: proof.trade.block_hash.clone().unwrap(),
            market_id: "1".into(),
            payer: proof.trade.seller.clone(),
            recipient: TIMELOCK_ADDRESS.into(),
            settlement_asset: MUSDC.into(),
            maker_tier: 0,
            taker_tier: 4,
            basis_native: 1_500_000_000,
            amount_native: 300_000,
        };
        assert!(repository
            .persist_canonical_v2_intent_fee(&proof, &fee)
            .await
            .unwrap());
        assert!(!repository
            .persist_canonical_v2_intent_fee(&proof, &fee)
            .await
            .unwrap());
        let history = repository
            .list_canonical_v2_perp_history(&proof.trade.seller, 10)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].fee_native, "300000");
        assert_eq!(history[0].signed_subaccount_id, Some(9));
        assert_eq!(history[0].subaccount_id, None);
        let other_wallet = repository
            .list_canonical_v2_perp_history("0x3333333333333333333333333333333333333333", 10)
            .await
            .unwrap();
        assert!(other_wallet.is_empty());
        let mut reorg = proof.clone();
        reorg.trade.block_hash = Some(format!("0x{}", "44".repeat(32)));
        let mut reorg_fee = fee.clone();
        reorg_fee.block_hash = reorg.trade.block_hash.clone().unwrap();
        assert!(repository
            .persist_canonical_v2_intent_fee(&reorg, &reorg_fee)
            .await
            .is_err());
    }
}
