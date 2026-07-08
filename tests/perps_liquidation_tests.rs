//! PERPS-LIQUIDATION-AND-RISK-V1 — liquidation engine tests.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead};
use deopt_v2_backend::perps::{
    evaluate_perp_liquidation, liquidate_perp_position_internal,
    list_perp_liquidations_for_account_view, positions::PerpPositionsStore, prefetch_mark_prices,
    run_perp_liquidation_tick, LiquidationEvaluation, PerpLiquidationStatus, PerpLiquidationsStore,
    PerpOrderStore, PerpPositionStatus, PerpSide, PerpsReadConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use std::collections::HashMap;
use tower::ServiceExt;

const ONE: u128 = 100_000_000;
const PRICE_ETH_3000: u128 = 3000 * ONE;

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

fn seed_long_position(
    positions: &mut PerpPositionsStore,
    account: &AccountId,
    size_1e8: u128,
    entry_1e8: u128,
    margin_1e8: u128,
) -> uuid::Uuid {
    let pos = deopt_v2_backend::perps::positions::new_position_skeleton(
        account.clone(),
        1,
        "ETH-PERP".to_string(),
        PerpSide::Long,
        size_1e8,
        entry_1e8,
        margin_1e8,
    );
    let id = pos.id;
    positions.insert_open(pos).unwrap();
    id
}

fn seed_short_position(
    positions: &mut PerpPositionsStore,
    account: &AccountId,
    size_1e8: u128,
    entry_1e8: u128,
    margin_1e8: u128,
) -> uuid::Uuid {
    let pos = deopt_v2_backend::perps::positions::new_position_skeleton(
        account.clone(),
        1,
        "ETH-PERP".to_string(),
        PerpSide::Short,
        size_1e8,
        entry_1e8,
        margin_1e8,
    );
    let id = pos.id;
    positions.insert_open(pos).unwrap();
    id
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

// =====================================================================
// A. Eligibility math
// =====================================================================

#[test]
fn healthy_long_is_not_liquidated() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let market = cfg.market_by_symbol("ETH-PERP").unwrap();
    let position = deopt_v2_backend::perps::positions::new_position_skeleton(
        addr("0x0000000000000000000000000000000000000aaa"),
        1,
        "ETH-PERP".to_string(),
        PerpSide::Long,
        ONE,
        PRICE_ETH_3000,
        300 * ONE, // 10x
    );
    // Mark at entry → uPnL = 0, equity = margin = 300, notional = 3000,
    // mm = 5% * 3000 = 150. equity (300) > mm (150) → Healthy.
    assert_eq!(
        evaluate_perp_liquidation(market, &position, Some(PRICE_ETH_3000)),
        LiquidationEvaluation::Healthy
    );
}

#[test]
fn under_maintenance_long_is_liquidated() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let market = cfg.market_by_symbol("ETH-PERP").unwrap();
    let position = deopt_v2_backend::perps::positions::new_position_skeleton(
        addr("0x0000000000000000000000000000000000000aaa"),
        1,
        "ETH-PERP".to_string(),
        PerpSide::Long,
        ONE,
        PRICE_ETH_3000,
        300 * ONE,
    );
    // Deep dump: mark $2700, uPnL = -300, equity = 0. mm at $2700
    // notional 2700 * 5% = 135. equity (0) <= mm (135) → Liquidatable.
    assert_eq!(
        evaluate_perp_liquidation(market, &position, Some(2700 * ONE)),
        LiquidationEvaluation::Liquidatable
    );
}

#[test]
fn under_maintenance_short_is_liquidated() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let market = cfg.market_by_symbol("ETH-PERP").unwrap();
    let position = deopt_v2_backend::perps::positions::new_position_skeleton(
        addr("0x0000000000000000000000000000000000000aaa"),
        1,
        "ETH-PERP".to_string(),
        PerpSide::Short,
        ONE,
        PRICE_ETH_3000,
        300 * ONE,
    );
    // Short at $3000; mark rips to $3400 → uPnL = -400, equity = -100.
    assert_eq!(
        evaluate_perp_liquidation(market, &position, Some(3400 * ONE)),
        LiquidationEvaluation::Liquidatable
    );
}

