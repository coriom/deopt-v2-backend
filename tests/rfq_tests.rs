use axum::body::to_bytes;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::mm::{AuthMode, MmSession, ServerMessage};
use deopt_v2_backend::rfq::service::{
    accept_quote, cancel_rfq, create_rfq, list_quotes, quote_signing_payload, submit_quote,
    CreateRfqInput, QuoteSigningPayloadInput, SubmitQuoteInput,
};
use deopt_v2_backend::rfq::{
    rfq_id_to_b256, rfq_quote_digest, RfqConfig, RfqQuote, RfqQuoteSignatureMode,
    RfqQuoteSignatureStatus, RfqQuoteSigningPayload, RfqQuoteStatus, RfqStatus,
};
use deopt_v2_backend::types::{AccountId, Side};
use k256::ecdsa::SigningKey;
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::time::Duration;
use tokio::sync::mpsc;
use tower::ServiceExt;

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

fn state() -> AppState {
    AppState::with_rfq_config(EngineState::with_default_markets(), rfq_config())
}

fn strict_state() -> AppState {
    let mut config = rfq_config();
    config.quote_signature_mode = RfqQuoteSignatureMode::Strict;
    AppState::with_rfq_config(EngineState::with_default_markets(), config)
}

fn taker() -> AccountId {
    AccountId::new("0x0000000000000000000000000000000000000001")
}

fn mm() -> AccountId {
    AccountId::new(test_account())
}

fn create_input(side: Side) -> CreateRfqInput {
    CreateRfqInput {
        taker: taker(),
        market_id: 1,
        side,
        size_1e8: 100_000_000,
        limit_price_1e8: Some(300_000_000_000),
        ttl_ms: Some(500),
    }
}

fn quote_input(rfq_id: uuid::Uuid) -> SubmitQuoteInput {
    SubmitQuoteInput {
        rfq_id,
        mm_account: mm(),
        session_id: None,
        client_quote_id: Some("quote-1".to_string()),
        price_1e8: 299_000_000_000,
        size_1e8: 100_000_000,
        quote_nonce: None,
        quote_ttl_ms: 100,
        signature: None,
    }
}

#[tokio::test]
async fn create_rfq_success() {
    let rfq = create_rfq(&state(), create_input(Side::Buy)).await.unwrap();

    assert_eq!(rfq.status, RfqStatus::Open);
    assert_eq!(rfq.taker, taker());
}

