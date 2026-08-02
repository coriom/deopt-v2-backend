//! `BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1` — in-memory
//! convergence + property tests for the projection store contract.
//!
//! These tests exercise the same `HybridV2ProjectionStore` trait used
//! by the PostgreSQL implementation but through an in-memory fake, so
//! CI runs remain green without a live database. The PG-backed proof
//! lives in `hybrid_v2_persistence_pg_proof.rs`, gated on
//! `HYBRID_V2_PG_TEST_DATABASE_URL`.
//!
//! Convergence contract:
//! - After applying a bounded canonical event sequence, the in-memory
//!   reducer state must equal the state persisted by the projection
//!   store (which internally accepts the full ProjectionState snapshot).
//! - The block-atomic writer accepts (block, raw_logs, decoded_events,
//!   state, cursor, readiness) as one unit — no partial commit.
//! - Idempotent re-application of the same block yields the same state.
//! - Different deployments produce isolated snapshots.

use deopt_v2_backend::hybrid_v2::events::{EventKind, HybridV2Event};
use deopt_v2_backend::hybrid_v2::manifest::{
    ActivationStatus, ManifestModuleAddresses, ManifestParams,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    CanonicalBlockRef, HybridV2ProjectionStore, InMemoryProjectionStore, ReadinessSnapshot,
    RuntimeCursorSnapshot,
};
use deopt_v2_backend::hybrid_v2::reducer::{apply, ApplyContext, ProjectionState};
use deopt_v2_backend::hybrid_v2::runtime::JournaledLog;
use serde_json::json;

fn now_ms() -> i64 {
    1_700_000_000_000
}

fn base_manifest(chain_id: u64, hash_tag: &str) -> ManifestParams {
    ManifestParams {
        chain_id,
        manifest_address: "0xmanifest".into(),
        manifest_hash: format!("0x{hash_tag}"),
        module_addresses_hash: "0x".into(),
        critical_config_hash: "0x".into(),
        architecture_version: 1,
        storage_version: 1,
        event_version: 1,
        deployment_version: 1,
        manifest_schema_version: 1,
        environment_tag: "TESTNET".into(),
        deployer: "0xdep".into(),
        deployment_block: 1,
        deployment_timestamp: 1_700_000_000,
        module_addresses: ManifestModuleAddresses {
            subaccount_registry: "0xreg".into(),
            collateral_vault: "0xvlt".into(),
            options_positions_ledger: "0xopl".into(),
            risk_module: "0xrsk".into(),
            margin_engine: "0xmg".into(),
            option_matching_engine: "0xmm".into(),
            escape_controller: "0xesc".into(),
            recovery_finalizer: "0xrec".into(),
            oracle_adapter: "0xoc".into(),
            options_risk_provider: "0xorp".into(),
            quote_token: "0xqt".into(),
            fees_manager_v2: None,
            option_execution_fee_adapter: None,
            protocol_timelock: None,
            governance: None,
            guardian: None,
        },
        protocol_fee_subkey: "0xpf".into(),
        rebate_budget_subkey: "0xrb".into(),
        insurance_fund_subkey: "0xif".into(),
        max_collateral_tokens: 8,
        max_active_series: 32,
        all_capabilities_mask: "0".into(),
        recovery_activation_delay_seconds: 3600,
        recovery_pause_max_duration_blocks: 100,
        activation_status: ActivationStatus::Pending,
    }
}

fn block_ref(n: u64) -> CanonicalBlockRef {
    CanonicalBlockRef {
        block_number: n,
        block_hash: format!("0xblk{n}"),
        parent_hash: if n == 1 {
            "0x0".into()
        } else {
            format!("0xblk{}", n - 1)
        },
        block_timestamp: 1_700_000_000 + n * 12,
    }
}

fn journaled_log(n: u64, log_index: u32) -> JournaledLog {
    JournaledLog {
        block_number: n,
        block_hash: format!("0xblk{n}"),
        parent_hash: if n == 1 {
            "0x0".into()
        } else {
            format!("0xblk{}", n - 1)
        },
        block_timestamp: 1_700_000_000 + n * 12,
        tx_hash: format!("0xtx{n}"),
        tx_index: 0,
        log_index,
        emitter: "0xreg".into(),
        topics: vec![[0u8; 32]],
        data: vec![0; 32],
        is_canonical: true,
        orphaned_at_block: None,
    }
}

fn ctx(n: u64, log_index: u32) -> ApplyContext {
    ApplyContext {
        block_number: n,
        tx_hash: format!("0xtx{n}"),
        log_index,
        block_timestamp: 1_700_000_000 + n * 12,
    }
}

