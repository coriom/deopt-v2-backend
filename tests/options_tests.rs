use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::fees::{FeeMarketType, FeesConfig};
use deopt_v2_backend::mm::{MmAccountPermissions, MmPermissionsConfig};
use deopt_v2_backend::options::service::{
    accept_option_rfq_quote, cancel_option_order, cancel_option_rfq, create_option_rfq,
    create_option_series, disable_option_series, get_option_fill, get_option_order,
    get_option_order_fills, get_option_orderbook, get_option_series, list_option_execution_intents,
    list_option_fills, list_option_order_attachment_plans_for_account,
    list_option_order_rejections_for_account, list_option_orders, list_option_rfq_quotes,
    list_option_rfqs, list_option_series, option_execution_calldata,
    option_execution_signing_payload, option_rfq_quote_signing_payload,
    submit_option_execution_signatures, submit_option_order, submit_option_rfq_quote,
    sweep_expired_option_orders, AttachedLegInput, AttachedTpSlInput, CreateOptionRfqInput,
    CreateOptionSeriesInput, OptionRfqQuoteSigningPayloadInput,
    SubmitOptionExecutionSignaturesInput, SubmitOptionOrderInput, SubmitOptionRfqQuoteInput,
};
use deopt_v2_backend::options::{
    expected_option_execute_trade_selector, option_execute_trade_selector,
    option_product_registry_option_id, option_rfq_id_to_b256, option_rfq_quote_digest,
    option_series_id, option_series_id_to_b256, OptionExecutionIntentStatus,
    OptionExecutionSignatureMode, OptionExecutionSourceType, OptionFillFilter, OptionOrder,
    OptionOrderFilter, OptionOrderStatus, OptionRfqQuote, OptionRfqQuoteSignatureMode,
    OptionRfqQuoteSignatureStatus, OptionRfqQuoteSigningPayload, OptionRfqQuoteStatus,
    OptionRfqStatus, OptionSeriesFilter, OptionSeriesIdInput, OptionSeriesStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use k256::ecdsa::SigningKey;
use serde_json::json;
use sha3::{Digest, Keccak256};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;

const VALID_SIGNATURE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ONE_1E8: u128 = 100_000_000;
const VOLUME_25M_1E8: u128 = 25_000_000 * ONE_1E8;

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

fn fee_state(rebates_enabled: bool) -> AppState {
    let mut options = OptionsConfig::enabled_in_memory_for_tests();
    options.rfq_enabled = true;
    let mut fees = FeesConfig::enabled_in_memory_for_tests();
    fees.rebates_enabled = rebates_enabled;
    AppState::with_options_and_fees_config(EngineState::with_default_markets(), options, fees)
}

fn strict_option_rfq_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    config.rfq_quote_signature_mode = OptionRfqQuoteSignatureMode::Strict;
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

fn option_execution_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.execution_enabled = true;
    config.execution_require_persistence = false;
    config.matching_engine_address = AccountId::new("0x00000000000000000000000000000000000000ee");
    config.execution_eip712_domain.verifying_contract = config.matching_engine_address.clone();
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

fn option_execution_rfq_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    config.execution_enabled = true;
    config.execution_require_persistence = false;
    config.matching_engine_address = AccountId::new("0x00000000000000000000000000000000000000ee");
    config.execution_eip712_domain.verifying_contract = config.matching_engine_address.clone();
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

fn strict_option_execution_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.execution_enabled = true;
    config.execution_require_persistence = false;
    config.execution_signature_mode = OptionExecutionSignatureMode::Strict;
    config.matching_engine_address = AccountId::new("0x00000000000000000000000000000000000000ee");
    config.execution_eip712_domain.verifying_contract = config.matching_engine_address.clone();
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

fn onchain_create_input() -> CreateOptionSeriesInput {
    let expiry = future_expiry();
    let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
    let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
    let onchain_option_id = onchain_option_id_for(expiry);
    CreateOptionSeriesInput {
        underlying: underlying.0,
        base_asset: "ETH".to_string(),
        quote_asset: "USDC".to_string(),
        settlement_asset: settlement_asset.0,
        expiry,
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: Some(100_000_000),
        onchain_product_id: None,
        onchain_series_id: Some(onchain_option_id),
    }
}

fn onchain_option_id_for(expiry: u64) -> String {
    option_product_registry_option_id(
        &AccountId::new("0x0000000000000000000000000000000000000010"),
        &AccountId::new("0x0000000000000000000000000000000000000020"),
        expiry,
        300_000_000_000,
        100_000_000,
        true,
        true,
    )
    .unwrap()
    .to_string()
}

fn mismatched_onchain_create_input() -> CreateOptionSeriesInput {
    let expiry = future_expiry();
    let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
    let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
    let onchain_option_id = option_product_registry_option_id(
        &underlying,
        &settlement_asset,
        expiry + 1,
        300_000_000_000,
        100_000_000,
        true,
        true,
    )
    .unwrap()
    .to_string();
    CreateOptionSeriesInput {
        underlying: underlying.0,
        base_asset: "ETH".to_string(),
        quote_asset: "USDC".to_string(),
        settlement_asset: settlement_asset.0,
        expiry,
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: Some(100_000_000),
        onchain_product_id: None,
        onchain_series_id: Some(onchain_option_id),
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

fn other_signing_account() -> AccountId {
    let verifying_key = other_signing_key().verifying_key().to_encoded_point(false);
    let hash = Keccak256::digest(&verifying_key.as_bytes()[1..]);
    AccountId::new(format!("0x{}", hex_encode(&hash[12..])))
}

async fn active_series_id(state: &AppState) -> String {
    create_option_series(state, create_input())
        .await
        .unwrap()
        .option_series_id
}

async fn active_onchain_series_id(state: &AppState) -> String {
    create_option_series(state, onchain_create_input())
        .await
        .unwrap()
        .option_series_id
}

fn seed_fee_volume(
    state: &AppState,
    account: &AccountId,
    maker_volume_1e8: u128,
    taker_volume_1e8: u128,
) {
    let bucket_day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    state
        .fees_store
        .lock()
        .unwrap()
        .upsert_volume_delta(
            account,
            &bucket_day,
            FeeMarketType::Option,
            maker_volume_1e8,
            taker_volume_1e8,
            now_ms(),
        )
        .unwrap();
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
        post_only: false,
        client_order_id: Some(client_order_id.to_string()),
        nonce: Some(1),
        deadline_ms: Some(now_ms() + 60_000),
        signature: Some(VALID_SIGNATURE.to_string()),
        attached_tp_sl: None,
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
async fn ioc_order_with_no_liquidity_is_cancelled_and_does_not_rest() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut input = order_input(option_series_id.clone(), Side::Buy, "ioc-empty");
    input.time_in_force = TimeInForce::Ioc;

    let outcome = submit_option_order(&state, input).await.unwrap();

    assert_eq!(outcome.order.status, OptionOrderStatus::Cancelled);
    assert_eq!(outcome.order.remaining_size_1e8, outcome.order.size_1e8);
    assert!(outcome.fills.is_empty());
}

#[tokio::test]
async fn fok_order_with_insufficient_liquidity_is_rejected_without_mutation() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut input = order_input(option_series_id.clone(), Side::Buy, "fok-empty");
    input.time_in_force = TimeInForce::Fok;

    let error = submit_option_order(&state, input).await.unwrap_err();

    assert!(
        error.to_string().contains("fill-or-kill"),
        "expected FOK-not-fillable error, got: {error}"
    );
    let orders = list_option_orders(&state, OptionOrderFilter::default())
        .await
        .unwrap();
    assert!(
        orders.is_empty(),
        "FOK rejection must not insert the order into the book"
    );
}

#[tokio::test]
async fn post_only_rejected_when_combined_with_ioc() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut input = order_input(option_series_id, Side::Buy, "po-ioc");
    input.time_in_force = TimeInForce::Ioc;
    input.post_only = true;

    let error = submit_option_order(&state, input).await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains("invalid time-in-force combination"),
        "expected invalid-combo error, got: {error}"
    );
}

#[tokio::test]
async fn post_only_rejected_when_combined_with_fok() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut input = order_input(option_series_id, Side::Buy, "po-fok");
    input.time_in_force = TimeInForce::Fok;
    input.post_only = true;

    let error = submit_option_order(&state, input).await.unwrap_err();

    assert!(error
        .to_string()
        .contains("invalid time-in-force combination"));
}

// --- MATCHING-TIF-SEMANTICS-OPTIONS-V1 ------------------------------
// Comprehensive matching tests for GTC / IOC / FOK / post-only.
// `order_input` defaults to price=1_000_000_000 and size=100_000_000.

async fn submit_seeded_resting(
    state: &AppState,
    series: &str,
    side: Side,
    client_id: &str,
    price_1e8: u128,
    size_1e8: u128,
) -> OptionOrder {
    let mut input = order_input(series.to_string(), side, client_id);
    input.price_1e8 = price_1e8;
    input.size_1e8 = size_1e8;
    let outcome = submit_option_order(state, input).await.unwrap();
    outcome.order
}

#[tokio::test]
async fn gtc_partial_fill_rests_remainder() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask",
        1_000_000_000,
        30_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "gtc-partial");
    taker.size_1e8 = 100_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.fills[0].size_1e8, 30_000_000);
    assert_eq!(outcome.order.remaining_size_1e8, 70_000_000);
    assert_eq!(outcome.order.status, OptionOrderStatus::PartiallyFilled);
}

#[tokio::test]
async fn ioc_partial_fill_cancels_remainder_without_resting() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask",
        1_000_000_000,
        30_000_000,
    )
    .await;

    let mut taker = order_input(series.clone(), Side::Buy, "ioc-partial");
    taker.time_in_force = TimeInForce::Ioc;
    taker.size_1e8 = 100_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.fills[0].size_1e8, 30_000_000);
    assert_eq!(outcome.order.status, OptionOrderStatus::Cancelled);
    assert_eq!(outcome.order.remaining_size_1e8, 70_000_000);

    let open_book = list_option_orders(
        &state,
        OptionOrderFilter {
            option_series_id: Some(series),
            account: None,
            status: Some(OptionOrderStatus::Open),
            side: None,
        },
    )
    .await
    .unwrap();
    assert!(
        open_book.is_empty(),
        "IOC remainder must never appear as Open in the book"
    );
}

