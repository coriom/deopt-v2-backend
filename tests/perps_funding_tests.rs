//! PERPS-FUNDING-V1 — funding engine tests (in-memory).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    calculate_funding_payment_1e8, list_perp_funding_events_for_account_view,
    positions::apply_funding_payment_to_margin, positions::PerpPositionsStore,
    prefetch_funding_indices, run_perp_funding_tick, FundingIndexRead,
    InMemoryPerpFundingIndexReader, PerpFundingEventsStore, PerpPositionStatus, PerpSide,
    PerpsReadConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use std::collections::HashMap;
use tower::ServiceExt;

const ONE: u128 = 100_000_000;
const ONE_1E18: i128 = 1_000_000_000_000_000_000;

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
// A. Payment formula
// =====================================================================

#[test]
fn long_pays_positive_funding() {
    // Long 1e8 (one contract), funding delta = +0.001 * 1e18 (0.1%).
    // payment = +1 * 1e8 * 1e15 / 1e18 = 1e5 = 0.001 in 1e8 quote.
    let delta = 1_000_000_000_000_000i128; // 0.001 * 1e18
    let payment = calculate_funding_payment_1e8(PerpSide::Long, ONE, 0, delta);
    assert_eq!(payment, 100_000);
    assert!(payment > 0, "long owes when delta positive");
}

#[test]
fn short_receives_positive_funding() {
    // Short 1e8, delta = +0.001 * 1e18. side_sign = -1 → payment = -100_000.
    let delta = 1_000_000_000_000_000i128;
    let payment = calculate_funding_payment_1e8(PerpSide::Short, ONE, 0, delta);
    assert_eq!(payment, -100_000);
    assert!(payment < 0, "short receives when delta positive");
}

#[test]
fn long_receives_negative_funding() {
    let delta = -1_000_000_000_000_000i128;
    let payment = calculate_funding_payment_1e8(PerpSide::Long, ONE, 0, delta);
    assert!(payment < 0, "long receives when delta negative");
    assert_eq!(payment, -100_000);
}

#[test]
fn short_pays_negative_funding() {
    let delta = -1_000_000_000_000_000i128;
    let payment = calculate_funding_payment_1e8(PerpSide::Short, ONE, 0, delta);
    assert!(payment > 0, "short owes when delta negative");
    assert_eq!(payment, 100_000);
}

#[test]
fn zero_delta_payment_is_zero() {
    let payment = calculate_funding_payment_1e8(PerpSide::Long, ONE, ONE_1E18, ONE_1E18);
    assert_eq!(payment, 0);
}

#[test]
fn zero_size_payment_is_zero() {
    let payment = calculate_funding_payment_1e8(PerpSide::Long, 0, 0, ONE_1E18);
    assert_eq!(payment, 0);
}

// =====================================================================
// B. Margin application (saturating semantics)
// =====================================================================

#[test]
fn positive_payment_reduces_margin() {
    let (m, bad) = apply_funding_payment_to_margin(1_000, 400);
    assert_eq!(m, 600);
    assert_eq!(bad, 0);
}

#[test]
fn negative_payment_increases_margin() {
    let (m, bad) = apply_funding_payment_to_margin(1_000, -400);
    assert_eq!(m, 1_400);
    assert_eq!(bad, 0);
}

#[test]
fn oversized_positive_payment_saturates_margin_and_records_bad_debt() {
    let (m, bad) = apply_funding_payment_to_margin(1_000, 1_500);
    assert_eq!(m, 0);
    assert_eq!(bad, 500);
}

#[test]
fn zero_payment_is_no_op_on_margin() {
    let (m, bad) = apply_funding_payment_to_margin(1_000, 0);
    assert_eq!(m, 1_000);
    assert_eq!(bad, 0);
}

// =====================================================================
// C. In-memory tick — end-to-end
// =====================================================================

