use crate::error::{BackendError, Result};
use crate::types::{AccountId, OrderId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

pub type OptionSeriesId = String;
pub type OptionOrderId = OrderId;
pub type OptionFillId = Uuid;
pub type OptionRfqId = Uuid;
pub type OptionRfqQuoteId = Uuid;
pub type OptionRfqFillId = Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionsConfig {
    pub enabled: bool,
    pub require_persistence: bool,
    pub allow_manual_series: bool,
    pub sync_onchain_registry: bool,
    pub default_contract_size_1e8: Size1e8,
    pub rfq_enabled: bool,
    pub rfq_require_persistence: bool,
    pub rfq_default_ttl_ms: u64,
    pub rfq_max_ttl_ms: u64,
    pub rfq_min_quote_ttl_ms: u64,
    pub rfq_max_quote_ttl_ms: u64,
    pub rfq_max_quotes_per_rfq: usize,
}

impl OptionsConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
            allow_manual_series: true,
            sync_onchain_registry: false,
            default_contract_size_1e8: 100_000_000,
            rfq_enabled: false,
            rfq_require_persistence: true,
            rfq_default_ttl_ms: 5_000,
            rfq_max_ttl_ms: 30_000,
            rfq_min_quote_ttl_ms: 500,
            rfq_max_quote_ttl_ms: 10_000,
            rfq_max_quotes_per_rfq: 50,
        }
    }

    pub fn enabled_in_memory_for_tests() -> Self {
        Self {
            enabled: true,
            require_persistence: false,
            ..Self::disabled()
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.enabled && self.require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "Options require persistence enabled when OPTIONS_REQUIRE_PERSISTENCE=true"
                    .to_string(),
            ));
        }
        if self.default_contract_size_1e8 == 0 {
            return Err(BackendError::Config(
                "OPTIONS_DEFAULT_CONTRACT_SIZE_1E8 must be nonzero".to_string(),
            ));
        }
        if self.rfq_enabled && self.rfq_require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "Option RFQ requires persistence enabled when OPTION_RFQ_REQUIRE_PERSISTENCE=true"
                    .to_string(),
            ));
        }
        if self.rfq_default_ttl_ms == 0 || self.rfq_max_ttl_ms == 0 {
            return Err(BackendError::Config(
                "Option RFQ TTL bounds must be nonzero".to_string(),
            ));
        }
        if self.rfq_default_ttl_ms > self.rfq_max_ttl_ms {
            return Err(BackendError::Config(
                "OPTION_RFQ_DEFAULT_TTL_MS must be <= OPTION_RFQ_MAX_TTL_MS".to_string(),
            ));
        }
        if self.rfq_min_quote_ttl_ms == 0 || self.rfq_max_quote_ttl_ms < self.rfq_min_quote_ttl_ms {
            return Err(BackendError::Config(
                "Option RFQ quote TTL bounds are invalid".to_string(),
            ));
        }
        if self.rfq_max_quotes_per_rfq == 0 {
            return Err(BackendError::Config(
                "OPTION_RFQ_MAX_QUOTES_PER_RFQ must be nonzero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionSeriesStatus {
    Active,
    Expired,
    Disabled,
}

impl OptionSeriesStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "disabled" => Ok(Self::Disabled),
            other => Err(BackendError::Persistence(format!(
                "invalid option series status: {other}"
            ))),
        }
    }
}

