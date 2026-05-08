use deopt_v2_backend::mm::protocol::{
    NotificationEnvelope, ResultEnvelope, RfqQuoteResultPayload, RfqRequestPayload, ServerMessage,
};
use deopt_v2_backend::mm::rate_limit::{
    check_cancels_per_bulk, check_message_rate, check_open_orders, check_orders_per_bulk,
};
use deopt_v2_backend::mm::transport::webtransport::{
    decode_json_frame, encode_frame, encode_json_frame, read_frame, validate_webtransport_startup,
    write_json_frame, MmFrameError, MmGatewayStartup, MM_GATEWAY_MAX_FRAME_BYTES,
};
use deopt_v2_backend::mm::{
    AuthMode, BulkSubmitResultPayload, ClientMessage, ErrorCode, HeartbeatResultPayload,
    MmGatewayConfig, MmGatewayService, MmSession, RateLimitDecision,
};
use deopt_v2_backend::rfq::service::{create_rfq, CreateRfqInput};
use deopt_v2_backend::rfq::{RfqConfig, RfqQuoteStatus};
use deopt_v2_backend::types::{AccountId, Side};
use deopt_v2_backend::{api::AppState, engine::EngineState};
use serde_json::json;
use std::time::Duration;

const VALID_SIGNATURE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn parse_valid_heartbeat_message() {
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "heartbeat",
        "request_id": "hb-1",
        "payload": {}
    }))
    .unwrap();

    assert_eq!(message.message_type(), "heartbeat");
    assert_eq!(message.request_id(), "hb-1");
}

#[test]
fn reject_unknown_message_type() {
    let error = serde_json::from_value::<ClientMessage>(json!({
        "type": "unknown",
        "request_id": "bad-1",
        "payload": {}
    }))
    .unwrap_err();

    assert!(error.to_string().contains("unknown message type"));
}

#[test]
fn format_success_response_envelope() {
    let response = ServerMessage::HeartbeatResult(ResultEnvelope::new(
        "heartbeat_result",
        "hb-1",
        HeartbeatResultPayload {
            session_id: "session-1".to_string(),
            last_heartbeat_at_ms: 20,
        },
    ));

    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["type"], "heartbeat_result");
    assert_eq!(value["request_id"], "hb-1");
    assert_eq!(value["ok"], true);
    assert_eq!(value["payload"]["session_id"], "session-1");
}

#[test]
fn format_error_response_envelope() {
    let response = ServerMessage::error("bad-1", ErrorCode::BadRequest, "invalid request");

    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["type"], "error");
    assert_eq!(value["request_id"], "bad-1");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "BAD_REQUEST");
    assert_eq!(value["error"]["message"], "invalid request");
}

#[test]
fn parse_rfq_quote_message() {
    let rfq_id = uuid::Uuid::new_v4();
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "rfq_quote",
        "request_id": "mm-quote-1",
        "payload": {
            "rfq_id": rfq_id,
            "mm_account": "0x0000000000000000000000000000000000000001",
            "price_1e8": "300100000000",
            "size_1e8": "100000000",
            "client_quote_id": "mm-rfq-quote-001",
            "quote_ttl_ms": 3000
        }
    }))
    .unwrap();

    let ClientMessage::RfqQuote(envelope) = message else {
        panic!("expected rfq_quote");
    };
    assert_eq!(envelope.request_id, "mm-quote-1");
    assert_eq!(envelope.payload.rfq_id, rfq_id);
    assert_eq!(
        envelope.payload.client_quote_id.as_deref(),
        Some("mm-rfq-quote-001")
    );
}

#[test]
fn serialize_rfq_request_message() {
    let rfq_id = uuid::Uuid::new_v4();
    let response = ServerMessage::RfqRequest(NotificationEnvelope::new(
        "rfq_request",
        "rfq-push-1",
        RfqRequestPayload {
            rfq_id,
            taker: AccountId::new("0x0000000000000000000000000000000000000002"),
            market_id: 1,
            side: Side::Buy,
            size_1e8: "100000000".to_string(),
            limit_price_1e8: Some("305000000000".to_string()),
            expires_at_ms: 1_770_000_005_000,
        },
    ));

    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["type"], "rfq_request");
    assert_eq!(value["request_id"], "rfq-push-1");
    assert_eq!(value["payload"]["rfq_id"], rfq_id.to_string());
    assert_eq!(value["payload"]["side"], "buy");
    assert!(value.get("ok").is_none());
}

