use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{
    cancel_option_order, create_option_series, disable_option_series, get_option_order,
    get_option_orderbook, get_option_series, list_option_orders, list_option_series,
    submit_option_order, CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{
    option_series_id, OptionOrderFilter, OptionOrderStatus, OptionSeriesFilter,
    OptionSeriesIdInput, OptionSeriesStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use serde_json::json;
use tower::ServiceExt;

const VALID_SIGNATURE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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

fn account() -> AccountId {
    AccountId::new("0x0000000000000000000000000000000000000001")
}

async fn active_series_id(state: &AppState) -> String {
    create_option_series(state, create_input())
        .await
        .unwrap()
        .option_series_id
}

fn order_input(
    option_series_id: String,
    side: Side,
    client_order_id: &str,
) -> SubmitOptionOrderInput {
    SubmitOptionOrderInput {
        option_series_id,
        account: account(),
        side,
        price_1e8: 1_000_000_000,
        size_1e8: 100_000_000,
        time_in_force: TimeInForce::Gtc,
        client_order_id: Some(client_order_id.to_string()),
        nonce: Some(1),
        deadline_ms: Some(now_ms() + 60_000),
        signature: Some(VALID_SIGNATURE.to_string()),
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

#[tokio::test]
async fn submit_option_buy_order_success() {
    let state = state();
    let option_series_id = active_series_id(&state).await;

    let order = submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Buy, "buy-1"),
    )
    .await
    .unwrap();

    assert_eq!(order.option_series_id, option_series_id);
    assert_eq!(order.side, Side::Buy);
    assert_eq!(order.status, OptionOrderStatus::Open);
}

#[tokio::test]
async fn submit_option_sell_order_success() {
    let state = state();
    let option_series_id = active_series_id(&state).await;

    let order = submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Sell, "sell-1"),
    )
    .await
    .unwrap();

    assert_eq!(order.option_series_id, option_series_id);
    assert_eq!(order.side, Side::Sell);
    assert_eq!(order.status, OptionOrderStatus::Open);
}

#[tokio::test]
async fn option_order_rejects_unknown_series() {
    let state = state();

    let error = submit_option_order(
        &state,
        order_input("unknown-series".to_string(), Side::Buy, "unknown"),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("invalid option series id"));
}

#[tokio::test]
async fn option_order_rejects_disabled_series() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    disable_option_series(&state, &option_series_id)
        .await
        .unwrap();

    let error = submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "disabled-series"),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("option series is not active"));
}

#[tokio::test]
async fn option_order_rejects_invalid_account() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut input = order_input(option_series_id, Side::Buy, "bad-account");
    input.account = AccountId::new("not-an-address");

    let error = submit_option_order(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("malformed account address"));
}

#[tokio::test]
async fn http_option_order_rejects_invalid_side() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let response = router(state)
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": option_series_id,
                "account": account().0,
                "side": "hold",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "client_order_id": "bad-side",
                "nonce": 1,
                "deadline_ms": now_ms() + 60_000,
                "signature": VALID_SIGNATURE
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn option_order_rejects_zero_price_and_size() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut zero_price = order_input(option_series_id.clone(), Side::Buy, "zero-price");
    zero_price.price_1e8 = 0;
    let mut zero_size = order_input(option_series_id, Side::Buy, "zero-size");
    zero_size.size_1e8 = 0;

    assert!(submit_option_order(&state, zero_price)
        .await
        .unwrap_err()
        .to_string()
        .contains("zero price"));
    assert!(submit_option_order(&state, zero_size)
        .await
        .unwrap_err()
        .to_string()
        .contains("zero size"));
}

#[tokio::test]
async fn option_order_rejects_unsupported_time_in_force() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut input = order_input(option_series_id, Side::Buy, "ioc");
    input.time_in_force = TimeInForce::Ioc;

    let error = submit_option_order(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("time in force is unsupported"));
}

#[tokio::test]
async fn duplicate_open_client_order_id_is_rejected() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Buy, "dup-client"),
    )
    .await
    .unwrap();

    let error = submit_option_order(
        &state,
        order_input(option_series_id, Side::Sell, "dup-client"),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("duplicate open client_order_id"));
}

