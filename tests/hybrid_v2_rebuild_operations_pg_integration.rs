//! `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — PG-gated
//! integration tests for the persisted rebuild state machine + unified
//! operation lock.
//!
//! Gating: skips cleanly when `HYBRID_V2_PG_TEST_DATABASE_URL` (or
//! `PG_INTEGRATION_URL`) is unset. Panics loudly if
//! `DEOPT_REQUIRE_PG_INTEGRATION=1` and no URL is provided.
//!
//! Each test resets the `public` schema and re-applies migrations, so
//! prior test state cannot leak.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::rebuild_operations::{
    OperationKind, RebuildConfig, RebuildMode, RebuildOperationsService, RebuildOutcome,
    RebuildPhase,
};
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

async fn build_store(pool: &PgPool) -> (Arc<dyn HybridV2ProjectionStore>, i64, i64) {
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    let manifest2 = baseline_manifest_alt(84532, "0xaa");
    let did2 = store
        .upsert_deployment(&manifest2, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment 2");
    (store, did, did2)
}

fn baseline_manifest_alt(
    chain_id: u64,
    manifest_addr_prefix: &str,
) -> deopt_v2_backend::hybrid_v2::manifest::ManifestParams {
    let mut m = baseline_manifest(chain_id);
    m.manifest_address = format!("{}{}", manifest_addr_prefix, "b".repeat(38));
    m.manifest_hash = format!(
        "0x{}",
        "d".repeat(64) // distinct manifest_hash → distinct deployment_id
    );
    // `hybrid_v2_deployments_version_uniq` is keyed on
    // `(chain_id, deployment_version)` — two manifests on the same
    // chain must carry distinct versions.
    m.deployment_version = m.deployment_version.wrapping_add(1);
    m
}

// -----------------------------------------------------------------
//                       TESTS
// -----------------------------------------------------------------

#[tokio::test]
async fn journal_replay_rebuild_nothing_to_do_when_projection_matches_journal() {
    let name = "journal_replay_rebuild_nothing_to_do_when_projection_matches_journal";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did, _did2) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let svc = RebuildOperationsService::new(did, RebuildConfig::default());
    let outcome = svc
        .rebuild_from_journal(&store, &manifest)
        .await
        .expect("rebuild ok");
    match outcome {
        RebuildOutcome::NothingToDo {
            epoch,
            events_replayed,
        } => {
            assert!(epoch >= 1);
            assert_eq!(events_replayed, 0);
        }
        other => panic!("expected NothingToDo, got {other:?}"),
    }
    let row = store
        .read_latest_rebuild_operation(did)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(row.phase, RebuildPhase::Complete);
    assert_eq!(row.mode, RebuildMode::JournalReplay);
    assert_eq!(row.verification_result.as_deref(), Some("MATCH"));
}

#[tokio::test]
async fn rebuild_operation_lock_blocks_reconciliation() {
    let name = "rebuild_operation_lock_blocks_reconciliation";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did, _did2) = build_store(&pool).await;

    // Manually acquire a REBUILD lock and DO NOT release it.
    let guard = store
        .try_acquire_operation_lock(did, OperationKind::Rebuild, 1, 1_700_000_000_000)
        .await
        .expect("first lock ok")
        .expect("first lock granted");
    // Second acquire for RECONCILIATION should fail (contention).
    let second = store
        .try_acquire_operation_lock(did, OperationKind::Reconciliation, 2, 1_700_000_000_001)
        .await
        .expect("second lock ok");
    assert!(second.is_none(), "reconciliation must not grab lock");
    // Release the first — now reconciliation can acquire.
    store
        .release_operation_lock(guard.deployment_id, guard.holder_epoch)
        .await
        .expect("release");
    let third = store
        .try_acquire_operation_lock(did, OperationKind::Reconciliation, 3, 1_700_000_000_002)
        .await
        .expect("third lock ok")
        .expect("third granted after release");
    store
        .release_operation_lock(third.deployment_id, third.holder_epoch)
        .await
        .unwrap();
}

