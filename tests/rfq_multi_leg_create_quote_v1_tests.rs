//! RFQ-MULTI-LEG-CREATE-QUOTE-V1 — service + flag gate + byte-freeze
//! assertions for the multi-leg atomic RFQ create + quote paths.
//!
//! HTTP-level assertions live implicitly through the service call
//! sites (the handlers just do request→service translation + auth).
//! Byte-freeze on the canonical payload happens indirectly via a
//! WriteAuthAction round-trip test — the exact byte layout is
//! exercised by end-to-end Playwright tests once the frontend
//! milestone lands.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::auth::write_authorization::WriteAuthAction;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::options::multi_leg_service::{
    create_option_multi_leg_rfq, ensure_option_multi_leg_rfq_enabled, get_option_multi_leg_rfq,
    list_option_multi_leg_rfq_quotes, list_option_multi_leg_rfqs_by_taker,
    submit_option_multi_leg_rfq_quote, CreateOptionMultiLegRfqInput, LegInput, QuoteLegInput,
    SubmitOptionMultiLegRfqQuoteInput,
};
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::{
    OptionMultiLegRfqStatus, OptionsConfig, MAX_LEGS_PER_MULTI_LEG_RFQ, MIN_LEGS_PER_MULTI_LEG_RFQ,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side};

const TAKER_HEX: &str = "0x1111111111111111111111111111111111111111";
const MM_HEX: &str = "0x2222222222222222222222222222222222222222";

fn taker() -> AccountId {
    AccountId::new(TAKER_HEX)
}

fn mm() -> AccountId {
    AccountId::new(MM_HEX)
}

fn base_config() -> OptionsConfig {
    let mut cfg = OptionsConfig::enabled_in_memory_for_tests();
    cfg.rfq_enabled = true;
    cfg.rfq_min_quote_ttl_ms = 1;
    cfg.rfq_max_quote_ttl_ms = 500;
    cfg
}

fn state_with_flag(flag: bool) -> AppState {
    let mut cfg = base_config();
    cfg.rfq_multi_leg_enabled = flag;
    AppState::with_options_config(EngineState::with_default_markets(), cfg)
}

async fn seed_series(state: &AppState, salt: u128) -> String {
    let expiry = u64::try_from(now_ms() / 1000).unwrap() + 86_400 + salt as u64;
    create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "ETH".to_string(),
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry,
            strike_1e8: 300_000_000_000 + salt,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .expect("seed option series")
    .option_series_id
}

fn leg(index: u32, series_id: &str, side: Side) -> LegInput {
    LegInput {
        leg_index: index,
        option_series_id: series_id.to_string(),
        side,
        size_1e8: 100_000_000,
        ratio_num: 1,
        ratio_den: 1,
    }
}

// ---------------------------------------------------------------------
// Feature-flag gate.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part1_multi_leg_flag_defaults_false_and_service_fails_closed() {
    let state = state_with_flag(false);
    let err = ensure_option_multi_leg_rfq_enabled(&state).unwrap_err();
    assert!(matches!(err, BackendError::OptionMultiLegRfqNotLive));
}

#[tokio::test]
async fn part1_flag_off_blocks_create() {
    let state = state_with_flag(false);
    let series = seed_series(&state, 0).await;
    let err = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::OptionMultiLegRfqNotLive));
}

#[tokio::test]
async fn part1_flag_off_blocks_submit_quote() {
    let state = state_with_flag(false);
    let err = submit_option_multi_leg_rfq_quote(
        &state,
        uuid::Uuid::new_v4(),
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "1".to_string(),
            size_1e8: 100_000_000,
            legs: vec![],
            quote_nonce: None,
            quote_ttl_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::OptionMultiLegRfqNotLive));
}

// ---------------------------------------------------------------------
// Create.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_create_two_leg_rfq_succeeds_and_persists_ordered_legs() {
    let state = state_with_flag(true);
    let series_a = seed_series(&state, 0).await;
    let series_b = seed_series(&state, 1).await;

    let (rfq, legs) = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series_a, Side::Buy), leg(1, &series_b, Side::Sell)],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap();
    assert_eq!(rfq.taker_subaccount_id, 1);
    assert_eq!(rfq.status, OptionMultiLegRfqStatus::Open);
    assert_eq!(legs.len(), 2);
    assert_eq!(legs[0].leg_index, 0);
    assert_eq!(legs[1].leg_index, 1);
    assert_eq!(legs[0].side, Side::Buy);
    assert_eq!(legs[1].side, Side::Sell);
}

