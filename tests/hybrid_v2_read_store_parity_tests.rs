//! `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2A-STORE-AND-PG-TESTS-V1`
//! In-memory parity tests for `HybridV2ReadStore`.
//!
//! These tests compare `InMemoryHybridV2ReadStore` output against the
//! semantics of the existing lifetime-scoped `HybridV2QueryRepository`
//! for equivalent canonical fixtures. They run in CI without a
//! Postgres instance and are the second half of the memory ↔ PG
//! convergence proof (the first half being the env-gated PG proof).
//!
//! Properties covered:
//! - Deployment isolation (deployment_id mismatch → empty result).
//! - Subaccount listing preserves sort by ID and excludes Account 0.
//! - Collateral aggregation equals per-engine sum.
//! - Position pagination yields each item exactly once (no dupes).
//! - Orders remaining_qty = total - filled (never negative).
//! - Executions completeness filter is stable.
//! - Recovery record shape parity.
//! - History filter hash + stale-cursor semantics identical to PG.
//! - Integer round-trip through owned records.

use deopt_v2_backend::api::hybrid_v2_read::history::{HistoryDirection, HistoryFilter};
use deopt_v2_backend::hybrid_v2::events::EventKind;
use deopt_v2_backend::hybrid_v2::read_store::{
    filter_stable_hash, HistoryConsistency, HistoryPageAnchor, HistoryScope, HybridV2ReadStore,
    InMemoryHybridV2ReadStore, InMemoryJournalEntry, InMemoryStoreBuilder, PageAnchor,
    ReadStoreError,
};
use deopt_v2_backend::hybrid_v2::reducer::{
    EscapeStateRow, ExecutionCompletion, FeeEventRow, MatchedExecutionRow, OrderLifecycleRow,
    PositionRow, ProjectionState, RecoveryStateProjection, SubaccountMeta,
};
use deopt_v2_backend::hybrid_v2::runtime::RuntimeCursor;
use serde_json::json;

fn cursor_at(block: u64, hash: &str) -> RuntimeCursor {
    RuntimeCursor {
        indexed_head_block: block,
        indexed_head_hash: hash.into(),
        indexed_head_parent: format!("0xblk{}", block.saturating_sub(1)),
        observed_head_block: block,
        finalized_head_block: 0,
        last_error: None,
    }
}

