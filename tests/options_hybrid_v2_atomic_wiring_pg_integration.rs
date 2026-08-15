//! OPTIONS-HYBRID-V2-CORRELATION-ATOMIC-WIRING-V1 Part K —
//! real-PostgreSQL matrix that proves the atomic
//! `insert_option_execution_intent_with_awaiting_correlation` +
//! `attach_tx_hash` wiring holds under every intended scenario.
//!
//! Loud-fail policy (per milestone brief):
//!   * The env var `OPTIONS_ATOMIC_WIRING_PG_URL` must be set to a
//!     writable PostgreSQL 16 database URL. If absent, `require_pool`
//!     panics with a clear operator message.
//!   * Any connection / migration failure panics — no test may
//!     silently skip when this file is invoked.
//!   * Set `OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP=1` to permit skips
//!     (used only for dev environments without a disposable PG). CI
//!     and milestone verdict runs MUST NOT set that flag.
//!
//! Every test prints `REAL_POSTGRES_CONNECTION_CONFIRMED` on success
//! to make the verdict evidence machine-scannable.
//!
//! Coverage (26 cases split into 5 groups):
//!   Atomic persistence (8): 1-8
//!   Prechain fingerprint (6): 9-14
//!   Tx attachment (6): 15-20
//!   Restart (3): 21-23
//!   Concurrency (3): 24-26
//!
//! Safety: no broadcast, no chain, no signer — repository writes only.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::options::correlation_repository::{
    attach_tx_hash, get_by_canonical_execution_id, upsert_awaiting_correlation_tx,
    AwaitingCorrelationInput, OptionCorrelationStatus, OptionExecutionKind,
};
use deopt_v2_backend::options::{
    OptionExecutionIntent, OptionExecutionIntentId, OptionExecutionIntentStatus,
    OptionExecutionSourceType,
};
use deopt_v2_backend::types::AccountId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

// ---------------------------------------------------------------
// Environment / connection plumbing
// ---------------------------------------------------------------

const URL_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_URL";
const SKIP_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP";

fn env_pg_url() -> Option<String> {
    std::env::var(URL_ENV).ok().filter(|v| !v.is_empty())
}

fn allow_skip() -> bool {
    std::env::var(SKIP_ENV).ok().as_deref() == Some("1")
}

/// Fetch a real Postgres URL or panic. Returns `None` ONLY when
/// `OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP=1` is set — every other
/// invocation MUST have a URL. This is the "fail loudly" gate.
fn require_pg_url() -> Option<String> {
    match env_pg_url() {
        Some(u) => Some(u),
        None if allow_skip() => None,
        None => panic!(
            "{URL_ENV} is not set. This milestone's PG verdict REQUIRES a real disposable \
             PostgreSQL 16 database. Set {URL_ENV}=postgres://... to run these tests. \
             (Dev-only opt-out: {SKIP_ENV}=1 skips instead of panicking — never use in CI.)"
        ),
    }
}

const SHARED_SERIES_ID: &str = "atomic-wiring-shared-series";

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
            // Seed the shared option_series row referenced by every
            // intent fixture in this file (FK dependency on
            // option_series.option_series_id).
            ensure_option_series(repo.pool(), SHARED_SERIES_ID).await;
        })
        .await;
}

async fn fresh_pool(url: &str) -> PgPool {
    ensure_migrated(url).await;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .expect("connect for atomic-wiring test")
}

async fn require_pool() -> Option<(PgPool, PgRepository)> {
    let url = require_pg_url()?;
    let pool = fresh_pool(&url).await;
    let repo = PgRepository::connect(&url)
        .await
        .expect("PgRepository connect");
    println!(
        "REAL_POSTGRES_CONNECTION_CONFIRMED url_hash={:x}",
        stable_hash(&url)
    );
    Some((pool, repo))
}

