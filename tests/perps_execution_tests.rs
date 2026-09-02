//! PERPS-ORDER-EXECUTION-INTERNAL-V1 — integration tests for the
//! internal Perps order execution service.
//!
//! These tests bypass the HTTP layer intentionally: the public Perps
//! mutation routes STILL return 503 `PerpsNotLive` (pinned by a
//! regression test below). The internal
//! `submit_perp_order_internal` / `cancel_perp_order_internal`
//! service is exercised directly to prove the orderbook + matching
//! + position updates work end-to-end.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::perps::{
    cancel_perp_order_internal, list_perp_fills_for_account, list_perp_orders_for_account,
    price_reader::{InMemoryPerpOraclePriceReader, RawPriceRead},
    submit_perp_order_internal, PerpOrderSide, PerpOrderStatus, PerpTimeInForce, PerpsReadConfig,
    SubmitPerpOrderInput,
};
use deopt_v2_backend::types::{now_ms, AccountId};
use tower::ServiceExt;

const ONE: u128 = 100_000_000; // 1e8
const PRICE_ETH_3000: u128 = 3000 * ONE; // $3000 → 300_000_000_000
const PRICE_ETH_3100: u128 = 3100 * ONE; // $3100 → 310_000_000_000
const MARGIN_10X_ETH: u128 = 300 * ONE; // $300 → 30_000_000_000

fn addr(hex: &str) -> AccountId {
    AccountId::new(hex.to_string())
}

fn state() -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.rpc_url = None;
    state.perps_read_config = cfg;
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

fn stale_price_reader() -> InMemoryPerpOraclePriceReader {
    InMemoryPerpOraclePriceReader::new().with_price(
        "ETH-PERP",
        RawPriceRead {
            price_1e8: PRICE_ETH_3000,
            updated_at_sec: 1, // ancient
            ok: true,
        },
    )
}

fn unavailable_price_reader() -> InMemoryPerpOraclePriceReader {
    InMemoryPerpOraclePriceReader::new().with_forced_error("simulated oracle transport failure")
}

fn base_input(
    account: AccountId,
    side: PerpOrderSide,
    price: u128,
    size: u128,
) -> SubmitPerpOrderInput {
    SubmitPerpOrderInput {
        account,
        subaccount_id: 1,
        market_id: "ETH-PERP".to_string(),
        side,
        price_1e8: price,
        size_1e8: size,
        time_in_force: PerpTimeInForce::Gtc,
        post_only: false,
        reduce_only: false,
        isolated_margin_1e8: MARGIN_10X_ETH, // $300 → 10x on 1 ETH @ $3000
        client_order_id: None,
        max_execution_price_1e8: 0,
        min_execution_price_1e8: 0,
    }
}

// =====================================================================
// A. Rest / cross / partial semantics
// =====================================================================

#[tokio::test]
async fn limit_buy_rests_when_no_sell_liquidity() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Buy,
                PRICE_ETH_3000,
                ONE,
            ),
        )
        .await
        .unwrap()
    };
    assert!(outcome.fills.is_empty());
    assert_eq!(outcome.order.status, PerpOrderStatus::Open);
    assert_eq!(outcome.order.remaining_size_1e8, ONE);
}

#[tokio::test]
async fn crossing_taker_matches_at_maker_price() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // Resting sell at $3000.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE,
            ),
        )
        .await
        .unwrap();
    }
    // Taker buy at $3100 — should fill at maker's $3000 price.
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000bbb"),
                PerpOrderSide::Buy,
                PRICE_ETH_3100,
                ONE,
            ),
        )
        .await
        .unwrap()
    };
    assert_eq!(outcome.fills.len(), 1);
    let fill = &outcome.fills[0];
    assert_eq!(fill.price_1e8, PRICE_ETH_3000);
    assert_eq!(fill.size_1e8, ONE);
    assert_eq!(outcome.order.status, PerpOrderStatus::Filled);
    // Positions on both sides updated via apply_perp_fill_for_account.
    let positions = state.perp_positions_store.lock().unwrap();
    let taker_pos = positions
        .get_active(
            &addr("0x0000000000000000000000000000000000000bbb"),
            1,
            "ETH-PERP",
        )
        .expect("taker position");
    assert_eq!(taker_pos.size_1e8, ONE);
    let maker_pos = positions
        .get_active(
            &addr("0x0000000000000000000000000000000000000aaa"),
            1,
            "ETH-PERP",
        )
        .expect("maker position");
    assert_eq!(maker_pos.size_1e8, ONE);
}

