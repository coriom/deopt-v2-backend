use deopt_v2_backend::mm::protocol::{
    NotificationEnvelope, OptionRfqQuoteAcceptedPayload, OptionRfqQuoteResultPayload,
    OptionRfqRequestPayload, ResultEnvelope, RfqQuoteResultPayload, RfqRequestPayload,
    ServerMessage,
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
    MmAccountPermissions, MmGatewayConfig, MmGatewayService, MmPermissionsConfig,
    MmProductPermission, MmSession, RateLimitDecision,
};
use deopt_v2_backend::options::service::{
    accept_option_rfq_quote, create_option_rfq, create_option_series, list_option_rfq_quotes,
    option_rfq_quote_signing_payload, submit_option_rfq_quote, CreateOptionRfqInput,
    CreateOptionSeriesInput, OptionRfqQuoteSigningPayloadInput, SubmitOptionRfqQuoteInput,
};
use deopt_v2_backend::options::{OptionRfqQuoteSignatureMode, OptionRfqQuoteStatus, OptionsConfig};
use deopt_v2_backend::rfq::service::{create_rfq, CreateRfqInput};
use deopt_v2_backend::rfq::{RfqConfig, RfqQuoteStatus};
use deopt_v2_backend::signing::personal_sign_digest;
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use deopt_v2_backend::{api::AppState, engine::EngineState};
use k256::ecdsa::SigningKey;
use serde_json::json;
use sha3::{Digest, Keccak256};
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
fn parse_auth_challenge_message() {
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "auth_challenge",
        "request_id": "auth-1",
        "payload": {
            "account": "0x0000000000000000000000000000000000000001"
        }
    }))
    .unwrap();

    let ClientMessage::AuthChallenge(envelope) = message else {
        panic!("expected auth_challenge");
    };
    assert_eq!(envelope.request_id, "auth-1");
    assert_eq!(
        envelope.payload.account,
        AccountId::new("0x0000000000000000000000000000000000000001")
    );
}

#[test]
fn parse_auth_verify_message() {
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "auth_verify",
        "request_id": "auth-2",
        "payload": {
            "account": "0x0000000000000000000000000000000000000001",
            "signature": VALID_SIGNATURE
        }
    }))
    .unwrap();

    let ClientMessage::AuthVerify(envelope) = message else {
        panic!("expected auth_verify");
    };
    assert_eq!(envelope.request_id, "auth-2");
    assert_eq!(envelope.payload.signature, VALID_SIGNATURE);
}

#[test]
fn parse_rfq_quote_with_signature_fields() {
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "rfq_quote",
        "request_id": "rfq-quote-1",
        "payload": {
            "rfq_id": "a1bbb9bf-2f33-4686-9cdc-30e292ff391f",
            "mm_account": "0x0000000000000000000000000000000000000001",
            "price_1e8": "300100000000",
            "size_1e8": "100000000",
            "client_quote_id": "mm-rfq-quote-001",
            "quote_nonce": 1,
            "quote_ttl_ms": 3000,
            "signature": VALID_SIGNATURE
        }
    }))
    .unwrap();

    let ClientMessage::RfqQuote(envelope) = message else {
        panic!("expected rfq_quote");
    };
    assert_eq!(envelope.payload.quote_nonce, Some(1));
    assert_eq!(envelope.payload.signature.as_deref(), Some(VALID_SIGNATURE));
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
fn parse_option_rfq_quote_message() {
    let option_rfq_id = uuid::Uuid::new_v4();
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "option_rfq_quote",
        "request_id": "mm-option-quote-1",
        "payload": {
            "option_rfq_id": option_rfq_id,
            "mm_account": "0x0000000000000000000000000000000000000001",
            "price_1e8": "1100000000",
            "size_1e8": "100000000",
            "client_quote_id": "mm-option-rfq-quote-001",
            "quote_ttl_ms": 3000
        }
    }))
    .unwrap();

    let ClientMessage::OptionRfqQuote(envelope) = message else {
        panic!("expected option_rfq_quote");
    };
    assert_eq!(envelope.request_id, "mm-option-quote-1");
    assert_eq!(envelope.payload.option_rfq_id, option_rfq_id);
    assert_eq!(
        envelope.payload.client_quote_id.as_deref(),
        Some("mm-option-rfq-quote-001")
    );
}