// Stable non-secret hash of URL for the confirmation log line — never
// prints the URL itself so no password leaks into CI output.
fn stable_hash(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------

// Every test tag becomes a canonical-id + source-id + intent-id
// derivation so parallel tests never collide.

fn canonical_id(tag: &str) -> String {
    // hex-formatted: 0x + 62 tag-derived chars padded to 64 hex chars
    let mut sanitised: String = tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(60)
        .collect();
    while sanitised.len() < 62 {
        sanitised.push('a');
    }
    format!("0x{sanitised:0<64}")
}

fn intent_source_id(tag: &str, salt: u32) -> String {
    format!("{tag}-{salt}")
}

fn awaiting_input(tag: &str) -> AwaitingCorrelationInput {
    AwaitingCorrelationInput {
        canonical_execution_id: canonical_id(tag),
        deployment_id: 1,
        chain_id: 84532,
        execution_kind: OptionExecutionKind::Trade,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: Some("100000000".to_string()),
        now_ms: 1_700_000_000_000,
    }
}

/// Minimal execution intent fixture that satisfies the schema. Only
/// fields that must be non-null / non-empty are filled explicitly.
fn intent_fixture(tag: &str, salt: u32, canonical: Option<String>) -> OptionExecutionIntent {
    OptionExecutionIntent {
        intent_id: OptionExecutionIntentId::from(Uuid::new_v4()),
        onchain_intent_id: format!("0x{:0<64}", format!("{tag}{salt}").as_str()),
        source_type: OptionExecutionSourceType::OptionOrderbookFill,
        source_id: intent_source_id(tag, salt),
        option_series_id: SHARED_SERIES_ID.to_string(),
        onchain_option_id: "1".to_string(),
        buyer: AccountId::new("0x00000000000000000000000000000000000000a1"),
        seller: AccountId::new("0x00000000000000000000000000000000000000b2"),
        underlying: AccountId::new("0x0000000000000000000000000000000000000010"),
        settlement_asset: AccountId::new("0x0000000000000000000000000000000000000020"),
        expiry: 2_000_000_000,
        strike_1e8: 300_000_000_000,
        is_call: true,
        contract_size_1e8: 100_000_000,
        quantity_contracts: 1,
        source_size_1e8: 100_000_000,
        source_price_1e8: 1_000_000_000,
        premium_per_contract_native: 1_000_000_000,
        buyer_is_maker: false,
        buyer_nonce: Some(salt as u128),
        seller_nonce: Some(salt as u128 + 1),
        deadline: 3_000_000_000,
        buyer_signature: None,
        seller_signature: None,
        calldata: None,
        status: OptionExecutionIntentStatus::SignaturesRequired,
        error: None,
        simulation_status: None,
        simulation_error: None,
        simulation_block_number: None,
        simulation_revert_data: None,
        simulation_revert_selector: None,
        simulated_at_ms: None,
        canonical_execution_id: canonical,
        created_at_ms: 1_700_000_000_000,
        updated_at_ms: 1_700_000_000_000,
    }
}

/// Insert the option_series row that the intent's foreign-key
/// reference points at. Idempotent (per-tag key).
async fn ensure_option_series(pool: &PgPool, series_id: &str) {
    sqlx::query(
        "INSERT INTO option_series (
             option_series_id, underlying, base_asset, quote_asset, settlement_asset,
             expiry, strike_1e8, is_call, contract_size_1e8, status, source,
             onchain_product_id, onchain_series_id, created_at_ms, updated_at_ms
         ) VALUES (
             $1, '0x0000000000000000000000000000000000000010', 'ETH', 'USDC',
             '0x0000000000000000000000000000000000000020', 2000000000, '300000000000',
             true, '100000000', 'active', 'operator', NULL, NULL,
             1700000000000, 1700000000000
         )
         ON CONFLICT (option_series_id) DO NOTHING",
    )
    .bind(series_id)
    .execute(pool)
    .await
    .expect("ensure_option_series insert");
}

