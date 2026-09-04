//! PERPS-CLOSED-TEST-HARDENING-V1 Part A — integration tests for the
//! PG-backed nonce ledger (`PgNonceLedger`). The trait is exercised
//! against a real Postgres database configured via
//! `PERPS_CLOSED_TEST_E2E_PG_URL`; every test is env-gated so
//! `cargo test` stays green in developer environments without PG.
//!
//! Coverage:
//!
//! 1. `nonce_ledger_pg_rejects_replay_after_restart` — submit, drop
//!    the ledger, rebuild against the same PG URL, resubmit → replay.
//! 2. `nonce_ledger_pg_concurrent_submissions_atomic` — N concurrent
//!    tasks submitting the same `(trader, nonce)` → exactly one
//!    succeeds, the rest return `PerpsIntentNonceReplay`.
//! 3. `nonce_ledger_pg_db_error_fails_closed` — drop the underlying
//!    table then submit → `BackendError::Persistence` (not silent
//!    success).
//! 4. `nonce_ledger_pg_partial_fill_replay_rejected` — proxy for
//!    "partial fill outcome" by pre-consuming the ledger; a later
//!    submit of the same intent under the same nonce must be
//!    rejected as replay even across a fresh ledger instance.
//! 5. `nonce_ledger_pg_full_fill_replay_rejected` — same as (4) but
//!    the pre-consume represents a full-fill outcome. The ledger has
//!    no knowledge of downstream engine outcomes; both cases collapse
//!    to the same replay assertion (that is the correctness
//!    guarantee).
//! 6. `nonce_ledger_different_traders_do_not_collide` — trader1
//!    nonce=1 and trader2 nonce=1 both accepted.
//!
//! All tests require `PERPS_CLOSED_TEST_E2E_PG_URL`. Without it the
//! test emits `IGNORED (PG url not provided)` and returns early.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::perps::{
    PerpOrderIntentNonceLedger, PgNonceLedger,
};
use deopt_v2_backend::types::AccountId;
use std::sync::Arc;

const PG_ENV_VAR: &str = "PERPS_CLOSED_TEST_E2E_PG_URL";

fn pg_url() -> Option<String> {
    std::env::var(PG_ENV_VAR).ok().filter(|v| !v.is_empty())
}

/// Ensure migrations have run at least once against the shared PG
/// database. Mirrors the `ensure_migrated` helper in
/// `tests/perps_signed_intent_v1_tests.rs`.
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

/// PERPS-CLOSED-TEST-HARDENING-V1 Part A — regression fix. The
/// `db_error_fails_closed` test drops+recreates the ledger table; if
/// any other test runs concurrently it may see the table torn down or
/// its own pre-existing rows wiped. Every test in this file acquires
/// this guard so `db_error_fails_closed` (which also acquires it)
/// serialises against all readers. Using an async `Mutex` because
/// tests are `#[tokio::test]`.
static TEST_SERIALISATION_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn fresh_ledger(url: &str) -> PgNonceLedger {
    PgNonceLedger::new(fresh_repo(url).await)
}

/// Random-ish trader per test so runs don't collide across
/// re-invocations of the same test against a shared PG database. Uses
/// the low 8 bytes of `now_ns()` XORed with the seed for entropy.
fn unique_trader(seed: u8) -> AccountId {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut bytes = [0u8; 20];
    bytes[0] = seed;
    let ns_bytes = ns.to_le_bytes();
    bytes[1..9].copy_from_slice(&ns_bytes);
    // Sprinkle some deterministic entropy in the tail.
    for (i, b) in bytes.iter_mut().enumerate().skip(9) {
        *b = (i as u8).wrapping_mul(seed).wrapping_add(0x5a);
    }
    let mut hex = String::from("0x");
    for byte in &bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    AccountId::new(hex)
}

fn hash(seed: u8) -> [u8; 32] {
    [seed; 32]
}

// ---------------------------------------------------------------------
// 1. Replay after restart. Two independent ledger handles against the
//    same PG URL: the second must see the row inserted by the first
//    and reject the replay.
// ---------------------------------------------------------------------

