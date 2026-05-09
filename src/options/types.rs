use crate::error::{BackendError, Result};
use crate::types::{AccountId, OrderId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub type OptionSeriesId = String;
pub type OptionOrderId = OrderId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionsConfig {
    pub enabled: bool,
    pub require_persistence: bool,
    pub allow_manual_series: bool,
    pub sync_onchain_registry: bool,
    pub default_contract_size_1e8: Size1e8,
}

impl OptionsConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
            allow_manual_series: true,
            sync_onchain_registry: false,
            default_contract_size_1e8: 100_000_000,
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
    Cancelled,
    Filled,
    Rejected,
    Expired,
}

impl OptionOrderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Cancelled => "cancelled",
            Self::Filled => "filled",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionOrderFilter {
    pub option_series_id: Option<OptionSeriesId>,
    pub account: Option<AccountId>,
    pub status: Option<OptionOrderStatus>,
    pub side: Option<Side>,
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
