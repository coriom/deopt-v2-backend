// OPTIONS-CONDITIONAL-ORDERS-TP-SL-V1 — end-to-end integration tests.
//
// Drives the conditional-orders service through its public surface
// (`create_conditional_orders`, `list_conditional_orders`,
// `cancel_conditional_order`, `evaluate_conditional_orders_tick`)
// against the in-memory store. Positions are built by submitting
// real matching orders so the reduce-only path is exercised end-to-
// end. The DB path mirrors the in-memory path through the repository
// (see `src/db/repository.rs`).

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::conditional_orders::{
    cancel_conditional_order, create_conditional_orders, evaluate_conditional_orders_tick,
    list_conditional_orders, ConditionalLegInput, ConditionalOrderFilter, ConditionalOrderStatus,
    ConditionalType, CreateConditionalOrderInput, OptionKind, PositionSide, TriggerCondition,
};
use deopt_v2_backend::options::service::{
    create_option_series, submit_option_order, CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use uuid::Uuid;

const ONE_1E8: u128 = 100_000_000;
const PREMIUM_1E8: u128 = 1_000_000_000; // $10.00

fn state() -> AppState {
    AppState::with_options_config(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
    )
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

fn account(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

const HOLDER: &str = "0x0000000000000000000000000000000000000111";
const MAKER: &str = "0x0000000000000000000000000000000000000222";

/// Build a long-100 position for HOLDER on `series` by matching a
/// HOLDER buy against a MAKER sell at `PREMIUM_1E8`.
async fn seed_long_position(state: &AppState, series: &str) {
    // Maker sells 1.00.
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
            client_order_id: Some("seed-maker".to_string()),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();
    // Holder buys 1.00 — matches the maker, leaves Holder long 1.00.
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
            client_order_id: Some("seed-holder".to_string()),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();
}

// ===== Creation =======================================================

#[tokio::test]
async fn create_long_call_tp_is_armed_with_gte_comparator() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;

    let rows = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series.clone(),
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: PREMIUM_1E8 * 2,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, ConditionalOrderStatus::Armed);
    assert_eq!(rows[0].trigger_condition, TriggerCondition::Gte);
    assert_eq!(rows[0].option_kind, OptionKind::Call);
    assert_eq!(rows[0].position_side, PositionSide::Long);
    assert!(rows[0].reduce_only);
}

#[tokio::test]
async fn create_long_call_sl_is_armed_with_lte_comparator() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;

    let rows = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::StopLoss,
                trigger_price_1e8: 60_000_000_000,
                limit_price_1e8: PREMIUM_1E8 / 2,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(rows[0].trigger_condition, TriggerCondition::Lte);
}

#[tokio::test]
async fn missing_position_rejected_with_no_reducible_position() {
    let state = state();
    let series = make_active_series(&state, true).await;
    // NO seed_long_position — holder has no position.
    let err = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: PREMIUM_1E8 * 2,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("no reducible option position"),
        "got {err}"
    );
}

#[tokio::test]
async fn excessive_quantity_rejected() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;
    let err = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8 * 5, // more than the seeded 1.00
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: PREMIUM_1E8 * 2,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("exceeds the reducible"), "{err}");
}

#[tokio::test]
async fn explicit_inconsistent_comparator_rejected() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;
    // long-call TP should derive Gte; client sends Lte → reject.
    let err = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: PREMIUM_1E8 * 2,
                explicit_trigger_condition: Some(TriggerCondition::Lte),
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("trigger direction"), "{err}");
}

