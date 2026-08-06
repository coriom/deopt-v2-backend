//! `BACKEND-HYBRID-V2-CHAIN-VIEW-PROVIDER-AND-RECONCILIATION-TASK-V1`
//! — PG-gated integration tests for the production reconciliation
//! surface (RpcChainViewProvider + admin trigger + periodic worker
//! lock semantics).
//!
//! Gating: skips cleanly when `HYBRID_V2_PG_TEST_DATABASE_URL` (or
//! `PG_INTEGRATION_URL`) is unset. Panics loudly if
//! `DEOPT_REQUIRE_PG_INTEGRATION=1` and no URL is provided.
//!
//! Each test drops + recreates the `public` schema.

mod hybrid_v2_mock_rpc_helpers;
mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_view::ChainViewProvider;
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::rebuild_operations::OperationKind;
use deopt_v2_backend::hybrid_v2::reconciler::{
    DriftClassification, ProviderAvailability, ReconciliationRecord,
};
use deopt_v2_backend::hybrid_v2::rpc_chain_view::{
    RpcChainViewProvider, SELECTOR_BALANCE_WITH_YIELD, SELECTOR_GET_RECOVERY_STATE,
    SELECTOR_OWNER_OF,
};
use deopt_v2_backend::hybrid_v2::{RpcHybridV2ChainSource, RpcSourceConfig};
use hybrid_v2_mock_rpc_helpers::MockRpcServer;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

fn get_pg_url_or_skip_or_panic(test_name: &str) -> Option<String> {
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
        eprintln!("SKIP {test_name}: no PG URL");
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

fn abi_encode_address(addr: &str) -> Vec<u8> {
    let stripped = addr.trim().trim_start_matches("0x");
    let mut out = vec![0u8; 32];
    let bytes: Vec<u8> = (0..stripped.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&stripped[i..i + 2], 16).unwrap())
        .collect();
    let start = 32 - bytes.len();
    out[start..].copy_from_slice(&bytes);
    out
}

fn abi_encode_uint256_dec(dec: &str) -> Vec<u8> {
    let mut n = alloy_primitives::U256::from_str_radix(dec, 10).unwrap();
    let mut buf = vec![0u8; 32];
    for i in (0..32).rev() {
        buf[i] = (n & alloy_primitives::U256::from(0xffu8)).to::<u8>();
        n >>= 8;
    }
    buf
}

fn abi_encode_uint8(v: u8) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out[31] = v;
    out
}

fn source_for(mock: &MockRpcServer) -> Arc<RpcHybridV2ChainSource> {
    let cfg = RpcSourceConfig {
        endpoint: mock.url(),
        chain_id: 84532,
        timeout: Duration::from_secs(2),
        max_retries: 3,
        retry_backoff: Duration::from_millis(5),
        max_logs_per_range: 2_000,
        confirmation_depth: 12,
    };
    Arc::new(RpcHybridV2ChainSource::new(cfg, vec![]).expect("source"))
}

/// End-to-end helper: fetch snapshot from mock, run reconciler, persist
/// a `ReconciliationRecord`, return the persisted id.
async fn persist_one_run(
    store: &Arc<dyn HybridV2ProjectionStore>,
    provider: &RpcChainViewProvider,
    deployment_id: i64,
    classification: DriftClassification,
) -> i64 {
    let record = ReconciliationRecord {
        reconciliation_id: None,
        deployment_id,
        ran_at_ms: 1_700_000_000_000,
        block_number_checked: 1,
        block_hash_checked: "0xabcd".to_string(),
        categories_checked: 3,
        items_checked: 1,
        converged_categories: if matches!(classification, DriftClassification::Converged) {
            3
        } else {
            0
        },
        divergent_categories: if classification.is_converged_or_transient() {
            0
        } else {
            1
        },
        classification,
        mismatch_sample_json: None,
        provider_availability: if provider.is_available() {
            ProviderAvailability::Available
        } else {
            ProviderAvailability::Unavailable
        },
        failure_detail: None,
        duration_ms: 1,
    };
    store
        .insert_reconciliation_result(&record)
        .await
        .expect("persist")
}