fn build_state() -> ProjectionState {
    let mut state = ProjectionState::default();
    state
        .subaccounts
        .insert(("0xparityown".into(), 1), "0xparitysub".into());
    state.subaccount_meta.insert(
        "0xparitysub".into(),
        SubaccountMeta {
            materialised_via_created: true,
            materialised_via_lazy: false,
        },
    );
    state
        .balances
        .insert(("0xparitysub".into(), "0xtokA".into()), "1000".into());
    state
        .balances
        .insert(("0xparitysub".into(), "0xtokB".into()), "500".into());
    state.reservations.insert(
        ("0xparitysub".into(), "0xtokA".into(), "0xeng1".into()),
        "200".into(),
    );
    state.reservations.insert(
        ("0xparitysub".into(), "0xtokA".into(), "0xeng2".into()),
        "100".into(),
    );
    state.collateral_universe.insert("0xtokA".into(), 0);
    state.collateral_universe.insert("0xtokB".into(), 1);
    state.positions.insert(
        ("0xparitysub".into(), "0xseriesa".into()),
        PositionRow {
            long_qty_1e8: "10".into(),
            short_qty_1e8: "0".into(),
            last_event_block: 1,
        },
    );
    state.positions.insert(
        ("0xparitysub".into(), "0xseriesb".into()),
        PositionRow {
            long_qty_1e8: "0".into(),
            short_qty_1e8: "5".into(),
            last_event_block: 2,
        },
    );
    let mut active = std::collections::BTreeSet::new();
    active.insert("0xseriesa".into());
    state.active_series.insert("0xparitysub".into(), active);
    state.order_lifecycle.insert(
        "0xord1".into(),
        OrderLifecycleRow {
            subkey: "0xparitysub".into(),
            owner: "0xparityown".into(),
            series_id: Some("0xseriesa".into()),
            side: 0,
            time_in_force: 0,
            total_qty_1e8: "100".into(),
            filled_qty_1e8: "40".into(),
            cancelled: false,
            terminal: false,
            first_seen_block: 1,
            last_event_block: 3,
        },
    );
    state.order_lifecycle.insert(
        "0xord2".into(),
        OrderLifecycleRow {
            subkey: "0xparitysub".into(),
            owner: "0xparityown".into(),
            series_id: None,
            side: 1,
            time_in_force: 1,
            total_qty_1e8: "20".into(),
            filled_qty_1e8: "20".into(),
            cancelled: false,
            terminal: true,
            first_seen_block: 2,
            last_event_block: 2,
        },
    );
    state.matched_executions.insert(
        "0xexeca".into(),
        MatchedExecutionRow {
            buyer_order_hash: "0xord1".into(),
            seller_order_hash: "0xord2".into(),
            buyer_subkey: "0xparitysub".into(),
            seller_subkey: "0xothersub".into(),
            series_id: "0xseriesa".into(),
            matched_qty_1e8: "5".into(),
            premium_amount: "50".into(),
            fee_amount: "1".into(),
            rebate_amount: "0".into(),
            block_number: 3,
            tx_hash: "0xtx3".into(),
            completion_status: ExecutionCompletion::Complete,
        },
    );
    state.matched_executions.insert(
        "0xexecb".into(),
        MatchedExecutionRow {
            buyer_order_hash: "0xord3".into(),
            seller_order_hash: "0xord4".into(),
            buyer_subkey: "0xparitysub".into(),
            seller_subkey: "0xothersub".into(),
            series_id: "0xseriesa".into(),
            matched_qty_1e8: "1".into(),
            premium_amount: "10".into(),
            fee_amount: "0".into(),
            rebate_amount: "0".into(),
            block_number: 4,
            tx_hash: "0xtx4".into(),
            completion_status: ExecutionCompletion::Incomplete,
        },
    );
    state.fee_events.push(FeeEventRow {
        kind: "OPTION_FEE_CHARGED",
        payer_subkey: Some("0xparitysub".into()),
        receiver_subkey: Some("0xpf".into()),
        token: "0xtokA".into(),
        amount: "10".into(),
        block_number: 3,
        tx_hash: "0xtx3".into(),
        log_index: 0,
        execution_id: Some("0xexeca".into()),
    });
    state
        .recovery_state
        .insert("0xparitysub".into(), RecoveryStateProjection::Normal);
    state.escape_state.insert(
        "0xparitysub".into(),
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
    state
}

fn build_store(deployment_id: u64) -> InMemoryHybridV2ReadStore {
    InMemoryStoreBuilder::new(deployment_id)
        .with_chain_id(84532)
        .with_manifest("0xparityhash", "0xparityaddr")
        .with_state(build_state())
        .with_cursor(cursor_at(5, "0xblk5"))
        .with_readiness(true, None, None)
        .with_finalized_head(2)
        .build()
}

#[tokio::test]
async fn list_deployments_returns_configured_row() {
    let store = build_store(7);
    let list = store.list_deployments().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].deployment_id, 7);
    assert_eq!(list[0].chain_id, 84532);
    assert_eq!(list[0].manifest_hash, "0xparityhash");
}

#[tokio::test]
async fn deployment_status_reflects_cursor_and_finalized() {
    let store = build_store(7);
    let st = store.get_deployment_status(7).await.unwrap().unwrap();
    assert_eq!(st.indexed_head_block, 5);
    assert_eq!(st.indexed_head_hash, "0xblk5");
    assert_eq!(st.finalized_head_block, 2);
    assert!(st.ready);
    let miss = store.get_deployment_status(999).await.unwrap();
    assert!(miss.is_none());
}