#[tokio::test]
async fn create_rfq_broadcasts_to_connected_mm_sessions() {
    let state = state();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let session = MmSession::with_ids(
        "session-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    state.mm_sessions.register(&session, sender).unwrap();

    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let message = receiver.recv().await.unwrap();

    let ServerMessage::RfqRequest(envelope) = message else {
        panic!("expected rfq_request");
    };
    assert_eq!(envelope.message_type, "rfq_request");
    assert_eq!(envelope.payload.rfq_id, rfq.rfq_id);
    assert_eq!(envelope.payload.size_1e8, "100000000");
}

#[tokio::test]
async fn create_rfq_succeeds_with_zero_connected_mm_sessions() {
    let state = state();

    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();

    assert_eq!(rfq.status, RfqStatus::Open);
    assert!(state.mm_sessions.list_active().unwrap().is_empty());
}

#[tokio::test]
async fn create_rfq_rejects_invalid_taker() {
    let mut input = create_input(Side::Buy);
    input.taker = AccountId::new("0xtaker");

    let error = create_rfq(&state(), input).await.unwrap_err();

    assert!(error.to_string().contains("malformed account address"));
}

#[tokio::test]
async fn create_rfq_rejects_invalid_side() {
    let response = router(state())
        .oneshot(json_post(
            "/rfqs",
            json!({
                "taker": taker().0,
                "market_id": 1,
                "side": "hold",
                "size_1e8": "100000000",
                "limit_price_1e8": "300000000000",
                "ttl_ms": 500
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn create_rfq_rejects_zero_size() {
    let mut input = create_input(Side::Buy);
    input.size_1e8 = 0;

    let error = create_rfq(&state(), input).await.unwrap_err();

    assert!(error.to_string().contains("zero size"));
}

#[tokio::test]
async fn create_rfq_caps_ttl() {
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(10_000);
    let rfq = create_rfq(&state(), input).await.unwrap();

    assert!(rfq.expires_at_ms - rfq.created_at_ms <= 1_000);
}

#[tokio::test]
async fn submit_quote_success() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();

    assert_eq!(quote.status, RfqQuoteStatus::Active);
    assert_eq!(quote.rfq_id, rfq.rfq_id);
    assert_eq!(quote.signature_status, RfqQuoteSignatureStatus::NotRequired);
}

#[test]
fn rfq_id_to_b256_is_deterministic() {
    let rfq_id = "a1bbb9bf-2f33-4686-9cdc-30e292ff391f";

    assert_eq!(rfq_id_to_b256(rfq_id), rfq_id_to_b256(rfq_id));
    assert_ne!(rfq_id_to_b256(rfq_id), rfq_id_to_b256("other-rfq-id"));
}

#[test]
fn rfq_quote_typehash_is_stable() {
    assert_eq!(
        format!(
            "0x{}",
            hex_encode(deopt_v2_backend::rfq::signing::rfq_quote_typehash().as_slice())
        ),
        "0x589aa31a96a086a541f2862ebe060acd4210af610b962e7e09e5e2124f81a8cc"
    );
}

#[test]
fn rfq_quote_digest_is_deterministic() {
    let payload = RfqQuoteSigningPayload {
        rfq_id: rfq_id_to_b256("rfq-1"),
        mm_account: mm(),
        market_id: 1,
        taker_is_buyer: true,
        price_1e8: 299_000_000_000,
        size_1e8: 100_000_000,
        quote_nonce: 7,
        expiry: 1_778_300_000,
    };
    let domain = RfqConfig::disabled().eip712_domain;

    assert_eq!(
        rfq_quote_digest(&payload, &domain).unwrap(),
        rfq_quote_digest(&payload, &domain).unwrap()
    );
}

#[tokio::test]
async fn quote_signing_payload_endpoint_returns_rfq_quote_fields() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let response = router(state)
        .oneshot(json_post(
            &format!("/rfqs/{}/quote-signing-payload", rfq.rfq_id),
            json!({
                "mm_account": mm().0,
                "price_1e8": "299000000000",
                "size_1e8": "100000000",
                "client_quote_id": "quote-1",
                "quote_nonce": 42,
                "quote_ttl_ms": 100
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["primary_type"], "RFQQuote");
    assert_eq!(json["message"]["mmAccount"], mm().0);
    assert_eq!(json["message"]["quoteNonce"], "42");
    assert!(json["digest"].as_str().unwrap().starts_with("0x"));
}

#[tokio::test]
async fn disabled_mode_accepts_unsigned_quote() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();

    assert_eq!(quote.status, RfqQuoteStatus::Active);
    assert_eq!(quote.signature_status, RfqQuoteSignatureStatus::NotRequired);
    assert!(quote.signature.is_none());
}

#[tokio::test]
async fn strict_mode_rejects_missing_signature() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.quote_nonce = Some(1);

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("signature is required"));
}

#[tokio::test]
async fn strict_mode_rejects_malformed_signature() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.quote_nonce = Some(1);
    input.signature = Some("not-a-signature".to_string());

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("malformed signature"));
}

#[tokio::test]
async fn strict_mode_rejects_invalid_signature() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.quote_nonce = Some(1);
    input.signature = Some(valid_signature_hex(0xaa));

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("signature"));
}

#[tokio::test]
async fn strict_mode_rejects_signer_mismatch() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.quote_nonce = Some(1);
    input.signature = Some(sign_quote_digest(
        &quote_payload_digest(&state, &rfq, &input).await,
        other_signing_key(),
    ));

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("signer does not match"));
}

#[tokio::test]
async fn strict_mode_accepts_valid_signature() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.quote_nonce = Some(11);
    input.signature = Some(sign_quote_digest(
        &quote_payload_digest(&state, &rfq, &input).await,
        test_signing_key(),
    ));

    let quote = submit_quote(&state, input).await.unwrap();

    assert_eq!(quote.status, RfqQuoteStatus::Active);
    assert_eq!(quote.signature_status, RfqQuoteSignatureStatus::Verified);
    assert_eq!(quote.recovered_signer, Some(mm()));
    assert_eq!(quote.quote_nonce.as_deref(), Some("11"));
    assert!(quote.quote_digest.unwrap().starts_with("0x"));
}

#[tokio::test]
async fn strict_mode_rejects_tampered_quote_after_signing() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.quote_nonce = Some(12);
    input.signature = Some(sign_quote_digest(
        &quote_payload_digest(&state, &rfq, &input).await,
        test_signing_key(),
    ));
    input.price_1e8 += 1;

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("signer does not match"));
}

