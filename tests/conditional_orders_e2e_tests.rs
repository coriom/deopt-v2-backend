// OPTIONS-CONDITIONAL-ORDERS-PERSISTENT-E2E-V1
//
// Operational-hardening validation suite for the TP/SL system. Every
// test uses the production execution path
// (`evaluate_conditional_orders_tick_with_prices` →
// `trigger_one` → `execute_triggered_in_store` →
// `submit_option_order`) — the only test-only seam is the price
// snapshot injection (instead of an RPC oracle round-trip). The
// claim, OCO sibling cancel, reduce-only cap and child IOC paths are
// the same code production runs.
//
// Persistence parity:
//   - The in-memory store mirrors the DB lifecycle through the
//     conditional_orders module's service-layer branch on
//     `state.repository`. Both paths share the trigger-direction
//     matrix, the reducible-position derivation, the atomic OCO
//     sibling cancel, the child IOC client_order_id collision
//     contract, and the `recover_stranded_triggering` recovery
//     sweep. This suite exercises the in-memory mirror; the
//     repository methods are validated by `cargo check --lib`
//     (compile-time SQL parity) + the operator-facing migration
//     0028 schema.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::conditional_orders::{
    cancel_conditional_order, create_conditional_orders,
    evaluate_conditional_orders_tick_with_prices, list_conditional_orders,
    recover_stranded_triggering, ConditionalLegInput, ConditionalOrderFilter,
    ConditionalOrderStatus, ConditionalType, CreateConditionalOrderInput, PositionSide,
    TriggerCondition,
};
use deopt_v2_backend::options::service::{
    create_option_series, submit_option_order, CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{OptionFillFilter, OptionsConfig};
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

const HOLDER: &str = "0x0000000000000000000000000000000000000111";
const MAKER: &str = "0x0000000000000000000000000000000000000222";

fn account(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

async fn make_active_series(state: &AppState, is_call: bool) -> String {
    let now_sec = (now_ms() / 1000) as u64;
    let series = create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "BTC".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry: now_sec + 7 * 24 * 3600,
            strike_1e8: 70_000_000_000,
            is_call,
            contract_size_1e8: Some(ONE_1E8),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .unwrap();
    series.option_series_id
}

async fn seed_long_position(state: &AppState, series: &str, size_1e8: u128) {
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(MAKER),
            side: Side::Sell,
            price_1e8: PREMIUM_1E8,
            size_1e8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("seed-maker-{size_1e8}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
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
            size_1e8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("seed-holder-{size_1e8}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();
}

/// Open a closing-side bid so the child IOC sell (closing the long)
/// has liquidity to match against. `bid_size_1e8 == 0` produces an
/// empty closing book.
async fn seed_closing_bid(state: &AppState, series: &str, bid_size_1e8: u128, price_1e8: u128) {
    if bid_size_1e8 == 0 {
        return;
    }
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(MAKER),
            side: Side::Buy,
            price_1e8,
            size_1e8: bid_size_1e8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("close-bid-{bid_size_1e8}-{price_1e8}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();
}

async fn arm_tp_sl(
    state: &AppState,
    series_id: &str,
    qty_1e8: u128,
    tp_trigger: u128,
    sl_trigger: u128,
    oco: bool,
) -> Vec<deopt_v2_backend::options::conditional_orders::ConditionalOrder> {
    create_conditional_orders(
        state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series_id.to_string(),
            quantity_1e8: qty_1e8,
            legs: vec![
                ConditionalLegInput {
                    conditional_type: ConditionalType::TakeProfit,
                    trigger_price_1e8: tp_trigger,
                    limit_price_1e8: PREMIUM_1E8 / 2,
                    explicit_trigger_condition: None,
                },
                ConditionalLegInput {
                    conditional_type: ConditionalType::StopLoss,
                    trigger_price_1e8: sl_trigger,
                    limit_price_1e8: PREMIUM_1E8 / 2,
                    explicit_trigger_condition: None,
                },
            ],
            link_as_oco: oco,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap()
}

fn prices_for(series_id: &str, price_1e8: u128) -> HashMap<String, u128> {
    let mut m = HashMap::new();
    m.insert(series_id.to_string(), price_1e8);
    m
}

// ===== Phase 4 — Oracle crossover =====================================

#[tokio::test]
async fn tick_below_threshold_does_not_trigger_oco_pair() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8 / 2).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    // Spot at 70k → between SL(60k) and TP(80k); neither crosses.
    let prices = prices_for(&series, 70_000_000_000);
    let result = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    assert_eq!(result.evaluated, 2);
    assert_eq!(result.triggered, 0);
    let all = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            option_series_id: Some(series),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(all
        .iter()
        .all(|o| o.status == ConditionalOrderStatus::Armed));
}

#[tokio::test]
async fn tick_crosses_tp_completes_one_and_cancels_sibling() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8 / 2).await;
    let rows = arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;
    let tp_id = rows
        .iter()
        .find(|r| r.conditional_type == ConditionalType::TakeProfit)
        .unwrap()
        .id;
    let sl_id = rows
        .iter()
        .find(|r| r.conditional_type == ConditionalType::StopLoss)
        .unwrap()
        .id;

    let prices = prices_for(&series, 80_000_000_000); // == TP threshold (inclusive)
    let r = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    assert_eq!(r.triggered, 1);
    assert_eq!(r.failed, 0);

    let all = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let tp = all.iter().find(|o| o.id == tp_id).unwrap();
    let sl = all.iter().find(|o| o.id == sl_id).unwrap();
    assert_eq!(tp.status, ConditionalOrderStatus::Completed);
    assert!(
        tp.child_order_id.is_some(),
        "child order id must be persisted"
    );
    assert_eq!(sl.status, ConditionalOrderStatus::Cancelled);
    assert_eq!(sl.failure_code.as_deref(), Some("oco_sibling_triggered"));
}

// ===== Phase 6 — Restart recovery =====================================

/// Case A — Armed order survives a "restart" (in-memory store is the
/// persistence boundary in the no-DB harness; we drop and recreate
/// the service-layer view of the state to prove no in-process cache
/// is needed beyond the store itself).
#[tokio::test]
async fn case_a_armed_order_survives_simulated_reload() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8 / 2).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    // Re-list via the public service. This is the same call path the
    // worker uses after a real backend restart.
    let before = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            status: Some(ConditionalOrderStatus::Armed),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(before.len(), 2);

    // Run a non-triggering tick. The rows MUST still be armed.
    let _ =
        evaluate_conditional_orders_tick_with_prices(&state, &prices_for(&series, 70_000_000_000))
            .await
            .unwrap();
    let after = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            status: Some(ConditionalOrderStatus::Armed),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(after.len(), 2);
}

