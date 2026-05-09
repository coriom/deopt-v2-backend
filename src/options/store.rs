use super::{
    OptionOrder, OptionOrderFilter, OptionOrderId, OptionOrderStatus, OptionSeries,
    OptionSeriesFilter, OptionSeriesId, OptionSeriesStatus,
};
use crate::error::{BackendError, Result};
use crate::types::Side;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct OptionSeriesStore {
    series: HashMap<OptionSeriesId, OptionSeries>,
    orders: HashMap<OptionOrderId, OptionOrder>,
}

impl OptionSeriesStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_series(&mut self, series: OptionSeries) -> OptionSeries {
        if let Some(existing) = self.series.get(&series.option_series_id) {
            return existing.clone();
        }
        self.series
            .insert(series.option_series_id.clone(), series.clone());
        series
    }

    pub fn list_series(&self, filter: &OptionSeriesFilter, now_sec: u64) -> Vec<OptionSeries> {
        let mut series = self
            .series
            .values()
            .filter(|series| filter.matches(series, now_sec))
            .cloned()
            .collect::<Vec<_>>();
        series.sort_by(|left, right| {
            left.expiry
                .cmp(&right.expiry)
                .then_with(|| left.strike_1e8.cmp(&right.strike_1e8))
                .then_with(|| left.option_series_id.cmp(&right.option_series_id))
        });
        series
    }

    pub fn get_series(&self, option_series_id: &str) -> Option<OptionSeries> {
        self.series.get(option_series_id).cloned()
    }

    pub fn disable_series(
        &mut self,
        option_series_id: &str,
        updated_at_ms: i64,
    ) -> Result<OptionSeries> {
        let series = self
            .series
            .get_mut(option_series_id)
            .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))?;
        series.status = OptionSeriesStatus::Disabled;
        series.updated_at_ms = updated_at_ms;
        Ok(series.clone())
    }

    pub fn insert_order(&mut self, order: OptionOrder) -> Result<OptionOrder> {
        if self.has_open_client_order(&order.account.0, order.client_order_id.as_deref()) {
            return Err(BackendError::InvalidOptionOrderState(
                "duplicate open client_order_id for option account".to_string(),
            ));
        }
        self.orders.insert(order.order_id, order.clone());
        Ok(order)
    }

    pub fn list_orders(&self, filter: &OptionOrderFilter) -> Vec<OptionOrder> {
        let mut orders = self
            .orders
            .values()
            .filter(|order| filter.matches(order))
            .cloned()
            .collect::<Vec<_>>();
        orders.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.order_id.cmp(&right.order_id))
        });
        orders
    }

    pub fn get_order(&self, order_id: OptionOrderId) -> Option<OptionOrder> {
        self.orders.get(&order_id).cloned()
    }

    pub fn cancel_order(
        &mut self,
        order_id: OptionOrderId,
        updated_at_ms: i64,
    ) -> Result<OptionOrder> {
        let order = self
            .orders
            .get_mut(&order_id)
            .ok_or(BackendError::InvalidOptionOrderId)?;
        if order.status != OptionOrderStatus::Open {
            return Err(BackendError::InvalidOptionOrderState(format!(
                "option order is {}",
                order.status.as_str()
            )));
        }
        order.status = OptionOrderStatus::Cancelled;
        order.updated_at_ms = updated_at_ms;
        Ok(order.clone())
    }

    pub fn open_orders_for_series(&self, option_series_id: &str) -> Vec<OptionOrder> {
        self.orders
            .values()
            .filter(|order| {
                order.option_series_id == option_series_id
                    && order.status == OptionOrderStatus::Open
                    && (order.side == Side::Buy || order.side == Side::Sell)
            })
            .cloned()
            .collect()
    }

    fn has_open_client_order(&self, account: &str, client_order_id: Option<&str>) -> bool {
        let Some(client_order_id) = client_order_id else {
            return false;
        };
        self.orders.values().any(|order| {
            order.status == OptionOrderStatus::Open
                && order.account.0.eq_ignore_ascii_case(account)
                && order.client_order_id.as_deref() == Some(client_order_id)
        })
    }
}
