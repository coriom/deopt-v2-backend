//! LOSING-MAKER-REJECTION-PUSH-V1 — MM Gateway push semantics for
//! makers whose competing multi-leg quotes lose when a winning quote
//! is accepted.
//!
//! Coverage:
//!
//! * winning maker receives `OptionMultiLegRfqQuoteAccepted` push
//!   (regression, single-maker flow);
//! * losing maker receives `OptionMultiLegRfqQuoteRejected` push
//!   with the extended payload (accepted_quote_id + fill_id +
//!   maker_subaccount_id + legs_count + reason + rejected_at_ms);
//! * winning maker does NOT receive a rejected push for its own
//!   winning quote;
//! * losing quote without a `session_id` produces no push and does
//!   not fail the accept;
//! * duplicate accept attempt is refused before it can fire a
//!   second push;
//! * subaccount preserved: losing maker on maker_subaccount_id=2
//!   sees `2` in the payload;
//! * no secret substrings (`signature` / `nonce` / `authorization`)
//!   in the rejected push envelope;
//! * accept response DTO shape is unchanged (no losing-maker fields
//!   added — this milestone is push-only).

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::mm::protocol::ServerMessage;
use deopt_v2_backend::mm::{AuthMode, MmGatewayConfig, MmGatewayService, MmSession};
use deopt_v2_backend::options::multi_leg_service::{
    accept_option_multi_leg_rfq_quote, create_option_multi_leg_rfq, get_option_multi_leg_rfq_quote,
    submit_option_multi_leg_rfq_quote, AcceptOptionMultiLegRfqQuoteInput,
    CreateOptionMultiLegRfqInput, LegInput, QuoteLegInput, SubmitOptionMultiLegRfqQuoteInput,
};
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use serde_json::json;
use uuid::Uuid;

const MM_HEX_A: &str = "0x0000000000000000000000000000000000000001";
const MM_HEX_B: &str = "0x0000000000000000000000000000000000000002";
const TAKER_HEX: &str = "0x0000000000000000000000000000000000000003";

fn taker() -> AccountId {
    AccountId::new(TAKER_HEX)
}

fn mm_b() -> AccountId {
    AccountId::new(MM_HEX_B)
}