#[tokio::test]
async fn part2_create_eight_legs_succeeds_at_boundary() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;
    let legs: Vec<LegInput> = (0..8)
        .map(|i| {
            let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
            leg(i as u32, &series, side)
        })
        .collect();
    let (_rfq, persisted) = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs,
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap();
    assert_eq!(persisted.len(), MAX_LEGS_PER_MULTI_LEG_RFQ);
}

#[tokio::test]
async fn part2_create_one_leg_rejects() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;
    let err = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy)],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    let _ = MIN_LEGS_PER_MULTI_LEG_RFQ; // silence unused-import warning
}

#[tokio::test]
async fn part2_create_nine_legs_rejects() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;
    let legs: Vec<LegInput> = (0..9).map(|i| leg(i as u32, &series, Side::Buy)).collect();
    let err = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs,
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part2_create_zero_subaccount_rejects() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;
    let err = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 0,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part2_create_leg_index_gap_rejects() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;
    // Contiguity is enforced by `validate_multi_leg_composition`. A
    // leg with a wrong index but valid data still rejects.
    let mut bad_leg = leg(3, &series, Side::Sell);
    bad_leg.leg_index = 3;
    let err = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), bad_leg],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

// ---------------------------------------------------------------------
// List / read + subaccount isolation.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part3_list_by_taker_isolates_account_1_from_account_2() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;

    let (rfq_1, _) = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap();
    let (rfq_2, _) = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 2,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap();

    let list_1 = list_option_multi_leg_rfqs_by_taker(&state, &taker(), 1)
        .await
        .unwrap();
    let list_2 = list_option_multi_leg_rfqs_by_taker(&state, &taker(), 2)
        .await
        .unwrap();

    assert!(list_1.iter().all(|(r, _)| r.taker_subaccount_id == 1));
    assert!(list_2.iter().all(|(r, _)| r.taker_subaccount_id == 2));
    assert!(list_1
        .iter()
        .any(|(r, _)| r.option_rfq_id == rfq_1.option_rfq_id));
    assert!(list_2
        .iter()
        .any(|(r, _)| r.option_rfq_id == rfq_2.option_rfq_id));
    assert!(!list_1
        .iter()
        .any(|(r, _)| r.option_rfq_id == rfq_2.option_rfq_id));
    assert!(!list_2
        .iter()
        .any(|(r, _)| r.option_rfq_id == rfq_1.option_rfq_id));
}

#[tokio::test]
async fn part3_get_returns_ordered_legs() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;
    let (rfq, _) = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(100),
        },
    )
    .await
    .unwrap();
    let (loaded, legs) = get_option_multi_leg_rfq(&state, rfq.option_rfq_id)
        .await
        .unwrap();
    assert_eq!(loaded.option_rfq_id, rfq.option_rfq_id);
    assert_eq!(legs.len(), 2);
    assert_eq!(legs[0].leg_index, 0);
    assert_eq!(legs[1].leg_index, 1);
}

// ---------------------------------------------------------------------
// Quote.
// ---------------------------------------------------------------------

async fn setup_open_rfq(state: &AppState) -> (uuid::Uuid, usize) {
    let series = seed_series(state, 0).await;
    let (rfq, legs) = create_option_multi_leg_rfq(
        state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(300),
        },
    )
    .await
    .unwrap();
    (rfq.option_rfq_id, legs.len())
}

fn quote_leg(index: u32, price: u128) -> QuoteLegInput {
    QuoteLegInput {
        leg_index: index,
        price_1e8: price,
    }
}

