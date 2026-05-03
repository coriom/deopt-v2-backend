use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::{EngineEvent, EngineState};
use deopt_v2_backend::execution::{intent_id_to_hex_bytes32, ExecutionConfig};
use deopt_v2_backend::indexer::IndexerConfig;
use deopt_v2_backend::signing::{Eip712Domain, SignatureVerificationMode, SignedOrder};
use deopt_v2_backend::types::now_ms;
use deopt_v2_backend::types::{AccountId, NewOrder, OrderId, OrderStatus, Side, TimeInForce};
use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};
use tower::ServiceExt;

const VALID_SIGNATURE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TEST_ONLY_PRIVATE_KEY_HEX: &str =
    "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

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
        signed_nonce: None,
        signed_deadline_ms: None,
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
            signed_order_body("0xmaker", "sell", "300000000000", "100000000", 1),
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
            signed_order_body("0xmaker", "sell", "not-a-number", "100000000", 1),
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
            signed_order_body("0xmaker", "sell", "300000000000", "not-a-number", 1),
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
            signed_order_body("0xmaker", "sell", "-300000000000", "100000000", 1),
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
            signed_order_body("0xmaker", "sell", "", "100000000", 1),
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
            signed_order_body("0xmaker", "sell", "300000000000", "100000000", 1),
        ))
        .await
        .unwrap();
    assert_eq!(maker_response.status(), StatusCode::OK);

    let taker_response = app
        .oneshot(json_post(
            "/orders",
            signed_order_body("0xtaker", "buy", "300000000000", "50000000", 1),
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

#[tokio::test]
async fn post_orders_rejects_expired_deadline() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/orders",
            signed_order_body_with_deadline(
                "0xmaker",
                "sell",
                "300000000000",
                "100000000",
                1,
                now_ms() - 1,
                VALID_SIGNATURE,
            ),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_orders_rejects_reused_nonce_for_same_account() {
    let app = router(AppState::new(EngineState::with_default_markets()));
    let first = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body("0xmaker", "sell", "300000000000", "100000000", 7),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(json_post(
            "/orders",
            signed_order_body("0xmaker", "sell", "300100000000", "100000000", 7),
        ))
        .await
        .unwrap();

    assert_eq!(second.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_orders_rejects_zero_nonce() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/orders",
            signed_order_body("0xmaker", "sell", "300000000000", "100000000", 0),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_orders_allows_same_nonce_for_different_accounts() {
    let app = router(AppState::new(EngineState::with_default_markets()));
    let first = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body("0xmaker-a", "sell", "300000000000", "100000000", 11),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(json_post(
            "/orders",
            signed_order_body("0xmaker-b", "sell", "300100000000", "100000000", 11),
        ))
        .await
        .unwrap();

    assert_eq!(second.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_orders_rejects_malformed_signature() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/orders",
            signed_order_body_with_deadline(
                "0xmaker",
                "sell",
                "300000000000",
                "100000000",
                1,
                future_deadline(),
                "not-a-signature",
            ),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn strict_signature_mode_accepts_valid_signed_order() {
    let app = router(AppState::with_signature_mode(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
    ));
    let fields = valid_strict_fields(1);

    let response = app
        .oneshot(json_post("/orders", strict_signed_order_body(&fields)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["status"], "accepted");
}

#[tokio::test]
async fn strict_signature_mode_rejects_signer_account_mismatch() {
    let app = router(AppState::with_signature_mode(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
    ));
    let mut fields = valid_strict_fields(2);
    let signature = strict_signature(&fields);
    fields.account = "0x0000000000000000000000000000000000000001".to_string();

    let response = app
        .oneshot(json_post(
            "/orders",
            strict_signed_order_body_with_signature(&fields, &signature),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn strict_signature_mode_rejects_malformed_account_address() {
    let app = router(AppState::with_signature_mode(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
    ));
    let mut fields = valid_strict_fields(3);
    fields.account = "0xmaker".to_string();

    let response = app
        .oneshot(json_post(
            "/orders",
            strict_signed_order_body_with_signature(&fields, VALID_SIGNATURE),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn strict_signature_mode_rejects_malformed_signature() {
    let app = router(AppState::with_signature_mode(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
    ));
    let fields = valid_strict_fields(4);

    let response = app
        .oneshot(json_post(
            "/orders",
            strict_signed_order_body_with_signature(&fields, "not-a-signature"),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn strict_signature_mode_rejects_tampered_price_after_signing() {
    let app = router(AppState::with_signature_mode(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
    ));
    let mut fields = valid_strict_fields(5);
    let signature = strict_signature(&fields);
    fields.price_1e8 = "300100000000".to_string();

    let response = app
        .oneshot(json_post(
            "/orders",
            strict_signed_order_body_with_signature(&fields, &signature),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn strict_signature_mode_rejects_tampered_nonce_after_signing() {
    let app = router(AppState::with_signature_mode(
        EngineState::with_default_markets(),
        SignatureVerificationMode::Strict,
    ));
    let mut fields = valid_strict_fields(6);
    let signature = strict_signature(&fields);
    fields.nonce = 7;

    let response = app
        .oneshot(json_post(
            "/orders",
            strict_signed_order_body_with_signature(&fields, &signature),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn executor_status_api_reports_scaffold_flags() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(
            Request::builder()
                .uri("/executor/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["executionEnabled"], false);
    assert_eq!(json["dryRun"], true);
    assert_eq!(json["realBroadcastEnabled"], false);
    assert_eq!(json["persistenceRequired"], true);
    assert_eq!(json["simulationEnabled"], false);
    assert_eq!(json["simulationRequiresPersistence"], true);
    assert_eq!(json["rpcConfigured"], false);
    assert_eq!(json["broadcastEnabled"], false);
}

#[tokio::test]
async fn indexer_status_api_reports_v1_flags() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(
            Request::builder()
                .uri("/indexer/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["indexerEnabled"], false);
    assert_eq!(json["rpcConfigured"], false);
    assert_eq!(json["persistenceRequired"], true);
    assert_eq!(json["lastIndexedBlock"], 0);
    assert_eq!(
        json["targetContract"],
        "0x0000000000000000000000000000000000000000"
    );
}

#[tokio::test]
async fn reconciliation_status_api_reports_confirmed_zero() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(
            Request::builder()
                .uri("/reconciliation/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["reconciliationEnabled"], false);
    assert_eq!(json["persistenceRequired"], true);
    assert_eq!(json["matchedReconciliations"], 0);
    assert_eq!(json["ambiguousReconciliations"], 0);
    assert_eq!(json["unmatchedReconciliations"], 0);
    assert_eq!(json["confirmed"], 0);
}

#[tokio::test]
async fn reconciliation_tick_rejects_when_disabled() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reconciliation/tick")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn indexer_default_does_not_claim_confirmation_lifecycle() {
    let status = IndexerConfig::disabled().status(0);

    assert!(!status.indexer_enabled);
    assert_eq!(status.last_indexed_block, 0);
}

#[tokio::test]
async fn simulate_endpoint_rejects_when_simulation_disabled() {
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            "/executor/simulate/00000000-0000-0000-0000-000000000001",
            "{}".to_string(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("simulation is disabled"));
}

#[tokio::test]
async fn simulate_endpoint_rejects_missing_signatures_before_rpc() {
    let app = router(
        AppState::with_signature_mode_domain_repository_and_execution_config(
            EngineState::with_default_markets(),
            SignatureVerificationMode::Disabled,
            Eip712Domain::default(),
            None,
            simulation_config_without_persistence_requirement(),
            84532,
        ),
    );
    let maker = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body(
                "0x0000000000000000000000000000000000000001",
                "sell",
                "300000000000",
                "100000000",
                41,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(maker.status(), StatusCode::OK);
    let taker = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body(
                "0x0000000000000000000000000000000000000002",
                "buy",
                "300000000000",
                "50000000",
                42,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(taker.status(), StatusCode::OK);
    let json = response_json(taker).await;
    let intent_id = json["execution_intents"][0]["intent_id"].as_str().unwrap();

    let response = app
        .oneshot(json_post(
            &format!("/executor/simulate/{intent_id}"),
            "{}".to_string(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("trade signatures are required"));
}

#[test]
fn executor_status_cannot_report_broadcast_enabled() {
    assert!(!ExecutionConfig::disabled().status().broadcast_enabled);
    assert!(!ExecutionConfig::disabled().status().real_broadcast_enabled);
}

#[tokio::test]
async fn broadcast_endpoint_rejects_when_disabled_without_fake_hash() {
    let intent_id = "00000000-0000-0000-0000-000000000001";
    let response = router(AppState::new(EngineState::with_default_markets()))
        .oneshot(json_post(
            &format!("/executor/broadcast/{intent_id}"),
            "{}".to_string(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["intent_id"], intent_id);
    assert_eq!(json["broadcast_enabled"], false);
    assert_eq!(json["submitted"], false);
    assert_eq!(json["confirmed"], false);
    assert_eq!(json["tx_hash"], serde_json::Value::Null);
    assert_eq!(json["reason"], "broadcast disabled");
}

#[tokio::test]
async fn executor_transactions_return_empty_without_persistence() {
    let app = router(AppState::new(EngineState::with_default_markets()));
    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/executor/transactions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let by_intent = app
        .oneshot(
            Request::builder()
                .uri("/executor/transactions/00000000-0000-0000-0000-000000000001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(list.status(), StatusCode::OK);
    assert_eq!(response_json(list).await, serde_json::json!([]));
    assert_eq!(by_intent.status(), StatusCode::OK);
    assert_eq!(response_json(by_intent).await, serde_json::json!([]));
}

#[tokio::test]
async fn signing_payload_endpoint_returns_perp_trade_fields() {
    let app = router(AppState::new(EngineState::with_default_markets()));
    let maker = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body(
                "0x0000000000000000000000000000000000000001",
                "sell",
                "300000000000",
                "100000000",
                21,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(maker.status(), StatusCode::OK);
    let taker = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body(
                "0x0000000000000000000000000000000000000002",
                "buy",
                "300000000000",
                "50000000",
                22,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(taker.status(), StatusCode::OK);
    let taker_json = response_json(taker).await;
    let intent_id = taker_json["execution_intents"][0]["intent_id"]
        .as_str()
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/execution-intents/{intent_id}/signing-payload"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["primary_type"], "PerpTrade");
    assert_eq!(json["domain"]["name"], "DeOptV2-PerpMatchingEngine");
    assert_eq!(json["domain"]["version"], "1");
    assert_eq!(json["types"][0]["name"], "intentId");
    assert_eq!(json["types"][0]["type"], "bytes32");
    assert_eq!(
        json["message"]["intentId"],
        intent_id_to_hex_bytes32(intent_id).unwrap()
    );
    assert_eq!(
        json["message"]["buyer"],
        "0x0000000000000000000000000000000000000002"
    );
    assert_eq!(
        json["message"]["seller"],
        "0x0000000000000000000000000000000000000001"
    );
    assert_eq!(json["message"]["marketId"], "1");
    assert_eq!(json["message"]["sizeDelta1e8"], "50000000");
    assert_eq!(json["message"]["executionPrice1e8"], "300000000000");
    assert_eq!(json["message"]["buyerIsMaker"], false);
    assert_eq!(json["message"]["buyerNonce"], "22");
    assert_eq!(json["message"]["sellerNonce"], "21");
    assert!(json["digest"].as_str().unwrap().starts_with("0x"));
}

#[tokio::test]
async fn signing_payload_missing_nonce_metadata_returns_clear_error() {
    let state = AppState::new(EngineState::with_default_markets());
    let intent_id = {
        let mut engine = state.engine.lock().unwrap();
        engine
            .submit_order(new_order(
                1,
                "0x0000000000000000000000000000000000000001",
                Side::Sell,
                100,
                10,
                TimeInForce::Gtc,
            ))
            .unwrap();
        engine
            .submit_order(new_order(
                1,
                "0x0000000000000000000000000000000000000002",
                Side::Buy,
                100,
                10,
                TimeInForce::Gtc,
            ))
            .unwrap();
        engine.execution_intents()[0].intent_id
    };

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!("/execution-intents/{intent_id}/signing-payload"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("missing PerpTrade metadata"));
}

#[tokio::test]
async fn signature_endpoint_rejects_malformed_buyer_sig() {
    let (app, intent_id) = app_with_signed_match().await;

    let response = app
        .oneshot(json_post(
            &format!("/execution-intents/{intent_id}/signatures"),
            format!(
                r#"{{
                    "buyer_sig": "0x1234",
                    "seller_sig": "{}"
                }}"#,
                trade_signature(0xbb)
            ),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn signature_endpoint_rejects_malformed_seller_sig() {
    let (app, intent_id) = app_with_signed_match().await;

    let response = app
        .oneshot(json_post(
            &format!("/execution-intents/{intent_id}/signatures"),
            format!(
                r#"{{
                    "buyer_sig": "{}",
                    "seller_sig": "not-a-signature"
                }}"#,
                trade_signature(0xaa)
            ),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn submitting_both_trade_signatures_marks_calldata_ready() {
    let (app, intent_id) = app_with_signed_match().await;

    let response = app
        .oneshot(json_post(
            &format!("/execution-intents/{intent_id}/signatures"),
            format!(
                r#"{{
                    "buyer_sig": "{}",
                    "seller_sig": "{}"
                }}"#,
                trade_signature(0xaa),
                trade_signature(0xbb)
            ),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["buyer_signature_present"], true);
    assert_eq!(json["seller_signature_present"], true);
    assert_eq!(json["calldata_ready"], true);
    assert_eq!(json["missing_signatures"], false);
}

fn json_post(uri: &str, body: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

async fn app_with_signed_match() -> (axum::Router, String) {
    let app = router(AppState::new(EngineState::with_default_markets()));
    let maker = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body(
                "0x0000000000000000000000000000000000000001",
                "sell",
                "300000000000",
                "100000000",
                31,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(maker.status(), StatusCode::OK);
    let taker = app
        .clone()
        .oneshot(json_post(
            "/orders",
            signed_order_body(
                "0x0000000000000000000000000000000000000002",
                "buy",
                "300000000000",
                "50000000",
                32,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(taker.status(), StatusCode::OK);
    let json = response_json(taker).await;
    let intent_id = json["execution_intents"][0]["intent_id"]
        .as_str()
        .unwrap()
        .to_string();
    (app, intent_id)
}

fn trade_signature(byte: u8) -> String {
    let mut signature = String::from("0x");
    for _ in 0..65 {
        signature.push_str(&format!("{byte:02x}"));
    }
    signature
}

fn simulation_config_without_persistence_requirement() -> ExecutionConfig {
    ExecutionConfig {
        execution_enabled: false,
        dry_run: true,
        poll_interval_ms: 1_000,
        max_batch_size: 10,
        real_broadcast_enabled: false,
        executor_private_key: None,
        executor_chain_id: 84532,
        max_gas_limit: 1_000_000,
        max_fee_per_gas_wei: None,
        max_priority_fee_per_gas_wei: None,
        require_simulation_ok: true,
        simulation_enabled: true,
        simulation_requires_persistence: false,
        rpc_url: Some("https://example.invalid".to_string()),
        executor_from_address: AccountId::new("0x0000000000000000000000000000000000000000"),
        perp_matching_engine_address: AccountId::new("0x0000000000000000000000000000000000000009"),
        perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
    }
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn signed_order_body(
    account: &str,
    side: &str,
    price_1e8: &str,
    size_1e8: &str,
    nonce: u64,
) -> String {
    signed_order_body_with_deadline(
        account,
        side,
        price_1e8,
        size_1e8,
        nonce,
        future_deadline(),
        VALID_SIGNATURE,
    )
}

fn signed_order_body_with_deadline(
    account: &str,
    side: &str,
    price_1e8: &str,
    size_1e8: &str,
    nonce: u64,
    deadline_ms: i64,
    signature: &str,
) -> String {
    format!(
        r#"{{
            "market_id": 1,
            "account": "{account}",
            "side": "{side}",
            "price_1e8": "{price_1e8}",
            "size_1e8": "{size_1e8}",
            "time_in_force": "gtc",
            "reduce_only": false,
            "post_only": false,
            "client_order_id": "client-{nonce}",
            "nonce": {nonce},
            "deadline_ms": {deadline_ms},
            "signature": "{signature}"
        }}"#
    )
}

fn future_deadline() -> i64 {
    now_ms() + 60_000
}

#[derive(Clone, Debug)]
struct StrictOrderFields {
    account: String,
    side: &'static str,
    price_1e8: String,
    size_1e8: String,
    nonce: u64,
    deadline_ms: i64,
    client_order_id: String,
}

fn valid_strict_fields(nonce: u64) -> StrictOrderFields {
    StrictOrderFields {
        account: test_account(),
        side: "sell",
        price_1e8: "300000000000".to_string(),
        size_1e8: "100000000".to_string(),
        nonce,
        deadline_ms: future_deadline(),
        client_order_id: format!("strict-client-{nonce}"),
    }
}

fn strict_signed_order_body(fields: &StrictOrderFields) -> String {
    let signature = strict_signature(fields);
    strict_signed_order_body_with_signature(fields, &signature)
}

fn strict_signed_order_body_with_signature(fields: &StrictOrderFields, signature: &str) -> String {
    let StrictOrderFields {
        account,
        side,
        price_1e8,
        size_1e8,
        nonce,
        deadline_ms,
        client_order_id,
    } = fields;
    format!(
        r#"{{
            "market_id": 1,
            "account": "{account}",
            "side": "{side}",
            "price_1e8": "{price_1e8}",
            "size_1e8": "{size_1e8}",
            "time_in_force": "gtc",
            "reduce_only": false,
            "post_only": false,
            "client_order_id": "{client_order_id}",
            "nonce": {nonce},
            "deadline_ms": {deadline_ms},
            "signature": "{signature}"
        }}"#
    )
}

fn strict_signature(fields: &StrictOrderFields) -> String {
    let order = SignedOrder {
        account: AccountId::new(fields.account.clone()),
        market_id: 1,
        side: match fields.side {
            "buy" => Side::Buy,
            "sell" => Side::Sell,
            other => panic!("unsupported test side: {other}"),
        },
        price_1e8: fields.price_1e8.parse().unwrap(),
        size_1e8: fields.size_1e8.parse().unwrap(),
        time_in_force: TimeInForce::Gtc,
        reduce_only: false,
        post_only: false,
        client_order_id: Some(fields.client_order_id.clone()),
        nonce: fields.nonce,
        deadline_ms: fields.deadline_ms,
        signature: String::new(),
    };
    let signing_key = test_signing_key();
    let digest = order.eip712_digest(&Eip712Domain::default()).unwrap();
    let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();

    let mut bytes = Vec::with_capacity(65);
    let signature_bytes = signature.to_bytes();
    bytes.extend_from_slice(&signature_bytes);
    bytes.push(recovery_id.to_byte() + 27);
    format!("0x{}", hex_encode(&bytes))
}

fn test_account() -> String {
    let verifying_key = test_signing_key().verifying_key().to_encoded_point(false);
    let hash = Keccak256::digest(&verifying_key.as_bytes()[1..]);
    format!("0x{}", hex_encode(&hash[12..]))
}

fn test_signing_key() -> SigningKey {
    let mut bytes = [0u8; 32];
    decode_hex_to_slice(TEST_ONLY_PRIVATE_KEY_HEX, &mut bytes).unwrap();
    SigningKey::from_slice(&bytes).unwrap()
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
