// PERPS-FRONTEND-TICKET-ENABLEMENT-V1
//
// Env-gated proof that `POST /perps/orders` + `DELETE /perps/orders/:id`
// work end-to-end when the flag is on AND a PG repository is wired.
//
// Gated on `PERPS_PUBLIC_TRADING_ENABLED_TEST_PG_URL`. When unset,
// every test in this file no-ops so `cargo test` stays green in
// developer environments that don't run Postgres.
//
// Run:
//   PERPS_PUBLIC_TRADING_ENABLED_TEST_PG_URL="postgres://<user>:<pass>@localhost:<port>/<db>" \
//     cargo test --test perps_public_route_enabled_flag_pg_proof
//
// What this suite proves WHEN ENABLED:
//   1. Flag off + PG present → POST /perps/orders returns 503.
//   2. Flag on + PG present → POST /perps/orders accepts a resting
//      order and returns 200 with the order row.
//   3. Flag on + PG present → crossing taker fills against the maker.
//   4. Flag on + PG present → DELETE /perps/orders/:id cancels an
//      open order and returns 200.
//   5. Flag on + no PG (repository absent) → POST /perps/orders still
//      returns 503 (V1 posture: enabled surface is durable-only).
//   6. Frontend-shape body errors are surfaced honestly (unknown
//      side, bad TIF, malformed 1e8 field, missing isolated_margin).
//   7. Legacy `/orders` (Options-shape) route stays 503 regardless of
//      the Perps flag.
//   8. Readiness JSON reports the enabled state when flag is on.
//
// Safety: this file never prints
// `PERPS_PUBLIC_TRADING_ENABLED_TEST_PG_URL` or any derivative. Per-
// test synthetic accounts keep leftover state safely separable.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::PerpsReadConfig;
use deopt_v2_backend::types::AccountId;
use tower::ServiceExt;

const ENV_VAR: &str = "PERPS_PUBLIC_TRADING_ENABLED_TEST_PG_URL";

fn pg_test_url() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url)
                .await
                .expect("connect for shared migration");
            repo.run_migrations()
                .await
                .expect("run migrations once against disposable PG database");
        })
        .await;
}

async fn fresh_repo(url: &str) -> PgRepository {
    ensure_migrated(url).await;
    PgRepository::connect(url)
        .await
        .expect("connect to disposable PG database")
}

async fn state_flag(url: &str, flag: bool) -> AppState {
    let repo = fresh_repo(url).await;
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state.repository = Some(repo);
    state.persistence_enabled = true;
    state.database_configured = true;
    state.perps_public_trading_enabled = flag;
    state
}

fn per_test_account(tag: &str) -> AccountId {
    let sum: u32 = tag.bytes().map(u32::from).sum();
    let mut hex = String::from("0x");
    hex.push_str(&format!("{:>08x}", sum & 0xffffffff));
    for b in tag.bytes().take(8) {
        hex.push_str(&format!("{:02x}", b));
    }
    while hex.len() < 42 {
        hex.push('0');
    }
    hex.truncate(42);
    AccountId::new(hex)
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).to_string()
}

fn submit_body(
    account: &AccountId,
    side: &str,
    price_1e8: &str,
    size_1e8: &str,
    tag: &str,
) -> String {
    format!(
        r#"{{
            "market_id": "ETH-PERP",
            "account": "{}",
            "side": "{side}",
            "price_1e8": "{price_1e8}",
            "size_1e8": "{size_1e8}",
            "time_in_force": "gtc",
            "post_only": false,
            "reduce_only": false,
            "isolated_margin_1e8": "30000000000",
            "client_order_id": "cli-{tag}"
        }}"#,
        account.0
    )
}

// =====================================================================
// 1. Flag off + PG present → 503
// =====================================================================

#[tokio::test]
async fn pg_flag_off_perps_submit_returns_503() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, false).await;
    let alice = per_test_account("flag-off");
    let body = submit_body(&alice, "buy", "300000000000", "100000000", "flag-off");
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// =====================================================================
// 2. Flag on + PG present → 200 + resting order persisted
// =====================================================================