#[test]
fn stale_price_returns_price_unavailable() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let market = cfg.market_by_symbol("ETH-PERP").unwrap();
    let position = deopt_v2_backend::perps::positions::new_position_skeleton(
        addr("0x0000000000000000000000000000000000000aaa"),
        1,
        "ETH-PERP".to_string(),
        PerpSide::Long,
        ONE,
        PRICE_ETH_3000,
        300 * ONE,
    );
    assert_eq!(
        evaluate_perp_liquidation(market, &position, None),
        LiquidationEvaluation::PriceUnavailable
    );
}

// =====================================================================
// B. liquidate_perp_position_internal — full flow
// =====================================================================

#[tokio::test]
async fn liquidation_closes_position_realises_pnl_and_records_event() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    let mut rx = state.lifecycle_events.subscribe();
    let event = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        liquidate_perp_position_internal(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &state.lifecycle_events,
            &alice,
            1,
            "ETH-PERP",
            Some(2700 * ONE),
            now_ms(),
        )
        .unwrap()
        .expect("liquidation event")
    };
    assert_eq!(event.status, PerpLiquidationStatus::Completed);
    assert_eq!(event.reason_code, "margin_breach");
    // uPnL = -300, realized = -300
    assert_eq!(event.realized_pnl_1e8, -300 * ONE as i128);
    // equity = 0, bad_debt = 0
    assert_eq!(event.bad_debt_1e8, 0);
    // Position store should mark it as Liquidated.
    let positions = state.perp_positions_store.lock().unwrap();
    assert!(positions.get_active(&alice, 1, "ETH-PERP").is_none());
    let history = positions.list_for_account(&alice);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, PerpPositionStatus::Liquidated);
    // Lifecycle: PerpPositionUpdated (status=liquidated) + PerpPositionLiquidated.
    let events = drain(&mut rx);
    let updated_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpPositionUpdated { .. }))
        .count();
    let liquidated_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpPositionLiquidated { .. }))
        .count();
    assert_eq!(updated_count, 1);
    assert_eq!(liquidated_count, 1);
}

#[tokio::test]
async fn liquidation_records_bad_debt_when_equity_is_negative() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    let event = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        // Mark drops to $2500 → uPnL = -500, equity = -200 → bad debt 200.
        liquidate_perp_position_internal(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &state.lifecycle_events,
            &alice,
            1,
            "ETH-PERP",
            Some(2500 * ONE),
            now_ms(),
        )
        .unwrap()
        .expect("liquidation event")
    };
    assert_eq!(event.bad_debt_1e8, 200 * ONE);
}

#[tokio::test]
async fn stale_price_liquidation_records_price_unavailable_event_no_state_change() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    let event = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        liquidate_perp_position_internal(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &state.lifecycle_events,
            &alice,
            1,
            "ETH-PERP",
            None, // mark unavailable
            now_ms(),
        )
        .unwrap()
        .expect("event should still be recorded to inform operators")
    };
    assert_eq!(event.status, PerpLiquidationStatus::PriceUnavailable);
    // Position remains Open.
    let positions = state.perp_positions_store.lock().unwrap();
    let pos = positions.get_active(&alice, 1, "ETH-PERP").unwrap();
    assert_eq!(pos.status, PerpPositionStatus::Open);
}

#[tokio::test]
async fn healthy_position_liquidation_returns_none_no_state_change() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    let result = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        liquidate_perp_position_internal(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &state.lifecycle_events,
            &alice,
            1,
            "ETH-PERP",
            Some(PRICE_ETH_3000), // healthy mark
            now_ms(),
        )
        .unwrap()
    };
    assert!(result.is_none());
    let positions = state.perp_positions_store.lock().unwrap();
    let pos = positions.get_active(&alice, 1, "ETH-PERP").unwrap();
    assert_eq!(pos.status, PerpPositionStatus::Open);
}

// =====================================================================
// C. Idempotency
// =====================================================================

#[tokio::test]
async fn repeated_liquidation_is_idempotent() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    let mut marks: HashMap<String, Option<u128>> = HashMap::new();
    marks.insert("ETH-PERP".to_string(), Some(2700 * ONE));
    marks.insert("BTC-PERP".to_string(), Some(65_000 * ONE));
    for _ in 0..3 {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        run_perp_liquidation_tick(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &marks,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap();
    }
    let liquidations = state.perp_liquidations_store.lock().unwrap();
    let all = liquidations.list_for_account(&alice);
    // Exactly one liquidation event across three tick runs.
    assert_eq!(all.len(), 1);
}

// =====================================================================
// D. Order cancellation on liquidation
// =====================================================================