async fn count_active_correlations(pool: &PgPool, canonical_execution_id: &str) -> i64 {
    let row = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_execution_correlations
         WHERE canonical_execution_id = $1
           AND correlation_status IN
               ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED', 'CORRELATED_CANONICAL')",
    )
    .bind(canonical_execution_id)
    .fetch_one(pool)
    .await
    .expect("count active correlations");
    row.try_get::<i64, _>("c").expect("count")
}

async fn count_intents_by_source(pool: &PgPool, source_id: &str) -> i64 {
    let row = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_execution_intents WHERE source_id = $1",
    )
    .bind(source_id)
    .fetch_one(pool)
    .await
    .expect("count intents");
    row.try_get::<i64, _>("c").expect("count")
}

// ---------------------------------------------------------------
// GROUP 1 — atomic persistence (8 cases)
// ---------------------------------------------------------------

#[tokio::test]
async fn c01_atomic_intent_and_correlation_commit_together() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c01atomic";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 1, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let (stored_intent, stored_corr) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("atomic insert must succeed");
    assert_eq!(
        stored_intent.canonical_execution_id.as_deref(),
        Some(canonical.as_str())
    );
    assert_eq!(stored_corr.canonical_execution_id, canonical);
    assert_eq!(
        stored_corr.correlation_status,
        OptionCorrelationStatus::AwaitingChainEvidence
    );
    assert_eq!(count_intents_by_source(&pool, &intent.source_id).await, 1);
    assert_eq!(count_active_correlations(&pool, &canonical).await, 1);
}

#[tokio::test]
async fn c02_precondition_mismatch_aborts_before_write() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c02precondition";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 2, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical_id("c02otherid"), // MISMATCH
        ..awaiting_input(tag)
    };
    let err = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect_err("mismatch must abort");
    assert!(
        format!("{err}").contains("does not match correlation input"),
        "precondition guard message"
    );
    assert_eq!(count_intents_by_source(&pool, &intent.source_id).await, 0);
    assert_eq!(count_active_correlations(&pool, &canonical).await, 0);
}

#[tokio::test]
async fn c03_duplicate_atomic_call_is_idempotent() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c03duplicate";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 3, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let (first_intent, first_corr) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("first insert");
    let (second_intent, second_corr) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("duplicate call must succeed idempotently");
    assert_eq!(first_intent.intent_id, second_intent.intent_id);
    assert_eq!(first_corr.correlation_id, second_corr.correlation_id);
    assert_eq!(count_intents_by_source(&pool, &intent.source_id).await, 1);
    assert_eq!(count_active_correlations(&pool, &canonical).await, 1);
}

#[tokio::test]
async fn c04_missing_canonical_id_is_rejected() {
    let Some((_pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c04nocanonical";
    let intent = intent_fixture(tag, 4, None);
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical_id(tag),
        ..awaiting_input(tag)
    };
    let err = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect_err("missing canonical id must abort");
    assert!(format!("{err}").contains("does not match correlation input"));
}

#[tokio::test]
async fn c05_prior_active_correlation_returned_unchanged() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c05prioractive";
    let canonical = canonical_id(tag);
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    // Prior ACTIVE correlation (as if left by a prior atomic call
    // that succeeded and then crashed before the client observed the
    // response).
    {
        let mut tx = pool.begin().await.expect("begin");
        upsert_awaiting_correlation_tx(&mut tx, &corr_input)
            .await
            .expect("prior insert");
        tx.commit().await.expect("commit");
    }
    let intent = intent_fixture(tag, 5, Some(canonical.clone()));
    let (stored_intent, stored_corr) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("must succeed via idempotent upsert");
    assert_eq!(
        stored_intent.canonical_execution_id.as_deref(),
        Some(canonical.as_str())
    );
    assert_eq!(stored_corr.canonical_execution_id, canonical);
    assert_eq!(count_active_correlations(&pool, &canonical).await, 1);
    assert_eq!(count_intents_by_source(&pool, &intent.source_id).await, 1);
}

