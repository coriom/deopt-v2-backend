//! Typed decoders for canonical Hybrid V2 events.
//!
//! Boundary:
//! - Input: a canonical raw log (already known to originate from a
//!   canonical manifest emitter).
//! - Output: a `HybridV2Event` with the projection-relevant fields
//!   pulled from indexed topics + data, and the full payload preserved
//!   as JSON in the `payload` field.
//!
//! Frozen rules:
//! - Topic 0 MUST match an entry in the pinned `TopicCatalogue`. Unknown
//!   topic 0 on a known canonical emitter → `DecoderError::UnknownTopic`.
//!   Callers MUST fail readiness on this error (per canonicality rule).
//! - Event version is preserved verbatim (never coerced).
//! - Address topics decode as lowercase hex without validation shortcuts.
//! - `uint256` / `int256` values remain as decimal strings — no lossy
//!   `u64` truncation.
//!
//! Scope for V1:
//! - We do not (yet) fully ABI-decode every non-indexed field into a
//!   typed struct per event kind. The decoder captures the projection-
//!   critical fields (subKey, owner, subaccount id, token, engine,
//!   execution id, order hash, series id) and preserves the raw payload
//!   as JSON. Follow-up backend milestones (`BACKEND-SUBACCOUNT-READ-API-...`)
//!   flesh out per-event typed schemas as they become needed by the
//!   read layer.

use crate::hybrid_v2::events::{EventKind, HybridV2Event};
use crate::hybrid_v2::topics::TopicCatalogue;
use serde::Serialize;
use serde_json::json;
use thiserror::Error;

