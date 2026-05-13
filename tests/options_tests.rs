use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{
    accept_option_rfq_quote, cancel_option_order, cancel_option_rfq, create_option_rfq,
    create_option_series, disable_option_series, get_option_fill, get_option_order,
    get_option_order_fills, get_option_orderbook, get_option_series, list_option_fills,
    list_option_orders, list_option_rfq_quotes, list_option_rfqs, list_option_series,
    option_rfq_quote_signing_payload, submit_option_order, submit_option_rfq_quote,
    CreateOptionRfqInput, CreateOptionSeriesInput, OptionRfqQuoteSigningPayloadInput,
    SubmitOptionOrderInput, SubmitOptionRfqQuoteInput,
};
use deopt_v2_backend::options::{
    option_rfq_id_to_b256, option_rfq_quote_digest, option_series_id, option_series_id_to_b256,
    OptionFillFilter, OptionOrderFilter, OptionOrderStatus, OptionRfqQuote,
    OptionRfqQuoteSignatureMode, OptionRfqQuoteSignatureStatus, OptionRfqQuoteSigningPayload,
    OptionRfqQuoteStatus, OptionRfqStatus, OptionSeriesFilter, OptionSeriesIdInput,
    OptionSeriesStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use k256::ecdsa::SigningKey;
use serde_json::json;
use sha3::{Digest, Keccak256};
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

fn strict_option_rfq_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    config.rfq_quote_signature_mode = OptionRfqQuoteSignatureMode::Strict;
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

fn signing_account() -> AccountId {
    AccountId::new(test_account())
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
        quote_nonce: None,
        quote_ttl_ms: Some(5_000),
        signature: None,
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

#[test]
fn option_rfq_id_to_b32_is_deterministic() {
    let option_rfq_id = "a1bbb9bf-2f33-4686-9cdc-30e292ff391f";

    assert_eq!(
        option_rfq_id_to_b256(option_rfq_id),
        option_rfq_id_to_b256(option_rfq_id)
    );
    assert_ne!(
        option_rfq_id_to_b256(option_rfq_id),
        option_rfq_id_to_b256("other-option-rfq-id")
    );
}

#[test]
fn option_series_id_b32_parses_hex_bytes32() {
    let option_series_id = option_series_id(OptionSeriesIdInput {
        underlying: "ETH",
        base_asset: "ETH",
        quote_asset: "USDC",
        settlement_asset: "USDC",
        expiry: 4_102_444_800,
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: 100_000_000,
    });

    let parsed = option_series_id_to_b256(&option_series_id).unwrap();

    assert_eq!(parsed.as_slice().len(), 32);
    assert!(option_series_id_to_b256("not-hex").is_err());
}

#[test]
fn option_rfq_quote_typehash_is_stable() {
    assert_eq!(
        format!(
            "0x{}",
            hex_encode(deopt_v2_backend::options::signing::option_rfq_quote_typehash().as_slice())
        ),
        "0xd44f79e2d92feff94554544c51241a58a4e34965412709747eb106545d3cad5e"
    );
}

#[test]
fn option_rfq_quote_digest_is_deterministic() {
    let payload = OptionRfqQuoteSigningPayload {
        option_rfq_id: option_rfq_id_to_b256("option-rfq-1"),
        mm_account: signing_account(),
        option_series_id: option_series_id_to_b256(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap(),
        taker_is_buyer: true,
        price_1e8: 1_000_000_000,
        size_1e8: 100_000_000,
        quote_nonce: 7,
        expiry: 1_778_300_000,
    };
    let domain = OptionsConfig::disabled().rfq_eip712_domain;

    assert_eq!(
        option_rfq_quote_digest(&payload, &domain).unwrap(),
        option_rfq_quote_digest(&payload, &domain).unwrap()
    );
}

#[tokio::test]
async fn option_rfq_quote_signing_payload_endpoint_returns_expected_structure() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let response = router(state)
        .oneshot(json_post(
            &format!("/options/rfqs/{}/quote-signing-payload", rfq.option_rfq_id),
            json!({
                "mm_account": signing_account().0,
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "client_quote_id": "payload-option-rfq-quote",
                "quote_nonce": 42,
                "quote_ttl_ms": 5000
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["primary_type"], "OptionRFQQuote");
    assert_eq!(json["message"]["mmAccount"], signing_account().0);
    assert_eq!(json["message"]["quoteNonce"], "42");
    assert!(json["option_rfq_id_b32"]
        .as_str()
        .unwrap()
        .starts_with("0x"));
    assert!(json["option_series_id_b32"]
        .as_str()
        .unwrap()
        .starts_with("0x"));
    assert!(json["digest"].as_str().unwrap().starts_with("0x"));
}

#[tokio::test]
async fn disabled_mode_accepts_unsigned_http_option_rfq_quote() {
    let state = option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let response = router(state)
        .oneshot(json_post(
            &format!("/options/rfqs/{}/quotes", rfq.option_rfq_id),
            json!({
                "mm_account": account_two().0,
                "client_quote_id": "unsigned-http-option-rfq-quote",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "quote_ttl_ms": 5000
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["signature_status"], "not_required");
    assert!(json["signature"].is_null());
}

#[tokio::test]
async fn strict_mode_rejects_missing_option_rfq_quote_signature() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let mut input = option_rfq_quote_input(signing_account(), "missing-signature");
    input.quote_nonce = Some(1);

    let error = submit_option_rfq_quote(&state, rfq.option_rfq_id, input)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("signature is required"));
}

#[tokio::test]
async fn strict_mode_rejects_missing_option_rfq_quote_nonce() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let mut input = option_rfq_quote_input(signing_account(), "missing-nonce");
    input.signature = Some(valid_signature_hex(0xaa));

    let error = submit_option_rfq_quote(&state, rfq.option_rfq_id, input)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("quote_nonce is required"));
}

#[tokio::test]
async fn strict_mode_rejects_malformed_option_rfq_quote_signature() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let mut input = option_rfq_quote_input(signing_account(), "malformed-signature");
    input.quote_nonce = Some(1);
    input.signature = Some("not-a-signature".to_string());

    let error = submit_option_rfq_quote(&state, rfq.option_rfq_id, input)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("malformed signature"));
}

#[tokio::test]
async fn strict_mode_rejects_invalid_option_rfq_quote_signature() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let mut input = option_rfq_quote_input(signing_account(), "invalid-signature");
    input.quote_nonce = Some(1);
    input.signature = Some(valid_signature_hex(0xaa));

    let error = submit_option_rfq_quote(&state, rfq.option_rfq_id, input)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("signature"));
}

