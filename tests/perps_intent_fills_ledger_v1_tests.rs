//! PERPS-CLOSED-TEST-HARDENING-V1 Part B — integration tests for the
//! PG-backed cumulative-fill ledger (`PgIntentFillsLedger`). The struct
//! is exercised against a real Postgres database configured via
//! `PERPS_CLOSED_TEST_E2E_PG_URL`; every test is env-gated so
//! `cargo test` stays green in developer environments without PG.
//!
//! Coverage:
//!
//! 1. `intent_fills_ledger_records_new_intent_ok` — happy path: fresh
//!    intent hash registered; ledger returns `signed_size` and
//!    `filled=0`.
//! 2. `intent_fills_ledger_rejects_size_mismatch_on_reregistration` —
//!    same intent_hash re-registered with a different `signed_size` →
//!    `PerpsIntentCumulativeOverfill`.
//! 3. `intent_fills_ledger_add_fill_bounded_by_signed_size` — legal
//!    fills accumulate; an overfill push returns
//!    `PerpsIntentCumulativeOverfill`; row unchanged after failure.
//! 4. `intent_fills_ledger_concurrent_adds_atomic` — two concurrent
//!    adds each within remaining size succeed; a third that would
//!    over-fill fails-closed. `SELECT ... FOR UPDATE` serialises.
//! 5. `intent_fills_ledger_different_intents_independent` — two
//!    distinct intent hashes accumulate independently.
//!
//! All tests require `PERPS_CLOSED_TEST_E2E_PG_URL`. Without it the
//! test emits `IGNORED (PG url not provided)` and returns early.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::perps::PgIntentFillsLedger;
use deopt_v2_backend::types::AccountId;
use std::sync::Arc;

const PG_ENV_VAR: &str = "PERPS_CLOSED_TEST_E2E_PG_URL";

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

async fn fresh_ledger(url: &str) -> PgIntentFillsLedger {
    PgIntentFillsLedger::new(fresh_repo(url).await)
}

/// Deterministic-but-unique per-test intent hash. Uses the seed to
/// distinguish tests, plus wall-clock nanoseconds to distinguish runs
/// against a shared PG. The tests do NOT rely on a specific hash value
/// — only on it being unique within a run.
fn unique_hash(seed: u8) -> [u8; 32] {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut h = [0u8; 32];
    h[0] = seed;
    let ns_bytes = ns.to_le_bytes();
    h[1..9].copy_from_slice(&ns_bytes);
    for (i, b) in h.iter_mut().enumerate().skip(9) {
        *b = (i as u8).wrapping_mul(seed).wrapping_add(0x91);
    }
    h
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

// ---------------------------------------------------------------------
// 1. Happy-path record.
// ---------------------------------------------------------------------

#[tokio::test]
async fn intent_fills_ledger_records_new_intent_ok() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [intent_fills_ledger_records_new_intent_ok] (PG url not provided)"
        );
        return;
    };
    let ledger = fresh_ledger(&url).await;
    let intent_hash = unique_hash(0x01);
    let trader = addr(0x01);
    let signed_size = 5_000_000_000u128; // 50 * 1e8

    ledger
        .record_intent(intent_hash, &trader, signed_size, 0, 1_700_000_000_000)
        .await
        .expect("record fresh intent");

    let row = ledger
        .get_row(intent_hash)
        .await
        .expect("read back")
        .expect("row present");
    assert_eq!(row.0, signed_size, "signed_size matches");
    assert_eq!(row.1, 0, "filled_size starts at zero");

    // Idempotent re-registration with the SAME signed_size is a no-op.
    ledger
        .record_intent(intent_hash, &trader, signed_size, 0, 1_700_000_000_001)
        .await
        .expect("idempotent re-register");
}

// ---------------------------------------------------------------------
// 2. Size-mismatch re-registration.
// ---------------------------------------------------------------------

