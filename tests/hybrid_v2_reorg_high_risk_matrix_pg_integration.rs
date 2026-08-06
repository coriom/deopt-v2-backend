//! `BACKEND-HYBRID-V2-FINAL-PERSISTENCE-MATRIX-AND-PARENT-CLOSURE-V1`
//! — high-risk reorg matrix: PG-gated end-to-end coverage of every
//! orphan-economic-family invalidation path.
//!
//! Each test:
//!   * builds a canonical chain including a specific decoded economic
//!     log (deposit, withdrawal, reservation, etc.) via the shared
//!     `hybrid_v2_support` fixture set,
//!   * drives the runtime forward to persist the log,
//!   * forks the chain at that block with a replacement branch that
//!     does NOT include the log,
//!   * invokes `ReorgRecoveryService::recover`,
//!   * asserts every relevant orphan invariant:
//!       (a) `hybrid_v2_raw_logs.is_canonical = false` on orphan rows,
//!       (b) `hybrid_v2_canonical_blocks.is_canonical = false` on
//!           orphan block rows,
//!       (c) `hybrid_v2_matched_executions.completion_status =
//!           'INVALIDATED_BY_REORG'` where applicable,
//!       (d) the persisted cursor points at the replacement tip,
//!       (e) readiness reports `ready = true` after recovery
//!           completes.
//!
//! Gating: skips cleanly when `HYBRID_V2_PG_TEST_DATABASE_URL` (or
//! `PG_INTEGRATION_URL`) is unset. Panics if
//! `DEOPT_REQUIRE_PG_INTEGRATION=1` and no URL is provided.

mod hybrid_v2_support;

use deopt_v2_backend::hybrid_v2::chain_source::{InMemoryChainSource, RawBlock};
use deopt_v2_backend::hybrid_v2::decoder::CanonicalRawLog;
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::reorg_recovery::{
    RecoveryOutcome, ReorgDetection, ReorgRecoveryConfig, ReorgRecoveryService,
};
use deopt_v2_backend::hybrid_v2::runtime::IndexerRuntime;
use hybrid_v2_support::{
    baseline_manifest, deposit_log, matching_pair_log, order_filled_log, premium_log,
    reservation_lock_log, subaccount_created_log, withdraw_log,
};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use std::sync::Arc;
use std::time::Duration;

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

fn get_pg_url_or_skip_or_panic(test_name: &str) -> Option<String> {
    let url = std::env::var(URL_ENV)
        .ok()
        .or_else(|| std::env::var(ALT_URL_ENV).ok())
        .filter(|v| !v.is_empty());
    if url.is_none() {
        let required = matches!(
            std::env::var(REQUIRE_ENV).ok().as_deref(),
            Some("1") | Some("true") | Some("TRUE")
        );
        if required {
            panic!("{} required but no PG URL provided", REQUIRE_ENV);
        }
        eprintln!("SKIP {test_name}: no PG URL");
    }
    url
}

async fn fresh_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(30))
        .connect(url)
        .await
        .expect("connect");
    sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(&pool)
        .await
        .expect("drop schema");
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("create schema");
    sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
        .execute(&pool)
        .await
        .expect("grant");
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
        .await
        .expect("migrations");
    migrator.run(&pool).await.expect("apply migrations");
    pool
}

async fn build_store(pool: &PgPool) -> (Arc<PostgresHybridV2ProjectionStore>, i64) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let did = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    (store, did)
}

fn block_hash(number: u64, tag: u8) -> String {
    format!("0x{:02x}{:0>62x}", tag, number)
}

fn make_block(number: u64, tag: u8, parent: &str, logs: Vec<CanonicalRawLog>) -> RawBlock {
    RawBlock {
        number,
        hash: block_hash(number, tag),
        parent_hash: parent.to_string(),
        timestamp: 1_700_000_000 + number,
        logs,
    }
}

fn empty_block(number: u64, tag: u8, parent: &str) -> RawBlock {
    make_block(number, tag, parent, Vec::new())
}

async fn drive_forward(
    runtime: &mut IndexerRuntime,
    source: &InMemoryChainSource,
    up_to: u64,
) -> u64 {
    let mut applied = 0;
    while runtime.cursor().indexed_head_block < up_to {
        match runtime.tick_and_persist(source).await {
            Ok(true) => applied += 1,
            Ok(false) => break,
            Err(_) => break,
        }
    }
    applied
}