#[tokio::test]
async fn partial_fill_leaves_taker_remainder_open() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // Small resting sell.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE / 2,
            ),
        )
        .await
        .unwrap();
    }
    // Bigger taker buy.
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                size_1e8: ONE,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    PRICE_ETH_3000,
                    ONE,
                )
            },
        )
        .await
        .unwrap()
    };
    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.order.status, PerpOrderStatus::PartiallyFilled);
    assert_eq!(outcome.order.remaining_size_1e8, ONE / 2);
}

// =====================================================================
// B. TIF + post-only
// =====================================================================

#[tokio::test]
async fn ioc_cancels_unfilled_remainder() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE / 2,
            ),
        )
        .await
        .unwrap();
    }
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                size_1e8: ONE,
                time_in_force: PerpTimeInForce::Ioc,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    PRICE_ETH_3000,
                    ONE,
                )
            },
        )
        .await
        .unwrap()
    };
    assert_eq!(outcome.order.status, PerpOrderStatus::Cancelled);
    assert_eq!(outcome.order.remaining_size_1e8, ONE / 2);
    assert_eq!(
        outcome.order.terminal_reason_code.as_deref(),
        Some("ioc_unfilled_remainder")
    );
}

#[tokio::test]
async fn fok_rejects_when_not_fully_fillable() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE / 2,
            ),
        )
        .await
        .unwrap();
    }
    let err = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                size_1e8: ONE,
                time_in_force: PerpTimeInForce::Fok,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    PRICE_ETH_3000,
                    ONE,
                )
            },
        )
        .await
        .unwrap_err()
    };
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpFokNotFillable
    ));
    // The maker's row must NOT have moved: FOK failure rolls back
    // any intended fill (in practice, we never mutate the maker
    // because FOK rejects before commit).
    let orders = state.perp_order_store.lock().unwrap();
    let maker = orders
        .list_orders_for_account(&addr("0x0000000000000000000000000000000000000aaa"))
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(maker.remaining_size_1e8, ONE / 2);
    assert_eq!(maker.status, PerpOrderStatus::Open);
}

#[tokio::test]
async fn post_only_rejects_when_marketable() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE,
            ),
        )
        .await
        .unwrap();
    }
    let err = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                post_only: true,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    PRICE_ETH_3100,
                    ONE,
                )
            },
        )
        .await
        .unwrap_err()
    };
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpPostOnlyWouldMatch
    ));
}

#[tokio::test]
async fn post_only_ioc_combination_rejected() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let mut orders = state.perp_order_store.lock().unwrap();
    let mut positions = state.perp_positions_store.lock().unwrap();
    let err = submit_perp_order_internal(
        &cfg,
        &mut orders,
        &mut positions,
        &reader,
        SubmitPerpOrderInput {
            post_only: true,
            time_in_force: PerpTimeInForce::Ioc,
            ..base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Buy,
                PRICE_ETH_3000,
                ONE,
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpInvalidTifCombination(_)
    ));
}

// =====================================================================
// C. Risk gates
// =====================================================================

#[tokio::test]
async fn opens_are_blocked_when_mark_is_stale() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = stale_price_reader();
    let mut orders = state.perp_order_store.lock().unwrap();
    let mut positions = state.perp_positions_store.lock().unwrap();
    let err = submit_perp_order_internal(
        &cfg,
        &mut orders,
        &mut positions,
        &reader,
        base_input(
            addr("0x0000000000000000000000000000000000000aaa"),
            PerpOrderSide::Buy,
            PRICE_ETH_3000,
            ONE,
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpMarkPriceUnavailable(_)
    ));
}

