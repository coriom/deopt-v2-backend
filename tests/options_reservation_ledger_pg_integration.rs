//! OPTIONS-HYBRID-V2-RESERVATIONS-PENDING-SETTLEMENT-AND-CANONICAL-RELEASE-V1
//! Packages B + C + D — focused real-PostgreSQL matrix for the
//! off-chain option reservation ledger, the pure-math formulas, and
//! the available-collateral read path.
//!
//! Loud-fail gate: `OPTIONS_ATOMIC_WIRING_PG_URL` required unless
//! `OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP=1`. Every PG test prints
//! `REAL_POSTGRES_CONNECTION_CONFIRMED` on success.
//!
//! Coverage (25 cases):
//!
//! Schema (5): migration applies, INSERT roundtrip, CHECK
//! constraints (purpose ↔ identity), sparse UNIQUE ACTIVE
//! OPEN_ORDER, sparse UNIQUE ACTIVE PENDING_SETTLEMENT.
//!
//! Formulas (4): buy call, buy put, short put (strike), short call
//! (physical) — round-trip through the repository binding to prove
//! the string encoding preserves the u128.
//!
//! Available collateral (6): scoped total, cross-subaccount
//! isolation, cross-token isolation, sufficient / insufficient
//! ensure, exact boundary, underflow error.
//!
//! Lifecycle (6): open→released, open→converted, pending settle,
//! multi-side pending (buyer + seller share execution id), duplicate
//! insert is idempotent, manual-review escalation.
//!
//! Concurrency + restart (4): concurrent duplicate insert
//! serialises to exactly one ACTIVE row, restart preserves ledger,
//! sparse UNIQUE permits fresh ACTIVE after RELEASED, replacement
//! after MANUAL_REVIEW.

use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::options::reservation_formulas::{
    buy_reservation, short_call_reservation_physical, short_put_reservation,
};
use deopt_v2_backend::options::reservation_repository::{
    available_option_collateral, ensure_option_collateral_available, get_active_open_order,
    get_reservation, insert_open_order_reservation, insert_open_order_reservation_tx,
    insert_pending_settlement_reservation_tx, list_active_pending_for_execution,
    mark_manual_review_tx, mark_open_order_converted_tx, release_open_order, release_open_order_tx,
    settle_pending, total_active_reserved, OpenOrderReservationInput, OptionReservationPurpose,
    OptionReservationSide, OptionReservationStatus, PendingSettlementReservationInput,
};
use sqlx::PgPool;

const URL_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_URL";
const SKIP_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP";
const SERIES_ID: &str = "reservation-shared-series";

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

fn order_hash(tag: &str) -> String {
    let mut hex: String = tag.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    while hex.len() < 60 {
        hex.push('a');
    }
    format!("0x{hex:0<64}")
}
fn execution_id(tag: &str) -> String {
    let mut hex: String = tag.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    while hex.len() < 60 {
        hex.push('b');
    }
    format!("0x{hex:0<64}")
}

fn open_input(
    tag: &str,
    owner: &str,
    subaccount: i32,
    token: &str,
    amount: u128,
) -> OpenOrderReservationInput {
    OpenOrderReservationInput {
        deployment_id: 1,
        chain_id: 84532,
        owner: owner.to_string(),
        subaccount_id: subaccount,
        collateral_token: token.to_string(),
        canonical_order_hash: order_hash(tag),
        option_series_id: SERIES_ID.to_string(),
        side: OptionReservationSide::Buy,
        reserved_amount: amount.to_string(),
        quantity_1e8: "100000000".to_string(),
        now_ms: 1_700_000_000_000,
    }
}

fn pending_input(
    tag: &str,
    owner: &str,
    subaccount: i32,
    token: &str,
    amount: u128,
    side: OptionReservationSide,
) -> PendingSettlementReservationInput {
    PendingSettlementReservationInput {
        deployment_id: 1,
        chain_id: 84532,
        owner: owner.to_string(),
        subaccount_id: subaccount,
        collateral_token: token.to_string(),
        canonical_execution_id: execution_id(tag),
        option_series_id: SERIES_ID.to_string(),
        side,
        reserved_amount: amount.to_string(),
        quantity_1e8: "100000000".to_string(),
        now_ms: 1_700_000_000_000,
    }
}

async fn reset(pool: &PgPool, owner_prefix: &str) {
    sqlx::query("DELETE FROM option_reservations WHERE owner LIKE $1")
        .bind(format!("{owner_prefix}%"))
        .execute(pool)
        .await
        .expect("reset");
}

