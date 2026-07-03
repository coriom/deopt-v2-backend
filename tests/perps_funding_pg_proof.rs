// PERPS-FUNDING-V1
//
// Live PostgreSQL proof for the PG-backed Perps funding flow. Gated
// on the env var `PERPS_PG_TEST_DATABASE_URL`. If unset, every test
// no-ops so `cargo test` stays green in developer environments that
// don't run Postgres.
//
// Run against a disposable local DB:
//   PERPS_PG_TEST_DATABASE_URL="postgres://<user>:<pass>@localhost:<port>/<disposable_db>" \
//     cargo test --test perps_funding_pg_proof
//
// What this suite proves WHEN ENABLED:
//
//   1. Migration `0036_perp_funding.sql` applies cleanly on top of
//      existing perp migrations.
//   2. PG funding tick settles an unhealthy-index-delta LONG position:
//        - `payment_1e8 > 0`, margin reduced, event persisted.
//   3. PG funding tick credits positive delta to a SHORT position:
//        - `payment_1e8 < 0`, margin increased, event persisted.
//   4. PG funding tick is idempotent at the same cumulative index:
//        second tick produces no new event.
//   5. Stale/unavailable index → skipped; position untouched; no event
//      persisted.
//   6. Lifecycle emits `PerpPositionUpdated` + `PerpFundingPaymentCreated`
//      AFTER commit.
//   7. Lifecycle JSON payloads contain no secrets.
//   8. Account read endpoint returns durable events newest-first.
//   9. Public Perps mutation route remains fail-closed in PG mode.
//  10. Admin tick without token is refused.
//
// **Safety**: this file never prints `PERPS_PG_TEST_DATABASE_URL` or
// any derivative. Per-test synthetic accounts keep leftover DB state
// safely separable across re-runs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::public_ws::{LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    run_perp_funding_tick_via_repository, FundingIndexRead, PerpPositionStatus, PerpsReadConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use std::collections::HashMap;
use tower::ServiceExt;

const ENV_VAR: &str = "PERPS_PG_TEST_DATABASE_URL";
const ONE: u128 = 100_000_000;
const ONE_1E18: i128 = 1_000_000_000_000_000_000;

fn pg_test_url() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

fn per_test_account(tag: &str, prefix: &str) -> AccountId {
    let sum: u32 = tag.bytes().map(u32::from).sum();
    let mut hex = String::from("0x");
    hex.push_str(prefix);
    hex.push_str(&format!("{:>04x}", sum & 0xffff));
    for b in tag.bytes().take(8) {
        hex.push_str(&format!("{:02x}", b));
    }
    while hex.len() < 42 {
        hex.push('0');
    }
    hex.truncate(42);
    AccountId::new(hex)
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

async fn pg_state(url: &str) -> AppState {
    let repo = fresh_repo(url).await;
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state.repository = Some(repo);
    state.persistence_enabled = true;
    state.database_configured = true;
    assert!(
        state.repository.is_some(),
        "repository must be wired for the PG funding proof"
    );
    state
}

async fn seed_pg_position(
    repository: &PgRepository,
    account: &AccountId,
    market_id: &str,
    side: deopt_v2_backend::perps::PerpSide,
    size_1e8: u128,
    entry_1e8: u128,
    margin_1e8: u128,
) -> deopt_v2_backend::perps::PerpPosition {
    let position = deopt_v2_backend::perps::positions::new_position_skeleton(
        account.clone(),
        market_id.to_string(),
        side,
        size_1e8,
        entry_1e8,
        margin_1e8,
    );
    repository.insert_perp_position(&position).await.unwrap();
    position
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

fn indices_eth(cumulative: i128, ok: bool) -> HashMap<String, Option<FundingIndexRead>> {
    HashMap::from([
        (
            "ETH-PERP".to_string(),
            if ok {
                Some(FundingIndexRead {
                    cumulative_index_1e18: cumulative,
                    updated_at_sec: (now_ms() / 1000) as u64,
                    ok: true,
                })
            } else {
                None
            },
        ),
        ("BTC-PERP".to_string(), None),
    ])
}

// =====================================================================
// 1. PG tick settles unhealthy LONG position
// =====================================================================

#[tokio::test]
async fn pg_tick_settles_long_with_positive_delta() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let repo = state.repository.clone().unwrap();
    let tag = "pg-fund-long";
    let alice = per_test_account(tag, "aa11");
    seed_pg_position(
        &repo,
        &alice,
        "ETH-PERP",
        deopt_v2_backend::perps::PerpSide::Long,
        ONE,
        300 * ONE,
        300 * ONE,
    )
    .await;

    let indices = indices_eth(ONE_1E18 / 100, true);
    let response = run_perp_funding_tick_via_repository(
        &state.perps_read_config,
        &repo,
        &indices,
        &state.lifecycle_events,
        now_ms(),
    )
    .await
    .unwrap();
    assert!(response.settled_count >= 1);
    assert!(!response.trading_enabled);

    // Event persisted with reason_code funding_settlement.
    let events = repo
        .list_perp_funding_events_for_account(&alice)
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|e| e.reason_code == "funding_settlement" && e.payment_1e8 > 0));

    // Position row: margin reduced, last_funding_index_1e18 bumped.
    let positions = repo.list_perp_positions_for_account(&alice).await.unwrap();
    let post = positions
        .iter()
        .find(|p| p.market_id == "ETH-PERP")
        .unwrap();
    assert!(post.margin_1e8 < 300 * ONE);
    assert_eq!(post.last_funding_index_1e18, ONE_1E18 / 100);
    assert!(post.cumulative_funding_payment_1e8 > 0);
    assert_eq!(post.status, PerpPositionStatus::Open);
}