#[test]
fn tick_settles_long_with_positive_index_delta_and_reduces_margin() {
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let seeded_id = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, 300 * ONE, 300 * ONE)
    };
    let indices = HashMap::from([
        (
            "ETH-PERP".to_string(),
            Some(FundingIndexRead {
                cumulative_index_1e18: ONE_1E18 / 100, // 0.01 * 1e18
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            }),
        ),
        ("BTC-PERP".to_string(), None),
    ]);
    let response = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    assert_eq!(response.checked_count, 1);
    assert_eq!(response.settled_count, 1);
    assert_eq!(response.skipped_source_unavailable_count, 0);
    assert!(!response.trading_enabled);
    assert_eq!(response.chain_id, 84532);

    // Position margin decreased.
    let positions = state.perp_positions_store.lock().unwrap();
    let post = positions.by_id(seeded_id).unwrap();
    assert!(post.margin_1e8 < 300 * ONE);
    assert_eq!(post.last_funding_index_1e18, ONE_1E18 / 100);
    assert!(post.cumulative_funding_payment_1e8 > 0);
}

#[test]
fn tick_settles_short_with_positive_index_delta_and_credits_margin() {
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let seeded_id = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_short_position(&mut positions, &alice, ONE, 300 * ONE, 300 * ONE)
    };
    let indices = HashMap::from([
        (
            "ETH-PERP".to_string(),
            Some(FundingIndexRead {
                cumulative_index_1e18: ONE_1E18 / 100,
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            }),
        ),
        ("BTC-PERP".to_string(), None),
    ]);
    let _ = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    let positions = state.perp_positions_store.lock().unwrap();
    let post = positions.by_id(seeded_id).unwrap();
    assert!(
        post.margin_1e8 > 300 * ONE,
        "short receives funding when delta positive"
    );
    assert!(post.cumulative_funding_payment_1e8 < 0);
}

#[test]
fn tick_is_idempotent_at_same_index() {
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let seeded_id = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, 300 * ONE, 300 * ONE)
    };
    let indices = HashMap::from([
        (
            "ETH-PERP".to_string(),
            Some(FundingIndexRead {
                cumulative_index_1e18: ONE_1E18 / 100,
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            }),
        ),
        ("BTC-PERP".to_string(), None),
    ]);
    // First tick.
    let r1 = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    assert_eq!(r1.settled_count, 1);
    // Second tick with the SAME index — no new settlement.
    let r2 = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    assert_eq!(r2.checked_count, 1);
    assert_eq!(r2.settled_count, 0, "no settlement at the same index");
    let events_store = state.perp_funding_events_store.lock().unwrap();
    assert_eq!(events_store.list_for_account(&alice).len(), 1);
    let _ = seeded_id;
}

#[test]
fn tick_skips_market_when_source_unavailable() {
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, 300 * ONE, 300 * ONE);
    }
    // Both markets report `None`.
    let indices = HashMap::from([
        ("ETH-PERP".to_string(), None),
        ("BTC-PERP".to_string(), None),
    ]);
    let response = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    assert_eq!(response.checked_count, 1);
    assert_eq!(response.settled_count, 0);
    assert_eq!(response.skipped_source_unavailable_count, 1);
    // Position is untouched.
    let positions = state.perp_positions_store.lock().unwrap();
    let post = positions.list_for_account(&alice)[0].clone();
    assert_eq!(post.status, PerpPositionStatus::Open);
    assert_eq!(post.margin_1e8, 300 * ONE);
    assert_eq!(post.last_funding_index_1e18, 0);
    assert_eq!(post.cumulative_funding_payment_1e8, 0);
}

#[test]
fn tick_settles_multiple_positions() {
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let bob = addr("0x0000000000000000000000000000000000000bbb");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, 300 * ONE, 300 * ONE);
        seed_short_position(&mut positions, &bob, ONE, 300 * ONE, 300 * ONE);
    }
    let indices = HashMap::from([
        (
            "ETH-PERP".to_string(),
            Some(FundingIndexRead {
                cumulative_index_1e18: ONE_1E18 / 100,
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            }),
        ),
        ("BTC-PERP".to_string(), None),
    ]);
    let response = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    assert_eq!(response.checked_count, 2);
    assert_eq!(response.settled_count, 2);
    assert_eq!(response.funding_event_ids.len(), 2);
}

