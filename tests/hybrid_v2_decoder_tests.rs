//! Decoder coverage: one success per canonical event kind + rejection tests.

use deopt_v2_backend::hybrid_v2::decoder::{
    build_raw_log, decode_log, decode_log_with, DecoderError, PackField,
};
use deopt_v2_backend::hybrid_v2::events::EventKind;
use deopt_v2_backend::hybrid_v2::topics::TopicCatalogue;

const EMITTER: &str = "0x0000000000000000000000000000000000000099";

fn topic_hex_for(event: &str) -> String {
    TopicCatalogue::get()
        .lookup_by_event(event)
        .unwrap_or_else(|| panic!("event {} not in pinned catalogue", event))
        .topic0_hex_lower
        .to_string()
}

fn bytes32(fill: u8) -> String {
    let mut s = String::from("0x");
    for _ in 0..32 {
        s.push_str(&format!("{:02x}", fill));
    }
    s
}

fn addr(fill: u8) -> String {
    let mut s = String::from("0x");
    for _ in 0..20 {
        s.push_str(&format!("{:02x}", fill));
    }
    s
}

fn sid_topic(id: u32) -> String {
    let mut s = String::from("0x");
    for _ in 0..28 {
        s.push_str("00");
    }
    for b in id.to_be_bytes() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn addr_topic(a: &str) -> String {
    let stripped = a.trim_start_matches("0x");
    let mut s = String::from("0x");
    for _ in 0..24 {
        s.push('0');
    }
    s.push_str(stripped);
    s
}

fn pack_bytes(fields: &[PackField]) -> String {
    let data = deopt_v2_backend::hybrid_v2::decoder::pack_data(fields);
    let mut out = String::from("0x");
    for b in data {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

// -------------------------- BOUNDARY TESTS --------------------------

#[test]
fn rejects_unknown_topic0() {
    let bad = format!("0x{}", "ab".repeat(32));
    let log = build_raw_log(EMITTER, &[&bad], "0x");
    assert!(matches!(
        decode_log(&log).unwrap_err(),
        DecoderError::UnknownTopic { .. }
    ));
}

#[test]
fn rejects_no_topics() {
    let log = build_raw_log(EMITTER, &[], "0x");
    assert!(matches!(
        decode_log(&log).unwrap_err(),
        DecoderError::NoTopics
    ));
}

#[test]
fn rejects_wrong_emitter() {
    let t = topic_hex_for("Deposit");
    let sk = bytes32(0xaa);
    let ow = addr_topic(&addr(0xbb));
    let sid = sid_topic(1);
    let data = pack_bytes(&[
        PackField::Address(&addr(0xcc)),
        PackField::U256("100"),
        PackField::Address(&addr(0xdd)),
        PackField::U16(1),
    ]);
    let log = build_raw_log(EMITTER, &[&t, &sk, &ow, &sid], &data);
    let err =
        decode_log_with(&log, Some("0x0000000000000000000000000000000000000001"), 1).unwrap_err();
    assert!(matches!(err, DecoderError::WrongEmitter { .. }));
}

#[test]
fn rejects_wrong_event_version() {
    let t = topic_hex_for("Deposit");
    let sk = bytes32(0xaa);
    let ow = addr_topic(&addr(0xbb));
    let sid = sid_topic(1);
    let data = pack_bytes(&[
        PackField::Address(&addr(0xcc)),
        PackField::U256("100"),
        PackField::Address(&addr(0xdd)),
        PackField::U16(2), // wrong version
    ]);
    let log = build_raw_log(EMITTER, &[&t, &sk, &ow, &sid], &data);
    let err = decode_log_with(&log, None, 1).unwrap_err();
    assert!(matches!(err, DecoderError::EventVersionMismatch { .. }));
}

#[test]
fn rejects_indexed_count_mismatch() {
    let t = topic_hex_for("CollateralLocked");
    let sk = bytes32(0xaa);
    // Missing 2 required indexed topics.
    let data = pack_bytes(&[PackField::U256("500"), PackField::U16(1)]);
    let log = build_raw_log(EMITTER, &[&t, &sk], &data);
    let err = decode_log(&log).unwrap_err();
    assert!(matches!(
        err,
        DecoderError::IndexedTopicCountMismatch { .. }
    ));
}

#[test]
fn rejects_data_too_short() {
    let t = topic_hex_for("Deposit");
    let sk = bytes32(0xaa);
    let ow = addr_topic(&addr(0xbb));
    let sid = sid_topic(1);
    // Only 1 word of data.
    let log = build_raw_log(
        EMITTER,
        &[&t, &sk, &ow, &sid],
        &pack_bytes(&[PackField::U16(1)]),
    );
    let err = decode_log(&log).unwrap_err();
    assert!(matches!(err, DecoderError::DataTooShort { .. }));
}

// -------------------------- HAPPY PATH TESTS --------------------------

#[test]
fn decode_subaccount_created() {
    let t = topic_hex_for("SubaccountCreated");
    let ow = addr_topic(&addr(0xa1));
    let sid = sid_topic(7);
    let sk = bytes32(0xcc);
    let data = pack_bytes(&[PackField::U256("100"), PackField::U16(1)]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &ow, &sid, &sk], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::SubaccountCreated);
    assert_eq!(ev.subaccount_id, Some(7));
    assert!(ev.subkey.is_some());
}

#[test]
fn decode_subaccount_lazy_registered() {
    let t = topic_hex_for("SubaccountLazyRegistered");
    let ow = addr_topic(&addr(0xa2));
    let sid = sid_topic(3);
    let sk = bytes32(0xdd);
    let data = pack_bytes(&[
        PackField::U256("101"),
        PackField::Address(&addr(0xee)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &ow, &sid, &sk], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::SubaccountLazyRegistered);
}

#[test]
fn decode_deposit_withdraw_internal_transfer() {
    for name in ["Deposit", "Withdraw"] {
        let t = topic_hex_for(name);
        let sk = bytes32(0xaa);
        let ow = addr_topic(&addr(0xbb));
        let sid = sid_topic(2);
        let data = pack_bytes(&[
            PackField::Address(&addr(0xe1)),
            PackField::U256("1000000"),
            PackField::Address(&addr(0xdd)),
            PackField::U16(1),
        ]);
        let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &ow, &sid], &data)).unwrap();
        assert_eq!(
            ev.payload.get("amount").and_then(|v| v.as_str()),
            Some("1000000")
        );
    }
    let t = topic_hex_for("InternalTransfer");
    let sk_from = bytes32(0x11);
    let sk_to = bytes32(0x22);
    let tok = addr_topic(&addr(0xe1));
    let data = pack_bytes(&[
        PackField::U256("500"),
        PackField::Address(&addr(0xee)),
        PackField::U32(1),
        PackField::U32(2),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(
        EMITTER,
        &[&t, &sk_from, &sk_to, &tok],
        &data,
    ))
    .unwrap();
    assert_eq!(ev.kind, EventKind::InternalTransfer);
    assert!(ev.payload.get("from_subkey").is_some());
    assert!(ev.payload.get("to_subkey").is_some());
    assert_eq!(
        ev.payload.get("amount").and_then(|v| v.as_str()),
        Some("500")
    );
}

#[test]
fn decode_reservation_events() {
    for name in ["CollateralLocked", "CollateralUnlocked"] {
        let t = topic_hex_for(name);
        let sk = bytes32(0xaa);
        let tok = addr_topic(&addr(0xe1));
        let eng = addr_topic(&addr(0xf2));
        let data = pack_bytes(&[PackField::U256("500"), PackField::U16(1)]);
        let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &tok, &eng], &data)).unwrap();
        assert_eq!(ev.engine.is_some(), true);
        assert_eq!(
            ev.payload.get("amount").and_then(|v| v.as_str()),
            Some("500")
        );
    }
    // OrphanedLockReleased has same topics + extra data.
    let t = topic_hex_for("OrphanedLockReleased");
    let sk = bytes32(0xaa);
    let tok = addr_topic(&addr(0xe1));
    let eng = addr_topic(&addr(0xf2));
    let data = pack_bytes(&[
        PackField::U256("500"),
        PackField::U256("0"),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &tok, &eng], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OrphanedLockReleased);
}

