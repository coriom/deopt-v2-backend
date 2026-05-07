use super::protocol::{
    AuthResultPayload, BulkCancelItemResult, BulkCancelPayload, BulkCancelResultPayload,
    BulkSubmitItemResult, BulkSubmitPayload, BulkSubmitResultPayload, CancelAllResultPayload,
    CancelOrderPayload, CancelOrderResultPayload, ClientMessage, ErrorCode,
    GetSessionResultPayload, HeartbeatResultPayload, ProtocolError, QuoteLegPayload,
    QuoteReplacePayload, QuoteReplaceResultPayload, ResultEnvelope, ServerMessage,
    SubmitOrderPayload, SubmitOrderResultPayload,
};
use super::rate_limit::{
    check_cancels_per_bulk, check_in_flight, check_message_rate, check_open_orders,
    check_orders_per_bulk, MmGatewayConfig, RateLimitDecision,
};
use super::session::MmSession;
use crate::types::TimestampMs;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmGatewayService {
    pub config: MmGatewayConfig,
}

impl MmGatewayService {
    pub fn new(config: MmGatewayConfig) -> Self {
        Self { config }
    }

    pub fn handle_message(
        &self,
        session: &mut MmSession,
        message: ClientMessage,
        now_ms: TimestampMs,
    ) -> ServerMessage {
        let request_id = message.request_id().to_string();

        if let Some(response) = decision_to_error(
            &request_id,
            check_message_rate(session, &self.config, now_ms),
        ) {
            return response;
        }
        if let Some(response) =
            decision_to_error(&request_id, check_in_flight(session, &self.config))
        {
            return response;
        }
        if self.config.require_auth && message.requires_auth() && !session.authenticated {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthRequired,
                "authentication is required for trading messages",
            );
        }

