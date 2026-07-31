//! Integration tests for the Hybrid V2 indexer runtime, reorg planner,
//! rebuild service, reconciliation, correlation, repository, and
//! readiness state machine.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_source::{ChainSource, InMemoryChainSource, RawBlock};
use deopt_v2_backend::hybrid_v2::chain_view::{
    ChainSnapshot, InMemoryChainViewProvider, Reconciler, ReconciliationResult,
};
use deopt_v2_backend::hybrid_v2::readiness::ReadinessReason;
use deopt_v2_backend::hybrid_v2::rebuild::RebuildService;
use deopt_v2_backend::hybrid_v2::repository::{HybridV2QueryRepository, PageCursor};
use deopt_v2_backend::hybrid_v2::runtime::{IndexerRuntime, RuntimeError};
use hybrid_v2_support::{
    baseline_manifest, block, deposit_log, matching_pair_log, order_filled_log, pad_address,
    pad_bytes32, premium_log, reservation_lock_log, reservation_unlock_log, subaccount_created_log,
    withdraw_log,
};

// -------------------- RUNTIME BASICS --------------------

#[test]
fn runtime_rejects_wrong_chain_id() {
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest);
    let source = InMemoryChainSource::new(31337);
    let err = runtime.tick(&source).unwrap_err();
    assert!(matches!(err, RuntimeError::WrongChain { .. }));
    assert!(!runtime.readiness().ready);
    assert!(matches!(
        runtime.readiness().reason.as_ref().unwrap(),
        ReadinessReason::WrongChain { .. }
    ));
}

#[test]
fn runtime_advances_and_applies_block() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source
        .push(block(
            1,
            "0xb1",
            "0xb0",
            1000,
            vec![
                subaccount_created_log(&manifest, "0xa1", 1, "0xabc1"),
                deposit_log(&manifest, "0xabc1", "0xa1", 1, "0xe1", "1000"),
            ],
        ))
        .push(block(
            2,
            "0xb2",
            "0xb1",
            1012,
            vec![withdraw_log(&manifest, "0xabc1", "0xa1", 1, "0xe1", "400")],
        ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    assert!(runtime.tick(&source).unwrap());
    assert!(runtime.tick(&source).unwrap());
    assert!(!runtime.tick(&source).unwrap()); // no more blocks
    let bal = runtime
        .projection()
        .balances
        .get(&(pad_bytes32("0xabc1"), pad_address("0xe1")))
        .unwrap();
    assert_eq!(bal, "600");
    assert_eq!(runtime.cursor().indexed_head_block, 2);
    assert!(runtime.readiness().ready);
}

#[test]
fn runtime_cursor_does_not_advance_after_failed_decode() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    // First block valid; second block contains an emitter-canonical log with
    // wrong-length data that must fail decoding and prevent cursor advance.
    let bad = withdraw_log(&manifest, "0xf1", "0xa9", 1, "0xef", "100");
    let mut broken = bad.clone();
    broken.data.truncate(3); // will fail DataTooShort
    source
        .push(block(
            1,
            "0xb1",
            "0xb0",
            1000,
            vec![subaccount_created_log(&manifest, "0xa9", 1, "0xf1")],
        ))
        .push(block(2, "0xb2", "0xb1", 1012, vec![broken]));
    let mut runtime = IndexerRuntime::new(1, manifest);
    runtime.tick(&source).unwrap();
    let err = runtime.tick(&source).unwrap_err();
    assert!(matches!(err, RuntimeError::Decoder { .. }));
    assert_eq!(runtime.cursor().indexed_head_block, 1);
    assert_eq!(runtime.metrics().decode_failures, 1);
}

#[test]
fn runtime_duplicate_block_call_is_noop() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(1, "0xb1", "0xb0", 1000, vec![]));
    let mut runtime = IndexerRuntime::new(1, manifest);
    runtime.tick(&source).unwrap();
    let head_before = runtime.cursor().indexed_head_block;
    // No new blocks — tick returns false, cursor unchanged.
    assert!(!runtime.tick(&source).unwrap());
    assert_eq!(runtime.cursor().indexed_head_block, head_before);
}