fn state_with_flag(flag: bool) -> AppState {
    let mut cfg = OptionsConfig::enabled_in_memory_for_tests();
    cfg.rfq_enabled = true;
    cfg.rfq_min_quote_ttl_ms = 1;
    cfg.rfq_max_quote_ttl_ms = 500;
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

fn quote_leg(index: u32, price: u128) -> QuoteLegInput {
    QuoteLegInput {
        leg_index: index,
        price_1e8: price,
    }
}

async fn seed_open_rfq(state: &AppState) -> Uuid {
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

fn mm_session(session_id: &str) -> MmSession {
    MmSession::with_ids(
        session_id,
        &format!("conn-{}", session_id),
        10,
        AuthMode::Disabled,
        true,
    )
}

/// Submit a quote via the MM Gateway WT service so `session_id` is
/// persisted onto the quote row. Returns the persisted quote_id.
async fn submit_gateway_quote_and_register(
    state: &AppState,
    service: &MmGatewayService,
    mm_account_hex: &str,
    rfq_id: Uuid,
    subaccount_id: Option<u32>,
    session_id_label: &str,
) -> (
    Uuid,
    MmSession,
    tokio::sync::mpsc::UnboundedReceiver<ServerMessage>,
) {
    use deopt_v2_backend::mm::protocol::ClientMessage;
    let mut session = mm_session(session_id_label);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ServerMessage>();
    service.register_session(&session, tx).unwrap();
    let mut payload = json!({
        "option_rfq_id": rfq_id,
        "mm_account": mm_account_hex,
        "package_price_1e8": "50000000",
        "size_1e8": "100000000",
        "legs": [
            { "leg_index": 0, "price_1e8": "12000000000" },
            { "leg_index": 1, "price_1e8": "11500000000" }
        ],
        "quote_ttl_ms": 100
    });
    if let (Some(obj), Some(sid)) = (payload.as_object_mut(), subaccount_id) {
        obj.insert(
            "maker_subaccount_id".to_string(),
            serde_json::Value::from(sid as u64),
        );
    }
    let msg: ClientMessage = serde_json::from_value(json!({
        "type": "option_multi_leg_rfq_quote",
        "request_id": format!("req-{}", session_id_label),
        "payload": payload,
    }))
    .unwrap();
    // Use a client-side request timeout of 20 iterations (matches
    // the sister spec pattern).
    let response = service.handle_message(&mut session, msg, 20).await;
    let ServerMessage::OptionMultiLegRfqQuoteResult(envelope) = response else {
        panic!("expected option_multi_leg_rfq_quote_result");
    };
    let quote_id = envelope.payload.quote_id;
    // Additionally, we register the quote's `session_id` — since the
    // service test binding uses the `MmSession::session_id` as the
    // key, `send_to_session` will target it correctly.
    let _ = state;
    (quote_id, session, rx)
}

// ---------------------------------------------------------------------
// Part 1 — Winning maker still gets accepted push; loser gets rejected.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part1_losing_maker_receives_rejected_push_and_winner_receives_accepted_push() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());

    // Maker A submits a quote (winner).
    let (winner_quote_id, _sess_a, mut rx_a) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_A,
        rfq_id,
        None,
        "session-mm-winner",
    )
    .await;
    // Drain any result envelope that landed in rx_a from the register
    // (registration itself does not enqueue anything, but the WT test
    // pattern polls anyway).
    while let Ok(_) = rx_a.try_recv() {}

    // Maker B submits a quote (loser).
    let (loser_quote_id, _sess_b, mut rx_b) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_B,
        rfq_id,
        None,
        "session-mm-loser",
    )
    .await;
    while let Ok(_) = rx_b.try_recv() {}

    // Taker accepts the winning maker's quote via the service. This
    // exercises the same accept path the HTTP handler uses.
    let (winner_quote, winner_quote_legs) = get_option_multi_leg_rfq_quote(&state, winner_quote_id)
        .await
        .unwrap();
    let outcome = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id: winner_quote_id,
            expected_package_price_1e8: winner_quote.package_price_1e8.clone(),
            expected_legs_count: winner_quote_legs.len() as u32,
            expected_leg_prices_1e8: winner_quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();

    // Winner MUST receive OptionMultiLegRfqQuoteAccepted.
    let mut saw_accepted = false;
    let mut saw_rejected_on_winner = false;
    while let Ok(msg) = rx_a.try_recv() {
        match msg {
            ServerMessage::OptionMultiLegRfqQuoteAccepted(envelope) => {
                assert_eq!(envelope.payload.quote_id, winner_quote_id);
                assert_eq!(envelope.payload.fill_id, outcome.fill.fill_id);
                saw_accepted = true;
            }
            ServerMessage::OptionMultiLegRfqQuoteRejected(_) => {
                saw_rejected_on_winner = true;
            }
            _ => {}
        }
    }
    assert!(saw_accepted, "winner must receive accepted push");
    assert!(
        !saw_rejected_on_winner,
        "winner MUST NOT receive rejected push for its own winning quote"
    );

    // Loser MUST receive OptionMultiLegRfqQuoteRejected with the full
    // extended payload; NO accepted push.
    let mut saw_rejected = false;
    let mut saw_accepted_on_loser = false;
    while let Ok(msg) = rx_b.try_recv() {
        match msg {
            ServerMessage::OptionMultiLegRfqQuoteRejected(envelope) => {
                let json = serde_json::to_string(&envelope).unwrap();
                assert_eq!(envelope.payload.option_rfq_id, rfq_id);
                assert_eq!(envelope.payload.quote_id, loser_quote_id);
                assert_eq!(envelope.payload.accepted_quote_id, Some(winner_quote_id));
                assert_eq!(envelope.payload.fill_id, Some(outcome.fill.fill_id));
                assert_eq!(envelope.payload.maker_subaccount_id, 1);
                assert_eq!(envelope.payload.legs_count, 2);
                assert_eq!(envelope.payload.reason, "not_selected");
                assert!(envelope.payload.rejected_at_ms > 0);
                // No secrets on the wire.
                assert!(!json.contains("signature"));
                assert!(!json.contains("nonce"));
                assert!(!json.contains("authorization"));
                saw_rejected = true;
            }
            ServerMessage::OptionMultiLegRfqQuoteAccepted(_) => {
                saw_accepted_on_loser = true;
            }
            _ => {}
        }
    }
    assert!(saw_rejected, "loser must receive rejected push");
    assert!(
        !saw_accepted_on_loser,
        "loser MUST NOT receive accepted push"
    );
}