        session.increment_in_flight();
        let response = match message {
            ClientMessage::Auth(envelope) => {
                if let Some(account) = envelope.payload.account {
                    session.bind_account(account);
                }
                if let Some(cancel_on_disconnect) = envelope.payload.cancel_on_disconnect {
                    session.cancel_on_disconnect = cancel_on_disconnect;
                }
                ServerMessage::AuthResult(ResultEnvelope::new(
                    "auth_result",
                    envelope.request_id,
                    AuthResultPayload {
                        authenticated: session.authenticated,
                        account: session.account.clone(),
                        auth_mode: session.auth_mode,
                    },
                ))
            }
            ClientMessage::Heartbeat(envelope) => {
                session.update_heartbeat(now_ms);
                ServerMessage::HeartbeatResult(ResultEnvelope::new(
                    "heartbeat_result",
                    envelope.request_id,
                    HeartbeatResultPayload {
                        session_id: session.session_id.clone(),
                        last_heartbeat_at_ms: session.last_heartbeat_at_ms,
                    },
                ))
            }
            ClientMessage::SubmitOrder(envelope) => {
                let payload = submit_order_result(session, envelope.payload).map_err(|error| {
                    ServerMessage::error(envelope.request_id.clone(), error.code, error.message)
                });
                match payload {
                    Ok(payload) => ServerMessage::SubmitOrderResult(ResultEnvelope::new(
                        "submit_order_result",
                        envelope.request_id,
                        payload,
                    )),
                    Err(error) => error,
                }
            }
            ClientMessage::BulkSubmit(envelope) => {
                self.handle_bulk_submit(session, envelope.request_id, envelope.payload)
            }
            ClientMessage::CancelOrder(envelope) => {
                let payload = cancel_order_result(session, envelope.payload);
                ServerMessage::CancelOrderResult(ResultEnvelope::new(
                    "cancel_order_result",
                    envelope.request_id,
                    payload,
                ))
            }
            ClientMessage::BulkCancel(envelope) => {
                self.handle_bulk_cancel(session, envelope.request_id, envelope.payload)
            }
            ClientMessage::CancelAll(envelope) => {
                let cancelled = session.clear_open_client_order_ids().len();
                ServerMessage::CancelAllResult(ResultEnvelope::new(
                    "cancel_all_result",
                    envelope.request_id,
                    CancelAllResultPayload {
                        cancelled,
                        planned: true,
                    },
                ))
            }
            ClientMessage::QuoteReplace(envelope) => {
                self.handle_quote_replace(session, envelope.request_id, envelope.payload)
            }
            ClientMessage::GetSession(envelope) => {
                ServerMessage::GetSessionResult(ResultEnvelope::new(
                    "get_session_result",
                    envelope.request_id,
                    GetSessionResultPayload {
                        session: session.public_snapshot(),
                    },
                ))
            }
        };
        session.decrement_in_flight();
        response
    }

    fn handle_bulk_submit(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: BulkSubmitPayload,
    ) -> ServerMessage {
        if let Some(response) = decision_to_error(
            &request_id,
            check_orders_per_bulk(payload.orders.len(), &self.config),
        ) {
            return response;
        }
        if let Some(response) = decision_to_error(
            &request_id,
            check_open_orders(
                session.open_client_order_ids.len(),
                payload.orders.len(),
                &self.config,
            ),
        ) {
            return response;
        }

        let mut accepted = 0;
        let mut rejected = 0;
        let mut results = Vec::with_capacity(payload.orders.len());
        for order in payload.orders {
            match validate_submit_order(&order) {
                Ok(()) => {
                    if let Some(client_order_id) = order.client_order_id.clone() {
                        session.register_open_client_order_id(client_order_id.clone());
                    }
                    accepted += 1;
                    results.push(BulkSubmitItemResult {
                        client_order_id: order.client_order_id,
                        ok: true,
                        planned: true,
                        order_id: None,
                        error: None,
                    });
                }
                Err(error) => {
                    rejected += 1;
                    results.push(BulkSubmitItemResult {
                        client_order_id: order.client_order_id,
                        ok: false,
                        planned: false,
                        order_id: None,
                        error: Some(error),
                    });
                }
            }
        }

        ServerMessage::BulkSubmitResult(ResultEnvelope::new(
            "bulk_submit_result",
            request_id,
            BulkSubmitResultPayload {
                accepted,
                rejected,
                results,
            },
        ))
    }

    fn handle_bulk_cancel(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: BulkCancelPayload,
    ) -> ServerMessage {
        if let Some(response) = decision_to_error(
            &request_id,
            check_cancels_per_bulk(payload.cancels.len(), &self.config),
        ) {
            return response;
        }

        let mut cancelled = 0;
        let mut rejected = 0;
        let mut results = Vec::with_capacity(payload.cancels.len());
        for cancel in payload.cancels {
            let client_order_id = cancel.client_order_id.clone();
            let order_id = cancel.order_id.map(|order_id| order_id.to_string());
            if let Some(id) = client_order_id.as_deref() {
                if session.unregister_open_client_order_id(id) {
                    cancelled += 1;
                    results.push(BulkCancelItemResult {
                        client_order_id,
                        order_id,
                        ok: true,
                        planned: true,
                        error: None,
                    });
                    continue;
                }
            }

            rejected += 1;
            results.push(BulkCancelItemResult {
                client_order_id,
                order_id,
                ok: false,
                planned: false,
                error: Some(ProtocolError::new(
                    ErrorCode::CancelRejected,
                    "client order id is not open for this session",
                )),
            });
        }

        ServerMessage::BulkCancelResult(ResultEnvelope::new(
            "bulk_cancel_result",
            request_id,
            BulkCancelResultPayload {
                cancelled,
                rejected,
                results,
            },
        ))
    }

    fn handle_quote_replace(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: QuoteReplacePayload,
    ) -> ServerMessage {
        let mut cancelled = 0;
        if payload.cancel_previous {
            cancelled = session.clear_open_client_order_ids().len();
        }

        let mut submitted = 0;
        let mut rejected = 0;
        for leg in [&payload.bid, &payload.ask].into_iter().flatten() {
            match validate_quote_leg(leg) {
                Ok(()) => {
                    session.register_open_client_order_id(leg.client_order_id.clone());
                    submitted += 1;
                }
                Err(_) => rejected += 1,
            }
        }

        ServerMessage::QuoteReplaceResult(ResultEnvelope::new(
            "quote_replace_result",
            request_id,
            QuoteReplaceResultPayload {
                market_id: payload.market_id,
                cancelled,
                submitted,
                rejected,
                matched_intents: Vec::new(),
                planned: true,
            },
        ))
    }
}

