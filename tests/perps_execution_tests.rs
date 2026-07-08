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
    assert!(matches!(
        err,
        deopt_v2_backend::error::BackendError::PerpZeroPrice
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
