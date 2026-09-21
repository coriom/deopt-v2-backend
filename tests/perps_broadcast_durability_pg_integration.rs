//! PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_DURABILITY_PG_V1 — PG-backed
//! integration coverage for the `execution_intent_broadcasts` table +
//! `PgRepository::record_prepared_transaction` /
//! `mark_intent_submitted` / `mark_intent_confirmed` /
//! `mark_intent_failed` durability path.
//!
//! All tests are env-gated via `PERPS_CLOSED_TEST_E2E_PG_URL` — CI
//! systems without a disposable Postgres skip them cleanly.
//!
//! Coverage:
//!
//! 1. migration applies cleanly.
//! 2. record_prepared_transaction inserts + is retrievable.
//! 3. duplicate tx_hash is rejected (unique constraint).
//! 4. duplicate (chain_id, executor, nonce) is rejected (multi-worker
//!    nonce safety).
//! 5. lifecycle: Prepared → Submitted → Confirmed persists correctly.
//! 6. lifecycle: Prepared → Failed persists correctly.
//! 7. list_unfinalized_broadcasts returns only {Prepared, Submitted}.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::execution::{ExecutionIntentStatus, PreparedTransactionRecord};
use deopt_v2_backend::types::AccountId;
use uuid::Uuid;

const PG_ENV_VAR: &str = "PERPS_CLOSED_TEST_E2E_PG_URL";
const PME_TARGET: &str = "0x774d96e5739bffadee91508b4d3d74f5be29f165";
const CHAIN_ID_BASE_SEPOLIA: u64 = 84532;

fn pg_url() -> Option<String> {
    std::env::var(PG_ENV_VAR).ok().filter(|v| !v.is_empty())
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
                .expect("run migrations once against disposable PG database");
        })
        .await;
}

async fn fresh_repo(url: &str) -> PgRepository {
    ensure_migrated(url).await;
    PgRepository::connect(url)
        .await
        .expect("connect to disposable PG database")
}

fn unique_intent_id(seed: u8) -> Uuid {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // Compose a stable-but-unique 128-bit id: upper 64 = timestamp, lower 8 = seed
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&ns.to_be_bytes());
    bytes[15] = seed;
    Uuid::from_bytes(bytes)
}

fn addr(seed: u8) -> AccountId {
    let mut bytes = [0u8; 20];
    bytes[0] = seed;
    for (i, b) in bytes.iter_mut().enumerate().skip(1) {
        *b = (i as u8).wrapping_mul(seed).wrapping_add(0x1f);
    }
    let mut hex = String::from("0x");
    for byte in &bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    AccountId::new(hex)
}

fn tx_hash_hex(seed: u8) -> String {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut s = String::from("0x");
    s.push_str(&format!("{ns:016x}"));
    for i in 0..24 {
        s.push_str(&format!("{:02x}", (i as u8).wrapping_mul(seed)));
    }
    s
}

/// Insert a bare parent `execution_intents` row so the FK-constrained
/// broadcast child row is insertable. Uses the repository's public
/// helper for the trade path.
async fn seed_execution_intent(repo: &PgRepository, intent_id: Uuid) {
    // Insert directly via raw SQL against the repository's pool — the
    // canonical high-level insert path involves matching + fill
    // creation which is out of scope for this table-level test.
    let pool = repo.pool();
    sqlx::query(
        "INSERT INTO execution_intents (
            intent_id, market_id, buyer, seller, price_1e8, size_1e8,
            buy_order_id, sell_order_id, status, created_at_ms, updated_at_ms
        ) VALUES ($1, 1, $2, $3, '300000000000', '100000000',
                  $4, $5, 'pending', $6, $6)
         ON CONFLICT (intent_id) DO NOTHING",
    )
    .bind(intent_id.to_string())
    .bind(addr(0x11).0)
    .bind(addr(0x12).0)
    .bind(Uuid::new_v4().to_string())
    .bind(Uuid::new_v4().to_string())
    .bind(1_700_000_000_000i64)
    .execute(pool)
    .await
    .expect("insert parent execution_intents row");
}

fn make_record(intent_id: Uuid, tx_hash: String, nonce: u64) -> PreparedTransactionRecord {
    PreparedTransactionRecord {
        intent_id,
        chain_id: CHAIN_ID_BASE_SEPOLIA,
        executor_address: AccountId::new("0x58ad437cb9e32b0faee810ef05810d5ba2ae52b8".to_string()),
        target_address: AccountId::new(PME_TARGET.to_string()),
        tx_hash,
        nonce,
        raw_tx_hex: "0x02f8...deadbeef".to_string(),
        prepared_at_ms: 1_700_000_000_000,
        protocol_version: deopt_v2_backend::execution::PerpsProtocolVersion::V1,
        expected_emitter: AccountId::new(PME_TARGET.to_string()),
    }
}