#[tokio::test]
async fn opens_are_blocked_when_oracle_is_unavailable() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = unavailable_price_reader();
    let mut orders = state.perp_order_store.lock().unwrap();
    let mut positions = state.perp_positions_store.lock().unwrap();
    let err = submit_perp_order_internal(
        &cfg,
        &mut orders,
        &mut positions,
        &reader,
        base_input(
            addr("0x0000000000000000000000000000000000000aaa"),
            PerpOrderSide::Buy,
            PRICE_ETH_3000,
            ONE,
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpMarkPriceUnavailable(_)
    ));
}

#[tokio::test]
async fn zero_size_rejected() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let mut orders = state.perp_order_store.lock().unwrap();
    let mut positions = state.perp_positions_store.lock().unwrap();
    let err = submit_perp_order_internal(
        &cfg,
        &mut orders,
        &mut positions,
        &reader,
        SubmitPerpOrderInput {
            size_1e8: 0,
            ..base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Buy,
                PRICE_ETH_3000,
                0,
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpZeroSize
    ));
}

#[tokio::test]
async fn zero_price_rejected() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let mut orders = state.perp_order_store.lock().unwrap();
    let mut positions = state.perp_positions_store.lock().unwrap();
    let err = submit_perp_order_internal(
        &cfg,
        &mut orders,
        &mut positions,
        &reader,
        SubmitPerpOrderInput {
            price_1e8: 0,
            ..base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Buy,
                0,
                ONE,
            )
        },
    )
    .await
    .unwrap_err();
    // PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — `price_1e8 == 0` now
    // marks a MARKET order (not a hard reject). Without a matching-side
    // user bound, validate_input_basics fail-closes with
    // `PerpsInvalidBoundForSide` instead of the pre-milestone
    // `PerpZeroPrice`.
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpsInvalidBoundForSide(_)
    ));
}

#[tokio::test]
async fn unsupported_market_rejected() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let mut orders = state.perp_order_store.lock().unwrap();
    let mut positions = state.perp_positions_store.lock().unwrap();
    let err = submit_perp_order_internal(
        &cfg,
        &mut orders,
        &mut positions,
        &reader,
        SubmitPerpOrderInput {
            market_id: "SOL-PERP".to_string(),
            ..base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Buy,
                PRICE_ETH_3000,
                ONE,
            )
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpsMarketNotFound(_)
    ));
}

#[tokio::test]
async fn insufficient_margin_rejected_at_position_layer() {
    // A resting sell that gets crossed will call apply_perp_fill,
    // which enforces initial margin. We prove the whole path by
    // submitting a buy taker with a tiny margin — the taker order's
    // margin gets pro-rated per fill, and the applicator refuses.
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE,
            ),
        )
        .await
        .unwrap();
    }
    // Taker with $10 margin at $3000 for 1 ETH → leverage ≫ 10x
    let err = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                isolated_margin_1e8: 100_000_000, // $1
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    PRICE_ETH_3000,
                    ONE,
                )
            },
        )
        .await
        .unwrap_err()
    };
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpInsufficientMargin(_)
    ));
}

#[tokio::test]
async fn self_trade_rejected() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let account = addr("0x0000000000000000000000000000000000000aaa");
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(account.clone(), PerpOrderSide::Sell, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap();
    }
    let err = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(account.clone(), PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap_err()
    };
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpSelfTrade
    ));
}

// =====================================================================
// D. Reduce-only + flip-reject
// =====================================================================

#[tokio::test]
async fn reduce_only_without_existing_opposite_position_rejected() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // Resting sell so we would cross.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE,
            ),
        )
        .await
        .unwrap();
    }
    // A reduce-only taker with no existing position must reject.
    let err = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                reduce_only: true,
                isolated_margin_1e8: 0,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    PRICE_ETH_3000,
                    ONE,
                )
            },
        )
        .await
        .unwrap_err()
    };
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpReduceOnlyViolation
    ));
}

// =====================================================================
// E. Cancel
// =====================================================================