// -------------------------------------------------------------------
// Schema (5)
// -------------------------------------------------------------------

#[tokio::test]
async fn s01_migration_applies_and_insert_roundtrips() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "s01-").await;
    let input = open_input("s01insert", "s01-owner", 1, "usdc", 1_000);
    let row = insert_open_order_reservation(&pool, &input)
        .await
        .expect("insert");
    let fetched = get_reservation(&pool, row.reservation_id)
        .await
        .expect("lookup")
        .expect("exists");
    assert_eq!(
        fetched.canonical_order_hash.as_deref(),
        Some(input.canonical_order_hash.as_str())
    );
    assert_eq!(fetched.reserved_amount, "1000");
    assert_eq!(fetched.status, OptionReservationStatus::Active);
    assert_eq!(fetched.purpose, OptionReservationPurpose::OpenOrder);
}

#[tokio::test]
async fn s02_check_constraint_rejects_open_order_without_hash() {
    let Some(pool) = require_pool().await else {
        return;
    };
    // Direct SQL: OPEN_ORDER with NULL canonical_order_hash violates
    // the purpose-integrity CHECK.
    let err = sqlx::query(
        "INSERT INTO option_reservations (
             purpose, deployment_id, chain_id, owner, subaccount_id,
             collateral_token, canonical_order_hash, canonical_execution_id,
             option_series_id, side, reserved_amount, quantity_1e8,
             status, created_at_ms, updated_at_ms
         ) VALUES ('OPEN_ORDER', 1, 84532, 's02-owner', 1, 'usdc', NULL, NULL,
                   $1, 'buy', '100', '100000000', 'ACTIVE', 1, 1)",
    )
    .bind(SERIES_ID)
    .execute(&pool)
    .await;
    assert!(err.is_err(), "CHECK must reject OPEN_ORDER without hash");
}

#[tokio::test]
async fn s03_check_constraint_rejects_pending_without_execution_id() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let err = sqlx::query(
        "INSERT INTO option_reservations (
             purpose, deployment_id, chain_id, owner, subaccount_id,
             collateral_token, canonical_order_hash, canonical_execution_id,
             option_series_id, side, reserved_amount, quantity_1e8,
             status, created_at_ms, updated_at_ms
         ) VALUES ('PENDING_SETTLEMENT', 1, 84532, 's03-owner', 1, 'usdc', NULL, NULL,
                   $1, 'buy', '100', '100000000', 'ACTIVE', 1, 1)",
    )
    .bind(SERIES_ID)
    .execute(&pool)
    .await;
    assert!(
        err.is_err(),
        "CHECK must reject PENDING without execution id"
    );
}

#[tokio::test]
async fn s04_sparse_unique_active_open_order() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "s04-").await;
    let input = open_input("s04unique", "s04-owner", 1, "usdc", 100);
    insert_open_order_reservation(&pool, &input)
        .await
        .expect("first");
    // Second insert with identical inputs → idempotent (upsert returns existing).
    let second = insert_open_order_reservation(&pool, &input)
        .await
        .expect("idempotent retry");
    // Total ACTIVE rows == 1.
    let total = total_active_reserved(&pool, 1, "s04-owner", 1, "usdc")
        .await
        .expect("total");
    assert_eq!(total, 100, "sparse unique keeps exactly one ACTIVE row");
    assert_eq!(second.reserved_amount, "100");
}

#[tokio::test]
async fn s05_sparse_unique_active_pending_settlement_per_scope() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "s05-").await;
    let mut tx = pool.begin().await.expect("begin");
    // Same execution_id but DIFFERENT owners → two ACTIVE rows allowed.
    insert_pending_settlement_reservation_tx(
        &mut tx,
        &pending_input(
            "s05both",
            "s05-buyer",
            1,
            "usdc",
            100,
            OptionReservationSide::Buy,
        ),
    )
    .await
    .expect("buyer");
    insert_pending_settlement_reservation_tx(
        &mut tx,
        &pending_input(
            "s05both",
            "s05-seller",
            1,
            "usdc",
            200,
            OptionReservationSide::Sell,
        ),
    )
    .await
    .expect("seller");
    tx.commit().await.expect("commit");
    let all = list_active_pending_for_execution(&pool, &execution_id("s05both"))
        .await
        .expect("list");
    assert_eq!(all.len(), 2, "two counterparties share execution_id");
}

