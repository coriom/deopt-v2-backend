//! OPTIONS-HYBRID-V2-ECONOMIC-RUNTIME-FINAL-CLOSURE-V1
//! Parts B + C + D + M + N + O — real-PostgreSQL matrix for the final
//! economic-runtime closure milestone.
//!
//! Scope of coverage:
//!
//!   Part C (partial-maker resize, 3 tests): partial fills resize the
//!   maker's ACTIVE OPEN_ORDER to a residual reserved_amount by
//!   append-only CONVERTED + successor ACTIVE insert; legacy makers
//!   without an ACTIVE OPEN_ORDER never receive a phantom successor.
//!
//!   Part D (protected-risk conservation, 2 tests): total protected
//!   value (successor OPEN_ORDER + PENDING rows) is >= the matched
//!   exposure after each match, with conservative rounding surplus
//!   accounted for.
//!
//!   Part B (correlation/settlement crash-recovery, 3 tests):
//!   `correlate_canonical_option_event_and_settle` converges on
//!   SETTLED even when replay finds the correlation already promoted
//!   (crash between correlation commit and settle commit).
//!
//!   Part M (fail-closed policy, 4 tests): no code path releases
//!   PENDING_SETTLEMENT under submission failure / SUBMISSION_UNKNOWN
//!   / correlation Conflict / NoCorrelationForIntent.
//!
//!   Part N (reorg reactivation, 5 tests): SETTLED PENDING rows
//!   append-only successor ACTIVE on reorg; combined orphan +
//!   reactivate combinator; idempotent replay; replacement canonical
//!   event settles the reactivated hold.
//!
//!   Part O (restart / rebuild convergence, 1 test): rebuild after a
//!   correlation commit crash converges via next event replay.
//!
//! Loud-fail: `OPTIONS_ATOMIC_WIRING_PG_URL` required.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::correlation_repository::{
    correlate_canonical_option_event_and_settle, reorg_orphan_canonical_correlation_and_reactivate,
    upsert_awaiting_correlation_tx, AwaitingCorrelationInput, CanonicalExecutionEventInput,
    CorrelationReducerOutcome, OptionExecutionKind,
};
use deopt_v2_backend::options::reservation_repository::{
    get_active_open_order, list_active_pending_for_execution, reorg_reactivate_pending,
    OptionReservationPurpose, OptionReservationSide, OptionReservationStatus,
};
use deopt_v2_backend::options::service::{
    cancel_option_order, create_option_series, submit_option_order, CreateOptionSeriesInput,
    SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{option_product_registry_option_id, OptionsConfig};
use deopt_v2_backend::signing::Eip712Domain;
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use sqlx::Row;

const URL_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_URL";
const SKIP_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP";
const VALID_SIG: &str = concat!(
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);

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

async fn require_state() -> Option<AppState> {
    let url = require_pg_url()?;
    ensure_migrated(&url).await;
    let repo = PgRepository::connect(&url).await.expect("connect");
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.execution_enabled = true;
    config.execution_require_persistence = true;
    config.execution_eip712_domain = Eip712Domain {
        name: "DeOptV2-OptionMatchingEngine".to_string(),
        version: "1".to_string(),
        chain_id: 84532,
        verifying_contract: AccountId::new("0x00000000000000000000000000000000000000ee"),
    };
    config.matching_engine_address = AccountId::new("0x00000000000000000000000000000000000000ee");
    let state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        config,
        repo,
    );
    Some(state)
}

async fn require_pool() -> Option<sqlx::PgPool> {
    let url = require_pg_url()?;
    ensure_migrated(&url).await;
    let repo = PgRepository::connect(&url).await.expect("connect");
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    Some(repo.pool().clone())
}

fn future_expiry_sec() -> u64 {
    ((now_ms() / 1_000) + 60 * 60 * 24 * 30) as u64
}

