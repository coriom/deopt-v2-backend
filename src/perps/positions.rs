//! PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1 — persistent Perps position
//! model + in-memory store.
//!
//! Scale conventions (all `1e8`):
//!   * `size_1e8` — base-asset contract size.
//!   * `entry_price_1e8` — weighted-average entry, quote per base.
//!   * `margin_1e8` — isolated collateral locked (mUSDC normalised to
//!     1e8 for internal math consistency; the 6-decimal on-chain
//!     encoding is a boundary concern, not a storage concern).
//!   * `realized_pnl_1e8` — realised PnL accumulated over the
//!     position's lifetime.

use crate::error::{BackendError, Result};
use crate::types::{now_ms, AccountId, TimestampMs};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpSide {
    Long,
    Short,
}

impl PerpSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "long" => Ok(Self::Long),
            "short" => Ok(Self::Short),
            other => Err(BackendError::Persistence(format!(
                "invalid perp side: {other}"
            ))),
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Self::Long => Self::Short,
            Self::Short => Self::Long,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpPositionStatus {
    Open,
    Closed,
    // PERPS-LIQUIDATION-AND-RISK-V1
    Liquidated,
}

impl PerpPositionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Liquidated => "liquidated",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "liquidated" => Ok(Self::Liquidated),
            other => Err(BackendError::Persistence(format!(
                "invalid perp position status: {other}"
            ))),
        }
    }
}

/// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — helper for `#[serde(default)]`
/// on optional subaccount fields. Legacy wire payloads (pre-milestone)
/// omit the field; default `1` maps to the wallet's Account 1.
pub fn one_subaccount_id() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpPosition {
    pub id: Uuid,
    pub account: AccountId,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — the subaccount this
    /// position belongs to. `1` is the default account. Positions are
    /// isolated across subaccounts: two positions with the same
    /// `(account, market_id)` but different `subaccount_id` do not net.
    #[serde(default = "one_subaccount_id")]
    pub subaccount_id: u32,
    pub market_id: String,
    pub side: PerpSide,
    /// Remaining position size, `1e8`.
    pub size_1e8: u128,
    /// Weighted-average entry price, `1e8`.
    pub entry_price_1e8: u128,
    /// Isolated collateral locked, `1e8`. Adjusted by funding settlements
    /// in `PERPS-FUNDING-V1` (funding payments reduce/increase margin;
    /// saturating to 0 with the residual booked as bad debt on the
    /// funding event).
    pub margin_1e8: u128,
    /// Cumulative realised PnL over the position's lifetime, `1e8`.
    /// Signed — negative values mean the trader has taken losses.
    pub realized_pnl_1e8: i128,
    pub status: PerpPositionStatus,
    pub opened_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
    pub closed_at_ms: Option<TimestampMs>,
    pub version: u64,
    /// PERPS-FUNDING-V1 — the signed `cumulativeFundingRate1e18` this
    /// position most recently settled against. Zero by default (a
    /// freshly-opened position settles from index 0 upward). Serialised
    /// as a signed 128-bit integer.
    pub last_funding_index_1e18: i128,
    /// PERPS-FUNDING-V1 — signed sum of every funding payment applied
    /// to this position over its lifetime. Positive means the trader
    /// has paid net funding; negative means the trader has received
    /// net funding. `1e8`-scaled.
    pub cumulative_funding_payment_1e8: i128,
}

impl PerpPosition {
    pub fn is_open(&self) -> bool {
        self.status == PerpPositionStatus::Open
    }

    pub fn notional_1e8(&self) -> u128 {
        // (size * price) / 1e8 — both are 1e8-scaled, so the product
        // needs one 1e8 division to stay in the quote 1e8 scale.
        (self.size_1e8 as u128).saturating_mul(self.entry_price_1e8 as u128) / 100_000_000
    }
}

/// In-memory Perps position store. Mirrors the shape of
/// `OptionSeriesStore` sub-tables: HashMap keyed by row id + a
/// per-account/market index for the hot path.
#[derive(Clone, Debug, Default)]
pub struct PerpPositionsStore {
    /// All positions by id (open + closed).
    by_id: HashMap<Uuid, PerpPosition>,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — subaccount-aware active
    /// index: (lower-cased account, subaccount_id, market_id) →
    /// open position id. Two positions on the same account+market but
    /// different subaccounts coexist here without netting.
    active_by_account_subaccount_market: HashMap<(String, u32, String), Uuid>,
}

