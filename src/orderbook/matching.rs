use crate::types::{Price1e8, Side};

pub fn prices_cross(taker_side: Side, maker_price: Price1e8, taker_price: Price1e8) -> bool {
    match taker_side {
        Side::Buy => maker_price <= taker_price,
        Side::Sell => maker_price >= taker_price,
    }
}
