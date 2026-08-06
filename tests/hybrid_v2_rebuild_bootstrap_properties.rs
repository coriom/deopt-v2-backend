//! `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1`
//! — in-memory property tests validating the corrective work:
//!
//! 1. `IndexerRuntime::bootstrap_from_persistence` returns
//!    `BootstrapResult::RebuildBlocked` and sets a hard-503
//!    readiness variant when the persisted rebuild-op row is
//!    active/failed/manual-intervention-required.
//! 2. The `snapshot_projection_state` PG stub previously returned
//!    `None` for every deployment; the InMemory equivalent
//!    surface honours a stored snapshot round-trip.
//! 3. `commit_rematerialization` (InMemory) updates the paired
//!    rebuild-op row phase to `Complete` and publishes the
//!    passed-in snapshot.
//!
//! These tests use `InMemoryProjectionStore` so they always run
//! in the default `cargo test` suite (no PG gating).

use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, InMemoryProjectionStore, ReadinessSnapshot, RuntimeCursorSnapshot,
};
use deopt_v2_backend::hybrid_v2::readiness::ReadinessReason;
use deopt_v2_backend::hybrid_v2::rebuild_operations::{
    RebuildMode, RebuildOperationState, RebuildPhase,
};
use deopt_v2_backend::hybrid_v2::reducer::{
    ExecutionCompletion, MatchedExecutionRow, ProjectionState,
};
use deopt_v2_backend::hybrid_v2::runtime::{BootstrapResult, IndexerRuntime};
use deopt_v2_backend::hybrid_v2::ManifestParams;
use std::sync::Arc;

mod hybrid_v2_support;
use hybrid_v2_support::baseline_manifest;

fn manifest_84532() -> ManifestParams {
    baseline_manifest(84532)
}

async fn build_store_with_deployment() -> (Arc<InMemoryProjectionStore>, i64) {
    let store: Arc<InMemoryProjectionStore> = Arc::new(InMemoryProjectionStore::new());
    let did = store
        .upsert_deployment(&manifest_84532(), "ACTIVE", 1_700_000_000_000)
        .await
        .expect("upsert_deployment");
    (store, did)
}

fn baseline_rebuild_row(did: i64, phase: RebuildPhase) -> RebuildOperationState {
    RebuildOperationState {
        deployment_id: did,
        rebuild_epoch: 42,
        mode: RebuildMode::JournalReplay,
        phase,
        requested_at_ms: 1_700_000_000_000,
        source_start_block: None,
        source_end_block: None,
        events_replayed: None,
        executions_correlated: None,
        verification_result: None,
        reconciliation_result: None,
        generation_id: None,
        retry_count: 0,
        last_failure_detail: None,
        updated_at_ms: 1_700_000_000_000,
        completed_at_ms: None,
    }
}

#[tokio::test]
async fn bootstrap_returns_rebuild_blocked_when_rebuild_active() {
    let (store, did) = build_store_with_deployment().await;
    // Insert an active rebuild row.
    let row = baseline_rebuild_row(did, RebuildPhase::Replaying);
    store.upsert_rebuild_operation(&row).await.unwrap();

    // Build the runtime and bootstrap.
    let manifest = manifest_84532();
    let mut runtime = IndexerRuntime::new(did as u64, manifest)
        .with_persistence(store.clone() as Arc<dyn HybridV2ProjectionStore>, did);
    let out = runtime.bootstrap_from_persistence().await.unwrap();
    match out {
        BootstrapResult::RebuildBlocked {
            rebuild_epoch,
            phase,
        } => {
            assert_eq!(rebuild_epoch, 42);
            assert_eq!(phase, "REPLAYING");
        }
        other => panic!("expected RebuildBlocked, got {other:?}"),
    }
    // Readiness must be hard-503 (RebuildInProgress).
    assert!(!runtime.readiness.ready);
    assert!(matches!(
        runtime.readiness.reason,
        Some(ReadinessReason::RebuildInProgress)
    ));
}

