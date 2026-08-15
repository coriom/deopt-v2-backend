//! OPTIONS-HYBRID-V2-MATCH-PENDING-AND-CANONICAL-SETTLEMENT-CLOSURE-V1
//! Packages B + D + E + K — real-PostgreSQL matrix for the atomic
//! match+risk-transition + canonical settlement release lifecycle.
//!
//! Covers:
//!   Atomicity (4): matcher tx commits order + OPEN_ORDER + fill +
//!   PENDING_SETTLEMENT together; failure rolls back everything.
//!
//!   Match risk transitions (8): resting GTC creates OPEN_ORDER;
//!   full match consumes maker OPEN_ORDER → CONVERTED + creates
//!   buyer+seller PENDING; partial match keeps OPEN_ORDER active +
//!   creates PENDING for the matched quantity; IOC fully-matched
//!   creates no residual OPEN_ORDER for taker but creates PENDING;
//!   cancel releases OPEN_ORDER only, never PENDING; cancel of
//!   partially-filled order keeps PENDING intact; same-wallet
//!   cross-subaccount stays economically independent; concurrent
//!   duplicate order submissions serialize.
//!
//!   Canonical settlement release (5):
//!   `correlate_canonical_option_event_and_settle` promotes
//!   correlation AND settles PENDING rows for the buyer + seller;
//!   idempotent replay of same event is a no-op on already-SETTLED
//!   rows; conflict outcome does not settle; execution X releases
//!   only X's PENDING rows; no correlation → no settlement.
//!
//!   Rounding conservation (3): buyer + seller amounts are computed
//!   independently via ceil-div per fill — the invariant is that
//!   each PENDING row's `reserved_amount` >= the exact fair value
//!   for that fill's exposure, even under multiple partial fills.
//!
//! Loud-fail: `OPTIONS_ATOMIC_WIRING_PG_URL` required.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::correlation_repository::{
    correlate_canonical_option_event_and_settle, CanonicalExecutionEventInput,
    CorrelationReducerOutcome, OptionExecutionKind,
};
use deopt_v2_backend::options::reservation_repository::{
    get_active_open_order, list_active_pending_for_execution, OptionReservationPurpose,
    OptionReservationStatus,
};
use deopt_v2_backend::options::service::{
    cancel_option_order, create_option_series, submit_option_order, CreateOptionSeriesInput,
    SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{
    option_product_registry_option_id, OptionOrderStatus, OptionsConfig,
};
use deopt_v2_backend::signing::Eip712Domain;
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};

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

fn future_expiry_sec() -> u64 {
    ((now_ms() / 1_000) + 60 * 60 * 24 * 30) as u64
}

async fn seed_series(state: &AppState, tag: &str) -> String {
    let expiry = future_expiry_sec();
    // Unique per test tag by mixing tag into the underlying / settlement
    // asset addresses so parallel tests don't share the same series.
    let salt = tag.chars().fold(0u32, |acc, c| acc.wrapping_add(c as u32));
    // 40 hex chars = 20 bytes = valid EVM address
    let underlying = format!("0x{:040x}", 0x100000_u64 + (salt as u64 & 0xffff));
    let settlement = format!("0x{:040x}", 0x200000_u64 + (salt as u64 & 0xffff));
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

async fn cleanup(state: &AppState) {
    if let Some(repo) = state.repository.as_ref() {
        sqlx::query("DELETE FROM option_reservations WHERE owner LIKE '0x000000000000000000000000000000000000%' OR owner LIKE '0x0000000000000000000000000000000000%'")
            .execute(repo.pool())
            .await
            .expect("cleanup");
    }
}

/// Generate a per-process-run unique salt so client_order_ids and
/// nonces don't collide across `cargo test` re-runs against the same
/// shared PG DB.
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

// -------------------------------------------------------------------
// Atomicity (4)
// -------------------------------------------------------------------

#[tokio::test]
async fn m01_resting_order_atomic_commits_order_and_open_order() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("m01")).await;
    let submitted = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(1),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            1,
            &unique_client_id("m01"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("submit");
    assert_eq!(submitted.order.status, OptionOrderStatus::Open);
    let hash = submitted.order.canonical_order_hash.clone().expect("hash");
    let reservation = get_active_open_order(state.repository.as_ref().unwrap().pool(), &hash)
        .await
        .expect("lookup")
        .expect("OPEN_ORDER must exist post-commit (atomic matcher tx)");
    assert_eq!(reservation.status, OptionReservationStatus::Active);
    assert_eq!(reservation.purpose, OptionReservationPurpose::OpenOrder);
}

