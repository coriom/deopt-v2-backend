//! OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Parts A3–A6 —
//! real-PostgreSQL coverage for the canonical Options execution
//! event reducer.
//!
//! Reducer contract:
//!   * `Promoted(row)` — AWAITING/SUBMISSION_UNKNOWN/SUBMITTED →
//!     CORRELATED_CANONICAL with fingerprint attached.
//!   * `AlreadyCorrelated(row)` — idempotent replay of the exact
//!     same (tx_hash, log_index) on a CORRELATED_CANONICAL row.
//!   * `Conflict(row)` — execution_kind mismatch, tx_hash mismatch,
//!     fill_quantity mismatch, envelope digest mismatch, or a second
//!     event for the same canonical_execution_id.
//!   * `NoCorrelationForIntent` — the event references a canonical id
//!     for which no correlation row exists (legacy pre-Part-E intent).
//!
//! Reorg:
//!   * `reorg_orphan_canonical_correlation` transitions
//!     CORRELATED_CANONICAL → ORPHANED. A replacement AWAITING can
//!     then be inserted for the same canonical_execution_id
//!     (sparse UNIQUE is ACTIVE-only).
//!
//! Loud-fail: same env gate as atomic-wiring suite.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::options::correlation_repository::{
    attach_local_tx_identity, correlate_canonical_option_event, get_by_canonical_execution_id,
    reorg_orphan_canonical_correlation, upsert_awaiting_correlation_tx, AwaitingCorrelationInput,
    CanonicalExecutionEventInput, CorrelationReducerOutcome, OptionCorrelationStatus,
    OptionExecutionKind,
};
use sqlx::PgPool;

const URL_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_URL";
const SKIP_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP";

fn env_pg_url() -> Option<String> {
    std::env::var(URL_ENV).ok().filter(|v| !v.is_empty())
}
fn allow_skip() -> bool {
    std::env::var(SKIP_ENV).ok().as_deref() == Some("1")
}
fn require_pg_url() -> Option<String> {
    match env_pg_url() {
        Some(u) => Some(u),
        None if allow_skip() => None,
        None => panic!("{URL_ENV} is not set"),
    }
}
async fn ensure_migrated(url: &str) {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    MIGRATED
        .get_or_init(|| async {
            let repo = PgRepository::connect(url).await.expect("connect");
            repo.run_migrations().await.expect("migrate");
        })
        .await;
}
async fn require_pool() -> Option<PgPool> {
    let url = require_pg_url()?;
    ensure_migrated(&url).await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("pool");
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    Some(pool)
}
fn canonical_id(tag: &str) -> String {
    let mut s: String = tag.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    while s.len() < 60 {
        s.push('a');
    }
    format!("0x{s:0<64}")
}
fn tx_hash(seed: u32) -> String {
    format!("0x{seed:064x}")
}
fn block_hash(seed: u32) -> String {
    format!("0x{:064x}", 0xB100_0000_u32.wrapping_add(seed))
}
fn awaiting(canonical: &str, kind: OptionExecutionKind) -> AwaitingCorrelationInput {
    AwaitingCorrelationInput {
        canonical_execution_id: canonical.to_string(),
        deployment_id: 1,
        chain_id: 84532,
        execution_kind: kind,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: Some("100000000".to_string()),
        now_ms: 1_700_000_000_000,
    }
}
/// Delete any pre-existing correlation rows for `canonical` so tests
/// are idempotent across re-runs. Without this the second run of a
/// test sees `AlreadyCorrelated` / `attach_local_tx_identity` errors
/// because the previous run left rows in terminal state.
async fn reset_canonical(pool: &PgPool, canonical: &str) {
    sqlx::query("DELETE FROM option_execution_correlations WHERE canonical_execution_id = $1")
        .bind(canonical)
        .execute(pool)
        .await
        .expect("reset");
}

async fn seed_awaiting(pool: &PgPool, canonical: &str, kind: OptionExecutionKind) {
    reset_canonical(pool, canonical).await;
    seed_awaiting_at(pool, canonical, kind, 1_700_000_000_000).await;
}
async fn seed_awaiting_at(pool: &PgPool, canonical: &str, kind: OptionExecutionKind, now_ms: i64) {
    let mut tx = pool.begin().await.expect("begin");
    let mut input = awaiting(canonical, kind);
    input.now_ms = now_ms;
    upsert_awaiting_correlation_tx(&mut tx, &input)
        .await
        .expect("upsert");
    tx.commit().await.expect("commit");
}

// ---------------- A3 promote path -----------------------------------