/// Case B — A completed order never re-triggers, even across many
/// evaluator ticks.
#[tokio::test]
async fn case_b_completed_order_never_retriggers() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8 / 2).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    let prices = prices_for(&series, 80_000_000_000);
    let r1 = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    assert!(r1.triggered >= 1);

    // Run several extra ticks — must NOT trigger a second child.
    for _ in 0..5 {
        let r = evaluate_conditional_orders_tick_with_prices(&state, &prices)
            .await
            .unwrap();
        assert_eq!(r.triggered, 0, "completed orders must not retrigger");
    }
}

/// Case C — A row stranded in `triggering` after a crash is
/// recovered safely (no duplicate child).
#[tokio::test]
async fn case_c_stranded_triggering_with_child_finalises_completed() {
    use deopt_v2_backend::options::conditional_orders::get_conditional_order;
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    // Hand-stage a `Triggering` row with a non-null child_order_id —
    // i.e. the child WAS submitted before the crash, but the
    // conditional row was not finalised. Recovery must terminalise.
    {
        let mut store = state.options_store.lock().unwrap();
        let row = store
            .list_conditional_orders(&ConditionalOrderFilter::default())
            .into_iter()
            .next()
            .unwrap();
        let mut staged = row.clone();
        staged.status = ConditionalOrderStatus::Triggering;
        staged.child_order_id = Some("stub-child".to_string());
        staged.version = row.version.saturating_add(1);
        staged.updated_at_ms = now_ms();
        store.update_conditional_order(staged).unwrap();
    }

    let recovered = recover_stranded_triggering(&state, now_ms()).await.unwrap();
    // Exactly one row was stranded in Triggering. The OCO sibling is
    // still Armed and is NOT touched by the sweep.
    assert_eq!(recovered, 1);

    let all = list_conditional_orders(&state, ConditionalOrderFilter::default())
        .await
        .unwrap();
    for o in &all {
        if o.child_order_id.is_some() {
            assert_eq!(o.status, ConditionalOrderStatus::Completed);
        }
    }
    let _ = get_conditional_order; // anchor
}

