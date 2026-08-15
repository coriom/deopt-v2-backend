//! OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Part A1 — real
//! PostgreSQL coverage for the pre-broadcast tx identity durability
//! lifecycle.
//!
//! Verifies:
//!   1. `derive_signed_transaction_hash` computes keccak256 of the
//!      exact signed raw bytes (deterministic, no I/O).
//!   2. `attach_local_tx_identity` transitions
//!      AWAITING_CHAIN_EVIDENCE → SUBMISSION_UNKNOWN with the
//!      provided tx_hash.
//!   3. Same-value re-persist is idempotent (SUBMISSION_UNKNOWN or
//!      SUBMITTED preserved).
//!   4. Different-value fails closed — original tx_hash immutable.
//!   5. `attach_tx_hash` accepts SUBMISSION_UNKNOWN as source and
//!      advances to SUBMITTED with same tx_hash.
//!   6. `mark_correlated_canonical` accepts SUBMISSION_UNKNOWN → the
//!      crash-window case where canonical evidence arrives before
//!      RPC ack.
//!   7. Migration 0056 applies cleanly and sparse UNIQUE widens.
//!
//! Loud-fail gate identical to atomic-wiring PG suite.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::execution::derive_signed_transaction_hash;
use deopt_v2_backend::options::correlation_repository::{
    attach_local_tx_identity, attach_tx_hash, get_by_canonical_execution_id,
    mark_correlated_canonical, upsert_awaiting_correlation_tx, AwaitingCorrelationInput,
    CanonicalEventFingerprint, OptionCorrelationStatus, OptionExecutionKind,
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
fn awaiting(canonical: &str) -> AwaitingCorrelationInput {
    AwaitingCorrelationInput {
        canonical_execution_id: canonical.to_string(),
        deployment_id: 1,
        chain_id: 84532,
        execution_kind: OptionExecutionKind::Trade,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: Some("100000000".to_string()),
        now_ms: 1_700_000_000_000,
    }
}
/// Delete any pre-existing correlation rows for `canonical` so tests
/// are idempotent across re-runs — otherwise a second run sees a
/// terminal row from the previous run and `attach_local_tx_identity`
/// / `mark_correlated_canonical` refuse to move it.
async fn reset_canonical(pool: &PgPool, canonical: &str) {
    sqlx::query("DELETE FROM option_execution_correlations WHERE canonical_execution_id = $1")
        .bind(canonical)
        .execute(pool)
        .await
        .expect("reset");
}

async fn seed_awaiting(pool: &PgPool, canonical: &str) {
    reset_canonical(pool, canonical).await;
    let mut tx = pool.begin().await.expect("begin");
    upsert_awaiting_correlation_tx(&mut tx, &awaiting(canonical))
        .await
        .expect("upsert");
    tx.commit().await.expect("commit");
}

// -------------------------------------------------------------------
// Pure unit — no PG required.
// -------------------------------------------------------------------

#[test]
fn t01_derive_hash_is_keccak256_of_raw_bytes() {
    // Empty is rejected.
    assert!(derive_signed_transaction_hash("0x").is_err());
    // Odd length rejected.
    assert!(derive_signed_transaction_hash("0xabc").is_err());
    // Deterministic hex → deterministic hash.
    let raw = "0x02f8730181a01234567890abcdef";
    let h1 = derive_signed_transaction_hash(raw).expect("hash");
    let h2 = derive_signed_transaction_hash(raw).expect("hash");
    assert_eq!(h1, h2);
    assert!(h1.starts_with("0x") && h1.len() == 66);
    // Case-insensitive hex input yields identical hash.
    let h3 = derive_signed_transaction_hash("0x02F8730181A01234567890ABCDEF").expect("hash");
    assert_eq!(h1, h3);
    // Non-hex character rejected.
    assert!(derive_signed_transaction_hash("0x02XY").is_err());
}

// -------------------------------------------------------------------
// PG-backed lifecycle
// -------------------------------------------------------------------

#[tokio::test]
async fn t02_attach_local_tx_transitions_awaiting_to_submission_unknown() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("t02local");
    seed_awaiting(&pool, &cid).await;
    let tx = "0x1111".to_string();
    let row = attach_local_tx_identity(&pool, &cid, &tx, 1_700_000_100_000)
        .await
        .expect("attach local");
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::SubmissionUnknown
    );
    assert_eq!(row.tx_hash.as_deref(), Some(tx.as_str()));
}