#[test]
fn serialize_rfq_quote_result_envelope() {
    let rfq_id = uuid::Uuid::new_v4();
    let quote_id = uuid::Uuid::new_v4();
    let response = ServerMessage::RfqQuoteResult(ResultEnvelope::new(
        "rfq_quote_result",
        "mm-quote-1",
        RfqQuoteResultPayload {
            quote_id,
            rfq_id,
            status: RfqQuoteStatus::Active,
            expires_at_ms: 1_770_000_003_000,
        },
    ));

    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["type"], "rfq_quote_result");
    assert_eq!(value["request_id"], "mm-quote-1");
    assert_eq!(value["ok"], true);
    assert_eq!(value["payload"]["quote_id"], quote_id.to_string());
    assert_eq!(value["payload"]["status"], "active");
}

#[tokio::test]
async fn heartbeat_updates_session_timestamp() {
    let service = mm_service(MmGatewayConfig::default());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "heartbeat",
        "request_id": "hb-1",
        "payload": {}
    }))
    .unwrap();

    service.handle_message(&mut session, message, 40).await;

    assert_eq!(session.last_heartbeat_at_ms, 40);
}

#[test]
fn heartbeat_timeout_decision() {
    let session = MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);

    assert!(!session.heartbeat_timed_out(20, 15));
    assert!(session.heartbeat_timed_out(30, 15));
}

#[test]
fn rate_limit_allows_under_threshold() {
    let config = MmGatewayConfig {
        rate_limit_per_sec: 2,
        ..MmGatewayConfig::default()
    };
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);

    assert!(check_message_rate(&mut session, &config, 10).is_allowed());
    assert!(check_message_rate(&mut session, &config, 20).is_allowed());
}

#[test]
fn rate_limit_rejects_over_threshold() {
    let config = MmGatewayConfig {
        rate_limit_per_sec: 1,
        ..MmGatewayConfig::default()
    };
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);

    assert!(check_message_rate(&mut session, &config, 10).is_allowed());
    assert!(matches!(
        check_message_rate(&mut session, &config, 20),
        RateLimitDecision::Rejected {
            code: ErrorCode::RateLimited,
            ..
        }
    ));
}

#[test]
fn max_orders_per_bulk_enforced() {
    let config = MmGatewayConfig {
        max_orders_per_bulk: 1,
        ..MmGatewayConfig::default()
    };

    assert!(check_orders_per_bulk(1, &config).is_allowed());
    assert!(matches!(
        check_orders_per_bulk(2, &config),
        RateLimitDecision::Rejected {
            code: ErrorCode::TooManyOrders,
            ..
        }
    ));
}

#[test]
fn max_cancels_per_bulk_enforced() {
    let config = MmGatewayConfig {
        max_cancels_per_bulk: 1,
        ..MmGatewayConfig::default()
    };

    assert!(check_cancels_per_bulk(1, &config).is_allowed());
    assert!(matches!(
        check_cancels_per_bulk(2, &config),
        RateLimitDecision::Rejected {
            code: ErrorCode::TooManyCancels,
            ..
        }
    ));
}

#[test]
fn max_open_orders_per_account_enforced() {
    let config = MmGatewayConfig {
        max_open_orders_per_account: 2,
        ..MmGatewayConfig::default()
    };

    assert!(check_open_orders(1, 1, &config).is_allowed());
    assert!(matches!(
        check_open_orders(2, 1, &config),
        RateLimitDecision::Rejected {
            code: ErrorCode::TooManyOrders,
            ..
        }
    ));
}

#[test]
fn cancel_on_disconnect_plan_returns_only_open_session_orders() {
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    session.register_open_client_order_id("open-1");
    session.register_open_client_order_id("cancelled-1");
    session.unregister_open_client_order_id("cancelled-1");

    let plan = session.plan_cancel_on_disconnect();

    assert_eq!(plan.client_order_ids, vec!["open-1".to_string()]);
}

#[test]
fn cancel_on_disconnect_is_idempotent() {
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    session.register_open_client_order_id("open-1");

    assert_eq!(
        session.plan_cancel_on_disconnect(),
        session.plan_cancel_on_disconnect()
    );
}