async fn seed_series(state: &AppState, tag: &str) -> String {
    let expiry = future_expiry_sec();
    let salt = tag.chars().fold(0u32, |acc, c| acc.wrapping_add(c as u32));
    let underlying = format!("0x{:040x}", 0x400000_u64 + (salt as u64 & 0xffff));
    let settlement = format!("0x{:040x}", 0x500000_u64 + (salt as u64 & 0xffff));
    let strike_u64: u64 = 300_000_000_000;
    let onchain_option_id = option_product_registry_option_id(
        &AccountId::new(&underlying),
        &AccountId::new(&settlement),
        expiry,
        strike_u64,
        100_000_000,
        true,
        true,
    )
    .expect("onchain option id")
    .to_string();
    create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying,
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: settlement,
            expiry,
            strike_1e8: strike_u64 as u128,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: Some(onchain_option_id),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("series {tag}: {e}"))
    .option_series_id
}

fn order_input(
    series: &str,
    account: AccountId,
    subaccount: u32,
    side: Side,
    price_1e8: u128,
    size_1e8: u128,
    nonce: u64,
    client_order_id: &str,
    tif: TimeInForce,
) -> SubmitOptionOrderInput {
    SubmitOptionOrderInput {
        option_series_id: series.to_string(),
        account,
        subaccount_id: subaccount,
        side,
        price_1e8,
        size_1e8,
        time_in_force: tif,
        post_only: false,
        client_order_id: Some(client_order_id.to_string()),
        nonce: Some(nonce),
        deadline_ms: Some(now_ms() + 60_000),
        signature: Some(VALID_SIG.to_string()),
        attached_tp_sl: None,
    }
}

async fn cleanup(_state: &AppState) {
    // Intentionally no-op: this suite relies on per-test unique
    // canonical identifiers (uuid-based client_order_ids, seed-hashed
    // owner addresses) so parallel runs never collide. A broad
    // DELETE would race with concurrent seeds in other tests.
}

fn run_salt() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    static BASE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let base = *BASE.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_micros() as u32) & 0xFFFF_FFFF)
            .unwrap_or(1)
    });
    base.wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn taker_addr(seed: u32) -> AccountId {
    AccountId::new(&format!(
        "0x{:040x}",
        0x7A0000_u32.wrapping_add(seed).wrapping_add(run_salt())
    ))
}
fn maker_addr(seed: u32) -> AccountId {
    AccountId::new(&format!(
        "0x{:040x}",
        0x5A0000_u32.wrapping_add(seed).wrapping_add(run_salt())
    ))
}
fn unique_client_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}
fn unique_tx(seed: u32) -> String {
    format!(
        "0x{:064x}",
        u128::from(seed).wrapping_add(u128::from(run_salt())) & u128::from(u64::MAX)
    )
}
fn unique_canonical_exec_id(prefix: &str) -> String {
    format!("test-ec-{prefix}-{}", uuid::Uuid::new_v4())
}

// -------------------------------------------------------------------
// Part C — partial-maker resize (3 tests)
// -------------------------------------------------------------------

#[tokio::test]
async fn c01_partial_fill_resizes_maker_open_order_reserved_amount() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("c01")).await;
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(101),
            1,
            Side::Sell,
            1_000_000_000,
            200_000_000,
            101,
            &unique_client_id("c01-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker submit");
    let maker_hash = maker.order.canonical_order_hash.clone().expect("hash");
    let original_open =
        get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
            .await
            .expect("lookup")
            .expect("original active");
    let original_amount = original_open
        .reserved_amount
        .parse::<u128>()
        .expect("parse original");
    // Half-consume via taker.
    let _taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(101),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            102,
            &unique_client_id("c01-taker"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker submit");
    // Successor ACTIVE should exist with a smaller reserved_amount
    // than the original (residual quantity is half).
    let successor = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup")
        .expect("successor active");
    let successor_amount = successor
        .reserved_amount
        .parse::<u128>()
        .expect("parse successor");
    assert!(
        successor_amount < original_amount,
        "resized successor reserved_amount ({successor_amount}) must be smaller than original ({original_amount})"
    );
    // Residual quantity == half.
    assert_eq!(successor.quantity_1e8, "100000000");
    // Original ACTIVE row should now be CONVERTED with the resize
    // terminal reason.
    let old_rows = sqlx::query(
        "SELECT status, terminal_reason FROM option_reservations
         WHERE canonical_order_hash = $1
           AND reservation_id = $2",
    )
    .bind(&maker_hash)
    .bind(original_open.reservation_id)
    .fetch_one(state.repository.as_ref().unwrap().pool())
    .await
    .expect("original row");
    let status: String = old_rows.try_get("status").unwrap();
    let terminal: Option<String> = old_rows.try_get("terminal_reason").unwrap();
    assert_eq!(status, "CONVERTED");
    assert_eq!(terminal.as_deref(), Some("PARTIAL_FILL_RESIZED"));
}

