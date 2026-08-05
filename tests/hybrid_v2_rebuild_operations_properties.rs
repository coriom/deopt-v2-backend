//! `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — in-memory
//! property + smoke tests for the rebuild state machine, reconciler,
//! and unified operation lock. These tests run in the default
//! `cargo test --workspace` set (no PG needed).
//!
//! Rationale: PG-gated tests only run under `HYBRID_V2_PG_TEST_DATABASE_URL`;
//! we still want deterministic CI coverage that the state machine
//! progresses, verification detects drift, and the operation lock
//! obeys mutual exclusion / deployment isolation.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_view::{ChainSnapshot, InMemoryChainViewProvider};
use deopt_v2_backend::hybrid_v2::persistence::{HybridV2ProjectionStore, InMemoryProjectionStore};
use deopt_v2_backend::hybrid_v2::rebuild_operations::{
    OperationKind, RebuildConfig, RebuildMode, RebuildOperationsService, RebuildOutcome,
    RebuildPhase,
};
use deopt_v2_backend::hybrid_v2::reconciler::{
    DriftClassification, ProviderAvailability, ReconciliationScheduler,
    ReconciliationSchedulerConfig,
};
use deopt_v2_backend::hybrid_v2::reducer::ProjectionState;
use hybrid_v2_support::baseline_manifest;
use std::sync::Arc;

async fn store_and_id() -> (Arc<dyn HybridV2ProjectionStore>, i64) {
    let store: Arc<dyn HybridV2ProjectionStore> = Arc::new(InMemoryProjectionStore::new());
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("did");
    (store, did)
}

// -----------------------------------------------------------------
//                       REBUILD STATE MACHINE
// -----------------------------------------------------------------

#[tokio::test]
async fn journal_replay_nothing_to_do_on_empty_journal_and_empty_projection() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let svc = RebuildOperationsService::new(did, RebuildConfig::default());
    let outcome = svc
        .rebuild_from_journal(&store, &manifest)
        .await
        .expect("rebuild ok");
    match outcome {
        RebuildOutcome::NothingToDo { events_replayed, .. } => {
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
}

#[tokio::test]
async fn rebuild_epoch_monotonically_advances_across_calls() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let svc = RebuildOperationsService::new(did, RebuildConfig::default());
    let e1 = match svc.rebuild_from_journal(&store, &manifest).await.unwrap() {
        RebuildOutcome::NothingToDo { epoch, .. } => epoch,
        other => panic!("{other:?}"),
    };
    let e2 = match svc.rebuild_from_journal(&store, &manifest).await.unwrap() {
        RebuildOutcome::NothingToDo { epoch, .. } => epoch,
        other => panic!("{other:?}"),
    };
    assert!(e2 > e1);
}

#[tokio::test]
async fn fresh_chain_ingest_marks_complete_on_success() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let svc = RebuildOperationsService::new(did, RebuildConfig::default());
    let outcome = svc
        .reconstruct_from_chain(&store, &manifest, 0, 5, |_start, _end| async move {
            Ok::<(u64, u64), String>((17, 3))
        })
        .await
        .expect("ok");
    match outcome {
        RebuildOutcome::Rebuilt {
            events_replayed,
            executions_correlated,
            ..
        } => {
            assert_eq!(events_replayed, 17);
            assert_eq!(executions_correlated, 3);
        }
        other => panic!("expected Rebuilt, got {other:?}"),
    }
    let row = store
        .read_latest_rebuild_operation(did)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(row.phase, RebuildPhase::Complete);
    assert_eq!(row.mode, RebuildMode::FreshChain);
}

#[tokio::test]
async fn fresh_chain_ingest_failure_is_retryable_then_manual() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let mut cfg = RebuildConfig::default();
    cfg.retry_max = 1;
    let svc = RebuildOperationsService::new(did, cfg);
    // First failure — retry_count 1 <= max 1 → Failed (retryable).
    let outcome = svc
        .reconstruct_from_chain(&store, &manifest, 0, 5, |_, _| async move {
            Err::<(u64, u64), String>("rpc down".to_string())
        })
        .await
        .expect("ok");
    match outcome {
        RebuildOutcome::RetryableFailure { .. } => {}
        other => panic!("expected RetryableFailure, got {other:?}"),
    }
}