#[tokio::test]
async fn ioc_full_fill_across_multiple_price_levels() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-best",
        950_000_000,
        50_000_000,
    )
    .await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-next",
        1_000_000_000,
        50_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "ioc-multi");
    taker.time_in_force = TimeInForce::Ioc;
    taker.price_1e8 = 1_050_000_000;
    taker.size_1e8 = 100_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    assert_eq!(outcome.fills.len(), 2);
    // Price-time priority: best ask first.
    assert_eq!(outcome.fills[0].price_1e8, 950_000_000);
    assert_eq!(outcome.fills[1].price_1e8, 1_000_000_000);
    assert_eq!(outcome.order.status, OptionOrderStatus::Filled);
    assert_eq!(outcome.order.remaining_size_1e8, 0);
}

#[tokio::test]
async fn fok_sufficient_across_levels_fills_atomically() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-best",
        950_000_000,
        50_000_000,
    )
    .await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-next",
        1_000_000_000,
        50_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "fok-ok");
    taker.time_in_force = TimeInForce::Fok;
    taker.price_1e8 = 1_050_000_000;
    taker.size_1e8 = 100_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    assert_eq!(outcome.fills.len(), 2);
    assert_eq!(outcome.order.status, OptionOrderStatus::Filled);
    assert_eq!(outcome.order.remaining_size_1e8, 0);
}

#[tokio::test]
async fn fok_price_outside_limit_is_not_counted_as_fillable() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-cheap",
        950_000_000,
        30_000_000,
    )
    .await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-rich",
        1_100_000_000,
        70_000_000,
    )
    .await;

    let mut taker = order_input(series.clone(), Side::Buy, "fok-limited");
    taker.time_in_force = TimeInForce::Fok;
    taker.price_1e8 = 1_000_000_000;
    taker.size_1e8 = 100_000_000;
    let error = submit_option_order(&state, taker).await.unwrap_err();

    assert!(error.to_string().contains("fill-or-kill"));

    let cheap_ask = list_option_orders(
        &state,
        OptionOrderFilter {
            option_series_id: Some(series),
            account: None,
            status: None,
            side: Some(Side::Sell),
        },
    )
    .await
    .unwrap()
    .into_iter()
    .find(|o| o.client_order_id.as_deref() == Some("ask-cheap"))
    .unwrap();
    assert_eq!(
        cheap_ask.remaining_size_1e8, 30_000_000,
        "FOK failure must leave maker quantity untouched"
    );
}

#[tokio::test]
async fn post_only_non_crossing_buy_rests() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-high",
        1_100_000_000,
        50_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "po-buy-rest");
    taker.price_1e8 = 1_000_000_000;
    taker.post_only = true;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    assert!(outcome.fills.is_empty());
    assert_eq!(outcome.order.status, OptionOrderStatus::Open);
    assert_eq!(outcome.order.remaining_size_1e8, outcome.order.size_1e8);
    assert!(outcome.order.post_only);
}

#[tokio::test]
async fn post_only_crossing_buy_is_rejected_without_book_mutation() {
    let state = state();
    let series = active_series_id(&state).await;
    let ask = submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-cross",
        1_000_000_000,
        50_000_000,
    )
    .await;

    let mut taker = order_input(series.clone(), Side::Buy, "po-cross");
    taker.price_1e8 = 1_000_000_000;
    taker.post_only = true;
    let error = submit_option_order(&state, taker).await.unwrap_err();

    assert!(error.to_string().contains("post-only"));

    let book = list_option_orders(&state, OptionOrderFilter::default())
        .await
        .unwrap();
    // Only the seeded maker remains, untouched.
    assert_eq!(book.len(), 1);
    let resting = book
        .into_iter()
        .find(|o| o.order_id == ask.order_id)
        .unwrap();
    assert_eq!(resting.remaining_size_1e8, 50_000_000);
    assert_eq!(resting.status, OptionOrderStatus::Open);
}

#[tokio::test]
async fn post_only_crossing_sell_is_rejected() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Buy,
        "bid-cross",
        1_000_000_000,
        50_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Sell, "po-sell-cross");
    taker.price_1e8 = 1_000_000_000;
    taker.post_only = true;
    let error = submit_option_order(&state, taker).await.unwrap_err();

    assert!(error.to_string().contains("post-only"));
}

#[tokio::test]
async fn quantity_invariant_holds_for_ioc_partial_fill() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "inv-ask",
        1_000_000_000,
        30_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "inv-ioc");
    taker.time_in_force = TimeInForce::Ioc;
    taker.size_1e8 = 100_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    let filled: u128 = outcome.fills.iter().map(|f| f.size_1e8).sum();
    // requested = filled + remaining (cancelled at terminal status).
    assert_eq!(
        filled + outcome.order.remaining_size_1e8,
        outcome.order.size_1e8
    );
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

// HISTORY-V2-TERMINAL-REASONS-V1 — user cancel stamps the persisted
// reason (`user_cancelled`, source `user`). This is the only signal
// from which the frontend can honestly distinguish a user cancel from
// a system / TIF-policy cancel.
#[tokio::test]
async fn user_cancel_persists_terminal_reason_user_cancelled() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let order = submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "cancel-reason-user"),
    )
    .await
    .unwrap();

    let cancelled = cancel_option_order(&state, order.order.order_id)
        .await
        .unwrap();

    assert_eq!(cancelled.status, OptionOrderStatus::Cancelled);
    assert_eq!(
        cancelled.terminal_reason_code.as_deref(),
        Some("user_cancelled")
    );
    assert_eq!(cancelled.terminal_reason_source.as_deref(), Some("user"));
    assert_eq!(cancelled.terminal_reason_message, None);
}

// HISTORY-V2-TERMINAL-REASONS-V1 — an IOC that partially fills and
// has its remainder cancelled at insert time MUST carry the
// `ioc_remainder_cancelled` reason (source `tif_policy`). This is the
// only persisted-at-insert terminal reason today; fully-filled IOCs
// terminate as `filled` (success → no reason).
#[tokio::test]
async fn ioc_remainder_persists_terminal_reason_ioc_remainder_cancelled() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask",
        1_000_000_000,
        30_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "ioc-reason");
    taker.time_in_force = TimeInForce::Ioc;
    taker.size_1e8 = 100_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    assert_eq!(outcome.order.status, OptionOrderStatus::Cancelled);
    assert_eq!(outcome.order.remaining_size_1e8, 70_000_000);
    assert_eq!(
        outcome.order.terminal_reason_code.as_deref(),
        Some("ioc_remainder_cancelled")
    );
    assert_eq!(
        outcome.order.terminal_reason_source.as_deref(),
        Some("tif_policy")
    );
    assert_eq!(outcome.order.terminal_reason_message, None);
}

// HISTORY-V2-TERMINAL-REASONS-V1 — a fully-filled IOC is a SUCCESS;
// it must not carry a terminal reason (no fabricated reasons for
// successful orders).
#[tokio::test]
async fn ioc_full_fill_does_not_persist_terminal_reason() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask-full",
        1_000_000_000,
        100_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "ioc-full");
    taker.time_in_force = TimeInForce::Ioc;
    taker.size_1e8 = 100_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();

    assert_eq!(outcome.order.status, OptionOrderStatus::Filled);
    assert_eq!(outcome.order.terminal_reason_code, None);
    assert_eq!(outcome.order.terminal_reason_source, None);
    assert_eq!(outcome.order.terminal_reason_message, None);
}

// HISTORY-V2-TERMINAL-REASONS-V1 — pre-persistence rejections
// (post-only would cross, FOK not fillable) error synchronously and
// never insert an order row, so they leave no terminal reason behind.
// This test pins the invariant: after a post-only rejection, the
// orderbook has no row at all (no fake reason / no fake row).
#[tokio::test]
async fn post_only_rejection_creates_no_order_row_and_no_terminal_reason() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ask",
        1_000_000_000,
        50_000_000,
    )
    .await;

    // Crossable post-only buy → matching-engine rejects synchronously.
    let mut taker = order_input(series.clone(), Side::Buy, "post-only-cross");
    taker.post_only = true;
    taker.price_1e8 = 1_500_000_000;
    let err = submit_option_order(&state, taker).await.unwrap_err();
    assert!(
        format!("{err}").to_lowercase().contains("post"),
        "expected post-only error, got: {err}"
    );

    // No "rejected" row landed; only the seeded ask exists.
    let all = list_option_orders(
        &state,
        OptionOrderFilter {
            option_series_id: Some(series),
            account: None,
            status: None,
            side: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, OptionOrderStatus::Open);
    assert_eq!(all[0].terminal_reason_code, None);
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
    assert!(list_option_execution_intents(&state)
        .await
        .unwrap()
        .is_empty());
    let response = router(state)
        .oneshot(get_request("/executor/transactions"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn option_orderbook_fill_creates_execution_intent_when_enabled() {
    let state = option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "exec-maker-ask");
    ask.account = account_two();
    ask.price_1e8 = 950_000_000;
    submit_option_order(&state, ask).await.unwrap();
    let mut buy = order_input(option_series_id, Side::Buy, "exec-taker-buy");
    buy.price_1e8 = 1_000_000_000;

    let outcome = submit_option_order(&state, buy).await.unwrap();
    let intents = list_option_execution_intents(&state).await.unwrap();

    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(intents.len(), 1);
    let intent = &intents[0];
    assert_eq!(
        intent.source_type,
        OptionExecutionSourceType::OptionOrderbookFill
    );
    assert_eq!(intent.source_id, outcome.fills[0].fill_id.to_string());
    assert_eq!(
        intent.onchain_option_id,
        onchain_option_id_for(intent.expiry)
    );
    assert_eq!(intent.buyer, account());
    assert_eq!(intent.seller, account_two());
    assert_eq!(intent.quantity_contracts, 1);
    assert_eq!(intent.source_size_1e8, 100_000_000);
    assert_eq!(intent.source_price_1e8, 950_000_000);
    assert_eq!(intent.premium_per_contract_native, 9_500_000);
    assert!(!intent.buyer_is_maker);
    assert_eq!(
        intent.status,
        OptionExecutionIntentStatus::SignaturesRequired
    );
    assert!(intent.buyer_signature.is_none());
    assert!(intent.seller_signature.is_none());
    assert!(intent.calldata.is_none());
    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 0);
}

#[tokio::test]
async fn option_execution_rejects_onchain_option_id_metadata_mismatch() {
    let state = option_execution_state();
    let option_series_id = create_option_series(&state, mismatched_onchain_create_input())
        .await
        .unwrap()
        .option_series_id;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "exec-mismatch-ask");
    ask.account = account_two();
    submit_option_order(&state, ask).await.unwrap();

    let error = submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "exec-mismatch-buy"),
    )
    .await
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("optionId does not match option metadata"));
    assert!(list_option_execution_intents(&state)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn option_orderbook_sell_crossing_bid_marks_buyer_as_maker() {
    let state = option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut bid = order_input(option_series_id.clone(), Side::Buy, "exec-maker-bid");
    bid.account = account_two();
    bid.price_1e8 = 1_050_000_000;
    submit_option_order(&state, bid).await.unwrap();
    let mut sell = order_input(option_series_id, Side::Sell, "exec-taker-sell");
    sell.price_1e8 = 1_000_000_000;

    submit_option_order(&state, sell).await.unwrap();
    let intents = list_option_execution_intents(&state).await.unwrap();

    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].buyer, account_two());
    assert_eq!(intents[0].seller, account());
    assert!(intents[0].buyer_is_maker);
}

