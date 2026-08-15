//! OPTIONS-HYBRID-V2-CORRELATION-ATOMIC-WIRING-V1 Part L — bounded
//! property assertions for the atomic intent+correlation wiring.
//!
//! Each property runs against a small deterministic sample set driven
//! by a fixed seed. No `proptest` dep is added — same convention as
//! `tests/hybrid_v2_broadcast_live_wiring_properties.rs`.
//!
//! Real-PG gate: identical to
//! `tests/options_hybrid_v2_atomic_wiring_pg_integration.rs`. Set
//! `OPTIONS_ATOMIC_WIRING_PG_URL=...` to run; `OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP=1`
//! opts out for dev.
//!
//! Six properties:
//!   P1 — new canonical matcher-derived intent never commits without
//!        its AWAITING correlation row.
//!   P2 — rollback (precondition violation) preserves zero half-state.
//!   P3 — duplicate atomic invocation produces one intent + one
//!        correlation regardless of interleaving order.
//!   P4 — same-value attach_tx_hash is idempotent under any repeat
//!        count.
//!   P5 — different tx_hash cannot overwrite an already-attached
//!        authoritative identity.
//!   P6 — process restart preserves exact (intent, correlation,
//!        tx_hash) linkage.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::options::correlation_repository::{
    attach_tx_hash, get_by_canonical_execution_id, AwaitingCorrelationInput,
    OptionCorrelationStatus, OptionExecutionKind,
};
use deopt_v2_backend::options::{
    OptionExecutionIntent, OptionExecutionIntentId, OptionExecutionIntentStatus,
    OptionExecutionSourceType,
};
use deopt_v2_backend::types::AccountId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

const URL_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_URL";
const SKIP_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP";
const SHARED_SERIES_ID: &str = "atomic-wiring-properties-series";

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
        None => panic!(
            "{URL_ENV} is not set. Set it to run atomic wiring properties, or {SKIP_ENV}=1 to skip."
        ),
    }
}

async fn ensure_series(pool: &PgPool) {
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
    .bind(SHARED_SERIES_ID)
    .execute(pool)
    .await
    .expect("ensure_series");
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
                .expect("run migrations against disposable PG");
            ensure_series(repo.pool()).await;
        })
        .await;
}

async fn setup() -> Option<(PgPool, PgRepository)> {
    let url = require_pg_url()?;
    ensure_migrated(&url).await;
    let repo = PgRepository::connect(&url)
        .await
        .expect("PgRepository connect");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("shared pool");
    Some((pool, repo))
}

fn canonical_id(prefix: &str, seed: u32) -> String {
    let hex: String = format!("{seed:08x}").repeat(8);
    let combined: String = format!("{prefix}{hex}");
    let sanitised: String = combined
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(64)
        .collect();
    let mut padded = sanitised;
    while padded.len() < 64 {
        padded.push('a');
    }
    format!("0x{padded}")
}

fn intent_fixture(prefix: &str, seed: u32, canonical: Option<String>) -> OptionExecutionIntent {
    // Per-fixture unique `onchain_intent_id`. Trimmed / padded to 66
    // char hex-with-0x so the UNIQUE constraint sees a distinct value
    // per (prefix, seed).
    let raw: String = format!("{prefix}{seed:08x}");
    let hex_body: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(64)
        .collect();
    let mut padded = hex_body;
    while padded.len() < 64 {
        padded.push('a');
    }
    let onchain_intent_id = format!("0x{padded}");
    OptionExecutionIntent {
        intent_id: OptionExecutionIntentId::from(Uuid::new_v4()),
        onchain_intent_id,
        source_type: OptionExecutionSourceType::OptionOrderbookFill,
        source_id: format!("prop-{prefix}-{seed}"),
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
        buyer_nonce: Some(seed as u128),
        seller_nonce: Some(seed as u128 + 1),
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

fn awaiting(canonical: &str, deployment: i64, chain: i64) -> AwaitingCorrelationInput {
    AwaitingCorrelationInput {
        canonical_execution_id: canonical.to_string(),
        deployment_id: deployment,
        chain_id: chain,
        execution_kind: OptionExecutionKind::Trade,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: Some("100000000".to_string()),
        now_ms: 1_700_000_000_000,
    }
}

async fn count_intents(pool: &PgPool, source_id: &str) -> i64 {
    let row = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_execution_intents WHERE source_id = $1",
    )
    .bind(source_id)
    .fetch_one(pool)
    .await
    .expect("count intents");
    row.try_get::<i64, _>("c").expect("count")
}

async fn count_active_correlations(pool: &PgPool, canonical: &str) -> i64 {
    let row = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_execution_correlations
         WHERE canonical_execution_id = $1
           AND correlation_status IN
               ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED', 'CORRELATED_CANONICAL')",
    )
    .bind(canonical)
    .fetch_one(pool)
    .await
    .expect("count correlations");
    row.try_get::<i64, _>("c").expect("count")
}

