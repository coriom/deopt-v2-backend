use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{
    accept_option_rfq_quote, cancel_option_order, cancel_option_rfq, create_option_rfq,
    create_option_series, disable_option_series, get_option_fill, get_option_order,
    get_option_order_fills, get_option_orderbook, get_option_series, list_option_fills,
    list_option_orders, list_option_rfq_quotes, list_option_rfqs, list_option_series,
    submit_option_order, submit_option_rfq_quote, CreateOptionRfqInput, CreateOptionSeriesInput,
    SubmitOptionOrderInput, SubmitOptionRfqQuoteInput,
};
use deopt_v2_backend::options::{
    option_series_id, OptionFillFilter, OptionOrderFilter, OptionOrderStatus, OptionRfqQuoteStatus,
    OptionRfqStatus, OptionSeriesFilter, OptionSeriesIdInput, OptionSeriesStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use serde_json::json;
use tokio::time::{sleep, Duration};
use tower::ServiceExt;

const VALID_SIGNATURE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn state() -> AppState {
    AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
    )
}

fn option_rfq_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    AppState::with_options_config(EngineState::with_default_markets(), config)
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

fn account_two() -> AccountId {
    AccountId::new("0x0000000000000000000000000000000000000002")
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

fn option_rfq_input(option_series_id: String, side: Side) -> CreateOptionRfqInput {
    CreateOptionRfqInput {
        taker: account(),
        option_series_id,
        side,
        size_1e8: 100_000_000,
        limit_price_1e8: Some(1_100_000_000),
        ttl_ms: Some(10_000),
    }
}

fn option_rfq_quote_input(
    mm_account: AccountId,
    client_quote_id: &str,
) -> SubmitOptionRfqQuoteInput {
    SubmitOptionRfqQuoteInput {
        mm_account,
        session_id: Some("test-mm-session".to_string()),
        client_quote_id: Some(client_quote_id.to_string()),
        price_1e8: 1_000_000_000,
        size_1e8: 100_000_000,
        quote_ttl_ms: Some(5_000),
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

    assert_eq!(order.order.option_series_id, option_series_id);
    assert_eq!(order.order.side, Side::Buy);
    assert_eq!(order.order.status, OptionOrderStatus::Open);
    assert!(order.fills.is_empty());
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

    assert_eq!(order.order.option_series_id, option_series_id);
    assert_eq!(order.order.side, Side::Sell);
    assert_eq!(order.order.status, OptionOrderStatus::Open);
    assert!(order.fills.is_empty());
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
    let fetched = get_option_order(&state, order.order.order_id)
        .await
        .unwrap();
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

    assert_eq!(fetched.order_id, order.order.order_id);
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
    let cancelled = cancel_option_order(&state, order.order.order_id)
        .await
        .unwrap();

    assert_eq!(cancelled.status, OptionOrderStatus::Cancelled);
    assert!(cancel_option_order(&state, order.order.order_id)
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

    cancel_option_order(&state, order.order.order_id)
        .await
        .unwrap();

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
async fn buy_crosses_ask_and_records_fill_at_resting_price() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "maker-ask");
    ask.price_1e8 = 950_000_000;
    let maker = submit_option_order(&state, ask).await.unwrap().order;
    let mut buy = order_input(option_series_id.clone(), Side::Buy, "taker-buy");
    buy.price_1e8 = 1_000_000_000;

    let outcome = submit_option_order(&state, buy).await.unwrap();

    assert_eq!(outcome.order.status, OptionOrderStatus::Filled);
    assert_eq!(outcome.order.remaining_size_1e8, 0);
    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.fills[0].price_1e8, 950_000_000);
    assert_eq!(outcome.fills[0].size_1e8, 100_000_000);
    assert_eq!(outcome.fills[0].maker_order_id, maker.order_id);
    assert_eq!(
        get_option_order(&state, maker.order_id)
            .await
            .unwrap()
            .status,
        OptionOrderStatus::Filled
    );
}

#[tokio::test]
async fn sell_crosses_bid_and_records_fill_at_resting_price() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut bid = order_input(option_series_id.clone(), Side::Buy, "maker-bid");
    bid.price_1e8 = 1_050_000_000;
    let maker = submit_option_order(&state, bid).await.unwrap().order;
    let mut sell = order_input(option_series_id, Side::Sell, "taker-sell");
    sell.price_1e8 = 1_000_000_000;

    let outcome = submit_option_order(&state, sell).await.unwrap();

    assert_eq!(outcome.order.status, OptionOrderStatus::Filled);
    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.fills[0].price_1e8, 1_050_000_000);
    assert_eq!(outcome.fills[0].buy_order_id, maker.order_id);
    assert_eq!(outcome.fills[0].taker_side, Side::Sell);
}

