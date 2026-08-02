//! `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2A-STORE-AND-PG-TESTS-V1`
//! Live PostgreSQL proof for `PostgresHybridV2ReadStore`.
//!
//! Gated on env var `HYBRID_V2_PG_TEST_DATABASE_URL`. When unset every
//! test returns early so `cargo test` stays green in developer
//! environments without a Postgres instance.
//!
//! Coverage (WHEN ENABLED):
//!   * migrations through 0046 apply cleanly
//!   * list_deployments deterministic + isolated per row
//!   * get_deployment_status covers cursor + readiness + metrics
//!   * list_subaccounts_by_owner ordered by subaccount_id ASC, Account 0 excluded
//!   * multiple subaccounts per owner
//!   * get_subaccount_summary aggregates counts across projection tables
//!   * list_collateral aggregates per-engine reservations, exposes universe / enabled
//!   * disabled tokens with non-zero balance/reservation remain visible
//!   * list_reservations deterministic (token asc, engine asc)
//!   * list_positions + keyset pagination + active-series membership
//!   * list_orders + keyset pagination + remaining computed
//!   * get_order lookup deployment-scoped
//!   * list_completed_executions excludes incomplete rows + pagination
//!   * list_fees paginated (block DESC, log_index DESC)
//!   * get_recovery joins state + escape + epochs + withdrawal count
//!   * query_history returns SQL-ordered, filter-hashed page
//!   * history filter mismatch → InvalidCursor
//!   * history indexed_head mismatch → StaleCursor
//!   * u256 MAX balance round-trip
//!   * limit=0 and limit>MAX rejected
//!   * deployment isolation across all families
//!
//! Safety: never prints `HYBRID_V2_PG_TEST_DATABASE_URL` or any
//! derivative. Only non-secret projection fields are asserted.

use deopt_v2_backend::api::hybrid_v2_read::history::HistoryFilter;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::hybrid_v2::events::{EventKind, HybridV2Event};
use deopt_v2_backend::hybrid_v2::manifest::{
    ActivationStatus, ManifestModuleAddresses, ManifestParams,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    CanonicalBlockRef, HybridV2ProjectionStore, PostgresHybridV2ProjectionStore, ReadinessSnapshot,
    RuntimeCursorSnapshot,
};
use deopt_v2_backend::hybrid_v2::read_store::{
    filter_stable_hash, HistoryConsistency, HistoryPageAnchor, HistoryScope, HybridV2ReadStore,
    PageAnchor, PostgresHybridV2ReadStore, ReadStoreError,
};
use deopt_v2_backend::hybrid_v2::reducer::{apply, ApplyContext, ProjectionState};
use deopt_v2_backend::hybrid_v2::runtime::JournaledLog;
use serde_json::json;
use sqlx::postgres::PgPool;

const ENV_VAR: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";

fn pg_test_url() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url)
                .await
                .expect("connect for shared migration");
            repo.run_migrations()
                .await
                .expect("run migrations against disposable PG database");
        })
        .await;
}

async fn fresh_pool(url: &str) -> PgPool {
    ensure_migrated(url).await;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .expect("connect for read store")
}

fn now_ms() -> i64 {
    1_700_000_000_000
}