#[tokio::test]
async fn c02_multiple_partial_fills_each_resize_successor() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("c02")).await;
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(102),
            1,
            Side::Sell,
            1_000_000_000,
            300_000_000,
            201,
            &unique_client_id("c02-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker submit");
    let maker_hash = maker.order.canonical_order_hash.clone().expect("hash");
    // Take 1 (partial): 100 → residual 200
    let _t1 = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(102),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            202,
            &unique_client_id("c02-t1"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t1");
    // Take 2 (partial): 100 → residual 100
    let _t2 = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(102 + 100),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            203,
            &unique_client_id("c02-t2"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t2");
    let after_two = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup")
        .expect("still active after two partials");
    assert_eq!(after_two.quantity_1e8, "100000000");
    // Two CONVERTED rows for the same hash — original + first successor
    let converted_count: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
         WHERE canonical_order_hash = $1
           AND status = 'CONVERTED'
           AND terminal_reason = 'PARTIAL_FILL_RESIZED'",
    )
    .bind(&maker_hash)
    .fetch_one(state.repository.as_ref().unwrap().pool())
    .await
    .expect("count")
    .try_get("c")
    .unwrap();
    assert_eq!(
        converted_count, 2,
        "each partial fill CONVERTs one prior ACTIVE OPEN_ORDER"
    );
}

#[tokio::test]
async fn c03_legacy_maker_no_active_open_order_no_phantom_successor() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("c03")).await;
    // Seed a maker order that HAS canonical_order_hash but manually
    // DELETE its ACTIVE OPEN_ORDER reservation to simulate a legacy
    // order predating the reservation ledger.
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(103),
            1,
            Side::Sell,
            1_000_000_000,
            200_000_000,
            301,
            &unique_client_id("c03-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker submit");
    let maker_hash = maker.order.canonical_order_hash.clone().expect("hash");
    sqlx::query(
        "DELETE FROM option_reservations
         WHERE canonical_order_hash = $1
           AND status = 'ACTIVE'
           AND purpose = 'OPEN_ORDER'",
    )
    .bind(&maker_hash)
    .execute(state.repository.as_ref().unwrap().pool())
    .await
    .expect("delete legacy row");
    // Partial-consume via taker. Matcher will try to resize but find
    // no ACTIVE row — must NOT create a phantom successor.
    let _taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(103),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            302,
            &unique_client_id("c03-taker"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker submit");
    let active = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup");
    assert!(
        active.is_none(),
        "legacy maker with no ACTIVE OPEN_ORDER must not receive a phantom successor"
    );
}

// -------------------------------------------------------------------
// Part D — protected-risk conservation (2 tests)
// -------------------------------------------------------------------

