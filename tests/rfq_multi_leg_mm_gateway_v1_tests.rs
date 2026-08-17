//! RFQ-MULTI-LEG-MM-GATEWAY-V1 — MM Gateway multi-leg RFQ package
//! quote submit + maker-side cancel + accept-time push.
//!
//! HTTP-layer multi-leg quote / accept / cancel are exercised by the
//! milestone-specific test suites; these tests focus on the WT
//! (WebTransport) surface + push notifications.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::mm::protocol::{ClientMessage, ErrorCode, ServerMessage};
use deopt_v2_backend::mm::{AuthMode, MmGatewayConfig, MmGatewayService, MmSession};
use deopt_v2_backend::options::multi_leg_service::{
    accept_option_multi_leg_rfq_quote, create_option_multi_leg_rfq, get_option_multi_leg_rfq,
    get_option_multi_leg_rfq_quote, AcceptOptionMultiLegRfqQuoteInput,
    CreateOptionMultiLegRfqInput, LegInput,
};
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::{
    OptionMultiLegRfqQuoteStatus, OptionMultiLegRfqStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use serde_json::json;

const MM_ACCOUNT_HEX: &str = "0x0000000000000000000000000000000000000001";
const OTHER_ACCOUNT_HEX: &str = "0x0000000000000000000000000000000000000099";
const TAKER_HEX: &str = "0x0000000000000000000000000000000000000003";

fn taker() -> AccountId {
    AccountId::new(TAKER_HEX)
}

fn mm() -> AccountId {
    AccountId::new(MM_ACCOUNT_HEX)
}

fn state_with_flag(flag: bool) -> AppState {
    // NOTE: `rfq_max_quote_ttl_ms` intentionally generous. Historically 500 ms,
    // which caused `part3_maker_cannot_cancel_accepted_quote` to flake under
    // parallel workspace load with "multi-leg option RFQ quote is not active".
    let mut cfg = OptionsConfig::enabled_in_memory_for_tests();
    cfg.rfq_enabled = true;
    cfg.rfq_min_quote_ttl_ms = 1;
    cfg.rfq_max_quote_ttl_ms = 30_000;
    cfg.rfq_multi_leg_enabled = flag;
    AppState::with_options_config(EngineState::with_default_markets(), cfg)
}

async fn seed_series(state: &AppState, salt: u128) -> String {
    let expiry = u64::try_from(now_ms() / 1000).unwrap() + 86_400 + salt as u64;
    create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "ETH".to_string(),
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry,
            strike_1e8: 300_000_000_000 + salt,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .unwrap()
    .option_series_id
}

fn leg(index: u32, series: &str, side: Side) -> LegInput {
    LegInput {
        leg_index: index,
        option_series_id: series.to_string(),
        side,
        size_1e8: 100_000_000,
        ratio_num: 1,
        ratio_den: 1,
    }
}

async fn seed_open_rfq(state: &AppState) -> uuid::Uuid {
    let series = seed_series(state, 0).await;
    let (rfq, _) = create_option_multi_leg_rfq(
        state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(30_000),
        },
    )
    .await
    .unwrap();
    rfq.option_rfq_id
}

async fn seed_subaccount_two(state: &AppState, owner: &AccountId) {
    let _ =
        deopt_v2_backend::subaccounts::ensure_default_subaccount(state.subaccounts.as_ref(), owner)
            .await;
    let created =
        deopt_v2_backend::subaccounts::create_subaccount(state.subaccounts.as_ref(), owner, None)
            .await
            .expect("allocate subaccount 2");
    assert_eq!(created.subaccount_id, 2);
}

fn quote_message(rfq_id: uuid::Uuid, extra: Option<serde_json::Value>) -> ClientMessage {
    // quote_ttl_ms deliberately generous so downstream accept steps are not
    // racing wall-clock expiry under parallel workspace load.
    let mut payload = json!({
        "option_rfq_id": rfq_id,
        "mm_account": MM_ACCOUNT_HEX,
        "package_price_1e8": "50000000",
        "size_1e8": "100000000",
        "legs": [
            { "leg_index": 0, "price_1e8": "12000000000" },
            { "leg_index": 1, "price_1e8": "11500000000" }
        ],
        "quote_ttl_ms": 10_000
    });
    if let (Some(obj), Some(extra_obj)) = (
        payload.as_object_mut(),
        extra.and_then(|v| v.as_object().cloned()),
    ) {
        for (k, v) in extra_obj {
            obj.insert(k, v);
        }
    }
    serde_json::from_value(json!({
        "type": "option_multi_leg_rfq_quote",
        "request_id": "mm-multi-leg-quote-1",
        "payload": payload,
    }))
    .unwrap()
}