#[tokio::test]
async fn c06_prior_correlation_deployment_mismatch_fails_closed() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c06prioMismatch";
    let canonical = canonical_id(tag);
    // Existing correlation on deployment 1.
    let existing = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        deployment_id: 1,
        ..awaiting_input(tag)
    };
    {
        let mut tx = pool.begin().await.expect("begin");
        upsert_awaiting_correlation_tx(&mut tx, &existing)
            .await
            .expect("prior insert");
        tx.commit().await.expect("commit");
    }
    // Retry with deployment 2 → cross-deployment mismatch must abort.
    let intent = intent_fixture(tag, 6, Some(canonical.clone()));
    let retry_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        deployment_id: 2,
        ..awaiting_input(tag)
    };
    let err = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &retry_input)
        .await
        .expect_err("cross-deployment reuse must fail closed");
    assert!(format!("{err}").contains("deployment_id mismatch"));
    // Intent INSERT rolled back — no half-state persisted.
    assert_eq!(count_intents_by_source(&pool, &intent.source_id).await, 0);
    // Existing correlation untouched (still deployment 1).
    let corr = get_by_canonical_execution_id(&pool, &canonical)
        .await
        .expect("lookup")
        .expect("row exists");
    assert_eq!(corr.deployment_id, 1);
}

#[tokio::test]
async fn c07_legacy_intent_without_canonical_id_bypasses_atomic_path() {
    // Historical: pre-migration intents inserted through the plain
    // pool variant remain valid; they simply do not have a
    // correlation row. This test proves the non-atomic path is
    // preserved.
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c07legacy";
    let intent = intent_fixture(tag, 7, None);
    let stored = repo
        .insert_option_execution_intent(&intent)
        .await
        .expect("legacy insert");
    assert!(stored.canonical_execution_id.is_none());
    assert_eq!(count_intents_by_source(&pool, &intent.source_id).await, 1);
    // No correlation row was inserted.
    let corr = get_by_canonical_execution_id(&pool, &canonical_id(tag))
        .await
        .expect("lookup");
    assert!(corr.is_none());
}

