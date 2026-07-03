//! PERPS-ORDER-EXECUTION-INTERNAL-V1 — in-memory store for Perps
//! orders + fills. Mirrors the shape of `PerpPositionsStore` from the
//! position engine milestone: a HashMap for random access + a
//! per-account/market index for hot-path listing.

use crate::error::{BackendError, Result};
use crate::perps::orders::{PerpFill, PerpOrder};
use crate::types::{AccountId, TimestampMs};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct PerpOrderStore {
    orders: HashMap<Uuid, PerpOrder>,
    fills: Vec<PerpFill>,
    /// (lower(account), client_order_id) → order id, for idempotency.
    by_client_order_id: HashMap<(String, String), Uuid>,
}

impl PerpOrderStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh order. Rejects a re-use of the same client
    /// order id from the same account.
    pub fn insert_order(&mut self, order: PerpOrder) -> Result<PerpOrder> {
        if let Some(client_id) = &order.client_order_id {
            let key = (order.account.0.to_lowercase(), client_id.clone());
            if self.by_client_order_id.contains_key(&key) {
                return Err(BackendError::PerpDuplicateClientOrderId(client_id.clone()));
            }
            self.by_client_order_id.insert(key, order.id);
        }
        self.orders.insert(order.id, order.clone());
        Ok(order)
    }

    pub fn get(&self, id: Uuid) -> Option<PerpOrder> {
        self.orders.get(&id).cloned()
    }

    pub fn update<F>(&mut self, id: Uuid, now: TimestampMs, mutator: F) -> Result<PerpOrder>
    where
        F: FnOnce(&mut PerpOrder) -> Result<()>,
    {
        let order = self
            .orders
            .get_mut(&id)
            .ok_or_else(|| BackendError::PerpOrderNotFound(id.to_string()))?;
        mutator(order)?;
        order.updated_at_ms = now;
        Ok(order.clone())
    }

    pub fn insert_fill(&mut self, fill: PerpFill) {
        self.fills.push(fill);
    }

    pub fn list_orders_for_account(&self, account: &AccountId) -> Vec<PerpOrder> {
        let want = account.0.to_lowercase();
        let mut rows: Vec<PerpOrder> = self
            .orders
            .values()
            .filter(|o| o.account.0.to_lowercase() == want)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows
    }

    pub fn list_fills_for_account(&self, account: &AccountId) -> Vec<PerpFill> {
        let want = account.0.to_lowercase();
        let mut rows: Vec<PerpFill> = self
            .fills
            .iter()
            .filter(|f| {
                f.taker_account.0.to_lowercase() == want || f.maker_account.0.to_lowercase() == want
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows
    }

    pub fn list_active_orders_for_market(&self, market_id: &str) -> Vec<PerpOrder> {
        self.orders
            .values()
            .filter(|o| o.market_id == market_id && o.status.is_active())
            .cloned()
            .collect()
    }

    /// Total resting bid + ask counts for one market — used by tests.
    pub fn active_order_count(&self, market_id: &str) -> usize {
        self.list_active_orders_for_market(market_id).len()
    }

    /// PERPS-LIQUIDATION-AND-RISK-V1 — cancel every open order
    /// belonging to `(account, market_id)` in one pass. Returns the
    /// cancelled rows so the caller can emit lifecycle frames for
    /// each. Idempotent: if the account has no open orders on that
    /// market, returns an empty vec.
    pub fn cancel_open_orders_for_account_market(
        &mut self,
        account: &AccountId,
        market_id: &str,
        reason_code: &str,
        reason_source: &str,
        now: TimestampMs,
    ) -> Vec<PerpOrder> {
        let want = account.0.to_lowercase();
        let ids: Vec<uuid::Uuid> = self
            .orders
            .values()
            .filter(|o| {
                o.market_id == market_id
                    && o.account.0.to_lowercase() == want
                    && o.status.is_active()
            })
            .map(|o| o.id)
            .collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(order) = self.orders.get_mut(&id) {
                order.status = crate::perps::orders::PerpOrderStatus::Cancelled;
                order.terminal_reason_code = Some(reason_code.to_string());
                order.terminal_reason_source = Some(reason_source.to_string());
                order.updated_at_ms = now;
                out.push(order.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perps::orders::{PerpOrderSide, PerpOrderStatus, PerpTimeInForce};
    use crate::types::now_ms;

    fn addr(hex: &str) -> AccountId {
        AccountId::new(hex.to_string())
    }

    fn sample_order(client_id: Option<&str>) -> PerpOrder {
        PerpOrder::new(
            addr("0x0000000000000000000000000000000000000aaa"),
            "ETH-PERP".to_string(),
            PerpOrderSide::Buy,
            300_000_000_000,
            100_000_000,
            PerpTimeInForce::Gtc,
            false,
            false,
            30_000_000_000,
            client_id.map(|s| s.to_string()),
            now_ms(),
        )
    }

    #[test]
    fn insert_and_get_round_trip() {
        let mut store = PerpOrderStore::new();
        let order = sample_order(None);
        let inserted = store.insert_order(order.clone()).unwrap();
        assert_eq!(store.get(inserted.id).unwrap(), inserted);
    }

    #[test]
    fn duplicate_client_order_id_rejected() {
        let mut store = PerpOrderStore::new();
        store.insert_order(sample_order(Some("cli-1"))).unwrap();
        let err = store.insert_order(sample_order(Some("cli-1"))).unwrap_err();
        assert!(matches!(err, BackendError::PerpDuplicateClientOrderId(_)));
    }

    #[test]
    fn list_active_orders_filters_terminal() {
        let mut store = PerpOrderStore::new();
        let order = sample_order(None);
        let id = order.id;
        store.insert_order(order).unwrap();
        assert_eq!(store.active_order_count("ETH-PERP"), 1);
        store
            .update(id, now_ms(), |o| {
                o.status = PerpOrderStatus::Cancelled;
                Ok(())
            })
            .unwrap();
        assert_eq!(store.active_order_count("ETH-PERP"), 0);
    }
}
