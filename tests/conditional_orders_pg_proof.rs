// OPTIONS-CONDITIONAL-ORDERS-LIVE-POSTGRES-PROOF-V1
//
// Live PostgreSQL proof for the TP/SL system. This suite is gated on
// the env var `CONDITIONAL_PG_TEST_DATABASE_URL` which the runner
// script (`scripts/conditional-orders-pg-proof.sh`) sets to a freshly
// created disposable database. If the env var is not set, every test
// returns early as a no-op so `cargo test` stays green in standard
// developer environments.
//
// What this suite proves WHEN ENABLED:
//
//   1. Migration 0028 applies cleanly against a fresh database.
//   2. The `options_conditional_orders` table exists with the four
//      required indexes + the `version` column.
//   3. The `option_orders` UNIQUE INDEX that prevents duplicate child
//      `client_order_id` IS present (defence against double-trigger
//      after restart).
//   4. The repository path actually executes (we assert
//      `state.repository.is_some()`).
//   5. Armed orders survive a simulated reload (re-read via a fresh
//      `PgRepository` connection).
//   6. Completed orders never retrigger across repeated ticks +
//      reload.
//   7. Stranded `triggering` rows recover correctly (with /
//      without `child_order_id`).
//   8. Stale oracle (empty price snapshot) creates no child.
//   9. Reduce-only quantity capping persists.
//  10. IOC with no liquidity records the documented terminal failure
//      and never resurfaces fills.
//  11. Two competing evaluator instances on the SAME database +
//      SAME OCO group produce exactly one winner + one cancelled
//      sibling + one child order. Repeated ticks after the race do
//      not create a second child.
//
// **Safety**: this file never prints `CONDITIONAL_PG_TEST_DATABASE_URL`
// or any derivative. All assertions read non-secret lifecycle fields
// only (status, child_order_id, version, oco_group_id, failure_code).

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::conditional_orders::{
    create_conditional_orders, evaluate_conditional_orders_tick_with_prices,
    list_conditional_orders, recover_stranded_triggering, ConditionalLegInput,
    ConditionalOrderFilter, ConditionalOrderStatus, ConditionalType, CreateConditionalOrderInput,
};
use deopt_v2_backend::options::service::{
    create_option_series, submit_option_order, CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::OptionsConfig;
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};
use std::collections::HashMap;
use std::sync::Arc;

const ENV_VAR: &str = "CONDITIONAL_PG_TEST_DATABASE_URL";

const ONE_1E8: u128 = 100_000_000;
const PREMIUM_1E8: u128 = 1_000_000_000;
const HOLDER: &str = "0x0000000000000000000000000000000000000111";
const MAKER: &str = "0x0000000000000000000000000000000000000222";

