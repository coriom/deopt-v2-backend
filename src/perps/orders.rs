//! PERPS-ORDER-EXECUTION-INTERNAL-V1 — Perps order model.
//!
//! Types + status enum + terminal-reason codes for the internal
//! execution surface. NO public HTTP handler constructs these — the
//! public Perps mutation routes still return `PerpsNotLive` at handler
//! entry. The internal `submit_perp_order_internal` +
//! `cancel_perp_order_internal` service functions are called from
//! unit tests today; future public routes will call them after
//! `PERPS-LIQUIDATION-AND-RISK-V1` and readiness gating close.

use crate::error::{BackendError, Result};
use crate::perps::positions::PerpSide;
use crate::types::{AccountId, TimestampMs};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpOrderSide {
    Buy,
    Sell,
}

impl PerpOrderSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "buy" => Ok(Self::Buy),
            "sell" => Ok(Self::Sell),
            other => Err(BackendError::Persistence(format!(
                "invalid perp order side: {other}"
            ))),
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }

    /// A buy order pushes the account **long**; a sell pushes it
    /// **short**. Reduces flip the direction relative to the
    /// existing position (handled by the fill applicator).
    pub fn position_side(self) -> PerpSide {
        match self {
            Self::Buy => PerpSide::Long,
            Self::Sell => PerpSide::Short,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpOrderType {
    Limit,
}

impl PerpOrderType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Limit => "limit",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "limit" => Ok(Self::Limit),
            other => Err(BackendError::Persistence(format!(
                "invalid perp order type: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpTimeInForce {
    Gtc,
    Ioc,
    Fok,
}

impl PerpTimeInForce {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gtc => "gtc",
            Self::Ioc => "ioc",
            Self::Fok => "fok",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "gtc" => Ok(Self::Gtc),
            "ioc" => Ok(Self::Ioc),
            "fok" => Ok(Self::Fok),
            other => Err(BackendError::PerpUnsupportedTif(other.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpOrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

impl PerpOrderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::PartiallyFilled => "partially_filled",
            Self::Filled => "filled",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "partially_filled" => Ok(Self::PartiallyFilled),
            "filled" => Ok(Self::Filled),
            "cancelled" => Ok(Self::Cancelled),
            "rejected" => Ok(Self::Rejected),
            other => Err(BackendError::Persistence(format!(
                "invalid perp order status: {other}"
            ))),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Filled | Self::Cancelled | Self::Rejected)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Open | Self::PartiallyFilled)
    }
}

/// Trader-facing terminal reason codes. Kept as `&'static str` so a
/// future rejected-attempts feed can filter by code without free-form
/// string parsing.
pub mod reason {
    pub const FILLED: &str = "filled";
    pub const USER_CANCELLED: &str = "user_cancelled";
    pub const IOC_UNFILLED_REMAINDER: &str = "ioc_unfilled_remainder";
    pub const FOK_NOT_FILLABLE: &str = "fok_not_fillable";
    pub const POST_ONLY_WOULD_MATCH: &str = "post_only_would_match";
    pub const REDUCE_ONLY_VIOLATION: &str = "reduce_only_violation";
    pub const INVALID_TIF_COMBINATION: &str = "invalid_tif_combination";
    pub const UNSUPPORTED_TIF: &str = "unsupported_tif";
    pub const ZERO_PRICE: &str = "zero_price";
    pub const ZERO_SIZE: &str = "zero_size";
    pub const UNKNOWN_MARKET: &str = "unknown_market";
    pub const STALE_MARK_PRICE: &str = "stale_mark_price";
    pub const INSUFFICIENT_MARGIN: &str = "insufficient_margin";
    pub const LEVERAGE_EXCEEDED: &str = "leverage_exceeded";
    pub const SELF_TRADE: &str = "self_trade";
    pub const POSITION_FLIP: &str = "position_flip";
    pub const DUPLICATE_CLIENT_ORDER_ID: &str = "duplicate_client_order_id";

    pub const SOURCE_MATCHING_POLICY: &str = "matching_policy";
    pub const SOURCE_REQUEST_VALIDATION: &str = "request_validation";
    pub const SOURCE_RISK: &str = "risk";
    // PERPS-LIQUIDATION-AND-RISK-V1 — order cancels driven by the
    // liquidation tick set `terminal_reason_code = "liquidated"` +
    // `terminal_reason_source = "liquidation_tick"`.
    pub const LIQUIDATED: &str = "liquidated";
    pub const SOURCE_LIQUIDATION_TICK: &str = "liquidation_tick";
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpOrder {
    pub id: Uuid,
    pub account: AccountId,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — the subaccount this order
    /// belongs to. `1` is the default account. Cross-subaccount cancel
    /// / matching is rejected: `cancel(account=X, subaccount=1)`
    /// cannot cancel an order with `subaccount_id=2` even when
    /// authenticated as X.
    #[serde(default = "crate::perps::positions::one_subaccount_id")]
    pub subaccount_id: u32,
    pub market_id: String,
    pub side: PerpOrderSide,
    pub order_type: PerpOrderType,
    pub price_1e8: u128,
    pub size_1e8: u128,
    pub remaining_size_1e8: u128,
    pub filled_size_1e8: u128,
    pub time_in_force: PerpTimeInForce,
    pub post_only: bool,
    pub reduce_only: bool,
    /// Isolated collateral posted at submit for opens/increases,
    /// pro-rated per fill. Reduce-only orders pass 0.
    pub isolated_margin_1e8: u128,
    pub status: PerpOrderStatus,
    pub client_order_id: Option<String>,
    pub terminal_reason_code: Option<String>,
    pub terminal_reason_message: Option<String>,
    pub terminal_reason_source: Option<String>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

impl PerpOrder {
    pub fn new(
        account: AccountId,
        subaccount_id: u32,
        market_id: String,
        side: PerpOrderSide,
        price_1e8: u128,
        size_1e8: u128,
        tif: PerpTimeInForce,
        post_only: bool,
        reduce_only: bool,
        isolated_margin_1e8: u128,
        client_order_id: Option<String>,
        now: TimestampMs,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            account,
            subaccount_id,
            market_id,
            side,
            order_type: PerpOrderType::Limit,
            price_1e8,
            size_1e8,
            remaining_size_1e8: size_1e8,
            filled_size_1e8: 0,
            time_in_force: tif,
            post_only,
            reduce_only,
            isolated_margin_1e8,
            status: PerpOrderStatus::Open,
            client_order_id,
            terminal_reason_code: None,
            terminal_reason_message: None,
            terminal_reason_source: None,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerpFill {
    pub id: Uuid,
    pub market_id: String,
    pub taker_order_id: Uuid,
    pub maker_order_id: Uuid,
    pub taker_account: AccountId,
    pub maker_account: AccountId,
    /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — side-specific subaccount
    /// ids. `1` = default account. Legacy wire payloads (pre-milestone)
    /// omit the fields; serde defaults to `1` for backward-compat with
    /// old JSON emitted before this ripple landed.
    #[serde(default = "crate::perps::positions::one_subaccount_id")]
    pub taker_subaccount_id: u32,
    #[serde(default = "crate::perps::positions::one_subaccount_id")]
    pub maker_subaccount_id: u32,
    pub taker_side: PerpOrderSide,
    pub price_1e8: u128,
    pub size_1e8: u128,
    pub created_at_ms: TimestampMs,
}