#[tokio::test]
async fn intent_fills_ledger_rejects_size_mismatch_on_reregistration() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [intent_fills_ledger_rejects_size_mismatch_on_reregistration] \
             (PG url not provided)"
        );
        return;
    };
    let ledger = fresh_ledger(&url).await;
    let intent_hash = unique_hash(0x02);
    let trader = addr(0x02);
    let signed_size_a: u128 = 1_000_000_000;
    let signed_size_b: u128 = 2_000_000_000;

    ledger
        .record_intent(intent_hash, &trader, signed_size_a, 0, 1_700_000_000_000)
        .await
        .expect("first register");

    let err = ledger
        .record_intent(intent_hash, &trader, signed_size_b, 0, 1_700_000_000_001)
        .await
        .expect_err("second register must fail on size mismatch");
    assert!(
        matches!(err, BackendError::PerpsIntentCumulativeOverfill(_)),
        "expected PerpsIntentCumulativeOverfill, got {err:?}"
    );

    // Row still carries the ORIGINAL signed_size — the failed second
    // registration did NOT overwrite.
    let row = ledger
        .get_row(intent_hash)
        .await
        .expect("read back")
        .expect("row present");
    assert_eq!(row.0, signed_size_a);
    assert_eq!(row.1, 0);
}

// ---------------------------------------------------------------------
// 3. try_add_fill respects the signed-size ceiling.
// ---------------------------------------------------------------------

#[tokio::test]
async fn intent_fills_ledger_add_fill_bounded_by_signed_size() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [intent_fills_ledger_add_fill_bounded_by_signed_size] \
             (PG url not provided)"
        );
        return;
    };
    let ledger = fresh_ledger(&url).await;
    let intent_hash = unique_hash(0x03);
    let trader = addr(0x03);
    let signed_size: u128 = 1_000_000_000; // 10 * 1e8

    ledger
        .record_intent(intent_hash, &trader, signed_size, 0, 1_700_000_000_000)
        .await
        .expect("register");

    // Partial fill 1 (300_000_000). Legal.
    let filled = ledger
        .try_add_fill(intent_hash, 300_000_000, 1_700_000_000_001)
        .await
        .expect("partial 1 legal");
    assert_eq!(filled, 300_000_000);

    // Partial fill 2 (600_000_000). Cumulative = 900_000_000. Legal.
    let filled = ledger
        .try_add_fill(intent_hash, 600_000_000, 1_700_000_000_002)
        .await
        .expect("partial 2 legal");
    assert_eq!(filled, 900_000_000);

    // Overfill attempt: 200_000_000 would push cumulative to
    // 1_100_000_000 > signed 1_000_000_000 → error.
    let err = ledger
        .try_add_fill(intent_hash, 200_000_000, 1_700_000_000_003)
        .await
        .expect_err("overfill must reject");
    assert!(
        matches!(err, BackendError::PerpsIntentCumulativeOverfill(_)),
        "expected PerpsIntentCumulativeOverfill, got {err:?}"
    );

    // Row is untouched by the failed transaction.
    let row = ledger
        .get_row(intent_hash)
        .await
        .expect("read back")
        .expect("row present");
    assert_eq!(row.1, 900_000_000, "filled unchanged after overfill reject");

    // Exact-fit fill: 100_000_000 pushes cumulative to signed. Legal.
    let filled = ledger
        .try_add_fill(intent_hash, 100_000_000, 1_700_000_000_004)
        .await
        .expect("exact-fit legal");
    assert_eq!(filled, signed_size);

    // One more byte over the top → error.
    let err = ledger
        .try_add_fill(intent_hash, 1, 1_700_000_000_005)
        .await
        .expect_err("any add past exact-fit must reject");
    assert!(matches!(err, BackendError::PerpsIntentCumulativeOverfill(_)));
}

// ---------------------------------------------------------------------
// 4. Concurrent adds serialise atomically.
// ---------------------------------------------------------------------