#[tokio::test]
async fn http_quote_endpoint_stores_signature_metadata() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let app = router(state.clone());
    let payload = quote_signing_payload(
        &state,
        QuoteSigningPayloadInput {
            rfq_id: rfq.rfq_id,
            mm_account: mm(),
            price_1e8: 299_000_000_000,
            size_1e8: 100_000_000,
            quote_nonce: 21,
            quote_ttl_ms: 100,
        },
    )
    .await
    .unwrap();
    let signature = sign_quote_digest(&payload.digest, test_signing_key());

    let response = app
        .oneshot(json_post(
            &format!("/rfqs/{}/quotes", rfq.rfq_id),
            json!({
                "mm_account": mm().0,
                "price_1e8": "299000000000",
                "size_1e8": "100000000",
                "client_quote_id": "quote-http-signed",
                "quote_nonce": 21,
                "quote_ttl_ms": 100,
                "signature": signature
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["signature_status"], "verified");
    assert_eq!(json["recovered_signer"], mm().0);
    assert_eq!(json["quote_nonce"], "21");
    assert!(json["quote_digest"].as_str().unwrap().starts_with("0x"));
}

#[tokio::test]
async fn strict_acceptance_requires_verified_quote() {
    let state = strict_state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = RfqQuote {
        quote_id: uuid::Uuid::new_v4(),
        rfq_id: rfq.rfq_id,
        mm_account: mm(),
        session_id: None,
        client_quote_id: Some("forced-unverified".to_string()),
        price_1e8: 299_000_000_000,
        size_1e8: 100_000_000,
        status: RfqQuoteStatus::Active,
        created_at_ms: deopt_v2_backend::types::now_ms(),
        expires_at_ms: rfq.expires_at_ms,
        signature: None,
        quote_digest: None,
        quote_nonce: None,
        signature_status: RfqQuoteSignatureStatus::Missing,
        recovered_signer: None,
    };
    state.rfq_store.lock().unwrap().insert_quote(quote.clone());

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("quote signature is missing"));
}

#[tokio::test]
async fn submit_quote_rejects_expired_rfq() {
    let state = state();
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(1);
    let rfq = create_rfq(&state, input).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;

    let error = submit_quote(&state, quote_input(rfq.rfq_id))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("RFQ has expired"));
}

#[tokio::test]
async fn submit_quote_rejects_wrong_or_zero_price() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.price_1e8 = 0;

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("zero price"));
}

#[tokio::test]
async fn submit_quote_rejects_size_above_rfq_size() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.size_1e8 = rfq.size_1e8 + 1;

    let error = submit_quote(&state, input).await.unwrap_err();

    assert!(error.to_string().contains("quote size exceeds RFQ size"));
}

#[tokio::test]
async fn submit_quote_caps_quote_ttl_to_rfq_expiry() {
    let state = state();
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(20);
    let rfq = create_rfq(&state, input).await.unwrap();
    let mut quote = quote_input(rfq.rfq_id);
    quote.quote_ttl_ms = 500;
    let quote = submit_quote(&state, quote).await.unwrap();

    assert!(quote.expires_at_ms <= rfq.expires_at_ms);
}

#[tokio::test]
async fn list_quotes_returns_all_quotes() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    let mut second = quote_input(rfq.rfq_id);
    second.client_quote_id = Some("quote-2".to_string());
    second.mm_account = AccountId::new("0x0000000000000000000000000000000000000003");
    submit_quote(&state, second).await.unwrap();

    assert_eq!(list_quotes(&state, rfq.rfq_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn accept_quote_success() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    let accepted = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();

    assert_eq!(accepted.status, RfqStatus::Accepted);
    assert!(accepted.onchain_intent_id.starts_with("0x"));
}

#[tokio::test]
async fn accept_quote_sends_notification_when_session_connected() {
    let state = state();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let session = MmSession::with_ids(
        "session-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    state.mm_sessions.register(&session, sender).unwrap();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    receiver.recv().await.unwrap();
    let mut quote_input = quote_input(rfq.rfq_id);
    quote_input.session_id = Some("session-rfq-1".to_string());
    let quote = submit_quote(&state, quote_input).await.unwrap();

    let accepted = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();
    let message = receiver.recv().await.unwrap();

    assert!(accepted.mm_notification_sent);
    assert!(accepted.mm_notification_warning.is_none());
    let ServerMessage::RfqQuoteAccepted(envelope) = message else {
        panic!("expected rfq_quote_accepted");
    };
    assert_eq!(envelope.payload.rfq_id, rfq.rfq_id);
    assert_eq!(envelope.payload.quote_id, quote.quote_id);
    assert_eq!(
        envelope.payload.execution_intent_id,
        accepted.execution_intent_id
    );
}

#[tokio::test]
async fn accept_quote_still_succeeds_if_notification_fails() {
    let state = state();
    let (sender, receiver) = mpsc::unbounded_channel();
    let session = MmSession::with_ids(
        "session-rfq-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    );
    state.mm_sessions.register(&session, sender).unwrap();
    drop(receiver);
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut quote_input = quote_input(rfq.rfq_id);
    quote_input.session_id = Some("session-rfq-1".to_string());
    let quote = submit_quote(&state, quote_input).await.unwrap();

    let accepted = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();

    assert_eq!(accepted.status, RfqStatus::Accepted);
    assert!(!accepted.mm_notification_sent);
    assert!(accepted.mm_notification_warning.is_some());
    assert_eq!(state.engine.lock().unwrap().execution_intents().len(), 1);
}

#[tokio::test]
async fn accept_quote_rejects_expired_rfq() {
    let state = state();
    let mut input = create_input(Side::Buy);
    input.ttl_ms = Some(10);
    let rfq = create_rfq(&state, input).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(12)).await;

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("RFQ has expired"));
}

#[tokio::test]
async fn accept_quote_rejects_expired_quote() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut quote_input = quote_input(rfq.rfq_id);
    quote_input.quote_ttl_ms = 1;
    let quote = submit_quote(&state, quote_input).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("quote has expired"));
}