fn decision_to_error(request_id: &str, decision: RateLimitDecision) -> Option<ServerMessage> {
    match decision {
        RateLimitDecision::Allowed => None,
        RateLimitDecision::Rejected { code, message } => {
            Some(ServerMessage::error(request_id.to_string(), code, message))
        }
    }
}

fn submit_order_result(
    session: &mut MmSession,
    payload: SubmitOrderPayload,
) -> Result<SubmitOrderResultPayload, ProtocolError> {
    validate_submit_order(&payload)?;
    if let Some(client_order_id) = payload.client_order_id.clone() {
        session.register_open_client_order_id(client_order_id.clone());
        Ok(SubmitOrderResultPayload {
            accepted: true,
            planned: true,
            client_order_id: Some(client_order_id),
            order_id: None,
            matched_intents: Vec::new(),
        })
    } else {
        Ok(SubmitOrderResultPayload {
            accepted: true,
            planned: true,
            client_order_id: None,
            order_id: None,
            matched_intents: Vec::new(),
        })
    }
}

fn cancel_order_result(
    session: &mut MmSession,
    payload: CancelOrderPayload,
) -> CancelOrderResultPayload {
    let cancelled = payload
        .client_order_id
        .as_deref()
        .is_some_and(|client_order_id| session.unregister_open_client_order_id(client_order_id));
    CancelOrderResultPayload {
        cancelled,
        client_order_id: payload.client_order_id,
        order_id: payload.order_id.map(|order_id| order_id.to_string()),
        planned: true,
    }
}

fn validate_submit_order(payload: &SubmitOrderPayload) -> Result<(), ProtocolError> {
    if payload.client_order_id.as_deref().unwrap_or("").is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::OrderRejected,
            "client_order_id is required for MM gateway V1A planning",
        ));
    }
    validate_positive_fixed("price_1e8", &payload.price_1e8)?;
    validate_positive_fixed("size_1e8", &payload.size_1e8)?;
    if payload.nonce == 0 {
        return Err(ProtocolError::new(
            ErrorCode::OrderRejected,
            "nonce must be nonzero",
        ));
    }
    if payload.signature.is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::OrderRejected,
            "signature is required",
        ));
    }

    Ok(())
}

fn validate_quote_leg(payload: &QuoteLegPayload) -> Result<(), ProtocolError> {
    if payload.client_order_id.is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::QuoteReplaceFailed,
            "client_order_id is required",
        ));
    }
    validate_positive_fixed("price_1e8", &payload.price_1e8)?;
    validate_positive_fixed("size_1e8", &payload.size_1e8)?;
    if payload.nonce == 0 {
        return Err(ProtocolError::new(
            ErrorCode::QuoteReplaceFailed,
            "nonce must be nonzero",
        ));
    }
    if payload.signature.is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::QuoteReplaceFailed,
            "signature is required",
        ));
    }

    Ok(())
}

fn validate_positive_fixed(field: &str, value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value
            .parse::<u128>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    {
        return Err(ProtocolError::new(
            ErrorCode::BadRequest,
            format!("{field} must be a positive unsigned integer string"),
        ));
    }

    Ok(())
}