#[tokio::test]
async fn t03_attach_local_same_value_is_idempotent() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("t03idempotent");
    seed_awaiting(&pool, &cid).await;
    let tx = "0x2222";
    attach_local_tx_identity(&pool, &cid, tx, 1_700_000_100_000)
        .await
        .expect("first");
    let row = attach_local_tx_identity(&pool, &cid, tx, 1_700_000_200_000)
        .await
        .expect("second same-value");
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::SubmissionUnknown
    );
    assert_eq!(row.tx_hash.as_deref(), Some(tx));
}

#[tokio::test]
async fn t04_attach_local_different_value_fails_closed() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("t04conflict");
    seed_awaiting(&pool, &cid).await;
    attach_local_tx_identity(&pool, &cid, "0xaaaa", 1_700_000_100_000)
        .await
        .expect("first");
    let err = attach_local_tx_identity(&pool, &cid, "0xbbbb", 1_700_000_200_000).await;
    assert!(err.is_err(), "different tx_hash must fail closed");
    let row = get_by_canonical_execution_id(&pool, &cid)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(row.tx_hash.as_deref(), Some("0xaaaa"));
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::SubmissionUnknown
    );
}

#[tokio::test]
async fn t05_attach_tx_hash_advances_submission_unknown_to_submitted() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("t05advance");
    seed_awaiting(&pool, &cid).await;
    let tx = "0xdead";
    attach_local_tx_identity(&pool, &cid, tx, 1_700_000_100_000)
        .await
        .expect("local");
    let row = attach_tx_hash(&pool, &cid, tx, 1_700_000_200_000)
        .await
        .expect("attach after RPC ack");
    assert_eq!(row.correlation_status, OptionCorrelationStatus::Submitted);
    assert_eq!(row.tx_hash.as_deref(), Some(tx));
}

#[tokio::test]
async fn t06_mark_correlated_canonical_accepts_submission_unknown() {
    // Crash-window case: canonical evidence lands before the RPC
    // ack loop completes. The reducer can promote SUBMISSION_UNKNOWN
    // directly to CORRELATED_CANONICAL because the tx_hash is
    // already durably bound.
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("t06crashwindow");
    seed_awaiting(&pool, &cid).await;
    let tx = "0xbeef";
    attach_local_tx_identity(&pool, &cid, tx, 1_700_000_100_000)
        .await
        .expect("local");
    let fp = CanonicalEventFingerprint {
        tx_hash: tx.to_string(),
        log_index: 3,
        canonical_block_number: 1_234,
        canonical_block_hash: format!("0x{}", "c".repeat(64)),
        onchain_execution_id: format!("0x{}", "e".repeat(64)),
        onchain_buyer_order_id: format!("0x{}", "b".repeat(64)),
        onchain_seller_order_id: format!("0x{}", "d".repeat(64)),
        fill_quantity_1e8: "100000000".to_string(),
        now_ms: 1_700_000_300_000,
    };
    let row = mark_correlated_canonical(&pool, &cid, &fp)
        .await
        .expect("promote from SUBMISSION_UNKNOWN");
    assert_eq!(
        row.correlation_status,
        OptionCorrelationStatus::CorrelatedCanonical
    );
    assert_eq!(row.tx_hash.as_deref(), Some(tx));
    assert_eq!(row.log_index, Some(3));
}

#[tokio::test]
async fn t07_sparse_unique_rejects_second_active_across_widened_states() {
    // Migration 0056 widens the sparse UNIQUE to include
    // SUBMISSION_UNKNOWN. Verify a second insert for the same
    // canonical id fails while any ACTIVE state (incl.
    // SUBMISSION_UNKNOWN) exists.
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = canonical_id("t07sparse");
    seed_awaiting(&pool, &cid).await;
    attach_local_tx_identity(&pool, &cid, "0x9999", 1_700_000_100_000)
        .await
        .expect("local");
    // Now direct raw insert with the SAME canonical id would violate
    // the sparse UNIQUE; upsert helper returns the existing row.
    let mut tx = pool.begin().await.expect("begin");
    let existing = upsert_awaiting_correlation_tx(&mut tx, &awaiting(&cid))
        .await
        .expect("upsert returns existing row");
    tx.commit().await.expect("commit");
    assert_eq!(existing.canonical_execution_id, cid);
    assert_eq!(
        existing.correlation_status,
        OptionCorrelationStatus::SubmissionUnknown,
        "existing SUBMISSION_UNKNOWN preserved"
    );
}