#[test]
fn decode_engine_capability_changed() {
    let t = topic_hex_for("EngineCapabilityChanged");
    let eng = addr_topic(&addr(0xf2));
    let data = pack_bytes(&[
        PackField::U256("15"),
        PackField::U256("0"),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &eng], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::EngineCapabilityChanged);
    assert_eq!(
        ev.payload.get("added_bits").and_then(|v| v.as_str()),
        Some("15")
    );
}

#[test]
fn decode_engine_guardian_and_guardian_changed() {
    for name in ["EngineGuardianRevoked", "GuardianChanged"] {
        let t = topic_hex_for(name);
        let a = addr_topic(&addr(0xa5));
        let b = addr_topic(&addr(0xb6));
        let data = pack_bytes(&[PackField::U16(1)]);
        let ev = decode_log(&build_raw_log(EMITTER, &[&t, &a, &b], &data)).unwrap();
        assert!(matches!(
            ev.kind,
            EventKind::EngineGuardianRevoked | EventKind::GuardianChanged
        ));
    }
}

#[test]
fn decode_supported_token_add_remove() {
    for name in ["SupportedTokenAdded", "SupportedTokenRemoved"] {
        let t = topic_hex_for(name);
        let tok = addr_topic(&addr(0xe1));
        let data = pack_bytes(&[PackField::U16(1)]);
        let ev = decode_log(&build_raw_log(EMITTER, &[&t, &tok], &data)).unwrap();
        assert!(ev.token.is_some());
    }
}

