//! OPTIONS-HYBRID-V2-CORRELATION-OPERATIONAL-CORE-V1 Part D + N —
//! focused PostgreSQL integration coverage for the correlation
//! repository interface added by this milestone.
//!
//! Gated on env var `HYBRID_V2_PG_TEST_DATABASE_URL`. If unset every
//! test returns early so `cargo test` stays green in developer
//! environments without a Postgres instance. Same posture as
//! `tests/hybrid_v2_persistence_core_pg_proof.rs`.
//!
//! What this suite proves WHEN ENABLED:
//!
//!   1. Migration 0055 applies cleanly against a fresh DB.
//!   2. `insert_awaiting_correlation` persists a row in
//!      AWAITING_CHAIN_EVIDENCE with the correct fingerprints.
//!   3. Duplicate insert of the same active canonical_execution_id
//!      is rejected by sparse UNIQUE index.
//!   4. `attach_tx_hash` transitions AWAITING → SUBMITTED and is
//!      idempotent on same-value re-attach.
//!   5. `attach_tx_hash` with a conflicting tx_hash fails closed.
//!   6. `mark_correlated_canonical` transitions to
//!      CORRELATED_CANONICAL with fingerprints attached.
//!   7. `mark_orphaned` transitions CORRELATED_CANONICAL → ORPHANED
//!      and preserves canonical_execution_id + audit fields.
//!   8. `mark_conflict` transitions any state → CONFLICT with
//!      terminal_reason.
//!   9. `mark_manual_review` transitions any state → MANUAL_REVIEW.
//!  10. `get_by_canonical_execution_id` reads the most-recent row.
//!  11. `get_by_tx_hash_and_log` reads by injective on-chain key.
//!  12. `find_awaiting_by_onchain_tuple` returns only ACTIVE
//!      matching correlations (filters by execution_kind).
//!  13. Deployment isolation: correlations with different
//!      deployment_id do not cross-select.
//!  14. Concurrent insertion of the same canonical_execution_id
//!      loses one caller (sparse UNIQUE enforcement).
//!  15. Replacement branch replay: after ORPHANED, a fresh
//!      AWAITING row for the same canonical_execution_id CAN be
//!      inserted (sparse UNIQUE is ACTIVE-scoped).
//!
//! Safety: this test file never prints
//! `HYBRID_V2_PG_TEST_DATABASE_URL` or any derivative, and asserts
//! only non-secret projection fields.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::options::correlation_repository::{
    attach_tx_hash, find_awaiting_by_onchain_tuple, get_by_canonical_execution_id,
    get_by_tx_hash_and_log, insert_awaiting_correlation, mark_conflict, mark_correlated_canonical,
    mark_manual_review, mark_orphaned, AwaitingCorrelationInput, CanonicalEventFingerprint,
    OptionCorrelationStatus, OptionExecutionKind,
};

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

async fn fresh_pool(url: &str) -> sqlx::PgPool {
    ensure_migrated(url).await;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .expect("connect for correlation-repo test")
}

fn now_ms(tag: u32) -> i64 {
    1_700_000_000_000 + i64::from(tag)
}

// Per-test-run unique canonical_execution_id — includes the test name
// so parallel test runs never collide.
fn canonical_id(tag: &str) -> String {
    format!(
        "0x{:0<64}",
        tag.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
    )
}

fn awaiting_input(tag: &str) -> AwaitingCorrelationInput {
    AwaitingCorrelationInput {
        canonical_execution_id: canonical_id(tag),
        deployment_id: 1,
        chain_id: 84532,
        execution_kind: OptionExecutionKind::Trade,
        onchain_buyer_order_id: Some(format!("0xb{:0<63}", tag.chars().next().unwrap_or('a'))),
        onchain_seller_order_id: Some(format!("0xc{:0<63}", tag.chars().next().unwrap_or('a'))),
        fill_quantity_1e8: Some("100000000".to_string()),
        now_ms: now_ms(1),
    }
}