#[tokio::test]
async fn intent_fills_ledger_concurrent_adds_atomic() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [intent_fills_ledger_concurrent_adds_atomic] (PG url not provided)"
        );
        return;
    };
    let ledger = Arc::new(fresh_ledger(&url).await);
    let intent_hash = unique_hash(0x04);
    let trader = addr(0x04);
    let signed_size: u128 = 1_000_000_000;

    ledger
        .record_intent(intent_hash, &trader, signed_size, 0, 1_700_000_000_000)
        .await
        .expect("register");

    // Spawn tasks each attempting to add 200_000_000. With signed_size
    // 1_000_000_000, exactly 5 must succeed; the rest must fail with
    // PerpsIntentCumulativeOverfill.
    //
    // Note: the shared PG pool is sized modestly (max_connections=5 in
    // `crate::db::pool`). We keep concurrency close to that ceiling so
    // the test exercises real racing on the `SELECT ... FOR UPDATE` row
    // lock without swamping the acquire-timeout budget. A larger `n`
    // starves the pool waiters and produces `pool timed out` errors that
    // are pool-plumbing artefacts, not ledger-correctness artefacts.
    let n: usize = 8;
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let ledger = ledger.clone();
        handles.push(tokio::spawn(async move {
            ledger
                .try_add_fill(intent_hash, 200_000_000, 1_700_000_000_100 + i as i64)
                .await
        }));
    }

    let mut ok_count = 0usize;
    let mut overfill_count = 0usize;
    let mut other_count = 0usize;
    for h in handles {
        match h.await.expect("task join") {
            Ok(_) => ok_count += 1,
            Err(BackendError::PerpsIntentCumulativeOverfill(_)) => overfill_count += 1,
            Err(other) => {
                eprintln!("unexpected concurrent error: {other:?}");
                other_count += 1;
            }
        }
    }
    assert_eq!(ok_count, 5, "exactly 5 fills fit into signed_size 1_000_000_000");
    assert_eq!(
        overfill_count + other_count,
        n - 5,
        "remaining tasks must reject; overfill={overfill_count} other={other_count}"
    );
    // At LEAST one must have hit the correctness-guaranteed overfill
    // path; pool-timeout artefacts (`Persistence("pool timed out ...")`)
    // are acceptable when the shared PG pool is contended, provided
    // they DO NOT mask the invariant (which is separately verified by
    // the final assertion below that cumulative == signed).
    assert!(
        overfill_count >= 1,
        "at least one over-limit task must trip PerpsIntentCumulativeOverfill"
    );

    let row = ledger
        .get_row(intent_hash)
        .await
        .expect("read back")
        .expect("row present");
    assert_eq!(row.1, signed_size, "cumulative fill equals signed size exactly");
}

// ---------------------------------------------------------------------
// 5. Different intent hashes accumulate independently.
// ---------------------------------------------------------------------

#[tokio::test]
async fn intent_fills_ledger_different_intents_independent() {
    let Some(url) = pg_url() else {
        eprintln!(
            "IGNORED [intent_fills_ledger_different_intents_independent] \
             (PG url not provided)"
        );
        return;
    };
    let ledger = fresh_ledger(&url).await;
    let hash_a = unique_hash(0x05);
    let hash_b = unique_hash(0x06);
    assert_ne!(hash_a, hash_b, "hashes must differ");
    let trader_a = addr(0x05);
    let trader_b = addr(0x06);
    let signed_a: u128 = 500_000_000;
    let signed_b: u128 = 700_000_000;

    ledger
        .record_intent(hash_a, &trader_a, signed_a, 0, 1_700_000_000_000)
        .await
        .expect("register A");
    ledger
        .record_intent(hash_b, &trader_b, signed_b, 0, 1_700_000_000_001)
        .await
        .expect("register B");

    // Fill A to 400/500 (80%).
    ledger
        .try_add_fill(hash_a, 400_000_000, 1_700_000_000_002)
        .await
        .expect("fill A");
    // Fill B to 100/700 (~14%).
    ledger
        .try_add_fill(hash_b, 100_000_000, 1_700_000_000_003)
        .await
        .expect("fill B");

    let a_row = ledger.get_row(hash_a).await.unwrap().unwrap();
    let b_row = ledger.get_row(hash_b).await.unwrap().unwrap();
    assert_eq!(a_row, (signed_a, 400_000_000));
    assert_eq!(b_row, (signed_b, 100_000_000));

    // Overfilling A must NOT affect B in any way.
    let err = ledger
        .try_add_fill(hash_a, 200_000_000, 1_700_000_000_004)
        .await
        .expect_err("A overfill");
    assert!(matches!(err, BackendError::PerpsIntentCumulativeOverfill(_)));

    let a_row_after = ledger.get_row(hash_a).await.unwrap().unwrap();
    let b_row_after = ledger.get_row(hash_b).await.unwrap().unwrap();
    assert_eq!(a_row_after, a_row, "A row unchanged after failed overfill");
    assert_eq!(b_row_after, b_row, "B row untouched by A operations");

    // B can still accept a fresh fill up to its own ceiling.
    ledger
        .try_add_fill(hash_b, 600_000_000, 1_700_000_000_005)
        .await
        .expect("fill B to ceiling");
    let b_final = ledger.get_row(hash_b).await.unwrap().unwrap();
    assert_eq!(b_final, (signed_b, signed_b));
}
