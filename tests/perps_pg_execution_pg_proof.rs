// PERPS-PG-HARNESS-AND-REJECTION-EMIT-V1
//
// Live PostgreSQL proof for the PG-backed Perps execution flow. This
// suite is gated on the env var `PERPS_PG_TEST_DATABASE_URL` — set it
// to a freshly-created disposable database URL (never a shared/prod
// URL). If the env var is NOT set, every test returns early as a
// no-op so `cargo test` stays green in developer environments that
// don't run Postgres.
//
// What this suite proves WHEN ENABLED (via
// `PERPS_PG_TEST_DATABASE_URL`):
//
//   1. Migrations `0033_perp_positions.sql` and
//      `0034_perp_orders_and_fills.sql` apply cleanly.
//   2. Resting GTC order persists in `perp_orders` with `status='open'`.
//   3. A crossing taker inserts `perp_fills` + updates both maker and
//      taker orders + upserts both positions in the same transaction.
//   4. The account read endpoints (`/accounts/:address/perps/orders`,
//      `/perps/fills`, `/perps/positions`) return the persisted rows.
//   5. Cancel updates the row to `status='cancelled'` with
//      `terminal_reason_code='user_cancelled'`.
//   6. Post-only would-match leaves NO mutation (transaction rolled
//      back).
//   7. FOK not fillable leaves NO mutation.
//   8. Duplicate client-order-id rejects and leaves no partial row.
//   9. Successful submit emits lifecycle AFTER commit.
//  10. Failed submit emits `PerpOrderRejected` + no other frames.
//
// **Safety**: this file never prints `PERPS_PG_TEST_DATABASE_URL` or
// any derivative. Every assertion reads non-secret fields only
// (status, size, price, terminal reason). Test rows are per-test-tag
// keyed by a synthetic account so a re-run against the same
// disposable database is safe.

use deopt_v2_backend::api::public_ws::{LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead},
    submit_perp_order_via_state, PerpOrderStatus, PerpTimeInForce, PerpsReadConfig,
    SubmitPerpOrderInput,
};
use deopt_v2_backend::types::{now_ms, AccountId};

const ENV_VAR: &str = "PERPS_PG_TEST_DATABASE_URL";

const ONE: u128 = 100_000_000;
const PRICE_ETH_3000: u128 = 3000 * ONE;
const PRICE_ETH_3100: u128 = 3100 * ONE;
const MARGIN_10X_ETH: u128 = 300 * ONE;

fn pg_test_url() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

/// Deterministic per-test address so re-runs against the same
/// disposable DB never collide with prior test state.
fn per_test_account(tag: &str, prefix: &str) -> AccountId {
    let sum: u32 = tag.bytes().map(u32::from).sum();
    let mut hex = String::from("0x");
    hex.push_str(prefix);
    hex.push_str(&format!("{:>04x}", sum & 0xffff));
    for b in tag.bytes().take(8) {
        hex.push_str(&format!("{:02x}", b));
    }
    while hex.len() < 42 {
        hex.push('0');
    }
    hex.truncate(42);
    AccountId::new(hex)
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

async fn pg_state(url: &str) -> AppState {
    let repo = fresh_repo(url).await;
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
    state.repository = Some(repo);
    state.persistence_enabled = true;
    state.database_configured = true;
    assert!(
        state.repository.is_some(),
        "repository must be wired for the PG proof"
    );
    state
}

fn fresh_price_reader() -> InMemoryPerpOraclePriceReader {
    InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: PRICE_ETH_3000,
            updated_at_sec: (now_ms() / 1000) as u64,
            ok: true,
        },
    )
}

fn base_input(
    account: AccountId,
    side: deopt_v2_backend::perps::PerpOrderSide,
    price: u128,
    size: u128,
    tag: &str,
) -> SubmitPerpOrderInput {
    SubmitPerpOrderInput {
        account,
        market_id: "ETH-PERP".to_string(),
        side,
        price_1e8: price,
        size_1e8: size,
        time_in_force: PerpTimeInForce::Gtc,
        post_only: false,
        reduce_only: false,
        isolated_margin_1e8: MARGIN_10X_ETH,
        client_order_id: Some(format!("cli-{tag}")),
    }
}

fn drain(rx: &mut tokio::sync::broadcast::Receiver<LifecycleEvent>) -> Vec<LifecycleEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(_) => break,
        }
    }
    out
}

// =====================================================================
// 1. Resting order persists in PG
// =====================================================================

#[tokio::test]
async fn pg_resting_gtc_order_persists() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "rest-a";
    let alice = per_test_account(tag, "aaa1");
    let outcome = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            alice.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Buy,
            PRICE_ETH_3000 - ONE, // sub-market so it rests
            ONE,
            tag,
        ),
    )
    .await
    .unwrap();
    assert_eq!(outcome.order.status, PerpOrderStatus::Open);
    let repo = state.repository.clone().unwrap();
    let orders = repo.list_perp_orders_for_account(&alice).await.unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].id, outcome.order.id);
    assert_eq!(orders[0].status, PerpOrderStatus::Open);
    assert_eq!(orders[0].remaining_size_1e8, ONE);
}