fn account(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

/// Read the disposable-DB URL from env. Returns `None` if the var is
/// not set so the test can no-op cleanly.
fn pg_test_url() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

/// Build a connected, migrated repository against the disposable DB.
async fn fresh_pg_repository(url: &str) -> PgRepository {
    let repo = PgRepository::connect(url)
        .await
        .expect("connect to disposable PG database");
    repo.run_migrations()
        .await
        .expect("run migrations against disposable PG database");
    repo
}

/// Build an `AppState` that routes the conditional-orders service
/// through the live repository (not the in-memory store).
async fn pg_state(url: &str) -> AppState {
    let repo = fresh_pg_repository(url).await;
    let state = AppState::with_options_config_and_repository(
        EngineState::with_default_markets(),
        OptionsConfig::enabled_in_memory_for_tests(),
        repo,
    );
    assert!(
        state.repository.is_some(),
        "repository must be wired for the PG proof"
    );
    assert!(
        state.persistence_enabled,
        "persistence_enabled must be true so DB code paths are used"
    );
    state
}

async fn seed_long_position(state: &AppState, series: &str, size_1e8: u128, tag: &str) {
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(MAKER),
            side: Side::Sell,
            price_1e8: PREMIUM_1E8,
            size_1e8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("pg-maker-{tag}-{size_1e8}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(HOLDER),
            side: Side::Buy,
            price_1e8: PREMIUM_1E8,
            size_1e8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("pg-holder-{tag}-{size_1e8}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();
}

async fn seed_closing_bid(
    state: &AppState,
    series: &str,
    size_1e8: u128,
    price_1e8: u128,
    tag: &str,
) {
    if size_1e8 == 0 {
        return;
    }
    submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: series.to_string(),
            account: account(MAKER),
            side: Side::Buy,
            price_1e8,
            size_1e8,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            client_order_id: Some(format!("pg-bid-{tag}-{size_1e8}-{price_1e8}")),
            nonce: None,
            deadline_ms: None,
            signature: None,
        },
    )
    .await
    .unwrap();
}

async fn make_series(state: &AppState, tag: &str) -> String {
    let now_sec = (now_ms() / 1000) as u64;
    // Use a deterministic-but-unique strike per test to avoid series-id
    // collisions across tests inside the same database.
    let strike = 70_000_000_000u128 + (tag.bytes().map(u128::from).sum::<u128>() * 1_000);
    let series = create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "BTC".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry: now_sec + 7 * 24 * 3600,
            strike_1e8: strike,
            is_call: true,
            contract_size_1e8: Some(ONE_1E8),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .unwrap();
    series.option_series_id
}

async fn arm_oco_tp_sl(
    state: &AppState,
    series: &str,
    qty: u128,
    tp: u128,
    sl: u128,
) -> Vec<deopt_v2_backend::options::conditional_orders::ConditionalOrder> {
    create_conditional_orders(
        state,
        CreateConditionalOrderInput {
            account: account(HOLDER),
            option_series_id: series.to_string(),
            quantity_1e8: qty,
            legs: vec![
                ConditionalLegInput {
                    conditional_type: ConditionalType::TakeProfit,
                    trigger_price_1e8: tp,
                    limit_price_1e8: PREMIUM_1E8 / 2,
                    explicit_trigger_condition: None,
                },
                ConditionalLegInput {
                    conditional_type: ConditionalType::StopLoss,
                    trigger_price_1e8: sl,
                    limit_price_1e8: PREMIUM_1E8 / 2,
                    explicit_trigger_condition: None,
                },
            ],
            link_as_oco: true,
            expires_at_ms: None,
        },
    )
    .await
    .unwrap()
}

fn prices_for(series_id: &str, price: u128) -> HashMap<String, u128> {
    let mut m = HashMap::new();
    m.insert(series_id.to_string(), price);
    m
}

/// Convenience wrapper: skip the test cleanly if the env var is unset.
macro_rules! pg_test {
    ($name:ident, $body:expr) => {
        #[tokio::test]
        async fn $name() {
            let Some(url) = pg_test_url() else {
                eprintln!(
                    "[pg-proof] {} → skipped (set {} via scripts/conditional-orders-pg-proof.sh)",
                    stringify!($name),
                    ENV_VAR
                );
                return;
            };
            ($body)(url).await;
        }
    };
}

// ===== Phase 3 — Schema asserts =======================================

pg_test!(
    migration_0028_applied_with_indexes_and_unique,
    |url: String| async move {
        let repo = fresh_pg_repository(&url).await;
        let pool = pool_handle(&repo).await;
        // Table exists.
        let table_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables
         WHERE table_name = 'options_conditional_orders')",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert!(table_exists, "options_conditional_orders table missing");
        // 4 required indexes.
        let index_names: Vec<String> = sqlx::query_scalar(
            "SELECT indexname FROM pg_indexes WHERE tablename = 'options_conditional_orders'",
        )
        .fetch_all(&*pool)
        .await
        .unwrap();
        let expected = [
            "idx_options_conditional_orders_armed",
            "idx_options_conditional_orders_account",
            "idx_options_conditional_orders_series",
            "idx_options_conditional_orders_oco",
        ];
        for name in expected {
            assert!(
                index_names.iter().any(|n| n == name),
                "missing index {name}; got {index_names:?}"
            );
        }
        // `version` optimistic-lock column.
        let version_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns
         WHERE table_name = 'options_conditional_orders' AND column_name = 'version')",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert!(version_exists, "version column missing");
        // Child order_id UNIQUE protection on `option_orders` (defence
        // against duplicate child after restart).
        let child_unique_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes
         WHERE tablename = 'option_orders'
           AND indexname = 'idx_option_orders_live_account_client_id')",
        )
        .fetch_one(&*pool)
        .await
        .unwrap();
        assert!(
            child_unique_exists,
            "option_orders unique-client-order-id index missing"
        );
    }
);