#[test]
fn quote_replace_parses_bid_only() {
    let message: ClientMessage = serde_json::from_value(quote_replace_json(
        json!({
            "price_1e8": "299900000000",
            "size_1e8": "100000000",
            "client_order_id": "eth-bid-001",
            "nonce": 1,
            "deadline_ms": 9999999999999i64,
            "signature": VALID_SIGNATURE
        }),
        ValueSide::None,
    ))
    .unwrap();

    let ClientMessage::QuoteReplace(envelope) = message else {
        panic!("expected quote_replace");
    };
    assert!(envelope.payload.bid.is_some());
    assert!(envelope.payload.ask.is_none());
}

#[test]
fn quote_replace_parses_ask_only() {
    let message: ClientMessage = serde_json::from_value(quote_replace_json(
        ValueSide::None,
        json!({
            "price_1e8": "300100000000",
            "size_1e8": "100000000",
            "client_order_id": "eth-ask-001",
            "nonce": 2,
            "deadline_ms": 9999999999999i64,
            "signature": VALID_SIGNATURE
        }),
    ))
    .unwrap();

    let ClientMessage::QuoteReplace(envelope) = message else {
        panic!("expected quote_replace");
    };
    assert!(envelope.payload.bid.is_none());
    assert!(envelope.payload.ask.is_some());
}

#[test]
fn quote_replace_parses_bid_and_ask() {
    let message: ClientMessage = serde_json::from_value(quote_replace_json(
        json!({
            "price_1e8": "299900000000",
            "size_1e8": "100000000",
            "client_order_id": "eth-bid-001",
            "nonce": 1,
            "deadline_ms": 9999999999999i64,
            "signature": VALID_SIGNATURE
        }),
        json!({
            "price_1e8": "300100000000",
            "size_1e8": "100000000",
            "client_order_id": "eth-ask-001",
            "nonce": 2,
            "deadline_ms": 9999999999999i64,
            "signature": VALID_SIGNATURE
        }),
    ))
    .unwrap();

    let ClientMessage::QuoteReplace(envelope) = message else {
        panic!("expected quote_replace");
    };
    assert!(envelope.payload.bid.is_some());
    assert!(envelope.payload.ask.is_some());
}

#[tokio::test]
async fn bulk_submit_partial_result_shape() {
    let service = mm_service(MmGatewayConfig::default());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "bulk_submit",
        "request_id": "bulk-1",
        "payload": {
            "orders": [
                valid_order("ok-1"),
                {
                    "market_id": 1,
                    "account": "0x0000000000000000000000000000000000000001",
                    "side": "buy",
                    "price_1e8": "0",
                    "size_1e8": "100000000",
                    "time_in_force": "gtc",
                    "client_order_id": "bad-1",
                    "nonce": 2,
                    "deadline_ms": 9999999999999i64,
                    "signature": "0xdef"
                }
            ]
        }
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 20).await;

    let ServerMessage::BulkSubmitResult(envelope) = response else {
        panic!("expected bulk_submit_result");
    };
    let payload: BulkSubmitResultPayload = envelope.payload;
    assert_eq!(payload.accepted, 1);
    assert_eq!(payload.rejected, 1);
    assert_eq!(payload.results.len(), 2);
    assert!(payload.results[0].ok);
    assert!(!payload.results[1].ok);
}

#[tokio::test]
async fn get_session_returns_public_session_snapshot() {
    let service = mm_service(MmGatewayConfig::default());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    session.register_open_client_order_id("open-1");
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "get_session",
        "request_id": "get-1",
        "payload": {}
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 20).await;

    let ServerMessage::GetSessionResult(envelope) = response else {
        panic!("expected get_session_result");
    };
    assert_eq!(envelope.payload.session.session_id, "session-1");
    assert_eq!(
        envelope.payload.session.open_client_order_ids,
        vec!["open-1".to_string()]
    );
}

#[tokio::test]
async fn gateway_handles_rfq_quote_and_stores_session_id() {
    let state = AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config());
    let rfq = create_rfq(&state, rfq_input()).await.unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = MmSession::with_ids(
        "session-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    let message = rfq_quote_message(rfq.rfq_id, "300100000000", "100000000");

    let response = service.handle_message(&mut session, message, 20).await;

    let ServerMessage::RfqQuoteResult(envelope) = response else {
        panic!("expected rfq_quote_result");
    };
    assert_eq!(envelope.payload.rfq_id, rfq.rfq_id);
    assert_eq!(envelope.payload.status, RfqQuoteStatus::Active);
    let quotes = deopt_v2_backend::rfq::service::list_quotes(&state, rfq.rfq_id)
        .await
        .unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].session_id.as_deref(), Some("session-rfq-1"));
}

