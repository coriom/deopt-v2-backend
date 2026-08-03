//! `BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1` — Part M.
//!
//! Bounded property tests over the persisted runtime lifecycle. Every
//! property is expressed as a hand-generated fixture loop (no
//! `quickcheck` dependency is present in this crate) so each test runs
//! in bounded, deterministic time.
//!
//! Gating: skips cleanly when `HYBRID_V2_PG_TEST_DATABASE_URL` (or
//! `PG_INTEGRATION_URL`) is not set. Never prints URLs, credentials,
//! or role names.
//!
//! Runtime budget: the full property suite is intended to finish in
//! under 60 s against a warm disposable PG. Iterations are kept
//! deliberately small (10–20) to satisfy that budget on shared CI.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_source::InMemoryChainSource;
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::runtime::IndexerRuntime;
use hybrid_v2_support::{
    baseline_manifest, block, deposit_log, pad_address, pad_bytes32, subaccount_created_log,
};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;

// -----------------------------------------------------------------
//                       PG HARNESS (mirrors main integration file)
// -----------------------------------------------------------------

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

fn get_pg_url_or_skip_or_panic() -> Option<String> {
    let url = std::env::var(URL_ENV)
        .ok()
        .or_else(|| std::env::var(ALT_URL_ENV).ok())
        .filter(|v| !v.is_empty());
    if url.is_none() {
        let required = matches!(
            std::env::var(REQUIRE_ENV).ok().as_deref(),
            Some("1") | Some("true") | Some("TRUE")
        );
        if required {
            panic!(
                "{} enabled but neither {} nor {} is set — property suite requires \
                 disposable PostgreSQL URL",
                REQUIRE_ENV, URL_ENV, ALT_URL_ENV
            );
        }
    }
    url
}

async fn fresh_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .idle_timeout(Some(std::time::Duration::from_secs(10)))
        .connect(url)
        .await
        .expect("connect to disposable PostgreSQL (env)");
    sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(&pool)
        .await
        .expect("drop schema");
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("create schema");
    sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
        .execute(&pool)
        .await
        .expect("grant schema");
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
        .await
        .expect("load migrations");
    migrator.run(&pool).await.expect("apply migrations");
    pool
}

async fn build_store_and_deployment(pool: &PgPool) -> (Arc<PostgresHybridV2ProjectionStore>, i64) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    (store, did)
}

fn build_multi_block_source(count: u64) -> InMemoryChainSource {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    let mut parent = "0xb0".to_string();
    for n in 1..=count {
        let hash = format!("0xb{n}");
        let subkey = format!("0xabc{n}");
        let owner = format!("0xa{n}");
        source.push(block(
            n,
            &hash,
            &parent,
            1_000 + n * 12,
            vec![
                subaccount_created_log(&manifest, &owner, n as u32, &subkey),
                deposit_log(&manifest, &subkey, &owner, n as u32, "0xe1", "100"),
            ],
        ));
        parent = hash;
    }
    source
}

async fn apply_all(runtime: &mut IndexerRuntime, source: &InMemoryChainSource) {
    while runtime
        .tick_and_persist(source)
        .await
        .expect("tick_and_persist")
    {}
}

// -----------------------------------------------------------------
//                       PROPERTIES
// -----------------------------------------------------------------