fn deposit_event(subkey: &str, token: &str, amount: &str) -> HybridV2Event {
    HybridV2Event {
        kind: EventKind::Deposit,
        event_version: 1,
        subkey: Some(subkey.into()),
        owner: None,
        subaccount_id: None,
        token: Some(token.into()),
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": amount }),
    }
}

fn cursor_at(name: &str, block: u64, hash: &str, parent: &str) -> RuntimeCursorSnapshot {
    RuntimeCursorSnapshot {
        cursor_name: name.into(),
        indexed_head_block: block,
        indexed_head_hash: hash.into(),
        indexed_head_parent: parent.into(),
        observed_head_block: block,
        finalized_head_block: 0,
        last_error: None,
        reorg_count: 0,
        max_reorg_depth_seen: 0,
        decode_failures: 0,
        projection_failures: 0,
        unknown_canonical_events: 0,
        last_success_block: block,
    }
}

fn ready_snap() -> ReadinessSnapshot {
    ReadinessSnapshot {
        ready: true,
        reason: None,
        reason_detail: None,
    }
}

#[tokio::test]
async fn upsert_deployment_is_idempotent_and_isolates_by_hash() {
    let store = InMemoryProjectionStore::new();
    let m1 = base_manifest(84532, "aa");
    let m2 = base_manifest(84532, "bb");
    let id1a = store
        .upsert_deployment(&m1, "PENDING", now_ms())
        .await
        .unwrap();
    let id1b = store
        .upsert_deployment(&m1, "PENDING", now_ms())
        .await
        .unwrap();
    let id2 = store
        .upsert_deployment(&m2, "PENDING", now_ms())
        .await
        .unwrap();
    assert_eq!(id1a, id1b, "same manifest → same deployment id");
    assert_ne!(
        id1a, id2,
        "different manifest_hash → different deployment id"
    );
}

#[tokio::test]
async fn block_atomic_write_snapshots_full_state() {
    let store = InMemoryProjectionStore::new();
    let m = base_manifest(84532, "cc");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state = ProjectionState::default();
    let e = deposit_event("0xa1", "0xtoken", "5000");
    let c = ctx(1, 0);
    apply(&mut state, &e, &c).unwrap();

    let raw = vec![journaled_log(1, 0)];
    let decoded = vec![(e, c)];
    let cursor = cursor_at("indexer", 1, "0xblk1", "0x0");
    store
        .persist_block_atomic(
            id,
            &block_ref(1),
            &raw,
            &decoded,
            &state,
            &cursor,
            &ready_snap(),
            now_ms(),
        )
        .await
        .unwrap();

    let snap = store.snapshot_state(id).expect("state persisted");
    assert_eq!(snap.balances.len(), 1);
    let bal = snap
        .balances
        .get(&("0xa1".to_string(), "0xtoken".to_string()));
    assert_eq!(bal.map(String::as_str), Some("5000"));
    assert_eq!(store.raw_log_count(id), 1);
    assert_eq!(store.block_count(id), 1);
}

#[tokio::test]
async fn cursor_persists_and_reads_back() {
    let store = InMemoryProjectionStore::new();
    let m = base_manifest(84532, "dd");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let state = ProjectionState::default();
    let cursor = cursor_at("indexer", 42, "0xhead", "0xparent");
    store
        .persist_block_atomic(
            id,
            &block_ref(42),
            &[],
            &[],
            &state,
            &cursor,
            &ready_snap(),
            now_ms(),
        )
        .await
        .unwrap();
    let read = store
        .read_cursor(id, "indexer")
        .await
        .unwrap()
        .expect("cursor persisted");
    assert_eq!(read.indexed_head_block, 42);
    assert_eq!(read.indexed_head_hash, "0xhead");
    assert_eq!(read.last_success_block, 42);
}

#[tokio::test]
async fn readiness_persists_and_reads_back() {
    let store = InMemoryProjectionStore::new();
    let m = base_manifest(84532, "ee");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();
    let snap = ReadinessSnapshot {
        ready: false,
        reason: Some("DECODE_FAILURE".into()),
        reason_detail: Some("bad topic0".into()),
    };
    let cursor = cursor_at("indexer", 1, "0xh", "0x0");
    store
        .persist_block_atomic(
            id,
            &block_ref(1),
            &[],
            &[],
            &ProjectionState::default(),
            &cursor,
            &snap,
            now_ms(),
        )
        .await
        .unwrap();
    let read = store
        .read_readiness(id)
        .await
        .unwrap()
        .expect("readiness persisted");
    assert_eq!(read.ready, false);
    assert_eq!(read.reason.as_deref(), Some("DECODE_FAILURE"));
}

