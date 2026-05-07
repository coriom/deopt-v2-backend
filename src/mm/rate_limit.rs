use super::protocol::ErrorCode;
use super::session::{AuthMode, MmSession};
use crate::types::TimestampMs;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MmGatewayTransport {
    WebTransport,
}

impl FromStr for MmGatewayTransport {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "webtransport" => Ok(Self::WebTransport),
            other => Err(format!("unsupported MM_GATEWAY_TRANSPORT: {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmGatewayConfig {
    pub enabled: bool,
    pub transport: MmGatewayTransport,
    pub host: String,
    pub port: u16,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub max_sessions: usize,
    pub max_in_flight_per_session: u32,
    pub rate_limit_per_sec: u32,
    pub heartbeat_timeout_ms: u64,
    pub max_orders_per_bulk: usize,
    pub max_cancels_per_bulk: usize,
    pub max_open_orders_per_account: usize,
    pub cancel_on_disconnect: bool,
    pub auth_mode: AuthMode,
    pub require_auth: bool,
}

impl Default for MmGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: MmGatewayTransport::WebTransport,
            host: "127.0.0.1".to_string(),
            port: 8443,
            cert_path: None,
            key_path: None,
            max_sessions: 100,
            max_in_flight_per_session: 128,
            rate_limit_per_sec: 100,
            heartbeat_timeout_ms: 15_000,
            max_orders_per_bulk: 50,
            max_cancels_per_bulk: 100,
            max_open_orders_per_account: 500,
            cancel_on_disconnect: true,
            auth_mode: AuthMode::Disabled,
            require_auth: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RateLimitDecision {
    Allowed,
    Rejected { code: ErrorCode, message: String },
}

impl RateLimitDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

pub fn check_message_rate(
    session: &mut MmSession,
    config: &MmGatewayConfig,
    now_ms: TimestampMs,
) -> RateLimitDecision {
    if now_ms.saturating_sub(session.window_started_at_ms) >= 1_000 {
        session.window_started_at_ms = now_ms;
        session.messages_in_current_window = 0;
    }

    if session.messages_in_current_window >= config.rate_limit_per_sec {
        return RateLimitDecision::Rejected {
            code: ErrorCode::RateLimited,
            message: "message rate limit exceeded".to_string(),
        };
    }

    session.messages_in_current_window = session.messages_in_current_window.saturating_add(1);
    RateLimitDecision::Allowed
}

pub fn check_in_flight(session: &MmSession, config: &MmGatewayConfig) -> RateLimitDecision {
    if session.in_flight_requests >= config.max_in_flight_per_session {
        return RateLimitDecision::Rejected {
            code: ErrorCode::RateLimited,
            message: "too many in-flight requests for session".to_string(),
        };
    }

    RateLimitDecision::Allowed
}

pub fn check_orders_per_bulk(count: usize, config: &MmGatewayConfig) -> RateLimitDecision {
    if count > config.max_orders_per_bulk {
        return RateLimitDecision::Rejected {
            code: ErrorCode::TooManyOrders,
            message: "too many orders in bulk request".to_string(),
        };
    }

    RateLimitDecision::Allowed
}

pub fn check_cancels_per_bulk(count: usize, config: &MmGatewayConfig) -> RateLimitDecision {
    if count > config.max_cancels_per_bulk {
        return RateLimitDecision::Rejected {
            code: ErrorCode::TooManyCancels,
            message: "too many cancels in bulk request".to_string(),
        };
    }

    RateLimitDecision::Allowed
}

pub fn check_open_orders(
    current_open: usize,
    additional_open: usize,
    config: &MmGatewayConfig,
) -> RateLimitDecision {
    if current_open.saturating_add(additional_open) > config.max_open_orders_per_account {
        return RateLimitDecision::Rejected {
            code: ErrorCode::TooManyOrders,
            message: "too many open orders for account".to_string(),
        };
    }

    RateLimitDecision::Allowed
}