#[tokio::test]
async fn gateway_rfq_quote_rejects_unknown_rfq() {
    let state = AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config());
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            rfq_quote_message(uuid::Uuid::new_v4(), "300100000000", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "RFQ_QUOTE_REJECTED");
}

#[tokio::test]
async fn gateway_rfq_quote_rejects_expired_rfq() {
    let state = AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config());
    let mut input = rfq_input();
    input.ttl_ms = Some(1);
    let rfq = create_rfq(&state, input).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            rfq_quote_message(rfq.rfq_id, "300100000000", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("RFQ has expired"));
}

#[tokio::test]
async fn gateway_rfq_quote_rejects_invalid_price_or_size() {
    let state = AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config());
    let rfq = create_rfq(&state, rfq_input()).await.unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            rfq_quote_message(rfq.rfq_id, "0", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("zero price"));
}

#[tokio::test]
async fn disabled_auth_mode_allows_dev_session() {
    let service = mm_service(MmGatewayConfig::default());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "bulk_submit",
        "request_id": "bulk-1",
        "payload": {
            "orders": [valid_order("ok-1")]
        }
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 20).await;

    assert!(matches!(response, ServerMessage::BulkSubmitResult(_)));
}

#[tokio::test]
async fn require_auth_mode_rejects_trading_message_before_auth() {
    let config = MmGatewayConfig {
        require_auth: true,
        ..MmGatewayConfig::default()
    };
    let service = mm_service(config);
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "bulk_submit",
        "request_id": "bulk-1",
        "payload": {
            "orders": [valid_order("ok-1")]
        }
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 20).await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "AUTH_REQUIRED");
}

#[tokio::test]
async fn submit_order_mutates_live_orderbook() {
    let state = AppState::new(EngineState::with_default_markets());
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "submit_order",
        "request_id": "submit-1",
        "payload": valid_order_with("open-1", "buy", 1)
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 20).await;

    let ServerMessage::SubmitOrderResult(envelope) = response else {
        panic!("expected submit_order_result");
    };
    assert!(envelope.payload.accepted);
    assert_eq!(envelope.payload.status, "accepted");
    assert!(envelope.payload.order_id.is_some());
    assert_eq!(session.open_client_order_ids.len(), 1);
    let snapshot = state.engine.lock().unwrap().orderbook_snapshot(1);
    assert_eq!(snapshot.bids.len(), 1);
    assert_eq!(snapshot.bids[0].total_size_1e8, 100000000);
}

#[tokio::test]
async fn cancel_order_by_client_order_id_mutates_live_orderbook() {
    let state = AppState::new(EngineState::with_default_markets());
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "submit_order",
                "request_id": "submit-1",
                "payload": valid_order_with("open-1", "buy", 1)
            }))
            .unwrap(),
            20,
        )
        .await;

    let response = service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "cancel_order",
                "request_id": "cancel-1",
                "payload": {
                    "account": "0x0000000000000000000000000000000000000001",
                    "market_id": 1,
                    "client_order_id": "open-1"
                }
            }))
            .unwrap(),
            30,
        )
        .await;

    let ServerMessage::CancelOrderResult(envelope) = response else {
        panic!("expected cancel_order_result");
    };
    assert!(envelope.payload.cancelled);
    assert!(session.open_client_order_ids.is_empty());
    let snapshot = state.engine.lock().unwrap().orderbook_snapshot(1);
    assert!(snapshot.bids.is_empty());
}

#[tokio::test]
async fn quote_replace_cancels_previous_then_submits_new_legs() {
    let state = AppState::new(EngineState::with_default_markets());
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    service
        .handle_message(
            &mut session,
            serde_json::from_value(quote_replace_json(
                quote_leg("old-bid", "299900000000", 1),
                ValueSide::None,
            ))
            .unwrap(),
            20,
        )
        .await;

    let response = service
        .handle_message(
            &mut session,
            serde_json::from_value(quote_replace_json_with_request(
                "qr-2",
                quote_leg("new-bid", "299800000000", 2),
                quote_leg("new-ask", "300200000000", 3),
            ))
            .unwrap(),
            30,
        )
        .await;

    let ServerMessage::QuoteReplaceResult(envelope) = response else {
        panic!("expected quote_replace_result");
    };
    assert_eq!(envelope.payload.cancelled, 1);
    assert_eq!(envelope.payload.submitted, 2);
    assert_eq!(envelope.payload.rejected, 0);
    assert_eq!(envelope.payload.results.len(), 2);
    let snapshot = state.engine.lock().unwrap().orderbook_snapshot(1);
    assert_eq!(snapshot.bids.len(), 1);
    assert_eq!(snapshot.bids[0].price_1e8, 299800000000);
    assert_eq!(snapshot.asks.len(), 1);
    assert_eq!(snapshot.asks[0].price_1e8, 300200000000);
}

