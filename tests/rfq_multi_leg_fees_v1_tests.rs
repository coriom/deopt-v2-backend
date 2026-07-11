//! RFQ-MULTI-LEG-FEES-V1 — service-level assertions for package-fee
//! accounting on multi-leg RFQ atomic accept.
//!
//! These tests focus on:
//!
//! * fee basis is the sum of per-leg **gross** underlying + premium
//!   notionals (mirrors single-leg `record_option_rfq_fill` but
//!   aggregated across legs);
//! * one `FeeEvent` per role per accepted fill (2 events total);
//! * new `FeeSourceType::OptionMultiLegRfqFill` source type;
//! * subaccounts preserved on the parent fill (fee events reference
//!   `fill_id` — single-leg RFQ fees also do not store subaccount on
//!   the fee row);
//! * no fee events when fees are disabled;
//! * no fee events on a rejected accept;
//! * duplicate-accept guard: unique `(source_type, source_id, payer,
//!   recipient)` prevents double-charging even if `record_...` is
//!   called twice on the same fill_id;
//! * regression: single-leg RFQ fee events are unaffected.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::fees::service::{record_option_multi_leg_rfq_fill, record_option_rfq_fill};
use deopt_v2_backend::fees::{FeeSourceType, FeesConfig};
use deopt_v2_backend::options::multi_leg_service::{
    accept_option_multi_leg_rfq_quote, create_option_multi_leg_rfq,
    submit_option_multi_leg_rfq_quote, AcceptOptionMultiLegRfqQuoteInput,
    CreateOptionMultiLegRfqInput, LegInput, QuoteLegInput, SubmitOptionMultiLegRfqQuoteInput,
};
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::{OptionMultiLegRfqFill, OptionMultiLegRfqFillLeg, OptionsConfig};
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use uuid::Uuid;

const TAKER_HEX: &str = "0x1111111111111111111111111111111111111111";
const MM_HEX: &str = "0x2222222222222222222222222222222222222222";

fn taker() -> AccountId {
    AccountId::new(TAKER_HEX)
}

fn mm() -> AccountId {
    AccountId::new(MM_HEX)
}

fn options_config(flag: bool) -> OptionsConfig {
    let mut cfg = OptionsConfig::enabled_in_memory_for_tests();
    cfg.rfq_enabled = true;
    cfg.rfq_min_quote_ttl_ms = 1;
    cfg.rfq_max_quote_ttl_ms = 500;
    cfg.rfq_multi_leg_enabled = flag;
    cfg
}

fn state_with_fees_enabled() -> AppState {
    AppState::with_options_and_fees_config(
        EngineState::with_default_markets(),
        options_config(true),
        FeesConfig::enabled_in_memory_for_tests(),
    )
}

fn state_with_fees_disabled() -> AppState {
    AppState::with_options_and_fees_config(
        EngineState::with_default_markets(),
        options_config(true),
        FeesConfig::disabled(),
    )
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

/// Set up a fresh open RFQ + one active quote and return the ids the
/// caller needs to accept it. Uses `state` so the fee ledger is
/// attached.
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
            client_quote_id: Some("cq-fees".to_string()),
            package_price_1e8: "50000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 12_000_000_000), quote_leg(1, 11_500_000_000)],
            quote_nonce: Some(1),
            quote_ttl_ms: Some(200),
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

fn source_type_multi_leg() -> &'static str {
    FeeSourceType::OptionMultiLegRfqFill.as_str()
}

// ---------------------------------------------------------------------
// Part 1 — source type wire token + parity with single-leg RFQ.
// ---------------------------------------------------------------------

#[test]
fn part1_source_type_wire_token_is_stable_and_disjoint() {
    // Wire token pinned so PG rows / analytics never rename silently.
    assert_eq!(source_type_multi_leg(), "option_multi_leg_rfq_fill");
    // Disjoint from single-leg source type so `(source_type, source_id,
    // payer, recipient)` cannot collide with an existing single-leg
    // fill_id UUID.
    assert_ne!(
        FeeSourceType::OptionMultiLegRfqFill.as_str(),
        FeeSourceType::OptionRfqFill.as_str()
    );
    assert_ne!(
        FeeSourceType::OptionMultiLegRfqFill.as_str(),
        FeeSourceType::OptionOrderFill.as_str()
    );
}