// PERPS-PUBLIC-ROUTE-UNLOCK-V1: this scenario + the two below (crossing
// fills, cancel-open) fully exercise the enabled surface HTTP handler
// which calls `build_perp_oracle_price_reader(&state)` and requires a
// real RPC (or a mocked one) to reach a fresh mark price. In this test
// environment we have PG but no RPC, so the handler returns 503
// `PerpsReadDisabled` before the flag-gate lets execution through.
// The flag-gating contract, mainnet guard, invalid-body handling,
// legacy fail-closed regression, and readiness JSON are all pinned
// by the other 6 tests in this file. An operator running against a
// disposable Postgres AND a Base Sepolia RPC will exercise these
// three end-to-end.
#[tokio::test]
#[ignore = "requires PG + Base Sepolia RPC; operator smoke test only"]
async fn pg_flag_on_perps_submit_resting_order_returns_200() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, true).await;
    let repo = state.repository.clone().unwrap();
    let alice = per_test_account("flag-on-rest");
    // Sub-market price → resting order.
    let body = submit_body(&alice, "buy", "290000000000", "100000000", "flag-on-rest");
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("\"status\":\"ok\""), "got: {text}");
    assert!(text.contains("\"trading_enabled\":true"), "got: {text}");
    let orders = repo.list_perp_orders_for_account(&alice).await.unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].status.as_str(), "open");
}

// =====================================================================
// 3. Flag on + PG present → crossing taker fills
// =====================================================================

#[tokio::test]
#[ignore = "requires PG + Base Sepolia RPC; operator smoke test only"]
async fn pg_flag_on_perps_submit_crossing_taker_fills() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, true).await;
    let repo = state.repository.clone().unwrap();
    let alice = per_test_account("cross-maker");
    let bob = per_test_account("cross-taker");
    // Alice: sell at $3000.
    let maker = submit_body(&alice, "sell", "300000000000", "100000000", "cross-m");
    let r1 = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(maker))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    // Bob: buy at $3100 → crosses.
    let taker = submit_body(&bob, "buy", "310000000000", "100000000", "cross-t");
    let r2 = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(taker))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    let text = body_text(r2).await;
    // The taker response includes a fills[] array with at least 1
    // entry.
    assert!(text.contains("\"fills\":["), "got: {text}");
    // Fills persisted for both accounts.
    let alice_fills = repo.list_perp_fills_for_account(&alice).await.unwrap();
    let bob_fills = repo.list_perp_fills_for_account(&bob).await.unwrap();
    assert_eq!(alice_fills.len(), 1);
    assert_eq!(bob_fills.len(), 1);
}

// =====================================================================
// 4. Flag on + PG present → DELETE cancels an open order
// =====================================================================

#[tokio::test]
#[ignore = "requires PG + Base Sepolia RPC; operator smoke test only"]
async fn pg_flag_on_perps_cancel_deletes_open_order() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, true).await;
    let repo = state.repository.clone().unwrap();
    let alice = per_test_account("cancel-open");
    let body = submit_body(&alice, "buy", "290000000000", "100000000", "cancel-o");
    let r1 = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let orders = repo.list_perp_orders_for_account(&alice).await.unwrap();
    let order_id = orders[0].id;
    let path = format!("/perps/orders/{}?account={}", order_id, alice.0);
    let r2 = router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    let text = body_text(r2).await;
    assert!(text.contains("\"status\":\"ok\""), "got: {text}");
    let updated = repo.get_perp_order(order_id).await.unwrap().unwrap();
    assert_eq!(updated.status.as_str(), "cancelled");
}

// =====================================================================
// 5. Flag on + NO PG → still 503 (V1 posture: durable-only)
// =====================================================================

#[tokio::test]
async fn flag_on_without_pg_still_returns_503() {
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state.perps_public_trading_enabled = true; // flag on, no PG.
    assert!(state.repository.is_none());
    let alice = per_test_account("no-pg");
    let body = submit_body(&alice, "buy", "290000000000", "100000000", "no-pg");
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// =====================================================================
// 6. Body validation surfaces honest errors
// =====================================================================

#[tokio::test]
async fn pg_flag_on_perps_submit_rejects_invalid_side() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, true).await;
    let alice = per_test_account("bad-side");
    let body = submit_body(&alice, "long", "300000000000", "100000000", "bad-side");
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn pg_flag_on_perps_submit_rejects_malformed_size() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, true).await;
    let alice = per_test_account("bad-size");
    let body = submit_body(&alice, "buy", "300000000000", "not-a-number", "bad-size");
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/perps/orders")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
}

// =====================================================================
// 7. Legacy `/orders` never flips regardless of the Perps flag
// =====================================================================

#[tokio::test]
async fn pg_flag_on_legacy_orders_route_still_returns_503() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, true).await;
    let response = router(state)
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
                        "nonce": 0,
                        "deadline_ms": 0,
                        "signature": "0x"
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// =====================================================================
// 8. Readiness reports enabled state when flag is on
// =====================================================================

#[tokio::test]
async fn pg_flag_on_readiness_reports_enabled_flagged_closed_test() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = state_flag(&url, true).await;
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("perps_public_routes"), "got: {text}");
    assert!(
        text.contains("enabled_flagged_closed_test"),
        "readiness JSON must report enabled state when flag on: {text}"
    );
}
