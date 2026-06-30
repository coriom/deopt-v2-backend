//! ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1 — worker-path emission tests.
//!
//! Exercises the conditional-orders worker through its public entry
//! points and asserts the lifecycle events it publishes onto
//! `state.lifecycle_events`. Tests are deterministic (no oracle RPC,
//! no chain) by driving `evaluate_conditional_orders_tick_with_prices`.
//!
//! Each scenario:
//!   1. Subscribes to the broadcast channel BEFORE the mutation.
//!   2. Performs the worker action.
//!   3. Drains the receiver and asserts the expected event sequence.
//!
//! Privacy/account filtering is verified by ensuring the events the
//! worker emits carry the expected `account` field — the WS handler's
//! per-session filter is unit-tested separately in
//! `src/api/public_ws/handler.rs::forward_lifecycle_event`.

use deopt_v2_backend::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::conditional_orders::{
    create_conditional_orders, evaluate_conditional_orders_tick_with_prices,
    list_conditional_orders, recover_stranded_triggering, ConditionalLegInput,
    ConditionalOrderFilter, ConditionalOrderStatus, ConditionalType, CreateConditionalOrderInput,
};
use deopt_v2_backend::options::service::{
    create_option_series, submit_option_order, CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use std::collections::HashMap;

const ONE_1E8: u128 = 100_000_000;
const PREMIUM_1E8: u128 = 1_000_000_000;

fn state() -> AppState {
    AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
    )
}

fn account(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

const HOLDER: &str = "0x0000000000000000000000000000000000000aaa";
const MAKER: &str = "0x0000000000000000000000000000000000000bbb";

async fn make_active_series(state: &AppState, tag: &str) -> String {
    let now_sec = (now_ms() / 1000) as u64;
    // Per-test unique strike avoids series-id collisions across tests.
    let strike = 70_000_000_000u128 + (tag.bytes().map(u128::from).sum::<u128>() * 1_000);
    let series = create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "BTC".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry: now_sec + 7 * 24 * 3600,
            strike_1e8: strike,
            is_call: true,
            contract_size_1e8: Some(ONE_1E8),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .unwrap();
    series.option_series_id
}

async fn seed_long_position(state: &AppState, series: &str, tag: &str) {
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(MAKER),
            side: Side::Sell,
            price_1e8: PREMIUM_1E8,
            size_1e8: ONE_1E8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("worker-life-maker-{tag}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
            attached_tp_sl: None,
        },
    )
    .await
    .unwrap();
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(HOLDER),
            side: Side::Buy,
            price_1e8: PREMIUM_1E8,
            size_1e8: ONE_1E8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("worker-life-holder-{tag}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
            attached_tp_sl: None,
        },
    )
    .await
    .unwrap();
}

/// Seed a closing-side liquidity offer so the child IOC can match.
/// `taker` is the side that the child IOC will hit, so this seeds the
/// opposite side (a passive sell when the holder is long calls).
async fn seed_closing_liquidity(state: &AppState, series: &str, tag: &str, price_1e8: u128) {
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(MAKER),
            side: Side::Buy,
            price_1e8,
            size_1e8: ONE_1E8,
            time_in_force: TimeInForce::Gtc,
            post_only: true,
            client_order_id: Some(format!("worker-life-bid-{tag}-{price_1e8}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
            attached_tp_sl: None,
        },
    )
    .await
    .unwrap();
}

async fn arm_tp(state: &AppState, series: &str, tag: &str) -> uuid::Uuid {
    let rows = create_conditional_orders(
        state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series.to_string(),
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: 900_000_000,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "tag={tag} expected single TP row");
    rows[0].id
}

