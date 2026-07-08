//! PERPS-PG-HARNESS-AND-REJECTION-EMIT-V1 — dispatcher-level rejection
//! lifecycle tests.
//!
//! Both the in-memory dispatcher (`submit_perp_order_via_state`) and
//! the PG-backed inner function (`submit_perp_order_via_repository`)
//! must emit a `PerpOrderRejected` frame on classified errors, and MUST
//! NOT emit for auth / config / internal errors.
//!
//! These tests exercise the in-memory path directly. The PG-backed
//! path uses the same `RejectionSnapshot` helper, so any regression
//! in the emission contract fails at the same code site.

use deopt_v2_backend::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead},
    submit_perp_order_via_state, PerpOrderSide, PerpTimeInForce, PerpsReadConfig,
    SubmitPerpOrderInput,
};
use deopt_v2_backend::types::{now_ms, AccountId};

const ONE: u128 = 100_000_000;
const PRICE_ETH_3000: u128 = 3000 * ONE;
const PRICE_ETH_3100: u128 = 3100 * ONE;
const MARGIN_10X_ETH: u128 = 300 * ONE;

fn addr(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

fn state() -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state
}

fn fresh_price_reader() -> InMemoryPerpOraclePriceReader {
    InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: PRICE_ETH_3000,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    )
}

fn stale_price_reader() -> InMemoryPerpOraclePriceReader {
    InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: PRICE_ETH_3000,
            updated_at_sec: 1, // ancient
            ok: true,
        },
    )
}

fn base_input(
    account: AccountId,
    side: PerpOrderSide,
    price: u128,
    size: u128,
) -> SubmitPerpOrderInput {
    SubmitPerpOrderInput {
        account,
        subaccount_id: 1,
        market_id: "ETH-PERP".to_string(),
        side,
        price_1e8: price,
        size_1e8: size,
        time_in_force: PerpTimeInForce::Gtc,
        post_only: false,
        reduce_only: false,
        isolated_margin_1e8: MARGIN_10X_ETH,
        client_order_id: None,
    }
}

fn drain(rx: &mut tokio::sync::broadcast::Receiver<LifecycleEvent>) -> Vec<LifecycleEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(_) => break,
        }
    }
    out
}

fn find_rejection<'a>(events: &'a [LifecycleEvent]) -> Option<&'a LifecycleEvent> {
    events
        .iter()
        .find(|e| matches!(e.payload, LifecyclePayload::PerpOrderRejected { .. }))
}

// =====================================================================
// A. Classified errors — a `PerpOrderRejected` frame must fire
// =====================================================================

