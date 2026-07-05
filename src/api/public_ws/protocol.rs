//! Public WebSocket protocol — JSON-RPC 2.0-inspired.
//!
//! Wire shape is intentionally close to JSON-RPC 2.0 but extends it with:
//!   * a `meta` block on every server message that mirrors the existing
//!     HTTP `Envelope::meta` block (`source`, `chain_id`, `request_id`,
//!     `generated_at_ms`);
//!   * a dedicated `"subscription"` server method for push events with a
//!     monotonic `seq` per channel and an `event_id`.
//!
//! Error codes are SCREAMING_SNAKE_CASE strings, matching the HTTP
//! `TradingErrorCode` style so frontend code can share an error-handling
//! switch across transports.
//!
//! No part of this module reads or emits secrets. The serialisation
//! layer is the only place that touches user-supplied JSON; every
//! handler runs against typed values produced here.

use crate::types::TimestampMs;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Channel identifier registry. Adding a new channel here is the
/// single point of truth — the handler dispatches off this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Channel {
    /// Public: trading-shell health subset (mirror of `/trading/health`).
    TradingHealth,
    /// Public: options product aggregates (mirror of `/options/products`).
    OptionsProducts,
    /// Public: ranked accounts by trading volume (mirror of `/leaderboard`).
    Leaderboard,

    // ---- Private (require authenticated session) ----
    // The frontend can subscribe to these AFTER `auth.verify` succeeds;
    // V1 always answers with `AUTH_REQUIRED` because `auth.verify` is
    // deferred to a follow-on milestone (no signature-verification
    // utility yet wired into the WS layer).
    AccountPositions,
    AccountPortfolio,
    AccountBalances,
    AccountOrders,
    AccountFills,
    AccountConditionalOrders,
    AccountHistory,
    AccountIntentStatus,
    AccountSettlements,
    AccountLiquidations,
    // PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1
    AccountPerpOrders,
    AccountPerpFills,
    AccountPerpPositions,
    // PERPS-FUNDING-V1
    AccountPerpFunding,
    /// OPTIONS-RFQ-LIFECYCLE-WS-V1 — Options RFQ lifecycle deltas
    /// for `/rfq-strategy` (RFQ created/quote-submitted/accepted/
    /// cancelled + fill_created). Authenticated + account-scoped.
    AccountRfqs,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TradingHealth => "trading.health",
            Self::OptionsProducts => "options.products",
            Self::Leaderboard => "leaderboard",
            Self::AccountPositions => "account.positions",
            Self::AccountPortfolio => "account.portfolio",
            Self::AccountBalances => "account.balances",
            Self::AccountOrders => "account.orders",
            Self::AccountFills => "account.fills",
            Self::AccountConditionalOrders => "account.conditional_orders",
            Self::AccountHistory => "account.history",
            Self::AccountIntentStatus => "account.intent_status",
            Self::AccountSettlements => "account.settlements",
            Self::AccountLiquidations => "account.liquidations",
            Self::AccountPerpOrders => "account.perp_orders",
            Self::AccountPerpFills => "account.perp_fills",
            Self::AccountPerpPositions => "account.perp_positions",
            Self::AccountPerpFunding => "account.perp_funding",
            Self::AccountRfqs => "account.rfqs",
        }
    }

    pub fn parse(v: &str) -> Option<Self> {
        match v {
            "trading.health" => Some(Self::TradingHealth),
            "options.products" => Some(Self::OptionsProducts),
            "leaderboard" => Some(Self::Leaderboard),
            "account.positions" => Some(Self::AccountPositions),
            "account.portfolio" => Some(Self::AccountPortfolio),
            "account.balances" => Some(Self::AccountBalances),
            "account.orders" => Some(Self::AccountOrders),
            "account.fills" => Some(Self::AccountFills),
            "account.conditional_orders" => Some(Self::AccountConditionalOrders),
            "account.history" => Some(Self::AccountHistory),
            "account.intent_status" => Some(Self::AccountIntentStatus),
            "account.settlements" => Some(Self::AccountSettlements),
            "account.liquidations" => Some(Self::AccountLiquidations),
            "account.perp_orders" => Some(Self::AccountPerpOrders),
            "account.perp_fills" => Some(Self::AccountPerpFills),
            "account.perp_positions" => Some(Self::AccountPerpPositions),
            "account.perp_funding" => Some(Self::AccountPerpFunding),
            "account.rfqs" => Some(Self::AccountRfqs),
            _ => None,
        }
    }

    /// Every account-scoped channel requires an authenticated session.
    pub fn requires_auth(self) -> bool {
        matches!(
            self,
            Self::AccountPositions
                | Self::AccountPortfolio
                | Self::AccountBalances
                | Self::AccountOrders
                | Self::AccountFills
                | Self::AccountConditionalOrders
                | Self::AccountHistory
                | Self::AccountIntentStatus
                | Self::AccountSettlements
                | Self::AccountLiquidations
                | Self::AccountPerpOrders
                | Self::AccountPerpFills
                | Self::AccountPerpPositions
                | Self::AccountPerpFunding
                | Self::AccountRfqs
        )
    }
}