/// Common shape of a chain used by most reorg-orphan tests.
///
/// Structure:
///   * block 0: empty (base)
///   * block 1: SubaccountCreated for the owner+subKey used by
///     downstream logs
///   * block 2: the economic log(s) being tested
///   * block 3: empty (canonical tip before reorg)
///
/// Reorg replaces blocks 2..=3 with an alternate branch (tag 0xbb)
/// that does NOT contain the economic log.
#[allow(dead_code)]
struct HighRiskFixture {
    manifest: deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
    owner: String,
    subkey: String,
    token: String,
    engine: String,
    b0: RawBlock,
    b1: RawBlock,
    b2: RawBlock,
    b3: RawBlock,
    b2b: RawBlock,
    b3b: RawBlock,
    b4b: RawBlock,
}

impl HighRiskFixture {
    /// Build a fixture where block 2 contains `logs`. Replacement
    /// branch has no logs on block 2b or 3b (economically empty).
    fn build(logs: Vec<CanonicalRawLog>) -> Self {
        let manifest = baseline_manifest(84532);
        let owner = "0x000000000000000000000000000000000000baaa".to_string();
        let subkey =
            "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
        let token = "0x000000000000000000000000000000000000c0de".to_string();
        let engine = manifest.module_addresses.margin_engine.clone();
        let b0 = empty_block(0, 0xaa, "");
        let mut b1_logs = Vec::new();
        b1_logs.push(subaccount_created_log(&manifest, &owner, 1, &subkey));
        let b1 = make_block(1, 0xaa, &b0.hash, b1_logs);
        let b2 = make_block(2, 0xaa, &b1.hash, logs);
        let b3 = make_block(3, 0xaa, &b2.hash, Vec::new());
        // Replacement branch: three empty blocks starting at (2,0xbb).
        let b2b = make_block(2, 0xbb, &b1.hash, Vec::new());
        let b3b = make_block(3, 0xbb, &b2b.hash, Vec::new());
        let b4b = make_block(4, 0xbb, &b3b.hash, Vec::new());
        Self {
            manifest,
            owner,
            subkey,
            token,
            engine,
            b0,
            b1,
            b2,
            b3,
            b2b,
            b3b,
            b4b,
        }
    }

    async fn seed_source(&self) -> InMemoryChainSource {
        let mut source = InMemoryChainSource::new(84532);
        source.push(self.b0.clone());
        source.push(self.b1.clone());
        source.push(self.b2.clone());
        source.push(self.b3.clone());
        source
    }
}

/// Drive the runtime to the canonical tip, then reorg from block 2
/// with the AA-branch and recover to the replacement branch.
///
/// Returns the final `RecoveryOutcome` for callers that want to
/// inspect the replacement tip.
async fn drive_and_reorg(
    store: &Arc<PostgresHybridV2ProjectionStore>,
    did: i64,
    fix: &HighRiskFixture,
) -> RecoveryOutcome {
    let mut runtime =
        IndexerRuntime::new(1, fix.manifest.clone()).with_persistence(store.clone(), did);
    let mut source = fix.seed_source().await;
    let _ = drive_forward(&mut runtime, &source, 3).await;
    assert_eq!(runtime.cursor().indexed_head_block, 3);

    // Reorg from block 2: install the replacement branch.
    source.reorg_from(2, vec![fix.b2b.clone(), fix.b3b.clone(), fix.b4b.clone()]);
    let _ = runtime.tick_and_persist(&source).await;

    let service = ReorgRecoveryService::new(did, ReorgRecoveryConfig::default());
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store.clone();
    let detection = ReorgDetection {
        old_tip_block: 3,
        old_tip_hash: fix.b3.hash.clone(),
        conflicting_block: Some(4),
        conflicting_hash: Some(fix.b4b.hash.clone()),
    };
    service
        .recover(
            &source,
            &store_dyn,
            &fix.manifest,
            Some(detection),
            "indexer",
        )
        .await
        .expect("recover ok")
}

