//! PERPS-PG-EXECUTION-INTEGRATION-V1 — dispatcher + regression tests.
//!
//! **Scope note.** The project test matrix does not currently spin
//! Postgres up for Perp tests. The PG-backed execution flow
//! (`submit_perp_order_via_repository`, `cancel_perp_order_via_repository`)
//! is code-integrated and compile-checked, but end-to-end PG
//! integration tests are deferred until a PG test harness lands.
//!
//! What this file DOES cover:
//!   * Dispatcher correctness in the in-memory branch (repository
//!     absent → in-memory path exercised, PG path untouched).
//!   * Lifecycle emission in the dispatcher's in-memory branch.
//!   * Public Perps mutation routes remain fail-closed regardless
//!     of the new dispatcher (regression pin).
//!   * PG-backed functions are present and callable at compile time.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::public_ws::{LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    cancel_perp_order_via_state,
    price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead},
    submit_perp_order_via_state, PerpOrderSide, PerpOrderStatus, PerpTimeInForce, PerpsReadConfig,
    SubmitPerpOrderInput,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use tower::ServiceExt;

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

fn base_input(
    account: AccountId,
    side: PerpOrderSide,
    price: u128,
    size: u128,
) -> SubmitPerpOrderInput {
    SubmitPerpOrderInput {
        account,
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

// =====================================================================
// A. Dispatcher: in-memory branch is used when repository is absent
// =====================================================================

#[tokio::test]
async fn dispatcher_uses_in_memory_when_repository_is_absent() {
    let state = state();
    assert!(state.repository.is_none());
    let reader = fresh_price_reader();
    let outcome = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            addr("0x0000000000000000000000000000000000000aaa"),
            PerpOrderSide::Buy,
            PRICE_ETH_3000,
            ONE,
        ),
    )
    .await
    .unwrap();
    assert!(outcome.fills.is_empty());
    assert_eq!(outcome.order.status, PerpOrderStatus::Open);
    // In-memory store must have the row.
    let store = state.perp_order_store.lock().unwrap();
    let orders = store.list_orders_for_account(&addr("0x0000000000000000000000000000000000000aaa"));
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].id, outcome.order.id);
}

#[tokio::test]
async fn dispatcher_emits_lifecycle_from_in_memory_branch() {
    let state = state();
    let reader = fresh_price_reader();
    // Maker sells so bob's buy crosses.
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            addr("0x0000000000000000000000000000000000000aaa"),
            PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE,
        ),
    )
    .await
    .unwrap();
    let mut rx = state.lifecycle_events.subscribe();
    let outcome = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            addr("0x0000000000000000000000000000000000000bbb"),
            PerpOrderSide::Buy,
            PRICE_ETH_3100,
            ONE,
        ),
    )
    .await
    .unwrap();
    assert_eq!(outcome.fills.len(), 1);

    let events = drain(&mut rx);
    // Expect at least 1 PerpOrderUpdated + 2 PerpFillCreated + 2 PerpPositionUpdated
    let order_events = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpOrderUpdated { .. }))
        .count();
    let fill_events = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpFillCreated { .. }))
        .count();
    let position_events = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpPositionUpdated { .. }))
        .count();
    assert!(order_events >= 1);
    assert_eq!(fill_events, 2);
    assert!(position_events >= 2);
}

#[tokio::test]
async fn dispatcher_cancel_uses_in_memory_when_repository_is_absent() {
    let state = state();
    let reader = fresh_price_reader();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let outcome = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(alice.clone(), PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
    )
    .await
    .unwrap();
    let cancelled = cancel_perp_order_via_state(&state, outcome.order.id, &alice)
        .await
        .unwrap();
    assert_eq!(cancelled.status, PerpOrderStatus::Cancelled);
    assert_eq!(
        cancelled.terminal_reason_code.as_deref(),
        Some("user_cancelled")
    );
}

// =====================================================================
// B. Public routes still fail-closed after PG execution ships
// =====================================================================

#[tokio::test]
async fn public_perp_submit_still_fail_closed_after_pg_execution_dispatcher_lands() {
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
// C. PG-backed functions are present + linkable at compile time.
//    We can't call them (no PG fixture) but the linker proves they
//    exist with the right signature.
// =====================================================================

#[test]
fn pg_backed_execution_functions_are_reachable_by_name() {
    // Zero-cost proof that the PG-backed entrypoints exist and are
    // exported from `crate::perps::`. Using them as `_ = FN_NAME`
    // takes their address, which forces the compiler to instantiate
    // and export them. Compilation success is the assertion.
    #[allow(clippy::no_effect_underscore_binding)]
    let _submit =
        deopt_v2_backend::perps::submit_perp_order_via_repository::<InMemoryPerpOraclePriceReader>;
    #[allow(clippy::no_effect_underscore_binding)]
    let _cancel = deopt_v2_backend::perps::cancel_perp_order_via_repository;
}