// ===== Phase 3 — Lifecycle invariants ================================

pg_test!(
    armed_orders_survive_repository_reload,
    |url: String| async move {
        let state_a = pg_state(&url).await;
        let series = make_series(&state_a, "armed_survive").await;
        seed_long_position(&state_a, &series, ONE_1E8, "armed_survive").await;
        arm_oco_tp_sl(&state_a, &series, ONE_1E8, 80_000_000_000, 60_000_000_000).await;

        // Reload: drop state_a, build state_b from a fresh PgRepository
        // connection (simulates a backend restart).
        drop(state_a);
        let state_b = pg_state(&url).await;
        let armed = list_conditional_orders(
            &state_b,
            ConditionalOrderFilter {
                account: Some(account(HOLDER)),
                option_series_id: Some(series),
                status: Some(ConditionalOrderStatus::Armed),
                oco_group_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            armed.len(),
            2,
            "both legs of the OCO pair must survive reload"
        );
    }
);

pg_test!(
    completed_orders_never_retrigger_after_reload,
    |url: String| async move {
        let state_a = pg_state(&url).await;
        let series = make_series(&state_a, "no_retrigger").await;
        seed_long_position(&state_a, &series, ONE_1E8, "no_retrigger").await;
        seed_closing_bid(&state_a, &series, ONE_1E8, PREMIUM_1E8 / 2, "no_retrigger").await;
        arm_oco_tp_sl(&state_a, &series, ONE_1E8, 80_000_000_000, 60_000_000_000).await;

        let prices = prices_for(&series, 80_000_000_000);
        let r = evaluate_conditional_orders_tick_with_prices(&state_a, &prices)
            .await
            .unwrap();
        assert!(r.triggered >= 1, "TP must trigger");

        // Reload + run 5 more ticks. No additional triggers.
        drop(state_a);
        let state_b = pg_state(&url).await;
        for _ in 0..5 {
            let r = evaluate_conditional_orders_tick_with_prices(&state_b, &prices)
                .await
                .unwrap();
            assert_eq!(r.triggered, 0, "completed orders must not retrigger");
        }
    }
);

pg_test!(
    stranded_triggering_recovers_with_or_without_child,
    |url: String| async move {
        let state = pg_state(&url).await;
        let series = make_series(&state, "stranded").await;
        seed_long_position(&state, &series, ONE_1E8, "stranded").await;
        let rows = arm_oco_tp_sl(&state, &series, ONE_1E8, 80_000_000_000, 60_000_000_000).await;
        let with_child = rows[0].id;
        let without_child = rows[1].id;

        // Stage one row with `Triggering + child_order_id=Some(stub)` and
        // another with `Triggering + child_order_id=None`.
        let repo = state.repository.clone().unwrap();
        let mut tp = repo
            .get_conditional_order(with_child)
            .await
            .unwrap()
            .unwrap();
        tp.status = ConditionalOrderStatus::Triggering;
        tp.child_order_id = Some("stub-child".to_string());
        tp.version = tp.version.saturating_add(1);
        repo.update_conditional_order(&tp).await.unwrap();

        let mut sl = repo
            .get_conditional_order(without_child)
            .await
            .unwrap()
            .unwrap();
        sl.status = ConditionalOrderStatus::Triggering;
        sl.child_order_id = None;
        sl.version = sl.version.saturating_add(1);
        repo.update_conditional_order(&sl).await.unwrap();

        let recovered = recover_stranded_triggering(&state, now_ms()).await.unwrap();
        assert_eq!(recovered, 2, "both stranded rows must be swept");

        let tp_after = repo
            .get_conditional_order(with_child)
            .await
            .unwrap()
            .unwrap();
        let sl_after = repo
            .get_conditional_order(without_child)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tp_after.status, ConditionalOrderStatus::Completed);
        assert_eq!(sl_after.status, ConditionalOrderStatus::Armed);
    }
);