#[tokio::test]
async fn bootstrap_returns_rebuild_blocked_when_rebuild_requested() {
    let (store, did) = build_store_with_deployment().await;
    let row = baseline_rebuild_row(did, RebuildPhase::Requested);
    store.upsert_rebuild_operation(&row).await.unwrap();

    let mut runtime = IndexerRuntime::new(did as u64, manifest_84532())
        .with_persistence(store.clone() as Arc<dyn HybridV2ProjectionStore>, did);
    let out = runtime.bootstrap_from_persistence().await.unwrap();
    assert!(matches!(
        out,
        BootstrapResult::RebuildBlocked { phase, .. } if phase == "REQUESTED"
    ));
    assert!(!runtime.readiness.ready);
    assert!(matches!(
        runtime.readiness.reason,
        Some(ReadinessReason::RebuildRequested { epoch: 42 })
    ));
}

#[tokio::test]
async fn bootstrap_returns_rebuild_blocked_when_rebuild_failed() {
    let (store, did) = build_store_with_deployment().await;
    let mut row = baseline_rebuild_row(did, RebuildPhase::Failed);
    row.last_failure_detail = Some("boom".to_string());
    store.upsert_rebuild_operation(&row).await.unwrap();

    let mut runtime = IndexerRuntime::new(did as u64, manifest_84532())
        .with_persistence(store.clone() as Arc<dyn HybridV2ProjectionStore>, did);
    let out = runtime.bootstrap_from_persistence().await.unwrap();
    assert!(matches!(
        out,
        BootstrapResult::RebuildBlocked { phase, .. } if phase == "FAILED"
    ));
    match runtime.readiness.reason {
        Some(ReadinessReason::RebuildFailed { detail }) => assert_eq!(detail, "boom"),
        other => panic!("expected RebuildFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn bootstrap_ignores_completed_rebuild_op() {
    let (store, did) = build_store_with_deployment().await;
    // Insert a Complete rebuild row — bootstrap should proceed to
    // the normal cursor/readiness path (FreshStart in this case).
    let mut row = baseline_rebuild_row(did, RebuildPhase::Complete);
    row.completed_at_ms = Some(1_700_000_000_100);
    store.upsert_rebuild_operation(&row).await.unwrap();

    let mut runtime = IndexerRuntime::new(did as u64, manifest_84532())
        .with_persistence(store.clone() as Arc<dyn HybridV2ProjectionStore>, did);
    let out = runtime.bootstrap_from_persistence().await.unwrap();
    assert!(matches!(out, BootstrapResult::FreshStart));
}

#[tokio::test]
async fn commit_rematerialization_publishes_snapshot_and_marks_complete() {
    let (store, did) = build_store_with_deployment().await;
    // Seed a Requested rebuild row.
    let mut row = baseline_rebuild_row(did, RebuildPhase::Requested);
    row.rebuild_epoch = 7;
    store.upsert_rebuild_operation(&row).await.unwrap();

    let mut state = ProjectionState::default();
    state.balances.insert(
        ("0xsub".to_string(), "0xtoken".to_string()),
        "1000".to_string(),
    );
    state.matched_executions.insert(
        "exec-1".to_string(),
        MatchedExecutionRow {
            buyer_order_hash: "0xbuy".to_string(),
            seller_order_hash: "0xsell".to_string(),
            buyer_subkey: "0xbuyer".to_string(),
            seller_subkey: "0xseller".to_string(),
            series_id: "0xseries".to_string(),
            matched_qty_1e8: "100".to_string(),
            premium_amount: "50".to_string(),
            fee_amount: "1".to_string(),
            rebate_amount: "0".to_string(),
            block_number: 42,
            tx_hash: "0xtx".to_string(),
            completion_status: ExecutionCompletion::Complete,
        },
    );

    let cursor = RuntimeCursorSnapshot {
        cursor_name: "hybrid_v2_indexer".to_string(),
        indexed_head_block: 42,
        indexed_head_hash: "0xhash".to_string(),
        indexed_head_parent: "0xparent".to_string(),
        observed_head_block: 42,
        finalized_head_block: 40,
        last_error: None,
        reorg_count: 0,
        max_reorg_depth_seen: 0,
        decode_failures: 0,
        projection_failures: 0,
        unknown_canonical_events: 0,
        last_success_block: 42,
    };
    let readiness = ReadinessSnapshot {
        ready: true,
        reason: None,
        reason_detail: None,
    };
    store
        .commit_rematerialization(did, 7, &state, &cursor, &readiness, 1_700_000_000_500)
        .await
        .unwrap();

    // Verify the rebuild-op row is Complete.
    let latest = store
        .read_latest_rebuild_operation(did)
        .await
        .unwrap()
        .expect("row exists");
    assert_eq!(latest.phase, RebuildPhase::Complete);
    assert_eq!(latest.completed_at_ms, Some(1_700_000_000_500));

    // Verify the published snapshot round-trips.
    let published = store
        .snapshot_projection_state(did)
        .await
        .unwrap()
        .expect("snapshot present");
    assert_eq!(published.balances, state.balances);
    assert_eq!(
        published
            .matched_executions
            .get("exec-1")
            .unwrap()
            .premium_amount,
        "50"
    );
}

// -----------------------------------------------------------------
//   AUTO-REMATERIALIZE FLOW (drift -> commit_rematerialization)
// -----------------------------------------------------------------

#[tokio::test]
async fn rebuild_from_journal_auto_rematerialize_rewrites_projection() {
    use deopt_v2_backend::hybrid_v2::rebuild_operations::{
        RebuildConfig, RebuildOperationsService, RebuildOutcome,
    };
    let (store, did) = build_store_with_deployment().await;

    // Seed a "drifted" published snapshot the rebuild will detect.
    let mut drift_state = ProjectionState::default();
    drift_state.balances.insert(
        ("0xsub".to_string(), "0xtoken".to_string()),
        "999_999".to_string(),
    );
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    // Publish it directly onto the InMemory snapshot bucket so the
    // rebuild finds drift against the (empty) journal replay.
    let cursor = RuntimeCursorSnapshot {
        cursor_name: "hybrid_v2_indexer".to_string(),
        indexed_head_block: 0,
        indexed_head_hash: String::new(),
        indexed_head_parent: String::new(),
        observed_head_block: 0,
        finalized_head_block: 0,
        last_error: None,
        reorg_count: 0,
        max_reorg_depth_seen: 0,
        decode_failures: 0,
        projection_failures: 0,
        unknown_canonical_events: 0,
        last_success_block: 0,
    };
    let readiness = ReadinessSnapshot {
        ready: true,
        reason: None,
        reason_detail: None,
    };
    // Seed a rebuild-op row that the commit will target.
    let seed_row = baseline_rebuild_row(did, RebuildPhase::Preparing);
    store_dyn.upsert_rebuild_operation(&seed_row).await.unwrap();
    store_dyn
        .commit_rematerialization(
            did,
            42,
            &drift_state,
            &cursor,
            &readiness,
            1_700_000_000_100,
        )
        .await
        .unwrap();

    // Sanity: drift snapshot is now visible.
    let live = store_dyn
        .snapshot_projection_state(did)
        .await
        .unwrap()
        .expect("snapshot");
    assert_eq!(live.balances.len(), 1);

    // Run rebuild_from_journal with auto_rematerialize=true. The
    // canonical journal is empty (no blocks ingested), so the rebuilt
    // state is default (no balances). Auto-rematerialize should
    // TRUNCATE + REINSERT (which drops the drifted balance row).
    let mut config = RebuildConfig::default();
    config.auto_rematerialize = true;
    let service = RebuildOperationsService::new(did, config);
    let manifest = manifest_84532();
    let outcome = service
        .rebuild_from_journal(&store_dyn, &manifest)
        .await
        .unwrap();
    match outcome {
        RebuildOutcome::Rebuilt {
            events_replayed,
            executions_correlated,
            ..
        } => {
            assert_eq!(events_replayed, 0);
            assert_eq!(executions_correlated, 0);
        }
        RebuildOutcome::NothingToDo { .. } => {
            // Also acceptable — depends on comparison semantics on
            // empty rebuilt state vs single-balance live.
        }
        other => panic!("expected Rebuilt or NothingToDo, got {other:?}"),
    }

    // After rebuild, the published snapshot must not contain the
    // drifted balance (or must have been reset to the rebuilt state).
    let after = store_dyn
        .snapshot_projection_state(did)
        .await
        .unwrap()
        .expect("snapshot");
    assert!(
        after.balances.is_empty(),
        "expected balances wiped, got {:?}",
        after.balances
    );
}