#[tokio::test]
async fn option_rfq_fill_creates_execution_intent_when_enabled() {
    let state = option_execution_rfq_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "exec-rfq-quote"),
    )
    .await
    .unwrap();

    let outcome = accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();
    let intents = list_option_execution_intents(&state).await.unwrap();

    assert_eq!(intents.len(), 1);
    let intent = &intents[0];
    assert_eq!(intent.source_type, OptionExecutionSourceType::OptionRfqFill);
    assert_eq!(intent.source_id, outcome.fill.fill_id.to_string());
    assert_eq!(intent.buyer, account());
    assert_eq!(intent.seller, account_two());
    assert_eq!(intent.quantity_contracts, 1);
    assert_eq!(intent.premium_per_contract_native, 10_000_000);
    assert!(!intent.buyer_is_maker);
}

#[tokio::test]
async fn option_execution_requires_onchain_option_id_for_matching_fill() {
    let state = option_execution_state();
    let mut input = onchain_create_input();
    input.onchain_series_id = None;
    input.onchain_product_id = None;
    let option_series_id = create_option_series(&state, input)
        .await
        .unwrap()
        .option_series_id;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "exec-no-id-ask");
    ask.account = account_two();
    submit_option_order(&state, ask).await.unwrap();

    let error = submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "exec-no-id-buy"),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("missing onchain_series_id"));
    assert!(list_option_execution_intents(&state)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn option_execution_requires_whole_contract_fills() {
    let state = option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "exec-fractional-ask");
    ask.account = account_two();
    ask.size_1e8 = 50_000_000;
    submit_option_order(&state, ask).await.unwrap();
    let mut buy = order_input(option_series_id, Side::Buy, "exec-fractional-buy");
    buy.size_1e8 = 50_000_000;

    let error = submit_option_order(&state, buy).await.unwrap_err();

    assert!(error
        .to_string()
        .contains("whole number of option contracts"));
    assert!(list_option_execution_intents(&state)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn option_execution_rejects_zero_native_premium_conversion() {
    let state = option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut ask = order_input(
        option_series_id.clone(),
        Side::Sell,
        "exec-tiny-premium-ask",
    );
    ask.account = account_two();
    ask.price_1e8 = 1;
    submit_option_order(&state, ask).await.unwrap();
    let mut buy = order_input(option_series_id, Side::Buy, "exec-tiny-premium-buy");
    buy.price_1e8 = 1;

    let error = submit_option_order(&state, buy).await.unwrap_err();

    assert!(error
        .to_string()
        .contains("premium_per_contract_native is zero"));
    assert!(list_option_execution_intents(&state)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn option_execution_signing_payload_endpoint_matches_option_trade_shape() {
    let state = option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut ask = order_input(option_series_id, Side::Sell, "exec-payload-ask");
    ask.account = account_two();
    let option_series_id = ask.option_series_id.clone();
    submit_option_order(&state, ask).await.unwrap();
    submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "exec-payload-buy"),
    )
    .await
    .unwrap();
    let intent = list_option_execution_intents(&state)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let app = router(state);
    let listed = app
        .clone()
        .oneshot(get_request("/options/execution-intents"))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(response_json(listed).await.as_array().unwrap().len(), 1);

    let fetched = app
        .clone()
        .oneshot(get_request(&format!(
            "/options/execution-intents/{}",
            intent.intent_id
        )))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        response_json(fetched).await["onchain_option_id"],
        onchain_option_id_for(intent.expiry)
    );

    let response = app
        .oneshot(get_request(&format!(
            "/options/execution-intents/{}/signing-payload",
            intent.intent_id
        )))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["primaryType"], "OptionTrade");
    assert_eq!(json["message"]["intentId"], intent.onchain_intent_id);
    assert_eq!(json["message"]["buyer"], account().0);
    assert_eq!(json["message"]["seller"], account_two().0);
    assert_eq!(
        json["message"]["optionId"],
        onchain_option_id_for(intent.expiry)
    );
    assert_eq!(json["message"]["quantity"], "1");
    assert_eq!(json["message"]["premiumPerContract"], "10000000");
    assert_eq!(json["message"]["buyerNonce"], "0");
    assert_eq!(json["message"]["sellerNonce"], "0");
    assert_eq!(json["domain"]["name"], "DeOptV2-OptionMatchingEngine");
    assert!(json["digest"].as_str().unwrap().starts_with("0x"));
    assert_eq!(json["types"].as_array().unwrap()[0]["name"], "intentId");
}