// -------------------- REORG --------------------

#[test]
fn runtime_reorg_one_block_replacement() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source
        .push(block(
            1,
            "0xb1",
            "0xb0",
            1000,
            vec![
                subaccount_created_log(&manifest, "0xa9", 1, "0xf1"),
                deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "1000"),
            ],
        ))
        .push(block(
            2,
            "0xb2a",
            "0xb1",
            1012,
            vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "500")],
        ));
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    runtime.tick(&source).unwrap();
    runtime.tick(&source).unwrap();
    let key = (pad_bytes32("0xf1"), pad_address("0xef"));
    assert_eq!(runtime.projection().balances.get(&key).unwrap(), "1500");
    // Chain reorgs from block 2 with a different block hash + different logs.
    source.reorg_from(
        2,
        vec![block(
            2,
            "0xb2b",
            "0xb1",
            1012,
            vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "200")],
        )],
    );
    // Next tick sees new block[2] whose parent still matches indexed_head (0xb1)
    // — that means block[2] must be treated as REPLACEMENT via the fact that
    // the currently-indexed block[2]=0xb2a is not the source's block[2]=0xb2b.
    // The runtime handles this by detecting cursor mismatch on parent hash for
    // any subsequent block; but here parent still matches because source at
    // block[2] is 0xb2b whose parent = 0xb1 = cursor. So instead we need to
    // append block[3] whose parent = 0xb2b:
    source.push(block(
        3,
        "0xb3",
        "0xb2b",
        1024,
        vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "77")],
    ));
    while runtime.tick(&source).unwrap() {}
    // After reorg replay: balance = 1000 (block1) + 200 (new block2) + 77 (block3) = 1277
    let key = (pad_bytes32("0xf1"), pad_address("0xef"));
    assert_eq!(runtime.projection().balances.get(&key).unwrap(), "1277");
    assert!(runtime.metrics().reorg_count >= 1);
}

#[test]
fn runtime_multi_block_reorg() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source
        .push(block(
            1,
            "0xb1",
            "0xb0",
            1000,
            vec![
                subaccount_created_log(&manifest, "0xa9", 1, "0xf1"),
                deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "100"),
            ],
        ))
        .push(block(
            2,
            "0xb2a",
            "0xb1",
            1012,
            vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "50")],
        ))
        .push(block(
            3,
            "0xb3a",
            "0xb2a",
            1024,
            vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "30")],
        ));
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    for _ in 0..3 {
        runtime.tick(&source).unwrap();
    }
    let key = (pad_bytes32("0xf1"), pad_address("0xef"));
    assert_eq!(runtime.projection().balances.get(&key).unwrap(), "180");
    // Multi-block reorg: replace 2..=3 with 2..=4 on new chain.
    source.reorg_from(
        2,
        vec![
            block(
                2,
                "0xb2b",
                "0xb1",
                1012,
                vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "10")],
            ),
            block(
                3,
                "0xb3b",
                "0xb2b",
                1024,
                vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "10")],
            ),
            block(
                4,
                "0xb4b",
                "0xb3b",
                1036,
                vec![deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "10")],
            ),
        ],
    );
    // Advance again: runtime should detect divergence and replay.
    while runtime.tick(&source).unwrap() {}
    let key = (pad_bytes32("0xf1"), pad_address("0xef"));
    assert_eq!(runtime.projection().balances.get(&key).unwrap(), "130");
    assert!(runtime.metrics().reorg_count >= 1);
    assert!(runtime.metrics().max_reorg_depth_seen >= 2);
}

// -------------------- REBUILD --------------------

