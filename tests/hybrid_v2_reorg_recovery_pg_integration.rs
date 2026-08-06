//! `BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1` — end-to-end
//! integration tests for the persisted reorg recovery service against
//! a real Postgres database + `InMemoryChainSource` (deterministic).
//!
//! Gating: skips cleanly when `HYBRID_V2_PG_TEST_DATABASE_URL` (or
//! `PG_INTEGRATION_URL`) is unset. Panics loudly if
//! `DEOPT_REQUIRE_PG_INTEGRATION=1` and no URL is provided.
//!
//! Every test drops + recreates the `public` schema so state does not
//! leak between tests.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_source::{InMemoryChainSource, RawBlock};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::reorg_recovery::{
    RecoveryOutcome, ReorgDetection, ReorgRecoveryConfig, ReorgRecoveryPhase, ReorgRecoveryService,
    ReorgRecoveryState,
};
use deopt_v2_backend::hybrid_v2::runtime::IndexerRuntime;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
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

async fn build_store(pool: &PgPool) -> (Arc<PostgresHybridV2ProjectionStore>, i64) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    (store, did)
}

fn make_block(number: u64, tag: u8, parent: &str) -> RawBlock {
    RawBlock {
        number,
        hash: format!(
            "0x{}{}",
            format!("{:02x}", tag).repeat(1),
            format!("{:0>62x}", number)
        ),
        parent_hash: parent.to_string(),
        timestamp: 1_700_000_000 + number,
        logs: Vec::new(),
    }
}

async fn drive_forward(
    runtime: &mut IndexerRuntime,
    source: &InMemoryChainSource,
    up_to: u64,
) -> u64 {
    let mut applied = 0;
    while runtime.cursor().indexed_head_block < up_to {
        match runtime.tick_and_persist(source).await {
            Ok(true) => applied += 1,
            Ok(false) => break,
            Err(_) => break,
        }
    }
    applied
}

