use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::{EngineEvent, EngineState};
use deopt_v2_backend::types::{AccountId, NewOrder, OrderId, OrderStatus, Side, TimeInForce};
use tower::ServiceExt;

fn new_order(
    market_id: u64,
    account: &str,
    side: Side,
    price_1e8: u128,
    size_1e8: u128,
    time_in_force: TimeInForce,
) -> NewOrder {
    NewOrder {
        market_id,
        account: AccountId::new(account),
        side,
        price_1e8,
        size_1e8,
        time_in_force,
        reduce_only: false,
        post_only: false,
        client_order_id: None,
    }
}

fn first_order_id(events: &[EngineEvent]) -> OrderId {
    events
        .iter()
        .find_map(|event| match event {
            EngineEvent::OrderAccepted { order } => Some(order.order_id),
            _ => None,
        })
        .unwrap()
}

#[test]
fn submit_order_creates_accepted_event() {
    let mut engine = EngineState::with_default_markets();
    let events = engine
        .submit_order(new_order(1, "maker", Side::Sell, 100, 10, TimeInForce::Gtc))
        .unwrap();

    assert!(matches!(events[0], EngineEvent::OrderAccepted { .. }));
}

#[test]
fn matched_orders_create_trade_event() {
    let mut engine = EngineState::with_default_markets();
    engine
        .submit_order(new_order(1, "maker", Side::Sell, 100, 10, TimeInForce::Gtc))
        .unwrap();
    let events = engine
        .submit_order(new_order(1, "taker", Side::Buy, 100, 10, TimeInForce::Gtc))
        .unwrap();

    assert!(events
        .iter()
        .any(|event| matches!(event, EngineEvent::TradeMatched { .. })));
}

#[test]
fn matched_orders_create_execution_intent() {
    let mut engine = EngineState::with_default_markets();
    engine
        .submit_order(new_order(1, "maker", Side::Sell, 100, 10, TimeInForce::Gtc))
        .unwrap();
    let events = engine
        .submit_order(new_order(1, "taker", Side::Buy, 100, 10, TimeInForce::Gtc))
        .unwrap();

    assert!(events
        .iter()
        .any(|event| matches!(event, EngineEvent::ExecutionIntentCreated { .. })));
    assert_eq!(engine.execution_intents().len(), 1);
}

#[test]
fn cancelled_order_cannot_be_matched_later() {
    let mut engine = EngineState::with_default_markets();
    let accepted = engine
        .submit_order(new_order(1, "maker", Side::Sell, 100, 10, TimeInForce::Gtc))
        .unwrap();
    let order_id = first_order_id(&accepted);

    engine.cancel_order(order_id).unwrap();
    let events = engine
        .submit_order(new_order(1, "taker", Side::Buy, 100, 10, TimeInForce::Gtc))
        .unwrap();

    assert!(!events
        .iter()
        .any(|event| matches!(event, EngineEvent::TradeMatched { .. })));
    assert_eq!(engine.execution_intents().len(), 0);
}

#[test]
fn multiple_markets_stay_isolated() {
    let mut engine = EngineState::with_default_markets();
    engine
        .submit_order(new_order(1, "maker", Side::Sell, 100, 10, TimeInForce::Gtc))
        .unwrap();
    let events = engine
        .submit_order(new_order(2, "taker", Side::Buy, 100, 10, TimeInForce::Gtc))
        .unwrap();

    assert!(!events
        .iter()
        .any(|event| matches!(event, EngineEvent::TradeMatched { .. })));
    assert_eq!(engine.orderbook_snapshot(1).asks.len(), 1);
    assert_eq!(engine.orderbook_snapshot(2).bids.len(), 1);
}

#[test]
fn filled_order_status_is_emitted() {
    let mut engine = EngineState::with_default_markets();
    engine
        .submit_order(new_order(1, "maker", Side::Sell, 100, 10, TimeInForce::Gtc))
        .unwrap();
    let events = engine
        .submit_order(new_order(1, "taker", Side::Buy, 100, 10, TimeInForce::Gtc))
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event,
            EngineEvent::OrderFilled {
                order
            } if order.status == OrderStatus::Filled
        )
    }));
}