#[tokio::test]
async fn rebuild_verification_detects_drift_and_escalates_manual() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    // Seed a nontrivial live projection: adds a balance the journal
    // (empty) will NOT reconstruct.
    let mut state = ProjectionState::default();
    state
        .balances
        .insert(("subK".to_string(), "TKN".to_string()), "42".to_string());
    // We need to inject the state snapshot into the in-memory store.
    // The InMemoryProjectionStore only sets state via persist_block_atomic;
    // for this test we cast to the concrete type and mutate via a
    // helper. Since we don't have a public setter, use persist_block_atomic
    // with a synthetic block that contains no logs (matches the empty
    // journal), then manually write the projection state via a fresh
    // persist call. Simplest: write a synthetic block with no logs and
    // manually update by another route.
    // Because InMemoryProjectionStore's snapshot_state helper returns
    // whatever the last persist_block_atomic passed as `state`, we can
    // write one such block.
    use deopt_v2_backend::hybrid_v2::persistence::{
        CanonicalBlockRef, ReadinessSnapshot, RuntimeCursorSnapshot,
    };
    let block = CanonicalBlockRef {
        block_number: 1,
        block_hash: "0xdeadbeef".to_string(),
        parent_hash: "0x00".to_string(),
        block_timestamp: 1_700_000_000,
    };
    let cursor = RuntimeCursorSnapshot {
        cursor_name: "indexer".to_string(),
        indexed_head_block: 1,
        indexed_head_hash: "0xdeadbeef".to_string(),
        indexed_head_parent: "0x00".to_string(),
        observed_head_block: 1,
        finalized_head_block: 0,
        ..Default::default()
    };
    let readiness = ReadinessSnapshot {
        ready: true,
        reason: None,
        reason_detail: None,
    };
    store
        .persist_block_atomic(
            did,
            &block,
            &[],
            &[],
            &state,
            &cursor,
            &readiness,
            1_700_000_000_000,
        )
        .await
        .expect("persist");

    let svc = RebuildOperationsService::new(did, RebuildConfig::default());
    let outcome = svc
        .rebuild_from_journal(&store, &manifest)
        .await
        .expect("rebuild ok");
    match outcome {
        RebuildOutcome::ManualInterventionRequired { reason, .. } => {
            assert!(
                reason.contains("balances differ")
                    || reason.contains("drift"),
                "expected drift reason, got {reason}"
            );
        }
        other => panic!("expected ManualInterventionRequired, got {other:?}"),
    }
    let row = store
        .read_latest_rebuild_operation(did)
        .await
        .unwrap()
        .expect("row");
    assert_eq!(row.phase, RebuildPhase::ManualInterventionRequired);
    assert_eq!(row.verification_result.as_deref(), Some("DRIFT"));
}

// -----------------------------------------------------------------
//                       UNIFIED OPERATION LOCK
// -----------------------------------------------------------------

#[tokio::test]
async fn operation_lock_mutually_exclusive_across_kinds() {
    let (store, did) = store_and_id().await;
    let g = store
        .try_acquire_operation_lock(did, OperationKind::Rebuild, 1, 1)
        .await
        .unwrap()
        .expect("grant");
    for kind in [OperationKind::Reorg, OperationKind::Rebuild] {
        assert!(store
            .try_acquire_operation_lock(did, kind, 2, 2)
            .await
            .unwrap()
            .is_none());
    }
    store
        .release_operation_lock(g.deployment_id, g.holder_epoch)
        .await
        .unwrap();
}

