//! RFQ-MULTI-LEG-ATOMIC-ACCEPT-V1 — service-level assertions for the
//! atomic accept path.
//!
//! HTTP-level auth (v2 canonical byte-freeze, resolve_options_v2_subaccount,
//! require_write_auth_v2_aware) is exercised indirectly through the
//! auth crate's existing tests and the WriteAuthAction round-trip.
//! These tests focus on service-layer behaviour: integrity guards,
//! atomicity, single-winner, subaccount isolation, lifecycle.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::auth::write_authorization::WriteAuthAction;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::options::multi_leg_service::{
    accept_option_multi_leg_rfq_quote, create_option_multi_leg_rfq,
    ensure_option_multi_leg_rfq_enabled, get_option_multi_leg_rfq, get_option_multi_leg_rfq_fill,
    list_option_multi_leg_rfq_quotes, submit_option_multi_leg_rfq_quote,
    AcceptOptionMultiLegRfqQuoteInput, CreateOptionMultiLegRfqInput, LegInput, QuoteLegInput,
    SubmitOptionMultiLegRfqQuoteInput,
};
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::{
    OptionMultiLegRfqQuoteStatus, OptionMultiLegRfqStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use uuid::Uuid;

const TAKER_HEX: &str = "0x1111111111111111111111111111111111111111";
const MM_HEX: &str = "0x2222222222222222222222222222222222222222";
const OTHER_HEX: &str = "0x3333333333333333333333333333333333333333";

fn taker() -> AccountId {
    AccountId::new(TAKER_HEX)
}

fn mm() -> AccountId {
    AccountId::new(MM_HEX)
}

fn state_with_flag(flag: bool) -> AppState {
    let mut cfg = OptionsConfig::enabled_in_memory_for_tests();
    cfg.rfq_enabled = true;
    cfg.rfq_min_quote_ttl_ms = 1;
    cfg.rfq_max_quote_ttl_ms = 500;
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
    .unwrap()
    .option_series_id
}

fn leg(index: u32, series: &str, side: Side) -> LegInput {
    LegInput {
        leg_index: index,
        option_series_id: series.to_string(),
        side,
        size_1e8: 100_000_000,
        ratio_num: 1,
        ratio_den: 1,
    }
}

fn quote_leg(index: u32, price: u128) -> QuoteLegInput {
    QuoteLegInput {
        leg_index: index,
        price_1e8: price,
    }
}

async fn setup_open_rfq_and_quote(state: &AppState) -> (Uuid, Uuid, Vec<u128>, String) {
    let series = seed_series(state, 0).await;
    let (rfq, _) = create_option_multi_leg_rfq(
        state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(30_000),
        },
    )
    .await
    .unwrap();
    let (quote, quote_legs) = submit_option_multi_leg_rfq_quote(
        state,
        rfq.option_rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: Some("cq-accept".to_string()),
            package_price_1e8: "50000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 12_000_000_000), quote_leg(1, 11_500_000_000)],
            quote_nonce: Some(1),
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap();
    (
        rfq.option_rfq_id,
        quote.quote_id,
        quote_legs.iter().map(|q| q.price_1e8).collect(),
        quote.package_price_1e8,
    )
}

// ---------------------------------------------------------------------
// Flag gate.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part1_flag_off_blocks_accept() {
    let state = state_with_flag(false);
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: Uuid::new_v4(),
            quote_id: Uuid::new_v4(),
            expected_package_price_1e8: "0".to_string(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: vec![1, 2],
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::OptionMultiLegRfqNotLive));
    assert!(ensure_option_multi_leg_rfq_enabled(&state).is_err());
}

// ---------------------------------------------------------------------
// Integrity + happy path.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_valid_accept_flips_statuses_and_creates_one_fill_with_n_legs() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id, leg_prices, package) = setup_open_rfq_and_quote(&state).await;

    let outcome = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package,
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome.rfq.status, OptionMultiLegRfqStatus::Accepted);
    assert_eq!(outcome.rfq.accepted_quote_id, Some(quote_id));
    assert_eq!(outcome.rfq.accepted_fill_id, Some(outcome.fill.fill_id));
    assert_eq!(outcome.quote.status, OptionMultiLegRfqQuoteStatus::Accepted);
    assert_eq!(outcome.fill.taker_subaccount_id, 1);
    assert_eq!(outcome.fill.maker_subaccount_id, 1);
    assert_eq!(outcome.fill_legs.len(), 2);
    assert_eq!(outcome.fill_legs[0].leg_index, 0);
    assert_eq!(outcome.fill_legs[1].leg_index, 1);
    assert_eq!(outcome.fill_legs[0].price_1e8, 12_000_000_000);
    assert_eq!(outcome.fill_legs[1].price_1e8, 11_500_000_000);
    let (fill, legs) = get_option_multi_leg_rfq_fill(&state, outcome.fill.fill_id)
        .await
        .unwrap();
    assert_eq!(fill.fill_id, outcome.fill.fill_id);
    assert_eq!(legs.len(), 2);
}

#[tokio::test]
async fn part2_second_accept_rejects_with_no_longer_open() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id, leg_prices, package) = setup_open_rfq_and_quote(&state).await;

    let _first = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package.clone(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices.clone(),
        },
    )
    .await
    .unwrap();
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package,
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part3_package_price_mismatch_rejects_before_persistence() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id, leg_prices, _package) = setup_open_rfq_and_quote(&state).await;
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: "999999".to_string(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
    let (rfq, _) = get_option_multi_leg_rfq(&state, rfq_id).await.unwrap();
    assert_eq!(rfq.status, OptionMultiLegRfqStatus::Open);
    assert!(rfq.accepted_fill_id.is_none());
}