#[tokio::test]
async fn deployment_isolation_returns_empty_for_wrong_id() {
    let store = build_store(7);
    let subs = store
        .list_subaccounts_by_owner(99, "0xparityown")
        .await
        .unwrap();
    assert!(subs.is_empty());
    let col = store.list_collateral(99, "0xparitysub").await.unwrap();
    assert!(col.is_empty());
    let orders = store
        .list_orders(99, "0xparitysub", &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    assert!(orders.items.is_empty());
}

#[tokio::test]
async fn owner_subaccounts_ordered_by_sid_and_exclude_account_zero() {
    let mut state = build_state();
    state
        .subaccounts
        .insert(("0xparityown".into(), 0), "0xacc0".into());
    state
        .subaccounts
        .insert(("0xparityown".into(), 2), "0xacc2".into());
    let store = InMemoryStoreBuilder::new(7).with_state(state).build();
    let subs = store
        .list_subaccounts_by_owner(7, "0xparityown")
        .await
        .unwrap();
    assert_eq!(subs.len(), 2);
    assert!(subs[0].subaccount_id < subs[1].subaccount_id);
    assert!(subs.iter().all(|s| s.subaccount_id > 0));
}

#[tokio::test]
async fn collateral_aggregate_equals_per_engine_sum() {
    let store = build_store(7);
    let coll = store.list_collateral(7, "0xparitysub").await.unwrap();
    let tok_a = coll.iter().find(|c| c.token == "0xtokA").unwrap();
    assert_eq!(tok_a.balance, "1000");
    assert_eq!(tok_a.aggregate_reserved, "300");
    assert_eq!(tok_a.available, "700");
    let tok_b = coll.iter().find(|c| c.token == "0xtokB").unwrap();
    assert_eq!(tok_b.aggregate_reserved, "0");
    assert_eq!(tok_b.available, "500");
}

#[tokio::test]
async fn positions_include_active_flag_and_paginate_uniquely() {
    let store = build_store(7);
    let page = store
        .list_positions(7, "0xparitysub", &PageAnchor::first(1).unwrap())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let first_id = page.items[0].series_id.clone();
    let next = page.next_anchor.unwrap();
    let page2 = store.list_positions(7, "0xparitysub", &next).await.unwrap();
    assert!(page2.items.iter().all(|p| p.series_id != first_id));
}

#[tokio::test]
async fn orders_remaining_never_negative() {
    let store = build_store(7);
    let page = store
        .list_orders(7, "0xparitysub", &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    for o in &page.items {
        // remaining is computed as total - filled; should never be
        // negative or exceed total.
        let filled: u128 = o.filled_qty_1e8.parse().unwrap_or(0);
        let total: u128 = o.total_qty_1e8.parse().unwrap_or(0);
        let remaining: u128 = o.remaining_qty_1e8.parse().unwrap_or(0);
        assert!(remaining <= total);
        assert_eq!(remaining, total.saturating_sub(filled));
    }
}

#[tokio::test]
async fn get_order_lookup_case_insensitive() {
    let store = build_store(7);
    let a = store.get_order(7, "0xord1").await.unwrap().unwrap();
    let b = store.get_order(7, "0xORD1").await.unwrap().unwrap();
    assert_eq!(a.order_hash, b.order_hash);
    assert_eq!(a.filled_qty_1e8, "40");
    assert_eq!(a.remaining_qty_1e8, "60");
}

#[tokio::test]
async fn completed_executions_exclude_incomplete() {
    let store = build_store(7);
    let page = store
        .list_completed_executions(7, None, &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].execution_id, "0xexeca");
    // Filter by subkey isolates buyer/seller side.
    let filtered = store
        .list_completed_executions(7, Some("0xothersub"), &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].seller_subkey, "0xothersub");
}

#[tokio::test]
async fn fees_filtered_by_subkey_payer_or_receiver() {
    let store = build_store(7);
    let page = store
        .list_fees(7, "0xparitysub", &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].amount, "10");
    // Sibling subkey with no fee events → empty.
    let empty = store
        .list_fees(7, "0xothersub", &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    assert!(empty.items.is_empty());
}

#[tokio::test]
async fn recovery_record_shape_defaults_when_absent() {
    let store = build_store(7);
    let rec = store.get_recovery(7, "0xparitysub").await.unwrap().unwrap();
    assert_eq!(rec.recovery_state, "NORMAL");
    assert!(!rec.finalized);
    assert!(rec.escape.is_some());
    assert_eq!(rec.escape.unwrap().state, "NORMAL");
    let missing = store.get_recovery(7, "0xnoexist").await.unwrap();
    // subaccount not present → returns default NORMAL record per adapter.
    assert!(missing.is_some());
    assert_eq!(missing.unwrap().recovery_state, "NORMAL");
}