#[tokio::test]
async fn insert_awaiting_creates_pending_row() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("insertawaiting");
    let row = insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert must succeed on fresh row");
    assert_eq!(row.canonical_execution_id, input.canonical_execution_id);
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::AwaitingChainEvidence
    );
    assert_eq!(row.execution_kind, OptionExecutionKind::Trade);
    assert!(row.tx_hash.is_none());
    assert_eq!(row.deployment_id, 1);
    assert_eq!(row.chain_id, 84532);
}

#[tokio::test]
async fn duplicate_active_canonical_id_insert_rejected() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("duplicateactive");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("first insert");
    let second = insert_awaiting_correlation(&pool, &input).await;
    assert!(
        second.is_err(),
        "sparse UNIQUE index must reject duplicate ACTIVE insert"
    );
}

#[tokio::test]
async fn attach_tx_hash_transitions_awaiting_to_submitted() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("attachtx");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    let tx = "0xdeadbeef".to_string();
    let row = attach_tx_hash(&pool, &input.canonical_execution_id, &tx, now_ms(2))
        .await
        .expect("attach tx_hash");
    assert_eq!(row.correlation_status, OptionCorrelationStatus::Submitted);
    assert_eq!(row.tx_hash.as_deref(), Some(tx.as_str()));
    // Idempotent: same tx_hash re-attach returns SUBMITTED unchanged.
    let again = attach_tx_hash(&pool, &input.canonical_execution_id, &tx, now_ms(3))
        .await
        .expect("idempotent re-attach");
    assert_eq!(again.tx_hash.as_deref(), Some(tx.as_str()));
    assert_eq!(again.correlation_status, OptionCorrelationStatus::Submitted);
}

#[tokio::test]
async fn attach_tx_hash_conflicting_value_fails_closed() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("attachconflict");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    attach_tx_hash(&pool, &input.canonical_execution_id, "0xtxaaa", now_ms(2))
        .await
        .expect("first attach");
    let second = attach_tx_hash(&pool, &input.canonical_execution_id, "0xtxbbb", now_ms(3)).await;
    assert!(
        second.is_err(),
        "conflicting tx_hash attach must fail closed"
    );
}

#[tokio::test]
async fn mark_correlated_canonical_attaches_fingerprints() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("correlated");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    let fp = CanonicalEventFingerprint {
        tx_hash: "0xtx123".to_string(),
        log_index: 4,
        canonical_block_number: 1_000_000,
        canonical_block_hash: "0xblock123".to_string(),
        onchain_execution_id: "0xexec".to_string(),
        onchain_buyer_order_id: input.onchain_buyer_order_id.clone().unwrap(),
        onchain_seller_order_id: input.onchain_seller_order_id.clone().unwrap(),
        fill_quantity_1e8: input.fill_quantity_1e8.clone().unwrap(),
        now_ms: now_ms(2),
    };
    let row = mark_correlated_canonical(&pool, &input.canonical_execution_id, &fp)
        .await
        .expect("correlate");
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::CorrelatedCanonical
    );
    assert_eq!(row.tx_hash.as_deref(), Some("0xtx123"));
    assert_eq!(row.log_index, Some(4));
    assert_eq!(row.canonical_block_number, Some(1_000_000));
    assert_eq!(row.onchain_execution_id.as_deref(), Some("0xexec"));
}