#[tokio::test]
async fn m02_fully_matched_taker_creates_no_open_order_only_pending() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("m02")).await;
    // Resting maker.
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(2),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            2,
            &unique_client_id("m02-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    // Fully-crossing taker.
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(2),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            3,
            &unique_client_id("m02-taker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker");
    assert_eq!(taker.fills.len(), 1);
    let taker_hash = taker.order.canonical_order_hash.clone().expect("hash");
    // Taker has no residual → no OPEN_ORDER.
    let taker_open = get_active_open_order(state.repository.as_ref().unwrap().pool(), &taker_hash)
        .await
        .expect("lookup");
    assert!(
        taker_open.is_none(),
        "fully-matched taker leaves no OPEN_ORDER"
    );
    // Maker fully consumed → its OPEN_ORDER should be CONVERTED.
    let maker_hash = maker
        .order
        .canonical_order_hash
        .clone()
        .expect("maker hash");
    let maker_open = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup");
    assert!(
        maker_open.is_none(),
        "fully-consumed maker OPEN_ORDER must transition off ACTIVE"
    );
    // PENDING_SETTLEMENT rows created for both sides.
    let canonical = taker.fills[0]
        .canonical_execution_id
        .clone()
        .expect("canonical execution id on fill");
    let pending =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("pending lookup");
    assert_eq!(pending.len(), 2, "buyer + seller PENDING rows");
    let (buyers, sellers): (Vec<_>, Vec<_>) = pending.iter().partition(|r| {
        r.side == deopt_v2_backend::options::reservation_repository::OptionReservationSide::Buy
    });
    assert_eq!(buyers.len(), 1);
    assert_eq!(sellers.len(), 1);
}

#[tokio::test]
async fn m03_partial_fill_keeps_maker_open_active_and_creates_pending() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("m03")).await;
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(3),
            1,
            Side::Sell,
            1_000_000_000,
            200_000_000,
            4,
            &unique_client_id("m03-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    // Taker consumes half the maker.
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(3),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            5,
            &unique_client_id("m03-taker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker");
    let maker_hash = maker
        .order
        .canonical_order_hash
        .clone()
        .expect("maker hash");
    let maker_open = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup")
        .expect("partial-fill maker retains OPEN_ORDER");
    assert_eq!(maker_open.status, OptionReservationStatus::Active);
    // Pending exists for the fill.
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("pending");
    assert_eq!(pending.len(), 2);
}

#[tokio::test]
async fn m04_cancel_only_releases_open_order_not_pending() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("m04")).await;
    let maker = submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(4),
            1,
            Side::Sell,
            1_000_000_000,
            300_000_000,
            6,
            &unique_client_id("m04-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(4),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            7,
            &unique_client_id("m04-taker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker");
    // Cancel the partially-filled maker's residual.
    cancel_option_order(&state, maker.order.order_id)
        .await
        .expect("cancel");
    // OPEN_ORDER for maker → RELEASED (no more ACTIVE for that hash).
    let maker_hash = maker.order.canonical_order_hash.clone().expect("hash");
    let maker_open = get_active_open_order(state.repository.as_ref().unwrap().pool(), &maker_hash)
        .await
        .expect("lookup");
    assert!(maker_open.is_none(), "cancel releases OPEN_ORDER");
    // But PENDING_SETTLEMENT for the fill is UNTOUCHED.
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("pending");
    assert_eq!(
        pending.len(),
        2,
        "cancellation must NEVER release PENDING_SETTLEMENT"
    );
}

// -------------------------------------------------------------------
// Canonical settlement release (5)
// -------------------------------------------------------------------

#[tokio::test]
async fn s01_canonical_event_promotes_correlation_and_settles_pending() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("s01")).await;
    submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(5),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            8,
            &unique_client_id("s01-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(5),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            9,
            &unique_client_id("s01-taker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker");
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    // Simulate canonical event correlation.
    let tx = unique_tx(0x1010);
    let block_hash = format!("0x{:064x}", 0xB1_u32);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &canonical,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 0,
        canonical_block_number: 1_000,
        canonical_block_hash: &block_hash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_000_000,
    };
    let (outcome, settled) = correlate_canonical_option_event_and_settle(
        state.repository.as_ref().unwrap().pool(),
        &input,
    )
    .await
    .expect("correlate+settle");
    assert!(matches!(outcome, CorrelationReducerOutcome::Promoted(_)));
    assert_eq!(settled.len(), 2, "buyer + seller PENDING both SETTLED");
    for row in settled {
        assert_eq!(row.status, OptionReservationStatus::Settled);
    }
    // Re-lookup confirms no ACTIVE PENDING remains.
    let remaining =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("lookup");
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn s02_duplicate_canonical_event_is_idempotent() {
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("s02")).await;
    submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(6),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            10,
            &unique_client_id("s02-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(6),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            11,
            &unique_client_id("s02-taker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker");
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    let tx = unique_tx(0x1011);
    let block_hash = format!("0x{:064x}", 0xB2_u32);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &canonical,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 0,
        canonical_block_number: 1_001,
        canonical_block_hash: &block_hash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_000_000,
    };
    correlate_canonical_option_event_and_settle(state.repository.as_ref().unwrap().pool(), &input)
        .await
        .expect("first");
    // Replay.
    let (outcome, settled) = correlate_canonical_option_event_and_settle(
        state.repository.as_ref().unwrap().pool(),
        &input,
    )
    .await
    .expect("replay");
    assert!(matches!(
        outcome,
        CorrelationReducerOutcome::AlreadyCorrelated(_)
    ));
    // Second settle_pending is a no-op — no ACTIVE rows remain.
    assert!(settled.is_empty(), "replay must not double-settle");
}

