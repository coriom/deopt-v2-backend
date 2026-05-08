use super::session::PublicSessionSnapshot;
use crate::types::{AccountId, MarketId, OrderId, Side, TimeInForce, TimestampMs};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BadRequest,
    UnknownMessageType,
    AuthRequired,
    AuthFailed,
    RateLimited,
    TooManyOrders,
    TooManyCancels,
    SessionClosed,
    OrderRejected,
    CancelRejected,
    QuoteReplaceFailed,
    InternalError,
    NotImplemented,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientMessage {
    Auth(ClientEnvelope<AuthPayload>),
    Heartbeat(ClientEnvelope<HeartbeatPayload>),
    SubmitOrder(ClientEnvelope<SubmitOrderPayload>),
    BulkSubmit(ClientEnvelope<BulkSubmitPayload>),
    CancelOrder(ClientEnvelope<CancelOrderPayload>),
    BulkCancel(ClientEnvelope<BulkCancelPayload>),
    CancelAll(ClientEnvelope<CancelAllPayload>),
    QuoteReplace(ClientEnvelope<QuoteReplacePayload>),
    GetSession(ClientEnvelope<GetSessionPayload>),
}

impl ClientMessage {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Auth(envelope) => &envelope.request_id,
            Self::Heartbeat(envelope) => &envelope.request_id,
            Self::SubmitOrder(envelope) => &envelope.request_id,
            Self::BulkSubmit(envelope) => &envelope.request_id,
            Self::CancelOrder(envelope) => &envelope.request_id,
            Self::BulkCancel(envelope) => &envelope.request_id,
            Self::CancelAll(envelope) => &envelope.request_id,
            Self::QuoteReplace(envelope) => &envelope.request_id,
            Self::GetSession(envelope) => &envelope.request_id,
        }
    }

    pub fn message_type(&self) -> &'static str {
        match self {
            Self::Auth(_) => "auth",
            Self::Heartbeat(_) => "heartbeat",
            Self::SubmitOrder(_) => "submit_order",
            Self::BulkSubmit(_) => "bulk_submit",
            Self::CancelOrder(_) => "cancel_order",
            Self::BulkCancel(_) => "bulk_cancel",
            Self::CancelAll(_) => "cancel_all",
            Self::QuoteReplace(_) => "quote_replace",
            Self::GetSession(_) => "get_session",
        }
    }

    pub fn requires_auth(&self) -> bool {
        !matches!(
            self,
            Self::Auth(_) | Self::Heartbeat(_) | Self::GetSession(_)
        )
    }
}