#[tokio::test]
async fn c08_atomic_call_persists_execution_kind_from_input() {
    let Some((_pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c08executionkind";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 8, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        execution_kind: OptionExecutionKind::RfqTrade,
        ..awaiting_input(tag)
    };
    let (_intent, corr) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("atomic insert");
    assert_eq!(corr.execution_kind, OptionExecutionKind::RfqTrade);
}

// ---------------------------------------------------------------
// GROUP 2 — prechain fingerprint (6 cases)
// ---------------------------------------------------------------

#[tokio::test]
async fn c09_prechain_populates_deployment_and_chain() {
    let Some((_pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c09deploychain";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 9, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        deployment_id: 7,
        chain_id: 42_161,
        ..awaiting_input(tag)
    };
    let (_i, c) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    assert_eq!(c.deployment_id, 7);
    assert_eq!(c.chain_id, 42_161);
}

#[tokio::test]
async fn c10_prechain_populates_fill_quantity() {
    let Some((_pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c10fillquantity";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 10, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        fill_quantity_1e8: Some("2500000000".to_string()),
        ..awaiting_input(tag)
    };
    let (_i, c) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    assert_eq!(c.fill_quantity_1e8.as_deref(), Some("2500000000"));
}

#[tokio::test]
async fn c11_prechain_leaves_post_mine_fields_null() {
    let Some((_pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c11postminenull";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 11, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let (_i, c) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    assert!(c.tx_hash.is_none());
    assert!(c.canonical_block_number.is_none());
    assert!(c.canonical_block_hash.is_none());
    assert!(c.log_index.is_none());
    assert!(c.onchain_execution_id.is_none());
}

#[tokio::test]
async fn c12_prechain_envelope_ids_left_null_by_design() {
    let Some((_pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c12envelopenul";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 12, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        ..awaiting_input(tag)
    };
    let (_i, c) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    assert!(c.onchain_buyer_order_id.is_none());
    assert!(c.onchain_seller_order_id.is_none());
}

#[tokio::test]
async fn c13_canonical_execution_id_immutable_after_insert() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c13immutable";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 13, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let (_i, corr) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    // Direct UPDATE attempt on canonical_execution_id must be rejected
    // by the immutability trigger from migration 0055.
    let update = sqlx::query(
        "UPDATE option_execution_correlations SET canonical_execution_id = $2 \
         WHERE correlation_id = $1",
    )
    .bind(corr.correlation_id)
    .bind(canonical_id("c13other"))
    .execute(&pool)
    .await;
    assert!(update.is_err(), "immutability trigger must reject rename");
}

#[tokio::test]
async fn c14_intent_and_correlation_share_canonical_execution_id() {
    let Some((_pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c14sharecanonical";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 14, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let (i, c) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    assert_eq!(
        i.canonical_execution_id.as_deref(),
        Some(canonical.as_str())
    );
    assert_eq!(c.canonical_execution_id, canonical);
}

// ---------------------------------------------------------------
// GROUP 3 — tx attachment (6 cases)
// ---------------------------------------------------------------

#[tokio::test]
async fn c15_attach_tx_hash_first_time_transitions_to_submitted() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c15attachfirst";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 15, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    let tx = "0xdeadbeef".to_string();
    let after = attach_tx_hash(&pool, &canonical, &tx, 1_700_000_100_000)
        .await
        .expect("attach must succeed");
    assert_eq!(after.tx_hash.as_deref(), Some(tx.as_str()));
    assert_eq!(after.correlation_status, OptionCorrelationStatus::Submitted);
}

#[tokio::test]
async fn c16_attach_tx_hash_same_value_is_idempotent() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c16samevalue";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 16, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    let tx = "0xf00".to_string();
    attach_tx_hash(&pool, &canonical, &tx, 1_700_000_100_000)
        .await
        .expect("first attach");
    let second = attach_tx_hash(&pool, &canonical, &tx, 1_700_000_200_000)
        .await
        .expect("second attach with same value must succeed idempotently");
    assert_eq!(second.tx_hash.as_deref(), Some(tx.as_str()));
    assert_eq!(
        second.correlation_status,
        OptionCorrelationStatus::Submitted
    );
}

#[tokio::test]
async fn c17_attach_tx_hash_conflicting_value_fails_closed() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c17conflictvalue";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 17, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    attach_tx_hash(&pool, &canonical, "0xaaaa", 1_700_000_100_000)
        .await
        .expect("first attach");
    let err = attach_tx_hash(&pool, &canonical, "0xbbbb", 1_700_000_200_000)
        .await
        .expect_err("conflicting tx_hash must fail closed");
    assert!(format!("{err}").contains("no ACTIVE correlation"));
    // Original tx_hash unchanged.
    let corr = get_by_canonical_execution_id(&pool, &canonical)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(corr.tx_hash.as_deref(), Some("0xaaaa"));
}

#[tokio::test]
async fn c18_attach_tx_hash_unknown_canonical_id_fails() {
    let Some((pool, _repo)) = require_pool().await else {
        return;
    };
    let err = attach_tx_hash(
        &pool,
        &canonical_id("c18unknown"),
        "0xabc",
        1_700_000_100_000,
    )
    .await
    .expect_err("no such correlation must fail");
    assert!(format!("{err}").contains("no ACTIVE correlation"));
}

#[tokio::test]
async fn c19_attach_tx_hash_cross_deployment_isolation() {
    // Two correlations with the SAME tx_hash across different
    // deployments: allowed because deployment_id scopes identity.
    // The sparse UNIQUE index on `(tx_hash, log_index)` fires only
    // WHERE status = CORRELATED_CANONICAL — SUBMITTED rows may share
    // a tx_hash across deployments (rare cross-domain relayer share).
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag_a = "c19deploya";
    let tag_b = "c19deployb";
    let canonical_a = canonical_id(tag_a);
    let canonical_b = canonical_id(tag_b);
    let intent_a = intent_fixture(tag_a, 19, Some(canonical_a.clone()));
    let intent_b = intent_fixture(tag_b, 20, Some(canonical_b.clone()));
    let corr_a = AwaitingCorrelationInput {
        canonical_execution_id: canonical_a.clone(),
        deployment_id: 1,
        ..awaiting_input(tag_a)
    };
    let corr_b = AwaitingCorrelationInput {
        canonical_execution_id: canonical_b.clone(),
        deployment_id: 2,
        ..awaiting_input(tag_b)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent_a, &corr_a)
        .await
        .expect("insert a");
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent_b, &corr_b)
        .await
        .expect("insert b");
    let tx = "0xcafe".to_string();
    attach_tx_hash(&pool, &canonical_a, &tx, 1_700_000_100_000)
        .await
        .expect("attach a");
    attach_tx_hash(&pool, &canonical_b, &tx, 1_700_000_100_000)
        .await
        .expect("attach b — SUBMITTED rows may share tx_hash across deployments");
}

#[tokio::test]
async fn c20_attach_tx_hash_leaves_intent_untouched() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c20intentuntouched";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 21, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let (before, _) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    attach_tx_hash(&pool, &canonical, "0xffff", 1_700_000_300_000)
        .await
        .expect("attach");
    let after = repo
        .get_option_execution_intent(before.intent_id)
        .await
        .expect("lookup")
        .expect("intent exists");
    assert_eq!(after.canonical_execution_id, before.canonical_execution_id);
    assert_eq!(after.source_id, before.source_id);
}

// ---------------------------------------------------------------
// GROUP 4 — restart (3 cases)
// ---------------------------------------------------------------

#[tokio::test]
async fn c21_restart_after_awaiting_correlation_preserves_state() {
    // Simulate process restart by opening a fresh pool AFTER the
    // atomic insert. The correlation must still be visible unchanged.
    let Some(url) = require_pg_url() else {
        return;
    };
    ensure_migrated(&url).await;
    let repo = PgRepository::connect(&url).await.expect("connect");
    println!(
        "REAL_POSTGRES_CONNECTION_CONFIRMED url_hash={:x}",
        stable_hash(&url)
    );
    let tag = "c21restartawaiting";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 22, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    drop(repo);
    // Fresh pool — simulate restart.
    let repo2 = PgRepository::connect(&url).await.expect("reconnect");
    let corr = get_by_canonical_execution_id(repo2.pool(), &canonical)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(
        corr.correlation_status,
        OptionCorrelationStatus::AwaitingChainEvidence
    );
}

#[tokio::test]
async fn c22_restart_after_tx_attachment_preserves_tx_hash() {
    let Some(url) = require_pg_url() else {
        return;
    };
    ensure_migrated(&url).await;
    let repo = PgRepository::connect(&url).await.expect("connect");
    println!(
        "REAL_POSTGRES_CONNECTION_CONFIRMED url_hash={:x}",
        stable_hash(&url)
    );
    let tag = "c22restarttx";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 23, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    attach_tx_hash(repo.pool(), &canonical, "0xdead", 1_700_000_400_000)
        .await
        .expect("attach");
    drop(repo);
    let repo2 = PgRepository::connect(&url).await.expect("reconnect");
    let corr = get_by_canonical_execution_id(repo2.pool(), &canonical)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(corr.correlation_status, OptionCorrelationStatus::Submitted);
    assert_eq!(corr.tx_hash.as_deref(), Some("0xdead"));
}

#[tokio::test]
async fn c23_duplicate_after_restart_is_idempotent() {
    let Some(url) = require_pg_url() else {
        return;
    };
    ensure_migrated(&url).await;
    let repo = PgRepository::connect(&url).await.expect("connect");
    println!(
        "REAL_POSTGRES_CONNECTION_CONFIRMED url_hash={:x}",
        stable_hash(&url)
    );
    let tag = "c23dupafterrestart";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 24, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let (first, first_corr) = repo
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("first");
    drop(repo);
    let repo2 = PgRepository::connect(&url).await.expect("reconnect");
    let (second, second_corr) = repo2
        .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("duplicate after restart must succeed");
    assert_eq!(first.intent_id, second.intent_id);
    assert_eq!(first_corr.correlation_id, second_corr.correlation_id);
    assert_eq!(count_active_correlations(repo2.pool(), &canonical).await, 1);
}

// ---------------------------------------------------------------
// GROUP 5 — concurrency (3 cases)
// ---------------------------------------------------------------

#[tokio::test]
async fn c24_simultaneous_duplicate_intent_creation_serialises() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c24concurrentdup";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 25, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    let repo1 = repo.clone();
    let intent1 = intent.clone();
    let corr1 = corr_input.clone();
    let repo2 = repo.clone();
    let intent2 = intent.clone();
    let corr2 = corr_input.clone();
    let (a, b) = tokio::join!(
        repo1.insert_option_execution_intent_with_awaiting_correlation(&intent1, &corr1),
        repo2.insert_option_execution_intent_with_awaiting_correlation(&intent2, &corr2),
    );
    // At least one must succeed. If both succeed, they return the
    // same row via idempotent upsert; if one loses to a unique-key
    // race it retries via the SELECT path — but the sparse UNIQUE
    // arbitrates so exactly one row lands.
    a.expect("one path");
    b.expect("other path");
    assert_eq!(count_active_correlations(&pool, &canonical).await, 1);
    assert_eq!(count_intents_by_source(&pool, &intent.source_id).await, 1);
}