fn cancel_message(rfq_id: uuid::Uuid, quote_id: uuid::Uuid) -> ClientMessage {
    serde_json::from_value(json!({
        "type": "option_multi_leg_rfq_quote_cancel",
        "request_id": "mm-multi-leg-cancel-1",
        "payload": {
            "option_rfq_id": rfq_id,
            "quote_id": quote_id,
            "mm_account": MM_ACCOUNT_HEX,
        }
    }))
    .unwrap()
}

fn fresh_session() -> MmSession {
    MmSession::with_ids(
        "session-mm-multi-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    )
}

// ---------------------------------------------------------------------
// Flag gate.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part1_flag_off_blocks_quote_submit() {
    let state = state_with_flag(false);
    // Even without an RFQ, the flag-off gate must fire first: the
    // handler will call the service, which returns
    // OptionMultiLegRfqNotLive before touching persistence.
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(&mut session, quote_message(uuid::Uuid::new_v4(), None), 20)
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(
        value["error"]["code"],
        "OPTION_MULTI_LEG_RFQ_QUOTE_REJECTED"
    );
    // The error message must be a truthful bounded label, no secrets.
    let msg = value["error"]["message"].as_str().unwrap();
    assert!(msg.to_ascii_lowercase().contains("multi-leg"));
    assert!(!msg.to_ascii_lowercase().contains("0x"));
}

#[tokio::test]
async fn part1_flag_off_blocks_maker_cancel() {
    let state = state_with_flag(false);
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(
            &mut session,
            cancel_message(uuid::Uuid::new_v4(), uuid::Uuid::new_v4()),
            20,
        )
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(
        value["error"]["code"],
        "OPTION_MULTI_LEG_RFQ_QUOTE_CANCEL_REJECTED"
    );
}

// ---------------------------------------------------------------------
// Quote submit — happy path + subaccount handling.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_happy_path_submit_returns_result_and_persists_session_id() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let session_id = session.session_id.clone();

    let response = service
        .handle_message(&mut session, quote_message(rfq_id, None), 20)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = response else {
        panic!("expected option_multi_leg_rfq_quote_result, got: {response:?}");
    };
    assert_eq!(envelope.payload.option_rfq_id, rfq_id);
    assert_eq!(envelope.payload.legs_count, 2);
    assert_eq!(
        envelope.payload.status,
        OptionMultiLegRfqQuoteStatus::Active
    );

    let (persisted_quote, _) = get_option_multi_leg_rfq_quote(&state, envelope.payload.quote_id)
        .await
        .unwrap();
    assert_eq!(
        persisted_quote.session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(persisted_quote.maker_subaccount_id, 1);
}

#[tokio::test]
async fn part2_omitted_maker_subaccount_defaults_to_one() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(&mut session, quote_message(rfq_id, None), 20)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = response else {
        panic!("expected result");
    };
    let (persisted_quote, _) = get_option_multi_leg_rfq_quote(&state, envelope.payload.quote_id)
        .await
        .unwrap();
    assert_eq!(persisted_quote.maker_subaccount_id, 1);
}

#[tokio::test]
async fn part2_explicit_maker_subaccount_two_routes_when_owned() {
    let state = state_with_flag(true);
    seed_subaccount_two(&state, &mm()).await;
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(
            &mut session,
            quote_message(rfq_id, Some(json!({"maker_subaccount_id": 2}))),
            20,
        )
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = response else {
        panic!("expected result, got: {response:?}");
    };
    let (persisted_quote, _) = get_option_multi_leg_rfq_quote(&state, envelope.payload.quote_id)
        .await
        .unwrap();
    assert_eq!(persisted_quote.maker_subaccount_id, 2);
}

#[tokio::test]
async fn part2_zero_maker_subaccount_rejects() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(
            &mut session,
            quote_message(rfq_id, Some(json!({"maker_subaccount_id": 0}))),
            20,
        )
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(
        value["error"]["code"],
        "OPTION_MULTI_LEG_RFQ_QUOTE_REJECTED"
    );
    assert!(value["error"]["message"].as_str().unwrap().contains(">= 1"));
}

#[tokio::test]
async fn part2_unknown_maker_subaccount_rejects() {
    let state = state_with_flag(true);
    // No subaccount 2 seeded for the maker.
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(
            &mut session,
            quote_message(rfq_id, Some(json!({"maker_subaccount_id": 99}))),
            20,
        )
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(
        value["error"]["code"],
        "OPTION_MULTI_LEG_RFQ_QUOTE_REJECTED"
    );
}