#[tokio::test]
async fn no_cross_when_buy_price_below_ask() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "no-cross-ask");
    ask.price_1e8 = 1_100_000_000;
    submit_option_order(&state, ask).await.unwrap();
    let mut buy = order_input(option_series_id.clone(), Side::Buy, "no-cross-buy");
    buy.price_1e8 = 1_000_000_000;

    let outcome = submit_option_order(&state, buy).await.unwrap();
    let book = get_option_orderbook(&state, option_series_id)
        .await
        .unwrap();

    assert!(outcome.fills.is_empty());
    assert_eq!(outcome.order.status, OptionOrderStatus::Open);
    assert_eq!(book.bids[0].price_1e8, "1000000000");
    assert_eq!(book.asks[0].price_1e8, "1100000000");
}

#[tokio::test]
async fn partial_fill_updates_incoming_resting_and_orderbook_remaining() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "small-ask");
    ask.price_1e8 = 900_000_000;
    ask.size_1e8 = 50_000_000;
    submit_option_order(&state, ask).await.unwrap();
    let mut buy = order_input(option_series_id.clone(), Side::Buy, "large-buy");
    buy.price_1e8 = 1_000_000_000;
    buy.size_1e8 = 100_000_000;

    let outcome = submit_option_order(&state, buy).await.unwrap();
    let book = get_option_orderbook(&state, option_series_id)
        .await
        .unwrap();

    assert_eq!(outcome.order.status, OptionOrderStatus::PartiallyFilled);
    assert_eq!(outcome.order.remaining_size_1e8, 50_000_000);
    assert_eq!(outcome.fills[0].size_1e8, 50_000_000);
    assert_eq!(book.bids[0].size_1e8, "50000000");
    assert!(book.asks.is_empty());
}

#[tokio::test]
async fn partial_fill_updates_resting_order_remaining_and_can_cancel_remainder() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "large-ask");
    ask.price_1e8 = 1_000_000_000;
    ask.size_1e8 = 100_000_000;
    let maker = submit_option_order(&state, ask).await.unwrap().order;
    let mut buy = order_input(option_series_id.clone(), Side::Buy, "small-buy");
    buy.price_1e8 = 1_000_000_000;
    buy.size_1e8 = 40_000_000;

    submit_option_order(&state, buy).await.unwrap();
    let maker = get_option_order(&state, maker.order_id).await.unwrap();
    let cancelled = cancel_option_order(&state, maker.order_id).await.unwrap();
    let book = get_option_orderbook(&state, option_series_id)
        .await
        .unwrap();

    assert_eq!(maker.status, OptionOrderStatus::PartiallyFilled);
    assert_eq!(maker.remaining_size_1e8, 60_000_000);
    assert_eq!(cancelled.status, OptionOrderStatus::Cancelled);
    assert!(book.asks.is_empty());
}

#[tokio::test]
async fn cannot_cancel_filled_order() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let ask = submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Sell, "filled-ask"),
    )
    .await
    .unwrap()
    .order;
    submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "fills-ask"),
    )
    .await
    .unwrap();

    let error = cancel_option_order(&state, ask.order_id).await.unwrap_err();

    assert!(error.to_string().contains("option order is filled"));
}

#[tokio::test]
async fn multiple_fills_use_price_then_time_priority() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut high_ask = order_input(option_series_id.clone(), Side::Sell, "high-ask");
    high_ask.price_1e8 = 1_100_000_000;
    let high = submit_option_order(&state, high_ask).await.unwrap().order;
    sleep(Duration::from_millis(2)).await;
    let mut low_ask_first = order_input(option_series_id.clone(), Side::Sell, "low-ask-first");
    low_ask_first.price_1e8 = 1_000_000_000;
    low_ask_first.size_1e8 = 40_000_000;
    let low_first = submit_option_order(&state, low_ask_first)
        .await
        .unwrap()
        .order;
    sleep(Duration::from_millis(2)).await;
    let mut low_ask_second = order_input(option_series_id.clone(), Side::Sell, "low-ask-second");
    low_ask_second.price_1e8 = 1_000_000_000;
    low_ask_second.size_1e8 = 50_000_000;
    let low_second = submit_option_order(&state, low_ask_second)
        .await
        .unwrap()
        .order;
    let mut buy = order_input(option_series_id, Side::Buy, "sweeping-buy");
    buy.price_1e8 = 1_200_000_000;
    buy.size_1e8 = 190_000_000;

    let outcome = submit_option_order(&state, buy).await.unwrap();

    assert_eq!(outcome.order.status, OptionOrderStatus::Filled);
    assert_eq!(outcome.fills.len(), 3);
    assert_eq!(outcome.fills[0].maker_order_id, low_first.order_id);
    assert_eq!(outcome.fills[0].price_1e8, 1_000_000_000);
    assert_eq!(outcome.fills[1].maker_order_id, low_second.order_id);
    assert_eq!(outcome.fills[1].price_1e8, 1_000_000_000);
    assert_eq!(outcome.fills[2].maker_order_id, high.order_id);
    assert_eq!(outcome.fills[2].price_1e8, 1_100_000_000);
}