#[test]
fn tick_skips_closed_positions() {
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, 300 * ONE, 300 * ONE);
        positions
            .close_active(&alice, "ETH-PERP", now_ms())
            .unwrap();
    }
    let indices = HashMap::from([
        (
            "ETH-PERP".to_string(),
            Some(FundingIndexRead {
                cumulative_index_1e18: ONE_1E18 / 100,
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            }),
        ),
        ("BTC-PERP".to_string(), None),
    ]);
    let response = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    assert_eq!(response.checked_count, 0);
    assert_eq!(response.settled_count, 0);
}

#[test]
fn tick_does_not_auto_liquidate_when_margin_zeroed() {
    // Set up a tiny position whose margin gets completely wiped by
    // funding. Position should remain Open (funding never
    // auto-liquidates); liquidation tick is the ONLY liquidator.
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let seeded_id = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, 300 * ONE, 30 * ONE)
    };
    // Big positive delta → funding payment > margin.
    // payment = size(1e8) * delta / 1e18 → to exceed margin=30*1e8=3e9
    // we need delta > 3e9 * 1e18 / 1e8 = 3e19. Use 100 * 1e18 = 1e20.
    let indices = HashMap::from([
        (
            "ETH-PERP".to_string(),
            Some(FundingIndexRead {
                cumulative_index_1e18: ONE_1E18.saturating_mul(100), // 100 * 1e18
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            }),
        ),
        ("BTC-PERP".to_string(), None),
    ]);
    let _ = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    let positions = state.perp_positions_store.lock().unwrap();
    let post = positions.by_id(seeded_id).unwrap();
    assert_eq!(post.status, PerpPositionStatus::Open, "no auto-liquidation");
    assert_eq!(post.margin_1e8, 0);
    let events_store = state.perp_funding_events_store.lock().unwrap();
    let events = events_store.list_for_account(&alice);
    assert_eq!(events.len(), 1);
    assert!(events[0].bad_debt_1e8 > 0);
}

// =====================================================================
// D. Lifecycle
// =====================================================================

#[tokio::test]
async fn tick_emits_perp_position_updated_and_funding_payment_created() {
    let state = state();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut positions = state.perp_positions_store.lock().unwrap();
        seed_long_position(&mut positions, &alice, ONE, 300 * ONE, 300 * ONE);
    }
    let mut rx = state.lifecycle_events.subscribe();
    let indices = HashMap::from([
        (
            "ETH-PERP".to_string(),
            Some(FundingIndexRead {
                cumulative_index_1e18: ONE_1E18 / 100,
                updated_at_sec: (now_ms() / 1000) as u64,
                ok: true,
            }),
        ),
        ("BTC-PERP".to_string(), None),
    ]);
    let _ = {
        let mut positions = state.perp_positions_store.lock().unwrap();
        let mut events = state.perp_funding_events_store.lock().unwrap();
        run_perp_funding_tick(
            &state.perps_read_config,
            &mut positions,
            &mut events,
            &indices,
            &state.lifecycle_events,
            now_ms(),
        )
        .unwrap()
    };
    let events = drain(&mut rx);
    let updated = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpPositionUpdated { .. }))
        .count();
    let funding = events
        .iter()
        .filter(|e| {
            matches!(
                e.payload,
                LifecyclePayload::PerpFundingPaymentCreated { .. }
            )
        })
        .count();
    assert_eq!(updated, 1, "PerpPositionUpdated must fire");
    assert_eq!(funding, 1, "PerpFundingPaymentCreated must fire");
    // Funding event routes on account.perp_funding.
    let funding_frame = events
        .iter()
        .find(|e| {
            matches!(
                e.payload,
                LifecyclePayload::PerpFundingPaymentCreated { .. }
            )
        })
        .unwrap();
    assert!(matches!(
        funding_frame.channel,
        LifecycleChannel::AccountPerpFunding
    ));
    // No secrets in any payload.
    for ev in &events {
        let s = serde_json::to_string(&ev.payload).unwrap().to_lowercase();
        for banned in [
            "signature",
            "auth",
            "bearer",
            "token",
            "cookie",
            "password",
            "private_key",
            "secret",
            "rpc_url",
        ] {
            assert!(
                !s.contains(banned),
                "lifecycle payload contains banned substring `{banned}`: {s}"
            );
        }
    }
}

