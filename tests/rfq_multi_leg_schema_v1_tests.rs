//! RFQ-MULTI-LEG-SCHEMA-V1 — foundation-only assertions.
//!
//! These tests pin the promises the schema milestone makes without
//! requiring a live Postgres. They cover:
//!
//! * feature flag defaults + parser round-trip;
//! * bounded leg count (2..=8) at the validation layer;
//! * contiguity + parent-id + ratio + size + price invariants;
//! * status enum re-exports round-trip through their string form;
//! * effective-status flip from Open → Expired on the multi-leg
//!   struct mirrors the single-leg behavior.
//!
//! Live Postgres repository CRUD assertions live in the sibling
//! `rfq_multi_leg_pg_proof.rs`, which is gated on the
//! `RFQ_MULTI_LEG_PG_TEST_DATABASE_URL` env var (skipped when
//! unset — matches the pattern used by
//! `perps_funding_pg_proof.rs` and `conditional_orders_pg_proof.rs`).

use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::options::{
    validate_multi_leg_composition, validate_multi_leg_fill_composition,
    validate_multi_leg_quote_composition, OptionMultiLegRfqFillLeg, OptionMultiLegRfqLeg,
    OptionMultiLegRfqQuoteLeg, OptionMultiLegRfqQuoteSignatureStatus, OptionMultiLegRfqQuoteStatus,
    OptionMultiLegRfqRequest, OptionMultiLegRfqStatus, OptionsConfig, MAX_LEGS_PER_MULTI_LEG_RFQ,
    MIN_LEGS_PER_MULTI_LEG_RFQ,
};
use deopt_v2_backend::types::{AccountId, Side};
use uuid::Uuid;