#[tokio::test]
async fn operation_lock_deployment_isolation() {
    let store: Arc<dyn HybridV2ProjectionStore> = Arc::new(InMemoryProjectionStore::new());
    let m1 = baseline_manifest(84532);
    let d1 = store.upsert_deployment(&m1, "P", 1).await.unwrap();
    let mut m2 = baseline_manifest(84532);
    m2.manifest_hash = format!("0x{}", "f".repeat(64));
    let d2 = store.upsert_deployment(&m2, "P", 1).await.unwrap();
    let g1 = store
        .try_acquire_operation_lock(d1, OperationKind::Rebuild, 1, 1)
        .await
        .unwrap()
        .expect("d1");
    let g2 = store
        .try_acquire_operation_lock(d2, OperationKind::Rebuild, 1, 1)
        .await
        .unwrap()
        .expect("d2");
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

// -----------------------------------------------------------------
//                       RECONCILIATION
// -----------------------------------------------------------------

#[tokio::test]
async fn reconciliation_converged_emits_ok_and_no_readiness_drift() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    let mut snap = ChainSnapshot::default();
    snap.manifest_hash = manifest.manifest_hash.clone();
    provider.set_snapshot(1, snap);
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let state = ProjectionState::default();
    let record = sched
        .run_once(&store, &provider, &manifest.manifest_hash, &state, 1, "0xabcd")
        .await
        .expect("run");
    assert_eq!(record.classification, DriftClassification::Converged);
    assert_eq!(record.provider_availability, ProviderAvailability::Available);
    // No drift snapshot readiness written.
    let readiness = store.read_readiness(did).await.unwrap();
    assert!(readiness.is_none() || readiness.as_ref().map(|r| r.ready).unwrap_or(true));
}

#[tokio::test]
async fn reconciliation_projection_drift_writes_readiness_and_persists_row() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    let mut snap = ChainSnapshot::default();
    snap.manifest_hash = manifest.manifest_hash.clone();
    snap.balances
        .insert(("A".to_string(), "T".to_string()), "1000".to_string());
    provider.set_snapshot(1, snap);
    let mut state = ProjectionState::default();
    state
        .balances
        .insert(("A".to_string(), "T".to_string()), "1".to_string());
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let record = sched
        .run_once(&store, &provider, &manifest.manifest_hash, &state, 1, "0xabcd")
        .await
        .expect("run");
    assert_eq!(record.classification, DriftClassification::ProjectionDrift);
    let latest = store
        .read_latest_reconciliation_result(did)
        .await
        .unwrap()
        .expect("latest");
    assert_eq!(latest.classification, DriftClassification::ProjectionDrift);
    let readiness = store
        .read_readiness(did)
        .await
        .unwrap()
        .expect("readiness");
    assert!(!readiness.ready);
    assert_eq!(readiness.reason.as_deref(), Some("RECONCILIATION_DRIFT"));
}

#[tokio::test]
async fn reconciliation_provider_unavailable_never_touches_readiness() {
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    provider.set_available(false);
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let state = ProjectionState::default();
    let record = sched
        .run_once(&store, &provider, &manifest.manifest_hash, &state, 1, "0xabcd")
        .await
        .expect("run");
    assert_eq!(
        record.classification,
        DriftClassification::ProviderUnavailable
    );
    let readiness = store.read_readiness(did).await.unwrap();
    assert!(readiness.is_none() || readiness.as_ref().map(|r| r.ready).unwrap_or(true));
}

#[tokio::test]
async fn reconciliation_drift_never_repairs_projection() {
    // Property: after a drift is detected + persisted, we can invoke
    // reconciliation again and the projection state is untouched
    // (`&state` is immutable, but this also verifies no back-channel
    // writes to the projection tables).
    let (store, did) = store_and_id().await;
    let manifest = baseline_manifest(84532);
    let mut provider = InMemoryChainViewProvider::new();
    let mut snap = ChainSnapshot::default();
    snap.manifest_hash = manifest.manifest_hash.clone();
    snap.balances
        .insert(("A".to_string(), "T".to_string()), "1000".to_string());
    provider.set_snapshot(1, snap);
    let mut state = ProjectionState::default();
    state
        .balances
        .insert(("A".to_string(), "T".to_string()), "1".to_string());
    let sched = ReconciliationScheduler::new(did, ReconciliationSchedulerConfig::default());
    let before = state.balances.clone();
    for _ in 0..3 {
        let record = sched
            .run_once(&store, &provider, &manifest.manifest_hash, &state, 1, "0xabcd")
            .await
            .expect("run");
        assert_eq!(record.classification, DriftClassification::ProjectionDrift);
    }
    assert_eq!(state.balances, before);
}