#[tokio::test]
async fn fixed_point_fields_are_serialized_as_strings_in_orderbook_api() {
    let state = AppState::new(EngineState::with_default_markets());
    {
        let mut engine = state.engine.lock().unwrap();
        engine
            .submit_order(new_order(
                1,
                "maker",
                Side::Sell,
                300_000_000_000,
                100_000_000,
                TimeInForce::Gtc,
            ))
            .unwrap();
    }
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/orderbook/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["asks"][0]["price1e8"], "300000000000");
    assert_eq!(json["asks"][0]["totalSize1e8"], "100000000");
}

#[tokio::test]
async fn post_orders_accepts_string_price_and_size() {
    let app = router(AppState::new(EngineState::with_default_markets()));
    let response = app
        .oneshot(json_post(
            "/orders",
            r#"{
                "market_id": 1,
                "account": "0xmaker",
                "side": "sell",
                "price_1e8": "300000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "maker-1"
            }"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["status"], "accepted");
    assert_eq!(json["events"][0]["order"]["price_1e8"], "300000000000");
    assert_eq!(json["events"][0]["order"]["size_1e8"], "100000000");
}

#[tokio::test]
async fn post_orders_rejects_non_numeric_price_string() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/orders",
            r#"{
                "market_id": 1,
                "account": "0xmaker",
                "side": "sell",
                "price_1e8": "not-a-number",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "maker-1"
            }"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_orders_rejects_non_numeric_size_string() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/orders",
            r#"{
                "market_id": 1,
                "account": "0xmaker",
                "side": "sell",
                "price_1e8": "300000000000",
                "size_1e8": "not-a-number",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "maker-1"
            }"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_orders_rejects_negative_string_values() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/orders",
            r#"{
                "market_id": 1,
                "account": "0xmaker",
                "side": "sell",
                "price_1e8": "-300000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "maker-1"
            }"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_orders_rejects_empty_string_values() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/orders",
            r#"{
                "market_id": 1,
                "account": "0xmaker",
                "side": "sell",
                "price_1e8": "",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "maker-1"
            }"#,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn matched_order_response_serializes_financial_quantities_as_strings() {
    let app = router(AppState::new(EngineState::with_default_markets()));
    let maker_response = app
        .clone()
        .oneshot(json_post(
            "/orders",
            r#"{
                "market_id": 1,
                "account": "0xmaker",
                "side": "sell",
                "price_1e8": "300000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "maker-1"
            }"#,
        ))
        .await
        .unwrap();
    assert_eq!(maker_response.status(), StatusCode::OK);

    let taker_response = app
        .oneshot(json_post(
            "/orders",
            r#"{
                "market_id": 1,
                "account": "0xtaker",
                "side": "buy",
                "price_1e8": "300000000000",
                "size_1e8": "50000000",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": "taker-1"
            }"#,
        ))
        .await
        .unwrap();

    assert_eq!(taker_response.status(), StatusCode::OK);
    let json = response_json(taker_response).await;
    let trade = json["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "trade_matched")
        .unwrap();
    assert_eq!(trade["trade"]["price_1e8"], "300000000000");
    assert_eq!(trade["trade"]["size_1e8"], "50000000");

    let intent = json["execution_intents"]
        .as_array()
        .unwrap()
        .first()
        .unwrap();
    assert_eq!(intent["price_1e8"], "300000000000");
    assert_eq!(intent["size_1e8"], "50000000");
}

#[tokio::test]
async fn execution_intents_api_serializes_financial_quantities_as_strings() {
    let state = AppState::new(EngineState::with_default_markets());
    {
        let mut engine = state.engine.lock().unwrap();
        engine
            .submit_order(new_order(
                1,
                "maker",
                Side::Sell,
                300_000_000_000,
                100_000_000,
                TimeInForce::Gtc,
            ))
            .unwrap();
        engine
            .submit_order(new_order(
                1,
                "taker",
                Side::Buy,
                300_000_000_000,
                50_000_000,
                TimeInForce::Gtc,
            ))
            .unwrap();
    }

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/execution-intents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json[0]["price_1e8"], "300000000000");
    assert_eq!(json[0]["size_1e8"], "50000000");
}

fn json_post(uri: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}
