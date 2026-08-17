//! OPTIONS-HYBRID-V2-ECONOMIC-RUNTIME-FINAL-VALIDATION-V1
//! Gap-close matrix for the economic runtime — real-PostgreSQL only.
//!
//! Group A — canonical position boundary (2 tests):
//!   * boundary01: pending settlement does NOT mutate the canonical
//!     position projection (`hybrid_v2_positions`); only PENDING rows
//!     exist for the fill.
//!   * boundary02: canonical_execution_id populated on the fill pre-chain
//!     but no premium settlement is applied (correlation is still
//!     AWAITING_CHAIN_EVIDENCE and PENDING pair is present).
//!
//! Group B — settlement economics truth table (5 tests):
//!   * truth01: CALL buyer full fill — buyer PENDING == buy_reservation
//!     ceil_div formula.
//!   * truth02: CALL seller short-physical reservation — seller PENDING
//!     == short_call_reservation_physical formula.
//!   * truth03: PUT buyer full fill — buyer PENDING same formula.
//!   * truth04: PUT seller reserves strike-notional — seller PENDING ==
//!     short_put_reservation formula.
//!   * truth05: partial fills use per-fill ceil_div — sum of per-fill
//!     amounts equals the sum of the PENDING amounts recorded.
//!
//! Group C — same-wallet cross-subaccount settlement isolation (1 test):
//!   * truth06: same owner submits both sides via different subaccounts.
//!     Each subaccount's PENDING amounts are independent (no netting).
//!
//! Group D — rebuild economic state convergence (3 tests):
//!   * rebuild01: submit + partial match + cancel residual — snapshot
//!     final reservation set and prove the set is deterministic under
//!     replay of the same inputs.
//!   * rebuild02: post-reactivate expected ACTIVE PENDING set (via r09
//!     reactivate path).
//!   * rebuild03: crash between correlate and settle — replay converges
//!     to the same final state as the no-crash case.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::correlation_repository::{
    correlate_canonical_option_event, correlate_canonical_option_event_and_settle,
    upsert_awaiting_correlation_tx, AwaitingCorrelationInput, CanonicalExecutionEventInput,
    OptionCorrelationStatus, OptionExecutionKind,
};
use deopt_v2_backend::options::reservation_repository::{
    list_active_pending_for_execution, reorg_reactivate_pending, OptionReservationStatus,
};
use deopt_v2_backend::options::service::{
    cancel_option_order, create_option_series, submit_option_order, CreateOptionSeriesInput,
    SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{option_product_registry_option_id, OptionsConfig};
use deopt_v2_backend::signing::Eip712Domain;
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use sqlx::{PgPool, Row};

const URL_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_URL";
const SKIP_ENV: &str = "OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP";
const VALID_SIG: &str = concat!(
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);
const CONTRACT_SIZE: u128 = 100_000_000;

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
    Some(AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        config,
        repo,
    ))
}
async fn require_pool() -> Option<PgPool> {
    let url = require_pg_url()?;
    ensure_migrated(&url).await;
    let repo = PgRepository::connect(&url).await.expect("connect");
    Some(repo.pool().clone())
}

fn future_expiry_sec() -> u64 {
    ((now_ms() / 1_000) + 60 * 60 * 24 * 30) as u64
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
fn unique_client_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}
fn unique_canonical_exec_id(prefix: &str) -> String {
    format!("test-fv-{prefix}-{}", uuid::Uuid::new_v4())
}
fn unique_tx(seed: u32) -> String {
    format!(
        "0x{:064x}",
        u128::from(seed).wrapping_add(u128::from(run_salt())) & u128::from(u64::MAX)
    )
}
fn addr(prefix: u32, seed: u32) -> AccountId {
    AccountId::new(&format!(
        "0x{:040x}",
        (prefix as u64) << 32 | (seed.wrapping_add(run_salt())) as u64
    ))
}

async fn seed_series_ex(
    state: &AppState,
    tag: &str,
    is_call: bool,
    underlying_prefix: u64,
    settlement_prefix: u64,
) -> String {
    let expiry = future_expiry_sec();
    let salt = tag.chars().fold(0u32, |acc, c| acc.wrapping_add(c as u32));
    let underlying = format!(
        "0x{:040x}",
        underlying_prefix + (salt as u64 & 0xffff) + run_salt() as u64
    );
    let settlement = format!(
        "0x{:040x}",
        settlement_prefix + (salt as u64 & 0xffff) + run_salt() as u64
    );
    let strike_u64: u64 = 300_000_000_000;
    let onchain_option_id = option_product_registry_option_id(
        &AccountId::new(&underlying),
        &AccountId::new(&settlement),
        expiry,
        strike_u64,
        100_000_000,
        is_call,
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
            is_call,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: Some(onchain_option_id),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("series {tag}: {e}"))
    .option_series_id
}