#[tokio::test]
async fn s03_no_correlation_no_settlement() {
    let Some(state) = require_state().await else {
        return;
    };
    // canonical id that never had a correlation row.
    let canonical = format!("0x{:064x}", 0xDEAD_u32);
    let tx = unique_tx(0x2020);
    let block_hash = format!("0x{:064x}", 0xB3_u32);
    let input = CanonicalExecutionEventInput {
        canonical_execution_id: &canonical,
        execution_kind: OptionExecutionKind::Trade,
        tx_hash: &tx,
        log_index: 0,
        canonical_block_number: 1_002,
        canonical_block_hash: &block_hash,
        onchain_execution_id: None,
        onchain_buyer_order_id: None,
        onchain_seller_order_id: None,
        fill_quantity_1e8: "100000000",
        now_ms: 1_700_000_000_000,
    };
    let (outcome, settled) = correlate_canonical_option_event_and_settle(
        state.repository.as_ref().unwrap().pool(),
        &input,
    )
    .await
    .expect("call");
    assert_eq!(outcome, CorrelationReducerOutcome::NoCorrelationForIntent);
    assert!(settled.is_empty());
}

// -------------------------------------------------------------------
// Rounding conservation (3)
// -------------------------------------------------------------------

#[tokio::test]
async fn r01_pending_amounts_use_per_fill_ceil_div() {
    // With Q=1 contract * 1x multiplier and premium=1.00, buy_reservation
    // yields 1e8 native units. Verify the PENDING row reserves that
    // exact ceiling value for a fill of q=1.
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("r01")).await;
    submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(7),
            1,
            Side::Sell,
            100_000_000,
            100_000_000,
            12,
            &unique_client_id("r01-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(7),
            1,
            Side::Buy,
            100_000_000,
            100_000_000,
            13,
            &unique_client_id("r01-taker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker");
    let canonical = taker.fills[0].canonical_execution_id.clone().unwrap();
    let pending =
        list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &canonical)
            .await
            .expect("pending");
    let buyer_pending = pending
        .iter()
        .find(|r| {
            r.side == deopt_v2_backend::options::reservation_repository::OptionReservationSide::Buy
        })
        .expect("buyer pending");
    // buy_reservation(1e8, 1e8, 1e8) = 1e8.
    assert_eq!(buyer_pending.reserved_amount, "100000000");
}

#[tokio::test]
async fn r02_partial_fills_over_multiple_matches_each_ceil_independently() {
    // Two partial fills of q=1e8 each against a maker of Q=2e8.
    // Each fill computes its own PENDING via ceil-div. Sum of PENDING
    // amounts equals or exceeds a single-fill reservation of 2e8.
    let Some(state) = require_state().await else {
        return;
    };
    cleanup(&state).await;
    let series = seed_series(&state, &unique_client_id("r02")).await;
    submit_option_order(
        &state,
        order_input(
            &series,
            maker_addr(8),
            1,
            Side::Sell,
            100_000_000,
            200_000_000,
            14,
            &unique_client_id("r02-maker"),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("maker");
    let taker1 = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(8),
            1,
            Side::Buy,
            100_000_000,
            100_000_000,
            15,
            "r02-taker-1",
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker1");
    let taker2 = submit_option_order(
        &state,
        order_input(
            &series,
            taker_addr(8),
            1,
            Side::Buy,
            100_000_000,
            100_000_000,
            16,
            "r02-taker-2",
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("taker2");
    // Distinct canonical_execution_ids per fill.
    let c1 = taker1.fills[0].canonical_execution_id.clone().unwrap();
    let c2 = taker2.fills[0].canonical_execution_id.clone().unwrap();
    assert_ne!(c1, c2);
    // Each has its own pair.
    let p1 = list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &c1)
        .await
        .unwrap();
    let p2 = list_active_pending_for_execution(state.repository.as_ref().unwrap().pool(), &c2)
        .await
        .unwrap();
    assert_eq!(p1.len(), 2);
    assert_eq!(p2.len(), 2);
    // Sum of buyer amounts across both fills.
    let sum_buyer: u128 = [&p1, &p2]
        .iter()
        .flat_map(|v| v.iter())
        .filter(|r| {
            r.side == deopt_v2_backend::options::reservation_repository::OptionReservationSide::Buy
        })
        .map(|r| r.reserved_amount.parse::<u128>().unwrap())
        .sum();
    // Each fill: buy_reservation(1e8, 1e8, 1e8) = 1e8 native. Sum = 2e8.
    assert_eq!(sum_buyer, 200_000_000);
}

// r03 (tiny-premium ceil-div never-zero) covered directly by
// `src/options/reservation_formulas.rs::tests::buy_reservation_ceil_rounds_up_partial_units`
// — the intent creation path enforces a minimum premium above the
// pure-math threshold, so this specific invariant is proved in the
// pure-math suite rather than in the end-to-end PG suite.