async fn arm_oco_tp_sl(state: &AppState, series: &str, _tag: &str) -> (uuid::Uuid, uuid::Uuid) {
    let rows = create_conditional_orders(
        state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series.to_string(),
            quantity_1e8: ONE_1E8,
            legs: vec![
                ConditionalLegInput {
                    conditional_type: ConditionalType::TakeProfit,
                    trigger_price_1e8: 80_000_000_000,
                    limit_price_1e8: 900_000_000,
                    explicit_trigger_condition: None,
                },
                ConditionalLegInput {
                    conditional_type: ConditionalType::StopLoss,
                    trigger_price_1e8: 60_000_000_000,
                    limit_price_1e8: 100_000_000,
                    explicit_trigger_condition: None,
                },
            ],
            link_as_oco: true,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].oco_group_id.is_some());
    assert_eq!(rows[0].oco_group_id, rows[1].oco_group_id);
    let tp = rows
        .iter()
        .find(|r| r.conditional_type == ConditionalType::TakeProfit)
        .unwrap()
        .id;
    let sl = rows
        .iter()
        .find(|r| r.conditional_type == ConditionalType::StopLoss)
        .unwrap()
        .id;
    (tp, sl)
}

/// Drain all currently-pending events from a receiver without blocking.
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

fn prices_for(series_id: &str, price: u128) -> HashMap<String, u128> {
    let mut m = HashMap::new();
    m.insert(series_id.to_string(), price);
    m
}

// =====================================================================
// 1) Worker trigger emits ConditionalOrderUpdated(triggering)
// =====================================================================

#[tokio::test]
async fn worker_trigger_emits_triggering_then_terminal() {
    let state = state();
    let series = make_active_series(&state, "trig-terminal").await;
    seed_long_position(&state, &series, "tt").await;
    seed_closing_liquidity(&state, &series, "tt", 900_000_000).await;
    let _tp_id = arm_tp(&state, &series, "tt").await;

    let mut rx = state.lifecycle_events.subscribe();
    let result =
        evaluate_conditional_orders_tick_with_prices(&state, &prices_for(&series, 81_000_000_000))
            .await
            .unwrap();
    assert!(result.triggered >= 1);

    let events = drain(&mut rx);
    let conditional_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.channel, LifecycleChannel::AccountConditionalOrders))
        .collect();
    // Expect at least: triggering + terminal (completed).
    let statuses: Vec<&str> = conditional_events
        .iter()
        .filter_map(|e| match &e.payload {
            LifecyclePayload::ConditionalOrderUpdated { status, .. } => Some(status.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        statuses.contains(&"triggering"),
        "expected `triggering` in {statuses:?}"
    );
    let terminal_seen = statuses.iter().any(|s| {
        matches!(
            *s,
            "completed" | "partially_filled" | "failed" | "cancelled"
        )
    });
    assert!(terminal_seen, "expected a terminal status in {statuses:?}");
    // All conditional events MUST carry HOLDER's account — no leak.
    for e in &conditional_events {
        assert!(
            e.account.0.eq_ignore_ascii_case(HOLDER),
            "account leak: {e:?}"
        );
    }
}

// =====================================================================
// 2) Worker-triggered child order emits OrderUpdated + per-fill FillCreated
// =====================================================================

#[tokio::test]
async fn worker_child_order_emits_order_updated_and_fill_created() {
    let state = state();
    let series = make_active_series(&state, "child-fill").await;
    seed_long_position(&state, &series, "cf").await;
    // Closing-side liquidity at limit (the child IOC sells; maker bid is at limit).
    seed_closing_liquidity(&state, &series, "cf", 900_000_000).await;
    let _ = arm_tp(&state, &series, "cf").await;

    let mut rx = state.lifecycle_events.subscribe();
    evaluate_conditional_orders_tick_with_prices(&state, &prices_for(&series, 81_000_000_000))
        .await
        .unwrap();
    let events = drain(&mut rx);

    let order_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.channel, LifecycleChannel::AccountOrders))
        .collect();
    let fill_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.channel, LifecycleChannel::AccountFills))
        .collect();
    assert!(
        !order_events.is_empty(),
        "expected at least one account.orders event from the worker child submission"
    );
    assert!(
        !fill_events.is_empty(),
        "expected at least one account.fills event from the worker-triggered match"
    );
    // The HOLDER must see THEIR fill (as the child-order owner / seller of the closing trade).
    assert!(
        fill_events
            .iter()
            .any(|e| e.account.0.eq_ignore_ascii_case(HOLDER)),
        "HOLDER must receive the fill event"
    );
    // The MAKER (counterparty) must ALSO see the fill on their side.
    assert!(
        fill_events
            .iter()
            .any(|e| e.account.0.eq_ignore_ascii_case(MAKER)),
        "MAKER counterparty must also receive the fill event"
    );
}

