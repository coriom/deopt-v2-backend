//! PERPS-ORDER-EXECUTION-INTERNAL-V1 — thin orderbook view over the
//! Perps order store.
//!
//! Instead of a stateful book (BTreeMap of price levels), the book
//! is derived lazily from `PerpOrderStore::list_active_orders_for_market`.
//! This keeps the store's mutable state minimal and avoids the
//! double-bookkeeping trap that a separate book structure invites.
//!
//! Match ordering:
//!   * best asks sorted low-to-high price, then oldest first.
//!   * best bids sorted high-to-low price, then oldest first.

use crate::perps::order_store::PerpOrderStore;
use crate::perps::orders::{PerpOrder, PerpOrderSide};

pub fn active_asks_sorted(store: &PerpOrderStore, market_id: &str) -> Vec<PerpOrder> {
    let mut asks: Vec<PerpOrder> = store
        .list_active_orders_for_market(market_id)
        .into_iter()
        .filter(|o| o.side == PerpOrderSide::Sell)
        .collect();
    asks.sort_by(|a, b| {
        a.price_1e8
            .cmp(&b.price_1e8)
            .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
    });
    asks
}

pub fn active_bids_sorted(store: &PerpOrderStore, market_id: &str) -> Vec<PerpOrder> {
    let mut bids: Vec<PerpOrder> = store
        .list_active_orders_for_market(market_id)
        .into_iter()
        .filter(|o| o.side == PerpOrderSide::Buy)
        .collect();
    bids.sort_by(|a, b| {
        b.price_1e8
            .cmp(&a.price_1e8)
            .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
    });
    bids
}

/// Compute the opposite-side counterparties in match order for a
/// taker of `side` on `market_id`.
pub fn counterparties_for(
    store: &PerpOrderStore,
    market_id: &str,
    taker_side: PerpOrderSide,
) -> Vec<PerpOrder> {
    match taker_side {
        PerpOrderSide::Buy => active_asks_sorted(store, market_id),
        PerpOrderSide::Sell => active_bids_sorted(store, market_id),
    }
}

/// Return `true` if the taker's limit price would cross the maker's
/// resting price under standard limit-book semantics.
pub fn crosses(taker_side: PerpOrderSide, taker_price_1e8: u128, maker_price_1e8: u128) -> bool {
    match taker_side {
        PerpOrderSide::Buy => taker_price_1e8 >= maker_price_1e8,
        PerpOrderSide::Sell => taker_price_1e8 <= maker_price_1e8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perps::orders::{PerpOrder, PerpTimeInForce};
    use crate::types::{now_ms, AccountId};

    fn addr() -> AccountId {
        AccountId::new("0x0000000000000000000000000000000000000aaa".to_string())
    }

    fn order(side: PerpOrderSide, price: u128, offset_ms: i64) -> PerpOrder {
        PerpOrder::new(
            addr(),
            "ETH-PERP".to_string(),
            side,
            price,
            100_000_000,
            PerpTimeInForce::Gtc,
            false,
            false,
            30_000_000_000,
            None,
            now_ms() + offset_ms,
        )
    }

    #[test]
    fn asks_sort_low_to_high_then_oldest_first() {
        let mut store = PerpOrderStore::new();
        store
            .insert_order(order(PerpOrderSide::Sell, 310, 2))
            .unwrap();
        store
            .insert_order(order(PerpOrderSide::Sell, 300, 1))
            .unwrap();
        store
            .insert_order(order(PerpOrderSide::Sell, 300, 0))
            .unwrap();
        let asks = active_asks_sorted(&store, "ETH-PERP");
        assert_eq!(asks.len(), 3);
        assert_eq!(asks[0].price_1e8, 300);
        assert_eq!(asks[1].price_1e8, 300);
        assert!(asks[0].created_at_ms <= asks[1].created_at_ms);
        assert_eq!(asks[2].price_1e8, 310);
    }

    #[test]
    fn bids_sort_high_to_low_then_oldest_first() {
        let mut store = PerpOrderStore::new();
        store
            .insert_order(order(PerpOrderSide::Buy, 290, 2))
            .unwrap();
        store
            .insert_order(order(PerpOrderSide::Buy, 300, 1))
            .unwrap();
        store
            .insert_order(order(PerpOrderSide::Buy, 300, 0))
            .unwrap();
        let bids = active_bids_sorted(&store, "ETH-PERP");
        assert_eq!(bids[0].price_1e8, 300);
        assert_eq!(bids[1].price_1e8, 300);
        assert_eq!(bids[2].price_1e8, 290);
    }

    #[test]
    fn crosses_semantics() {
        assert!(crosses(PerpOrderSide::Buy, 300, 300));
        assert!(crosses(PerpOrderSide::Buy, 310, 300));
        assert!(!crosses(PerpOrderSide::Buy, 290, 300));

        assert!(crosses(PerpOrderSide::Sell, 300, 300));
        assert!(crosses(PerpOrderSide::Sell, 290, 300));
        assert!(!crosses(PerpOrderSide::Sell, 310, 300));
    }
}