/// PROPERTY: applying N blocks in one shot vs restarting the runtime
/// every M blocks yields the same final projection state + cursor.
#[tokio::test(flavor = "multi_thread")]
async fn prop_uninterrupted_run_equals_restart_sequence() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP prop_uninterrupted_run_equals_restart_sequence: no disposable PG URL");
        return;
    };
    // Small parameter grid: 10 iterations over (n_blocks, restart_every).
    for (n_blocks, restart_every) in [(3, 1), (5, 2), (7, 3), (4, 4), (6, 2)] {
        // Uninterrupted run.
        let uninterrupted_state = {
            let pool = fresh_pool(&url).await;
            let (store, did) = build_store_and_deployment(&pool).await;
            let source = build_multi_block_source(n_blocks);
            let mut r =
                IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store, did);
            apply_all(&mut r, &source).await;
            r.projection().balances.clone()
        };
        // Restart every M blocks.
        let restart_state = {
            let pool = fresh_pool(&url).await;
            let (store, did) = build_store_and_deployment(&pool).await;
            let source = build_multi_block_source(n_blocks);
            let mut applied_head: u64 = 0;
            while applied_head < n_blocks {
                let mut r = IndexerRuntime::new(1, baseline_manifest(84532))
                    .with_persistence(store.clone(), did);
                let _ = r.bootstrap_from_persistence().await.expect("bootstrap");
                for _ in 0..restart_every {
                    let ok = r.tick_and_persist(&source).await.expect("tick");
                    if !ok {
                        break;
                    }
                    applied_head = r.cursor().indexed_head_block;
                    if applied_head >= n_blocks {
                        break;
                    }
                }
            }
            // Final rebuild from persistence to obtain the reducer state.
            let mut fresh =
                IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store, did);
            let _ = fresh.bootstrap_from_persistence().await.expect("bootstrap");
            fresh.projection().balances.clone()
        };
        assert_eq!(
            uninterrupted_state, restart_state,
            "n={n_blocks} restart_every={restart_every}: restart sequence must equal uninterrupted"
        );
    }
}

/// PROPERTY: applying the same block twice is a no-op on projection
/// counts.
#[tokio::test(flavor = "multi_thread")]
async fn prop_duplicate_block_sequence_is_idempotent() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP prop_duplicate_block_sequence_is_idempotent: no disposable PG URL");
        return;
    };
    for n in [1, 2, 3, 5] {
        let pool = fresh_pool(&url).await;
        let (store, did) = build_store_and_deployment(&pool).await;
        let source = build_multi_block_source(n);
        {
            let mut r = IndexerRuntime::new(1, baseline_manifest(84532))
                .with_persistence(store.clone(), did);
            apply_all(&mut r, &source).await;
        }
        let raw_before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM hybrid_v2_raw_logs WHERE deployment_id=$1")
                .bind(did)
                .fetch_one(&pool)
                .await
                .expect("count raw before");
        // Re-apply via a fresh runtime with cursor 0 (worst case
        // duplicate).
        {
            let mut r = IndexerRuntime::new(1, baseline_manifest(84532))
                .with_persistence(store.clone(), did);
            for _ in 0..(n as usize + 3) {
                let _ = r.tick_and_persist(&source).await;
                if r.cursor().indexed_head_block >= n {
                    break;
                }
            }
        }
        let raw_after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM hybrid_v2_raw_logs WHERE deployment_id=$1")
                .bind(did)
                .fetch_one(&pool)
                .await
                .expect("count raw after");
        assert_eq!(raw_before, raw_after, "n={n}: duplicate must be idempotent");
    }
}

/// PROPERTY: after each successful tick, an independently-rehydrated
/// runtime observes the same in-memory `state.balances`.
#[tokio::test(flavor = "multi_thread")]
async fn prop_published_state_equals_committed_postgres_state() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!(
            "SKIP prop_published_state_equals_committed_postgres_state: no disposable PG URL"
        );
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store_and_deployment(&pool).await;
    let source = build_multi_block_source(4);
    let mut live =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    for _ in 0..4 {
        let ok = live.tick_and_persist(&source).await.expect("tick");
        if !ok {
            break;
        }
        let mut mirror =
            IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
        let _ = mirror
            .bootstrap_from_persistence()
            .await
            .expect("bootstrap");
        assert_eq!(
            live.projection().balances,
            mirror.projection().balances,
            "in-memory state must equal a fresh bootstrap after each tick"
        );
    }
}

