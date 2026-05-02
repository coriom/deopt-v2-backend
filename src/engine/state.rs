use super::{EngineCommand, EngineEvent};
use crate::error::{BackendError, Result};
use crate::execution::{ExecutionIntent, ExecutionIntentStatus, ExecutionQueue};
use crate::orderbook::{OrderBook, OrderBookSnapshot, OrderSubmitResult};
use crate::types::{now_ms, Market, MarketId, NewOrder, Order, OrderId, OrderStatus, TradeMatch};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct EngineState {
    markets: Vec<Market>,
    orderbooks: HashMap<MarketId, OrderBook>,
    execution_queue: ExecutionQueue,
}

impl EngineState {
    pub fn new(markets: Vec<Market>) -> Self {
        let orderbooks = markets
            .iter()
            .map(|market| (market.market_id, OrderBook::new(market.market_id)))
            .collect();
        Self {
            markets,
            orderbooks,
            execution_queue: ExecutionQueue::new(),
        }
    }

    pub fn default_markets() -> Vec<Market> {
        vec![
            Market {
                market_id: 1,
                symbol: "ETH-PERP".to_string(),
                kind: "perp".to_string(),
            },
            Market {
                market_id: 2,
                symbol: "BTC-PERP".to_string(),
                kind: "perp".to_string(),
            },
        ]
    }

    pub fn with_default_markets() -> Self {
        Self::new(Self::default_markets())
    }

    pub fn markets(&self) -> &[Market] {
        &self.markets
    }

    pub fn has_market(&self, market_id: MarketId) -> bool {
        self.orderbooks.contains_key(&market_id)
    }

    pub fn execution_intents(&self) -> Vec<ExecutionIntent> {
        self.execution_queue.all()
    }

    pub fn orderbook_snapshot(&self, market_id: MarketId) -> OrderBookSnapshot {
        self.orderbooks
            .get(&market_id)
            .map(OrderBook::snapshot)
            .unwrap_or_else(|| OrderBook::new(market_id).snapshot())
    }

    pub fn process(&mut self, command: EngineCommand) -> Result<Vec<EngineEvent>> {
        match command {
            EngineCommand::SubmitOrder(order) => self.submit_order(order),
            EngineCommand::CancelOrder { order_id } => self.cancel_order(order_id),
            EngineCommand::ReplaceOrder { .. } => Err(BackendError::UnsupportedCommand(
                "replace_order_scaffold".to_string(),
            )),
        }
    }

    pub fn submit_order(&mut self, order: NewOrder) -> Result<Vec<EngineEvent>> {
        let order_id = OrderId::new();
        let market_id = order.market_id;
        let created_at_ms = now_ms();
        let order = order.into_order(order_id, created_at_ms);
        let Some(book) = self.orderbooks.get_mut(&market_id) else {
            let reason = format!("unknown market: {market_id}");
            return Ok(vec![EngineEvent::OrderRejected { order_id, reason }]);
        };

        match book.submit(order) {
            Ok(result) => {
                let mut events = vec![EngineEvent::OrderAccepted {
                    order: result.order.clone(),
                }];
                self.append_fill_events(&mut events, result);
                Ok(events)
            }
            Err(error) => Ok(vec![EngineEvent::OrderRejected {
                order_id,
                reason: error.to_string(),
            }]),
        }
    }

    pub fn cancel_order(&mut self, order_id: OrderId) -> Result<Vec<EngineEvent>> {
        for book in self.orderbooks.values_mut() {
            if book.get_order(order_id).is_some() {
                let order = book.cancel(order_id)?;
                return Ok(vec![EngineEvent::OrderCancelled { order }]);
            }
        }

        Err(BackendError::OrderNotFound(order_id))
    }

    fn append_fill_events(&mut self, events: &mut Vec<EngineEvent>, result: OrderSubmitResult) {
        for maker in result.maker_order_updates {
            push_order_fill_event(events, maker);
        }

        push_order_fill_event(events, result.order.clone());

        for trade in result.trades {
            events.push(EngineEvent::TradeMatched {
                trade: trade.clone(),
            });

            let intent = intent_from_trade(&trade);
            self.execution_queue.push(intent.clone());
            events.push(EngineEvent::ExecutionIntentCreated { intent });
        }
    }
}

fn push_order_fill_event(events: &mut Vec<EngineEvent>, order: Order) {
    match order.status {
        OrderStatus::PartiallyFilled => events.push(EngineEvent::OrderPartiallyFilled { order }),
        OrderStatus::Filled => events.push(EngineEvent::OrderFilled { order }),
        _ => {}
    }
}

fn intent_from_trade(trade: &TradeMatch) -> ExecutionIntent {
    let taker_is_buyer = trade.buyer == trade.taker_account;
    let (buy_order_id, sell_order_id) = if taker_is_buyer {
        (trade.taker_order_id, trade.maker_order_id)
    } else {
        (trade.maker_order_id, trade.taker_order_id)
    };
    let buyer_is_maker = trade.buyer == trade.maker_account;
    let (buyer_nonce, seller_nonce) = if buyer_is_maker {
        (trade.maker_nonce, trade.taker_nonce)
    } else {
        (trade.taker_nonce, trade.maker_nonce)
    };
    let deadline_ms = match (trade.maker_deadline_ms, trade.taker_deadline_ms) {
        (Some(maker_deadline), Some(taker_deadline)) => Some(maker_deadline.min(taker_deadline)),
        _ => None,
    };

    ExecutionIntent {
        intent_id: Uuid::new_v4(),
        market_id: trade.market_id,
        buyer: trade.buyer.clone(),
        seller: trade.seller.clone(),
        price_1e8: trade.price_1e8,
        size_1e8: trade.size_1e8,
        buy_order_id,
        sell_order_id,
        buyer_is_maker: Some(buyer_is_maker),
        buyer_nonce,
        seller_nonce,
        deadline_ms,
        created_at_ms: trade.created_at_ms,
        status: ExecutionIntentStatus::Pending,
    }
}
