use super::events::{EthLog, IndexedPerpTrade};
use crate::error::{BackendError, Result};
use crate::signing::eip712::keccak256;
use crate::types::now_ms;
use alloy_primitives::U256;

pub const TRADE_EXECUTED_SIGNATURE: &str =
    "TradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)";

pub fn trade_executed_topic0() -> String {
    hex_0x(&keccak256(TRADE_EXECUTED_SIGNATURE.as_bytes()))
}

pub fn decode_trade_executed_log(log: &EthLog) -> Result<IndexedPerpTrade> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "TradeExecuted log must have four topics".to_string(),
        ));
    }
    if !eq_hex(&log.topics[0], &trade_executed_topic0()) {
        return Err(BackendError::Indexer(
            "log topic0 does not match TradeExecuted".to_string(),
        ));
    }

    let tx_hash = required_field(log.transaction_hash.as_ref(), "transactionHash")?.clone();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 6 {
        return Err(BackendError::Indexer(
            "TradeExecuted data must contain six ABI words".to_string(),
        ));
    }

    let event_id = format!("{tx_hash}:{log_index}");
    Ok(IndexedPerpTrade {
        event_id,
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        onchain_intent_id: Some(decode_topic_bytes32(&log.topics[1])?),
        buyer: decode_topic_address(&log.topics[2])?,
        seller: decode_topic_address(&log.topics[3])?,
        market_id: decode_data_u256(&data, 0)?.to_string(),
        size_delta_1e8: decode_data_u256(&data, 1)?.to_string(),
        execution_price_1e8: decode_data_u256(&data, 2)?.to_string(),
        buyer_is_maker: decode_bool(&data, 3)?,
        buyer_nonce: decode_data_u256(&data, 4)?.to_string(),
        seller_nonce: decode_data_u256(&data, 5)?.to_string(),
        created_at_ms: now_ms(),
    })
}

fn required_field<'a>(value: Option<&'a String>, field: &str) -> Result<&'a String> {
    value.ok_or_else(|| BackendError::Indexer(format!("log missing {field}")))
}

fn decode_topic_address(topic: &str) -> Result<String> {
    let bytes = decode_fixed_hex(topic, 32)?;
    Ok(format!("0x{}", hex_lower(&bytes[12..])))
}

fn decode_topic_bytes32(topic: &str) -> Result<String> {
    Ok(hex_0x(&decode_fixed_hex(topic, 32)?))
}

fn decode_data_u256(data: &[u8], word_index: usize) -> Result<U256> {
    let start = word_index * 32;
    let end = start + 32;
    let word = data.get(start..end).ok_or_else(|| {
        BackendError::Indexer(format!("missing ABI data word at index {word_index}"))
    })?;
    Ok(U256::from_be_slice(word))
}

fn decode_bool(data: &[u8], word_index: usize) -> Result<bool> {
    let value = decode_data_u256(data, word_index)?;
    if value == U256::ZERO {
        Ok(false)
    } else if value == U256::from(1u8) {
        Ok(true)
    } else {
        Err(BackendError::Indexer(
            "invalid ABI bool in TradeExecuted data".to_string(),
        ))
    }
}

fn decode_fixed_hex(value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let bytes = decode_hex_bytes(value)?;
    if bytes.len() != expected_len {
        return Err(BackendError::Indexer(format!(
            "expected {expected_len} hex bytes"
        )));
    }
    Ok(bytes)
}

pub fn parse_hex_quantity(value: &str) -> Result<u64> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| BackendError::Indexer("invalid hex quantity".to_string()))?;
    u64::from_str_radix(hex, 16)
        .map_err(|error| BackendError::Indexer(format!("invalid hex quantity: {error}")))
}

pub fn decode_hex_bytes(value: &str) -> Result<Vec<u8>> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| BackendError::Indexer("invalid hex bytes".to_string()))?;
    if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::Indexer("invalid hex bytes".to_string()));
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for index in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[index..index + 2], 16)
            .map_err(|error| BackendError::Indexer(format!("invalid hex bytes: {error}")))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

pub fn hex_quantity(value: u64) -> String {
    format!("0x{value:x}")
}

pub fn hex_0x(bytes: &[u8]) -> String {
    format!("0x{}", hex_lower(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn eq_hex(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_executed_topic0_matches_signature_hash() {
        assert_eq!(
            trade_executed_topic0(),
            "0x5018a0a73d56c00e01815636cf5e029fd7ed9440d42b3eea0e75404dfedb3f80"
        );
    }

    #[test]
    fn decoder_decodes_synthetic_trade_executed_log() {
        let log = EthLog {
            address: "0x0000000000000000000000000000000000000009".to_string(),
            topics: vec![
                trade_executed_topic0(),
                word(99),
                topic_address("0000000000000000000000000000000000000001"),
                topic_address("0000000000000000000000000000000000000002"),
            ],
            data: format!(
                "0x{}{}{}{}{}{}",
                word_no_prefix(7),
                word_no_prefix(100_000_000),
                word_no_prefix(300_000_000_000),
                word_no_prefix(1),
                word_no_prefix(11),
                word_no_prefix(12),
            ),
            block_number: Some("0x7b".to_string()),
            block_hash: Some("0xblock".to_string()),
            transaction_hash: Some("0xtx".to_string()),
            log_index: Some("0x2".to_string()),
        };

        let trade = decode_trade_executed_log(&log).unwrap();

        assert_eq!(trade.event_id, "0xtx:2");
        assert_eq!(trade.tx_hash, "0xtx");
        assert_eq!(trade.log_index, 2);
        assert_eq!(trade.block_number, 123);
        assert_eq!(trade.block_hash.as_deref(), Some("0xblock"));
        assert_eq!(
            trade.onchain_intent_id.as_deref(),
            Some("0x0000000000000000000000000000000000000000000000000000000000000063")
        );
        assert_eq!(trade.buyer, "0x0000000000000000000000000000000000000001");
        assert_eq!(trade.seller, "0x0000000000000000000000000000000000000002");
        assert_eq!(trade.market_id, "7");
        assert_eq!(trade.size_delta_1e8, "100000000");
        assert_eq!(trade.execution_price_1e8, "300000000000");
        assert!(trade.buyer_is_maker);
        assert_eq!(trade.buyer_nonce, "11");
        assert_eq!(trade.seller_nonce, "12");
    }

    fn topic_address(address_without_prefix: &str) -> String {
        format!("0x{:0>64}", address_without_prefix)
    }

    fn word(value: u128) -> String {
        format!("0x{}", word_no_prefix(value))
    }

    fn word_no_prefix(value: u128) -> String {
        format!("{value:064x}")
    }
}