#[tokio::test]
async fn accept_quote_rejects_quote_from_different_rfq() {
    let state = state();
    let first = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let second = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(first.rfq_id))
        .await
        .unwrap();

    let error = accept_quote(&state, second.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("quote does not belong to RFQ"));
}

#[tokio::test]
async fn accept_quote_rejects_price_beyond_taker_limit() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let mut input = quote_input(rfq.rfq_id);
    input.price_1e8 = 301_000_000_000;
    let quote = submit_quote(&state, input).await.unwrap();

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("beyond taker limit"));
}

#[tokio::test]
async fn accept_quote_creates_execution_intent_with_correct_sides_for_taker_buy() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();

    let intents = state.engine.lock().unwrap().execution_intents();
    assert_eq!(intents[0].buyer, taker());
    assert_eq!(intents[0].seller, mm());
    assert_eq!(intents[0].buyer_is_maker, Some(false));
}

#[tokio::test]
async fn accept_quote_creates_execution_intent_with_correct_sides_for_taker_sell() {
    let state = state();
    let mut input = create_input(Side::Sell);
    input.limit_price_1e8 = Some(299_000_000_000);
    let rfq = create_rfq(&state, input).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap();

    let intents = state.engine.lock().unwrap().execution_intents();
    assert_eq!(intents[0].buyer, mm());
    assert_eq!(intents[0].seller, taker());
    assert_eq!(intents[0].buyer_is_maker, Some(true));
}

#[tokio::test]
async fn accept_quote_is_single_winner() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let first = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    let mut second = quote_input(rfq.rfq_id);
    second.client_quote_id = Some("quote-2".to_string());
    second.mm_account = AccountId::new("0x0000000000000000000000000000000000000003");
    let second = submit_quote(&state, second).await.unwrap();
    accept_quote(&state, rfq.rfq_id, first.quote_id)
        .await
        .unwrap();

    let error = accept_quote(&state, rfq.rfq_id, second.quote_id)
        .await
        .unwrap_err();
    let quotes = list_quotes(&state, rfq.rfq_id).await.unwrap();

    assert!(error.to_string().contains("RFQ is accepted"));
    assert!(
        quotes
            .iter()
            .any(|quote| quote.quote_id == second.quote_id
                && quote.status == RfqQuoteStatus::Rejected)
    );
}

#[tokio::test]
async fn cancel_rfq_prevents_quote_acceptance() {
    let state = state();
    let rfq = create_rfq(&state, create_input(Side::Buy)).await.unwrap();
    let quote = submit_quote(&state, quote_input(rfq.rfq_id)).await.unwrap();
    cancel_rfq(&state, rfq.rfq_id).await.unwrap();

    let error = accept_quote(&state, rfq.rfq_id, quote.quote_id)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("RFQ is cancelled"));
}

fn json_post(uri: &str, value: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn quote_payload_digest(
    state: &AppState,
    rfq: &deopt_v2_backend::rfq::RfqRequest,
    input: &SubmitQuoteInput,
) -> String {
    quote_signing_payload(
        state,
        QuoteSigningPayloadInput {
            rfq_id: rfq.rfq_id,
            mm_account: input.mm_account.clone(),
            price_1e8: input.price_1e8,
            size_1e8: input.size_1e8,
            quote_nonce: input.quote_nonce.unwrap(),
            quote_ttl_ms: input.quote_ttl_ms,
        },
    )
    .await
    .unwrap()
    .digest
}

fn sign_quote_digest(digest: &str, signing_key: SigningKey) -> String {
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