pg_test!(stale_oracle_creates_no_child, |url: String| async move {
    let state = pg_state(&url).await;
    let series = make_series(&state, "stale_oracle").await;
    seed_long_position(&state, &series, ONE_1E8, "stale_oracle").await;
    arm_oco_tp_sl(&state, &series, ONE_1E8, 80_000_000_000, 60_000_000_000).await;

    let empty: HashMap<String, u128> = HashMap::new();
    let r = evaluate_conditional_orders_tick_with_prices(&state, &empty)
        .await
        .unwrap();
    assert_eq!(r.triggered, 0);

    // SQL invariant: zero child option orders exist for this series.
    let repo = state.repository.clone().unwrap();
    let child_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM option_orders WHERE client_order_id LIKE 'cond-%'",
    )
    .fetch_one(&*pool_handle(&repo).await)
    .await
    .unwrap();
    assert_eq!(child_count, 0, "no child IOC order created on stale oracle");
});

pg_test!(
    reduced_position_quantity_cap_persists,
    |url: String| async move {
        let state = pg_state(&url).await;
        let series = make_series(&state, "reduced").await;
        seed_long_position(&state, &series, ONE_1E8, "reduced").await;
        arm_oco_tp_sl(&state, &series, ONE_1E8, 80_000_000_000, 60_000_000_000).await;

        // Holder partially closes 0.40 manually.
        seed_closing_bid(
            &state,
            &series,
            ONE_1E8 * 4 / 10,
            PREMIUM_1E8,
            "reduced-pre",
        )
        .await;
        submit_option_order(
            &state,
            SubmitOptionOrderInput {
                option_series_id: series.clone(),
                account: account(HOLDER),
                side: Side::Sell,
                price_1e8: PREMIUM_1E8,
                size_1e8: ONE_1E8 * 4 / 10,
                time_in_force: TimeInForce::Gtc,
                post_only: false,
                client_order_id: Some("reduced-manual-close".to_string()),
                nonce: None,
                deadline_ms: None,
                signature: None,
            },
        )
        .await
        .unwrap();

        seed_closing_bid(
            &state,
            &series,
            ONE_1E8 * 6 / 10,
            PREMIUM_1E8 / 2,
            "reduced-post",
        )
        .await;
        let prices = prices_for(&series, 80_000_000_000);
        let r = evaluate_conditional_orders_tick_with_prices(&state, &prices)
            .await
            .unwrap();
        assert!(r.triggered >= 1);

        // SQL invariant: at most one cond-* child exists, with size <= 0.60.
        let repo = state.repository.clone().unwrap();
        let row: (i64, Option<String>) = sqlx::query_as(
            "SELECT COUNT(*), MAX(size_1e8) FROM option_orders WHERE client_order_id LIKE 'cond-%'",
        )
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        assert_eq!(row.0, 1, "exactly one child IOC order");
        let max_size: u128 = row.1.unwrap().parse().unwrap();
        assert!(
            max_size <= ONE_1E8 * 6 / 10,
            "child quantity must be capped at reducible (got {max_size})"
        );
    }
);

pg_test!(
    ioc_no_liquidity_terminal_failed_no_resting_order,
    |url: String| async move {
        let state = pg_state(&url).await;
        let series = make_series(&state, "no_liquidity").await;
        seed_long_position(&state, &series, ONE_1E8, "no_liquidity").await;
        arm_oco_tp_sl(&state, &series, ONE_1E8, 80_000_000_000, 60_000_000_000).await;

        // NO closing bid → child IOC will find zero opposing liquidity.
        let prices = prices_for(&series, 80_000_000_000);
        let _ = evaluate_conditional_orders_tick_with_prices(&state, &prices)
            .await
            .unwrap();

        let repo = state.repository.clone().unwrap();
        // SQL invariant: any cond-* child order is in a terminal status
        // (cancelled or filled), NEVER `open` or `partially_filled`.
        let resting: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_orders
         WHERE client_order_id LIKE 'cond-%'
           AND status IN ('open', 'partially_filled')",
        )
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        assert_eq!(resting, 0, "IOC child must never rest");

        // Conditional row marked failed with no_liquidity.
        let failed = list_conditional_orders(
            &state,
            ConditionalOrderFilter {
                account: Some(account(HOLDER)),
                option_series_id: Some(series),
                status: Some(ConditionalOrderStatus::Failed),
                oco_group_id: None,
            },
        )
        .await
        .unwrap();
        assert!(
            failed
                .iter()
                .any(|o| o.failure_code.as_deref() == Some("no_liquidity")),
            "expected at least one Failed/no_liquidity row"
        );
    }
);

// ===== Phase 3 — Two-worker OCO race ==================================