impl PerpPositionsStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh open position. Fails when an open position
    /// already exists for the same `(account, subaccount_id, market_id)`
    /// — the caller is expected to reduce/close the existing one first.
    /// Cross-subaccount coexistence is allowed: Account 1 and Account
    /// 2 can each hold one open ETH-PERP position on the same wallet.
    pub fn insert_open(&mut self, position: PerpPosition) -> Result<PerpPosition> {
        if position.status != PerpPositionStatus::Open {
            return Err(BackendError::Config(
                "PerpPositionsStore::insert_open requires status='open'".to_string(),
            ));
        }
        let key = index_key(
            &position.account,
            position.subaccount_id,
            &position.market_id,
        );
        if self.active_by_account_subaccount_market.contains_key(&key) {
            return Err(BackendError::PerpMarketPaused(format!(
                "account+subaccount already has an open position for market {}",
                position.market_id
            )));
        }
        self.active_by_account_subaccount_market
            .insert(key, position.id);
        self.by_id.insert(position.id, position.clone());
        Ok(position)
    }

    pub fn get_active(
        &self,
        account: &AccountId,
        subaccount_id: u32,
        market_id: &str,
    ) -> Option<PerpPosition> {
        let key = index_key(account, subaccount_id, market_id);
        let id = self.active_by_account_subaccount_market.get(&key)?;
        self.by_id.get(id).cloned()
    }

    /// Apply an in-place mutation. Returns the post-mutation row.
    /// Bumps `version` + `updated_at_ms`. Scoped to
    /// `(account, subaccount_id, market_id)` — cross-subaccount reads
    /// on the same wallet + market do NOT match.
    pub fn update_active<F>(
        &mut self,
        account: &AccountId,
        subaccount_id: u32,
        market_id: &str,
        now: TimestampMs,
        mutator: F,
    ) -> Result<PerpPosition>
    where
        F: FnOnce(&mut PerpPosition) -> Result<()>,
    {
        let key = index_key(account, subaccount_id, market_id);
        let id = *self
            .active_by_account_subaccount_market
            .get(&key)
            .ok_or(BackendError::PerpPositionNotFound)?;
        let position = self.by_id.get_mut(&id).ok_or_else(|| {
            BackendError::Persistence(format!("perp position {id} indexed but absent"))
        })?;
        mutator(position)?;
        position.updated_at_ms = now;
        position.version = position.version.saturating_add(1);
        Ok(position.clone())
    }

    /// PERPS-LIQUIDATION-AND-RISK-V1 — flip the active position to
    /// `Liquidated` status. Same shape as `close_active` but with a
    /// distinct terminal status so history / lifecycle can
    /// distinguish trader-cancel vs risk-driven liquidation. Scoped
    /// to the exact `(account, subaccount_id, market_id)` key so a
    /// liquidation tick on Account 2 does not touch Account 1.
    pub fn liquidate_active(
        &mut self,
        account: &AccountId,
        subaccount_id: u32,
        market_id: &str,
        realized_pnl_delta_1e8: i128,
        now: TimestampMs,
    ) -> Result<PerpPosition> {
        let key = index_key(account, subaccount_id, market_id);
        let id = self
            .active_by_account_subaccount_market
            .remove(&key)
            .ok_or(BackendError::PerpPositionNotFound)?;
        let position = self.by_id.get_mut(&id).ok_or_else(|| {
            BackendError::Persistence(format!("perp position {id} indexed but absent"))
        })?;
        position.status = PerpPositionStatus::Liquidated;
        position.realized_pnl_1e8 = position
            .realized_pnl_1e8
            .saturating_add(realized_pnl_delta_1e8);
        position.updated_at_ms = now;
        position.closed_at_ms = Some(now);
        position.version = position.version.saturating_add(1);
        Ok(position.clone())
    }

    /// Close the active position for
    /// `(account, subaccount_id, market_id)`. Cross-subaccount rows on
    /// the same wallet + market are not affected.
    pub fn close_active(
        &mut self,
        account: &AccountId,
        subaccount_id: u32,
        market_id: &str,
        now: TimestampMs,
    ) -> Result<PerpPosition> {
        let key = index_key(account, subaccount_id, market_id);
        let id = self
            .active_by_account_subaccount_market
            .remove(&key)
            .ok_or(BackendError::PerpPositionNotFound)?;
        let position = self.by_id.get_mut(&id).ok_or_else(|| {
            BackendError::Persistence(format!("perp position {id} indexed but absent"))
        })?;
        position.status = PerpPositionStatus::Closed;
        position.updated_at_ms = now;
        position.closed_at_ms = Some(now);
        position.version = position.version.saturating_add(1);
        Ok(position.clone())
    }

    /// Direct mutable access to a stored row by id. Exposed for the
    /// fill-application helper that needs to patch a closed row's
    /// realised PnL after `close_active` has already run.
    pub fn by_id_mut(&mut self, id: Uuid) -> Option<&mut PerpPosition> {
        self.by_id.get_mut(&id)
    }

    /// Direct read of a stored row by id, whether active or closed.
    pub fn by_id(&self, id: Uuid) -> Option<PerpPosition> {
        self.by_id.get(&id).cloned()
    }

    /// PERPS-LIQUIDATION-AND-RISK-V1 — snapshot every row (open +
    /// closed + liquidated) for the tick's read-only scan.
    pub fn by_id_values_iter(&self) -> Vec<PerpPosition> {
        self.by_id.values().cloned().collect()
    }

    /// List positions for one wallet across every subaccount, newest-
    /// first. Callers who want subaccount-scoped views should use
    /// `list_for_account_and_subaccount`. This method is preserved for
    /// the `?all=true` read path.
    pub fn list_for_account(&self, account: &AccountId) -> Vec<PerpPosition> {
        let want = account.0.to_lowercase();
        let mut rows: Vec<PerpPosition> = self
            .by_id
            .values()
            .filter(|p| p.account.0.to_lowercase() == want)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
        rows
    }

    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — subaccount-scoped list.
    /// Default read path (no `?subaccount_id=`) uses `subaccount_id=1`.
    pub fn list_for_account_and_subaccount(
        &self,
        account: &AccountId,
        subaccount_id: u32,
    ) -> Vec<PerpPosition> {
        let want = account.0.to_lowercase();
        let mut rows: Vec<PerpPosition> = self
            .by_id
            .values()
            .filter(|p| p.account.0.to_lowercase() == want && p.subaccount_id == subaccount_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
        rows
    }
}