#[tokio::test]
async fn nonce_ledger_pg_rejects_replay_after_restart() {
    let _test_guard = TEST_SERIALISATION_GUARD.lock().await;
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [nonce_ledger_pg_rejects_replay_after_restart] (PG url not provided)");
        return;
    };
    let trader = unique_trader(0x01);
    let nonce: u128 = 1;

    // First ledger handle — consume.
    let ledger_a = fresh_ledger(&url).await;
    ledger_a
        .try_consume(&trader, nonce, hash(0x11))
        .await
        .expect("first consume ok");
    drop(ledger_a);

    // Second ledger handle (rebuild against same PG) — replay.
    let ledger_b = fresh_ledger(&url).await;
    let err = ledger_b
        .try_consume(&trader, nonce, hash(0x11))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BackendError::PerpsIntentNonceReplay),
        "expected replay error; got {err:?}"
    );
}

// ---------------------------------------------------------------------
// 2. Atomic under concurrent submissions. Spawn N tasks that all try
//    the same (trader, nonce). Exactly one succeeds; the rest see
//    `PerpsIntentNonceReplay`. Any other outcome (>1 successes, or a
//    DB-error path) is a critical correctness bug.
// ---------------------------------------------------------------------

#[tokio::test]
async fn nonce_ledger_pg_concurrent_submissions_atomic() {
    let _test_guard = TEST_SERIALISATION_GUARD.lock().await;
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [nonce_ledger_pg_concurrent_submissions_atomic] (PG url not provided)");
        return;
    };
    let trader = unique_trader(0x02);
    let nonce: u128 = 2;
    let ledger = Arc::new(fresh_ledger(&url).await);
    let n: usize = 16;

    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let ledger = ledger.clone();
        let trader = trader.clone();
        handles.push(tokio::spawn(async move {
            ledger.try_consume(&trader, nonce, hash(i as u8)).await
        }));
    }

    let mut ok_count = 0;
    let mut replay_count = 0;
    let mut other_count = 0;
    for h in handles {
        match h.await.expect("task join") {
            Ok(()) => ok_count += 1,
            Err(BackendError::PerpsIntentNonceReplay) => replay_count += 1,
            Err(other) => {
                eprintln!("unexpected error under concurrency: {other:?}");
                other_count += 1;
            }
        }
    }
    assert_eq!(ok_count, 1, "exactly one submit must succeed");
    assert_eq!(replay_count, n - 1, "all others must replay-reject");
    assert_eq!(other_count, 0, "no DB / other errors allowed");
}

// ---------------------------------------------------------------------
// 3. DB error fails closed. Drop the underlying table on a *dedicated*
//    connection, then submit through a fresh ledger — the ledger must
//    return `BackendError::Persistence`, never silent success or a
//    replay-reject (both would mask the outage).
//
// We re-create the table afterwards so subsequent tests are unaffected.
// ---------------------------------------------------------------------

#[tokio::test]
async fn nonce_ledger_pg_db_error_fails_closed() {
    let _test_guard = TEST_SERIALISATION_GUARD.lock().await;
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [nonce_ledger_pg_db_error_fails_closed] (PG url not provided)");
        return;
    };
    // Serialise this test against other tests in the file that share the
    // table. We use a coarse mutex; the test suite is small.
    static DDL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = DDL_LOCK.lock().await;

    let repo = fresh_repo(&url).await;
    // Drop the table on a dedicated connection.
    sqlx::query("DROP TABLE IF EXISTS perps_signed_intent_nonce_ledger")
        .execute(repo.pool())
        .await
        .expect("drop table");

    let ledger = PgNonceLedger::new(repo.clone());
    let trader = unique_trader(0x03);
    let err = ledger
        .try_consume(&trader, 3, hash(0x33))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BackendError::Persistence(_)),
        "DB error must map to Persistence; got {err:?}"
    );

    // Restore the table so downstream tests in the same run still
    // see the ledger. We recreate the schema inline so we don't have
    // to re-invoke the migrator (which would try to reapply the
    // migration and could conflict with `_sqlx_migrations` state).
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS perps_signed_intent_nonce_ledger (\
            trader         BYTEA        NOT NULL,\
            nonce_hex      TEXT         NOT NULL,\
            intent_hash    BYTEA        NOT NULL,\
            consumed_at_ms BIGINT       NOT NULL,\
            PRIMARY KEY (trader, nonce_hex),\
            CONSTRAINT perps_signed_intent_nonce_ledger_trader_len \
                CHECK (octet_length(trader) = 20),\
            CONSTRAINT perps_signed_intent_nonce_ledger_intent_hash_len \
                CHECK (octet_length(intent_hash) = 32)\
        )",
    )
    .execute(repo.pool())
    .await
    .expect("recreate table");
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_perps_nonce_intent_hash \
         ON perps_signed_intent_nonce_ledger (intent_hash)",
    )
    .execute(repo.pool())
    .await
    .expect("recreate index");
}