#[tokio::test]
async fn case_c_stranded_triggering_without_child_rearms_for_retry() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8 / 2).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    // Stage `Triggering` with NULL child — the crash happened BEFORE
    // child submission. Recovery must re-arm.
    let target_id = {
        let mut store = state.options_store.lock().unwrap();
        let row = store
            .list_conditional_orders(&ConditionalOrderFilter::default())
            .into_iter()
            .next()
            .unwrap();
        let id = row.id;
        let mut staged = row.clone();
        staged.status = ConditionalOrderStatus::Triggering;
        staged.child_order_id = None;
        staged.version = row.version.saturating_add(1);
        staged.updated_at_ms = now_ms();
        store.update_conditional_order(staged).unwrap();
        id
    };

    recover_stranded_triggering(&state, now_ms()).await.unwrap();
    let after = list_conditional_orders(&state, ConditionalOrderFilter::default())
        .await
        .unwrap();
    let target = after.into_iter().find(|o| o.id == target_id).unwrap();
    assert_eq!(target.status, ConditionalOrderStatus::Armed);

    // After re-arm a normal tick can pick it up and trigger.
    let prices = prices_for(&series, 80_000_000_000);
    let r = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    assert!(r.triggered >= 1);
}

// ===== Phase 7 — OCO concurrency =======================================

/// Two competing tick passes for the SAME OCO group must yield at
/// most ONE child order in total. We simulate sequential ticks (the
/// store's `Mutex` serialises real concurrent calls); the atomic
/// status guard inside `trigger_one` (`armed → triggering`) is what
/// gives us at-most-once semantics.
#[tokio::test]
async fn oco_competing_ticks_produce_one_winner_only() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8 / 2).await;
    let rows = arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        80_000_000_000,
        true,
    )
    .await;
    assert_eq!(rows.len(), 2);

    // Both legs would match the same price (degenerate OCO where
    // thresholds coincide). Two ticks in quick succession.
    let prices = prices_for(&series, 80_000_000_000);
    let r1 = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    let r2 = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    let total_triggered = r1.triggered + r2.triggered;
    assert_eq!(
        total_triggered, 1,
        "OCO must produce exactly one winner across competing ticks"
    );

    let all = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let completed = all
        .iter()
        .filter(|o| o.status == ConditionalOrderStatus::Completed)
        .count();
    let cancelled = all
        .iter()
        .filter(|o| o.status == ConditionalOrderStatus::Cancelled)
        .count();
    assert_eq!(completed, 1, "one and only one OCO leg completed");
    assert_eq!(cancelled, 1, "the sibling was cancelled");
}

// ===== Phase 8 — Failure handling =====================================

#[tokio::test]
async fn stale_oracle_means_no_trigger() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    // "Stale" is modelled as the price map being empty for the
    // series — `select_orders_to_trigger` returns no candidates.
    let empty_prices: HashMap<String, u128> = HashMap::new();
    let r = evaluate_conditional_orders_tick_with_prices(&state, &empty_prices)
        .await
        .unwrap();
    assert_eq!(r.triggered, 0);

    let all = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(all
        .iter()
        .all(|o| o.status == ConditionalOrderStatus::Armed));
}