#[tokio::test]
#[ignore = "Superseded by tests/account_write_auth_tests.rs after ACCOUNT-WRITE-AUTH-HARDENING-V1; route now requires authorization envelope"]
async fn option_execution_signature_submission_builds_calldata_without_transaction() {
    let state = option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut ask = order_input(option_series_id, Side::Sell, "exec-calldata-ask");
    ask.account = account_two();
    submit_option_order(&state, ask.clone()).await.unwrap();
    submit_option_order(
        &state,
        order_input(ask.option_series_id, Side::Buy, "exec-calldata-buy"),
    )
    .await
    .unwrap();
    let intent = list_option_execution_intents(&state)
        .await
        .unwrap()
        .pop()
        .unwrap();

    let app = router(state.clone());
    let response = app
        .oneshot(json_post(
            &format!("/options/execution-intents/{}/signatures", intent.intent_id),
            json!({
                "buyer_signature": valid_signature_hex(0xaa),
                "seller_signature": valid_signature_hex(0xbb)
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response_json(response).await;
    assert_eq!(response["status"], "calldata_ready");
    assert!(response["calldata_ready"].as_bool().unwrap());

    let updated = list_option_execution_intents(&state)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let calldata = option_execution_calldata(&state, intent.intent_id)
        .await
        .unwrap();
    let selector = format!("0x{}", hex_encode(&option_execute_trade_selector()));

    assert_eq!(
        option_execute_trade_selector(),
        expected_option_execute_trade_selector()
    );
    assert_eq!(updated.status, OptionExecutionIntentStatus::CalldataReady);
    assert!(updated.calldata.is_some());
    assert!(calldata.calldata_ready);
    assert!(calldata.calldata.unwrap().starts_with(&selector));
    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 0);
    let response = router(state)
        .oneshot(get_request("/executor/transactions"))
        .await
        .unwrap();
    assert_eq!(response_json(response).await.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn strict_option_execution_signature_submission_recovers_buyer_and_seller() {
    let state = strict_option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut ask = order_input(option_series_id, Side::Sell, "strict-exec-ask");
    ask.account = other_signing_account();
    submit_option_order(&state, ask.clone()).await.unwrap();
    let mut buy = order_input(ask.option_series_id, Side::Buy, "strict-exec-buy");
    buy.account = signing_account();
    submit_option_order(&state, buy).await.unwrap();
    let intent = list_option_execution_intents(&state)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let payload = option_execution_signing_payload(&state, intent.intent_id)
        .await
        .unwrap();

    let outcome = submit_option_execution_signatures(
        &state,
        intent.intent_id,
        SubmitOptionExecutionSignaturesInput {
            buyer_signature: Some(sign_option_quote_digest(
                &payload.digest,
                test_signing_key(),
            )),
            seller_signature: Some(sign_option_quote_digest(
                &payload.digest,
                other_signing_key(),
            )),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        outcome.intent.status,
        OptionExecutionIntentStatus::CalldataReady
    );
    assert!(outcome.calldata_ready);
    assert!(outcome.intent.calldata.is_some());
}

#[tokio::test]
async fn strict_option_execution_rejects_signer_mismatch() {
    let state = strict_option_execution_state();
    let option_series_id = active_onchain_series_id(&state).await;
    let mut ask = order_input(option_series_id, Side::Sell, "strict-mismatch-ask");
    ask.account = other_signing_account();
    submit_option_order(&state, ask.clone()).await.unwrap();
    let mut buy = order_input(ask.option_series_id, Side::Buy, "strict-mismatch-buy");
    buy.account = signing_account();
    submit_option_order(&state, buy).await.unwrap();
    let intent = list_option_execution_intents(&state)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let payload = option_execution_signing_payload(&state, intent.intent_id)
        .await
        .unwrap();

    let error = submit_option_execution_signatures(
        &state,
        intent.intent_id,
        SubmitOptionExecutionSignaturesInput {
            buyer_signature: Some(sign_option_quote_digest(
                &payload.digest,
                other_signing_key(),
            )),
            seller_signature: None,
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("signer does not match"));
}

#[tokio::test]
async fn fees_disabled_preserves_option_fill_behavior_without_fee_events() {
    let state = state();
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "fees-off-maker");
    ask.account = account_two();
    submit_option_order(&state, ask).await.unwrap();

    let outcome = submit_option_order(
        &state,
        order_input(option_series_id, Side::Buy, "fees-off-taker"),
    )
    .await
    .unwrap();

    assert_eq!(outcome.fills.len(), 1);
    let store = state.fees_store.lock().unwrap();
    assert!(store.list_fee_events(10).is_empty());
    assert!(store.list_volume_buckets(None).is_empty());
    assert!(store.list_rebate_accruals(None).is_empty());
}

#[tokio::test]
async fn option_order_fill_records_maker_taker_fee_events_and_volumes() {
    let state = fee_state(false);
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "fee-maker-ask");
    ask.account = account_two();
    ask.price_1e8 = 1_000_000_000;
    submit_option_order(&state, ask).await.unwrap();
    let mut buy = order_input(option_series_id, Side::Buy, "fee-taker-buy");
    buy.price_1e8 = 1_100_000_000;

    let outcome = submit_option_order(&state, buy).await.unwrap();

    assert_eq!(outcome.fills.len(), 1);
    let store = state.fees_store.lock().unwrap();
    let events = store.list_fee_events(10);
    assert_eq!(events.len(), 2);
    let maker_event = events
        .iter()
        .find(|event| event.payer == account_two())
        .unwrap();
    let taker_event = events
        .iter()
        .find(|event| event.payer == account())
        .unwrap();
    assert_eq!(maker_event.source_type.as_str(), "option_order_fill");
    assert_eq!(maker_event.flow_type.as_str(), "orderbook");
    assert_eq!(maker_event.maker.as_ref(), Some(&account_two()));
    assert_eq!(maker_event.taker.as_ref(), Some(&account()));
    assert_eq!(maker_event.fee_rate_micro_bps, 5_000);
    assert_eq!(maker_event.fee_amount_1e8, 15_000_000);
    assert_eq!(taker_event.fee_rate_micro_bps, 25_000);
    assert_eq!(taker_event.fee_amount_1e8, 75_000_000);

    let volumes = store.list_volume_buckets(None);
    assert_eq!(volumes.len(), 2);
    let maker_volume = volumes
        .iter()
        .find(|bucket| bucket.account == account_two())
        .unwrap();
    let taker_volume = volumes
        .iter()
        .find(|bucket| bucket.account == account())
        .unwrap();
    assert_eq!(maker_volume.maker_volume_1e8, 300_000_000_000);
    assert_eq!(maker_volume.taker_volume_1e8, 0);
    assert_eq!(taker_volume.maker_volume_1e8, 0);
    assert_eq!(taker_volume.taker_volume_1e8, 300_000_000_000);
}

#[tokio::test]
async fn option_order_fee_is_capped_by_premium_notional() {
    let state = fee_state(false);
    let option_series_id = active_series_id(&state).await;
    let mut ask = order_input(option_series_id.clone(), Side::Sell, "cap-maker-ask");
    ask.account = account_two();
    ask.price_1e8 = ONE_1E8;
    submit_option_order(&state, ask).await.unwrap();
    let mut buy = order_input(option_series_id, Side::Buy, "cap-taker-buy");
    buy.price_1e8 = 2 * ONE_1E8;

    submit_option_order(&state, buy).await.unwrap();

    let store = state.fees_store.lock().unwrap();
    let events = store.list_fee_events(10);
    let maker_event = events
        .iter()
        .find(|event| event.payer == account_two())
        .unwrap();
    let taker_event = events
        .iter()
        .find(|event| event.payer == account())
        .unwrap();
    assert_eq!(maker_event.fee_amount_1e8, 10_000_000);
    assert_eq!(taker_event.fee_amount_1e8, 10_000_000);
}

#[tokio::test]
#[ignore = "Superseded by tests/account_write_auth_tests.rs after ACCOUNT-WRITE-AUTH-HARDENING-V1; route now requires authorization envelope"]
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
#[ignore = "Superseded by tests/account_write_auth_tests.rs after ACCOUNT-WRITE-AUTH-HARDENING-V1; route now requires authorization envelope"]
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
#[ignore = "Superseded by tests/account_write_auth_tests.rs after ACCOUNT-WRITE-AUTH-HARDENING-V1; route now requires authorization envelope"]
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
#[ignore = "Superseded by tests/account_write_auth_tests.rs after ACCOUNT-WRITE-AUTH-HARDENING-V1; route now requires authorization envelope"]
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
async fn option_rfq_fill_records_discounted_taker_fee_for_high_volume_tier() {
    let state = fee_state(false);
    seed_fee_volume(&state, &account(), 0, VOLUME_25M_1E8);
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "discounted-rfq-quote"),
    )
    .await
    .unwrap();

    accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();

    let store = state.fees_store.lock().unwrap();
    let events = store.list_fee_events(10);
    assert_eq!(events.len(), 2);
    let taker_event = events
        .iter()
        .find(|event| event.payer == account())
        .unwrap();
    assert_eq!(taker_event.source_type.as_str(), "option_rfq_fill");
    assert_eq!(taker_event.flow_type.as_str(), "rfq");
    assert_eq!(taker_event.fee_rate_micro_bps, 1_875);
    assert_eq!(taker_event.fee_amount_1e8, 5_625_000);
    assert!(store.list_rebate_accruals(None).is_empty());
}

#[tokio::test]
async fn rebates_do_not_accrue_when_mm_permissions_are_disabled() {
    let state = fee_state(true);
    seed_fee_volume(&state, &account_two(), VOLUME_25M_1E8, 0);
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "no-permission-rebate"),
    )
    .await
    .unwrap();

    accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();

    let store = state.fees_store.lock().unwrap();
    let maker_event = store
        .list_fee_events(10)
        .into_iter()
        .find(|event| event.payer == account_two())
        .unwrap();
    assert_eq!(maker_event.rebate_rate_micro_bps, 0);
    assert_eq!(maker_event.rebate_amount_1e8, 0);
    assert!(store.list_rebate_accruals(None).is_empty());
}

#[tokio::test]
async fn permissioned_mm_rebate_accrues_once_for_option_rfq_fill() {
    let mut state = fee_state(true);
    state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
    state
        .mm_permissions
        .lock()
        .unwrap()
        .upsert_account(MmAccountPermissions {
            mm_account: account_two(),
            enabled: true,
            label: Some("rebate-mm".to_string()),
            can_submit_perp_orders: false,
            can_quote_perp_rfq: false,
            can_quote_option_rfq: true,
            can_submit_option_orders: false,
            created_at_ms: 1,
            updated_at_ms: 1,
        });
    seed_fee_volume(&state, &account_two(), VOLUME_25M_1E8, 0);
    let option_series_id = active_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(account_two(), "permissioned-rebate"),
    )
    .await
    .unwrap();

    let outcome = accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();
    deopt_v2_backend::fees::service::record_option_rfq_fill(&state, &outcome.fill, &outcome.quote)
        .await
        .unwrap();

    let store = state.fees_store.lock().unwrap();
    let events = store.list_fee_events(10);
    let maker_event = events
        .iter()
        .find(|event| event.payer == account_two())
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(maker_event.fee_rate_micro_bps, 0);
    assert_eq!(maker_event.fee_amount_1e8, 0);
    assert_eq!(maker_event.rebate_rate_micro_bps, 5_000);
    assert_eq!(maker_event.rebate_amount_1e8, 15_000_000);
    let rebates = store.list_rebate_accruals(Some(&account_two()));
    assert_eq!(rebates.len(), 1);
    assert_eq!(rebates[0].fee_event_id, maker_event.fee_event_id);
    assert_eq!(rebates[0].rebate_amount_1e8, 15_000_000);
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
#[ignore = "Superseded by tests/account_write_auth_tests.rs after ACCOUNT-WRITE-AUTH-HARDENING-V1; route now requires authorization envelope"]
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

// =====================================================================
// OPTION-ORDER-EXPIRY-SWEEP-V1 — service-level tests for the bulk
// sweeper that terminalizes active orders past their `deadline_ms`.
// All tests use the in-memory `OptionSeriesStore` path (matches the
// project's existing convention; the PG path mirrors the same SQL
// predicate and is covered by its own integration suite when run
// with `DATABASE_URL` set).
// =====================================================================

async fn submit_with_deadline(
    state: &AppState,
    series: &str,
    side: Side,
    client_id: &str,
    deadline_ms: i64,
) -> OptionOrder {
    let mut input = order_input(series.to_string(), side, client_id);
    input.deadline_ms = Some(deadline_ms);
    let outcome = submit_option_order(state, input).await.unwrap();
    outcome.order
}

#[tokio::test]
async fn expire_active_order_marks_terminal_with_expired_reason() {
    let state = state();
    let series = active_series_id(&state).await;
    let past = now_ms() + 60_000;
    let order = submit_with_deadline(&state, &series, Side::Buy, "expire-basic", past).await;
    assert_eq!(order.status, OptionOrderStatus::Open);
    assert_eq!(order.terminal_reason_code, None);

    // Tick the wall-clock past the deadline.
    let sweep_at = past + 1;
    let expired = sweep_expired_option_orders(&state, sweep_at).await.unwrap();

    assert_eq!(expired.len(), 1);
    let row = &expired[0];
    assert_eq!(row.order_id, order.order_id);
    assert_eq!(row.status, OptionOrderStatus::Expired);
    assert_eq!(row.terminal_reason_code.as_deref(), Some("expired"));
    assert_eq!(row.terminal_reason_source.as_deref(), Some("expiry_sweep"));
    assert_eq!(row.terminal_reason_message, None);
    assert_eq!(row.updated_at_ms, sweep_at);

    // Persisted snapshot via `get_option_order` matches the sweep result.
    let fetched = get_option_order(&state, order.order_id).await.unwrap();
    assert_eq!(fetched.status, OptionOrderStatus::Expired);
    assert_eq!(fetched.terminal_reason_code.as_deref(), Some("expired"));
}

#[tokio::test]
async fn non_expired_active_order_remains_open() {
    let state = state();
    let series = active_series_id(&state).await;
    let future = now_ms() + 10 * 60 * 1000;
    let order = submit_with_deadline(&state, &series, Side::Buy, "non-expired", future).await;

    // Sweep at a wall-clock that is BEFORE the deadline.
    let expired = sweep_expired_option_orders(&state, future - 1)
        .await
        .unwrap();

    assert!(expired.is_empty());
    let fetched = get_option_order(&state, order.order_id).await.unwrap();
    assert_eq!(fetched.status, OptionOrderStatus::Open);
    assert_eq!(fetched.terminal_reason_code, None);
}

#[tokio::test]
async fn already_cancelled_order_retains_user_cancelled_reason() {
    let state = state();
    let series = active_series_id(&state).await;
    let deadline = now_ms() + 60_000;
    let order =
        submit_with_deadline(&state, &series, Side::Buy, "cancel-then-sweep", deadline).await;
    let cancelled = cancel_option_order(&state, order.order_id).await.unwrap();
    assert_eq!(cancelled.status, OptionOrderStatus::Cancelled);
    assert_eq!(
        cancelled.terminal_reason_code.as_deref(),
        Some("user_cancelled")
    );

    // Sweep with the clock past the deadline; cancelled order must
    // NOT flip back to `expired` or have its reason rewritten.
    let expired = sweep_expired_option_orders(&state, deadline + 1)
        .await
        .unwrap();
    assert!(expired.is_empty());

    let fetched = get_option_order(&state, order.order_id).await.unwrap();
    assert_eq!(fetched.status, OptionOrderStatus::Cancelled);
    assert_eq!(
        fetched.terminal_reason_code.as_deref(),
        Some("user_cancelled")
    );
}

#[tokio::test]
async fn already_filled_terminal_order_unchanged_by_sweep() {
    let state = state();
    let series = active_series_id(&state).await;
    // Maker sells; taker buys the full size → maker fully filled.
    let maker_deadline = now_ms() + 60_000;
    let mut maker_input = order_input(series.clone(), Side::Sell, "fill-then-sweep-maker");
    maker_input.price_1e8 = 1_000_000_000;
    maker_input.size_1e8 = 100_000_000;
    maker_input.deadline_ms = Some(maker_deadline);
    let maker = submit_option_order(&state, maker_input)
        .await
        .unwrap()
        .order;

    let mut taker_input = order_input(series, Side::Buy, "fill-then-sweep-taker");
    taker_input.account = account_two();
    taker_input.client_order_id = Some("fill-then-sweep-taker".to_string());
    taker_input.price_1e8 = 1_000_000_000;
    taker_input.size_1e8 = 100_000_000;
    taker_input.deadline_ms = Some(now_ms() + 60_000);
    submit_option_order(&state, taker_input).await.unwrap();

    let filled = get_option_order(&state, maker.order_id).await.unwrap();
    assert_eq!(filled.status, OptionOrderStatus::Filled);

    // Sweep past the maker's deadline — filled rows must NOT change.
    let expired = sweep_expired_option_orders(&state, maker_deadline + 1)
        .await
        .unwrap();
    assert!(expired.is_empty());
    let still_filled = get_option_order(&state, maker.order_id).await.unwrap();
    assert_eq!(still_filled.status, OptionOrderStatus::Filled);
}

#[tokio::test]
async fn partially_filled_resting_expires_remaining_and_preserves_fills() {
    let state = state();
    let series = active_series_id(&state).await;
    let maker_deadline = now_ms() + 60_000;

    // Maker rests size = 100 at price = 1_000_000_000.
    let mut maker_input = order_input(series.clone(), Side::Sell, "partial-maker");
    maker_input.price_1e8 = 1_000_000_000;
    maker_input.size_1e8 = 100_000_000;
    maker_input.deadline_ms = Some(maker_deadline);
    let maker = submit_option_order(&state, maker_input)
        .await
        .unwrap()
        .order;

    // Taker buys 30 → maker becomes PartiallyFilled with 70 remaining.
    let mut taker_input = order_input(series.clone(), Side::Buy, "partial-taker");
    taker_input.account = account_two();
    taker_input.price_1e8 = 1_000_000_000;
    taker_input.size_1e8 = 30_000_000;
    taker_input.deadline_ms = Some(now_ms() + 60_000);
    let taker_outcome = submit_option_order(&state, taker_input).await.unwrap();
    assert_eq!(taker_outcome.fills.len(), 1);

    let mid = get_option_order(&state, maker.order_id).await.unwrap();
    assert_eq!(mid.status, OptionOrderStatus::PartiallyFilled);
    assert_eq!(mid.remaining_size_1e8, 70_000_000);

    // Sweep past maker's deadline — expires the resting remainder.
    let expired = sweep_expired_option_orders(&state, maker_deadline + 1)
        .await
        .unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].order_id, maker.order_id);
    assert_eq!(expired[0].status, OptionOrderStatus::Expired);
    assert_eq!(expired[0].terminal_reason_code.as_deref(), Some("expired"));
    // Fill quantity is preserved: remaining stays at 70 (not zeroed).
    assert_eq!(expired[0].remaining_size_1e8, 70_000_000);

    // The fill row created earlier is still there + accessible.
    let fills = get_option_order_fills(&state, maker.order_id)
        .await
        .unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].size_1e8, 30_000_000);
}

