use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::orderbook::OrderBook;
use deopt_v2_backend::types::{
    AccountId, MarketId, NewOrder, OrderId, OrderStatus, Side, TimeInForce,
};
use std::fs;
use uuid::Uuid;

const MARKET: MarketId = 1;

fn order_id(n: u128) -> OrderId {
    OrderId(Uuid::from_u128(n))
}

fn new_order(
    id: u128,
    account: &str,
    side: Side,
    price_1e8: u128,
    size_1e8: u128,
    time_in_force: TimeInForce,
    post_only: bool,
) -> deopt_v2_backend::types::Order {
    NewOrder {
        market_id: MARKET,
        account: AccountId::new(account),
        side,
        price_1e8,
        size_1e8,
        time_in_force,
        reduce_only: false,
        post_only,
        client_order_id: Some(format!("client-{id}")),
        signed_nonce: None,
        signed_deadline_ms: None,
    }
    .into_order(order_id(id), id as i64)
}

#[test]
fn bid_sorting_high_to_low() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "a",
        Side::Buy,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    book.submit(new_order(
        2,
        "b",
        Side::Buy,
        110,
        20,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    book.submit(new_order(
        3,
        "c",
        Side::Buy,
        90,
        30,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();

    let prices: Vec<_> = book
        .snapshot()
        .bids
        .into_iter()
        .map(|level| level.price_1e8)
        .collect();
    assert_eq!(prices, vec![110, 100, 90]);
}

#[test]
fn ask_sorting_low_to_high() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "a",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    book.submit(new_order(
        2,
        "b",
        Side::Sell,
        90,
        20,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    book.submit(new_order(
        3,
        "c",
        Side::Sell,
        110,
        30,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();

    let prices: Vec<_> = book
        .snapshot()
        .asks
        .into_iter()
        .map(|level| level.price_1e8)
        .collect();
    assert_eq!(prices, vec![90, 100, 110]);
}

#[test]
fn fifo_at_same_price() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "maker-a",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    book.submit(new_order(
        2,
        "maker-b",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();

    let result = book
        .submit(new_order(
            3,
            "taker",
            Side::Buy,
            100,
            15,
            TimeInForce::Ioc,
            false,
        ))
        .unwrap();

    assert_eq!(result.trades[0].maker_order_id, order_id(1));
    assert_eq!(result.trades[0].size_1e8, 10);
    assert_eq!(result.trades[1].maker_order_id, order_id(2));
    assert_eq!(result.trades[1].size_1e8, 5);
}

#[test]
fn full_fill_removes_orders() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "maker",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    let result = book
        .submit(new_order(
            2,
            "taker",
            Side::Buy,
            100,
            10,
            TimeInForce::Gtc,
            false,
        ))
        .unwrap();

    assert_eq!(result.order.status, OrderStatus::Filled);
    assert!(book.snapshot().asks.is_empty());
    assert!(book.snapshot().bids.is_empty());
}

#[test]
fn partial_fill_updates_maker_remainder() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "maker",
        Side::Sell,
        100,
        25,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    let result = book
        .submit(new_order(
            2,
            "taker",
            Side::Buy,
            100,
            10,
            TimeInForce::Ioc,
            false,
        ))
        .unwrap();

    assert_eq!(result.trades.len(), 1);
    assert_eq!(book.snapshot().asks[0].total_size_1e8, 15);
    assert_eq!(
        book.get_order(order_id(1)).unwrap().status,
        OrderStatus::PartiallyFilled
    );
}

#[test]
fn resting_gtc_remainder() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "maker",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    let result = book
        .submit(new_order(
            2,
            "taker",
            Side::Buy,
            100,
            25,
            TimeInForce::Gtc,
            false,
        ))
        .unwrap();

    assert_eq!(result.order.status, OrderStatus::PartiallyFilled);
    assert_eq!(result.order.remaining_size_1e8, 15);
    assert_eq!(book.snapshot().bids[0].total_size_1e8, 15);
}

#[test]
fn ioc_cancels_unfilled_remainder() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "maker",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    let result = book
        .submit(new_order(
            2,
            "taker",
            Side::Buy,
            100,
            25,
            TimeInForce::Ioc,
            false,
        ))
        .unwrap();

    assert_eq!(result.order.status, OrderStatus::PartiallyFilled);
    assert_eq!(result.order.remaining_size_1e8, 0);
    assert!(book.snapshot().bids.is_empty());
}

#[test]
fn post_only_rejected_if_immediately_matchable() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "maker",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    let error = book
        .submit(new_order(
            2,
            "taker",
            Side::Buy,
            100,
            10,
            TimeInForce::Gtc,
            true,
        ))
        .unwrap_err();

    assert!(matches!(error, BackendError::PostOnlyWouldMatch));
}

#[test]
fn cancel_open_order() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "maker",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    let cancelled = book.cancel(order_id(1)).unwrap();

    assert_eq!(cancelled.status, OrderStatus::Cancelled);
    assert!(book.snapshot().asks.is_empty());
}

#[test]
fn reject_zero_price() {
    let mut book = OrderBook::new(MARKET);
    let error = book
        .submit(new_order(1, "a", Side::Buy, 0, 10, TimeInForce::Gtc, false))
        .unwrap_err();
    assert!(matches!(error, BackendError::ZeroPrice));
}

#[test]
fn reject_zero_size() {
    let mut book = OrderBook::new(MARKET);
    let error = book
        .submit(new_order(
            1,
            "a",
            Side::Buy,
            100,
            0,
            TimeInForce::Gtc,
            false,
        ))
        .unwrap_err();
    assert!(matches!(error, BackendError::ZeroSize));
}

#[test]
fn reject_self_trade_without_partial_side_effects() {
    let mut book = OrderBook::new(MARKET);
    book.submit(new_order(
        1,
        "same",
        Side::Sell,
        100,
        10,
        TimeInForce::Gtc,
        false,
    ))
    .unwrap();
    let error = book
        .submit(new_order(
            2,
            "same",
            Side::Buy,
            100,
            10,
            TimeInForce::Gtc,
            false,
        ))
        .unwrap_err();

    assert!(matches!(error, BackendError::SelfTrade));
    assert_eq!(book.snapshot().asks[0].total_size_1e8, 10);
}

#[test]
fn no_float_types_used_in_domain_model() {
    let files = [
        "src/types.rs",
        "src/orderbook/book.rs",
        "src/execution/intent.rs",
    ];
    for file in files {
        let source = fs::read_to_string(file).unwrap();
        assert!(!source.contains("f32"));
        assert!(!source.contains("f64"));
    }
}