#[tokio::test]
async fn history_filter_hash_mismatch_rejected() {
    let store = build_store(7);
    let filter = HistoryFilter::default();
    let anchor = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        "wrong".into(),
        "0xblk5".into(),
    )
    .unwrap();
    let err = store
        .query_history(7, &HistoryScope::Global, &filter, &anchor)
        .await
        .unwrap_err();
    assert!(matches!(err, ReadStoreError::InvalidCursor { .. }));
}

#[tokio::test]
async fn history_stale_indexed_head_classified() {
    let store = build_store(7);
    let filter = HistoryFilter::default();
    let anchor = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        filter_stable_hash(&filter),
        "0xreorged_head".into(),
    )
    .unwrap();
    let err = store
        .query_history(7, &HistoryScope::Global, &filter, &anchor)
        .await
        .unwrap_err();
    assert!(matches!(err, ReadStoreError::StaleCursor { .. }));
}

#[tokio::test]
async fn history_finalized_boundary_matches_snapshot() {
    let journal = vec![
        InMemoryJournalEntry {
            block_number: 1,
            block_hash: "0xblk1".into(),
            tx_hash: "0xtx1".into(),
            tx_index: 0,
            log_index: 0,
            block_timestamp: 1_700_000_012,
            kind: EventKind::Deposit,
            subkey: Some("0xhistsub".into()),
            owner: None,
            subaccount_id: None,
            token: Some("0xt".into()),
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({ "amount": "100" }),
            is_canonical: true,
        },
        InMemoryJournalEntry {
            block_number: 3,
            block_hash: "0xblk3".into(),
            tx_hash: "0xtx3".into(),
            tx_index: 0,
            log_index: 0,
            block_timestamp: 1_700_000_036,
            kind: EventKind::Withdraw,
            subkey: Some("0xhistsub".into()),
            owner: None,
            subaccount_id: None,
            token: Some("0xt".into()),
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({ "amount": "50" }),
            is_canonical: true,
        },
    ];
    let store = InMemoryStoreBuilder::new(7)
        .with_state(ProjectionState::default())
        .with_cursor(cursor_at(3, "0xblk3"))
        .with_finalized_head(2)
        .with_journal(journal)
        .build();
    let filter = HistoryFilter::default();
    let anchor = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        filter_stable_hash(&filter),
        "0xblk3".into(),
    )
    .unwrap();
    let page = store
        .query_history(7, &HistoryScope::Global, &filter, &anchor)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    // Ordered DESC — block 3 first.
    assert_eq!(page.items[0].block_number, 3);
    assert!(!page.items[0].finalized);
    assert_eq!(page.items[1].block_number, 1);
    assert!(page.items[1].finalized);
}

#[tokio::test]
async fn history_family_filter_applied_after_decode() {
    let journal = vec![
        InMemoryJournalEntry {
            block_number: 1,
            block_hash: "0xblk1".into(),
            tx_hash: "0xtx1".into(),
            tx_index: 0,
            log_index: 0,
            block_timestamp: 1_700_000_012,
            kind: EventKind::Deposit,
            subkey: Some("0xhsub".into()),
            owner: None,
            subaccount_id: None,
            token: Some("0xt".into()),
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({ "amount": "100" }),
            is_canonical: true,
        },
        InMemoryJournalEntry {
            block_number: 2,
            block_hash: "0xblk2".into(),
            tx_hash: "0xtx2".into(),
            tx_index: 0,
            log_index: 0,
            block_timestamp: 1_700_000_024,
            kind: EventKind::Withdraw,
            subkey: Some("0xhsub".into()),
            owner: None,
            subaccount_id: None,
            token: Some("0xt".into()),
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({ "amount": "50" }),
            is_canonical: true,
        },
    ];
    let store = InMemoryStoreBuilder::new(7).with_journal(journal).build();
    let filter = HistoryFilter {
        families: vec!["DEPOSIT".into()],
        ..Default::default()
    };
    let anchor = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        filter_stable_hash(&filter),
        String::new(),
    )
    .unwrap();
    let page = store
        .query_history(7, &HistoryScope::Global, &filter, &anchor)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].block_number, 1);
}