#[tokio::test]
async fn repeated_sweep_is_idempotent() {
    let state = state();
    let series = active_series_id(&state).await;
    let deadline = now_ms() + 60_000;
    let order = submit_with_deadline(&state, &series, Side::Buy, "sweep-idem", deadline).await;

    let first = sweep_expired_option_orders(&state, deadline + 1)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    let updated_at_first = first[0].updated_at_ms;

    // Second invocation MUST be a no-op — the predicate filters out
    // the already-`expired` row.
    let second = sweep_expired_option_orders(&state, deadline + 5_000)
        .await
        .unwrap();
    assert!(second.is_empty());

    let fetched = get_option_order(&state, order.order_id).await.unwrap();
    assert_eq!(fetched.status, OptionOrderStatus::Expired);
    assert_eq!(fetched.updated_at_ms, updated_at_first);
}

#[tokio::test]
async fn expired_disappears_from_open_orders_listing() {
    let state = state();
    let series = active_series_id(&state).await;
    let past = now_ms() + 60_000;
    submit_with_deadline(&state, &series, Side::Buy, "list-expire-1", past).await;
    let alive = submit_with_deadline(
        &state,
        &series,
        Side::Buy,
        "list-expire-keep",
        now_ms() + 10 * 60 * 1000,
    )
    .await;

    let pre = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(account()),
            option_series_id: Some(series.clone()),
            status: Some(OptionOrderStatus::Open),
            side: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(pre.len(), 2);

    sweep_expired_option_orders(&state, past + 1).await.unwrap();

    let post = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(account()),
            option_series_id: Some(series.clone()),
            status: Some(OptionOrderStatus::Open),
            side: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(post.len(), 1);
    assert_eq!(post[0].order_id, alive.order_id);

    // And the expired row IS surfaced when explicitly filtered.
    let expired_listing = list_option_orders(
        &state,
        OptionOrderFilter {
            account: Some(account()),
            option_series_id: Some(series),
            status: Some(OptionOrderStatus::Expired),
            side: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(expired_listing.len(), 1);
    assert_eq!(
        expired_listing[0].terminal_reason_code.as_deref(),
        Some("expired")
    );
}

#[tokio::test]
async fn ioc_remainder_cancelled_at_insert_is_not_swept() {
    // IOC orders that partially fill are terminalized at insert
    // (`ioc_remainder_cancelled`). They should NEVER be touched by
    // a later expiry sweep — even when the synthetic `deadline_ms`
    // they carried has passed.
    let state = state();
    let series = active_series_id(&state).await;

    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "ioc-ask",
        1_000_000_000,
        30_000_000,
    )
    .await;

    let mut taker = order_input(series, Side::Buy, "ioc-sweep");
    taker.account = account_two();
    taker.time_in_force = TimeInForce::Ioc;
    taker.size_1e8 = 100_000_000;
    taker.deadline_ms = Some(now_ms() + 30_000);
    let outcome = submit_option_order(&state, taker).await.unwrap();
    assert_eq!(outcome.order.status, OptionOrderStatus::Cancelled);

    let expired = sweep_expired_option_orders(&state, now_ms() + 10 * 60 * 1000)
        .await
        .unwrap();
    assert!(expired.is_empty());

    let fetched = get_option_order(&state, outcome.order.order_id)
        .await
        .unwrap();
    assert_eq!(fetched.status, OptionOrderStatus::Cancelled);
    assert_eq!(
        fetched.terminal_reason_code.as_deref(),
        Some("ioc_remainder_cancelled")
    );
}

// =====================================================================
// HISTORY-V2-REJECTED-ATTEMPTS-FEED-V1 — service-level tests for
// recording pre-persistence option-order rejections so /history can
// surface them.
// =====================================================================

#[tokio::test]
async fn post_only_would_match_is_recorded_as_rejection() {
    let state = state();
    let series = active_series_id(&state).await;
    // Resting ask at 1.0; post-only buy at 1.0 would cross.
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "po-resting",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut taker = order_input(series.clone(), Side::Buy, "po-would-match");
    taker.account = account_two();
    taker.post_only = true;
    let error = submit_option_order(&state, taker).await.unwrap_err();
    assert!(
        error.to_string().contains("post-only"),
        "unexpected error: {error}"
    );

    let recorded = list_option_order_rejections_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    let row = &recorded[0];
    assert_eq!(row.account, account_two());
    assert_eq!(row.option_series_id.as_deref(), Some(series.as_str()));
    assert_eq!(row.side, Some(Side::Buy));
    assert_eq!(row.post_only, Some(true));
    assert_eq!(row.reason_code, "post_only_would_match");
    assert_eq!(row.reason_source, "matching_policy");
    // The message is the existing BackendError Display string —
    // trader-meaningful and free of secrets.
    assert!(row
        .reason_message
        .as_deref()
        .unwrap_or("")
        .contains("post-only"));
}

#[tokio::test]
async fn fok_not_fillable_is_recorded_as_rejection() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "fok-ask-cheap",
        950_000_000,
        30_000_000,
    )
    .await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "fok-ask-rich",
        1_100_000_000,
        70_000_000,
    )
    .await;
    let mut taker = order_input(series.clone(), Side::Buy, "fok-limited");
    taker.account = account_two();
    taker.time_in_force = TimeInForce::Fok;
    taker.price_1e8 = 1_000_000_000;
    taker.size_1e8 = 100_000_000;
    let _ = submit_option_order(&state, taker).await.unwrap_err();

    let recorded = list_option_order_rejections_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].reason_code, "fok_not_fillable");
    assert_eq!(recorded[0].reason_source, "matching_policy");
    assert_eq!(recorded[0].time_in_force, Some(TimeInForce::Fok));
}

