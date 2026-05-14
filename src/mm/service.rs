use super::protocol::{
    AuthChallengeResultPayload, AuthResultPayload, AuthVerifyResultPayload, BulkCancelItemResult,
    BulkCancelPayload, BulkCancelResultPayload, BulkSubmitItemResult, BulkSubmitPayload,
    BulkSubmitResultPayload, CancelAllPayload, CancelAllResultPayload, CancelOrderPayload,
    CancelOrderResultPayload, ClientMessage, ErrorCode, GetSessionResultPayload,
    HeartbeatResultPayload, OptionRfqQuotePayload, OptionRfqQuoteResultPayload, ProtocolError,
    QuoteLegPayload, QuoteReplaceLegResult, QuoteReplacePayload, QuoteReplaceResultPayload,
    ResultEnvelope, RfqQuotePayload, RfqQuoteResultPayload, ServerMessage, SubmitOrderPayload,
    SubmitOrderResultPayload,
};
use super::rate_limit::{
    check_cancels_per_bulk, check_in_flight, check_message_rate, check_open_orders,
    check_orders_per_bulk, MmGatewayConfig, RateLimitDecision,
};
use super::session::MmSession;
use crate::api::dto::parse_fixed_u128;
use crate::api::AppState;
use crate::error::BackendError;
use crate::options::service::{submit_option_rfq_quote, SubmitOptionRfqQuoteInput};
use crate::orders::service::{
    cancel_order, cancel_resting_orders, submit_signed_order, CancelOrderInput,
    CancelRestingFilter, SubmitOrderOutcome,
};
use crate::rfq::service::{submit_quote as submit_rfq_quote, SubmitQuoteInput};
use crate::signing::eip712::parse_evm_address;
use crate::signing::recover_personal_signer;
use crate::signing::SignedOrder;
use crate::types::{AccountId, MarketId, Side, TimeInForce, TimestampMs};
use crate::{execution::transaction::hex_0x, mm::AuthMode};
use std::collections::BTreeSet;
use tracing::info;

#[derive(Clone)]
pub struct MmGatewayService {
    pub config: MmGatewayConfig,
    state: AppState,
}

impl MmGatewayService {
    pub fn new(config: MmGatewayConfig, state: AppState) -> Self {
        Self { config, state }
    }