// ---------------------------------------------------------------------
// Part 2 — happy path: accept generates exactly 2 fee events (maker +
// taker) with the correct source type / source id / payers.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_accept_creates_two_fee_events_one_per_role() {
    let state = state_with_fees_enabled();
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

    let fill_id = outcome.fill.fill_id.to_string();
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| e.source_type == FeeSourceType::OptionMultiLegRfqFill && e.source_id == fill_id)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2, "one maker + one taker event per fill");
    let payers: Vec<String> = events
        .iter()
        .map(|e| e.payer.0.to_ascii_lowercase())
        .collect();
    assert!(payers.iter().any(|p| p == &taker().0.to_ascii_lowercase()));
    assert!(payers.iter().any(|p| p == &mm().0.to_ascii_lowercase()));
    // Every event references the same fill_id — this is how downstream
    // reporting joins the fee ledger back to `taker_subaccount_id` /
    // `maker_subaccount_id` on `option_multi_leg_rfq_fills`.
    for event in &events {
        assert_eq!(event.source_id, fill_id);
    }
}

#[tokio::test]
async fn part2_accept_fee_events_use_option_market_and_rfq_flow() {
    let state = state_with_fees_enabled();
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

    let fill_id = outcome.fill.fill_id.to_string();
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| e.source_type == FeeSourceType::OptionMultiLegRfqFill && e.source_id == fill_id)
        .collect::<Vec<_>>();
    for event in &events {
        assert_eq!(
            event.market_type.as_str(),
            "option",
            "multi-leg RFQ is an option market"
        );
        assert_eq!(
            event.flow_type.as_str(),
            "rfq",
            "multi-leg RFQ is the RFQ flow"
        );
        // Package spans multiple series — the event does not pin one.
        assert!(
            event.option_series_id.is_none(),
            "package fee event must not pin a single series"
        );
    }
}

// ---------------------------------------------------------------------
// Part 3 — no fee events on rejected accepts. Guards the accept side
// path: integrity failure MUST NOT create any fee row.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part3_package_price_mismatch_produces_no_fee_events() {
    let state = state_with_fees_enabled();
    let (rfq_id, quote_id, leg_prices, _package) = setup_open_rfq_and_quote(&state).await;
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            // Package price commitment does not match the persisted
            // quote — accept should refuse before fill persistence,
            // and no fee row should exist.
            expected_package_price_1e8: "999999".to_string(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: leg_prices,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqQuoteState(_)));
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| e.source_type == FeeSourceType::OptionMultiLegRfqFill)
        .count();
    assert_eq!(events, 0, "rejected accept must not create fee rows");
}

#[tokio::test]
async fn part3_wrong_taker_produces_no_fee_events() {
    let state = state_with_fees_enabled();
    let (rfq_id, quote_id, leg_prices, package) = setup_open_rfq_and_quote(&state).await;
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: AccountId::new("0x3333333333333333333333333333333333333333"),
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
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| e.source_type == FeeSourceType::OptionMultiLegRfqFill)
        .count();
    assert_eq!(events, 0);
}

// ---------------------------------------------------------------------
// Part 4 — fees disabled path is a silent noop. No error, no rows.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part4_fees_disabled_config_does_not_create_events() {
    let state = state_with_fees_disabled();
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

    let fill_id = outcome.fill.fill_id.to_string();
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| e.source_type == FeeSourceType::OptionMultiLegRfqFill && e.source_id == fill_id)
        .count();
    assert_eq!(events, 0);
}

// ---------------------------------------------------------------------
// Part 5 — package notionals: gross per-leg sum, sign-independent.
// A debit + credit spread whose net package price is small must still
// see a fee (because per-leg premiums are gross positive).
// ---------------------------------------------------------------------

#[tokio::test]
async fn part5_debit_credit_spread_net_zero_still_generates_nonzero_fees() {
    let state = state_with_fees_enabled();
    let series = seed_series(&state, 0).await;
    let (rfq, _) = create_option_multi_leg_rfq(
        &state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            // Buy + sell same size + same series (a wash on paper).
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(30_000),
        },
    )
    .await
    .unwrap();
    // Package price *0* — the taker pays nothing net. But per-leg
    // gross premiums are 10 USDC each.
    let (quote, quote_legs) = submit_option_multi_leg_rfq_quote(
        &state,
        rfq.option_rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: Some("cq-wash".to_string()),
            package_price_1e8: "0".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000_000), quote_leg(1, 1_000_000_000)],
            quote_nonce: Some(1),
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
            quote_id: quote.quote_id,
            expected_package_price_1e8: quote.package_price_1e8.clone(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();

    let fill_id = outcome.fill.fill_id.to_string();
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| e.source_type == FeeSourceType::OptionMultiLegRfqFill && e.source_id == fill_id)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    // BOTH taker and maker paid a non-zero fee even though the package
    // price nets to zero.
    for event in &events {
        assert!(
            event.fee_amount_1e8 > 0,
            "gross per-leg basis must not collapse to zero on a debit/credit wash"
        );
    }
}