#[tokio::test]
async fn cancel_on_disconnect_cancels_real_session_orders() {
    let state = AppState::new(EngineState::with_default_markets());
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "submit_order",
                "request_id": "submit-1",
                "payload": valid_order_with("open-1", "buy", 1)
            }))
            .unwrap(),
            20,
        )
        .await;

    let cancelled = service.cancel_on_disconnect(&mut session).await;

    assert_eq!(cancelled, 1);
    assert!(session.open_client_order_ids.is_empty());
    let snapshot = state.engine.lock().unwrap().orderbook_snapshot(1);
    assert!(snapshot.bids.is_empty());
}

#[tokio::test]
async fn cancel_order_rejects_non_owner() {
    let state = AppState::new(EngineState::with_default_markets());
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut owner = MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    service
        .handle_message(
            &mut owner,
            serde_json::from_value(json!({
                "type": "submit_order",
                "request_id": "submit-1",
                "payload": valid_order_with("open-1", "buy", 1)
            }))
            .unwrap(),
            20,
        )
        .await;
    let mut other = MmSession::with_ids("session-2", "connection-2", 10, AuthMode::Disabled, true);

    let response = service
        .handle_message(
            &mut other,
            serde_json::from_value(json!({
                "type": "cancel_order",
                "request_id": "cancel-1",
                "payload": {
                    "account": "0x0000000000000000000000000000000000000002",
                    "market_id": 1,
                    "client_order_id": "open-1"
                }
            }))
            .unwrap(),
            30,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "CANCEL_REJECTED");
    let snapshot = state.engine.lock().unwrap().orderbook_snapshot(1);
    assert_eq!(snapshot.bids.len(), 1);
}

#[tokio::test]
async fn cancel_all_cancels_all_account_resting_orders() {
    let state = AppState::new(EngineState::with_default_markets());
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "submit_order",
                "request_id": "submit-1",
                "payload": valid_order_with("open-1", "buy", 1)
            }))
            .unwrap(),
            20,
        )
        .await;
    service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "submit_order",
                "request_id": "submit-2",
                "payload": valid_order_with("open-2", "buy", 2)
            }))
            .unwrap(),
            30,
        )
        .await;

    let response = service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "cancel_all",
                "request_id": "cancel-all-1",
                "payload": {
                    "account": "0x0000000000000000000000000000000000000001",
                    "market_id": 1
                }
            }))
            .unwrap(),
            40,
        )
        .await;

    let ServerMessage::CancelAllResult(envelope) = response else {
        panic!("expected cancel_all_result");
    };
    assert_eq!(envelope.payload.cancelled, 2);
    assert!(session.open_client_order_ids.is_empty());
    let snapshot = state.engine.lock().unwrap().orderbook_snapshot(1);
    assert!(snapshot.bids.is_empty());
}

#[test]
fn frame_encode_decode_roundtrip() {
    let payload = br#"{"type":"heartbeat","request_id":"hb-1","payload":{}}"#;

    let frame = encode_frame(payload, MM_GATEWAY_MAX_FRAME_BYTES).unwrap();
    let decoded: serde_json::Value = decode_json_frame(&frame, MM_GATEWAY_MAX_FRAME_BYTES).unwrap();

    assert_eq!(decoded["type"], "heartbeat");
    assert_eq!(decoded["request_id"], "hb-1");
}

#[test]
fn oversized_frame_rejected() {
    let payload = vec![1_u8; 8];

    let error = encode_frame(&payload, 4).unwrap_err();

    assert!(matches!(error, MmFrameError::Oversized { len: 8, max: 4 }));
}

#[test]
fn invalid_json_rejected() {
    let frame = encode_frame(b"{not-json", MM_GATEWAY_MAX_FRAME_BYTES).unwrap();

    let error =
        decode_json_frame::<serde_json::Value>(&frame, MM_GATEWAY_MAX_FRAME_BYTES).unwrap_err();

    assert!(matches!(error, MmFrameError::Json(_)));
}

#[test]
fn disabled_config_does_not_start_gateway() {
    let config = MmGatewayConfig::default();

    let startup = validate_webtransport_startup(&config).unwrap();

    assert_eq!(startup, MmGatewayStartup::Disabled);
}

