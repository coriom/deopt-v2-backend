use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::rfq::service::{
    accept_quote, cancel_rfq, create_rfq, list_quotes, submit_quote, CreateRfqInput,
    SubmitQuoteInput,
};
use deopt_v2_backend::rfq::{RfqConfig, RfqQuoteStatus, RfqStatus};
use deopt_v2_backend::types::{AccountId, Side};
use serde_json::json;
use std::time::Duration;
use tower::ServiceExt;

fn rfq_config() -> RfqConfig {
    RfqConfig {
        enabled: true,
        require_persistence: false,
        default_ttl_ms: 100,
        max_ttl_ms: 1_000,
        min_quote_ttl_ms: 1,
        max_quote_ttl_ms: 500,
        max_quotes_per_rfq: 50,
    }
}

fn state() -> AppState {
    AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config())
}

fn taker() -> AccountId {
    AccountId::new("0x0000000000000000000000000000000000000001")
}

fn mm() -> AccountId {
    AccountId::new("0x0000000000000000000000000000000000000002")
}

fn create_input(side: Side) -> CreateRfqInput {
    CreateRfqInput {
        taker: taker(),
        market_id: 1,
        side,
        size_1e8: 100_000_000,
        limit_price_1e8: Some(300_000_000_000),
        ttl_ms: Some(500),
    }
}

fn quote_input(rfq_id: uuid::Uuid) -> SubmitQuoteInput {
    SubmitQuoteInput {
        rfq_id,
        mm_account: mm(),
        session_id: None,
        client_quote_id: Some("quote-1".to_string()),
        price_1e8: 299_000_000_000,
        size_1e8: 100_000_000,
        quote_ttl_ms: 100,
    }
}

#[tokio::test]
async fn create_rfq_success() {
    let rfq = create_rfq(&state(), create_input(Side::Buy)).await.unwrap();

    assert_eq!(rfq.status, RfqStatus::Open);
    assert_eq!(rfq.taker, taker());
}

#[tokio::test]
async fn create_rfq_rejects_invalid_taker() {
    let mut input = create_input(Side::Buy);
    input.taker = AccountId::new("0xtaker");

    let error = create_rfq(&state(), input).await.unwrap_err();

    assert!(error.to_string().contains("malformed account address"));
}

#[tokio::test]
async fn create_rfq_rejects_invalid_side() {
    let response = router(state())
        .oneshot(json_post(
            "/rfqs",
            json!({
                "taker": taker().0,
                "market_id": 1,
                "side": "hold",
                "size_1e8": "100000000",
                "limit_price_1e8": "300000000000",
                "ttl_ms": 500
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn create_rfq_rejects_zero_size() {
    let mut input = create_input(Side::Buy);
    input.size_1e8 = 0;

    let error = create_rfq(&state(), input).await.unwrap_err();

    assert!(error.to_string().contains("zero size"));
}

#[tokio::test]
async fn create_rfq_caps_ttl() {
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(10_000);
    let rfq = create_rfq(&state(), input).await.unwrap();

    assert!(rfq.expires_at_ms - rfq.created_at_ms <= 1_000);
}

#[tokio::test]
async fn submit_quote_success() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();

    assert_eq!(quote.status, RfqQuoteStatus::Active);
    assert_eq!(quote.rfq_id, rfq.rfq_id);
}

#[tokio::test]
async fn submit_quote_rejects_expired_rfq() {
    let state = state();
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(1);
    let rfq = create_rfq(&state, input).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;

    let error = submit_quote(&state, quote_input(rfq.rfq_id))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("RFQ has expired"));
}

#[tokio::test]
async fn submit_quote_rejects_wrong_or_zero_price() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.price_1e8 = 0;

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("zero price"));
}

#[tokio::test]
async fn submit_quote_rejects_size_above_rfq_size() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.size_1e8 = rfq.size_1e8 + 1;

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("quote size exceeds RFQ size"));
}