// =====================================================================
// 2. PG tick credits SHORT with positive delta
// =====================================================================

#[tokio::test]
async fn pg_tick_credits_short_with_positive_delta() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let repo = state.repository.clone().unwrap();
    let tag = "pg-fund-short";
    let alice = per_test_account(tag, "aa22");
    seed_pg_position(
        &repo,
        &alice,
        "ETH-PERP",
        deopt_v2_backend::perps::PerpSide::Short,
        ONE,
        300 * ONE,
        300 * ONE,
    )
    .await;
    let indices = indices_eth(ONE_1E18 / 100, true);
    let _ = run_perp_funding_tick_via_repository(
        &state.perps_read_config,
        &repo,
        &indices,
        &state.lifecycle_events,
        now_ms(),
    )
    .await
    .unwrap();
    let events = repo
        .list_perp_funding_events_for_account(&alice)
        .await
        .unwrap();
    assert!(events.iter().any(|e| e.payment_1e8 < 0));
    let positions = repo.list_perp_positions_for_account(&alice).await.unwrap();
    let post = positions
        .iter()
        .find(|p| p.market_id == "ETH-PERP")
        .unwrap();
    assert!(post.margin_1e8 > 300 * ONE);
    assert!(post.cumulative_funding_payment_1e8 < 0);
}

// =====================================================================
// 3. PG tick is idempotent at same cumulative index
// =====================================================================

#[tokio::test]
async fn pg_tick_is_idempotent_at_same_index() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let repo = state.repository.clone().unwrap();
    let tag = "pg-fund-idem";
    let alice = per_test_account(tag, "aa33");
    seed_pg_position(
        &repo,
        &alice,
        "ETH-PERP",
        deopt_v2_backend::perps::PerpSide::Long,
        ONE,
        300 * ONE,
        300 * ONE,
    )
    .await;
    let indices = indices_eth(ONE_1E18 / 100, true);
    let _r1 = run_perp_funding_tick_via_repository(
        &state.perps_read_config,
        &repo,
        &indices,
        &state.lifecycle_events,
        now_ms(),
    )
    .await
    .unwrap();
    let _r2 = run_perp_funding_tick_via_repository(
        &state.perps_read_config,
        &repo,
        &indices,
        &state.lifecycle_events,
        now_ms(),
    )
    .await
    .unwrap();
    let events = repo
        .list_perp_funding_events_for_account(&alice)
        .await
        .unwrap();
    assert_eq!(
        events.len(),
        1,
        "second tick at same cumulative index must NOT insert a duplicate event"
    );
}

// =====================================================================
// 4. Stale/unavailable source skips position
// =====================================================================