#[test]
fn rebuild_matches_incremental_projection() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source
        .push(block(
            1,
            "0xb1",
            "0xb0",
            1000,
            vec![
                subaccount_created_log(&manifest, "0xa4", 1, "0xff01"),
                subaccount_created_log(&manifest, "0xo2", 1, "0xsk2"),
                deposit_log(&manifest, "0xff01", "0xa4", 1, "0xef", "500"),
                reservation_lock_log(&manifest, "0xff01", "0xef", "0xef", "100"),
            ],
        ))
        .push(block(
            2,
            "0xb2",
            "0xb1",
            1012,
            vec![
                deposit_log(&manifest, "0xsk2", "0xo2", 1, "0xef", "300"),
                reservation_unlock_log(&manifest, "0xff01", "0xef", "0xef", "50"),
            ],
        ));
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    while runtime.tick(&source).unwrap() {}
    let incremental_state = runtime.projection().clone();
    let rebuild = RebuildService::new(manifest.event_version);
    let (rebuilt_state, _, outcome) = rebuild.replay_all(&runtime.raw_logs);
    assert_eq!(outcome.events_replayed as usize, runtime.raw_logs.len());
    assert_eq!(incremental_state.balances, rebuilt_state.balances);
    assert_eq!(incremental_state.reservations, rebuilt_state.reservations);
    assert_eq!(incremental_state.subaccounts, rebuilt_state.subaccounts);
}

// -------------------- CORRELATION --------------------