fn index_key(account: &AccountId, subaccount_id: u32, market_id: &str) -> (String, u32, String) {
    (
        account.0.to_lowercase(),
        subaccount_id,
        market_id.to_string(),
    )
}

/// Build a fresh position skeleton — the actual `size`, `entry`, and
/// `margin` come from the fill-application logic.
pub fn new_position_skeleton(
    account: AccountId,
    subaccount_id: u32,
    market_id: String,
    side: PerpSide,
    size_1e8: u128,
    entry_price_1e8: u128,
    margin_1e8: u128,
) -> PerpPosition {
    let now = now_ms();
    PerpPosition {
        id: Uuid::new_v4(),
        account,
        subaccount_id,
        market_id,
        side,
        size_1e8,
        entry_price_1e8,
        margin_1e8,
        realized_pnl_1e8: 0,
        status: PerpPositionStatus::Open,
        opened_at_ms: now,
        updated_at_ms: now,
        closed_at_ms: None,
        version: 1,
        last_funding_index_1e18: 0,
        cumulative_funding_payment_1e8: 0,
    }
}

/// PERPS-FUNDING-V1 — apply a signed funding payment to `margin_1e8`
/// with saturating semantics.
///
/// Positive payment → the trader owes funding; margin is reduced.
///   * If payment > margin, margin saturates to 0 and the excess is
///     returned as `bad_debt_1e8`.
///
/// Negative payment → the trader receives funding; margin is increased.
///   * No bad debt (returns 0).
///
/// Returns `(new_margin_1e8, bad_debt_1e8)`.
pub fn apply_funding_payment_to_margin(margin_1e8: u128, payment_1e8: i128) -> (u128, u128) {
    if payment_1e8 == 0 {
        return (margin_1e8, 0);
    }
    if payment_1e8 > 0 {
        let payment_abs = payment_1e8 as u128;
        if payment_abs > margin_1e8 {
            (0, payment_abs - margin_1e8)
        } else {
            (margin_1e8 - payment_abs, 0)
        }
    } else {
        let credit = (-payment_1e8) as u128;
        (margin_1e8.saturating_add(credit), 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(hex: &str) -> AccountId {
        AccountId::new(hex.to_string())
    }

    #[test]
    fn insert_open_indexes_position() {
        let mut store = PerpPositionsStore::new();
        let account = addr("0x0000000000000000000000000000000000000aaa");
        let position = new_position_skeleton(
            account.clone(),
            1,
            "ETH-PERP".to_string(),
            PerpSide::Long,
            100_000_000,
            300_000_000_000,
            30_000_000_000,
        );
        let inserted = store.insert_open(position.clone()).unwrap();
        assert_eq!(inserted.id, position.id);
        let hit = store.get_active(&account, 1, "ETH-PERP").unwrap();
        assert_eq!(hit.id, position.id);
    }

    #[test]
    fn duplicate_open_position_rejected() {
        let mut store = PerpPositionsStore::new();
        let account = addr("0x0000000000000000000000000000000000000aaa");
        let p1 = new_position_skeleton(
            account.clone(),
            1,
            "ETH-PERP".to_string(),
            PerpSide::Long,
            100_000_000,
            300_000_000_000,
            30_000_000_000,
        );
        store.insert_open(p1).unwrap();
        let p2 = new_position_skeleton(
            account,
            1,
            "ETH-PERP".to_string(),
            PerpSide::Long,
            100_000_000,
            300_000_000_000,
            30_000_000_000,
        );
        let err = store.insert_open(p2).unwrap_err();
        assert!(matches!(err, BackendError::PerpMarketPaused(_)));
    }

    #[test]
    fn close_active_moves_to_history_and_frees_index() {
        let mut store = PerpPositionsStore::new();
        let account = addr("0x0000000000000000000000000000000000000aaa");
        let p1 = new_position_skeleton(
            account.clone(),
            1,
            "ETH-PERP".to_string(),
            PerpSide::Long,
            100_000_000,
            300_000_000_000,
            30_000_000_000,
        );
        store.insert_open(p1).unwrap();
        let closed = store
            .close_active(&account, 1, "ETH-PERP", 1_782_000_000_000)
            .unwrap();
        assert_eq!(closed.status, PerpPositionStatus::Closed);
        assert!(store.get_active(&account, 1, "ETH-PERP").is_none());
        // Now re-opening on the same account+market should succeed.
        let p2 = new_position_skeleton(
            account.clone(),
            1,
            "ETH-PERP".to_string(),
            PerpSide::Short,
            50_000_000,
            310_000_000_000,
            15_500_000_000,
        );
        store.insert_open(p2).unwrap();
        assert_eq!(
            store.get_active(&account, 1, "ETH-PERP").unwrap().side,
            PerpSide::Short
        );
    }

    #[test]
    fn list_for_account_orders_newest_first_and_filters_case_insensitively() {
        let mut store = PerpPositionsStore::new();
        let account_lower = addr("0x0000000000000000000000000000000000000aaa");
        let account_upper = addr("0x0000000000000000000000000000000000000AAA");
        let mut p1 = new_position_skeleton(
            account_lower.clone(),
            1,
            "ETH-PERP".to_string(),
            PerpSide::Long,
            100_000_000,
            300_000_000_000,
            30_000_000_000,
        );
        p1.updated_at_ms = 1_000;
        let mut p2 = new_position_skeleton(
            account_upper.clone(),
            1,
            "BTC-PERP".to_string(),
            PerpSide::Short,
            50_000_000,
            6_500_000_000_000,
            10_000_000_000,
        );
        p2.updated_at_ms = 2_000;
        store.insert_open(p1).unwrap();
        store.insert_open(p2).unwrap();
        let list = store.list_for_account(&account_lower);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].market_id, "BTC-PERP"); // newest first
    }

    // PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — cross-subaccount isolation.
    #[test]
    fn same_wallet_same_market_can_coexist_across_subaccounts() {
        let mut store = PerpPositionsStore::new();
        let account = addr("0x0000000000000000000000000000000000000aaa");
        let p1 = new_position_skeleton(
            account.clone(),
            1,
            "ETH-PERP".to_string(),
            PerpSide::Long,
            100_000_000,
            300_000_000_000,
            30_000_000_000,
        );
        let p2 = new_position_skeleton(
            account.clone(),
            2,
            "ETH-PERP".to_string(),
            PerpSide::Short,
            50_000_000,
            300_000_000_000,
            15_000_000_000,
        );
        store.insert_open(p1).unwrap();
        // MUST succeed — Account 2 is isolated from Account 1.
        store.insert_open(p2).unwrap();
        let a1 = store.get_active(&account, 1, "ETH-PERP").unwrap();
        let a2 = store.get_active(&account, 2, "ETH-PERP").unwrap();
        assert_eq!(a1.side, PerpSide::Long);
        assert_eq!(a1.subaccount_id, 1);
        assert_eq!(a2.side, PerpSide::Short);
        assert_eq!(a2.subaccount_id, 2);
    }

    #[test]
    fn subaccount_scoped_list_isolates_rows() {
        let mut store = PerpPositionsStore::new();
        let account = addr("0x0000000000000000000000000000000000000aaa");
        store
            .insert_open(new_position_skeleton(
                account.clone(),
                1,
                "ETH-PERP".to_string(),
                PerpSide::Long,
                100_000_000,
                300_000_000_000,
                30_000_000_000,
            ))
            .unwrap();
        store
            .insert_open(new_position_skeleton(
                account.clone(),
                2,
                "ETH-PERP".to_string(),
                PerpSide::Short,
                50_000_000,
                300_000_000_000,
                15_000_000_000,
            ))
            .unwrap();
        assert_eq!(store.list_for_account_and_subaccount(&account, 1).len(), 1);
        assert_eq!(store.list_for_account_and_subaccount(&account, 2).len(), 1);
        assert_eq!(store.list_for_account(&account).len(), 2);
    }
}