#[tokio::test]
async fn two_deployments_snapshots_are_isolated() {
    let store = InMemoryProjectionStore::new();
    let m1 = base_manifest(84532, "ff");
    let m2 = base_manifest(84532, "0f");
    let id1 = store
        .upsert_deployment(&m1, "PENDING", now_ms())
        .await
        .unwrap();
    let id2 = store
        .upsert_deployment(&m2, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state1 = ProjectionState::default();
    let e1 = deposit_event("0xa1", "0xt", "100");
    apply(&mut state1, &e1, &ctx(1, 0)).unwrap();

    let mut state2 = ProjectionState::default();
    let e2 = deposit_event("0xb2", "0xt", "999");
    apply(&mut state2, &e2, &ctx(1, 0)).unwrap();

    store
        .persist_block_atomic(
            id1,
            &block_ref(1),
            &[journaled_log(1, 0)],
            &[(e1, ctx(1, 0))],
            &state1,
            &cursor_at("indexer", 1, "0xblk1", "0x0"),
            &ready_snap(),
            now_ms(),
        )
        .await
        .unwrap();
    store
        .persist_block_atomic(
            id2,
            &block_ref(1),
            &[journaled_log(1, 0)],
            &[(e2, ctx(1, 0))],
            &state2,
            &cursor_at("indexer", 1, "0xblk1", "0x0"),
            &ready_snap(),
            now_ms(),
        )
        .await
        .unwrap();

    let s1 = store.snapshot_state(id1).unwrap();
    let s2 = store.snapshot_state(id2).unwrap();
    assert!(s1.balances.contains_key(&("0xa1".into(), "0xt".into())));
    assert!(!s1.balances.contains_key(&("0xb2".into(), "0xt".into())));
    assert!(s2.balances.contains_key(&("0xb2".into(), "0xt".into())));
    assert!(!s2.balances.contains_key(&("0xa1".into(), "0xt".into())));
}

#[tokio::test]
async fn idempotent_reapplication_yields_same_state() {
    let store = InMemoryProjectionStore::new();
    let m = base_manifest(84532, "10");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state = ProjectionState::default();
    let e = deposit_event("0xa1", "0xt", "42");
    apply(&mut state, &e, &ctx(1, 0)).unwrap();
    let cursor = cursor_at("indexer", 1, "0xblk1", "0x0");
    let raw = vec![journaled_log(1, 0)];
    let decoded = vec![(e, ctx(1, 0))];

    for _ in 0..3 {
        store
            .persist_block_atomic(
                id,
                &block_ref(1),
                &raw,
                &decoded,
                &state,
                &cursor,
                &ready_snap(),
                now_ms(),
            )
            .await
            .unwrap();
    }
    let snap = store.snapshot_state(id).unwrap();
    assert_eq!(snap.balances.len(), 1);
    assert_eq!(
        snap.balances
            .get(&("0xa1".to_string(), "0xt".to_string()))
            .map(String::as_str),
        Some("42")
    );
}

#[tokio::test]
async fn record_reorg_event_captured() {
    let store = InMemoryProjectionStore::new();
    let m = base_manifest(84532, "11");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();
    store
        .record_reorg_event(id, now_ms(), 10, "0xold", 10, "0xnew", 3, 5)
        .await
        .unwrap();
    let events = store.reorg_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["depth"], 3);
    assert_eq!(events[0]["orphaned_log_count"], 5);
    assert_eq!(events[0]["from_hash"].as_str(), Some("0xold"));
    assert_eq!(events[0]["to_hash"].as_str(), Some("0xnew"));
}