// =====================================================================
// 3) OCO sibling cancellation emits per-sibling ConditionalOrderUpdated
//    with failure_code="oco_sibling_triggered"
// =====================================================================

#[tokio::test]
async fn oco_sibling_cancellation_emits_per_sibling_update() {
    let state = state();
    let series = make_active_series(&state, "oco-sibling").await;
    seed_long_position(&state, &series, "oco").await;
    seed_closing_liquidity(&state, &series, "oco", 900_000_000).await;
    let (tp_id, sl_id) = arm_oco_tp_sl(&state, &series, "oco").await;

    let mut rx = state.lifecycle_events.subscribe();
    evaluate_conditional_orders_tick_with_prices(&state, &prices_for(&series, 81_000_000_000))
        .await
        .unwrap();
    let events = drain(&mut rx);

    let sibling_cancel = events.iter().find(|e| match &e.payload {
        LifecyclePayload::ConditionalOrderUpdated {
            conditional_order_id,
            status,
            failure_code,
            ..
        } => {
            conditional_order_id == &sl_id.to_string()
                && status == "cancelled"
                && failure_code.as_deref() == Some("oco_sibling_triggered")
        }
        _ => false,
    });
    assert!(
        sibling_cancel.is_some(),
        "expected oco_sibling_triggered cancel for SL leg; events={events:?}"
    );
    // Winner (TP) MUST also emit at least one event (triggering + terminal).
    let winner_events = events
        .iter()
        .filter(|e| match &e.payload {
            LifecyclePayload::ConditionalOrderUpdated {
                conditional_order_id,
                ..
            } => conditional_order_id == &tp_id.to_string(),
            _ => false,
        })
        .count();
    assert!(winner_events >= 1, "TP winner must emit lifecycle updates");
}

// =====================================================================
// 4) Stale oracle / no price emits NO lifecycle events
// =====================================================================

#[tokio::test]
async fn stale_oracle_emits_no_lifecycle_events() {
    let state = state();
    let series = make_active_series(&state, "stale").await;
    seed_long_position(&state, &series, "stale").await;
    let _ = arm_tp(&state, &series, "stale").await;

    let mut rx = state.lifecycle_events.subscribe();
    // No price for the series → the worker selects 0 orders to trigger.
    let empty = HashMap::new();
    let result = evaluate_conditional_orders_tick_with_prices(&state, &empty)
        .await
        .unwrap();
    assert_eq!(result.triggered, 0);
    let events = drain(&mut rx);
    let lifecycle_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.channel, LifecycleChannel::AccountConditionalOrders))
        .collect();
    assert!(
        lifecycle_events.is_empty(),
        "stale oracle must not emit any conditional lifecycle event; got {lifecycle_events:?}"
    );
}

// =====================================================================
// 5) Repeated no-op ticks do NOT re-emit terminal events
// =====================================================================

#[tokio::test]
async fn repeated_ticks_do_not_re_emit_terminal_event() {
    let state = state();
    let series = make_active_series(&state, "no-dupe").await;
    seed_long_position(&state, &series, "no-dupe").await;
    seed_closing_liquidity(&state, &series, "no-dupe", 900_000_000).await;
    let _ = arm_tp(&state, &series, "no-dupe").await;

    let mut rx = state.lifecycle_events.subscribe();
    // First tick: triggers + terminalizes.
    evaluate_conditional_orders_tick_with_prices(&state, &prices_for(&series, 81_000_000_000))
        .await
        .unwrap();
    let first = drain(&mut rx).len();
    assert!(first > 0, "first tick must produce events");

    // Subsequent ticks: no armed rows → nothing to do.
    for _ in 0..3 {
        evaluate_conditional_orders_tick_with_prices(&state, &prices_for(&series, 81_000_000_000))
            .await
            .unwrap();
    }
    let extra = drain(&mut rx);
    assert!(
        extra.is_empty(),
        "no-op ticks must not re-emit terminal events; got {extra:?}"
    );
}