#[tokio::test]
async fn cancel_owned_resting_order_terminates_it() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let account = addr("0x0000000000000000000000000000000000000aaa");
    let id = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(account.clone(), PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap()
        .order
        .id
    };
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        cancel_perp_order_internal(&mut orders, id, &account).unwrap();
    }
    let orders = state.perp_order_store.lock().unwrap();
    let row = orders.get(id).unwrap();
    assert_eq!(row.status, PerpOrderStatus::Cancelled);
    assert_eq!(row.terminal_reason_code.as_deref(), Some("user_cancelled"));
}

#[tokio::test]
async fn cancel_by_stranger_rejected() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let owner = addr("0x0000000000000000000000000000000000000aaa");
    let stranger = addr("0x0000000000000000000000000000000000000bbb");
    let id = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(owner.clone(), PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap()
        .order
        .id
    };
    let mut orders = state.perp_order_store.lock().unwrap();
    let err = cancel_perp_order_internal(&mut orders, id, &stranger).unwrap_err();
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpInvalidOrderState(_)
    ));
}

// =====================================================================
// F. Listing helpers
// =====================================================================

#[tokio::test]
async fn list_orders_and_fills_for_account_returns_expected_rows() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    let alice = addr("0x0000000000000000000000000000000000000aaa");
    let bob = addr("0x0000000000000000000000000000000000000bbb");
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(alice.clone(), PerpOrderSide::Sell, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap();
    }
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(bob.clone(), PerpOrderSide::Buy, PRICE_ETH_3000, ONE),
        )
        .await
        .unwrap();
    }
    let orders = state.perp_order_store.lock().unwrap();
    assert_eq!(list_perp_orders_for_account(&orders, &alice).len(), 1);
    assert_eq!(list_perp_orders_for_account(&orders, &bob).len(), 1);
    // The single fill shows up on BOTH accounts' feeds.
    assert_eq!(list_perp_fills_for_account(&orders, &alice).len(), 1);
    assert_eq!(list_perp_fills_for_account(&orders, &bob).len(), 1);
    let _ = MARGIN_10X_ETH;
}

// =====================================================================
// G. Fail-closed regression pin — public routes still 503
// =====================================================================

#[tokio::test]
async fn public_perp_submit_still_fail_closed_after_internal_execution_ships() {
    let state = state();
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "market_id": 1,
                        "account": "0x0000000000000000000000000000000000000abc",
                        "side": "buy",
                        "price_1e8": "1000",
                        "size_1e8": "10",
                        "time_in_force": "gtc",
                        "reduce_only": false,
                        "post_only": false,
                        "client_order_id": null,
                        "signed_nonce": null,
                        "signed_deadline_ms": null
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("perp"));
}

// =====================================================================
// H. PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — Market-order attack
//    scenarios (Part G, scenarios 9-12). Covers user-bound gating,
//    liquidity exhaustion, and no-fabrication guarantees. The walker
//    reuses `plan_fills` — market orders substitute the user bound as
//    the effective limit price (see `effective_limit_price_1e8`).
// =====================================================================

/// Scenario 9 — market order exhausts allowed liquidity: taker size
/// exceeds available depth within the user bound → walker takes what
/// exists, cancels the remainder (IOC-like). No fabricated fill; the
/// taker's leftover is visible in `remaining_size_1e8`.
#[tokio::test]
async fn scenario_9_market_order_exhausts_allowed_liquidity() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // Two small resting asks at the top of book, both within a
    // permissive user bound. Total available: 0.75 ETH.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE / 2,
            ),
        )
        .await
        .unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aab"),
                PerpOrderSide::Sell,
                PRICE_ETH_3100,
                ONE / 4,
            ),
        )
        .await
        .unwrap();
    }
    // Market buy for 2 ETH with a bound above both asks. Walker
    // consumes both levels, cancels the 1.25 ETH remainder — the
    // user's bound authorises higher prints but the book is empty.
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                price_1e8: 0, // market
                max_execution_price_1e8: PRICE_ETH_3100 + ONE, // above both asks
                time_in_force: PerpTimeInForce::Ioc,
                // 3x MARGIN_10X_ETH: safely covers the blended
                // pro-rated initial margin across two levels ($3000
                // + $3100) even though only 0.75 ETH will fill.
                isolated_margin_1e8: 3 * MARGIN_10X_ETH,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    0,
                    2 * ONE,
                )
            },
        )
        .await
        .unwrap()
    };
    assert_eq!(outcome.fills.len(), 2, "walker consumed both resting asks");
    let total_filled: u128 = outcome.fills.iter().map(|f| f.size_1e8).sum();
    assert_eq!(total_filled, ONE / 2 + ONE / 4);
    assert_eq!(
        outcome.order.remaining_size_1e8,
        2 * ONE - total_filled,
        "residual size visible; walker did NOT fabricate a fill"
    );
    assert_eq!(outcome.order.status, PerpOrderStatus::Cancelled);
}