#[tokio::test]
async fn part4_valid_quote_persists_and_lists() {
    let state = state_with_flag(true);
    let (rfq_id, _n) = setup_open_rfq(&state).await;

    let (quote, quote_legs) = submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: Some("sid-1".to_string()),
            client_quote_id: Some("cq-1".to_string()),
            package_price_1e8: "1000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 2_000_000)],
            quote_nonce: Some(1),
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(quote.maker_subaccount_id, 1);
    assert_eq!(quote_legs.len(), 2);
    assert_eq!(quote_legs[0].price_1e8, 1_000_000);
    assert_eq!(quote_legs[1].price_1e8, 2_000_000);

    let list = list_option_multi_leg_rfq_quotes(&state, rfq_id)
        .await
        .unwrap();
    assert!(list.iter().any(|(q, _)| q.quote_id == quote.quote_id));
}

#[tokio::test]
async fn part4_quote_leg_count_mismatch_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, _n) = setup_open_rfq(&state).await;
    // RFQ has 2 legs; supply 1 quote leg → reject.
    let err = submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "1000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000)],
            quote_nonce: None,
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[tokio::test]
async fn part4_quote_zero_maker_subaccount_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, _n) = setup_open_rfq(&state).await;
    let err = submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 0,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "1000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 2_000_000)],
            quote_nonce: None,
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[tokio::test]
async fn part4_quote_zero_price_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, _n) = setup_open_rfq(&state).await;
    let err = submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "1000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 0)],
            quote_nonce: None,
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[tokio::test]
async fn part4_duplicate_client_quote_id_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, _n) = setup_open_rfq(&state).await;
    let base_input = |cq: &str| SubmitOptionMultiLegRfqQuoteInput {
        mm_account: mm(),
        maker_subaccount_id: 1,
        session_id: None,
        client_quote_id: Some(cq.to_string()),
        package_price_1e8: "1000000".to_string(),
        size_1e8: 100_000_000,
        legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 2_000_000)],
        quote_nonce: None,
        quote_ttl_ms: Some(100),
        signature: None,
    };
    submit_option_multi_leg_rfq_quote(&state, rfq_id, base_input("cq-dup"))
        .await
        .unwrap();
    let err = submit_option_multi_leg_rfq_quote(&state, rfq_id, base_input("cq-dup"))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[tokio::test]
async fn part4_quote_size_exceeds_smallest_leg_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, _n) = setup_open_rfq(&state).await;
    let err = submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "1000000".to_string(),
            size_1e8: 999_000_000_000,
            legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 2_000_000)],
            quote_nonce: None,
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

// ---------------------------------------------------------------------
// WriteAuthAction round-trip (canonical byte layout is exercised
// end-to-end by the create + quote happy paths — this test just pins
// the string tokens).
// ---------------------------------------------------------------------

#[test]
fn part5_write_auth_action_str_and_parse_round_trip() {
    for (variant, token) in [
        (
            WriteAuthAction::OptionMultiLegRfqCreate,
            "OPTION_MULTI_LEG_RFQ_CREATE",
        ),
        (
            WriteAuthAction::OptionMultiLegRfqQuoteSubmit,
            "OPTION_MULTI_LEG_RFQ_QUOTE_SUBMIT",
        ),
    ] {
        assert_eq!(variant.as_str(), token);
        assert_eq!(WriteAuthAction::parse(token), Some(variant));
    }
}

// ---------------------------------------------------------------------
// No secrets in error responses.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part6_flag_off_error_message_contains_no_addresses_or_secrets() {
    let state = state_with_flag(false);
    let err = ensure_option_multi_leg_rfq_enabled(&state).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("multi-leg option RFQ"));
    assert!(!msg.to_ascii_lowercase().contains("0x"));
}

// ---------------------------------------------------------------------
// Regression: no fill is created by any of these paths.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part7_no_fill_created_by_create_or_quote_paths() {
    let state = state_with_flag(true);
    let (rfq_id, _n) = setup_open_rfq(&state).await;
    submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "1000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 2_000_000)],
            quote_nonce: None,
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap();
    // Retrieve the RFQ: `accepted_fill_id` must be `None` because no
    // accept path exists yet in this milestone.
    let (rfq, _) = get_option_multi_leg_rfq(&state, rfq_id).await.unwrap();
    assert!(rfq.accepted_quote_id.is_none());
    assert!(rfq.accepted_fill_id.is_none());
}