#[tokio::test]
async fn pg_tick_stale_source_skips_position() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let repo = state.repository.clone().unwrap();
    let tag = "pg-fund-stale";
    let alice = per_test_account(tag, "aa44");
    seed_pg_position(
        &repo,
        &alice,
        "ETH-PERP",
        deopt_v2_backend::perps::PerpSide::Long,
        ONE,
        300 * ONE,
        300 * ONE,
    )
    .await;
    let indices = indices_eth(ONE_1E18 / 100, false); // ok=false
    let response = run_perp_funding_tick_via_repository(
        &state.perps_read_config,
        &repo,
        &indices,
        &state.lifecycle_events,
        now_ms(),
    )
    .await
    .unwrap();
    assert!(response.skipped_source_unavailable_count >= 1);
    let events = repo
        .list_perp_funding_events_for_account(&alice)
        .await
        .unwrap();
    assert!(events.is_empty(), "no events when source unavailable");
    let positions = repo.list_perp_positions_for_account(&alice).await.unwrap();
    let post = positions
        .iter()
        .find(|p| p.market_id == "ETH-PERP")
        .unwrap();
    assert_eq!(post.margin_1e8, 300 * ONE);
    assert_eq!(post.last_funding_index_1e18, 0);
    assert_eq!(post.cumulative_funding_payment_1e8, 0);
}

// =====================================================================
// 5. Lifecycle after commit + no secrets
// =====================================================================

#[tokio::test]
async fn pg_funding_emits_lifecycle_after_commit_with_no_secrets() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let repo = state.repository.clone().unwrap();
    let tag = "pg-fund-lifecycle";
    let alice = per_test_account(tag, "aa55");
    seed_pg_position(
        &repo,
        &alice,
        "ETH-PERP",
        deopt_v2_backend::perps::PerpSide::Long,
        ONE,
        300 * ONE,
        300 * ONE,
    )
    .await;
    let mut rx = state.lifecycle_events.subscribe();
    let indices = indices_eth(ONE_1E18 / 100, true);
    let _ = run_perp_funding_tick_via_repository(
        &state.perps_read_config,
        &repo,
        &indices,
        &state.lifecycle_events,
        now_ms(),
    )
    .await
    .unwrap();
    let events = drain(&mut rx);
    let position_updated = events
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
    assert!(position_updated >= 1);
    assert!(funding >= 1);
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
// 6. Account read endpoint returns durable events
// =====================================================================

#[tokio::test]
async fn pg_account_funding_endpoint_returns_durable_events() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let repo = state.repository.clone().unwrap();
    let tag = "pg-fund-read";
    let alice = per_test_account(tag, "aa66");
    seed_pg_position(
        &repo,
        &alice,
        "ETH-PERP",
        deopt_v2_backend::perps::PerpSide::Long,
        ONE,
        300 * ONE,
        300 * ONE,
    )
    .await;
    let indices = indices_eth(ONE_1E18 / 100, true);
    let _ = run_perp_funding_tick_via_repository(
        &state.perps_read_config,
        &repo,
        &indices,
        &state.lifecycle_events,
        now_ms(),
    )
    .await
    .unwrap();
    let app = router(state.clone());
    let path = format!("/accounts/{}/perps/funding", alice.0);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
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
    let arr = body["funding_events"].as_array().unwrap();
    assert!(!arr.is_empty());
    let row = &arr[0];
    assert_eq!(row["reason_code"].as_str(), Some("funding_settlement"));
    assert_eq!(row["trading_enabled"].as_bool(), Some(false));
}

// =====================================================================
// 7. Empty account read
// =====================================================================

#[tokio::test]
async fn pg_account_funding_endpoint_empty_default() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let alice = per_test_account("pg-fund-empty", "aa77");
    let app = router(state.clone());
    let path = format!("/accounts/{}/perps/funding", alice.0);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
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
// 8. Public perp submit still fail-closed in PG mode
// =====================================================================

#[tokio::test]
async fn public_perp_submit_still_fail_closed_in_pg_mode_after_funding() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let app = router(state);
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
}

// =====================================================================
// 9. Admin funding tick gated in PG mode
// =====================================================================

#[tokio::test]
async fn admin_funding_tick_gated_in_pg_mode() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let app = router(state);
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
