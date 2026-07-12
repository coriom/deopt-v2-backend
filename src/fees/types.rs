use crate::error::{BackendError, Result};
use crate::types::{AccountId, MarketId, TimestampMs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeesConfig {
    pub enabled: bool,
    pub require_persistence: bool,
    pub rebates_enabled: bool,
    pub protocol_fee_recipient: String,
    pub default_fee_asset: String,
    pub option_fee_basis: OptionFeeBasis,
    pub option_premium_cap_pct: u64,
}

impl FeesConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
            rebates_enabled: false,
            protocol_fee_recipient: "treasury".to_string(),
            default_fee_asset: "USDC".to_string(),
            option_fee_basis: OptionFeeBasis::PremiumOrUnderlyingCapped,
            option_premium_cap_pct: 10,
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
                "Fees require persistence enabled when FEES_REQUIRE_PERSISTENCE=true".to_string(),
            ));
        }
        if self.enabled && self.protocol_fee_recipient.trim().is_empty() {
            return Err(BackendError::Config(
                "FEES_PROTOCOL_FEE_RECIPIENT must be non-empty".to_string(),
            ));
        }
        if self.enabled && self.default_fee_asset.trim().is_empty() {
            return Err(BackendError::Config(
                "FEES_DEFAULT_FEE_ASSET must be non-empty".to_string(),
            ));
        }
        if self.option_premium_cap_pct == 0 || self.option_premium_cap_pct > 100 {
            return Err(BackendError::Config(
                "FEES_OPTION_PREMIUM_CAP_PCT must be in 1..=100".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionFeeBasis {
    #[default]
    PremiumOrUnderlyingCapped,
}

impl OptionFeeBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PremiumOrUnderlyingCapped => "premium_or_underlying_capped",
        }
    }
}

impl FromStr for OptionFeeBasis {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "premium_or_underlying_capped" => Ok(Self::PremiumOrUnderlyingCapped),
            other => Err(BackendError::Config(format!(
                "invalid FEES_OPTION_FEE_BASIS: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeSourceType {
    OptionOrderFill,
    OptionRfqFill,
    OptionMultiLegRfqFill,
    PerpTrade,
    PerpExecution,
    PerpRfq,
}

impl FeeSourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptionOrderFill => "option_order_fill",
            Self::OptionRfqFill => "option_rfq_fill",
            Self::OptionMultiLegRfqFill => "option_multi_leg_rfq_fill",
            Self::PerpTrade => "perp_trade",
            Self::PerpExecution => "perp_execution",
            Self::PerpRfq => "perp_rfq",
        }
    }
}

impl FromStr for FeeSourceType {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "option_order_fill" => Ok(Self::OptionOrderFill),
            "option_rfq_fill" => Ok(Self::OptionRfqFill),
            "option_multi_leg_rfq_fill" => Ok(Self::OptionMultiLegRfqFill),
            "perp_trade" => Ok(Self::PerpTrade),
            "perp_execution" => Ok(Self::PerpExecution),
            "perp_rfq" => Ok(Self::PerpRfq),
            other => Err(BackendError::Persistence(format!(
                "invalid fee source_type: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeMarketType {
    Perp,
    Option,
}

impl FeeMarketType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Perp => "perp",
            Self::Option => "option",
        }
    }
}

impl FromStr for FeeMarketType {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "perp" => Ok(Self::Perp),
            "option" => Ok(Self::Option),
            other => Err(BackendError::Persistence(format!(
                "invalid fee market_type: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeFlowType {
    Orderbook,
    Rfq,
}

impl FeeFlowType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Orderbook => "orderbook",
            Self::Rfq => "rfq",
        }
    }
}

impl FromStr for FeeFlowType {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "orderbook" => Ok(Self::Orderbook),
            "rfq" => Ok(Self::Rfq),
            other => Err(BackendError::Persistence(format!(
                "invalid fee flow_type: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeStatus {
    Accrued,
}

impl FeeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accrued => "accrued",
        }
    }
}

impl FromStr for FeeStatus {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "accrued" => Ok(Self::Accrued),
            other => Err(BackendError::Persistence(format!(
                "invalid fee status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebateStatus {
    Accrued,
}

impl RebateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accrued => "accrued",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeeEvent {
    pub fee_event_id: String,
    pub source_type: FeeSourceType,
    pub source_id: String,
    pub market_type: FeeMarketType,
    pub flow_type: FeeFlowType,
    pub market_id: Option<MarketId>,
    pub option_series_id: Option<String>,
    pub maker: Option<AccountId>,
    pub taker: Option<AccountId>,
    pub payer: AccountId,
    pub recipient: String,
    pub fee_asset: String,
    pub notional_1e8: u128,
    pub fee_rate_micro_bps: u64,
    pub fee_amount_1e8: u128,
    pub rebate_rate_micro_bps: u64,
    pub rebate_amount_1e8: u128,
    pub protocol_amount_1e8: u128,
    pub status: FeeStatus,
    pub created_at_ms: TimestampMs,
}

impl FeeEvent {
    pub fn unique_key(&self) -> (String, String, String, String) {
        (
            self.source_type.as_str().to_string(),
            self.source_id.clone(),
            self.payer.0.to_ascii_lowercase(),
            self.recipient.to_ascii_lowercase(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VolumeBucket {
    pub bucket_id: String,
    pub account: AccountId,
    pub bucket_day: String,
    pub market_type: FeeMarketType,
    pub maker_volume_1e8: u128,
    pub taker_volume_1e8: u128,
    pub total_volume_1e8: u128,
    pub updated_at_ms: TimestampMs,
}

impl VolumeBucket {
    pub fn key(account: &AccountId, bucket_day: &str, market_type: FeeMarketType) -> String {
        format!(
            "vol-{}-{}-{}",
            account.0.to_ascii_lowercase(),
            bucket_day,
            market_type.as_str()
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RebateAccrual {
    pub rebate_id: String,
    pub fee_event_id: String,
    pub account: AccountId,
    pub source_type: FeeSourceType,
    pub source_id: String,
    pub rebate_asset: String,
    pub rebate_amount_1e8: u128,
    pub status: RebateStatus,
    pub created_at_ms: TimestampMs,
}

impl RebateAccrual {
    pub fn unique_key(&self) -> (String, String) {
        (
            self.fee_event_id.clone(),
            self.account.0.to_ascii_lowercase(),
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeeLedgerSummary {
    pub event_count: u64,
    pub fee_total_1e8: u128,
    pub rebate_total_1e8: u128,
    pub protocol_total_1e8: u128,
    pub status_counts: BTreeMap<String, u64>,
    pub source_type_counts: BTreeMap<String, u64>,
    pub market_type_counts: BTreeMap<String, u64>,
    pub flow_type_counts: BTreeMap<String, u64>,
}