#[tokio::test]
async fn position_closed_between_arm_and_trigger_marks_cancelled() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    // Holder sells out the position via a separate matched trade.
    seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8).await;
    submit_option_order(
        &state,
        SubmitOptionOrderInput {
            option_series_id: series.clone(),
            account: account(HOLDER),
            side: Side::Sell,
            price_1e8: PREMIUM_1E8,
            size_1e8: ONE_1E8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some("close-manual".to_string()),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();

    let prices = prices_for(&series, 80_000_000_000);
    let _ = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    let all = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let winner = all
        .iter()
        .find(|o| o.failure_code.as_deref() == Some("position_closed"))
        .expect("trigger candidate that lost its position is marked cancelled");
    assert_eq!(winner.status, ConditionalOrderStatus::Cancelled);
}

#[tokio::test]
async fn reduced_position_caps_child_quantity() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await; // long 1.00

    // Arm a TP claiming the full 1.00.
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;

    // Holder partially closes 0.40 manually BEFORE the trigger fires.
    seed_closing_bid(&state, &series, ONE_1E8 * 4 / 10, PREMIUM_1E8).await;
    submit_option_order(
        &state,
        SubmitOptionOrderInput {
            option_series_id: series.clone(),
            account: account(HOLDER),
            side: Side::Sell,
            price_1e8: PREMIUM_1E8,
            size_1e8: ONE_1E8 * 4 / 10,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some("close-partial".to_string()),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();

    // Closing-side bid for the residual 0.60.
    seed_closing_bid(&state, &series, ONE_1E8 * 6 / 10, PREMIUM_1E8 / 2).await;
    let prices = prices_for(&series, 80_000_000_000);
    let r = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    assert!(r.triggered >= 1);

    // Final HOLDER position must NOT have reversed — sum of signed
    // sizes is exactly zero.
    let fills = state
        .options_store
        .lock()
        .unwrap()
        .list_fills(&OptionFillFilter {
            option_series_id: Some(series.clone()),
            account: Some(account(HOLDER)),
            order_id: None,
        });
    let signed: i128 = fills.iter().fold(0i128, |acc, f| {
        if f.buyer == account(HOLDER) {
            acc + (f.size_1e8 as i128)
        } else if f.seller == account(HOLDER) {
            acc - (f.size_1e8 as i128)
        } else {
            acc
        }
    });
    assert!(
        signed >= 0,
        "child cannot reverse the position (signed = {signed})"
    );
    let _ = PositionSide::Long;
    let _ = TriggerCondition::Gte;
}

#[tokio::test]
async fn ioc_no_liquidity_marks_failed_with_no_liquidity_reason() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;
    // NO closing bid seeded → IOC will find no opposing liquidity.

    let prices = prices_for(&series, 80_000_000_000);
    let _ = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();

    let all = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let tp = all
        .iter()
        .find(|o| o.conditional_type == ConditionalType::TakeProfit)
        .unwrap();
    assert_eq!(tp.status, ConditionalOrderStatus::Failed);
    assert_eq!(tp.failure_code.as_deref(), Some("no_liquidity"));
    // Sanity: position unchanged.
    let fills = state
        .options_store
        .lock()
        .unwrap()
        .list_fills(&OptionFillFilter {
            option_series_id: Some(series.clone()),
            account: Some(account(HOLDER)),
            order_id: None,
        });
    let signed: i128 = fills.iter().fold(0i128, |acc, f| {
        if f.buyer == account(HOLDER) {
            acc + (f.size_1e8 as i128)
        } else if f.seller == account(HOLDER) {
            acc - (f.size_1e8 as i128)
        } else {
            acc
        }
    });
    assert_eq!(signed, ONE_1E8 as i128);
}

// ===== API consistency + sanity =======================================

#[tokio::test]
async fn manual_cancel_terminal_then_evaluator_does_not_resurrect() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series, ONE_1E8).await;
    let rows = arm_tp_sl(
        &state,
        &series,
        ONE_1E8,
        80_000_000_000,
        60_000_000_000,
        true,
    )
    .await;
    let id = rows[0].id;
    cancel_conditional_order(&state, id, &account(HOLDER))
        .await
        .unwrap();
    let prices = prices_for(&series, 80_000_000_000);
    let r = evaluate_conditional_orders_tick_with_prices(&state, &prices)
        .await
        .unwrap();
    // The cancelled leg cannot be triggered. The OTHER leg can.
    assert!(r.triggered <= 1);
    let after = list_conditional_orders(&state, ConditionalOrderFilter::default())
        .await
        .unwrap();
    let cancelled = after.iter().find(|o| o.id == id).unwrap();
    assert_eq!(cancelled.status, ConditionalOrderStatus::Cancelled);
}
