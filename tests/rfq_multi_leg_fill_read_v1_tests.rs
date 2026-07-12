//! RFQ-MULTI-LEG-FILL-READ-V1 — HTTP-level assertions for the
//! read-only per-fill route.
//!
//! Covered:
//!
//! * flag gate returns 503 when `OPTION_RFQ_MULTI_LEG_ENABLED=false`;
//! * taker can read own fill on the correct subaccount;
//! * maker can read own fill on the correct subaccount;
//! * unrelated account is refused with 404 (privacy-preserving);
//! * taker wrong-subaccount is refused with 404;
//! * maker wrong-subaccount is refused with 404;
//! * Account 0 (subaccount_id = 0) is refused with 404;
//! * missing fill_id is refused with 404;
//! * rfq_id / fill_id mismatch is refused with 404;
//! * response includes parent fill + N ordered fill legs;
//! * response includes taker_subaccount_id + maker_subaccount_id +
//!   package_price_1e8;
//! * fee summary reports `available=true` with per-side amounts when
//!   fees are enabled;
//! * fee summary reports `available=false` when fees are disabled;
//! * response body contains no `signature` / `nonce` /
//!   `authorization` substrings.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::fees::FeesConfig;
use deopt_v2_backend::options::multi_leg_service::{
    accept_option_multi_leg_rfq_quote, create_option_multi_leg_rfq,
    submit_option_multi_leg_rfq_quote, AcceptOptionMultiLegRfqQuoteInput,
    CreateOptionMultiLegRfqInput, LegInput, QuoteLegInput, SubmitOptionMultiLegRfqQuoteInput,
};
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use tower::ServiceExt;
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

fn options_config(multi_leg_flag: bool) -> OptionsConfig {
    let mut cfg = OptionsConfig::enabled_in_memory_for_tests();
    cfg.rfq_enabled = true;
    cfg.rfq_min_quote_ttl_ms = 1;
    cfg.rfq_max_quote_ttl_ms = 500;
    cfg.rfq_multi_leg_enabled = multi_leg_flag;
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

fn state_with_flag_off() -> AppState {
    AppState::with_options_and_fees_config(
        EngineState::with_default_markets(),
        options_config(false),
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

/// Create an RFQ, submit a quote, accept it. Returns
/// `(rfq_id, fill_id)` — everything else the test needs is available
/// via the read route.
async fn create_accepted_fill(state: &AppState) -> (Uuid, Uuid) {
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
            client_quote_id: Some("cq-fillread".to_string()),
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
    let outcome = accept_option_multi_leg_rfq_quote(
        state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq.option_rfq_id,
            quote_id: quote.quote_id,
            expected_package_price_1e8: quote.package_price_1e8,
            expected_legs_count: 2,
            expected_leg_prices_1e8: quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();
    (rfq.option_rfq_id, outcome.fill.fill_id)
}

async fn get_json(state: AppState, path: &str) -> (StatusCode, String) {
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

// ---------------------------------------------------------------------
// Part 1 — Flag gate.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part1_flag_off_returns_503() {
    let state = state_with_flag_off();
    // Even a well-formed URL must 503 when the flag is off.
    let rfq_id = Uuid::new_v4();
    let fill_id = Uuid::new_v4();
    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, fill_id, TAKER_HEX
    );
    let (status, _) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------
// Part 2 — Happy path taker read.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_taker_can_read_own_fill_and_response_carries_no_secrets() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;

    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, fill_id, TAKER_HEX
    );
    let (status, body) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::OK);

    // Response shape — the taker sees taker + maker subaccounts, the
    // fill's ordered legs, and the fee summary.
    assert!(body.contains("\"fill_id\""));
    assert!(body.contains("\"option_rfq_id\""));
    assert!(body.contains("\"taker_subaccount_id\":1"));
    assert!(body.contains("\"maker_subaccount_id\":1"));
    assert!(body.contains("\"package_price_1e8\":\"50000000\""));
    assert!(body.contains("\"legs\""));
    assert!(body.contains("\"leg_index\":0"));
    assert!(body.contains("\"leg_index\":1"));
    assert!(body.contains("\"fees\""));
    assert!(body.contains("\"source_type\":\"option_multi_leg_rfq_fill\""));

    // No secrets in the response body.
    assert!(!body.contains("\"signature\""));
    assert!(!body.contains("\"nonce\""));
    assert!(!body.contains("\"authorization\""));
    assert!(!body.contains("\"private_key\""));
}

// ---------------------------------------------------------------------
// Part 2b — Maker can read own fill.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_maker_can_read_own_fill() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;

    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, fill_id, MM_HEX
    );
    let (status, body) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"fill_id\""));
}