#[tokio::test]
async fn r01_reducer_promotes_awaiting_to_correlated_canonical() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r01promote");
    seed_awaiting(&pool, &cid, OptionExecutionKind::Trade).await;
    let tx = tx_hash(0x01);
    let bhash = block_hash(0x01);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 4,
        canonical_block_number: 5_000_000,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_100_000,
    };
    match correlate_canonical_option_event(&pool, &input)
        .await
        .expect("reducer")
    {
        CorrelationReducerOutcome::Promoted(row) => {
            assert_eq!(
                row.correlation_status,
                OptionCorrelationStatus::CorrelatedCanonical
            );
            assert_eq!(row.tx_hash.as_deref(), Some(tx.as_str()));
            assert_eq!(row.log_index, Some(4));
            assert_eq!(row.canonical_block_number, Some(5_000_000));
            assert_eq!(row.canonical_block_hash.as_deref(), Some(bhash.as_str()));
            // Optional envelope digests remain NULL (not corrupted to
            // empty string).
            assert!(row.onchain_buyer_order_id.is_none());
            assert!(row.onchain_seller_order_id.is_none());
            assert!(row.onchain_execution_id.is_none());
        }
        other => panic!("expected Promoted, got {other:?}"),
    }
}

#[tokio::test]
async fn r02_reducer_promotes_from_submission_unknown_directly() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r02subunkown");
    seed_awaiting(&pool, &cid, OptionExecutionKind::Trade).await;
    let tx = tx_hash(0x02);
    attach_local_tx_identity(&pool, &cid, &tx, 1_700_000_100_000)
        .await
        .expect("attach local");
    let bhash = block_hash(0x02);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 2,
        canonical_block_number: 5_000_001,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_200_000,
    };
    let outcome = correlate_canonical_option_event(&pool, &input)
        .await
        .expect("reducer");
    assert!(
        matches!(outcome, CorrelationReducerOutcome::Promoted(_)),
        "SUBMISSION_UNKNOWN must promote directly to CORRELATED_CANONICAL"
    );
}

// ---------------- A3 no-correlation path ----------------------------

#[tokio::test]
async fn r03_reducer_returns_no_correlation_for_legacy_intent() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r03legacy");
    reset_canonical(&pool, &cid).await;
    // Deliberately do NOT seed a correlation row — legacy intent.
    let tx = tx_hash(0x03);
    let bhash = block_hash(0x03);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 0,
        canonical_block_number: 5_000_002,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_300_000,
    };
    let outcome = correlate_canonical_option_event(&pool, &input)
        .await
        .expect("reducer");
    assert_eq!(outcome, CorrelationReducerOutcome::NoCorrelationForIntent);
}

// ---------------- A3 idempotent replay ------------------------------

#[tokio::test]
async fn r04_reducer_replays_same_event_idempotently() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r04replay");
    seed_awaiting(&pool, &cid, OptionExecutionKind::Trade).await;
    let tx = tx_hash(0x04);
    let bhash = block_hash(0x04);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 7,
        canonical_block_number: 5_000_003,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_400_000,
    };
    correlate_canonical_option_event(&pool, &input)
        .await
        .expect("first");
    let second = correlate_canonical_option_event(&pool, &input)
        .await
        .expect("replay");
    assert!(matches!(
        second,
        CorrelationReducerOutcome::AlreadyCorrelated(_)
    ));
}

// ---------------- A5 execution_kind mismatch ------------------------

#[tokio::test]
async fn r05_reducer_conflict_on_execution_kind_mismatch() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r05kindmismatch");
    // Correlation says RFQ; event claims Trade.
    seed_awaiting(&pool, &cid, OptionExecutionKind::RfqTrade).await;
    let tx = tx_hash(0x05);
    let bhash = block_hash(0x05);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 1,
        canonical_block_number: 5_000_004,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_500_000,
    };
    match correlate_canonical_option_event(&pool, &input)
        .await
        .expect("reducer")
    {
        CorrelationReducerOutcome::Conflict(row) => {
            assert_eq!(row.correlation_status, OptionCorrelationStatus::Conflict);
            assert!(row
                .terminal_reason
                .as_deref()
                .unwrap_or("")
                .contains("execution_kind"));
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
}

// ---------------- A5 tx_hash mismatch -------------------------------

#[tokio::test]
async fn r06_reducer_conflict_on_tx_hash_mismatch_against_pre_persisted() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r06txmismatch");
    seed_awaiting(&pool, &cid, OptionExecutionKind::Trade).await;
    let local_tx = tx_hash(0xAAAA);
    attach_local_tx_identity(&pool, &cid, &local_tx, 1_700_000_100_000)
        .await
        .expect("attach");
    let other_tx = tx_hash(0xBBBB);
    let bhash = block_hash(0x06);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &other_tx,
        log_index: 3,
        canonical_block_number: 5_000_005,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_600_000,
    };
    match correlate_canonical_option_event(&pool, &input)
        .await
        .expect("reducer")
    {
        CorrelationReducerOutcome::Conflict(row) => {
            assert_eq!(row.correlation_status, OptionCorrelationStatus::Conflict);
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
}

// ---------------- A5 fill_quantity mismatch -------------------------

#[tokio::test]
async fn r07_reducer_conflict_on_fill_quantity_mismatch() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r07qtymismatch");
    seed_awaiting(&pool, &cid, OptionExecutionKind::Trade).await;
    let tx = tx_hash(0x07);
    let bhash = block_hash(0x07);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 0,
        canonical_block_number: 5_000_006,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        // Pre-persisted was 100000000 (from seed_awaiting).
        fill_quantity_1e8: "999999999",
        now_ms: 1_700_000_700_000,
    };
    let outcome = correlate_canonical_option_event(&pool, &input)
        .await
        .expect("reducer");
    assert!(
        matches!(outcome, CorrelationReducerOutcome::Conflict(_)),
        "fill_quantity mismatch must escalate CONFLICT"
    );
}

