//! SUBACCOUNTS-MM-GATEWAY-QUOTES-V1 — MM Gateway subaccount routing.
//!
//! These tests pin the maker-subaccount contract for the WebTransport
//! `option_rfq_quote` message:
//!
//! * omitted `maker_subaccount_id` → defaults to Account 1
//!   (backward-compatible with pre-V1 clients);
//! * explicit `maker_subaccount_id = 1` → works;
//! * explicit `maker_subaccount_id = 2` for a maker owning
//!   Account 2 → works and persists id `2`;
//! * explicit `maker_subaccount_id = 0` → rejected
//!   (`CHECK (>= 1)` invariant);
//! * unknown / cross-account subaccount → rejected with
//!   `OPTION_RFQ_QUOTE_REJECTED` (the `(owner, id)` composite key
//!   guarantees the maker cannot address another wallet's subaccount);
//! * standard RFQ quote path stays unchanged for legacy callers;
//! * lifecycle event carries the resolved `maker_subaccount_id`.
//!
//! No signatures, no admin tokens, no secrets appear on any wire
//! observed by these tests.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::mm::protocol::{ClientMessage, ServerMessage};
use deopt_v2_backend::mm::{AuthMode, MmGatewayConfig, MmGatewayService, MmSession};
use deopt_v2_backend::options::service::{
    create_option_rfq, create_option_series, list_option_rfq_quotes, CreateOptionRfqInput,
    CreateOptionSeriesInput,
};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use serde_json::json;

const MM_ACCOUNT_HEX: &str = "0x0000000000000000000000000000000000000001";
const OTHER_ACCOUNT_HEX: &str = "0x0000000000000000000000000000000000000099";

fn option_rfq_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.rfq_enabled = true;
    config.rfq_min_quote_ttl_ms = 1;
    config.rfq_max_quote_ttl_ms = 500;
    AppState::with_options_config(EngineState::with_default_markets(), config)
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

async fn active_option_series_id(state: &AppState) -> String {
    create_option_series(state, option_series_input())
        .await
        .unwrap()
        .option_series_id
}

fn option_rfq_input(option_series_id: String, side: Side) -> CreateOptionRfqInput {
    CreateOptionRfqInput {
        taker: AccountId::new("0x0000000000000000000000000000000000000003"),
        taker_subaccount_id: 1,
        option_series_id,
        side,
        size_1e8: 100_000_000,
        limit_price_1e8: Some(1_100_000_000),
        ttl_ms: Some(500),
    }
}

fn base_payload(option_rfq_id: uuid::Uuid) -> serde_json::Value {
    json!({
        "option_rfq_id": option_rfq_id,
        "mm_account": MM_ACCOUNT_HEX,
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "client_quote_id": "mm-option-rfq-quote-sub-v1",
        "quote_ttl_ms": 100
    })
}

fn message_with_payload(payload: serde_json::Value) -> ClientMessage {
    serde_json::from_value(json!({
        "type": "option_rfq_quote",
        "request_id": "mm-option-quote-sub-v1",
        "payload": payload,
    }))
    .unwrap()
}

fn message_without_subaccount(option_rfq_id: uuid::Uuid) -> ClientMessage {
    message_with_payload(base_payload(option_rfq_id))
}

fn message_with_subaccount(option_rfq_id: uuid::Uuid, sub: u32) -> ClientMessage {
    let mut payload = base_payload(option_rfq_id);
    payload["maker_subaccount_id"] = json!(sub);
    message_with_payload(payload)
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

fn fresh_session() -> MmSession {
    MmSession::with_ids(
        "session-sub-mm-1",
        "connection-1",
        10,
        AuthMode::Disabled,
        true,
    )
}

async fn setup_rfq() -> (AppState, uuid::Uuid) {
    let state = option_rfq_state();
    let option_series_id = active_option_series_id(&state).await;
    let rfq = create_option_rfq(&state, option_rfq_input(option_series_id, Side::Buy))
        .await
        .unwrap();
    (state, rfq.option_rfq_id)
}

#[tokio::test]
async fn omitted_maker_subaccount_id_defaults_to_one() {
    let (state, option_rfq_id) = setup_rfq().await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    let response = service
        .handle_message(&mut session, message_without_subaccount(option_rfq_id), 20)
        .await;

    let ServerMessage::OptionRfqQuoteResult(envelope) = response else {
        panic!("expected option_rfq_quote_result, got: {response:?}");
    };
    assert_eq!(envelope.payload.option_rfq_id, option_rfq_id);

    let quotes = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(
        quotes[0].maker_subaccount_id, 1,
        "omitted field must default to Account 1"
    );
}

#[tokio::test]
async fn explicit_maker_subaccount_id_one_is_accepted() {
    let (state, option_rfq_id) = setup_rfq().await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    let response = service
        .handle_message(&mut session, message_with_subaccount(option_rfq_id, 1), 20)
        .await;

    let ServerMessage::OptionRfqQuoteResult(_) = response else {
        panic!("expected option_rfq_quote_result, got: {response:?}");
    };
    let quotes = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].maker_subaccount_id, 1);
}

#[tokio::test]
async fn explicit_maker_subaccount_id_two_routes_when_owned() {
    let (state, option_rfq_id) = setup_rfq().await;
    let mm_account = AccountId::new(MM_ACCOUNT_HEX);
    seed_subaccount_two(&state, &mm_account).await;

    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    let response = service
        .handle_message(&mut session, message_with_subaccount(option_rfq_id, 2), 20)
        .await;

    let ServerMessage::OptionRfqQuoteResult(_) = response else {
        panic!("expected option_rfq_quote_result, got: {response:?}");
    };

    let quotes = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(
        quotes[0].maker_subaccount_id, 2,
        "quote must persist the routed maker subaccount id"
    );
    assert_eq!(quotes[0].mm_account, mm_account);
}