#[tokio::test]
async fn strict_mode_rejects_option_rfq_quote_signer_mismatch() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let mut input = option_rfq_quote_input(signing_account(), "signer-mismatch");
    input.quote_nonce = Some(1);
    input.signature = Some(sign_option_quote_digest(
        &option_quote_payload_digest(&state, &rfq, &input).await,
        other_signing_key(),
    ));

    let error = submit_option_rfq_quote(&state, rfq.option_rfq_id, input)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("signer does not match"));
}

#[tokio::test]
async fn strict_mode_accepts_valid_option_rfq_quote_signature_and_stores_metadata() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let mut input = option_rfq_quote_input(signing_account(), "valid-signature");
    input.quote_nonce = Some(11);
    input.signature = Some(sign_option_quote_digest(
        &option_quote_payload_digest(&state, &rfq, &input).await,
        test_signing_key(),
    ));

    let quote = submit_option_rfq_quote(&state, rfq.option_rfq_id, input)
        .await
        .unwrap();

    assert_eq!(quote.status, OptionRfqQuoteStatus::Active);
    assert_eq!(
        quote.signature_status,
        OptionRfqQuoteSignatureStatus::Verified
    );
    assert_eq!(quote.recovered_signer, Some(signing_account()));
    assert_eq!(quote.quote_nonce.as_deref(), Some("11"));
    assert!(quote.quote_digest.as_deref().unwrap().starts_with("0x"));
}

#[tokio::test]
async fn strict_http_option_rfq_quote_endpoint_stores_signature_metadata() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let payload = option_rfq_quote_signing_payload(
        &state,
        OptionRfqQuoteSigningPayloadInput {
            option_rfq_id: rfq.option_rfq_id,
            mm_account: signing_account(),
            price_1e8: 1_000_000_000,
            size_1e8: 100_000_000,
            quote_nonce: 21,
            quote_ttl_ms: 5_000,
        },
    )
    .await
    .unwrap();
    let signature = sign_option_quote_digest(&payload.digest, test_signing_key());

    let response = router(state)
        .oneshot(json_post(
            &format!("/options/rfqs/{}/quotes", rfq.option_rfq_id),
            json!({
                "mm_account": signing_account().0,
                "client_quote_id": "signed-http-option-rfq-quote",
                "price_1e8": "1000000000",
                "size_1e8": "100000000",
                "quote_nonce": 21,
                "quote_ttl_ms": 5000,
                "signature": signature
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["signature_status"], "verified");
    assert_eq!(json["recovered_signer"], signing_account().0);
    assert_eq!(json["quote_nonce"], "21");
    assert!(json["quote_digest"].as_str().unwrap().starts_with("0x"));
}

#[tokio::test]
async fn strict_acceptance_requires_active_verified_option_rfq_quote() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let quote = OptionRfqQuote {
        quote_id: uuid::Uuid::new_v4(),
        option_rfq_id: rfq.option_rfq_id,
        mm_account: signing_account(),
        session_id: None,
        client_quote_id: Some("forced-unverified-option-rfq-quote".to_string()),
        price_1e8: 1_000_000_000,
        size_1e8: 100_000_000,
        status: OptionRfqQuoteStatus::Active,
        created_at_ms: now_ms(),
        expires_at_ms: rfq.expires_at_ms,
        signature: None,
        quote_digest: None,
        quote_nonce: None,
        signature_status: OptionRfqQuoteSignatureStatus::Missing,
        recovered_signer: None,
    };
    state
        .options_store
        .lock()
        .unwrap()
        .insert_option_rfq_quote(quote.clone())
        .unwrap();

    let error = accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("option RFQ quote signature is missing"));
}