#[test]
fn decode_collateral_universe_entry() {
    let t = topic_hex_for("CollateralTokenEnteredUniverse");
    let tok = addr_topic(&addr(0xe1));
    let data = pack_bytes(&[PackField::U256("3"), PackField::U16(1)]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &tok], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::CollateralTokenEnteredUniverse);
    assert_eq!(
        ev.payload.get("universe_index").and_then(|v| v.as_str()),
        Some("3")
    );
}

#[test]
fn decode_option_premium_fee_rebate() {
    for name in [
        "OptionPremiumTransferred",
        "OptionFeeCharged",
        "OptionRebatePaid",
    ] {
        let t = topic_hex_for(name);
        let sk_a = bytes32(0x11);
        let sk_b = bytes32(0x22);
        let tok = addr_topic(&addr(0xe1));
        let data = pack_bytes(&[
            PackField::U256("42"),
            PackField::Address(&addr(0xff)),
            PackField::U16(1),
        ]);
        let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk_a, &sk_b, &tok], &data)).unwrap();
        assert_eq!(
            ev.payload.get("amount").and_then(|v| v.as_str()),
            Some("42")
        );
    }
}

#[test]
fn decode_recovery_finalization_withdrawn() {
    let t = topic_hex_for("RecoveryFinalizationWithdrawn");
    let sk = bytes32(0xdd);
    let ow = addr_topic(&addr(0xa1));
    let tok = addr_topic(&addr(0xe1));
    let data = pack_bytes(&[
        PackField::U256("100"),
        PackField::Address(&addr(0xff)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &ow, &tok], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::RecoveryFinalizationWithdrawn);
}

#[test]
fn decode_pause_and_bad_debt() {
    let t = topic_hex_for("PauseFlagChanged");
    let sk = bytes32(0xaa);
    let data = pack_bytes(&[
        PackField::Bool(true),
        PackField::Address(&addr(0xa1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::PauseFlagChanged);
    assert_eq!(
        ev.payload.get("paused").and_then(|v| v.as_bool()),
        Some(true)
    );

    let t = topic_hex_for("BadDebtSocialized");
    let sk = bytes32(0xaa);
    let tok = addr_topic(&addr(0xe1));
    let data = pack_bytes(&[PackField::U256("77"), PackField::U16(1)]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &tok], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::BadDebtSocialized);
    assert_eq!(
        ev.payload.get("amount").and_then(|v| v.as_str()),
        Some("77")
    );
}

#[test]
fn decode_initialisation_events() {
    let t = topic_hex_for("ProtocolSubaccountsInitialized");
    let data = pack_bytes(&[
        PackField::Bytes32(&bytes32(0x11)),
        PackField::Bytes32(&bytes32(0x22)),
        PackField::Bytes32(&bytes32(0x33)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::ProtocolSubaccountsInitialized);
    for name in [
        "EscapeControllerInitialized",
        "RecoveryFinalizerInitialized",
    ] {
        let t = topic_hex_for(name);
        let a = addr_topic(&addr(0xa1));
        let data = pack_bytes(&[PackField::U16(1)]);
        let ev = decode_log(&build_raw_log(EMITTER, &[&t, &a], &data)).unwrap();
        assert!(matches!(
            ev.kind,
            EventKind::EscapeControllerInitialized | EventKind::RecoveryFinalizerInitialized
        ));
    }
}

#[test]
fn decode_option_positions() {
    let sk = bytes32(0xaa);
    let series = bytes32(0x99);
    let t = topic_hex_for("OptionPositionOpened");
    let data = pack_bytes(&[
        PackField::U8(0),
        PackField::U128("1000"),
        PackField::U128("0"),
        PackField::Address(&addr(0xa1)),
        PackField::Address(&addr(0xb1)),
        PackField::U32(1),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &series], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OptionPositionOpened);

    let t = topic_hex_for("OptionPositionModified");
    let data = pack_bytes(&[
        PackField::U8(0),
        PackField::I256("-100"),
        PackField::U128("0"),
        PackField::Address(&addr(0xa1)),
        PackField::Address(&addr(0xb1)),
        PackField::U32(1),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &series], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OptionPositionModified);
    assert_eq!(
        ev.payload
            .get("long_delta_1e8_signed")
            .and_then(|v| v.as_str()),
        Some("-100")
    );

    let t = topic_hex_for("OptionPositionClosed");
    let data = pack_bytes(&[
        PackField::U8(0),
        PackField::Address(&addr(0xa1)),
        PackField::Address(&addr(0xb1)),
        PackField::U32(1),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &series], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OptionPositionClosed);
}

#[test]
fn decode_option_orders_and_min_nonce() {
    let sk = bytes32(0xaa);
    let oh = bytes32(0xbb);
    let t = topic_hex_for("OptionOrderFilled");
    let data = pack_bytes(&[
        PackField::U256("1"),
        PackField::U8(0),
        PackField::U8(0),
        PackField::U128("100"),
        PackField::U128("500"),
        PackField::U128("100"),
        PackField::U128("400"),
        PackField::Bool(false),
        PackField::U8(0),
        PackField::Address(&addr(0xa1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &oh], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OptionOrderFilled);
    assert_eq!(
        ev.payload.get("filled_delta_1e8").and_then(|v| v.as_str()),
        Some("100")
    );

    let t = topic_hex_for("OptionOrderCancelled");
    let data = pack_bytes(&[
        PackField::Address(&addr(0xa1)),
        PackField::Address(&addr(0xb1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &oh], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OptionOrderCancelled);

    let t = topic_hex_for("OptionSubaccountMinValidOrderNonceAdvanced");
    let ow = addr_topic(&addr(0xa1));
    let data = pack_bytes(&[
        PackField::U256("1"),
        PackField::U256("5"),
        PackField::Address(&addr(0xb1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &ow], &data)).unwrap();
    assert_eq!(
        ev.payload
            .get("new_min_valid_nonce")
            .and_then(|v| v.as_str()),
        Some("5")
    );
}

#[test]
fn decode_recovery_events() {
    let sk = bytes32(0xdd);
    let ow = addr_topic(&addr(0xa1));
    let sid = sid_topic(1);
    for (name, data) in [
        (
            "RecoveryRequested",
            pack_bytes(&[
                PackField::U256("1"),
                PackField::U64(9999),
                PackField::U16(1),
            ]),
        ),
        (
            "RecoveryActivated",
            pack_bytes(&[PackField::U256("1"), PackField::U16(1)]),
        ),
        ("RecoveryCancelled", pack_bytes(&[PackField::U16(1)])),
        (
            "RecoveryFinalized",
            pack_bytes(&[
                PackField::U256("1"),
                PackField::U64(1000),
                PackField::U8(0),
                PackField::Address(&addr(0xff)),
                PackField::U16(1),
            ]),
        ),
    ] {
        let t = topic_hex_for(name);
        let _ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &ow, &sid], &data)).unwrap();
    }
    let t = topic_hex_for("RecoveryEpochIncremented");
    let data = pack_bytes(&[PackField::U8(0), PackField::U256("2"), PackField::U16(1)]);
    let _ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &ow], &data)).unwrap();

    let t = topic_hex_for("RecoveryPauseSet");
    let data = pack_bytes(&[
        PackField::Bool(true),
        PackField::U64(999),
        PackField::Address(&addr(0xa1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t], &data)).unwrap();
    assert_eq!(
        ev.payload.get("paused").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn decode_replay_and_epoch_events() {
    let sk = bytes32(0xdd);
    let ow = addr_topic(&addr(0xa1));
    let t = topic_hex_for("IntentConsumed");
    let ih = bytes32(0x77);
    let data = pack_bytes(&[
        PackField::Address(&addr(0xa1)),
        PackField::Bytes32(&bytes32(0)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &ih, &ow], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::IntentConsumed);

    let t = topic_hex_for("NonceCancelled");
    let data = pack_bytes(&[
        PackField::U256("0"),
        PackField::U256("5"),
        PackField::Address(&addr(0xa1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &ow], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::NonceCancelled);

    let t = topic_hex_for("OwnerRecoveryEpochAdvanced");
    let data = pack_bytes(&[
        PackField::U256("0"),
        PackField::U256("1"),
        PackField::Address(&addr(0xa1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &ow], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OwnerRecoveryEpochAdvanced);

    let t = topic_hex_for("SubaccountRecoveryEpochAdvanced");
    let data = pack_bytes(&[
        PackField::U32(1),
        PackField::U256("0"),
        PackField::U256("1"),
        PackField::Address(&addr(0xa1)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &ow], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::SubaccountRecoveryEpochAdvanced);
}

#[test]
fn decode_risk_and_manifest_events() {
    let t = topic_hex_for("RiskParamsSet");
    let paramsHash = bytes32(0x66);
    let data = pack_bytes(&[PackField::U256("0"), PackField::U16(1)]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &paramsHash], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::RiskParamsSet);

    let t = topic_hex_for("RiskModuleActivated");
    let data = pack_bytes(&[PackField::U16(1), PackField::U16(1), PackField::U16(1)]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::RiskModuleActivated);

    let t = topic_hex_for("LiquidationTriggered");
    let sk = bytes32(0xaa);
    let data = pack_bytes(&[PackField::U8(0), PackField::U16(1)]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::LiquidationTriggered);

    let t = topic_hex_for("DeploymentManifestDeclared");
    let mkey = bytes32(0x55);
    let data = pack_bytes(&[
        PackField::U256("84532"),
        PackField::Address(&addr(0xa1)),
        PackField::Bytes32(&bytes32(0x77)),
        PackField::U16(1),
        PackField::U16(1),
        PackField::U16(1),
        PackField::Bytes32(&bytes32(0x88)),
        PackField::Bytes32(&bytes32(0x99)),
        PackField::U64(1000),
        PackField::U64(2000),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &mkey], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::DeploymentManifestDeclared);
}

#[test]
fn decode_int256_negative_roundtrip() {
    let sk = bytes32(0xaa);
    let series = bytes32(0x99);
    let t = topic_hex_for("OptionExercised");
    let data = pack_bytes(&[
        PackField::U128("100"),
        PackField::U128("500"),
        PackField::I256("-42"),
        PackField::Address(&addr(0xa1)),
        PackField::U32(1),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &series], &data)).unwrap();
    assert_eq!(ev.kind, EventKind::OptionExercised);
    assert_eq!(
        ev.payload.get("pnl_1e8").and_then(|v| v.as_str()),
        Some("-42")
    );
}

#[test]
fn decode_max_uint256_roundtrip() {
    let t = topic_hex_for("Deposit");
    let sk = bytes32(0xaa);
    let ow = addr_topic(&addr(0xbb));
    let sid = sid_topic(1);
    let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    let data = pack_bytes(&[
        PackField::Address(&addr(0xe1)),
        PackField::U256(max),
        PackField::Address(&addr(0xff)),
        PackField::U16(1),
    ]);
    let ev = decode_log(&build_raw_log(EMITTER, &[&t, &sk, &ow, &sid], &data)).unwrap();
    assert_eq!(ev.payload.get("amount").and_then(|v| v.as_str()), Some(max));
}

// Coverage summary: at least one success decode per canonical event
// touched by the tests above; boundary tests exercise every declared
// DecoderError variant.