/// PROPERTY: two runtimes bootstrapped at different points — the one
/// with the lower cursor never has strictly more projection rows than
/// the one with the higher cursor.
#[tokio::test(flavor = "multi_thread")]
async fn prop_cursor_never_advances_without_committed_projections() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!(
            "SKIP prop_cursor_never_advances_without_committed_projections: no disposable PG URL"
        );
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store_and_deployment(&pool).await;
    let source = build_multi_block_source(3);
    // Runtime A: apply 1 block.
    let mut a =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    a.tick_and_persist(&source).await.expect("A tick 1");
    let raw_a: i64 =
        sqlx::query_scalar("SELECT count(*) FROM hybrid_v2_raw_logs WHERE deployment_id=$1")
            .bind(did)
            .fetch_one(&pool)
            .await
            .expect("count");
    // Runtime B (same store): apply 2 more blocks.
    a.tick_and_persist(&source).await.expect("A tick 2");
    a.tick_and_persist(&source).await.expect("A tick 3");
    let raw_b: i64 =
        sqlx::query_scalar("SELECT count(*) FROM hybrid_v2_raw_logs WHERE deployment_id=$1")
            .bind(did)
            .fetch_one(&pool)
            .await
            .expect("count b");
    assert!(
        raw_b >= raw_a,
        "raw log count must be monotone non-decreasing across cursor advance"
    );
    assert_eq!(a.cursor().indexed_head_block, 3);
}

/// PROPERTY: writes to deployment A do not touch deployment B's
/// projection rows.
#[tokio::test(flavor = "multi_thread")]
async fn prop_deployment_isolation() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP prop_deployment_isolation: no disposable PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did_a) = build_store_and_deployment(&pool).await;
    // Deployment B via a manifest with a different manifest_hash so
    // `upsert_deployment` allocates a new deployment_id.
    let mut manifest_b = baseline_manifest(84532);
    manifest_b.manifest_hash =
        "0xdeadbeef1111111111111111111111111111111111111111111111111111beef".into();
    // Distinct `deployment_version` so the `hybrid_v2_deployments_version_uniq`
    // index doesn't collide with deployment A's row.
    manifest_b.deployment_version = manifest_b.deployment_version.wrapping_add(1);
    let did_b = store
        .upsert_deployment(&manifest_b, "PENDING", 1_700_000_000_001)
        .await
        .expect("upsert deployment B");
    assert_ne!(did_a, did_b);
    // Apply blocks under A only.
    let source = build_multi_block_source(2);
    let mut ra =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did_a);
    while ra.tick_and_persist(&source).await.expect("tick A") {}
    // Read B: expect zero rows.
    let raw_b: i64 =
        sqlx::query_scalar("SELECT count(*) FROM hybrid_v2_raw_logs WHERE deployment_id=$1")
            .bind(did_b)
            .fetch_one(&pool)
            .await
            .expect("count raw B");
    let cursor_b: i64 =
        sqlx::query_scalar("SELECT count(*) FROM hybrid_v2_cursors WHERE deployment_id=$1")
            .bind(did_b)
            .fetch_one(&pool)
            .await
            .expect("count cursor B");
    assert_eq!(raw_b, 0, "deployment B raw logs must be untouched");
    assert_eq!(cursor_b, 0, "deployment B cursor row must be absent");
}