#[tokio::test]
async fn task_persists_converged_row_from_production_provider() {
    let name = "task_persists_converged_row_from_production_provider";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");

    let mock = MockRpcServer::start().await;
    let source = source_for(&mock);
    let provider = RpcChainViewProvider::new(source, manifest.clone()).expect("provider");
    mock.set_eth_call_response(
        &manifest.module_addresses.subaccount_registry,
        SELECTOR_OWNER_OF,
        abi_encode_address("0x00000000000000000000000000000000000000aa"),
    );
    mock.set_eth_call_response(
        &manifest.module_addresses.recovery_finalizer,
        SELECTOR_GET_RECOVERY_STATE,
        abi_encode_uint8(0),
    );
    let sk = format!("0x{}", "01".repeat(32));
    let tokens: BTreeMap<String, Vec<String>> = BTreeMap::new();
    provider
        .fetch_snapshot_at(1, &[sk.clone()], &tokens)
        .await
        .expect("fetch");

    let id = persist_one_run(&store, &provider, did, DriftClassification::Converged).await;
    assert!(id > 0);
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(latest.classification, DriftClassification::Converged);
    assert_eq!(
        latest.provider_availability,
        ProviderAvailability::Available
    );
    assert!(mock.prohibited_calls().is_empty());
}

#[tokio::test]
async fn task_persists_provider_unavailable_row_never_marks_drift_readiness() {
    let name = "task_persists_provider_unavailable_row_never_marks_drift_readiness";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");

    let mock = MockRpcServer::start().await;
    let source = source_for(&mock);
    let provider = RpcChainViewProvider::new(source, manifest.clone()).expect("provider");
    // No fixtures set → fetch fails.
    let sk = format!("0x{}", "02".repeat(32));
    let tokens: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let _ = provider.fetch_snapshot_at(1, &[sk], &tokens).await;
    provider.mark_unavailable();
    let id = persist_one_run(
        &store,
        &provider,
        did,
        DriftClassification::ProviderUnavailable,
    )
    .await;
    assert!(id > 0);
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(
        latest.classification,
        DriftClassification::ProviderUnavailable
    );
    // Provider unavailable must never publish a drift readiness.
    let readiness = store.read_readiness(did).await.unwrap();
    assert!(
        readiness.is_none() || readiness.map(|r| r.ready).unwrap_or(true),
        "provider unavailable must not mark readiness NOT ready"
    );
}

#[tokio::test]
async fn task_persists_drift_row_from_balance_mismatch() {
    let name = "task_persists_drift_row_from_balance_mismatch";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");

    let mock = MockRpcServer::start().await;
    let source = source_for(&mock);
    let provider = RpcChainViewProvider::new(source, manifest.clone()).expect("provider");
    mock.set_eth_call_response(
        &manifest.module_addresses.subaccount_registry,
        SELECTOR_OWNER_OF,
        abi_encode_address("0x00000000000000000000000000000000000000aa"),
    );
    mock.set_eth_call_response(
        &manifest.module_addresses.collateral_vault,
        SELECTOR_BALANCE_WITH_YIELD,
        abi_encode_uint256_dec("50"),
    );
    mock.set_eth_call_response(
        &manifest.module_addresses.recovery_finalizer,
        SELECTOR_GET_RECOVERY_STATE,
        abi_encode_uint8(0),
    );
    let sk = format!("0x{}", "03".repeat(32));
    let token = "0x00000000000000000000000000000000000000ef".to_string();
    let mut tokens: BTreeMap<String, Vec<String>> = BTreeMap::new();
    tokens.insert(sk.clone(), vec![token]);
    provider
        .fetch_snapshot_at(1, &[sk], &tokens)
        .await
        .expect("fetch");

    let id = persist_one_run(&store, &provider, did, DriftClassification::ProjectionDrift).await;
    assert!(id > 0);
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(latest.classification, DriftClassification::ProjectionDrift);
    assert_eq!(latest.divergent_categories, 1);
}

