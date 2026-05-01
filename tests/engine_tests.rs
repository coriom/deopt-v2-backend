use axum::body::Body;
use axum::http::{Request, StatusCode};
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