// P1 — new canonical matcher-derived intent never commits without
// its AWAITING correlation row.
#[tokio::test]
async fn p1_new_canonical_intent_always_has_correlation() {
    let Some((pool, repo)) = setup().await else {
        return;
    };
    // Deterministic seeds — cover several parameter shapes.
    for seed in [11u32, 23, 47, 59, 79, 97, 113, 137] {
        let canonical = canonical_id("p1", seed);
        let intent = intent_fixture("p1", seed, Some(canonical.clone()));
        let corr = awaiting(&canonical, 1, 84532);
        repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr)
            .await
            .expect("atomic insert");
        assert_eq!(
            count_intents(&pool, &intent.source_id).await,
            1,
            "seed={seed}"
        );
        assert_eq!(
            count_active_correlations(&pool, &canonical).await,
            1,
            "seed={seed}",
        );
    }
}

// P2 — precondition rollback preserves zero half-state (intent
// INSERT never lands when correlation input is invalid).
#[tokio::test]
async fn p2_rollback_preserves_zero_half_state() {
    let Some((pool, repo)) = setup().await else {
        return;
    };
    for seed in [3u32, 17, 29, 41, 61] {
        let canonical = canonical_id("p2", seed);
        let intent = intent_fixture("p2", seed, Some(canonical.clone()));
        // Mismatch: input canonical id differs from intent's.
        let bad_corr = awaiting(&canonical_id("p2other", seed + 1), 1, 84532);
        let err = repo
            .insert_option_execution_intent_with_awaiting_correlation(&intent, &bad_corr)
            .await;
        assert!(err.is_err(), "seed={seed}: precondition must abort");
        assert_eq!(
            count_intents(&pool, &intent.source_id).await,
            0,
            "seed={seed}"
        );
        assert_eq!(count_active_correlations(&pool, &canonical).await, 0);
    }
}

// P3 — duplicate atomic invocation produces exactly one intent + one
// correlation regardless of repetition count.
#[tokio::test]
async fn p3_duplicate_invocation_is_idempotent() {
    let Some((pool, repo)) = setup().await else {
        return;
    };
    for (seed, repeats) in [(5u32, 2usize), (13, 3), (37, 5)] {
        let canonical = canonical_id("p3", seed);
        let intent = intent_fixture("p3", seed, Some(canonical.clone()));
        let corr = awaiting(&canonical, 1, 84532);
        for _ in 0..repeats {
            repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr)
                .await
                .expect("insert must succeed each repeat");
        }
        assert_eq!(count_intents(&pool, &intent.source_id).await, 1);
        assert_eq!(count_active_correlations(&pool, &canonical).await, 1);
    }
}