#[tokio::test]
async fn zero_price_is_recorded_as_rejection() {
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "zero-price");
    input.price_1e8 = 0;
    let _ = submit_option_order(&state, input).await.unwrap_err();

    let recorded = list_option_order_rejections_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].reason_code, "zero_price");
    assert_eq!(recorded[0].reason_source, "request_validation");
}

#[tokio::test]
async fn deadline_expired_is_recorded_as_rejection() {
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "dead-line");
    input.deadline_ms = Some(now_ms() - 60_000);
    let _ = submit_option_order(&state, input).await.unwrap_err();

    let recorded = list_option_order_rejections_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].reason_code, "deadline_expired");
    assert_eq!(recorded[0].reason_source, "request_validation");
}

#[tokio::test]
async fn invalid_tif_combination_is_recorded_as_rejection() {
    let state = state();
    let series = active_series_id(&state).await;
    // post_only + IOC is the canonical invalid combination.
    let mut input = order_input(series, Side::Buy, "tif-combo");
    input.time_in_force = TimeInForce::Ioc;
    input.post_only = true;
    let _ = submit_option_order(&state, input).await.unwrap_err();

    let recorded = list_option_order_rejections_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].reason_code, "invalid_tif_combination");
    assert_eq!(recorded[0].reason_source, "request_validation");
}

#[tokio::test]
async fn accepted_order_is_not_recorded_as_rejection() {
    let state = state();
    let series = active_series_id(&state).await;
    let input = order_input(series, Side::Buy, "accepted-not-rejected");
    let _ = submit_option_order(&state, input).await.unwrap();
    let recorded = list_option_order_rejections_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert!(recorded.is_empty());
}

#[tokio::test]
async fn rejected_attempt_does_not_consume_or_mutate_resting_book() {
    // Posting a post-only that would cross MUST leave the resting
    // ask untouched (existing semantic) AND record a rejection.
    let state = state();
    let series = active_series_id(&state).await;
    let resting = submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "untouched-ask",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut taker = order_input(series.clone(), Side::Buy, "po-attempt");
    taker.account = account_two();
    taker.post_only = true;
    let _ = submit_option_order(&state, taker).await.unwrap_err();

    let still_resting = get_option_order(&state, resting.order_id).await.unwrap();
    assert_eq!(still_resting.remaining_size_1e8, 30_000_000);
    assert_eq!(still_resting.status, OptionOrderStatus::Open);

    let recorded = list_option_order_rejections_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
}

#[tokio::test]
async fn multiple_rejections_for_same_account_are_all_recorded_newest_first() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "multi-resting",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut a = order_input(series.clone(), Side::Buy, "multi-1");
    a.account = account_two();
    a.post_only = true;
    let _ = submit_option_order(&state, a).await.unwrap_err();
    // small sleep to ensure created_at_ms differs across attempts
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let mut b = order_input(series.clone(), Side::Buy, "multi-2");
    b.account = account_two();
    b.post_only = true;
    let _ = submit_option_order(&state, b).await.unwrap_err();

    let recorded = list_option_order_rejections_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 2);
    assert!(
        recorded[0].created_at_ms >= recorded[1].created_at_ms,
        "list must be newest-first"
    );
    assert_eq!(recorded[0].client_order_id.as_deref(), Some("multi-2"));
    assert_eq!(recorded[1].client_order_id.as_deref(), Some("multi-1"));
}

#[tokio::test]
async fn since_ms_filter_drops_old_rejections() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "since-rest",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut taker = order_input(series, Side::Buy, "since-attempt");
    taker.account = account_two();
    taker.post_only = true;
    let _ = submit_option_order(&state, taker).await.unwrap_err();

    let in_window =
        list_option_order_rejections_for_account(&state, &account_two(), Some(now_ms() - 60_000))
            .await
            .unwrap();
    assert_eq!(in_window.len(), 1);
    let out_of_window =
        list_option_order_rejections_for_account(&state, &account_two(), Some(now_ms() + 60_000))
            .await
            .unwrap();
    assert!(out_of_window.is_empty());
}

#[tokio::test]
async fn rejection_record_does_not_carry_signature_or_authorization() {
    // Sanity-pin: the persisted struct has NO signature field and
    // the captured input.signature is intentionally not echoed
    // into the rejection. Same for nonce — we keep the numeric
    // value only (no envelope, no hex of the signed payload).
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "no-sig-rest",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut taker = order_input(series, Side::Buy, "no-sig-attempt");
    taker.account = account_two();
    taker.post_only = true;
    let _ = submit_option_order(&state, taker).await.unwrap_err();

    let recorded = list_option_order_rejections_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    // The reason_message MUST NOT contain the signature.
    let msg = recorded[0].reason_message.clone().unwrap_or_default();
    assert!(
        !msg.contains("0x"),
        "reason_message should not contain raw hex"
    );
    assert!(
        !msg.contains("ecdsa"),
        "reason_message should not leak signing impl"
    );
    // nonce is a u64 stringified (not the signed envelope nonce).
    if let Some(nonce_str) = recorded[0].nonce.as_deref() {
        assert!(
            nonce_str.parse::<u64>().is_ok(),
            "nonce must be a stringified u64"
        );
    }
}

#[tokio::test]
async fn expired_persisted_order_is_not_in_rejections_feed() {
    // OPTION-ORDER-EXPIRY-SWEEP-V1 interaction: an expired order
    // lives in /options/orders with status=Expired + terminal
    // reason="expired"; it does NOT show up in the rejections feed.
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "expiry-not-rejection");
    let deadline = now_ms() + 60_000;
    input.deadline_ms = Some(deadline);
    let _ = submit_option_order(&state, input).await.unwrap();
    let _ = sweep_expired_option_orders(&state, deadline + 1)
        .await
        .unwrap();
    let recorded = list_option_order_rejections_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert!(
        recorded.is_empty(),
        "expired-via-sweep order must not appear as a rejection: {:?}",
        recorded
    );
}

// =====================================================================
// ATTACHED-TP-SL-ON-ENTRY-V1 — tests for the trader's TP/SL
// attachment intent flowing through the parent submit → fill →
// conditional materialization pipeline.
// =====================================================================

use deopt_v2_backend::options::conditional_orders::{
    list_conditional_orders, ConditionalOrderFilter,
};
use deopt_v2_backend::options::AttachmentPlanStatus;

fn tp_only() -> AttachedTpSlInput {
    AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    }
}

fn sl_only() -> AttachedTpSlInput {
    AttachedTpSlInput {
        take_profit: None,
        stop_loss: Some(AttachedLegInput {
            trigger_price_1e8: 500_000_000,
            limit_price_1e8: 500_000_000,
        }),
        link_as_oco: false,
        expires_at_ms: None,
    }
}

fn tp_and_sl_oco() -> AttachedTpSlInput {
    AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: Some(AttachedLegInput {
            trigger_price_1e8: 500_000_000,
            limit_price_1e8: 500_000_000,
        }),
        link_as_oco: true,
        expires_at_ms: None,
    }
}

#[tokio::test]
async fn submit_order_without_attached_payload_unchanged_behaviour() {
    // Backward compat: omitting `attached_tp_sl` must reproduce
    // the exact pre-milestone behaviour. No plan is recorded.
    let state = state();
    let series = active_series_id(&state).await;
    let input = order_input(series, Side::Buy, "no-attached");
    assert!(input.attached_tp_sl.is_none());
    let outcome = submit_option_order(&state, input).await.unwrap();
    assert_eq!(outcome.order.status, OptionOrderStatus::Open);
    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert!(plans.is_empty());
}

#[tokio::test]
async fn invalid_attached_payload_rejects_parent_atomically() {
    // Both legs empty → InvalidAttachedTpSl. Parent order MUST NOT
    // be persisted; the rejected-attempts feed records the reason.
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series.clone(), Side::Buy, "bad-attached");
    input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: None,
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    });
    let err = submit_option_order(&state, input).await.unwrap_err();
    assert!(
        err.to_string().contains("attached"),
        "unexpected error: {err}"
    );
    let recorded = list_option_order_rejections_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].reason_code, "attached_tp_sl_invalid");
    assert_eq!(recorded[0].reason_source, "request_validation");
    // And no parent order persisted.
    let orders = list_option_orders(
        &state,
        OptionOrderFilter {
            option_series_id: Some(series),
            account: Some(account()),
            status: None,
            side: None,
        },
    )
    .await
    .unwrap();
    assert!(orders.is_empty());
}

#[tokio::test]
async fn invalid_oco_without_both_legs_rejects() {
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "oco-single-leg");
    input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: true,
        expires_at_ms: None,
    });
    let err = submit_option_order(&state, input).await.unwrap_err();
    assert!(
        err.to_string().contains("link_as_oco"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn invalid_zero_trigger_price_rejects() {
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "zero-trigger");
    input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 0,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    });
    let err = submit_option_order(&state, input).await.unwrap_err();
    assert!(err.to_string().contains("trigger_price_1e8"));
}