/// Scenario 10 — market order stops walking at the user's slippage
/// bound. Book has one ask at the bound and a second ask above it.
/// The walker takes the first, refuses the second, cancels the rest.
#[tokio::test]
async fn scenario_10_market_order_stops_at_user_slippage_boundary() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // First ask AT the bound; second ask ABOVE.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                ONE / 2,
            ),
        )
        .await
        .unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aab"),
                PerpOrderSide::Sell,
                PRICE_ETH_3100, // above bound
                ONE,
            ),
        )
        .await
        .unwrap();
    }
    // Market buy 2 ETH with a tight bound at $3000 exactly — walker
    // fills the first level (equal to bound is inclusive), stops
    // before crossing the second (above bound).
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                price_1e8: 0,
                max_execution_price_1e8: PRICE_ETH_3000, // strict cap
                time_in_force: PerpTimeInForce::Ioc,
                isolated_margin_1e8: 2 * MARGIN_10X_ETH, // 10x on 2 ETH
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    0,
                    2 * ONE,
                )
            },
        )
        .await
        .unwrap()
    };
    assert_eq!(outcome.fills.len(), 1, "only the at-bound level fills");
    assert_eq!(outcome.fills[0].price_1e8, PRICE_ETH_3000);
    assert_eq!(outcome.fills[0].size_1e8, ONE / 2);
    assert_eq!(outcome.order.status, PerpOrderStatus::Cancelled);
    assert_eq!(outcome.order.remaining_size_1e8, 2 * ONE - ONE / 2);
}

/// Scenario 11 — partial market fill semantics: one resting ask of
/// less size than requested; walker fills what exists, cancels the
/// rest. Symmetric to Scenario 9 but with a single level for
/// clarity of the partial-fill assertion.
#[tokio::test]
async fn scenario_11_market_order_partial_fill() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // Single resting ask of 0.3 ETH.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3000,
                (3 * ONE) / 10,
            ),
        )
        .await
        .unwrap();
    }
    // Market buy 1 ETH.
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                price_1e8: 0,
                max_execution_price_1e8: PRICE_ETH_3000 + ONE,
                time_in_force: PerpTimeInForce::Ioc,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    0,
                    ONE,
                )
            },
        )
        .await
        .unwrap()
    };
    assert_eq!(outcome.fills.len(), 1);
    assert_eq!(outcome.fills[0].size_1e8, (3 * ONE) / 10);
    assert_eq!(outcome.order.filled_size_1e8, (3 * ONE) / 10);
    assert_eq!(outcome.order.remaining_size_1e8, ONE - (3 * ONE) / 10);
    assert_eq!(outcome.order.status, PerpOrderStatus::Cancelled);
}