#[tokio::test]
async fn part2_cross_account_maker_subaccount_rejects() {
    // A subaccount 2 belonging to a DIFFERENT wallet is unreachable
    // from the authenticated MM session.
    let state = state_with_flag(true);
    seed_subaccount_two(&state, &AccountId::new(OTHER_ACCOUNT_HEX)).await;
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(
            &mut session,
            quote_message(rfq_id, Some(json!({"maker_subaccount_id": 2}))),
            20,
        )
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(
        value["error"]["code"],
        "OPTION_MULTI_LEG_RFQ_QUOTE_REJECTED"
    );
}

#[tokio::test]
async fn part2_wt_quote_visible_to_http_read_and_cannot_create_fill() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let response = service
        .handle_message(&mut session, quote_message(rfq_id, None), 20)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = response else {
        panic!("expected result");
    };
    // HTTP read sees the WT-submitted quote.
    let (loaded, legs) = get_option_multi_leg_rfq_quote(&state, envelope.payload.quote_id)
        .await
        .unwrap();
    assert_eq!(loaded.status, OptionMultiLegRfqQuoteStatus::Active);
    assert_eq!(legs.len(), 2);
    // Submitting a quote does NOT create a fill.
    let (rfq, _) = get_option_multi_leg_rfq(&state, rfq_id).await.unwrap();
    assert!(rfq.accepted_fill_id.is_none());
}

// ---------------------------------------------------------------------
// Maker cancel.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part3_maker_cancels_own_active_quote() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    let submit_response = service
        .handle_message(&mut session, quote_message(rfq_id, None), 20)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = submit_response else {
        panic!("expected result");
    };
    let quote_id = envelope.payload.quote_id;

    let cancel_response = service
        .handle_message(&mut session, cancel_message(rfq_id, quote_id), 30)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteCancelResult(envelope) = cancel_response else {
        panic!("expected cancel result, got: {cancel_response:?}");
    };
    assert_eq!(envelope.payload.quote_id, quote_id);
    assert_eq!(
        envelope.payload.status,
        OptionMultiLegRfqQuoteStatus::Cancelled
    );

    // RFQ status is unchanged.
    let (rfq, _) = get_option_multi_leg_rfq(&state, rfq_id).await.unwrap();
    assert_eq!(rfq.status, OptionMultiLegRfqStatus::Open);
}

#[tokio::test]
async fn part3_maker_cannot_cancel_someone_elses_quote() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let submit_response = service
        .handle_message(&mut session, quote_message(rfq_id, None), 20)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = submit_response else {
        panic!("expected result");
    };
    let quote_id = envelope.payload.quote_id;

    // A different MM session tries to cancel it.
    let other_service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut other_session = MmSession::with_ids(
        "session-other",
        "connection-2",
        10,
        AuthMode::Disabled,
        true,
    );
    let msg: ClientMessage = serde_json::from_value(json!({
        "type": "option_multi_leg_rfq_quote_cancel",
        "request_id": "mm-multi-leg-cancel-other",
        "payload": {
            "option_rfq_id": rfq_id,
            "quote_id": quote_id,
            "mm_account": OTHER_ACCOUNT_HEX,
        }
    }))
    .unwrap();
    let response = other_service
        .handle_message(&mut other_session, msg, 40)
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(
        value["error"]["code"],
        "OPTION_MULTI_LEG_RFQ_QUOTE_CANCEL_REJECTED"
    );

    // Original quote unaffected.
    let (persisted_quote, _) = get_option_multi_leg_rfq_quote(&state, quote_id)
        .await
        .unwrap();
    assert_eq!(persisted_quote.status, OptionMultiLegRfqQuoteStatus::Active);
}

#[tokio::test]
async fn part3_maker_cannot_cancel_accepted_quote() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let submit_response = service
        .handle_message(&mut session, quote_message(rfq_id, None), 20)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = submit_response else {
        panic!("expected result");
    };
    let quote_id = envelope.payload.quote_id;
    let (quote, quote_legs) = get_option_multi_leg_rfq_quote(&state, quote_id)
        .await
        .unwrap();

    // Taker accepts the quote via the service directly.
    let _ = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: quote.package_price_1e8.clone(),
            expected_legs_count: quote_legs.len() as u32,
            expected_leg_prices_1e8: quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();

    // Attempt to cancel the accepted quote.
    let response = service
        .handle_message(&mut session, cancel_message(rfq_id, quote_id), 40)
        .await;
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(
        value["error"]["code"],
        "OPTION_MULTI_LEG_RFQ_QUOTE_CANCEL_REJECTED"
    );
}

