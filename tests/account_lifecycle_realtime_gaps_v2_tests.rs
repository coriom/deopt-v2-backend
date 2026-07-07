//! ACCOUNT-LIFECYCLE-REALTIME-GAPS-V2 — asserts the two new
//! account-scoped lifecycle payloads land on the broadcast channel:
//!
//!   * `OrderRejected` on `account.orders`
//!   * `AttachmentPlanUpdated` on `account.conditional_orders`
//!
//! Emission MUST happen AFTER durable persistence and MUST NOT
//! carry signatures, nonces, auth envelopes, headers, or bearer
//! tokens.

use deopt_v2_backend::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{
    cancel_option_order, create_option_series, submit_option_order, AttachedLegInput,
    AttachedTpSlInput, CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{OptionOrderStatus, OptionsConfig};
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};

const ONE_1E8: u128 = 100_000_000;
const PREMIUM_1E8: u128 = 1_000_000_000;

fn state() -> AppState {
    AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
    )
}

fn account_a() -> AccountId {
    AccountId::new("0x000000000000000000000000000000000000aaaa")
}

fn account_b() -> AccountId {
    AccountId::new("0x000000000000000000000000000000000000bbbb")
}

async fn active_series(state: &AppState, tag: &str) -> String {
    let now_sec = (now_ms() / 1000) as u64;
    let strike = 70_000_000_000u128 + (tag.bytes().map(u128::from).sum::<u128>() * 1_000);
    create_option_series(
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
    .unwrap()
    .option_series_id
}

fn base_input(
    series: &str,
    account: AccountId,
    side: Side,
    client_id: &str,
) -> SubmitOptionOrderInput {
    SubmitOptionOrderInput {
        option_series_id: series.to_string(),
        account,
        subaccount_id: 1,
        side,
        price_1e8: PREMIUM_1E8,
        size_1e8: ONE_1E8,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        client_order_id: Some(client_id.to_string()),
        nonce: None,
        deadline_ms: None,
        signature: None,
        attached_tp_sl: None,
    }
}

async fn seed_resting(
    state: &AppState,
    series: &str,
    side: Side,
    tag: &str,
    price: u128,
    size: u128,
) {
    let mut input = base_input(series, account_a(), side, tag);
    input.price_1e8 = price;
    input.size_1e8 = size;
    submit_option_order(state, input).await.unwrap();
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

fn tp_only_leg() -> AttachedTpSlInput {
    AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: None,
        link_as_oco: false,
        expires_at_ms: None,
    }
}

fn tp_and_sl_oco_legs() -> AttachedTpSlInput {
    AttachedTpSlInput {
        take_profit: Some(AttachedLegInput {
            trigger_price_1e8: 1_500_000_000,
            limit_price_1e8: 1_500_000_000,
        }),
        stop_loss: Some(AttachedLegInput {
            trigger_price_1e8: 500_000_000,
            limit_price_1e8: 500_000_000,
        }),
        link_as_oco: true,
        expires_at_ms: None,
    }
}

// =====================================================================
// OrderRejected — emitted on account.orders after durable persistence
// =====================================================================

#[tokio::test]
async fn post_only_rejection_emits_order_rejected_on_account_orders() {
    let state = state();
    let series = active_series(&state, "po-emit").await;
    // Resting ask; post-only buy at same price would cross → rejection.
    seed_resting(
        &state,
        &series,
        Side::Sell,
        "po-resting",
        PREMIUM_1E8,
        ONE_1E8,
    )
    .await;
    let mut rx = state.lifecycle_events.subscribe();

    let mut taker = base_input(&series, account_b(), Side::Buy, "po-cross");
    taker.post_only = true;
    let _ = submit_option_order(&state, taker).await.unwrap_err();

    let events = drain(&mut rx);
    let rejections: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.channel, LifecycleChannel::AccountOrders))
        .filter(|e| matches!(e.payload, LifecyclePayload::OrderRejected { .. }))
        .collect();
    assert_eq!(rejections.len(), 1, "expected exactly one OrderRejected");
    let event = rejections[0];
    // Account routing: emit MUST carry the taker's account.
    assert_eq!(event.account, account_b());
    match &event.payload {
        LifecyclePayload::OrderRejected {
            rejection_id,
            option_series_id,
            side,
            price_1e8,
            size_1e8,
            time_in_force,
            post_only,
            client_order_id,
            reason_code,
            reason_message,
            reason_source,
            created_at_ms,
        } => {
            assert!(!rejection_id.is_empty());
            assert_eq!(option_series_id.as_deref(), Some(series.as_str()));
            assert_eq!(side.as_deref(), Some("buy"));
            assert_eq!(price_1e8.as_deref(), Some(PREMIUM_1E8.to_string().as_str()));
            assert_eq!(size_1e8.as_deref(), Some(ONE_1E8.to_string().as_str()));
            assert_eq!(time_in_force.as_deref(), Some("gtc"));
            assert_eq!(post_only, &Some(true));
            assert_eq!(client_order_id.as_deref(), Some("po-cross"));
            assert_eq!(reason_code, "post_only_would_match");
            assert_eq!(reason_source, "matching_policy");
            assert!(reason_message
                .as_deref()
                .unwrap_or("")
                .contains("post-only"));
            assert!(*created_at_ms > 0);
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn fok_rejection_emits_order_rejected() {
    let state = state();
    let series = active_series(&state, "fok-emit").await;
    seed_resting(
        &state,
        &series,
        Side::Sell,
        "fok-shallow",
        950_000_000,
        30_000_000,
    )
    .await;
    seed_resting(
        &state,
        &series,
        Side::Sell,
        "fok-deep",
        1_100_000_000,
        70_000_000,
    )
    .await;
    let mut rx = state.lifecycle_events.subscribe();

    let mut taker = base_input(&series, account_b(), Side::Buy, "fok-taker");
    taker.time_in_force = TimeInForce::Fok;
    taker.price_1e8 = 1_000_000_000;
    taker.size_1e8 = ONE_1E8;
    let _ = submit_option_order(&state, taker).await.unwrap_err();

    let events = drain(&mut rx);
    let rejection = events
        .iter()
        .find(|e| matches!(e.payload, LifecyclePayload::OrderRejected { .. }))
        .expect("expected an OrderRejected event");
    assert_eq!(rejection.account, account_b());
    assert!(matches!(rejection.channel, LifecycleChannel::AccountOrders));
    match &rejection.payload {
        LifecyclePayload::OrderRejected {
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
async fn accepted_open_order_does_not_emit_order_rejected() {
    // A parent that rests cleanly must not produce an `OrderRejected`.
    let state = state();
    let series = active_series(&state, "clean-open").await;
    let mut rx = state.lifecycle_events.subscribe();
    let input = base_input(&series, account_a(), Side::Buy, "clean-buy");
    let outcome = submit_option_order(&state, input).await.unwrap();
    assert_eq!(outcome.order.status, OptionOrderStatus::Open);

    let events = drain(&mut rx);
    let rejections: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::OrderRejected { .. }))
        .collect();
    assert!(
        rejections.is_empty(),
        "unexpected OrderRejected on clean open: {rejections:?}"
    );
}

// Serialization sanity: an `OrderRejected` frame carries no fields
// that could leak a secret. The struct is enum-shaped so this test
// is a compile-time + serde spot-check.
#[tokio::test]
async fn order_rejected_frame_has_no_secret_fields() {
    let state = state();
    let series = active_series(&state, "secret-shape").await;
    seed_resting(
        &state,
        &series,
        Side::Sell,
        "sh-resting",
        PREMIUM_1E8,
        ONE_1E8,
    )
    .await;
    let mut rx = state.lifecycle_events.subscribe();
    let mut taker = base_input(&series, account_b(), Side::Buy, "sh-cross");
    taker.post_only = true;
    let _ = submit_option_order(&state, taker).await.unwrap_err();
    let ev = drain(&mut rx)
        .into_iter()
        .find(|e| matches!(e.payload, LifecyclePayload::OrderRejected { .. }))
        .expect("rejection event");
    let json = serde_json::to_string(&ev).unwrap();
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
            "OrderRejected frame contained banned field `{banned}`: {json}"
        );
    }
}

// =====================================================================
// AttachmentPlanUpdated — emitted on account.conditional_orders
// after each durable plan transition.
// =====================================================================

#[tokio::test]
async fn attachment_plan_pending_create_emits_updated() {
    let state = state();
    let series = active_series(&state, "plan-pending").await;
    let mut rx = state.lifecycle_events.subscribe();
    // No counterparty → parent rests. Plan is created in Pending state.
    let mut input = base_input(&series, account_a(), Side::Buy, "plan-pending-buy");
    input.attached_tp_sl = Some(tp_only_leg());
    submit_option_order(&state, input).await.unwrap();

    let events = drain(&mut rx);
    let plan_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::AttachmentPlanUpdated { .. }))
        .collect();
    assert_eq!(
        plan_events.len(),
        1,
        "expected exactly one AttachmentPlanUpdated on pending create"
    );
    let ev = plan_events[0];
    assert!(matches!(
        ev.channel,
        LifecycleChannel::AccountConditionalOrders
    ));
    assert_eq!(ev.account, account_a());
    match &ev.payload {
        LifecyclePayload::AttachmentPlanUpdated {
            status,
            materialized_size_1e8,
            tp_conditional_order_id,
            sl_conditional_order_id,
            ..
        } => {
            assert_eq!(status, "pending");
            assert!(materialized_size_1e8.is_none());
            assert!(tp_conditional_order_id.is_none());
            assert!(sl_conditional_order_id.is_none());
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn attachment_plan_active_materialization_emits_updated() {
    let state = state();
    let series = active_series(&state, "plan-active").await;
    // Resting ask so the taker fills immediately → plan materialises → Active.
    seed_resting(
        &state,
        &series,
        Side::Sell,
        "plan-active-resting",
        PREMIUM_1E8,
        ONE_1E8,
    )
    .await;
    let mut rx = state.lifecycle_events.subscribe();
    let mut taker = base_input(&series, account_b(), Side::Buy, "plan-active-taker");
    taker.attached_tp_sl = Some(tp_and_sl_oco_legs());
    submit_option_order(&state, taker).await.unwrap();

    let events = drain(&mut rx);
    let plan_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::AttachmentPlanUpdated { .. }))
        .collect();
    // Expect at least two frames: pending create, then active materialisation.
    assert!(
        plan_events.len() >= 2,
        "expected pending + active AttachmentPlanUpdated frames, got {}",
        plan_events.len()
    );
    // The last plan event for account_b MUST be active.
    let last_for_b = plan_events
        .iter()
        .filter(|e| e.account == account_b())
        .last()
        .unwrap();
    match &last_for_b.payload {
        LifecyclePayload::AttachmentPlanUpdated {
            status,
            materialized_size_1e8,
            tp_conditional_order_id,
            sl_conditional_order_id,
            oco_group_id,
            ..
        } => {
            assert_eq!(status, "active");
            assert_eq!(
                materialized_size_1e8.as_deref(),
                Some(ONE_1E8.to_string().as_str())
            );
            assert!(tp_conditional_order_id.is_some());
            assert!(sl_conditional_order_id.is_some());
            assert!(oco_group_id.is_some(), "oco plan must carry oco_group_id");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn attachment_plan_cancelled_before_fill_emits_updated() {
    let state = state();
    let series = active_series(&state, "plan-cancel").await;
    // No counterparty → plan rests in Pending.
    let mut input = base_input(&series, account_a(), Side::Buy, "plan-cancel-buy");
    input.attached_tp_sl = Some(tp_only_leg());
    let parent = submit_option_order(&state, input).await.unwrap().order;

    let mut rx = state.lifecycle_events.subscribe();
    cancel_option_order(&state, parent.order_id).await.unwrap();

    let events = drain(&mut rx);
    let plan_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::AttachmentPlanUpdated { .. }))
        .collect();
    let cancel_frame = plan_events
        .iter()
        .find(|e| {
            matches!(
                &e.payload,
                LifecyclePayload::AttachmentPlanUpdated { status, .. } if status == "cancelled"
            )
        })
        .expect("expected a cancelled AttachmentPlanUpdated frame");
    assert_eq!(cancel_frame.account, account_a());
    match &cancel_frame.payload {
        LifecyclePayload::AttachmentPlanUpdated {
            status,
            failure_code,
            ..
        } => {
            assert_eq!(status, "cancelled");
            assert_eq!(failure_code.as_deref(), Some("parent_terminal_before_fill"));
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn attachment_plan_maker_fill_materializes_and_emits_updated() {
    // V2 hook: a resting parent with attached TP/SL later gets a maker-side
    // fill from a taker; the plan MUST materialise and emit `AttachmentPlanUpdated`.
    let state = state();
    let series = active_series(&state, "plan-maker").await;

    // Maker (account_a) rests a SELL with attached TP.
    let mut maker_input = base_input(&series, account_a(), Side::Sell, "plan-maker-sell");
    maker_input.attached_tp_sl = Some(tp_only_leg());
    let maker = submit_option_order(&state, maker_input)
        .await
        .unwrap()
        .order;
    assert_eq!(maker.status, OptionOrderStatus::Open);

    // Subscribe AFTER pending-create so we only see the maker-fill transition.
    let mut rx = state.lifecycle_events.subscribe();

    // Taker (account_b) crosses.
    let mut taker = base_input(&series, account_b(), Side::Buy, "plan-maker-taker");
    taker.size_1e8 = ONE_1E8;
    submit_option_order(&state, taker).await.unwrap();

    let events = drain(&mut rx);
    // The maker's plan MUST transition to Active on their account.
    let maker_plan_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::AttachmentPlanUpdated { .. }))
        .filter(|e| e.account == account_a())
        .collect();
    assert!(
        !maker_plan_events.is_empty(),
        "expected at least one AttachmentPlanUpdated for the maker's account"
    );
    let last = maker_plan_events.last().unwrap();
    match &last.payload {
        LifecyclePayload::AttachmentPlanUpdated {
            status,
            materialized_size_1e8,
            tp_conditional_order_id,
            ..
        } => {
            assert_eq!(status, "active");
            assert_eq!(
                materialized_size_1e8.as_deref(),
                Some(ONE_1E8.to_string().as_str())
            );
            assert!(tp_conditional_order_id.is_some());
        }
        _ => unreachable!(),
    }
    // Account isolation: no plan events for the maker leaked to the taker's channel.
    let taker_plan_events: Vec<&LifecycleEvent> = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::AttachmentPlanUpdated { .. }))
        .filter(|e| e.account == account_b())
        .collect();
    assert!(
        taker_plan_events.is_empty(),
        "maker plan leaked onto taker's account channel: {taker_plan_events:?}"
    );
}

#[tokio::test]
async fn attachment_plan_event_has_no_secret_fields() {
    let state = state();
    let series = active_series(&state, "plan-noleak").await;
    let mut rx = state.lifecycle_events.subscribe();
    let mut input = base_input(&series, account_a(), Side::Buy, "plan-noleak-buy");
    input.attached_tp_sl = Some(tp_only_leg());
    submit_option_order(&state, input).await.unwrap();
    let ev = drain(&mut rx)
        .into_iter()
        .find(|e| matches!(e.payload, LifecyclePayload::AttachmentPlanUpdated { .. }))
        .expect("plan event");
    let json = serde_json::to_string(&ev).unwrap();
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
            "AttachmentPlanUpdated contained banned field `{banned}`: {json}"
        );
    }
}