// ---------------------------------------------------------------------
// 1. Migration applies + basic insert/read.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_migration_and_insert_read() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [broadcast_durability_migration_and_insert_read] (PG url not provided)");
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x01);
    seed_execution_intent(&repo, intent_id).await;
    let tx_hash = tx_hash_hex(0x01);
    repo.record_prepared_transaction(make_record(intent_id, tx_hash.clone(), 100))
        .await
        .expect("insert prepared broadcast");
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .expect("read prepared broadcast")
        .expect("row present");
    assert_eq!(row.tx_hash, tx_hash);
    assert_eq!(row.nonce, 100);
    assert_eq!(row.status, ExecutionIntentStatus::Prepared);
    assert_eq!(row.send_attempts, 0);
}

// ---------------------------------------------------------------------
// 2. Duplicate tx_hash rejected.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_duplicate_tx_hash_rejected() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [broadcast_durability_duplicate_tx_hash_rejected] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_a = unique_intent_id(0x02);
    let intent_b = unique_intent_id(0x03);
    seed_execution_intent(&repo, intent_a).await;
    seed_execution_intent(&repo, intent_b).await;
    let tx_hash = tx_hash_hex(0x02);
    repo.record_prepared_transaction(make_record(intent_a, tx_hash.clone(), 200))
        .await
        .expect("first insert");
    let err = repo
        .record_prepared_transaction(make_record(intent_b, tx_hash.clone(), 201))
        .await
        .expect_err("duplicate tx_hash must be rejected");
    assert!(matches!(err, BackendError::BroadcastRejected(_)));
}

// ---------------------------------------------------------------------
// 3. Duplicate (chain_id, executor, nonce) rejected → multi-worker safety.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_duplicate_executor_nonce_rejected() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [broadcast_durability_duplicate_executor_nonce_rejected] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_a = unique_intent_id(0x04);
    let intent_b = unique_intent_id(0x05);
    seed_execution_intent(&repo, intent_a).await;
    seed_execution_intent(&repo, intent_b).await;
    repo.record_prepared_transaction(make_record(intent_a, tx_hash_hex(0x04), 300))
        .await
        .expect("first insert");
    let err = repo
        .record_prepared_transaction(make_record(intent_b, tx_hash_hex(0x05), 300))
        .await
        .expect_err("duplicate nonce must be rejected");
    assert!(matches!(err, BackendError::BroadcastRejected(_)));
}

// ---------------------------------------------------------------------
// 4. Lifecycle: Prepared → Submitted → Confirmed.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_lifecycle_confirmed() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [broadcast_durability_lifecycle_confirmed] (PG url not provided)");
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x06);
    seed_execution_intent(&repo, intent_id).await;
    repo.record_prepared_transaction(make_record(intent_id, tx_hash_hex(0x06), 400))
        .await
        .expect("insert prepared");
    repo.mark_intent_submitted(intent_id, 1_700_000_100_000)
        .await
        .expect("mark submitted");
    repo.mark_intent_confirmed(intent_id, 46_800_000, 1_700_000_200_000)
        .await
        .expect("mark confirmed");
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, ExecutionIntentStatus::Confirmed);
    assert_eq!(row.receipt_block_number, Some(46_800_000));
    assert_eq!(row.receipt_status, Some(1));
    assert_eq!(row.confirmed_at_ms, Some(1_700_000_200_000));
    assert!(row.first_submission_at_ms.is_some());
}

// ---------------------------------------------------------------------
// 5. Lifecycle: Prepared → Failed.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_lifecycle_failed() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [broadcast_durability_lifecycle_failed] (PG url not provided)");
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x07);
    seed_execution_intent(&repo, intent_id).await;
    repo.record_prepared_transaction(make_record(intent_id, tx_hash_hex(0x07), 500))
        .await
        .expect("insert prepared");
    repo.mark_intent_failed(
        intent_id,
        "deterministic_reject: nonce too low".to_string(),
        1_700_000_300_000,
    )
    .await
    .expect("mark failed");
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, ExecutionIntentStatus::Failed);
    assert!(row
        .failure_reason
        .as_deref()
        .unwrap()
        .contains("nonce too low"));
    assert_eq!(row.failed_at_ms, Some(1_700_000_300_000));
}