#[tokio::test]
async fn get_and_list_option_orders() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let order = submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Buy, "list-1"),
    )
    .await
    .unwrap();
    let fetched = get_option_order(&state, order.order_id).await.unwrap();
    let by_series = list_option_orders(
        &state,
        OptionOrderFilter {
            option_series_id: Some(option_series_id),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .unwrap();
    let by_account = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(account()),
            ..OptionOrderFilter::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(fetched.order_id, order.order_id);
    assert_eq!(by_series.len(), 1);
    assert_eq!(by_account.len(), 1);
}

#[tokio::test]
async fn cancel_option_order_and_reject_second_cancel() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let order = submit_option_order(&state, order_input(option_series_id, Side::Buy, "cancel-1"))
        .await
        .unwrap();
    let cancelled = cancel_option_order(&state, order.order_id).await.unwrap();

    assert_eq!(cancelled.status, OptionOrderStatus::Cancelled);
    assert!(cancel_option_order(&state, order.order_id)
        .await
        .unwrap_err()
        .to_string()
        .contains("option order is cancelled"));
}

#[tokio::test]
async fn option_orderbook_aggregates_and_sorts_levels() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut bid_low = order_input(option_series_id.clone(), Side::Buy, "bid-low");
    bid_low.price_1e8 = 900_000_000;
    let mut bid_high_a = order_input(option_series_id.clone(), Side::Buy, "bid-high-a");
    bid_high_a.price_1e8 = 1_000_000_000;
    bid_high_a.size_1e8 = 100_000_000;
    let mut bid_high_b = order_input(option_series_id.clone(), Side::Buy, "bid-high-b");
    bid_high_b.price_1e8 = 1_000_000_000;
    bid_high_b.size_1e8 = 200_000_000;
    let mut ask_low_a = order_input(option_series_id.clone(), Side::Sell, "ask-low-a");
    ask_low_a.price_1e8 = 1_100_000_000;
    ask_low_a.size_1e8 = 100_000_000;
    let mut ask_low_b = order_input(option_series_id.clone(), Side::Sell, "ask-low-b");
    ask_low_b.price_1e8 = 1_100_000_000;
    ask_low_b.size_1e8 = 300_000_000;
    let mut ask_high = order_input(option_series_id.clone(), Side::Sell, "ask-high");
    ask_high.price_1e8 = 1_200_000_000;

    for input in [
        bid_low, bid_high_a, bid_high_b, ask_low_a, ask_low_b, ask_high,
    ] {
        submit_option_order(&state, input).await.unwrap();
    }

    let book = get_option_orderbook(&state, option_series_id)
        .await
        .unwrap();

    assert_eq!(book.bids[0].price_1e8, "1000000000");
    assert_eq!(book.bids[0].size_1e8, "300000000");
    assert_eq!(book.bids[1].price_1e8, "900000000");
    assert_eq!(book.asks[0].price_1e8, "1100000000");
    assert_eq!(book.asks[0].size_1e8, "400000000");
    assert_eq!(book.asks[1].price_1e8, "1200000000");
}

#[tokio::test]
async fn cancelling_option_order_removes_it_from_orderbook() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let order = submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Buy, "book-cancel"),
    )
    .await
    .unwrap();
    assert_eq!(
        get_option_orderbook(&state, option_series_id.clone())
            .await
            .unwrap()
            .bids
            .len(),
        1
    );

    cancel_option_order(&state, order.order_id).await.unwrap();

    assert_eq!(
        get_option_orderbook(&state, option_series_id)
            .await
            .unwrap()
            .bids
            .len(),
        0
    );
}

#[tokio::test]
async fn option_order_does_not_create_execution_intent() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "no-intent"),
    )
    .await
    .unwrap();

    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 0);
}

#[tokio::test]
async fn http_option_order_lifecycle() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let app = router(state);
    let submitted = app
        .clone()
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": option_series_id,
                "account": account().0,
                "side": "buy",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "client_order_id": "http-option-order",
                "nonce": 1,
                "deadline_ms": now_ms() + 60_000,
                "signature": VALID_SIGNATURE
            }),
        ))
        .await
        .unwrap();
    assert_eq!(submitted.status(), StatusCode::OK);
    let submitted = response_json(submitted).await;
    let order_id = submitted["order_id"].as_str().unwrap();
    assert_eq!(submitted["status"], "open");

    let fetched = app
        .clone()
        .oneshot(get_request(&format!("/options/orders/{order_id}")))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);

    let listed = app
        .clone()
        .oneshot(get_request(&format!(
            "/options/orders?option_series_id={option_series_id}&side=buy&status=open"
        )))
        .await
        .unwrap();
    assert_eq!(response_json(listed).await.as_array().unwrap().len(), 1);

    let cancelled = app
        .oneshot(json_post(
            &format!("/options/orders/{order_id}/cancel"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    assert_eq!(response_json(cancelled).await["status"], "cancelled");
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