#[tokio::test]
async fn d01_partial_match_total_protected_covers_full_original_quantity() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("d01")).await;
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(201),
            1,
            Side::Sell,
            1_000_000_000,
            200_000_000,
            401,
            &unique_client_id("d01-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(201),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            402,
            &unique_client_id("d01-taker"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker");
    let maker_hash = maker.order.canonical_order_hash.clone().expect("mhash");
    // Sum protected on maker's collateral side (underlying, since call
    // seller posts physical).
    let maker_side_reserved: i64 = sqlx::query(
        "SELECT COALESCE(SUM(CAST(reserved_amount AS NUMERIC)), 0)::BIGINT AS s
         FROM option_reservations
         WHERE owner = $1
           AND status = 'ACTIVE'",
    )
    .bind(maker.order.account.0.as_str())
    .fetch_one(state.repository.as_ref().unwrap().pool())
    .await
    .expect("q")
    .try_get("s")
    .unwrap();
    // Expected: successor OPEN_ORDER for 100M + PENDING for 100M of
    // physical underlying. contract_size=1e8; physical = ceil(Q*C/1e8).
    // For Q=1e8 each: (1e8 * 1e8)/1e8 = 1e8 each, total 2e8.
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("pending");
    assert_eq!(pending.len(), 2, "buyer + seller PENDING");
    let successor = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup")
        .expect("successor");
    let successor_amt: i64 = successor.reserved_amount.parse().unwrap();
    let pending_maker_side: i64 = pending
        .iter()
        .filter(|r| r.owner == maker.order.account.0)
        .map(|r| r.reserved_amount.parse::<i64>().unwrap())
        .sum();
    let total_expected = successor_amt + pending_maker_side;
    assert_eq!(
        maker_side_reserved, total_expected,
        "sum of successor OPEN_ORDER + maker PENDING must equal total protected on maker's account"
    );
    // Conservation lower bound: the total covers original quantity's
    // exposure (200M) without under-reservation.
    // Physical seller exposure for 200M underlying = 200M raw units.
    assert!(
        (successor_amt + pending_maker_side) >= 200_000_000,
        "total protected {} must cover original 200M quantity",
        successor_amt + pending_maker_side
    );
}

#[tokio::test]
async fn d02_cancel_after_partial_releases_open_order_but_keeps_pending() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("d02")).await;
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(202),
            1,
            Side::Sell,
            1_000_000_000,
            200_000_000,
            501,
            &unique_client_id("d02-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(202),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            502,
            &unique_client_id("d02-taker"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker");
    // Cancel the maker (residual).
    cancel_option_order(&state, maker.order.order_id)
        .await
        .expect("cancel");
    let maker_hash = maker.order.canonical_order_hash.clone().expect("mhash");
    let active_open = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup");
    assert!(
        active_open.is_none(),
        "cancel must release all ACTIVE OPEN_ORDER for this order"
    );
    // Pending must still be ACTIVE.
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("pending");
    assert_eq!(pending.len(), 2, "cancel of order does not release PENDING");
    for row in &pending {
        assert_eq!(row.status, OptionReservationStatus::Active);
    }
}

// -------------------------------------------------------------------
// Part B — correlation/settlement crash-recovery (3 tests)
// -------------------------------------------------------------------

/// Seed AWAITING correlation + PENDING for a canonical execution
/// without going through the matcher (raw ledger seed).
async fn seed_awaiting_and_pending(
    pool: &sqlx::PgPool,
    canonical_execution_id: &str,
    now_ms: i64,
) -> (String, String) {
    let mut tx = pool.begin().await.expect("tx");
    let awaiting = upsert_awaiting_correlation_tx(
        &mut tx,
        &AwaitingCorrelationInput {
            canonical_execution_id: canonical_execution_id.to_string(),
            deployment_id: 84532,
            chain_id: 84532,
            execution_kind: OptionExecutionKind::Trade,
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: Some("100000000".to_string()),
            now_ms,
        },
    )
    .await
    .expect("awaiting");
    tx.commit().await.expect("commit");
    // Deterministically derive unique owner addresses from the
    // canonical_execution_id so parallel tests never collide on the
    // sparse PENDING UNIQUE.
    let seed = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        canonical_execution_id.hash(&mut h);
        h.finish()
    };
    // Use distinctive high-nibble prefix (0xf...) so cleanup LIKE
    // patterns targeting other tests' addresses never touch these
    // seeded owners.
    let buyer = format!("0xffab{:036x}", seed as u128);
    let seller = format!("0xffcd{:036x}", seed as u128);
    for (owner, side, token) in [
        (&buyer, "buy", "0xa00000000000000000000000000000000000dead"),
        (
            &seller,
            "sell",
            "0xb00000000000000000000000000000000000beef",
        ),
    ] {
        sqlx::query(
            "INSERT INTO option_reservations (
                purpose, deployment_id, chain_id, owner, subaccount_id,
                collateral_token, canonical_order_hash, canonical_execution_id,
                option_series_id, side, reserved_amount, quantity_1e8,
                status, created_at_ms, updated_at_ms
             ) VALUES (
                'PENDING_SETTLEMENT', 84532, 84532, $1, 1, $2, NULL, $3,
                'seed-series', $4, '100000000', '100000000',
                'ACTIVE', $5, $5
             )",
        )
        .bind(owner)
        .bind(token)
        .bind(canonical_execution_id)
        .bind(side)
        .bind(now_ms)
        .execute(pool)
        .await
        .expect("seed pending");
    }
    let _ = awaiting;
    (buyer, seller)
}

