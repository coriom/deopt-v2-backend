//! RFQ-MULTI-LEG-CREATE-QUOTE-V1 — byte-freeze on the canonical v2
//! payload layout for create + quote submit.
//!
//! The wire is `action|k1=v1|k2=v2|…` (see
//! `canonical_payload_bytes` in `src/auth/write_authorization.rs`).
//! Multi-leg emits an explicit `legs_count` field before the leg
//! entries so a client cannot inject an extra leg without bumping
//! the count in the signature.

use deopt_v2_backend::auth::write_authorization::{
    canonical_payload_bytes, CanonicalValue, WriteAuthAction,
};
use deopt_v2_backend::types::{AccountId, Side};

fn taker() -> AccountId {
    AccountId::new("0x1111111111111111111111111111111111111111")
}

fn mm() -> AccountId {
    AccountId::new("0x2222222222222222222222222222222222222222")
}

// The frozen bytes below are computed via `canonical_payload_bytes`
// with the same field list the route handler builds. They are the
// bytes a signer commits to today — any change is a wire-breaking
// change and MUST bump the action name.

#[test]
fn part1_multi_leg_rfq_create_canonical_bytes_are_frozen_for_two_legs() {
    let fields: Vec<(&'static str, CanonicalValue)> = vec![
        ("taker", CanonicalValue::Address(taker())),
        ("subaccount_id", CanonicalValue::U64(2)),
        ("legs_count", CanonicalValue::U64(2)),
        (
            "leg_0_option_series_id",
            CanonicalValue::Str("ETH-30JAN2026-3000-C".to_string()),
        ),
        (
            "leg_0_side",
            CanonicalValue::Str(match Side::Buy {
                Side::Buy => "buy".to_string(),
                Side::Sell => "sell".to_string(),
            }),
        ),
        (
            "leg_0_size_1e8",
            CanonicalValue::Str("100000000".to_string()),
        ),
        ("leg_0_ratio_num", CanonicalValue::U64(1)),
        ("leg_0_ratio_den", CanonicalValue::U64(1)),
        (
            "leg_1_option_series_id",
            CanonicalValue::Str("ETH-30JAN2026-3100-C".to_string()),
        ),
        (
            "leg_1_side",
            CanonicalValue::Str(match Side::Sell {
                Side::Buy => "buy".to_string(),
                Side::Sell => "sell".to_string(),
            }),
        ),
        (
            "leg_1_size_1e8",
            CanonicalValue::Str("100000000".to_string()),
        ),
        ("leg_1_ratio_num", CanonicalValue::U64(1)),
        ("leg_1_ratio_den", CanonicalValue::U64(1)),
        ("ttl_ms", CanonicalValue::U64(30_000)),
    ];
    let bytes = canonical_payload_bytes(WriteAuthAction::OptionMultiLegRfqCreate, &fields);
    let expected = concat!(
        "OPTION_MULTI_LEG_RFQ_CREATE",
        "|taker=\"0x1111111111111111111111111111111111111111\"",
        "|subaccount_id=2",
        "|legs_count=2",
        "|leg_0_option_series_id=\"ETH-30JAN2026-3000-C\"",
        "|leg_0_side=\"buy\"",
        "|leg_0_size_1e8=\"100000000\"",
        "|leg_0_ratio_num=1",
        "|leg_0_ratio_den=1",
        "|leg_1_option_series_id=\"ETH-30JAN2026-3100-C\"",
        "|leg_1_side=\"sell\"",
        "|leg_1_size_1e8=\"100000000\"",
        "|leg_1_ratio_num=1",
        "|leg_1_ratio_den=1",
        "|ttl_ms=30000",
    );
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        expected,
        "MULTI_LEG_RFQ_CREATE canonical bytes must not drift; existing signers commit to this shape"
    );
}

#[test]
fn part2_multi_leg_quote_submit_canonical_bytes_are_frozen_for_two_legs() {
    let fields: Vec<(&'static str, CanonicalValue)> = vec![
        (
            "option_rfq_id",
            CanonicalValue::Str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
        ),
        ("mm_account", CanonicalValue::Address(mm())),
        ("subaccount_id", CanonicalValue::U64(3)),
        (
            "package_price_1e8",
            CanonicalValue::Str("50000000".to_string()),
        ),
        ("size_1e8", CanonicalValue::Str("100000000".to_string())),
        ("legs_count", CanonicalValue::U64(2)),
        (
            "leg_0_price_1e8",
            CanonicalValue::Str("12000000000".to_string()),
        ),
        (
            "leg_1_price_1e8",
            CanonicalValue::Str("11500000000".to_string()),
        ),
        ("client_quote_id", CanonicalValue::Str("cq-42".to_string())),
        ("quote_nonce", CanonicalValue::U64(4711)),
        ("quote_ttl_ms", CanonicalValue::U64(5_000)),
    ];
    let bytes = canonical_payload_bytes(WriteAuthAction::OptionMultiLegRfqQuoteSubmit, &fields);
    let expected = concat!(
        "OPTION_MULTI_LEG_RFQ_QUOTE_SUBMIT",
        "|option_rfq_id=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"",
        "|mm_account=\"0x2222222222222222222222222222222222222222\"",
        "|subaccount_id=3",
        "|package_price_1e8=\"50000000\"",
        "|size_1e8=\"100000000\"",
        "|legs_count=2",
        "|leg_0_price_1e8=\"12000000000\"",
        "|leg_1_price_1e8=\"11500000000\"",
        "|client_quote_id=\"cq-42\"",
        "|quote_nonce=4711",
        "|quote_ttl_ms=5000",
    );
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        expected,
        "MULTI_LEG_QUOTE_SUBMIT canonical bytes must not drift"
    );
}

