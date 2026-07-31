//! Property-style tests for the Hybrid V2 reducer + runtime.
//!
//! These are bounded, seeded, deterministic — no external randomness dep.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_source::InMemoryChainSource;
use deopt_v2_backend::hybrid_v2::rebuild::RebuildService;
use deopt_v2_backend::hybrid_v2::runtime::IndexerRuntime;
use hybrid_v2_support::{
    baseline_manifest, block, deposit_log, reservation_lock_log, reservation_unlock_log,
    subaccount_created_log, withdraw_log,
};

/// Duplicate application (via re-tick over the same block chain) is
/// idempotent because the runtime cursor rejects already-indexed blocks.
#[test]
fn duplicate_tick_over_same_source_is_idempotent() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xaa", 1, "0xbb"),
            deposit_log(&manifest, "0xbb", "0xaa", 1, "0xcc", "100"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    while runtime.tick(&source).unwrap() {}
    let first_state = runtime.projection().clone();
    // Ticking again with no new blocks changes nothing.
    for _ in 0..5 {
        runtime.tick(&source).unwrap();
    }
    assert_eq!(&first_state, runtime.projection());
}

/// Full rebuild from the canonical raw log journal equals the incremental
/// state exactly.
#[test]
fn rebuild_equals_incremental() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    for i in 1u64..=10 {
        source.push(block(
            i,
            &format!("0xb{:x}", i),
            &format!("0xb{:x}", i - 1),
            1000 + i * 12,
            vec![
                subaccount_created_log(&manifest, "0xa1", i as u32, &format!("0xf{:x}", i)),
                deposit_log(
                    &manifest,
                    &format!("0xf{:x}", i),
                    "0xa1",
                    i as u32,
                    "0xef",
                    "100",
                ),
            ],
        ));
    }
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    while runtime.tick(&source).unwrap() {}
    let incr = runtime.projection().clone();
    let rebuilt = RebuildService::new(manifest.event_version)
        .replay_all(&runtime.raw_logs)
        .0;
    assert_eq!(incr.balances, rebuilt.balances);
    assert_eq!(incr.reservations, rebuilt.reservations);
    assert_eq!(incr.subaccounts, rebuilt.subaccounts);
}

/// Balances never go negative — every withdraw is bounded by prior deposits.
#[test]
fn balance_never_negative_under_bounded_ops() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xaa", 1, "0xbb"),
            deposit_log(&manifest, "0xbb", "0xaa", 1, "0xcc", "1000"),
            withdraw_log(&manifest, "0xbb", "0xaa", 1, "0xcc", "1000"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    runtime.tick(&source).unwrap();
    for (_, bal) in runtime.projection().balances.iter() {
        assert!(!bal.starts_with('-'));
    }
}

/// Reservations never go negative under bounded lock/unlock sequences.
#[test]
fn reservation_never_negative() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xaa", 1, "0xbb"),
            reservation_lock_log(&manifest, "0xbb", "0xcc", "0xef", "500"),
            reservation_unlock_log(&manifest, "0xbb", "0xcc", "0xef", "500"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    runtime.tick(&source).unwrap();
    for (_, r) in runtime.projection().reservations.iter() {
        assert!(!r.starts_with('-'));
    }
}

/// Two deployments never contaminate each other's projection state.
#[test]
fn deployments_are_isolated() {
    let manifest_a = baseline_manifest(84532);
    let mut manifest_b = baseline_manifest(84532);
    manifest_b.deployment_version = 2;
    manifest_b.manifest_hash = "0xdead".repeat(16); // 64 chars

    let mut source_a = InMemoryChainSource::new(84532);
    let mut source_b = InMemoryChainSource::new(84532);
    source_a.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest_a, "0xaa", 1, "0xbb"),
            deposit_log(&manifest_a, "0xbb", "0xaa", 1, "0xcc", "100"),
        ],
    ));
    source_b.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest_b, "0xaa", 1, "0xbb"),
            deposit_log(&manifest_b, "0xbb", "0xaa", 1, "0xcc", "999"),
        ],
    ));

    let mut ra = IndexerRuntime::new(1, manifest_a);
    let mut rb = IndexerRuntime::new(2, manifest_b);
    ra.tick(&source_a).unwrap();
    rb.tick(&source_b).unwrap();
    let bal_a = ra.projection().balances.values().next().cloned().unwrap();
    let bal_b = rb.projection().balances.values().next().cloned().unwrap();
    assert_eq!(bal_a, "100");
    assert_eq!(bal_b, "999");
    // Modifying one runtime does not touch the other.
    assert_ne!(ra.projection(), rb.projection());
}