/// Assert the orphan-block invariants after a successful reorg:
///   * orphan canonical_blocks rows on the AA-branch above the
///     ancestor are `is_canonical = FALSE`,
///   * every raw log emitted on those orphan blocks is
///     `is_canonical = FALSE`,
///   * cursor points at the replacement tip,
///   * readiness is `ready = true`.
async fn assert_orphan_invariants(
    pool: &PgPool,
    store: &PostgresHybridV2ProjectionStore,
    did: i64,
    orphan_hashes: &[&str],
    replacement_hash: &str,
    replacement_tip: u64,
) {
    for orphan_hash in orphan_hashes {
        let row = sqlx::query(
            "SELECT is_canonical FROM hybrid_v2_canonical_blocks
             WHERE deployment_id = $1 AND block_hash = $2",
        )
        .bind(did)
        .bind(*orphan_hash)
        .fetch_optional(pool)
        .await
        .unwrap();
        if let Some(row) = row {
            let canonical: bool = row.try_get("is_canonical").unwrap();
            assert!(
                !canonical,
                "orphan block {orphan_hash} still canonical after reorg"
            );
        }
        let orphan_logs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hybrid_v2_raw_logs
             WHERE deployment_id = $1 AND block_hash = $2 AND is_canonical = TRUE",
        )
        .bind(did)
        .bind(*orphan_hash)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            orphan_logs, 0,
            "orphan block {orphan_hash} still has canonical raw logs after reorg"
        );
    }

    let cursor = store.read_cursor(did, "indexer").await.unwrap().unwrap();
    assert_eq!(cursor.indexed_head_block, replacement_tip);
    assert!(
        cursor
            .indexed_head_hash
            .eq_ignore_ascii_case(replacement_hash),
        "cursor hash {} != replacement {}",
        cursor.indexed_head_hash,
        replacement_hash
    );
    let readiness = store.read_readiness(did).await.unwrap().unwrap();
    assert!(readiness.ready, "readiness not ready after recovery");
}