#[test]
fn parse_option_rfq_quote_with_signature_fields() {
    let option_rfq_id = uuid::Uuid::new_v4();
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "option_rfq_quote",
        "request_id": "mm-option-quote-1",
        "payload": {
            "option_rfq_id": option_rfq_id,
            "mm_account": "0x0000000000000000000000000000000000000001",
            "price_1e8": "1100000000",
            "size_1e8": "100000000",
            "client_quote_id": "mm-option-rfq-quote-001",
            "quote_nonce": 1001,
            "quote_ttl_ms": 3000,
            "signature": VALID_SIGNATURE
        }
    }))
    .unwrap();

    let ClientMessage::OptionRfqQuote(envelope) = message else {
        panic!("expected option_rfq_quote");
    };
    assert_eq!(envelope.payload.option_rfq_id, option_rfq_id);
    assert_eq!(envelope.payload.quote_nonce, Some(1001));
    assert_eq!(envelope.payload.signature.as_deref(), Some(VALID_SIGNATURE));
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
fn serialize_option_rfq_request_message() {
    let option_rfq_id = uuid::Uuid::new_v4();
    let response = ServerMessage::OptionRfqRequest(NotificationEnvelope::new(
        "option_rfq_request",
        "option-rfq-push-1",
        OptionRfqRequestPayload {
            option_rfq_id,
            taker: AccountId::new("0x0000000000000000000000000000000000000002"),
            option_series_id: "series-1".to_string(),
            side: Side::Buy,
            size_1e8: "100000000".to_string(),
            limit_price_1e8: Some("1200000000".to_string()),
            expires_at_ms: 1_770_000_005_000,
        },
    ));

    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["type"], "option_rfq_request");
    assert_eq!(value["request_id"], "option-rfq-push-1");
    assert_eq!(value["payload"]["option_rfq_id"], option_rfq_id.to_string());
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

#[test]
fn serialize_option_rfq_quote_result_envelope() {
    let option_rfq_id = uuid::Uuid::new_v4();
    let quote_id = uuid::Uuid::new_v4();
    let response = ServerMessage::OptionRfqQuoteResult(ResultEnvelope::new(
        "option_rfq_quote_result",
        "mm-option-quote-1",
        OptionRfqQuoteResultPayload {
            quote_id,
            option_rfq_id,
            status: OptionRfqQuoteStatus::Active,
            expires_at_ms: 1_770_000_003_000,
        },
    ));

    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["type"], "option_rfq_quote_result");
    assert_eq!(value["request_id"], "mm-option-quote-1");
    assert_eq!(value["ok"], true);
    assert_eq!(value["payload"]["quote_id"], quote_id.to_string());
    assert_eq!(value["payload"]["option_rfq_id"], option_rfq_id.to_string());
    assert_eq!(value["payload"]["status"], "active");
}

#[test]
fn serialize_option_rfq_quote_accepted_message() {
    let option_rfq_id = uuid::Uuid::new_v4();
    let quote_id = uuid::Uuid::new_v4();
    let option_fill_id = uuid::Uuid::new_v4();
    let response = ServerMessage::OptionRfqQuoteAccepted(NotificationEnvelope::new(
        "option_rfq_quote_accepted",
        "option-rfq-accepted-1",
        OptionRfqQuoteAcceptedPayload {
            option_rfq_id,
            quote_id,
            option_fill_id,
        },
    ));

    let value = serde_json::to_value(response).unwrap();

    assert_eq!(value["type"], "option_rfq_quote_accepted");
    assert_eq!(value["request_id"], "option-rfq-accepted-1");
    assert_eq!(value["payload"]["option_rfq_id"], option_rfq_id.to_string());
    assert_eq!(value["payload"]["quote_id"], quote_id.to_string());
    assert_eq!(
        value["payload"]["option_fill_id"],
        option_fill_id.to_string()
    );
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
async fn gateway_rfq_quote_permissions_reject_missing_disabled_and_wrong_capability() {
    let cases = [
        ("missing", None),
        (
            "disabled",
            Some(mm_account_permissions(false, false, true, false)),
        ),
        (
            "wrong capability",
            Some(mm_account_permissions(true, false, false, false)),
        ),
    ];

    for (case, account) in cases {
        let mut state =
            AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config());
        state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
        if let Some(account) = account {
            seed_mm_account(&state, account);
        }
        let rfq = create_rfq(&state, rfq_input()).await.unwrap();
        let service = MmGatewayService::new(MmGatewayConfig::default(), state);
        let mut session = MmSession::with_ids(
            format!("session-rfq-{case}"),
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
        assert_eq!(value["type"], "error", "{case}");
        assert_eq!(value["error"]["code"], "RFQ_QUOTE_REJECTED", "{case}");
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("MM permission denied"),
            "{case}"
        );
    }
}