impl FromStr for OptionSeriesStatus {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "disabled" => Ok(Self::Disabled),
            other => Err(BackendError::Config(format!(
                "invalid option series status filter: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionSeriesSource {
    Manual,
    Onchain,
}

impl OptionSeriesSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Onchain => "onchain",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "manual" => Ok(Self::Manual),
            "onchain" => Ok(Self::Onchain),
            other => Err(BackendError::Persistence(format!(
                "invalid option series source: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionSeries {
    pub option_series_id: OptionSeriesId,
    pub underlying: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: Price1e8,
    pub is_call: bool,
    pub contract_size_1e8: Size1e8,
    pub status: OptionSeriesStatus,
    pub source: OptionSeriesSource,
    pub onchain_product_id: Option<String>,
    pub onchain_series_id: Option<String>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

impl OptionSeries {
    pub fn effective_status(&self, now_sec: u64) -> OptionSeriesStatus {
        if self.status == OptionSeriesStatus::Active && now_sec >= self.expiry {
            OptionSeriesStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionSeriesFilter {
    pub underlying: Option<String>,
    pub expiry: Option<u64>,
    pub is_call: Option<bool>,
    pub status: Option<OptionSeriesStatus>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionOrderStatus {
    Open,
    PartiallyFilled,
    Cancelled,
    Filled,
    Rejected,
    Expired,
}

impl OptionOrderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::PartiallyFilled => "partially_filled",
            Self::Cancelled => "cancelled",
            Self::Filled => "filled",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "partially_filled" => Ok(Self::PartiallyFilled),
            "cancelled" => Ok(Self::Cancelled),
            "filled" => Ok(Self::Filled),
            "rejected" => Ok(Self::Rejected),
            "expired" => Ok(Self::Expired),
            other => Err(BackendError::Persistence(format!(
                "invalid option order status: {other}"
            ))),
        }
    }
}

impl FromStr for OptionOrderStatus {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "partially_filled" => Ok(Self::PartiallyFilled),
            "cancelled" => Ok(Self::Cancelled),
            "filled" => Ok(Self::Filled),
            "rejected" => Ok(Self::Rejected),
            "expired" => Ok(Self::Expired),
            other => Err(BackendError::Config(format!(
                "invalid option order status filter: {other}"
            ))),
        }
    }
}

impl OptionOrderStatus {
    pub fn is_live(self) -> bool {
        matches!(self, Self::Open | Self::PartiallyFilled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionOrder {
    pub order_id: OptionOrderId,
    pub option_series_id: OptionSeriesId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub remaining_size_1e8: Size1e8,
    pub time_in_force: TimeInForce,
    pub client_order_id: Option<String>,
    pub nonce: Option<u64>,
    pub deadline_ms: Option<TimestampMs>,
    pub signature: Option<String>,
    pub status: OptionOrderStatus,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionFill {
    pub fill_id: OptionFillId,
    pub option_series_id: OptionSeriesId,
    pub buy_order_id: OptionOrderId,
    pub sell_order_id: OptionOrderId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub maker_order_id: OptionOrderId,
    pub taker_order_id: OptionOrderId,
    pub taker_side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub created_at_ms: TimestampMs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionRfqStatus {
    Open,
    Expired,
    Accepted,
    Cancelled,
    Failed,
}

impl OptionRfqStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Expired => "expired",
            Self::Accepted => "accepted",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "expired" => Ok(Self::Expired),
            "accepted" => Ok(Self::Accepted),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => Err(BackendError::Persistence(format!(
                "invalid option RFQ status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionRfqQuoteStatus {
    Active,
    Expired,
    Accepted,
    Rejected,
    Cancelled,
}

impl OptionRfqQuoteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(BackendError::Persistence(format!(
                "invalid option RFQ quote status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionRfqRequest {
    pub option_rfq_id: OptionRfqId,
    pub taker: AccountId,
    pub option_series_id: OptionSeriesId,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub limit_price_1e8: Option<Price1e8>,
    pub status: OptionRfqStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub accepted_quote_id: Option<OptionRfqQuoteId>,
    pub option_fill_id: Option<OptionRfqFillId>,
}

impl OptionRfqRequest {
    pub fn effective_status(&self, now_ms: TimestampMs) -> OptionRfqStatus {
        if self.status == OptionRfqStatus::Open && now_ms >= self.expires_at_ms {
            OptionRfqStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionRfqQuote {
    pub quote_id: OptionRfqQuoteId,
    pub option_rfq_id: OptionRfqId,
    pub mm_account: AccountId,
    pub session_id: Option<String>,
    pub client_quote_id: Option<String>,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub status: OptionRfqQuoteStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
}

impl OptionRfqQuote {
    pub fn effective_status(&self, now_ms: TimestampMs) -> OptionRfqQuoteStatus {
        if self.status == OptionRfqQuoteStatus::Active && now_ms >= self.expires_at_ms {
            OptionRfqQuoteStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionRfqFill {
    pub fill_id: OptionRfqFillId,
    pub option_rfq_id: OptionRfqId,
    pub quote_id: OptionRfqQuoteId,
    pub option_series_id: OptionSeriesId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub taker: AccountId,
    pub mm_account: AccountId,
    pub taker_side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub created_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionOrderFilter {
    pub option_series_id: Option<OptionSeriesId>,
    pub account: Option<AccountId>,
    pub status: Option<OptionOrderStatus>,
    pub side: Option<Side>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionFillFilter {
    pub option_series_id: Option<OptionSeriesId>,
    pub account: Option<AccountId>,
    pub order_id: Option<OptionOrderId>,
}

impl OptionFillFilter {
    pub fn matches(&self, fill: &OptionFill) -> bool {
        if let Some(option_series_id) = &self.option_series_id {
            if &fill.option_series_id != option_series_id {
                return false;
            }
        }
        if let Some(account) = &self.account {
            if !fill.buyer.0.eq_ignore_ascii_case(&account.0)
                && !fill.seller.0.eq_ignore_ascii_case(&account.0)
            {
                return false;
            }
        }
        if let Some(order_id) = self.order_id {
            if fill.buy_order_id != order_id
                && fill.sell_order_id != order_id
                && fill.maker_order_id != order_id
                && fill.taker_order_id != order_id
            {
                return false;
            }
        }
        true
    }
}

impl OptionOrderFilter {
    pub fn matches(&self, order: &OptionOrder) -> bool {
        if let Some(option_series_id) = &self.option_series_id {
            if &order.option_series_id != option_series_id {
                return false;
            }
        }
        if let Some(account) = &self.account {
            if !order.account.0.eq_ignore_ascii_case(&account.0) {
                return false;
            }
        }
        if let Some(status) = self.status {
            if order.status != status {
                return false;
            }
        }
        if let Some(side) = self.side {
            if order.side != side {
                return false;
            }
        }
        true
    }
}

impl OptionSeriesFilter {
    pub fn matches(&self, series: &OptionSeries, now_sec: u64) -> bool {
        if let Some(underlying) = &self.underlying {
            if !series.underlying.eq_ignore_ascii_case(underlying) {
                return false;
            }
        }
        if let Some(expiry) = self.expiry {
            if series.expiry != expiry {
                return false;
            }
        }
        if let Some(is_call) = self.is_call {
            if series.is_call != is_call {
                return false;
            }
        }
        if let Some(status) = self.status {
            if series.effective_status(now_sec) != status {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionOrderbookSnapshot {
    pub option_series_id: OptionSeriesId,
    pub status: OptionSeriesStatus,
    pub bids: Vec<OptionOrderbookLevel>,
    pub asks: Vec<OptionOrderbookLevel>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionOrderbookLevel {
    pub price_1e8: String,
    pub size_1e8: String,
}