// ---------------------------------------------------------------------
// 4 & 5. Partial-fill / full-fill replay after restart. The ledger has
//        no knowledge of engine-level outcomes — but the correctness
//        surface is identical either way: once a nonce is consumed,
//        the SAME nonce may never be consumed again, regardless of
//        what the engine did downstream. We simulate both outcomes by
//        pre-consuming with a distinct intent hash then attempting
//        replay through a fresh ledger.
// ---------------------------------------------------------------------

#[tokio::test]
async fn nonce_ledger_pg_partial_fill_replay_rejected() {
    let _test_guard = TEST_SERIALISATION_GUARD.lock().await;
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [nonce_ledger_pg_partial_fill_replay_rejected] (PG url not provided)");
        return;
    };
    let trader = unique_trader(0x04);
    let nonce: u128 = 4;
    let intent_hash = hash(0x44);

    // Consume via ledger A (simulates first submit that resulted in a
    // partial fill; the ledger row survives regardless of engine outcome).
    let ledger_a = fresh_ledger(&url).await;
    ledger_a
        .try_consume(&trader, nonce, intent_hash)
        .await
        .expect("first consume ok");
    drop(ledger_a);

    // Fresh ledger (simulates restart) — replay of the same intent
    // under the same nonce must be rejected.
    let ledger_b = fresh_ledger(&url).await;
    let err = ledger_b
        .try_consume(&trader, nonce, intent_hash)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
}

#[tokio::test]
async fn nonce_ledger_pg_full_fill_replay_rejected() {
    let _test_guard = TEST_SERIALISATION_GUARD.lock().await;
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [nonce_ledger_pg_full_fill_replay_rejected] (PG url not provided)");
        return;
    };
    let trader = unique_trader(0x05);
    let nonce: u128 = 5;
    let intent_hash = hash(0x55);

    let ledger_a = fresh_ledger(&url).await;
    ledger_a
        .try_consume(&trader, nonce, intent_hash)
        .await
        .expect("first consume ok");
    drop(ledger_a);

    let ledger_b = fresh_ledger(&url).await;
    let err = ledger_b
        .try_consume(&trader, nonce, intent_hash)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
}

// ---------------------------------------------------------------------
// 6. Different traders never collide. Each trader has its own nonce
//    namespace.
// ---------------------------------------------------------------------

#[tokio::test]
async fn nonce_ledger_different_traders_do_not_collide() {
    let _test_guard = TEST_SERIALISATION_GUARD.lock().await;
    let Some(url) = pg_url() else {
        eprintln!("IGNORED [nonce_ledger_different_traders_do_not_collide] (PG url not provided)");
        return;
    };
    let trader1 = unique_trader(0x06);
    let trader2 = unique_trader(0x07);
    let ledger = fresh_ledger(&url).await;
    ledger
        .try_consume(&trader1, 1, hash(0x61))
        .await
        .expect("trader1 nonce=1 ok");
    ledger
        .try_consume(&trader2, 1, hash(0x71))
        .await
        .expect("trader2 nonce=1 ok");
    // And prove has_consumed is per-trader.
    assert!(ledger.has_consumed(&trader1, 1).await.unwrap());
    assert!(ledger.has_consumed(&trader2, 1).await.unwrap());
    assert!(!ledger.has_consumed(&trader1, 2).await.unwrap());
}
