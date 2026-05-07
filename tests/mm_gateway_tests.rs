use deopt_v2_backend::mm::protocol::{ResultEnvelope, ServerMessage};
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
use serde_json::json;

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
fn heartbeat_updates_session_timestamp() {
    let service = MmGatewayService::new(MmGatewayConfig::default());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "heartbeat",
        "request_id": "hb-1",
        "payload": {}
    }))
    .unwrap();

    service.handle_message(&mut session, message, 40);

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
            "signature": "0xabc"
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
            "signature": "0xdef"
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
            "signature": "0xabc"
        }),
        json!({
            "price_1e8": "300100000000",
            "size_1e8": "100000000",
            "client_order_id": "eth-ask-001",
            "nonce": 2,
            "signature": "0xdef"
        }),
    ))
    .unwrap();

    let ClientMessage::QuoteReplace(envelope) = message else {
        panic!("expected quote_replace");
    };
    assert!(envelope.payload.bid.is_some());
    assert!(envelope.payload.ask.is_some());
}

#[test]
fn bulk_submit_partial_result_shape() {
    let service = MmGatewayService::new(MmGatewayConfig::default());
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

    let response = service.handle_message(&mut session, message, 20);

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

#[test]
fn get_session_returns_public_session_snapshot() {
    let service = MmGatewayService::new(MmGatewayConfig::default());
    let mut session =
        MmSession::with_ids("session-1", "connection-1", 10, AuthMode::Disabled, true);
    session.register_open_client_order_id("open-1");
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "get_session",
        "request_id": "get-1",
        "payload": {}
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 20);

    let ServerMessage::GetSessionResult(envelope) = response else {
        panic!("expected get_session_result");
    };
    assert_eq!(envelope.payload.session.session_id, "session-1");
    assert_eq!(
        envelope.payload.session.open_client_order_ids,
        vec!["open-1".to_string()]
    );
}

#[test]
fn disabled_auth_mode_allows_dev_session() {
    let service = MmGatewayService::new(MmGatewayConfig::default());
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

    let response = service.handle_message(&mut session, message, 20);

    assert!(matches!(response, ServerMessage::BulkSubmitResult(_)));
}

#[test]
fn require_auth_mode_rejects_trading_message_before_auth() {
    let config = MmGatewayConfig {
        require_auth: true,
        ..MmGatewayConfig::default()
    };
    let service = MmGatewayService::new(config);
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

    let response = service.handle_message(&mut session, message, 20);

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "AUTH_REQUIRED");
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
    json!({
        "market_id": 1,
        "account": "0x0000000000000000000000000000000000000001",
        "side": "buy",
        "price_1e8": "299900000000",
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "client_order_id": client_order_id,
        "nonce": 1,
        "deadline_ms": 9999999999999i64,
        "signature": "0xabc"
    })
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
    let bid = bid.into();
    let ask = ask.into();
    json!({
        "type": "quote_replace",
        "request_id": "qr-1",
        "payload": {
            "market_id": 1,
            "account": "0x0000000000000000000000000000000000000001",
            "cancel_previous": true,
            "bid": if bid.is_null() { serde_json::Value::Null } else { bid },
            "ask": if ask.is_null() { serde_json::Value::Null } else { ask }
        }
    })
}
