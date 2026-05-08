use crate::error::{BackendError, Result};
use crate::types::{AccountId, MarketId, Price1e8, Side, Size1e8, TimestampMs};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

pub type RfqId = Uuid;
pub type QuoteId = Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfqConfig {
    pub enabled: bool,
    pub require_persistence: bool,
    pub default_ttl_ms: u64,
    pub max_ttl_ms: u64,
    pub min_quote_ttl_ms: u64,
    pub max_quote_ttl_ms: u64,
    pub max_quotes_per_rfq: usize,
}

impl RfqConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
            default_ttl_ms: 5_000,
            max_ttl_ms: 30_000,
            min_quote_ttl_ms: 500,
            max_quote_ttl_ms: 10_000,
            max_quotes_per_rfq: 50,
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
                "RFQ requires persistence enabled when RFQ_REQUIRE_PERSISTENCE=true".to_string(),
            ));
        }
        if self.default_ttl_ms == 0 || self.max_ttl_ms == 0 {
            return Err(BackendError::Config(
                "RFQ TTL bounds must be nonzero".to_string(),
            ));
        }
        if self.default_ttl_ms > self.max_ttl_ms {
            return Err(BackendError::Config(
                "RFQ_DEFAULT_TTL_MS must be <= RFQ_MAX_TTL_MS".to_string(),
            ));
        }
        if self.min_quote_ttl_ms == 0 || self.max_quote_ttl_ms < self.min_quote_ttl_ms {
            return Err(BackendError::Config(
                "RFQ quote TTL bounds are invalid".to_string(),
            ));
        }
        if self.max_quotes_per_rfq == 0 {
            return Err(BackendError::Config(
                "RFQ_MAX_QUOTES_PER_RFQ must be nonzero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RfqStatus {
    Open,
    Expired,
    Accepted,
    Cancelled,
    Failed,
}

impl RfqStatus {
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
                "invalid RFQ status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RfqQuoteStatus {
    Active,
    Expired,
    Accepted,
    Rejected,
    Cancelled,
}

impl RfqQuoteStatus {
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
                "invalid RFQ quote status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RfqRequest {
    pub rfq_id: RfqId,
    pub taker: AccountId,
    pub market_id: MarketId,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub limit_price_1e8: Option<Price1e8>,
    pub status: RfqStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub accepted_quote_id: Option<QuoteId>,
    pub execution_intent_id: Option<Uuid>,
}

impl RfqRequest {
    pub fn effective_status(&self, now_ms: TimestampMs) -> RfqStatus {
        if self.status == RfqStatus::Open && now_ms >= self.expires_at_ms {
            RfqStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RfqQuote {
    pub quote_id: QuoteId,
    pub rfq_id: RfqId,
    pub mm_account: AccountId,
    pub session_id: Option<String>,
    pub client_quote_id: Option<String>,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub status: RfqQuoteStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
}

impl RfqQuote {
    pub fn effective_status(&self, now_ms: TimestampMs) -> RfqQuoteStatus {
        if self.status == RfqQuoteStatus::Active && now_ms >= self.expires_at_ms {
            RfqQuoteStatus::Expired
        } else {
            self.status
        }
    }
}

pub fn parse_rfq_id(value: &str) -> Result<RfqId> {
    Uuid::from_str(value).map_err(|_| BackendError::InvalidRfqId)
}

pub fn parse_quote_id(value: &str) -> Result<QuoteId> {
    Uuid::from_str(value).map_err(|_| BackendError::InvalidRfqQuoteId)
}