// =====================================================================
// 6) `position_closed` cancellation emits the right failure_code
// =====================================================================

#[tokio::test]
async fn position_closed_emits_cancelled_with_failure_code() {
    let state = state();
    let series = make_active_series(&state, "pos-closed").await;
    seed_long_position(&state, &series, "pc").await;
    let _ = arm_tp(&state, &series, "pc").await;
    // Close the holder's position before the trigger fires: seed a
    // closing offer and have the HOLDER hit it (sells back the 1 contract).
    seed_closing_liquidity(&state, &series, "pc", 950_000_000).await;
    submit_option_order(
        &state,
        SubmitOptionOrderInput {
            option_series_id: series.clone(),
            account: account(HOLDER),
            side: Side::Sell,
            price_1e8: 900_000_000,
            size_1e8: ONE_1E8,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            client_order_id: Some("pc-close".to_string()),
            nonce: None,
            deadline_ms: None,
            signature: None,
            attached_tp_sl: None,
        },
    )
    .await
    .unwrap();

    let mut rx = state.lifecycle_events.subscribe();
    evaluate_conditional_orders_tick_with_prices(&state, &prices_for(&series, 81_000_000_000))
        .await
        .unwrap();
    let events = drain(&mut rx);

    let cancelled = events.iter().find(|e| match &e.payload {
        LifecyclePayload::ConditionalOrderUpdated {
            status,
            failure_code,
            ..
        } => status == "cancelled" && failure_code.as_deref() == Some("position_closed"),
        _ => false,
    });
    assert!(
        cancelled.is_some(),
        "expected position_closed cancellation; got {events:?}"
    );
}

// =====================================================================
// 7) Stranded-triggering recovery emits a lifecycle update
// =====================================================================

#[tokio::test]
async fn stranded_recovery_emits_lifecycle_update() {
    let state = state();
    let series = make_active_series(&state, "stranded").await;
    seed_long_position(&state, &series, "stranded").await;
    let cond_id = arm_tp(&state, &series, "stranded").await;

    // Manually flip the conditional row into the stranded `Triggering`
    // status (no child) — modelling a crash between claim and child
    // submission. We do this through the in-memory store directly so
    // we don't depend on the worker actually crashing.
    {
        let mut store = state.options_store.lock().unwrap();
        let mut row = store.get_conditional_order(cond_id).unwrap();
        row.status = ConditionalOrderStatus::Triggering;
        row.child_order_id = None;
        row.updated_at_ms = now_ms();
        row.version = row.version.saturating_add(1);
        store.update_conditional_order(row).unwrap();
    }

    let mut rx = state.lifecycle_events.subscribe();
    let recovered = recover_stranded_triggering(&state, now_ms()).await.unwrap();
    assert!(recovered >= 1);
    let events = drain(&mut rx);
    let recovered_event = events.iter().find(|e| match &e.payload {
        LifecyclePayload::ConditionalOrderUpdated {
            conditional_order_id,
            ..
        } => conditional_order_id == &cond_id.to_string(),
        _ => false,
    });
    assert!(
        recovered_event.is_some(),
        "stranded recovery must emit a lifecycle update; got {events:?}"
    );

    // Sanity: the recovered row is back in `armed` (no child) per the
    // recovery policy.
    let after = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(after.iter().any(|r| r.id == cond_id
        && matches!(
            r.status,
            ConditionalOrderStatus::Armed | ConditionalOrderStatus::Completed
        )));
}