const RFQ_UUID: Uuid = uuid::uuid!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
const QUOTE_UUID: Uuid = uuid::uuid!("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
const FILL_UUID: Uuid = uuid::uuid!("cccccccc-cccc-cccc-cccc-cccccccccccc");

fn taker() -> AccountId {
    AccountId::new("0x1111111111111111111111111111111111111111")
}

fn make_leg(index: u32) -> OptionMultiLegRfqLeg {
    OptionMultiLegRfqLeg {
        option_rfq_id: RFQ_UUID,
        leg_index: index,
        option_series_id: format!("ETH-{index}"),
        side: if index % 2 == 0 {
            Side::Buy
        } else {
            Side::Sell
        },
        size_1e8: 100_000_000,
        ratio_num: 1,
        ratio_den: 1,
    }
}

// ---------------------------------------------------------------------
// Feature flag.
// ---------------------------------------------------------------------

#[test]
fn part1_options_config_disabled_leaves_multi_leg_flag_false() {
    let config = OptionsConfig::disabled();
    assert!(
        !config.rfq_multi_leg_enabled,
        "OptionsConfig::disabled() must leave rfq_multi_leg_enabled=false"
    );
}

#[test]
fn part1_options_config_enabled_in_memory_for_tests_still_defaults_multi_leg_off() {
    // The multi-leg subsystem is orthogonal to the master OPTIONS_ENABLED
    // switch. Turning options on for tests must NOT accidentally
    // unlock the multi-leg schema paths.
    let config = OptionsConfig::enabled_in_memory_for_tests();
    assert!(
        !config.rfq_multi_leg_enabled,
        "enabled_in_memory_for_tests must not opt in to multi-leg"
    );
}

// ---------------------------------------------------------------------
// Leg count bounds.
// ---------------------------------------------------------------------

#[test]
fn part2_validate_rejects_zero_legs() {
    let err = validate_multi_leg_composition(RFQ_UUID, &[]).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part2_validate_rejects_single_leg() {
    let err = validate_multi_leg_composition(RFQ_UUID, &[make_leg(0)]).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part2_validate_accepts_boundary_min_two_legs() {
    let legs: Vec<_> = (0..MIN_LEGS_PER_MULTI_LEG_RFQ as u32)
        .map(make_leg)
        .collect();
    validate_multi_leg_composition(RFQ_UUID, &legs).expect("2 legs must validate");
}

#[test]
fn part2_validate_accepts_boundary_max_eight_legs() {
    let legs: Vec<_> = (0..MAX_LEGS_PER_MULTI_LEG_RFQ as u32)
        .map(make_leg)
        .collect();
    validate_multi_leg_composition(RFQ_UUID, &legs).expect("8 legs must validate");
}

#[test]
fn part2_validate_rejects_nine_legs() {
    let legs: Vec<_> = (0..9u32).map(make_leg).collect();
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part2_leg_bounds_constants_are_correct() {
    assert_eq!(MIN_LEGS_PER_MULTI_LEG_RFQ, 2);
    assert_eq!(MAX_LEGS_PER_MULTI_LEG_RFQ, 8);
}

// ---------------------------------------------------------------------
// Contiguity + invariants.
// ---------------------------------------------------------------------

#[test]
fn part3_validate_rejects_gap_in_leg_index() {
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[1].leg_index = 3;
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part3_validate_rejects_duplicate_leg_index() {
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[1].leg_index = 0;
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part3_validate_rejects_leg_referencing_different_rfq() {
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[1].option_rfq_id = uuid::uuid!("dddddddd-dddd-dddd-dddd-dddddddddddd");
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part3_validate_rejects_zero_ratio_num() {
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[0].ratio_num = 0;
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part3_validate_rejects_zero_ratio_den() {
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[1].ratio_den = 0;
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part3_validate_rejects_zero_size() {
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[0].size_1e8 = 0;
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[test]
fn part3_validate_accepts_non_unit_ratios() {
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[0].ratio_num = 2;
    legs[0].ratio_den = 3;
    legs[1].ratio_num = 5;
    legs[1].ratio_den = 7;
    validate_multi_leg_composition(RFQ_UUID, &legs).expect("non-unit ratios must validate");
}

// ---------------------------------------------------------------------
// Quote-leg + fill-leg symmetry.
// ---------------------------------------------------------------------

#[test]
fn part4_validate_quote_legs_count_must_match_rfq_leg_count() {
    let legs = vec![OptionMultiLegRfqQuoteLeg {
        quote_id: QUOTE_UUID,
        leg_index: 0,
        price_1e8: 1000,
    }];
    let err = validate_multi_leg_quote_composition(QUOTE_UUID, 2, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[test]
fn part4_validate_quote_legs_reject_zero_price() {
    let legs = vec![
        OptionMultiLegRfqQuoteLeg {
            quote_id: QUOTE_UUID,
            leg_index: 0,
            price_1e8: 1000,
        },
        OptionMultiLegRfqQuoteLeg {
            quote_id: QUOTE_UUID,
            leg_index: 1,
            price_1e8: 0,
        },
    ];
    let err = validate_multi_leg_quote_composition(QUOTE_UUID, 2, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[test]
fn part4_validate_quote_legs_reject_wrong_parent_on_leg() {
    let legs = vec![
        OptionMultiLegRfqQuoteLeg {
            quote_id: QUOTE_UUID,
            leg_index: 0,
            price_1e8: 1000,
        },
        OptionMultiLegRfqQuoteLeg {
            quote_id: uuid::uuid!("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"),
            leg_index: 1,
            price_1e8: 2000,
        },
    ];
    let err = validate_multi_leg_quote_composition(QUOTE_UUID, 2, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[test]
fn part4_validate_fill_legs_reject_gap() {
    let legs = vec![
        OptionMultiLegRfqFillLeg {
            fill_id: FILL_UUID,
            leg_index: 0,
            option_series_id: "ETH-0".to_string(),
            side: Side::Buy,
            size_1e8: 100,
            price_1e8: 10,
        },
        OptionMultiLegRfqFillLeg {
            fill_id: FILL_UUID,
            leg_index: 2,
            option_series_id: "ETH-2".to_string(),
            side: Side::Sell,
            size_1e8: 100,
            price_1e8: 10,
        },
    ];
    let err = validate_multi_leg_fill_composition(FILL_UUID, 2, &legs).unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[test]
fn part4_validate_fill_legs_accept_valid_composition() {
    let legs = vec![
        OptionMultiLegRfqFillLeg {
            fill_id: FILL_UUID,
            leg_index: 0,
            option_series_id: "ETH-0".to_string(),
            side: Side::Buy,
            size_1e8: 100,
            price_1e8: 10,
        },
        OptionMultiLegRfqFillLeg {
            fill_id: FILL_UUID,
            leg_index: 1,
            option_series_id: "ETH-1".to_string(),
            side: Side::Sell,
            size_1e8: 100,
            price_1e8: 10,
        },
    ];
    validate_multi_leg_fill_composition(FILL_UUID, 2, &legs)
        .expect("valid fill composition must accept");
}

// ---------------------------------------------------------------------
// Status enum aliasing.
// ---------------------------------------------------------------------

#[test]
fn part5_multi_leg_status_reuses_single_leg_tokens() {
    // The re-export must preserve the exact wire tokens so any
    // downstream consumer that already handles single-leg RFQ
    // statuses can decode multi-leg statuses without change.
    assert_eq!(OptionMultiLegRfqStatus::Open.as_str(), "open");
    assert_eq!(OptionMultiLegRfqStatus::Expired.as_str(), "expired");
    assert_eq!(OptionMultiLegRfqStatus::Accepted.as_str(), "accepted");
    assert_eq!(OptionMultiLegRfqStatus::Cancelled.as_str(), "cancelled");
    assert_eq!(OptionMultiLegRfqStatus::Failed.as_str(), "failed");

    assert_eq!(OptionMultiLegRfqQuoteStatus::Active.as_str(), "active");
    assert_eq!(OptionMultiLegRfqQuoteStatus::Expired.as_str(), "expired");
    assert_eq!(OptionMultiLegRfqQuoteStatus::Accepted.as_str(), "accepted");
    assert_eq!(OptionMultiLegRfqQuoteStatus::Rejected.as_str(), "rejected");
    assert_eq!(
        OptionMultiLegRfqQuoteStatus::Cancelled.as_str(),
        "cancelled"
    );

    assert_eq!(
        OptionMultiLegRfqQuoteSignatureStatus::NotRequired.as_str(),
        "not_required"
    );
    assert_eq!(
        OptionMultiLegRfqQuoteSignatureStatus::Verified.as_str(),
        "verified"
    );
}

#[test]
fn part5_multi_leg_rfq_effective_status_flips_after_expiry() {
    let rfq = OptionMultiLegRfqRequest {
        option_rfq_id: RFQ_UUID,
        taker: taker(),
        taker_subaccount_id: 1,
        status: OptionMultiLegRfqStatus::Open,
        created_at_ms: 0,
        expires_at_ms: 100,
        accepted_quote_id: None,
        accepted_fill_id: None,
    };
    assert_eq!(rfq.effective_status(50), OptionMultiLegRfqStatus::Open);
    assert_eq!(rfq.effective_status(100), OptionMultiLegRfqStatus::Expired);
    assert_eq!(rfq.effective_status(200), OptionMultiLegRfqStatus::Expired);
}

// ---------------------------------------------------------------------
// No secrets in error messages.
// ---------------------------------------------------------------------

#[test]
fn part6_validation_error_messages_do_not_carry_addresses_or_secrets() {
    // Give a leg a very unusual address-looking string in a place it
    // would be tempting to embed it in an error message. Confirm the
    // resulting error message does NOT echo the address.
    let mut legs = vec![make_leg(0), make_leg(1)];
    legs[1].option_series_id = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string();
    legs[1].size_1e8 = 0;
    let err = validate_multi_leg_composition(RFQ_UUID, &legs).unwrap_err();
    let message = err.to_string();
    assert!(!message.to_ascii_lowercase().contains("deadbeef"));
    assert!(!message.to_ascii_lowercase().contains("0x"));
}