// ---------------------------------------------------------------------
// 6. bump_send_attempt increments monotonically.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_bump_send_attempt_monotonic() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [broadcast_durability_bump_send_attempt_monotonic] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x08);
    seed_execution_intent(&repo, intent_id).await;
    repo.record_prepared_transaction(make_record(intent_id, tx_hash_hex(0x08), 600))
        .await
        .expect("insert prepared");
    let a = repo
        .bump_send_attempt(intent_id, 1_700_000_400_000)
        .await
        .unwrap();
    let b = repo
        .bump_send_attempt(intent_id, 1_700_000_500_000)
        .await
        .unwrap();
    let c = repo
        .bump_send_attempt(intent_id, 1_700_000_600_000)
        .await
        .unwrap();
    assert_eq!(a, 1);
    assert_eq!(b, 2);
    assert_eq!(c, 3);
}

// ---------------------------------------------------------------------
// 7. Status monotonicity: Confirmed cannot regress.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_confirmed_cannot_regress() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [broadcast_durability_confirmed_cannot_regress] (PG url not provided)");
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x09);
    seed_execution_intent(&repo, intent_id).await;
    repo.record_prepared_transaction(make_record(intent_id, tx_hash_hex(0x09), 700))
        .await
        .expect("insert prepared");
    repo.mark_intent_submitted(intent_id, 1_700_000_100_000)
        .await
        .unwrap();
    repo.mark_intent_confirmed(intent_id, 46_800_100, 1_700_000_200_000)
        .await
        .unwrap();
    // Attempt to regress → WHERE status IN (prepared|submitted) blocks it.
    repo.mark_intent_failed(intent_id, "spurious".to_string(), 1_700_000_300_000)
        .await
        .unwrap();
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.status,
        ExecutionIntentStatus::Confirmed,
        "broadcast lifecycle must not regress from Confirmed"
    );
}

// ---------------------------------------------------------------------
// 8. Failed cannot regress.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_failed_cannot_regress() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [broadcast_durability_failed_cannot_regress] (PG url not provided)");
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x0a);
    seed_execution_intent(&repo, intent_id).await;
    repo.record_prepared_transaction(make_record(intent_id, tx_hash_hex(0x0a), 800))
        .await
        .expect("insert prepared");
    repo.mark_intent_failed(
        intent_id,
        "deterministic reject".to_string(),
        1_700_000_100_000,
    )
    .await
    .unwrap();
    repo.mark_intent_confirmed(intent_id, 46_800_200, 1_700_000_300_000)
        .await
        .unwrap();
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.status,
        ExecutionIntentStatus::Failed,
        "broadcast lifecycle must not regress from Failed"
    );
}

// ---------------------------------------------------------------------
// 9. list_unfinalized_broadcasts filters correctly.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_list_unfinalized_filter() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [broadcast_durability_list_unfinalized_filter] (PG url not provided)");
        return;
    };
    let repo = fresh_repo(&url).await;
    let prepared = unique_intent_id(0x11);
    let submitted = unique_intent_id(0x12);
    let confirmed = unique_intent_id(0x13);
    let failed = unique_intent_id(0x14);
    for id in [prepared, submitted, confirmed, failed] {
        seed_execution_intent(&repo, id).await;
    }
    repo.record_prepared_transaction(make_record(prepared, tx_hash_hex(0x11), 1100))
        .await
        .unwrap();
    repo.record_prepared_transaction(make_record(submitted, tx_hash_hex(0x12), 1101))
        .await
        .unwrap();
    repo.record_prepared_transaction(make_record(confirmed, tx_hash_hex(0x13), 1102))
        .await
        .unwrap();
    repo.record_prepared_transaction(make_record(failed, tx_hash_hex(0x14), 1103))
        .await
        .unwrap();
    repo.mark_intent_submitted(submitted, 1_700_000_100_000)
        .await
        .unwrap();
    repo.mark_intent_submitted(confirmed, 1_700_000_100_000)
        .await
        .unwrap();
    repo.mark_intent_confirmed(confirmed, 42, 1_700_000_200_000)
        .await
        .unwrap();
    repo.mark_intent_failed(failed, "test".to_string(), 1_700_000_200_000)
        .await
        .unwrap();

    let unfinalized = repo.list_unfinalized_broadcasts(100).await.unwrap();
    let ids: std::collections::HashSet<_> = unfinalized.iter().map(|i| i.intent_id).collect();
    assert!(ids.contains(&prepared), "prepared must appear");
    assert!(ids.contains(&submitted), "submitted must appear");
    assert!(!ids.contains(&confirmed), "confirmed must NOT appear");
    assert!(!ids.contains(&failed), "failed must NOT appear");
}