    pub async fn handle_message(
        &self,
        session: &mut MmSession,
        message: ClientMessage,
        now_ms: TimestampMs,
    ) -> ServerMessage {
        let request_id = message.request_id().to_string();
        if !matches!(&message, ClientMessage::AuthVerify(_)) {
            session.clear_expired_challenge(now_ms);
        }

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
                if self.config.auth_mode == AuthMode::WalletChallenge {
                    session.decrement_in_flight();
                    return ServerMessage::error(
                        envelope.request_id,
                        ErrorCode::AuthFailed,
                        "auth message is disabled when MM_GATEWAY_AUTH_MODE=wallet_challenge",
                    );
                }
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
            ClientMessage::AuthChallenge(envelope) => self.handle_auth_challenge(
                session,
                envelope.request_id,
                envelope.payload.account,
                now_ms,
            ),
            ClientMessage::AuthVerify(envelope) => self.handle_auth_verify(
                session,
                envelope.request_id,
                envelope.payload.account,
                envelope.payload.signature,
                now_ms,
            ),
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
                self.handle_submit_order(session, envelope.request_id, envelope.payload)
                    .await
            }
            ClientMessage::BulkSubmit(envelope) => {
                self.handle_bulk_submit(session, envelope.request_id, envelope.payload)
                    .await
            }
            ClientMessage::CancelOrder(envelope) => {
                self.handle_cancel_order(session, envelope.request_id, envelope.payload)
                    .await
            }
            ClientMessage::BulkCancel(envelope) => {
                self.handle_bulk_cancel(session, envelope.request_id, envelope.payload)
                    .await
            }
            ClientMessage::CancelAll(envelope) => {
                self.handle_cancel_all(session, envelope.request_id, envelope.payload)
                    .await
            }
            ClientMessage::QuoteReplace(envelope) => {
                self.handle_quote_replace(session, envelope.request_id, envelope.payload)
                    .await
            }
            ClientMessage::RfqQuote(envelope) => {
                self.handle_rfq_quote(session, envelope.request_id, envelope.payload)
                    .await
            }
            ClientMessage::OptionRfqQuote(envelope) => {
                self.handle_option_rfq_quote(session, envelope.request_id, envelope.payload)
                    .await
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

    pub async fn cancel_on_disconnect(&self, session: &mut MmSession) -> usize {
        if !session.cancel_on_disconnect {
            return 0;
        }
        let Some(account) = session.account.clone() else {
            return 0;
        };
        let ids: BTreeSet<String> = session.open_client_order_ids.iter().cloned().collect();
        if ids.is_empty() {
            return 0;
        }

        match cancel_resting_orders(
            &self.state,
            CancelRestingFilter {
                account,
                market_id: None,
                client_order_ids: Some(ids),
            },
        )
        .await
        {
            Ok(outcome) => {
                for cancel in &outcome.cancelled {
                    if let Some(client_order_id) = cancel.client_order_id.as_deref() {
                        session.unregister_open_client_order_id(client_order_id);
                        session.unregister_quote_client_order_id(client_order_id);
                    }
                }
                outcome.cancelled.len()
            }
            Err(error) => {
                info!(
                    session_id = %session.session_id,
                    error = %error,
                    "MM cancel-on-disconnect failed"
                );
                0
            }
        }
    }

    pub fn register_session(
        &self,
        session: &MmSession,
        sender: tokio::sync::mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::error::Result<()> {
        self.state.mm_sessions.register(session, sender)
    }

    pub fn update_session(&self, session: &MmSession) -> crate::error::Result<()> {
        self.state.mm_sessions.update(session)
    }

    pub fn unregister_session(&self, session_id: &str) -> crate::error::Result<()> {
        self.state.mm_sessions.unregister(session_id)
    }

    pub fn active_sessions(&self) -> crate::error::Result<Vec<super::PublicSessionSnapshot>> {
        self.state.mm_sessions.list_active()
    }

    fn handle_auth_challenge(
        &self,
        session: &mut MmSession,
        request_id: String,
        account: AccountId,
        now_ms: TimestampMs,
    ) -> ServerMessage {
        if self.config.auth_mode != AuthMode::WalletChallenge {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "wallet challenge auth is disabled",
            );
        }
        if session.authenticated {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "session is already authenticated",
            );
        }

        let account = match canonical_account(&account) {
            Ok(account) => account,
            Err(error) => return ServerMessage::error(request_id, error.code, error.message),
        };
        let issued_at_ms = now_ms;
        let expires_at_ms = issued_at_ms.saturating_add(challenge_ttl_i64(&self.config));
        let nonce = uuid::Uuid::new_v4().to_string();
        let challenge = mm_auth_challenge_string(
            &session.session_id,
            &account,
            self.state.chain_id,
            issued_at_ms,
            expires_at_ms,
            &nonce,
        );
        session.set_challenge(account.clone(), nonce, issued_at_ms, expires_at_ms);

        ServerMessage::AuthChallengeResult(ResultEnvelope::new(
            "auth_challenge_result",
            request_id,
            AuthChallengeResultPayload {
                session_id: session.session_id.clone(),
                account,
                challenge,
                issued_at_ms,
                expires_at_ms,
            },
        ))
    }

    fn handle_auth_verify(
        &self,
        session: &mut MmSession,
        request_id: String,
        account: AccountId,
        signature: String,
        now_ms: TimestampMs,
    ) -> ServerMessage {
        if self.config.auth_mode != AuthMode::WalletChallenge {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "wallet challenge auth is disabled",
            );
        }