#[tokio::test]
async fn part3_expected_legs_count_mismatch_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id, leg_prices, package) = setup_open_rfq_and_quote(&state).await;
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package,
            expected_legs_count: 3,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[tokio::test]
async fn part3_expected_leg_price_mismatch_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id, mut leg_prices, package) = setup_open_rfq_and_quote(&state).await;
    // Wrong per-leg price commitment.
    leg_prices[1] = 999_999;
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package,
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

#[tokio::test]
async fn part3_wrong_taker_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id, leg_prices, package) = setup_open_rfq_and_quote(&state).await;
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: AccountId::new(OTHER_HEX),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package,
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part3_taker_subaccount_mismatch_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id, leg_prices, package) = setup_open_rfq_and_quote(&state).await;
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 2,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package,
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part3_zero_taker_subaccount_rejects() {
    let state = state_with_flag(true);
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 0,
            option_rfq_id: Uuid::new_v4(),
            quote_id: Uuid::new_v4(),
            expected_package_price_1e8: "0".to_string(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: vec![],
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part3_quote_for_wrong_rfq_rejects() {
    let state = state_with_flag(true);
    let (_rfq_id_a, quote_id_a, leg_prices_a, package_a) = setup_open_rfq_and_quote(&state).await;
    let (rfq_id_b, _quote_id_b, _leg_prices_b, _package_b) = setup_open_rfq_and_quote(&state).await;
    // Try to accept quote-A on RFQ-B.
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id_b,
            quote_id: quote_id_a,
            expected_package_price_1e8: package_a,
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices_a,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
}

// ---------------------------------------------------------------------
// Single-winner across competing quotes.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part4_losing_quote_flipped_to_rejected_after_accept() {
    let state = state_with_flag(true);
    let series = seed_series(&state, 0).await;
    let (rfq, _) = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(30_000),
        },
    )
    .await
    .unwrap();
    let (quote_winner, winner_legs) = submit_option_multi_leg_rfq_quote(
        &state,
        rfq.option_rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: Some("winner".to_string()),
            package_price_1e8: "10".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1000), quote_leg(1, 2000)],
            quote_nonce: Some(1),
            quote_ttl_ms: Some(200),
            signature: None,
        },
    )
    .await
    .unwrap();
    let (quote_loser, _) = submit_option_multi_leg_rfq_quote(
        &state,
        rfq.option_rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: Some("loser".to_string()),
            package_price_1e8: "20".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 5000), quote_leg(1, 6000)],
            quote_nonce: Some(2),
            quote_ttl_ms: Some(200),
            signature: None,
        },
    )
    .await
    .unwrap();

    let outcome = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq.option_rfq_id,
            quote_id: quote_winner.quote_id,
            expected_package_price_1e8: quote_winner.package_price_1e8.clone(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: winner_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.quote.status, OptionMultiLegRfqQuoteStatus::Accepted);

    // Loser should now be Rejected.
    let quotes = list_option_multi_leg_rfq_quotes(&state, rfq.option_rfq_id)
        .await
        .unwrap();
    let loser = quotes
        .iter()
        .find(|(q, _)| q.quote_id == quote_loser.quote_id)
        .expect("loser quote present");
    assert_eq!(loser.0.status, OptionMultiLegRfqQuoteStatus::Rejected);
}

// ---------------------------------------------------------------------
// Lifecycle event content — no secrets.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part5_accepted_lifecycle_payload_has_taker_and_maker_subaccounts_no_secrets() {
    let state = state_with_flag(true);
    // Subscribe to the lifecycle broadcast BEFORE the accept.
    let mut rx = state.lifecycle_events.subscribe();
    let (rfq_id, quote_id, leg_prices, package) = setup_open_rfq_and_quote(&state).await;
    accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: package,
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap();

    // Drain until we find the accepted variant. `AccountRfqs` receives
    // create + quote submitted first; skip past those.
    let mut saw_accepted = false;
    for _ in 0..16 {
        match rx.try_recv() {
            Ok(event) => {
                let json = serde_json::to_string(&event.payload).unwrap();
                if json.contains("option_multi_leg_rfq_accepted") {
                    saw_accepted = true;
                    // Subaccount metadata carried; no secrets.
                    assert!(json.contains("\"taker_subaccount_id\":1"));
                    assert!(json.contains("\"maker_subaccount_id\":1"));
                    assert!(!json.contains("signature"));
                    assert!(!json.contains("nonce"));
                    assert!(!json.contains("authorization"));
                }
            }
            Err(_) => break,
        }
    }
    assert!(saw_accepted, "accepted lifecycle payload must be broadcast");
}

// ---------------------------------------------------------------------
// WriteAuthAction round-trip.
// ---------------------------------------------------------------------

#[test]
fn part6_accept_action_str_and_parse_round_trip() {
    assert_eq!(
        WriteAuthAction::OptionMultiLegRfqAccept.as_str(),
        "OPTION_MULTI_LEG_RFQ_ACCEPT"
    );
    assert_eq!(
        WriteAuthAction::parse("OPTION_MULTI_LEG_RFQ_ACCEPT"),
        Some(WriteAuthAction::OptionMultiLegRfqAccept)
    );
    // Cross-action isolation.
    assert_ne!(
        WriteAuthAction::OptionMultiLegRfqAccept.as_str(),
        WriteAuthAction::OptionMultiLegRfqCreate.as_str()
    );
    assert_ne!(
        WriteAuthAction::OptionMultiLegRfqAccept.as_str(),
        WriteAuthAction::OptionMultiLegRfqQuoteSubmit.as_str()
    );
}