async fn seed_call_series(state: &AppState, tag: &str) -> String {
    seed_series_ex(state, tag, true, 0x600000, 0x700000).await
}

async fn seed_put_series(state: &AppState, tag: &str) -> String {
    seed_series_ex(state, tag, false, 0x680000, 0x780000).await
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

// ceil_div formulas mirrored (u128 wide-enough for the test range).
fn buy_res_ceil(q: u128, c: u128, p: u128) -> u128 {
    // ceil_div(q*c*p, 1e16)
    let num = q * c * p;
    (num + 10_000_000_000_000_000u128 - 1) / 10_000_000_000_000_000u128
}
fn short_call_phys_ceil(q: u128, c: u128) -> u128 {
    // ceil_div(q*c, 1e8)
    let num = q * c;
    (num + 100_000_000u128 - 1) / 100_000_000u128
}
fn short_put_ceil(q: u128, c: u128, s: u128) -> u128 {
    // ceil_div(q*c*s, 1e16)
    let num = q * c * s;
    (num + 10_000_000_000_000_000u128 - 1) / 10_000_000_000_000_000u128
}

async fn seed_awaiting_and_pending(
    pool: &PgPool,
    canonical_execution_id: &str,
    now_ms_i: i64,
    tag_seed: u32,
) -> (String, String) {
    let mut tx = pool.begin().await.expect("tx");
    let _awaiting = upsert_awaiting_correlation_tx(
        &mut tx,
        &AwaitingCorrelationInput {
            canonical_execution_id: canonical_execution_id.to_string(),
            deployment_id: 84532,
            chain_id: 84532,
            execution_kind: OptionExecutionKind::Trade,
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: Some("100000000".to_string()),
            now_ms: now_ms_i,
        },
    )
    .await
    .expect("awaiting");
    tx.commit().await.expect("commit");
    let seed = tag_seed.wrapping_add(run_salt());
    let buyer = format!("0xffab{:036x}", seed as u128);
    let seller = format!("0xffcd{:036x}", seed as u128);
    for (owner, side, token) in [
        (
            &buyer,
            "buy",
            format!("0xa000000000000000000000000000{:012x}", seed as u128),
        ),
        (
            &seller,
            "sell",
            format!("0xb000000000000000000000000000{:012x}", seed as u128),
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
        .bind(&token)
        .bind(canonical_execution_id)
        .bind(side)
        .bind(now_ms_i)
        .execute(pool)
        .await
        .expect("seed pending");
    }
    (buyer, seller)
}

// -------------------------------------------------------------------
// Group A — canonical position boundary
// -------------------------------------------------------------------

#[tokio::test]
async fn boundary01_pending_settlement_does_not_mutate_canonical_position() {
    // The canonical position projection is `hybrid_v2_positions`
    // (chain-derived via the HV2 reducer). Off-chain matcher writes
    // must NEVER touch that table — only PENDING_SETTLEMENT
    // reservations should exist for the fill until the canonical
    // chain event lands.
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("bd01")).await;
    let maker = addr(0x51, 1);
    let taker = addr(0x51, 2);
    let pool = state.repository.as_ref().unwrap().pool().clone();

    // Snapshot canonical position count for both participants
    // BEFORE the match.
    let before_maker = position_row_count(&pool, &maker.0).await;
    let before_taker = position_row_count(&pool, &taker.0).await;

    submit_option_order(
        &state,
        order_input(
            &series,
            maker.clone(),
            1,
            Side::Sell,
            1_000_000_000,
            CONTRACT_SIZE,
            1,
            &unique_client_id("bd01-m"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let t = submit_option_order(
        &state,
        order_input(
            &series,
            taker.clone(),
            1,
            Side::Buy,
            1_000_000_000,
            CONTRACT_SIZE,
            2,
            &unique_client_id("bd01-t"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker");
    assert_eq!(t.fills.len(), 1);
    let cid = t.fills[0].canonical_execution_id.clone().unwrap();
    // PENDING pair must exist post-match.
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2, "PENDING pair exists for the fill");
    for row in &pending {
        assert_eq!(row.status, OptionReservationStatus::Active);
    }
    // No canonical (chain-derived) position row should have been
    // created — matcher must never mutate `hybrid_v2_positions`.
    let after_maker = position_row_count(&pool, &maker.0).await;
    let after_taker = position_row_count(&pool, &taker.0).await;
    assert_eq!(
        after_maker, before_maker,
        "matcher must not write to canonical position projection for maker"
    );
    assert_eq!(
        after_taker, before_taker,
        "matcher must not write to canonical position projection for taker"
    );
}

#[tokio::test]
async fn boundary02_canonical_execution_id_present_prechain_no_premium_transfer() {
    // After match, before any chain event lands: the fill row carries
    // a canonical_execution_id AND a PENDING pair exists, but the
    // correlation is still AWAITING_CHAIN_EVIDENCE (no premium
    // transfer / no on-chain settlement).
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("bd02")).await;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x52, 1),
            1,
            Side::Sell,
            1_000_000_000,
            CONTRACT_SIZE,
            10,
            &unique_client_id("bd02-m"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x52, 2),
            1,
            Side::Buy,
            1_000_000_000,
            CONTRACT_SIZE,
            11,
            &unique_client_id("bd02-t"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker");
    let fill = &taker.fills[0];
    let cid = fill.canonical_execution_id.clone().unwrap();
    // (1) Fill row in DB carries canonical_execution_id
    let db_cid: Option<String> =
        sqlx::query("SELECT canonical_execution_id FROM option_fills WHERE fill_id = $1")
            .bind(fill.fill_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fill row")
            .try_get("canonical_execution_id")
            .unwrap();
    assert_eq!(db_cid.as_deref(), Some(cid.as_str()));
    // (2) PENDING pair present
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
    // (3) Correlation is AWAITING_CHAIN_EVIDENCE with tx_hash NULL
    // and canonical_block_number NULL (no chain event processed).
    let row = sqlx::query(
        "SELECT correlation_status, tx_hash, canonical_block_number, onchain_execution_id
         FROM option_execution_correlations WHERE canonical_execution_id = $1",
    )
    .bind(&cid)
    .fetch_one(&pool)
    .await
    .expect("correlation row");
    let status: String = row.try_get("correlation_status").unwrap();
    let tx_hash: Option<String> = row.try_get("tx_hash").unwrap();
    let block: Option<i64> = row.try_get("canonical_block_number").unwrap();
    let onchain_exec: Option<String> = row.try_get("onchain_execution_id").unwrap();
    assert_eq!(
        status,
        OptionCorrelationStatus::AwaitingChainEvidence.as_str()
    );
    assert!(tx_hash.is_none(), "no tx_hash pre-chain");
    assert!(block.is_none(), "no canonical block pre-chain");
    assert!(onchain_exec.is_none(), "no onchain execution id pre-chain");
}

// -------------------------------------------------------------------
// Group B — settlement economics truth table
// -------------------------------------------------------------------

#[tokio::test]
async fn truth01_call_buyer_full_fill_ceildiv_matches_spec() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("t01")).await;
    let price = 1_000_000_000u128;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x53, 1),
            1,
            Side::Sell,
            price,
            CONTRACT_SIZE,
            100,
            &unique_client_id("t01-m"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("m");
    let buyer_addr = addr(0x53, 2);
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            buyer_addr.clone(),
            1,
            Side::Buy,
            price,
            CONTRACT_SIZE,
            101,
            &unique_client_id("t01-t"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t");
    let cid = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
    let buyer_pending = pending
        .iter()
        .find(|r| r.owner == buyer_addr.0)
        .expect("buyer pending");
    let expected = buy_res_ceil(CONTRACT_SIZE, CONTRACT_SIZE, price);
    let actual: u128 = buyer_pending.reserved_amount.parse().unwrap();
    assert_eq!(actual, expected, "buyer PENDING == ceil_div(Q*C*P, 1e16)");
}

#[tokio::test]
async fn truth02_call_seller_short_physical_reservation() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("t02")).await;
    let price = 1_000_000_000u128;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    let seller_addr = addr(0x54, 1);
    submit_option_order(
        &state,
        order_input(
            &series,
            seller_addr.clone(),
            1,
            Side::Sell,
            price,
            CONTRACT_SIZE,
            110,
            &unique_client_id("t02-m"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("m");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x54, 2),
            1,
            Side::Buy,
            price,
            CONTRACT_SIZE,
            111,
            &unique_client_id("t02-t"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t");
    let cid = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    let seller_pending = pending
        .iter()
        .find(|r| r.owner == seller_addr.0)
        .expect("seller pending");
    let expected = short_call_phys_ceil(CONTRACT_SIZE, CONTRACT_SIZE);
    let actual: u128 = seller_pending.reserved_amount.parse().unwrap();
    assert_eq!(
        actual, expected,
        "seller PENDING == ceil_div(Q*C, 1e8) == Q (for Q=C=1e8)"
    );
}

#[tokio::test]
async fn truth03_put_buyer_full_fill_ceildiv_matches_spec() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_put_series(&state, &unique_client_id("t03")).await;
    let price = 500_000_000u128;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x55, 1),
            1,
            Side::Sell,
            price,
            CONTRACT_SIZE,
            120,
            &unique_client_id("t03-m"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("m");
    let buyer_addr = addr(0x55, 2);
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            buyer_addr.clone(),
            1,
            Side::Buy,
            price,
            CONTRACT_SIZE,
            121,
            &unique_client_id("t03-t"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t");
    let cid = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    let buyer_pending = pending
        .iter()
        .find(|r| r.owner == buyer_addr.0)
        .expect("buyer pending");
    let expected = buy_res_ceil(CONTRACT_SIZE, CONTRACT_SIZE, price);
    let actual: u128 = buyer_pending.reserved_amount.parse().unwrap();
    assert_eq!(
        actual, expected,
        "put buyer PENDING == ceil_div(Q*C*P, 1e16)"
    );
}

#[tokio::test]
async fn truth04_put_seller_short_reserves_strike_notional() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_put_series(&state, &unique_client_id("t04")).await;
    let price = 500_000_000u128;
    let strike = 300_000_000_000u128;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    let seller_addr = addr(0x56, 1);
    submit_option_order(
        &state,
        order_input(
            &series,
            seller_addr.clone(),
            1,
            Side::Sell,
            price,
            CONTRACT_SIZE,
            130,
            &unique_client_id("t04-m"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("m");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x56, 2),
            1,
            Side::Buy,
            price,
            CONTRACT_SIZE,
            131,
            &unique_client_id("t04-t"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t");
    let cid = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    let seller_pending = pending
        .iter()
        .find(|r| r.owner == seller_addr.0)
        .expect("seller pending");
    let expected = short_put_ceil(CONTRACT_SIZE, CONTRACT_SIZE, strike);
    let actual: u128 = seller_pending.reserved_amount.parse().unwrap();
    assert_eq!(
        actual, expected,
        "put seller PENDING == ceil_div(Q*C*S, 1e16)"
    );
}

#[tokio::test]
async fn truth05_partial_fill_reservations_use_per_fill_ceildiv_no_double_count() {
    // Maker of 3 contracts; take 2, then take 1. Sum of per-fill
    // ceil_div amounts must equal the sum of PENDING amounts recorded.
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("t05")).await;
    let price = 1_000_000_000u128;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    let maker_addr = addr(0x57, 1);
    submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr.clone(),
            1,
            Side::Sell,
            price,
            3 * CONTRACT_SIZE,
            140,
            &unique_client_id("t05-m"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("m");
    let t1 = submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x57, 2),
            1,
            Side::Buy,
            price,
            2 * CONTRACT_SIZE,
            141,
            &unique_client_id("t05-t1"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t1");
    let t2 = submit_option_order(
        &state,
        order_input(
            &series,
            addr(0x57, 3),
            1,
            Side::Buy,
            price,
            CONTRACT_SIZE,
            142,
            &unique_client_id("t05-t2"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("t2");
    assert_eq!(t1.fills.len(), 1);
    assert_eq!(t2.fills.len(), 1);
    let mut expected_seller_total: u128 = 0;
    let mut actual_seller_total: u128 = 0;
    for f in t1.fills.iter().chain(t2.fills.iter()) {
        let cid = f.canonical_execution_id.clone().unwrap();
        let pending = list_active_pending_for_execution(&pool, &cid)
            .await
            .unwrap();
        let seller_pending = pending
            .iter()
            .find(|r| r.owner == maker_addr.0)
            .expect("seller row");
        actual_seller_total += seller_pending.reserved_amount.parse::<u128>().unwrap();
        expected_seller_total += short_call_phys_ceil(f.size_1e8, CONTRACT_SIZE);
    }
    assert_eq!(
        actual_seller_total, expected_seller_total,
        "sum of per-fill ceil_div amounts must equal PENDING sum"
    );
    // Total for maker (3 contracts, physical) exactly 3e8.
    assert_eq!(
        actual_seller_total,
        3 * CONTRACT_SIZE,
        "no double-count: total maker PENDING = 3 * contract_size"
    );
}

// -------------------------------------------------------------------
// Group C — same-wallet cross-subaccount settlement isolation
// -------------------------------------------------------------------

#[tokio::test]
async fn truth06_same_wallet_different_subaccounts_settle_independently() {
    // Same owner submits both sides via different subaccounts. Each
    // subaccount's PENDING amount is independent (no netting).
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("t06")).await;
    let price = 1_000_000_000u128;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    let owner = addr(0x58, 1);
    // Seller on subaccount 1
    submit_option_order(
        &state,
        order_input(
            &series,
            owner.clone(),
            1,
            Side::Sell,
            price,
            CONTRACT_SIZE,
            150,
            &unique_client_id("t06-s"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("s");
    // Buyer on subaccount 2
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            owner.clone(),
            2,
            Side::Buy,
            price,
            CONTRACT_SIZE,
            151,
            &unique_client_id("t06-b"),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("b");
    let cid = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2, "two PENDING rows, no netting");
    let subs: std::collections::HashSet<i32> = pending.iter().map(|r| r.subaccount_id).collect();
    assert_eq!(subs, [1, 2].into_iter().collect(), "distinct subaccounts");
    // Amounts are per-side independent (each on its own collateral
    // token even for same owner — buyer side uses settlement asset,
    // seller side uses underlying).
    let sub1 = pending
        .iter()
        .find(|r| r.subaccount_id == 1)
        .expect("sub1 row");
    let sub2 = pending
        .iter()
        .find(|r| r.subaccount_id == 2)
        .expect("sub2 row");
    let sub1_expected = short_call_phys_ceil(CONTRACT_SIZE, CONTRACT_SIZE);
    let sub2_expected = buy_res_ceil(CONTRACT_SIZE, CONTRACT_SIZE, price);
    assert_eq!(
        sub1.reserved_amount.parse::<u128>().unwrap(),
        sub1_expected,
        "subaccount 1 (seller) reservation independent"
    );
    assert_eq!(
        sub2.reserved_amount.parse::<u128>().unwrap(),
        sub2_expected,
        "subaccount 2 (buyer) reservation independent"
    );
    // Now settle the pair; both PENDING rows on the two subaccounts
    // settle in one settle_pending call — proving lifecycle completes
    // independently for both subaccounts.
    let settled = deopt_v2_backend::options::reservation_repository::settle_pending(
        &pool,
        &cid,
        now_ms() as i64,
    )
    .await
    .expect("settle");
    assert_eq!(settled.len(), 2, "both subaccounts settled");
    let after = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    assert!(after.is_empty(), "no residual PENDING after settlement");
}

// -------------------------------------------------------------------
// Group D — rebuild economic state convergence
// -------------------------------------------------------------------

#[tokio::test]
async fn rebuild01_reservation_state_deterministic_from_inputs() {
    // Two independent runs of {submit maker, partial match via IOC,
    // cancel residual} using distinct salts must produce the same
    // shape of final reservation state (audit truth). Distinct salts
    // means the actual owner + hash differ but the CARDINALITY /
    // STATE SET must be identical.
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("rb01")).await;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    let price = 1_000_000_000u128;

    async fn run_once(
        state: &AppState,
        pool: &PgPool,
        series: &str,
        price: u128,
        salt: u32,
    ) -> (i64, i64, i64) {
        let maker = addr(0x59, salt);
        let taker = addr(0x59, salt + 100);
        let m = submit_option_order(
            state,
            order_input(
                series,
                maker.clone(),
                1,
                Side::Sell,
                price,
                2 * CONTRACT_SIZE,
                (salt as u64) * 10 + 1,
                &unique_client_id(&format!("rb01-m-{salt}")),
                TimeInForce::Gtc,
            ),
        )
        .await
        .expect("m");
        let _t = submit_option_order(
            state,
            order_input(
                series,
                taker.clone(),
                1,
                Side::Buy,
                price,
                CONTRACT_SIZE,
                (salt as u64) * 10 + 2,
                &unique_client_id(&format!("rb01-t-{salt}")),
                TimeInForce::Ioc,
            ),
        )
        .await
        .expect("t");
        // Cancel maker residual.
        let _ = cancel_option_order(state, m.order.order_id).await;
        // Now snapshot: (active count, converted count, pending count).
        let active_count: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner IN ($1, $2) AND status = 'ACTIVE'",
        )
        .bind(maker.0.as_str())
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .expect("active count")
        .try_get("c")
        .unwrap();
        let converted_count: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner IN ($1, $2) AND status = 'CONVERTED'",
        )
        .bind(maker.0.as_str())
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .expect("converted count")
        .try_get("c")
        .unwrap();
        let released_count: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner IN ($1, $2) AND status = 'RELEASED'",
        )
        .bind(maker.0.as_str())
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .expect("released count")
        .try_get("c")
        .unwrap();
        (active_count, converted_count, released_count)
    }
    let a = run_once(&state, &pool, &series, price, 1).await;
    let b = run_once(&state, &pool, &series, price, 2).await;
    assert_eq!(
        a, b,
        "identical operational shapes produce identical reservation cardinalities"
    );
    // Sanity: exactly 2 ACTIVE PENDING rows (buyer + seller), 1
    // CONVERTED (partial-fill resize), 1 RELEASED (cancel of residual).
    assert_eq!(a.0, 2, "2 ACTIVE PENDING after settlement pair");
    assert_eq!(a.1, 1, "1 CONVERTED (maker resize on partial fill)");
    assert_eq!(a.2, 1, "1 RELEASED (cancel of residual maker successor)");
}

#[tokio::test]
async fn rebuild02_post_reactivate_matches_expected_active_pending_set() {
    // Seed pending + correlation, settle (SETTLED), then reactivate
    // (reorg path). Verify the reactivated ACTIVE PENDING set is
    // exactly the pre-settle set (per (owner, subaccount, token)).
    let Some(pool) = require_pool().await else {
        return;
    };
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    let cid = unique_canonical_exec_id("rb02");
    let now = now_ms() as i64;
    let (buyer, seller) = seed_awaiting_and_pending(&pool, &cid, now, 900).await;
    // Snapshot expected PENDING set (owner, sub, token).
    let expected_pending = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    assert_eq!(expected_pending.len(), 2);
    let expected_set: std::collections::BTreeSet<(String, i32, String, String)> = expected_pending
        .iter()
        .map(|r| {
            (
                r.owner.clone(),
                r.subaccount_id,
                r.collateral_token.clone(),
                r.reserved_amount.clone(),
            )
        })
        .collect();
    // Settle via canonical event.
    let evt = CanonicalExecutionEventInput {
        canonical_execution_id: &cid,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &unique_tx(2001),
        log_index: 0,
        canonical_block_number: 12000,
        canonical_block_hash: &unique_tx(2002),
        onchain_execution_id: Some("oe-rb02"),
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: now,
    };
    let (_out, settled) = correlate_canonical_option_event_and_settle(&pool, &evt)
        .await
        .expect("settle");
    assert_eq!(settled.len(), 2);
    // Reactivate.
    let successors = reorg_reactivate_pending(&pool, &cid, now + 100)
        .await
        .expect("reactivate");
    assert_eq!(successors.len(), 2);
    // Read the current ACTIVE PENDING set.
    let after = list_active_pending_for_execution(&pool, &cid)
        .await
        .unwrap();
    let after_set: std::collections::BTreeSet<(String, i32, String, String)> = after
        .iter()
        .map(|r| {
            (
                r.owner.clone(),
                r.subaccount_id,
                r.collateral_token.clone(),
                r.reserved_amount.clone(),
            )
        })
        .collect();
    assert_eq!(
        after_set, expected_set,
        "reactivated ACTIVE PENDING set exactly matches the expected pre-settle set"
    );
    let _ = (buyer, seller);
}

#[tokio::test]
async fn rebuild03_crash_between_correlate_and_settle_converges_deterministically() {
    // Two branches:
    //   Branch H (happy path): one call to the wrapper settles.
    //   Branch C (crash path): direct reducer promotes, then a replay
    //   of the wrapper converges to the same final state.
    // Both branches must land at identical (SETTLED count = 2,
    // ACTIVE PENDING = 0, correlation = CORRELATED_CANONICAL).
    let Some(pool) = require_pool().await else {
        return;
    };
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");

    async fn happy(pool: &PgPool, seed: u32) -> (i64, i64, String) {
        let cid = unique_canonical_exec_id(&format!("rb03h-{seed}"));
        let now = now_ms() as i64;
        let _ = seed_awaiting_and_pending(pool, &cid, now, seed).await;
        let evt = CanonicalExecutionEventInput {
            canonical_execution_id: &cid,
            execution_kind: OptionExecutionKind::Trade,
            tx_hash: &unique_tx(3000 + seed),
            log_index: 0,
            canonical_block_number: 20000 + seed as i64,
            canonical_block_hash: &unique_tx(4000 + seed),
            onchain_execution_id: Some("oe-rb03h"),
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: "100000000",
            now_ms: now,
        };
        let _ = correlate_canonical_option_event_and_settle(pool, &evt)
            .await
            .expect("settle");
        collect_state(pool, &cid).await
    }

    async fn crash_then_replay(pool: &PgPool, seed: u32) -> (i64, i64, String) {
        let cid = unique_canonical_exec_id(&format!("rb03c-{seed}"));
        let now = now_ms() as i64;
        let _ = seed_awaiting_and_pending(pool, &cid, now, seed + 1000).await;
        let evt = CanonicalExecutionEventInput {
            canonical_execution_id: &cid,
            execution_kind: OptionExecutionKind::Trade,
            tx_hash: &unique_tx(5000 + seed),
            log_index: 0,
            canonical_block_number: 30000 + seed as i64,
            canonical_block_hash: &unique_tx(6000 + seed),
            onchain_execution_id: Some("oe-rb03c"),
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: "100000000",
            now_ms: now,
        };
        // Promote-only (simulated crash between correlate + settle).
        let _ = correlate_canonical_option_event(pool, &evt)
            .await
            .expect("promote");
        // Replay the wrapper — must converge.
        let _ = correlate_canonical_option_event_and_settle(pool, &evt)
            .await
            .expect("replay");
        collect_state(pool, &cid).await
    }

    let happy_state = happy(&pool, 7).await;
    let crash_state = crash_then_replay(&pool, 8).await;
    assert_eq!(
        happy_state, crash_state,
        "crash-then-replay converges to identical final state as happy path"
    );
    // Sanity: both branches finish with 0 ACTIVE PENDING, 2 SETTLED,
    // correlation CORRELATED_CANONICAL.
    assert_eq!(happy_state.0, 0, "0 ACTIVE PENDING after settlement");
    assert_eq!(happy_state.1, 2, "2 SETTLED PENDING");
    assert_eq!(
        happy_state.2,
        OptionCorrelationStatus::CorrelatedCanonical.as_str()
    );
}

async fn collect_state(pool: &PgPool, cid: &str) -> (i64, i64, String) {
    let active_pending: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
         WHERE canonical_execution_id = $1
           AND status = 'ACTIVE'
           AND purpose = 'PENDING_SETTLEMENT'",
    )
    .bind(cid)
    .fetch_one(pool)
    .await
    .expect("q")
    .try_get("c")
    .unwrap();
    let settled: i64 = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
         WHERE canonical_execution_id = $1
           AND status = 'SETTLED'
           AND purpose = 'PENDING_SETTLEMENT'",
    )
    .bind(cid)
    .fetch_one(pool)
    .await
    .expect("q")
    .try_get("c")
    .unwrap();
    let status: String = sqlx::query(
        "SELECT correlation_status FROM option_execution_correlations
         WHERE canonical_execution_id = $1",
    )
    .bind(cid)
    .fetch_one(pool)
    .await
    .expect("q")
    .try_get("correlation_status")
    .unwrap();
    (active_pending, settled, status)
}

async fn position_row_count(pool: &PgPool, subkey_or_owner: &str) -> i64 {
    // The canonical HV2 position projection is keyed by subkey. Any
    // row that references our owner address (as substring of subkey)
    // would be surprising; the count is expected to be 0 for a
    // freshly-created wallet in a matcher-only path.
    sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM hybrid_v2_positions
         WHERE subkey ILIKE '%' || $1 || '%'",
    )
    .bind(subkey_or_owner.trim_start_matches("0x"))
    .fetch_one(pool)
    .await
    .expect("hv2 pos count")
    .try_get("c")
    .unwrap()
}