#[tokio::test]
async fn strict_signed_option_rfq_accept_creates_fill_only() {
    let state = strict_option_rfq_state();
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let mut input = option_rfq_quote_input(signing_account(), "signed-accept-fill-only");
    input.quote_nonce = Some(31);
    input.signature = Some(sign_option_quote_digest(
        &option_quote_payload_digest(&state, &rfq, &input).await,
        test_signing_key(),
    ));
    let quote = submit_option_rfq_quote(&state, rfq.option_rfq_id, input)
        .await
        .unwrap();

    let outcome = accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();
    let transactions = router(state.clone())
        .oneshot(get_request("/executor/transactions"))
        .await
        .unwrap();

    assert_eq!(outcome.rfq.status, OptionRfqStatus::Accepted);
    assert_eq!(outcome.quote.status, OptionRfqQuoteStatus::Accepted);
    assert_eq!(outcome.fill.mm_account, signing_account());
    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 0);
    assert_eq!(
        response_json(transactions).await.as_array().unwrap().len(),
        0
    );
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

async fn option_quote_payload_digest(
    state: &AppState,
    rfq: &deopt_v2_backend::options::OptionRfqRequest,
    input: &SubmitOptionRfqQuoteInput,
) -> String {
    option_rfq_quote_signing_payload(
        state,
        OptionRfqQuoteSigningPayloadInput {
            option_rfq_id: rfq.option_rfq_id,
            mm_account: input.mm_account.clone(),
            price_1e8: input.price_1e8,
            size_1e8: input.size_1e8,
            quote_nonce: input.quote_nonce.unwrap(),
            quote_ttl_ms: input.quote_ttl_ms.unwrap(),
        },
    )
    .await
    .unwrap()
    .digest
}

fn sign_option_quote_digest(digest: &str, signing_key: SigningKey) -> String {
    let digest = parse_digest(digest);
    let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&signature.to_bytes());
    bytes.push(recovery_id.to_byte() + 27);
    format!("0x{}", hex_encode(&bytes))
}

fn valid_signature_hex(byte: u8) -> String {
    let mut signature = String::from("0x");
    for _ in 0..65 {
        signature.push_str(&format!("{byte:02x}"));
    }
    signature
}

fn test_account() -> String {
    let verifying_key = test_signing_key().verifying_key().to_encoded_point(false);
    let hash = Keccak256::digest(&verifying_key.as_bytes()[1..]);
    format!("0x{}", hex_encode(&hash[12..]))
}

fn test_signing_key() -> SigningKey {
    signing_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318")
}

fn other_signing_key() -> SigningKey {
    signing_key_from_hex("6c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318")
}

fn signing_key_from_hex(hex: &str) -> SigningKey {
    let mut bytes = [0u8; 32];
    decode_hex_to_slice(hex, &mut bytes).unwrap();
    SigningKey::from_slice(&bytes).unwrap()
}

fn parse_digest(value: &str) -> [u8; 32] {
    let hex = value.strip_prefix("0x").unwrap();
    let mut bytes = [0u8; 32];
    decode_hex_to_slice(hex, &mut bytes).unwrap();
    bytes
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex_to_slice(hex: &str, out: &mut [u8]) -> std::result::Result<(), ()> {
    if hex.len() != out.len() * 2 {
        return Err(());
    }

    for (index, byte) in out.iter_mut().enumerate() {
        let high = decode_hex_nibble(hex.as_bytes()[index * 2])?;
        let low = decode_hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }

    Ok(())
}

fn decode_hex_nibble(byte: u8) -> std::result::Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}