// ---------------------------------------------------------------------
// Part 3 — Access control refusals: privacy-preserving 404.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part3_unrelated_account_returns_404() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;

    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, fill_id, OTHER_HEX
    );
    let (status, _) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn part3_taker_wrong_subaccount_returns_404() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;
    // Fill was created under subaccount 1 for both sides; ask as
    // subaccount 2 → 404 (do not reveal existence).
    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=2",
        rfq_id, fill_id, TAKER_HEX
    );
    let (status, _) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn part3_maker_wrong_subaccount_returns_404() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;
    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=2",
        rfq_id, fill_id, MM_HEX
    );
    let (status, _) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn part3_account_zero_subaccount_returns_404() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;
    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=0",
        rfq_id, fill_id, TAKER_HEX
    );
    let (status, _) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------
// Part 4 — Missing / mismatched ids.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part4_unknown_fill_id_returns_404() {
    let state = state_with_fees_enabled();
    let (rfq_id, _) = create_accepted_fill(&state).await;
    let bogus_fill = Uuid::new_v4();
    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, bogus_fill, TAKER_HEX
    );
    let (status, _) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn part4_rfq_id_fill_id_mismatch_returns_404() {
    let state = state_with_fees_enabled();
    let (_rfq_id, fill_id) = create_accepted_fill(&state).await;
    // Real fill_id but wrong rfq_id (unrelated random UUID). The
    // handler must refuse before the seat check, so callers cannot
    // fish for fill_id existence by trying random rfq_ids.
    let wrong_rfq = Uuid::new_v4();
    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        wrong_rfq, fill_id, TAKER_HEX
    );
    let (status, _) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------
// Part 5 — Fee summary behaviour.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part5_fees_enabled_returns_available_true_with_nonzero_amounts() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;

    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, fill_id, TAKER_HEX
    );
    let (status, body) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"available\":true"));
    assert!(body.contains("\"events_count\":2"));
    // Both taker + maker paid non-zero (see Part 8 of the fees test
    // for the amount derivation). We just assert the "0" placeholder
    // is NOT emitted for either side.
    assert!(!body.contains("\"taker_fee_1e8\":\"0\""));
    assert!(!body.contains("\"maker_fee_1e8\":\"0\""));
}

#[tokio::test]
async fn part5_fees_disabled_returns_available_false_without_fabricating_amounts() {
    let state = state_with_fees_disabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;

    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, fill_id, TAKER_HEX
    );
    let (status, body) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"available\":false"));
    assert!(body.contains("\"events_count\":0"));
    assert!(body.contains("\"taker_fee_1e8\":\"0\""));
    assert!(body.contains("\"maker_fee_1e8\":\"0\""));
    assert!(body.contains("\"protocol_amount_1e8\":\"0\""));
}

// ---------------------------------------------------------------------
// Part 6 — Legs are returned in stable ascending index order.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part6_legs_are_ordered_by_leg_index_ascending() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;

    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?account={}&subaccount_id=1",
        rfq_id, fill_id, TAKER_HEX
    );
    let (status, body) = get_json(state, &path).await;
    assert_eq!(status, StatusCode::OK);
    // Assert leg 0 appears before leg 1 in the serialized body.
    let idx0 = body.find("\"leg_index\":0").expect("leg 0 present");
    let idx1 = body.find("\"leg_index\":1").expect("leg 1 present");
    assert!(idx0 < idx1, "leg_index must be emitted in ascending order");
}

// ---------------------------------------------------------------------
// Part 7 — Missing `account` query parameter → 400 (axum returns 400
// on a Query parse error). This is different from the privacy 404 but
// documents that the handler refuses under-specified requests early.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part7_missing_account_query_returns_client_error() {
    let state = state_with_fees_enabled();
    let (rfq_id, fill_id) = create_accepted_fill(&state).await;
    let path = format!(
        "/options/multi-leg-rfqs/{}/fills/{}?subaccount_id=1",
        rfq_id, fill_id
    );
    let (status, _) = get_json(state, &path).await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "missing account query returns 4xx (got {})",
        status
    );
}