#[test]
fn execution_correlation_marks_complete_and_incomplete() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    // Complete group.
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xa2", 1, "0xdd01"),
            subaccount_created_log(&manifest, "0xa3", 1, "0xdd02"),
            matching_pair_log(
                &manifest, "0xcc01", "0xbb01", "0xbb02", "0xdd01", "0xdd02", "10", "20",
            ),
            order_filled_log(&manifest, "0xdd01", "0xbb01", "10", "10", true),
            order_filled_log(&manifest, "0xdd02", "0xbb02", "10", "10", true),
            premium_log(&manifest, "0xdd01", "0xdd02", "0xef", "20"),
        ],
    ));
    // Incomplete group (missing seller fill).
    source.push(block(
        2,
        "0xb2",
        "0xb1",
        1012,
        vec![
            matching_pair_log(
                &manifest, "0xcc02", "0xbb03", "0xbb04", "0xdd01", "0xdd02", "5", "5",
            ),
            order_filled_log(&manifest, "0xdd01", "0xbb03", "5", "5", true),
            premium_log(&manifest, "0xdd01", "0xdd02", "0xef", "5"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    while runtime.tick(&source).unwrap() {}
    let m = &runtime.projection().matched_executions;
    assert_eq!(
        m.get(&pad_bytes32("0xcc01")).unwrap().completion_status,
        deopt_v2_backend::hybrid_v2::reducer::ExecutionCompletion::Complete
    );
    assert_eq!(
        m.get(&pad_bytes32("0xcc02")).unwrap().completion_status,
        deopt_v2_backend::hybrid_v2::reducer::ExecutionCompletion::Incomplete
    );
}

// -------------------- RECONCILIATION --------------------

#[test]
fn reconciliation_convergence_and_drift() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xa9", 1, "0xf1"),
            deposit_log(&manifest, "0xf1", "0xa9", 1, "0xef", "100"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    runtime.tick(&source).unwrap();
    let reconciler = Reconciler::new();

    // Converged provider.
    let mut provider = InMemoryChainViewProvider::new();
    let padded_sk = pad_bytes32("0xf1");
    let padded_owner = pad_address("0xa9");
    let padded_token = pad_address("0xef");
    let snap = ChainSnapshot {
        manifest_hash: manifest.manifest_hash.clone(),
        subaccount_owners: [(padded_sk.clone(), padded_owner.clone())]
            .into_iter()
            .collect(),
        balances: [((padded_sk.clone(), padded_token.clone()), "100".to_string())]
            .into_iter()
            .collect(),
        reservations: Default::default(),
        recovery_state: Default::default(),
    };
    provider.set_snapshot(1, snap);
    let result = reconciler.reconcile(
        &manifest.manifest_hash,
        runtime.cursor().indexed_head_block,
        runtime.projection(),
        &provider,
    );
    assert!(matches!(result, ReconciliationResult::Converged { .. }));

    // Drift provider.
    let mut drift = InMemoryChainViewProvider::new();
    let mut drift_snap = ChainSnapshot::default();
    drift_snap.manifest_hash = manifest.manifest_hash.clone();
    drift_snap
        .balances
        .insert((padded_sk.clone(), padded_token.clone()), "42".to_string());
    drift.set_snapshot(1, drift_snap);
    let result = reconciler.reconcile(
        &manifest.manifest_hash,
        runtime.cursor().indexed_head_block,
        runtime.projection(),
        &drift,
    );
    assert!(matches!(
        result,
        ReconciliationResult::ProjectionDrift { .. }
    ));

    // Provider unavailable.
    let mut down = InMemoryChainViewProvider::new();
    down.set_available(false);
    assert!(matches!(
        reconciler.reconcile(&manifest.manifest_hash, 1, runtime.projection(), &down),
        ReconciliationResult::ProviderUnavailable
    ));

    // Manifest mismatch.
    let mut wrong = InMemoryChainViewProvider::new();
    let mut wrong_snap = ChainSnapshot::default();
    wrong_snap.manifest_hash = "0xdead".into();
    wrong.set_snapshot(1, wrong_snap);
    assert!(matches!(
        reconciler.reconcile(&manifest.manifest_hash, 1, runtime.projection(), &wrong),
        ReconciliationResult::ManifestMismatch { .. }
    ));

    // Indexer behind (provider head > indexed).
    let mut ahead = InMemoryChainViewProvider::new();
    let mut ahead_snap = ChainSnapshot::default();
    ahead_snap.manifest_hash = manifest.manifest_hash.clone();
    ahead.set_snapshot(5, ahead_snap);
    assert!(matches!(
        reconciler.reconcile(&manifest.manifest_hash, 1, runtime.projection(), &ahead),
        ReconciliationResult::IndexerBehind { .. }
    ));
}

// -------------------- REPOSITORY --------------------

#[test]
fn repository_query_methods_return_expected_shapes() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xa1", 1, "0xff01"),
            deposit_log(
                &manifest,
                "0xff01",
                "0xa1",
                1,
                "0xef",
                "9999999999999999999999999999999999999999999999999999999999999999999999999",
            ),
            reservation_lock_log(&manifest, "0xff01", "0xef", "0xef", "42"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    runtime.tick(&source).unwrap();
    let repo = HybridV2QueryRepository::new(
        1,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        runtime
            .readiness()
            .reason
            .clone()
            .map(|r| format!("{:?}", r)),
    );
    let owner_key = pad_address("0xa1");
    let sk_key = pad_bytes32("0xff01");
    let subs = repo.subaccounts_by_owner(&owner_key);
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].subkey, sk_key);
    let details = repo.subaccount_details(&sk_key).unwrap();
    assert_eq!(details.owner, owner_key);
    let balances = repo.collateral_balances(&sk_key);
    assert_eq!(balances[0].token, pad_address("0xef"));
    // Uint256 preserved exactly.
    assert!(balances[0].balance.len() > 30);
    let reservations = repo.reservations(&sk_key);
    assert_eq!(reservations[0].reserved, "42");
    let page = PageCursor::new(50);
    let execs = repo.matched_executions(&page);
    assert!(execs.is_empty());
    let status = repo.deployment_status();
    assert_eq!(status.indexed_block, 1);
}

// -------------------- READINESS --------------------

#[test]
fn readiness_flips_to_ready_after_first_block() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(1, "0xb1", "0xb0", 1000, vec![]));
    let mut runtime = IndexerRuntime::new(1, manifest);
    assert!(!runtime.readiness().ready);
    runtime.tick(&source).unwrap();
    assert!(runtime.readiness().ready);
}