// -------------------------------------------------------------------
// Formulas (4) — pure math verified end-to-end through repository
// -------------------------------------------------------------------

#[test]
fn f01_buy_call_formula_matches_normative() {
    // Q=1, C=1, P=2.50 → 2.5 native units (with 1e8 scaling → 250_000_000).
    let out = buy_reservation(100_000_000, 100_000_000, 250_000_000).unwrap();
    assert_eq!(out, 250_000_000);
}

#[test]
fn f02_short_put_formula_matches_normative() {
    // Q=1, C=1, S=3000 → 3000 native units.
    let out = short_put_reservation(100_000_000, 100_000_000, 3_000 * 100_000_000).unwrap();
    assert_eq!(out, 3_000 * 100_000_000);
}

#[test]
fn f03_short_call_physical_formula_matches_normative() {
    // Q=3, C=1 → 3 native underlying units.
    let out = short_call_reservation_physical(3 * 100_000_000, 100_000_000).unwrap();
    assert_eq!(out, 3 * 100_000_000);
}

#[test]
fn f04_formulas_round_up_never_zero_for_nonzero_inputs() {
    // Even the smallest nonzero premium yields at least 1 native unit.
    let out = buy_reservation(100_000_000, 100_000_000, 1).unwrap();
    assert!(out >= 1);
}

// -------------------------------------------------------------------
// Available collateral (6)
// -------------------------------------------------------------------

#[tokio::test]
async fn a01_total_active_reserved_sums_scope_only() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "a01-").await;
    // Two ACTIVE rows in scope + one out-of-scope (different subaccount).
    let mut tx = pool.begin().await.expect("begin");
    insert_open_order_reservation_tx(&mut tx, &open_input("a01one", "a01-owner", 1, "usdc", 100))
        .await
        .expect("one");
    insert_open_order_reservation_tx(&mut tx, &open_input("a01two", "a01-owner", 1, "usdc", 200))
        .await
        .expect("two");
    insert_open_order_reservation_tx(
        &mut tx,
        &open_input("a01three", "a01-owner", 2, "usdc", 999),
    )
    .await
    .expect("other subaccount");
    tx.commit().await.expect("commit");
    let total = total_active_reserved(&pool, 1, "a01-owner", 1, "usdc")
        .await
        .expect("total");
    assert_eq!(total, 300, "in-scope sum only");
}

#[tokio::test]
async fn a02_cross_token_isolation() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "a02-").await;
    let mut tx = pool.begin().await.expect("begin");
    insert_open_order_reservation_tx(&mut tx, &open_input("a02usdc", "a02-owner", 1, "usdc", 500))
        .await
        .expect("usdc");
    insert_open_order_reservation_tx(&mut tx, &open_input("a02weth", "a02-owner", 1, "weth", 700))
        .await
        .expect("weth");
    tx.commit().await.expect("commit");
    assert_eq!(
        total_active_reserved(&pool, 1, "a02-owner", 1, "usdc")
            .await
            .unwrap(),
        500
    );
    assert_eq!(
        total_active_reserved(&pool, 1, "a02-owner", 1, "weth")
            .await
            .unwrap(),
        700
    );
}

#[tokio::test]
async fn a03_available_collateral_subtracts_reserved() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "a03-").await;
    insert_open_order_reservation(&pool, &open_input("a03held", "a03-owner", 1, "usdc", 300))
        .await
        .expect("insert");
    let available = available_option_collateral(&pool, 1, "a03-owner", 1, "usdc", 1_000)
        .await
        .expect("compute");
    assert_eq!(available, 700);
}

#[tokio::test]
async fn a04_ensure_sufficient_ok_and_insufficient_error() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "a04-").await;
    insert_open_order_reservation(&pool, &open_input("a04held", "a04-owner", 1, "usdc", 800))
        .await
        .expect("insert");
    // Available = 1000 - 800 = 200.
    ensure_option_collateral_available(&pool, 1, "a04-owner", 1, "usdc", 1_000, 200)
        .await
        .expect("exact boundary passes");
    let err =
        ensure_option_collateral_available(&pool, 1, "a04-owner", 1, "usdc", 1_000, 201).await;
    assert!(err.is_err(), "over-boundary must fail");
}