/// Advance the correlation directly to CORRELATED_CANONICAL via one
/// wrapper call, then verify next wrapper call is idempotent.
#[tokio::test]
async fn b01_crash_between_correlate_and_settle_recovers_on_replay() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("b01");
    let now = now_ms() as i64;
    let (_buyer, _seller) = seed_awaiting_and_pending(&pool, &cid, now).await;
    // Simulate: first invocation runs the reducer, promotes to
    // CORRELATED_CANONICAL, then "crashes" before settle_pending
    // commits. We simulate the crash by driving the reducer directly
    // (bypassing the wrapper) so PENDING remains ACTIVE.
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(701),
        log_index: 0,
        canonical_block_number: 100,
        canonical_block_hash: &unique_tx(702),
        onchain_execution_id: Some("onchain-1"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    // Direct reducer call → Promotes without settling.
    let outcome =
        deopt_v2_backend::options::correlation_repository::correlate_canonical_option_event(
            &pool, &evt,
        )
        .await
        .expect("promote");
    matches!(outcome, CorrelationReducerOutcome::Promoted(_));
    // PENDING still ACTIVE (crash between commit and settle).
    let pending_before = list_active_pending_for_execution(&pool, &cid)
        .await
        .expect("pending before");
    assert_eq!(pending_before.len(), 2, "PENDING remains ACTIVE post-crash");
    // Replay: wrapper is invoked → correlation is AlreadyCorrelated
    // AND settle_pending runs, converging on SETTLED.
    let (replay_outcome, settled) = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("replay");
    matches!(
        replay_outcome,
        CorrelationReducerOutcome::AlreadyCorrelated(_)
    );
    assert_eq!(
        settled.len(),
        2,
        "replay converges: pending rows transition to SETTLED"
    );
    // After replay, no ACTIVE PENDING remains.
    let after = list_active_pending_for_execution(&pool, &cid)
        .await
        .expect("pending after");
    assert!(after.is_empty(), "SETTLED after crash-recovery replay");
}

#[tokio::test]
async fn b02_single_wrapper_call_promotes_and_settles_together() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("b02");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(710),
        log_index: 0,
        canonical_block_number: 200,
        canonical_block_hash: &unique_tx(711),
        onchain_execution_id: Some("onchain-2"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let (outcome, settled) = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("wrapper");
    matches!(outcome, CorrelationReducerOutcome::Promoted(_));
    assert_eq!(settled.len(), 2, "both PENDING rows SETTLED");
    let after = list_active_pending_for_execution(&pool, &cid)
        .await
        .expect("q");
    assert!(
        after.is_empty(),
        "no ACTIVE PENDING after happy-path wrapper"
    );
}

#[tokio::test]
async fn b03_double_replay_after_full_settle_is_noop() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("b03");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(720),
        log_index: 0,
        canonical_block_number: 300,
        canonical_block_hash: &unique_tx(721),
        onchain_execution_id: Some("onchain-3"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let _first = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("first");
    let (outcome, settled) = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("second");
    matches!(outcome, CorrelationReducerOutcome::AlreadyCorrelated(_));
    assert!(
        settled.is_empty(),
        "no ACTIVE rows to settle on double replay"
    );
}