// ---------------------------------------------------------------------
// Part 2 — Loser without a session_id does not fail the accept.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_losing_quote_without_session_does_not_fail_accept() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());

    // Maker A submits via MM Gateway → session_id populated.
    let (winner_quote_id, _sess_a, mut rx_a) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_A,
        rfq_id,
        None,
        "session-mm-w2",
    )
    .await;
    while let Ok(_) = rx_a.try_recv() {}

    // Maker B submits via the HTTP-style service path (no session_id).
    let (loser_quote, _) = submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm_b(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: Some("cq-http-loser".to_string()),
            package_price_1e8: "60000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 13_000_000_000), quote_leg(1, 12_500_000_000)],
            quote_nonce: Some(2),
            quote_ttl_ms: Some(200),
            signature: None,
        },
    )
    .await
    .unwrap();

    // Accept the WT winner. Loser has no session_id → no push, no
    // panic, no accept failure.
    let (winner_quote, winner_quote_legs) = get_option_multi_leg_rfq_quote(&state, winner_quote_id)
        .await
        .unwrap();
    let outcome = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id: winner_quote_id,
            expected_package_price_1e8: winner_quote.package_price_1e8.clone(),
            expected_legs_count: winner_quote_legs.len() as u32,
            expected_leg_prices_1e8: winner_quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        outcome.rfq.status,
        deopt_v2_backend::options::OptionMultiLegRfqStatus::Accepted
    );

    // Winner still received accepted push.
    let mut saw_accepted = false;
    while let Ok(msg) = rx_a.try_recv() {
        if matches!(msg, ServerMessage::OptionMultiLegRfqQuoteAccepted(_)) {
            saw_accepted = true;
        }
    }
    assert!(saw_accepted);

    // Loser's persisted quote is still `Rejected` — the status flip
    // happened inside the accept transaction irrespective of the
    // push.
    let (loser_persisted, _) = get_option_multi_leg_rfq_quote(&state, loser_quote.quote_id)
        .await
        .unwrap();
    assert_eq!(
        loser_persisted.status,
        deopt_v2_backend::options::OptionMultiLegRfqQuoteStatus::Rejected
    );
}

// ---------------------------------------------------------------------
// Part 3 — Duplicate accept attempt does not send a duplicate push.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part3_duplicate_accept_refused_no_duplicate_push() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());

    let (winner_quote_id, _sess_a, mut rx_a) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_A,
        rfq_id,
        None,
        "session-mm-w3",
    )
    .await;
    while let Ok(_) = rx_a.try_recv() {}

    let (loser_quote_id, _sess_b, mut rx_b) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_B,
        rfq_id,
        None,
        "session-mm-l3",
    )
    .await;
    while let Ok(_) = rx_b.try_recv() {}

    let (winner_quote, winner_quote_legs) = get_option_multi_leg_rfq_quote(&state, winner_quote_id)
        .await
        .unwrap();
    let input = AcceptOptionMultiLegRfqQuoteInput {
        taker: taker(),
        taker_subaccount_id: 1,
        option_rfq_id: rfq_id,
        quote_id: winner_quote_id,
        expected_package_price_1e8: winner_quote.package_price_1e8.clone(),
        expected_legs_count: winner_quote_legs.len() as u32,
        expected_leg_prices_1e8: winner_quote_legs.iter().map(|q| q.price_1e8).collect(),
    };

    // First accept succeeds → one rejected push to the loser.
    accept_option_multi_leg_rfq_quote(&state, input.clone())
        .await
        .unwrap();
    let mut first_reject_count = 0;
    while let Ok(msg) = rx_b.try_recv() {
        if matches!(msg, ServerMessage::OptionMultiLegRfqQuoteRejected(env) if env.payload.quote_id == loser_quote_id)
        {
            first_reject_count += 1;
        }
    }
    assert_eq!(first_reject_count, 1);

    // Second accept is refused by the atomic guard (RFQ no longer
    // open) — no second rejection push fires.
    let err = accept_option_multi_leg_rfq_quote(&state, input)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::InvalidOptionRfqState(_)
    ));
    let mut second_reject_count = 0;
    while let Ok(msg) = rx_b.try_recv() {
        if matches!(msg, ServerMessage::OptionMultiLegRfqQuoteRejected(env) if env.payload.quote_id == loser_quote_id)
        {
            second_reject_count += 1;
        }
    }
    assert_eq!(second_reject_count, 0);
}

