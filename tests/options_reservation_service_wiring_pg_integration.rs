//! OPTIONS-HYBRID-V2-RESERVATIONS-PENDING-SETTLEMENT-AND-CANONICAL-RELEASE-V1
//! Package E — service-layer wiring integration tests.
//!
//! Verifies that:
//!   1. Submitting a resting option order creates an ACTIVE OPEN_ORDER
//!      reservation for the residual quantity.
//!   2. Cancelling that order transitions the reservation to RELEASED
//!      (no reservation leak).
//!   3. A fully-crossed IOC that leaves no residual creates no
//!      OPEN_ORDER reservation.
//!
//! The Part E wire is best-effort post-match (documented atomicity
//! trade-off): reservation insert happens after the matcher tx
//! commits. These tests exercise the happy path where the insert
//! succeeds.
//!
//! Loud-fail gate identical to sibling PG suites.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::reservation_repository::{
    get_active_open_order, OptionReservationStatus,
};
use deopt_v2_backend::options::service::{
    cancel_option_order, create_option_series, submit_option_order, CreateOptionSeriesInput,
    SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{
    option_product_registry_option_id, OptionOrder, OptionOrderStatus, OptionsConfig,
};
use deopt_v2_backend::signing::Eip712Domain;
use uuid::Uuid;
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
    let underlying = format!("0x{:039}1", 0);
    let settlement = format!("0x{:039}2", 0);
    let strike_u64: u64 = 300_000_000_000;
    let strike: u128 = strike_u64 as u128;
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
            strike_1e8: strike,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: Some(onchain_option_id),
        },
    )
    .await
    .expect(&format!("series {tag}"))
    .option_series_id
}

fn order_input(
    series: &str,
    account: AccountId,
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
        subaccount_id: 1,
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

async fn cleanup(state: &AppState, canonical_order_hash: &str) {
    if let Some(repo) = state.repository.as_ref() {
        sqlx::query("DELETE FROM option_reservations WHERE canonical_order_hash = $1")
            .bind(canonical_order_hash)
            .execute(repo.pool())
            .await
            .expect("cleanup");
    }
}

// -------------------------------------------------------------------

#[tokio::test]
async fn w01_resting_order_creates_active_open_order_reservation() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_series(&state, &format!("{}-{}", "w01", Uuid::new_v4())).await;
    let account = AccountId::new("0x0000000000000000000000000000000000000001");
    // GTC sell that will REST (no counterparty on the book).
    let submitted = submit_option_order(
        &state,
        order_input(
            &series,
            account.clone(),
            Side::Sell,
            1_000_000_000,
            100_000_000,
            1,
            &format!("{}-{}", "w01-order", Uuid::new_v4()),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("submit");
    assert_eq!(submitted.order.status, OptionOrderStatus::Open);
    let hash = submitted
        .order
        .canonical_order_hash
        .clone()
        .expect("canonical hash");
    let reservation = get_active_open_order(state.repository.as_ref().unwrap().pool(), &hash)
        .await
        .expect("lookup")
        .expect("reservation must exist for resting order");
    assert_eq!(reservation.status, OptionReservationStatus::Active);
    assert_eq!(
        reservation.canonical_order_hash.as_deref(),
        Some(hash.as_str())
    );
    cleanup(&state, &hash).await;
}

#[tokio::test]
async fn w02_cancel_releases_open_order_reservation() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_series(&state, &format!("{}-{}", "w02", Uuid::new_v4())).await;
    let account = AccountId::new("0x0000000000000000000000000000000000000002");
    let submitted = submit_option_order(
        &state,
        order_input(
            &series,
            account.clone(),
            Side::Sell,
            1_000_000_000,
            100_000_000,
            2,
            &format!("{}-{}", "w02-order", Uuid::new_v4()),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("submit");
    let hash = submitted.order.canonical_order_hash.clone().expect("hash");
    // Cancel.
    cancel_option_order(&state, submitted.order.order_id)
        .await
        .expect("cancel");
    // Active row must be gone; look up the row directly (any status).
    let active = get_active_open_order(state.repository.as_ref().unwrap().pool(), &hash)
        .await
        .expect("lookup");
    assert!(active.is_none(), "ACTIVE row released on cancel");
    // Fetch by primary key using the raw SQL — verify RELEASED status.
    let released_row = sqlx::query(
        "SELECT reservation_id FROM option_reservations WHERE canonical_order_hash = $1
         AND status = 'RELEASED' LIMIT 1",
    )
    .bind(&hash)
    .fetch_optional(state.repository.as_ref().unwrap().pool())
    .await
    .expect("query");
    assert!(released_row.is_some(), "RELEASED audit row must persist");
    cleanup(&state, &hash).await;
}

#[tokio::test]
async fn w03_fully_matched_ioc_creates_no_open_order_reservation() {
    let Some(state) = require_state().await else {
        return;
    };
    let series = seed_series(&state, &format!("{}-{}", "w03", Uuid::new_v4())).await;
    let maker = AccountId::new("0x0000000000000000000000000000000000000003");
    let taker = AccountId::new("0x0000000000000000000000000000000000000004");
    // Resting sell.
    let resting = submit_option_order(
        &state,
        order_input(
            &series,
            maker.clone(),
            Side::Sell,
            1_000_000_000,
            100_000_000,
            3,
            &format!("{}-{}", "w03-maker", Uuid::new_v4()),
            TimeInForce::Gtc,
        ),
    )
    .await
    .expect("resting");
    // IOC buy that fully crosses.
    let taker_order = submit_option_order(
        &state,
        order_input(
            &series,
            taker.clone(),
            Side::Buy,
            1_000_000_000,
            100_000_000,
            4,
            &format!("{}-{}", "w03-taker", Uuid::new_v4()),
            TimeInForce::Ioc,
        ),
    )
    .await
    .expect("taker");
    assert_eq!(taker_order.fills.len(), 1);
    let taker_hash = taker_order
        .order
        .canonical_order_hash
        .clone()
        .expect("taker hash");
    // Taker has no residual → no ACTIVE OPEN_ORDER reservation.
    let taker_reservation =
        get_active_open_order(state.repository.as_ref().unwrap().pool(), &taker_hash)
            .await
            .expect("lookup");
    assert!(
        taker_reservation.is_none(),
        "fully-matched IOC leaves no OPEN_ORDER reservation"
    );
    // The maker order was fully consumed too → its reservation should
    // be released via cleanup path. But actually today's wiring only
    // releases on explicit cancel — after a full fill the resting
    // reservation remains ACTIVE and would need a separate hook. This
    // is a known follow-up: Package G integrates match → CONVERTED
    // transition. Cleanup for now to keep test data hygienic.
    let maker_hash = resting
        .order
        .canonical_order_hash
        .clone()
        .expect("maker hash");
    cleanup(&state, &maker_hash).await;
    cleanup(&state, &taker_hash).await;
}