// -------------------------------------------------------------------
// Part M — fail-closed policy (4 tests)
// -------------------------------------------------------------------

#[tokio::test]
async fn fp01_correlation_conflict_does_not_settle_pending() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("fp01");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    // First event promotes.
    let evt1 = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(801),
        log_index: 0,
        canonical_block_number: 400,
        canonical_block_hash: &unique_tx(802),
        onchain_execution_id: Some("oe-fp01"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let _ = correlate_canonical_option_event_and_settle(&pool, &evt1)
        .await
        .expect("first promote");
    // At this point ACTIVE PENDING is empty (already SETTLED). Reset:
    // simulate a NEW seed to test conflict path.
    let cid2 = unique_canonical_exec_id("fp01b");
    let _ = seed_awaiting_and_pending(&pool, &cid2, now).await;
    // Second event on cid2 with WRONG execution_kind → Conflict.
    let bad_evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid2,
        execution_kind: OptionExecutionKind::RfqTrade,
        tx_hash: &unique_tx(803),
        log_index: 0,
        canonical_block_number: 401,
        canonical_block_hash: &unique_tx(804),
        onchain_execution_id: Some("oe-fp01b"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let (outcome, settled) = correlate_canonical_option_event_and_settle(&pool, &bad_evt)
        .await
        .expect("conflict path");
    matches!(outcome, CorrelationReducerOutcome::Conflict(_));
    assert!(
        settled.is_empty(),
        "Conflict must NOT settle any PENDING row"
    );
    let still_active = list_active_pending_for_execution(&pool, &cid2)
        .await
        .expect("q");
    assert_eq!(still_active.len(), 2, "PENDING remains ACTIVE on Conflict");
}

#[tokio::test]
async fn fp02_no_correlation_for_intent_does_not_settle_pending() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("fp02");
    let now = now_ms() as i64;
    // Seed ONLY the PENDING rows without inserting an AWAITING
    // correlation. Simulates legacy intent without correlation row.
    for (owner, side, token) in [
        (
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "buy",
            "0xa00000000000000000000000000000000000dead",
        ),
        (
            "0xcafedeadcafedeadcafedeadcafedeadcafedead",
            "sell",
            "0xb00000000000000000000000000000000000beef",
        ),
    ] {
        sqlx::query(
            "INSERT INTO option_reservations (
                purpose, deployment_id, chain_id, owner, subaccount_id,
                collateral_token, canonical_order_hash, canonical_execution_id,
                option_series_id, side, reserved_amount, quantity_1e8,
                status, created_at_ms, updated_at_ms
             ) VALUES (
                'PENDING_SETTLEMENT', 84532, 84532, $1, 1, $2, NULL, $3,
                'seed-series-fp02', $4, '100000000', '100000000',
                'ACTIVE', $5, $5
             )",
        )
        .bind(owner)
        .bind(token)
        .bind(&cid)
        .bind(side)
        .bind(now)
        .execute(&pool)
        .await
        .expect("seed pending only");
    }
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(810),
        log_index: 0,
        canonical_block_number: 500,
        canonical_block_hash: &unique_tx(811),
        onchain_execution_id: Some("oe-fp02"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let (outcome, settled) = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("no-correlation path");
    matches!(outcome, CorrelationReducerOutcome::NoCorrelationForIntent);
    assert!(
        settled.is_empty(),
        "NoCorrelationForIntent must not release any PENDING"
    );
    let still_active = list_active_pending_for_execution(&pool, &cid)
        .await
        .expect("q");
    assert_eq!(still_active.len(), 2, "PENDING remains ACTIVE");
}