// ---------------------------------------------------------------------
// Accept-time push to the maker session.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part4_accept_pushes_option_multi_leg_rfq_quote_accepted_to_maker_session() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    // Register the session so `send_to_session` has a receiver.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ServerMessage>();
    service.register_session(&session, tx).unwrap();

    let submit_response = service
        .handle_message(&mut session, quote_message(rfq_id, None), 20)
        .await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = submit_response else {
        panic!("expected result");
    };
    let quote_id = envelope.payload.quote_id;
    let (quote, quote_legs) = get_option_multi_leg_rfq_quote(&state, quote_id)
        .await
        .unwrap();

    // Drain the submit-result envelope from the session tx queue if any.
    // (register_session does not auto-send the submit result; it only
    // registers.)
    while let Ok(_) = rx.try_recv() {}

    // Taker accepts.
    let outcome = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: quote.package_price_1e8.clone(),
            expected_legs_count: quote_legs.len() as u32,
            expected_leg_prices_1e8: quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();

    // Poll for the accepted push.
    let mut saw_accepted = false;
    while let Ok(msg) = rx.try_recv() {
        if let ServerMessage::OptionMultiLegRfqQuoteAccepted(envelope) = msg {
            saw_accepted = true;
            let json = serde_json::to_string(&envelope).unwrap();
            assert_eq!(envelope.payload.option_rfq_id, rfq_id);
            assert_eq!(envelope.payload.quote_id, quote_id);
            assert_eq!(envelope.payload.fill_id, outcome.fill.fill_id);
            assert_eq!(envelope.payload.legs_count, 2);
            // No secrets on the wire.
            assert!(!json.contains("signature"));
            assert!(!json.contains("nonce"));
            assert!(!json.contains("authorization"));
        }
    }
    assert!(
        saw_accepted,
        "accept must push OptionMultiLegRfqQuoteAccepted"
    );
}

// ---------------------------------------------------------------------
// ClientMessage protocol parsing.
// ---------------------------------------------------------------------

#[test]
fn part5_client_message_parses_option_multi_leg_rfq_quote() {
    let rfq_id = uuid::Uuid::new_v4();
    let msg = json!({
        "type": "option_multi_leg_rfq_quote",
        "request_id": "req-1",
        "payload": {
            "option_rfq_id": rfq_id,
            "mm_account": MM_ACCOUNT_HEX,
            "package_price_1e8": "50000000",
            "size_1e8": "100000000",
            "legs": [{ "leg_index": 0, "price_1e8": "12000000000" }],
            "quote_ttl_ms": 100
        }
    });
    let parsed: ClientMessage = serde_json::from_value(msg).unwrap();
    match parsed {
        ClientMessage::OptionMultiLegRfqQuote(envelope) => {
            assert_eq!(envelope.request_id, "req-1");
            assert_eq!(envelope.payload.option_rfq_id, rfq_id);
            assert_eq!(envelope.payload.legs.len(), 1);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn part5_client_message_parses_option_multi_leg_rfq_quote_cancel() {
    let rfq_id = uuid::Uuid::new_v4();
    let quote_id = uuid::Uuid::new_v4();
    let msg = json!({
        "type": "option_multi_leg_rfq_quote_cancel",
        "request_id": "req-c",
        "payload": {
            "option_rfq_id": rfq_id,
            "quote_id": quote_id,
            "mm_account": MM_ACCOUNT_HEX,
            "maker_subaccount_id": 2
        }
    });
    let parsed: ClientMessage = serde_json::from_value(msg).unwrap();
    match parsed {
        ClientMessage::OptionMultiLegRfqQuoteCancel(envelope) => {
            assert_eq!(envelope.payload.option_rfq_id, rfq_id);
            assert_eq!(envelope.payload.quote_id, quote_id);
            assert_eq!(envelope.payload.maker_subaccount_id, Some(2));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn part5_error_codes_serialize_expected_strings() {
    // The wire strings feed error-code assertions in downstream
    // consumers (frontend, ops); pin them.
    let e1 = ErrorCode::OptionMultiLegRfqQuoteRejected;
    let e2 = ErrorCode::OptionMultiLegRfqQuoteCancelRejected;
    assert_eq!(
        serde_json::to_value(e1).unwrap(),
        "OPTION_MULTI_LEG_RFQ_QUOTE_REJECTED"
    );
    assert_eq!(
        serde_json::to_value(e2).unwrap(),
        "OPTION_MULTI_LEG_RFQ_QUOTE_CANCEL_REJECTED"
    );
}