// ---------------------------------------------------------------------
// 10. Concurrent nonce race — exactly one wins.
// ---------------------------------------------------------------------

#[tokio::test]
async fn broadcast_durability_concurrent_nonce_race_exactly_one_wins() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [broadcast_durability_concurrent_nonce_race_exactly_one_wins] (PG url not provided)");
        return;
    };
    let repo_seed = fresh_repo(&url).await;
    const N: u8 = 5;
    let mut intents = Vec::with_capacity(N as usize);
    for i in 0..N {
        let id = unique_intent_id(0x20 + i);
        seed_execution_intent(&repo_seed, id).await;
        intents.push(id);
    }
    let shared_nonce: u64 = 9_000;

    let mut handles = Vec::new();
    for (i, id) in intents.iter().copied().enumerate() {
        let url = url.clone();
        let tx_hash = tx_hash_hex(0x30 + i as u8);
        handles.push(tokio::spawn(async move {
            let repo = fresh_repo(&url).await;
            let record = make_record(id, tx_hash, shared_nonce);
            repo.record_prepared_transaction(record).await
        }));
    }
    let mut successes = 0;
    let mut rejections = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(()) => successes += 1,
            Err(BackendError::BroadcastRejected(_)) => rejections += 1,
            Err(other) => panic!("unexpected error class: {other:?}"),
        }
    }
    assert_eq!(
        successes, 1,
        "exactly one worker allocates the shared nonce"
    );
    assert_eq!(
        rejections,
        (N - 1) as usize,
        "all other workers are rejected"
    );
}

// =====================================================================
// PERPS_V2_BACKEND_RECONCILIATION_V1 §2, §5, §9, §18 — durable
// per-row generation identity round-trip.
//
// These tests exercise the invariant that a persisted broadcast
// row's `protocol_version` and `expected_emitter` survive an
// insert → SELECT round-trip byte-identically, and that a V2
// insert coexists with V1 rows without disturbing them.
// =====================================================================

const PME_V1_ADDR: &str = "0x774d96e5739bffadee91508b4d3d74f5be29f165";
const PME_V2_ADDR: &str = "0x1111111111111111111111111111111111111112";

fn make_record_versioned(
    intent_id: Uuid,
    tx_hash: String,
    nonce: u64,
    protocol_version: deopt_v2_backend::execution::PerpsProtocolVersion,
    emitter_hex: &str,
) -> PreparedTransactionRecord {
    PreparedTransactionRecord {
        intent_id,
        chain_id: CHAIN_ID_BASE_SEPOLIA,
        executor_address: AccountId::new("0x58ad437cb9e32b0faee810ef05810d5ba2ae52b8".to_string()),
        target_address: AccountId::new(emitter_hex.to_string()),
        tx_hash,
        nonce,
        raw_tx_hex: "0x02f8...cafe".to_string(),
        prepared_at_ms: 1_700_000_000_000,
        protocol_version,
        expected_emitter: AccountId::new(emitter_hex.to_string()),
    }
}

#[tokio::test]
async fn v1_row_persists_and_reads_back_with_v1_generation_identity() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [v1_row_persists_and_reads_back_with_v1_generation_identity] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x51);
    seed_execution_intent(&repo, intent_id).await;
    let tx = tx_hash_hex(0x51);
    repo.record_prepared_transaction(make_record_versioned(
        intent_id,
        tx.clone(),
        5001,
        deopt_v2_backend::execution::PerpsProtocolVersion::V1,
        PME_V1_ADDR,
    ))
    .await
    .expect("insert V1 row");
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .expect("read prepared broadcast")
        .expect("row present");
    assert_eq!(
        row.protocol_version,
        deopt_v2_backend::execution::PerpsProtocolVersion::V1,
        "V1 row must read back as V1"
    );
    assert!(
        row.expected_emitter
            .0
            .eq_ignore_ascii_case(PME_V1_ADDR),
        "V1 row expected_emitter must be the V1 PME address"
    );
    assert_eq!(row.tx_hash, tx);
    assert_eq!(row.nonce, 5001);
}

