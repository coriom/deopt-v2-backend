//! Rebuild a wallet's open contract counts from canonical indexed receipts.
//! Economic fields are intentionally absent: these events alone cannot prove
//! per-wallet fees, collateral, or current mark-to-market value.
use super::OptionExecutionEvent;
use alloy_primitives::U256;
use std::collections::BTreeMap;

pub fn open_contracts(
    events: &[OptionExecutionEvent],
    wallet: &str,
) -> Result<BTreeMap<U256, i128>, &'static str> {
    let mut positions: BTreeMap<U256, i128> = BTreeMap::new();
    for event in events {
        if event.block_hash.is_none() {
            return Err("indexed receipt lacks block hash");
        }
        let id = event
            .option_id
            .as_deref()
            .ok_or("Options event lacks series ID")?
            .parse::<U256>()
            .map_err(|_| "invalid Options series ID")?;
        match event.event_name.as_str() {
            "OptionTradeExecuted" | "OptionRfqTradeExecuted" => {
                let buyer = event.buyer.as_deref().ok_or("trade lacks buyer")?;
                let seller = event.seller.as_deref().ok_or("trade lacks seller")?;
                if buyer.eq_ignore_ascii_case(seller) {
                    return Err("self trade cannot be projected");
                }
                let quantity = event
                    .quantity_contracts
                    .as_deref()
                    .ok_or("trade lacks quantity")?
                    .parse::<i128>()
                    .map_err(|_| "invalid trade quantity")?;
                if quantity <= 0 {
                    return Err("nonpositive trade quantity");
                }
                let signed = if buyer.eq_ignore_ascii_case(wallet) {
                    quantity
                } else if seller.eq_ignore_ascii_case(wallet) {
                    -quantity
                } else {
                    return Err("trade is not owned by wallet");
                };
                let current = positions.get(&id).copied().unwrap_or_default();
                positions.insert(id, current.checked_add(signed).ok_or("position overflow")?);
            }
            "AccountSettled" => {
                if !event
                    .account
                    .as_deref()
                    .is_some_and(|account| account.eq_ignore_ascii_case(wallet))
                {
                    return Err("settlement is not owned by wallet");
                }
                positions.insert(id, 0);
            }
            _ => return Err("unexpected Options position event"),
        }
    }
    positions.retain(|_, quantity| *quantity != 0);
    Ok(positions)
}