#[tokio::test]
async fn full_state_snapshot_roundtrips_all_field_categories() {
    // Populate every ProjectionState field category and confirm the
    // snapshot round-trips through the store.
    use deopt_v2_backend::hybrid_v2::reducer::{
        EscapeStateRow, FeeEventRow, MatchedExecutionRow, OrderLifecycleRow, PositionRow,
        RecoveryStateProjection,
    };

    let store = InMemoryProjectionStore::new();
    let m = base_manifest(84532, "12");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state = ProjectionState::default();
    // Identity
    state.subaccounts.insert(("0xown".into(), 1), "0xa1".into());
    // Vault
    state
        .balances
        .insert(("0xa1".into(), "0xt".into()), "10000".into());
    state
        .reservations
        .insert(("0xa1".into(), "0xt".into(), "0xeng".into()), "500".into());
    state
        .bad_debt
        .insert(("0xa1".into(), "0xt".into()), "3".into());
    state.capability_grants.insert("0xeng".into(), "1".into());
    state.collateral_universe.insert("0xt".into(), 0);
    state.pause_flags.insert("0xa1".into(), false);
    // Positions
    state.positions.insert(
        ("0xa1".into(), "0xseries".into()),
        PositionRow {
            long_qty_1e8: "1000".into(),
            short_qty_1e8: "0".into(),
            last_event_block: 1,
        },
    );
    // Orders
    state.order_lifecycle.insert(
        "0xorder".into(),
        OrderLifecycleRow {
            subkey: "0xa1".into(),
            owner: "0xown".into(),
            series_id: Some("0xseries".into()),
            side: 0,
            time_in_force: 0,
            total_qty_1e8: "1000".into(),
            filled_qty_1e8: "500".into(),
            cancelled: false,
            terminal: false,
            first_seen_block: 1,
            last_event_block: 2,
        },
    );
    state.min_valid_nonce.insert("0xa1".into(), "1".into());
    // Executions
    state.matched_executions.insert(
        "0xexec".into(),
        MatchedExecutionRow {
            buyer_order_hash: "0xbuy".into(),
            seller_order_hash: "0xsell".into(),
            buyer_subkey: "0xa1".into(),
            seller_subkey: "0xb2".into(),
            series_id: "0xseries".into(),
            matched_qty_1e8: "500".into(),
            premium_amount: "1000".into(),
            fee_amount: "10".into(),
            rebate_amount: "5".into(),
            block_number: 2,
            tx_hash: "0xtx2".into(),
            completion_status: deopt_v2_backend::hybrid_v2::reducer::ExecutionCompletion::Complete,
        },
    );
    // Fees
    state.fee_events.push(FeeEventRow {
        kind: "OPTION_FEE_CHARGED",
        payer_subkey: Some("0xa1".into()),
        receiver_subkey: Some("0xpf".into()),
        token: "0xt".into(),
        amount: "10".into(),
        block_number: 2,
        tx_hash: "0xtx2".into(),
        log_index: 0,
        execution_id: Some("0xexec".into()),
    });
    // Recovery
    state
        .recovery_state
        .insert("0xa1".into(), RecoveryStateProjection::Normal);
    state.escape_state.insert(
        "0xa1".into(),
        EscapeStateRow {
            state: "NORMAL",
            requested_ts: None,
            activation_eligible_at: None,
            activated_ts: None,
            cancelled_ts: None,
            finalized_ts: None,
            last_event_block: 1,
        },
    );

    let cursor = cursor_at("indexer", 2, "0xblk2", "0xblk1");
    store
        .persist_block_atomic(
            id,
            &block_ref(2),
            &[],
            &[],
            &state,
            &cursor,
            &ready_snap(),
            now_ms(),
        )
        .await
        .unwrap();

    let snap = store.snapshot_state(id).unwrap();
    assert_eq!(snap.balances, state.balances);
    assert_eq!(snap.reservations, state.reservations);
    assert_eq!(snap.bad_debt, state.bad_debt);
    assert_eq!(snap.capability_grants, state.capability_grants);
    assert_eq!(snap.collateral_universe, state.collateral_universe);
    assert_eq!(snap.pause_flags, state.pause_flags);
    assert_eq!(snap.positions, state.positions);
    assert_eq!(snap.order_lifecycle, state.order_lifecycle);
    assert_eq!(snap.min_valid_nonce, state.min_valid_nonce);
    assert_eq!(snap.matched_executions, state.matched_executions);
    assert_eq!(snap.fee_events, state.fee_events);
    assert_eq!(snap.recovery_state, state.recovery_state);
    assert_eq!(snap.escape_state, state.escape_state);
    assert_eq!(snap.subaccounts, state.subaccounts);
}

#[tokio::test]
async fn cursor_advance_updates_counters_monotonically() {
    let store = InMemoryProjectionStore::new();
    let m = base_manifest(84532, "13");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();
    for n in 1..=5 {
        let mut cursor = cursor_at(
            "indexer",
            n,
            &format!("0xblk{n}"),
            &format!("0xblk{}", n.saturating_sub(1)),
        );
        cursor.last_success_block = n;
        cursor.reorg_count = if n == 3 { 1 } else { 0 };
        store
            .persist_block_atomic(
                id,
                &block_ref(n),
                &[],
                &[],
                &ProjectionState::default(),
                &cursor,
                &ready_snap(),
                now_ms(),
            )
            .await
            .unwrap();
    }
    let read = store.read_cursor(id, "indexer").await.unwrap().unwrap();
    assert_eq!(read.indexed_head_block, 5);
    assert_eq!(read.last_success_block, 5);
}