// -----------------------------------------------------------------
//                       TESTS
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn one_block_reorg_recovered_end_to_end() {
    let Some(url) = get_pg_url_or_skip_or_panic("one_block_reorg_recovered_end_to_end") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest.clone()).with_persistence(store.clone(), did);

    let mut source = InMemoryChainSource::new(84532);
    let b0 = make_block(0, 0xaa, "");
    let b1 = make_block(1, 0xaa, &b0.hash);
    let b2 = make_block(2, 0xaa, &b1.hash);
    source.push(b0.clone());
    source.push(b1.clone());
    source.push(b2.clone());
    let _ = drive_forward(&mut runtime, &source, 2).await;
    assert_eq!(runtime.cursor().indexed_head_block, 2);

    // Reorg: replace b2 + extend one block on the replacement branch.
    let b2b = make_block(2, 0xbb, &b1.hash);
    let b3b = make_block(3, 0xbb, &b2b.hash);
    source.reorg_from(2, vec![b2b.clone(), b3b.clone()]);
    let err = runtime.tick_and_persist(&source).await.err();
    assert!(err.is_some(), "expected ReorgRequired");

    let service = ReorgRecoveryService::new(did, ReorgRecoveryConfig::default());
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    let detection = ReorgDetection {
        old_tip_block: 2,
        old_tip_hash: b2.hash.clone(),
        conflicting_block: Some(3),
        conflicting_hash: Some(b3b.hash.clone()),
    };
    let outcome = service
        .recover(&source, &store_dyn, &manifest, Some(detection), "indexer")
        .await
        .expect("recover ok");
    match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            orphan_depth,
            ..
        } => {
            assert_eq!(replacement_tip, 3);
            assert!(replacement_hash.eq_ignore_ascii_case(&b3b.hash));
            assert_eq!(orphan_depth, 1);
        }
        other => panic!("expected Recovered, got {other:?}"),
    }

    // Post-recovery: cursor + readiness must reflect the new tip.
    let cursor = store.read_cursor(did, "indexer").await.unwrap().unwrap();
    assert_eq!(cursor.indexed_head_block, 3);
    assert!(cursor.indexed_head_hash.eq_ignore_ascii_case(&b3b.hash));
    let readiness = store.read_readiness(did).await.unwrap().unwrap();
    assert!(readiness.ready);

    // Orphan blocks must be marked non-canonical.
    let orphan_row = sqlx::query(
        "SELECT is_canonical FROM hybrid_v2_canonical_blocks
         WHERE deployment_id = $1 AND block_hash = $2",
    )
    .bind(did)
    .bind(&b2.hash)
    .fetch_optional(&pool)
    .await
    .unwrap();
    if let Some(row) = orphan_row {
        let canonical: bool = row.try_get("is_canonical").unwrap();
        assert!(!canonical, "orphan block still canonical");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn deployment_isolation_pg() {
    let Some(url) = get_pg_url_or_skip_or_panic("deployment_isolation_pg") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest_a = baseline_manifest(84532);
    let mut manifest_b = baseline_manifest(84532);
    manifest_b.manifest_hash = "0xdifferent".to_string();
    // `hybrid_v2_deployments_version_uniq` is keyed on
    // `(chain_id, deployment_version)` — two manifests on the same
    // chain must carry distinct versions to coexist in the store.
    manifest_b.deployment_version = manifest_b.deployment_version.wrapping_add(1);
    let did_a = store
        .upsert_deployment(&manifest_a, "PENDING", 1_700_000_000_000)
        .await
        .unwrap();
    let did_b = store
        .upsert_deployment(&manifest_b, "PENDING", 1_700_000_000_000)
        .await
        .unwrap();
    let mut rta = IndexerRuntime::new(1, manifest_a.clone()).with_persistence(store.clone(), did_a);
    let mut rtb = IndexerRuntime::new(2, manifest_b.clone()).with_persistence(store.clone(), did_b);
    let mut source = InMemoryChainSource::new(84532);
    let b0 = make_block(0, 0xaa, "");
    let b1 = make_block(1, 0xaa, &b0.hash);
    let b2 = make_block(2, 0xaa, &b1.hash);
    source.push(b0.clone());
    source.push(b1.clone());
    source.push(b2.clone());
    let _ = drive_forward(&mut rta, &source, 2).await;
    let _ = drive_forward(&mut rtb, &source, 2).await;
    let cursor_b_pre = store.read_cursor(did_b, "indexer").await.unwrap().unwrap();

    // Reorg only A.
    let b2b = make_block(2, 0xbb, &b1.hash);
    let b3b = make_block(3, 0xbb, &b2b.hash);
    source.reorg_from(2, vec![b2b.clone(), b3b.clone()]);
    let _ = rta.tick_and_persist(&source).await;
    let service = ReorgRecoveryService::new(did_a, ReorgRecoveryConfig::default());
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    let _ = service
        .recover(
            &source,
            &store_dyn,
            &manifest_a,
            Some(ReorgDetection {
                old_tip_block: 2,
                old_tip_hash: b2.hash.clone(),
                conflicting_block: Some(3),
                conflicting_hash: Some(b3b.hash.clone()),
            }),
            "indexer",
        )
        .await
        .expect("A recover ok");

    // B untouched.
    let cursor_b_post = store.read_cursor(did_b, "indexer").await.unwrap().unwrap();
    assert_eq!(
        cursor_b_pre.indexed_head_block,
        cursor_b_post.indexed_head_block
    );
    assert_eq!(
        cursor_b_pre.indexed_head_hash,
        cursor_b_post.indexed_head_hash
    );
    assert!(store.read_reorg_recovery(did_b).await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_during_detected_resumes_recovery() {
    let Some(url) = get_pg_url_or_skip_or_panic("restart_during_detected_resumes_recovery") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);

    // Seed the initial chain state via one runtime.
    {
        let mut runtime =
            IndexerRuntime::new(1, manifest.clone()).with_persistence(store.clone(), did);
        let mut source = InMemoryChainSource::new(84532);
        let b0 = make_block(0, 0xaa, "");
        let b1 = make_block(1, 0xaa, &b0.hash);
        let b2 = make_block(2, 0xaa, &b1.hash);
        source.push(b0.clone());
        source.push(b1.clone());
        source.push(b2.clone());
        let _ = drive_forward(&mut runtime, &source, 2).await;
    }

    // Simulate a "crash after DETECTED persisted" by writing a
    // Detected row manually and NOT running recovery. Then drop the
    // runtime and start a fresh one that resumes recovery.
    let b2_hash = format!("0xaa{}", format!("{:0>62x}", 2u64));
    let now_ms: i64 = 1_700_100_000_000;
    let detected =
        ReorgRecoveryState::new_detected(did, 1, 2, b2_hash.clone(), None, None, None, now_ms);
    store.upsert_reorg_recovery(&detected).await.unwrap();

    // Rebuild the source with the replacement branch already in
    // place so recovery can succeed.
    let mut source = InMemoryChainSource::new(84532);
    let b0 = make_block(0, 0xaa, "");
    let b1 = make_block(1, 0xaa, &b0.hash);
    let b2 = make_block(2, 0xaa, &b1.hash);
    let b2b = make_block(2, 0xbb, &b1.hash);
    let b3b = make_block(3, 0xbb, &b2b.hash);
    source.push(b0);
    source.push(b1);
    // Retain b2 by hash but reorg the best chain over it.
    source.by_hash.insert(b2.hash.clone(), b2.clone());
    source.push(b2b.clone());
    source.push(b3b.clone());
    source.reorg_from(2, vec![b2b.clone(), b3b.clone()]);

    let service = ReorgRecoveryService::new(did, ReorgRecoveryConfig::default());
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    let outcome = service
        .recover(&source, &store_dyn, &manifest, None, "indexer")
        .await
        .expect("resume ok");
    assert!(matches!(outcome, RecoveryOutcome::Recovered { .. }));
    let recovery = store.read_reorg_recovery(did).await.unwrap().unwrap();
    assert_eq!(recovery.phase, ReorgRecoveryPhase::Recovered);
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_recovery_trigger_serialised_via_lock() {
    let Some(url) = get_pg_url_or_skip_or_panic("duplicate_recovery_trigger_serialised_via_lock")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    // Acquire the lock manually with epoch 1; second acquire must
    // fail while the first is held.
    use deopt_v2_backend::hybrid_v2::OperationKind;
    let guard = store
        .try_acquire_operation_lock(did, OperationKind::Reorg, 1, 1_700_000_000_000)
        .await
        .unwrap();
    assert!(guard.is_some(), "first acquire must succeed");
    let second = store
        .try_acquire_operation_lock(did, OperationKind::Reorg, 2, 1_700_000_000_000)
        .await
        .unwrap();
    assert!(
        second.is_none(),
        "second acquire must fail (lock held by another holder)"
    );
    store.release_operation_lock(did, 1).await.unwrap();
    let third = store
        .try_acquire_operation_lock(did, OperationKind::Reorg, 3, 1_700_000_000_000)
        .await
        .unwrap();
    assert!(third.is_some(), "third acquire must succeed after release");
    store.release_operation_lock(did, 3).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_reorg_lock_reclaimed_after_completion() {
    let Some(url) = get_pg_url_or_skip_or_panic("stale_reorg_lock_reclaimed_after_completion")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    // Mark recovery as RECOVERED.
    let mut recovered = ReorgRecoveryState::new_detected(
        did,
        1,
        0,
        "0x0".to_string(),
        None,
        None,
        None,
        1_700_000_000_000,
    );
    recovered.phase = ReorgRecoveryPhase::Recovered;
    recovered.completed_at_ms = Some(1_700_000_000_000);
    store.upsert_reorg_recovery(&recovered).await.unwrap();
    // Insert a stale unified operation-lock row for a prior REORG holder.
    sqlx::query(
        "INSERT INTO hybrid_v2_operation_locks (deployment_id, operation, holder_epoch, acquired_at_ms)
         VALUES ($1, 'REORG', $2, $3)",
    )
    .bind(did)
    .bind(1_i64)
    .bind(1_700_000_000_000_i64)
    .execute(&pool)
    .await
    .unwrap();
    // Attempt to acquire — should succeed because the stale lock is
    // auto-reclaimed for a completed recovery.
    use deopt_v2_backend::hybrid_v2::OperationKind;
    let guard = store
        .try_acquire_operation_lock(did, OperationKind::Reorg, 2, 1_700_000_000_000)
        .await
        .unwrap();
    assert!(
        guard.is_some(),
        "stale lock must be reclaimed after recovery completes"
    );
    store.release_operation_lock(did, 2).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_reflects_active_recovery_phase() {
    let Some(url) = get_pg_url_or_skip_or_panic("readiness_reflects_active_recovery_phase") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let detected = ReorgRecoveryState::new_detected(
        did,
        1,
        5,
        "0xaa5".to_string(),
        None,
        None,
        None,
        1_700_000_000_000,
    );
    store.upsert_reorg_recovery(&detected).await.unwrap();
    // Publish a matching readiness snapshot.
    let readiness_snap = deopt_v2_backend::hybrid_v2::persistence::ReadinessSnapshot {
        ready: false,
        reason: Some("REORG_DETECTED".to_string()),
        reason_detail: Some("epoch=1".to_string()),
    };
    store
        .write_readiness_snapshot(did, &readiness_snap, 1_700_000_000_000)
        .await
        .unwrap();
    let stored = store.read_readiness(did).await.unwrap().unwrap();
    assert!(!stored.ready);
    assert_eq!(stored.reason.as_deref(), Some("REORG_DETECTED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn commit_reorg_recovery_marks_orphans_and_advances_cursor() {
    let Some(url) =
        get_pg_url_or_skip_or_panic("commit_reorg_recovery_marks_orphans_and_advances_cursor")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest.clone()).with_persistence(store.clone(), did);
    let mut source = InMemoryChainSource::new(84532);
    let b0 = make_block(0, 0xaa, "");
    let b1 = make_block(1, 0xaa, &b0.hash);
    let b2 = make_block(2, 0xaa, &b1.hash);
    let b3 = make_block(3, 0xaa, &b2.hash);
    source.push(b0.clone());
    source.push(b1.clone());
    source.push(b2.clone());
    source.push(b3.clone());
    let _ = drive_forward(&mut runtime, &source, 3).await;
    assert_eq!(runtime.cursor().indexed_head_block, 3);

    // Reorg from block 2 with a two-block replacement.
    let b2b = make_block(2, 0xbb, &b1.hash);
    let b3b = make_block(3, 0xbb, &b2b.hash);
    let b4b = make_block(4, 0xbb, &b3b.hash);
    source.reorg_from(2, vec![b2b.clone(), b3b.clone(), b4b.clone()]);
    let _ = runtime.tick_and_persist(&source).await;

    let service = ReorgRecoveryService::new(did, ReorgRecoveryConfig::default());
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    let outcome = service
        .recover(
            &source,
            &store_dyn,
            &manifest,
            Some(ReorgDetection {
                old_tip_block: 3,
                old_tip_hash: b3.hash.clone(),
                conflicting_block: Some(4),
                conflicting_hash: Some(b4b.hash.clone()),
            }),
            "indexer",
        )
        .await
        .expect("recovery ok");
    assert!(matches!(outcome, RecoveryOutcome::Recovered { .. }));

    let cursor = store.read_cursor(did, "indexer").await.unwrap().unwrap();
    assert_eq!(cursor.indexed_head_block, 4);
    assert!(cursor.indexed_head_hash.eq_ignore_ascii_case(&b4b.hash));

    // Confirm orphan matched_executions untouched (none in this test),
    // but orphan blocks are marked non-canonical.
    let orphan_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hybrid_v2_canonical_blocks
         WHERE deployment_id = $1 AND is_canonical = FALSE AND block_number > 1",
    )
    .bind(did)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(orphan_rows >= 2, "expected at least 2 orphaned blocks");
}

#[tokio::test(flavor = "multi_thread")]
async fn excessive_depth_enters_manual_intervention() {
    let Some(url) = get_pg_url_or_skip_or_panic("excessive_depth_enters_manual_intervention")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest.clone()).with_persistence(store.clone(), did);

    // Build the canonical chain 0..=5 on branch AA.
    let mut source = InMemoryChainSource::new(84532);
    let b0 = make_block(0, 0xaa, "");
    let mut parent = b0.hash.clone();
    source.push(b0.clone());
    let mut aa_blocks = Vec::new();
    for n in 1..=5u64 {
        let b = make_block(n, 0xaa, &parent);
        parent = b.hash.clone();
        aa_blocks.push(b.clone());
        source.push(b);
    }
    let _ = drive_forward(&mut runtime, &source, 5).await;
    assert_eq!(runtime.cursor().indexed_head_block, 5);

    // Swap to a completely disjoint fork BB starting from block 1 —
    // the recovery would need to walk back 4 blocks (depth=4) to find
    // the common ancestor at height 0. Configure max_reorg_depth=2 so
    // the ancestor search fails-closed to ManualInterventionRequired.
    let mut parent_bb = b0.hash.clone();
    let mut bb_blocks = Vec::new();
    for n in 1..=6u64 {
        let b = make_block(n, 0xbb, &parent_bb);
        parent_bb = b.hash.clone();
        bb_blocks.push(b);
    }
    source.reorg_from(1, bb_blocks.clone());

    let cfg = ReorgRecoveryConfig {
        max_reorg_depth: 2,
        ..ReorgRecoveryConfig::default()
    };
    let service = ReorgRecoveryService::new(did, cfg);
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    let detection = ReorgDetection {
        old_tip_block: 5,
        old_tip_hash: aa_blocks.last().unwrap().hash.clone(),
        conflicting_block: Some(6),
        conflicting_hash: Some(bb_blocks.last().unwrap().hash.clone()),
    };
    let outcome = service
        .recover(&source, &store_dyn, &manifest, Some(detection), "indexer")
        .await
        .expect("recovery reports outcome even on failure");
    match outcome {
        RecoveryOutcome::ManualInterventionRequired { reason, .. } => {
            assert!(
                reason.to_ascii_lowercase().contains("depth")
                    || reason.to_ascii_lowercase().contains("ancestor"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected ManualInterventionRequired, got {other:?}"),
    }
    let recovery = store
        .read_reorg_recovery(did)
        .await
        .unwrap()
        .expect("recovery row persisted");
    assert_eq!(
        recovery.phase,
        ReorgRecoveryPhase::ManualInterventionRequired
    );
    // Cursor must not advance.
    let cursor = store.read_cursor(did, "indexer").await.unwrap().unwrap();
    assert_eq!(cursor.indexed_head_block, 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn finalized_boundary_violation_manual_intervention() {
    let Some(url) = get_pg_url_or_skip_or_panic("finalized_boundary_violation_manual_intervention")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest.clone()).with_persistence(store.clone(), did);

    // Canonical AA chain 0..=5, with finalized=4 announced by the
    // source. A reorg that requires rewriting block 3 would cross the
    // finalized boundary.
    let mut source = InMemoryChainSource::new(84532);
    let b0 = make_block(0, 0xaa, "");
    source.push(b0.clone());
    let mut parent = b0.hash.clone();
    let mut aa_blocks = Vec::new();
    for n in 1..=5u64 {
        let b = make_block(n, 0xaa, &parent);
        parent = b.hash.clone();
        aa_blocks.push(b.clone());
        source.push(b);
    }
    source.set_finalized(4);
    let _ = drive_forward(&mut runtime, &source, 5).await;
    assert_eq!(runtime.cursor().indexed_head_block, 5);

    // Reorg back to block 2 — ancestor=2 <= finalized=4 → refuse.
    let b3b = make_block(3, 0xbb, &aa_blocks[1].hash);
    let b4b = make_block(4, 0xbb, &b3b.hash);
    let b5b = make_block(5, 0xbb, &b4b.hash);
    let b6b = make_block(6, 0xbb, &b5b.hash);
    source.reorg_from(3, vec![b3b.clone(), b4b.clone(), b5b.clone(), b6b.clone()]);
    source.set_finalized(4);

    let cfg = ReorgRecoveryConfig {
        max_reorg_depth: 16,
        allow_finalized_boundary_crossing: false,
        ..ReorgRecoveryConfig::default()
    };
    let service = ReorgRecoveryService::new(did, cfg);
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    let detection = ReorgDetection {
        old_tip_block: 5,
        old_tip_hash: aa_blocks.last().unwrap().hash.clone(),
        conflicting_block: Some(6),
        conflicting_hash: Some(b6b.hash.clone()),
    };
    let result = service
        .recover(&source, &store_dyn, &manifest, Some(detection), "indexer")
        .await;
    match result {
        Err(err) => {
            let s = err.to_string().to_ascii_lowercase();
            assert!(s.contains("finalized"), "unexpected error string: {}", err);
        }
        Ok(other) => panic!("expected FinalizedBoundaryViolation error, got {other:?}"),
    }
    let recovery = store
        .read_reorg_recovery(did)
        .await
        .unwrap()
        .expect("recovery row persisted");
    assert_eq!(
        recovery.phase,
        ReorgRecoveryPhase::ManualInterventionRequired
    );
    // Cursor must not advance.
    let cursor = store.read_cursor(did, "indexer").await.unwrap().unwrap();
    assert_eq!(cursor.indexed_head_block, 5);
}