#[tokio::test]
async fn bid_price_priority_uses_highest_bid_first() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut low_bid = order_input(option_series_id.clone(), Side::Buy, "low-bid");
    low_bid.price_1e8 = 900_000_000;
    let low = submit_option_order(&state, low_bid).await.unwrap().order;
    let mut high_bid = order_input(option_series_id.clone(), Side::Buy, "high-bid");
    high_bid.price_1e8 = 1_000_000_000;
    let high = submit_option_order(&state, high_bid).await.unwrap().order;
    let mut sell = order_input(option_series_id, Side::Sell, "sweep-bids");
    sell.price_1e8 = 800_000_000;
    sell.size_1e8 = 200_000_000;

    let outcome = submit_option_order(&state, sell).await.unwrap();

    assert_eq!(outcome.fills[0].maker_order_id, high.order_id);
    assert_eq!(outcome.fills[0].price_1e8, 1_000_000_000);
    assert_eq!(outcome.fills[1].maker_order_id, low.order_id);
    assert_eq!(outcome.fills[1].price_1e8, 900_000_000);
}

#[tokio::test]
async fn list_and_get_option_fills_by_series_order_and_account() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "fill-list-ask");
    ask.account = account_two();
    submit_option_order(&state, ask).await.unwrap();
    let outcome = submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Buy, "fill-list-buy"),
    )
    .await
    .unwrap();
    let fill = outcome.fills[0].clone();

    let by_series = list_option_fills(
        &state,
        OptionFillFilter {
            option_series_id: Some(option_series_id),
            ..OptionFillFilter::default()
        },
    )
    .await
    .unwrap();
    let by_order = get_option_order_fills(&state, outcome.order.order_id)
        .await
        .unwrap();
    let by_account = list_option_fills(
        &state,
        OptionFillFilter {
            account: Some(account_two()),
            ..OptionFillFilter::default()
        },
    )
    .await
    .unwrap();
    let fetched = get_option_fill(&state, fill.fill_id).await.unwrap();

    assert_eq!(by_series.len(), 1);
    assert_eq!(by_order.len(), 1);
    assert_eq!(by_account.len(), 1);
    assert_eq!(fetched.fill_id, fill.fill_id);
}