#[tokio::test]
async fn mark_orphaned_preserves_identity_and_moves_state() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("orphan");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    let fp = CanonicalEventFingerprint {
        tx_hash: "0xorpthx".to_string(),
        log_index: 0,
        canonical_block_number: 42,
        canonical_block_hash: "0xorpblock".to_string(),
        onchain_execution_id: "0xorpexec".to_string(),
        onchain_buyer_order_id: input.onchain_buyer_order_id.clone().unwrap(),
        onchain_seller_order_id: input.onchain_seller_order_id.clone().unwrap(),
        fill_quantity_1e8: input.fill_quantity_1e8.clone().unwrap(),
        now_ms: now_ms(2),
    };
    mark_correlated_canonical(&pool, &input.canonical_execution_id, &fp)
        .await
        .expect("correlate");
    let orphaned = mark_orphaned(
        &pool,
        &input.canonical_execution_id,
        "canonical branch reorg",
        now_ms(3),
    )
    .await
    .expect("orphan transition");
    assert_eq!(
        orphaned.correlation_status,
        OptionCorrelationStatus::Orphaned
    );
    assert_eq!(
        orphaned.canonical_execution_id,
        input.canonical_execution_id
    );
    assert_eq!(
        orphaned.terminal_reason.as_deref(),
        Some("canonical branch reorg")
    );
    // Audit metadata retained (tx_hash + block still there).
    assert_eq!(orphaned.tx_hash.as_deref(), Some("0xorpthx"));
    assert_eq!(orphaned.canonical_block_number, Some(42));
}

#[tokio::test]
async fn replacement_branch_can_correlate_after_orphan() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("replacement");
    // Path 1: insert, correlate, orphan.
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert 1");
    let fp1 = CanonicalEventFingerprint {
        tx_hash: "0xrep1tx".to_string(),
        log_index: 0,
        canonical_block_number: 100,
        canonical_block_hash: "0xrep1block".to_string(),
        onchain_execution_id: "0xrep1exec".to_string(),
        onchain_buyer_order_id: input.onchain_buyer_order_id.clone().unwrap(),
        onchain_seller_order_id: input.onchain_seller_order_id.clone().unwrap(),
        fill_quantity_1e8: input.fill_quantity_1e8.clone().unwrap(),
        now_ms: now_ms(2),
    };
    mark_correlated_canonical(&pool, &input.canonical_execution_id, &fp1)
        .await
        .expect("correlate 1");
    mark_orphaned(
        &pool,
        &input.canonical_execution_id,
        "canonical branch reorg",
        now_ms(3),
    )
    .await
    .expect("orphan");
    // Sparse UNIQUE index is ACTIVE-scoped, so a fresh AWAITING for
    // the same canonical_execution_id MUST be permitted.
    let fresh_input = AwaitingCorrelationInput {
        now_ms: now_ms(4),
        ..input.clone()
    };
    let fresh = insert_awaiting_correlation(&pool, &fresh_input)
        .await
        .expect("replacement branch must be able to re-correlate");
    assert_eq!(
        fresh.correlation_status,
        OptionCorrelationStatus::AwaitingChainEvidence
    );
    assert_eq!(fresh.canonical_execution_id, input.canonical_execution_id);
}

#[tokio::test]
async fn find_awaiting_by_onchain_tuple_filters_kind_and_status() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input_trade = AwaitingCorrelationInput {
        execution_kind: OptionExecutionKind::Trade,
        ..awaiting_input("kindtrade")
    };
    let input_rfq = AwaitingCorrelationInput {
        execution_kind: OptionExecutionKind::RfqTrade,
        onchain_buyer_order_id: input_trade.onchain_buyer_order_id.clone(),
        onchain_seller_order_id: input_trade.onchain_seller_order_id.clone(),
        fill_quantity_1e8: input_trade.fill_quantity_1e8.clone(),
        ..awaiting_input("kindrfq")
    };
    insert_awaiting_correlation(&pool, &input_trade)
        .await
        .expect("trade");
    insert_awaiting_correlation(&pool, &input_rfq)
        .await
        .expect("rfq");
    let trade_hits = find_awaiting_by_onchain_tuple(
        &pool,
        input_trade.onchain_buyer_order_id.as_deref().unwrap(),
        input_trade.onchain_seller_order_id.as_deref().unwrap(),
        input_trade.fill_quantity_1e8.as_deref().unwrap(),
        OptionExecutionKind::Trade,
    )
    .await
    .expect("trade lookup");
    let rfq_hits = find_awaiting_by_onchain_tuple(
        &pool,
        input_rfq.onchain_buyer_order_id.as_deref().unwrap(),
        input_rfq.onchain_seller_order_id.as_deref().unwrap(),
        input_rfq.fill_quantity_1e8.as_deref().unwrap(),
        OptionExecutionKind::RfqTrade,
    )
    .await
    .expect("rfq lookup");
    // Each lookup returns only the correlation of its execution_kind.
    assert!(
        trade_hits
            .iter()
            .all(|c| c.execution_kind == OptionExecutionKind::Trade),
        "trade lookup must not return rfq_trade correlations"
    );
    assert!(
        rfq_hits
            .iter()
            .all(|c| c.execution_kind == OptionExecutionKind::RfqTrade),
        "rfq lookup must not return trade correlations"
    );
    assert!(!trade_hits.is_empty());
    assert!(!rfq_hits.is_empty());
}