#[tokio::test]
async fn gateway_rfq_quote_permissions_enforce_market_scope() {
    let mut state = AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config());
    state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
    seed_mm_account(&state, mm_account_permissions(true, false, true, false));
    seed_mm_product_permission(&state, "perp-market-1", Some(1), None, true);
    let allowed_rfq = create_rfq(&state, rfq_input()).await.unwrap();
    let mut blocked_input = rfq_input();
    blocked_input.market_id = 2;
    let blocked_rfq = create_rfq(&state, blocked_input).await.unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-rfq-scope",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let allowed = service
        .handle_message(
            &mut session,
            rfq_quote_message(allowed_rfq.rfq_id, "300100000000", "100000000"),
            20,
        )
        .await;
    assert!(matches!(allowed, ServerMessage::RfqQuoteResult(_)));

    let blocked = service
        .handle_message(
            &mut session,
            rfq_quote_message(blocked_rfq.rfq_id, "300100000000", "100000000"),
            30,
        )
        .await;
    let value = serde_json::to_value(blocked).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("market_id 2"));
}

#[tokio::test]
async fn gateway_handles_option_rfq_quote_and_stores_session_id() {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = MmSession::with_ids(
        "session-option-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    let message = option_rfq_quote_message(rfq.option_rfq_id, "1000000000", "100000000");

    let response = service.handle_message(&mut session, message, 20).await;

    let ServerMessage::OptionRfqQuoteResult(envelope) = response else {
        panic!("expected option_rfq_quote_result");
    };
    assert_eq!(envelope.payload.option_rfq_id, rfq.option_rfq_id);
    assert_eq!(envelope.payload.status, OptionRfqQuoteStatus::Active);
    let quotes = list_option_rfq_quotes(&state, rfq.option_rfq_id)
        .await
        .unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(
        quotes[0].session_id.as_deref(),
        Some("session-option-rfq-1")
    );
}

#[tokio::test]
async fn strict_gateway_rejects_unsigned_option_rfq_quote() {
    let state = strict_option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-option-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            option_rfq_quote_message(rfq.option_rfq_id, "1000000000", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("quote_nonce is required"));
}

#[tokio::test]
async fn strict_gateway_accepts_signed_option_rfq_quote() {
    let state = strict_option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let signature = sign_option_quote_digest(
        &gateway_option_quote_payload_digest(&state, rfq.option_rfq_id, 77).await,
        test_signing_key(),
    );
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = MmSession::with_ids(
        "session-option-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            option_rfq_quote_message_with_signature(rfq.option_rfq_id, 77, &signature),
            20,
        )
        .await;

    let ServerMessage::OptionRfqQuoteResult(envelope) = response else {
        panic!("expected option_rfq_quote_result");
    };
    assert_eq!(envelope.payload.status, OptionRfqQuoteStatus::Active);
    let quotes = list_option_rfq_quotes(&state, rfq.option_rfq_id)
        .await
        .unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].quote_nonce.as_deref(), Some("77"));
    assert_eq!(quotes[0].recovered_signer, Some(signing_account()));
}