#[tokio::test]
async fn oco_pair_shares_group_and_assigns_both_legs() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;
    let rows = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8,
            legs: vec![
                ConditionalLegInput {
                    conditional_type: ConditionalType::TakeProfit,
                    trigger_price_1e8: 80_000_000_000,
                    limit_price_1e8: PREMIUM_1E8 * 2,
                    explicit_trigger_condition: None,
                },
                ConditionalLegInput {
                    conditional_type: ConditionalType::StopLoss,
                    trigger_price_1e8: 60_000_000_000,
                    limit_price_1e8: PREMIUM_1E8 / 2,
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
    let g0 = rows[0].oco_group_id.expect("oco group set");
    let g1 = rows[1].oco_group_id.expect("oco group set");
    assert_eq!(g0, g1);
}

// ===== Triggering — evaluator skip-safely ============================
//
// The full trigger → child IOC → terminal flow is exercised by the
// in-store unit tests + the (kind, side, conditional_type) →
// comparator unit tests in `src/options/conditional_orders.rs`. Here
// we cover the evaluator surface that callers see: when oracle is
// unconfigured, the tick must be a no-op with `skipped_oracle_unavailable=true`.

/// Lock the in-memory store and drive the trigger→execute→OCO-cancel
/// path directly. This exercises the same code the worker uses once
/// `select_orders_to_trigger` has chosen the winner, but lets us
/// skip the oracle round-trip.
#[tokio::test]
async fn oco_winner_completes_and_cancels_sibling_via_in_store_execute() {
    use deopt_v2_backend::options::conditional_orders::execute_triggered_in_store;
    use deopt_v2_backend::options::service::get_option_series;
    let state = state();
    let series_id = make_active_series(&state, true).await;
    seed_long_position(&state, &series_id).await;

    // Arm a TP + SL OCO pair on the long position.
    let rows = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series_id.clone(),
            quantity_1e8: ONE_1E8,
            legs: vec![
                ConditionalLegInput {
                    conditional_type: ConditionalType::TakeProfit,
                    trigger_price_1e8: 80_000_000_000,
                    limit_price_1e8: PREMIUM_1E8 / 2,
                    explicit_trigger_condition: None,
                },
                ConditionalLegInput {
                    conditional_type: ConditionalType::StopLoss,
                    trigger_price_1e8: 60_000_000_000,
                    limit_price_1e8: PREMIUM_1E8 / 2,
                    explicit_trigger_condition: None,
                },
            ],
            link_as_oco: true,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    let tp = rows
        .iter()
        .find(|r| r.conditional_type == ConditionalType::TakeProfit)
        .unwrap()
        .clone();
    let sl = rows
        .iter()
        .find(|r| r.conditional_type == ConditionalType::StopLoss)
        .unwrap()
        .clone();

    // Maker now puts a Buy 1.00 on the book so the IOC child (which
    // is a Sell to close the long) can match.
    submit_option_order(
        &state,
        SubmitOptionOrderInput {
            option_series_id: series_id.clone(),
            account: account(MAKER),
            side: Side::Buy,
            price_1e8: PREMIUM_1E8 / 2,
            size_1e8: ONE_1E8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some("close-bid".to_string()),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();

    // Fire the TP leg directly.
    let series = get_option_series(&state, &series_id).await.unwrap();
    let mut store = state.options_store.lock().unwrap();
    // ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1: the 5th arg is the
    // optional `WorkerLifecycleBatch` used by `trigger_one` to collect
    // emit-after-commit events. Pure unit tests don't observe the WS
    // surface so `None` is the right value here.
    let completed = execute_triggered_in_store(&mut store, &series, tp.id, now_ms(), None).unwrap();
    assert_eq!(completed.status, ConditionalOrderStatus::Completed);
    assert!(completed.child_order_id.is_some());

    // SL sibling must have been atomically cancelled with reason.
    let sl_after = store.get_conditional_order(sl.id).unwrap();
    assert_eq!(sl_after.status, ConditionalOrderStatus::Cancelled);
    assert_eq!(
        sl_after.failure_code.as_deref(),
        Some("oco_sibling_triggered")
    );
}

#[tokio::test]
async fn evaluator_skips_when_oracle_unconfigured_or_provider_missing() {
    let state = state();
    // No provider → skip.
    let out = evaluate_conditional_orders_tick::<deopt_v2_backend::execution::HttpJsonRpcProvider>(
        &state, None,
    )
    .await
    .unwrap();
    assert!(out.skipped_oracle_unavailable);
    assert_eq!(out.evaluated, 0);
    assert_eq!(out.triggered, 0);
}

// ===== Cancellation ==================================================

#[tokio::test]
async fn cancel_armed_order_transitions_to_cancelled() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;
    let rows = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: PREMIUM_1E8 * 2,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    let id = rows[0].id;
    let cancelled = cancel_conditional_order(&state, id, &account(HOLDER))
        .await
        .unwrap();
    assert_eq!(cancelled.status, ConditionalOrderStatus::Cancelled);
    assert!(cancelled.completed_at_ms.is_some());

    // Second cancel must fail with already_terminal.
    let err = cancel_conditional_order(&state, id, &account(HOLDER))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already terminal"), "{err}");
}

#[tokio::test]
async fn cross_wallet_cancel_rejected_as_invalid_id() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;
    let rows = create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: PREMIUM_1E8 * 2,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    let id = rows[0].id;
    let err = cancel_conditional_order(&state, id, &account(MAKER))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("invalid conditional order id"),
        "{err}"
    );
}

#[tokio::test]
async fn list_returns_only_owner_account_rows() {
    let state = state();
    let series = make_active_series(&state, true).await;
    seed_long_position(&state, &series).await;
    create_conditional_orders(
        &state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series,
            quantity_1e8: ONE_1E8,
            legs: vec![ConditionalLegInput {
                conditional_type: ConditionalType::TakeProfit,
                trigger_price_1e8: 80_000_000_000,
                limit_price_1e8: PREMIUM_1E8 * 2,
                explicit_trigger_condition: None,
            }],
            link_as_oco: false,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap();
    let holder_rows = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(HOLDER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(holder_rows.len(), 1);

    let stranger_rows = list_conditional_orders(
        &state,
        ConditionalOrderFilter {
            account: Some(account(MAKER)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(stranger_rows.len(), 0);
}

#[tokio::test]
async fn get_by_unknown_id_returns_none() {
    let state = state();
    let got = deopt_v2_backend::options::conditional_orders::get_conditional_order(
        &state,
        Uuid::new_v4(),
    )
    .await
    .unwrap();
    assert!(got.is_none());
}