// P4 — same-value attach_tx_hash is idempotent under any repeat count.
#[tokio::test]
async fn p4_same_value_attach_is_idempotent() {
    let Some((pool, repo)) = setup().await else {
        return;
    };
    for (seed, tx, repeats) in [(7u32, "0xaa", 2usize), (19, "0xbb", 4), (43, "0xcc", 6)] {
        let canonical = canonical_id("p4", seed);
        let intent = intent_fixture("p4", seed, Some(canonical.clone()));
        let corr = awaiting(&canonical, 1, 84532);
        repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr)
            .await
            .expect("insert");
        for i in 0..repeats {
            attach_tx_hash(&pool, &canonical, tx, 1_700_000_100_000 + i as i64)
                .await
                .expect("same-value attach must be idempotent");
        }
        let row = get_by_canonical_execution_id(&pool, &canonical)
            .await
            .expect("lookup")
            .expect("exists");
        assert_eq!(row.tx_hash.as_deref(), Some(tx));
        assert_eq!(row.correlation_status, OptionCorrelationStatus::Submitted);
    }
}

// P5 — different tx_hash cannot overwrite an already-attached
// authoritative identity.
#[tokio::test]
async fn p5_different_tx_hash_cannot_overwrite() {
    let Some((pool, repo)) = setup().await else {
        return;
    };
    let table = [
        (2u32, "0xd1", "0xe1"),
        (8, "0xd2", "0xe2"),
        (14, "0xd3", "0xe3"),
        (26, "0xd4", "0xe4"),
    ];
    for (seed, first_tx, second_tx) in table {
        let canonical = canonical_id("p5", seed);
        let intent = intent_fixture("p5", seed, Some(canonical.clone()));
        let corr = awaiting(&canonical, 1, 84532);
        repo.insert_option_execution_intent_with_awaiting_correlation(&intent, &corr)
            .await
            .expect("insert");
        attach_tx_hash(&pool, &canonical, first_tx, 1_700_000_200_000)
            .await
            .expect("first attach");
        let err = attach_tx_hash(&pool, &canonical, second_tx, 1_700_000_300_000).await;
        assert!(err.is_err(), "seed={seed}: overwrite must fail closed");
        let row = get_by_canonical_execution_id(&pool, &canonical)
            .await
            .expect("lookup")
            .expect("exists");
        assert_eq!(row.tx_hash.as_deref(), Some(first_tx));
    }
}

// P6 — process restart preserves exact linkage.
#[tokio::test]
async fn p6_restart_preserves_linkage() {
    let Some(url) = require_pg_url() else {
        return;
    };
    ensure_migrated(&url).await;
    for seed in [4u32, 22, 68] {
        let canonical = canonical_id("p6", seed);
        // Session A: insert intent + correlation, attach tx.
        let repo_a = PgRepository::connect(&url).await.expect("connect A");
        let intent = intent_fixture("p6", seed, Some(canonical.clone()));
        let corr = awaiting(&canonical, 1, 84532);
        repo_a
            .insert_option_execution_intent_with_awaiting_correlation(&intent, &corr)
            .await
            .expect("insert");
        attach_tx_hash(repo_a.pool(), &canonical, "0xabc", 1_700_000_400_000)
            .await
            .expect("attach");
        drop(repo_a);
        // Session B: fresh pool — simulate restart.
        let repo_b = PgRepository::connect(&url).await.expect("connect B");
        let corr_b = get_by_canonical_execution_id(repo_b.pool(), &canonical)
            .await
            .expect("lookup")
            .expect("exists");
        assert_eq!(
            corr_b.correlation_status,
            OptionCorrelationStatus::Submitted
        );
        assert_eq!(corr_b.tx_hash.as_deref(), Some("0xabc"));
        // Look up the persisted intent via (source_type, source_id).
        // Using intent.intent_id is unsafe across re-runs: the DB
        // preserves the ORIGINAL intent_id from the first run because
        // `insert_option_execution_intent` uses `ON CONFLICT
        // (source_type, source_id) DO NOTHING`; the fresh Uuid on the
        // second run is discarded. The canonical_execution_id / source
        // path survives regardless of run count.
        let stored_intent = repo_b
            .get_option_execution_intent_by_source(intent.source_type, &intent.source_id)
            .await
            .expect("lookup intent by source")
            .expect("intent survives restart");
        assert_eq!(
            stored_intent.canonical_execution_id.as_deref(),
            Some(canonical.as_str())
        );
    }
}