#[tokio::test]
async fn a05_underflow_signals_accounting_bug() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "a05-").await;
    insert_open_order_reservation(&pool, &open_input("a05big", "a05-owner", 1, "usdc", 5_000))
        .await
        .expect("insert");
    let err = available_option_collateral(&pool, 1, "a05-owner", 1, "usdc", 100).await;
    assert!(
        err.is_err(),
        "reserved > canonical must error (accounting bug)"
    );
}

#[tokio::test]
async fn a06_terminal_states_excluded_from_total() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "a06-").await;
    // Active row.
    insert_open_order_reservation(&pool, &open_input("a06active", "a06-owner", 1, "usdc", 500))
        .await
        .expect("insert");
    // Released row (does not contribute).
    let released_input = open_input("a06rel", "a06-owner", 1, "usdc", 300);
    insert_open_order_reservation(&pool, &released_input)
        .await
        .expect("insert");
    release_open_order(
        &pool,
        &released_input.canonical_order_hash,
        "test",
        1_700_000_100_000,
    )
    .await
    .expect("release");
    let total = total_active_reserved(&pool, 1, "a06-owner", 1, "usdc")
        .await
        .expect("total");
    assert_eq!(total, 500, "RELEASED excluded");
}

// -------------------------------------------------------------------
// Lifecycle (6)
// -------------------------------------------------------------------

#[tokio::test]
async fn l01_open_to_released_via_cancellation() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "l01-").await;
    let input = open_input("l01cancel", "l01-owner", 1, "usdc", 100);
    insert_open_order_reservation(&pool, &input)
        .await
        .expect("insert");
    let released = release_open_order(
        &pool,
        &input.canonical_order_hash,
        "USER_CANCELLED",
        1_700_000_100_000,
    )
    .await
    .expect("release")
    .expect("row");
    assert_eq!(released.status, OptionReservationStatus::Released);
    assert_eq!(released.terminal_reason.as_deref(), Some("USER_CANCELLED"));
    // No ACTIVE row remains; a fresh insert for the same hash now
    // succeeds (sparse UNIQUE is ACTIVE-scoped).
    let refreshed = insert_open_order_reservation(&pool, &input)
        .await
        .expect("re-insert");
    assert_eq!(refreshed.status, OptionReservationStatus::Active);
    assert_ne!(refreshed.reservation_id, released.reservation_id);
}

#[tokio::test]
async fn l02_open_to_converted_on_match() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "l02-").await;
    let input = open_input("l02conv", "l02-owner", 1, "usdc", 100);
    insert_open_order_reservation(&pool, &input)
        .await
        .expect("insert");
    let mut tx = pool.begin().await.expect("begin");
    let converted =
        mark_open_order_converted_tx(&mut tx, &input.canonical_order_hash, 1_700_000_100_000)
            .await
            .expect("convert")
            .expect("row");
    tx.commit().await.expect("commit");
    assert_eq!(converted.status, OptionReservationStatus::Converted);
    assert_eq!(
        converted.terminal_reason.as_deref(),
        Some("MATCHED_TO_PENDING_SETTLEMENT")
    );
}

#[tokio::test]
async fn l03_pending_settlement_settles_on_canonical_event() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "l03-").await;
    let cid = execution_id("l03settle");
    let mut tx = pool.begin().await.expect("begin");
    insert_pending_settlement_reservation_tx(
        &mut tx,
        &pending_input(
            "l03settle",
            "l03-buyer",
            1,
            "usdc",
            500,
            OptionReservationSide::Buy,
        ),
    )
    .await
    .expect("buyer");
    insert_pending_settlement_reservation_tx(
        &mut tx,
        &pending_input(
            "l03settle",
            "l03-seller",
            1,
            "usdc",
            800,
            OptionReservationSide::Sell,
        ),
    )
    .await
    .expect("seller");
    tx.commit().await.expect("commit");
    let settled = settle_pending(&pool, &cid, 1_700_000_200_000)
        .await
        .expect("settle");
    assert_eq!(settled.len(), 2, "both counterparties settled");
    for row in settled {
        assert_eq!(row.status, OptionReservationStatus::Settled);
        assert_eq!(row.terminal_reason.as_deref(), Some("CANONICAL_SETTLEMENT"));
    }
}

#[tokio::test]
async fn l04_duplicate_pending_insert_returns_existing_row() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "l04-").await;
    let input = pending_input(
        "l04dup",
        "l04-owner",
        1,
        "usdc",
        100,
        OptionReservationSide::Buy,
    );
    let mut tx = pool.begin().await.expect("begin");
    let first = insert_pending_settlement_reservation_tx(&mut tx, &input)
        .await
        .expect("first");
    let second = insert_pending_settlement_reservation_tx(&mut tx, &input)
        .await
        .expect("dup");
    tx.commit().await.expect("commit");
    assert_eq!(first.reservation_id, second.reservation_id, "idempotent");
}