#[tokio::test]
async fn maker_subaccount_id_zero_is_rejected() {
    let (state, option_rfq_id) = setup_rfq().await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    let response = service
        .handle_message(&mut session, message_with_subaccount(option_rfq_id, 0), 20)
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
    assert!(
        value["error"]["message"].as_str().unwrap().contains(">= 1"),
        "rejection must cite the >= 1 invariant, got: {value}"
    );

    let quotes = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    assert!(quotes.is_empty(), "rejected quote must not be persisted");
}

#[tokio::test]
async fn unknown_maker_subaccount_is_rejected() {
    let (state, option_rfq_id) = setup_rfq().await;
    // No subaccount 2 is seeded for `MM_ACCOUNT_HEX` — the lookup
    // must return SubaccountNotFound and route back as an
    // OPTION_RFQ_QUOTE_REJECTED error.
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    let response = service
        .handle_message(&mut session, message_with_subaccount(option_rfq_id, 99), 20)
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
            .contains("subaccount"),
        "rejection must reference the missing subaccount, got: {value}"
    );

    let quotes = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    assert!(quotes.is_empty());
}

#[tokio::test]
async fn cross_account_maker_subaccount_is_rejected() {
    let (state, option_rfq_id) = setup_rfq().await;
    // Seed subaccount 2 for a DIFFERENT wallet. The authenticated
    // session for `MM_ACCOUNT_HEX` must NOT be able to reach it —
    // the (owner, id) composite key produces `SubaccountNotFound`
    // relative to the caller's identity.
    let other = AccountId::new(OTHER_ACCOUNT_HEX);
    seed_subaccount_two(&state, &other).await;

    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    let response = service
        .handle_message(&mut session, message_with_subaccount(option_rfq_id, 2), 20)
        .await;

    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "OPTION_RFQ_QUOTE_REJECTED");

    let quotes = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    assert!(
        quotes.is_empty(),
        "cross-account subaccount attempt must not persist a quote"
    );
}

#[tokio::test]
async fn resolved_subaccount_flows_into_persisted_quote_and_session_id() {
    let (state, option_rfq_id) = setup_rfq().await;
    let mm_account = AccountId::new(MM_ACCOUNT_HEX);
    seed_subaccount_two(&state, &mm_account).await;

    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();
    let expected_session_id = session.session_id.clone();

    let _ = service
        .handle_message(&mut session, message_with_subaccount(option_rfq_id, 2), 20)
        .await;

    let quotes = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].maker_subaccount_id, 2);
    assert_eq!(
        quotes[0].session_id.as_deref(),
        Some(expected_session_id.as_str())
    );
}

#[tokio::test]
async fn legacy_omitted_field_regression_still_persists_id_one() {
    // Regression proof: an MM client that pre-dates
    // SUBACCOUNTS-MM-GATEWAY-QUOTES-V1 sends no `maker_subaccount_id`
    // field at all. The wire must remain byte-identical to before.
    let (state, option_rfq_id) = setup_rfq().await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());
    let mut session = fresh_session();

    // Two back-to-back legacy quotes on separate RFQs to prove
    // repeatability, both routing to Account 1.
    let option_series_id = active_option_series_id(&state).await;
    let rfq_b = create_option_rfq(&state, {
        let mut input = option_rfq_input(option_series_id, Side::Sell);
        input.limit_price_1e8 = Some(900_000_000);
        input
    })
    .await
    .unwrap();

    let _ = service
        .handle_message(&mut session, message_without_subaccount(option_rfq_id), 20)
        .await;
    let _ = service
        .handle_message(
            &mut session,
            message_without_subaccount(rfq_b.option_rfq_id),
            21,
        )
        .await;

    let quotes_a = list_option_rfq_quotes(&state, option_rfq_id).await.unwrap();
    let quotes_b = list_option_rfq_quotes(&state, rfq_b.option_rfq_id)
        .await
        .unwrap();
    assert_eq!(quotes_a[0].maker_subaccount_id, 1);
    assert_eq!(quotes_b[0].maker_subaccount_id, 1);
}

#[test]
fn payload_deserializes_without_maker_subaccount_id() {
    // Wire-compat proof: the protocol must accept payloads that
    // omit the field entirely.
    let option_rfq_id = uuid::Uuid::new_v4();
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "option_rfq_quote",
        "request_id": "legacy-1",
        "payload": {
            "option_rfq_id": option_rfq_id,
            "mm_account": MM_ACCOUNT_HEX,
            "price_1e8": "1000000000",
            "size_1e8": "100000000",
            "quote_ttl_ms": 100
        }
    }))
    .expect("legacy payload must still deserialize");
    let ClientMessage::OptionRfqQuote(envelope) = message else {
        panic!("expected option_rfq_quote");
    };
    assert_eq!(envelope.payload.maker_subaccount_id, None);
}

#[test]
fn payload_deserializes_with_maker_subaccount_id() {
    let option_rfq_id = uuid::Uuid::new_v4();
    let message: ClientMessage = serde_json::from_value(json!({
        "type": "option_rfq_quote",
        "request_id": "v1-1",
        "payload": {
            "option_rfq_id": option_rfq_id,
            "mm_account": MM_ACCOUNT_HEX,
            "price_1e8": "1000000000",
            "size_1e8": "100000000",
            "quote_ttl_ms": 100,
            "maker_subaccount_id": 3
        }
    }))
    .expect("payload with subaccount must deserialize");
    let ClientMessage::OptionRfqQuote(envelope) = message else {
        panic!("expected option_rfq_quote");
    };
    assert_eq!(envelope.payload.maker_subaccount_id, Some(3));
}