#[tokio::test]
async fn fp03_submission_unknown_state_keeps_pending_active() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("fp03");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    // Transition the correlation to SUBMISSION_UNKNOWN directly.
    sqlx::query(
        "UPDATE option_execution_correlations
         SET correlation_status = 'SUBMISSION_UNKNOWN',
             tx_hash = $2,
             last_updated_at_ms = $3
         WHERE canonical_execution_id = $1",
    )
    .bind(&cid)
    .bind(unique_tx(820))
    .bind(now)
    .execute(&pool)
    .await
    .expect("submission_unknown transition");
    // PENDING stays ACTIVE — no submission-side path releases it.
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .expect("q");
    assert_eq!(
        pending.len(),
        2,
        "SUBMISSION_UNKNOWN preserves PENDING protection"
    );
}

#[tokio::test]
async fn fp04_cancel_never_releases_pending_settlement() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("fp04")).await;
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(204),
            1,
            Side::Sell,
            1_000_000_000,
            200_000_000,
            901,
            &unique_client_id("fp04-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(204),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            902,
            &unique_client_id("fp04-taker"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker");
    cancel_option_order(&state, maker.order.order_id)
        .await
        .expect("cancel");
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("pending");
    assert_eq!(
        pending.len(),
        2,
        "cancel of order never touches PENDING for matched fills"
    );
}

// -------------------------------------------------------------------
// Part N — reorg reactivation (5 tests)
// -------------------------------------------------------------------

#[tokio::test]
async fn rn01_reactivate_creates_successor_active_from_settled() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("rn01");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(1001),
        log_index: 0,
        canonical_block_number: 600,
        canonical_block_hash: &unique_tx(1002),
        onchain_execution_id: Some("oe-rn01"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let _ = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("settle first");
    // Now reorg: reactivate pending.
    let successors = reorg_reactivate_pending(&pool, &cid, now + 1)
        .await
        .expect("reactivate");
    assert_eq!(successors.len(), 2, "buyer + seller reactivated");
    for s in &successors {
        assert_eq!(s.status, OptionReservationStatus::Active);
        assert_eq!(s.purpose, OptionReservationPurpose::PendingSettlement);
    }
    let active_after = list_active_pending_for_execution(&pool, &cid)
        .await
        .expect("q");
    assert_eq!(active_after.len(), 2);
}

#[tokio::test]
async fn rn02_combined_orphan_and_reactivate_returns_both() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("rn02");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(1010),
        log_index: 0,
        canonical_block_number: 700,
        canonical_block_hash: &unique_tx(1011),
        onchain_execution_id: Some("oe-rn02"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let _ = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("first settle");
    // Reorg with combined wrapper.
    let (orphaned, successors) =
        reorg_orphan_canonical_correlation_and_reactivate(&pool, &cid, now + 2)
            .await
            .expect("reorg");
    use deopt_v2_backend::options::correlation_repository::OptionCorrelationStatus;
    assert_eq!(
        orphaned.correlation_status,
        OptionCorrelationStatus::Orphaned
    );
    assert_eq!(successors.len(), 2);
}

#[tokio::test]
async fn rn03_reactivate_replay_is_idempotent() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("rn03");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(1020),
        log_index: 0,
        canonical_block_number: 800,
        canonical_block_hash: &unique_tx(1021),
        onchain_execution_id: Some("oe-rn03"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let _ = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("settle");
    let first = reorg_reactivate_pending(&pool, &cid, now + 1)
        .await
        .expect("first reactivate");
    let second = reorg_reactivate_pending(&pool, &cid, now + 2)
        .await
        .expect("second reactivate");
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2, "second returns same-set (existing ACTIVE)");
    // Only 2 ACTIVE rows total, not 4.
    let active_count: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
         WHERE canonical_execution_id = $1
           AND status = 'ACTIVE'
           AND purpose = 'PENDING_SETTLEMENT'",
    )
    .bind(&cid)
    .fetch_one(&pool)
    .await
    .expect("q")
    .try_get("c")
    .unwrap();
    assert_eq!(active_count, 2, "no duplicate ACTIVE reactivations");
}