// ---------------- A4 multi-event: second event same canonical id ----

#[tokio::test]
async fn r08_reducer_conflict_on_second_canonical_event() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r08secondevt");
    seed_awaiting(&pool, &cid, OptionExecutionKind::Trade).await;
    let tx1 = tx_hash(0x08);
    let bhash = block_hash(0x08);
    let first = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx1,
        log_index: 0,
        canonical_block_number: 5_000_007,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_800_000,
    };
    correlate_canonical_option_event(&pool, &first)
        .await
        .expect("first promote");
    // A different tx/log claims the SAME canonical_execution_id.
    let tx2 = tx_hash(0x09);
    let second = CanonicalExecutionEventInput {
        tx_hash: &tx2,
        log_index: 5,
        ..first.clone()
    };
    match correlate_canonical_option_event(&pool, &second)
        .await
        .expect("reducer")
    {
        CorrelationReducerOutcome::Conflict(row) => {
            assert_eq!(row.correlation_status, OptionCorrelationStatus::Conflict);
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
}

// ---------------- A6 reorg / orphan / replacement -------------------

#[tokio::test]
async fn r09_reorg_transitions_correlated_to_orphaned_and_permits_replacement() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("r09reorg");
    seed_awaiting(&pool, &cid, OptionExecutionKind::Trade).await;
    let tx1 = tx_hash(0x10);
    let bhash1 = block_hash(0x10);
    let input1 = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx1,
        log_index: 1,
        canonical_block_number: 5_000_100,
        canonical_block_hash: &bhash1,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_001_000_000,
    };
    correlate_canonical_option_event(&pool, &input1)
        .await
        .expect("first promote");
    // Canonical chain reorgs — orphan the correlation.
    let orphaned = reorg_orphan_canonical_correlation(&pool, &cid, 1_700_001_100_000)
        .await
        .expect("orphan");
    assert_eq!(
        orphaned.correlation_status,
        OptionCorrelationStatus::Orphaned
    );
    // Sparse UNIQUE is ACTIVE-only → a fresh AWAITING row for the
    // same canonical_execution_id can be inserted (replacement).
    // Post-date the seed so `get_by_canonical_execution_id` (ORDER
    // BY last_updated_at_ms DESC) returns the replacement.
    seed_awaiting_at(&pool, &cid, OptionExecutionKind::Trade, 1_700_002_000_000).await;
    let active = get_by_canonical_execution_id(&pool, &cid)
        .await
        .expect("lookup")
        .expect("row");
    assert_eq!(
        active.correlation_status,
        OptionCorrelationStatus::AwaitingChainEvidence
    );
    // A replacement canonical event on the successor branch promotes
    // the new AWAITING to CORRELATED_CANONICAL.
    let tx2 = tx_hash(0x11);
    let bhash2 = block_hash(0x11);
    let input2 = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx2,
        log_index: 0,
        canonical_block_number: 5_000_200,
        canonical_block_hash: &bhash2,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_001_200_000,
    };
    let outcome = correlate_canonical_option_event(&pool, &input2)
        .await
        .expect("reducer");
    assert!(matches!(outcome, CorrelationReducerOutcome::Promoted(_)));
}

// ---------------- A6 restart preserves promoted state ---------------

#[tokio::test]
async fn r10_restart_preserves_correlated_canonical_row() {
    let Some(url) = require_pg_url() else {
        return;
    };
    ensure_migrated(&url).await;
    let cid = canonical_id("r10restart");
    let pool_a = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("pool a");
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    seed_awaiting(&pool_a, &cid, OptionExecutionKind::Trade).await;
    let tx = tx_hash(0x12);
    let bhash = block_hash(0x12);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 2,
        canonical_block_number: 5_000_300,
        canonical_block_hash: &bhash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_001_300_000,
    };
    correlate_canonical_option_event(&pool_a, &input)
        .await
        .expect("promote");
    drop(pool_a);
    // Fresh pool — simulate restart.
    let pool_b = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("pool b");
    let row = get_by_canonical_execution_id(&pool_b, &cid)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::CorrelatedCanonical
    );
    assert_eq!(row.tx_hash.as_deref(), Some(tx.as_str()));
    assert_eq!(row.log_index, Some(2));
    // Reducer replay after restart is idempotent.
    let outcome = correlate_canonical_option_event(&pool_b, &input)
        .await
        .expect("replay");
    assert!(matches!(
        outcome,
        CorrelationReducerOutcome::AlreadyCorrelated(_)
    ));
}