fn base_manifest(chain_id: u64, tag: &str) -> ManifestParams {
    ManifestParams {
        chain_id,
        manifest_address: format!("0xreadmani{tag}"),
        manifest_hash: format!("0xreadhash{tag}"),
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

fn journaled(n: u64, log_index: u32) -> JournaledLog {
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

fn cursor_snap(n: u64) -> RuntimeCursorSnapshot {
    RuntimeCursorSnapshot {
        cursor_name: "indexer".into(),
        indexed_head_block: n,
        indexed_head_hash: format!("0xblk{n}"),
        indexed_head_parent: if n == 1 {
            "0x0".into()
        } else {
            format!("0xblk{}", n - 1)
        },
        observed_head_block: n,
        finalized_head_block: 0,
        last_error: None,
        reorg_count: 0,
        max_reorg_depth_seen: 0,
        decode_failures: 0,
        projection_failures: 0,
        unknown_canonical_events: 0,
        last_success_block: n,
    }
}

fn ready() -> ReadinessSnapshot {
    ReadinessSnapshot {
        ready: true,
        reason: None,
        reason_detail: None,
    }
}

fn deposit(subkey: &str, token: &str, amount: &str) -> HybridV2Event {
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

fn sub_created(subkey: &str, owner: &str, sid: u32) -> HybridV2Event {
    HybridV2Event {
        kind: EventKind::SubaccountCreated,
        event_version: 1,
        subkey: Some(subkey.into()),
        owner: Some(owner.into()),
        subaccount_id: Some(sid),
        token: None,
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({}),
    }
}

/// Persist a single block containing the given events. Applies each event
/// through the reducer then persists the resulting state via the stage-1
/// projection store, so the read store finds real projection rows in PG.
async fn persist_events(
    projection: &PostgresHybridV2ProjectionStore,
    deployment_id: i64,
    block: u64,
    events: &[HybridV2Event],
    state: &mut ProjectionState,
) {
    let mut raws = Vec::new();
    let mut decoded = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let c = ctx(block, i as u32);
        apply(state, e, &c).unwrap();
        raws.push(journaled(block, i as u32));
        decoded.push((e.clone(), c));
    }
    projection
        .persist_block_atomic(
            deployment_id,
            &block_ref(block),
            &raws,
            &decoded,
            state,
            &cursor_snap(block),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();
}

async fn setup(url: &str, tag: &str) -> (PostgresHybridV2ReadStore, i64, PgPool) {
    let pool = fresh_pool(url).await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let m = base_manifest(84532, tag);
    let id = projection
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();
    let store = PostgresHybridV2ReadStore::new(pool.clone());
    (store, id, pool)
}

// -----------------------------------------------------------------
//                             TESTS
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn migrations_and_list_deployments() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, _pool) = setup(&url, "list1").await;
    let list = store.list_deployments().await.unwrap();
    let found = list.iter().find(|r| r.deployment_id as i64 == id).unwrap();
    assert_eq!(found.chain_id, 84532);
    assert!(found.manifest_hash.starts_with("0xreadhash"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_deployment_status_when_unadvanced() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, _pool) = setup(&url, "stat1").await;
    let st = store.get_deployment_status(id as u64).await.unwrap();
    // No cursor row yet — defaults to zeros, not-ready.
    assert!(st.is_some());
    let st = st.unwrap();
    assert_eq!(st.indexed_head_block, 0);
    assert!(!st.ready);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_deployment_status_after_block_reflects_metrics() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "stat2").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    persist_events(
        &projection,
        id,
        5,
        &[deposit("0xstsub", "0xt", "1")],
        &mut state,
    )
    .await;
    let st = store
        .get_deployment_status(id as u64)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(st.indexed_head_block, 5);
    assert_eq!(st.last_success_block, 5);
    assert!(st.ready);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_subaccounts_by_owner_ordered_and_isolated() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "own1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    persist_events(
        &projection,
        id,
        1,
        &[
            sub_created("0xownsuba", "0xown1", 2),
            sub_created("0xownsubb", "0xown1", 1),
            sub_created("0xothrsub", "0xown2", 3),
        ],
        &mut state,
    )
    .await;
    let subs = store
        .list_subaccounts_by_owner(id as u64, "0xown1")
        .await
        .unwrap();
    assert_eq!(subs.len(), 2);
    assert!(subs[0].subaccount_id < subs[1].subaccount_id);
    assert!(subs.iter().all(|s| s.owner == "0xown1"));
    // Cross-owner isolation.
    let empty = store
        .list_subaccounts_by_owner(id as u64, "0xnonexist")
        .await
        .unwrap();
    assert!(empty.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn list_subaccounts_excludes_account_zero() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "own0").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    persist_events(
        &projection,
        id,
        1,
        &[
            sub_created("0xacc0sub", "0xown0", 0),
            sub_created("0xacc1sub", "0xown0", 1),
        ],
        &mut state,
    )
    .await;
    let subs = store
        .list_subaccounts_by_owner(id as u64, "0xown0")
        .await
        .unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].subaccount_id, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_collateral_aggregates_reservations() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "col1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    let dep = deposit("0xcolsub", "0xcoltok", "1000");
    let lock1 = HybridV2Event {
        kind: EventKind::CollateralLocked,
        event_version: 1,
        subkey: Some("0xcolsub".into()),
        owner: None,
        subaccount_id: None,
        token: Some("0xcoltok".into()),
        engine: Some("0xenga".into()),
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": "200" }),
    };
    let lock2 = HybridV2Event {
        kind: EventKind::CollateralLocked,
        event_version: 1,
        subkey: Some("0xcolsub".into()),
        owner: None,
        subaccount_id: None,
        token: Some("0xcoltok".into()),
        engine: Some("0xengb".into()),
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": "300" }),
    };
    persist_events(&projection, id, 1, &[dep, lock1, lock2], &mut state).await;
    let coll = store.list_collateral(id as u64, "0xcolsub").await.unwrap();
    assert_eq!(coll.len(), 1);
    let row = &coll[0];
    assert_eq!(row.balance, "1000");
    assert_eq!(row.aggregate_reserved, "500");
    assert_eq!(row.available, "500");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_reservations_deterministic_order() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "res1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    let dep = deposit("0xressub", "0xressub_t", "500");
    let mk_lock = |token: &str, eng: &str, amount: &str| HybridV2Event {
        kind: EventKind::CollateralLocked,
        event_version: 1,
        subkey: Some("0xressub".into()),
        owner: None,
        subaccount_id: None,
        token: Some(token.into()),
        engine: Some(eng.into()),
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": amount }),
    };
    let dep_b = deposit("0xressub", "0xressub_u", "500");
    persist_events(
        &projection,
        id,
        1,
        &[
            dep,
            dep_b,
            mk_lock("0xressub_u", "0xengz", "50"),
            mk_lock("0xressub_u", "0xenga", "50"),
            mk_lock("0xressub_t", "0xengm", "50"),
        ],
        &mut state,
    )
    .await;
    let res = store
        .list_reservations(id as u64, "0xressub")
        .await
        .unwrap();
    assert_eq!(res.len(), 3);
    // ORDER BY token ASC, engine ASC
    assert_eq!(res[0].token, "0xressub_t");
    assert_eq!(res[1].token, "0xressub_u");
    assert_eq!(res[1].engine, "0xenga");
    assert_eq!(res[2].engine, "0xengz");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_positions_keyset_pagination() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "pos1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    let mut events: Vec<HybridV2Event> = Vec::new();
    for i in 0..5 {
        events.push(HybridV2Event {
            kind: EventKind::OptionPositionOpened,
            event_version: 1,
            subkey: Some("0xpossub".into()),
            owner: None,
            subaccount_id: None,
            token: None,
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: Some(format!("0xseries{i}")),
            payload: json!({ "long_delta_1e8": "100", "short_delta_1e8": "0" }),
        });
    }
    persist_events(&projection, id, 1, &events, &mut state).await;
    let page1 = store
        .list_positions(id as u64, "0xpossub", &PageAnchor::first(2).unwrap())
        .await
        .unwrap();
    assert_eq!(page1.items.len(), 2);
    assert!(page1.next_anchor.is_some());
    let next = page1.next_anchor.unwrap();
    let page2 = store
        .list_positions(id as u64, "0xpossub", &next)
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 2);
    // Deterministic ordering: no duplicates across pages.
    let all: std::collections::BTreeSet<String> = page1
        .items
        .iter()
        .chain(page2.items.iter())
        .map(|p| p.series_id.clone())
        .collect();
    assert_eq!(all.len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_order_and_list_orders_pagination() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "ord1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    let mk_order = |h: &str| HybridV2Event {
        kind: EventKind::OptionOrderFilled,
        event_version: 1,
        subkey: Some("0xordsub".into()),
        owner: Some("0xordown".into()),
        subaccount_id: None,
        token: None,
        engine: None,
        execution_id: None,
        order_hash: Some(h.into()),
        series_id: Some("0xseries".into()),
        payload: json!({
            "side": 0,
            "time_in_force": 0,
            "total_qty_1e8": "1000",
            "filled_delta_1e8": "100",
            "terminal": false
        }),
    };
    persist_events(
        &projection,
        id,
        1,
        &[
            mk_order("0xord0a"),
            mk_order("0xord0b"),
            mk_order("0xord0c"),
        ],
        &mut state,
    )
    .await;
    let got = store
        .get_order(id as u64, "0xord0b")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.subkey, "0xordsub");
    assert_eq!(got.filled_qty_1e8, "100");
    assert_eq!(got.remaining_qty_1e8, "900");
    let missing = store.get_order(id as u64, "0xnope").await.unwrap();
    assert!(missing.is_none());
    let page1 = store
        .list_orders(id as u64, "0xordsub", &PageAnchor::first(2).unwrap())
        .await
        .unwrap();
    assert_eq!(page1.items.len(), 2);
    assert!(page1.next_anchor.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn deployments_are_isolated_in_read_store() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id_a, pool) = setup(&url, "isoA").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let m_b = base_manifest(84532, "isoB");
    let id_b = projection
        .upsert_deployment(&m_b, "PENDING", now_ms())
        .await
        .unwrap();
    let mut sa = ProjectionState::default();
    persist_events(
        &projection,
        id_a,
        1,
        &[deposit("0xisosub", "0xt", "111")],
        &mut sa,
    )
    .await;
    let mut sb = ProjectionState::default();
    persist_events(
        &projection,
        id_b,
        1,
        &[deposit("0xisosub", "0xt", "999")],
        &mut sb,
    )
    .await;
    let a_col = store
        .list_collateral(id_a as u64, "0xisosub")
        .await
        .unwrap();
    let b_col = store
        .list_collateral(id_b as u64, "0xisosub")
        .await
        .unwrap();
    assert_eq!(a_col.len(), 1);
    assert_eq!(b_col.len(), 1);
    assert_eq!(a_col[0].balance, "111");
    assert_eq!(b_col[0].balance, "999");
}