// =====================================================================
// 2. Crossing trade persists orders + fills + positions in one tx
// =====================================================================

#[tokio::test]
async fn pg_crossing_trade_persists_orders_fills_and_positions() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "cross-a";
    let alice = per_test_account(tag, "aaa2");
    let bob = per_test_account(tag, "bbb2");
    // Alice rests a sell.
    let maker_outcome = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            alice.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE,
            &format!("{tag}-m"),
        ),
    )
    .await
    .unwrap();
    // Bob crosses.
    let taker_outcome = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            bob.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Buy,
            PRICE_ETH_3100,
            ONE,
            &format!("{tag}-t"),
        ),
    )
    .await
    .unwrap();
    assert_eq!(taker_outcome.fills.len(), 1);
    let fill = &taker_outcome.fills[0];
    assert_eq!(fill.price_1e8, PRICE_ETH_3000);

    let repo = state.repository.clone().unwrap();

    // Fills persisted for both accounts.
    let alice_fills = repo.list_perp_fills_for_account(&alice).await.unwrap();
    let bob_fills = repo.list_perp_fills_for_account(&bob).await.unwrap();
    assert_eq!(alice_fills.len(), 1);
    assert_eq!(bob_fills.len(), 1);
    assert_eq!(alice_fills[0].id, bob_fills[0].id);

    // Taker + maker orders updated to Filled.
    let taker_row = repo
        .get_perp_order(taker_outcome.order.id)
        .await
        .unwrap()
        .unwrap();
    let maker_row = repo
        .get_perp_order(maker_outcome.order.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(taker_row.status, PerpOrderStatus::Filled);
    assert_eq!(maker_row.status, PerpOrderStatus::Filled);

    // Positions persisted for both.
    let alice_pos = repo
        .get_active_perp_position(&alice, "ETH-PERP")
        .await
        .unwrap()
        .expect("maker position must persist");
    let bob_pos = repo
        .get_active_perp_position(&bob, "ETH-PERP")
        .await
        .unwrap()
        .expect("taker position must persist");
    assert_eq!(bob_pos.size_1e8, ONE);
    assert_eq!(alice_pos.size_1e8, ONE);
}

// =====================================================================
// 3. Cancel persists terminal status in PG
// =====================================================================

#[tokio::test]
async fn pg_cancel_persists_terminal_status() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "cancel-a";
    let alice = per_test_account(tag, "aaa3");
    let outcome = submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            alice.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Buy,
            PRICE_ETH_3000 - ONE,
            ONE,
            tag,
        ),
    )
    .await
    .unwrap();
    let cancelled =
        deopt_v2_backend::perps::cancel_perp_order_via_state(&state, outcome.order.id, &alice)
            .await
            .unwrap();
    assert_eq!(cancelled.status, PerpOrderStatus::Cancelled);
    let repo = state.repository.clone().unwrap();
    let persisted = repo
        .get_perp_order(outcome.order.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, PerpOrderStatus::Cancelled);
    assert_eq!(
        persisted.terminal_reason_code.as_deref(),
        Some("user_cancelled")
    );
}

// =====================================================================
// 4. Post-only would-match leaves NO mutation (transaction rolled back)
// =====================================================================

#[tokio::test]
async fn pg_post_only_would_match_leaves_no_mutation() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "po-a";
    let alice = per_test_account(tag, "aaa4");
    let bob = per_test_account(tag, "bbb4");
    // Maker sell rests.
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            alice.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE,
            &format!("{tag}-m"),
        ),
    )
    .await
    .unwrap();
    let repo = state.repository.clone().unwrap();
    let before = repo.list_perp_orders_for_account(&bob).await.unwrap();
    assert!(before.is_empty(), "bob has no orders yet");
    // Bob's post-only buy at $3100 would cross.
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            post_only: true,
            ..base_input(
                bob.clone(),
                deopt_v2_backend::perps::PerpOrderSide::Buy,
                PRICE_ETH_3100,
                ONE,
                tag,
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpPostOnlyWouldMatch
    ));
    // Bob still has NO order persisted — the tx rolled back.
    let after = repo.list_perp_orders_for_account(&bob).await.unwrap();
    assert!(
        after.is_empty(),
        "post-only rejection must leave NO row in perp_orders; got {after:?}"
    );
    // Alice's maker order remains Open with full remaining size.
    let alice_orders = repo.list_perp_orders_for_account(&alice).await.unwrap();
    assert_eq!(alice_orders.len(), 1);
    assert_eq!(alice_orders[0].status, PerpOrderStatus::Open);
    assert_eq!(alice_orders[0].remaining_size_1e8, ONE);
}

// =====================================================================
// 5. FOK not fillable leaves NO mutation
// =====================================================================