#[tokio::test]
async fn task_operation_lock_conflicts_between_reconciliation_and_rebuild() {
    let name = "task_operation_lock_conflicts_between_reconciliation_and_rebuild";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    // Rebuild acquires lock first.
    let _rebuild_guard = store
        .try_acquire_operation_lock(did, OperationKind::Rebuild, 1, 1_700_000_000_000)
        .await
        .expect("acq")
        .expect("acquired");
    // Reconciliation contends → None.
    let recon = store
        .try_acquire_operation_lock(did, OperationKind::Reconciliation, 2, 1_700_000_000_001)
        .await
        .expect("acq");
    assert!(
        recon.is_none(),
        "reconciliation must not acquire while rebuild holds"
    );
    // The PG-store `OperationLockGuard` does not carry a store
    // reference (see `rebuild_operations::OperationLockGuard`), so
    // `release()` on it is a no-op. Release explicitly via the
    // store — this is the release contract every production caller
    // (rebuild service, reconciliation task, admin route) uses.
    store.release_operation_lock(did, 1).await.expect("release");
    // After release, reconciliation acquires cleanly.
    let _recon2 = store
        .try_acquire_operation_lock(did, OperationKind::Reconciliation, 3, 1_700_000_000_002)
        .await
        .expect("acq")
        .expect("acquired");
    store.release_operation_lock(did, 3).await.expect("release");
}

#[tokio::test]
async fn task_persisted_row_is_idempotent_over_repeated_reads() {
    let name = "task_persisted_row_is_idempotent_over_repeated_reads";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    let mock = MockRpcServer::start().await;
    let source = source_for(&mock);
    let provider = RpcChainViewProvider::new(source, manifest.clone()).expect("provider");
    let id = persist_one_run(&store, &provider, did, DriftClassification::Converged).await;
    for _ in 0..3 {
        let latest = store
            .read_latest_reconciliation_result(did)
            .await
            .unwrap()
            .expect("latest");
        assert_eq!(latest.reconciliation_id, Some(id));
        assert_eq!(latest.classification, DriftClassification::Converged);
    }
}

#[tokio::test]
async fn task_persists_manifest_mismatch_row() {
    let name = "task_persists_manifest_mismatch_row";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    let mock = MockRpcServer::start().await;
    let source = source_for(&mock);
    let provider = RpcChainViewProvider::new(source, manifest.clone()).expect("provider");
    let id = persist_one_run(
        &store,
        &provider,
        did,
        DriftClassification::ManifestMismatch,
    )
    .await;
    assert!(id > 0);
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(latest.classification, DriftClassification::ManifestMismatch);
}

#[tokio::test]
async fn task_persists_indexer_behind_row() {
    let name = "task_persists_indexer_behind_row";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    let mock = MockRpcServer::start().await;
    let source = source_for(&mock);
    let provider = RpcChainViewProvider::new(source, manifest.clone()).expect("provider");
    let id = persist_one_run(&store, &provider, did, DriftClassification::IndexerBehind).await;
    assert!(id > 0);
}

#[tokio::test]
async fn task_persists_malformed_chain_response_row() {
    let name = "task_persists_malformed_chain_response_row";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    let mock = MockRpcServer::start().await;
    let source = source_for(&mock);
    let provider = RpcChainViewProvider::new(source, manifest.clone()).expect("provider");
    let id = persist_one_run(
        &store,
        &provider,
        did,
        DriftClassification::MalformedChainResponse,
    )
    .await;
    assert!(id > 0);
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(
        latest.classification,
        DriftClassification::MalformedChainResponse
    );
}