impl<'de> Deserialize<'de> for ClientMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawClientEnvelope::deserialize(deserializer)?;
        parse_raw_client_envelope(raw).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientEnvelope<T> {
    #[serde(rename = "type")]
    pub message_type: String,
    pub request_id: String,
    pub payload: T,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct RawClientEnvelope {
    #[serde(rename = "type")]
    message_type: String,
    request_id: String,
    payload: Value,
}

fn parse_raw_client_envelope(raw: RawClientEnvelope) -> Result<ClientMessage, ProtocolError> {
    match raw.message_type.as_str() {
        "auth" => Ok(ClientMessage::Auth(parse_payload(raw)?)),
        "heartbeat" => Ok(ClientMessage::Heartbeat(parse_payload(raw)?)),
        "submit_order" => Ok(ClientMessage::SubmitOrder(parse_payload(raw)?)),
        "bulk_submit" => Ok(ClientMessage::BulkSubmit(parse_payload(raw)?)),
        "cancel_order" => Ok(ClientMessage::CancelOrder(parse_payload(raw)?)),
        "bulk_cancel" => Ok(ClientMessage::BulkCancel(parse_payload(raw)?)),
        "cancel_all" => Ok(ClientMessage::CancelAll(parse_payload(raw)?)),
        "quote_replace" => Ok(ClientMessage::QuoteReplace(parse_payload(raw)?)),
        "get_session" => Ok(ClientMessage::GetSession(parse_payload(raw)?)),
        _ => Err(ProtocolError::new(
            ErrorCode::UnknownMessageType,
            "unknown message type",
        )),
    }
}

fn parse_payload<T: DeserializeOwned>(
    raw: RawClientEnvelope,
) -> Result<ClientEnvelope<T>, ProtocolError> {
    let payload = serde_json::from_value(raw.payload).map_err(|_| {
        ProtocolError::new(ErrorCode::BadRequest, "invalid payload for message type")
    })?;
    Ok(ClientEnvelope {
        message_type: raw.message_type,
        request_id: raw.request_id,
        payload,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct AuthPayload {
    pub account: Option<AccountId>,
    pub token: Option<String>,
    pub cancel_on_disconnect: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default, Deserialize)]
pub struct HeartbeatPayload {}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct SubmitOrderPayload {
    pub market_id: MarketId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: String,
    pub size_1e8: String,
    pub time_in_force: TimeInForce,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub nonce: u64,
    pub deadline_ms: TimestampMs,
    pub signature: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct BulkSubmitPayload {
    pub orders: Vec<SubmitOrderPayload>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct CancelOrderPayload {
    pub account: Option<AccountId>,
    pub market_id: Option<MarketId>,
    pub order_id: Option<OrderId>,
    pub client_order_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct BulkCancelPayload {
    pub cancels: Vec<CancelOrderPayload>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct CancelAllPayload {
    pub account: Option<AccountId>,
    pub market_id: Option<MarketId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct QuoteReplacePayload {
    pub market_id: MarketId,
    pub account: AccountId,
    #[serde(default)]
    pub cancel_previous: bool,
    pub bid: Option<QuoteLegPayload>,
    pub ask: Option<QuoteLegPayload>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct QuoteLegPayload {
    pub price_1e8: String,
    pub size_1e8: String,
    pub client_order_id: String,
    pub nonce: u64,
    pub deadline_ms: TimestampMs,
    pub signature: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default, Deserialize)]
pub struct GetSessionPayload {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AuthResultPayload {
    pub authenticated: bool,
    pub account: Option<AccountId>,
    pub auth_mode: super::session::AuthMode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HeartbeatResultPayload {
    pub session_id: String,
    pub last_heartbeat_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubmitOrderResultPayload {
    pub accepted: bool,
    pub client_order_id: Option<String>,
    pub order_id: Option<String>,
    pub status: String,
    pub matched_intents: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BulkSubmitResultPayload {
    pub accepted: usize,
    pub rejected: usize,
    pub results: Vec<BulkSubmitItemResult>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BulkSubmitItemResult {
    pub client_order_id: Option<String>,
    pub ok: bool,
    pub order_id: Option<String>,
    pub matched_intents: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CancelOrderResultPayload {
    pub cancelled: bool,
    pub client_order_id: Option<String>,
    pub order_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BulkCancelResultPayload {
    pub cancelled: usize,
    pub rejected: usize,
    pub results: Vec<BulkCancelItemResult>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BulkCancelItemResult {
    pub client_order_id: Option<String>,
    pub order_id: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CancelAllResultPayload {
    pub cancelled: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuoteReplaceResultPayload {
    pub market_id: MarketId,
    pub cancelled: usize,
    pub submitted: usize,
    pub rejected: usize,
    pub results: Vec<QuoteReplaceLegResult>,
    pub matched_intents: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuoteReplaceLegResult {
    pub side: Side,
    pub client_order_id: String,
    pub ok: bool,
    pub order_id: Option<String>,
    pub matched_intents: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GetSessionResultPayload {
    pub session: PublicSessionSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage {
    AuthResult(ResultEnvelope<AuthResultPayload>),
    HeartbeatResult(ResultEnvelope<HeartbeatResultPayload>),
    SubmitOrderResult(ResultEnvelope<SubmitOrderResultPayload>),
    BulkSubmitResult(ResultEnvelope<BulkSubmitResultPayload>),
    CancelOrderResult(ResultEnvelope<CancelOrderResultPayload>),
    BulkCancelResult(ResultEnvelope<BulkCancelResultPayload>),
    CancelAllResult(ResultEnvelope<CancelAllResultPayload>),
    QuoteReplaceResult(ResultEnvelope<QuoteReplaceResultPayload>),
    GetSessionResult(ResultEnvelope<GetSessionResultPayload>),
    Error(ErrorEnvelope),
}

impl ServerMessage {
    pub fn error(
        request_id: impl Into<String>,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self::Error(ErrorEnvelope {
            message_type: "error".to_string(),
            request_id: request_id.into(),
            ok: false,
            error: ProtocolError::new(code, message),
        })
    }
}

impl Serialize for ServerMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::AuthResult(envelope) => envelope.serialize(serializer),
            Self::HeartbeatResult(envelope) => envelope.serialize(serializer),
            Self::SubmitOrderResult(envelope) => envelope.serialize(serializer),
            Self::BulkSubmitResult(envelope) => envelope.serialize(serializer),
            Self::CancelOrderResult(envelope) => envelope.serialize(serializer),
            Self::BulkCancelResult(envelope) => envelope.serialize(serializer),
            Self::CancelAllResult(envelope) => envelope.serialize(serializer),
            Self::QuoteReplaceResult(envelope) => envelope.serialize(serializer),
            Self::GetSessionResult(envelope) => envelope.serialize(serializer),
            Self::Error(envelope) => envelope.serialize(serializer),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResultEnvelope<T> {
    #[serde(rename = "type")]
    pub message_type: String,
    pub request_id: String,
    pub ok: bool,
    pub payload: T,
}

impl<T> ResultEnvelope<T> {
    pub fn new(message_type: impl Into<String>, request_id: impl Into<String>, payload: T) -> Self {
        Self {
            message_type: message_type.into(),
            request_id: request_id.into(),
            ok: true,
            payload,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ErrorEnvelope {
    #[serde(rename = "type")]
    pub message_type: String,
    pub request_id: String,
    pub ok: bool,
    pub error: ProtocolError,
}