#[test]
fn service_response_can_be_serialized_into_frame() {
    let response = ServerMessage::error("bad-1", ErrorCode::BadRequest, "invalid request");

    let frame = encode_json_frame(&response, MM_GATEWAY_MAX_FRAME_BYTES).unwrap();
    let decoded: serde_json::Value = decode_json_frame(&frame, MM_GATEWAY_MAX_FRAME_BYTES).unwrap();

    assert_eq!(decoded["type"], "error");
    assert_eq!(decoded["request_id"], "bad-1");
    assert_eq!(decoded["ok"], false);
}

#[tokio::test]
async fn async_frame_read_write_roundtrip() {
    let (mut client, mut server) = tokio::io::duplex(1024);
    let response = ServerMessage::HeartbeatResult(ResultEnvelope::new(
        "heartbeat_result",
        "hb-1",
        HeartbeatResultPayload {
            session_id: "session-1".to_string(),
            last_heartbeat_at_ms: 20,
        },
    ));

    write_json_frame(&mut client, &response, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .unwrap();
    let payload = read_frame(&mut server, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .unwrap()
        .unwrap();
    let decoded: serde_json::Value = serde_json::from_slice(&payload).unwrap();

    assert_eq!(decoded["type"], "heartbeat_result");
    assert_eq!(decoded["request_id"], "hb-1");
}

fn valid_order(client_order_id: &str) -> serde_json::Value {
    valid_order_with(client_order_id, "buy", 1)
}

fn valid_order_with(client_order_id: &str, side: &str, nonce: u64) -> serde_json::Value {
    json!({
        "market_id": 1,
        "account": "0x0000000000000000000000000000000000000001",
        "side": side,
        "price_1e8": "299900000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "client_order_id": client_order_id,
        "nonce": nonce,
        "deadline_ms": 9999999999999i64,
        "signature": VALID_SIGNATURE
    })
}

fn mm_service(config: MmGatewayConfig) -> MmGatewayService {
    MmGatewayService::new(config, AppState::new(EngineState::with_default_markets()))
}

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

fn rfq_input() -> CreateRfqInput {
    CreateRfqInput {
        taker: AccountId::new("0x0000000000000000000000000000000000000002"),
        market_id: 1,
        side: Side::Buy,
        size_1e8: 100_000_000,
        limit_price_1e8: Some(305_000_000_000),
        ttl_ms: Some(500),
    }
}

fn rfq_quote_message(rfq_id: uuid::Uuid, price_1e8: &str, size_1e8: &str) -> ClientMessage {
    serde_json::from_value(json!({
        "type": "rfq_quote",
        "request_id": "mm-quote-1",
        "payload": {
            "rfq_id": rfq_id,
            "mm_account": "0x0000000000000000000000000000000000000001",
            "price_1e8": price_1e8,
            "size_1e8": size_1e8,
            "client_quote_id": "mm-rfq-quote-001",
            "quote_ttl_ms": 100
        }
    }))
    .unwrap()
}

enum ValueSide {
    None,
}

impl From<ValueSide> for serde_json::Value {
    fn from(_: ValueSide) -> Self {
        serde_json::Value::Null
    }
}

fn quote_replace_json(
    bid: impl Into<serde_json::Value>,
    ask: impl Into<serde_json::Value>,
) -> serde_json::Value {
    quote_replace_json_with_request("qr-1", bid, ask)
}

fn quote_replace_json_with_request(
    request_id: &str,
    bid: impl Into<serde_json::Value>,
    ask: impl Into<serde_json::Value>,
) -> serde_json::Value {
    let bid = bid.into();
    let ask = ask.into();
    json!({
        "type": "quote_replace",
        "request_id": request_id,
        "payload": {
            "market_id": 1,
            "account": "0x0000000000000000000000000000000000000001",
            "cancel_previous": true,
            "bid": if bid.is_null() { serde_json::Value::Null } else { bid },
            "ask": if ask.is_null() { serde_json::Value::Null } else { ask }
        }
    })
}

fn quote_leg(client_order_id: &str, price_1e8: &str, nonce: u64) -> serde_json::Value {
    json!({
        "price_1e8": price_1e8,
        "size_1e8": "100000000",
        "client_order_id": client_order_id,
        "nonce": nonce,
        "deadline_ms": 9999999999999i64,
        "signature": VALID_SIGNATURE
    })
}