#[tokio::test]
async fn gateway_option_rfq_quote_rejects_unknown_rfq() {
    let state = option_rfq_state();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-option-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            option_rfq_quote_message(uuid::Uuid::new_v4(), "1000000000", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
}

#[tokio::test]
async fn gateway_option_rfq_quote_rejects_expired_rfq() {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let mut input = option_rfq_input(option_series_id, Side::Buy);
    input.ttl_ms = Some(1);
    let rfq = create_option_rfq(&state, input).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-option-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            option_rfq_quote_message(rfq.option_rfq_id, "1000000000", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not open"));
}

#[tokio::test]
async fn gateway_option_rfq_quote_rejects_invalid_price_or_size() {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-option-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            option_rfq_quote_message(rfq.option_rfq_id, "0", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("zero price"));
}

#[tokio::test]
async fn gateway_option_rfq_quote_permissions_require_capability() {
    let mut state = option_rfq_state();
    state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
    seed_mm_account(&state, mm_account_permissions(true, false, true, false));
    let option_series_id = active_option_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-option-permission",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let response = service
        .handle_message(
            &mut session,
            option_rfq_quote_message(rfq.option_rfq_id, "1000000000", "100000000"),
            20,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("can_quote_option_rfq"));
}

#[tokio::test]
async fn gateway_option_rfq_quote_permissions_enforce_series_scope() {
    let mut state = option_rfq_state();
    state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
    seed_mm_account(&state, mm_account_permissions(true, false, false, true));
    let allowed_series = active_option_series_id(&state).await;
    let blocked_series = second_active_option_series_id(&state).await;
    seed_mm_product_permission(
        &state,
        "option-series-1",
        None,
        Some(allowed_series.clone()),
        true,
    );
    let allowed_rfq =
        create_option_rfq(&state, option_rfq_input(allowed_series.clone(), Side::Buy))
            .await
            .unwrap();
    let blocked_rfq = create_option_rfq(&state, option_rfq_input(blocked_series, Side::Buy))
        .await
        .unwrap();
    let service = MmGatewayService::new(MmGatewayConfig::default(), state);
    let mut session = MmSession::with_ids(
        "session-option-scope",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let allowed = service
        .handle_message(
            &mut session,
            option_rfq_quote_message(allowed_rfq.option_rfq_id, "1000000000", "100000000"),
            20,
        )
        .await;
    assert!(matches!(allowed, ServerMessage::OptionRfqQuoteResult(_)));

    let blocked = service
        .handle_message(
            &mut session,
            option_rfq_quote_message(blocked_rfq.option_rfq_id, "1000000000", "100000000"),
            30,
        )
        .await;
    let value = serde_json::to_value(blocked).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("option_series_id"));
}

#[tokio::test]
async fn option_rfq_creation_broadcasts_to_connected_mock_session() {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let session = MmSession::with_ids(
        "session-option-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    service.register_session(&session, sender).unwrap();

    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();

    let message = receiver.recv().await.unwrap();
    let ServerMessage::OptionRfqRequest(envelope) = message else {
        panic!("expected option_rfq_request");
    };
    assert_eq!(envelope.payload.option_rfq_id, rfq.option_rfq_id);
}

#[tokio::test]
async fn option_rfq_creation_succeeds_with_zero_sessions() {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;

    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();

    assert!(!rfq.option_rfq_id.is_nil());
}

#[tokio::test]
async fn option_rfq_accept_sends_accepted_and_competing_rejected_notifications() {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let accepted_session = MmSession::with_ids(
        "session-option-accepted",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    let rejected_session = MmSession::with_ids(
        "session-option-rejected",
        "connection-2",
        10,
        AuthMode::Disabled,
        true,
    );
    let (accepted_sender, mut accepted_receiver) = tokio::sync::mpsc::unbounded_channel();
    let (rejected_sender, mut rejected_receiver) = tokio::sync::mpsc::unbounded_channel();
    service
        .register_session(&accepted_session, accepted_sender)
        .unwrap();
    service
        .register_session(&rejected_session, rejected_sender)
        .unwrap();
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let _ = accepted_receiver.recv().await.unwrap();
    let _ = rejected_receiver.recv().await.unwrap();
    let winner = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(
            AccountId::new("0x0000000000000000000000000000000000000001"),
            "session-option-accepted",
            "winner-option-rfq-quote",
        ),
    )
    .await
    .unwrap();
    let loser = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(
            AccountId::new("0x0000000000000000000000000000000000000002"),
            "session-option-rejected",
            "loser-option-rfq-quote",
        ),
    )
    .await
    .unwrap();

    let outcome = accept_option_rfq_quote(&state, rfq.option_rfq_id, winner.quote_id)
        .await
        .unwrap();

    assert!(outcome.mm_notification_sent);
    assert!(outcome.mm_notification_warning.is_none());
    let accepted = accepted_receiver.recv().await.unwrap();
    let ServerMessage::OptionRfqQuoteAccepted(envelope) = accepted else {
        panic!("expected option_rfq_quote_accepted");
    };
    assert_eq!(envelope.payload.quote_id, winner.quote_id);
    assert_eq!(envelope.payload.option_fill_id, outcome.fill.fill_id);
    let rejected = rejected_receiver.recv().await.unwrap();
    let ServerMessage::OptionRfqQuoteRejected(envelope) = rejected else {
        panic!("expected option_rfq_quote_rejected");
    };
    assert_eq!(envelope.payload.quote_id, loser.quote_id);
    assert_eq!(envelope.payload.reason, "competing quote accepted");
}

#[tokio::test]
async fn option_rfq_accept_succeeds_even_if_notification_fails_without_forbidden_mutations() {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    let quote = submit_option_rfq_quote(
        &state,
        rfq.option_rfq_id,
        option_rfq_quote_input(
            AccountId::new("0x0000000000000000000000000000000000000001"),
            "missing-option-session",
            "notify-fail-option-rfq-quote",
        ),
    )
    .await
    .unwrap();

    let outcome = accept_option_rfq_quote(&state, rfq.option_rfq_id, quote.quote_id)
        .await
        .unwrap();

    assert_eq!(
        outcome.rfq.status,
        deopt_v2_backend::options::OptionRfqStatus::Accepted
    );
    assert!(!outcome.mm_notification_sent);
    assert!(outcome.mm_notification_warning.is_some());
    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 0);
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
async fn wallet_challenge_authenticates_session_with_personal_signature() {
    let service = mm_service(wallet_auth_config());
    let mut session = wallet_challenge_session();
    let challenge_response = service
        .handle_message(
            &mut session,
            auth_challenge_message(&signing_account().0),
            100,
        )
        .await;

    let ServerMessage::AuthChallengeResult(challenge_envelope) = challenge_response else {
        panic!("expected auth_challenge_result");
    };
    assert_eq!(challenge_envelope.payload.session_id, "session-wallet-1");
    assert_eq!(challenge_envelope.payload.issued_at_ms, 100);
    assert_eq!(challenge_envelope.payload.expires_at_ms, 60_100);
    assert!(!session.authenticated);
    assert!(session.challenge_active());

    let signature =
        sign_personal_message(&challenge_envelope.payload.challenge, test_signing_key());
    let verify_response = service
        .handle_message(
            &mut session,
            auth_verify_message(&signing_account().0, &signature),
            120,
        )
        .await;

    let ServerMessage::AuthVerifyResult(verify_envelope) = verify_response else {
        panic!("expected auth_verify_result");
    };
    assert!(verify_envelope.payload.authenticated);
    assert_eq!(verify_envelope.payload.account, signing_account());
    assert!(session.authenticated);
    assert_eq!(session.account, Some(signing_account()));
    assert!(!session.challenge_active());
    assert!(session.challenge_nonce.is_none());
}

#[tokio::test]
async fn wallet_challenge_rejects_expired_challenge() {
    let config = MmGatewayConfig {
        auth_mode: AuthMode::WalletChallenge,
        require_auth: true,
        challenge_ttl_ms: 5,
        ..MmGatewayConfig::default()
    };
    let service = mm_service(config);
    let mut session = wallet_challenge_session();
    let challenge_response = service
        .handle_message(
            &mut session,
            auth_challenge_message(&signing_account().0),
            100,
        )
        .await;
    let ServerMessage::AuthChallengeResult(challenge_envelope) = challenge_response else {
        panic!("expected auth_challenge_result");
    };
    let signature =
        sign_personal_message(&challenge_envelope.payload.challenge, test_signing_key());

    let response = service
        .handle_message(
            &mut session,
            auth_verify_message(&signing_account().0, &signature),
            106,
        )
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "AUTH_FAILED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("expired"));
    assert!(!session.authenticated);
}

#[tokio::test]
async fn wallet_challenge_rejects_trading_account_mismatch() {
    let mut state = AppState::new(EngineState::with_default_markets());
    state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
    seed_mm_account(&state, mm_account_permissions(true, true, true, true));
    let service = MmGatewayService::new(wallet_auth_config(), state);
    let mut session = authenticated_wallet_session(&service).await;
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "submit_order",
        "request_id": "submit-1",
        "payload": valid_order_with("wrong-account", "buy", 1)
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 140).await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "ORDER_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("does not match authenticated session"));
}

#[tokio::test]
async fn wallet_challenge_account_checks_are_case_insensitive() {
    let service = mm_service(wallet_auth_config());
    let mixed_case_account = uppercase_hex_account(&signing_account());
    let mut session = wallet_challenge_session();
    let challenge_response = service
        .handle_message(
            &mut session,
            auth_challenge_message(&mixed_case_account.0),
            100,
        )
        .await;
    let ServerMessage::AuthChallengeResult(challenge_envelope) = challenge_response else {
        panic!("expected auth_challenge_result");
    };
    let signature =
        sign_personal_message(&challenge_envelope.payload.challenge, test_signing_key());
    let verify_response = service
        .handle_message(
            &mut session,
            auth_verify_message(&mixed_case_account.0, &signature),
            120,
        )
        .await;
    assert!(matches!(
        verify_response,
        ServerMessage::AuthVerifyResult(_)
    ));

    let mut order = valid_order_with("case-ok", "buy", 1);
    order["account"] = json!(mixed_case_account.0);
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "submit_order",
        "request_id": "submit-1",
        "payload": order
    }))
    .unwrap();

    let response = service.handle_message(&mut session, message, 140).await;

    let ServerMessage::SubmitOrderResult(envelope) = response else {
        panic!("expected submit_order_result");
    };
    assert!(envelope.payload.accepted);
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
async fn submit_order_permissions_require_capability_and_market_scope() {
    let mut state = AppState::new(EngineState::with_default_markets());
    state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
    seed_mm_account(&state, mm_account_permissions(true, false, true, true));
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = MmSession::with_ids(
        "session-submit-permission",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );

    let rejected = service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "submit_order",
                "request_id": "submit-1",
                "payload": valid_order_with("blocked-cap", "buy", 1)
            }))
            .unwrap(),
            20,
        )
        .await;
    let value = serde_json::to_value(rejected).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "ORDER_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("can_submit_perp_orders"));

    seed_mm_account(&state, mm_account_permissions(true, true, true, true));
    seed_mm_product_permission(&state, "submit-market-2", Some(2), None, true);
    let wrong_market = service
        .handle_message(
            &mut session,
            serde_json::from_value(json!({
                "type": "submit_order",
                "request_id": "submit-2",
                "payload": valid_order_with("blocked-market", "buy", 2)
            }))
            .unwrap(),
            30,
        )
        .await;
    let value = serde_json::to_value(wrong_market).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "ORDER_REJECTED");
    assert!(value["error"]["message"]
        .as_str()
        .unwrap()
        .contains("market_id 1"));
}