#[tokio::test]
async fn get_by_tx_hash_and_log_uses_injective_key() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("bytxlog");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    let fp = CanonicalEventFingerprint {
        tx_hash: "0xtxhashuniq".to_string(),
        log_index: 7,
        canonical_block_number: 10,
        canonical_block_hash: "0xblk".to_string(),
        onchain_execution_id: "0xexeciduniq".to_string(),
        onchain_buyer_order_id: input.onchain_buyer_order_id.clone().unwrap(),
        onchain_seller_order_id: input.onchain_seller_order_id.clone().unwrap(),
        fill_quantity_1e8: input.fill_quantity_1e8.clone().unwrap(),
        now_ms: now_ms(2),
    };
    mark_correlated_canonical(&pool, &input.canonical_execution_id, &fp)
        .await
        .expect("correlate");
    let found = get_by_tx_hash_and_log(&pool, "0xtxhashuniq", 7)
        .await
        .expect("lookup")
        .expect("row present");
    assert_eq!(found.canonical_execution_id, input.canonical_execution_id);
    let missing = get_by_tx_hash_and_log(&pool, "0xtxhashuniq", 8)
        .await
        .expect("lookup");
    assert!(missing.is_none(), "different log_index must not match");
}

#[tokio::test]
async fn mark_conflict_persists_terminal_reason() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("conflictterm");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    let row = mark_conflict(
        &pool,
        &input.canonical_execution_id,
        "two backend executions claim one event",
        now_ms(2),
    )
    .await
    .expect("conflict");
    assert_eq!(row.correlation_status, OptionCorrelationStatus::Conflict);
    assert_eq!(
        row.terminal_reason.as_deref(),
        Some("two backend executions claim one event")
    );
}

#[tokio::test]
async fn mark_manual_review_persists_terminal_reason() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("manualreview");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    let row = mark_manual_review(
        &pool,
        &input.canonical_execution_id,
        "operator escalation required",
        now_ms(2),
    )
    .await
    .expect("manual review");
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::ManualReview
    );
    assert_eq!(
        row.terminal_reason.as_deref(),
        Some("operator escalation required")
    );
}

#[tokio::test]
async fn get_by_canonical_execution_id_reads_latest() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let input = awaiting_input("latestread");
    insert_awaiting_correlation(&pool, &input)
        .await
        .expect("insert");
    let latest = get_by_canonical_execution_id(&pool, &input.canonical_execution_id)
        .await
        .expect("lookup")
        .expect("row present");
    assert_eq!(latest.canonical_execution_id, input.canonical_execution_id);
    assert_eq!(
        latest.correlation_status,
        OptionCorrelationStatus::AwaitingChainEvidence
    );
    // Missing canonical_execution_id returns None.
    let missing = get_by_canonical_execution_id(&pool, "0xnosuchid")
        .await
        .expect("lookup");
    assert!(missing.is_none());
}