#[tokio::test]
async fn operation_lock_is_deployment_scoped() {
    let name = "operation_lock_is_deployment_scoped";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did, did2) = build_store(&pool).await;
    let g1 = store
        .try_acquire_operation_lock(did, OperationKind::Rebuild, 1, 1)
        .await
        .unwrap()
        .expect("grant 1");
    let g2 = store
        .try_acquire_operation_lock(did2, OperationKind::Rebuild, 1, 1)
        .await
        .unwrap()
        .expect("grant 2");
    assert_ne!(g1.deployment_id, g2.deployment_id);
    store
        .release_operation_lock(g1.deployment_id, g1.holder_epoch)
        .await
        .unwrap();
    store
        .release_operation_lock(g2.deployment_id, g2.holder_epoch)
        .await
        .unwrap();
}

#[tokio::test]
async fn rebuild_phase_persisted_across_restart() {
    let name = "rebuild_phase_persisted_across_restart";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did, _did2) = build_store(&pool).await;

    // Simulate a mid-flight replaying phase persisted before crash.
    let state = deopt_v2_backend::hybrid_v2::rebuild_operations::RebuildOperationState {
        deployment_id: did,
        rebuild_epoch: 5,
        mode: RebuildMode::JournalReplay,
        phase: RebuildPhase::Replaying,
        requested_at_ms: 1_700_000_000_000,
        source_start_block: Some(0),
        source_end_block: Some(10),
        events_replayed: Some(3),
        executions_correlated: Some(0),
        verification_result: None,
        reconciliation_result: None,
        generation_id: None,
        retry_count: 0,
        last_failure_detail: None,
        updated_at_ms: 1_700_000_000_500,
        completed_at_ms: None,
    };
    store
        .upsert_rebuild_operation(&state)
        .await
        .expect("upsert");
    // "Restart": drop the store, reconnect.
    drop(store);
    let store2: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let row = store2
        .read_rebuild_operation(did, 5)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(row.phase, RebuildPhase::Replaying);
    assert_eq!(row.events_replayed, Some(3));
}

#[tokio::test]
async fn duplicate_rebuild_request_is_idempotent_per_epoch() {
    let name = "duplicate_rebuild_request_is_idempotent_per_epoch";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did, _did2) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let svc = RebuildOperationsService::new(did, RebuildConfig::default());

    let a = svc
        .rebuild_from_journal(&store, &manifest)
        .await
        .expect("a");
    let b = svc
        .rebuild_from_journal(&store, &manifest)
        .await
        .expect("b");
    let (ea, eb) = match (a, b) {
        (
            RebuildOutcome::NothingToDo { epoch: ea, .. },
            RebuildOutcome::NothingToDo { epoch: eb, .. },
        ) => (ea, eb),
        other => panic!("expected two NothingToDo, got {other:?}"),
    };
    // Second call opens a new epoch (previous is terminal).
    assert!(eb > ea);
    // Both rows present.
    let latest = store
        .read_latest_rebuild_operation(did)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(latest.rebuild_epoch, eb);
    assert_eq!(latest.phase, RebuildPhase::Complete);
}

#[tokio::test]
async fn latest_rebuild_operation_returns_highest_epoch() {
    let name = "latest_rebuild_operation_returns_highest_epoch";
    let Some(url) = get_pg_url_or_skip_or_panic(name) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did, _did2) = build_store(&pool).await;
    for epoch in [3, 7, 1] {
        let state = deopt_v2_backend::hybrid_v2::rebuild_operations::RebuildOperationState {
            deployment_id: did,
            rebuild_epoch: epoch,
            mode: RebuildMode::JournalReplay,
            phase: RebuildPhase::Complete,
            requested_at_ms: 100 + epoch,
            source_start_block: None,
            source_end_block: None,
            events_replayed: None,
            executions_correlated: None,
            verification_result: None,
            reconciliation_result: None,
            generation_id: None,
            retry_count: 0,
            last_failure_detail: None,
            updated_at_ms: 100 + epoch,
            completed_at_ms: Some(200 + epoch),
        };
        store.upsert_rebuild_operation(&state).await.unwrap();
    }
    let latest = store
        .read_latest_rebuild_operation(did)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(latest.rebuild_epoch, 7);
}
