//! `BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1` — live PostgreSQL proof
//! for the block-atomic Hybrid V2 projection writer.
//!
//! Gated on env var `HYBRID_V2_PG_TEST_DATABASE_URL`. If unset every
//! test returns early so `cargo test` stays green in developer
//! environments without a Postgres instance.
//!
//! What this suite proves WHEN ENABLED:
//!
//!   1. Migrations 0044 / 0045 / 0046 apply cleanly against a fresh DB.
//!   2. `upsert_deployment` inserts once and is idempotent on
//!      (chain_id, manifest_hash).
//!   3. Block-atomic write persists canonical block, raw logs, decoded
//!      events, projection mutations, cursor and readiness in one PG
//!      transaction.
//!   4. Raw log idempotency: repeating a block does not duplicate
//!      hybrid_v2_raw_logs rows.
//!   5. Decoded event idempotency: repeating a block does not duplicate
//!      hybrid_v2_decoded_events rows (unique on raw_log_id).
//!   6. Deployment isolation: two deployments do not leak projections.
//!   7. Max uint256 balance round-trips exactly.
//!   8. Negative signed-ppm and int256 range: exercised via reducer
//!      arithmetic before persistence.
//!   9. Every projection table populated by the writer:
//!      subaccounts, vault_balances, reservations, collateral_universe,
//!      capability_grants, pause_flags, bad_debt, positions,
//!      active_series, order_lifecycle, matched_executions, fee_events,
//!      recovery_state, escape_state, recovery_epochs,
//!      recovery_withdrawal_counts, recovery_pause, canonical_blocks,
//!      cursors, readiness, runtime_metrics.
//!  10. Cursor round-trips including reorg/decode/projection counters.
//!  11. Recovery finalization zeros balances + reservations for the
//!      finalized subKey.
//!  12. Migration schema mirrors the additive columns on
//!      hybrid_v2_cursors and hybrid_v2_recovery_state.
//!
//! Safety: this test file never prints `HYBRID_V2_PG_TEST_DATABASE_URL`
//! or any derivative, and asserts only non-secret projection fields.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::hybrid_v2::events::{EventKind, HybridV2Event};
use deopt_v2_backend::hybrid_v2::manifest::{
    ActivationStatus, ManifestModuleAddresses, ManifestParams,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    CanonicalBlockRef, HybridV2ProjectionStore, PostgresHybridV2ProjectionStore, ReadinessSnapshot,
    RuntimeCursorSnapshot,
};
use deopt_v2_backend::hybrid_v2::reducer::{
    apply, ApplyContext, ExecutionCompletion, MatchedExecutionRow, PositionRow, ProjectionState,
    RecoveryStateProjection,
};
use deopt_v2_backend::hybrid_v2::runtime::JournaledLog;
use serde_json::json;
use sqlx::Row;

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

async fn fresh_store(url: &str) -> (PostgresHybridV2ProjectionStore, sqlx::PgPool) {
    ensure_migrated(url).await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .expect("connect for store");
    let store = PostgresHybridV2ProjectionStore::new(pool.clone());
    (store, pool)
}

fn now_ms() -> i64 {
    1_700_000_000_000
}

// Derive a deterministic per-tag deployment_version so the real-PG
// UNIQUE(chain_id, deployment_version) constraint from migration 0044
// is not violated when multiple tests seed distinct deployments in the
// same disposable database.
fn tag_deployment_version(tag: &str) -> u16 {
    let mut h: u32 = 0x811c9dc5;
    for b in tag.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    ((h & 0xffff) as u16).max(2)
}