        let account = match canonical_account(&account) {
            Ok(account) => account,
            Err(error) => return ServerMessage::error(request_id, error.code, error.message),
        };
        let Some(challenge_account) = session.challenge_account.clone() else {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "auth challenge missing",
            );
        };
        if !accounts_equal(&challenge_account, &account) {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "auth_verify account does not match active challenge",
            );
        }
        let Some(nonce) = session.challenge_nonce.clone() else {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "auth challenge missing",
            );
        };
        let Some(issued_at_ms) = session.challenge_issued_at_ms else {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "auth challenge missing",
            );
        };
        let Some(expires_at_ms) = session.challenge_expires_at_ms else {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "auth challenge missing",
            );
        };
        if now_ms > expires_at_ms {
            session.clear_challenge();
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "auth challenge expired",
            );
        }

        let challenge = mm_auth_challenge_string(
            &session.session_id,
            &account,
            self.state.chain_id,
            issued_at_ms,
            expires_at_ms,
            &nonce,
        );
        let recovered = match recover_personal_signer(&challenge, &signature) {
            Ok(recovered) => recovered,
            Err(error) => {
                return ServerMessage::error(request_id, ErrorCode::AuthFailed, error.to_string());
            }
        };
        if !accounts_equal(&recovered, &account) {
            return ServerMessage::error(
                request_id,
                ErrorCode::AuthFailed,
                "auth signature signer does not match account",
            );
        }

        session.bind_account(account.clone());
        ServerMessage::AuthVerifyResult(ResultEnvelope::new(
            "auth_verify_result",
            request_id,
            AuthVerifyResultPayload {
                session_id: session.session_id.clone(),
                authenticated: true,
                account,
            },
        ))
    }

    async fn handle_submit_order(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: SubmitOrderPayload,
    ) -> ServerMessage {
        let client_order_id = payload.client_order_id.clone();
        let outcome = self.submit_payload(session, payload).await;
        match outcome {
            Ok(outcome) => ServerMessage::SubmitOrderResult(ResultEnvelope::new(
                "submit_order_result",
                request_id,
                submit_result_payload(outcome),
            )),
            Err(error) => ServerMessage::error(
                request_id,
                error.code,
                format_client_error(error.message, client_order_id.as_deref()),
            ),
        }
    }

    async fn handle_bulk_submit(
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
            let client_order_id = order.client_order_id.clone();
            match self.submit_payload(session, order).await {
                Ok(outcome) => {
                    accepted += 1;
                    results.push(BulkSubmitItemResult {
                        client_order_id: outcome.client_order_id,
                        ok: true,
                        order_id: outcome.order_id.map(|order_id| order_id.to_string()),
                        matched_intents: outcome.matched_intents,
                        error: None,
                    });
                }
                Err(error) => {
                    rejected += 1;
                    results.push(BulkSubmitItemResult {
                        client_order_id,
                        ok: false,
                        order_id: None,
                        matched_intents: Vec::new(),
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

    async fn handle_cancel_order(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: CancelOrderPayload,
    ) -> ServerMessage {
        match self.cancel_payload(session, payload).await {
            Ok(payload) => ServerMessage::CancelOrderResult(ResultEnvelope::new(
                "cancel_order_result",
                request_id,
                payload,
            )),
            Err(error) => ServerMessage::error(request_id, error.code, error.message),
        }
    }

    async fn handle_bulk_cancel(
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
            match self.cancel_payload(session, cancel).await {
                Ok(payload) => {
                    cancelled += 1;
                    results.push(BulkCancelItemResult {
                        client_order_id: payload.client_order_id,
                        order_id: payload.order_id,
                        ok: true,
                        error: None,
                    });
                }
                Err(error) => {
                    rejected += 1;
                    results.push(BulkCancelItemResult {
                        client_order_id,
                        order_id,
                        ok: false,
                        error: Some(error),
                    });
                }
            }
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

    async fn handle_cancel_all(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: CancelAllPayload,
    ) -> ServerMessage {
        let Some(account) = session_account(session, payload.account) else {
            return ServerMessage::error(
                request_id,
                ErrorCode::CancelRejected,
                "account is required for cancel_all",
            );
        };

        match cancel_resting_orders(
            &self.state,
            CancelRestingFilter {
                account,
                market_id: payload.market_id,
                client_order_ids: None,
            },
        )
        .await
        {
            Ok(outcome) => {
                let cancelled = outcome.cancelled.len();
                for cancel in outcome.cancelled {
                    if let Some(client_order_id) = cancel.client_order_id.as_deref() {
                        session.unregister_open_client_order_id(client_order_id);
                        session.unregister_quote_client_order_id(client_order_id);
                    }
                }
                ServerMessage::CancelAllResult(ResultEnvelope::new(
                    "cancel_all_result",
                    request_id,
                    CancelAllResultPayload { cancelled },
                ))
            }
            Err(error) => backend_error_response(request_id, ErrorCode::CancelRejected, error),
        }
    }

    async fn handle_quote_replace(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: QuoteReplacePayload,
    ) -> ServerMessage {
        if session_account(session, Some(payload.account.clone())).is_none() {
            return ServerMessage::error(
                request_id,
                ErrorCode::QuoteReplaceFailed,
                "quote_replace account does not match authenticated session",
            );
        }

        let mut cancelled = 0;
        if payload.cancel_previous {
            let ids: BTreeSet<String> = session.quote_client_order_ids.iter().cloned().collect();
            match cancel_resting_orders(
                &self.state,
                CancelRestingFilter {
                    account: payload.account.clone(),
                    market_id: Some(payload.market_id),
                    client_order_ids: Some(ids),
                },
            )
            .await
            {
                Ok(outcome) => {
                    cancelled = outcome.cancelled.len();
                    for cancel in outcome.cancelled {
                        if let Some(client_order_id) = cancel.client_order_id.as_deref() {
                            session.unregister_open_client_order_id(client_order_id);
                            session.unregister_quote_client_order_id(client_order_id);
                        }
                    }
                }
                Err(error) => {
                    return backend_error_response(
                        request_id,
                        ErrorCode::QuoteReplaceFailed,
                        error,
                    );
                }
            }
        }

        let mut submitted = 0;
        let mut rejected = 0;
        let mut matched_intents = Vec::new();
        let mut results = Vec::new();
        for (side, leg) in [(Side::Buy, payload.bid), (Side::Sell, payload.ask)] {
            let Some(leg) = leg else {
                continue;
            };
            let client_order_id = leg.client_order_id.clone();
            match self
                .submit_quote_leg(
                    session,
                    payload.market_id,
                    payload.account.clone(),
                    side,
                    leg,
                )
                .await
            {
                Ok(outcome) => {
                    submitted += 1;
                    matched_intents.extend(outcome.matched_intents.clone());
                    if outcome.resting {
                        session.register_quote_client_order_id(client_order_id.clone());
                    }
                    results.push(QuoteReplaceLegResult {
                        side,
                        client_order_id,
                        ok: true,
                        order_id: outcome.order_id.map(|order_id| order_id.to_string()),
                        matched_intents: outcome.matched_intents,
                        error: None,
                    });
                }
                Err(error) => {
                    rejected += 1;
                    results.push(QuoteReplaceLegResult {
                        side,
                        client_order_id,
                        ok: false,
                        order_id: None,
                        matched_intents: Vec::new(),
                        error: Some(error),
                    });
                }
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
                results,
                matched_intents,
            },
        ))
    }

    async fn handle_rfq_quote(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: RfqQuotePayload,
    ) -> ServerMessage {
        let Some(mm_account) = session_account(session, Some(payload.mm_account.clone())) else {
            return ServerMessage::error(
                request_id,
                ErrorCode::RfqQuoteRejected,
                "rfq_quote account does not match authenticated session",
            );
        };

        let price_1e8 = match parse_fixed_u128("price_1e8", &payload.price_1e8) {
            Ok(value) => value,
            Err(error) => {
                return backend_error_response(request_id, ErrorCode::RfqQuoteRejected, error);
            }
        };
        let size_1e8 = match parse_fixed_u128("size_1e8", &payload.size_1e8) {
            Ok(value) => value,
            Err(error) => {
                return backend_error_response(request_id, ErrorCode::RfqQuoteRejected, error);
            }
        };

        match submit_rfq_quote(
            &self.state,
            SubmitQuoteInput {
                rfq_id: payload.rfq_id,
                mm_account,
                session_id: Some(session.session_id.clone()),
                client_quote_id: payload.client_quote_id,
                price_1e8,
                size_1e8,
                quote_nonce: payload.quote_nonce,
                quote_ttl_ms: payload.quote_ttl_ms,
                signature: payload.signature,
            },
        )
        .await
        {
            Ok(quote) => ServerMessage::RfqQuoteResult(ResultEnvelope::new(
                "rfq_quote_result",
                request_id,
                RfqQuoteResultPayload {
                    quote_id: quote.quote_id,
                    rfq_id: quote.rfq_id,
                    status: quote.status,
                    expires_at_ms: quote.expires_at_ms,
                },
            )),
            Err(error) => backend_error_response(request_id, ErrorCode::RfqQuoteRejected, error),
        }
    }

    async fn handle_option_rfq_quote(
        &self,
        session: &mut MmSession,
        request_id: String,
        payload: OptionRfqQuotePayload,
    ) -> ServerMessage {
        let Some(mm_account) = session_account(session, Some(payload.mm_account.clone())) else {
            return ServerMessage::error(
                request_id,
                ErrorCode::OptionRfqQuoteRejected,
                "option_rfq_quote account does not match authenticated session",
            );
        };

        let price_1e8 = match parse_fixed_u128("price_1e8", &payload.price_1e8) {
            Ok(value) => value,
            Err(error) => {
                return backend_error_response(
                    request_id,
                    ErrorCode::OptionRfqQuoteRejected,
                    error,
                );
            }
        };
        let size_1e8 = match parse_fixed_u128("size_1e8", &payload.size_1e8) {
            Ok(value) => value,
            Err(error) => {
                return backend_error_response(
                    request_id,
                    ErrorCode::OptionRfqQuoteRejected,
                    error,
                );
            }
        };

        match submit_option_rfq_quote(
            &self.state,
            payload.option_rfq_id,
            SubmitOptionRfqQuoteInput {
                mm_account,
                session_id: Some(session.session_id.clone()),
                client_quote_id: payload.client_quote_id,
                price_1e8,
                size_1e8,
                quote_nonce: payload.quote_nonce,
                quote_ttl_ms: Some(payload.quote_ttl_ms),
                signature: payload.signature,
            },
        )
        .await
        {
            Ok(quote) => ServerMessage::OptionRfqQuoteResult(ResultEnvelope::new(
                "option_rfq_quote_result",
                request_id,
                OptionRfqQuoteResultPayload {
                    quote_id: quote.quote_id,
                    option_rfq_id: quote.option_rfq_id,
                    status: quote.status,
                    expires_at_ms: quote.expires_at_ms,
                },
            )),
            Err(error) => {
                backend_error_response(request_id, ErrorCode::OptionRfqQuoteRejected, error)
            }
        }
    }

    async fn submit_payload(
        &self,
        session: &mut MmSession,
        payload: SubmitOrderPayload,
    ) -> Result<SubmitOrderOutcome, ProtocolError> {
        let account = payload.account.clone();
        if session_account(session, Some(account.clone())).is_none() {
            return Err(ProtocolError::new(
                ErrorCode::OrderRejected,
                "order account does not match authenticated session",
            ));
        }
        if payload.client_order_id.as_deref().unwrap_or("").is_empty() {
            return Err(ProtocolError::new(
                ErrorCode::OrderRejected,
                "client_order_id is required for MM gateway orders",
            ));
        }
        let open_orders = self
            .live_open_order_count(&account)
            .map_err(|error| protocol_error(ErrorCode::OrderRejected, error))?;
        if let RateLimitDecision::Rejected { code, message } =
            check_open_orders(open_orders, 1, &self.config)
        {
            return Err(ProtocolError::new(code, message));
        }

        let signed_order = signed_order_from_submit(payload)?;
        let outcome = submit_signed_order(&self.state, signed_order)
            .await
            .map_err(|error| protocol_error(ErrorCode::OrderRejected, error))?;
        if outcome.status != "accepted" {
            return Err(ProtocolError::new(
                ErrorCode::OrderRejected,
                rejection_reason(&outcome),
            ));
        }
        if outcome.status == "accepted" && outcome.resting {
            if let Some(client_order_id) = outcome.client_order_id.clone() {
                session.register_open_client_order_id(client_order_id);
            }
        }
        Ok(outcome)
    }

    async fn submit_quote_leg(
        &self,
        session: &mut MmSession,
        market_id: MarketId,
        account: AccountId,
        side: Side,
        leg: QuoteLegPayload,
    ) -> Result<SubmitOrderOutcome, ProtocolError> {
        if leg.client_order_id.is_empty() {
            return Err(ProtocolError::new(
                ErrorCode::QuoteReplaceFailed,
                "client_order_id is required",
            ));
        }
        self.submit_payload(
            session,
            SubmitOrderPayload {
                market_id,
                account,
                side,
                price_1e8: leg.price_1e8,
                size_1e8: leg.size_1e8,
                time_in_force: TimeInForce::Gtc,
                reduce_only: false,
                post_only: false,
                client_order_id: Some(leg.client_order_id),
                nonce: leg.nonce,
                deadline_ms: leg.deadline_ms,
                signature: leg.signature,
            },
        )
        .await
        .map_err(|error| ProtocolError::new(ErrorCode::QuoteReplaceFailed, error.message))
    }

    async fn cancel_payload(
        &self,
        session: &mut MmSession,
        payload: CancelOrderPayload,
    ) -> Result<CancelOrderResultPayload, ProtocolError> {
        let Some(account) = session_account(session, payload.account.clone()) else {
            return Err(ProtocolError::new(
                ErrorCode::CancelRejected,
                "account is required for cancel",
            ));
        };
        if payload.order_id.is_none() && payload.client_order_id.is_none() {
            return Err(ProtocolError::new(
                ErrorCode::CancelRejected,
                "order_id or client_order_id is required",
            ));
        }

        let outcome = cancel_order(
            &self.state,
            CancelOrderInput {
                account: Some(account),
                market_id: payload.market_id,
                order_id: payload.order_id,
                client_order_id: payload.client_order_id,
            },
        )
        .await
        .map_err(|error| protocol_error(ErrorCode::CancelRejected, error))?;

        if let Some(client_order_id) = outcome.client_order_id.as_deref() {
            session.unregister_open_client_order_id(client_order_id);
            session.unregister_quote_client_order_id(client_order_id);
        }

        Ok(CancelOrderResultPayload {
            cancelled: true,
            client_order_id: outcome.client_order_id,
            order_id: Some(outcome.order_id.to_string()),
        })
    }

    fn live_open_order_count(&self, account: &AccountId) -> Result<usize, BackendError> {
        let engine = self
            .state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
        Ok(engine
            .resting_orders()
            .into_iter()
            .filter(|order| &order.account == account)
            .count())
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

fn signed_order_from_submit(payload: SubmitOrderPayload) -> Result<SignedOrder, ProtocolError> {
    Ok(SignedOrder {
        account: payload.account,
        market_id: payload.market_id,
        side: payload.side,
        price_1e8: parse_fixed_u128("price_1e8", &payload.price_1e8)
            .map_err(|error| protocol_error(ErrorCode::OrderRejected, error))?,
        size_1e8: parse_fixed_u128("size_1e8", &payload.size_1e8)
            .map_err(|error| protocol_error(ErrorCode::OrderRejected, error))?,
        time_in_force: payload.time_in_force,
        reduce_only: payload.reduce_only,
        post_only: payload.post_only,
        client_order_id: payload.client_order_id,
        nonce: payload.nonce,
        deadline_ms: payload.deadline_ms,
        signature: payload.signature,
    })
}

fn submit_result_payload(outcome: SubmitOrderOutcome) -> SubmitOrderResultPayload {
    SubmitOrderResultPayload {
        accepted: outcome.status == "accepted",
        client_order_id: outcome.client_order_id,
        order_id: outcome.order_id.map(|order_id| order_id.to_string()),
        status: outcome.status,
        matched_intents: outcome.matched_intents,
    }
}

fn session_account(session: &mut MmSession, account: Option<AccountId>) -> Option<AccountId> {
    match (session.account.clone(), account) {
        (Some(session_account), Some(payload_account))
            if accounts_equal(&session_account, &payload_account) =>
        {
            Some(session_account)
        }
        (Some(_), Some(_)) => None,
        (Some(session_account), None) => Some(session_account),
        (None, Some(payload_account))
            if !session.authenticated && session.auth_mode == AuthMode::Disabled =>
        {
            session.bind_account(payload_account.clone());
            Some(payload_account)
        }
        (None, Some(payload_account)) => Some(payload_account),
        (None, None) => None,
    }
}

fn canonical_account(account: &AccountId) -> Result<AccountId, ProtocolError> {
    let bytes =
        parse_evm_address(account).map_err(|error| protocol_error(ErrorCode::AuthFailed, error))?;
    Ok(AccountId::new(hex_0x(&bytes)))
}

fn accounts_equal(left: &AccountId, right: &AccountId) -> bool {
    left.0.eq_ignore_ascii_case(&right.0)
}

fn challenge_ttl_i64(config: &MmGatewayConfig) -> TimestampMs {
    TimestampMs::try_from(config.challenge_ttl_ms).unwrap_or(TimestampMs::MAX)
}

pub fn mm_auth_challenge_string(
    session_id: &str,
    account: &AccountId,
    chain_id: u64,
    issued_at_ms: TimestampMs,
    expires_at_ms: TimestampMs,
    nonce: &str,
) -> String {
    format!(
        "DeOpt v2 MM Gateway Authentication\n\nsession_id: {session_id}\naccount: {}\nchain_id: {chain_id}\nissued_at_ms: {issued_at_ms}\nexpires_at_ms: {expires_at_ms}\nnonce: {nonce}",
        account.0
    )
}

fn protocol_error(code: ErrorCode, error: BackendError) -> ProtocolError {
    ProtocolError::new(code, error.to_string())
}

fn backend_error_response(
    request_id: String,
    code: ErrorCode,
    error: BackendError,
) -> ServerMessage {
    ServerMessage::error(request_id, code, error.to_string())
}

fn format_client_error(message: String, client_order_id: Option<&str>) -> String {
    match client_order_id {
        Some(client_order_id) => format!("{client_order_id}: {message}"),
        None => message,
    }
}

fn rejection_reason(outcome: &SubmitOrderOutcome) -> String {
    outcome
        .events
        .iter()
        .find_map(|event| match event {
            crate::engine::EngineEvent::OrderRejected { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "order rejected".to_string())
}
