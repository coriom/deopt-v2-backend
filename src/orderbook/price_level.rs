use crate::types::{OrderId, Size1e8};
use std::collections::VecDeque;

#[derive(Clone, Debug, Default)]
pub struct PriceLevel {
    order_ids: VecDeque<OrderId>,
}

impl PriceLevel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_back(&mut self, order_id: OrderId) {
        self.order_ids.push_back(order_id);
    }

    pub fn front(&self) -> Option<OrderId> {
        self.order_ids.front().copied()
    }

    pub fn pop_front(&mut self) -> Option<OrderId> {
        self.order_ids.pop_front()
    }

    pub fn remove(&mut self, order_id: OrderId) -> bool {
        let Some(index) = self.order_ids.iter().position(|id| *id == order_id) else {
            return false;
        };
        self.order_ids.remove(index).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.order_ids.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = OrderId> + '_ {
        self.order_ids.iter().copied()
    }

    pub fn total_size_1e8<F>(&self, mut remaining_size: F) -> Size1e8
    where
        F: FnMut(OrderId) -> Size1e8,
    {
        self.order_ids
            .iter()
            .copied()
            .map(&mut remaining_size)
            .sum()
    }
}
