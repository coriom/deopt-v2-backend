use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{
    create_option_series, disable_option_series, get_option_series, list_option_series,
    CreateOptionSeriesInput,
};
use deopt_v2_backend::options::{
    option_series_id, OptionSeriesFilter, OptionSeriesIdInput, OptionSeriesStatus, OptionsConfig,
};
use deopt_v2_backend::types::now_ms;
use serde_json::json;
use tower::ServiceExt;

fn state() -> AppState {
    AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
    )
}

fn future_expiry() -> u64 {
    u64::try_from(now_ms() / 1000).unwrap() + 86_400
}

fn create_input() -> CreateOptionSeriesInput {
    CreateOptionSeriesInput {
        underlying: "ETH".to_string(),
        base_asset: "ETH".to_string(),
        quote_asset: "USDC".to_string(),
        settlement_asset: "USDC".to_string(),
        expiry: future_expiry(),
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: Some(100_000_000),
        onchain_product_id: None,
        onchain_series_id: None,
    }
}

#[tokio::test]
async fn option_series_creation_success() {
    let state = state();
    let series = create_option_series(&state, create_input()).await.unwrap();

    assert_eq!(series.underlying, "ETH");
    assert_eq!(series.strike_1e8, 300_000_000_000);
    assert_eq!(series.contract_size_1e8, 100_000_000);
    assert_eq!(series.status, OptionSeriesStatus::Active);
    assert!(series.option_series_id.starts_with("0x"));
}

#[tokio::test]
async fn option_series_rejects_zero_strike() {
    let state = state();
    let mut input = create_input();
    input.strike_1e8 = 0;

    let error = create_option_series(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("strike_1e8"));
}

#[tokio::test]
async fn option_series_rejects_zero_contract_size() {
    let state = state();
    let mut input = create_input();
    input.contract_size_1e8 = Some(0);

    let error = create_option_series(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("contract_size_1e8"));
}

#[tokio::test]
async fn option_series_rejects_expired_expiry() {
    let state = state();
    let mut input = create_input();
    input.expiry = 1;

    let error = create_option_series(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("expiry must be in the future"));
}

#[test]
fn option_series_id_is_deterministic_and_case_normalized() {
    let left = option_series_id(OptionSeriesIdInput {
        underlying: "ETH",
        base_asset: "ETH",
        quote_asset: "USDC",
        settlement_asset: "USDC",
        expiry: 4_102_444_800,
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: 100_000_000,
    });
    let right = option_series_id(OptionSeriesIdInput {
        underlying: "eth",
        base_asset: "eth",
        quote_asset: "usdc",
        settlement_asset: "usdc",
        expiry: 4_102_444_800,
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: 100_000_000,
    });

    assert_eq!(left, right);
    assert_ne!(
        left,
        option_series_id(OptionSeriesIdInput {
            underlying: "ETH",
            base_asset: "ETH",
            quote_asset: "USDC",
            settlement_asset: "USDC",
            expiry: 4_102_444_800,
            strike_1e8: 300_000_000_000,
            is_call: false,
            contract_size_1e8: 100_000_000,
        })
    );
}

#[tokio::test]
async fn duplicate_option_series_returns_existing_series() {
    let state = state();
    let first = create_option_series(&state, create_input()).await.unwrap();
    let second = create_option_series(&state, create_input()).await.unwrap();

    assert_eq!(first.option_series_id, second.option_series_id);
    assert_eq!(first.created_at_ms, second.created_at_ms);
}

#[tokio::test]
async fn list_option_series_by_status_and_underlying() {
    let state = state();
    create_option_series(&state, create_input()).await.unwrap();

    let active = list_option_series(
        &state,
        OptionSeriesFilter {
            status: Some(OptionSeriesStatus::Active),
            ..OptionSeriesFilter::default()
        },
    )
    .await
    .unwrap();
    let eth = list_option_series(
        &state,
        OptionSeriesFilter {
            underlying: Some("eth".to_string()),
            ..OptionSeriesFilter::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(active.len(), 1);
    assert_eq!(eth.len(), 1);
}

#[tokio::test]
async fn get_and_disable_option_series() {
    let state = state();
    let created = create_option_series(&state, create_input()).await.unwrap();
    let fetched = get_option_series(&state, &created.option_series_id)
        .await
        .unwrap();
    let disabled = disable_option_series(&state, &created.option_series_id)
        .await
        .unwrap();

    assert_eq!(fetched.option_series_id, created.option_series_id);
    assert_eq!(disabled.status, OptionSeriesStatus::Disabled);
}

#[tokio::test]
async fn http_option_series_endpoints_and_empty_orderbook() {
    let state = state();
    let app = router(state);
    let response = app
        .clone()
        .oneshot(json_post(
            "/options/series",
            json!({
                "underlying": "ETH",
                "base_asset": "ETH",
                "quote_asset": "USDC",
                "settlement_asset": "USDC",
                "expiry": future_expiry(),
                "strike_1e8": "300000000000",
                "is_call": true,
                "contract_size_1e8": "100000000",
                "onchain_product_id": null,
                "onchain_series_id": null
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let created = response_json(response).await;
    let option_series_id = created["option_series_id"].as_str().unwrap();

    let list = app
        .clone()
        .oneshot(get_request("/options/series?underlying=ETH&status=active"))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    assert_eq!(response_json(list).await.as_array().unwrap().len(), 1);

    let fetched = app
        .clone()
        .oneshot(get_request(&format!("/options/series/{option_series_id}")))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);

    let orderbook = app
        .oneshot(get_request(&format!(
            "/options/orderbooks/{option_series_id}"
        )))
        .await
        .unwrap();
    assert_eq!(orderbook.status(), StatusCode::OK);
    let orderbook = response_json(orderbook).await;
    assert_eq!(orderbook["option_series_id"], option_series_id);
    assert_eq!(orderbook["bids"].as_array().unwrap().len(), 0);
    assert_eq!(orderbook["asks"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn option_orderbook_rejects_unknown_series() {
    let response = router(state())
        .oneshot(get_request("/options/orderbooks/unknown-series"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

fn json_post(uri: &str, value: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