#[tokio::test]
async fn submit_quote_caps_quote_ttl_to_rfq_expiry() {
    let state = state();
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(20);
    let rfq = create_rfq(&state, input).await.unwrap();
    let mut quote = quote_input(rfq.rfq_id);
    quote.quote_ttl_ms = 500;
    let quote = submit_quote(&state, quote).await.unwrap();

    assert!(quote.expires_at_ms <= rfq.expires_at_ms);
}

#[tokio::test]
async fn list_quotes_returns_all_quotes() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    let mut second = quote_input(rfq.rfq_id);
    second.client_quote_id = Some("quote-2".to_string());
    second.mm_account = AccountId::new("0x0000000000000000000000000000000000000003");
    submit_quote(&state, second).await.unwrap();

    assert_eq!(list_quotes(&state, rfq.rfq_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn accept_quote_success() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    let accepted = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();

    assert_eq!(accepted.status, RfqStatus::Accepted);
    assert!(accepted.onchain_intent_id.starts_with("0x"));
}

#[tokio::test]
async fn accept_quote_rejects_expired_rfq() {
    let state = state();
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(10);
    let rfq = create_rfq(&state, input).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(12)).await;

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("RFQ has expired"));
}

#[tokio::test]
async fn accept_quote_rejects_expired_quote() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut quote_input = quote_input(rfq.rfq_id);
    quote_input.quote_ttl_ms = 1;
    let quote = submit_quote(&state, quote_input).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("quote has expired"));
}

#[tokio::test]
async fn accept_quote_rejects_quote_from_different_rfq() {
    let state = state();
    let first = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let second = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(first.rfq_id))
        .await
        .unwrap();

    let error = accept_quote(&state, second.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("quote does not belong to RFQ"));
}

#[tokio::test]
async fn accept_quote_rejects_price_beyond_taker_limit() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.price_1e8 = 301_000_000_000;
    let quote = submit_quote(&state, input).await.unwrap();

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("beyond taker limit"));
}

#[tokio::test]
async fn accept_quote_creates_execution_intent_with_correct_sides_for_taker_buy() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();

    let intents = state.engine.lock().unwrap().execution_intents();
    assert_eq!(intents[0].buyer, taker());
    assert_eq!(intents[0].seller, mm());
    assert_eq!(intents[0].buyer_is_maker, Some(false));
}

#[tokio::test]
async fn accept_quote_creates_execution_intent_with_correct_sides_for_taker_sell() {
    let state = state();
    let mut input = create_input(Side::Sell);
    input.limit_price_1e8 = Some(299_000_000_000);
    let rfq = create_rfq(&state, input).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();

    let intents = state.engine.lock().unwrap().execution_intents();
    assert_eq!(intents[0].buyer, mm());
    assert_eq!(intents[0].seller, taker());
    assert_eq!(intents[0].buyer_is_maker, Some(true));
}

#[tokio::test]
async fn accept_quote_is_single_winner() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let first = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    let mut second = quote_input(rfq.rfq_id);
    second.client_quote_id = Some("quote-2".to_string());
    second.mm_account = AccountId::new("0x0000000000000000000000000000000000000003");
    let second = submit_quote(&state, second).await.unwrap();
    accept_quote(&state, rfq.rfq_id, first.quote_id)
        .await
        .unwrap();

    let error = accept_quote(&state, rfq.rfq_id, second.quote_id)
        .await
        .unwrap_err();
    let quotes = list_quotes(&state, rfq.rfq_id).await.unwrap();

    assert!(error.to_string().contains("RFQ is accepted"));
    assert!(
        quotes
            .iter()
            .any(|quote| quote.quote_id == second.quote_id
                && quote.status == RfqQuoteStatus::Rejected)
    );
}

#[tokio::test]
async fn cancel_rfq_prevents_quote_acceptance() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    cancel_rfq(&state, rfq.rfq_id).await.unwrap();

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("RFQ is cancelled"));
}

fn json_post(uri: &str, value: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}