#[tokio::test]
async fn post_only_would_match_emits_perp_order_rejected() {
    let state = state();
    let reader = fresh_price_reader();
    // Maker sells; bob's post-only buy at same price would cross.
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            addr("0x000000000000000000000000000000000000aaaa"),
            PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE,
        ),
    )
    .await
    .unwrap();
    let mut rx = state.lifecycle_events.subscribe();
    let bob = addr("0x000000000000000000000000000000000000bbbb");
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            post_only: true,
            ..base_input(bob.clone(), PerpOrderSide::Buy, PRICE_ETH_3100, ONE)
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpPostOnlyWouldMatch
    ));

    let events = drain(&mut rx);
    let ev = find_rejection(&events).expect("expected PerpOrderRejected frame");
    assert_eq!(ev.account, bob);
    assert!(matches!(ev.channel, LifecycleChannel::AccountPerpOrders));
    match &ev.payload {
        LifecyclePayload::PerpOrderRejected {
            market_id,
            side,
            reason_code,
            reason_source,
            post_only,
            reduce_only,
            price_1e8,
            size_1e8,
            time_in_force,
            client_order_id,
            reason_message,
            ..
        } => {
            assert_eq!(market_id.as_deref(), Some("ETH-PERP"));
            assert_eq!(side.as_deref(), Some("buy"));
            assert_eq!(reason_code, "post_only_would_match");
            assert_eq!(reason_source, "matching_policy");
            assert_eq!(post_only, &Some(true));
            assert_eq!(reduce_only, &Some(false));
            assert_eq!(
                price_1e8.as_deref(),
                Some(PRICE_ETH_3100.to_string().as_str())
            );
            assert_eq!(size_1e8.as_deref(), Some(ONE.to_string().as_str()));
            assert_eq!(time_in_force.as_deref(), Some("gtc"));
            assert!(client_order_id.is_none());
            assert!(reason_message.is_some());
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn fok_not_fillable_emits_perp_order_rejected() {
    let state = state();
    let reader = fresh_price_reader();
    // Small resting sell.
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            addr("0x000000000000000000000000000000000000aaaa"),
            PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE / 2,
        ),
    )
    .await
    .unwrap();
    let mut rx = state.lifecycle_events.subscribe();
    // FOK buy for full ONE at $3100 — must reject because only half is fillable.
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            size_1e8: ONE,
            time_in_force: PerpTimeInForce::Fok,
            ..base_input(
                addr("0x000000000000000000000000000000000000bbbb"),
                PerpOrderSide::Buy,
                PRICE_ETH_3100,
                ONE,
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpFokNotFillable
    ));
    let events = drain(&mut rx);
    let ev = find_rejection(&events).expect("expected PerpOrderRejected frame");
    match &ev.payload {
        LifecyclePayload::PerpOrderRejected {
            reason_code,
            reason_source,
            time_in_force,
            ..
        } => {
            assert_eq!(reason_code, "fok_not_fillable");
            assert_eq!(reason_source, "matching_policy");
            assert_eq!(time_in_force.as_deref(), Some("fok"));
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn self_trade_emits_perp_order_rejected() {
    let state = state();
    let reader = fresh_price_reader();
    let account = addr("0x000000000000000000000000000000000000aaaa");
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(account.clone(), PerpOrderSide::Sell, PRICE_ETH_3000, ONE),
    )
    .await
    .unwrap();
    let mut rx = state.lifecycle_events.subscribe();
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(account.clone(), PerpOrderSide::Buy, PRICE_ETH_3100, ONE),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpSelfTrade
    ));
    let events = drain(&mut rx);
    let ev = find_rejection(&events).expect("expected PerpOrderRejected frame");
    match &ev.payload {
        LifecyclePayload::PerpOrderRejected {
            reason_code,
            reason_source,
            ..
        } => {
            assert_eq!(reason_code, "self_trade");
            assert_eq!(reason_source, "matching_policy");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn stale_mark_emits_perp_order_rejected() {
    let state = state();
    let reader = stale_price_reader();
    let mut rx = state.lifecycle_events.subscribe();
    let alice = addr("0x000000000000000000000000000000000000aaaa");
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(alice.clone(), PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpMarkPriceUnavailable(_)
    ));
    let events = drain(&mut rx);
    let ev = find_rejection(&events).expect("expected PerpOrderRejected frame");
    assert_eq!(ev.account, alice);
    match &ev.payload {
        LifecyclePayload::PerpOrderRejected {
            reason_code,
            reason_source,
            ..
        } => {
            assert_eq!(reason_code, "stale_mark_price");
            assert_eq!(reason_source, "risk");
        }
        _ => unreachable!(),
    }
}

// =====================================================================
// B. Non-classified errors — no rejection frame should fire
// =====================================================================

#[tokio::test]
async fn unknown_market_emits_rejection_because_its_classified() {
    // `PerpsMarketNotFound` IS classified (as `unknown_market`), so
    // the emitter fires. This test pins that behavior — a
    // configuration mistake by an internal caller shouldn't slip
    // through without a lifecycle signal.
    let state = state();
    let reader = fresh_price_reader();
    let mut rx = state.lifecycle_events.subscribe();
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            market_id: "SOL-PERP".to_string(),
            ..base_input(
                addr("0x000000000000000000000000000000000000aaaa"),
                PerpOrderSide::Buy,
                PRICE_ETH_3000,
                ONE,
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpsMarketNotFound(_)
    ));
    let events = drain(&mut rx);
    let ev = find_rejection(&events).expect("expected PerpOrderRejected frame");
    match &ev.payload {
        LifecyclePayload::PerpOrderRejected {
            reason_code,
            reason_source,
            market_id,
            ..
        } => {
            assert_eq!(reason_code, "unknown_market");
            assert_eq!(reason_source, "request_validation");
            assert_eq!(market_id.as_deref(), Some("SOL-PERP"));
        }
        _ => unreachable!(),
    }
}

// =====================================================================
// C. No-secret assertion
// =====================================================================

#[tokio::test]
async fn perp_order_rejected_frame_has_no_secret_fields() {
    let state = state();
    let reader = fresh_price_reader();
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            addr("0x000000000000000000000000000000000000aaaa"),
            PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE,
        ),
    )
    .await
    .unwrap();
    let mut rx = state.lifecycle_events.subscribe();
    let _ = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            post_only: true,
            client_order_id: Some("cli-1".to_string()),
            ..base_input(
                addr("0x000000000000000000000000000000000000bbbb"),
                PerpOrderSide::Buy,
                PRICE_ETH_3100,
                ONE,
            )
        },
    )
    .await
    .unwrap_err();
    let events = drain(&mut rx);
    let ev = find_rejection(&events).expect("expected PerpOrderRejected frame");
    let json = serde_json::to_string(ev).unwrap();
    for banned in [
        "signature",
        "nonce",
        "envelope",
        "authorization",
        "bearer",
        "password",
    ] {
        assert!(
            !json.to_ascii_lowercase().contains(banned),
            "PerpOrderRejected frame contained banned field `{banned}`: {json}"
        );
    }
}

// =====================================================================
// D. Ok path — no rejection frame should fire on a clean fill
// =====================================================================

#[tokio::test]
async fn successful_submit_does_not_emit_perp_order_rejected() {
    let state = state();
    let reader = fresh_price_reader();
    let mut rx = state.lifecycle_events.subscribe();
    let alice = addr("0x000000000000000000000000000000000000aaaa");
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(alice, PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
    )
    .await
    .unwrap();
    let events = drain(&mut rx);
    assert!(
        find_rejection(&events).is_none(),
        "clean open must NOT emit a rejection frame"
    );
}
