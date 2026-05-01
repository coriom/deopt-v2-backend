use super::matching::prices_cross;
use super::price_level::PriceLevel;
use crate::error::{BackendError, Result};
use crate::types::{
    now_ms, MarketId, Order, OrderId, OrderStatus, Price1e8, Side, Size1e8, TimeInForce, TradeMatch,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OrderLocation {
    side: Side,
    price_1e8: Price1e8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderSubmitResult {
    pub order: Order,
    pub trades: Vec<TradeMatch>,
    pub maker_order_updates: Vec<Order>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BookSideSnapshot {
    pub price_1e8: Price1e8,
    pub total_size_1e8: Size1e8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OrderBookSnapshot {
    pub market_id: MarketId,
    pub bids: Vec<BookSideSnapshot>,
    pub asks: Vec<BookSideSnapshot>,
}

#[derive(Clone, Debug)]
pub struct OrderBook {
    market_id: MarketId,
    bids: BTreeMap<Price1e8, PriceLevel>,
    asks: BTreeMap<Price1e8, PriceLevel>,
    locations: HashMap<OrderId, OrderLocation>,
    orders: HashMap<OrderId, Order>,
}

impl OrderBook {
    pub fn new(market_id: MarketId) -> Self {
        Self {
            market_id,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            locations: HashMap::new(),
            orders: HashMap::new(),
        }
    }

    pub fn market_id(&self) -> MarketId {
        self.market_id
    }

    pub fn submit(&mut self, mut order: Order) -> Result<OrderSubmitResult> {
        validate_order(&order)?;

        if order.time_in_force == TimeInForce::Fok {
            order.status = OrderStatus::Rejected;
            self.orders.insert(order.order_id, order.clone());
            return Err(BackendError::UnsupportedTimeInForce("fok".to_string()));
        }

        if order.post_only && self.would_match(&order) {
            order.status = OrderStatus::Rejected;
            self.orders.insert(order.order_id, order.clone());
            return Err(BackendError::PostOnlyWouldMatch);
        }

        if self.would_self_trade(&order) {
            order.status = OrderStatus::Rejected;
            self.orders.insert(order.order_id, order.clone());
            return Err(BackendError::SelfTrade);
        }

        let mut taker = order;
        taker.status = OrderStatus::Open;
        self.orders.insert(taker.order_id, taker.clone());

        let mut trades = Vec::new();
        let mut maker_order_updates = Vec::new();

        while taker.remaining_size_1e8 > 0 {
            let Some(maker_order_id) = self.best_match_order_id(taker.side, taker.price_1e8) else {
                break;
            };

            let maker = self
                .orders
                .get(&maker_order_id)
                .cloned()
                .ok_or(BackendError::OrderNotFound(maker_order_id))?;

            let fill_size = taker.remaining_size_1e8.min(maker.remaining_size_1e8);
            let created_at_ms = now_ms();
            let (buyer, seller) = match taker.side {
                Side::Buy => (taker.account.clone(), maker.account.clone()),
                Side::Sell => (maker.account.clone(), taker.account.clone()),
            };

            trades.push(TradeMatch {
                market_id: self.market_id,
                maker_order_id: maker.order_id,
                taker_order_id: taker.order_id,
                maker_account: maker.account.clone(),
                taker_account: taker.account.clone(),
                price_1e8: maker.price_1e8,
                size_1e8: fill_size,
                buyer,
                seller,
                created_at_ms,
            });

            taker.remaining_size_1e8 -= fill_size;
            let maker_after = self.apply_maker_fill(maker_order_id, fill_size)?;
            maker_order_updates.push(maker_after);
        }

        taker.status = if taker.remaining_size_1e8 == 0 {
            OrderStatus::Filled
        } else if taker.remaining_size_1e8 < taker.size_1e8 {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::Open
        };

        if taker.remaining_size_1e8 > 0 {
            match taker.time_in_force {
                TimeInForce::Gtc => {
                    self.rest_order(&taker);
                }
                TimeInForce::Ioc => {
                    taker.remaining_size_1e8 = 0;
                    taker.status = if trades.is_empty() {
                        OrderStatus::Cancelled
                    } else {
                        OrderStatus::PartiallyFilled
                    };
                }
                TimeInForce::Fok => unreachable!("FOK rejected before matching"),
            }
        }

        self.orders.insert(taker.order_id, taker.clone());

        Ok(OrderSubmitResult {
            order: taker,
            trades,
            maker_order_updates,
        })
    }

    pub fn cancel(&mut self, order_id: OrderId) -> Result<Order> {
        let location = self
            .locations
            .remove(&order_id)
            .ok_or(BackendError::OrderNotOpen(order_id))?;

        let level = self.level_mut(location.side, location.price_1e8);
        if let Some(level) = level {
            level.remove(order_id);
        }
        self.remove_empty_level(location.side, location.price_1e8);

        let order = self
            .orders
            .get_mut(&order_id)
            .ok_or(BackendError::OrderNotFound(order_id))?;
        order.status = OrderStatus::Cancelled;
        order.remaining_size_1e8 = 0;
        Ok(order.clone())
    }

    pub fn get_order(&self, order_id: OrderId) -> Option<&Order> {
        self.orders.get(&order_id)
    }

    pub fn snapshot(&self) -> OrderBookSnapshot {
        let bids = self
            .bids
            .iter()
            .rev()
            .filter_map(|(price_1e8, level)| self.snapshot_level(*price_1e8, level))
            .collect();
        let asks = self
            .asks
            .iter()
            .filter_map(|(price_1e8, level)| self.snapshot_level(*price_1e8, level))
            .collect();

        OrderBookSnapshot {
            market_id: self.market_id,
            bids,
            asks,
        }
    }

    fn snapshot_level(&self, price_1e8: Price1e8, level: &PriceLevel) -> Option<BookSideSnapshot> {
        let total_size_1e8 = level.total_size_1e8(|order_id| {
            self.orders
                .get(&order_id)
                .map(|order| order.remaining_size_1e8)
                .unwrap_or(0)
        });

        (total_size_1e8 > 0).then_some(BookSideSnapshot {
            price_1e8,
            total_size_1e8,
        })
    }

    fn would_match(&self, order: &Order) -> bool {
        let Some(best_price) = self.best_opposite_price(order.side) else {
            return false;
        };
        prices_cross(order.side, best_price, order.price_1e8)
    }

    fn would_self_trade(&self, order: &Order) -> bool {
        let levels = match order.side {
            Side::Buy => &self.asks,
            Side::Sell => &self.bids,
        };

        levels
            .iter()
            .filter(|(price, _)| prices_cross(order.side, **price, order.price_1e8))
            .flat_map(|(_, level)| level.iter())
            .any(|order_id| {
                self.orders
                    .get(&order_id)
                    .map(|maker| maker.account == order.account)
                    .unwrap_or(false)
            })
    }

    fn best_match_order_id(&self, taker_side: Side, taker_price: Price1e8) -> Option<OrderId> {
        let best_price = self.best_opposite_price(taker_side)?;
        if !prices_cross(taker_side, best_price, taker_price) {
            return None;
        }

        self.level(taker_side.opposite(), best_price)?.front()
    }

    fn best_opposite_price(&self, taker_side: Side) -> Option<Price1e8> {
        match taker_side {
            Side::Buy => self.asks.keys().next().copied(),
            Side::Sell => self.bids.keys().next_back().copied(),
        }
    }

    fn apply_maker_fill(&mut self, order_id: OrderId, fill_size: Size1e8) -> Result<Order> {
        let (side, price, filled) = {
            let maker = self
                .orders
                .get_mut(&order_id)
                .ok_or(BackendError::OrderNotFound(order_id))?;
            maker.remaining_size_1e8 -= fill_size;
            maker.status = if maker.remaining_size_1e8 == 0 {
                OrderStatus::Filled
            } else {
                OrderStatus::PartiallyFilled
            };
            (maker.side, maker.price_1e8, maker.remaining_size_1e8 == 0)
        };

        if filled {
            if let Some(level) = self.level_mut(side, price) {
                level.pop_front();
            }
            self.locations.remove(&order_id);
            self.remove_empty_level(side, price);
        }

        self.orders
            .get(&order_id)
            .cloned()
            .ok_or(BackendError::OrderNotFound(order_id))
    }

    fn rest_order(&mut self, order: &Order) {
        let level = match order.side {
            Side::Buy => self.bids.entry(order.price_1e8).or_default(),
            Side::Sell => self.asks.entry(order.price_1e8).or_default(),
        };
        level.push_back(order.order_id);
        self.locations.insert(
            order.order_id,
            OrderLocation {
                side: order.side,
                price_1e8: order.price_1e8,
            },
        );
    }

    fn level(&self, side: Side, price_1e8: Price1e8) -> Option<&PriceLevel> {
        match side {
            Side::Buy => self.bids.get(&price_1e8),
            Side::Sell => self.asks.get(&price_1e8),
        }
    }

    fn level_mut(&mut self, side: Side, price_1e8: Price1e8) -> Option<&mut PriceLevel> {
        match side {
            Side::Buy => self.bids.get_mut(&price_1e8),
            Side::Sell => self.asks.get_mut(&price_1e8),
        }
    }

    fn remove_empty_level(&mut self, side: Side, price_1e8: Price1e8) {
        let should_remove = self
            .level(side, price_1e8)
            .map(PriceLevel::is_empty)
            .unwrap_or(false);
        if should_remove {
            match side {
                Side::Buy => {
                    self.bids.remove(&price_1e8);
                }
                Side::Sell => {
                    self.asks.remove(&price_1e8);
                }
            }
        }
    }
}

fn validate_order(order: &Order) -> Result<()> {
    if order.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if order.size_1e8 == 0 || order.remaining_size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    Ok(())
}