#[tokio::test]
async fn quote_replace_permissions_reject_before_cancelling_previous_quotes() {
    let mut state = AppState::new(EngineState::with_default_markets());
    state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
    seed_mm_account(&state, mm_account_permissions(true, true, true, true));
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = MmSession::with_ids(
        "session-qr-permission",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    let accepted = service
        .handle_message(
            &mut session,
            serde_json::from_value(quote_replace_json(
                quote_leg("old-bid-permission", "299900000000", 1),
                ValueSide::None,
            ))
            .unwrap(),
            20,
        )
        .await;
    assert!(matches!(accepted, ServerMessage::QuoteReplaceResult(_)));
    seed_mm_account(&state, mm_account_permissions(true, false, true, true));

    let rejected = service
        .handle_message(
            &mut session,
            serde_json::from_value(quote_replace_json_with_request(
                "qr-permission-2",
                quote_leg("new-bid-permission", "299800000000", 2),
                ValueSide::None,
            ))
            .unwrap(),
            30,
        )
        .await;

    let value = serde_json::to_value(rejected).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "QUOTE_REPLACE_FAILED");
    let snapshot = state.engine.lock().unwrap().orderbook_snapshot(1);
    assert_eq!(snapshot.bids.len(), 1);
    assert_eq!(snapshot.bids[0].price_1e8, 299900000000);
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

fn wallet_auth_config() -> MmGatewayConfig {
    MmGatewayConfig {
        auth_mode: AuthMode::WalletChallenge,
        require_auth: true,
        ..MmGatewayConfig::default()
    }
}

fn wallet_challenge_session() -> MmSession {
    MmSession::with_ids(
        "session-wallet-1",
        "connection-1",
        10,
        AuthMode::WalletChallenge,
        true,
    )
}

async fn authenticated_wallet_session(service: &MmGatewayService) -> MmSession {
    let mut session = wallet_challenge_session();
    let response = service
        .handle_message(
            &mut session,
            auth_challenge_message(&signing_account().0),
            100,
        )
        .await;
    let ServerMessage::AuthChallengeResult(envelope) = response else {
        panic!("expected auth_challenge_result");
    };
    let signature = sign_personal_message(&envelope.payload.challenge, test_signing_key());
    let response = service
        .handle_message(
            &mut session,
            auth_verify_message(&signing_account().0, &signature),
            120,
        )
        .await;
    assert!(matches!(response, ServerMessage::AuthVerifyResult(_)));
    session
}

fn auth_challenge_message(account: &str) -> ClientMessage {
    serde_json::from_value(json!({
        "type": "auth_challenge",
        "request_id": "auth-1",
        "payload": {
            "account": account
        }
    }))
    .unwrap()
}

fn auth_verify_message(account: &str, signature: &str) -> ClientMessage {
    serde_json::from_value(json!({
        "type": "auth_verify",
        "request_id": "auth-2",
        "payload": {
            "account": account,
            "signature": signature
        }
    }))
    .unwrap()
}

fn sign_personal_message(message: &str, signing_key: SigningKey) -> String {
    let digest = personal_sign_digest(message);
    let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&signature.to_bytes());
    bytes.push(recovery_id.to_byte() + 27);
    format!("0x{}", hex_encode(&bytes))
}