#[tokio::test]
async fn c25_simultaneous_same_tx_hash_attachments_serialise() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c25concurrenttx";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 26, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    let tx = "0xbeef".to_string();
    let pool1 = pool.clone();
    let pool2 = pool.clone();
    let canonical1 = canonical.clone();
    let canonical2 = canonical.clone();
    let tx1 = tx.clone();
    let tx2 = tx.clone();
    let (a, b) = tokio::join!(
        attach_tx_hash(&pool1, &canonical1, &tx1, 1_700_000_500_000),
        attach_tx_hash(&pool2, &canonical2, &tx2, 1_700_000_500_000),
    );
    a.expect("one wins");
    b.expect("other wins idempotently — same value");
    let corr = get_by_canonical_execution_id(&pool, &canonical)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(corr.tx_hash.as_deref(), Some(tx.as_str()));
}

#[tokio::test]
async fn c26_simultaneous_conflicting_tx_hash_attachments_one_fails() {
    let Some((pool, repo)) = require_pool().await else {
        return;
    };
    let tag = "c26concurrentconflict";
    let canonical = canonical_id(tag);
    let intent = intent_fixture(tag, 27, Some(canonical.clone()));
    let corr_input = AwaitingCorrelationInput {
        canonical_execution_id: canonical.clone(),
        ..awaiting_input(tag)
    };
    repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr_input)
        .await
        .expect("insert");
    let pool1 = pool.clone();
    let pool2 = pool.clone();
    let canonical1 = canonical.clone();
    let canonical2 = canonical.clone();
    let (a, b) = tokio::join!(
        attach_tx_hash(&pool1, &canonical1, "0x1111", 1_700_000_600_000),
        attach_tx_hash(&pool2, &canonical2, "0x2222", 1_700_000_600_000),
    );
    // Exactly one succeeds; the other must fail closed.
    let ok_count = [a.is_ok(), b.is_ok()].iter().filter(|v| **v).count();
    let err_count = [a.is_err(), b.is_err()].iter().filter(|v| **v).count();
    assert_eq!(ok_count, 1, "exactly one attach may succeed");
    assert_eq!(err_count, 1, "the other must fail closed");
}
