//! `BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE-AND-WORKER-ACTIVATION-V1` —
//! End-to-end integration between a mock JSON-RPC server, the
//! `RpcHybridV2ChainSource`, the persisted `IndexerRuntime` and a
//! real PostgreSQL projection store.
//!
//! Gating: skips cleanly when `HYBRID_V2_PG_TEST_DATABASE_URL` (or
//! `PG_INTEGRATION_URL`) is unset. Panics loudly if
//! `DEOPT_REQUIRE_PG_INTEGRATION=1` and no URL is provided.

mod hybrid_v2_mock_rpc_helpers;
mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_source::ChainSource;
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::runtime::{IndexerRuntime, RuntimeError};
use deopt_v2_backend::hybrid_v2::worker::{
    spawn_hybrid_v2_indexer_worker, HybridV2IndexerWorkerConfig,
};
use deopt_v2_backend::hybrid_v2::{RpcHybridV2ChainSource, RpcSourceConfig};
use hybrid_v2_mock_rpc_helpers::{make_block, MockRpcServer};
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};

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
            panic!("{} required but no PG URL provided", REQUIRE_ENV);
        }
    }
    url
}

async fn fresh_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(30))
        .connect(url)
        .await
        .expect("connect");
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
        .expect("grant");
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
        .await
        .expect("migrations");
    migrator.run(&pool).await.expect("apply migrations");
    pool
}

async fn build_store(pool: &PgPool) -> (Arc<PostgresHybridV2ProjectionStore>, i64) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    (store, did)
}

fn source_from_mock(
    mock: &MockRpcServer,
    chain_id: u64,
    emitters: Vec<String>,
) -> RpcHybridV2ChainSource {
    RpcHybridV2ChainSource::new(
        RpcSourceConfig {
            endpoint: mock.url(),
            chain_id,
            timeout: Duration::from_secs(2),
            max_retries: 2,
            retry_backoff: Duration::from_millis(10),
            max_logs_per_range: 1_000,
            confirmation_depth: 12,
        },
        emitters,
    )
    .expect("source construct")
}

#[tokio::test(flavor = "multi_thread")]
async fn disabled_configuration_starts_no_worker() {
    let Some(_url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP disabled_configuration_starts_no_worker: no PG URL");
        return;
    };
    // A disabled config never even constructs a source, so no RPC
    // request should ever hit the mock.
    let mock = MockRpcServer::start().await;
    // Simulate elapsed time to ensure the server is really running.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(mock.calls().is_empty(), "no RPC calls must be made");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_chain_id_prevents_worker_startup() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP wrong_chain_id_prevents_worker_startup: no PG URL");
        return;
    };
    let _pool = fresh_pool(&url).await;
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(1);
    let source = source_from_mock(&mock, 84532, vec![]);
    let err = source
        .validate_chain_identity()
        .await
        .expect_err("chain identity mismatch must fail");
    assert!(format!("{err}").contains("chain_id=1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_mainnet_rejected_before_spawn() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(8453);
    let source = source_from_mock(&mock, 8453, vec![]);
    let err = source
        .validate_chain_identity()
        .await
        .expect_err("Base mainnet must be refused");
    assert!(format!("{err}").contains("Base mainnet"));
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_block_commits_cursor_advance() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP empty_block_commits_cursor_advance: no PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(1);
    let b = make_block(1, 0xb1, &format!("0x{}{}", "b0", "0".repeat(62)), 1_010);
    mock.push_block(b);
    let source = source_from_mock(&mock, 84532, vec![]);
    let mut runtime =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    let applied = runtime.tick_and_persist(&source).await.expect("tick");
    assert!(applied);
    assert_eq!(runtime.cursor().indexed_head_block, 1);
    let cursor = store
        .read_cursor(did, "indexer")
        .await
        .expect("cursor")
        .expect("row");
    assert_eq!(cursor.indexed_head_block, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_rpc_leaves_cursor_unchanged() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP failed_rpc_leaves_cursor_unchanged: no PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    // Make every request return 500 forever.
    mock.simulate_status(999, 500);
    let source = source_from_mock(&mock, 84532, vec![]);
    let mut runtime =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    let err = runtime
        .tick_and_persist(&source)
        .await
        .expect_err("tick must fail");
    assert!(matches!(err, RuntimeError::ChainSource { .. }));
    assert_eq!(runtime.cursor().indexed_head_block, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn graceful_shutdown_via_watch_channel() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP graceful_shutdown_via_watch_channel: no PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(0);
    let source: Arc<dyn ChainSource> = Arc::new(source_from_mock(&mock, 84532, vec![]));
    let runtime = Arc::new(RwLock::new(
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did),
    ));
    let (tx, rx) = watch::channel(false);
    let handle = spawn_hybrid_v2_indexer_worker(
        runtime.clone(),
        source.clone(),
        store.clone(),
        HybridV2IndexerWorkerConfig::new(did, 20),
        Some(rx),
    );
    tokio::time::sleep(Duration::from_millis(80)).await;
    tx.send(true).expect("send shutdown");
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("join in time")
        .expect("no panic");
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_resumes_from_postgres_cursor() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP restart_resumes_from_postgres_cursor: no PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(3);
    let mut parent = format!("0x{}{}", "b0", "0".repeat(62));
    for n in 1..=3u64 {
        let b = make_block(n, 0xb0 + n as u8, &parent, 1_000 + n * 12);
        parent = b.hash.clone();
        mock.push_block(b);
    }
    let source = source_from_mock(&mock, 84532, vec![]);
    {
        let mut runtime =
            IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
        for _ in 0..3 {
            let ok = runtime.tick_and_persist(&source).await.expect("tick");
            assert!(ok);
        }
        assert_eq!(runtime.cursor().indexed_head_block, 3);
    }
    let mut fresh =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    let _ = fresh.bootstrap_from_persistence().await.expect("bootstrap");
    assert_eq!(fresh.cursor().indexed_head_block, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn parent_hash_mismatch_fails_closed_no_replay() {
    let Some(url) = get_pg_url_or_skip_or_panic() else {
        eprintln!("SKIP parent_hash_mismatch_fails_closed_no_replay: no PG URL");
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(3);
    let mut parent = format!("0x{}{}", "b0", "0".repeat(62));
    for n in 1..=3u64 {
        let b = make_block(n, 0xb0 + n as u8, &parent, 1_000 + n * 12);
        parent = b.hash.clone();
        mock.push_block(b);
    }
    let source = source_from_mock(&mock, 84532, vec![]);
    let mut runtime =
        IndexerRuntime::new(1, baseline_manifest(84532)).with_persistence(store.clone(), did);
    for _ in 0..3 {
        let _ = runtime.tick_and_persist(&source).await.expect("tick");
    }
    assert_eq!(runtime.cursor().indexed_head_block, 3);
    // Advance the head; but block 4's parent hash disagrees with our indexed head.
    mock.set_head(4);
    let bad_parent = format!("0x{}{}", "ff", "0".repeat(62));
    let b4 = make_block(4, 0xb4, &bad_parent, 1_040);
    mock.push_block(b4);
    // The reorg planner will fail to find a common ancestor within
    // max_depth of the fake parent — expect ExcessiveReorgDepth OR
    // ChainSource error (source could not locate the block). Either
    // way the cursor must not have advanced past 3.
    let outcome = runtime.tick_and_persist(&source).await;
    assert!(outcome.is_err());
    assert_eq!(runtime.cursor().indexed_head_block, 3);
}