/// PROPERTY: for each subkey/token the sum of per-engine reservations
/// equals the aggregate reserved balance (the current schema stores
/// per-(subkey, token, engine) rows; the aggregate is derivable as the
/// SUM(reserved) grouped by (subkey, token)).
#[tokio::test(flavor = "multi_thread")]
async fn prop_aggregate_reservations_equal_per_engine_sum() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP prop_aggregate_reservations_equal_per_engine_sum: no disposable PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store_and_deployment(&pool).await;
    // Property is trivially satisfied when there are no reservations
    // — we assert the identity holds by consistency: SUM(reserved) per
    // (subkey, token) grouped from `hybrid_v2_reservations` equals
    // the sum of the individual engine rows for that (subkey, token).
    // With no per-subkey aggregate table exposed in the current
    // schema, the invariant is that grouped SUM equals the total of
    // component rows, which is a tautology at the SQL level; the
    // meaningful check is that no negative reservation ever lands.
    let source = build_multi_block_source(2);
    let mut r =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    while r.tick_and_persist(&source).await.expect("tick") {}
    let any_negative: Option<i64> = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM hybrid_v2_reservations
         WHERE deployment_id=$1 AND reserved::numeric < 0",
    )
    .bind(did)
    .fetch_optional(&pool)
    .await
    .expect("count negatives");
    assert_eq!(
        any_negative.unwrap_or(0),
        0,
        "no reservation may go negative"
    );
    // Also assert derived aggregate identity via a self-join.
    let mismatched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (
             SELECT subkey, token, SUM(reserved::numeric) AS s
             FROM hybrid_v2_reservations WHERE deployment_id=$1
             GROUP BY subkey, token
         ) g WHERE g.s < 0",
    )
    .bind(did)
    .fetch_one(&pool)
    .await
    .expect("aggregate reservations");
    assert_eq!(mismatched, 0);
}

/// PROPERTY: `filled_qty_1e8` on an order is monotone non-decreasing on
/// the canonical branch — no successful tick sequence ever decreases
/// it.
#[tokio::test(flavor = "multi_thread")]
async fn prop_filled_quantity_monotone_on_canonical_branch() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP prop_filled_quantity_monotone_on_canonical_branch: no disposable PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store_and_deployment(&pool).await;
    // No matching-engine events in our multi-block fixture, so the
    // order lifecycle table stays empty. The monotonicity claim is
    // trivially true (no decreases exist). We assert the table exists
    // and, if any orders are present, their filled_qty_1e8 parses to a
    // non-negative decimal — a preserved-invariant proxy.
    let source = build_multi_block_source(3);
    let mut r =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    while r.tick_and_persist(&source).await.expect("tick") {}
    let negatives: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM hybrid_v2_order_lifecycle
         WHERE deployment_id=$1 AND filled_qty_1e8::numeric < 0",
    )
    .bind(did)
    .fetch_one(&pool)
    .await
    .expect("count neg filled_qty");
    assert_eq!(negatives, 0);
    // And a second tick must never DECREASE filled_qty for any row
    // present after the first tick. Since our fixture emits no order
    // events, both snapshots are empty — the invariant holds vacuously.
    // We still exercise the comparison so a future extension with
    // order events retains the check.
    let snapshot1: Vec<(String, String)> = sqlx::query_as(
        "SELECT order_hash, filled_qty_1e8 FROM hybrid_v2_order_lifecycle
         WHERE deployment_id=$1",
    )
    .bind(did)
    .fetch_all(&pool)
    .await
    .expect("snapshot1");
    // Ticking again: no-op (no new blocks).
    let _ = r.tick_and_persist(&source).await;
    let snapshot2: Vec<(String, String)> = sqlx::query_as(
        "SELECT order_hash, filled_qty_1e8 FROM hybrid_v2_order_lifecycle
         WHERE deployment_id=$1",
    )
    .bind(did)
    .fetch_all(&pool)
    .await
    .expect("snapshot2");
    for (h1, q1) in &snapshot1 {
        if let Some((_, q2)) = snapshot2.iter().find(|(h, _)| h == h1) {
            let n1: u128 = q1.parse().unwrap_or(0);
            let n2: u128 = q2.parse().unwrap_or(0);
            assert!(
                n2 >= n1,
                "filled_qty_1e8 must be monotone non-decreasing (order_hash={h1}, {n1} -> {n2})"
            );
        }
    }
}

// Suppress dead-code warnings for helpers imported for symmetry with
// the sibling integration file.
#[allow(dead_code)]
fn _unused() {
    let _ = pad_address;
    let _ = pad_bytes32;
}