#[test]
fn part3_action_names_are_isolated_from_single_leg() {
    // A single-leg accept nonce can never satisfy a multi-leg
    // create signature: the action name is different, so
    // `used_nonces_v2 (account, subaccount_id, action, nonce)` sees
    // distinct tuples.
    assert_ne!(
        WriteAuthAction::OptionRfqCreate.as_str(),
        WriteAuthAction::OptionMultiLegRfqCreate.as_str()
    );
    assert_ne!(
        WriteAuthAction::OptionRfqQuoteSubmit.as_str(),
        WriteAuthAction::OptionMultiLegRfqQuoteSubmit.as_str()
    );
    assert_ne!(
        WriteAuthAction::OptionRfqAccept.as_str(),
        WriteAuthAction::OptionMultiLegRfqAccept.as_str()
    );
    assert_ne!(
        WriteAuthAction::OptionRfqCancel.as_str(),
        WriteAuthAction::OptionMultiLegRfqCancel.as_str()
    );
}

// RFQ-MULTI-LEG-CANCEL-V1 — cancel canonical byte freeze.
#[test]
fn part5_multi_leg_cancel_canonical_bytes_are_frozen() {
    let fields: Vec<(&'static str, CanonicalValue)> = vec![
        ("taker", CanonicalValue::Address(taker())),
        ("subaccount_id", CanonicalValue::U64(2)),
        (
            "option_rfq_id",
            CanonicalValue::Str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
        ),
    ];
    let bytes = canonical_payload_bytes(WriteAuthAction::OptionMultiLegRfqCancel, &fields);
    let expected = concat!(
        "OPTION_MULTI_LEG_RFQ_CANCEL",
        "|taker=\"0x1111111111111111111111111111111111111111\"",
        "|subaccount_id=2",
        "|option_rfq_id=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"",
    );
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        expected,
        "MULTI_LEG_RFQ_CANCEL canonical bytes must not drift; existing signers commit to this shape"
    );
}

// RFQ-MULTI-LEG-ATOMIC-ACCEPT-V1 — accept canonical byte freeze.
#[test]
fn part4_multi_leg_accept_canonical_bytes_are_frozen_for_two_legs() {
    let fields: Vec<(&'static str, CanonicalValue)> = vec![
        ("taker", CanonicalValue::Address(taker())),
        ("subaccount_id", CanonicalValue::U64(2)),
        (
            "option_rfq_id",
            CanonicalValue::Str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string()),
        ),
        (
            "quote_id",
            CanonicalValue::Str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".to_string()),
        ),
        (
            "expected_package_price_1e8",
            CanonicalValue::Str("50000000".to_string()),
        ),
        ("legs_count", CanonicalValue::U64(2)),
        (
            "leg_0_price_1e8",
            CanonicalValue::Str("12000000000".to_string()),
        ),
        (
            "leg_1_price_1e8",
            CanonicalValue::Str("11500000000".to_string()),
        ),
    ];
    let bytes = canonical_payload_bytes(WriteAuthAction::OptionMultiLegRfqAccept, &fields);
    let expected = concat!(
        "OPTION_MULTI_LEG_RFQ_ACCEPT",
        "|taker=\"0x1111111111111111111111111111111111111111\"",
        "|subaccount_id=2",
        "|option_rfq_id=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"",
        "|quote_id=\"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\"",
        "|expected_package_price_1e8=\"50000000\"",
        "|legs_count=2",
        "|leg_0_price_1e8=\"12000000000\"",
        "|leg_1_price_1e8=\"11500000000\"",
    );
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        expected,
        "MULTI_LEG_RFQ_ACCEPT canonical bytes must not drift; existing signers commit to this shape"
    );
    // Guard: the mm variable is used only in part 2; touch it here to
    // avoid a dead-code warning if part 2 ever changes.
    let _ = mm();
}