/// Frontend-facing error codes. Stable string surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WsErrorCode {
    InvalidRequest,
    InvalidParams,
    InvalidAddress,
    UnknownMethod,
    InvalidChannel,
    SubscriptionNotFound,
    AlreadySubscribed,
    TooManySubscriptions,
    AuthRequired,
    AuthUnavailable,
    AuthFailed,
    AuthExpired,
    AuthInvalidSignature,
    AuthAddressMismatch,
    AuthChallengeNotFound,
    RateLimited,
    FrameTooLarge,
    SourceUnavailable,
    NotImplemented,
    InternalError,
}

impl WsErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::InvalidParams => "INVALID_PARAMS",
            Self::InvalidAddress => "INVALID_ADDRESS",
            Self::UnknownMethod => "UNKNOWN_METHOD",
            Self::InvalidChannel => "INVALID_CHANNEL",
            Self::SubscriptionNotFound => "SUBSCRIPTION_NOT_FOUND",
            Self::AlreadySubscribed => "ALREADY_SUBSCRIBED",
            Self::TooManySubscriptions => "TOO_MANY_SUBSCRIPTIONS",
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::AuthUnavailable => "AUTH_UNAVAILABLE",
            Self::AuthFailed => "AUTH_FAILED",
            Self::AuthExpired => "AUTH_EXPIRED",
            Self::AuthInvalidSignature => "AUTH_INVALID_SIGNATURE",
            Self::AuthAddressMismatch => "AUTH_ADDRESS_MISMATCH",
            Self::AuthChallengeNotFound => "AUTH_CHALLENGE_NOT_FOUND",
            Self::RateLimited => "RATE_LIMITED",
            Self::FrameTooLarge => "FRAME_TOO_LARGE",
            Self::SourceUnavailable => "SOURCE_UNAVAILABLE",
            Self::NotImplemented => "NOT_IMPLEMENTED",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }
}

/// Stable canonical-challenge label used as part of the EIP-191
/// personal-sign message. Surfaced in the `domain` field of
/// `auth.challenge` and `auth.verify` so the wallet can show the user
/// what they're signing.
pub const PUBLIC_WS_AUTH_DOMAIN: &str = "deopt-v2-public-ws";

/// Build the canonical EIP-191 personal-sign payload for a public-WS
/// authentication challenge. The exact byte string is reconstructed
/// server-side at `auth.verify` time and the recovered signer must
/// match the supplied address.
///
/// The format is human-readable so a wallet that surfaces the
/// signing prompt to the user shows exactly what is being signed:
///
/// ```text
/// DeOpt Public WebSocket Authentication
///
/// Address: 0x...
/// Chain ID: 84532
/// Nonce: nonce_...
/// Issued At: 1700000000000
/// Expires At: 1700000060000
/// Domain: deopt-v2-public-ws
/// ```
///
/// Addresses are written in their lower-cased form to remove
/// case-folding ambiguity. Numeric fields are decimal milliseconds.
pub fn build_canonical_challenge_message(
    address: &str,
    chain_id: u64,
    nonce: &str,
    issued_at_ms: i64,
    expires_at_ms: i64,
    domain: &str,
) -> String {
    format!(
        "DeOpt Public WebSocket Authentication\n\nAddress: {}\nChain ID: {}\nNonce: {}\nIssued At: {}\nExpires At: {}\nDomain: {}",
        address.to_ascii_lowercase(),
        chain_id,
        nonce,
        issued_at_ms,
        expires_at_ms,
        domain,
    )
}