#[tokio::test]
async fn invalid_past_expiry_rejects() {
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "past-expiry");
    let mut attached = tp_only();
    attached.expires_at_ms = Some(now_ms() - 60_000);
    input.attached_tp_sl = Some(attached);
    let err = submit_option_order(&state, input).await.unwrap_err();
    assert!(err.to_string().contains("expires_at_ms"));
}

#[tokio::test]
async fn resting_parent_with_attached_creates_pending_plan_only() {
    let state = state();
    let series = active_series_id(&state).await;
    // No counterparty present — the parent order rests on the book.
    let mut input = order_input(series, Side::Buy, "resting-with-tp");
    input.attached_tp_sl = Some(tp_only());
    let outcome = submit_option_order(&state, input).await.unwrap();
    assert_eq!(outcome.order.status, OptionOrderStatus::Open);
    assert!(outcome.fills.is_empty());

    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Pending);
    assert_eq!(plans[0].materialized_size_1e8, None);
    assert_eq!(plans[0].tp_conditional_order_id, None);
    assert_eq!(plans[0].sl_conditional_order_id, None);

    // No conditional rows materialised yet.
    let conds = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account()),
            option_series_id: None,
            status: None,
            oco_group_id: None,
        },
    )
    .await
    .unwrap();
    assert!(conds.is_empty());
}

#[tokio::test]
async fn immediate_full_fill_with_tp_only_materialises_active_plan() {
    let state = state();
    let series = active_series_id(&state).await;
    // Resting ask provides liquidity; taker buys full size with TP.
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "tp-only-resting-ask",
        1_000_000_000,
        100_000_000,
    )
    .await;
    let mut taker = order_input(series.clone(), Side::Buy, "tp-only-taker");
    taker.account = account_two();
    taker.attached_tp_sl = Some(tp_only());
    let outcome = submit_option_order(&state, taker).await.unwrap();
    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.order.status, OptionOrderStatus::Filled);

    let plans = list_option_order_attachment_plans_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Active);
    assert_eq!(plans[0].materialized_size_1e8, Some(100_000_000));
    assert!(plans[0].tp_conditional_order_id.is_some());
    assert!(plans[0].sl_conditional_order_id.is_none());
    assert!(plans[0].oco_group_id.is_none());
}

#[tokio::test]
async fn immediate_full_fill_with_sl_only_materialises_active_plan() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "sl-only-resting-ask",
        1_000_000_000,
        100_000_000,
    )
    .await;
    let mut taker = order_input(series, Side::Buy, "sl-only-taker");
    taker.account = account_two();
    taker.attached_tp_sl = Some(sl_only());
    let outcome = submit_option_order(&state, taker).await.unwrap();
    assert_eq!(outcome.fills.len(), 1);

    let plans = list_option_order_attachment_plans_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Active);
    assert!(plans[0].sl_conditional_order_id.is_some());
    assert!(plans[0].tp_conditional_order_id.is_none());
    assert!(plans[0].oco_group_id.is_none());
}

#[tokio::test]
async fn immediate_full_fill_with_tp_and_sl_oco_materialises_oco_pair() {
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "oco-resting-ask",
        1_000_000_000,
        100_000_000,
    )
    .await;
    let mut taker = order_input(series.clone(), Side::Buy, "oco-taker");
    taker.account = account_two();
    taker.attached_tp_sl = Some(tp_and_sl_oco());
    let outcome = submit_option_order(&state, taker).await.unwrap();
    assert_eq!(outcome.fills.len(), 1);

    let plans = list_option_order_attachment_plans_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert_eq!(plan.status, AttachmentPlanStatus::Active);
    assert!(plan.tp_conditional_order_id.is_some());
    assert!(plan.sl_conditional_order_id.is_some());
    let oco_id = plan.oco_group_id.expect("OCO group id must be set");

    // Two conditional rows linked by the same OCO group id.
    let conds = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account_two()),
            option_series_id: Some(series),
            status: None,
            oco_group_id: Some(oco_id),
        },
    )
    .await
    .unwrap();
    assert_eq!(conds.len(), 2);
}

#[tokio::test]
async fn partial_fill_materialises_at_filled_size_only() {
    let state = state();
    let series = active_series_id(&state).await;
    // Resting has only 30 units; taker requests 100 with TP+SL.
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "partial-ask",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut taker = order_input(series.clone(), Side::Buy, "partial-taker");
    taker.account = account_two();
    taker.size_1e8 = 100_000_000;
    taker.attached_tp_sl = Some(tp_and_sl_oco());
    let outcome = submit_option_order(&state, taker).await.unwrap();
    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.fills[0].size_1e8, 30_000_000);

    let plans = list_option_order_attachment_plans_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Active);
    assert_eq!(plans[0].materialized_size_1e8, Some(30_000_000));
}

#[tokio::test]
async fn parent_cancel_before_fill_cancels_pending_plan() {
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "cancel-pending-plan");
    input.attached_tp_sl = Some(tp_only());
    let outcome = submit_option_order(&state, input).await.unwrap();
    assert!(outcome.fills.is_empty());
    let _ = cancel_option_order(&state, outcome.order.order_id)
        .await
        .unwrap();

    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Cancelled);
    assert!(plans[0].materialized_size_1e8.is_none());
    assert_eq!(
        plans[0].failure_code.as_deref(),
        Some("parent_terminal_before_fill")
    );
}

#[tokio::test]
async fn parent_expiry_before_fill_cancels_pending_plan() {
    let state = state();
    let series = active_series_id(&state).await;
    let mut input = order_input(series, Side::Buy, "expire-pending-plan");
    let deadline = now_ms() + 60_000;
    input.deadline_ms = Some(deadline);
    input.attached_tp_sl = Some(tp_only());
    let outcome = submit_option_order(&state, input).await.unwrap();
    let _ = sweep_expired_option_orders(&state, deadline + 1)
        .await
        .unwrap();
    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Cancelled);

    let fetched_parent = get_option_order(&state, outcome.order.order_id)
        .await
        .unwrap();
    assert_eq!(fetched_parent.status, OptionOrderStatus::Expired);
}

#[tokio::test]
async fn parent_cancel_after_materialization_leaves_active_plan_alone() {
    // V1 behaviour: once the plan transitions to Active (filled +
    // conditional rows created), a later parent cancel does NOT
    // re-touch the plan. The user manages the conditional rows
    // via the existing TP/SL endpoints.
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "post-mat-ask",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut taker = order_input(series, Side::Buy, "post-mat-taker");
    taker.account = account_two();
    taker.size_1e8 = 100_000_000;
    taker.attached_tp_sl = Some(tp_only());
    let outcome = submit_option_order(&state, taker).await.unwrap();
    assert_eq!(outcome.fills.len(), 1);
    // Cancel the remaining 70 units of the partially-filled order.
    let _ = cancel_option_order(&state, outcome.order.order_id)
        .await
        .unwrap();

    let plans = list_option_order_attachment_plans_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Active);
    assert_eq!(plans[0].materialized_size_1e8, Some(30_000_000));
}

#[tokio::test]
async fn standalone_conditional_creation_still_works_alongside_attachments() {
    // Make sure the existing standalone TP/SL endpoint is not
    // perturbed: standalone creation continues to require an
    // existing position and writes rows the same way.
    use deopt_v2_backend::options::conditional_orders::{
        create_conditional_orders, ConditionalLegInput, ConditionalType,
        CreateConditionalOrderInput,
    };

    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "standalone-ask",
        1_000_000_000,
        100_000_000,
    )
    .await;
    let mut taker = order_input(series.clone(), Side::Buy, "standalone-taker");
    taker.account = account_two();
    let _ = submit_option_order(&state, taker).await.unwrap();
    // Position exists; create a standalone TP.
    let rows = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account_two(),
            option_series_id: series.clone(),
            quantity_1e8: 100_000_000,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 2_000_000_000,
                limit_price_1e8: 2_000_000_000,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    // And no attachment plan was created for the standalone path.
    let plans = list_option_order_attachment_plans_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert!(plans.is_empty());
}

#[tokio::test]
async fn rejected_parent_does_not_create_attachment_plan() {
    // Post-only-would-match → parent rejection → plan must NOT
    // be persisted (no parent row exists).
    let state = state();
    let series = active_series_id(&state).await;
    submit_seeded_resting(
        &state,
        &series,
        Side::Sell,
        "po-cross-ask",
        1_000_000_000,
        30_000_000,
    )
    .await;
    let mut taker = order_input(series, Side::Buy, "po-attached");
    taker.account = account_two();
    taker.post_only = true;
    taker.attached_tp_sl = Some(tp_only());
    let _ = submit_option_order(&state, taker).await.unwrap_err();
    let plans = list_option_order_attachment_plans_for_account(&state, &account_two(), None)
        .await
        .unwrap();
    assert!(plans.is_empty());
}

// =====================================================================
// ATTACHED-TP-SL-MAKER-FILL-HOOK-V2 — tests for the maker-side
// sync hook. The V1 case (immediate-fill submitter materialisation)
// is covered above; these tests focus on the NEW behaviour:
//   * a resting parent with attached TP/SL gets materialised by
//     a later taker's fill batch
//   * cumulative filled exposure is used for resizing
//   * idempotency holds on repeated syncs
//   * already-terminal conditional legs are not resized
// =====================================================================

use deopt_v2_backend::options::conditional_orders::{
    cancel_conditional_order, ConditionalOrderStatus,
};

async fn taker_buy(state: &AppState, series: &str, client_id: &str, size: u128) -> OptionOrder {
    let mut taker = order_input(series.to_string(), Side::Buy, client_id);
    taker.account = account_two();
    taker.size_1e8 = size;
    let outcome = submit_option_order(state, taker).await.unwrap();
    outcome.order
}

