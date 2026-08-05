//! `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — PG-gated
//! integration tests for the reconciliation scheduler + persisted
//! drift-classification results.
//!
//! Gating: skips cleanly when `HYBRID_V2_PG_TEST_DATABASE_URL` (or
//! `PG_INTEGRATION_URL`) is unset. Panics loudly if
//! `DEOPT_REQUIRE_PG_INTEGRATION=1` and no URL is provided.
//!
//! Every test drops + recreates the `public` schema.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_view::{ChainSnapshot, InMemoryChainViewProvider};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::rebuild_operations::OperationKind;
use deopt_v2_backend::hybrid_v2::reconciler::{
    DriftClassification, ProviderAvailability, ReconciliationRecord, ReconciliationScheduler,
    ReconciliationSchedulerConfig,
};
use deopt_v2_backend::hybrid_v2::reducer::ProjectionState;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};
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

async fn build_store(pool: &PgPool) -> (Arc<dyn HybridV2ProjectionStore>, i64) {
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    (store, did)
}

fn empty_state() -> ProjectionState {
    ProjectionState::default()
}

#[tokio::test]
async fn reconciliation_converged_persists_row() {
    let name = "reconciliation_converged_persists_row";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    let mut snap = ChainSnapshot::default();
    snap.manifest_hash = manifest.manifest_hash.clone();
    provider.set_snapshot(1, snap);
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let state = empty_state();
    let record = sched
        .run_once(
            &store,
            &provider,
            &manifest.manifest_hash,
            &state,
            1,
            "0xabcd",
        )
        .await
        .expect("run");
    assert_eq!(record.classification, DriftClassification::Converged);
    assert_eq!(
        record.provider_availability,
        ProviderAvailability::Available
    );
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(latest.classification, DriftClassification::Converged);
    assert!(latest.reconciliation_id.is_some());
}

#[tokio::test]
async fn reconciliation_detects_manifest_mismatch() {
    let name = "reconciliation_detects_manifest_mismatch";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    let mut snap = ChainSnapshot::default();
    // Distinct manifest hash on chain → MANIFEST_MISMATCH.
    snap.manifest_hash = "0xdeadbeef".to_string();
    provider.set_snapshot(1, snap);
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let state = empty_state();
    let record = sched
        .run_once(
            &store,
            &provider,
            &manifest.manifest_hash,
            &state,
            1,
            "0xabcd",
        )
        .await
        .expect("run");
    assert_eq!(record.classification, DriftClassification::ManifestMismatch);
    // Persisted.
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(latest.classification, DriftClassification::ManifestMismatch);
    // Readiness snapshot must have been marked NOT ready.
    let readiness = store
        .read_readiness(did)
        .await
        .unwrap()
        .expect("readiness snapshot");
    assert!(!readiness.ready);
    assert_eq!(readiness.reason.as_deref(), Some("RECONCILIATION_DRIFT"));
}

#[tokio::test]
async fn reconciliation_provider_unavailable_never_publishes_drift() {
    let name = "reconciliation_provider_unavailable_never_publishes_drift";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    provider.set_available(false);
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let state = empty_state();
    let record = sched
        .run_once(
            &store,
            &provider,
            &manifest.manifest_hash,
            &state,
            1,
            "0xabcd",
        )
        .await
        .expect("run");
    assert_eq!(
        record.classification,
        DriftClassification::ProviderUnavailable
    );
    // Persisted.
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(
        latest.classification,
        DriftClassification::ProviderUnavailable
    );
    // Readiness NOT touched (no drift written).
    let readiness = store.read_readiness(did).await.unwrap();
    assert!(
        readiness.is_none() || readiness.as_ref().map(|r| r.ready).unwrap_or(true),
        "provider unavailable must not mark readiness NOT ready"
    );
}

#[tokio::test]
async fn reconciliation_no_auto_repair_on_drift() {
    let name = "reconciliation_no_auto_repair_on_drift";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    let mut snap = ChainSnapshot::default();
    snap.manifest_hash = manifest.manifest_hash.clone();
    // Chain has a balance our projection does not.
    snap.balances
        .insert(("subX".to_string(), "TKN".to_string()), "1000".to_string());
    provider.set_snapshot(1, snap);
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let mut state = empty_state();
    state
        .balances
        .insert(("subX".to_string(), "TKN".to_string()), "500".to_string());
    let before = state.balances.clone();
    let record = sched
        .run_once(
            &store,
            &provider,
            &manifest.manifest_hash,
            &state,
            1,
            "0xabcd",
        )
        .await
        .expect("run");
    assert_eq!(record.classification, DriftClassification::ProjectionDrift);
    // Projection state passed in unchanged (borrow was &state); no auto-repair.
    assert_eq!(state.balances, before);
    // Readiness = drift.
    let readiness = store.read_readiness(did).await.unwrap().expect("readiness");
    assert!(!readiness.ready);
    assert_eq!(readiness.reason.as_deref(), Some("RECONCILIATION_DRIFT"));
}

#[tokio::test]
async fn reconciliation_history_append_only() {
    let name = "reconciliation_history_append_only";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    // Manually append three records with increasing ran_at_ms.
    for (i, cls) in [
        DriftClassification::ProjectionDrift,
        DriftClassification::ProviderUnavailable,
        DriftClassification::Converged,
    ]
    .into_iter()
    .enumerate()
    {
        let r = ReconciliationRecord {
            reconciliation_id: None,
            deployment_id: did,
            ran_at_ms: 1_700_000_000_000 + i as i64,
            block_number_checked: i as u64 + 1,
            block_hash_checked: format!("0x{}", "a".repeat(64)),
            categories_checked: 6,
            items_checked: 0,
            converged_categories: if matches!(cls, DriftClassification::Converged) {
                6
            } else {
                0
            },
            divergent_categories: if matches!(cls, DriftClassification::ProjectionDrift) {
                1
            } else {
                0
            },
            classification: cls,
            mismatch_sample_json: None,
            provider_availability: ProviderAvailability::Available,
            failure_detail: None,
            duration_ms: 5,
        };
        store
            .insert_reconciliation_result(&r)
            .await
            .expect("insert");
    }
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    // Highest ran_at_ms wins → Converged.
    assert_eq!(latest.classification, DriftClassification::Converged);
}

#[tokio::test]
async fn reconciliation_lock_exclusive_with_rebuild() {
    let name = "reconciliation_lock_exclusive_with_rebuild";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    // Take rebuild lock first.
    let rebuild_guard = store
        .try_acquire_operation_lock(did, OperationKind::Rebuild, 1, 1_700_000_000_000)
        .await
        .unwrap()
        .expect("rebuild lock");
    // Reconciliation must be blocked.
    let recon_lock = store
        .try_acquire_operation_lock(did, OperationKind::Reconciliation, 2, 1_700_000_000_001)
        .await
        .unwrap();
    assert!(recon_lock.is_none());
    store
        .release_operation_lock(rebuild_guard.deployment_id, rebuild_guard.holder_epoch)
        .await
        .unwrap();
    // Now reconciliation can proceed.
    let after = store
        .try_acquire_operation_lock(did, OperationKind::Reconciliation, 3, 1_700_000_000_002)
        .await
        .unwrap()
        .expect("reconciliation after release");
    store
        .release_operation_lock(after.deployment_id, after.holder_epoch)
        .await
        .unwrap();
}