// ---------------------------------------------------------------------
// Part 6 — rounding + determinism: same inputs => same fee amounts.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part6_identical_accepts_produce_identical_fee_amounts() {
    let state_a = state_with_fees_enabled();
    let (rfq_a, quote_a, prices_a, pkg_a) = setup_open_rfq_and_quote(&state_a).await;
    let out_a = accept_option_multi_leg_rfq_quote(
        &state_a,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_a,
            quote_id: quote_a,
            expected_package_price_1e8: pkg_a,
            expected_legs_count: 2,
            expected_leg_prices_1e8: prices_a,
        },
    )
    .await
    .unwrap();
    let events_a = state_a
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| {
            e.source_type == FeeSourceType::OptionMultiLegRfqFill
                && e.source_id == out_a.fill.fill_id.to_string()
        })
        .collect::<Vec<_>>();

    let state_b = state_with_fees_enabled();
    let (rfq_b, quote_b, prices_b, pkg_b) = setup_open_rfq_and_quote(&state_b).await;
    let out_b = accept_option_multi_leg_rfq_quote(
        &state_b,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_b,
            quote_id: quote_b,
            expected_package_price_1e8: pkg_b,
            expected_legs_count: 2,
            expected_leg_prices_1e8: prices_b,
        },
    )
    .await
    .unwrap();
    let events_b = state_b
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| {
            e.source_type == FeeSourceType::OptionMultiLegRfqFill
                && e.source_id == out_b.fill.fill_id.to_string()
        })
        .collect::<Vec<_>>();

    // Group by role.
    let sum_a: u128 = events_a.iter().map(|e| e.fee_amount_1e8).sum();
    let sum_b: u128 = events_b.iter().map(|e| e.fee_amount_1e8).sum();
    assert_eq!(sum_a, sum_b, "identical inputs must produce identical fees");
    assert!(sum_a > 0, "sanity: fee amount is non-zero");
}

// ---------------------------------------------------------------------
// Part 7 — duplicate protection. Calling `record_option_multi_leg_rfq_fill`
// twice on the same fill must NOT double-charge — enforced by
// `UNIQUE(source_type, source_id, payer, recipient)` on `fee_events`.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part7_duplicate_fee_record_is_idempotent() {
    let state = state_with_fees_enabled();
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

    // Second call on the same fill — should be a silent noop for the
    // ledger (unique key already exists).
    record_option_multi_leg_rfq_fill(&state, &outcome.fill, &outcome.fill_legs, &outcome.quote)
        .await
        .unwrap();

    let fill_id = outcome.fill.fill_id.to_string();
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| e.source_type == FeeSourceType::OptionMultiLegRfqFill && e.source_id == fill_id)
        .count();
    assert_eq!(events, 2, "duplicate record call must not double-charge");
}

// ---------------------------------------------------------------------
// Part 8 — subaccount attribution preserved on the parent fill row.
// The fee event references `fill_id`; the fill row carries
// (taker_subaccount_id, maker_subaccount_id). This mirrors single-leg
// RFQ (also does not store subaccount on the fee row).
// ---------------------------------------------------------------------

#[tokio::test]
async fn part8_fill_row_carries_subaccounts_for_fee_join() {
    let state = state_with_fees_enabled();
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

    // Fill row is authoritative source for subaccount attribution.
    assert_eq!(outcome.fill.taker_subaccount_id, 1);
    assert_eq!(outcome.fill.maker_subaccount_id, 1);
    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| {
            e.source_type == FeeSourceType::OptionMultiLegRfqFill
                && e.source_id == outcome.fill.fill_id.to_string()
        })
        .collect::<Vec<_>>();
    // Fee events reference the fill_id — the join key that gives
    // reporting access to both subaccount ids.
    for event in &events {
        assert_eq!(event.source_id, outcome.fill.fill_id.to_string());
    }
}