fn uppercase_hex_account(account: &AccountId) -> AccountId {
    AccountId::new(format!("0x{}", account.0[2..].to_ascii_uppercase()))
}

fn mm_service(config: MmGatewayConfig) -> MmGatewayService {
    MmGatewayService::new(config, AppState::new(EngineState::with_default_markets()))
}

fn mm_account_permissions(
    enabled: bool,
    can_submit_perp_orders: bool,
    can_quote_perp_rfq: bool,
    can_quote_option_rfq: bool,
) -> MmAccountPermissions {
    MmAccountPermissions {
        mm_account: AccountId::new("0x0000000000000000000000000000000000000001"),
        enabled,
        label: Some("MM Alpha".to_string()),
        can_submit_perp_orders,
        can_quote_perp_rfq,
        can_quote_option_rfq,
        can_submit_option_orders: false,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn seed_mm_account(state: &AppState, account: MmAccountPermissions) {
    state.mm_permissions.lock().unwrap().upsert_account(account);
}

fn seed_mm_product_permission(
    state: &AppState,
    id: &str,
    market_id: Option<u64>,
    option_series_id: Option<String>,
    enabled: bool,
) {
    state
        .mm_permissions
        .lock()
        .unwrap()
        .insert_product_permission(MmProductPermission {
            id: id.to_string(),
            mm_account: AccountId::new("0x0000000000000000000000000000000000000001"),
            market_id,
            option_series_id,
            enabled,
            created_at_ms: 1,
            updated_at_ms: 1,
        });
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
        ..RfqConfig::disabled()
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

fn option_rfq_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    config.rfq_min_quote_ttl_ms = 1;
    config.rfq_max_quote_ttl_ms = 500;
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

fn strict_option_rfq_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    config.rfq_min_quote_ttl_ms = 1;
    config.rfq_max_quote_ttl_ms = 500;
    config.rfq_quote_signature_mode = OptionRfqQuoteSignatureMode::Strict;
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

async fn active_option_series_id(state: &AppState) -> String {
    create_option_series(state, option_series_input())
        .await
        .unwrap()
        .option_series_id
}

async fn second_active_option_series_id(state: &AppState) -> String {
    let mut input = option_series_input();
    input.strike_1e8 = 310_000_000_000;
    create_option_series(state, input)
        .await
        .unwrap()
        .option_series_id
}

fn option_series_input() -> CreateOptionSeriesInput {
    let expiry = u64::try_from(now_ms() / 1000).unwrap() + 86_400;
    CreateOptionSeriesInput {
        underlying: "ETH".to_string(),
        base_asset: "ETH".to_string(),
        quote_asset: "USDC".to_string(),
        settlement_asset: "USDC".to_string(),
        expiry,
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: Some(100_000_000),
        onchain_product_id: None,
        onchain_series_id: None,
    }
}

fn option_rfq_input(option_series_id: String, side: Side) -> CreateOptionRfqInput {
    CreateOptionRfqInput {
        taker: AccountId::new("0x0000000000000000000000000000000000000003"),
        option_series_id,
        side,
        size_1e8: 100_000_000,
        limit_price_1e8: Some(1_100_000_000),
        ttl_ms: Some(500),
    }
}

fn option_rfq_quote_input(
    mm_account: AccountId,
    session_id: &str,
    client_quote_id: &str,
) -> SubmitOptionRfqQuoteInput {
    SubmitOptionRfqQuoteInput {
        mm_account,
        session_id: Some(session_id.to_string()),
        client_quote_id: Some(client_quote_id.to_string()),
        price_1e8: 1_000_000_000,
        size_1e8: 100_000_000,
        quote_nonce: None,
        quote_ttl_ms: Some(100),
        signature: None,
    }
}

fn option_rfq_quote_message(
    option_rfq_id: uuid::Uuid,
    price_1e8: &str,
    size_1e8: &str,
) -> ClientMessage {
    serde_json::from_value(json!({
        "type": "option_rfq_quote",
        "request_id": "mm-option-quote-1",
        "payload": {
            "option_rfq_id": option_rfq_id,
            "mm_account": "0x0000000000000000000000000000000000000001",
            "price_1e8": price_1e8,
            "size_1e8": size_1e8,
            "client_quote_id": "mm-option-rfq-quote-001",
            "quote_ttl_ms": 100
        }
    }))
    .unwrap()
}

fn option_rfq_quote_message_with_signature(
    option_rfq_id: uuid::Uuid,
    quote_nonce: u64,
    signature: &str,
) -> ClientMessage {
    serde_json::from_value(json!({
        "type": "option_rfq_quote",
        "request_id": "mm-option-quote-1",
        "payload": {
            "option_rfq_id": option_rfq_id,
            "mm_account": signing_account().0,
            "price_1e8": "1000000000",
            "size_1e8": "100000000",
            "client_quote_id": "mm-option-rfq-signed-quote-001",
            "quote_nonce": quote_nonce,
            "quote_ttl_ms": 100,
            "signature": signature
        }
    }))
    .unwrap()
}

async fn gateway_option_quote_payload_digest(
    state: &AppState,
    option_rfq_id: uuid::Uuid,
    quote_nonce: u64,
) -> String {
    option_rfq_quote_signing_payload(
        state,
        OptionRfqQuoteSigningPayloadInput {
            option_rfq_id,
            mm_account: signing_account(),
            price_1e8: 1_000_000_000,
            size_1e8: 100_000_000,
            quote_nonce,
            quote_ttl_ms: 100,
        },
    )
    .await
    .unwrap()
    .digest
}

fn signing_account() -> AccountId {
    AccountId::new(test_account())
}

fn sign_option_quote_digest(digest: &str, signing_key: SigningKey) -> String {
    let digest = parse_digest(digest);
    let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&signature.to_bytes());
    bytes.push(recovery_id.to_byte() + 27);
    format!("0x{}", hex_encode(&bytes))
}

fn test_account() -> String {
    let verifying_key = test_signing_key().verifying_key().to_encoded_point(false);
    let hash = Keccak256::digest(&verifying_key.as_bytes()[1..]);
    format!("0x{}", hex_encode(&hash[12..]))
}

fn test_signing_key() -> SigningKey {
    signing_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318")
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