pg_test!(
    two_workers_compete_for_oco_group_one_winner_only,
    |url: String| async move {
        // Single state + two evaluator tasks. Each task gets the SAME
        // AppState (same PgRepository pool), but they call
        // `evaluate_conditional_orders_tick_with_prices` concurrently from
        // separate tokio tasks. The atomic `claim_conditional_order_armed`
        // UPDATE WHERE status='armed' is the cross-process serialiser; it
        // is the same primitive two separate `cargo run` processes would
        // use against the same database.
        let state = pg_state(&url).await;
        let series = make_series(&state, "race").await;
        seed_long_position(&state, &series, ONE_1E8, "race").await;
        seed_closing_bid(&state, &series, ONE_1E8, PREMIUM_1E8 / 2, "race").await;
        let rows = arm_oco_tp_sl(&state, &series, ONE_1E8, 80_000_000_000, 80_000_000_000).await;
        assert_eq!(rows.len(), 2, "OCO pair created");

        let state_arc = Arc::new(state);
        let prices = Arc::new(prices_for(&series, 80_000_000_000));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let s = state_arc.clone();
            let p = prices.clone();
            handles.push(tokio::spawn(async move {
                evaluate_conditional_orders_tick_with_prices(&s, &p)
                    .await
                    .expect("tick")
            }));
        }
        let mut total_triggered = 0;
        for h in handles {
            let r = h.await.unwrap();
            total_triggered += r.triggered;
        }
        assert_eq!(
            total_triggered, 1,
            "exactly one OCO leg can win across concurrent evaluators"
        );

        let repo = state_arc.repository.clone().unwrap();
        // SQL invariants on the conditional row population.
        let completed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM options_conditional_orders
         WHERE option_series_id = $1 AND status = 'completed'",
        )
        .bind(&series)
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        let cancelled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM options_conditional_orders
         WHERE option_series_id = $1 AND status = 'cancelled'
           AND failure_code = 'oco_sibling_triggered'",
        )
        .bind(&series)
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        let cond_orders_total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM options_conditional_orders WHERE option_series_id = $1",
        )
        .bind(&series)
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        assert_eq!(completed, 1, "exactly one completed leg");
        assert_eq!(cancelled, 1, "exactly one OCO-cancelled sibling");
        assert_eq!(cond_orders_total, 2, "no extra conditional rows created");

        // SQL invariants on the child option order + fills.
        let child_orders: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_orders
         WHERE option_series_id = $1 AND client_order_id LIKE 'cond-%'",
        )
        .bind(&series)
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        assert_eq!(
            child_orders, 1,
            "exactly one child IOC order across competing evaluators"
        );

        let child_fills: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_fills f
         JOIN option_orders o ON (f.buy_order_id = o.order_id OR f.sell_order_id = o.order_id)
         WHERE f.option_series_id = $1
           AND o.client_order_id LIKE 'cond-%'",
        )
        .bind(&series)
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        assert!(child_fills >= 1, "at least one fill recorded");

        // After race: 5 more ticks must not create a second child.
        for _ in 0..5 {
            evaluate_conditional_orders_tick_with_prices(&state_arc, &prices)
                .await
                .unwrap();
        }
        let child_orders_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM option_orders
         WHERE option_series_id = $1 AND client_order_id LIKE 'cond-%'",
        )
        .bind(&series)
        .fetch_one(&*pool_handle(&repo).await)
        .await
        .unwrap();
        assert_eq!(
            child_orders_after, 1,
            "no second child after repeated ticks"
        );
    }
);

// ---- Helpers ---------------------------------------------------------

/// Async helper that opens (and caches via `OnceCell`) a short-lived
/// `sqlx::PgPool` against the disposable DB so the tests can run raw
/// SQL assertions without going through the repository abstraction.
/// The URL is read from `ENV_VAR` at call time; the secret never
/// leaves process memory and is never printed by this file.
async fn pool_handle(_repo: &PgRepository) -> std::sync::Arc<sqlx::PgPool> {
    static POOL: tokio::sync::OnceCell<std::sync::Arc<sqlx::PgPool>> =
        tokio::sync::OnceCell::const_new();
    let url = std::env::var(ENV_VAR).expect("ENV var must be set inside pg_test! body");
    POOL.get_or_init(|| async {
        std::sync::Arc::new(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await
                .expect("connect to disposable DB for raw SQL assertions"),
        )
    })
    .await
    .clone()
}