#[tokio::test]
async fn v2_row_persists_and_reads_back_with_v2_generation_identity() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [v2_row_persists_and_reads_back_with_v2_generation_identity] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x52);
    seed_execution_intent(&repo, intent_id).await;
    let tx = tx_hash_hex(0x52);
    repo.record_prepared_transaction(make_record_versioned(
        intent_id,
        tx.clone(),
        5002,
        deopt_v2_backend::execution::PerpsProtocolVersion::V2,
        PME_V2_ADDR,
    ))
    .await
    .expect("insert V2 row");
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .expect("read prepared broadcast")
        .expect("row present");
    assert_eq!(
        row.protocol_version,
        deopt_v2_backend::execution::PerpsProtocolVersion::V2,
        "V2 row must read back as V2"
    );
    assert!(
        row.expected_emitter
            .0
            .eq_ignore_ascii_case(PME_V2_ADDR),
        "V2 row expected_emitter must be the V2 PME address"
    );
    assert_eq!(row.tx_hash, tx);
    assert_eq!(row.nonce, 5002);
}

// §9 mirror. Simulates the config-drift restart:
//   * insert a V2 broadcast row against the V2 PME;
//   * *do not* mutate the row;
//   * simulate a config flip by reading back the row and asserting
//     its expected_emitter is still the V2 PME.
// This is the reconciler's ONLY source of truth; a runtime
// `PERPS_ACTIVE_ENGINE_VERSION` flip in the interim cannot
// retarget it because the reconciler consumes row.expected_emitter,
// not config.perp_matching_engine_address.
#[tokio::test]
async fn v2_broadcast_row_survives_simulated_active_version_flip() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [v2_broadcast_row_survives_simulated_active_version_flip] (PG url not provided)"
        );
        return;
    };
    let repo = fresh_repo(&url).await;
    let intent_id = unique_intent_id(0x53);
    seed_execution_intent(&repo, intent_id).await;
    let tx = tx_hash_hex(0x53);
    repo.record_prepared_transaction(make_record_versioned(
        intent_id,
        tx.clone(),
        5003,
        deopt_v2_backend::execution::PerpsProtocolVersion::V2,
        PME_V2_ADDR,
    ))
    .await
    .expect("insert V2 row");

    // No mutation; simulate elapsed operator flip of active version.
    // The read-back MUST still be V2 with the V2 PME.
    let row = repo
        .get_prepared_broadcast(intent_id)
        .await
        .expect("read prepared broadcast")
        .expect("row present");
    assert_eq!(
        row.protocol_version,
        deopt_v2_backend::execution::PerpsProtocolVersion::V2,
        "V2 row must NOT be retargeted by a runtime active-version flip"
    );
    assert!(
        row.expected_emitter
            .0
            .eq_ignore_ascii_case(PME_V2_ADDR),
        "V2 row expected_emitter must remain the V2 PME address"
    );
}

// V1 and V2 rows for different intents may coexist without
// interfering with each other's persistence.
#[tokio::test]
async fn mixed_v1_and_v2_rows_coexist() {
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [mixed_v1_and_v2_rows_coexist] (PG url not provided)");
        return;
    };
    let repo = fresh_repo(&url).await;
    let v1_intent = unique_intent_id(0x54);
    let v2_intent = unique_intent_id(0x55);
    seed_execution_intent(&repo, v1_intent).await;
    seed_execution_intent(&repo, v2_intent).await;
    repo.record_prepared_transaction(make_record_versioned(
        v1_intent,
        tx_hash_hex(0x54),
        5010,
        deopt_v2_backend::execution::PerpsProtocolVersion::V1,
        PME_V1_ADDR,
    ))
    .await
    .expect("insert V1 row");
    repo.record_prepared_transaction(make_record_versioned(
        v2_intent,
        tx_hash_hex(0x55),
        5011,
        deopt_v2_backend::execution::PerpsProtocolVersion::V2,
        PME_V2_ADDR,
    ))
    .await
    .expect("insert V2 row");

    let v1_row = repo
        .get_prepared_broadcast(v1_intent)
        .await
        .expect("read V1")
        .expect("row");
    let v2_row = repo
        .get_prepared_broadcast(v2_intent)
        .await
        .expect("read V2")
        .expect("row");
    assert_eq!(
        v1_row.protocol_version,
        deopt_v2_backend::execution::PerpsProtocolVersion::V1
    );
    assert_eq!(
        v2_row.protocol_version,
        deopt_v2_backend::execution::PerpsProtocolVersion::V2
    );
    assert!(
        v1_row.expected_emitter.0.eq_ignore_ascii_case(PME_V1_ADDR),
        "V1 emitter untouched by V2 insert"
    );
    assert!(
        v2_row.expected_emitter.0.eq_ignore_ascii_case(PME_V2_ADDR),
        "V2 emitter untouched by V1 insert"
    );
}