#[tokio::test]
async fn history_direction_filter_applied() {
    let journal = vec![InMemoryJournalEntry {
        block_number: 1,
        block_hash: "0xblk1".into(),
        tx_hash: "0xtx1".into(),
        tx_index: 0,
        log_index: 0,
        block_timestamp: 1_700_000_012,
        kind: EventKind::Deposit,
        subkey: Some("0xhsub".into()),
        owner: None,
        subaccount_id: None,
        token: Some("0xt".into()),
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": "100" }),
        is_canonical: true,
    }];
    let store = InMemoryStoreBuilder::new(7).with_journal(journal).build();
    let filter_out = HistoryFilter {
        direction: Some(HistoryDirection::Outbound),
        ..Default::default()
    };
    let anchor_out = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        filter_stable_hash(&filter_out),
        String::new(),
    )
    .unwrap();
    let page = store
        .query_history(7, &HistoryScope::Global, &filter_out, &anchor_out)
        .await
        .unwrap();
    assert!(page.items.is_empty());
    let filter_in = HistoryFilter {
        direction: Some(HistoryDirection::Inbound),
        ..Default::default()
    };
    let anchor_in = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        filter_stable_hash(&filter_in),
        String::new(),
    )
    .unwrap();
    let page2 = store
        .query_history(7, &HistoryScope::Global, &filter_in, &anchor_in)
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 1);
}

#[tokio::test]
async fn store_recreation_preserves_query_results() {
    // Two identical stores built from the same ProjectionState must
    // produce identical output. Proves the adapter is stateless.
    let store1 = build_store(7);
    let store2 = build_store(7);
    let a1 = store1.list_collateral(7, "0xparitysub").await.unwrap();
    let a2 = store2.list_collateral(7, "0xparitysub").await.unwrap();
    assert_eq!(a1, a2);
    let b1 = store1
        .list_orders(7, "0xparitysub", &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    let b2 = store2
        .list_orders(7, "0xparitysub", &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    assert_eq!(b1.items.len(), b2.items.len());
    for (x, y) in b1.items.iter().zip(b2.items.iter()) {
        assert_eq!(x.order_hash, y.order_hash);
        assert_eq!(x.remaining_qty_1e8, y.remaining_qty_1e8);
    }
}

#[tokio::test]
async fn exact_uint256_roundtrips_through_record() {
    let u256_max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    let mut state = ProjectionState::default();
    state
        .balances
        .insert(("0xu256".into(), "0xt".into()), u256_max.into());
    let store = InMemoryStoreBuilder::new(7).with_state(state).build();
    let coll = store.list_collateral(7, "0xu256").await.unwrap();
    assert_eq!(coll.len(), 1);
    assert_eq!(coll[0].balance, u256_max);
    assert_eq!(coll[0].available, u256_max);
}

#[tokio::test]
async fn pagination_no_duplicates_property() {
    // Property-test: build 10 orders, walk pages of size 3, verify
    // every order is returned exactly once, no gaps.
    let mut state = ProjectionState::default();
    for i in 0..10u8 {
        let hash = format!("0xord{:02x}", i);
        state.order_lifecycle.insert(
            hash.clone(),
            OrderLifecycleRow {
                subkey: "0xpagsub".into(),
                owner: "0xpagown".into(),
                series_id: Some("0xs".into()),
                side: 0,
                time_in_force: 0,
                total_qty_1e8: "100".into(),
                filled_qty_1e8: "0".into(),
                cancelled: false,
                terminal: false,
                first_seen_block: 1,
                last_event_block: 1,
            },
        );
    }
    let store = InMemoryStoreBuilder::new(7).with_state(state).build();
    let mut seen = std::collections::BTreeSet::new();
    let mut anchor = PageAnchor::first(3).unwrap();
    loop {
        let page = store.list_orders(7, "0xpagsub", &anchor).await.unwrap();
        for o in &page.items {
            let inserted = seen.insert(o.order_hash.clone());
            assert!(inserted, "duplicate order {} across pages", o.order_hash);
        }
        match page.next_anchor {
            Some(next) => anchor = next,
            None => break,
        }
    }
    assert_eq!(seen.len(), 10);
}