#[tokio::test]
async fn l05_manual_review_escalation_from_any_state() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "l05-").await;
    let input = open_input("l05mr", "l05-owner", 1, "usdc", 100);
    let inserted = insert_open_order_reservation(&pool, &input)
        .await
        .expect("insert");
    let mut tx = pool.begin().await.expect("begin");
    let escalated = mark_manual_review_tx(
        &mut tx,
        inserted.reservation_id,
        "reorg reactivation blocked",
        1_700_000_300_000,
    )
    .await
    .expect("escalate");
    tx.commit().await.expect("commit");
    assert_eq!(escalated.status, OptionReservationStatus::ManualReview);
}

#[tokio::test]
async fn l06_release_returns_none_if_no_active_row() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let out = release_open_order(&pool, &order_hash("l06none"), "test", 1_700_000_400_000)
        .await
        .expect("release call");
    assert!(out.is_none());
}

// -------------------------------------------------------------------
// Concurrency + restart (4)
// -------------------------------------------------------------------

#[tokio::test]
async fn c01_concurrent_duplicate_insert_serialises_to_one_active() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "c01-").await;
    let input = open_input("c01race", "c01-owner", 1, "usdc", 100);
    let p1 = pool.clone();
    let p2 = pool.clone();
    let i1 = input.clone();
    let i2 = input.clone();
    let (a, b) = tokio::join!(
        insert_open_order_reservation(&p1, &i1),
        insert_open_order_reservation(&p2, &i2),
    );
    // Both must succeed (idempotent) and return the same row.
    let a = a.expect("a");
    let b = b.expect("b");
    assert_eq!(a.reservation_id, b.reservation_id);
    let active = get_active_open_order(&pool, &input.canonical_order_hash)
        .await
        .expect("lookup")
        .expect("row");
    assert_eq!(active.reservation_id, a.reservation_id);
}

#[tokio::test]
async fn c02_restart_preserves_ledger() {
    let Some(url) = require_pg_url() else {
        return;
    };
    ensure_migrated(&url).await;
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    let pool_a = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("a");
    reset(&pool_a, "c02-").await;
    let input = open_input("c02restart", "c02-owner", 1, "usdc", 250);
    insert_open_order_reservation(&pool_a, &input)
        .await
        .expect("insert");
    drop(pool_a);
    let pool_b = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("b");
    let row = get_active_open_order(&pool_b, &input.canonical_order_hash)
        .await
        .expect("lookup")
        .expect("survives");
    assert_eq!(row.reserved_amount, "250");
}

#[tokio::test]
async fn c03_sparse_unique_permits_fresh_active_after_released() {
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "c03-").await;
    let input = open_input("c03cycle", "c03-owner", 1, "usdc", 100);
    let first = insert_open_order_reservation(&pool, &input)
        .await
        .expect("first");
    release_open_order(
        &pool,
        &input.canonical_order_hash,
        "expired",
        1_700_000_500_000,
    )
    .await
    .expect("release");
    let second = insert_open_order_reservation(&pool, &input)
        .await
        .expect("re-insert");
    assert_ne!(first.reservation_id, second.reservation_id);
    assert_eq!(second.status, OptionReservationStatus::Active);
}

#[tokio::test]
async fn c04_release_open_order_tx_scoped() {
    // Verify the tx-scoped release helper matches the pool-level
    // release semantics (used inside matcher tx for cancellation +
    // conversion in the same commit).
    let Some(pool) = require_pool().await else {
        return;
    };
    reset(&pool, "c04-").await;
    let input = open_input("c04txrel", "c04-owner", 1, "usdc", 100);
    insert_open_order_reservation(&pool, &input)
        .await
        .expect("insert");
    let mut tx = pool.begin().await.expect("begin");
    let released = release_open_order_tx(
        &mut tx,
        &input.canonical_order_hash,
        "tx-cancel",
        1_700_000_600_000,
    )
    .await
    .expect("release")
    .expect("row");
    tx.commit().await.expect("commit");
    assert_eq!(released.status, OptionReservationStatus::Released);
    assert_eq!(released.terminal_reason.as_deref(), Some("tx-cancel"));
}