/// Meta block included on every server frame. Mirrors the HTTP
/// `crate::api::trading::MetaBlock` so frontend code reading the WS
/// stream sees the same fields it already parses for REST responses.
#[derive(Clone, Debug, Serialize)]
pub struct WsMeta {
    pub source: &'static str,
    pub chain_id: u64,
    pub request_id: String,
    pub generated_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Serialize)]
pub struct WsError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl WsError {
    pub fn new(code: WsErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code.as_str(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

// ---------------------------------------------------------------------
// Client → server
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct ClientRequest {
    pub jsonrpc: String,
    /// Optional. JSON-RPC 2.0 allows numeric / string / null ids; the
    /// public WS API surfaces back whatever the client sent.
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SubscribeParams {
    pub channel: String,
    /// Optional address hint. For private (`account.*`) channels the
    /// authenticated session address is always authoritative; if this
    /// field is present and disagrees, the server returns
    /// `AUTH_ADDRESS_MISMATCH` rather than silently using the bound
    /// address.
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UnsubscribeParams {
    pub subscription_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthChallengeParams {
    pub address: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthVerifyParams {
    pub address: String,
    pub signature: String,
}

// ---------------------------------------------------------------------
// Server → client
// ---------------------------------------------------------------------

/// Result/error response to a client request. Method-style push events
/// use `ServerNotification` below.
#[derive(Clone, Debug, Serialize)]
pub struct ServerResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WsError>,
    pub meta: WsMeta,
}

impl ServerResponse {
    pub fn ok(id: Option<Value>, result: Value, meta: WsMeta) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
            meta,
        }
    }

    pub fn err(id: Option<Value>, error: WsError, meta: WsMeta) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
            meta,
        }
    }
}

/// Server-initiated push event. Uses method name `"subscription"`.
#[derive(Clone, Debug, Serialize)]
pub struct ServerNotification {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: SubscriptionPushParams,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubscriptionPushParams {
    pub subscription_id: String,
    pub channel: &'static str,
    pub seq: u64,
    pub event_id: String,
    pub source: &'static str,
    pub chain_id: u64,
    pub generated_at_ms: TimestampMs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    pub data: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_round_trips_through_str() {
        for c in [
            Channel::TradingHealth,
            Channel::OptionsProducts,
            Channel::Leaderboard,
            Channel::AccountPositions,
            Channel::AccountPortfolio,
            Channel::AccountBalances,
            Channel::AccountOrders,
            Channel::AccountFills,
            Channel::AccountHistory,
            Channel::AccountIntentStatus,
            Channel::AccountSettlements,
            Channel::AccountLiquidations,
        ] {
            let s = c.as_str();
            assert_eq!(Channel::parse(s), Some(c));
        }
        assert_eq!(Channel::parse("garbage"), None);
    }

    #[test]
    fn account_channels_require_auth() {
        assert!(!Channel::TradingHealth.requires_auth());
        assert!(!Channel::OptionsProducts.requires_auth());
        assert!(!Channel::Leaderboard.requires_auth());
        assert!(Channel::AccountPositions.requires_auth());
        assert!(Channel::AccountPortfolio.requires_auth());
        assert!(Channel::AccountBalances.requires_auth());
        assert!(Channel::AccountOrders.requires_auth());
        assert!(Channel::AccountFills.requires_auth());
        assert!(Channel::AccountHistory.requires_auth());
        assert!(Channel::AccountIntentStatus.requires_auth());
        assert!(Channel::AccountSettlements.requires_auth());
        assert!(Channel::AccountLiquidations.requires_auth());
    }

    #[test]
    fn error_codes_serialize_as_screaming_snake_case_strings() {
        for (code, expected) in [
            (WsErrorCode::InvalidRequest, "INVALID_REQUEST"),
            (WsErrorCode::InvalidParams, "INVALID_PARAMS"),
            (WsErrorCode::UnknownMethod, "UNKNOWN_METHOD"),
            (WsErrorCode::InvalidChannel, "INVALID_CHANNEL"),
            (WsErrorCode::SubscriptionNotFound, "SUBSCRIPTION_NOT_FOUND"),
            (WsErrorCode::AlreadySubscribed, "ALREADY_SUBSCRIBED"),
            (WsErrorCode::TooManySubscriptions, "TOO_MANY_SUBSCRIPTIONS"),
            (WsErrorCode::AuthRequired, "AUTH_REQUIRED"),
            (WsErrorCode::AuthUnavailable, "AUTH_UNAVAILABLE"),
            (WsErrorCode::AuthFailed, "AUTH_FAILED"),
            (WsErrorCode::RateLimited, "RATE_LIMITED"),
            (WsErrorCode::FrameTooLarge, "FRAME_TOO_LARGE"),
            (WsErrorCode::SourceUnavailable, "SOURCE_UNAVAILABLE"),
            (WsErrorCode::NotImplemented, "NOT_IMPLEMENTED"),
            (WsErrorCode::InternalError, "INTERNAL_ERROR"),
        ] {
            assert_eq!(code.as_str(), expected);
            let v = serde_json::to_value(code).unwrap();
            assert_eq!(v, serde_json::Value::String(expected.to_string()));
        }
    }

    #[test]
    fn server_response_serializes_with_jsonrpc_and_meta() {
        let r = ServerResponse::ok(
            Some(serde_json::json!("req_1")),
            serde_json::json!({"pong": true}),
            WsMeta {
                source: "backend",
                chain_id: 84532,
                request_id: "req_xyz".to_string(),
                generated_at_ms: 100,
            },
        );
        let v = serde_json::to_value(r).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], "req_1");
        assert_eq!(v["result"]["pong"], true);
        assert!(v["error"].is_null());
        assert_eq!(v["meta"]["chain_id"], 84532);
    }

    #[test]
    fn canonical_challenge_message_is_deterministic_and_includes_every_field() {
        let m = build_canonical_challenge_message(
            "0xABCDEF1234567890ABCDEF1234567890ABCDEF12",
            31337,
            "nonce_test",
            100,
            60_100,
            PUBLIC_WS_AUTH_DOMAIN,
        );
        // Address must be lower-cased.
        assert!(m.contains("0xabcdef1234567890abcdef1234567890abcdef12"));
        assert!(m.contains("Chain ID: 31337"));
        assert!(m.contains("Nonce: nonce_test"));
        assert!(m.contains("Issued At: 100"));
        assert!(m.contains("Expires At: 60100"));
        assert!(m.contains("Domain: deopt-v2-public-ws"));
        assert!(m.starts_with("DeOpt Public WebSocket Authentication"));
        // Determinism.
        let again = build_canonical_challenge_message(
            "0xabcdef1234567890abcdef1234567890abcdef12",
            31337,
            "nonce_test",
            100,
            60_100,
            PUBLIC_WS_AUTH_DOMAIN,
        );
        assert_eq!(m, again);
    }

    #[test]
    fn client_request_parses_with_optional_params_and_id() {
        let s = r#"{"jsonrpc":"2.0","method":"ping"}"#;
        let r: ClientRequest = serde_json::from_str(s).unwrap();
        assert_eq!(r.method, "ping");
        assert!(r.id.is_none());
        assert!(r.params.is_none());

        let s2 = r#"{"jsonrpc":"2.0","id":42,"method":"subscribe","params":{"channel":"trading.health"}}"#;
        let r2: ClientRequest = serde_json::from_str(s2).unwrap();
        assert_eq!(r2.method, "subscribe");
        assert_eq!(r2.id.as_ref().unwrap().as_i64(), Some(42));
        assert!(r2.params.is_some());
    }
}