#[tokio::test]
async fn liquidation_cancels_open_orders_for_same_account_and_market() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let bob = addr("0x0000000000000000000000000000000000000bbb");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    // Alice has open ETH-PERP AND BTC-PERP orders; Bob has an
    // unrelated ETH-PERP order.
    let (alice_eth_id, alice_btc_id, bob_eth_id) = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let alice_eth = deopt_v2_backend::perps::PerpOrder::new(
            alice.clone(),
            1,
            "ETH-PERP".to_string(),
            deopt_v2_backend::perps::PerpOrderSide::Buy,
            2900 * ONE,
            ONE,
            deopt_v2_backend::perps::PerpTimeInForce::Gtc,
            false,
            false,
            300 * ONE,
            None,
            now_ms(),
        );
        let alice_btc = deopt_v2_backend::perps::PerpOrder::new(
            alice.clone(),
            1,
            "BTC-PERP".to_string(),
            deopt_v2_backend::perps::PerpOrderSide::Buy,
            60_000 * ONE,
            ONE,
            deopt_v2_backend::perps::PerpTimeInForce::Gtc,
            false,
            false,
            12_000 * ONE,
            None,
            now_ms(),
        );
        let bob_eth = deopt_v2_backend::perps::PerpOrder::new(
            bob.clone(),
            1,
            "ETH-PERP".to_string(),
            deopt_v2_backend::perps::PerpOrderSide::Buy,
            2800 * ONE,
            ONE,
            deopt_v2_backend::perps::PerpTimeInForce::Gtc,
            false,
            false,
            300 * ONE,
            None,
            now_ms(),
        );
        let alice_eth_id = alice_eth.id;
        let alice_btc_id = alice_btc.id;
        let bob_eth_id = bob_eth.id;
        orders.insert_order(alice_eth).unwrap();
        orders.insert_order(alice_btc).unwrap();
        orders.insert_order(bob_eth).unwrap();
        (alice_eth_id, alice_btc_id, bob_eth_id)
    };
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        liquidate_perp_position_internal(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &state.lifecycle_events,
            &alice,
            1,
            "ETH-PERP",
            Some(2700 * ONE),
            now_ms(),
        )
        .unwrap();
    }
    let orders = state.perp_order_store.lock().unwrap();
    let alice_eth = orders.get(alice_eth_id).unwrap();
    let alice_btc = orders.get(alice_btc_id).unwrap();
    let bob_eth = orders.get(bob_eth_id).unwrap();
    assert_eq!(
        alice_eth.status,
        deopt_v2_backend::perps::PerpOrderStatus::Cancelled
    );
    assert_eq!(
        alice_eth.terminal_reason_code.as_deref(),
        Some("liquidated")
    );
    // Different market for alice — unaffected.
    assert!(alice_btc.status.is_active());
    // Different account — unaffected.
    assert!(bob_eth.status.is_active());
}

// =====================================================================
// E. Tick — mix of healthy/liquidatable/unavailable
// =====================================================================

#[tokio::test]
async fn tick_liquidates_only_the_unhealthy_positions() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let healthy = addr("0x0000000000000000000000000000000000000aaa");
    let unhealthy = addr("0x0000000000000000000000000000000000000bbb");
    let short_unhealthy = addr("0x0000000000000000000000000000000000000ccc");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &healthy, ONE, PRICE_ETH_3000, 300 * ONE);
        seed_long_position(&mut positions, &unhealthy, ONE, PRICE_ETH_3000, 300 * ONE);
        seed_short_position(
            &mut positions,
            &short_unhealthy,
            ONE,
            PRICE_ETH_3000,
            300 * ONE,
        );
    }
    let mut marks: HashMap<String, Option<u128>> = HashMap::new();
    // Long is healthy at $3000; unhealthy_long fails at $2700; short
    // fails at $3400 — but we can only set one mark per market, so
    // let's use $2700 (long-unfriendly). Then healthy_long is also
    // unhealthy at $2700, so let's just do one healthy + one unhealthy
    // in the same market at $2700. The short at $2700 is even more
    // healthy.
    marks.insert("ETH-PERP".to_string(), Some(2700 * ONE));
    marks.insert("BTC-PERP".to_string(), None);
    let response = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        run_perp_liquidation_tick(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &marks,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    assert_eq!(response.checked_count, 3);
    assert_eq!(response.liquidated_count, 2, "both longs get liquidated");
    let short_pos = state
        .perp_positions_store
        .lock()
        .unwrap()
        .get_active(&short_unhealthy, 1, "ETH-PERP")
        .unwrap();
    assert_eq!(short_pos.status, PerpPositionStatus::Open);
}

