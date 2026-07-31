//! Typed decoders for canonical Hybrid V2 events.
//!
//! Every canonical event has an inline `EventLayout` describing:
//! - the ordered indexed topic slots (excluding topic0)
//! - the ordered data words + type
//!
//! The trailing `uint16 eventVersion` is enforced separately.
//!
//! Frozen rules:
//! - Topic 0 MUST match an entry in the pinned `TopicCatalogue`. Unknown
//!   topic 0 on a known canonical emitter → `DecoderError::UnknownTopic`.
//!   Callers MUST fail readiness on this error (per canonicality rule).
//! - Wrong indexed topic count → `IndexedTopicCountMismatch`.
//! - Insufficient data → `DataTooShort`.
//! - Event version mismatch vs manifest → `EventVersionMismatch`.
//! - Uint256/int256 values are preserved as decimal strings (no truncation).

use crate::hybrid_v2::events::{EventKind, HybridV2Event};
use crate::hybrid_v2::topics::TopicCatalogue;
use serde::Serialize;
use serde_json::{json, Map, Value};
use thiserror::Error;

/// Simplified canonical raw log input.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
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
    #[error("event {event} is catalogued but no typed decoder is defined")]
    UnimplementedForVariant { event: &'static str },
    #[error("event {event} expected {expected} indexed topics but got {actual}")]
    IndexedTopicCountMismatch {
        event: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("event {event} data payload too short (need >= {need} bytes, got {got})")]
    DataTooShort {
        event: &'static str,
        need: usize,
        got: usize,
    },
    #[error("event {event} event version {actual} != expected {expected}")]
    EventVersionMismatch {
        event: &'static str,
        expected: u16,
        actual: u16,
    },
    #[error("event {event} emitter {actual} does not match expected canonical emitter {expected}")]
    WrongEmitter {
        event: &'static str,
        expected: String,
        actual: String,
    },
}

/// Kinds of indexed topic slots we know how to extract.
#[derive(Debug, Clone, Copy)]
pub enum IndexedField {
    SubKey,       // bytes32
    OtherSubKey,  // bytes32 (e.g. from/to in InternalTransfer)
    Owner,        // address
    SubaccountId, // uint32 (low 4 bytes)
    Token,        // address
    Engine,       // address
    OrderHash,    // bytes32
    ExecutionId,  // bytes32
    SeriesId,     // bytes32 or uint256 in-topic
    IntentHash,   // bytes32
    Actor,        // address
    Skip,         // reserved topic slot we don't project
}

impl IndexedField {
    pub fn label(&self) -> &'static str {
        match self {
            IndexedField::SubKey => "subkey",
            IndexedField::OtherSubKey => "other_subkey",
            IndexedField::Owner => "owner",
            IndexedField::SubaccountId => "subaccount_id",
            IndexedField::Token => "token",
            IndexedField::Engine => "engine",
            IndexedField::OrderHash => "order_hash",
            IndexedField::ExecutionId => "execution_id",
            IndexedField::SeriesId => "series_id",
            IndexedField::IntentHash => "intent_hash",
            IndexedField::Actor => "actor",
            IndexedField::Skip => "_skip",
        }
    }
}