#[tokio::test]
async fn option_match_does_not_create_execution_intent_or_transaction() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    submit_option_order(
        &state,
        order_input(option_series_id.clone(), Side::Sell, "safe-ask"),
    )
    .await
    .unwrap();
    submit_option_order(&state, order_input(option_series_id, Side::Buy, "safe-buy"))
        .await
        .unwrap();

    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 0);
    let response = router(state)
        .oneshot(get_request("/executor/transactions"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await.as_array().unwrap().len(), 0);
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

#[tokio::test]
async fn http_option_match_returns_fills_and_fill_endpoints() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let app = router(state);
    let ask = app
        .clone()
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": option_series_id,
                "account": account_two().0,
                "side": "sell",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "client_order_id": "http-fill-ask",
                "nonce": 1,
                "deadline_ms": now_ms() + 60_000,
                "signature": VALID_SIGNATURE
            }),
        ))
        .await
        .unwrap();
    assert_eq!(ask.status(), StatusCode::OK);

    let buy = app
        .clone()
        .oneshot(json_post(
            "/options/orders",
            json!({
                "option_series_id": option_series_id,
                "account": account().0,
                "side": "buy",
                "price_1e8": "1100000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "client_order_id": "http-fill-buy",
                "nonce": 2,
                "deadline_ms": now_ms() + 60_000,
                "signature": VALID_SIGNATURE
            }),
        ))
        .await
        .unwrap();
    assert_eq!(buy.status(), StatusCode::OK);
    let buy = response_json(buy).await;
    let order_id = buy["order_id"].as_str().unwrap();
    let fills = buy["fills"].as_array().unwrap();
    assert_eq!(buy["status"], "filled");
    assert_eq!(fills.len(), 1);
    let fill_id = fills[0]["fill_id"].as_str().unwrap();

    let fill = app
        .clone()
        .oneshot(get_request(&format!("/options/fills/{fill_id}")))
        .await
        .unwrap();
    assert_eq!(fill.status(), StatusCode::OK);
    assert_eq!(response_json(fill).await["price_1e8"], "1000000000");

    let by_order = app
        .clone()
        .oneshot(get_request(&format!("/options/orders/{order_id}/fills")))
        .await
        .unwrap();
    assert_eq!(response_json(by_order).await.as_array().unwrap().len(), 1);

    let listed = app
        .oneshot(get_request(&format!("/options/fills?order_id={order_id}")))
        .await
        .unwrap();
    assert_eq!(response_json(listed).await.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn option_rfq_create_quote_accept_buy_creates_offchain_fill_only() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(
        &state,
        option_rfq_input(option_series_id.clone(), Side::Buy),
    )
    .await
    .unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "buy-rfq-quote"),
    )
    .await
    .unwrap();

    let outcome = accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();
    let quotes = list_option_rfq_quotes(&state, rfq.option_rfq_id)
        .await
        .unwrap();
    let rfqs = list_option_rfqs(&state).await.unwrap();

    assert_eq!(outcome.rfq.status, OptionRfqStatus::Accepted);
    assert_eq!(outcome.quote.status, OptionRfqQuoteStatus::Accepted);
    assert_eq!(outcome.fill.option_series_id, option_series_id);
    assert_eq!(outcome.fill.buyer, account());
    assert_eq!(outcome.fill.seller, account_two());
    assert_eq!(outcome.fill.taker_side, Side::Buy);
    assert_eq!(outcome.fill.price_1e8, 1_000_000_000);
    assert_eq!(outcome.fill.size_1e8, 100_000_000);
    assert_eq!(quotes.len(), 1);
    assert_eq!(rfqs.len(), 1);
    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 0);
}

#[tokio::test]
async fn option_rfq_accept_sell_maps_buyer_to_mm_and_seller_to_taker() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let mut input = option_rfq_input(option_series_id, Side::Sell);
    input.limit_price_1e8 = Some(900_000_000);
    let rfq = create_option_rfq(&state, input).await.unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "sell-rfq-quote"),
    )
    .await
    .unwrap();

    let outcome = accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();

    assert_eq!(outcome.fill.buyer, account_two());
    assert_eq!(outcome.fill.seller, account());
    assert_eq!(outcome.fill.taker, account());
    assert_eq!(outcome.fill.mm_account, account_two());
    assert_eq!(outcome.fill.taker_side, Side::Sell);
}

#[tokio::test]
async fn option_rfq_rejects_disabled_service_and_invalid_inputs() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let error = create_option_rfq(
        &state,
        option_rfq_input(option_series_id.clone(), Side::Buy),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("option RFQ is disabled"));

    let state = option_rfq_state();
    let mut input = option_rfq_input(option_series_id, Side::Buy);
    input.taker = AccountId::new("bad-address");
    assert!(create_option_rfq(&state, input)
        .await
        .unwrap_err()
        .to_string()
        .contains("malformed account address"));

    let mut input = option_rfq_input("unknown-series".to_string(), Side::Buy);
    input.taker = account();
    assert!(create_option_rfq(&state, input)
        .await
        .unwrap_err()
        .to_string()
        .contains("invalid option series id"));
}

#[tokio::test]
async fn option_rfq_quote_validations_and_duplicate_client_quote_id() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "dup-rfq-quote"),
    )
    .await
    .unwrap();

    let duplicate = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "dup-rfq-quote"),
    )
    .await
    .unwrap_err();
    assert!(duplicate.to_string().contains("duplicate client_quote_id"));

    let mut zero_price = option_rfq_quote_input(account_two(), "zero-price-rfq-quote");
    zero_price.price_1e8 = 0;
    assert!(
        submit_option_rfq_quote(&state, rfq.option_rfq_id, zero_price)
            .await
            .unwrap_err()
            .to_string()
            .contains("zero price")
    );

    let mut too_large = option_rfq_quote_input(account_two(), "too-large-rfq-quote");
    too_large.size_1e8 = 200_000_000;
    assert!(
        submit_option_rfq_quote(&state, rfq.option_rfq_id, too_large)
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds requested size")
    );
}