/// Scenario 12 — no acceptable liquidity within the user's bound:
/// resting ask above the bound, walker refuses to cross and cancels
/// the entire taker. Zero fills. No fabricated position.
#[tokio::test]
async fn scenario_12_market_order_no_acceptable_liquidity() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // Sole ask sits ABOVE the user's slippage bound.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Sell,
                PRICE_ETH_3100,
                ONE,
            ),
        )
        .await
        .unwrap();
    }
    // Market buy 1 ETH with a bound BELOW the only ask. Walker
    // refuses to cross. Zero fills. Order cancelled. No fabricated
    // position for the taker.
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                price_1e8: 0,
                max_execution_price_1e8: PRICE_ETH_3000, // below the ask
                time_in_force: PerpTimeInForce::Ioc,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Buy,
                    0,
                    ONE,
                )
            },
        )
        .await
        .unwrap()
    };
    assert!(outcome.fills.is_empty(), "walker refuses to cross bound");
    assert_eq!(outcome.order.filled_size_1e8, 0);
    assert_eq!(outcome.order.remaining_size_1e8, ONE);
    assert_eq!(outcome.order.status, PerpOrderStatus::Cancelled);
    // Sanity: no position created for the taker.
    let positions = state.perp_positions_store.lock().unwrap();
    assert!(positions
        .get_active(
            &addr("0x0000000000000000000000000000000000000bbb"),
            1,
            "ETH-PERP"
        )
        .is_none());
}

/// Scenario 13 — Market Sell sweeps bids best→worse (high price
/// first, low price last). Symmetric to scenario 9 for the sell side.
/// Two resting bids at $3100 and $3000. Market sell 0.75 ETH with a
/// `min_execution_price_1e8` bound BELOW both bids: walker consumes
/// the top bid ($3100 for 0.5 ETH), then the second ($3000 for 0.25
/// ETH), cancels any remainder. Proves the walker's bid-side sort +
/// stop-conditions mirror the ask-side path.
#[tokio::test]
async fn scenario_13_market_sell_sweeps_bids_best_to_worst() {
    let state = state();
    let cfg = state.perps_read_config.clone();
    let reader = fresh_price_reader();
    // Two resting bids. Insertion order deliberately not best-first:
    // the walker MUST use price-time priority (bids descending), so
    // the $3100 bid ships FIRST even though it was submitted SECOND.
    {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        // $3000 bid, 0.25 ETH (submitted first).
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aaa"),
                PerpOrderSide::Buy,
                PRICE_ETH_3000,
                ONE / 4,
            ),
        )
        .await
        .unwrap();
        // $3100 bid, 0.5 ETH (submitted second — higher price, better bid).
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            base_input(
                addr("0x0000000000000000000000000000000000000aab"),
                PerpOrderSide::Buy,
                PRICE_ETH_3100,
                ONE / 2,
            ),
        )
        .await
        .unwrap();
    }
    // Market sell 0.75 ETH with a bound at $3000 (accepts both bids).
    let outcome = {
        let mut orders = state.perp_order_store.lock().unwrap();
        let mut positions = state.perp_positions_store.lock().unwrap();
        submit_perp_order_internal(
            &cfg,
            &mut orders,
            &mut positions,
            &reader,
            SubmitPerpOrderInput {
                price_1e8: 0, // market
                min_execution_price_1e8: PRICE_ETH_3000, // accepts both
                time_in_force: PerpTimeInForce::Ioc,
                isolated_margin_1e8: MARGIN_10X_ETH,
                ..base_input(
                    addr("0x0000000000000000000000000000000000000bbb"),
                    PerpOrderSide::Sell,
                    0,
                    (3 * ONE) / 4,
                )
            },
        )
        .await
        .unwrap()
    };
    assert_eq!(outcome.fills.len(), 2, "walker consumed both resting bids");
    // First fill MUST be the $3100 bid (best-first sort), NOT the
    // $3000 bid even though $3000 was submitted first in wall time.
    assert_eq!(
        outcome.fills[0].price_1e8, PRICE_ETH_3100,
        "walker took the top bid first"
    );
    assert_eq!(outcome.fills[0].size_1e8, ONE / 2);
    assert_eq!(outcome.fills[1].price_1e8, PRICE_ETH_3000);
    assert_eq!(outcome.fills[1].size_1e8, ONE / 4);
    assert_eq!(outcome.order.filled_size_1e8, (3 * ONE) / 4);
    assert_eq!(outcome.order.remaining_size_1e8, 0);
    assert_eq!(outcome.order.status, PerpOrderStatus::Filled);
}