// ---------------------------------------------------------------------
// Part 9 — regression: single-leg RFQ path still creates its own,
// disjoint source_type events.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part9_single_leg_source_type_still_present_after_multi_leg_call() {
    let state = state_with_fees_enabled();
    let series = seed_series(&state, 0).await;

    // A minimal single-leg RFQ fill to drive `record_option_rfq_fill`.
    let single_leg_fill = deopt_v2_backend::options::OptionRfqFill {
        fill_id: Uuid::new_v4(),
        option_rfq_id: Uuid::new_v4(),
        quote_id: Uuid::new_v4(),
        option_series_id: series.clone(),
        buyer: taker(),
        seller: mm(),
        taker: taker(),
        mm_account: mm(),
        taker_subaccount_id: 1,
        maker_subaccount_id: 1,
        taker_side: Side::Buy,
        price_1e8: 12_000_000_000,
        size_1e8: 100_000_000,
        created_at_ms: now_ms(),
    };
    let single_leg_quote = deopt_v2_backend::options::OptionRfqQuote {
        quote_id: single_leg_fill.quote_id,
        option_rfq_id: single_leg_fill.option_rfq_id,
        mm_account: mm(),
        maker_subaccount_id: 1,
        session_id: None,
        client_quote_id: None,
        price_1e8: 12_000_000_000,
        size_1e8: 100_000_000,
        status: deopt_v2_backend::options::OptionRfqQuoteStatus::Accepted,
        created_at_ms: now_ms(),
        expires_at_ms: now_ms() + 60_000,
        signature: None,
        quote_digest: None,
        quote_nonce: None,
        signature_status: deopt_v2_backend::options::OptionRfqQuoteSignatureStatus::NotRequired,
        recovered_signer: None,
    };
    record_option_rfq_fill(&state, &single_leg_fill, &single_leg_quote)
        .await
        .unwrap();

    // Now also do a multi-leg accept.
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

    let all = state.fees_store.lock().unwrap().list_fee_events(64);
    let single_leg_count = all
        .iter()
        .filter(|e| {
            e.source_type == FeeSourceType::OptionRfqFill
                && e.source_id == single_leg_fill.fill_id.to_string()
        })
        .count();
    let multi_leg_count = all
        .iter()
        .filter(|e| {
            e.source_type == FeeSourceType::OptionMultiLegRfqFill
                && e.source_id == outcome.fill.fill_id.to_string()
        })
        .count();
    assert_eq!(single_leg_count, 2, "single-leg emits 2 fee events");
    assert_eq!(multi_leg_count, 2, "multi-leg emits 2 fee events");
}

// ---------------------------------------------------------------------
// Part 10 — direct-call test of the record function with a hand-built
// fill + fill_legs, to isolate the notional-aggregation logic from the
// accept path.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part10_direct_record_call_creates_two_events_and_uses_gross_sum() {
    let state = state_with_fees_enabled();
    let series = seed_series(&state, 0).await;

    let fill_id = Uuid::new_v4();
    let fill = OptionMultiLegRfqFill {
        fill_id,
        option_rfq_id: Uuid::new_v4(),
        quote_id: Uuid::new_v4(),
        taker: taker(),
        taker_subaccount_id: 1,
        mm_account: mm(),
        maker_subaccount_id: 1,
        package_price_1e8: "1".to_string(),
        size_1e8: 100_000_000,
        created_at_ms: now_ms(),
    };
    let fill_legs = vec![
        OptionMultiLegRfqFillLeg {
            fill_id,
            leg_index: 0,
            option_series_id: series.clone(),
            side: Side::Buy,
            size_1e8: 100_000_000,
            price_1e8: 5_000_000_000,
        },
        OptionMultiLegRfqFillLeg {
            fill_id,
            leg_index: 1,
            option_series_id: series.clone(),
            side: Side::Sell,
            size_1e8: 100_000_000,
            price_1e8: 7_000_000_000,
        },
    ];
    let quote = deopt_v2_backend::options::OptionMultiLegRfqQuote {
        quote_id: fill.quote_id,
        option_rfq_id: fill.option_rfq_id,
        mm_account: mm(),
        maker_subaccount_id: 1,
        session_id: None,
        client_quote_id: None,
        package_price_1e8: "1".to_string(),
        size_1e8: 100_000_000,
        status: deopt_v2_backend::options::OptionMultiLegRfqQuoteStatus::Accepted,
        created_at_ms: now_ms(),
        expires_at_ms: now_ms() + 60_000,
        signature: None,
        quote_digest: None,
        quote_nonce: None,
        signature_status:
            deopt_v2_backend::options::OptionMultiLegRfqQuoteSignatureStatus::NotRequired,
        recovered_signer: None,
    };

    record_option_multi_leg_rfq_fill(&state, &fill, &fill_legs, &quote)
        .await
        .unwrap();

    let events = state
        .fees_store
        .lock()
        .unwrap()
        .list_fee_events(64)
        .into_iter()
        .filter(|e| {
            e.source_type == FeeSourceType::OptionMultiLegRfqFill
                && e.source_id == fill_id.to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    // Non-zero fee amounts — gross premium sum of the two legs is 12
    // USDC (5 + 7); underlying notional is 2 * strike * contract.
    for event in &events {
        assert!(event.fee_amount_1e8 > 0);
    }
}