/// Kinds of 32-byte data words we know how to extract.
#[derive(Debug, Clone, Copy)]
pub enum DataField {
    Address(&'static str),
    U256(&'static str),
    I256(&'static str),
    U128(&'static str),
    U64(&'static str),
    U32(&'static str),
    U16(&'static str),
    U8(&'static str),
    Bool(&'static str),
    Bytes32(&'static str),
    /// Skip a 32-byte word we don't project.
    Skip,
}

pub struct EventLayout {
    pub indexed: &'static [IndexedField],
    pub data: &'static [DataField],
}

impl EventLayout {
    pub const fn new(indexed: &'static [IndexedField], data: &'static [DataField]) -> Self {
        Self { indexed, data }
    }
}

pub fn layout_for(kind: EventKind) -> EventLayout {
    use DataField::{Address, Bool, Bytes32, Skip, I256, U128, U16, U256, U32, U64, U8};
    use IndexedField::{
        Actor, Engine, ExecutionId, IntentHash, OrderHash, OtherSubKey, Owner, SeriesId, SubKey,
        SubaccountId, Token,
    };
    match kind {
        // Registry
        EventKind::SubaccountCreated => {
            EventLayout::new(&[Owner, SubaccountId, SubKey], &[U256("nonce"), U16("_ev")])
        }
        EventKind::SubaccountLazyRegistered => EventLayout::new(
            &[Owner, SubaccountId, SubKey],
            &[U256("nonce"), Address("materialiser"), U16("_ev")],
        ),
        // Vault balances
        EventKind::Deposit | EventKind::Withdraw => EventLayout::new(
            &[SubKey, Owner, SubaccountId],
            &[
                Address("token"),
                U256("amount"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        EventKind::InternalTransfer => EventLayout::new(
            &[SubKey, OtherSubKey, Token],
            &[
                U256("amount"),
                Address("actor"),
                U32("from_subaccount_id"),
                U32("to_subaccount_id"),
                U16("_ev"),
            ],
        ),
        EventKind::CollateralLocked | EventKind::CollateralUnlocked => {
            EventLayout::new(&[SubKey, Token, Engine], &[U256("amount"), U16("_ev")])
        }
        EventKind::OrphanedLockReleased => EventLayout::new(
            &[SubKey, Token, Engine],
            &[U256("amount"), Skip, U16("_ev")],
        ),
        EventKind::SupportedTokenAdded | EventKind::SupportedTokenRemoved => {
            EventLayout::new(&[Token], &[U16("_ev")])
        }
        EventKind::CollateralTokenEnteredUniverse => {
            EventLayout::new(&[Token], &[U256("universe_index"), U16("_ev")])
        }
        EventKind::OptionPremiumTransferred => EventLayout::new(
            &[SubKey, OtherSubKey, Token],
            &[U256("amount"), Address("actor"), U16("_ev")],
        ),
        EventKind::OptionFeeCharged => EventLayout::new(
            &[SubKey, OtherSubKey, Token],
            &[U256("amount"), Address("actor"), U16("_ev")],
        ),
        EventKind::OptionRebatePaid => EventLayout::new(
            &[SubKey, OtherSubKey, Token],
            &[U256("amount"), Address("actor"), U16("_ev")],
        ),
        EventKind::RecoveryFinalizationWithdrawn => EventLayout::new(
            &[SubKey, Owner, Token],
            &[U256("amount"), Address("actor"), U16("_ev")],
        ),
        EventKind::PauseFlagChanged => {
            EventLayout::new(&[SubKey], &[Bool("paused"), Address("actor"), U16("_ev")])
        }
        EventKind::BadDebtSocialized => {
            EventLayout::new(&[SubKey, Token], &[U256("amount"), U16("_ev")])
        }
        EventKind::ProtocolSubaccountsInitialized => EventLayout::new(
            &[],
            &[
                Bytes32("protocol_fee_subkey"),
                Bytes32("rebate_budget_subkey"),
                Bytes32("insurance_fund_subkey"),
                U16("_ev"),
            ],
        ),
        EventKind::EscapeControllerInitialized => EventLayout::new(&[Actor], &[U16("_ev")]),
        EventKind::RecoveryFinalizerInitialized => EventLayout::new(&[Actor], &[U16("_ev")]),
        // Capability
        EventKind::EngineCapabilityChanged => EventLayout::new(
            &[Engine],
            &[U256("added_bits"), U256("removed_bits"), U16("_ev")],
        ),
        EventKind::EngineGuardianRevoked => EventLayout::new(&[Engine, Actor], &[U16("_ev")]),
        EventKind::GuardianChanged => EventLayout::new(&[Actor, IndexedField::Skip], &[U16("_ev")]),
        // Options ledger
        EventKind::OptionPositionOpened => EventLayout::new(
            &[SubKey, SeriesId],
            &[
                U8("kind"),
                U128("long_delta_1e8"),
                U128("short_delta_1e8"),
                Address("actor"),
                Address("engine"),
                U32("subaccount_id"),
                U16("_ev"),
            ],
        ),
        EventKind::OptionPositionModified => EventLayout::new(
            &[SubKey, SeriesId],
            &[
                U8("kind"),
                I256("long_delta_1e8_signed"),
                U128("short_delta_1e8"),
                Address("actor"),
                Address("engine"),
                U32("subaccount_id"),
                U16("_ev"),
            ],
        ),
        EventKind::OptionPositionClosed => EventLayout::new(
            &[SubKey, SeriesId],
            &[
                U8("kind"),
                Address("actor"),
                Address("engine"),
                U32("subaccount_id"),
                U16("_ev"),
            ],
        ),
        EventKind::OptionExercised => EventLayout::new(
            &[SubKey, SeriesId],
            &[
                U128("qty_1e8"),
                U128("proceeds"),
                I256("pnl_1e8"),
                Address("actor"),
                U32("subaccount_id"),
                U16("_ev"),
            ],
        ),
        EventKind::OptionSettled => EventLayout::new(
            &[SubKey, SeriesId],
            &[
                U128("qty_1e8"),
                I256("pnl_1e8"),
                Address("actor"),
                U32("subaccount_id"),
                U16("_ev"),
            ],
        ),
        EventKind::OptionPositionLiquidated => EventLayout::new(
            &[SubKey, SeriesId],
            &[
                U128("qty_1e8"),
                U128("proceeds"),
                Bytes32("reason_tag"),
                U16("_ev"),
            ],
        ),
        // Options engine
        EventKind::OptionOrderPairExecuted => EventLayout::new(
            &[ExecutionId, OrderHash, SubKey],
            &[
                U256("seller_execution_seed"),
                Bytes32("buyer_order_hash"),
                Bytes32("seller_order_hash"),
                Address("buyer_owner"),
                Address("seller_owner"),
                U32("buyer_subaccount_id"),
                U32("seller_subaccount_id"),
                U128("matched_qty_1e8"),
                U128("premium_1e8"),
                U256("series_id"),
                Address("token"),
                U8("buyer_side"),
                U8("seller_side"),
                U128("buyer_fee"),
                U128("seller_rebate"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        EventKind::OptionOrderFilled => EventLayout::new(
            &[SubKey, OrderHash],
            &[
                U256("series_id"),
                U8("side"),
                U8("time_in_force"),
                U128("filled_delta_1e8"),
                U128("total_qty_1e8"),
                U128("filled_qty_1e8"),
                U128("remaining_qty_1e8"),
                Bool("terminal"),
                U8("terminal_reason"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        EventKind::OptionOrderCancelled => EventLayout::new(
            &[SubKey, OrderHash],
            &[Address("owner"), Address("actor"), U16("_ev")],
        ),
        EventKind::OptionSubaccountMinValidOrderNonceAdvanced => EventLayout::new(
            &[SubKey, Owner],
            &[
                U256("previous_min_valid_nonce"),
                U256("new_min_valid_nonce"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        // Escape
        EventKind::RecoveryRequested => EventLayout::new(
            &[SubKey, Owner, SubaccountId],
            &[U256("nonce"), U64("activation_eligible_at"), U16("_ev")],
        ),
        EventKind::RecoveryActivated => {
            EventLayout::new(&[SubKey, Owner, SubaccountId], &[U256("nonce"), U16("_ev")])
        }
        EventKind::RecoveryCancelled => {
            EventLayout::new(&[SubKey, Owner, SubaccountId], &[U16("_ev")])
        }
        EventKind::RecoveryEpochIncremented => EventLayout::new(
            &[SubKey, Owner],
            &[U8("scope"), U256("new_epoch"), U16("_ev")],
        ),
        EventKind::RecoveryPauseSet => EventLayout::new(
            &[],
            &[
                Bool("paused"),
                U64("until_ts"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        EventKind::RecoveryFinalized => EventLayout::new(
            &[SubKey, Owner, SubaccountId],
            &[
                U256("nonce"),
                U64("finalized_ts"),
                U8("terminal_reason"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        // Replay + epoch controller
        EventKind::IntentConsumed => EventLayout::new(
            &[IntentHash, Owner],
            &[Address("actor"), Bytes32("scope"), U16("_ev")],
        ),
        EventKind::NonceCancelled => EventLayout::new(
            &[Owner],
            &[
                U256("previous_min_valid_nonce"),
                U256("new_min_valid_nonce"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        EventKind::OwnerRecoveryEpochAdvanced => EventLayout::new(
            &[Owner],
            &[
                U256("previous_epoch"),
                U256("new_epoch"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        EventKind::SubaccountRecoveryEpochAdvanced => EventLayout::new(
            &[SubKey, Owner],
            &[
                U32("subaccount_id"),
                U256("previous_epoch"),
                U256("new_epoch"),
                Address("actor"),
                U16("_ev"),
            ],
        ),
        // Risk module
        EventKind::RiskParamsSet => EventLayout::new(&[Bytes32Idx], &[Skip, U16("_ev")]),
        EventKind::RiskModuleActivated => EventLayout::new(
            &[],
            &[
                U16("architecture_version"),
                U16("storage_version"),
                U16("_ev"),
            ],
        ),
        EventKind::LiquidationTriggered => EventLayout::new(&[SubKey], &[U8("kind"), U16("_ev")]),
        // Manifest
        EventKind::DeploymentManifestDeclared => EventLayout::new(
            &[Bytes32Idx],
            &[
                U256("chain_id"),
                Address("deployer"),
                Bytes32("manifest_hash"),
                U16("architecture_version"),
                U16("storage_version"),
                U16("event_version"),
                Bytes32("module_addresses_hash"),
                Bytes32("critical_config_hash"),
                U64("deployment_block"),
                U64("deployment_timestamp"),
                U16("_ev"),
            ],
        ),
    }
}

// Alias for readability when a bytes32 indexed slot isn't tied to a canonical
// projection identity (RiskParamsSet paramsHash, DeploymentManifestDeclared
// manifest key).
#[allow(non_upper_case_globals)]
const Bytes32Idx: IndexedField = IndexedField::Skip;

/// The canonical event version for the frozen `eventVersion=1` surface.
pub const CANONICAL_EVENT_VERSION: u16 = 1;

/// Decode a canonical raw log into a typed `HybridV2Event`.
///
/// - `expected_emitter`: when Some, rejects logs from any other address.
/// - `expected_event_version`: fails on event-version drift.
pub fn decode_log(log: &CanonicalRawLog) -> Result<HybridV2Event, DecoderError> {
    decode_log_with(log, None, CANONICAL_EVENT_VERSION)
}

pub fn decode_log_with(
    log: &CanonicalRawLog,
    expected_emitter: Option<&str>,
    expected_event_version: u16,
) -> Result<HybridV2Event, DecoderError> {
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
    if let Some(exp) = expected_emitter {
        if !addr_eq(exp, &log.emitter) {
            return Err(DecoderError::WrongEmitter {
                event: entry.event,
                expected: exp.to_string(),
                actual: log.emitter.clone(),
            });
        }
    }
    let layout = layout_for(kind);
    let indexed_actual = log.topics.len().saturating_sub(1);
    if indexed_actual != layout.indexed.len() {
        return Err(DecoderError::IndexedTopicCountMismatch {
            event: entry.event,
            expected: layout.indexed.len(),
            actual: indexed_actual,
        });
    }
    // Enforce minimum data length: each 32-byte word.
    let min_bytes = layout.data.len() * 32;
    if log.data.len() < min_bytes {
        return Err(DecoderError::DataTooShort {
            event: entry.event,
            need: min_bytes,
            got: log.data.len(),
        });
    }

    let mut payload = Map::new();
    let mut subkey: Option<String> = None;
    let mut owner: Option<String> = None;
    let mut subaccount_id: Option<u32> = None;
    let mut token: Option<String> = None;
    let mut engine: Option<String> = None;
    let mut execution_id: Option<String> = None;
    let mut order_hash: Option<String> = None;
    let mut series_id: Option<String> = None;

    // Extract indexed fields.
    for (i, field) in layout.indexed.iter().enumerate() {
        let topic = &log.topics[i + 1];
        match field {
            IndexedField::SubKey => {
                subkey = Some(bytes32_hex(topic));
            }
            IndexedField::OtherSubKey => {
                payload.insert(field.label().to_string(), Value::String(bytes32_hex(topic)));
                // Convention: InternalTransfer -> subkey = from, other_subkey = to
                if subkey.is_none() {
                    subkey = Some(bytes32_hex(topic));
                }
            }
            IndexedField::Owner => {
                owner = Some(topic_to_address_hex(topic));
            }
            IndexedField::SubaccountId => {
                subaccount_id = Some(topic_to_u32_low(topic));
            }
            IndexedField::Token => {
                token = Some(topic_to_address_hex(topic));
            }
            IndexedField::Engine => {
                engine = Some(topic_to_address_hex(topic));
            }
            IndexedField::OrderHash => {
                order_hash = Some(bytes32_hex(topic));
            }
            IndexedField::ExecutionId => {
                execution_id = Some(bytes32_hex(topic));
            }
            IndexedField::SeriesId => {
                series_id = Some(bytes32_hex(topic));
            }
            IndexedField::IntentHash => {
                payload.insert("intent_hash".to_string(), Value::String(bytes32_hex(topic)));
            }
            IndexedField::Actor => {
                payload.insert(
                    "actor".to_string(),
                    Value::String(topic_to_address_hex(topic)),
                );
            }
            IndexedField::Skip => {
                payload.insert(
                    format!("indexed_slot_{}", i),
                    Value::String(bytes32_hex(topic)),
                );
            }
        }
    }

    // Extract data fields.
    let mut event_version = CANONICAL_EVENT_VERSION;
    for (i, field) in layout.data.iter().enumerate() {
        let word = &log.data[i * 32..(i + 1) * 32];
        match field {
            DataField::Address(name) => {
                payload.insert((*name).to_string(), Value::String(word_to_address(word)));
                if name == &"token" && token.is_none() {
                    token = Some(word_to_address(word));
                }
            }
            DataField::U256(name) => {
                payload.insert((*name).to_string(), Value::String(u256_be_to_decimal(word)));
                if name == &"series_id" && series_id.is_none() {
                    series_id = Some(bytes32_hex_slice(word));
                }
            }
            DataField::I256(name) => {
                payload.insert((*name).to_string(), Value::String(i256_be_to_decimal(word)));
            }
            DataField::U128(name) => {
                payload.insert((*name).to_string(), Value::String(u256_be_to_decimal(word)));
            }
            DataField::U64(name) => {
                let n = read_u64_be(&word[24..32]);
                payload.insert((*name).to_string(), Value::from(n));
            }
            DataField::U32(name) => {
                let n = read_u32_be(&word[28..32]);
                payload.insert((*name).to_string(), Value::from(n));
            }
            DataField::U16(name) => {
                let n = u16::from_be_bytes([word[30], word[31]]);
                if name == &"_ev" {
                    event_version = n;
                } else {
                    payload.insert((*name).to_string(), Value::from(n));
                }
            }
            DataField::U8(name) => {
                payload.insert((*name).to_string(), Value::from(word[31]));
            }
            DataField::Bool(name) => {
                let b = word[31] != 0;
                payload.insert((*name).to_string(), Value::from(b));
            }
            DataField::Bytes32(name) => {
                payload.insert((*name).to_string(), Value::String(bytes32_hex_slice(word)));
                if name == &"buyer_order_hash" && order_hash.is_none() {
                    order_hash = Some(bytes32_hex_slice(word));
                }
            }
            DataField::Skip => {}
        }
    }

    // Convention rewrites so reducer field-name expectations match.
    match kind {
        EventKind::InternalTransfer => {
            // Pull from_subkey (topic1) and to_subkey (topic2) into payload.
            payload.insert(
                "from_subkey".to_string(),
                Value::String(bytes32_hex(&log.topics[1])),
            );
            payload.insert(
                "to_subkey".to_string(),
                Value::String(bytes32_hex(&log.topics[2])),
            );
        }
        EventKind::OptionPremiumTransferred => {
            payload.insert(
                "from_subkey".to_string(),
                Value::String(bytes32_hex(&log.topics[1])),
            );
            payload.insert(
                "to_subkey".to_string(),
                Value::String(bytes32_hex(&log.topics[2])),
            );
        }
        EventKind::OptionFeeCharged => {
            payload.insert(
                "fee_subkey".to_string(),
                Value::String(bytes32_hex(&log.topics[2])),
            );
        }
        EventKind::OptionRebatePaid => {
            payload.insert(
                "rebate_subkey".to_string(),
                Value::String(bytes32_hex(&log.topics[2])),
            );
        }
        EventKind::OptionOrderPairExecuted => {
            // Buyer/seller order hashes present in data; execution_id already
            // extracted from topic1.
            payload.insert(
                "buyer_subkey".to_string(),
                Value::String(bytes32_hex(&log.topics[3])),
            );
            // series_id in this event is u256; we surface both the bytes hex + decimal.
            if let Some(s) = payload.get("series_id").cloned() {
                payload.insert("series_id_uint".to_string(), s);
            }
        }
        _ => {}
    }

    if event_version != expected_event_version {
        return Err(DecoderError::EventVersionMismatch {
            event: entry.event,
            expected: expected_event_version,
            actual: event_version,
        });
    }

    Ok(HybridV2Event {
        kind,
        event_version,
        subkey,
        owner,
        subaccount_id,
        token,
        engine,
        execution_id,
        order_hash,
        series_id,
        payload: Value::Object(payload),
    })
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn addr_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn bytes32_hex(topic: &[u8; 32]) -> String {
    format!("0x{}", hex_lower(topic))
}

fn bytes32_hex_slice(word: &[u8]) -> String {
    format!("0x{}", hex_lower(word))
}

fn topic_to_address_hex(topic: &[u8; 32]) -> String {
    format!("0x{}", hex_lower(&topic[12..]))
}

fn word_to_address(word: &[u8]) -> String {
    format!("0x{}", hex_lower(&word[12..32]))
}

fn topic_to_u32_low(topic: &[u8; 32]) -> u32 {
    u32::from_be_bytes([topic[28], topic[29], topic[30], topic[31]])
}

fn read_u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_u64_be(bytes: &[u8]) -> u64 {
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(out)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", b);
    }
    out
}

fn u256_be_to_decimal(bytes: &[u8]) -> String {
    let mut buf = [0u8; 32];
    let n = bytes.len().min(32);
    buf[32 - n..].copy_from_slice(&bytes[..n]);
    u256_bytes_to_decimal(&buf)
}

fn u256_bytes_to_decimal(bytes: &[u8; 32]) -> String {
    if bytes[..16].iter().all(|b| *b == 0) {
        let mut hi = 0u128;
        for b in &bytes[16..] {
            hi = (hi << 8) | (*b as u128);
        }
        return hi.to_string();
    }
    let mut digits: Vec<u8> = Vec::with_capacity(78);
    let mut n = bytes.to_vec();
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

/// Convert a 32-byte big-endian two's-complement int256 to a decimal string.
fn i256_be_to_decimal(bytes: &[u8]) -> String {
    let mut buf = [0u8; 32];
    let n = bytes.len().min(32);
    buf[32 - n..].copy_from_slice(&bytes[..n]);
    if buf[0] & 0x80 == 0 {
        return u256_bytes_to_decimal(&buf);
    }
    // Negative: two's-complement -> magnitude = ~x + 1
    let mut mag = [0u8; 32];
    for i in 0..32 {
        mag[i] = !buf[i];
    }
    // Add 1
    let mut carry: u16 = 1;
    for i in (0..32).rev() {
        let s = mag[i] as u16 + carry;
        mag[i] = (s & 0xff) as u8;
        carry = s >> 8;
    }
    let mut out = "-".to_string();
    out.push_str(&u256_bytes_to_decimal(&mag));
    out
}

// -----------------------------------------------------------------
//                          MODULE JSON HELPER
// -----------------------------------------------------------------

/// Convenience used by fixtures: build a raw log from indexed hex topics
/// and a hex data blob.
pub fn build_raw_log(emitter: &str, topics: &[&str], data_hex: &str) -> CanonicalRawLog {
    let mut ts = Vec::with_capacity(topics.len());
    for t in topics {
        let stripped = t.trim_start_matches("0x");
        let padded = pad_hex(stripped, 64);
        let mut out = [0u8; 32];
        for i in 0..32 {
            let s = &padded[i * 2..i * 2 + 2];
            out[i] = u8::from_str_radix(s, 16).unwrap_or(0);
        }
        ts.push(out);
    }
    let d = data_hex.trim_start_matches("0x");
    let mut data = Vec::with_capacity(d.len() / 2);
    for i in (0..d.len()).step_by(2) {
        data.push(u8::from_str_radix(&d[i..i + 2], 16).unwrap_or(0));
    }
    // Pad the emitter to a standard 40-char hex address for lowercase
    // comparison against the manifest module set.
    let emit_stripped = emitter.trim_start_matches("0x");
    let emit_padded = format!("0x{}", pad_hex(emit_stripped, 40));
    CanonicalRawLog {
        emitter: emit_padded,
        topics: ts,
        data,
    }
}

fn pad_hex(s: &str, width: usize) -> String {
    if s.len() >= width {
        return s.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(width);
    for _ in s.len()..width {
        out.push('0');
    }
    out.push_str(&s.to_ascii_lowercase());
    out
}

/// Convenience: pack u256/u128/u32/u64/u16/u8/address/bool/bytes32 values
/// into a canonical ABI data blob.
pub fn pack_data(fields: &[PackField]) -> Vec<u8> {
    let mut out = Vec::with_capacity(fields.len() * 32);
    for f in fields {
        let mut word = [0u8; 32];
        match f {
            PackField::U256(dec) => {
                let bits = parse_dec_uint256(dec);
                word.copy_from_slice(&bits);
            }
            PackField::I256(signed) => {
                let bits = parse_dec_int256(signed);
                word.copy_from_slice(&bits);
            }
            PackField::U128(v) => {
                let n = v.parse::<u128>().unwrap_or(0);
                word[16..].copy_from_slice(&n.to_be_bytes());
            }
            PackField::U64(v) => {
                word[24..].copy_from_slice(&v.to_be_bytes());
            }
            PackField::U32(v) => {
                word[28..].copy_from_slice(&v.to_be_bytes());
            }
            PackField::U16(v) => {
                word[30..].copy_from_slice(&v.to_be_bytes());
            }
            PackField::U8(v) => {
                word[31] = *v;
            }
            PackField::Bool(b) => {
                word[31] = if *b { 1 } else { 0 };
            }
            PackField::Address(addr) => {
                let stripped = addr.trim_start_matches("0x");
                let padded = pad_hex(stripped, 40);
                for i in 0..20 {
                    let s = &padded[i * 2..i * 2 + 2];
                    word[12 + i] = u8::from_str_radix(s, 16).unwrap_or(0);
                }
            }
            PackField::Bytes32(hex) => {
                let stripped = hex.trim_start_matches("0x");
                let padded = pad_hex(stripped, 64);
                for i in 0..32 {
                    let s = &padded[i * 2..i * 2 + 2];
                    word[i] = u8::from_str_radix(s, 16).unwrap_or(0);
                }
            }
        }
        out.extend_from_slice(&word);
    }
    out
}

pub enum PackField<'a> {
    U256(&'a str),
    I256(&'a str),
    U128(&'a str),
    U64(u64),
    U32(u32),
    U16(u16),
    U8(u8),
    Bool(bool),
    Address(&'a str),
    Bytes32(&'a str),
}

fn parse_dec_uint256(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for ch in s.chars() {
        if !ch.is_ascii_digit() {
            continue;
        }
        let digit = (ch as u8 - b'0') as u16;
        let mut carry: u16 = digit;
        for byte in out.iter_mut().rev() {
            let product = (*byte as u16) * 10 + carry;
            *byte = (product & 0xff) as u8;
            carry = product >> 8;
        }
    }
    out
}

fn parse_dec_int256(s: &str) -> [u8; 32] {
    let (neg, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };
    let mag = parse_dec_uint256(digits);
    if !neg {
        return mag;
    }
    // Two's-complement negation.
    let mut inv = [0u8; 32];
    for i in 0..32 {
        inv[i] = !mag[i];
    }
    let mut carry: u16 = 1;
    for i in (0..32).rev() {
        let s = inv[i] as u16 + carry;
        inv[i] = (s & 0xff) as u8;
        carry = s >> 8;
    }
    inv
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
        let mut b = [0u8; 32];
        b[15] = 0x02;
        let expected = "680564733841876926926749214863536422912";
        assert_eq!(u256_be_to_decimal(&b), expected);
    }

    #[test]
    fn int256_negative_decodes() {
        // -1 in two's-complement is all 0xff
        let b = [0xffu8; 32];
        assert_eq!(i256_be_to_decimal(&b), "-1");
    }

    #[test]
    fn int256_negative_large() {
        // -(2^128)
        let mut b = [0xffu8; 32];
        for byte in b.iter_mut().skip(16) {
            *byte = 0;
        }
        // now b = 0xff...ff 00...00 = 2^256 - 2^128 = -(2^128)
        assert_eq!(
            i256_be_to_decimal(&b),
            "-340282366920938463463374607431768211456"
        );
    }

    #[test]
    fn pack_and_decode_deposit() {
        // Simulate a Deposit event.
        let subkey = "0xaa000000000000000000000000000000000000000000000000000000000000ff";
        let owner = "0x00000000000000000000000000000000000000000000000000000000000000dd";
        let sid_topic = "0x0000000000000000000000000000000000000000000000000000000000000007";
        let emitter = "0x0000000000000000000000000000000000001111";
        let topic0 = TopicCatalogue::get()
            .lookup_by_event("Deposit")
            .expect("deposit in catalogue")
            .topic0_hex_lower
            .to_string();
        let token = "0x0000000000000000000000000000000000002222";
        let data = pack_data(&[
            PackField::Address(token),
            PackField::U256("1000000"),
            PackField::Address("0x0000000000000000000000000000000000003333"),
            PackField::U16(1),
        ]);
        let data_hex = format!("0x{}", super::hex_lower(&data));
        let log = build_raw_log(emitter, &[&topic0, subkey, owner, sid_topic], &data_hex);
        let ev = decode_log(&log).expect("must decode");
        assert_eq!(ev.kind, EventKind::Deposit);
        assert_eq!(ev.subkey.as_deref(), Some(subkey));
        assert_eq!(ev.token.as_deref(), Some(token));
        assert_eq!(ev.subaccount_id, Some(7));
        assert_eq!(ev.payload.get("amount").unwrap().as_str(), Some("1000000"));
    }
}

// Value alias for `serde_json::Value::from(...)` on numeric primitives
// so match arms compile.
#[allow(dead_code)]
fn _bind_value_from() -> Value {
    json!(0u64)
}