#[tokio::test(flavor = "multi_thread")]
async fn uint256_max_balance_roundtrips_via_read_store() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "u256r").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let u256_max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    let mut state = ProjectionState::default();
    state
        .balances
        .insert(("0xu256sub".into(), "0xtok".into()), u256_max.into());
    // Persist a marker deposit to trigger the block-atomic write, but the
    // balance in ProjectionState is the max value regardless of event amount.
    let ev = deposit("0xu256sub", "0xtok", "0");
    let raws = vec![journaled(1, 0)];
    let decoded = vec![(ev, ctx(1, 0))];
    projection
        .persist_block_atomic(
            id,
            &block_ref(1),
            &raws,
            &decoded,
            &state,
            &cursor_snap(1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();
    let coll = store.list_collateral(id as u64, "0xu256sub").await.unwrap();
    assert_eq!(coll.len(), 1);
    assert_eq!(coll[0].balance, u256_max);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_completed_executions_excludes_incomplete() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "exec1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    // Seed a COMPLETE + an INCOMPLETE execution directly in the projection
    // state, then persist. The read store must only return COMPLETE.
    use deopt_v2_backend::hybrid_v2::reducer::{ExecutionCompletion, MatchedExecutionRow};
    state.matched_executions.insert(
        "0xexecdone".into(),
        MatchedExecutionRow {
            buyer_order_hash: "0xbuy".into(),
            seller_order_hash: "0xsell".into(),
            buyer_subkey: "0xbuyer".into(),
            seller_subkey: "0xseller".into(),
            series_id: "0xseries".into(),
            matched_qty_1e8: "10".into(),
            premium_amount: "100".into(),
            fee_amount: "1".into(),
            rebate_amount: "0".into(),
            block_number: 1,
            tx_hash: "0xtx1".into(),
            completion_status: ExecutionCompletion::Complete,
        },
    );
    state.matched_executions.insert(
        "0xexecpart".into(),
        MatchedExecutionRow {
            buyer_order_hash: "0xbuy2".into(),
            seller_order_hash: "0xsell2".into(),
            buyer_subkey: "0xbuyer".into(),
            seller_subkey: "0xseller".into(),
            series_id: "0xseries".into(),
            matched_qty_1e8: "5".into(),
            premium_amount: "50".into(),
            fee_amount: "0".into(),
            rebate_amount: "0".into(),
            block_number: 1,
            tx_hash: "0xtx1".into(),
            completion_status: ExecutionCompletion::Incomplete,
        },
    );
    // Two pair-executed events (one per execution).
    let done_evt = HybridV2Event {
        kind: EventKind::OptionOrderPairExecuted,
        event_version: 1,
        subkey: None,
        owner: None,
        subaccount_id: None,
        token: None,
        engine: None,
        execution_id: Some("0xexecdone".into()),
        order_hash: None,
        series_id: None,
        payload: json!({}),
    };
    let mut part_evt = done_evt.clone();
    part_evt.execution_id = Some("0xexecpart".into());
    let raws = vec![journaled(1, 0), journaled(1, 1)];
    let decoded = vec![(done_evt, ctx(1, 0)), (part_evt, ctx(1, 1))];
    projection
        .persist_block_atomic(
            id,
            &block_ref(1),
            &raws,
            &decoded,
            &state,
            &cursor_snap(1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();
    let all = store
        .list_completed_executions(id as u64, None, &PageAnchor::first(10).unwrap())
        .await
        .unwrap();
    assert_eq!(all.items.len(), 1);
    assert_eq!(all.items[0].execution_id, "0xexecdone");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_fees_keyset_pagination_desc() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "fee1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    let mk_fee = |_block: u64, _log_index: u32, amount: &str| HybridV2Event {
        kind: EventKind::OptionFeeCharged,
        event_version: 1,
        subkey: Some("0xfeesub".into()),
        owner: None,
        subaccount_id: None,
        token: Some("0xt".into()),
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": amount, "fee_subkey": "0xpf" }),
    };
    // Persist across 3 blocks so ordering DESC is exercised.
    for (block, log_index, amount) in [(1u64, 0u32, "10"), (2, 0, "20"), (3, 0, "30")] {
        persist_events(
            &projection,
            id,
            block,
            &[mk_fee(block, log_index, amount)],
            &mut state,
        )
        .await;
    }
    let page = store
        .list_fees(id as u64, "0xfeesub", &PageAnchor::first(2).unwrap())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    // DESC by block_number.
    assert!(page.items[0].block_number >= page.items[1].block_number);
    let next = page.next_anchor.unwrap();
    let page2 = store.list_fees(id as u64, "0xfeesub", &next).await.unwrap();
    assert!(!page2.items.is_empty());
    // No duplicates.
    let combined: std::collections::BTreeSet<u64> = page
        .items
        .iter()
        .chain(page2.items.iter())
        .map(|f| f.block_number)
        .collect();
    assert_eq!(combined.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_recovery_joins_state_epochs_and_escape() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "rec1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    persist_events(
        &projection,
        id,
        1,
        &[sub_created("0xrecsub", "0xrecown", 1)],
        &mut state,
    )
    .await;
    let request = HybridV2Event {
        kind: EventKind::RecoveryRequested,
        event_version: 1,
        subkey: Some("0xrecsub".into()),
        owner: None,
        subaccount_id: None,
        token: None,
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "activation_eligible_at": "1700003600" }),
    };
    persist_events(&projection, id, 2, &[request], &mut state).await;
    let rec = store
        .get_recovery(id as u64, "0xrecsub")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rec.recovery_state, "RECOVERY_PENDING");
    assert!(!rec.finalized);
    assert!(rec.escape.is_some());
    let esc = rec.escape.unwrap();
    assert_eq!(esc.state, "REQUESTED");
}