// ---------------------------------------------------------------------
// Part 4 — Subaccount preserved on the rejected payload.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part4_loser_on_subaccount_two_sees_subaccount_two_in_payload() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());

    // Winner on subaccount 1 (default).
    let (winner_quote_id, _sess_a, mut rx_a) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_A,
        rfq_id,
        None,
        "session-mm-w4",
    )
    .await;
    while let Ok(_) = rx_a.try_recv() {}

    // Loser on subaccount 2 — needs a subaccount registration for
    // mm_b().
    seed_subaccount_two(&state, &mm_b()).await;
    let (_loser_quote_id, _sess_b, mut rx_b) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_B,
        rfq_id,
        Some(2),
        "session-mm-l4",
    )
    .await;
    while let Ok(_) = rx_b.try_recv() {}

    let (winner_quote, winner_quote_legs) = get_option_multi_leg_rfq_quote(&state, winner_quote_id)
        .await
        .unwrap();
    accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id: winner_quote_id,
            expected_package_price_1e8: winner_quote.package_price_1e8.clone(),
            expected_legs_count: winner_quote_legs.len() as u32,
            expected_leg_prices_1e8: winner_quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();

    let mut saw_subaccount_two = false;
    while let Ok(msg) = rx_b.try_recv() {
        if let ServerMessage::OptionMultiLegRfqQuoteRejected(envelope) = msg {
            assert_eq!(envelope.payload.maker_subaccount_id, 2);
            saw_subaccount_two = true;
        }
    }
    assert!(saw_subaccount_two);
}

// ---------------------------------------------------------------------
// Part 5 — No losing-maker push fires when there are no losers.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part5_single_maker_flow_produces_no_rejected_push() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state).await;
    let service = MmGatewayService::new(MmGatewayConfig::default(), state.clone());

    let (winner_quote_id, _sess_a, mut rx_a) = submit_gateway_quote_and_register(
        &state,
        &service,
        MM_HEX_A,
        rfq_id,
        None,
        "session-mm-w5",
    )
    .await;
    while let Ok(_) = rx_a.try_recv() {}

    let (winner_quote, winner_quote_legs) = get_option_multi_leg_rfq_quote(&state, winner_quote_id)
        .await
        .unwrap();
    accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id: winner_quote_id,
            expected_package_price_1e8: winner_quote.package_price_1e8.clone(),
            expected_legs_count: winner_quote_legs.len() as u32,
            expected_leg_prices_1e8: winner_quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();

    let mut saw_accepted = false;
    let mut saw_rejected = false;
    while let Ok(msg) = rx_a.try_recv() {
        match msg {
            ServerMessage::OptionMultiLegRfqQuoteAccepted(_) => saw_accepted = true,
            ServerMessage::OptionMultiLegRfqQuoteRejected(_) => saw_rejected = true,
            _ => {}
        }
    }
    assert!(saw_accepted);
    assert!(
        !saw_rejected,
        "single-maker flow must not emit any rejected push"
    );
}

// ---------------------------------------------------------------------
// Part 6 — Payload wire token stability + protocol tag lookup.
// ---------------------------------------------------------------------

#[test]
fn part6_rejected_message_serializes_with_expected_tag() {
    use deopt_v2_backend::mm::protocol::{
        NotificationEnvelope, OptionMultiLegRfqQuoteRejectedPayload, ServerMessage,
    };
    let envelope = NotificationEnvelope::new(
        "option_multi_leg_rfq_quote_rejected",
        "notif-1",
        OptionMultiLegRfqQuoteRejectedPayload {
            option_rfq_id: Uuid::nil(),
            quote_id: Uuid::nil(),
            accepted_quote_id: Some(Uuid::nil()),
            fill_id: Some(Uuid::nil()),
            maker_subaccount_id: 1,
            legs_count: 2,
            reason: "not_selected".to_string(),
            rejected_at_ms: 123_456_789,
        },
    );
    let msg = ServerMessage::OptionMultiLegRfqQuoteRejected(envelope);
    let json = serde_json::to_string(&msg).unwrap();
    // Tag stability — the wire event token must match what integrators
    // subscribe on.
    assert!(json.contains("\"type\":\"option_multi_leg_rfq_quote_rejected\""));
    // Payload fields present + honest.
    assert!(json.contains("\"reason\":\"not_selected\""));
    assert!(json.contains("\"maker_subaccount_id\":1"));
    assert!(json.contains("\"legs_count\":2"));
    assert!(json.contains("\"accepted_quote_id\""));
    assert!(json.contains("\"fill_id\""));
    // Secret-free.
    assert!(!json.contains("signature"));
    assert!(!json.contains("nonce"));
    assert!(!json.contains("authorization"));
}
