//! BACKEND WP-01 decoder tests.
//!
//! Verify the canonical Hybrid V2 decoder against synthetic logs that
//! match the pinned Solidity signatures. Signatures come from the
//! pinned catalogue — the decoder MUST never invent a topic.

use deopt_v2_backend::hybrid_v2::decoder::{decode_log, CanonicalRawLog, DecoderError};
use deopt_v2_backend::hybrid_v2::events::EventKind;
use deopt_v2_backend::hybrid_v2::topics::TopicCatalogue;

fn topic_for(event: &str) -> [u8; 32] {
    TopicCatalogue::get()
        .lookup_by_event(event)
        .unwrap_or_else(|| panic!("event {} not in pinned catalogue", event))
        .topic0
}

fn pad_left(word: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let start = 32 - word.len();
    out[start..].copy_from_slice(word);
    out
}

fn as_addr_topic(addr_hex_low20: &[u8; 20]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..32].copy_from_slice(addr_hex_low20);
    out
}

fn u256_word(value: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&value.to_be_bytes());
    out
}

#[test]
fn decoder_rejects_unknown_topic0() {
    let log = CanonicalRawLog {
        emitter: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into(),
        topics: vec![[0x77; 32]],
        data: vec![],
    };
    let err = decode_log(&log).unwrap_err();
    assert!(matches!(err, DecoderError::UnknownTopic { .. }));
}

#[test]
fn decoder_rejects_zero_topics() {
    let log = CanonicalRawLog {
        emitter: "0x0000000000000000000000000000000000000001".into(),
        topics: vec![],
        data: vec![],
    };
    let err = decode_log(&log).unwrap_err();
    assert!(matches!(err, DecoderError::NoTopics));
}

#[test]
fn decoder_decodes_subaccount_created() {
    let mut owner = [0u8; 20];
    owner[19] = 0xA1;
    let subkey = [0xCC; 32];
    let log = CanonicalRawLog {
        emitter: "0x0000000000000000000000000000000000000001".into(),
        topics: vec![
            topic_for("SubaccountCreated"),
            as_addr_topic(&owner),
            u256_word(7), // subaccountId
            subkey,       // subKey (bytes32, indexed)
        ],
        data: {
            // chainId (u256) + eventVersion (u16 packed as u256)
            let mut d = Vec::new();
            d.extend_from_slice(&u256_word(84532));
            d.extend_from_slice(&u256_word(1));
            d
        },
    };
    let event = decode_log(&log).unwrap();
    assert_eq!(event.kind, EventKind::SubaccountCreated);
    assert_eq!(
        event.owner.as_deref(),
        Some("0x00000000000000000000000000000000000000a1")
    );
    assert_eq!(event.subaccount_id, Some(7));
    assert_eq!(
        event.subkey.as_deref(),
        Some("0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
    );
    assert_eq!(event.event_version, 1);
}

#[test]
fn decoder_decodes_deposit() {
    let subkey = [0xAA; 32];
    let mut owner = [0u8; 20];
    owner[19] = 0xB0;
    let sid_topic = u256_word(3);
    let mut token = [0u8; 20];
    token[19] = 0xE1;
    let depositor = token; // 20-byte address, same shape
    let mut data = Vec::new();
    data.extend_from_slice(&as_addr_topic(&token)); // token
    data.extend_from_slice(&u256_word(1_000)); // amount
    data.extend_from_slice(&as_addr_topic(&depositor)); // depositor
    data.extend_from_slice(&u256_word(1)); // eventVersion

    let log = CanonicalRawLog {
        emitter: "0x0000000000000000000000000000000000000002".into(),
        topics: vec![
            topic_for("Deposit"),
            subkey,
            as_addr_topic(&owner),
            sid_topic,
        ],
        data,
    };
    let event = decode_log(&log).unwrap();
    assert_eq!(event.kind, EventKind::Deposit);
    assert_eq!(event.subaccount_id, Some(3));
    assert_eq!(
        event.token.as_deref(),
        Some("0x00000000000000000000000000000000000000e1")
    );
    assert_eq!(
        event.payload.get("amount").and_then(|v| v.as_str()),
        Some("1000")
    );
}

#[test]
fn decoder_decodes_collateral_locked() {
    let subkey = [0xBB; 32];
    let mut token = [0u8; 20];
    token[19] = 0xE1;
    let mut engine = [0u8; 20];
    engine[19] = 0xF2;

    let mut data = Vec::new();
    data.extend_from_slice(&u256_word(500)); // amount
    data.extend_from_slice(&u256_word(1)); // eventVersion
    let log = CanonicalRawLog {
        emitter: "0x0000000000000000000000000000000000000002".into(),
        topics: vec![
            topic_for("CollateralLocked"),
            subkey,
            as_addr_topic(&token),
            as_addr_topic(&engine),
        ],
        data,
    };
    let event = decode_log(&log).unwrap();
    assert_eq!(event.kind, EventKind::CollateralLocked);
    assert_eq!(
        event.engine.as_deref(),
        Some("0x00000000000000000000000000000000000000f2")
    );
    assert_eq!(
        event.payload.get("amount").and_then(|v| v.as_str()),
        Some("500")
    );
}

#[test]
fn decoder_decodes_recovery_finalized_shape() {
    let subkey = [0xDD; 32];
    let mut owner = [0u8; 20];
    owner[19] = 0xC0;
    let sid_topic = u256_word(1);
    // Data has multiple non-indexed fields; we only require decoder to
    // populate the identity slots.
    let mut data = Vec::new();
    for _ in 0..5 {
        data.extend_from_slice(&u256_word(0));
    }
    data.extend_from_slice(&u256_word(1)); // eventVersion at end

    let log = CanonicalRawLog {
        emitter: "0x0000000000000000000000000000000000000008".into(),
        topics: vec![
            topic_for("RecoveryFinalized"),
            subkey,
            as_addr_topic(&owner),
            sid_topic,
        ],
        data,
    };
    let event = decode_log(&log).unwrap();
    assert_eq!(event.kind, EventKind::RecoveryFinalized);
    assert_eq!(event.subaccount_id, Some(1));
    assert_eq!(
        event.owner.as_deref(),
        Some("0x00000000000000000000000000000000000000c0")
    );
}

// Suppress an unused-import warning if `pad_left` is trimmed later.
#[allow(dead_code)]
fn _unused_helper_ok() -> [u8; 32] {
    pad_left(&[0u8; 4])
}