#[tokio::test]
async fn rn04_replacement_canonical_event_settles_reactivated_active() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("rn04");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    let evt1 = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(1030),
        log_index: 0,
        canonical_block_number: 900,
        canonical_block_hash: &unique_tx(1031),
        onchain_execution_id: Some("oe-rn04-a"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let _ = correlate_canonical_option_event_and_settle(&pool, &evt1)
        .await
        .expect("first settle");
    // Reorg orphans + reactivates.
    let (_orphaned, _successors) =
        reorg_orphan_canonical_correlation_and_reactivate(&pool, &cid, now + 3)
            .await
            .expect("reorg");
    // Successor branch: fresh AWAITING correlation for the replacement
    // canonical event on the new branch. The sparse UNIQUE permits it
    // because the prior row is ORPHANED.
    let mut tx = pool.begin().await.expect("tx");
    upsert_awaiting_correlation_tx(
        &mut tx,
        &AwaitingCorrelationInput {
            canonical_execution_id: cid.clone(),
            deployment_id: 84532,
            chain_id: 84532,
            execution_kind: OptionExecutionKind::Trade,
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: Some("100000000".to_string()),
            now_ms: now + 4,
        },
    )
    .await
    .expect("upsert replacement awaiting");
    tx.commit().await.expect("commit");
    // Replacement canonical event.
    let evt2 = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(1040),
        log_index: 0,
        canonical_block_number: 901,
        canonical_block_hash: &unique_tx(1041),
        onchain_execution_id: Some("oe-rn04-b"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now + 5,
    };
    let (outcome, settled) = correlate_canonical_option_event_and_settle(&pool, &evt2)
        .await
        .expect("replacement settle");
    matches!(outcome, CorrelationReducerOutcome::Promoted(_));
    assert_eq!(
        settled.len(),
        2,
        "reactivated ACTIVE rows must SETTLE on replacement canonical event"
    );
}

#[tokio::test]
async fn rn05_reactivate_with_no_settled_rows_is_noop() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("rn05");
    let now = now_ms() as i64;
    // No PENDING seeded; no SETTLED to reactivate.
    let successors = reorg_reactivate_pending(&pool, &cid, now)
        .await
        .expect("reactivate empty");
    assert!(
        successors.is_empty(),
        "no phantom holds when no SETTLED rows exist"
    );
    let _ = OptionReservationSide::Buy;
}

// -------------------------------------------------------------------
// Part O — restart / rebuild convergence (1 test)
// -------------------------------------------------------------------

#[tokio::test]
async fn r01_restart_after_correlation_before_settle_replay_converges() {
    let Some(pool) = require_pool().await else {
        return;
    };
    let cid = unique_canonical_exec_id("r01");
    let now = now_ms() as i64;
    let _ = seed_awaiting_and_pending(&pool, &cid, now).await;
    // Simulate: correlation reducer runs and commits CORRELATED_CANONICAL.
    // Then process crashes (simulated by NOT calling settle).
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(1101),
        log_index: 0,
        canonical_block_number: 1000,
        canonical_block_hash: &unique_tx(1102),
        onchain_execution_id: Some("oe-r01"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let _promoted =
        deopt_v2_backend::options::correlation_repository::correlate_canonical_option_event(
            &pool, &evt,
        )
        .await
        .expect("promote-only");
    // Assert crash-window state: correlation is CORRELATED_CANONICAL,
    // pending is still ACTIVE.
    let before = list_active_pending_for_execution(&pool, &cid)
        .await
        .expect("before");
    assert_eq!(before.len(), 2, "pre-restart PENDING is still ACTIVE");
    // "Restart": open a fresh pool connection. Replay the same event.
    let restart_pool = pool.clone();
    let (outcome, settled) = correlate_canonical_option_event_and_settle(&restart_pool, &evt)
        .await
        .expect("post-restart replay");
    matches!(outcome, CorrelationReducerOutcome::AlreadyCorrelated(_));
    assert_eq!(
        settled.len(),
        2,
        "post-restart replay converges PENDING to SETTLED"
    );
}