// =====================================================================
// F. Read endpoint
// =====================================================================

#[tokio::test]
async fn account_liquidations_endpoint_returns_empty_by_default() {
    let app = router(state());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/accounts/0x000000000000000000000000000000000000abcd/perps/liquidations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["liquidations"].as_array().map(|a| a.len()), Some(0));
    assert_eq!(body["chain_id"].as_u64(), Some(84532));
    assert_eq!(body["trading_enabled"].as_bool(), Some(false));
}

#[tokio::test]
async fn account_liquidations_endpoint_surfaces_persisted_events() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x000000000000000000000000000000000000dEaD");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        liquidate_perp_position_internal(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &state.lifecycle_events,
            &alice,
            1,
            "ETH-PERP",
            Some(2500 * ONE),
            now_ms(),
        )
        .unwrap();
    }
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/accounts/0x000000000000000000000000000000000000dEaD/perps/liquidations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let list = body["liquidations"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    let row = &list[0];
    assert_eq!(row["market_id"].as_str(), Some("ETH-PERP"));
    assert_eq!(row["side"].as_str(), Some("long"));
    assert_eq!(row["reason_code"].as_str(), Some("margin_breach"));
    assert_eq!(row["trading_enabled"].as_bool(), Some(false));
}

// =====================================================================
// G. Admin tick gate + fail-closed regression
// =====================================================================

#[tokio::test]
async fn admin_tick_refuses_without_admin_token() {
    let app = router(state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/perps/liquidations/tick")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // No admin token → refused by admin gate.
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn public_perp_submit_still_fail_closed_after_liquidation_ships() {
    let app = router(state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "market_id": 1,
                        "account": "0x0000000000000000000000000000000000000abc",
                        "side": "buy",
                        "price_1e8": "1000",
                        "size_1e8": "10",
                        "time_in_force": "gtc",
                        "reduce_only": false,
                        "post_only": false,
                        "client_order_id": null,
                        "signed_nonce": null,
                        "signed_deadline_ms": null
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("perp"));
}

// =====================================================================
// H. No-secret assertion on liquidation lifecycle frame
// =====================================================================

#[tokio::test]
async fn perp_position_liquidated_frame_has_no_secret_fields() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, PRICE_ETH_3000, 300 * ONE);
    }
    let mut rx = state.lifecycle_events.subscribe();
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut liquidations = state.perp_liquidations_store.lock().unwrap();
        liquidate_perp_position_internal(
            &cfg,
            &mut positions,
            &mut orders,
            &mut liquidations,
            &state.lifecycle_events,
            &alice,
            1,
            "ETH-PERP",
            Some(2500 * ONE),
            now_ms(),
        )
        .unwrap();
    }
    let events = drain(&mut rx);
    let liq_frame = events
        .iter()
        .find(|e| matches!(e.payload, LifecyclePayload::PerpPositionLiquidated { .. }))
        .expect("liquidation frame");
    let json = serde_json::to_string(liq_frame).unwrap();
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
            "PerpPositionLiquidated frame contained banned field `{banned}`: {json}"
        );
    }
    assert!(matches!(
        liq_frame.channel,
        LifecycleChannel::AccountPerpPositions
    ));
    assert_eq!(liq_frame.account, alice);
}

// =====================================================================
// I. Prefetch mark prices helper — smoke
// =====================================================================

#[tokio::test]
async fn prefetch_mark_prices_returns_a_map_per_configured_market() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let reader = InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: PRICE_ETH_3000,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    );
    let marks = prefetch_mark_prices(&cfg, &reader, now_ms()).await;
    assert_eq!(
        marks.get("ETH-PERP").copied().unwrap(),
        Some(PRICE_ETH_3000)
    );
    // BTC-PERP wasn't seeded → None.
    assert_eq!(marks.get("BTC-PERP").copied().unwrap(), None);
}

// Silence unused import warning if any.
#[test]
fn _list_view_helper_is_reachable() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let store = PerpLiquidationsStore::new();
    let resp = list_perp_liquidations_for_account_view(
        &cfg,
        &store,
        &addr("0x0000000000000000000000000000000000000aaa"),
    );
    assert_eq!(resp.chain_id, 84532);
    assert!(resp.liquidations.is_empty());
    assert!(!resp.trading_enabled);
    let _ = PerpOrderStore::new(); // silence unused import
}