// =====================================================================
// E. Account read endpoint
// =====================================================================

#[test]
fn list_view_helper_surfaces_persisted_events_newest_first() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let mut store = PerpFundingEventsStore::new();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let ev = deopt_v2_backend::perps::PerpFundingEvent {
        id: uuid::Uuid::new_v4(),
        account: alice.clone(),
        market_id: "ETH-PERP".to_string(),
        position_id: uuid::Uuid::new_v4(),
        side: PerpSide::Long,
        position_size_1e8: ONE,
        funding_index_before_1e18: 0,
        funding_index_after_1e18: ONE_1E18 / 100,
        funding_delta_1e18: ONE_1E18 / 100,
        payment_1e8: 100_000,
        margin_before_1e8: 300 * ONE,
        margin_after_1e8: 300 * ONE - 100_000,
        bad_debt_1e8: 0,
        reason_code: "funding_settlement".to_string(),
        created_at_ms: now_ms(),
    };
    store.insert(ev.clone());
    let view = list_perp_funding_events_for_account_view(&cfg, &store, &alice);
    assert_eq!(view.funding_events.len(), 1);
    assert!(!view.trading_enabled);
    assert_eq!(view.chain_id, 84532);
    assert_eq!(view.funding_events[0].reason_code, "funding_settlement");
}

#[tokio::test]
async fn account_funding_endpoint_returns_empty_by_default() {
    let app = router(state());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/accounts/0x0000000000000000000000000000000000000abc/perps/funding")
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
    assert_eq!(body["chain_id"].as_u64(), Some(84532));
    assert_eq!(body["trading_enabled"].as_bool(), Some(false));
    assert_eq!(body["funding_events"].as_array().unwrap().len(), 0);
}

// =====================================================================
// F. Prefetch helper
// =====================================================================

#[tokio::test]
async fn prefetch_maps_per_configured_market() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let reader = InMemoryPerpFundingIndexReader::new().with_index(
        "ETH-PERP",
        FundingIndexRead {
            cumulative_index_1e18: ONE_1E18,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    );
    let map = prefetch_funding_indices(&cfg, &reader, now_ms()).await;
    assert_eq!(map.len(), 2);
    // ETH-PERP is present with a value; BTC-PERP is present with None
    // (in-memory reader defaults to ok=false for unseeded symbols).
    assert!(map["ETH-PERP"].is_some());
    assert!(map["BTC-PERP"].is_none());
}

#[tokio::test]
async fn stale_index_prefetches_as_none() {
    let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    let reader = InMemoryPerpFundingIndexReader::new().with_index(
        "ETH-PERP",
        FundingIndexRead {
            cumulative_index_1e18: ONE_1E18,
            updated_at_sec: 1, // ancient
            ok: true,
        },
    );
    let map = prefetch_funding_indices(&cfg, &reader, now_ms()).await;
    assert!(
        map["ETH-PERP"].is_none(),
        "stale updated_at_sec must render as unavailable"
    );
}

// =====================================================================
// G. Admin gate + fail-closed regression
// =====================================================================

#[tokio::test]
async fn admin_funding_tick_refuses_without_admin_token() {
    let app = router(state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/perps/funding/tick")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn public_perp_submit_still_fail_closed_after_funding_ships() {
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
