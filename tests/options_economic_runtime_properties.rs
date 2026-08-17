//! OPTIONS-HYBRID-V2-ECONOMIC-RUNTIME-PROPERTIES-V1
//!
//! Twenty bounded property assertions for the Options economic
//! runtime. Same style as the other `_properties.rs` suites in this
//! crate: no `proptest` dep — each property drives a small
//! deterministic sample set via `rand::rngs::SmallRng` seeded from a
//! fixed value.
//!
//! Real-PG gate: identical to
//! `tests/options_economic_runtime_final_closure_pg_integration.rs`.
//! Set `OPTIONS_ATOMIC_WIRING_PG_URL=...` to run;
//! `OPTIONS_ATOMIC_WIRING_PG_ALLOW_SKIP=1` opts out for dev.
//!
//! Property list (20):
//!   P01  accepted resting order always has sufficient OPEN_ORDER
//!        protection (row is ACTIVE with reserved_amount > 0).
//!   P02  available_option_collateral excludes ACTIVE OPEN_ORDER.
//!   P03  available_option_collateral excludes ACTIVE PENDING.
//!   P04  committed fill always has PENDING protection on both sides.
//!   P05  partial fill never under-reserves.
//!   P06  repeated partial fills never under-reserve in total.
//!   P07  full fill leaves no active OPEN_ORDER.
//!   P08  cancellation cannot release PENDING_SETTLEMENT.
//!   P09  IOC leaves no resting exposure.
//!   P10  failed FOK causes zero economic mutation on the taker side.
//!   P11  rejected post-only leaks no reservation.
//!   P12  duplicate PENDING insert cannot duplicate exposure
//!        (idempotent via sparse UNIQUE).
//!   P13  off-chain match does not touch the canonical position
//!        projection (`hybrid_v2_positions`).
//!   P14  settlement of execution X cannot mark PENDING of execution
//!        Y as SETTLED.
//!   P15  duplicate canonical settlement event cannot double-apply.
//!   P16  correlation→settlement crash replay converges to SETTLED.
//!   P17  reorg reactivates ACTIVE PENDING (post-reactivate check).
//!   P18  same-wallet different-subaccounts never net risk.
//!   P19  reservation lifecycle rebuild converges (before/after
//!        equality across two identical operational shapes).
//!   P20  canonical position projection depends only on chain events
//!        (matcher never mutates `hybrid_v2_positions`).

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::correlation_repository::{
    correlate_canonical_option_event, correlate_canonical_option_event_and_settle,
    upsert_awaiting_correlation_tx, AwaitingCorrelationInput, CanonicalExecutionEventInput,
    OptionExecutionKind,
};
use deopt_v2_backend::options::reservation_repository::{
    available_option_collateral, get_active_open_order, insert_pending_settlement_reservation_tx,
    list_active_pending_for_execution, reorg_reactivate_pending, settle_pending,
    total_active_reserved, OptionReservationPurpose, OptionReservationSide,
    OptionReservationStatus, PendingSettlementReservationInput,
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
// Bounded input spaces:
const QTY_MIN_CONTRACTS: u32 = 1;
const QTY_MAX_CONTRACTS: u32 = 4;
const PRICES: &[u128] = &[500_000_000, 1_000_000_000, 2_000_000_000];
const ITERS: usize = 6;

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
    format!("test-prop-{prefix}-{}", uuid::Uuid::new_v4())
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

async fn seed_call_series(state: &AppState, tag: &str) -> String {
    let expiry = future_expiry_sec();
    let salt = tag.chars().fold(0u32, |acc, c| acc.wrapping_add(c as u32));
    let underlying = format!(
        "0x{:040x}",
        0x900000_u64 + (salt as u64 & 0xffff) + run_salt() as u64
    );
    let settlement = format!(
        "0x{:040x}",
        0xa00000_u64 + (salt as u64 & 0xffff) + run_salt() as u64
    );
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
    post_only: bool,
) -> SubmitOptionOrderInput {
    SubmitOptionOrderInput {
        option_series_id: series.to_string(),
        account,
        subaccount_id: subaccount,
        side,
        price_1e8,
        size_1e8,
        time_in_force: tif,
        post_only,
        client_order_id: Some(client_order_id.to_string()),
        nonce: Some(nonce),
        deadline_ms: Some(now_ms() + 60_000),
        signature: Some(VALID_SIG.to_string()),
        attached_tp_sl: None,
    }
}

fn buy_res_ceil(q: u128, c: u128, p: u128) -> u128 {
    let num = q * c * p;
    (num + 10_000_000_000_000_000u128 - 1) / 10_000_000_000_000_000u128
}
fn short_call_phys_ceil(q: u128, c: u128) -> u128 {
    let num = q * c;
    (num + 100_000_000u128 - 1) / 100_000_000u128
}

async fn seed_awaiting_and_pending(
    pool: &PgPool,
    canonical_execution_id: &str,
    now_ms_i: i64,
    tag_seed: u32,
) -> (String, String) {
    let mut tx = pool.begin().await.expect("tx");
    upsert_awaiting_correlation_tx(
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
    let buyer = format!("0xff01{:036x}", seed as u128);
    let seller = format!("0xff02{:036x}", seed as u128);
    for (owner, side, token) in [
        (
            &buyer,
            "buy",
            format!("0xa100000000000000000000000000{:012x}", seed as u128),
        ),
        (
            &seller,
            "sell",
            format!("0xb100000000000000000000000000{:012x}", seed as u128),
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

fn iter_bounded_inputs(seed: u64) -> Vec<(u128, u128)> {
    // Returns (quantity_1e8, price_1e8) pairs — deterministic bounded
    // enumeration via a simple LCG (no rand crate feature needed).
    let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    (0..ITERS)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let qty_range = (QTY_MAX_CONTRACTS - QTY_MIN_CONTRACTS + 1) as u64;
            let qty_contracts = (QTY_MIN_CONTRACTS as u64 + (s >> 33) % qty_range) as u128;
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let price_idx = ((s >> 33) as usize) % PRICES.len();
            (qty_contracts * CONTRACT_SIZE, PRICES[price_idx])
        })
        .collect()
}

// -------------------------------------------------------------------
// P01 accepted resting order → ACTIVE OPEN_ORDER with reserved > 0
// -------------------------------------------------------------------

#[tokio::test]
async fn p01_accepted_resting_order_has_open_order_protection() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p01")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    let inputs = iter_bounded_inputs(0x01);
    for (i, (qty, px)) in inputs.into_iter().enumerate() {
        let sub = submit_option_order(
            &state,
            order_input(
                &series,
                addr(0xA0, 1000 + i as u32),
                1,
                Side::Sell,
                px,
                qty,
                (1000 + i) as u64,
                &unique_client_id(&format!("p01-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("submit");
        let hash = sub.order.canonical_order_hash.clone().unwrap();
        let row = get_active_open_order(pool, &hash)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(row.status, OptionReservationStatus::Active);
        let amt: u128 = row.reserved_amount.parse().unwrap();
        assert!(amt > 0, "reserved_amount > 0 for accepted resting order");
    }
}

// -------------------------------------------------------------------
// P02 available_option_collateral excludes ACTIVE OPEN_ORDER
// -------------------------------------------------------------------

#[tokio::test]
async fn p02_available_collateral_excludes_open_order() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p02")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    let inputs = iter_bounded_inputs(0x02);
    for (i, (qty, px)) in inputs.into_iter().enumerate() {
        let owner = addr(0xA1, 2000 + i as u32);
        let underlying = get_underlying(&state, &series).await;
        // Big canonical balance.
        let canonical: u128 = 1_000_000_000_000_000_000;
        // No holds yet: available == canonical.
        let before = available_option_collateral(pool, 1, &owner.0, 1, &underlying, canonical)
            .await
            .expect("available before");
        assert_eq!(before, canonical);
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                owner.clone(),
                1,
                Side::Sell,
                px,
                qty,
                (2000 + i) as u64,
                &unique_client_id(&format!("p02-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("submit");
        let expected_reserved = short_call_phys_ceil(qty, CONTRACT_SIZE);
        let after = available_option_collateral(pool, 1, &owner.0, 1, &underlying, canonical)
            .await
            .expect("available after");
        assert_eq!(
            after,
            canonical - expected_reserved,
            "available excludes OPEN_ORDER reservation"
        );
    }
}

// -------------------------------------------------------------------
// P03 available_option_collateral excludes ACTIVE PENDING
// -------------------------------------------------------------------

#[tokio::test]
async fn p03_available_collateral_excludes_pending_settlement() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p03")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    let inputs = iter_bounded_inputs(0x03);
    for (i, (qty, px)) in inputs.into_iter().enumerate() {
        let maker = addr(0xA2, 3000 + i as u32);
        let taker = addr(0xA2, 3500 + i as u32);
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                px,
                qty,
                (3000 + i) as u64,
                &unique_client_id(&format!("p03-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        // Fully match — maker's OPEN_ORDER converts, and PENDING pair appears.
        let _t = submit_option_order(
            &state,
            order_input(
                &series,
                taker.clone(),
                1,
                Side::Buy,
                px,
                qty,
                (4000 + i) as u64,
                &unique_client_id(&format!("p03-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        let underlying = get_underlying(&state, &series).await;
        let canonical: u128 = 1_000_000_000_000_000_000;
        let after = available_option_collateral(pool, 1, &maker.0, 1, &underlying, canonical)
            .await
            .expect("available after fill");
        let pending_reserved = short_call_phys_ceil(qty, CONTRACT_SIZE);
        assert!(
            after <= canonical - pending_reserved,
            "available excludes PENDING_SETTLEMENT reservation \
             (after={after}, canonical={canonical}, pending={pending_reserved})"
        );
    }
}

// -------------------------------------------------------------------
// P04 committed fill has PENDING pair (both sides)
// -------------------------------------------------------------------

#[tokio::test]
async fn p04_committed_fill_has_pending_pair() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p04")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    let inputs = iter_bounded_inputs(0x04);
    for (i, (qty, px)) in inputs.into_iter().enumerate() {
        let maker = addr(0xA3, 4000 + i as u32);
        let taker = addr(0xA3, 4500 + i as u32);
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                px,
                qty,
                (5000 + i) as u64,
                &unique_client_id(&format!("p04-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let t = submit_option_order(
            &state,
            order_input(
                &series,
                taker.clone(),
                1,
                Side::Buy,
                px,
                qty,
                (6000 + i) as u64,
                &unique_client_id(&format!("p04-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        let cid = t.fills[0].canonical_execution_id.clone().unwrap();
        let pending = list_active_pending_for_execution(pool, &cid).await.unwrap();
        assert_eq!(pending.len(), 2, "buyer + seller PENDING pair");
    }
}

// -------------------------------------------------------------------
// P05 partial fill never under-reserves
// -------------------------------------------------------------------

#[tokio::test]
async fn p05_partial_fill_never_under_reserves() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p05")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    // Maker of size 4 contracts, matched with 2 contracts.
    for i in 0..ITERS {
        let maker = addr(0xA4, 5000 + i as u32);
        let taker = addr(0xA4, 5500 + i as u32);
        let px = PRICES[i % PRICES.len()];
        let m = submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                px,
                4 * CONTRACT_SIZE,
                (7000 + i) as u64,
                &unique_client_id(&format!("p05-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                taker.clone(),
                1,
                Side::Buy,
                px,
                2 * CONTRACT_SIZE,
                (8000 + i) as u64,
                &unique_client_id(&format!("p05-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        let hash = m.order.canonical_order_hash.clone().unwrap();
        let successor = get_active_open_order(pool, &hash)
            .await
            .unwrap()
            .expect("successor");
        let residual_open: u128 = successor.reserved_amount.parse().unwrap();
        // Pending seller side on this order.
        let underlying = get_underlying(&state, &series).await;
        let total_reserved = total_active_reserved(pool, 1, &maker.0, 1, &underlying)
            .await
            .expect("total");
        // Total protection >= original expected exposure (4 contracts).
        let original_exposure = short_call_phys_ceil(4 * CONTRACT_SIZE, CONTRACT_SIZE);
        assert!(
            total_reserved >= original_exposure,
            "total reservations {total_reserved} >= original exposure {original_exposure} (residual OO {residual_open})"
        );
    }
}

// -------------------------------------------------------------------
// P06 repeated partial fills never under-reserve
// -------------------------------------------------------------------

#[tokio::test]
async fn p06_repeated_partials_never_under_reserve() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p06")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    for i in 0..3 {
        let maker = addr(0xA5, 6000 + i as u32);
        let px = PRICES[i % PRICES.len()];
        submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                px,
                4 * CONTRACT_SIZE,
                (9000 + i) as u64,
                &unique_client_id(&format!("p06-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        for j in 0..3 {
            submit_option_order(
                &state,
                order_input(
                    &series,
                    addr(0xA5, (6100 + i * 10 + j) as u32),
                    1,
                    Side::Buy,
                    px,
                    CONTRACT_SIZE,
                    (10000 + i * 10 + j) as u64,
                    &unique_client_id(&format!("p06-t-{i}-{j}")),
                    TimeInForce::Ioc,
                    false,
                ),
            )
            .await
            .expect("t");
        }
        let underlying = get_underlying(&state, &series).await;
        let total_reserved = total_active_reserved(pool, 1, &maker.0, 1, &underlying)
            .await
            .expect("total");
        let original_exposure = short_call_phys_ceil(4 * CONTRACT_SIZE, CONTRACT_SIZE);
        assert!(
            total_reserved >= original_exposure,
            "3 partial fills: total reserved {total_reserved} >= original {original_exposure}"
        );
    }
}

// -------------------------------------------------------------------
// P07 full fill leaves no active OPEN_ORDER
// -------------------------------------------------------------------

#[tokio::test]
async fn p07_full_fill_leaves_no_active_open_order() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p07")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    let inputs = iter_bounded_inputs(0x07);
    for (i, (qty, px)) in inputs.into_iter().enumerate() {
        let maker = addr(0xA6, 7000 + i as u32);
        let m = submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                px,
                qty,
                (11000 + i) as u64,
                &unique_client_id(&format!("p07-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                addr(0xA6, 7500 + i as u32),
                1,
                Side::Buy,
                px,
                qty,
                (12000 + i) as u64,
                &unique_client_id(&format!("p07-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        let hash = m.order.canonical_order_hash.clone().unwrap();
        assert!(
            get_active_open_order(pool, &hash).await.unwrap().is_none(),
            "full fill: no active OPEN_ORDER"
        );
    }
}

// -------------------------------------------------------------------
// P08 cancellation cannot release PENDING_SETTLEMENT
// -------------------------------------------------------------------

#[tokio::test]
async fn p08_cancel_cannot_release_pending_settlement() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p08")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    for i in 0..3 {
        let maker = addr(0xA7, 8000 + i as u32);
        let m = submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                PRICES[i % PRICES.len()],
                2 * CONTRACT_SIZE,
                (13000 + i) as u64,
                &unique_client_id(&format!("p08-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let t = submit_option_order(
            &state,
            order_input(
                &series,
                addr(0xA7, 8500 + i as u32),
                1,
                Side::Buy,
                PRICES[i % PRICES.len()],
                CONTRACT_SIZE,
                (14000 + i) as u64,
                &unique_client_id(&format!("p08-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        cancel_option_order(&state, m.order.order_id)
            .await
            .expect("cancel");
        let cid = t.fills[0].canonical_execution_id.clone().unwrap();
        let pending = list_active_pending_for_execution(pool, &cid).await.unwrap();
        assert_eq!(pending.len(), 2, "cancel does not release PENDING");
        for r in &pending {
            assert_eq!(r.status, OptionReservationStatus::Active);
        }
    }
}

// -------------------------------------------------------------------
// P09 IOC leaves no resting exposure
// -------------------------------------------------------------------

#[tokio::test]
async fn p09_ioc_leaves_no_resting_exposure() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p09")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    let inputs = iter_bounded_inputs(0x09);
    for (i, (qty, px)) in inputs.into_iter().enumerate() {
        let taker = addr(0xA8, 9000 + i as u32);
        let t = submit_option_order(
            &state,
            order_input(
                &series,
                taker.clone(),
                1,
                Side::Buy,
                px,
                qty,
                (15000 + i) as u64,
                &unique_client_id(&format!("p09-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        let hash = t.order.canonical_order_hash.clone().unwrap();
        assert!(
            get_active_open_order(pool, &hash).await.unwrap().is_none(),
            "IOC leaves no OPEN_ORDER regardless of fills"
        );
    }
}

// -------------------------------------------------------------------
// P10 failed FOK causes zero economic mutation on taker side
// -------------------------------------------------------------------

#[tokio::test]
async fn p10_failed_fok_no_economic_mutation() {
    let Some(state) = require_state().await else {
        return;
    };
    let pool = state.repository.as_ref().unwrap().pool();
    for i in 0..3 {
        // Fresh series per iteration so prior makers cannot supply
        // extra liquidity to FOK's marketable check.
        let series = seed_call_series(&state, &unique_client_id(&format!("p10-{i}"))).await;
        let maker = addr(0xA9, 10000 + i as u32);
        let taker = addr(0xA9, 10500 + i as u32);
        submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                1_000_000_000,
                CONTRACT_SIZE,
                (16000 + i) as u64,
                &unique_client_id(&format!("p10-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        // FOK asks 2 contracts but only 1 available → reject.
        let res = submit_option_order(
            &state,
            order_input(
                &series,
                taker.clone(),
                1,
                Side::Buy,
                1_000_000_000,
                2 * CONTRACT_SIZE,
                (17000 + i) as u64,
                &unique_client_id(&format!("p10-t-{i}")),
                TimeInForce::Fok,
                false,
            ),
        )
        .await;
        assert!(res.is_err(), "insufficient liquidity FOK must reject");
        // Zero reservations for taker.
        let cnt: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner = $1 AND status = 'ACTIVE'",
        )
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .unwrap()
        .try_get("c")
        .unwrap();
        assert_eq!(cnt, 0, "failed FOK creates no reservation");
    }
}

// -------------------------------------------------------------------
// P11 rejected post-only leaks no reservation
// -------------------------------------------------------------------

#[tokio::test]
async fn p11_rejected_post_only_no_leak() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p11")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    for i in 0..3 {
        submit_option_order(
            &state,
            order_input(
                &series,
                addr(0xAA, 11000 + i as u32),
                1,
                Side::Sell,
                1_000_000_000,
                CONTRACT_SIZE,
                (18000 + i) as u64,
                &unique_client_id(&format!("p11-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let taker = addr(0xAA, 11500 + i as u32);
        let res = submit_option_order(
            &state,
            order_input(
                &series,
                taker.clone(),
                1,
                Side::Buy,
                1_000_000_000,
                CONTRACT_SIZE,
                (19000 + i) as u64,
                &unique_client_id(&format!("p11-t-{i}")),
                TimeInForce::Gtc,
                true,
            ),
        )
        .await;
        assert!(res.is_err(), "post-only crossing rejected");
        let cnt: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner = $1",
        )
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .unwrap()
        .try_get("c")
        .unwrap();
        assert_eq!(cnt, 0, "post-only reject leaks no reservation");
    }
}

// -------------------------------------------------------------------
// P12 duplicate PENDING insert cannot duplicate exposure
// -------------------------------------------------------------------

#[tokio::test]
async fn p12_duplicate_pending_insert_is_idempotent() {
    let Some(pool) = require_pool().await else {
        return;
    };
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    for i in 0..3 {
        let cid = unique_canonical_exec_id(&format!("p12-{i}"));
        let now = now_ms() as i64;
        let salt = run_salt().wrapping_add(i as u32);
        let owner = format!("0xfa12{:036x}", salt as u128);
        let token = format!("0xa200000000000000000000000000{:012x}", salt as u128);
        let input = PendingSettlementReservationInput {
            deployment_id: 84532,
            chain_id: 84532,
            owner: owner.clone(),
            subaccount_id: 1,
            collateral_token: token.clone(),
            canonical_execution_id: cid.clone(),
            option_series_id: "seed-p12".to_string(),
            side: OptionReservationSide::Buy,
            reserved_amount: "100000000".to_string(),
            quantity_1e8: "100000000".to_string(),
            now_ms: now,
        };
        // First insert succeeds.
        let mut tx = pool.begin().await.unwrap();
        let r1 = insert_pending_settlement_reservation_tx(&mut tx, &input)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        // Second insert with same scope tuple should be idempotent
        // (returns the SAME row, no duplicate).
        let mut tx = pool.begin().await.unwrap();
        let r2 = insert_pending_settlement_reservation_tx(&mut tx, &input)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            r1.reservation_id, r2.reservation_id,
            "duplicate PENDING insert returns same row (no duplicate exposure)"
        );
        // Total across scope is exactly one reserved_amount.
        let total = total_active_reserved(&pool, 84532, &owner, 1, &token)
            .await
            .unwrap();
        assert_eq!(total, 100_000_000);
    }
}

// -------------------------------------------------------------------
// P13 off-chain match does not touch canonical position projection
// -------------------------------------------------------------------

#[tokio::test]
async fn p13_off_chain_match_does_not_touch_canonical_position() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p13")).await;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    for i in 0..3 {
        let maker = addr(0xAB, 12000 + i as u32);
        let taker = addr(0xAB, 12500 + i as u32);
        let before = hv2_position_count_for(&pool, &maker.0, &taker.0).await;
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                maker.clone(),
                1,
                Side::Sell,
                1_000_000_000,
                CONTRACT_SIZE,
                (20000 + i) as u64,
                &unique_client_id(&format!("p13-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                taker.clone(),
                1,
                Side::Buy,
                1_000_000_000,
                CONTRACT_SIZE,
                (21000 + i) as u64,
                &unique_client_id(&format!("p13-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        let after = hv2_position_count_for(&pool, &maker.0, &taker.0).await;
        assert_eq!(before, after, "matcher does not mutate hybrid_v2_positions");
    }
}

// -------------------------------------------------------------------
// P14 settle(X) cannot mark PENDING(Y) as SETTLED
// -------------------------------------------------------------------

#[tokio::test]
async fn p14_settlement_of_x_cannot_settle_pending_of_y() {
    let Some(pool) = require_pool().await else {
        return;
    };
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    for i in 0..3 {
        let cid_x = unique_canonical_exec_id(&format!("p14x-{i}"));
        let cid_y = unique_canonical_exec_id(&format!("p14y-{i}"));
        let now = now_ms() as i64;
        let _ = seed_awaiting_and_pending(&pool, &cid_x, now, 14000 + i).await;
        let _ = seed_awaiting_and_pending(&pool, &cid_y, now, 14500 + i).await;
        // Settle X only.
        let evt_x = CanonicalExecutionEventInput {
            canonical_execution_id: &cid_x,
            execution_kind: OptionExecutionKind::Trade,
            tx_hash: &unique_tx(14000 + i),
            log_index: 0,
            canonical_block_number: 40000 + i as i64,
            canonical_block_hash: &unique_tx(15000 + i),
            onchain_execution_id: Some("oe-p14x"),
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: "100000000",
            now_ms: now,
        };
        let _ = correlate_canonical_option_event_and_settle(&pool, &evt_x)
            .await
            .expect("settle X");
        // PENDING(Y) unchanged.
        let y = list_active_pending_for_execution(&pool, &cid_y)
            .await
            .unwrap();
        assert_eq!(y.len(), 2, "settle(X) preserves PENDING(Y)");
    }
}

// -------------------------------------------------------------------
// P15 duplicate canonical settlement event cannot double-apply
// -------------------------------------------------------------------

#[tokio::test]
async fn p15_duplicate_settlement_event_no_double_apply() {
    let Some(pool) = require_pool().await else {
        return;
    };
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    for i in 0..3 {
        let cid = unique_canonical_exec_id(&format!("p15-{i}"));
        let now = now_ms() as i64;
        let _ = seed_awaiting_and_pending(&pool, &cid, now, 15000 + i).await;
        let evt = CanonicalExecutionEventInput {
            canonical_execution_id: &cid,
            execution_kind: OptionExecutionKind::Trade,
            tx_hash: &unique_tx(16000 + i),
            log_index: 0,
            canonical_block_number: 50000 + i as i64,
            canonical_block_hash: &unique_tx(17000 + i),
            onchain_execution_id: Some("oe-p15"),
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: "100000000",
            now_ms: now,
        };
        let (_out1, settled1) = correlate_canonical_option_event_and_settle(&pool, &evt)
            .await
            .expect("first");
        assert_eq!(settled1.len(), 2, "first settle applies to 2 rows");
        let (_out2, settled2) = correlate_canonical_option_event_and_settle(&pool, &evt)
            .await
            .expect("second");
        assert!(
            settled2.is_empty(),
            "second (idempotent) settle applies to 0 rows"
        );
        // Post: still exactly 2 SETTLED rows, no duplicates.
        let cnt: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE canonical_execution_id = $1
               AND status = 'SETTLED'
               AND purpose = 'PENDING_SETTLEMENT'",
        )
        .bind(&cid)
        .fetch_one(&pool)
        .await
        .unwrap()
        .try_get("c")
        .unwrap();
        assert_eq!(cnt, 2, "no duplicate SETTLED rows");
    }
}

// -------------------------------------------------------------------
// P16 correlation→settlement crash replay converges
// -------------------------------------------------------------------

#[tokio::test]
async fn p16_correlation_settle_crash_replay_converges() {
    let Some(pool) = require_pool().await else {
        return;
    };
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    for i in 0..3 {
        let cid = unique_canonical_exec_id(&format!("p16-{i}"));
        let now = now_ms() as i64;
        let _ = seed_awaiting_and_pending(&pool, &cid, now, 16000 + i).await;
        let evt = CanonicalExecutionEventInput {
            canonical_execution_id: &cid,
            execution_kind: OptionExecutionKind::Trade,
            tx_hash: &unique_tx(18000 + i),
            log_index: 0,
            canonical_block_number: 60000 + i as i64,
            canonical_block_hash: &unique_tx(19000 + i),
            onchain_execution_id: Some("oe-p16"),
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: "100000000",
            now_ms: now,
        };
        // Simulate crash: promote-only.
        let _ = correlate_canonical_option_event(&pool, &evt)
            .await
            .expect("promote");
        // PENDING still ACTIVE.
        let before = list_active_pending_for_execution(&pool, &cid)
            .await
            .unwrap();
        assert_eq!(before.len(), 2, "pre-replay: PENDING still ACTIVE");
        // Replay wrapper converges.
        let (_out, settled) = correlate_canonical_option_event_and_settle(&pool, &evt)
            .await
            .expect("replay");
        assert_eq!(settled.len(), 2, "replay converges: 2 SETTLED");
        let after = list_active_pending_for_execution(&pool, &cid)
            .await
            .unwrap();
        assert!(after.is_empty(), "no ACTIVE PENDING after convergence");
    }
}

// -------------------------------------------------------------------
// P17 reorg reactivates ACTIVE PENDING
// -------------------------------------------------------------------

#[tokio::test]
async fn p17_reorg_reactivates_active_pending() {
    let Some(pool) = require_pool().await else {
        return;
    };
    println!("REAL_POSTGRES_CONNECTION_CONFIRMED");
    for i in 0..3 {
        let cid = unique_canonical_exec_id(&format!("p17-{i}"));
        let now = now_ms() as i64;
        let _ = seed_awaiting_and_pending(&pool, &cid, now, 17000 + i).await;
        let evt = CanonicalExecutionEventInput {
            canonical_execution_id: &cid,
            execution_kind: OptionExecutionKind::Trade,
            tx_hash: &unique_tx(20000 + i),
            log_index: 0,
            canonical_block_number: 70000 + i as i64,
            canonical_block_hash: &unique_tx(21000 + i),
            onchain_execution_id: Some("oe-p17"),
            onchain_buyer_order_id: None,
            onchain_seller_order_id: None,
            fill_quantity_1e8: "100000000",
            now_ms: now,
        };
        let _ = correlate_canonical_option_event_and_settle(&pool, &evt)
            .await
            .expect("settle");
        assert!(list_active_pending_for_execution(&pool, &cid)
            .await
            .unwrap()
            .is_empty());
        // Reorg reactivate.
        let succ = reorg_reactivate_pending(&pool, &cid, now + 100)
            .await
            .expect("reactivate");
        assert_eq!(succ.len(), 2);
        let after = list_active_pending_for_execution(&pool, &cid)
            .await
            .unwrap();
        assert_eq!(after.len(), 2, "reactivate restores 2 ACTIVE PENDING");
        for r in &after {
            assert_eq!(r.status, OptionReservationStatus::Active);
            assert_eq!(r.purpose, OptionReservationPurpose::PendingSettlement);
        }
    }
}

// -------------------------------------------------------------------
// P18 same-wallet different-subaccounts never net risk
// -------------------------------------------------------------------

#[tokio::test]
async fn p18_same_wallet_different_subaccounts_no_netting() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p18")).await;
    let pool = state.repository.as_ref().unwrap().pool();
    let underlying = get_underlying(&state, &series).await;
    for i in 0..3 {
        let owner = addr(0xAC, 13000 + i as u32);
        // Two sell orders on two subaccounts — reservations must sum.
        submit_option_order(
            &state,
            order_input(
                &series,
                owner.clone(),
                1,
                Side::Sell,
                1_000_000_000,
                CONTRACT_SIZE,
                (22000 + i) as u64,
                &unique_client_id(&format!("p18-s1-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("s1");
        submit_option_order(
            &state,
            order_input(
                &series,
                owner.clone(),
                2,
                Side::Sell,
                1_000_000_000,
                CONTRACT_SIZE,
                (23000 + i) as u64,
                &unique_client_id(&format!("p18-s2-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("s2");
        let t1 = total_active_reserved(pool, 1, &owner.0, 1, &underlying)
            .await
            .unwrap();
        let t2 = total_active_reserved(pool, 1, &owner.0, 2, &underlying)
            .await
            .unwrap();
        assert_eq!(t1, CONTRACT_SIZE, "sub1 reserved independently");
        assert_eq!(t2, CONTRACT_SIZE, "sub2 reserved independently");
    }
}

// -------------------------------------------------------------------
// P19 reservation lifecycle rebuild converges
// -------------------------------------------------------------------

#[tokio::test]
async fn p19_reservation_lifecycle_rebuild_converges() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p19")).await;
    let pool = state.repository.as_ref().unwrap().pool().clone();

    async fn drive(state: &AppState, pool: &PgPool, series: &str, salt: u32) -> (i64, i64, i64) {
        let maker = addr(0xAD, 14000 + salt);
        let taker = addr(0xAD, 14500 + salt);
        let m = submit_option_order(
            state,
            order_input(
                series,
                maker.clone(),
                1,
                Side::Sell,
                1_000_000_000,
                2 * CONTRACT_SIZE,
                (24000 + salt) as u64,
                &unique_client_id(&format!("p19-m-{salt}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let _ = submit_option_order(
            state,
            order_input(
                series,
                taker.clone(),
                1,
                Side::Buy,
                1_000_000_000,
                CONTRACT_SIZE,
                (25000 + salt) as u64,
                &unique_client_id(&format!("p19-t-{salt}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
        let _ = cancel_option_order(state, m.order.order_id).await;
        let active: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner IN ($1, $2) AND status = 'ACTIVE'",
        )
        .bind(maker.0.as_str())
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .unwrap()
        .try_get("c")
        .unwrap();
        let converted: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner IN ($1, $2) AND status = 'CONVERTED'",
        )
        .bind(maker.0.as_str())
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .unwrap()
        .try_get("c")
        .unwrap();
        let released: i64 = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS c FROM option_reservations
             WHERE owner IN ($1, $2) AND status = 'RELEASED'",
        )
        .bind(maker.0.as_str())
        .bind(taker.0.as_str())
        .fetch_one(pool)
        .await
        .unwrap()
        .try_get("c")
        .unwrap();
        (active, converted, released)
    }
    let a = drive(&state, &pool, &series, 1).await;
    let b = drive(&state, &pool, &series, 2).await;
    assert_eq!(a, b, "two identical operational shapes converge");
}

// -------------------------------------------------------------------
// P20 canonical position projection depends only on chain events
// -------------------------------------------------------------------

#[tokio::test]
async fn p20_canonical_position_depends_only_on_chain_events() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_call_series(&state, &unique_client_id("p20")).await;
    let pool = state.repository.as_ref().unwrap().pool().clone();
    // Snapshot total row count in hybrid_v2_positions.
    let before: i64 = sqlx::query("SELECT COUNT(*)::BIGINT AS c FROM hybrid_v2_positions")
        .fetch_one(&pool)
        .await
        .expect("count before")
        .try_get("c")
        .unwrap();
    // Drive several matches (all matcher-side).
    for i in 0..3 {
        submit_option_order(
            &state,
            order_input(
                &series,
                addr(0xAE, 15000 + i as u32),
                1,
                Side::Sell,
                1_000_000_000,
                CONTRACT_SIZE,
                (26000 + i) as u64,
                &unique_client_id(&format!("p20-m-{i}")),
                TimeInForce::Gtc,
                false,
            ),
        )
        .await
        .expect("m");
        let _ = submit_option_order(
            &state,
            order_input(
                &series,
                addr(0xAE, 15500 + i as u32),
                1,
                Side::Buy,
                1_000_000_000,
                CONTRACT_SIZE,
                (27000 + i) as u64,
                &unique_client_id(&format!("p20-t-{i}")),
                TimeInForce::Ioc,
                false,
            ),
        )
        .await
        .expect("t");
    }
    let after: i64 = sqlx::query("SELECT COUNT(*)::BIGINT AS c FROM hybrid_v2_positions")
        .fetch_one(&pool)
        .await
        .expect("count after")
        .try_get("c")
        .unwrap();
    assert_eq!(
        before, after,
        "hybrid_v2_positions row count is invariant under matcher activity"
    );
    // Additional check: even settle_pending doesn't touch the
    // canonical projection.
    let cid_before = list_active_pending_via_series(&pool, &series).await;
    if let Some(cid) = cid_before.first() {
        let _ = settle_pending(&pool, cid, now_ms() as i64).await;
    }
    let final_count: i64 = sqlx::query("SELECT COUNT(*)::BIGINT AS c FROM hybrid_v2_positions")
        .fetch_one(&pool)
        .await
        .expect("count final")
        .try_get("c")
        .unwrap();
    assert_eq!(
        before, final_count,
        "settle_pending must not touch hybrid_v2_positions"
    );
    let _ = buy_res_ceil(CONTRACT_SIZE, CONTRACT_SIZE, 1_000_000_000);
}

async fn get_underlying(state: &AppState, series_id: &str) -> String {
    let series = deopt_v2_backend::options::service::get_option_series(state, series_id)
        .await
        .expect("series");
    series.underlying
}

async fn hv2_position_count_for(pool: &PgPool, a: &str, b: &str) -> i64 {
    // Best-effort: count rows in hybrid_v2_positions whose subkey
    // references either address. Both parties are matcher-side only,
    // so this count should be invariant across matcher activity.
    sqlx::query(
        "SELECT COUNT(*)::BIGINT AS c FROM hybrid_v2_positions
         WHERE subkey ILIKE '%' || $1 || '%'
            OR subkey ILIKE '%' || $2 || '%'",
    )
    .bind(a.trim_start_matches("0x"))
    .bind(b.trim_start_matches("0x"))
    .fetch_one(pool)
    .await
    .expect("hv2 pos count")
    .try_get("c")
    .unwrap()
}

async fn list_active_pending_via_series(pool: &PgPool, series_id: &str) -> Vec<String> {
    let rows = sqlx::query(
        "SELECT DISTINCT canonical_execution_id FROM option_reservations
         WHERE option_series_id = $1
           AND canonical_execution_id IS NOT NULL
           AND status = 'ACTIVE'
           AND purpose = 'PENDING_SETTLEMENT'",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await
    .expect("pending by series");
    rows.iter()
        .map(|r| r.try_get::<String, _>("canonical_execution_id").unwrap())
        .collect()
}