/// Simplified canonical raw log input.
#[derive(Debug, Clone, Serialize)]
pub struct CanonicalRawLog {
    pub emitter: String,
    pub topics: Vec<[u8; 32]>,
    pub data: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum DecoderError {
    #[error("log has zero topics; cannot resolve topic 0")]
    NoTopics,
    #[error("topic 0 {topic0} not in pinned Hybrid V2 catalogue")]
    UnknownTopic { topic0: String },
    #[error("event {event} decoding is not implemented in the V1 decoder scope")]
    UnimplementedForVariant { event: &'static str },
    #[error("event {event} expected {expected} indexed topics but got {actual}")]
    IndexedTopicCountMismatch {
        event: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("event {event} data payload was too short (need >= {need} bytes, got {got})")]
    DataTooShort {
        event: &'static str,
        need: usize,
        got: usize,
    },
}

pub fn decode_log(log: &CanonicalRawLog) -> Result<HybridV2Event, DecoderError> {
    if log.topics.is_empty() {
        return Err(DecoderError::NoTopics);
    }
    let topic0 = log.topics[0];
    let entry = TopicCatalogue::get()
        .lookup_by_topic0(&topic0)
        .ok_or_else(|| DecoderError::UnknownTopic {
            topic0: format!("0x{}", hex_lower(&topic0)),
        })?;
    let kind = EventKind::parse(entry.event)
        .ok_or(DecoderError::UnimplementedForVariant { event: entry.event })?;

    // Every canonical Hybrid V2 event's last data word is `uint16 eventVersion`.
    // For V1 we defensively read it when data is at least 32 bytes; otherwise
    // fall back to 1 (the frozen V1 event version) and let the caller enforce
    // via `applied = false` if the payload doesn't match a decoder.
    let event_version = read_last_uint16(&log.data).unwrap_or(1);

    Ok(match kind {
        EventKind::SubaccountCreated | EventKind::SubaccountLazyRegistered => {
            let (owner, sid, subkey) = decode_subaccount_identity(log, kind.as_str())?;
            HybridV2Event {
                kind,
                event_version,
                subkey: Some(subkey),
                owner: Some(owner),
                subaccount_id: Some(sid),
                token: None,
                engine: None,
                execution_id: None,
                order_hash: None,
                series_id: None,
                payload: json!({}),
            }
        }
        EventKind::Deposit | EventKind::Withdraw => {
            // topics: [t0, subKey, owner, subaccountId]
            let (subkey, owner, sid) = expect_three_indexed(log, kind.as_str())?;
            let (token, amount) = decode_token_and_amount(log, kind.as_str())?;
            HybridV2Event {
                kind,
                event_version,
                subkey: Some(subkey),
                owner: Some(owner),
                subaccount_id: Some(sid),
                token: Some(token),
                engine: None,
                execution_id: None,
                order_hash: None,
                series_id: None,
                payload: json!({ "amount": amount }),
            }
        }
        EventKind::CollateralLocked | EventKind::CollateralUnlocked => {
            // topics: [t0, subKey, token, engine]
            let subkey = topic_to_bytes32_hex(&log.topics, 1, kind.as_str())?;
            let token = topic_to_address_hex(&log.topics, 2, kind.as_str())?;
            let engine = topic_to_address_hex(&log.topics, 3, kind.as_str())?;
            let amount = read_first_uint256(&log.data, kind.as_str())?;
            HybridV2Event {
                kind,
                event_version,
                subkey: Some(subkey),
                owner: None,
                subaccount_id: None,
                token: Some(token),
                engine: Some(engine),
                execution_id: None,
                order_hash: None,
                series_id: None,
                payload: json!({ "amount": amount }),
            }
        }
        EventKind::CollateralTokenEnteredUniverse => {
            // topics: [t0, token]; data: index (uint256), eventVersion (uint16)
            let token = topic_to_address_hex(&log.topics, 1, kind.as_str())?;
            let index = read_first_uint256(&log.data, kind.as_str())?;
            HybridV2Event {
                kind,
                event_version,
                subkey: None,
                owner: None,
                subaccount_id: None,
                token: Some(token),
                engine: None,
                execution_id: None,
                order_hash: None,
                series_id: None,
                payload: json!({ "universe_index": index }),
            }
        }
        EventKind::EngineCapabilityChanged => {
            // topics: [t0, engine]; data: addedBits (u256), removedBits (u256), eventVersion (u16)
            let engine = topic_to_address_hex(&log.topics, 1, kind.as_str())?;
            let added = read_uint256_at(&log.data, 0, kind.as_str())?;
            let removed = read_uint256_at(&log.data, 1, kind.as_str())?;
            HybridV2Event {
                kind,
                event_version,
                subkey: None,
                owner: None,
                subaccount_id: None,
                token: None,
                engine: Some(engine),
                execution_id: None,
                order_hash: None,
                series_id: None,
                payload: json!({ "added_bits": added, "removed_bits": removed }),
            }
        }
        EventKind::RecoveryRequested
        | EventKind::RecoveryActivated
        | EventKind::RecoveryCancelled => {
            // topics: [t0, subKey, owner, subaccountId]
            let (subkey, owner, sid) = expect_three_indexed(log, kind.as_str())?;
            HybridV2Event {
                kind,
                event_version,
                subkey: Some(subkey),
                owner: Some(owner),
                subaccount_id: Some(sid),
                token: None,
                engine: None,
                execution_id: None,
                order_hash: None,
                series_id: None,
                payload: json!({}),
            }
        }
        EventKind::RecoveryFinalized => {
            // topics: [t0, subKey, owner, subaccountId]
            let (subkey, owner, sid) = expect_three_indexed(log, kind.as_str())?;
            HybridV2Event {
                kind,
                event_version,
                subkey: Some(subkey),
                owner: Some(owner),
                subaccount_id: Some(sid),
                token: None,
                engine: None,
                execution_id: None,
                order_hash: None,
                series_id: None,
                payload: json!({}),
            }
        }
        // All other canonical Hybrid V2 events are catalogued but their
        // typed payload decoding is deferred to a follow-up backend
        // milestone that consumes them. Journaling them here preserves
        // the raw evidence so that the reducer can extend later without
        // reindexing the chain.
        _ => HybridV2Event {
            kind,
            event_version,
            subkey: None,
            owner: None,
            subaccount_id: None,
            token: None,
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({ "raw_data_hex": format!("0x{}", hex_lower(&log.data)) }),
        },
    })
}

// -----------------------------------------------------------------
//                          DECODE HELPERS
// -----------------------------------------------------------------

fn decode_subaccount_identity(
    log: &CanonicalRawLog,
    event: &'static str,
) -> Result<(String, u32, String), DecoderError> {
    // topics: [t0, owner, subaccountId, subKey]
    if log.topics.len() < 4 {
        return Err(DecoderError::IndexedTopicCountMismatch {
            event,
            expected: 3,
            actual: log.topics.len().saturating_sub(1),
        });
    }
    let owner = topic_to_address_hex(&log.topics, 1, event)?;
    let sid = topic_to_u32_low(&log.topics, 2)?;
    let subkey = topic_to_bytes32_hex(&log.topics, 3, event)?;
    Ok((owner, sid, subkey))
}

fn expect_three_indexed(
    log: &CanonicalRawLog,
    event: &'static str,
) -> Result<(String, String, u32), DecoderError> {
    if log.topics.len() < 4 {
        return Err(DecoderError::IndexedTopicCountMismatch {
            event,
            expected: 3,
            actual: log.topics.len().saturating_sub(1),
        });
    }
    let subkey = topic_to_bytes32_hex(&log.topics, 1, event)?;
    let owner = topic_to_address_hex(&log.topics, 2, event)?;
    let sid = topic_to_u32_low(&log.topics, 3)?;
    Ok((subkey, owner, sid))
}

fn decode_token_and_amount(
    log: &CanonicalRawLog,
    event: &'static str,
) -> Result<(String, String), DecoderError> {
    // data: address token, uint256 amount, address depositor, uint16 eventVersion
    let token = read_address_at(&log.data, 0, event)?;
    let amount = read_uint256_at(&log.data, 1, event)?;
    Ok((token, amount))
}

fn topic_to_bytes32_hex(
    topics: &[[u8; 32]],
    idx: usize,
    event: &'static str,
) -> Result<String, DecoderError> {
    if idx >= topics.len() {
        return Err(DecoderError::IndexedTopicCountMismatch {
            event,
            expected: idx,
            actual: topics.len(),
        });
    }
    Ok(format!("0x{}", hex_lower(&topics[idx])))
}

fn topic_to_address_hex(
    topics: &[[u8; 32]],
    idx: usize,
    event: &'static str,
) -> Result<String, DecoderError> {
    if idx >= topics.len() {
        return Err(DecoderError::IndexedTopicCountMismatch {
            event,
            expected: idx,
            actual: topics.len(),
        });
    }
    // Last 20 bytes.
    let addr: &[u8] = &topics[idx][12..];
    Ok(format!("0x{}", hex_lower(addr)))
}

fn topic_to_u32_low(topics: &[[u8; 32]], idx: usize) -> Result<u32, DecoderError> {
    if idx >= topics.len() {
        return Err(DecoderError::IndexedTopicCountMismatch {
            event: "<topic_to_u32_low>",
            expected: idx,
            actual: topics.len(),
        });
    }
    // Last 4 bytes as big-endian.
    let bytes = &topics[idx][28..32];
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_first_uint256(data: &[u8], event: &'static str) -> Result<String, DecoderError> {
    read_uint256_at(data, 0, event)
}

fn read_uint256_at(
    data: &[u8],
    word_index: usize,
    event: &'static str,
) -> Result<String, DecoderError> {
    let start = word_index * 32;
    let end = start + 32;
    if data.len() < end {
        return Err(DecoderError::DataTooShort {
            event,
            need: end,
            got: data.len(),
        });
    }
    Ok(u256_be_to_decimal(&data[start..end]))
}

fn read_address_at(
    data: &[u8],
    word_index: usize,
    event: &'static str,
) -> Result<String, DecoderError> {
    let start = word_index * 32;
    let end = start + 32;
    if data.len() < end {
        return Err(DecoderError::DataTooShort {
            event,
            need: end,
            got: data.len(),
        });
    }
    Ok(format!("0x{}", hex_lower(&data[start + 12..end])))
}

fn read_last_uint16(data: &[u8]) -> Option<u16> {
    if data.len() < 32 {
        return None;
    }
    let start = data.len() - 32;
    let bytes = &data[start + 30..start + 32];
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", b);
    }
    out
}

/// Convert a big-endian uint256 into a base-10 decimal string. Handles
/// values up to and including `2^256 - 1` without precision loss.
fn u256_be_to_decimal(bytes: &[u8]) -> String {
    assert!(bytes.len() == 32, "expected 32-byte uint256 slice");
    // BCD-ish long division against 2^256 base bytes.
    let mut digits: Vec<u8> = Vec::with_capacity(78);
    let mut n = bytes.to_vec();
    // Fast path: fits in u128?
    if n[..16].iter().all(|b| *b == 0) {
        let mut hi = 0u128;
        for b in &n[16..] {
            hi = (hi << 8) | (*b as u128);
        }
        return hi.to_string();
    }
    // Otherwise, long-divide by 10 repeatedly.
    while n.iter().any(|b| *b != 0) {
        let mut rem: u32 = 0;
        for byte in n.iter_mut() {
            let acc = (rem << 8) | (*byte as u32);
            *byte = (acc / 10) as u8;
            rem = acc % 10;
        }
        digits.push(rem as u8);
    }
    if digits.is_empty() {
        return "0".to_string();
    }
    digits.reverse();
    digits.into_iter().map(|d| (b'0' + d) as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uint256_zero_decimal_is_zero() {
        let z = [0u8; 32];
        assert_eq!(u256_be_to_decimal(&z), "0");
    }

    #[test]
    fn uint256_max_u64_decimal_matches() {
        let mut b = [0u8; 32];
        for byte in b.iter_mut().skip(24) {
            *byte = 0xff;
        }
        assert_eq!(u256_be_to_decimal(&b), u64::MAX.to_string());
    }

    #[test]
    fn uint256_beyond_u128_still_decodes() {
        // 2^129
        let mut b = [0u8; 32];
        b[15] = 0x02;
        let expected = "680564733841876926926749214863536422912";
        assert_eq!(u256_be_to_decimal(&b), expected);
    }
}