#[tokio::test]
async fn option_rfq_accept_rejects_price_limits_and_expiry() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(
        &state,
        option_rfq_input(option_series_id.clone(), Side::Buy),
    )
    .await
    .unwrap();
    let mut expensive = option_rfq_quote_input(account_two(), "expensive-rfq-quote");
    expensive.price_1e8 = 1_200_000_000;
    let expensive = submit_option_rfq_quote(&state, rfq.option_rfq_id, expensive)
        .await
        .unwrap();
    assert!(
        accept_option_rfq_quote(&state, rfq.option_rfq_id, expensive.quote_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("violates limit")
    );

    let mut expiring_rfq_input = option_rfq_input(option_series_id, Side::Buy);
    expiring_rfq_input.ttl_ms = Some(1);
    let expired_rfq = create_option_rfq(&state, expiring_rfq_input).await.unwrap();
    sleep(Duration::from_millis(2)).await;
    assert!(submit_option_rfq_quote(
        &state,
        expired_rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "expired-rfq-quote")
    )
    .await
    .unwrap_err()
    .to_string()
    .contains("not open"));
}

#[tokio::test]
async fn option_rfq_accept_is_single_winner_and_rejects_competing_quotes() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let first = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "winner-rfq-quote"),
    )
    .await
    .unwrap();
    let second = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(
            AccountId::new("0x0000000000000000000000000000000000000003"),
            "loser-rfq-quote",
        ),
    )
    .await
    .unwrap();

    accept_option_rfq_quote(&state, rfq.option_rfq_id, first.quote_id)
        .await
        .unwrap();
    let quotes = list_option_rfq_quotes(&state, rfq.option_rfq_id)
        .await
        .unwrap();
    let rejected = quotes
        .iter()
        .find(|quote| quote.quote_id == second.quote_id)
        .unwrap();

    assert_eq!(rejected.status, OptionRfqQuoteStatus::Rejected);
    assert!(
        accept_option_rfq_quote(&state, rfq.option_rfq_id, second.quote_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("not open")
    );
}

#[tokio::test]
async fn option_rfq_cancel_cancels_active_quotes_and_blocks_accept() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "cancel-rfq-quote"),
    )
    .await
    .unwrap();

    let cancelled = cancel_option_rfq(&state, rfq.option_rfq_id).await.unwrap();
    let quotes = list_option_rfq_quotes(&state, rfq.option_rfq_id)
        .await
        .unwrap();

    assert_eq!(cancelled.status, OptionRfqStatus::Cancelled);
    assert_eq!(quotes[0].status, OptionRfqQuoteStatus::Cancelled);
    assert!(
        accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("not open")
    );
}

#[tokio::test]
async fn option_rfq_http_lifecycle() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let app = router(state);
    let created = app
        .clone()
        .oneshot(json_post(
            "/options/rfqs",
            json!({
                "taker": account().0,
                "option_series_id": option_series_id,
                "side": "buy",
                "size_1e8": "100000000",
                "limit_price_1e8": "1100000000",
                "ttl_ms": 10000
            }),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let created = response_json(created).await;
    let option_rfq_id = created["option_rfq_id"].as_str().unwrap();
    assert_eq!(created["status"], "open");

    let quote = app
        .clone()
        .oneshot(json_post(
            &format!("/options/rfqs/{option_rfq_id}/quotes"),
            json!({
                "mm_account": account_two().0,
                "session_id": "http-mm-session",
                "client_quote_id": "http-option-rfq-quote",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "quote_ttl_ms": 5000
            }),
        ))
        .await
        .unwrap();
    assert_eq!(quote.status(), StatusCode::OK);
    let quote = response_json(quote).await;
    let quote_id = quote["quote_id"].as_str().unwrap();

    let listed_rfqs = app
        .clone()
        .oneshot(get_request("/options/rfqs"))
        .await
        .unwrap();
    assert_eq!(
        response_json(listed_rfqs).await.as_array().unwrap().len(),
        1
    );

    let listed_quotes = app
        .clone()
        .oneshot(get_request(&format!(
            "/options/rfqs/{option_rfq_id}/quotes"
        )))
        .await
        .unwrap();
    assert_eq!(
        response_json(listed_quotes).await.as_array().unwrap().len(),
        1
    );

    let accepted = app
        .clone()
        .oneshot(json_post(
            &format!("/options/rfqs/{option_rfq_id}/accept/{quote_id}"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted = response_json(accepted).await;
    assert_eq!(accepted["status"], "accepted");
    assert_eq!(accepted["quote_status"], "accepted");
    assert_eq!(accepted["fill"]["buyer"], account().0);
    assert_eq!(accepted["fill"]["seller"], account_two().0);

    let executor_transactions = app
        .oneshot(get_request("/executor/transactions"))
        .await
        .unwrap();
    assert_eq!(
        response_json(executor_transactions)
            .await
            .as_array()
            .unwrap()
            .len(),
        0
    );
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