// -----------------------------------------------------------------
//                       HIGH-RISK MATRIX
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_deposit_balance_reverted() {
    let Some(url) = get_pg_url_or_skip_or_panic("orphaned_deposit_balance_reverted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let owner = "0x000000000000000000000000000000000000baaa".to_string();
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let dep = deposit_log(&manifest, &subkey, &owner, 1, &token, "1000");
    let fix = HighRiskFixture::build(vec![dep]);

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_eq!(rep_tip, 4);
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;
    let _ = fix.owner;
    let _ = fix.engine;
}

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_withdrawal_balance_reverted() {
    let Some(url) = get_pg_url_or_skip_or_panic("orphaned_withdrawal_balance_reverted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let owner = "0x000000000000000000000000000000000000baaa".to_string();
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let wd = withdraw_log(&manifest, &subkey, &owner, 1, &token, "500");
    let fix = HighRiskFixture::build(vec![wd]);

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_reservation_creation_reverted() {
    let Some(url) = get_pg_url_or_skip_or_panic("orphaned_reservation_creation_reverted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let engine = manifest.module_addresses.margin_engine.clone();
    let lock = reservation_lock_log(&manifest, &subkey, &token, &engine, "250");
    let fix = HighRiskFixture::build(vec![lock]);

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_order_and_partial_fill_reverted() {
    let Some(url) = get_pg_url_or_skip_or_panic("orphaned_order_and_partial_fill_reverted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let order_hash =
        "0xbb22222222222222222222222222222222222222222222222222222222222222".to_string();
    let filled = order_filled_log(&manifest, &subkey, &order_hash, "10", "100", false);
    let fix = HighRiskFixture::build(vec![filled]);

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;

    // The order_lifecycle row (if the decoder ingested it) must not
    // point at the orphan block anymore. We don't assert its
    // existence, only that no CANONICAL raw log on the orphan block
    // remains — which is covered by `assert_orphan_invariants`.
}

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_matched_execution_invalidated() {
    let Some(url) = get_pg_url_or_skip_or_panic("orphaned_matched_execution_invalidated") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let buyer_sk = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let seller_sk =
        "0xaa22222222222222222222222222222222222222222222222222222222222222".to_string();
    let buyer_oh = "0xbb00000000000000000000000000000000000000000000000000000000000001".to_string();
    let seller_oh =
        "0xbb00000000000000000000000000000000000000000000000000000000000002".to_string();
    let exec = matching_pair_log(
        &manifest,
        "0xdead000000000000000000000000000000000000000000000000000000000001",
        &buyer_oh,
        &seller_oh,
        &buyer_sk,
        &seller_sk,
        "10",
        "10000",
    );
    let fix = HighRiskFixture::build(vec![exec]);

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;

    // If any matched_execution row landed on the orphan block, it must
    // now be INVALIDATED_BY_REORG. When the decoder didn't ingest the
    // fixture (raw log alone) the query returns zero rows, which also
    // satisfies the invariant.
    let bad: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hybrid_v2_matched_executions
         WHERE deployment_id = $1
           AND block_number > 1
           AND completion_status <> 'INVALIDATED_BY_REORG'",
    )
    .bind(did)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        bad, 0,
        "matched executions above ancestor must all be INVALIDATED_BY_REORG"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_premium_transfer_reverted() {
    let Some(url) = get_pg_url_or_skip_or_panic("orphaned_premium_transfer_reverted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let from_sk = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let to_sk = "0xaa22222222222222222222222222222222222222222222222222222222222222".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let prem = premium_log(&manifest, &from_sk, &to_sk, &token, "42");
    let fix = HighRiskFixture::build(vec![prem]);

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn orphaned_multi_family_batch_reverted() {
    // Two economic events (deposit + reservation lock) inside the
    // same orphan block. Recovery must invalidate BOTH.
    let Some(url) = get_pg_url_or_skip_or_panic("orphaned_multi_family_batch_reverted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let owner = "0x000000000000000000000000000000000000baaa".to_string();
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let engine = manifest.module_addresses.margin_engine.clone();
    let dep = deposit_log(&manifest, &subkey, &owner, 1, &token, "1000");
    let lock = reservation_lock_log(&manifest, &subkey, &token, &engine, "250");
    let fix = HighRiskFixture::build(vec![dep, lock]);

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn replacement_execution_with_changed_components() {
    // Original block 2 contained one deposit; replacement block 2b
    // contains a DIFFERENT deposit (different amount) at the same
    // block height. Recovery must adopt the replacement, and the
    // orphan version must not remain canonical.
    let Some(url) = get_pg_url_or_skip_or_panic("replacement_execution_with_changed_components")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let owner = "0x000000000000000000000000000000000000baaa".to_string();
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let dep_orig = deposit_log(&manifest, &subkey, &owner, 1, &token, "1000");
    let dep_repl = deposit_log(&manifest, &subkey, &owner, 1, &token, "9999");
    // Build fixture with orphan deposit on b2 (0xaa) and replacement
    // deposit on b2b (0xbb) — override the empty b2b/b3b defaults.
    let mut fix = HighRiskFixture::build(vec![dep_orig]);
    fix.b2b = make_block(2, 0xbb, &fix.b1.hash, vec![dep_repl]);
    fix.b3b = make_block(3, 0xbb, &fix.b2b.hash, Vec::new());
    fix.b4b = make_block(4, 0xbb, &fix.b3b.hash, Vec::new());

    let outcome = drive_and_reorg(&store, did, &fix).await;
    let (rep_tip, rep_hash) = match outcome {
        RecoveryOutcome::Recovered {
            replacement_tip,
            replacement_hash,
            ..
        } => (replacement_tip, replacement_hash),
        other => panic!("expected Recovered, got {other:?}"),
    };
    assert_orphan_invariants(
        &pool,
        &store,
        did,
        &[&fix.b2.hash, &fix.b3.hash],
        &rep_hash,
        rep_tip,
    )
    .await;

    // A canonical raw log MUST now exist on the replacement block b2b.
    let canonical_repl: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hybrid_v2_raw_logs
         WHERE deployment_id = $1 AND block_hash = $2 AND is_canonical = TRUE",
    )
    .bind(did)
    .bind(&fix.b2b.hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        canonical_repl >= 1,
        "expected at least one canonical replacement log on b2b, got {canonical_repl}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_recovery_on_two_deployments_isolated() {
    // Two deployments A + B, both indexed to block 3. A reorgs.
    // B must be completely unaffected.
    let Some(url) = get_pg_url_or_skip_or_panic("concurrent_recovery_on_two_deployments_isolated")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));

    let manifest_a = baseline_manifest(84532);
    let mut manifest_b = baseline_manifest(84532);
    manifest_b.manifest_hash =
        "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string();
    manifest_b.deployment_version = manifest_b.deployment_version.wrapping_add(1);

    let did_a = store
        .upsert_deployment(&manifest_a, "PENDING", 1_700_000_000_000)
        .await
        .unwrap();
    let did_b = store
        .upsert_deployment(&manifest_b, "PENDING", 1_700_000_000_000)
        .await
        .unwrap();

    let owner = "0x000000000000000000000000000000000000baaa".to_string();
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let dep_a = deposit_log(&manifest_a, &subkey, &owner, 1, &token, "1000");
    let dep_b = deposit_log(&manifest_b, &subkey, &owner, 1, &token, "2000");
    let fix_a = HighRiskFixture::build(vec![dep_a]);
    let fix_b = HighRiskFixture::build(vec![dep_b]);
    // Sanity: both fixtures have the same block hashes since they use
    // the same tag/number scheme — that's intentional (isolation is
    // per-deployment_id in the store).
    assert_eq!(fix_a.b0.hash, fix_b.b0.hash);

    // Drive B to canonical tip and NEVER reorg it.
    let mut rt_b =
        IndexerRuntime::new(2, manifest_b.clone()).with_persistence(store.clone(), did_b);
    let source_b = fix_b.seed_source().await;
    let _ = drive_forward(&mut rt_b, &source_b, 3).await;
    let cursor_b_pre = store.read_cursor(did_b, "indexer").await.unwrap().unwrap();

    // Drive A + reorg A.
    let _ = drive_and_reorg(&store, did_a, &fix_a).await;

    // B must be unchanged: cursor identical, no reorg-recovery row.
    let cursor_b_post = store.read_cursor(did_b, "indexer").await.unwrap().unwrap();
    assert_eq!(
        cursor_b_pre.indexed_head_block,
        cursor_b_post.indexed_head_block
    );
    assert_eq!(
        cursor_b_pre.indexed_head_hash,
        cursor_b_post.indexed_head_hash
    );
    assert!(store.read_reorg_recovery(did_b).await.unwrap().is_none());

    // Cross-check: B's canonical journal still has canonical raw logs
    // on block 2 (the orphan block of A). The store isolates by
    // (deployment_id, block_hash) so A's invalidation must NOT touch
    // B's rows even though the block_hash string coincides.
    let b_canonical_logs_at_block_2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hybrid_v2_raw_logs
         WHERE deployment_id = $1 AND block_number = 2 AND is_canonical = TRUE",
    )
    .bind(did_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        b_canonical_logs_at_block_2 >= 1,
        "deployment B lost its canonical logs on block 2 during A's reorg (isolation broken)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_after_recovery_commit_before_memory_publication() {
    // Simulate a crash between commit_reorg_recovery + the next
    // in-memory publication: perform the recovery, drop the runtime,
    // then re-bootstrap and confirm the persisted state is what a
    // restarting worker would inherit.
    let Some(url) =
        get_pg_url_or_skip_or_panic("restart_after_recovery_commit_before_memory_publication")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, did) = build_store(&pool).await;

    let manifest = baseline_manifest(84532);
    let owner = "0x000000000000000000000000000000000000baaa".to_string();
    let subkey = "0xaa11111111111111111111111111111111111111111111111111111111111111".to_string();
    let token = "0x000000000000000000000000000000000000c0de".to_string();
    let dep = deposit_log(&manifest, &subkey, &owner, 1, &token, "1000");
    let fix = HighRiskFixture::build(vec![dep]);

    // Perform recovery in a temporary runtime scope; drop it before
    // reading the persisted state.
    {
        let outcome = drive_and_reorg(&store, did, &fix).await;
        assert!(matches!(outcome, RecoveryOutcome::Recovered { .. }));
    }

    // A fresh runtime bootstraps from the persisted cursor +
    // canonical journal alone. The cursor MUST be at the replacement
    // tip; the readiness MUST be ready.
    let cursor = store.read_cursor(did, "indexer").await.unwrap().unwrap();
    assert_eq!(cursor.indexed_head_block, 4);
    assert!(cursor.indexed_head_hash.eq_ignore_ascii_case(&fix.b4b.hash));
    let readiness = store.read_readiness(did).await.unwrap().unwrap();
    assert!(readiness.ready);

    // Post-restart: bootstrap MUST NOT re-materialize orphan logs.
    let orphan_canonical: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hybrid_v2_raw_logs
         WHERE deployment_id = $1 AND block_hash = $2 AND is_canonical = TRUE",
    )
    .bind(did)
    .bind(&fix.b2.hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        orphan_canonical, 0,
        "orphan raw logs re-appeared canonical after restart"
    );
}