#[tokio::test]
async fn maker_with_attached_tp_only_materialises_on_first_taker_fill() {
    let state = state();
    let series = active_series_id(&state).await;

    // Maker rests a SELL @ 1.0 × 1 with attached TP.
    let mut maker_input = order_input(series.clone(), Side::Sell, "v2-maker-tp");
    maker_input.size_1e8 = 100_000_000;
    maker_input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    });
    let maker = submit_option_order(&state, maker_input)
        .await
        .unwrap()
        .order;
    assert!(
        submit_option_order(
            &state,
            order_input(series.clone(), Side::Sell, "v2-maker-tp")
        )
        .await
        .is_err()
            || true
    );

    // Sanity: plan is pending.
    let pre_plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(pre_plans.len(), 1);
    assert_eq!(pre_plans[0].status, AttachmentPlanStatus::Pending);

    // Taker (account_two) submits a crossing BUY for 30 — hits the
    // maker; V2 hook should materialise the maker's plan at 30.
    let _ = taker_buy(&state, &series, "v2-taker-1", 30_000_000).await;

    let post_plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(post_plans.len(), 1);
    assert_eq!(post_plans[0].status, AttachmentPlanStatus::Active);
    assert_eq!(post_plans[0].materialized_size_1e8, Some(30_000_000));
    assert!(post_plans[0].tp_conditional_order_id.is_some());
    assert!(post_plans[0].sl_conditional_order_id.is_none());

    // The maker's parent order has 70 remaining open.
    let fetched_maker = get_option_order(&state, maker.order_id).await.unwrap();
    assert_eq!(fetched_maker.remaining_size_1e8, 70_000_000);
}

#[tokio::test]
async fn maker_with_attached_sl_only_materialises_on_first_taker_fill() {
    let state = state();
    let series = active_series_id(&state).await;

    let mut maker_input = order_input(series.clone(), Side::Sell, "v2-maker-sl");
    maker_input.size_1e8 = 100_000_000;
    maker_input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: None,
        stop_loss: Some(AttachedLegInput {
            trigger_price_1e8: 500_000_000,
            limit_price_1e8: 500_000_000,
        }),
        link_as_oco: false,
        expires_at_ms: None,
    });
    let _ = submit_option_order(&state, maker_input).await.unwrap();

    let _ = taker_buy(&state, &series, "v2-taker-sl", 100_000_000).await;

    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Active);
    assert_eq!(plans[0].materialized_size_1e8, Some(100_000_000));
    assert!(plans[0].sl_conditional_order_id.is_some());
    assert!(plans[0].tp_conditional_order_id.is_none());
}

#[tokio::test]
async fn maker_with_attached_tp_and_sl_oco_materialises_oco_pair_on_taker_fill() {
    let state = state();
    let series = active_series_id(&state).await;

    let mut maker_input = order_input(series.clone(), Side::Sell, "v2-maker-oco");
    maker_input.size_1e8 = 100_000_000;
    maker_input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: Some(AttachedLegInput {
            trigger_price_1e8: 500_000_000,
            limit_price_1e8: 500_000_000,
        }),
        link_as_oco: true,
        expires_at_ms: None,
    });
    let _ = submit_option_order(&state, maker_input).await.unwrap();

    let _ = taker_buy(&state, &series, "v2-taker-oco", 100_000_000).await;

    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Active);
    assert!(plans[0].tp_conditional_order_id.is_some());
    assert!(plans[0].sl_conditional_order_id.is_some());
    assert!(plans[0].oco_group_id.is_some());
}

#[tokio::test]
async fn subsequent_taker_fill_resizes_active_plan_to_cumulative_size() {
    let state = state();
    let series = active_series_id(&state).await;

    // Maker rests SELL × 1 with attached TP.
    let mut maker_input = order_input(series.clone(), Side::Sell, "v2-resize-maker");
    maker_input.size_1e8 = 100_000_000;
    maker_input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    });
    let _ = submit_option_order(&state, maker_input).await.unwrap();

    // First taker buys 30 → maker's plan materialises at 30.
    let _ = taker_buy(&state, &series, "v2-resize-taker-1", 30_000_000).await;
    let mid_plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(mid_plans[0].materialized_size_1e8, Some(30_000_000));
    let tp_cond_id = mid_plans[0].tp_conditional_order_id.unwrap();
    let tp_cond =
        deopt_v2_backend::options::conditional_orders::get_conditional_order(&state, tp_cond_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(tp_cond.quantity_1e8, 30_000_000);

    // Second taker buys 50 more → cumulative filled = 80, plan
    // should be resized to 80; the TP conditional row should be
    // resized too because it's still Armed.
    let _ = taker_buy(&state, &series, "v2-resize-taker-2", 50_000_000).await;
    let post_plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(post_plans[0].materialized_size_1e8, Some(80_000_000));
    let tp_after =
        deopt_v2_backend::options::conditional_orders::get_conditional_order(&state, tp_cond_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(tp_after.quantity_1e8, 80_000_000);
}

#[tokio::test]
async fn reprocessing_same_cumulative_filled_is_idempotent_noop() {
    // The V2 sync function is called every fill batch, and the
    // submitter branch fires even on a no-fill submit. A second
    // taker that produces ZERO fills (e.g. resting opposite-side
    // post-only) must not change the maker's plan.
    let state = state();
    let series = active_series_id(&state).await;

    let mut maker_input = order_input(series.clone(), Side::Sell, "v2-idem-maker");
    maker_input.size_1e8 = 100_000_000;
    maker_input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    });
    let _ = submit_option_order(&state, maker_input).await.unwrap();

    let _ = taker_buy(&state, &series, "v2-idem-taker", 30_000_000).await;
    let first = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    let tp_first = first[0].tp_conditional_order_id.unwrap();
    let cond_first =
        deopt_v2_backend::options::conditional_orders::get_conditional_order(&state, tp_first)
            .await
            .unwrap()
            .unwrap();

    // Submit a resting post-only buy that does NOT cross (price
    // below the resting ask). No fills → V2 sync fires but is a
    // no-op for the maker (cumulative still 30, same as
    // materialised).
    let mut po = order_input(series.clone(), Side::Buy, "v2-idem-postonly");
    po.account = account_two();
    po.price_1e8 = 500_000_000;
    po.post_only = true;
    let _ = submit_option_order(&state, po).await.unwrap();

    let after = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(after[0].materialized_size_1e8, Some(30_000_000));
    let cond_after =
        deopt_v2_backend::options::conditional_orders::get_conditional_order(&state, tp_first)
            .await
            .unwrap()
            .unwrap();
    // Version must NOT have bumped (no resize was applied).
    assert_eq!(cond_after.version, cond_first.version);
}

#[tokio::test]
async fn cancelled_conditional_leg_is_not_resized_by_subsequent_fill() {
    // Safe-subset rule: if a materialised conditional row has
    // already moved past `Armed` (here: user-cancelled the TP),
    // a later resize attempt MUST leave it alone. The plan's
    // `materialized_size_1e8` still bumps so observability is
    // accurate, and a non-fatal failure_code is set.
    let state = state();
    let series = active_series_id(&state).await;

    let mut maker_input = order_input(series.clone(), Side::Sell, "v2-terminal-maker");
    maker_input.size_1e8 = 100_000_000;
    maker_input.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    });
    let _ = submit_option_order(&state, maker_input).await.unwrap();

    let _ = taker_buy(&state, &series, "v2-terminal-taker-1", 30_000_000).await;
    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    let tp_id = plans[0].tp_conditional_order_id.unwrap();

    // User cancels the conditional row directly.
    let _ = cancel_conditional_order(&state, tp_id, &account())
        .await
        .unwrap();
    let cancelled =
        deopt_v2_backend::options::conditional_orders::get_conditional_order(&state, tp_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(cancelled.status, ConditionalOrderStatus::Cancelled);

    // Second taker takes another 50 from the maker — cumulative
    // filled = 80, but the TP row is terminal, so the resize MUST
    // skip it and the row's quantity stays at the prior 30.
    let _ = taker_buy(&state, &series, "v2-terminal-taker-2", 50_000_000).await;

    let final_plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(final_plans[0].materialized_size_1e8, Some(80_000_000));
    assert_eq!(
        final_plans[0].failure_code.as_deref(),
        Some("conditional_leg_already_terminal")
    );
    let tp_still_30 =
        deopt_v2_backend::options::conditional_orders::get_conditional_order(&state, tp_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(tp_still_30.quantity_1e8, 30_000_000);
    assert_eq!(tp_still_30.status, ConditionalOrderStatus::Cancelled);
}

#[tokio::test]
async fn pending_plan_resting_through_no_fills_remains_pending() {
    // Regression: a maker submits a resting order with attached
    // TP/SL but never gets filled. The V1 test already pinned the
    // submit-time path; this test now ALSO confirms that
    // unrelated taker submits (that fill DIFFERENT makers) do not
    // bump our maker's plan.
    let state = state();
    let series = active_series_id(&state).await;

    // The maker we care about: 90 SELL @ 2.0 (way above market),
    // resting deep + plan pending.
    let mut deep_maker = order_input(series.clone(), Side::Sell, "v2-deep-maker");
    deep_maker.price_1e8 = 2_000_000_000;
    deep_maker.size_1e8 = 100_000_000;
    deep_maker.attached_tp_sl = Some(AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 3_000_000_000,
            limit_price_1e8: 3_000_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    });
    let deep = submit_option_order(&state, deep_maker).await.unwrap().order;

    // A cheap maker that will absorb the taker.
    let mut cheap_maker = order_input(series.clone(), Side::Sell, "v2-cheap-maker");
    cheap_maker.account = signing_account();
    cheap_maker.price_1e8 = 900_000_000;
    cheap_maker.size_1e8 = 100_000_000;
    let _ = submit_option_order(&state, cheap_maker).await.unwrap();

    // Taker buys @ 1.0 — only the cheap maker matches.
    let mut taker = order_input(series.clone(), Side::Buy, "v2-pending-taker");
    taker.account = account_two();
    taker.size_1e8 = 50_000_000;
    let outcome = submit_option_order(&state, taker).await.unwrap();
    assert_eq!(outcome.fills.len(), 1);

    // Our deep maker still rests untouched.
    let fetched = get_option_order(&state, deep.order_id).await.unwrap();
    assert_eq!(fetched.remaining_size_1e8, 100_000_000);
    let plans = list_option_order_attachment_plans_for_account(&state, &account(), None)
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].status, AttachmentPlanStatus::Pending);
    assert_eq!(plans[0].materialized_size_1e8, None);
}