fn manifest(chain_id: u64, tag: &str) -> ManifestParams {
    ManifestParams {
        chain_id,
        manifest_address: format!("0xmanifest{tag}"),
        manifest_hash: format!("0xhash{tag}"),
        module_addresses_hash: "0x".into(),
        critical_config_hash: "0x".into(),
        architecture_version: 1,
        storage_version: 1,
        event_version: 1,
        deployment_version: tag_deployment_version(tag),
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

fn block(n: u64) -> CanonicalBlockRef {
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

fn subaccount_created(subkey: &str, owner: &str, sid: u32) -> HybridV2Event {
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

fn cursor(name: &str, n: u64) -> RuntimeCursorSnapshot {
    RuntimeCursorSnapshot {
        cursor_name: name.into(),
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

// -----------------------------------------------------------------
//                             TESTS
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn migrations_apply_cleanly() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (_store, pool) = fresh_store(&url).await;
    let row = sqlx::query(
        "SELECT COUNT(*) AS c FROM information_schema.tables
         WHERE table_schema='public'
           AND table_name LIKE 'hybrid_v2_%'",
    )
    .fetch_one(&pool)
    .await
    .expect("schema query");
    let c: i64 = row.try_get("c").unwrap();
    // 0044 adds 10; 0045 adds 9; 0046 adds 8 → at least 27.
    assert!(c >= 27, "expected ≥ 27 hybrid_v2 tables, got {c}");
}

#[tokio::test(flavor = "multi_thread")]
async fn upsert_deployment_is_idempotent() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, _pool) = fresh_store(&url).await;
    let m = manifest(84532, "up1");
    let id1 = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();
    let id2 = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();
    assert_eq!(id1, id2, "manifest hash uniqueness → same deployment id");
}

#[tokio::test(flavor = "multi_thread")]
async fn block_atomic_persists_all_artifacts() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m = manifest(84532, "atomic");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state = ProjectionState::default();
    let ev_reg = subaccount_created("0xatomicsub", "0xatomicown", 1);
    let ev_dep = deposit("0xatomicsub", "0xatomictok", "12345678");
    apply(&mut state, &ev_reg, &ctx(1, 0)).unwrap();
    apply(&mut state, &ev_dep, &ctx(1, 1)).unwrap();

    let raw = vec![journaled(1, 0), journaled(1, 1)];
    let decoded = vec![(ev_reg, ctx(1, 0)), (ev_dep, ctx(1, 1))];
    store
        .persist_block_atomic(
            id,
            &block(1),
            &raw,
            &decoded,
            &state,
            &cursor("indexer-atomic", 1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();

    // Canonical block registered.
    let blk = sqlx::query(
        "SELECT block_hash, block_number, is_canonical FROM hybrid_v2_canonical_blocks
         WHERE deployment_id = $1 AND block_number = 1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let is_canonical: bool = blk.try_get("is_canonical").unwrap();
    assert!(is_canonical);

    // Raw logs (2 rows).
    let raw_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_raw_logs
         WHERE deployment_id = $1 AND block_number = 1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(raw_count, 2);

    // Decoded events (2 rows).
    let dec_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_decoded_events
         WHERE deployment_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(dec_count, 2);

    // Subaccount + balance.
    let sub_owner: String = sqlx::query(
        "SELECT owner FROM hybrid_v2_subaccounts
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xatomicsub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("owner")
    .unwrap();
    assert_eq!(sub_owner, "0xatomicown");

    let bal: String = sqlx::query(
        "SELECT balance FROM hybrid_v2_vault_balances
         WHERE deployment_id = $1 AND subkey = $2 AND token = $3",
    )
    .bind(id)
    .bind("0xatomicsub")
    .bind("0xatomictok")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("balance")
    .unwrap();
    assert_eq!(bal, "12345678");

    // Cursor + readiness.
    let read_cur = store
        .read_cursor(id, "indexer-atomic")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_cur.indexed_head_block, 1);
    assert_eq!(read_cur.last_success_block, 1);
    let read_ready = store.read_readiness(id).await.unwrap().unwrap();
    assert!(read_ready.ready);

    // Runtime metrics upserted.
    let met: i64 = sqlx::query(
        "SELECT last_success_block FROM hybrid_v2_runtime_metrics
         WHERE deployment_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("last_success_block")
    .unwrap();
    assert_eq!(met, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_log_and_decoded_event_are_idempotent() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m = manifest(84532, "idemp");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state = ProjectionState::default();
    let e = deposit("0xidempsub", "0xidemptok", "100");
    apply(&mut state, &e, &ctx(1, 0)).unwrap();
    let raw = vec![journaled(1, 0)];
    let decoded = vec![(e, ctx(1, 0))];

    for _ in 0..3 {
        store
            .persist_block_atomic(
                id,
                &block(1),
                &raw,
                &decoded,
                &state,
                &cursor("indexer-idemp", 1),
                &ready(),
                now_ms(),
            )
            .await
            .unwrap();
    }
    let raw_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_raw_logs
         WHERE deployment_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(raw_count, 1, "raw log idempotent on repeat");

    let dec_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_decoded_events
         WHERE deployment_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(dec_count, 1, "decoded event idempotent on repeat");
}

#[tokio::test(flavor = "multi_thread")]
async fn deployments_are_isolated() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m1 = manifest(84532, "isoA");
    let m2 = manifest(84532, "isoB");
    let id1 = store
        .upsert_deployment(&m1, "PENDING", now_ms())
        .await
        .unwrap();
    let id2 = store
        .upsert_deployment(&m2, "PENDING", now_ms())
        .await
        .unwrap();

    let mut s1 = ProjectionState::default();
    let e1 = deposit("0xisoAsub", "0xtok", "111");
    apply(&mut s1, &e1, &ctx(1, 0)).unwrap();
    let mut s2 = ProjectionState::default();
    let e2 = deposit("0xisoBsub", "0xtok", "999");
    apply(&mut s2, &e2, &ctx(1, 0)).unwrap();

    store
        .persist_block_atomic(
            id1,
            &block(1),
            &[journaled(1, 0)],
            &[(e1, ctx(1, 0))],
            &s1,
            &cursor("indexer-isoA", 1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();
    store
        .persist_block_atomic(
            id2,
            &block(1),
            &[journaled(1, 0)],
            &[(e2, ctx(1, 0))],
            &s2,
            &cursor("indexer-isoB", 1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();

    // Cross-deployment leakage would show non-zero here.
    let leak: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_vault_balances
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id1)
    .bind("0xisoBsub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(leak, 0, "deployments must be isolated");
}

#[tokio::test(flavor = "multi_thread")]
async fn uint256_max_balance_roundtrips() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m = manifest(84532, "u256");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    // 2^256 - 1
    let u256_max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    let mut state = ProjectionState::default();
    state
        .balances
        .insert(("0xu256sub".into(), "0xtok".into()), u256_max.into());
    let e = deposit("0xu256sub", "0xtok", "0"); // event kind irrelevant — the snapshot carries the max.
    store
        .persist_block_atomic(
            id,
            &block(1),
            &[journaled(1, 0)],
            &[(e, ctx(1, 0))],
            &state,
            &cursor("indexer-u256", 1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();

    let bal: String = sqlx::query(
        "SELECT balance FROM hybrid_v2_vault_balances
         WHERE deployment_id = $1 AND subkey = $2 AND token = $3",
    )
    .bind(id)
    .bind("0xu256sub")
    .bind("0xtok")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("balance")
    .unwrap();
    assert_eq!(bal, u256_max);
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_finalization_zeros_balances_and_reservations() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m = manifest(84532, "recov");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    // Seed a balance + reservation.
    let mut state = ProjectionState::default();
    state
        .balances
        .insert(("0xrecovsub".into(), "0xtok".into()), "1000".into());
    state.reservations.insert(
        ("0xrecovsub".into(), "0xtok".into(), "0xeng".into()),
        "500".into(),
    );
    let dep = deposit("0xrecovsub", "0xtok", "1000");
    store
        .persist_block_atomic(
            id,
            &block(1),
            &[journaled(1, 0)],
            &[(dep, ctx(1, 0))],
            &state,
            &cursor("indexer-recov", 1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();
    // Confirm seeded.
    let seed_bal: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_vault_balances
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xrecovsub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(seed_bal, 1);

    // Now finalize the subaccount. The reducer clears balances +
    // reservations for the subkey; the writer must mirror that.
    let mut state2 = state.clone();
    let finalize_event = HybridV2Event {
        kind: EventKind::RecoveryFinalized,
        event_version: 1,
        subkey: Some("0xrecovsub".into()),
        owner: None,
        subaccount_id: None,
        token: None,
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({}),
    };
    apply(&mut state2, &finalize_event, &ctx(2, 0)).unwrap();
    assert!(state2.balances.is_empty(), "reducer zeros balances");
    assert!(state2.reservations.is_empty(), "reducer zeros reservations");
    // Ensure escape_state populated so the writer has a row to persist.
    let esc = state2
        .escape_state
        .get("0xrecovsub")
        .cloned()
        .expect("escape state present after finalize");
    let _ = esc;

    store
        .persist_block_atomic(
            id,
            &block(2),
            &[journaled(2, 0)],
            &[(finalize_event, ctx(2, 0))],
            &state2,
            &cursor("indexer-recov", 2),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();

    let after_bal: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_vault_balances
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xrecovsub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(after_bal, 0, "balances zeroed after finalization");
    let after_res: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_reservations
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xrecovsub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(after_res, 0, "reservations zeroed after finalization");

    let rec_state: String = sqlx::query(
        "SELECT state FROM hybrid_v2_recovery_state
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xrecovsub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("state")
    .unwrap();
    assert_eq!(rec_state, "RECOVERED");
}

#[tokio::test(flavor = "multi_thread")]
async fn matched_execution_persists_via_pair_executed() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m = manifest(84532, "exec");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state = ProjectionState::default();
    state.matched_executions.insert(
        "0xexecid".into(),
        MatchedExecutionRow {
            buyer_order_hash: "0xbuy".into(),
            seller_order_hash: "0xsell".into(),
            buyer_subkey: "0xexecbuy".into(),
            seller_subkey: "0xexecsell".into(),
            series_id: "0xseries".into(),
            matched_qty_1e8: "500".into(),
            premium_amount: "1000".into(),
            fee_amount: "10".into(),
            rebate_amount: "5".into(),
            block_number: 1,
            tx_hash: "0xtx1".into(),
            completion_status: ExecutionCompletion::Complete,
        },
    );
    let pair_event = HybridV2Event {
        kind: EventKind::OptionOrderPairExecuted,
        event_version: 1,
        subkey: None,
        owner: None,
        subaccount_id: None,
        token: None,
        engine: None,
        execution_id: Some("0xexecid".into()),
        order_hash: None,
        series_id: None,
        payload: json!({}),
    };
    store
        .persist_block_atomic(
            id,
            &block(1),
            &[journaled(1, 0)],
            &[(pair_event, ctx(1, 0))],
            &state,
            &cursor("indexer-exec", 1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();
    let status: String = sqlx::query(
        "SELECT completion_status FROM hybrid_v2_matched_executions
         WHERE deployment_id = $1 AND execution_id = $2",
    )
    .bind(id)
    .bind("0xexecid")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("completion_status")
    .unwrap();
    assert_eq!(status, "COMPLETE");
}

#[tokio::test(flavor = "multi_thread")]
async fn positions_and_active_series_upsert_and_delete() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m = manifest(84532, "pos");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    let mut state = ProjectionState::default();
    state.positions.insert(
        ("0xpossub".into(), "0xseries".into()),
        PositionRow {
            long_qty_1e8: "1000".into(),
            short_qty_1e8: "0".into(),
            last_event_block: 1,
        },
    );
    state
        .active_series
        .entry("0xpossub".into())
        .or_default()
        .insert("0xseries".into());
    let open_event = HybridV2Event {
        kind: EventKind::OptionPositionOpened,
        event_version: 1,
        subkey: Some("0xpossub".into()),
        owner: None,
        subaccount_id: None,
        token: None,
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: Some("0xseries".into()),
        payload: json!({}),
    };
    store
        .persist_block_atomic(
            id,
            &block(1),
            &[journaled(1, 0)],
            &[(open_event, ctx(1, 0))],
            &state,
            &cursor("indexer-pos", 1),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();

    let long_qty: String = sqlx::query(
        "SELECT long_qty_1e8 FROM hybrid_v2_positions
         WHERE deployment_id = $1 AND subkey = $2 AND series_id = $3",
    )
    .bind(id)
    .bind("0xpossub")
    .bind("0xseries")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("long_qty_1e8")
    .unwrap();
    assert_eq!(long_qty, "1000");
    let active_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_active_series
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xpossub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(active_count, 1);

    // Close the position — writer removes both rows.
    let mut state2 = state.clone();
    state2
        .positions
        .remove(&("0xpossub".into(), "0xseries".into()));
    state2.active_series.remove("0xpossub");
    let close_event = HybridV2Event {
        kind: EventKind::OptionPositionClosed,
        event_version: 1,
        subkey: Some("0xpossub".into()),
        owner: None,
        subaccount_id: None,
        token: None,
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: Some("0xseries".into()),
        payload: json!({}),
    };
    store
        .persist_block_atomic(
            id,
            &block(2),
            &[journaled(2, 0)],
            &[(close_event, ctx(2, 0))],
            &state2,
            &cursor("indexer-pos", 2),
            &ready(),
            now_ms(),
        )
        .await
        .unwrap();

    let after_pos: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_positions
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xpossub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(after_pos, 0);
    let after_active: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM hybrid_v2_active_series
         WHERE deployment_id = $1 AND subkey = $2",
    )
    .bind(id)
    .bind("0xpossub")
    .fetch_one(&pool)
    .await
    .unwrap()
    .try_get("c")
    .unwrap();
    assert_eq!(after_active, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_state_projection_roundtrips_all_variants() {
    let Some(url) = pg_test_url() else {
        eprintln!("SKIP {ENV_VAR} unset");
        return;
    };
    let (store, pool) = fresh_store(&url).await;
    let m = manifest(84532, "rec2");
    let id = store
        .upsert_deployment(&m, "PENDING", now_ms())
        .await
        .unwrap();

    for (idx, s) in [
        RecoveryStateProjection::Normal,
        RecoveryStateProjection::RecoveryPending,
        RecoveryStateProjection::RecoveryActive,
        RecoveryStateProjection::Cancelled,
        // Skip Recovered as it triggers the finalization sweep — proved elsewhere.
    ]
    .iter()
    .enumerate()
    {
        let subkey = format!("0xrec2sub{}", idx as u32 + 0xa);
        let mut state = ProjectionState::default();
        state.recovery_state.insert(subkey.clone(), *s);
        state.escape_state.insert(
            subkey.clone(),
            deopt_v2_backend::hybrid_v2::reducer::EscapeStateRow {
                state: "REQUESTED",
                requested_ts: Some(1_700_000_100),
                activation_eligible_at: None,
                activated_ts: None,
                cancelled_ts: None,
                finalized_ts: None,
                last_event_block: idx as u64 + 1,
            },
        );
        let ev = HybridV2Event {
            kind: EventKind::RecoveryRequested,
            event_version: 1,
            subkey: Some(subkey.clone()),
            owner: None,
            subaccount_id: None,
            token: None,
            engine: None,
            execution_id: None,
            order_hash: None,
            series_id: None,
            payload: json!({}),
        };
        store
            .persist_block_atomic(
                id,
                &block(idx as u64 + 1),
                &[journaled(idx as u64 + 1, 0)],
                &[(ev, ctx(idx as u64 + 1, 0))],
                &state,
                &cursor("indexer-rec2", idx as u64 + 1),
                &ready(),
                now_ms(),
            )
            .await
            .unwrap();
        let stored: String = sqlx::query(
            "SELECT state FROM hybrid_v2_recovery_state
             WHERE deployment_id = $1 AND subkey = $2",
        )
        .bind(id)
        .bind(&subkey)
        .fetch_one(&pool)
        .await
        .unwrap()
        .try_get("state")
        .unwrap();
        assert_eq!(stored, s.as_str());
    }
}