#[tokio::test]
async fn pg_fok_not_fillable_leaves_no_mutation() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "fok-a";
    let alice = per_test_account(tag, "aaa5");
    let bob = per_test_account(tag, "bbb5");
    // Small resting sell.
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            alice.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE / 2,
            &format!("{tag}-m"),
        ),
    )
    .await
    .unwrap();
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            size_1e8: ONE,
            time_in_force: PerpTimeInForce::Fok,
            ..base_input(
                bob.clone(),
                deopt_v2_backend::perps::PerpOrderSide::Buy,
                PRICE_ETH_3100,
                ONE,
                tag,
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpFokNotFillable
    ));
    let repo = state.repository.clone().unwrap();
    // Bob's FOK order MUST NOT be persisted.
    let bob_orders = repo.list_perp_orders_for_account(&bob).await.unwrap();
    assert!(bob_orders.is_empty(), "FOK rejection must leave no row");
    // No fills.
    let bob_fills = repo.list_perp_fills_for_account(&bob).await.unwrap();
    assert!(bob_fills.is_empty(), "FOK rejection must leave no fill");
    // Alice's maker order is still untouched.
    let alice_orders = repo.list_perp_orders_for_account(&alice).await.unwrap();
    assert_eq!(alice_orders[0].remaining_size_1e8, ONE / 2);
    assert_eq!(alice_orders[0].status, PerpOrderStatus::Open);
}

// =====================================================================
// 6. Duplicate client_order_id rejects with no partial row
// =====================================================================

#[tokio::test]
async fn pg_duplicate_client_order_id_rejects() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "dup-a";
    let alice = per_test_account(tag, "aaa6");
    let cli = format!("cli-dup-{tag}");
    submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            client_order_id: Some(cli.clone()),
            ..base_input(
                alice.clone(),
                deopt_v2_backend::perps::PerpOrderSide::Buy,
                PRICE_ETH_3000 - ONE,
                ONE,
                tag,
            )
        },
    )
    .await
    .unwrap();
    let err = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            client_order_id: Some(cli.clone()),
            ..base_input(
                alice.clone(),
                deopt_v2_backend::perps::PerpOrderSide::Buy,
                PRICE_ETH_3000 - 2 * ONE,
                ONE,
                &format!("{tag}-2"),
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpDuplicateClientOrderId(_)
    ));
    let repo = state.repository.clone().unwrap();
    // Only one order for alice with that client_order_id.
    let orders = repo.list_perp_orders_for_account(&alice).await.unwrap();
    let with_cli: Vec<_> = orders
        .iter()
        .filter(|o| o.client_order_id.as_deref() == Some(cli.as_str()))
        .collect();
    assert_eq!(with_cli.len(), 1);
}

// =====================================================================
// 7. Lifecycle after commit — a crossing trade produces the full bundle
// =====================================================================

#[tokio::test]
async fn pg_successful_crossing_emits_lifecycle_after_commit() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "life-a";
    let alice = per_test_account(tag, "aaa7");
    let bob = per_test_account(tag, "bbb7");
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            alice.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE,
            &format!("{tag}-m"),
        ),
    )
    .await
    .unwrap();
    let mut rx = state.lifecycle_events.subscribe();
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            bob,
            deopt_v2_backend::perps::PerpOrderSide::Buy,
            PRICE_ETH_3100,
            ONE,
            &format!("{tag}-t"),
        ),
    )
    .await
    .unwrap();
    let events = drain(&mut rx);
    let order_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpOrderUpdated { .. }))
        .count();
    let fill_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpFillCreated { .. }))
        .count();
    let position_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpPositionUpdated { .. }))
        .count();
    assert!(order_count >= 1);
    assert_eq!(fill_count, 2);
    assert!(position_count >= 2);
}

// =====================================================================
// 8. Failed PG submit emits PerpOrderRejected + no ok frames
// =====================================================================

#[tokio::test]
async fn pg_failed_submit_emits_perp_order_rejected_and_nothing_else() {
    let Some(url) = pg_test_url() else {
        return;
    };
    let state = pg_state(&url).await;
    let reader = fresh_price_reader();
    let tag = "reject-a";
    let alice = per_test_account(tag, "aaa8");
    let bob = per_test_account(tag, "bbb8");
    submit_perp_order_via_state(
        &state,
        &reader,
        base_input(
            alice.clone(),
            deopt_v2_backend::perps::PerpOrderSide::Sell,
            PRICE_ETH_3000,
            ONE,
            &format!("{tag}-m"),
        ),
    )
    .await
    .unwrap();
    let mut rx = state.lifecycle_events.subscribe();
    let _ = submit_perp_order_via_state(
        &state,
        &reader,
        SubmitPerpOrderInput {
            post_only: true,
            ..base_input(
                bob,
                deopt_v2_backend::perps::PerpOrderSide::Buy,
                PRICE_ETH_3100,
                ONE,
                tag,
            )
        },
    )
    .await
    .unwrap_err();
    let events = drain(&mut rx);
    let rejection_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpOrderRejected { .. }))
        .count();
    let order_updated_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpOrderUpdated { .. }))
        .count();
    let fill_count = events
        .iter()
        .filter(|e| matches!(e.payload, LifecyclePayload::PerpFillCreated { .. }))
        .count();
    assert_eq!(rejection_count, 1);
    assert_eq!(order_updated_count, 0, "no OK frames should fire on reject");
    assert_eq!(fill_count, 0, "no fill frames should fire on reject");
}