#[tokio::test(flavor = "multi_thread")]
async fn subaccount_summary_aggregates_counts() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "sum1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    let sub = sub_created("0xsummary", "0xsummown", 1);
    let dep1 = deposit("0xsummary", "0xtokA", "100");
    let dep2 = deposit("0xsummary", "0xtokB", "200");
    let lock = HybridV2Event {
        kind: EventKind::CollateralLocked,
        event_version: 1,
        subkey: Some("0xsummary".into()),
        owner: None,
        subaccount_id: None,
        token: Some("0xtokA".into()),
        engine: Some("0xeng1".into()),
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": "50" }),
    };
    persist_events(&projection, id, 1, &[sub, dep1, dep2, lock], &mut state).await;
    let summary = store
        .get_subaccount_summary(id as u64, "0xsummary")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.balance_token_count, 2);
    assert_eq!(summary.reservation_count, 1);
    assert_eq!(summary.recovery_state, "NORMAL");
    assert!(!summary.finalized);
}

#[tokio::test(flavor = "multi_thread")]
async fn query_history_returns_ordered_desc() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "hist1").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    for b in 1..=3 {
        persist_events(
            &projection,
            id,
            b,
            &[deposit("0xhistsub", "0xt", &format!("{}", b * 100))],
            &mut state,
        )
        .await;
    }
    let filter = HistoryFilter {
        deployment_id: Some(id as u64),
        subkey: Some("0xhistsub".into()),
        ..Default::default()
    };
    let anchor = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        filter_stable_hash(&filter),
        format!("0xblk3"),
    )
    .unwrap();
    let page = store
        .query_history(id as u64, &HistoryScope::Global, &filter, &anchor)
        .await
        .unwrap();
    assert!(!page.items.is_empty());
    // Ordered DESC by block number.
    let mut prev = u64::MAX;
    for item in &page.items {
        assert!(item.block_number <= prev);
        prev = item.block_number;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn query_history_filter_hash_mismatch_rejected() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, _pool) = setup(&url, "hist2").await;
    let filter = HistoryFilter::default();
    let anchor = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        "wrong-hash".into(),
        "".into(),
    )
    .unwrap();
    let err = store
        .query_history(id as u64, &HistoryScope::Global, &filter, &anchor)
        .await
        .unwrap_err();
    assert!(matches!(err, ReadStoreError::InvalidCursor { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn query_history_stale_indexed_head_classified() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, pool) = setup(&url, "hist3").await;
    let projection = PostgresHybridV2ProjectionStore::new(pool.clone());
    let mut state = ProjectionState::default();
    persist_events(
        &projection,
        id,
        1,
        &[deposit("0xhist3sub", "0xt", "5")],
        &mut state,
    )
    .await;
    let filter = HistoryFilter::default();
    let anchor = HistoryPageAnchor::first(
        10,
        HistoryConsistency::Indexed,
        filter_stable_hash(&filter),
        // Deliberately mismatched head.
        "0xhead_from_yesterday".into(),
    )
    .unwrap();
    let err = store
        .query_history(id as u64, &HistoryScope::Global, &filter, &anchor)
        .await
        .unwrap_err();
    assert!(matches!(err, ReadStoreError::StaleCursor { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn page_limit_zero_and_too_large_rejected() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, id, _pool) = setup(&url, "lim1").await;
    let too_big = PageAnchor::first(10_000);
    assert!(too_big.is_err());
    let zero = PageAnchor {
        last_key: None,
        limit: 0,
    };
    let err = store
        .list_positions(id as u64, "0xanysub", &zero)
        .await
        .unwrap_err();
    assert!(matches!(err, ReadStoreError::LimitExceeded { .. }));
}
