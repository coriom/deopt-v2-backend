use super::AppState;
use crate::admin::{authenticate, require_role, required_role_for, AdminAuthError, AdminIdentity};
use crate::api::dto::{
    parse_fixed_u128, ApiEngineEvent, ApiExecutionIntent, SubmitOrderRequest, SubmitOrderResponse,
};
use crate::confirmation::{
    decide_confirmation, ConfirmationDecision, ConfirmationDecisionInput, ConfirmationStatus,
};
use crate::db::PgRepository;
use crate::error::{BackendError, Result as BackendResult};
use crate::execution::{
    b256_to_hex_bytes32, build_execution_transaction_request, ensure_no_submitted_transaction,
    perp_trade_digest, simulate_execution_intent, DecodedRevertError, ExecutionIntentStatus,
    ExecutionTransaction, ExecutionTransactionStatus, Executor, ExecutorSigner,
    HttpJsonRpcProvider, PerpTradeDomain, PerpTradePayload, SimulationResult,
    StoredTradeSignatures, TradeSignatureStatus, TransactionBroadcastProvider,
    TransactionReceiptProvider, PERP_TRADE_TYPE,
};
use crate::fees::service::{
    admin_fee_events as admin_fee_events_service, admin_fee_rebates as admin_fee_rebates_service,
    admin_fee_summary as admin_fee_summary_service, admin_fee_volumes as admin_fee_volumes_service,
    admin_onchain_fees as admin_onchain_fees_service, record_indexed_perp_trade_fees,
};
use crate::fees::smoke_readiness::admin_v2_smoke_readiness as admin_v2_smoke_readiness_service;
use crate::fees::v2_observability::admin_v2_observability as admin_v2_observability_service;
use crate::fees::vault_observability as vault_obs;
use crate::indexer::{Indexer, IndexerStatus, IndexerTickResult};
use crate::mm::permissions::{
    list_permission_accounts, list_product_permissions, MmProductPermission,
};
use crate::monitoring::{readiness, render_metrics};
use crate::nonce_sync::{
    read_option_nonce, read_perp_nonce, OptionNonceResponse, PerpNonceResponse,
};
use crate::options::service::{
    accept_option_rfq_quote as accept_option_rfq_quote_service,
    broadcast_option_execution_intent as broadcast_option_execution_intent_service,
    cancel_option_order as cancel_option_order_service,
    cancel_option_rfq as cancel_option_rfq_service,
    confirm_option_execution_intent as confirm_option_execution_intent_service,
    create_option_rfq as create_option_rfq_service,
    create_option_series as create_option_series_service,
    disable_option_series as disable_option_series_service,
    get_option_execution_intent as get_option_execution_intent_service,
    get_option_fill as get_option_fill_service, get_option_order as get_option_order_service,
    get_option_order_fills as get_option_order_fills_service,
    get_option_orderbook as get_option_orderbook_service, get_option_rfq as get_option_rfq_service,
    get_option_series as get_option_series_service,
    list_option_execution_intents as list_option_execution_intents_service,
    list_option_fills as list_option_fills_service,
    list_option_orders as list_option_orders_service,
    list_option_rfq_quotes as list_option_rfq_quotes_service,
    list_option_rfqs as list_option_rfqs_service, list_option_series as list_option_series_service,
    option_execution_calldata as option_execution_calldata_service,
    option_execution_signing_payload as option_execution_signing_payload_service,
    option_execution_simulation_status as option_execution_simulation_status_service,
    option_rfq_quote_signing_payload as option_rfq_quote_signing_payload_service,
    persist_option_execution_simulation_unavailable, prepare_option_execution_simulation,
    simulate_prepared_option_execution_intent,
    submit_option_execution_signatures as submit_option_execution_signatures_service,
    submit_option_order as submit_option_order_service,
    submit_option_rfq_quote as submit_option_rfq_quote_service, CreateOptionRfqInput,
    CreateOptionSeriesInput, OptionRfqQuoteSigningPayloadInput,
    SubmitOptionExecutionSignaturesInput, SubmitOptionOrderInput, SubmitOptionRfqQuoteInput,
};
use crate::options::{
    list_option_execution_events as list_option_execution_events_service,
    option_execution_intent_id_to_hex_bytes32, option_rfq_id_to_hex_bytes32,
    option_series_id_to_hex_bytes32,
    summarize_option_execution_events as summarize_option_execution_events_service,
    summarize_option_execution_events_by_contract_address as summarize_option_execution_events_by_contract_address_service,
    OptionExecutionIntent, OptionExecutionIntentId, OptionExecutionIntentStatus,
    OptionExecutionSimulationResult, OptionExecutionSimulationStatus, OptionFill, OptionFillFilter,
    OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderStatus, OptionOrderbookSnapshot,
    OptionRfqFill, OptionRfqId, OptionRfqQuote, OptionRfqQuoteId, OptionRfqQuoteSignatureStatus,
    OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus, OptionSeries, OptionSeriesFilter,
    OptionSeriesStatus, OPTION_EVENT_INDEXER_STATE_ID, OPTION_RFQ_QUOTE_TYPE, OPTION_TRADE_TYPE,
};
use crate::orders::service::{
    cancel_order as cancel_order_shared, submit_response_from_events, submit_signed_order,
    CancelOrderInput,
};
use crate::reconciliation::{
    decide_direct_reconciliation, DirectReconciliationInput, ExecutionReconciliation,
    ReconciliationCounts, ReconciliationStatus, ReconciliationTickResult,
};
use crate::rfq::service::{
    accept_quote as accept_rfq_quote, cancel_rfq as cancel_rfq_service,
    create_rfq as create_rfq_service, get_rfq as get_rfq_service,
    list_quotes as list_rfq_quotes_service, list_rfqs as list_rfqs_service,
    quote_signing_payload as rfq_quote_signing_payload, submit_quote as submit_rfq_quote,
    AcceptQuoteOutcome, CreateRfqInput, QuoteSigningPayloadInput, SubmitQuoteInput,
};
use crate::rfq::{
    parse_quote_id, parse_rfq_id, rfq_id_to_hex_bytes32, RfqQuote, RfqQuoteSignatureStatus,
    RfqQuoteStatus, RfqRequest, RfqStatus, RFQ_QUOTE_TYPE,
};
use crate::types::TimeInForce;
use crate::types::{now_ms, AccountId, MarketId, OrderId, Side};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

/// V2G-W2 — admin route gate middleware.
///
/// Runs for every incoming request. For non-`/admin/*` paths, passes
/// through untouched. For `/admin/*` paths:
///
/// 1. Resolve the required role via `required_role_for(method, path)`
///    (V2G-W1 mapping).
/// 2. Call `authenticate(&state.admin_config, …)` to derive the
///    caller's identity under the configured `AuthMode`.
/// 3. Call `require_role(identity, required)` to enforce the
///    minimum-authority gate.
/// 4. Emit a single audit-log line via `tracing::info!` with
///    `target: "deopt.admin.audit"` — method, path, required role,
///    granted role (on allow), identity name, decision, deny reason
///    (on deny), and the configured auth_mode. The audit log NEVER
///    contains the candidate token, the configured token, the
///    `Authorization` header value, or any private-key material.
/// 5. On deny: short-circuit with HTTP 403 + `{"error":"…"}` JSON,
///    matching today's `ensure_admin_access` error payload shape.
/// 6. On allow: pass through to the route handler. Handler-side
///    `ensure_admin_access` continues to run as a defense-in-depth
///    layer; both gates accept the same token + run the same V2G-W0
///    constant-time compare, so the second check is a no-op in the
///    happy path. This dual-check is intentional during the V2G-W2
///    cutover window; V2G-W3 removes the handler-side check after a
///    soak window confirms the middleware is the only authoritative
///    gate.
async fn admin_route_gate(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = request.uri().path().to_string();
    if !path.starts_with("/admin/") {
        return next.run(request).await;
    }

    let method = request.method().clone();
    let method_str = method.as_str();
    let required = required_role_for(method_str, &path);
    let auth_mode = state.admin_config.auth_mode();

    // Build a header lookup closure that doesn't keep references to
    // header values past the borrow scope.
    let headers = request.headers().clone();
    let header_lookup = |name: &str| -> Option<String> {
        headers
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.to_str().ok())
            .map(|s| s.to_string())
    };

    match authenticate(&state.admin_config, header_lookup) {
        Ok(identity) => match require_role(&identity, required) {
            Ok(()) => {
                tracing::info!(
                    target: "deopt.admin.audit",
                    method = %method_str,
                    path = %path,
                    required_role = %required.as_str(),
                    granted_role = %identity.role().as_str(),
                    identity = %identity.name(),
                    decision = %"allow",
                    auth_mode = %auth_mode.as_str(),
                    "admin request allowed"
                );
                next.run(request).await
            }
            Err(err) => admin_audit_deny(&method, &path, required, Some(&identity), auth_mode, err),
        },
        Err(err) => admin_audit_deny(&method, &path, required, None, auth_mode, err),
    }
}

/// V2G-W2 — emit a `deny` audit-log line and build the HTTP 403
/// response in the same shape as `ApiError::forbidden`. Never logs
/// secret material; the `AdminAuthError::Display` impl was hardened
/// in V2G-W1 to redact tokens / JWTs / configured-token bytes.
fn admin_audit_deny(
    method: &axum::http::Method,
    path: &str,
    required: crate::admin::AdminRole,
    identity: Option<&AdminIdentity>,
    auth_mode: crate::admin::AuthMode,
    err: AdminAuthError,
) -> Response {
    let reason = err.to_string();
    if let Some(identity) = identity {
        tracing::warn!(
            target: "deopt.admin.audit",
            method = %method.as_str(),
            path = %path,
            required_role = %required.as_str(),
            granted_role = %identity.role().as_str(),
            identity = %identity.name(),
            decision = %"deny",
            reason = %reason,
            auth_mode = %auth_mode.as_str(),
            "admin request denied (insufficient role)"
        );
    } else {
        tracing::warn!(
            target: "deopt.admin.audit",
            method = %method.as_str(),
            path = %path,
            required_role = %required.as_str(),
            decision = %"deny",
            reason = %reason,
            auth_mode = %auth_mode.as_str(),
            "admin request denied (auth failure)"
        );
    }
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({ "error": reason })),
    )
        .into_response()
}

pub fn router(state: AppState) -> Router {
    let gate_state = state.clone();
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/markets", get(markets))
        .route("/orderbook/:market_id", get(orderbook))
        .route(
            "/options/series",
            post(create_option_series).get(list_option_series),
        )
        .route("/options/series/:option_series_id", get(get_option_series))
        .route(
            "/options/series/:option_series_id/disable",
            post(disable_option_series),
        )
        .route(
            "/options/orderbooks/:option_series_id",
            get(get_option_orderbook),
        )
        .route(
            "/options/rfqs",
            post(create_option_rfq).get(list_option_rfqs),
        )
        .route("/options/rfqs/:option_rfq_id", get(get_option_rfq))
        .route(
            "/options/rfqs/:option_rfq_id/quote-signing-payload",
            post(option_rfq_quote_payload),
        )
        .route(
            "/options/rfqs/:option_rfq_id/quotes",
            post(submit_option_rfq_quote).get(list_option_rfq_quotes),
        )
        .route(
            "/options/rfqs/:option_rfq_id/accept/:quote_id",
            post(accept_option_rfq_quote),
        )
        .route(
            "/options/rfqs/:option_rfq_id/cancel",
            post(cancel_option_rfq),
        )
        .route(
            "/options/orders",
            post(submit_option_order).get(list_option_orders),
        )
        .route(
            "/options/execution-intents",
            get(list_option_execution_intents),
        )
        .route(
            "/options/execution-intents/:intent_id",
            get(get_option_execution_intent),
        )
        .route(
            "/options/execution-intents/:intent_id/signing-payload",
            get(option_execution_signing_payload),
        )
        .route(
            "/options/execution-intents/:intent_id/signatures",
            post(submit_option_execution_signatures),
        )
        .route(
            "/options/execution-intents/:intent_id/calldata",
            get(option_execution_calldata),
        )
        .route(
            "/options/execution-intents/:intent_id/simulate",
            post(simulate_option_execution_intent),
        )
        .route(
            "/options/execution-intents/:intent_id/simulation",
            get(option_execution_simulation),
        )
        .route(
            "/options/execution-intents/:intent_id/broadcast",
            post(broadcast_option_execution_intent),
        )
        .route(
            "/options/execution-intents/:intent_id/confirm",
            post(confirm_option_execution_intent),
        )
        .route("/options/fills", get(list_option_fills))
        .route("/options/fills/:fill_id", get(get_option_fill))
        .route(
            "/options/orders/:order_id/fills",
            get(get_option_order_fills),
        )
        .route("/options/orders/:order_id", get(get_option_order))
        .route(
            "/options/orders/:order_id/cancel",
            post(cancel_option_order),
        )
        .route("/accounts/:address/perp-nonce", get(account_perp_nonce))
        .route("/accounts/:address/option-nonce", get(account_option_nonce))
        .route("/orders", post(submit_order))
        .route("/orders/:order_id", delete(cancel_order))
        .route("/rfqs", post(create_rfq).get(list_rfqs))
        .route("/rfqs/:rfq_id", get(get_rfq))
        .route(
            "/rfqs/:rfq_id/quote-signing-payload",
            post(rfq_quote_payload),
        )
        .route("/rfqs/:rfq_id/quotes", post(submit_quote).get(list_quotes))
        .route("/rfqs/:rfq_id/accept/:quote_id", post(accept_quote))
        .route("/rfqs/:rfq_id/cancel", post(cancel_rfq))
        .route("/execution-intents", get(execution_intents))
        .route(
            "/execution-intents/:intent_id/signing-payload",
            get(execution_intent_signing_payload),
        )
        .route(
            "/execution-intents/:intent_id/signatures",
            post(submit_execution_intent_signatures),
        )
        .route("/executor/status", get(executor_status))
        .route("/executor/health/v2", get(executor_health_v2))
        .route("/executor/tick", post(executor_tick))
        .route(
            "/executor/simulate/:intent_id",
            post(simulate_executor_intent),
        )
        .route(
            "/executor/broadcast/:intent_id",
            post(broadcast_executor_intent),
        )
        .route("/executor/transactions", get(executor_transactions))
        .route(
            "/executor/transactions/:intent_id",
            get(executor_transactions_for_intent),
        )
        .route("/executor/confirmations/status", get(confirmation_status))
        .route(
            "/executor/confirm/:intent_id",
            post(confirm_executor_intent),
        )
        .route("/executor/confirmations/tick", post(confirmation_tick))
        .route(
            "/executor/confirmations/:intent_id",
            get(confirmation_for_intent),
        )
        .route("/indexer/status", get(indexer_status))
        .route("/indexer/tick", post(indexer_tick))
        .route("/indexed/perp-trades", get(indexed_perp_trades))
        .route("/reconciliation/status", get(reconciliation_status))
        .route("/reconciliation/tick", post(reconciliation_tick))
        .route(
            "/reconciliation/intents/:intent_id",
            get(reconciliations_for_intent),
        )
        .route("/reconciliations", get(reconciliations))
        .route("/admin/status", get(admin_status))
        .route("/admin/config", get(admin_config))
        .route("/admin/db", get(admin_db))
        .route(
            "/admin/options/confirmations",
            get(admin_option_confirmations),
        )
        .route("/admin/options/events", get(admin_option_events))
        .route("/admin/options/events/tick", post(admin_option_events_tick))
        .route(
            "/admin/options/reconciliations",
            get(admin_option_reconciliations),
        )
        .route(
            "/admin/options/reconciliations/tick",
            post(admin_option_reconciliations_tick),
        )
        .route(
            "/admin/options/executions/:intent_id/lifecycle",
            get(admin_option_execution_lifecycle),
        )
        .route("/admin/mm/sessions", get(admin_mm_sessions))
        .route("/admin/mm/permissions", get(admin_mm_permissions))
        .route("/admin/execution/summary", get(admin_execution_summary))
        .route("/admin/rfq/summary", get(admin_rfq_summary))
        .route("/admin/options/summary", get(admin_options_summary))
        .route("/admin/fees/summary", get(admin_fee_summary))
        .route("/admin/fees/events", get(admin_fee_events))
        .route("/admin/fees/onchain", get(admin_fee_onchain))
        .route("/admin/fees/volumes", get(admin_fee_volumes))
        .route("/admin/fees/rebates", get(admin_fee_rebates))
        .route(
            "/admin/fees/v2/observability",
            get(admin_fees_v2_observability),
        )
        .route(
            "/admin/fees/v2/smoke/readiness",
            get(admin_fees_v2_smoke_readiness),
        )
        .route("/admin/fees/vault/summary", get(admin_fees_vault_summary))
        .route("/admin/fees/vault/balances", get(admin_fees_vault_balances))
        .route(
            "/admin/fees/vault/reconciliation",
            get(admin_fees_vault_reconciliation),
        )
        .route("/admin/recent", get(admin_recent))
        .layer(axum::middleware::from_fn_with_state(
            gate_state,
            admin_route_gate,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "deopt-v2-backend",
    })
}

async fn ready(State(state): State<AppState>) -> Response {
    let response = readiness(&state).await;
    let status = if response.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response)).into_response()
}

async fn metrics(headers: HeaderMap, State(state): State<AppState>) -> Result<Response, ApiError> {
    ensure_metrics_access(&state, &headers)?;
    let body = render_metrics(&state).await?;
    Ok((
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response())
}

async fn admin_status(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    Ok(Json(serde_json::json!({
        "service": "deopt-v2-backend",
        "ok": true,
        "timestamp_ms": now_ms(),
        "network": state.network_name,
        "chain_id": state.chain_id,
        "persistence_enabled": state.persistence_enabled,
        "execution_enabled": state.execution_config.execution_enabled,
        "real_broadcast_enabled": state.execution_config.real_broadcast_enabled,
        "indexer_enabled": state.indexer_config.enabled,
        "reconciliation_enabled": state.reconciliation_config.enabled,
        "confirmation_enabled": state.confirmation_config.enabled,
        "metrics_enabled": state.metrics_config.enabled,
        "mm_gateway_enabled": state.mm_gateway_config.enabled,
        "mm_permissions_enabled": state.mm_permissions_config.enabled,
        "rfq_enabled": state.rfq_config.enabled,
        "options_enabled": state.options_config.enabled,
        "option_rfq_enabled": state.options_config.rfq_enabled,
        "option_execution_enabled": state.options_config.execution_enabled,
        "option_execution_simulation_enabled": state.options_config.execution_simulation_enabled,
        "option_execution_broadcast_enabled": state.options_config.execution_broadcast_enabled,
        "option_nonce_sync_enabled": state.option_nonce_sync_config.enabled,
        "fees_enabled": state.fees_config.enabled,
        "rebates_enabled": state.fees_config.rebates_enabled
    })))
}

async fn admin_config(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    Ok(Json(serde_json::json!({
        "network": state.network_name,
        "chain_id": state.chain_id,
        "admin": {
            "enabled": state.admin_config.enabled,
            "require_token": state.admin_config.require_token,
            "token_configured": state.admin_config.token_configured()
        },
        "metrics": {
            "enabled": state.metrics_config.enabled,
            "require_admin_token": state.metrics_config.require_admin_token
        },
        "features": {
            "persistence_enabled": state.persistence_enabled,
            "execution_enabled": state.execution_config.execution_enabled,
            "real_broadcast_enabled": state.execution_config.real_broadcast_enabled,
            "simulation_enabled": state.execution_config.simulation_enabled,
            "indexer_enabled": state.indexer_config.enabled,
            "reconciliation_enabled": state.reconciliation_config.enabled,
            "confirmation_enabled": state.confirmation_config.enabled,
            "metrics_enabled": state.metrics_config.enabled,
            "rfq_enabled": state.rfq_config.enabled,
            "options_enabled": state.options_config.enabled,
            "option_rfq_enabled": state.options_config.rfq_enabled,
            "option_execution_enabled": state.options_config.execution_enabled,
            "option_execution_simulation_enabled": state.options_config.execution_simulation_enabled,
            "option_execution_broadcast_enabled": state.options_config.execution_broadcast_enabled,
            "option_event_indexer_enabled": state.option_event_indexer_config.enabled,
        "option_reconciliation_worker_enabled": state.option_reconciliation_config.enabled,
            "option_nonce_sync_enabled": state.option_nonce_sync_config.enabled,
            "fees_enabled": state.fees_config.enabled,
            "rebates_enabled": state.fees_config.rebates_enabled,
            "mm_gateway_enabled": state.mm_gateway_config.enabled,
            "mm_permissions_enabled": state.mm_permissions_config.enabled,
            "perp_nonce_sync_enabled": state.perp_nonce_sync_config.enabled
        },
        "configured": {
            "database": state.database_configured,
            "rpc": state.execution_config.rpc_url.is_some(),
            "executor_private_key": state.execution_config.executor_private_key.is_some()
        },
        "contracts": {
            "executor_from_address": state.execution_config.executor_from_address,
            "perp_matching_engine_address": state.execution_config.perp_matching_engine_address,
            "option_matching_engine_address": state.options_config.matching_engine_address,
            "perp_engine_address": state.execution_config.perp_engine_address,
            "order_eip712_verifying_contract": state.eip712_domain.verifying_contract,
            "rfq_eip712_verifying_contract": state.rfq_config.eip712_domain.verifying_contract,
            "option_rfq_eip712_verifying_contract": state.options_config.rfq_eip712_domain.verifying_contract,
            "option_execution_eip712_verifying_contract": state.options_config.execution_eip712_domain.verifying_contract
        },
        "signatures": {
            "order_mode": state.signature_verification_mode,
            "rfq_quote_mode": state.rfq_config.quote_signature_mode,
            "option_rfq_quote_mode": state.options_config.rfq_quote_signature_mode,
            "option_execution_trade_mode": state.options_config.execution_signature_mode
        },
        "execution": {
            "dry_run": state.execution_config.dry_run,
            "poll_interval_ms": state.execution_config.poll_interval_ms,
            "max_batch_size": state.execution_config.max_batch_size,
            "executor_chain_id": state.execution_config.executor_chain_id,
            "max_gas_limit": state.execution_config.max_gas_limit,
            "require_simulation_ok": state.execution_config.require_simulation_ok,
            "simulation_requires_persistence": state.execution_config.simulation_requires_persistence,
            "max_fee_per_gas_configured": state.execution_config.max_fee_per_gas_wei.is_some(),
            "max_priority_fee_per_gas_configured": state.execution_config.max_priority_fee_per_gas_wei.is_some()
        },
        "mm_gateway": {
            "enabled": state.mm_gateway_config.enabled,
            "transport": mm_gateway_transport_label(&state),
            "host": state.mm_gateway_config.enabled.then_some(state.mm_gateway_config.host.clone()),
            "port": state.mm_gateway_config.enabled.then_some(state.mm_gateway_config.port),
            "cert_configured": state.mm_gateway_config.cert_path.is_some(),
            "key_configured": state.mm_gateway_config.key_path.is_some(),
            "max_sessions": state.mm_gateway_config.max_sessions,
            "max_in_flight_per_session": state.mm_gateway_config.max_in_flight_per_session,
            "rate_limit_per_sec": state.mm_gateway_config.rate_limit_per_sec,
            "heartbeat_timeout_ms": state.mm_gateway_config.heartbeat_timeout_ms,
            "cancel_on_disconnect": state.mm_gateway_config.cancel_on_disconnect,
            "auth_mode": state.mm_gateway_config.auth_mode,
            "require_auth": state.mm_gateway_config.require_auth,
            "challenge_ttl_ms": state.mm_gateway_config.challenge_ttl_ms
        },
        "mm_permissions": {
            "enabled": state.mm_permissions_config.enabled,
            "require_persistence": state.mm_permissions_config.require_persistence
        },
        "fees": {
            "enabled": state.fees_config.enabled,
            "require_persistence": state.fees_config.require_persistence,
            "rebates_enabled": state.fees_config.rebates_enabled,
            "protocol_fee_recipient": state.fees_config.protocol_fee_recipient.clone(),
            "default_fee_asset": state.fees_config.default_fee_asset.clone(),
            "option_fee_basis": state.fees_config.option_fee_basis.as_str(),
            "option_premium_cap_pct": state.fees_config.option_premium_cap_pct,
            "rate_unit": "micro_bps"
        },
        "rfq": {
            "enabled": state.rfq_config.enabled,
            "require_persistence": state.rfq_config.require_persistence,
            "default_ttl_ms": state.rfq_config.default_ttl_ms,
            "max_ttl_ms": state.rfq_config.max_ttl_ms,
            "min_quote_ttl_ms": state.rfq_config.min_quote_ttl_ms,
            "max_quote_ttl_ms": state.rfq_config.max_quote_ttl_ms,
            "max_quotes_per_rfq": state.rfq_config.max_quotes_per_rfq,
            "eip712_name": state.rfq_config.eip712_domain.name,
            "eip712_version": state.rfq_config.eip712_domain.version,
            "eip712_chain_id": state.rfq_config.eip712_domain.chain_id
        },
        "options": {
            "enabled": state.options_config.enabled,
            "require_persistence": state.options_config.require_persistence,
            "allow_manual_series": state.options_config.allow_manual_series,
            "sync_onchain_registry": state.options_config.sync_onchain_registry,
            "default_contract_size_1e8": state.options_config.default_contract_size_1e8.to_string(),
            "rfq_enabled": state.options_config.rfq_enabled,
            "rfq_require_persistence": state.options_config.rfq_require_persistence,
            "rfq_default_ttl_ms": state.options_config.rfq_default_ttl_ms,
            "rfq_max_ttl_ms": state.options_config.rfq_max_ttl_ms,
            "rfq_min_quote_ttl_ms": state.options_config.rfq_min_quote_ttl_ms,
            "rfq_max_quote_ttl_ms": state.options_config.rfq_max_quote_ttl_ms,
            "rfq_max_quotes_per_rfq": state.options_config.rfq_max_quotes_per_rfq,
            "rfq_eip712_name": state.options_config.rfq_eip712_domain.name,
            "rfq_eip712_version": state.options_config.rfq_eip712_domain.version,
            "rfq_eip712_chain_id": state.options_config.rfq_eip712_domain.chain_id,
            "execution_enabled": state.options_config.execution_enabled,
            "execution_require_persistence": state.options_config.execution_require_persistence,
            "execution_signature_mode": state.options_config.execution_signature_mode,
            "execution_default_settlement_decimals": state.options_config.execution_default_settlement_decimals,
            "execution_simulation_enabled": state.options_config.execution_simulation_enabled,
            "execution_require_rpc_for_simulation": state.options_config.execution_require_rpc_for_simulation,
            "execution_simulation_gas_limit": state.options_config.execution_simulation_gas_limit,
            "execution_simulation_from": state.options_config.execution_simulation_from,
            "execution_simulation_rpc_configured": state.options_config.execution_simulation_rpc_url.is_some(),
            "execution_broadcast_enabled": state.options_config.execution_broadcast_enabled,
            "execution_require_simulation_ok": state.options_config.execution_require_simulation_ok,
            "execution_broadcast_gas_limit": state.options_config.execution_broadcast_gas_limit,
            "execution_gas_safety_bps": state.options_config.execution_gas_safety_bps,
            "option_nonce_sync_enabled": state.option_nonce_sync_config.enabled,
            "option_nonce_sync_require_rpc": state.option_nonce_sync_config.require_rpc,
            "option_nonce_sync_strict": state.option_nonce_sync_config.strict,
            "option_nonce_sync_rpc_configured": state.option_nonce_sync_config.rpc_url.is_some(),
            "execution_eip712_name": state.options_config.execution_eip712_domain.name,
            "execution_eip712_version": state.options_config.execution_eip712_domain.version,
            "execution_eip712_chain_id": state.options_config.execution_eip712_domain.chain_id,
            "confirmation_worker": {
                "enabled": state.option_confirmation_config.enabled,
                "poll_interval_ms": state.option_confirmation_config.poll_interval_ms,
                "finality_blocks": state.option_confirmation_config.finality_blocks,
                "batch_size": state.option_confirmation_config.batch_size,
                "require_rpc": state.option_confirmation_config.require_rpc,
                "rpc_configured": state.option_confirmation_config.rpc_url.is_some()
            },
            "event_indexer": {
                "enabled": state.option_event_indexer_config.enabled,
                "poll_interval_ms": state.option_event_indexer_config.poll_interval_ms,
                "from_block": state.option_event_indexer_config.from_block,
                "batch_blocks": state.option_event_indexer_config.batch_blocks,
                "confirmation_blocks": state.option_event_indexer_config.confirmation_blocks,
                "require_rpc": state.option_event_indexer_config.require_rpc,
                "rpc_configured": state.option_event_indexer_config.rpc_url.is_some(),
                "target_contract": state.option_event_indexer_config.matching_engine_address,
                "emitter_contracts": state.option_event_indexer_config.emitter_contracts(),
                "matching_engine_address": state.option_event_indexer_config.matching_engine_address,
                "margin_engine_address": state.option_event_indexer_config.margin_engine_address,
                "collateral_vault_address": state.option_event_indexer_config.collateral_vault_address,
                "fees_manager_address": state.option_event_indexer_config.fees_manager_address,
                "fees_manager_v2_address": state.option_event_indexer_config.fees_manager_v2_address
            },
            "reconciliation_worker": {
                "enabled": state.option_reconciliation_config.enabled,
                "poll_interval_ms": state.option_reconciliation_config.poll_interval_ms,
                "batch_size": state.option_reconciliation_config.batch_size,
                "require_events": state.option_reconciliation_config.require_events,
                "require_rpc": state.option_reconciliation_config.require_rpc,
                "strict": state.option_reconciliation_config.strict,
                "state_checks_enabled": state.option_reconciliation_config.state_checks_enabled,
                "state_checks_require_rpc": state.option_reconciliation_config.state_checks_require_rpc,
                "state_checks_strict": state.option_reconciliation_config.state_checks_strict,
                "rpc_configured": state.option_reconciliation_config.rpc_url.is_some()
            }
        }
    })))
}

async fn admin_db(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let Some(repository) = state.repository.clone() else {
        return Ok(Json(serde_json::json!({
            "persistence_enabled": false,
            "database_configured": state.database_configured,
            "connected": false,
            "migrations": {
                "available": false,
                "installed_count": 0,
                "latest_version": null
            },
            "counts": {}
        })));
    };

    repository.admin_ping().await?;
    Ok(Json(serde_json::json!({
        "persistence_enabled": true,
        "database_configured": state.database_configured,
        "connected": true,
        "migrations": repository.admin_migration_status().await?,
        "counts": repository.admin_table_counts().await?
    })))
}

async fn admin_option_confirmations(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let counts: serde_json::Map<String, serde_json::Value> =
        if let Some(repository) = state.repository.clone() {
            repository
                .summarize_option_execution_confirmations()
                .await?
                .into_iter()
                .map(|(status, count)| (status, serde_json::Value::from(count)))
                .collect()
        } else {
            state
                .options_store
                .lock()
                .map_err(|_| ApiError::internal())?
                .summarize_option_execution_confirmations()
                .into_iter()
                .map(|(status, count)| (status, serde_json::Value::from(count)))
                .collect()
        };
    let latest_tick = state
        .option_confirmation_last_tick
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    Ok(Json(serde_json::json!({
        "config": {
            "enabled": state.option_confirmation_config.enabled,
            "poll_interval_ms": state.option_confirmation_config.poll_interval_ms,
            "finality_blocks": state.option_confirmation_config.finality_blocks,
            "batch_size": state.option_confirmation_config.batch_size,
            "require_rpc": state.option_confirmation_config.require_rpc,
            "rpc_configured": state.option_confirmation_config.rpc_url.is_some()
        },
        "counts": serde_json::Value::Object(counts),
        "latest_tick": latest_tick,
    })))
}

async fn admin_option_events(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let counts_by_event_name = summarize_option_execution_events_service(&state).await?;
    let counts_by_contract_address =
        summarize_option_execution_events_by_contract_address_service(&state).await?;
    let recent = list_option_execution_events_service(&state, 20).await?;
    let latest_tick = state
        .option_event_indexer_last_tick
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    let cursor_state = if let Some(repository) = state.repository.clone() {
        repository
            .get_option_event_indexer_state(OPTION_EVENT_INDEXER_STATE_ID)
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| ApiError::internal())?
            .get_option_event_indexer_state(OPTION_EVENT_INDEXER_STATE_ID)
    };
    let last_indexed_block = cursor_state
        .as_ref()
        .map(|state| state.last_indexed_block)
        .unwrap_or(state.option_event_indexer_config.from_block);
    let last_error = cursor_state.and_then(|state| state.last_error);

    Ok(Json(serde_json::json!({
        "indexer_enabled": state.option_event_indexer_config.enabled,
        "from_block": state.option_event_indexer_config.from_block,
        "poll_interval_ms": state.option_event_indexer_config.poll_interval_ms,
        "batch_blocks": state.option_event_indexer_config.batch_blocks,
        "confirmation_blocks": state.option_event_indexer_config.confirmation_blocks,
        "require_rpc": state.option_event_indexer_config.require_rpc,
        "rpc_configured": state.option_event_indexer_config.rpc_url.is_some(),
        "target_contract": state.option_event_indexer_config.matching_engine_address,
        "emitter_contracts": state.option_event_indexer_config.emitter_contracts(),
        "counts_by_event_name": counts_by_event_name.clone(),
        "counts_by_contract_address": counts_by_contract_address,
        "last_indexed_block": last_indexed_block,
        "last_error": last_error,
        "config": {
            "enabled": state.option_event_indexer_config.enabled,
            "from_block": state.option_event_indexer_config.from_block,
            "poll_interval_ms": state.option_event_indexer_config.poll_interval_ms,
            "batch_blocks": state.option_event_indexer_config.batch_blocks,
            "confirmation_blocks": state.option_event_indexer_config.confirmation_blocks,
            "require_rpc": state.option_event_indexer_config.require_rpc,
            "rpc_configured": state.option_event_indexer_config.rpc_url.is_some(),
            "target_contract": state.option_event_indexer_config.matching_engine_address,
            "emitter_contracts": state.option_event_indexer_config.emitter_contracts(),
            "matching_engine_address": state.option_event_indexer_config.matching_engine_address,
            "margin_engine_address": state.option_event_indexer_config.margin_engine_address,
            "collateral_vault_address": state.option_event_indexer_config.collateral_vault_address,
            "fees_manager_address": state.option_event_indexer_config.fees_manager_address,
            "fees_manager_v2_address": state.option_event_indexer_config.fees_manager_v2_address
        },
        "latest_tick": latest_tick,
        "counts": counts_by_event_name,
        "recent": recent
    })))
}

async fn admin_option_events_tick(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<crate::options::OptionEventIndexerTickResult>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    if !state.option_event_indexer_config.enabled {
        return Err(BackendError::Config("option event indexer is disabled".to_string()).into());
    }
    let rpc_url = state
        .option_event_indexer_config
        .rpc_url
        .clone()
        .ok_or_else(|| {
            BackendError::Config(
                "option event indexer requires RPC_URL to run a one-shot tick".to_string(),
            )
        })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    let result = crate::options::index_option_events_with_provider(&state, &provider).await?;
    Ok(Json(result))
}

async fn admin_option_reconciliations(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let counts: serde_json::Map<String, serde_json::Value> =
        crate::options::summarize_option_execution_reconciliations(&state)
            .await?
            .into_iter()
            .map(|(status, count)| (status, serde_json::Value::from(count)))
            .collect();
    let recent = if let Some(repository) = state.repository.clone() {
        repository.list_option_execution_reconciliations(20).await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| ApiError::internal())?
            .list_option_execution_reconciliations(20)
    };
    let check_counts = option_reconciliation_check_counts(&recent);
    let latest_tick = state
        .option_reconciliation_last_tick
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    Ok(Json(serde_json::json!({
        "config": {
            "enabled": state.option_reconciliation_config.enabled,
            "poll_interval_ms": state.option_reconciliation_config.poll_interval_ms,
            "batch_size": state.option_reconciliation_config.batch_size,
            "require_events": state.option_reconciliation_config.require_events,
            "require_rpc": state.option_reconciliation_config.require_rpc,
            "strict": state.option_reconciliation_config.strict,
            "state_checks_enabled": state.option_reconciliation_config.state_checks_enabled,
            "state_checks_require_rpc": state.option_reconciliation_config.state_checks_require_rpc,
            "state_checks_strict": state.option_reconciliation_config.state_checks_strict,
            "rpc_configured": state.option_reconciliation_config.rpc_url.is_some()
        },
        "counts": serde_json::Value::Object(counts),
        "check_counts": check_counts,
        "latest_tick": latest_tick,
        "recent": recent,
    })))
}

fn option_reconciliation_check_counts(
    rows: &[crate::options::OptionExecutionReconciliation],
) -> serde_json::Value {
    let mut state_check_status = BTreeMap::<String, u64>::new();
    let mut nonce_check_status = BTreeMap::<String, u64>::new();
    let mut position_check_status = BTreeMap::<String, u64>::new();
    let mut vault_check_status = BTreeMap::<String, u64>::new();
    let fee_check_status = BTreeMap::<String, u64>::new();
    let premium_check_status = BTreeMap::<String, u64>::new();

    for row in rows {
        let state_checks = row.details.get("state_checks");
        increment_count(
            &mut state_check_status,
            state_checks
                .and_then(|value| value.get("overall_status"))
                .and_then(|value| value.as_str())
                .unwrap_or("missing"),
        );
        increment_count(
            &mut nonce_check_status,
            state_checks
                .and_then(|value| value.get("nonce_check_status"))
                .and_then(|value| value.as_str())
                .unwrap_or("missing"),
        );
        increment_count(
            &mut position_check_status,
            state_checks
                .and_then(|value| value.get("position_check_status"))
                .and_then(|value| value.as_str())
                .unwrap_or("missing"),
        );
        increment_count(
            &mut vault_check_status,
            state_checks
                .and_then(|value| value.get("vault_check_status"))
                .and_then(|value| value.as_str())
                .unwrap_or("missing"),
        );
    }

    serde_json::json!({
        "state_check_status": state_check_status,
        "nonce_check_status": nonce_check_status,
        "position_check_status": position_check_status,
        "vault_check_status": vault_check_status,
        "fee_check_status": fee_check_status,
        "premium_check_status": premium_check_status,
        "scope": "recent"
    })
}

fn increment_count(counts: &mut BTreeMap<String, u64>, bucket: &str) {
    *counts.entry(bucket.to_string()).or_default() += 1;
}

async fn admin_option_reconciliations_tick(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<crate::options::OptionReconciliationTickResult>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    if !state.option_reconciliation_config.enabled {
        return Err(
            BackendError::Config("option reconciliation worker is disabled".to_string()).into(),
        );
    }
    let result = crate::options::reconcile_option_executions(&state).await?;
    Ok(Json(result))
}

async fn admin_option_execution_lifecycle(
    headers: HeaderMap,
    Path(intent_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<crate::options::OptionExecutionLifecycle>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let parsed_intent_id =
        Uuid::parse_str(&intent_id).map_err(|_| BackendError::InvalidOptionExecutionIntentId)?;
    let view = crate::options::get_option_execution_lifecycle(&state, parsed_intent_id).await?;
    Ok(Json(view))
}

async fn admin_mm_sessions(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    if !state.mm_gateway_config.enabled {
        return Ok(Json(serde_json::json!({
            "enabled": false,
            "sessions": []
        })));
    }

    let sessions = state
        .mm_sessions
        .list_active()?
        .into_iter()
        .map(|session| {
            serde_json::json!({
                "session_id": session.session_id,
                "authenticated": session.authenticated,
                "account": session.account,
                "auth_mode": session.auth_mode,
                "challenge_active": session.challenge_active,
                "challenge_expires_at_ms": session.challenge_expires_at_ms,
                "connected_at_ms": session.connected_at_ms,
                "last_heartbeat_at_ms": session.last_heartbeat_at_ms,
                "open_client_order_ids_count": session.open_client_order_ids.len(),
                "cancel_on_disconnect": session.cancel_on_disconnect
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "enabled": true,
        "sessions": sessions
    })))
}

async fn admin_mm_permissions(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let product_permissions = list_product_permissions(&state).await?;
    let mut scopes_by_account = BTreeMap::<String, Vec<MmProductPermission>>::new();
    for permission in product_permissions {
        scopes_by_account
            .entry(permission.mm_account.0.to_ascii_lowercase())
            .or_default()
            .push(permission);
    }

    let accounts = list_permission_accounts(&state)
        .await?
        .into_iter()
        .map(|account| {
            let scopes = scopes_by_account
                .remove(&account.mm_account.0.to_ascii_lowercase())
                .unwrap_or_default()
                .into_iter()
                .map(|permission| {
                    serde_json::json!({
                        "id": permission.id,
                        "market_id": permission.market_id,
                        "option_series_id": permission.option_series_id,
                        "enabled": permission.enabled
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "mm_account": account.mm_account,
                "enabled": account.enabled,
                "label": account.label,
                "can_submit_perp_orders": account.can_submit_perp_orders,
                "can_quote_perp_rfq": account.can_quote_perp_rfq,
                "can_quote_option_rfq": account.can_quote_option_rfq,
                "can_submit_option_orders": account.can_submit_option_orders,
                "market_permissions": scopes
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(serde_json::json!({
        "enabled": state.mm_permissions_config.enabled,
        "require_persistence": state.mm_permissions_config.require_persistence,
        "persistence_enabled": state.persistence_enabled,
        "accounts": accounts
    })))
}

async fn admin_execution_summary(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    if let Some(repository) = state.repository.clone() {
        let intent_status_counts = repository
            .admin_count_by_column("execution_intents", "status")
            .await?;
        let simulation_status_counts = repository
            .admin_count_by_column("execution_simulations", "status")
            .await?;
        let transaction_status_counts = repository
            .admin_count_by_column("execution_transactions", "status")
            .await?;
        let calldata_ready_intents = repository
            .admin_count_where(
                "execution_intent_signatures",
                "buyer_sig IS NOT NULL AND seller_sig IS NOT NULL",
            )
            .await?;
        let submitted_transactions = repository
            .admin_count_where("execution_transactions", "status = 'submitted'")
            .await?;
        let confirmed_transactions = repository
            .admin_count_where(
                "execution_transactions",
                "confirmation_status = 'confirmed'",
            )
            .await?;
        let recent_unconfirmed_txs = repository
            .list_submitted_unconfirmed_execution_transactions(10)
            .await?
            .into_iter()
            .map(compact_execution_transaction)
            .collect::<Vec<_>>();
        return Ok(Json(serde_json::json!({
            "persistence_enabled": true,
            "intent_status_counts": intent_status_counts,
            "pending_execution_intents": count_from_map(&intent_status_counts, "pending"),
            "calldata_ready_intents": calldata_ready_intents,
            "simulation_status_counts": simulation_status_counts,
            "simulation_ok": count_from_map(&simulation_status_counts, "simulation_ok"),
            "simulation_failed": count_from_map(&simulation_status_counts, "simulation_failed"),
            "transaction_status_counts": transaction_status_counts,
            "submitted_transactions": submitted_transactions,
            "confirmed_transactions": confirmed_transactions,
            "recent_failed_simulations": repository.admin_recent_failed_simulations(10).await?,
            "recent_unconfirmed_transactions": recent_unconfirmed_txs,
            "recent_confirmation_errors": repository.admin_recent_confirmation_errors(10).await?
        })));
    }

    let intents = state
        .engine
        .lock()
        .map_err(|_| ApiError::internal())?
        .execution_intents();
    let mut intent_status_counts = BTreeMap::new();
    for intent in &intents {
        bump_count(
            &mut intent_status_counts,
            execution_status_key(intent.status),
        );
    }
    let calldata_ready_intents = state
        .trade_signatures
        .lock()
        .map_err(|_| ApiError::internal())?
        .values()
        .filter(|signatures| signatures.calldata_ready())
        .count();
    Ok(Json(serde_json::json!({
        "persistence_enabled": false,
        "intent_status_counts": intent_status_counts,
        "pending_execution_intents": intents.iter().filter(|intent| intent.status == ExecutionIntentStatus::Pending).count(),
        "calldata_ready_intents": calldata_ready_intents,
        "simulation_status_counts": {},
        "simulation_ok": 0,
        "simulation_failed": 0,
        "transaction_status_counts": {},
        "submitted_transactions": 0,
        "confirmed_transactions": 0,
        "recent_failed_simulations": [],
        "recent_unconfirmed_transactions": [],
        "recent_confirmation_errors": []
    })))
}

async fn admin_rfq_summary(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    if let Some(repository) = state.repository.clone() {
        let rfq_status_counts = repository.admin_count_by_column("rfqs", "status").await?;
        let quote_status_counts = repository
            .admin_count_by_column("rfq_quotes", "status")
            .await?;
        let quote_signature_status_counts = repository
            .admin_count_by_column("rfq_quotes", "signature_status")
            .await?;
        let recent_rfqs = repository.admin_recent_rfqs(20).await?;
        let recent_accepted_rfqs = recent_rfqs
            .iter()
            .filter(|rfq| rfq["status"] == "accepted")
            .cloned()
            .collect::<Vec<_>>();
        return Ok(Json(serde_json::json!({
            "persistence_enabled": true,
            "enabled": state.rfq_config.enabled,
            "rfq_status_counts": rfq_status_counts,
            "quote_status_counts": quote_status_counts,
            "quote_signature_status_counts": quote_signature_status_counts,
            "verified_quotes": count_from_map(&quote_signature_status_counts, "verified"),
            "recent_accepted_rfqs": recent_accepted_rfqs,
            "recent_rfqs": recent_rfqs
        })));
    }

    let now = now_ms();
    let store = state.rfq_store.lock().map_err(|_| ApiError::internal())?;
    let rfqs = store.list_rfqs();
    let mut rfq_status_counts = BTreeMap::new();
    let mut quote_status_counts = BTreeMap::new();
    let mut quote_signature_status_counts = BTreeMap::new();
    for rfq in &rfqs {
        bump_count(&mut rfq_status_counts, rfq.effective_status(now).as_str());
        for quote in store.list_quotes(rfq.rfq_id) {
            bump_count(
                &mut quote_status_counts,
                quote.effective_status(now).as_str(),
            );
            bump_count(
                &mut quote_signature_status_counts,
                quote.signature_status.as_str(),
            );
        }
    }
    let recent_accepted_rfqs = rfqs
        .iter()
        .rev()
        .filter(|rfq| rfq.status == RfqStatus::Accepted)
        .take(20)
        .map(compact_rfq)
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "persistence_enabled": false,
        "enabled": state.rfq_config.enabled,
        "rfq_status_counts": rfq_status_counts,
        "quote_status_counts": quote_status_counts,
        "quote_signature_status_counts": quote_signature_status_counts,
        "verified_quotes": count_from_map(&quote_signature_status_counts, "verified"),
        "recent_accepted_rfqs": recent_accepted_rfqs,
        "recent_rfqs": rfqs.iter().rev().take(20).map(compact_rfq).collect::<Vec<_>>()
    })))
}

async fn admin_options_summary(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    if let Some(repository) = state.repository.clone() {
        let series_status_counts = repository
            .admin_count_by_column("option_series", "status")
            .await?;
        let order_status_counts = repository
            .admin_count_by_column("option_orders", "status")
            .await?;
        let option_rfq_status_counts = repository
            .admin_count_by_column("option_rfqs", "status")
            .await?;
        let option_rfq_quote_status_counts = repository
            .admin_count_by_column("option_rfq_quotes", "status")
            .await?;
        let option_rfq_quote_signature_status_counts = repository
            .admin_count_by_column("option_rfq_quotes", "signature_status")
            .await?;
        let option_execution_intent_status_counts = repository
            .admin_count_by_column("option_execution_intents", "status")
            .await?;
        let option_execution_transaction_status_counts = repository
            .admin_count_by_column("option_execution_transactions", "status")
            .await
            .unwrap_or_default();
        let mut option_execution_simulation_status_counts = repository
            .admin_count_by_column("option_execution_intents", "simulation_status")
            .await?;
        if let Some(pending) = option_execution_simulation_status_counts.remove("null") {
            *option_execution_simulation_status_counts
                .entry("simulation_pending".to_string())
                .or_default() += pending;
        }
        return Ok(Json(serde_json::json!({
            "persistence_enabled": true,
            "enabled": state.options_config.enabled,
            "option_rfq_enabled": state.options_config.rfq_enabled,
            "option_execution_enabled": state.options_config.execution_enabled,
            "option_execution_simulation_enabled": state.options_config.execution_simulation_enabled,
            "option_execution_broadcast_enabled": state.options_config.execution_broadcast_enabled,
            "series_status_counts": series_status_counts,
            "order_status_counts": order_status_counts,
            "option_fills_count": repository.admin_count_where("option_fills", "TRUE").await?,
            "option_rfq_status_counts": option_rfq_status_counts,
            "option_rfq_quote_status_counts": option_rfq_quote_status_counts,
            "option_rfq_quote_signature_status_counts": option_rfq_quote_signature_status_counts,
            "verified_option_rfq_quotes": count_from_map(&option_rfq_quote_signature_status_counts, "verified"),
            "option_rfq_fills_count": repository.admin_count_where("option_rfq_fills", "TRUE").await?,
            "option_execution_intent_status_counts": option_execution_intent_status_counts,
            "option_execution_simulation_status_counts": option_execution_simulation_status_counts,
            "option_execution_transaction_status_counts": option_execution_transaction_status_counts,
            "option_execution_intents_count": repository.admin_count_where("option_execution_intents", "TRUE").await?,
            "option_execution_transactions_count": repository.admin_count_where("option_execution_transactions", "TRUE").await.unwrap_or_default(),
            "option_execution_calldata_ready": count_from_map(&option_execution_intent_status_counts, "calldata_ready"),
            "option_execution_pending_signatures": count_from_map(&option_execution_intent_status_counts, "signatures_required"),
            "option_execution_broadcast_submitted": count_from_map(&option_execution_intent_status_counts, "broadcast_submitted"),
            "option_execution_simulation_ok": count_from_map(&option_execution_simulation_status_counts, "simulation_ok"),
            "option_execution_simulation_failed": count_from_map(&option_execution_simulation_status_counts, "simulation_failed"),
            "option_execution_simulation_unavailable": count_from_map(&option_execution_simulation_status_counts, "simulation_unavailable"),
            "recent_option_execution_intents": repository.admin_recent_option_execution_intents(20).await?,
            "recent_option_rfq_fills": repository.admin_recent_option_rfq_fills(20).await?,
            "recent_option_order_fills": repository.admin_recent_option_fills(20).await?
        })));
    }

    let now = now_ms();
    let now_sec = u64::try_from(now / 1000).unwrap_or_default();
    let store = state
        .options_store
        .lock()
        .map_err(|_| ApiError::internal())?;
    let series = store.list_series(&OptionSeriesFilter::default(), now_sec);
    let orders = store.list_orders(&OptionOrderFilter::default());
    let fills = store.list_fills(&OptionFillFilter::default());
    let option_rfqs = store.list_option_rfqs();
    let option_rfq_fills = store.list_option_rfq_fills();
    let option_execution_intents = store.list_option_execution_intents();
    let mut series_status_counts = BTreeMap::new();
    let mut order_status_counts = BTreeMap::new();
    let mut option_rfq_status_counts = BTreeMap::new();
    let mut option_rfq_quote_status_counts = BTreeMap::new();
    let mut option_rfq_quote_signature_status_counts = BTreeMap::new();
    let mut option_execution_intent_status_counts = BTreeMap::new();
    let mut option_execution_simulation_status_counts = BTreeMap::new();
    for item in &series {
        bump_count(
            &mut series_status_counts,
            item.effective_status(now_sec).as_str(),
        );
    }
    for order in &orders {
        bump_count(&mut order_status_counts, order.status.as_str());
    }
    for rfq in &option_rfqs {
        bump_count(
            &mut option_rfq_status_counts,
            rfq.effective_status(now).as_str(),
        );
        for quote in store.list_option_rfq_quotes(rfq.option_rfq_id) {
            bump_count(
                &mut option_rfq_quote_status_counts,
                quote.effective_status(now).as_str(),
            );
            bump_count(
                &mut option_rfq_quote_signature_status_counts,
                quote.signature_status.as_str(),
            );
        }
    }
    for intent in &option_execution_intents {
        bump_count(
            &mut option_execution_intent_status_counts,
            intent.status.as_str(),
        );
        bump_count(
            &mut option_execution_simulation_status_counts,
            intent
                .simulation_status
                .unwrap_or(OptionExecutionSimulationStatus::SimulationPending)
                .as_str(),
        );
    }
    Ok(Json(serde_json::json!({
        "persistence_enabled": false,
        "enabled": state.options_config.enabled,
        "option_rfq_enabled": state.options_config.rfq_enabled,
        "option_execution_enabled": state.options_config.execution_enabled,
        "option_execution_simulation_enabled": state.options_config.execution_simulation_enabled,
        "option_execution_broadcast_enabled": state.options_config.execution_broadcast_enabled,
        "series_status_counts": series_status_counts,
        "order_status_counts": order_status_counts,
        "option_fills_count": fills.len(),
        "option_rfq_status_counts": option_rfq_status_counts,
        "option_rfq_quote_status_counts": option_rfq_quote_status_counts,
        "option_rfq_quote_signature_status_counts": option_rfq_quote_signature_status_counts,
        "verified_option_rfq_quotes": count_from_map(&option_rfq_quote_signature_status_counts, "verified"),
        "option_rfq_fills_count": option_rfq_fills.len(),
        "option_execution_intent_status_counts": option_execution_intent_status_counts,
        "option_execution_simulation_status_counts": option_execution_simulation_status_counts,
        "option_execution_transaction_status_counts": {},
        "option_execution_intents_count": option_execution_intents.len(),
        "option_execution_transactions_count": 0,
        "option_execution_calldata_ready": count_from_map(&option_execution_intent_status_counts, "calldata_ready"),
        "option_execution_pending_signatures": count_from_map(&option_execution_intent_status_counts, "signatures_required"),
        "option_execution_broadcast_submitted": count_from_map(&option_execution_intent_status_counts, "broadcast_submitted"),
        "option_execution_simulation_ok": count_from_map(&option_execution_simulation_status_counts, "simulation_ok"),
        "option_execution_simulation_failed": count_from_map(&option_execution_simulation_status_counts, "simulation_failed"),
        "option_execution_simulation_unavailable": count_from_map(&option_execution_simulation_status_counts, "simulation_unavailable"),
        "recent_option_execution_intents": option_execution_intents.iter().rev().take(20).map(compact_option_execution_intent).collect::<Vec<_>>(),
        "recent_option_rfq_fills": option_rfq_fills.iter().rev().take(20).map(compact_option_rfq_fill).collect::<Vec<_>>(),
        "recent_option_order_fills": fills.iter().rev().take(20).map(compact_option_fill).collect::<Vec<_>>()
    })))
}

async fn admin_fee_summary(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    Ok(Json(admin_fee_summary_service(&state).await?))
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct AdminFeeEventsQuery {
    limit: Option<u32>,
}

async fn admin_fee_events(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<AdminFeeEventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let limit = query.limit.unwrap_or(20).min(100);
    Ok(Json(serde_json::json!({
        "limit": limit,
        "events": admin_fee_events_service(&state, limit).await?
    })))
}

#[derive(Clone, Debug, Default, Deserialize)]
struct AdminFeeOnchainQuery {
    tx_hash: Option<String>,
    limit: Option<u32>,
}

async fn admin_fee_onchain(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<AdminFeeOnchainQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let limit = query.limit.unwrap_or(50);
    let tx_hash = query
        .tx_hash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    Ok(Json(
        admin_onchain_fees_service(&state, tx_hash, limit).await?,
    ))
}

#[derive(Clone, Debug, Deserialize)]
struct AdminFeeAccountQuery {
    account: Option<AccountId>,
}

async fn admin_fee_volumes(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<AdminFeeAccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let account = query.account;
    let volumes = admin_fee_volumes_service(&state, account.clone()).await?;
    Ok(Json(serde_json::json!({
        "account": account,
        "volumes": volumes
    })))
}

async fn admin_fee_rebates(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<AdminFeeAccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let account = query.account;
    let rebates = admin_fee_rebates_service(&state, account.clone()).await?;
    Ok(Json(serde_json::json!({
        "account": account,
        "rebates": rebates
    })))
}

/// V2G-G: read-only V2 fee observability snapshot. Surfaces the same
/// data the `/metrics` endpoint exposes (PERP + OPTION FeeChargedV2 /
/// FeeRebatedV2 by consumer bucket, FeesManagerV2 rebate budget per
/// settlement asset) plus the configured NEW / OLD engine addresses
/// the classifier is using. Useful for a quick one-shot operator
/// check without needing to scrape Prometheus.
///
/// See `docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md`.
async fn admin_fees_v2_observability(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    Ok(Json(admin_v2_observability_service(&state).await?))
}

/// V2G-M: read-only V2 fee smoke readiness snapshot. Surfaces the
/// V2G-D2 EOA registry, the per-tier fee profile snapshot, the
/// canonical dry-run packet templates, and the broadcast-gate state
/// (with `safe_to_broadcast_today=false` during the soak window).
///
/// Never embeds a private key. The env-var NAMES (`PERP_SMOKE_BUYER_PRIVATE_KEY`
/// etc.) are surfaced so the operator can wire signing flows; whether
/// each name resolves to a non-empty value is reported as a boolean.
///
/// See `docs/V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md`.
async fn admin_fees_v2_smoke_readiness(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    Ok(Json(admin_v2_smoke_readiness_service(&state).await?))
}

/// V2G-R5-OBS-P0 — build the per-request `VaultObservabilityConfig`
/// from existing backend config + env. Centralised so the three vault
/// handlers + the monitoring exporter agree on configuration source.
async fn vault_observability_config(state: &AppState) -> vault_obs::VaultObservabilityConfig {
    let rebate_budget_assets = load_rebate_budget_assets(state).await;
    vault_obs::build_config(
        state.execution_config.rpc_url.clone(),
        state
            .option_event_indexer_config
            .protocol_fee_vault_address
            .clone(),
        Some(
            state
                .option_event_indexer_config
                .collateral_vault_address
                .clone(),
        ),
        state
            .option_event_indexer_config
            .fees_manager_v2_address
            .clone(),
        rebate_budget_assets,
    )
}

async fn load_rebate_budget_assets(state: &AppState) -> Vec<String> {
    if let Some(repository) = state.repository.clone() {
        if repository.admin_ping().await.is_ok() {
            if let Ok(map) = repository
                .admin_fees_manager_v2_rebate_budget_by_asset()
                .await
            {
                return map.into_keys().collect();
            }
        }
    }
    state
        .options_store
        .lock()
        .map(|s| {
            s.fees_manager_v2_rebate_budget_by_asset()
                .into_keys()
                .collect()
        })
        .unwrap_or_default()
}

async fn load_rebate_budget_map(state: &AppState) -> std::collections::BTreeMap<String, u64> {
    if let Some(repository) = state.repository.clone() {
        if repository.admin_ping().await.is_ok() {
            if let Ok(map) = repository
                .admin_fees_manager_v2_rebate_budget_by_asset()
                .await
            {
                return map;
            }
        }
    }
    state
        .options_store
        .lock()
        .map(|s| s.fees_manager_v2_rebate_budget_by_asset())
        .unwrap_or_default()
}

/// V2G-R5-OBS-P0: read-only summary endpoint. Safe to call when the
/// vault is not yet deployed — returns `configured=false` + a reason.
async fn admin_fees_vault_summary(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let cfg = vault_observability_config(&state).await;
    let budget = load_rebate_budget_map(&state).await;
    let snapshot = vault_obs::read_snapshot(&cfg, &budget).await?;
    Ok(Json(vault_obs::summary_view(&snapshot)))
}

/// V2G-R5-OBS-P0: read-only per-asset bucket breakdown.
async fn admin_fees_vault_balances(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let cfg = vault_observability_config(&state).await;
    let budget = load_rebate_budget_map(&state).await;
    let snapshot = vault_obs::read_snapshot(&cfg, &budget).await?;
    Ok(Json(vault_obs::balances_view(&snapshot)))
}

/// V2G-R5-OBS-P0: drift-focused reconciliation view. Surfaces
/// `feeBalance + rebateReserve` vs `CV.balances(vault, asset)` plus
/// raw ERC-20 dust per asset.
async fn admin_fees_vault_reconciliation(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let cfg = vault_observability_config(&state).await;
    let budget = load_rebate_budget_map(&state).await;
    let snapshot = vault_obs::read_snapshot(&cfg, &budget).await?;
    Ok(Json(vault_obs::reconciliation_view(&snapshot)))
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct AdminRecentQuery {
    limit: Option<u32>,
}

async fn admin_recent(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<AdminRecentQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_admin_access(&state, &headers)?;
    let limit = query.limit.unwrap_or(20).min(100);
    if let Some(repository) = state.repository.clone() {
        let transactions = repository
            .list_recent_execution_transactions(limit)
            .await?
            .into_iter()
            .map(compact_execution_transaction)
            .collect::<Vec<_>>();
        return Ok(Json(serde_json::json!({
            "limit": limit,
            "execution_intents": repository.admin_recent_execution_intents(limit).await?,
            "simulations": repository.admin_recent_execution_simulations(limit).await?,
            "transactions": transactions,
            "rfqs": repository.admin_recent_rfqs(limit).await?,
            "option_rfqs": repository.admin_recent_option_rfqs(limit).await?,
            "option_fills": repository.admin_recent_option_fills(limit).await?,
            "option_rfq_fills": repository.admin_recent_option_rfq_fills(limit).await?,
            "option_execution_intents": repository.admin_recent_option_execution_intents(limit).await?
        })));
    }

    let mut execution_intents = state
        .engine
        .lock()
        .map_err(|_| ApiError::internal())?
        .execution_intents();
    execution_intents.sort_by(|left, right| {
        right
            .created_at_ms
            .cmp(&left.created_at_ms)
            .then_with(|| right.intent_id.cmp(&left.intent_id))
    });
    let rfqs = {
        let store = state.rfq_store.lock().map_err(|_| ApiError::internal())?;
        store.list_rfqs()
    };
    let (option_rfqs, option_fills, option_rfq_fills, option_execution_intents) = {
        let store = state
            .options_store
            .lock()
            .map_err(|_| ApiError::internal())?;
        (
            store.list_option_rfqs(),
            store.list_fills(&OptionFillFilter::default()),
            store.list_option_rfq_fills(),
            store.list_option_execution_intents(),
        )
    };
    Ok(Json(serde_json::json!({
        "limit": limit,
        "execution_intents": execution_intents.iter().take(limit as usize).map(compact_execution_intent).collect::<Vec<_>>(),
        "simulations": [],
        "transactions": [],
        "rfqs": rfqs.iter().rev().take(limit as usize).map(compact_rfq).collect::<Vec<_>>(),
        "option_rfqs": option_rfqs.iter().rev().take(limit as usize).map(compact_option_rfq).collect::<Vec<_>>(),
        "option_fills": option_fills.iter().rev().take(limit as usize).map(compact_option_fill).collect::<Vec<_>>(),
        "option_rfq_fills": option_rfq_fills.iter().rev().take(limit as usize).map(compact_option_rfq_fill).collect::<Vec<_>>(),
        "option_execution_intents": option_execution_intents.iter().rev().take(limit as usize).map(compact_option_execution_intent).collect::<Vec<_>>()
    })))
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if !state.admin_config.enabled {
        return Err(ApiError::forbidden("admin API is disabled"));
    }
    if state.admin_config.require_token {
        let token = headers
            .get("x-admin-token")
            .and_then(|value| value.to_str().ok());
        match token {
            Some(token) if state.admin_config.token_matches(token) => {}
            Some(_) => return Err(ApiError::forbidden("invalid admin token")),
            None => return Err(ApiError::forbidden("admin token is required")),
        }
    }
    Ok(())
}

fn ensure_metrics_access(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if !state.metrics_config.enabled {
        return Err(ApiError::forbidden("metrics endpoint is disabled"));
    }
    if state.metrics_config.require_admin_token {
        let token = headers
            .get("x-admin-token")
            .and_then(|value| value.to_str().ok());
        match token {
            Some(token) if state.admin_config.token_matches(token) => {}
            Some(_) => return Err(ApiError::forbidden("invalid metrics token")),
            None => return Err(ApiError::forbidden("metrics token is required")),
        }
    }
    Ok(())
}

fn mm_gateway_transport_label(_state: &AppState) -> &'static str {
    "webtransport"
}

fn bump_count(counts: &mut BTreeMap<String, u64>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

fn count_from_map(counts: &BTreeMap<String, u64>, key: &str) -> u64 {
    counts.get(key).copied().unwrap_or_default()
}

fn execution_status_key(status: ExecutionIntentStatus) -> &'static str {
    match status {
        ExecutionIntentStatus::Pending => "pending",
        ExecutionIntentStatus::DryRun => "dry_run",
        ExecutionIntentStatus::CalldataReady => "calldata_ready",
        ExecutionIntentStatus::SimulationOk => "simulation_ok",
        ExecutionIntentStatus::SimulationFailed => "simulation_failed",
        ExecutionIntentStatus::Submitted => "submitted",
        ExecutionIntentStatus::Confirmed => "confirmed",
        ExecutionIntentStatus::Failed => "failed",
    }
}

fn compact_execution_intent(intent: &crate::execution::ExecutionIntent) -> serde_json::Value {
    serde_json::json!({
        "intent_id": intent.intent_id,
        "market_id": intent.market_id,
        "buyer": intent.buyer,
        "seller": intent.seller,
        "price_1e8": intent.price_1e8.to_string(),
        "size_1e8": intent.size_1e8.to_string(),
        "status": intent.status,
        "created_at_ms": intent.created_at_ms
    })
}

fn compact_execution_transaction(transaction: ExecutionTransaction) -> serde_json::Value {
    serde_json::json!({
        "transaction_id": transaction.transaction_id,
        "intent_id": transaction.intent_id,
        "onchain_intent_id": transaction.onchain_intent_id,
        "target": transaction.target,
        "tx_hash": transaction.tx_hash,
        "status": transaction.status,
        "error": transaction.error,
        "confirmation_status": transaction.confirmation_status,
        "confirmation_error": transaction.confirmation_error,
        "created_at_ms": transaction.created_at_ms,
        "updated_at_ms": transaction.updated_at_ms
    })
}

fn compact_rfq(rfq: &RfqRequest) -> serde_json::Value {
    serde_json::json!({
        "rfq_id": rfq.rfq_id,
        "taker": rfq.taker,
        "market_id": rfq.market_id,
        "side": rfq.side,
        "size_1e8": rfq.size_1e8.to_string(),
        "limit_price_1e8": rfq.limit_price_1e8.map(|value| value.to_string()),
        "status": rfq.status,
        "accepted_quote_id": rfq.accepted_quote_id,
        "execution_intent_id": rfq.execution_intent_id,
        "created_at_ms": rfq.created_at_ms,
        "expires_at_ms": rfq.expires_at_ms
    })
}

fn compact_option_rfq(rfq: &OptionRfqRequest) -> serde_json::Value {
    serde_json::json!({
        "option_rfq_id": rfq.option_rfq_id,
        "taker": rfq.taker,
        "option_series_id": rfq.option_series_id,
        "side": rfq.side,
        "size_1e8": rfq.size_1e8.to_string(),
        "limit_price_1e8": rfq.limit_price_1e8.map(|value| value.to_string()),
        "status": rfq.status,
        "accepted_quote_id": rfq.accepted_quote_id,
        "option_fill_id": rfq.option_fill_id,
        "created_at_ms": rfq.created_at_ms,
        "expires_at_ms": rfq.expires_at_ms
    })
}

fn compact_option_fill(fill: &OptionFill) -> serde_json::Value {
    serde_json::json!({
        "fill_id": fill.fill_id,
        "option_series_id": fill.option_series_id,
        "buyer": fill.buyer,
        "seller": fill.seller,
        "taker_side": fill.taker_side,
        "price_1e8": fill.price_1e8.to_string(),
        "size_1e8": fill.size_1e8.to_string(),
        "created_at_ms": fill.created_at_ms
    })
}

fn compact_option_rfq_fill(fill: &OptionRfqFill) -> serde_json::Value {
    serde_json::json!({
        "fill_id": fill.fill_id,
        "option_rfq_id": fill.option_rfq_id,
        "quote_id": fill.quote_id,
        "option_series_id": fill.option_series_id,
        "buyer": fill.buyer,
        "seller": fill.seller,
        "taker": fill.taker,
        "mm_account": fill.mm_account,
        "taker_side": fill.taker_side,
        "price_1e8": fill.price_1e8.to_string(),
        "size_1e8": fill.size_1e8.to_string(),
        "created_at_ms": fill.created_at_ms
    })
}

fn compact_option_execution_intent(intent: &OptionExecutionIntent) -> serde_json::Value {
    serde_json::json!({
        "intent_id": intent.intent_id,
        "onchain_intent_id": intent.onchain_intent_id,
        "source_type": intent.source_type.as_str(),
        "source_id": intent.source_id,
        "option_series_id": intent.option_series_id,
        "onchain_option_id": intent.onchain_option_id,
        "buyer": intent.buyer,
        "seller": intent.seller,
        "quantity_contracts": intent.quantity_contracts.to_string(),
        "premium_per_contract_native": intent.premium_per_contract_native.to_string(),
        "buyer_is_maker": intent.buyer_is_maker,
        "status": intent.status,
        "calldata_ready": intent.calldata.is_some(),
        "simulation_status": intent.simulation_status.map(|status| status.as_str()),
        "simulation_error": intent.simulation_error,
        "simulation_block_number": intent.simulation_block_number,
        "simulation_revert_data": intent.simulation_revert_data,
        "simulation_revert_selector": intent.simulation_revert_selector,
        "simulated_at_ms": intent.simulated_at_ms,
        "created_at_ms": intent.created_at_ms
    })
}

async fn markets(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let engine = state.engine.lock().map_err(|_| ApiError::internal())?;
    Ok(Json(serde_json::json!(engine.markets())))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct BookLevelResponse {
    #[serde(rename = "price1e8")]
    price_1e8: String,
    #[serde(rename = "totalSize1e8")]
    total_size_1e8: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OrderBookResponse {
    #[serde(rename = "marketId")]
    market_id: MarketId,
    bids: Vec<BookLevelResponse>,
    asks: Vec<BookLevelResponse>,
}

async fn orderbook(
    State(state): State<AppState>,
    Path(market_id): Path<MarketId>,
) -> Result<Json<OrderBookResponse>, ApiError> {
    let engine = state.engine.lock().map_err(|_| ApiError::internal())?;
    let snapshot = engine.orderbook_snapshot(market_id);
    Ok(Json(OrderBookResponse {
        market_id: snapshot.market_id,
        bids: snapshot
            .bids
            .into_iter()
            .map(|level| BookLevelResponse {
                price_1e8: level.price_1e8.to_string(),
                total_size_1e8: level.total_size_1e8.to_string(),
            })
            .collect(),
        asks: snapshot
            .asks
            .into_iter()
            .map(|level| BookLevelResponse {
                price_1e8: level.price_1e8.to_string(),
                total_size_1e8: level.total_size_1e8.to_string(),
            })
            .collect(),
    }))
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct CreateOptionSeriesRequest {
    underlying: String,
    base_asset: String,
    quote_asset: String,
    settlement_asset: String,
    expiry: u64,
    strike_1e8: String,
    is_call: bool,
    contract_size_1e8: Option<String>,
    onchain_product_id: Option<String>,
    onchain_series_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct ListOptionSeriesQuery {
    underlying: Option<String>,
    expiry: Option<u64>,
    is_call: Option<bool>,
    status: Option<OptionSeriesStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct SubmitOptionOrderRequest {
    option_series_id: String,
    account: AccountId,
    side: Side,
    price_1e8: String,
    size_1e8: String,
    time_in_force: TimeInForce,
    client_order_id: Option<String>,
    nonce: Option<u64>,
    deadline_ms: Option<i64>,
    signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct ListOptionOrdersQuery {
    option_series_id: Option<String>,
    account: Option<AccountId>,
    status: Option<OptionOrderStatus>,
    side: Option<Side>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct ListOptionFillsQuery {
    option_series_id: Option<String>,
    account: Option<AccountId>,
    order_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct CreateOptionRfqRequest {
    taker: AccountId,
    option_series_id: String,
    side: Side,
    size_1e8: String,
    limit_price_1e8: Option<String>,
    ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct SubmitOptionRfqQuoteRequest {
    mm_account: AccountId,
    session_id: Option<String>,
    client_quote_id: Option<String>,
    price_1e8: String,
    size_1e8: String,
    quote_nonce: Option<u64>,
    quote_ttl_ms: Option<u64>,
    signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionSeriesResponse {
    option_series_id: String,
    underlying: String,
    base_asset: String,
    quote_asset: String,
    settlement_asset: String,
    expiry: u64,
    strike_1e8: String,
    is_call: bool,
    contract_size_1e8: String,
    status: OptionSeriesStatus,
    source: String,
    onchain_product_id: Option<String>,
    onchain_series_id: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionRfqResponse {
    option_rfq_id: String,
    taker: AccountId,
    option_series_id: String,
    side: Side,
    size_1e8: String,
    limit_price_1e8: Option<String>,
    status: OptionRfqStatus,
    created_at_ms: i64,
    expires_at_ms: i64,
    accepted_quote_id: Option<String>,
    option_fill_id: Option<String>,
}

impl From<OptionRfqRequest> for OptionRfqResponse {
    fn from(rfq: OptionRfqRequest) -> Self {
        let status = rfq.effective_status(now_ms());
        Self {
            option_rfq_id: rfq.option_rfq_id.to_string(),
            taker: rfq.taker,
            option_series_id: rfq.option_series_id,
            side: rfq.side,
            size_1e8: rfq.size_1e8.to_string(),
            limit_price_1e8: rfq.limit_price_1e8.map(|value| value.to_string()),
            status,
            created_at_ms: rfq.created_at_ms,
            expires_at_ms: rfq.expires_at_ms,
            accepted_quote_id: rfq.accepted_quote_id.map(|id| id.to_string()),
            option_fill_id: rfq.option_fill_id.map(|id| id.to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionRfqQuoteResponse {
    quote_id: String,
    option_rfq_id: String,
    mm_account: AccountId,
    session_id: Option<String>,
    client_quote_id: Option<String>,
    price_1e8: String,
    size_1e8: String,
    status: OptionRfqQuoteStatus,
    created_at_ms: i64,
    expires_at_ms: i64,
    signature: Option<String>,
    quote_digest: Option<String>,
    quote_nonce: Option<String>,
    signature_status: OptionRfqQuoteSignatureStatus,
    recovered_signer: Option<AccountId>,
}

impl From<OptionRfqQuote> for OptionRfqQuoteResponse {
    fn from(quote: OptionRfqQuote) -> Self {
        let status = quote.effective_status(now_ms());
        Self {
            quote_id: quote.quote_id.to_string(),
            option_rfq_id: quote.option_rfq_id.to_string(),
            mm_account: quote.mm_account,
            session_id: quote.session_id,
            client_quote_id: quote.client_quote_id,
            price_1e8: quote.price_1e8.to_string(),
            size_1e8: quote.size_1e8.to_string(),
            status,
            created_at_ms: quote.created_at_ms,
            expires_at_ms: quote.expires_at_ms,
            signature: quote.signature,
            quote_digest: quote.quote_digest,
            quote_nonce: quote.quote_nonce,
            signature_status: quote.signature_status,
            recovered_signer: quote.recovered_signer,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct OptionRfqQuoteSigningPayloadRequest {
    mm_account: AccountId,
    price_1e8: String,
    size_1e8: String,
    client_quote_id: Option<String>,
    quote_nonce: u64,
    quote_ttl_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionRfqQuoteSigningPayloadResponse {
    option_rfq_id: String,
    option_rfq_id_b32: String,
    option_series_id_b32: String,
    digest: String,
    domain: SigningPayloadDomain,
    primary_type: &'static str,
    types: Vec<SigningPayloadTypeField>,
    message: OptionRfqQuoteSigningPayloadMessage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionRfqQuoteSigningPayloadMessage {
    #[serde(rename = "optionRfqId")]
    option_rfq_id: String,
    #[serde(rename = "mmAccount")]
    mm_account: String,
    #[serde(rename = "optionSeriesId")]
    option_series_id: String,
    #[serde(rename = "takerIsBuyer")]
    taker_is_buyer: bool,
    #[serde(rename = "price1e8")]
    price_1e8: String,
    #[serde(rename = "size1e8")]
    size_1e8: String,
    #[serde(rename = "quoteNonce")]
    quote_nonce: String,
    expiry: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionRfqFillResponse {
    fill_id: String,
    option_rfq_id: String,
    quote_id: String,
    option_series_id: String,
    buyer: AccountId,
    seller: AccountId,
    taker: AccountId,
    mm_account: AccountId,
    taker_side: Side,
    price_1e8: String,
    size_1e8: String,
    created_at_ms: i64,
}

impl From<OptionRfqFill> for OptionRfqFillResponse {
    fn from(fill: OptionRfqFill) -> Self {
        Self {
            fill_id: fill.fill_id.to_string(),
            option_rfq_id: fill.option_rfq_id.to_string(),
            quote_id: fill.quote_id.to_string(),
            option_series_id: fill.option_series_id,
            buyer: fill.buyer,
            seller: fill.seller,
            taker: fill.taker,
            mm_account: fill.mm_account,
            taker_side: fill.taker_side,
            price_1e8: fill.price_1e8.to_string(),
            size_1e8: fill.size_1e8.to_string(),
            created_at_ms: fill.created_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct AcceptOptionRfqQuoteResponse {
    option_rfq_id: String,
    quote_id: String,
    status: OptionRfqStatus,
    quote_status: OptionRfqQuoteStatus,
    option_fill_id: String,
    fill: OptionRfqFillResponse,
    mm_notification_sent: bool,
    mm_notification_warning: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionOrderResponse {
    order_id: String,
    option_series_id: String,
    account: AccountId,
    side: Side,
    price_1e8: String,
    size_1e8: String,
    remaining_size_1e8: String,
    time_in_force: TimeInForce,
    client_order_id: Option<String>,
    nonce: Option<String>,
    deadline_ms: Option<i64>,
    signature: Option<String>,
    status: OptionOrderStatus,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SubmitOptionOrderResponse {
    #[serde(flatten)]
    order: OptionOrderResponse,
    fills: Vec<OptionFillResponse>,
}

impl From<OptionOrder> for OptionOrderResponse {
    fn from(order: OptionOrder) -> Self {
        Self {
            order_id: order.order_id.to_string(),
            option_series_id: order.option_series_id,
            account: order.account,
            side: order.side,
            price_1e8: order.price_1e8.to_string(),
            size_1e8: order.size_1e8.to_string(),
            remaining_size_1e8: order.remaining_size_1e8.to_string(),
            time_in_force: order.time_in_force,
            client_order_id: order.client_order_id,
            nonce: order.nonce.map(|value| value.to_string()),
            deadline_ms: order.deadline_ms,
            signature: order.signature,
            status: order.status,
            created_at_ms: order.created_at_ms,
            updated_at_ms: order.updated_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionFillResponse {
    fill_id: String,
    option_series_id: String,
    buy_order_id: String,
    sell_order_id: String,
    buyer: AccountId,
    seller: AccountId,
    maker_order_id: String,
    taker_order_id: String,
    taker_side: Side,
    price_1e8: String,
    size_1e8: String,
    created_at_ms: i64,
}

impl From<OptionFill> for OptionFillResponse {
    fn from(fill: OptionFill) -> Self {
        Self {
            fill_id: fill.fill_id.to_string(),
            option_series_id: fill.option_series_id,
            buy_order_id: fill.buy_order_id.to_string(),
            sell_order_id: fill.sell_order_id.to_string(),
            buyer: fill.buyer,
            seller: fill.seller,
            maker_order_id: fill.maker_order_id.to_string(),
            taker_order_id: fill.taker_order_id.to_string(),
            taker_side: fill.taker_side,
            price_1e8: fill.price_1e8.to_string(),
            size_1e8: fill.size_1e8.to_string(),
            created_at_ms: fill.created_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionExecutionIntentResponse {
    intent_id: String,
    onchain_intent_id: String,
    source_type: String,
    source_id: String,
    option_series_id: String,
    onchain_option_id: String,
    buyer: AccountId,
    seller: AccountId,
    underlying: AccountId,
    settlement_asset: AccountId,
    expiry: u64,
    strike_1e8: String,
    is_call: bool,
    contract_size_1e8: String,
    quantity_contracts: String,
    source_size_1e8: String,
    source_price_1e8: String,
    premium_per_contract_native: String,
    buyer_is_maker: bool,
    buyer_nonce: Option<String>,
    seller_nonce: Option<String>,
    deadline: String,
    buyer_signature_present: bool,
    seller_signature_present: bool,
    calldata_ready: bool,
    status: OptionExecutionIntentStatus,
    error: Option<String>,
    simulation_status: Option<OptionExecutionSimulationStatus>,
    simulation_error: Option<String>,
    simulation_block_number: Option<u64>,
    simulation_revert_data: Option<String>,
    simulation_revert_selector: Option<String>,
    simulated_at_ms: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl From<OptionExecutionIntent> for OptionExecutionIntentResponse {
    fn from(intent: OptionExecutionIntent) -> Self {
        Self {
            intent_id: intent.intent_id.to_string(),
            onchain_intent_id: intent.onchain_intent_id,
            source_type: intent.source_type.as_str().to_string(),
            source_id: intent.source_id,
            option_series_id: intent.option_series_id,
            onchain_option_id: intent.onchain_option_id,
            buyer: intent.buyer,
            seller: intent.seller,
            underlying: intent.underlying,
            settlement_asset: intent.settlement_asset,
            expiry: intent.expiry,
            strike_1e8: intent.strike_1e8.to_string(),
            is_call: intent.is_call,
            contract_size_1e8: intent.contract_size_1e8.to_string(),
            quantity_contracts: intent.quantity_contracts.to_string(),
            source_size_1e8: intent.source_size_1e8.to_string(),
            source_price_1e8: intent.source_price_1e8.to_string(),
            premium_per_contract_native: intent.premium_per_contract_native.to_string(),
            buyer_is_maker: intent.buyer_is_maker,
            buyer_nonce: intent.buyer_nonce.map(|value| value.to_string()),
            seller_nonce: intent.seller_nonce.map(|value| value.to_string()),
            deadline: intent.deadline.to_string(),
            buyer_signature_present: intent.buyer_signature.is_some(),
            seller_signature_present: intent.seller_signature.is_some(),
            calldata_ready: intent.calldata.is_some(),
            status: intent.status,
            error: intent.error,
            simulation_status: intent.simulation_status,
            simulation_error: intent.simulation_error,
            simulation_block_number: intent.simulation_block_number,
            simulation_revert_data: intent.simulation_revert_data,
            simulation_revert_selector: intent.simulation_revert_selector,
            simulated_at_ms: intent.simulated_at_ms,
            created_at_ms: intent.created_at_ms,
            updated_at_ms: intent.updated_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionExecutionSimulationResponse {
    intent_id: String,
    simulation_status: OptionExecutionSimulationStatus,
    block_number: Option<u64>,
    error: Option<String>,
    revert_data: Option<String>,
    revert_selector: Option<String>,
    simulated_at_ms: Option<i64>,
    submitted: bool,
    confirmed: bool,
}

impl From<OptionExecutionSimulationResult> for OptionExecutionSimulationResponse {
    fn from(result: OptionExecutionSimulationResult) -> Self {
        Self {
            intent_id: result.intent_id.to_string(),
            simulation_status: result.simulation_status,
            block_number: result.block_number,
            error: result.error,
            revert_data: result.revert_data,
            revert_selector: result.revert_selector,
            simulated_at_ms: (result.simulated_at_ms > 0).then_some(result.simulated_at_ms),
            submitted: false,
            confirmed: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionExecutionSigningPayloadResponse {
    intent_id: String,
    onchain_intent_id: String,
    digest: String,
    domain: SigningPayloadDomain,
    #[serde(rename = "primaryType")]
    primary_type: &'static str,
    types: Vec<SigningPayloadTypeField>,
    message: OptionExecutionSigningPayloadMessage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionExecutionSigningPayloadMessage {
    #[serde(rename = "intentId")]
    intent_id: String,
    buyer: String,
    seller: String,
    #[serde(rename = "optionId")]
    option_id: String,
    underlying: String,
    #[serde(rename = "settlementAsset")]
    settlement_asset: String,
    expiry: String,
    #[serde(rename = "strike1e8")]
    strike_1e8: String,
    #[serde(rename = "isCall")]
    is_call: bool,
    #[serde(rename = "contractSize1e8")]
    contract_size_1e8: String,
    quantity: String,
    #[serde(rename = "premiumPerContract")]
    premium_per_contract: String,
    #[serde(rename = "buyerIsMaker")]
    buyer_is_maker: bool,
    #[serde(rename = "buyerNonce")]
    buyer_nonce: String,
    #[serde(rename = "sellerNonce")]
    seller_nonce: String,
    deadline: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct SubmitOptionExecutionSignaturesRequest {
    buyer_signature: Option<String>,
    seller_signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SubmitOptionExecutionSignaturesResponse {
    intent_id: String,
    status: OptionExecutionIntentStatus,
    buyer_signature_present: bool,
    seller_signature_present: bool,
    calldata_ready: bool,
    missing_signatures: bool,
    calldata: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionExecutionCalldataResponse {
    intent_id: String,
    status: OptionExecutionIntentStatus,
    calldata: Option<String>,
    calldata_ready: bool,
    missing_signatures: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionExecutionConfirmationResponse {
    intent_id: String,
    intent_status: OptionExecutionIntentStatus,
    transaction_id: String,
    tx_hash: Option<String>,
    confirmation_status: crate::options::OptionExecutionConfirmationStatus,
    receipt_status: Option<u64>,
    confirmed_block_number: Option<u64>,
    confirmed_at_ms: Option<crate::types::TimestampMs>,
    confirmation_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionExecutionBroadcastResponse {
    intent_id: String,
    status: OptionExecutionIntentStatus,
    tx_hash: Option<String>,
    to: AccountId,
    from: AccountId,
    transaction_id: String,
    broadcast_enabled: bool,
    submitted: bool,
    duplicate: bool,
    confirmed: bool,
    estimated_gas: Option<u64>,
    required_gas: Option<u64>,
    simulation_gas_limit: Option<u64>,
    broadcast_gas_limit: Option<u64>,
    gas_safety_bps: Option<u32>,
    gas_check_status: Option<crate::options::OptionExecutionGasCheckStatus>,
    gas_check_error: Option<String>,
}

impl From<OptionSeries> for OptionSeriesResponse {
    fn from(series: OptionSeries) -> Self {
        Self {
            option_series_id: series.option_series_id,
            underlying: series.underlying,
            base_asset: series.base_asset,
            quote_asset: series.quote_asset,
            settlement_asset: series.settlement_asset,
            expiry: series.expiry,
            strike_1e8: series.strike_1e8.to_string(),
            is_call: series.is_call,
            contract_size_1e8: series.contract_size_1e8.to_string(),
            status: series.status,
            source: series.source.as_str().to_string(),
            onchain_product_id: series.onchain_product_id,
            onchain_series_id: series.onchain_series_id,
            created_at_ms: series.created_at_ms,
            updated_at_ms: series.updated_at_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionOrderbookResponse {
    option_series_id: String,
    status: OptionSeriesStatus,
    bids: Vec<OptionBookLevelResponse>,
    asks: Vec<OptionBookLevelResponse>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionBookLevelResponse {
    price_1e8: String,
    size_1e8: String,
}

impl From<OptionOrderbookSnapshot> for OptionOrderbookResponse {
    fn from(snapshot: OptionOrderbookSnapshot) -> Self {
        Self {
            option_series_id: snapshot.option_series_id,
            status: snapshot.status,
            bids: snapshot
                .bids
                .into_iter()
                .map(|level| OptionBookLevelResponse {
                    price_1e8: level.price_1e8,
                    size_1e8: level.size_1e8,
                })
                .collect(),
            asks: snapshot
                .asks
                .into_iter()
                .map(|level| OptionBookLevelResponse {
                    price_1e8: level.price_1e8,
                    size_1e8: level.size_1e8,
                })
                .collect(),
        }
    }
}

async fn create_option_series(
    State(state): State<AppState>,
    Json(request): Json<CreateOptionSeriesRequest>,
) -> Result<Json<OptionSeriesResponse>, ApiError> {
    let series = create_option_series_service(
        &state,
        CreateOptionSeriesInput {
            underlying: request.underlying,
            base_asset: request.base_asset,
            quote_asset: request.quote_asset,
            settlement_asset: request.settlement_asset,
            expiry: request.expiry,
            strike_1e8: parse_fixed_u128("strike_1e8", &request.strike_1e8)?,
            is_call: request.is_call,
            contract_size_1e8: request
                .contract_size_1e8
                .as_deref()
                .map(|value| parse_fixed_u128("contract_size_1e8", value))
                .transpose()?,
            onchain_product_id: request.onchain_product_id,
            onchain_series_id: request.onchain_series_id,
        },
    )
    .await?;
    Ok(Json(series.into()))
}

async fn list_option_series(
    State(state): State<AppState>,
    Query(query): Query<ListOptionSeriesQuery>,
) -> Result<Json<Vec<OptionSeriesResponse>>, ApiError> {
    let series = list_option_series_service(
        &state,
        OptionSeriesFilter {
            underlying: query.underlying,
            expiry: query.expiry,
            is_call: query.is_call,
            status: query.status,
        },
    )
    .await?;
    Ok(Json(
        series.into_iter().map(OptionSeriesResponse::from).collect(),
    ))
}

async fn get_option_series(
    State(state): State<AppState>,
    Path(option_series_id): Path<String>,
) -> Result<Json<OptionSeriesResponse>, ApiError> {
    Ok(Json(
        get_option_series_service(&state, &option_series_id)
            .await?
            .into(),
    ))
}

async fn disable_option_series(
    State(state): State<AppState>,
    Path(option_series_id): Path<String>,
) -> Result<Json<OptionSeriesResponse>, ApiError> {
    Ok(Json(
        disable_option_series_service(&state, &option_series_id)
            .await?
            .into(),
    ))
}

async fn get_option_orderbook(
    State(state): State<AppState>,
    Path(option_series_id): Path<String>,
) -> Result<Json<OptionOrderbookResponse>, ApiError> {
    Ok(Json(
        get_option_orderbook_service(&state, option_series_id)
            .await?
            .into(),
    ))
}

async fn submit_option_order(
    State(state): State<AppState>,
    Json(request): Json<SubmitOptionOrderRequest>,
) -> Result<Json<SubmitOptionOrderResponse>, ApiError> {
    let outcome = submit_option_order_service(
        &state,
        SubmitOptionOrderInput {
            option_series_id: request.option_series_id,
            account: request.account,
            side: request.side,
            price_1e8: parse_fixed_u128("price_1e8", &request.price_1e8)?,
            size_1e8: parse_fixed_u128("size_1e8", &request.size_1e8)?,
            time_in_force: request.time_in_force,
            client_order_id: request.client_order_id,
            nonce: request.nonce,
            deadline_ms: request.deadline_ms,
            signature: request.signature,
        },
    )
    .await?;
    Ok(Json(SubmitOptionOrderResponse {
        order: outcome.order.into(),
        fills: outcome
            .fills
            .into_iter()
            .map(OptionFillResponse::from)
            .collect(),
    }))
}

async fn list_option_orders(
    State(state): State<AppState>,
    Query(query): Query<ListOptionOrdersQuery>,
) -> Result<Json<Vec<OptionOrderResponse>>, ApiError> {
    let orders = list_option_orders_service(
        &state,
        OptionOrderFilter {
            option_series_id: query.option_series_id,
            account: query.account,
            status: query.status,
            side: query.side,
        },
    )
    .await?;
    Ok(Json(
        orders.into_iter().map(OptionOrderResponse::from).collect(),
    ))
}

async fn get_option_order(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> Result<Json<OptionOrderResponse>, ApiError> {
    let order_id = parse_option_order_id(&order_id)?;
    Ok(Json(
        get_option_order_service(&state, order_id).await?.into(),
    ))
}

fn parse_option_order_id(order_id: &str) -> BackendResult<OrderId> {
    OrderId::from_str(order_id).map_err(|_| BackendError::InvalidOptionOrderId)
}

fn parse_option_fill_id(fill_id: &str) -> BackendResult<OptionFillId> {
    OptionFillId::from_str(fill_id).map_err(|_| BackendError::InvalidOptionFillId)
}

fn parse_option_rfq_id(option_rfq_id: &str) -> BackendResult<OptionRfqId> {
    OptionRfqId::from_str(option_rfq_id).map_err(|_| BackendError::InvalidOptionRfqId)
}

fn parse_option_rfq_quote_id(quote_id: &str) -> BackendResult<OptionRfqQuoteId> {
    OptionRfqQuoteId::from_str(quote_id).map_err(|_| BackendError::InvalidOptionRfqQuoteId)
}

fn parse_option_execution_intent_id(intent_id: &str) -> BackendResult<OptionExecutionIntentId> {
    OptionExecutionIntentId::from_str(intent_id)
        .map_err(|_| BackendError::InvalidOptionExecutionIntentId)
}

async fn list_option_fills(
    State(state): State<AppState>,
    Query(query): Query<ListOptionFillsQuery>,
) -> Result<Json<Vec<OptionFillResponse>>, ApiError> {
    let order_id = query
        .order_id
        .as_deref()
        .map(parse_option_order_id)
        .transpose()?;
    let fills = list_option_fills_service(
        &state,
        OptionFillFilter {
            option_series_id: query.option_series_id,
            account: query.account,
            order_id,
        },
    )
    .await?;
    Ok(Json(
        fills.into_iter().map(OptionFillResponse::from).collect(),
    ))
}

async fn get_option_fill(
    State(state): State<AppState>,
    Path(fill_id): Path<String>,
) -> Result<Json<OptionFillResponse>, ApiError> {
    Ok(Json(
        get_option_fill_service(&state, parse_option_fill_id(&fill_id)?)
            .await?
            .into(),
    ))
}

async fn get_option_order_fills(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> Result<Json<Vec<OptionFillResponse>>, ApiError> {
    let fills = get_option_order_fills_service(&state, parse_option_order_id(&order_id)?).await?;
    Ok(Json(
        fills.into_iter().map(OptionFillResponse::from).collect(),
    ))
}

async fn cancel_option_order(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> Result<Json<OptionOrderResponse>, ApiError> {
    let order_id = parse_option_order_id(&order_id)?;
    Ok(Json(
        cancel_option_order_service(&state, order_id).await?.into(),
    ))
}

async fn list_option_execution_intents(
    State(state): State<AppState>,
) -> Result<Json<Vec<OptionExecutionIntentResponse>>, ApiError> {
    let intents = list_option_execution_intents_service(&state).await?;
    Ok(Json(
        intents
            .into_iter()
            .map(OptionExecutionIntentResponse::from)
            .collect(),
    ))
}

async fn get_option_execution_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<OptionExecutionIntentResponse>, ApiError> {
    Ok(Json(
        get_option_execution_intent_service(&state, parse_option_execution_intent_id(&intent_id)?)
            .await?
            .into(),
    ))
}

async fn option_execution_signing_payload(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<OptionExecutionSigningPayloadResponse>, ApiError> {
    let intent_id = parse_option_execution_intent_id(&intent_id)?;
    let outcome = option_execution_signing_payload_service(&state, intent_id).await?;
    let domain = state.options_config.execution_eip712_domain.clone();
    Ok(Json(OptionExecutionSigningPayloadResponse {
        intent_id: outcome.intent.intent_id.to_string(),
        onchain_intent_id: option_execution_intent_id_to_hex_bytes32(
            &outcome.intent.intent_id.to_string(),
        )?,
        digest: outcome.digest,
        domain: SigningPayloadDomain {
            name: domain.name,
            version: domain.version,
            chain_id: domain.chain_id,
            verifying_contract: domain.verifying_contract.0,
        },
        primary_type: "OptionTrade",
        types: option_trade_type_fields(),
        message: OptionExecutionSigningPayloadMessage {
            intent_id: outcome.intent.onchain_intent_id,
            buyer: outcome.payload.buyer.0,
            seller: outcome.payload.seller.0,
            option_id: outcome.intent.onchain_option_id,
            underlying: outcome.payload.underlying.0,
            settlement_asset: outcome.payload.settlement_asset.0,
            expiry: outcome.payload.expiry.to_string(),
            strike_1e8: outcome.payload.strike_1e8.to_string(),
            is_call: outcome.payload.is_call,
            contract_size_1e8: outcome.payload.contract_size_1e8.to_string(),
            quantity: outcome.payload.quantity.to_string(),
            premium_per_contract: outcome.payload.premium_per_contract.to_string(),
            buyer_is_maker: outcome.payload.buyer_is_maker,
            buyer_nonce: outcome.payload.buyer_nonce.to_string(),
            seller_nonce: outcome.payload.seller_nonce.to_string(),
            deadline: outcome.payload.deadline.to_string(),
        },
    }))
}

async fn submit_option_execution_signatures(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
    Json(request): Json<SubmitOptionExecutionSignaturesRequest>,
) -> Result<Json<SubmitOptionExecutionSignaturesResponse>, ApiError> {
    let intent_id = parse_option_execution_intent_id(&intent_id)?;
    let outcome = submit_option_execution_signatures_service(
        &state,
        intent_id,
        SubmitOptionExecutionSignaturesInput {
            buyer_signature: request.buyer_signature,
            seller_signature: request.seller_signature,
        },
    )
    .await?;
    Ok(Json(SubmitOptionExecutionSignaturesResponse {
        intent_id: outcome.intent.intent_id.to_string(),
        status: outcome.intent.status,
        buyer_signature_present: outcome.buyer_signature_present,
        seller_signature_present: outcome.seller_signature_present,
        calldata_ready: outcome.calldata_ready,
        missing_signatures: outcome.missing_signatures,
        calldata: outcome.intent.calldata,
    }))
}

async fn option_execution_calldata(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<OptionExecutionCalldataResponse>, ApiError> {
    let intent_id = parse_option_execution_intent_id(&intent_id)?;
    let outcome = option_execution_calldata_service(&state, intent_id).await?;
    Ok(Json(OptionExecutionCalldataResponse {
        intent_id: outcome.intent.intent_id.to_string(),
        status: outcome.intent.status,
        calldata: outcome.calldata,
        calldata_ready: outcome.calldata_ready,
        missing_signatures: outcome.missing_signatures,
    }))
}

async fn simulate_option_execution_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<OptionExecutionSimulationResponse>, ApiError> {
    let intent_id = parse_option_execution_intent_id(&intent_id)?;
    let intent = prepare_option_execution_simulation(&state, intent_id).await?;
    let Some(rpc_url) = state.options_config.execution_simulation_rpc_url.clone() else {
        let error = if state.options_config.execution_require_rpc_for_simulation {
            "RPC_URL is required for option execution simulation"
        } else {
            "RPC_URL is not configured for option execution simulation"
        };
        let result =
            persist_option_execution_simulation_unavailable(&state, intent.intent_id, error)
                .await?;
        if state.options_config.execution_require_rpc_for_simulation {
            return Err(BackendError::Config(error.to_string()).into());
        }
        return Ok(Json(OptionExecutionSimulationResponse::from(result)));
    };
    let provider = HttpJsonRpcProvider::new(rpc_url);
    let result = simulate_prepared_option_execution_intent(&state, &intent, &provider).await?;
    Ok(Json(OptionExecutionSimulationResponse::from(result)))
}

async fn option_execution_simulation(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<OptionExecutionSimulationResponse>, ApiError> {
    let intent_id = parse_option_execution_intent_id(&intent_id)?;
    let result = option_execution_simulation_status_service(&state, intent_id).await?;
    Ok(Json(OptionExecutionSimulationResponse::from(result)))
}

async fn confirm_option_execution_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<OptionExecutionConfirmationResponse>, ApiError> {
    let intent_id = parse_option_execution_intent_id(&intent_id)?;
    let outcome = confirm_option_execution_intent_service(&state, intent_id).await?;
    Ok(Json(OptionExecutionConfirmationResponse {
        intent_id: outcome.intent.intent_id.to_string(),
        intent_status: outcome.intent.status,
        transaction_id: outcome.transaction.transaction_id,
        tx_hash: outcome.transaction.tx_hash,
        confirmation_status: outcome.confirmation_status,
        receipt_status: outcome.receipt_status,
        confirmed_block_number: outcome.block_number,
        confirmed_at_ms: outcome.transaction.confirmed_at_ms,
        confirmation_error: outcome.error,
    }))
}

async fn broadcast_option_execution_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<OptionExecutionBroadcastResponse>, ApiError> {
    let intent_id = parse_option_execution_intent_id(&intent_id)?;
    let outcome = broadcast_option_execution_intent_service(&state, intent_id).await?;
    Ok(Json(OptionExecutionBroadcastResponse {
        intent_id: outcome.intent.intent_id.to_string(),
        status: outcome.intent.status,
        tx_hash: outcome.transaction.tx_hash,
        to: outcome.transaction.to,
        from: outcome.transaction.from,
        transaction_id: outcome.transaction.transaction_id,
        broadcast_enabled: outcome.broadcast_enabled,
        submitted: outcome.submitted,
        duplicate: outcome.duplicate,
        confirmed: false,
        estimated_gas: outcome.transaction.estimated_gas,
        required_gas: outcome.transaction.required_gas,
        simulation_gas_limit: outcome.transaction.simulation_gas_limit,
        broadcast_gas_limit: outcome.transaction.broadcast_gas_limit,
        gas_safety_bps: outcome.transaction.gas_safety_bps,
        gas_check_status: outcome.transaction.gas_check_status,
        gas_check_error: outcome.transaction.gas_check_error,
    }))
}

async fn create_option_rfq(
    State(state): State<AppState>,
    Json(request): Json<CreateOptionRfqRequest>,
) -> Result<Json<OptionRfqResponse>, ApiError> {
    let rfq = create_option_rfq_service(
        &state,
        CreateOptionRfqInput {
            taker: request.taker,
            option_series_id: request.option_series_id,
            side: request.side,
            size_1e8: parse_fixed_u128("size_1e8", &request.size_1e8)?,
            limit_price_1e8: request
                .limit_price_1e8
                .as_deref()
                .map(|value| parse_fixed_u128("limit_price_1e8", value))
                .transpose()?,
            ttl_ms: request.ttl_ms,
        },
    )
    .await?;
    Ok(Json(rfq.into()))
}

async fn list_option_rfqs(
    State(state): State<AppState>,
) -> Result<Json<Vec<OptionRfqResponse>>, ApiError> {
    let rfqs = list_option_rfqs_service(&state).await?;
    Ok(Json(
        rfqs.into_iter().map(OptionRfqResponse::from).collect(),
    ))
}

async fn get_option_rfq(
    State(state): State<AppState>,
    Path(option_rfq_id): Path<String>,
) -> Result<Json<OptionRfqResponse>, ApiError> {
    let option_rfq_id = parse_option_rfq_id(&option_rfq_id)?;
    Ok(Json(
        get_option_rfq_service(&state, option_rfq_id).await?.into(),
    ))
}

async fn option_rfq_quote_payload(
    State(state): State<AppState>,
    Path(option_rfq_id): Path<String>,
    Json(request): Json<OptionRfqQuoteSigningPayloadRequest>,
) -> Result<Json<OptionRfqQuoteSigningPayloadResponse>, ApiError> {
    let option_rfq_id = parse_option_rfq_id(&option_rfq_id)?;
    let outcome = option_rfq_quote_signing_payload_service(
        &state,
        OptionRfqQuoteSigningPayloadInput {
            option_rfq_id,
            mm_account: request.mm_account,
            price_1e8: parse_fixed_u128("price_1e8", &request.price_1e8)?,
            size_1e8: parse_fixed_u128("size_1e8", &request.size_1e8)?,
            quote_nonce: request.quote_nonce,
            quote_ttl_ms: request.quote_ttl_ms,
        },
    )
    .await?;
    let domain = state.options_config.rfq_eip712_domain.clone();
    let _ = request.client_quote_id;

    Ok(Json(OptionRfqQuoteSigningPayloadResponse {
        option_rfq_id: option_rfq_id.to_string(),
        option_rfq_id_b32: option_rfq_id_to_hex_bytes32(&option_rfq_id.to_string()),
        option_series_id_b32: option_series_id_to_hex_bytes32(&outcome.rfq.option_series_id)?,
        digest: outcome.digest,
        domain: SigningPayloadDomain {
            name: domain.name,
            version: domain.version,
            chain_id: domain.chain_id,
            verifying_contract: domain.verifying_contract.0,
        },
        primary_type: "OptionRFQQuote",
        types: option_rfq_quote_type_fields(),
        message: OptionRfqQuoteSigningPayloadMessage {
            option_rfq_id: option_rfq_id_to_hex_bytes32(&outcome.rfq.option_rfq_id.to_string()),
            mm_account: outcome.payload.mm_account.0,
            option_series_id: option_series_id_to_hex_bytes32(&outcome.rfq.option_series_id)?,
            taker_is_buyer: outcome.payload.taker_is_buyer,
            price_1e8: outcome.payload.price_1e8.to_string(),
            size_1e8: outcome.payload.size_1e8.to_string(),
            quote_nonce: outcome.payload.quote_nonce.to_string(),
            expiry: outcome.payload.expiry.to_string(),
        },
    }))
}

async fn submit_option_rfq_quote(
    State(state): State<AppState>,
    Path(option_rfq_id): Path<String>,
    Json(request): Json<SubmitOptionRfqQuoteRequest>,
) -> Result<Json<OptionRfqQuoteResponse>, ApiError> {
    let option_rfq_id = parse_option_rfq_id(&option_rfq_id)?;
    let quote = submit_option_rfq_quote_service(
        &state,
        option_rfq_id,
        SubmitOptionRfqQuoteInput {
            mm_account: request.mm_account,
            session_id: request.session_id,
            client_quote_id: request.client_quote_id,
            price_1e8: parse_fixed_u128("price_1e8", &request.price_1e8)?,
            size_1e8: parse_fixed_u128("size_1e8", &request.size_1e8)?,
            quote_nonce: request.quote_nonce,
            quote_ttl_ms: request.quote_ttl_ms,
            signature: request.signature,
        },
    )
    .await?;
    Ok(Json(quote.into()))
}

async fn list_option_rfq_quotes(
    State(state): State<AppState>,
    Path(option_rfq_id): Path<String>,
) -> Result<Json<Vec<OptionRfqQuoteResponse>>, ApiError> {
    let option_rfq_id = parse_option_rfq_id(&option_rfq_id)?;
    let quotes = list_option_rfq_quotes_service(&state, option_rfq_id).await?;
    Ok(Json(
        quotes
            .into_iter()
            .map(OptionRfqQuoteResponse::from)
            .collect(),
    ))
}

async fn accept_option_rfq_quote(
    State(state): State<AppState>,
    Path((option_rfq_id, quote_id)): Path<(String, String)>,
) -> Result<Json<AcceptOptionRfqQuoteResponse>, ApiError> {
    let option_rfq_id = parse_option_rfq_id(&option_rfq_id)?;
    let quote_id = parse_option_rfq_quote_id(&quote_id)?;
    let outcome = accept_option_rfq_quote_service(&state, option_rfq_id, quote_id).await?;
    let option_fill_id = outcome.fill.fill_id.to_string();
    Ok(Json(AcceptOptionRfqQuoteResponse {
        option_rfq_id: outcome.rfq.option_rfq_id.to_string(),
        quote_id: outcome.quote.quote_id.to_string(),
        status: outcome.rfq.status,
        quote_status: outcome.quote.status,
        option_fill_id,
        fill: outcome.fill.into(),
        mm_notification_sent: outcome.mm_notification_sent,
        mm_notification_warning: outcome.mm_notification_warning,
    }))
}

async fn cancel_option_rfq(
    State(state): State<AppState>,
    Path(option_rfq_id): Path<String>,
) -> Result<Json<OptionRfqResponse>, ApiError> {
    let option_rfq_id = parse_option_rfq_id(&option_rfq_id)?;
    Ok(Json(
        cancel_option_rfq_service(&state, option_rfq_id)
            .await?
            .into(),
    ))
}

async fn submit_order(
    State(state): State<AppState>,
    Json(request): Json<SubmitOrderRequest>,
) -> Result<Json<SubmitOrderResponse>, ApiError> {
    let signed_order = request.into_signed_order()?;
    let outcome = submit_signed_order(&state, signed_order).await?;
    Ok(Json(submit_response_from_events(outcome.events)))
}

async fn account_perp_nonce(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<PerpNonceResponse>, ApiError> {
    if !state.perp_nonce_sync_config.enabled {
        return Err(BackendError::PerpNonceSyncDisabled.into());
    }
    let rpc_url = state
        .perp_nonce_sync_config
        .rpc_url
        .clone()
        .ok_or_else(|| {
            BackendError::Config("RPC_URL is required for perp nonce sync".to_string())
        })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    let account = crate::types::AccountId::new(address);
    Ok(Json(
        read_perp_nonce(&state.perp_nonce_sync_config, &provider, &account).await?,
    ))
}

async fn account_option_nonce(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<OptionNonceResponse>, ApiError> {
    if !state.option_nonce_sync_config.enabled {
        return Err(BackendError::OptionNonceSyncDisabled.into());
    }
    let rpc_url = state
        .option_nonce_sync_config
        .rpc_url
        .clone()
        .ok_or_else(|| {
            BackendError::Config("RPC_URL is required for option nonce sync".to_string())
        })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    let account = crate::types::AccountId::new(address);
    Ok(Json(
        read_option_nonce(&state.option_nonce_sync_config, &provider, &account).await?,
    ))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CancelOrderResponse {
    status: String,
    event: ApiEngineEvent,
}

async fn cancel_order(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> Result<Json<CancelOrderResponse>, ApiError> {
    let order_id = OrderId::from_str(&order_id).map_err(|_| BackendError::InvalidOrderId)?;
    let outcome = cancel_order_shared(
        &state,
        CancelOrderInput {
            account: None,
            market_id: None,
            order_id: Some(order_id),
            client_order_id: None,
        },
    )
    .await?;
    let Some(event) = outcome.events.into_iter().next() else {
        return Err(ApiError::internal());
    };
    Ok(Json(CancelOrderResponse {
        status: "cancelled".to_string(),
        event: event.into(),
    }))
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct CreateRfqRequest {
    taker: AccountId,
    market_id: MarketId,
    side: Side,
    size_1e8: String,
    limit_price_1e8: Option<String>,
    ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RfqResponse {
    rfq_id: Uuid,
    taker: AccountId,
    market_id: MarketId,
    side: Side,
    size_1e8: String,
    limit_price_1e8: Option<String>,
    status: RfqStatus,
    created_at_ms: i64,
    expires_at_ms: i64,
    accepted_quote_id: Option<Uuid>,
    execution_intent_id: Option<Uuid>,
}

impl From<RfqRequest> for RfqResponse {
    fn from(rfq: RfqRequest) -> Self {
        let now = now_ms();
        let status = rfq.effective_status(now);
        Self {
            rfq_id: rfq.rfq_id,
            taker: rfq.taker,
            market_id: rfq.market_id,
            side: rfq.side,
            size_1e8: rfq.size_1e8.to_string(),
            limit_price_1e8: rfq.limit_price_1e8.map(|value| value.to_string()),
            status,
            created_at_ms: rfq.created_at_ms,
            expires_at_ms: rfq.expires_at_ms,
            accepted_quote_id: rfq.accepted_quote_id,
            execution_intent_id: rfq.execution_intent_id,
        }
    }
}

async fn create_rfq(
    State(state): State<AppState>,
    Json(request): Json<CreateRfqRequest>,
) -> Result<Json<RfqResponse>, ApiError> {
    let limit_price_1e8 = request
        .limit_price_1e8
        .as_deref()
        .map(|value| parse_fixed_u128("limit_price_1e8", value))
        .transpose()?;
    let rfq = create_rfq_service(
        &state,
        CreateRfqInput {
            taker: request.taker,
            market_id: request.market_id,
            side: request.side,
            size_1e8: parse_fixed_u128("size_1e8", &request.size_1e8)?,
            limit_price_1e8,
            ttl_ms: request.ttl_ms,
        },
    )
    .await?;
    Ok(Json(rfq.into()))
}

async fn list_rfqs(State(state): State<AppState>) -> Result<Json<Vec<RfqResponse>>, ApiError> {
    Ok(Json(
        list_rfqs_service(&state)
            .await?
            .into_iter()
            .map(RfqResponse::from)
            .collect(),
    ))
}

async fn get_rfq(
    State(state): State<AppState>,
    Path(rfq_id): Path<String>,
) -> Result<Json<RfqResponse>, ApiError> {
    Ok(Json(
        get_rfq_service(&state, parse_rfq_id(&rfq_id)?)
            .await?
            .into(),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct SubmitQuoteRequest {
    mm_account: AccountId,
    price_1e8: String,
    size_1e8: String,
    client_quote_id: Option<String>,
    quote_nonce: Option<u64>,
    quote_ttl_ms: u64,
    signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RfqQuoteResponse {
    quote_id: Uuid,
    rfq_id: Uuid,
    mm_account: AccountId,
    session_id: Option<String>,
    client_quote_id: Option<String>,
    price_1e8: String,
    size_1e8: String,
    status: RfqQuoteStatus,
    created_at_ms: i64,
    expires_at_ms: i64,
    signature: Option<String>,
    quote_digest: Option<String>,
    quote_nonce: Option<String>,
    signature_status: RfqQuoteSignatureStatus,
    recovered_signer: Option<AccountId>,
}

impl From<RfqQuote> for RfqQuoteResponse {
    fn from(quote: RfqQuote) -> Self {
        let now = now_ms();
        let status = quote.effective_status(now);
        Self {
            quote_id: quote.quote_id,
            rfq_id: quote.rfq_id,
            mm_account: quote.mm_account,
            session_id: quote.session_id,
            client_quote_id: quote.client_quote_id,
            price_1e8: quote.price_1e8.to_string(),
            size_1e8: quote.size_1e8.to_string(),
            status,
            created_at_ms: quote.created_at_ms,
            expires_at_ms: quote.expires_at_ms,
            signature: quote.signature,
            quote_digest: quote.quote_digest,
            quote_nonce: quote.quote_nonce,
            signature_status: quote.signature_status,
            recovered_signer: quote.recovered_signer,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct RfqQuoteSigningPayloadRequest {
    mm_account: AccountId,
    price_1e8: String,
    size_1e8: String,
    client_quote_id: Option<String>,
    quote_nonce: u64,
    quote_ttl_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RfqQuoteSigningPayloadResponse {
    rfq_id: Uuid,
    rfq_id_b32: String,
    digest: String,
    domain: SigningPayloadDomain,
    primary_type: &'static str,
    types: Vec<SigningPayloadTypeField>,
    message: RfqQuoteSigningPayloadMessage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RfqQuoteSigningPayloadMessage {
    #[serde(rename = "rfqId")]
    rfq_id: String,
    #[serde(rename = "mmAccount")]
    mm_account: String,
    #[serde(rename = "marketId")]
    market_id: String,
    #[serde(rename = "takerIsBuyer")]
    taker_is_buyer: bool,
    #[serde(rename = "price1e8")]
    price_1e8: String,
    #[serde(rename = "size1e8")]
    size_1e8: String,
    #[serde(rename = "quoteNonce")]
    quote_nonce: String,
    expiry: String,
}

async fn rfq_quote_payload(
    State(state): State<AppState>,
    Path(rfq_id): Path<String>,
    Json(request): Json<RfqQuoteSigningPayloadRequest>,
) -> Result<Json<RfqQuoteSigningPayloadResponse>, ApiError> {
    let rfq_id = parse_rfq_id(&rfq_id)?;
    let outcome = rfq_quote_signing_payload(
        &state,
        QuoteSigningPayloadInput {
            rfq_id,
            mm_account: request.mm_account,
            price_1e8: parse_fixed_u128("price_1e8", &request.price_1e8)?,
            size_1e8: parse_fixed_u128("size_1e8", &request.size_1e8)?,
            quote_nonce: request.quote_nonce,
            quote_ttl_ms: request.quote_ttl_ms,
        },
    )
    .await?;
    let domain = state.rfq_config.eip712_domain.clone();
    let _ = request.client_quote_id;

    Ok(Json(RfqQuoteSigningPayloadResponse {
        rfq_id,
        rfq_id_b32: rfq_id_to_hex_bytes32(&rfq_id.to_string()),
        digest: outcome.digest,
        domain: SigningPayloadDomain {
            name: domain.name,
            version: domain.version,
            chain_id: domain.chain_id,
            verifying_contract: domain.verifying_contract.0,
        },
        primary_type: "RFQQuote",
        types: rfq_quote_type_fields(),
        message: RfqQuoteSigningPayloadMessage {
            rfq_id: rfq_id_to_hex_bytes32(&outcome.rfq.rfq_id.to_string()),
            mm_account: outcome.payload.mm_account.0,
            market_id: outcome.payload.market_id.to_string(),
            taker_is_buyer: outcome.payload.taker_is_buyer,
            price_1e8: outcome.payload.price_1e8.to_string(),
            size_1e8: outcome.payload.size_1e8.to_string(),
            quote_nonce: outcome.payload.quote_nonce.to_string(),
            expiry: outcome.payload.expiry.to_string(),
        },
    }))
}

async fn submit_quote(
    State(state): State<AppState>,
    Path(rfq_id): Path<String>,
    Json(request): Json<SubmitQuoteRequest>,
) -> Result<Json<RfqQuoteResponse>, ApiError> {
    let quote = submit_rfq_quote(
        &state,
        SubmitQuoteInput {
            rfq_id: parse_rfq_id(&rfq_id)?,
            mm_account: request.mm_account,
            session_id: None,
            client_quote_id: request.client_quote_id,
            price_1e8: parse_fixed_u128("price_1e8", &request.price_1e8)?,
            size_1e8: parse_fixed_u128("size_1e8", &request.size_1e8)?,
            quote_nonce: request.quote_nonce,
            quote_ttl_ms: request.quote_ttl_ms,
            signature: request.signature,
        },
    )
    .await?;
    Ok(Json(quote.into()))
}

async fn list_quotes(
    State(state): State<AppState>,
    Path(rfq_id): Path<String>,
) -> Result<Json<Vec<RfqQuoteResponse>>, ApiError> {
    Ok(Json(
        list_rfq_quotes_service(&state, parse_rfq_id(&rfq_id)?)
            .await?
            .into_iter()
            .map(RfqQuoteResponse::from)
            .collect(),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct AcceptQuoteResponse {
    rfq_id: Uuid,
    quote_id: Uuid,
    status: RfqStatus,
    execution_intent_id: Uuid,
    onchain_intent_id: String,
    mm_notification_sent: bool,
    mm_notification_warning: Option<String>,
}

impl From<AcceptQuoteOutcome> for AcceptQuoteResponse {
    fn from(outcome: AcceptQuoteOutcome) -> Self {
        Self {
            rfq_id: outcome.rfq_id,
            quote_id: outcome.quote_id,
            status: outcome.status,
            execution_intent_id: outcome.execution_intent_id,
            onchain_intent_id: outcome.onchain_intent_id,
            mm_notification_sent: outcome.mm_notification_sent,
            mm_notification_warning: outcome.mm_notification_warning,
        }
    }
}

async fn accept_quote(
    State(state): State<AppState>,
    Path((rfq_id, quote_id)): Path<(String, String)>,
) -> Result<Json<AcceptQuoteResponse>, ApiError> {
    Ok(Json(
        accept_rfq_quote(&state, parse_rfq_id(&rfq_id)?, parse_quote_id(&quote_id)?)
            .await?
            .into(),
    ))
}

async fn cancel_rfq(
    State(state): State<AppState>,
    Path(rfq_id): Path<String>,
) -> Result<Json<RfqResponse>, ApiError> {
    Ok(Json(
        cancel_rfq_service(&state, parse_rfq_id(&rfq_id)?)
            .await?
            .into(),
    ))
}

async fn execution_intents(
    State(state): State<AppState>,
) -> Result<Json<Vec<ApiExecutionIntent>>, ApiError> {
    if let Some(repository) = state.repository.clone() {
        return Ok(Json(
            repository
                .list_execution_intents()
                .await?
                .into_iter()
                .map(ApiExecutionIntent::from)
                .collect(),
        ));
    }

    let engine = state.engine.lock().map_err(|_| ApiError::internal())?;
    Ok(Json(
        engine
            .execution_intents()
            .into_iter()
            .map(ApiExecutionIntent::from)
            .collect(),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SigningPayloadResponse {
    intent_id: Uuid,
    domain: SigningPayloadDomain,
    primary_type: &'static str,
    types: Vec<SigningPayloadTypeField>,
    message: SigningPayloadMessage,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SigningPayloadDomain {
    name: String,
    version: String,
    #[serde(rename = "chainId")]
    chain_id: u64,
    #[serde(rename = "verifyingContract")]
    verifying_contract: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SigningPayloadTypeField {
    name: &'static str,
    #[serde(rename = "type")]
    type_name: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SigningPayloadMessage {
    #[serde(rename = "intentId")]
    intent_id: String,
    buyer: String,
    seller: String,
    #[serde(rename = "marketId")]
    market_id: String,
    #[serde(rename = "sizeDelta1e8")]
    size_delta_1e8: String,
    #[serde(rename = "executionPrice1e8")]
    execution_price_1e8: String,
    #[serde(rename = "buyerIsMaker")]
    buyer_is_maker: bool,
    #[serde(rename = "buyerNonce")]
    buyer_nonce: String,
    #[serde(rename = "sellerNonce")]
    seller_nonce: String,
    deadline: String,
}

async fn execution_intent_signing_payload(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<SigningPayloadResponse>, ApiError> {
    let intent_id = parse_uuid(&intent_id)?;
    let intent = get_execution_intent(&state, intent_id).await?;
    let payload = intent.perp_trade_payload()?;
    let domain = perp_trade_domain(&state);
    let digest = perp_trade_digest(&payload, &domain)?;

    Ok(Json(SigningPayloadResponse {
        intent_id,
        domain: SigningPayloadDomain {
            name: domain.name,
            version: domain.version,
            chain_id: domain.chain_id,
            verifying_contract: domain.verifying_contract.0,
        },
        primary_type: "PerpTrade",
        types: perp_trade_type_fields(),
        message: signing_payload_message(payload),
        digest,
    }))
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct SubmitTradeSignaturesRequest {
    buyer_sig: Option<String>,
    seller_sig: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SubmitTradeSignaturesResponse {
    intent_id: Uuid,
    buyer_signature_present: bool,
    seller_signature_present: bool,
    calldata_ready: bool,
    missing_signatures: bool,
}

async fn submit_execution_intent_signatures(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
    Json(request): Json<SubmitTradeSignaturesRequest>,
) -> Result<Json<SubmitTradeSignaturesResponse>, ApiError> {
    let intent_id = parse_uuid(&intent_id)?;
    let intent = get_execution_intent(&state, intent_id).await?;
    let signatures =
        upsert_trade_signatures(&state, intent_id, request.buyer_sig, request.seller_sig).await?;

    let status = TradeSignatureStatus::from(&signatures);
    if status.calldata_ready {
        let payload = intent.perp_trade_payload()?;
        let bundle = signatures
            .bundle()?
            .ok_or(BackendError::MissingTradeSignatures)?;
        crate::execution::build_perp_execution_call(
            &state.execution_config.perp_matching_engine_address,
            intent_id,
            &payload,
            Some(&bundle),
        )?;
        update_execution_intent_status(&state, intent_id, ExecutionIntentStatus::CalldataReady)
            .await?;
    }

    Ok(Json(SubmitTradeSignaturesResponse {
        intent_id,
        buyer_signature_present: status.buyer_signature_present,
        seller_signature_present: status.seller_signature_present,
        calldata_ready: status.calldata_ready,
        missing_signatures: status.missing_signatures,
    }))
}

async fn executor_status(State(state): State<AppState>) -> Json<crate::execution::ExecutionStatus> {
    Json(state.execution_config.status())
}

async fn executor_health_v2(
    State(state): State<AppState>,
) -> Json<crate::api::ExecutorHealthV2Response> {
    Json(crate::api::build_executor_health_v2(&state))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ExecutorTickResponse {
    #[serde(rename = "pendingSeen")]
    pending_seen: usize,
    #[serde(rename = "dryRunUpdated")]
    dry_run_updated: usize,
    #[serde(rename = "calldataReady")]
    calldata_ready: usize,
    #[serde(rename = "missingSignatures")]
    missing_signatures: usize,
    #[serde(rename = "callsPrepared")]
    calls_prepared: usize,
}

async fn executor_tick(
    State(state): State<AppState>,
) -> Result<Json<ExecutorTickResponse>, ApiError> {
    if !state.execution_config.execution_enabled {
        return Err(BackendError::Config("execution is disabled".to_string()).into());
    }
    if !state.execution_config.dry_run {
        return Err(BackendError::Config(
            "real on-chain execution is not implemented yet; set EXECUTOR_DRY_RUN=true".to_string(),
        )
        .into());
    }

    let repository = state
        .repository
        .clone()
        .ok_or_else(|| BackendError::Config("executor requires persistence enabled".to_string()))?;
    let result = Executor::new(state.execution_config, repository)
        .tick()
        .await?;

    Ok(Json(ExecutorTickResponse {
        pending_seen: result.pending_seen,
        dry_run_updated: result.dry_run_updated,
        calldata_ready: result.calldata_ready,
        missing_signatures: result.missing_signatures,
        calls_prepared: result.prepared_calls.len(),
    }))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SimulationResponse {
    intent_id: Uuid,
    simulation_status: ExecutionIntentStatus,
    block_number: Option<u64>,
    error: Option<String>,
    revert_data: Option<String>,
    revert_selector: Option<String>,
    decoded_error: Option<DecodedRevertError>,
    submitted: bool,
    confirmed: bool,
}

impl From<SimulationResult> for SimulationResponse {
    fn from(result: SimulationResult) -> Self {
        Self {
            intent_id: result.intent_id,
            simulation_status: result.status,
            block_number: result.block_number,
            error: result.error,
            revert_data: result.revert_data,
            revert_selector: result.revert_selector,
            decoded_error: result.decoded_error,
            submitted: false,
            confirmed: false,
        }
    }
}

async fn simulate_executor_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<SimulationResponse>, ApiError> {
    if !state.execution_config.simulation_enabled {
        return Err(BackendError::Config("simulation is disabled".to_string()).into());
    }
    if state.execution_config.simulation_requires_persistence && state.repository.is_none() {
        return Err(
            BackendError::Config("simulation requires persistence enabled".to_string()).into(),
        );
    }

    let intent_id = parse_uuid(&intent_id)?;
    let intent = get_execution_intent(&state, intent_id).await?;
    let signatures = get_trade_signatures(&state, intent_id).await?;
    if !signatures.calldata_ready() {
        return Err(BackendError::MissingTradeSignatures.into());
    }

    let rpc_url =
        state.execution_config.rpc_url.clone().ok_or_else(|| {
            BackendError::Config("RPC_URL is required for simulation".to_string())
        })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    let result =
        simulate_execution_intent(&provider, &state.execution_config, &intent, &signatures).await?;
    persist_simulation_result(&state, &result).await?;

    Ok(Json(SimulationResponse::from(result)))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct BroadcastResponse {
    intent_id: Uuid,
    onchain_intent_id: String,
    broadcast_enabled: bool,
    submitted: bool,
    confirmed: bool,
    tx_hash: Option<String>,
    reason: Option<String>,
}

async fn broadcast_executor_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<BroadcastResponse>, ApiError> {
    let intent_id = parse_uuid(&intent_id)?;
    let onchain_intent_id = crate::execution::intent_id_to_hex_bytes32(&intent_id.to_string())?;

    if !state.execution_config.real_broadcast_enabled {
        return Ok(Json(BroadcastResponse {
            intent_id,
            onchain_intent_id,
            broadcast_enabled: false,
            submitted: false,
            confirmed: false,
            tx_hash: None,
            reason: Some("broadcast disabled".to_string()),
        }));
    }

    let Some(repository) = state.repository.clone() else {
        return Err(
            BackendError::Config("broadcast requires persistence enabled".to_string()).into(),
        );
    };
    ensure_no_submitted_transaction(
        repository
            .find_submitted_transaction_by_intent(intent_id)
            .await?
            .is_some(),
    )?;

    let intent = get_execution_intent(&state, intent_id).await?;
    let signatures = get_trade_signatures(&state, intent_id).await?;
    let request =
        build_execution_transaction_request(&state.execution_config, &intent, &signatures)?;
    let now = now_ms();
    let calldata = request.calldata_hex();
    let Some(private_key) = state.execution_config.executor_private_key.as_ref() else {
        return Err(BackendError::Config(
            "EXECUTOR_PRIVATE_KEY is required when EXECUTOR_REAL_BROADCAST_ENABLED=true"
                .to_string(),
        )
        .into());
    };
    let Some(rpc_url) = state.execution_config.rpc_url.clone() else {
        return Err(BackendError::Config(
            "RPC_URL is required when EXECUTOR_REAL_BROADCAST_ENABLED=true".to_string(),
        )
        .into());
    };
    let signer = ExecutorSigner::from_private_key(private_key)?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    let rpc_chain_id = provider.chain_id().await?;
    if rpc_chain_id != request.chain_id {
        let transaction = execution_transaction_from_request(
            &request,
            calldata,
            ExecutionTransactionStatus::Rejected,
            None,
            Some(format!(
                "RPC chain id {rpc_chain_id} does not match EXECUTOR_CHAIN_ID {}",
                request.chain_id
            )),
            now,
        );
        repository
            .insert_execution_transaction(&transaction)
            .await?;
        return Err(BackendError::BroadcastRejected(
            "RPC chain id does not match EXECUTOR_CHAIN_ID".to_string(),
        )
        .into());
    }
    let nonce = provider.transaction_count(signer.address().clone()).await?;
    let raw_transaction = crate::execution::sign_eip1559_transaction(&request, nonce, &signer)?;
    let tx_hash = match provider.send_raw_transaction(raw_transaction).await {
        Ok(tx_hash) => tx_hash,
        Err(error) => {
            let transaction = execution_transaction_from_request(
                &request,
                calldata,
                ExecutionTransactionStatus::Failed,
                None,
                Some(error.to_string()),
                now,
            );
            repository
                .insert_execution_transaction(&transaction)
                .await?;
            return Err(error.into());
        }
    };
    let transaction = execution_transaction_from_request(
        &request,
        calldata,
        ExecutionTransactionStatus::Submitted,
        Some(tx_hash.clone()),
        None,
        now,
    );
    repository
        .insert_execution_transaction(&transaction)
        .await?;
    update_execution_intent_status(&state, intent_id, ExecutionIntentStatus::Submitted).await?;

    Ok(Json(BroadcastResponse {
        intent_id,
        onchain_intent_id: request.onchain_intent_id,
        broadcast_enabled: true,
        submitted: true,
        confirmed: false,
        tx_hash: Some(tx_hash),
        reason: None,
    }))
}

fn execution_transaction_from_request(
    request: &crate::execution::ExecutionTransactionRequest,
    calldata: String,
    status: ExecutionTransactionStatus,
    tx_hash: Option<String>,
    error: Option<String>,
    now: i64,
) -> ExecutionTransaction {
    ExecutionTransaction {
        transaction_id: Uuid::new_v4().to_string(),
        intent_id: request.intent_id,
        onchain_intent_id: Some(request.onchain_intent_id.clone()),
        target: request.to.clone(),
        calldata,
        value_wei: request.value_wei.to_string(),
        tx_hash,
        status,
        error,
        confirmed_at_ms: None,
        confirmed_block_number: None,
        confirmation_status: None,
        confirmation_error: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct ExecutorTransactionsQuery {
    limit: Option<u32>,
}

async fn executor_transactions(
    State(state): State<AppState>,
    Query(query): Query<ExecutorTransactionsQuery>,
) -> Result<Json<Vec<ExecutionTransaction>>, ApiError> {
    let Some(repository) = state.repository.clone() else {
        return Ok(Json(Vec::new()));
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(
        repository.list_recent_execution_transactions(limit).await?,
    ))
}

async fn executor_transactions_for_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<Vec<ExecutionTransaction>>, ApiError> {
    let Some(repository) = state.repository.clone() else {
        return Ok(Json(Vec::new()));
    };
    let intent_id = parse_uuid(&intent_id)?;
    Ok(Json(
        repository.get_transactions_for_intent(intent_id).await?,
    ))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ConfirmationStatusResponse {
    #[serde(rename = "confirmationEnabled")]
    confirmation_enabled: bool,
    #[serde(rename = "persistenceRequired")]
    persistence_required: bool,
    #[serde(rename = "rpcConfigured")]
    rpc_configured: bool,
    #[serde(rename = "requiredConfirmations")]
    required_confirmations: u64,
    #[serde(rename = "maxBatchSize")]
    max_batch_size: u32,
    #[serde(rename = "requireReconciliation")]
    require_reconciliation: bool,
    confirmed: u64,
}

async fn confirmation_status(
    State(state): State<AppState>,
) -> Result<Json<ConfirmationStatusResponse>, ApiError> {
    let confirmed = if let Some(repository) = state.repository.clone() {
        repository.count_confirmed_execution_transactions().await?
    } else {
        0
    };

    Ok(Json(ConfirmationStatusResponse {
        confirmation_enabled: state.confirmation_config.enabled,
        persistence_required: state.confirmation_config.require_persistence,
        rpc_configured: state.confirmation_config.rpc_url.is_some(),
        required_confirmations: state.confirmation_config.required_blocks,
        max_batch_size: state.confirmation_config.max_batch_size,
        require_reconciliation: state.confirmation_config.require_reconciliation,
        confirmed,
    }))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ConfirmationResponse {
    intent_id: Uuid,
    tx_hash: Option<String>,
    confirmation_status: ConfirmationStatus,
    confirmed: bool,
    receipt_status: Option<u64>,
    receipt_block_number: Option<u64>,
    current_block_number: Option<u64>,
    required_confirmations: u64,
    indexed_event_found: bool,
    reconciliation_matched: bool,
    reason: Option<String>,
}

impl ConfirmationResponse {
    fn from_decision(
        intent_id: Uuid,
        tx_hash: Option<String>,
        decision: ConfirmationDecision,
    ) -> Self {
        Self {
            intent_id,
            tx_hash,
            confirmation_status: decision.confirmation_status,
            confirmed: decision.confirmed,
            receipt_status: decision.receipt_status,
            receipt_block_number: decision.receipt_block_number,
            current_block_number: decision.current_block_number,
            required_confirmations: decision.required_confirmations,
            indexed_event_found: decision.indexed_event_found,
            reconciliation_matched: decision.reconciliation_matched,
            reason: decision.reason,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ConfirmationTickResponse {
    processed: usize,
    confirmed: usize,
    pending: usize,
    failed: usize,
    results: Vec<ConfirmationResponse>,
}

async fn confirm_executor_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<ConfirmationResponse>, ApiError> {
    ensure_confirmation_enabled(&state)?;
    let repository = confirmation_repository(&state)?;
    let provider = confirmation_provider(&state)?;
    let intent_id = parse_uuid(&intent_id)?;
    let Some(transaction) = repository
        .find_submitted_transaction_by_intent(intent_id)
        .await?
    else {
        return Ok(Json(ConfirmationResponse::from_decision(
            intent_id,
            None,
            missing_submitted_transaction_decision(&state),
        )));
    };

    Ok(Json(
        confirm_transaction(&state, &repository, &provider, transaction).await?,
    ))
}

async fn confirmation_tick(
    State(state): State<AppState>,
) -> Result<Json<ConfirmationTickResponse>, ApiError> {
    ensure_confirmation_enabled(&state)?;
    let repository = confirmation_repository(&state)?;
    let provider = confirmation_provider(&state)?;
    let transactions = repository
        .list_submitted_unconfirmed_execution_transactions(state.confirmation_config.max_batch_size)
        .await?;
    let mut results = Vec::with_capacity(transactions.len());
    for transaction in transactions {
        results.push(confirm_transaction(&state, &repository, &provider, transaction).await?);
    }
    let confirmed = results.iter().filter(|result| result.confirmed).count();
    let failed = results
        .iter()
        .filter(|result| result.confirmation_status == ConfirmationStatus::Failed)
        .count();
    let pending = results.len().saturating_sub(confirmed + failed);

    Ok(Json(ConfirmationTickResponse {
        processed: results.len(),
        confirmed,
        pending,
        failed,
        results,
    }))
}

async fn confirmation_for_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<ConfirmationResponse>, ApiError> {
    let repository = confirmation_repository(&state)?;
    let intent_id = parse_uuid(&intent_id)?;
    let Some(transaction) = repository
        .find_submitted_transaction_by_intent(intent_id)
        .await?
    else {
        return Ok(Json(ConfirmationResponse::from_decision(
            intent_id,
            None,
            missing_submitted_transaction_decision(&state),
        )));
    };

    Ok(Json(ConfirmationResponse {
        intent_id,
        tx_hash: transaction.tx_hash,
        confirmation_status: transaction
            .confirmation_status
            .unwrap_or(ConfirmationStatus::Pending),
        confirmed: transaction.confirmation_status == Some(ConfirmationStatus::Confirmed),
        receipt_status: None,
        receipt_block_number: transaction.confirmed_block_number,
        current_block_number: None,
        required_confirmations: state.confirmation_config.required_blocks,
        indexed_event_found: false,
        reconciliation_matched: false,
        reason: transaction.confirmation_error,
    }))
}

async fn confirm_transaction(
    state: &AppState,
    repository: &PgRepository,
    provider: &HttpJsonRpcProvider,
    transaction: ExecutionTransaction,
) -> BackendResult<ConfirmationResponse> {
    let tx_hash = transaction.tx_hash.clone();
    let Some(tx_hash_value) = tx_hash.clone() else {
        let decision = ConfirmationDecision {
            confirmation_status: ConfirmationStatus::Failed,
            confirmed: false,
            receipt_status: None,
            receipt_block_number: None,
            current_block_number: None,
            required_confirmations: state.confirmation_config.required_blocks,
            indexed_event_found: false,
            reconciliation_matched: false,
            reason: Some("submitted transaction hash not found".to_string()),
        };
        repository
            .apply_confirmation_decision(
                &transaction.transaction_id,
                transaction.intent_id,
                &decision,
                now_ms(),
            )
            .await?;
        return Ok(ConfirmationResponse::from_decision(
            transaction.intent_id,
            tx_hash,
            decision,
        ));
    };
    let Some(onchain_intent_id) = transaction.onchain_intent_id.as_deref() else {
        let decision = ConfirmationDecision {
            confirmation_status: ConfirmationStatus::Failed,
            confirmed: false,
            receipt_status: None,
            receipt_block_number: None,
            current_block_number: None,
            required_confirmations: state.confirmation_config.required_blocks,
            indexed_event_found: false,
            reconciliation_matched: false,
            reason: Some("onchain intent id not found for submitted transaction".to_string()),
        };
        repository
            .apply_confirmation_decision(
                &transaction.transaction_id,
                transaction.intent_id,
                &decision,
                now_ms(),
            )
            .await?;
        return Ok(ConfirmationResponse::from_decision(
            transaction.intent_id,
            tx_hash,
            decision,
        ));
    };

    let receipt = provider.transaction_receipt(tx_hash_value.clone()).await?;
    let current_block_number = if receipt.is_some() {
        Some(provider.block_number().await?)
    } else {
        None
    };
    let indexed_event_found = repository
        .has_indexed_trade_for_confirmation(&tx_hash_value, onchain_intent_id)
        .await?;
    let reconciliation_matched = repository
        .has_matched_reconciliation_for_confirmation(
            transaction.intent_id,
            onchain_intent_id,
            &tx_hash_value,
        )
        .await?;
    let decision = decide_confirmation(&ConfirmationDecisionInput {
        tx_hash: tx_hash.clone(),
        receipt,
        current_block_number,
        required_confirmations: state.confirmation_config.required_blocks,
        indexed_event_found,
        reconciliation_matched,
        require_reconciliation: state.confirmation_config.require_reconciliation,
    });
    if decision.confirmed && state.fees_config.enabled {
        let indexed_trade = repository
            .find_matched_indexed_trade_for_confirmation(
                transaction.intent_id,
                onchain_intent_id,
                &tx_hash_value,
            )
            .await?
            .ok_or_else(|| {
                BackendError::Persistence(
                    "matched indexed perp trade not found for confirmed transaction".to_string(),
                )
            })?;
        let flow_type = repository
            .perp_fee_flow_for_intent(transaction.intent_id)
            .await?;
        record_indexed_perp_trade_fees(state, &indexed_trade, flow_type).await?;
    }
    repository
        .apply_confirmation_decision(
            &transaction.transaction_id,
            transaction.intent_id,
            &decision,
            now_ms(),
        )
        .await?;

    Ok(ConfirmationResponse::from_decision(
        transaction.intent_id,
        tx_hash,
        decision,
    ))
}

fn ensure_confirmation_enabled(state: &AppState) -> BackendResult<()> {
    if state.confirmation_config.enabled {
        Ok(())
    } else {
        Err(BackendError::Config("confirmation is disabled".to_string()))
    }
}

fn confirmation_repository(state: &AppState) -> BackendResult<PgRepository> {
    state.repository.clone().ok_or_else(|| {
        BackendError::Config("confirmation requires persistence enabled".to_string())
    })
}

fn confirmation_provider(state: &AppState) -> BackendResult<HttpJsonRpcProvider> {
    let rpc_url =
        state.confirmation_config.rpc_url.clone().ok_or_else(|| {
            BackendError::Config("RPC_URL is required for confirmation".to_string())
        })?;
    Ok(HttpJsonRpcProvider::new(rpc_url))
}

fn missing_submitted_transaction_decision(state: &AppState) -> ConfirmationDecision {
    ConfirmationDecision {
        confirmation_status: ConfirmationStatus::Failed,
        confirmed: false,
        receipt_status: None,
        receipt_block_number: None,
        current_block_number: None,
        required_confirmations: state.confirmation_config.required_blocks,
        indexed_event_found: false,
        reconciliation_matched: false,
        reason: Some("submitted execution transaction not found".to_string()),
    }
}

async fn indexer_status(State(state): State<AppState>) -> Result<Json<IndexerStatus>, ApiError> {
    let last_indexed_block = if let Some(repository) = state.repository.clone() {
        repository
            .get_indexer_cursor(crate::indexer::runner::PERP_MATCHING_ENGINE_CURSOR)
            .await?
            .unwrap_or(state.indexer_config.start_block)
    } else {
        state.indexer_config.start_block
    };

    Ok(Json(IndexerStatus {
        indexer_enabled: state.indexer_config.enabled,
        rpc_configured: state.indexer_config.rpc_url.is_some(),
        persistence_required: state.indexer_config.require_persistence,
        last_indexed_block,
        target_contract: state.indexer_config.perp_matching_engine_address.0.clone(),
    }))
}

async fn indexer_tick(State(state): State<AppState>) -> Result<Json<IndexerTickResult>, ApiError> {
    if !state.indexer_config.enabled {
        return Err(BackendError::Config("indexer is disabled".to_string()).into());
    }
    let repository = state
        .repository
        .clone()
        .ok_or_else(|| BackendError::Config("indexer requires persistence enabled".to_string()))?;
    let indexer = Indexer::from_config_and_repository(state.indexer_config, repository)?;
    Ok(Json(indexer.tick().await?))
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct IndexedPerpTradesQuery {
    limit: Option<u32>,
}

async fn indexed_perp_trades(
    State(state): State<AppState>,
    Query(query): Query<IndexedPerpTradesQuery>,
) -> Result<Json<Vec<crate::indexer::IndexedPerpTrade>>, ApiError> {
    let Some(repository) = state.repository.clone() else {
        return Ok(Json(Vec::new()));
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(repository.list_indexed_perp_trades(limit).await?))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ReconciliationStatusResponse {
    #[serde(rename = "reconciliationEnabled")]
    reconciliation_enabled: bool,
    #[serde(rename = "persistenceRequired")]
    persistence_required: bool,
    #[serde(rename = "matchedReconciliations")]
    matched_reconciliations: u64,
    #[serde(rename = "ambiguousReconciliations")]
    ambiguous_reconciliations: u64,
    #[serde(rename = "unmatchedReconciliations")]
    unmatched_reconciliations: u64,
    confirmed: u64,
}

async fn reconciliation_status(
    State(state): State<AppState>,
) -> Result<Json<ReconciliationStatusResponse>, ApiError> {
    let counts = if let Some(repository) = state.repository.clone() {
        repository.count_reconciliations_by_status().await?
    } else {
        ReconciliationCounts::default()
    };

    Ok(Json(ReconciliationStatusResponse {
        reconciliation_enabled: state.reconciliation_config.enabled,
        persistence_required: state.reconciliation_config.require_persistence,
        matched_reconciliations: counts.matched,
        ambiguous_reconciliations: counts.ambiguous,
        unmatched_reconciliations: counts.unmatched,
        confirmed: counts.confirmed(),
    }))
}

async fn reconciliation_tick(
    State(state): State<AppState>,
) -> Result<Json<ReconciliationTickResult>, ApiError> {
    if !state.reconciliation_config.enabled {
        return Err(BackendError::Config("reconciliation is disabled".to_string()).into());
    }
    let repository = state.repository.clone().ok_or_else(|| {
        BackendError::Config("reconciliation requires persistence enabled".to_string())
    })?;

    let indexed_trades = repository
        .list_unreconciled_indexed_perp_trades(state.reconciliation_config.max_batch_size)
        .await?;
    let mut result = ReconciliationTickResult::default();
    for indexed_trade in indexed_trades {
        result.indexed_trades_checked += 1;
        let Some(onchain_intent_id) = indexed_trade.onchain_intent_id.as_deref() else {
            continue;
        };
        let matching_intents = repository
            .find_execution_intents_by_onchain_intent_id(onchain_intent_id)
            .await?;
        let matching_indexed_trades = repository
            .find_indexed_trades_by_onchain_intent_id(onchain_intent_id)
            .await?;
        let decision = decide_direct_reconciliation(&DirectReconciliationInput {
            onchain_intent_id: Some(onchain_intent_id.to_string()),
            matching_intent_count: matching_intents.len(),
            matching_indexed_event_count: matching_indexed_trades.len(),
        });

        match decision {
            ReconciliationStatus::Matched => {
                if let Some(intent) = matching_intents.first() {
                    let row = reconciliation_row(
                        onchain_intent_id,
                        &intent.intent_id.to_string(),
                        &indexed_trade,
                        ReconciliationStatus::Matched,
                    );
                    repository.insert_execution_reconciliation(&row).await?;
                }
                result.matched += 1;
            }
            ReconciliationStatus::Ambiguous => {
                for intent in &matching_intents {
                    let row = reconciliation_row(
                        onchain_intent_id,
                        &intent.intent_id.to_string(),
                        &indexed_trade,
                        ReconciliationStatus::Ambiguous,
                    );
                    repository.insert_execution_reconciliation(&row).await?;
                }
                result.ambiguous += 1;
            }
            ReconciliationStatus::Unmatched => {
                result.unmatched += 1;
            }
            ReconciliationStatus::Ignored => {}
        }
    }
    result.confirmed = 0;

    Ok(Json(result))
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct ReconciliationsQuery {
    limit: Option<u32>,
}

async fn reconciliations(
    State(state): State<AppState>,
    Query(query): Query<ReconciliationsQuery>,
) -> Result<Json<Vec<ExecutionReconciliation>>, ApiError> {
    let Some(repository) = state.repository.clone() else {
        return Ok(Json(Vec::new()));
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(repository.list_recent_reconciliations(limit).await?))
}

async fn reconciliations_for_intent(
    State(state): State<AppState>,
    Path(intent_id): Path<String>,
) -> Result<Json<Vec<ExecutionReconciliation>>, ApiError> {
    let Some(repository) = state.repository.clone() else {
        return Ok(Json(Vec::new()));
    };
    let intent_id = parse_uuid(&intent_id)?;
    Ok(Json(
        repository.get_reconciliations_for_intent(intent_id).await?,
    ))
}

fn reconciliation_row(
    onchain_intent_id: &str,
    intent_id: &str,
    indexed_trade: &crate::indexer::IndexedPerpTrade,
    status: ReconciliationStatus,
) -> ExecutionReconciliation {
    ExecutionReconciliation {
        reconciliation_id: Uuid::new_v4().to_string(),
        onchain_intent_id: onchain_intent_id.to_string(),
        intent_id: intent_id.to_string(),
        indexed_event_id: indexed_trade.event_id.clone(),
        tx_hash: indexed_trade.tx_hash.clone(),
        block_number: indexed_trade.block_number,
        log_index: indexed_trade.log_index,
        status,
        created_at_ms: now_ms(),
    }
}

async fn get_execution_intent(
    state: &AppState,
    intent_id: Uuid,
) -> BackendResult<crate::execution::ExecutionIntent> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_execution_intent(intent_id)
            .await?
            .ok_or(BackendError::InvalidExecutionIntentId);
    }

    let engine = state
        .engine
        .lock()
        .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
    engine
        .execution_intents()
        .into_iter()
        .find(|intent| intent.intent_id == intent_id)
        .ok_or(BackendError::InvalidExecutionIntentId)
}

async fn upsert_trade_signatures(
    state: &AppState,
    intent_id: Uuid,
    buyer_sig: Option<String>,
    seller_sig: Option<String>,
) -> BackendResult<StoredTradeSignatures> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .upsert_execution_intent_signatures(intent_id, buyer_sig, seller_sig, now_ms())
            .await;
    }

    let mut signatures = state
        .trade_signatures
        .lock()
        .map_err(|_| BackendError::Config("signature store lock poisoned".to_string()))?;
    let entry = signatures.entry(intent_id).or_default();
    entry.upsert(buyer_sig, seller_sig)?;
    Ok(entry.clone())
}

async fn get_trade_signatures(
    state: &AppState,
    intent_id: Uuid,
) -> BackendResult<StoredTradeSignatures> {
    if let Some(repository) = state.repository.clone() {
        return repository.get_execution_intent_signatures(intent_id).await;
    }

    let signatures = state
        .trade_signatures
        .lock()
        .map_err(|_| BackendError::Config("signature store lock poisoned".to_string()))?;
    Ok(signatures.get(&intent_id).cloned().unwrap_or_default())
}

async fn update_execution_intent_status(
    state: &AppState,
    intent_id: Uuid,
    status: ExecutionIntentStatus,
) -> BackendResult<()> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .update_execution_intent_status(intent_id, status, now_ms())
            .await;
    }

    let mut engine = state
        .engine
        .lock()
        .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
    if engine.update_execution_intent_status(intent_id, status) {
        Ok(())
    } else {
        Err(BackendError::InvalidExecutionIntentId)
    }
}

async fn persist_simulation_result(
    state: &AppState,
    result: &SimulationResult,
) -> BackendResult<()> {
    if let Some(repository) = state.repository.clone() {
        return repository.persist_simulation_result(result).await;
    }

    let mut engine = state
        .engine
        .lock()
        .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
    if engine.update_execution_intent_status(result.intent_id, result.status) {
        Ok(())
    } else {
        Err(BackendError::InvalidExecutionIntentId)
    }
}

fn parse_uuid(value: &str) -> BackendResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| BackendError::InvalidExecutionIntentId)
}

fn perp_trade_domain(state: &AppState) -> PerpTradeDomain {
    PerpTradeDomain::new(
        state.chain_id,
        state.execution_config.perp_matching_engine_address.clone(),
    )
}

fn signing_payload_message(payload: PerpTradePayload) -> SigningPayloadMessage {
    SigningPayloadMessage {
        intent_id: b256_to_hex_bytes32(&payload.intent_id),
        buyer: payload.buyer.0,
        seller: payload.seller.0,
        market_id: payload.market_id.to_string(),
        size_delta_1e8: payload.size_delta_1e8.to_string(),
        execution_price_1e8: payload.execution_price_1e8.to_string(),
        buyer_is_maker: payload.buyer_is_maker,
        buyer_nonce: payload.buyer_nonce.to_string(),
        seller_nonce: payload.seller_nonce.to_string(),
        deadline: payload.deadline.to_string(),
    }
}

fn rfq_quote_type_fields() -> Vec<SigningPayloadTypeField> {
    let _ = RFQ_QUOTE_TYPE;
    vec![
        SigningPayloadTypeField {
            name: "rfqId",
            type_name: "bytes32",
        },
        SigningPayloadTypeField {
            name: "mmAccount",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "marketId",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "takerIsBuyer",
            type_name: "bool",
        },
        SigningPayloadTypeField {
            name: "price1e8",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "size1e8",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "quoteNonce",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "expiry",
            type_name: "uint256",
        },
    ]
}

fn option_rfq_quote_type_fields() -> Vec<SigningPayloadTypeField> {
    let _ = OPTION_RFQ_QUOTE_TYPE;
    vec![
        SigningPayloadTypeField {
            name: "optionRfqId",
            type_name: "bytes32",
        },
        SigningPayloadTypeField {
            name: "mmAccount",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "optionSeriesId",
            type_name: "bytes32",
        },
        SigningPayloadTypeField {
            name: "takerIsBuyer",
            type_name: "bool",
        },
        SigningPayloadTypeField {
            name: "price1e8",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "size1e8",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "quoteNonce",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "expiry",
            type_name: "uint256",
        },
    ]
}

fn option_trade_type_fields() -> Vec<SigningPayloadTypeField> {
    let _ = OPTION_TRADE_TYPE;
    vec![
        SigningPayloadTypeField {
            name: "intentId",
            type_name: "bytes32",
        },
        SigningPayloadTypeField {
            name: "buyer",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "seller",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "optionId",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "underlying",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "settlementAsset",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "expiry",
            type_name: "uint64",
        },
        SigningPayloadTypeField {
            name: "strike1e8",
            type_name: "uint64",
        },
        SigningPayloadTypeField {
            name: "isCall",
            type_name: "bool",
        },
        SigningPayloadTypeField {
            name: "contractSize1e8",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "quantity",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "premiumPerContract",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "buyerIsMaker",
            type_name: "bool",
        },
        SigningPayloadTypeField {
            name: "buyerNonce",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "sellerNonce",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "deadline",
            type_name: "uint256",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{AdminConfig, MetricsConfig};
    use crate::engine::EngineState;
    use crate::execution::{DecodedRevertError, PrivateKeySecret};
    use crate::fees::{FeeEvent, FeeFlowType, FeeMarketType, FeeSourceType, FeeStatus};
    use crate::mm::protocol::ServerMessage;
    use crate::mm::{
        AuthMode, MmAccountPermissions, MmGatewayConfig, MmPermissionsConfig, MmProductPermission,
        MmSession,
    };
    use crate::nonce_sync::OptionNonceSyncConfig;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    #[test]
    fn simulation_response_keeps_submitted_and_confirmed_false() {
        let result = SimulationResult {
            simulation_id: Uuid::from_u128(1),
            intent_id: Uuid::from_u128(2),
            status: ExecutionIntentStatus::SimulationFailed,
            block_number: None,
            error: Some("simulation failed: execution reverted".to_string()),
            revert_data: Some("0x12345678".to_string()),
            revert_selector: Some("0x12345678".to_string()),
            decoded_error: Some(DecodedRevertError {
                kind: "unknown_custom_error".to_string(),
                name: None,
                selector: Some("0x12345678".to_string()),
                args: None,
                decoded: None,
            }),
            created_at_ms: 123,
        };

        let response = SimulationResponse::from(result);

        assert!(!response.submitted);
        assert!(!response.confirmed);
        assert_eq!(response.revert_data.as_deref(), Some("0x12345678"));
        assert_eq!(
            response.decoded_error.unwrap().kind,
            "unknown_custom_error".to_string()
        );
    }

    #[tokio::test]
    async fn admin_disabled_returns_clear_error() {
        let response = router(AppState::new(EngineState::with_default_markets()))
            .oneshot(get_request("/admin/status", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["error"], "admin API is disabled");
    }

    #[tokio::test]
    async fn metrics_enabled_by_default_renders_backend_up() {
        let response = router(AppState::new(EngineState::with_default_markets()))
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let headers = response.headers().clone();
        let body = response_text(response).await;
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/plain; version=0.0.4; charset=utf-8")
        );
        assert!(body.contains("# HELP deopt_backend_up Backend process is up."));
        assert!(body.contains("# TYPE deopt_backend_up gauge"));
        assert!(body.contains("deopt_backend_up 1"));
        assert!(body.contains("deopt_persistence_enabled 0"));
        assert!(body.contains("deopt_real_broadcast_enabled 0"));
    }

    #[tokio::test]
    async fn metrics_disabled_returns_clear_error() {
        let mut state = AppState::new(EngineState::with_default_markets());
        state.metrics_config.enabled = false;

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["error"], "metrics endpoint is disabled");
    }

    #[tokio::test]
    async fn metrics_token_required_rejects_missing_and_accepts_valid_token() {
        let mut state = admin_state(true);
        state.metrics_config = MetricsConfig {
            enabled: true,
            require_admin_token: true,
        };
        let app = router(state);

        let missing = app
            .clone()
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::FORBIDDEN);
        let json = response_json(missing).await;
        assert_eq!(json["error"], "metrics token is required");

        let valid = app
            .oneshot(get_request("/metrics", Some("test-admin-token")))
            .await
            .unwrap();
        assert_eq!(valid.status(), StatusCode::OK);
        let body = response_text(valid).await;
        assert!(body.contains("deopt_metrics_require_admin_token 1"));
    }

    #[tokio::test]
    async fn metrics_do_not_expose_secrets_or_high_cardinality_values() {
        let wallet = "0x00000000000000000000000000000000000000aa";
        let tx_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let private_key = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let mut state = admin_state(true);
        state.execution_config.rpc_url = Some("https://rpc.example/sensitive-provider-key".into());
        state.execution_config.executor_private_key =
            Some(PrivateKeySecret::new(private_key.to_string()));
        state.execution_config.executor_from_address = AccountId::new(wallet);
        state
            .fees_store
            .lock()
            .unwrap()
            .insert_fee_event(FeeEvent {
                fee_event_id: tx_hash.to_string(),
                source_type: FeeSourceType::PerpTrade,
                source_id: uuid.to_string(),
                market_type: FeeMarketType::Perp,
                flow_type: FeeFlowType::Orderbook,
                market_id: Some(1),
                option_series_id: None,
                maker: Some(AccountId::new(wallet)),
                taker: None,
                payer: AccountId::new(wallet),
                recipient: "treasury".to_string(),
                fee_asset: "USDC".to_string(),
                notional_1e8: 100,
                fee_rate_micro_bps: 1,
                fee_amount_1e8: 1,
                rebate_rate_micro_bps: 0,
                rebate_amount_1e8: 0,
                protocol_amount_1e8: 1,
                status: FeeStatus::Accrued,
                created_at_ms: 1,
            })
            .unwrap();

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains(
            "deopt_fee_events_total{market_type=\"perp\",flow_type=\"orderbook\",source_type=\"perp_trade\",status=\"accrued\"} 1"
        ));
        assert!(!body.contains("test-admin-token"));
        assert!(!body.contains("sensitive-provider-key"));
        assert!(!body.contains(private_key));
        assert!(!body.contains(wallet));
        assert!(!body.contains(tx_hash));
        assert!(!body.contains(uuid));
    }

    /// `/executor/health/v2` returns HTTP 200 with the canonical
    /// non-sensitive JSON envelope (status + flags + observability), even
    /// on a fully-disabled bare state.
    #[tokio::test]
    async fn executor_health_v2_returns_200_with_envelope() {
        let state = AppState::new(EngineState::with_default_markets());
        let response = router(state)
            .oneshot(get_request("/executor/health/v2", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["service"]["name"], "deopt-v2-backend");
        assert_eq!(json["overall_status"], "green");
        assert!(json["execution_flags"].is_object());
        assert!(json["signer"].is_object());
        assert!(json["policy_gate"].is_object());
        assert!(json["live_provider_config"].is_object());
        assert!(json["chain_state_last_seen"].is_object());
        assert!(json["economics_last_seen"].is_object());
        assert!(json["r5"].is_object());
        assert!(json["recent_policy_decisions"].is_object());
        assert!(json["recent_signer_events"].is_object());
        assert!(json["observability"].is_object());
        assert!(json["warnings"].is_array());
        assert!(json["hard_stops"].is_array());
        assert!(json["not_tracked_yet"].is_array());
        assert!(json["reasons"].is_array());
    }

    /// `/executor/health/v2` must never emit any secret. Even when the
    /// state happens to carry an RPC URL with an embedded token and an
    /// executor private key in memory (defence-in-depth: those should
    /// never be present on a hardened mainnet runtime), the JSON output
    /// must omit them.
    #[tokio::test]
    async fn executor_health_v2_does_not_expose_secrets() {
        let private_key = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let rpc_with_token = "https://rpc.example/sensitive-provider-key";
        let admin_token = "test-admin-token";
        let mut state = admin_state(true);
        state.execution_config.rpc_url = Some(rpc_with_token.to_string());
        state.execution_config.executor_private_key =
            Some(PrivateKeySecret::new(private_key.to_string()));
        state.execution_config.backend_signer_endpoint =
            Some("https://signer.example/secret-mtls-path".to_string());

        let response = router(state)
            .oneshot(get_request("/executor/health/v2", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(!body.contains(private_key));
        assert!(!body.contains("sensitive-provider-key"));
        assert!(!body.contains("secret-mtls-path"));
        assert!(!body.contains(admin_token));
        // structural assertions
        assert!(body.contains("\"overall_status\""));
        assert!(body.contains("\"hard_stops\""));
    }

    /// `/executor/health/v2` exposes the PFV / FM_V2 / CV `configured`
    /// booleans so frontend / operator consumers can detect a launch-blocker
    /// without scraping `/metrics`.
    #[tokio::test]
    async fn executor_health_v2_reports_pfv_fm_v2_cv_configured_booleans() {
        let mut state = AppState::new(EngineState::with_default_markets());
        state.option_event_indexer_config.protocol_fee_vault_address =
            Some(AccountId::new("0x00000000000000000000000000000000000000aa"));
        state.option_event_indexer_config.fees_manager_v2_address =
            Some(AccountId::new("0x00000000000000000000000000000000000000bb"));
        state.option_event_indexer_config.collateral_vault_address =
            AccountId::new("0x00000000000000000000000000000000000000cc");

        let response = router(state)
            .oneshot(get_request("/executor/health/v2", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(
            json["live_provider_config"]["protocol_fee_vault_configured"],
            true
        );
        assert_eq!(
            json["live_provider_config"]["fees_manager_v2_configured"],
            true
        );
        assert_eq!(
            json["live_provider_config"]["collateral_vault_configured"],
            true
        );
        assert_eq!(
            json["live_provider_config"]["protocol_fee_vault_address"].as_str(),
            Some("0x00000000000000000000000000000000000000aa")
        );
    }

    /// `/executor/health/v2` lists currently-unimplemented fields in
    /// `not_tracked_yet` so downstream consumers do not silently miss a
    /// gap. After BACKEND-OBSERVABILITY-LAST-SINGLETON-FIELDS shipped,
    /// `signer.last_signer_error_code` / `policy_gate.last_reject_source_type`
    /// / `policy_gate.econ_data_available_last` MUST no longer appear in
    /// the array — they are now persisted by the snapshot.
    #[tokio::test]
    async fn executor_health_v2_lists_not_tracked_yet_fields() {
        let state = AppState::new(EngineState::with_default_markets());
        let response = router(state)
            .oneshot(get_request("/executor/health/v2", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let not_tracked = json["not_tracked_yet"]
            .as_array()
            .expect("not_tracked_yet must be an array")
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>();
        for shipped in [
            "signer.last_signer_error_code",
            "policy_gate.last_reject_source_type",
            "policy_gate.econ_data_available_last",
            "policy_gate.last_policy_data_failure_type",
        ] {
            assert!(
                !not_tracked.iter().any(|f| f == shipped),
                "{shipped} should no longer be in not_tracked_yet"
            );
        }
        assert!(not_tracked
            .iter()
            .any(|f| f == "economics_last_seen.effective_maker_ppm"));
        assert!(not_tracked
            .iter()
            .any(|f| f == "execution_flags.be_balance_floor_wei"));
    }

    /// `/executor/health/v2` exposes `policy_gate.last_policy_data_failure_type`
    /// end-to-end via the HTTP envelope after
    /// BACKEND-OBSERVABILITY-LAST-POLICY-DATA-FAILURE-SINGLETON shipped.
    #[tokio::test]
    async fn executor_health_v2_surfaces_last_policy_data_failure_type() {
        let state = AppState::new(EngineState::with_default_markets());
        state.broadcast_observability.record_policy_data_failure(
            crate::options::broadcast_policy_data::read_type::PFV_REBATE_RESERVE,
        );
        let response = router(state)
            .oneshot(get_request("/executor/health/v2", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(
            json["policy_gate"]["last_policy_data_failure_type"],
            "pfv_rebate_reserve"
        );
    }

    /// `/executor/health/v2` surfaces the three newly-persisted
    /// singletons (`last_reject_source_type`, `last_signer_error_code`,
    /// `econ_data_available_last`) end-to-end via the HTTP envelope.
    #[tokio::test]
    async fn executor_health_v2_surfaces_singleton_observability_fields() {
        let state = AppState::new(EngineState::with_default_markets());
        // Drive each singleton.
        state.broadcast_observability.record_policy_rejected(
            "rebate-reserve",
            crate::options::types::OptionExecutionSourceType::OptionRfqFill,
        );
        state
            .broadcast_observability
            .record_signer_denied("kms-timeout", "remote");
        state
            .broadcast_observability
            .record_econ_data_available(true);

        let response = router(state)
            .oneshot(get_request("/executor/health/v2", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["policy_gate"]["last_reject_code"], "rebate-reserve");
        assert_eq!(json["policy_gate"]["last_reject_source_type"], "rfq");
        assert_eq!(json["policy_gate"]["econ_data_available_last"], true);
        assert_eq!(json["signer"]["last_signer_error_code"], "kms-timeout");
    }

    /// V2F-P: the `deopt_perp_fee_charged_v2_total{consumer=...}` metric
    /// is always emitted with the three low-cardinality bucket labels
    /// (`new`, `old`, `unknown`) — even when no PERP fee events have
    /// been indexed yet — so the
    /// `increase(...{consumer="old"}[5m]) > 0` Prometheus rule has a
    /// stable time series to alert on from the first scrape.
    #[tokio::test]
    async fn perp_fee_charged_v2_metric_emits_three_buckets_at_zero() {
        let state = admin_state(false);
        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body
            .contains("# HELP deopt_perp_fee_charged_v2_total PERP FeeChargedV2 events bucketed"));
        assert!(body.contains("# TYPE deopt_perp_fee_charged_v2_total gauge"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"unknown\"} 0"));
    }

    /// V2F-P: a NEW-emitted PERP `FeeChargedV2` lands in the `new`
    /// bucket. An OPTION-flavoured FeeChargedV2 must not contribute to
    /// this counter, and a `FeeRebatedV2` must not contribute either.
    #[tokio::test]
    async fn perp_fee_charged_v2_metric_classifies_new_and_excludes_option_and_rebate() {
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let old_perp_engine = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old_perp_engine));

        let tx_hash =
            "0x400acedf36381034ae37c983cc50e80d11a81587ca8065fbaef40293ff63a79a".to_string();
        let perp_new = build_fee_charged_v2_perp_log_row(
            1,
            tx_hash.as_str(),
            183,
            "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34",
            1,
            300,
            false,
        );
        let option_v2 = build_fee_charged_v2_log_row(
            5,
            "0x1100000000000000000000000000000000000000000000000000000000000000",
            7,
            "0xc0a76c2a00000000000000000000000000000000",
            25,
            250,
            false,
        );
        let rebate = build_fee_rebated_v2_log_row(
            8,
            "0x2200000000000000000000000000000000000000000000000000000000000000",
            9,
            "0xbaf0976a00000000000000000000000000000000",
            5,
            -50,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[perp_new, option_v2, rebate],
                42_188_599,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        // PERP NEW FeeChargedV2 is counted once.
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"new\"} 1"));
        // No PERP event was emitted by OLD; `old` stays at zero.
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"unknown\"} 0"));
        // Raw consumer/trader addresses must NOT appear anywhere in
        // the rendered metric output — labels must stay low-cardinality.
        assert!(!body.contains(new_perp_engine));
        assert!(!body.contains(old_perp_engine));
        assert!(!body.contains("0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34"));
        assert!(!body.contains("0xbaf0976a00000000000000000000000000000000"));
    }

    /// V2F-P: an OLD-emitted PERP `FeeChargedV2` lands in the `old`
    /// bucket. This is the scenario the
    /// `perp_fee_charged_from_old_engine` Prometheus alert must fire
    /// on.
    #[tokio::test]
    async fn perp_fee_charged_v2_metric_classifies_old_consumer() {
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let old_perp_engine = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old_perp_engine));

        let perp_old_event = build_fee_charged_v2_perp_log_row_for_consumer(
            7,
            "0x9999999999999999999999999999999999999999999999999999999999999999",
            42,
            old_perp_engine,
            "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34",
            1,
            300,
            false,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[perp_old_event],
                42_200_000,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"old\"} 1"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"unknown\"} 0"));
        // Raw OLD address must not leak into the rendered metric.
        assert!(!body.contains(old_perp_engine));
    }

    /// V2F-P: a PERP `FeeChargedV2` whose consumer matches neither the
    /// configured NEW nor OLD engine lands in the `unknown` bucket.
    #[tokio::test]
    async fn perp_fee_charged_v2_metric_classifies_unknown_consumer() {
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let old_perp_engine = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let stray_consumer = "0xdeadbeef00000000000000000000000000000001";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old_perp_engine));

        let perp_unknown = build_fee_charged_v2_perp_log_row_for_consumer(
            11,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            13,
            stray_consumer,
            "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34",
            1,
            300,
            false,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[perp_unknown],
                42_200_001,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"unknown\"} 1"));
        // Neither the configured nor the stray address leaks as a
        // label value.
        assert!(!body.contains(new_perp_engine));
        assert!(!body.contains(old_perp_engine));
        assert!(!body.contains(stray_consumer));
    }

    /// V2F-Q: the `deopt_perp_fee_rebated_v2_total{consumer=...}`
    /// metric is always emitted with the three low-cardinality bucket
    /// labels (`new`, `old`, `unknown`) — even on an empty backend —
    /// so the `PerpFeeRebatedFromOldEngine` Prometheus rule has a
    /// stable time series to alert on from the first scrape.
    #[tokio::test]
    async fn perp_fee_rebated_v2_metric_emits_three_buckets_at_zero() {
        let state = admin_state(false);
        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body
            .contains("# HELP deopt_perp_fee_rebated_v2_total PERP FeeRebatedV2 events bucketed"));
        assert!(body.contains("# TYPE deopt_perp_fee_rebated_v2_total gauge"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"unknown\"} 0"));
    }

    /// V2F-Q: a NEW-emitted PERP `FeeRebatedV2` lands in the `new`
    /// bucket; an OPTION-flavoured `FeeRebatedV2` and a PERP
    /// `FeeChargedV2` both stay excluded from the rebate counter.
    #[tokio::test]
    async fn perp_fee_rebated_v2_metric_classifies_new_and_excludes_option_and_charged() {
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let old_perp_engine = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old_perp_engine));

        let tx_hash =
            "0x9123000000000000000000000000000000000000000000000000000000000001".to_string();
        let perp_rebate_new = build_fee_rebated_v2_perp_log_row(
            1,
            tx_hash.as_str(),
            190,
            "0x475fe397fa56884952d350aa9ee1c3946964bc0c",
            5,
            -50,
        );
        let option_rebate = build_fee_rebated_v2_log_row(
            2,
            "0x9123000000000000000000000000000000000000000000000000000000000002",
            5,
            "0xbaf0976a00000000000000000000000000000000",
            5,
            -50,
        );
        let perp_charged = build_fee_charged_v2_perp_log_row(
            3,
            "0x9123000000000000000000000000000000000000000000000000000000000003",
            4,
            "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34",
            1,
            300,
            false,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[perp_rebate_new, option_rebate, perp_charged],
                42_188_700,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        // PERP rebate from NEW: counted once.
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"new\"} 1"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"unknown\"} 0"));
        // The PERP FeeChargedV2 must NOT inflate the rebate counter,
        // but it must still inflate the charged counter.
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"new\"} 1"));
        // Raw addresses must not appear anywhere in the rendered body.
        assert!(!body.contains(new_perp_engine));
        assert!(!body.contains(old_perp_engine));
        assert!(!body.contains("0x475fe397fa56884952d350aa9ee1c3946964bc0c"));
        assert!(!body.contains("0xbaf0976a00000000000000000000000000000000"));
        assert!(!body.contains("0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34"));
    }

    /// V2F-Q: a PERP `FeeRebatedV2` emitted by OLD lands in the `old`
    /// bucket — this is what `PerpFeeRebatedFromOldEngine` must fire on.
    #[tokio::test]
    async fn perp_fee_rebated_v2_metric_classifies_old_consumer() {
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let old_perp_engine = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old_perp_engine));

        let perp_rebate_old = build_fee_rebated_v2_perp_log_row_for_consumer(
            9,
            "0x9123000000000000000000000000000000000000000000000000000000000004",
            44,
            old_perp_engine,
            "0x475fe397fa56884952d350aa9ee1c3946964bc0c",
            5,
            -50,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[perp_rebate_old],
                42_200_000,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"old\"} 1"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"unknown\"} 0"));
        // Raw OLD address must not appear anywhere in the rendered metric.
        assert!(!body.contains(old_perp_engine));
    }

    /// V2F-Q: a PERP `FeeRebatedV2` whose consumer matches neither the
    /// configured NEW nor OLD engine lands in the `unknown` bucket.
    #[tokio::test]
    async fn perp_fee_rebated_v2_metric_classifies_unknown_consumer() {
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let old_perp_engine = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let stray_consumer = "0xdeadbeef00000000000000000000000000000002";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old_perp_engine));

        let perp_rebate_unknown = build_fee_rebated_v2_perp_log_row_for_consumer(
            13,
            "0x9123000000000000000000000000000000000000000000000000000000000005",
            17,
            stray_consumer,
            "0x475fe397fa56884952d350aa9ee1c3946964bc0c",
            5,
            -50,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[perp_rebate_unknown],
                42_200_001,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"unknown\"} 1"));
        assert!(!body.contains(new_perp_engine));
        assert!(!body.contains(old_perp_engine));
        assert!(!body.contains(stray_consumer));
    }

    /// V2G-F: the `deopt_option_fee_charged_v2_total{consumer=...}`
    /// metric is always emitted with the three low-cardinality bucket
    /// labels (`new`, `old`, `unknown`) — even when no OPTION fee
    /// events have been indexed yet — so the
    /// `increase(...{consumer="old"}[5m]) > 0` Prometheus rule has a
    /// stable time series to alert on from the first scrape.
    #[tokio::test]
    async fn option_fee_charged_v2_metric_emits_three_buckets_at_zero() {
        let state = admin_state(false);
        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains(
            "# HELP deopt_option_fee_charged_v2_total OPTION FeeChargedV2 events bucketed"
        ));
        assert!(body.contains("# TYPE deopt_option_fee_charged_v2_total gauge"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"unknown\"} 0"));
        // Sibling rebated metric is also pre-seeded at zero.
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"unknown\"} 0"));
    }

    /// V2G-F: a NEW-emitted OPTION `FeeChargedV2` lands in the `new`
    /// bucket. A PERP-flavoured `FeeChargedV2` must not contribute to
    /// the OPTION counter, and a `FeeRebatedV2` must not either.
    #[tokio::test]
    async fn option_fee_charged_v2_metric_classifies_new_and_excludes_perp_and_rebate() {
        let new_margin_engine = "0x287cef479be5889eefca847f9e73c860898f48cc";
        let old_margin_engine = "0x6c5665de05e7314cb63cd77f82dfa86508a5b5f8";
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let mut state = admin_state(false);
        state.option_event_indexer_config.margin_engine_address = AccountId::new(new_margin_engine);
        state.option_event_indexer_config.old_margin_engine_address =
            Some(AccountId::new(old_margin_engine));
        // PERP NEW so the cross-PERP charged event falls into `new`
        // (and we can assert PERP/OPTION separation in both directions).
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);

        let option_new = build_fee_charged_v2_option_log_row(
            21,
            "0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149",
            125,
            "0x77ca9dd6ccce2d692fb23877a2db7178807b0020",
            25,
            125,
            false,
        );
        // PERP `FeeChargedV2` MUST NOT contribute to the OPTION counter.
        let perp_charged = build_fee_charged_v2_perp_log_row(
            22,
            "0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394",
            153,
            "0x77ca9dd6ccce2d692fb23877a2db7178807b0020",
            6,
            200,
            false,
        );
        // OPTION `FeeRebatedV2` MUST NOT contribute to the OPTION charged counter.
        let option_rebate = build_fee_rebated_v2_option_log_row(
            23,
            "0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149",
            133,
            "0x290bd12c93e467bf51c51f5273d35bddb19e9274",
            10,
            -50,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[option_new, perp_charged, option_rebate],
                42_206_003,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        // OPTION NEW FeeChargedV2 counted once; PERP and rebate excluded.
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"new\"} 1"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"unknown\"} 0"));
        // PERP metric still sees its own NEW event (no cross-contamination).
        assert!(body.contains("deopt_perp_fee_charged_v2_total{consumer=\"new\"} 1"));
        // OPTION rebated metric sees the rebate.
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"new\"} 1"));
        // No raw address (consumer or trader) leaks as a label value.
        assert!(!body.contains(new_margin_engine));
        assert!(!body.contains(old_margin_engine));
        assert!(!body.contains("0x77ca9dd6ccce2d692fb23877a2db7178807b0020"));
        assert!(!body.contains("0x290bd12c93e467bf51c51f5273d35bddb19e9274"));
    }

    /// V2G-F: an OLD-emitted OPTION `FeeChargedV2` lands in the `old`
    /// bucket when `OLD_MARGIN_ENGINE_ADDRESS` is configured.
    #[tokio::test]
    async fn option_fee_charged_v2_metric_classifies_old_consumer() {
        let new_margin_engine = "0x287cef479be5889eefca847f9e73c860898f48cc";
        let old_margin_engine = "0x6c5665de05e7314cb63cd77f82dfa86508a5b5f8";
        let mut state = admin_state(false);
        state.option_event_indexer_config.margin_engine_address = AccountId::new(new_margin_engine);
        state.option_event_indexer_config.old_margin_engine_address =
            Some(AccountId::new(old_margin_engine));

        let option_old = build_fee_charged_v2_option_log_row_for_consumer(
            31,
            "0x8888888888888888888888888888888888888888888888888888888888888888",
            44,
            old_margin_engine,
            "0x77ca9dd6ccce2d692fb23877a2db7178807b0020",
            25,
            125,
            false,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[option_old],
                42_300_000,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"old\"} 1"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"unknown\"} 0"));
        assert!(!body.contains(old_margin_engine));
    }

    /// V2G-F: an OPTION `FeeChargedV2` from a consumer that matches
    /// neither NEW nor OLD lands in `unknown`. Also covers the case
    /// where `OLD_MARGIN_ENGINE_ADDRESS` is unset.
    #[tokio::test]
    async fn option_fee_charged_v2_metric_classifies_unknown_consumer() {
        let new_margin_engine = "0x287cef479be5889eefca847f9e73c860898f48cc";
        let stray_consumer = "0xdeadbeef00000000000000000000000000000003";
        let mut state = admin_state(false);
        state.option_event_indexer_config.margin_engine_address = AccountId::new(new_margin_engine);
        // OLD intentionally unset — non-NEW consumers must fall through
        // to `unknown` (never silently become `old`).
        state.option_event_indexer_config.old_margin_engine_address = None;

        let option_unknown = build_fee_charged_v2_option_log_row_for_consumer(
            33,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab",
            15,
            stray_consumer,
            "0x77ca9dd6ccce2d692fb23877a2db7178807b0020",
            25,
            125,
            false,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[option_unknown],
                42_300_001,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_option_fee_charged_v2_total{consumer=\"unknown\"} 1"));
        assert!(!body.contains(new_margin_engine));
        assert!(!body.contains(stray_consumer));
    }

    /// V2G-F: OPTION `FeeRebatedV2` from NEW lands in `new`; a
    /// PERP-flavoured `FeeRebatedV2` must not contribute to the OPTION
    /// rebated counter (PERP/OPTION separation in the other direction).
    #[tokio::test]
    async fn option_fee_rebated_v2_metric_classifies_new_and_excludes_perp() {
        let new_margin_engine = "0x287cef479be5889eefca847f9e73c860898f48cc";
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let mut state = admin_state(false);
        state.option_event_indexer_config.margin_engine_address = AccountId::new(new_margin_engine);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);

        let option_rebate_new = build_fee_rebated_v2_option_log_row(
            41,
            "0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149",
            133,
            "0x290bd12c93e467bf51c51f5273d35bddb19e9274",
            10,
            -50,
        );
        let perp_rebate_new = build_fee_rebated_v2_perp_log_row(
            42,
            "0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394",
            147,
            "0x290bd12c93e467bf51c51f5273d35bddb19e9274",
            3,
            -100,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[option_rebate_new, perp_rebate_new],
                42_300_002,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        // OPTION rebate: NEW = 1, PERP rebate excluded.
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"new\"} 1"));
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"unknown\"} 0"));
        // PERP rebate metric still sees its own event.
        assert!(body.contains("deopt_perp_fee_rebated_v2_total{consumer=\"new\"} 1"));
    }

    /// V2G-F: the derived
    /// `deopt_fees_manager_v2_rebate_budget_native{asset=...}` gauge
    /// reflects `SUM(RebateBudgetFunded.amount) −
    /// SUM(RebateBudgetSpent.amount) − SUM(RebateBudgetWithdrawn.amount)`
    /// per settlement asset, computed from indexed events. With no
    /// events the gauge emits nothing; once events land the gauge
    /// exposes one series per (lowercased) asset address.
    #[tokio::test]
    async fn fees_manager_v2_rebate_budget_metric_reflects_funded_minus_spent_and_withdrawn() {
        use super::tests::OPTION_EVENT_INDEXER_STATE_ID;
        let m_usdc = "0x6eae407f5640b006fac9965182e238582a3b412e";
        let other_asset = "0x0000000000000000000000000000000000001234";
        let state = admin_state(false);

        // Initial scrape — no RebateBudget events yet → no asset
        // series should be emitted (we never emit a fake zero baseline
        // for an unknown asset).
        let response = router(state.clone())
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("# HELP deopt_fees_manager_v2_rebate_budget_native"));
        assert!(body.contains("# TYPE deopt_fees_manager_v2_rebate_budget_native gauge"));
        assert!(!body.contains("deopt_fees_manager_v2_rebate_budget_native{asset=\""));

        let funded = build_rebate_budget_event(
            61,
            "0xc11c0000000000000000000000000000000000000000000000000000000000c1",
            1,
            "RebateBudgetFunded",
            m_usdc,
            1_000_000,
        );
        let spent_perp = build_rebate_budget_event(
            62,
            "0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394",
            148,
            "RebateBudgetSpent",
            m_usdc,
            3,
        );
        let spent_option = build_rebate_budget_event(
            63,
            "0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149",
            134,
            "RebateBudgetSpent",
            m_usdc,
            10,
        );
        // A separate (smaller) asset to verify the gauge produces one
        // series per (lowercased) asset address — no cross-leak.
        let funded_other = build_rebate_budget_event(
            64,
            "0xc11c0000000000000000000000000000000000000000000000000000000000c2",
            2,
            "RebateBudgetFunded",
            other_asset,
            42,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[funded, spent_perp, spent_option, funded_other],
                42_206_010,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        // mUSDC: 1_000_000 funded − 3 PERP spent − 10 OPTION spent = 999_987.
        assert!(body.contains(
            "deopt_fees_manager_v2_rebate_budget_native{asset=\"0x6eae407f5640b006fac9965182e238582a3b412e\"} 999987"
        ));
        // Other asset only had a funded event → 42.
        assert!(body.contains(
            "deopt_fees_manager_v2_rebate_budget_native{asset=\"0x0000000000000000000000000000000000001234\"} 42"
        ));
    }

    /// V2G-G: `/admin/fees/v2/observability` returns a JSON snapshot
    /// of the V2 fee surface. It must:
    ///   - require the admin token when one is configured;
    ///   - pre-seed every consumer bucket at zero on an empty backend;
    ///   - reuse the same classifier as `/metrics` so a NEW PERP event
    ///     lands in `new` and an OLD PERP event lands in `old`;
    ///   - mirror the rebate budget gauge under the same lowercased
    ///     asset key;
    ///   - never leak raw NEW / OLD / trader / tx-hash strings into the
    ///     bucket labels or top-level `notes`.
    #[tokio::test]
    async fn admin_v2_observability_snapshot_emits_three_buckets_at_zero() {
        let response = router(admin_state(false))
            .oneshot(get_request("/admin/fees/v2/observability", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["milestone"], "V2G-G");
        for metric in [
            "perp_fee_charged_v2_by_consumer",
            "perp_fee_rebated_v2_by_consumer",
            "option_fee_charged_v2_by_consumer",
            "option_fee_rebated_v2_by_consumer",
        ] {
            let buckets = &json["metrics"][metric];
            assert_eq!(buckets["new"], 0, "{metric}.new should default to 0");
            assert_eq!(buckets["old"], 0, "{metric}.old should default to 0");
            assert_eq!(
                buckets["unknown"], 0,
                "{metric}.unknown should default to 0"
            );
        }
        assert_eq!(json["anomaly_totals"]["old_consumer_events"], 0);
        assert_eq!(json["anomaly_totals"]["unknown_consumer_events"], 0);
        // No events yet → no per-asset rebate budget series.
        assert!(json["metrics"]["fees_manager_v2_rebate_budget_native"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn admin_v2_observability_requires_admin_token_when_configured() {
        let response = router(admin_state(true))
            .oneshot(get_request("/admin/fees/v2/observability", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = router(admin_state(true))
            .oneshot(get_request(
                "/admin/fees/v2/observability",
                Some("test-admin-token"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_v2_observability_classifies_new_old_and_unknown_buckets() {
        let new_perp_engine = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let old_perp_engine = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let new_margin_engine = "0x287cef479be5889eefca847f9e73c860898f48cc";
        let fees_manager_v2 = "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f";
        let stray_consumer = "0xdeadbeef00000000000000000000000000000099";
        let trader = "0x290bd12c93e467bf51c51f5273d35bddb19e9274";
        let m_usdc = "0x6eae407f5640b006fac9965182e238582a3b412e";

        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(new_perp_engine);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old_perp_engine));
        state.option_event_indexer_config.margin_engine_address = AccountId::new(new_margin_engine);
        state.option_event_indexer_config.fees_manager_v2_address =
            Some(AccountId::new(fees_manager_v2));

        let perp_new = build_fee_charged_v2_perp_log_row(
            201,
            "0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394",
            153,
            trader,
            6,
            200,
            false,
        );
        let perp_old = build_fee_charged_v2_perp_log_row_for_consumer(
            202,
            "0x4444444444444444444444444444444444444444444444444444444444444444",
            42,
            old_perp_engine,
            trader,
            1,
            300,
            false,
        );
        let option_new = build_fee_charged_v2_option_log_row(
            203,
            "0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149",
            125,
            trader,
            25,
            125,
            false,
        );
        let option_rebate_new = build_fee_rebated_v2_option_log_row(
            204,
            "0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149",
            133,
            trader,
            10,
            -50,
        );
        let option_unknown = build_fee_charged_v2_option_log_row_for_consumer(
            205,
            "0x5555555555555555555555555555555555555555555555555555555555555555",
            7,
            stray_consumer,
            trader,
            25,
            125,
            false,
        );
        let funded = build_rebate_budget_event(
            206,
            "0xc11c0000000000000000000000000000000000000000000000000000000000c1",
            1,
            "RebateBudgetFunded",
            m_usdc,
            1_000_000,
        );
        let spent = build_rebate_budget_event(
            207,
            "0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149",
            134,
            "RebateBudgetSpent",
            m_usdc,
            13,
        );

        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[
                    perp_new,
                    perp_old,
                    option_new,
                    option_rebate_new,
                    option_unknown,
                    funded,
                    spent,
                ],
                42_300_010,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/admin/fees/v2/observability", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;

        let perp_charged = &json["metrics"]["perp_fee_charged_v2_by_consumer"];
        assert_eq!(perp_charged["new"], 1);
        assert_eq!(perp_charged["old"], 1);
        assert_eq!(perp_charged["unknown"], 0);

        let option_charged = &json["metrics"]["option_fee_charged_v2_by_consumer"];
        assert_eq!(option_charged["new"], 1);
        assert_eq!(option_charged["old"], 0);
        assert_eq!(option_charged["unknown"], 1);

        let option_rebated = &json["metrics"]["option_fee_rebated_v2_by_consumer"];
        assert_eq!(option_rebated["new"], 1);
        assert_eq!(option_rebated["old"], 0);
        assert_eq!(option_rebated["unknown"], 0);

        // Budget gauge mirrors the metric pipeline: 1_000_000 − 13 = 999_987.
        assert_eq!(
            json["metrics"]["fees_manager_v2_rebate_budget_native"][m_usdc],
            999_987
        );

        // Anomaly totals roll up across all four consumer-bucket metrics.
        assert_eq!(json["anomaly_totals"]["old_consumer_events"], 1);
        assert_eq!(json["anomaly_totals"]["unknown_consumer_events"], 1);

        // Configured contract addresses surface as opaque strings so the
        // operator can confirm the classifier is using the right NEW /
        // OLD engines.
        assert_eq!(json["contracts"]["perp_engine_new"], new_perp_engine);
        assert_eq!(json["contracts"]["perp_engine_old"], old_perp_engine);
        assert_eq!(json["contracts"]["margin_engine_new"], new_margin_engine);
        assert!(json["contracts"]["margin_engine_old"].is_null());
        assert_eq!(json["contracts"]["fees_manager_v2"], fees_manager_v2);

        // Cardinality contract: raw event-level addresses (trader, stray
        // consumer, tx hash) must NOT appear in the JSON bucket keys
        // or counts. The classifier only emits one of three labels.
        let metrics_obj = json["metrics"].as_object().unwrap();
        for (_, buckets) in metrics_obj
            .iter()
            .filter(|(k, _)| k.as_str() != "fees_manager_v2_rebate_budget_native")
        {
            let map = buckets.as_object().unwrap();
            assert_eq!(map.len(), 3, "metric {buckets:?} must have 3 buckets");
            for bucket in ["new", "old", "unknown"] {
                assert!(map.contains_key(bucket));
            }
        }
        let serialized = serde_json::to_string(&json["metrics"]).unwrap();
        // The 'metrics' subtree must not leak any of these raw strings.
        for forbidden in [trader, stray_consumer] {
            assert!(
                !serialized.contains(forbidden),
                "metrics leaked raw value {forbidden}: {serialized}"
            );
        }
    }

    /// V2G-M: `/admin/fees/v2/smoke/readiness` returns the dry-run
    /// packet skeleton with the V2G-D2 EOAs hard-pinned, the
    /// broadcast-gate snapshot, and `safe_to_broadcast_today=false`
    /// during the soak window. Never embeds a private-key-shaped
    /// string.
    // V2G-M: holding the std::sync::Mutex guard across .await is
    // intentional — the guard serialises process-env mutation against
    // the sibling smoke-readiness tests that share the same env-var
    // names. Test-only.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn admin_v2_smoke_readiness_returns_packet_with_default_eoas() {
        let perp_new = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let perp_old = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let margin_new = "0x287cef479be5889eefca847f9e73c860898f48cc";
        let fmv2 = "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f";

        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(perp_new);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(perp_old));
        state.option_event_indexer_config.margin_engine_address = AccountId::new(margin_new);
        state.option_event_indexer_config.fees_manager_v2_address = Some(AccountId::new(fmv2));

        // V2G-M: serialise env-var manipulation with the other smoke-
        // readiness tests so cargo's parallel runner doesn't race.
        let _guard = crate::fees::smoke_readiness::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::env::remove_var(crate::fees::smoke_readiness::MAKER_KEY_ENV);
        std::env::remove_var(crate::fees::smoke_readiness::TAKER_KEY_ENV);

        let response = router(state)
            .oneshot(get_request("/admin/fees/v2/smoke/readiness", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["milestone"], "V2G-M");

        // V2G-D2 EOAs surface as addresses-only, lowercased.
        let eoas = &json["smoke_eoas"];
        assert_eq!(
            eoas["tier4_maker_address"],
            "0x290bd12c93e467bf51c51f5273d35bddb19e9274"
        );
        assert_eq!(
            eoas["tier2_taker_address"],
            "0x77ca9dd6ccce2d692fb23877a2db7178807b0020"
        );
        // Env-var NAMES are surfaced; values never read.
        assert_eq!(
            eoas["key_env_vars"]["maker"],
            "PERP_SMOKE_BUYER_PRIVATE_KEY"
        );
        assert_eq!(
            eoas["key_env_vars"]["taker"],
            "PERP_SMOKE_SELLER_PRIVATE_KEY"
        );

        // Dry-run packet skeletons for both products.
        let perp_packet = &json["dry_run_packets"]["perp"];
        assert_eq!(perp_packet["product"], "PERP");
        assert_eq!(perp_packet["fee_consumer_address"], perp_new);
        assert!(perp_packet["basis_amount_native"].is_null());
        assert!(perp_packet["expected_fee_amount_native"].is_null());

        let option_packet = &json["dry_run_packets"]["option"];
        assert_eq!(option_packet["product"], "OPTION");
        assert_eq!(option_packet["fee_consumer_address"], margin_new);

        // Broadcast gates: every safety toggle is false / dry-run by
        // default; no maker/taker key visible.
        let gates = &json["broadcast_gates"];
        assert_eq!(gates["execution_enabled"], false);
        assert_eq!(gates["executor_dry_run"], true);
        assert_eq!(gates["executor_real_broadcast_enabled"], false);
        assert_eq!(gates["option_execution_broadcast_enabled"], false);
        assert_eq!(gates["maker_key_env_set"], false);
        assert_eq!(gates["taker_key_env_set"], false);

        // No private-key-shaped strings in the response body.
        let body = serde_json::to_string(&json).unwrap();
        assert!(!body.contains("4c0883"));
        assert!(!body.contains("private_key\":\"0x"));
        assert!(!body.contains("PRIVATE_KEY=0x"));
    }

    #[tokio::test]
    async fn admin_v2_smoke_readiness_requires_admin_token_when_configured() {
        let response = router(admin_state(true))
            .oneshot(get_request("/admin/fees/v2/smoke/readiness", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = router(admin_state(true))
            .oneshot(get_request(
                "/admin/fees/v2/smoke/readiness",
                Some("test-admin-token"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// V2G-M hard-rule pin: if the active PERP address equals the OLD
    /// stranded one (operator misconfiguration), the readiness packet
    /// refuses to mark safe. This is the "OLD_PERP_ENGINE never
    /// active" invariant surfaced at the readiness layer.
    #[tokio::test]
    async fn admin_v2_smoke_readiness_refuses_when_active_equals_old() {
        let old = "0xb36395b67d0798ada981731c9fa5239f4362b53b";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(old);
        state.execution_config.old_perp_engine_address = Some(AccountId::new(old));

        let response = router(state)
            .oneshot(get_request("/admin/fees/v2/smoke/readiness", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["active_perp_is_old_engine"], true);
        assert_eq!(json["soak_safe_for_local_compose"], false);
    }

    /// V2G-M: confirm no key-shaped value leaks into the response even
    /// when the operator's env carries one. The endpoint reads only
    /// the boolean presence of the env vars.
    #[allow(clippy::await_holding_lock)] // same rationale as the sibling test
    #[tokio::test]
    async fn admin_v2_smoke_readiness_never_leaks_env_key_value() {
        use crate::fees::smoke_readiness::{MAKER_KEY_ENV, TAKER_KEY_ENV};

        let perp_new = "0xc6c592100723fe0c66343a16e95ec34cc0c2141c";
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address = AccountId::new(perp_new);
        state.execution_config.old_perp_engine_address =
            Some(AccountId::new("0xb36395b67d0798ada981731c9fa5239f4362b53b"));

        // V2G-M: serialise env-var manipulation with the other smoke-
        // readiness tests; cargo's default parallel runner shares the
        // process env across threads.
        let _guard = crate::fees::smoke_readiness::TEST_ENV_GUARD
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        // Set the env var values to a recognisable secret-shaped token.
        // The endpoint must NOT echo them anywhere.
        let secret = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        std::env::set_var(MAKER_KEY_ENV, secret);
        std::env::set_var(TAKER_KEY_ENV, secret);

        let response = router(state)
            .oneshot(get_request("/admin/fees/v2/smoke/readiness", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let body = serde_json::to_string(&json).unwrap();

        // Boolean presence is reported.
        assert_eq!(json["broadcast_gates"]["maker_key_env_set"], true);
        assert_eq!(json["broadcast_gates"]["taker_key_env_set"], true);
        // The raw key value MUST NOT appear anywhere in the body.
        assert!(!body.contains(secret), "secret leaked into response");
        assert!(!body.contains("deadbeef"));

        std::env::remove_var(MAKER_KEY_ENV);
        std::env::remove_var(TAKER_KEY_ENV);
    }

    #[tokio::test]
    async fn admin_v2_observability_omits_zero_address_contracts() {
        let mut state = admin_state(false);
        state.execution_config.perp_engine_address =
            AccountId::new("0x0000000000000000000000000000000000000000");
        state.execution_config.old_perp_engine_address = None;
        state.option_event_indexer_config.margin_engine_address =
            AccountId::new("0x0000000000000000000000000000000000000000");
        state.option_event_indexer_config.fees_manager_v2_address = None;

        let response = router(state)
            .oneshot(get_request("/admin/fees/v2/observability", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        for key in [
            "perp_engine_new",
            "perp_engine_old",
            "margin_engine_new",
            "margin_engine_old",
            "fees_manager_v2",
        ] {
            assert!(
                json["contracts"][key].is_null(),
                "{key} should be null when unset / zero, got {:?}",
                json["contracts"][key]
            );
        }
    }

    /// V2G-F: OPTION `FeeRebatedV2` from a stray consumer lands in
    /// `unknown` and never silently becomes `old`.
    #[tokio::test]
    async fn option_fee_rebated_v2_metric_classifies_unknown_consumer() {
        let new_margin_engine = "0x287cef479be5889eefca847f9e73c860898f48cc";
        let stray_consumer = "0xdeadbeef00000000000000000000000000000004";
        let mut state = admin_state(false);
        state.option_event_indexer_config.margin_engine_address = AccountId::new(new_margin_engine);
        state.option_event_indexer_config.old_margin_engine_address = None;

        let option_rebate_unknown = build_fee_rebated_v2_option_log_row_for_consumer(
            44,
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            19,
            stray_consumer,
            "0x290bd12c93e467bf51c51f5273d35bddb19e9274",
            10,
            -50,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[option_rebate_unknown],
                42_300_003,
                1,
            );

        let response = router(state)
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"new\"} 0"));
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"old\"} 0"));
        assert!(body.contains("deopt_option_fee_rebated_v2_total{consumer=\"unknown\"} 1"));
        assert!(!body.contains(new_margin_engine));
        assert!(!body.contains(stray_consumer));
    }

    #[tokio::test]
    async fn metrics_persistence_disabled_empty_state_renders() {
        let response = router(AppState::new(EngineState::with_default_markets()))
            .oneshot(get_request("/metrics", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        for metric in [
            "deopt_db_up 0",
            "# HELP deopt_execution_intents_total",
            "# HELP deopt_rfqs_total",
            "# HELP deopt_option_series_total",
            "# HELP deopt_fee_events_total",
            "deopt_mm_sessions_total 0",
        ] {
            assert!(body.contains(metric), "missing metric {metric}");
        }
    }

    #[tokio::test]
    async fn ready_succeeds_with_persistence_disabled() {
        let response = router(AppState::new(EngineState::with_default_markets()))
            .oneshot(get_request("/ready", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["ready"], true);
        assert_eq!(json["checks"][2]["name"], "database");
        assert_eq!(json["checks"][2]["status"], "persistence_disabled");
    }

    #[tokio::test]
    async fn ready_reports_not_ready_when_persistence_enabled_without_repository() {
        let mut state = AppState::new(EngineState::with_default_markets());
        state.persistence_enabled = true;
        state.database_configured = true;
        state.repository = None;

        let response = router(state)
            .oneshot(get_request("/ready", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json = response_json(response).await;
        assert_eq!(json["ready"], false);
        assert_eq!(json["checks"][2]["name"], "database");
        assert_eq!(json["checks"][2]["status"], "repository_unavailable");
    }

    #[tokio::test]
    async fn admin_enabled_status_works() {
        let response = router(admin_state(false))
            .oneshot(get_request("/admin/status", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["service"], "deopt-v2-backend");
        assert_eq!(json["ok"], true);
        assert_eq!(json["network"], "admin-test");
        assert_eq!(json["real_broadcast_enabled"], false);
    }

    #[tokio::test]
    async fn admin_token_required_rejects_missing_token() {
        let response = router(admin_state(true))
            .oneshot(get_request("/admin/status", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["error"], "admin token is required");
    }

    #[tokio::test]
    async fn admin_token_required_accepts_valid_token() {
        let response = router(admin_state(true))
            .oneshot(get_request("/admin/status", Some("test-admin-token")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    // ============================================================
    //   V2G-W2 — middleware route-gate integration tests
    // ============================================================

    // The SharedToken auth mode (today's production posture) returns
    // an `Operator` identity, so every currently-routed admin
    // endpoint passes the role gate. These tests pin behavioural
    // parity for the new middleware on top of the existing
    // ensure_admin_access handler check.

    #[tokio::test]
    async fn v2gw2_middleware_lets_valid_token_reach_viewer_route() {
        let response = router(admin_state(true))
            .oneshot(get_request("/admin/status", Some("test-admin-token")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn v2gw2_middleware_lets_valid_token_reach_operator_route() {
        // /admin/fees/v2/smoke/readiness is the only operator-class
        // GET route today. SharedToken => Operator identity => allow.
        let response = router(admin_state(true))
            .oneshot(get_request(
                "/admin/fees/v2/smoke/readiness",
                Some("test-admin-token"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn v2gw2_middleware_blocks_missing_token_on_viewer_route() {
        let response = router(admin_state(true))
            .oneshot(get_request("/admin/status", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["error"], "admin token is required");
    }

    #[tokio::test]
    async fn v2gw2_middleware_blocks_wrong_token_on_viewer_route() {
        let response = router(admin_state(true))
            .oneshot(get_request("/admin/status", Some("not-the-token")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["error"], "invalid admin token");
    }

    #[tokio::test]
    async fn v2gw2_middleware_blocks_operator_route_under_jwt_fail_closed() {
        // AuthMode::Jwt is intentionally NOT implemented in V2G-W1.
        // The middleware must fail-closed: even a "Bearer …" header
        // never authenticates while the verifier is a stub.
        let mut state = admin_state(true);
        state
            .admin_config
            .set_auth_mode(crate::admin::AuthMode::Jwt);
        let response = router(state)
            .oneshot(get_request(
                "/admin/fees/v2/smoke/readiness",
                Some("test-admin-token"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["error"], "admin JWT auth mode is not implemented");
    }

    #[tokio::test]
    async fn v2gw2_middleware_disabled_mode_lets_request_through_without_token() {
        // AuthMode::Disabled is the explicit local-dev escape hatch.
        // Middleware grants Breakglass identity → every role passes.
        // NOTE: validate_startup blocks Disabled + require_token=true
        // for boot-time misconfig; this test sets the mode at runtime
        // to verify the *middleware* behaves as documented.
        let mut state = admin_state(false); // require_token=false to match disabled posture
        state
            .admin_config
            .set_auth_mode(crate::admin::AuthMode::Disabled);
        let response = router(state)
            .oneshot(get_request("/admin/status", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn v2gw2_middleware_passes_through_non_admin_paths() {
        // `/health` must remain reachable without any admin
        // authentication, even when require_token=true. The
        // middleware short-circuits on non-/admin/* paths.
        let response = router(admin_state(true))
            .oneshot(get_request("/health", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn v2gw2_middleware_403_body_never_contains_token_material() {
        let mut state = admin_state(true);
        // Set a recognisable token-looking string so the negative
        // assertions are meaningful.
        state.admin_config = crate::admin::AdminConfig::new(
            true,
            true,
            Some("very-secret-admin-token-canary".to_string()),
        );
        let response = router(state)
            .oneshot(get_request(
                "/admin/status",
                Some("attacker-supplied-token-canary"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !body_text.contains("very-secret-admin-token-canary"),
            "403 body leaks configured token: {body_text}"
        );
        assert!(
            !body_text.contains("attacker-supplied-token-canary"),
            "403 body leaks candidate token: {body_text}"
        );
        assert!(
            !body_text.contains("eyJ"),
            "403 body contains base64-flavoured JWT material: {body_text}"
        );
    }

    #[tokio::test]
    async fn admin_config_redacts_secrets() {
        let mut state = admin_state(true);
        state.execution_config.rpc_url = Some("https://rpc.example/sensitive-provider-key".into());
        let response = router(state)
            .oneshot(get_request("/admin/config", Some("test-admin-token")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_text = String::from_utf8(body.to_vec()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["configured"]["rpc"], true);
        assert_eq!(json["admin"]["token_configured"], true);
        assert_eq!(
            json["options"]["reconciliation_worker"]["state_checks_enabled"],
            false
        );
        assert_eq!(
            json["options"]["reconciliation_worker"]["state_checks_require_rpc"],
            true
        );
        assert_eq!(
            json["options"]["reconciliation_worker"]["state_checks_strict"],
            false
        );
        assert!(!body_text.contains("sensitive-provider-key"));
        assert!(!body_text.contains("test-admin-token"));
        assert!(json.get("database_url").is_none());
        assert!(json.get("rpc_url").is_none());
        assert!(json.get("admin_token").is_none());
    }

    #[tokio::test]
    async fn admin_db_handles_persistence_disabled() {
        let response = router(admin_state(false))
            .oneshot(get_request("/admin/db", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["persistence_enabled"], false);
        assert_eq!(json["connected"], false);
        assert_eq!(json["counts"].as_object().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn admin_mm_sessions_empty_when_gateway_disabled() {
        let response = router(admin_state(false))
            .oneshot(get_request("/admin/mm/sessions", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["enabled"], false);
        assert_eq!(json["sessions"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn admin_mm_sessions_return_sanitized_snapshot() {
        let mut state = admin_state(false);
        state.mm_gateway_config = MmGatewayConfig {
            enabled: true,
            auth_mode: AuthMode::WalletChallenge,
            ..MmGatewayConfig::default()
        };
        let mut session = MmSession::with_ids(
            "session-admin-1",
            "connection-admin-1",
            123,
            AuthMode::WalletChallenge,
            true,
        );
        session.set_challenge(
            AccountId::new("0x0000000000000000000000000000000000000001"),
            "secret-nonce-not-exposed".to_string(),
            124,
            60_124,
        );
        session.register_open_client_order_id("order-a");
        session.register_open_client_order_id("order-b");
        let (sender, _receiver) = mpsc::unbounded_channel::<ServerMessage>();
        state.mm_sessions.register(&session, sender).unwrap();

        let response = router(state)
            .oneshot(get_request("/admin/mm/sessions", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["enabled"], true);
        assert_eq!(json["sessions"][0]["session_id"], "session-admin-1");
        assert_eq!(json["sessions"][0]["authenticated"], false);
        assert_eq!(json["sessions"][0]["auth_mode"], "wallet_challenge");
        assert_eq!(json["sessions"][0]["challenge_active"], true);
        assert_eq!(json["sessions"][0]["challenge_expires_at_ms"], 60_124);
        assert_eq!(json["sessions"][0]["open_client_order_ids_count"], 2);
        assert!(json["sessions"][0].get("open_client_order_ids").is_none());
        assert!(json["sessions"][0].get("connection_id").is_none());
        assert!(json["sessions"][0].get("challenge").is_none());
        assert!(json["sessions"][0].get("challenge_nonce").is_none());
    }

    #[tokio::test]
    async fn admin_mm_permissions_returns_sanitized_accounts() {
        let mut state = admin_state(false);
        state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
        state
            .mm_permissions
            .lock()
            .unwrap()
            .upsert_account(MmAccountPermissions {
                mm_account: AccountId::new("0x0000000000000000000000000000000000000001"),
                enabled: true,
                label: Some("MM Alpha".to_string()),
                can_submit_perp_orders: true,
                can_quote_perp_rfq: true,
                can_quote_option_rfq: true,
                can_submit_option_orders: false,
                created_at_ms: 1,
                updated_at_ms: 2,
            });
        state
            .mm_permissions
            .lock()
            .unwrap()
            .insert_product_permission(MmProductPermission {
                id: "scope-1".to_string(),
                mm_account: AccountId::new("0x0000000000000000000000000000000000000001"),
                market_id: Some(1),
                option_series_id: None,
                enabled: true,
                created_at_ms: 1,
                updated_at_ms: 2,
            });

        let response = router(state)
            .oneshot(get_request("/admin/mm/permissions", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["enabled"], true);
        assert_eq!(json["accounts"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["accounts"][0]["mm_account"],
            "0x0000000000000000000000000000000000000001"
        );
        assert_eq!(json["accounts"][0]["label"], "MM Alpha");
        assert_eq!(json["accounts"][0]["can_submit_perp_orders"], true);
        assert_eq!(json["accounts"][0]["can_quote_perp_rfq"], true);
        assert_eq!(json["accounts"][0]["can_quote_option_rfq"], true);
        assert_eq!(json["accounts"][0]["can_submit_option_orders"], false);
        assert_eq!(json["accounts"][0]["market_permissions"][0]["market_id"], 1);
        assert!(json["accounts"][0].get("created_at_ms").is_none());
        assert!(json["accounts"][0].get("updated_at_ms").is_none());
    }

    #[tokio::test]
    async fn admin_mm_permissions_has_no_write_endpoint() {
        let response = router(admin_state(false))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/mm/permissions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn admin_fees_has_no_write_endpoint() {
        let response = router(admin_state(false))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/fees/summary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn admin_empty_summaries_are_valid() {
        for path in [
            "/admin/execution/summary",
            "/admin/rfq/summary",
            "/admin/options/summary",
            "/admin/fees/summary",
        ] {
            let response = router(admin_state(false))
                .oneshot(get_request(path, None))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let json = response_json(response).await;
            assert_eq!(json["persistence_enabled"], false);
        }
    }

    #[tokio::test]
    async fn option_execution_simulate_disabled_returns_clear_error() {
        let state = option_simulation_state(false, true);
        let intent = insert_route_option_intent(&state, route_calldata_ready_intent());

        let response = router(state)
            .oneshot(post_request(&format!(
                "/options/execution-intents/{}/simulate",
                intent.intent_id
            )))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(
            json["error"],
            "configuration error: option execution simulation is disabled"
        );
    }

    #[tokio::test]
    async fn option_execution_simulate_missing_rpc_rejects_and_records_unavailable() {
        let state = option_simulation_state(true, true);
        let intent = insert_route_option_intent(&state, route_calldata_ready_intent());
        let app = router(state);

        let response = app
            .clone()
            .oneshot(post_request(&format!(
                "/options/execution-intents/{}/simulate",
                intent.intent_id
            )))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(
            json["error"],
            "configuration error: RPC_URL is required for option execution simulation"
        );

        let status = app
            .clone()
            .oneshot(get_request(
                &format!("/options/execution-intents/{}/simulation", intent.intent_id),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        let json = response_json(status).await;
        assert_eq!(json["simulation_status"], "simulation_unavailable");
        assert_eq!(
            json["error"],
            "RPC_URL is required for option execution simulation"
        );
        assert_eq!(json["submitted"], false);
        assert_eq!(json["confirmed"], false);

        let transactions = app
            .oneshot(get_request("/executor/transactions", None))
            .await
            .unwrap();
        assert_eq!(transactions.status(), StatusCode::OK);
        assert_eq!(
            response_json(transactions).await.as_array().unwrap().len(),
            0
        );
    }

    #[tokio::test]
    async fn option_execution_simulation_status_defaults_pending() {
        let state = option_simulation_state(true, false);
        let intent = insert_route_option_intent(&state, route_calldata_ready_intent());

        let response = router(state)
            .oneshot(get_request(
                &format!("/options/execution-intents/{}/simulation", intent.intent_id),
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["simulation_status"], "simulation_pending");
        assert_eq!(json["error"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn option_execution_broadcast_disabled_returns_clear_error() {
        let state = option_simulation_state(true, false);
        let intent = insert_route_option_intent(&state, route_calldata_ready_intent());

        let response = router(state)
            .oneshot(post_request(&format!(
                "/options/execution-intents/{}/broadcast",
                intent.intent_id
            )))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(
            json["error"],
            "configuration error: option execution broadcast is disabled"
        );
    }

    #[tokio::test]
    async fn option_nonce_endpoint_disabled_returns_clear_error() {
        let response = router(AppState::new(EngineState::with_default_markets()))
            .oneshot(get_request(
                "/accounts/0x0000000000000000000000000000000000000001/option-nonce",
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["error"], "option nonce sync is disabled");
    }

    #[tokio::test]
    async fn option_nonce_endpoint_enabled_reads_mocked_onchain_nonce() {
        let rpc_url = spawn_nonce_rpc(42).await;
        let mut state = AppState::new(EngineState::with_default_markets());
        state.option_nonce_sync_config = OptionNonceSyncConfig {
            enabled: true,
            require_rpc: true,
            strict: true,
            rpc_url: Some(rpc_url),
            option_matching_engine_address: AccountId::new(
                "0x00000000000000000000000000000000000000ee",
            ),
        };

        let response = router(state)
            .oneshot(get_request(
                "/accounts/0x0000000000000000000000000000000000000001/option-nonce",
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(
            json["account"],
            "0x0000000000000000000000000000000000000001"
        );
        assert_eq!(
            json["option_matching_engine"],
            "0x00000000000000000000000000000000000000ee"
        );
        assert_eq!(json["nonce"], "42");
        assert_eq!(json["source"], "onchain");
    }

    #[tokio::test]
    async fn admin_options_summary_includes_option_simulation_counts() {
        let mut state = admin_state(false);
        state.options_config = option_simulation_state(true, false).options_config;
        let mut simulated = route_calldata_ready_intent();
        simulated.simulation_status = Some(OptionExecutionSimulationStatus::SimulationOk);
        insert_route_option_intent(&state, simulated);
        let mut pending = route_calldata_ready_intent();
        pending.intent_id = Uuid::from_u128(12);
        pending.source_id = "fill-2".to_string();
        insert_route_option_intent(&state, pending);

        let response = router(state)
            .oneshot(get_request("/admin/options/summary", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(
            json["option_execution_simulation_status_counts"]["simulation_ok"],
            1
        );
        assert_eq!(
            json["option_execution_simulation_status_counts"]["simulation_pending"],
            1
        );
        assert_eq!(json["option_execution_simulation_ok"], 1);
    }

    #[tokio::test]
    async fn admin_option_events_returns_config_counts_and_latest_tick() {
        let mut state = admin_state(false);
        state.option_event_indexer_config = crate::options::OptionEventIndexerConfig {
            enabled: true,
            poll_interval_ms: 15_000,
            from_block: 10,
            batch_blocks: 25,
            confirmation_blocks: 3,
            require_rpc: true,
            rpc_url: Some("https://rpc.example/redacted-key".to_string()),
            matching_engine_address: AccountId::new("0x00000000000000000000000000000000000000ee"),
            margin_engine_address: AccountId::new("0x00000000000000000000000000000000000000aa"),
            collateral_vault_address: AccountId::new("0x00000000000000000000000000000000000000bb"),
            fees_manager_address: Some(AccountId::new(
                "0x00000000000000000000000000000000000000cc",
            )),
            fees_manager_v2_address: None,
            old_margin_engine_address: None,
            protocol_fee_vault_address: None,
        };
        let event = route_option_execution_event();
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[event],
                25,
                1_000,
            );
        *state.option_event_indexer_last_tick.lock().unwrap() =
            Some(crate::options::OptionEventIndexerTickResult {
                enabled: true,
                chain_id: 84532,
                current_block_number: Some(30),
                safe_head: Some(27),
                from_block: 11,
                to_block: 25,
                batch_blocks: 25,
                confirmation_blocks: 3,
                logs_found: 1,
                events_decoded: 1,
                events_indexed: 1,
                cursor_updated: true,
                last_indexed_block: 25,
            });

        let response = router(state)
            .oneshot(get_request("/admin/options/events", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["indexer_enabled"], true);
        assert_eq!(json["from_block"], 10);
        assert_eq!(json["batch_blocks"], 25);
        assert_eq!(json["confirmation_blocks"], 3);
        assert_eq!(json["rpc_configured"], true);
        assert_eq!(json["last_indexed_block"], 25);
        assert_eq!(json["counts"]["OptionTradeExecuted"], 1);
        assert_eq!(json["counts"]["OptionPositionUpdated"], 0);
        assert_eq!(json["counts_by_event_name"]["OptionTradeExecuted"], 1);
        assert_eq!(
            json["counts_by_contract_address"]["0x00000000000000000000000000000000000000ee"],
            1
        );
        assert_eq!(json["emitter_contracts"].as_array().unwrap().len(), 4);
        assert_eq!(
            json["config"]["margin_engine_address"],
            "0x00000000000000000000000000000000000000aa"
        );
        assert_eq!(json["latest_tick"]["to_block"], 25);
        assert_eq!(json["recent"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn admin_option_events_tick_rejects_when_indexer_disabled() {
        let state = admin_state(false);
        let response = router(state)
            .oneshot(post_request("/admin/options/events/tick"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(
            json["error"],
            "configuration error: option event indexer is disabled"
        );
    }

    #[tokio::test]
    async fn admin_option_events_tick_rejects_when_rpc_missing() {
        let mut state = admin_state(false);
        state.option_event_indexer_config = crate::options::OptionEventIndexerConfig {
            enabled: true,
            poll_interval_ms: 15_000,
            from_block: 41_856_963,
            batch_blocks: 5,
            confirmation_blocks: 3,
            require_rpc: false,
            rpc_url: None,
            matching_engine_address: AccountId::new("0x00000000000000000000000000000000000000ee"),
            margin_engine_address: AccountId::new("0x00000000000000000000000000000000000000aa"),
            collateral_vault_address: AccountId::new("0x00000000000000000000000000000000000000bb"),
            fees_manager_address: None,
            fees_manager_v2_address: None,
            old_margin_engine_address: None,
            protocol_fee_vault_address: None,
        };

        let response = router(state)
            .oneshot(post_request("/admin/options/events/tick"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(
            json["error"],
            "configuration error: option event indexer requires RPC_URL to run a one-shot tick"
        );
    }

    #[tokio::test]
    async fn admin_option_events_tick_runs_once_and_is_idempotent() {
        let logs = vec![route_option_trade_log()];
        let rpc_url = spawn_logs_rpc(41_856_967, logs).await;
        let mut state = admin_state(false);
        state.option_event_indexer_config = crate::options::OptionEventIndexerConfig {
            enabled: true,
            poll_interval_ms: 15_000,
            from_block: 41_856_963,
            batch_blocks: 5,
            confirmation_blocks: 3,
            require_rpc: true,
            rpc_url: Some(rpc_url),
            matching_engine_address: AccountId::new("0x00000000000000000000000000000000000000ee"),
            margin_engine_address: AccountId::new("0x00000000000000000000000000000000000000aa"),
            collateral_vault_address: AccountId::new("0x00000000000000000000000000000000000000bb"),
            fees_manager_address: None,
            fees_manager_v2_address: None,
            old_margin_engine_address: None,
            protocol_fee_vault_address: None,
        };
        let app = router(state.clone());

        let first = app
            .clone()
            .oneshot(post_request("/admin/options/events/tick"))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let json = response_json(first).await;
        assert_eq!(json["enabled"], true);
        assert_eq!(json["from_block"], 41_856_964);
        assert_eq!(json["to_block"], 41_856_964);
        assert_eq!(json["logs_found"], 1);
        assert_eq!(json["events_decoded"], 1);
        assert_eq!(json["events_indexed"], 1);
        assert_eq!(json["cursor_updated"], true);

        let stored = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(stored.len(), 1);

        let second = app
            .clone()
            .oneshot(post_request("/admin/options/events/tick"))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        let json = response_json(second).await;
        assert_eq!(json["logs_found"], 0);
        assert_eq!(json["events_indexed"], 0);
        assert_eq!(
            state
                .options_store
                .lock()
                .unwrap()
                .list_option_execution_events(10)
                .len(),
            1
        );
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admin_recent_respects_limit_cap() {
        let response = router(admin_state(false))
            .oneshot(get_request("/admin/recent?limit=999", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["limit"], 100);
    }

    #[tokio::test]
    async fn admin_option_reconciliations_returns_counts_and_latest_tick() {
        let mut state = admin_state(false);
        state.option_reconciliation_config = crate::options::OptionReconciliationConfig {
            enabled: true,
            poll_interval_ms: 15_000,
            batch_size: 25,
            require_events: true,
            require_rpc: true,
            strict: true,
            state_checks_enabled: true,
            state_checks_require_rpc: true,
            state_checks_strict: false,
            rpc_url: Some("https://rpc.example/redacted-key".to_string()),
        };
        *state.option_reconciliation_last_tick.lock().unwrap() =
            Some(crate::options::OptionReconciliationTickResult {
                enabled: true,
                batch_size: 25,
                strict: true,
                require_events: true,
                require_rpc: true,
                state_checks_enabled: true,
                state_checks_require_rpc: true,
                state_checks_strict: false,
                considered: 2,
                reconciled: 1,
                partially_reconciled: 0,
                reconciliation_failed: 1,
                missing_events: 0,
                skipped: 0,
                decisions: Vec::new(),
            });

        let response = router(state)
            .oneshot(get_request("/admin/options/reconciliations", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["config"]["enabled"], true);
        assert_eq!(json["config"]["strict"], true);
        assert_eq!(json["config"]["require_events"], true);
        assert_eq!(json["config"]["state_checks_enabled"], true);
        assert_eq!(json["config"]["state_checks_require_rpc"], true);
        assert_eq!(json["config"]["state_checks_strict"], false);
        assert_eq!(json["config"]["rpc_configured"], true);
        assert_eq!(json["counts"]["reconciled"], 0);
        assert_eq!(json["counts"]["partially_reconciled"], 0);
        assert_eq!(json["counts"]["reconciliation_failed"], 0);
        assert_eq!(json["counts"]["missing_events"], 0);
        assert_eq!(json["counts"]["skipped"], 0);
        assert_eq!(json["latest_tick"]["reconciliation_failed"], 1);
        assert_eq!(json["latest_tick"]["reconciled"], 1);
        assert!(json["check_counts"]["state_check_status"].is_object());
        assert!(json["check_counts"]["nonce_check_status"].is_object());
        assert!(json["check_counts"]["position_check_status"].is_object());
        assert!(json["check_counts"]["fee_check_status"].is_object());
        assert!(json["check_counts"]["premium_check_status"].is_object());
        assert!(json["recent"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admin_option_reconciliations_tick_rejects_when_disabled() {
        let response = router(admin_state(false))
            .oneshot(post_request("/admin/options/reconciliations/tick"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(
            json["error"],
            "configuration error: option reconciliation worker is disabled"
        );
    }

    #[tokio::test]
    async fn admin_option_execution_lifecycle_returns_404_for_unknown_intent() {
        let state = admin_state(false);
        let response = router(state)
            .oneshot(get_request(
                "/admin/options/executions/e6d2941b-65f7-413a-958f-74ab22c53b08/lifecycle",
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn admin_option_execution_lifecycle_returns_aggregated_view() {
        let state = admin_state(false);
        let intent_id = Uuid::from_u128(7777);
        let intent = OptionExecutionIntent {
            intent_id,
            onchain_intent_id: "0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5"
                .to_string(),
            source_type: crate::options::OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-route-1".to_string(),
            option_series_id: "series-route-1".to_string(),
            onchain_option_id: "7".to_string(),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            underlying: AccountId::new("0x0000000000000000000000000000000000000010"),
            settlement_asset: AccountId::new("0x0000000000000000000000000000000000000020"),
            expiry: 4_102_444_800,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: 100_000_000,
            quantity_contracts: 1,
            source_size_1e8: 100_000_000,
            source_price_1e8: 10_000_000,
            premium_per_contract_native: 10_000,
            buyer_is_maker: false,
            buyer_nonce: Some(0),
            seller_nonce: Some(0),
            deadline: 0,
            buyer_signature: Some("0x01".to_string()),
            seller_signature: Some("0x02".to_string()),
            calldata: Some("0x031f77b3deadbeef".to_string()),
            status: OptionExecutionIntentStatus::SimulationOk,
            error: None,
            simulation_status: Some(OptionExecutionSimulationStatus::SimulationOk),
            simulation_error: None,
            simulation_block_number: Some(41_856_962),
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: Some(1),
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        state
            .options_store
            .lock()
            .unwrap()
            .insert_option_execution_intent(intent);

        let response = router(state)
            .oneshot(get_request(
                &format!("/admin/options/executions/{intent_id}/lifecycle"),
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["intent_id"], intent_id.to_string());
        assert_eq!(json["status"], "simulation_ok");
        assert_eq!(json["calldata"]["present"], true);
        assert_eq!(json["calldata"]["selector"], "0x031f77b3");
        assert_eq!(json["calldata"]["byte_length"], 8);
        assert_eq!(json["trade"]["quantity_contracts"], "1");
        assert_eq!(json["trade"]["premium_per_contract_native"], "10000");
        assert!(json["broadcast"].is_null());
        assert_eq!(json["events"]["total"], 0);
        assert_eq!(json["health"]["stage"], "simulation_ok");
        assert_eq!(json["health"]["is_terminal_success"], false);
    }

    #[tokio::test]
    async fn admin_fees_onchain_exposes_observed_trading_fee_events() {
        let state = admin_state(false);
        let tx_hash =
            "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125".to_string();
        let buyer_event = build_trading_fee_log_row(
            1,
            tx_hash.as_str(),
            4,
            "0xc0a76c2a00000000000000000000000000000000",
            6,
            false,
        );
        let seller_event = build_trading_fee_log_row(
            2,
            tx_hash.as_str(),
            5,
            "0xbaf0976a00000000000000000000000000000000",
            4,
            true,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[buyer_event.clone(), seller_event.clone()],
                buyer_event.block_number,
                1,
            );
        let before_fee_events = state.fees_store.lock().unwrap().list_fee_events(10).len();
        let before_option_events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(100)
            .len();

        let response = router(state.clone())
            .oneshot(get_request(
                &format!("/admin/fees/onchain?tx_hash={tx_hash}"),
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["source_of_truth"], "onchain");
        assert_eq!(json["trading_fee_event_count"], 2);
        assert_eq!(json["observed_total"], "10");
        assert_eq!(json["reconciliation_status"], "onchain_observed");
        assert_eq!(json["backend_ledger_enabled"], false);
        assert_eq!(json["backend_ledger_status"], "disabled");
        assert_eq!(
            json["by_trader"]["0xc0a76c2a00000000000000000000000000000000"],
            "6"
        );
        assert_eq!(
            json["by_trader"]["0xbaf0976a00000000000000000000000000000000"],
            "4"
        );
        assert_eq!(json["by_side"]["taker"], "6");
        assert_eq!(json["by_side"]["maker"], "4");
        let transactions = json["transactions"].as_array().unwrap();
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0]["tx_hash"], tx_hash);
        assert_eq!(transactions[0]["observed_total"], "10");

        // Read-only — the backend fee ledger and the indexed event ledger
        // must be unchanged by the admin call.
        assert_eq!(
            state.fees_store.lock().unwrap().list_fee_events(10).len(),
            before_fee_events
        );
        assert_eq!(
            state
                .options_store
                .lock()
                .unwrap()
                .list_option_execution_events(100)
                .len(),
            before_option_events
        );
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn admin_fees_onchain_exposes_v2_charged_and_rebated_totals() {
        let state = admin_state(false);
        let tx_hash =
            "0x7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e".to_string();
        let charged = build_fee_charged_v2_log_row(
            1,
            tx_hash.as_str(),
            4,
            "0xc0a76c2a00000000000000000000000000000000",
            25,
            250,
            false,
        );
        let rebated = build_fee_rebated_v2_log_row(
            2,
            tx_hash.as_str(),
            5,
            "0xbaf0976a00000000000000000000000000000000",
            5,
            -50,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[charged.clone(), rebated.clone()],
                charged.block_number,
                1,
            );

        let response = router(state.clone())
            .oneshot(get_request(
                &format!("/admin/fees/onchain?tx_hash={tx_hash}"),
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["source_of_truth"], "onchain");
        assert_eq!(json["event_model"], "v2");
        assert_eq!(json["fee_charged_v2_count"], 1);
        assert_eq!(json["fee_rebated_v2_count"], 1);
        assert_eq!(json["trading_fee_event_count"], 0);
        assert_eq!(json["observed_total"], "25");
        assert_eq!(json["observed_total_charged"], "25");
        assert_eq!(json["observed_total_rebated"], "5");
        assert_eq!(json["net_protocol_fee"], "20");
        assert_eq!(json["by_side"]["taker"], "25");
        assert_eq!(
            json["rebated_by_trader"]["0xbaf0976a00000000000000000000000000000000"],
            "5"
        );
        let transactions = json["transactions"].as_array().unwrap();
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0]["event_model"], "v2");
        assert_eq!(transactions[0]["observed_total_charged"], "25");
        assert_eq!(transactions[0]["observed_total_rebated"], "5");
        assert_eq!(transactions[0]["net_protocol_fee"], "20");
        // V2E-I: both V2 entries surface basis_amount through the
        // /admin/fees/onchain.events list.
        let events = json["events"].as_array().unwrap();
        let v2_with_basis = events
            .iter()
            .filter(|payload| {
                (payload["event_name"] == "FeeChargedV2" || payload["event_name"] == "FeeRebatedV2")
                    && payload["basis_amount"] == "10000"
            })
            .count();
        assert_eq!(v2_with_basis, 2);
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    /// V2F-N: the V2F-LM PERP fee smoke transaction
    /// (`0x400acedf…ff63a79a`, block 42188599) emitted two `FeeChargedV2`
    /// events with `productKind = PERP`, `flowKind = ORDERBOOK`,
    /// `basisAmount = 30`, and `feeAmount = 1` each (taker `feePpm = 300`,
    /// maker `feePpm = 50`). `/admin/fees/onchain?tx_hash=…` must summarize
    /// the PERP economics without any option lifecycle, and the per-event
    /// payloads must surface `product_kind = "perp"` so the admin UI can
    /// label the row correctly.
    #[tokio::test]
    async fn admin_fees_onchain_summarizes_v2f_lm_perp_fee_tx() {
        let state = admin_state(false);
        let tx_hash =
            "0x400acedf36381034ae37c983cc50e80d11a81587ca8065fbaef40293ff63a79a".to_string();
        let buyer_taker = "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34";
        let seller_maker = "0x475fe397fa56884952d350aa9ee1c3946964bc0c";
        let taker_leg =
            build_fee_charged_v2_perp_log_row(1, tx_hash.as_str(), 117, buyer_taker, 1, 300, false);
        let maker_leg =
            build_fee_charged_v2_perp_log_row(2, tx_hash.as_str(), 118, seller_maker, 1, 50, true);
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[taker_leg.clone(), maker_leg.clone()],
                taker_leg.block_number,
                1,
            );

        let response = router(state.clone())
            .oneshot(get_request(
                &format!("/admin/fees/onchain?tx_hash={tx_hash}"),
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["source_of_truth"], "onchain");
        assert_eq!(json["event_model"], "v2");
        assert_eq!(json["fee_charged_v2_count"], 2);
        assert_eq!(json["fee_rebated_v2_count"], 0);
        assert_eq!(json["trading_fee_event_count"], 0);
        assert_eq!(json["observed_total"], "2");
        assert_eq!(json["observed_total_charged"], "2");
        assert_eq!(json["observed_total_rebated"], "0");
        assert_eq!(json["net_protocol_fee"], "2");
        assert_eq!(json["by_side"]["taker"], "1");
        assert_eq!(json["by_side"]["maker"], "1");
        assert_eq!(json["by_trader"][buyer_taker], "1");
        assert_eq!(json["by_trader"][seller_maker], "1");
        let transactions = json["transactions"].as_array().unwrap();
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0]["tx_hash"], tx_hash);
        assert_eq!(transactions[0]["event_model"], "v2");
        assert_eq!(transactions[0]["observed_total_charged"], "2");
        assert_eq!(transactions[0]["fee_charged_v2_count"], 2);

        let events = json["events"].as_array().unwrap();
        let perp_events: Vec<&serde_json::Value> = events
            .iter()
            .filter(|payload| {
                payload["event_name"] == "FeeChargedV2"
                    && payload["product_kind"] == "perp"
                    && payload["flow_kind"] == "orderbook"
                    && payload["basis_amount"] == "30"
            })
            .collect();
        assert_eq!(perp_events.len(), 2);
        let taker_payload = perp_events
            .iter()
            .find(|payload| payload["trader"] == buyer_taker)
            .unwrap();
        assert_eq!(taker_payload["fee_ppm"], 300);
        assert_eq!(taker_payload["fee_amount"], "1");
        assert_eq!(taker_payload["is_maker"], false);
        let maker_payload = perp_events
            .iter()
            .find(|payload| payload["trader"] == seller_maker)
            .unwrap();
        assert_eq!(maker_payload["fee_ppm"], 50);
        assert_eq!(maker_payload["fee_amount"], "1");
        assert_eq!(maker_payload["is_maker"], true);

        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    /// V2F-N: even when PerpEngineV2 still emits a V1 `TradingFeeCharged`
    /// breadcrumb alongside the V2 events during the bridging window, the
    /// admin endpoint must report `source_priority = "v2"` and keep
    /// `observed_total = 2` rather than double-counting (1 + 1 + 1 = 3).
    #[tokio::test]
    async fn admin_fees_onchain_v2f_lm_perp_mixed_does_not_double_count() {
        let state = admin_state(false);
        let tx_hash =
            "0x400acedf36381034ae37c983cc50e80d11a81587ca8065fbaef40293ff63a79b".to_string();
        let buyer_taker = "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34";
        let v1_breadcrumb =
            build_trading_fee_log_row(11, tx_hash.as_str(), 116, buyer_taker, 1, false);
        let taker_leg = build_fee_charged_v2_perp_log_row(
            12,
            tx_hash.as_str(),
            117,
            buyer_taker,
            1,
            300,
            false,
        );
        let maker_leg = build_fee_charged_v2_perp_log_row(
            13,
            tx_hash.as_str(),
            118,
            "0x475fe397fa56884952d350aa9ee1c3946964bc0c",
            1,
            50,
            true,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[v1_breadcrumb.clone(), taker_leg.clone(), maker_leg.clone()],
                taker_leg.block_number,
                1,
            );

        let response = router(state.clone())
            .oneshot(get_request(
                &format!("/admin/fees/onchain?tx_hash={tx_hash}"),
                None,
            ))
            .await
            .unwrap();

        let json = response_json(response).await;
        assert_eq!(json["event_model"], "mixed");
        assert_eq!(json["source_priority"], "v2");
        assert_eq!(json["fee_charged_v2_count"], 2);
        assert_eq!(json["trading_fee_event_count"], 1);
        // V2 wins: 1 + 1 = 2, not 1 + 1 + 1 = 3.
        assert_eq!(json["observed_total"], "2");
        assert_eq!(json["observed_total_charged"], "2");
        assert_eq!(json["observed_total_rebated"], "0");
        assert_eq!(json["net_protocol_fee"], "2");
        assert!(state.repository.is_none());
    }

    #[tokio::test]
    async fn admin_fees_onchain_mixed_v1_v2_uses_v2_priority() {
        let state = admin_state(false);
        let tx_hash =
            "0x8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e8e".to_string();
        let v1 = build_trading_fee_log_row(
            1,
            tx_hash.as_str(),
            4,
            "0xc0a76c2a00000000000000000000000000000000",
            6,
            false,
        );
        let v2 = build_fee_charged_v2_log_row(
            2,
            tx_hash.as_str(),
            5,
            "0xc0a76c2a00000000000000000000000000000000",
            25,
            250,
            false,
        );
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                &[v1.clone(), v2.clone()],
                v1.block_number,
                1,
            );

        let response = router(state.clone())
            .oneshot(get_request(
                &format!("/admin/fees/onchain?tx_hash={tx_hash}"),
                None,
            ))
            .await
            .unwrap();

        let json = response_json(response).await;
        assert_eq!(json["event_model"], "mixed");
        assert_eq!(json["source_priority"], "v2");
        assert_eq!(json["trading_fee_event_count"], 1);
        assert_eq!(json["fee_charged_v2_count"], 1);
        // Must not be 31 (would be V1 + V2 double-counted).
        assert_eq!(json["observed_total"], "25");
        assert_eq!(json["observed_total_charged"], "25");
        assert_eq!(json["observed_total_rebated"], "0");
        assert_eq!(json["net_protocol_fee"], "25");
        assert!(state.repository.is_none());
    }

    fn build_fee_charged_v2_log_row(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        fee_amount: u128,
        fee_ppm: i32,
        is_maker: bool,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(2_000 + id_seed),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000dd".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 41_856_965,
            block_hash: None,
            event_name: "FeeChargedV2".to_string(),
            event_signature: "FeeChargedV2".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some(fee_amount.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "consumer": "0x00000000000000000000000000000000000000aa",
                "trader": trader,
                "recipient": "0x009f38440f058d095b61e0e2ee7fabdf05be7500",
                "settlementAsset": "0x0000000000000000000000000000000000000020",
                "productKind": "option",
                "flowKind": "orderbook",
                "isMaker": is_maker,
                "feePpm": fee_ppm,
                "basisAmount": "10000",
                "feeAmount": fee_amount.to_string(),
            })),
            created_at_ms: 6,
            updated_at_ms: 6,
        }
    }

    /// V2F-P: PERP-flavoured `FeeChargedV2` log row with a caller-chosen
    /// `consumer` address. Used by the metric-classification tests so
    /// they can exercise the `new` / `old` / `unknown` buckets without
    /// editing the default V2F-LM helper.
    #[allow(clippy::too_many_arguments)]
    fn build_fee_charged_v2_perp_log_row_for_consumer(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        consumer: &str,
        trader: &str,
        fee_amount: u128,
        fee_ppm: i32,
        is_maker: bool,
    ) -> crate::options::OptionExecutionEvent {
        let mut event = build_fee_charged_v2_perp_log_row(
            id_seed, tx_hash, log_index, trader, fee_amount, fee_ppm, is_maker,
        );
        if let Some(decoded) = event.decoded.as_mut() {
            if let Some(map) = decoded.as_object_mut() {
                map.insert(
                    "consumer".to_string(),
                    serde_json::Value::String(consumer.to_ascii_lowercase()),
                );
            }
        }
        event
    }

    /// V2F-N: a PERP-flavoured `FeeChargedV2` log row mirroring an event
    /// emitted by FeesManagerV2 when PerpEngineV2 calls `chargeFee`.
    /// Reproduces the V2F-LM live event shape (`productKind = perp`,
    /// `flowKind = orderbook`, `basisAmount = 30`).
    fn build_fee_charged_v2_perp_log_row(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        fee_amount: u128,
        fee_ppm: i32,
        is_maker: bool,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(4_000 + id_seed),
            chain_id: 84532,
            contract_address: "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 42_188_599,
            block_hash: None,
            event_name: "FeeChargedV2".to_string(),
            event_signature: "FeeChargedV2".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some(fee_amount.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "consumer": "0xc6c592100723fe0c66343a16e95ec34cc0c2141c",
                "trader": trader,
                "recipient": "0x009f38440f058d095b61e0e2ee7fabdf05be7500",
                "settlementAsset": "0x6eae407f5640b006fac9965182e238582a3b412e",
                "productKind": "perp",
                "productKindRaw": 1,
                "flowKind": "orderbook",
                "flowKindRaw": 0,
                "isMaker": is_maker,
                "feePpm": fee_ppm,
                "basisAmount": "30",
                "feeAmount": fee_amount.to_string(),
            })),
            created_at_ms: 6,
            updated_at_ms: 6,
        }
    }

    /// V2F-Q: PERP-flavoured `FeeRebatedV2` log row.
    /// `consumer` defaults to NEW PerpEngine; `productKind = "perp"`.
    /// Mirrors `build_fee_charged_v2_perp_log_row` for symmetry.
    fn build_fee_rebated_v2_perp_log_row(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        rebate_amount: u128,
        rebate_ppm: i32,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(5_000 + id_seed),
            chain_id: 84532,
            contract_address: "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 42_188_600,
            block_hash: None,
            event_name: "FeeRebatedV2".to_string(),
            event_signature: "FeeRebatedV2".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some(rebate_amount.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "consumer": "0xc6c592100723fe0c66343a16e95ec34cc0c2141c",
                "trader": trader,
                "recipient": trader,
                "settlementAsset": "0x6eae407f5640b006fac9965182e238582a3b412e",
                "productKind": "perp",
                "productKindRaw": 1,
                "flowKind": "orderbook",
                "flowKindRaw": 0,
                "isMaker": true,
                "rebatePpm": rebate_ppm,
                "basisAmount": "30",
                "rebateAmount": rebate_amount.to_string(),
            })),
            created_at_ms: 6,
            updated_at_ms: 6,
        }
    }

    /// V2F-Q: PERP-flavoured `FeeRebatedV2` log row with a caller-chosen
    /// `consumer` address. Used by the rebate-metric classification
    /// tests to exercise the `new` / `old` / `unknown` buckets.
    #[allow(clippy::too_many_arguments)]
    fn build_fee_rebated_v2_perp_log_row_for_consumer(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        consumer: &str,
        trader: &str,
        rebate_amount: u128,
        rebate_ppm: i32,
    ) -> crate::options::OptionExecutionEvent {
        let mut event = build_fee_rebated_v2_perp_log_row(
            id_seed,
            tx_hash,
            log_index,
            trader,
            rebate_amount,
            rebate_ppm,
        );
        if let Some(decoded) = event.decoded.as_mut() {
            if let Some(map) = decoded.as_object_mut() {
                map.insert(
                    "consumer".to_string(),
                    serde_json::Value::String(consumer.to_ascii_lowercase()),
                );
            }
        }
        event
    }

    fn build_fee_rebated_v2_log_row(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        rebate_amount: u128,
        rebate_ppm: i32,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(3_000 + id_seed),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000dd".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 41_856_965,
            block_hash: None,
            event_name: "FeeRebatedV2".to_string(),
            event_signature: "FeeRebatedV2".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some(rebate_amount.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "consumer": "0x00000000000000000000000000000000000000aa",
                "trader": trader,
                "recipient": trader,
                "settlementAsset": "0x0000000000000000000000000000000000000020",
                "productKind": "option",
                "flowKind": "orderbook",
                "isMaker": true,
                "rebatePpm": rebate_ppm,
                "basisAmount": "10000",
                "rebateAmount": rebate_amount.to_string(),
            })),
            created_at_ms: 6,
            updated_at_ms: 6,
        }
    }

    /// V2G-F: OPTION-flavoured `FeeChargedV2` log row with NEW MarginEngine
    /// as the consumer by default. Mirrors `build_fee_charged_v2_perp_log_row`
    /// but with `productKind = "option"` and contract_address pointing at
    /// FeesManagerV2. Used by the OPTION metric tests and the
    /// PERP/OPTION-separation test.
    fn build_fee_charged_v2_option_log_row(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        fee_amount: u128,
        fee_ppm: i32,
        is_maker: bool,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(6_000 + id_seed),
            chain_id: 84532,
            contract_address: "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 42_206_003,
            block_hash: None,
            event_name: "FeeChargedV2".to_string(),
            event_signature: "FeeChargedV2".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some(fee_amount.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "consumer": "0x287cef479be5889eefca847f9e73c860898f48cc",
                "trader": trader,
                "recipient": "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588",
                "settlementAsset": "0x6eae407f5640b006fac9965182e238582a3b412e",
                "productKind": "option",
                "productKindRaw": 0,
                "flowKind": "orderbook",
                "flowKindRaw": 0,
                "isMaker": is_maker,
                "feePpm": fee_ppm,
                "basisAmount": "200000",
                "feeAmount": fee_amount.to_string(),
            })),
            created_at_ms: 6,
            updated_at_ms: 6,
        }
    }

    /// V2G-F: OPTION-flavoured `FeeChargedV2` log row with a caller-chosen
    /// `consumer` address. Used by the OPTION metric tests to exercise
    /// the `new` / `old` / `unknown` buckets.
    #[allow(clippy::too_many_arguments)]
    fn build_fee_charged_v2_option_log_row_for_consumer(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        consumer: &str,
        trader: &str,
        fee_amount: u128,
        fee_ppm: i32,
        is_maker: bool,
    ) -> crate::options::OptionExecutionEvent {
        let mut event = build_fee_charged_v2_option_log_row(
            id_seed, tx_hash, log_index, trader, fee_amount, fee_ppm, is_maker,
        );
        if let Some(decoded) = event.decoded.as_mut() {
            if let Some(map) = decoded.as_object_mut() {
                map.insert(
                    "consumer".to_string(),
                    serde_json::Value::String(consumer.to_ascii_lowercase()),
                );
            }
        }
        event
    }

    /// V2G-F: OPTION-flavoured `FeeRebatedV2` log row with NEW MarginEngine
    /// as the consumer by default. Mirrors `build_fee_rebated_v2_perp_log_row`
    /// but with `productKind = "option"`.
    fn build_fee_rebated_v2_option_log_row(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        rebate_amount: u128,
        rebate_ppm: i32,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(7_000 + id_seed),
            chain_id: 84532,
            contract_address: "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 42_206_003,
            block_hash: None,
            event_name: "FeeRebatedV2".to_string(),
            event_signature: "FeeRebatedV2".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some(rebate_amount.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "consumer": "0x287cef479be5889eefca847f9e73c860898f48cc",
                "trader": trader,
                "recipient": trader,
                "settlementAsset": "0x6eae407f5640b006fac9965182e238582a3b412e",
                "productKind": "option",
                "productKindRaw": 0,
                "flowKind": "orderbook",
                "flowKindRaw": 0,
                "isMaker": true,
                "rebatePpm": rebate_ppm,
                "basisAmount": "200000",
                "rebateAmount": rebate_amount.to_string(),
            })),
            created_at_ms: 6,
            updated_at_ms: 6,
        }
    }

    /// V2G-F: OPTION-flavoured `FeeRebatedV2` log row with a caller-chosen
    /// `consumer` address.
    fn build_fee_rebated_v2_option_log_row_for_consumer(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        consumer: &str,
        trader: &str,
        rebate_amount: u128,
        rebate_ppm: i32,
    ) -> crate::options::OptionExecutionEvent {
        let mut event = build_fee_rebated_v2_option_log_row(
            id_seed,
            tx_hash,
            log_index,
            trader,
            rebate_amount,
            rebate_ppm,
        );
        if let Some(decoded) = event.decoded.as_mut() {
            if let Some(map) = decoded.as_object_mut() {
                map.insert(
                    "consumer".to_string(),
                    serde_json::Value::String(consumer.to_ascii_lowercase()),
                );
            }
        }
        event
    }

    /// V2G-F: a minimal RebateBudgetFunded / Spent / Withdrawn event
    /// row with caller-chosen `settlementAsset` (lowercased) and
    /// `amount` (native units). Used by the budget gauge test.
    fn build_rebate_budget_event(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        event_name: &str,
        settlement_asset: &str,
        amount: u128,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(8_000 + id_seed),
            chain_id: 84532,
            contract_address: "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 42_206_000,
            block_hash: None,
            event_name: event_name.to_string(),
            event_signature: event_name.to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: None,
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some(amount.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "settlementAsset": settlement_asset.to_ascii_lowercase(),
                "amount": amount.to_string(),
            })),
            created_at_ms: 6,
            updated_at_ms: 6,
        }
    }

    fn build_trading_fee_log_row(
        id_seed: u128,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        applied_fee: u128,
        is_maker: bool,
    ) -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(900 + id_seed),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000aa".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 41_856_964,
            block_hash: None,
            event_name: "TradingFeeCharged".to_string(),
            event_signature: "TradingFeeCharged".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: Some("7".to_string()),
            quantity_contracts: None,
            premium_per_contract_native: Some(applied_fee.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "trader": trader,
                "appliedFee": applied_fee.to_string(),
                "isMaker": is_maker,
                "recipient": "0x009f3849df0d4f2547cfb72cc3e7500",
                "settlementAsset": "0x0000000000000000000000000000000000000020"
            })),
            created_at_ms: 5,
            updated_at_ms: 5,
        }
    }

    #[tokio::test]
    async fn admin_endpoints_do_not_mutate_state() {
        let state = admin_state(false);
        let app = router(state.clone());
        let before_intents = state.engine.lock().unwrap().execution_intents().len();
        let before_rfqs = state.rfq_store.lock().unwrap().list_rfqs().len();
        let before_option_rfqs = state.options_store.lock().unwrap().list_option_rfqs().len();
        let before_option_events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(100)
            .len();
        let before_fee_events = state.fees_store.lock().unwrap().list_fee_events(10).len();

        for path in [
            "/admin/status",
            "/admin/config",
            "/admin/db",
            "/admin/mm/sessions",
            "/admin/mm/permissions",
            "/admin/execution/summary",
            "/admin/rfq/summary",
            "/admin/options/summary",
            "/admin/options/events",
            "/admin/options/reconciliations",
            "/admin/fees/summary",
            "/admin/fees/events?limit=5",
            "/admin/fees/onchain?limit=5",
            "/admin/fees/volumes",
            "/admin/fees/rebates",
            "/admin/recent?limit=5",
        ] {
            let response = app.clone().oneshot(get_request(path, None)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        assert_eq!(
            state.engine.lock().unwrap().execution_intents().len(),
            before_intents
        );
        assert_eq!(
            state.rfq_store.lock().unwrap().list_rfqs().len(),
            before_rfqs
        );
        assert_eq!(
            state.options_store.lock().unwrap().list_option_rfqs().len(),
            before_option_rfqs
        );
        assert_eq!(
            state
                .options_store
                .lock()
                .unwrap()
                .list_option_execution_events(100)
                .len(),
            before_option_events
        );
        assert_eq!(
            state.fees_store.lock().unwrap().list_fee_events(10).len(),
            before_fee_events
        );
    }

    #[tokio::test]
    async fn metrics_and_readiness_do_not_mutate_state() {
        let state = admin_state(false);
        let app = router(state.clone());
        let before_intents = state.engine.lock().unwrap().execution_intents().len();
        let before_rfqs = state.rfq_store.lock().unwrap().list_rfqs().len();
        let before_option_rfqs = state.options_store.lock().unwrap().list_option_rfqs().len();
        let before_fee_events = state.fees_store.lock().unwrap().list_fee_events(10).len();

        for path in ["/metrics", "/ready"] {
            let response = app.clone().oneshot(get_request(path, None)).await.unwrap();
            assert!(response.status().is_success());
        }

        assert_eq!(
            state.engine.lock().unwrap().execution_intents().len(),
            before_intents
        );
        assert_eq!(
            state.rfq_store.lock().unwrap().list_rfqs().len(),
            before_rfqs
        );
        assert_eq!(
            state.options_store.lock().unwrap().list_option_rfqs().len(),
            before_option_rfqs
        );
        assert_eq!(
            state.fees_store.lock().unwrap().list_fee_events(10).len(),
            before_fee_events
        );
    }

    fn admin_state(require_token: bool) -> AppState {
        let mut state = AppState::new(EngineState::with_default_markets());
        state.admin_config = AdminConfig::new(
            true,
            require_token,
            require_token.then(|| "test-admin-token".to_string()),
        );
        state.network_name = "admin-test".to_string();
        state
    }

    fn option_simulation_state(simulation_enabled: bool, require_rpc: bool) -> AppState {
        let mut options_config = crate::options::OptionsConfig::enabled_in_memory_for_tests();
        options_config.execution_enabled = true;
        options_config.execution_require_persistence = false;
        options_config.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        options_config.execution_eip712_domain.verifying_contract =
            options_config.matching_engine_address.clone();
        options_config.execution_simulation_enabled = simulation_enabled;
        options_config.execution_require_rpc_for_simulation = require_rpc;
        AppState::with_options_config(EngineState::with_default_markets(), options_config)
    }

    fn insert_route_option_intent(
        state: &AppState,
        intent: OptionExecutionIntent,
    ) -> OptionExecutionIntent {
        state
            .options_store
            .lock()
            .unwrap()
            .insert_option_execution_intent(intent)
    }

    fn route_option_execution_event() -> crate::options::OptionExecutionEvent {
        crate::options::OptionExecutionEvent {
            id: Uuid::from_u128(500),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000ee".to_string(),
            tx_hash: "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125"
                .to_string(),
            log_index: 2,
            block_number: 20,
            block_hash: Some(
                "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            event_name: "OptionTradeExecuted".to_string(),
            event_signature: crate::options::OPTION_TRADE_EXECUTED_SIGNATURE.to_string(),
            intent_id: Some(Uuid::from_u128(11)),
            onchain_intent_id: Some(
                "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            ),
            option_execution_transaction_id: Some("option-tx-1".to_string()),
            buyer: Some("0x0000000000000000000000000000000000000001".to_string()),
            seller: Some("0x0000000000000000000000000000000000000002".to_string()),
            account: None,
            option_id: Some("7".to_string()),
            quantity_contracts: Some("1".to_string()),
            premium_per_contract_native: Some("10000".to_string()),
            raw_topics: serde_json::json!([
                crate::options::option_trade_executed_topic0(),
                "0x1111111111111111111111111111111111111111111111111111111111111111"
            ]),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({"buyerIsMaker": false})),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn route_calldata_ready_intent() -> OptionExecutionIntent {
        let expiry = 4_102_444_800;
        let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
        let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
        let onchain_option_id = crate::options::option_product_registry_option_id(
            &underlying,
            &settlement_asset,
            expiry,
            300_000_000_000,
            100_000_000,
            true,
            true,
        )
        .unwrap()
        .to_string();
        OptionExecutionIntent {
            intent_id: Uuid::from_u128(11),
            onchain_intent_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
            source_type: crate::options::OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-1".to_string(),
            option_series_id: "series-1".to_string(),
            onchain_option_id,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            underlying,
            settlement_asset,
            expiry,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: 100_000_000,
            quantity_contracts: 1,
            source_size_1e8: 100_000_000,
            source_price_1e8: 10_000_000,
            premium_per_contract_native: 10_000,
            buyer_is_maker: false,
            buyer_nonce: Some(0),
            seller_nonce: Some(0),
            deadline: 0,
            buyer_signature: Some("0x01".to_string()),
            seller_signature: Some("0x02".to_string()),
            calldata: Some("0x12345678".to_string()),
            status: OptionExecutionIntentStatus::CalldataReady,
            error: None,
            simulation_status: None,
            simulation_error: None,
            simulation_block_number: None,
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn eth_log_to_json(log: &crate::indexer::EthLog) -> serde_json::Value {
        serde_json::json!({
            "address": log.address,
            "topics": log.topics,
            "data": log.data,
            "blockNumber": log.block_number,
            "blockHash": log.block_hash,
            "transactionHash": log.transaction_hash,
            "logIndex": log.log_index,
        })
    }

    fn route_option_trade_log() -> crate::indexer::EthLog {
        let topic0 = crate::options::option_trade_executed_topic0();
        let mut data = String::from("0x");
        for value in [7u128, 1, 10_000, 0, 0, 0] {
            data.push_str(&format!("{value:064x}"));
        }
        crate::indexer::EthLog {
            address: "0x00000000000000000000000000000000000000ee".to_string(),
            topics: vec![
                topic0,
                "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
                format!("0x{:0>64}", "0000000000000000000000000000000000000001"),
                format!("0x{:0>64}", "0000000000000000000000000000000000000002"),
            ],
            data,
            block_number: Some(format!("0x{:x}", 41_856_964u64)),
            block_hash: Some(
                "0x53d62c21ecbe462e2868e216b4655474de0d2b7b832f15ab6e72b216fb1f3853".to_string(),
            ),
            transaction_hash: Some(
                "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125".to_string(),
            ),
            log_index: Some(format!("0x{:x}", 2u64)),
        }
    }

    async fn spawn_logs_rpc(head: u64, logs: Vec<crate::indexer::EthLog>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let head_hex = format!("0x{head:x}");
        let logs = std::sync::Arc::new(logs);
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(move |Json(payload): Json<serde_json::Value>| {
                let head_hex = head_hex.clone();
                let logs = logs.clone();
                async move {
                    let method = payload
                        .get("method")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    let id = payload.get("id").cloned().unwrap_or(serde_json::json!(1));
                    let result = match method {
                        "eth_blockNumber" => serde_json::Value::String(head_hex),
                        "eth_getLogs" => {
                            let filter = payload
                                .get("params")
                                .and_then(|params| params.get(0))
                                .cloned()
                                .unwrap_or(serde_json::Value::Null);
                            let address = filter
                                .get("address")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_ascii_lowercase();
                            let topic0 = filter
                                .get("topics")
                                .and_then(|value| value.as_array())
                                .and_then(|values| values.first())
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_ascii_lowercase();
                            let filtered = logs
                                .iter()
                                .filter(|log| log.address.eq_ignore_ascii_case(&address))
                                .filter(|log| {
                                    log.topics
                                        .first()
                                        .map(|value| value.eq_ignore_ascii_case(&topic0))
                                        .unwrap_or(false)
                                })
                                .map(eth_log_to_json)
                                .collect::<Vec<_>>();
                            serde_json::Value::Array(filtered)
                        }
                        _ => serde_json::Value::Null,
                    };
                    Json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }))
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    async fn spawn_nonce_rpc(nonce: u128) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let output = uint256_hex(nonce);
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(move |Json(payload): Json<serde_json::Value>| {
                let output = output.clone();
                async move {
                    let method = payload
                        .get("method")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    let result = match method {
                        "eth_blockNumber" => serde_json::json!("0x7b"),
                        "eth_call" => serde_json::json!(output),
                        _ => serde_json::Value::Null,
                    };
                    Json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": payload.get("id").cloned().unwrap_or(serde_json::json!(1)),
                        "result": result
                    }))
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn uint256_hex(value: u128) -> String {
        let mut bytes = [0u8; 32];
        bytes[16..32].copy_from_slice(&value.to_be_bytes());
        let mut encoded = String::from("0x");
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        encoded
    }

    fn get_request(path: &str, token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(path);
        if let Some(token) = token {
            builder = builder.header("x-admin-token", token);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn post_request(path: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn response_text(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }
}

fn perp_trade_type_fields() -> Vec<SigningPayloadTypeField> {
    let _ = PERP_TRADE_TYPE;
    vec![
        SigningPayloadTypeField {
            name: "intentId",
            type_name: "bytes32",
        },
        SigningPayloadTypeField {
            name: "buyer",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "seller",
            type_name: "address",
        },
        SigningPayloadTypeField {
            name: "marketId",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "sizeDelta1e8",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "executionPrice1e8",
            type_name: "uint128",
        },
        SigningPayloadTypeField {
            name: "buyerIsMaker",
            type_name: "bool",
        },
        SigningPayloadTypeField {
            name: "buyerNonce",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "sellerNonce",
            type_name: "uint256",
        },
        SigningPayloadTypeField {
            name: "deadline",
            type_name: "uint256",
        },
    ]
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal server error".to_string(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }
}

impl From<BackendError> for ApiError {
    fn from(value: BackendError) -> Self {
        let status = match value {
            BackendError::InvalidOrderId => StatusCode::BAD_REQUEST,
            BackendError::InvalidExecutionIntentId => StatusCode::NOT_FOUND,
            BackendError::InvalidRfqId | BackendError::InvalidRfqQuoteId => StatusCode::NOT_FOUND,
            BackendError::InvalidOptionSeriesId(_) => StatusCode::NOT_FOUND,
            BackendError::InvalidOptionOrderId => StatusCode::NOT_FOUND,
            BackendError::InvalidOptionFillId => StatusCode::NOT_FOUND,
            BackendError::InvalidOptionExecutionIntentId => StatusCode::NOT_FOUND,
            BackendError::InvalidOptionRfqId | BackendError::InvalidOptionRfqQuoteId => {
                StatusCode::NOT_FOUND
            }
            BackendError::OrderNotFound(_) | BackendError::OrderNotOpen(_) => StatusCode::NOT_FOUND,
            BackendError::MmPermissionDenied(_) => StatusCode::FORBIDDEN,
            BackendError::InvalidFixedPoint { .. } => StatusCode::BAD_REQUEST,
            BackendError::DeadlineExpired
            | BackendError::InvalidNonce
            | BackendError::NonceAlreadyUsed
            | BackendError::RfqDisabled
            | BackendError::InvalidRfqState(_)
            | BackendError::InvalidRfqQuoteState(_)
            | BackendError::OptionsDisabled
            | BackendError::OptionRfqDisabled
            | BackendError::InvalidOptionSeriesState(_)
            | BackendError::InvalidOptionOrderState(_)
            | BackendError::InvalidOptionExecutionIntentState(_)
            | BackendError::InvalidOptionRfqState(_)
            | BackendError::InvalidOptionRfqQuoteState(_)
            | BackendError::PerpNonceSyncDisabled
            | BackendError::OptionNonceSyncDisabled
            | BackendError::PerpNonceMismatch { .. }
            | BackendError::MalformedSignature
            | BackendError::MissingTradeSignatures
            | BackendError::BroadcastRejected(_)
            | BackendError::MissingExecutionMetadata(_)
            | BackendError::InvalidPerpTradeIntentId
            | BackendError::MalformedAccountAddress
            | BackendError::UnsupportedSignatureV
            | BackendError::SignatureRecoveryFailed
            | BackendError::SignatureSignerMismatch
            | BackendError::StrictSignatureVerificationUnavailable
            | BackendError::UnknownMarket(_) => StatusCode::BAD_REQUEST,
            BackendError::ZeroPrice
            | BackendError::ZeroSize
            | BackendError::PostOnlyWouldMatch
            | BackendError::SelfTrade
            | BackendError::UnsupportedTimeInForce(_)
            | BackendError::UnsupportedCommand(_)
            | BackendError::Simulation(_)
            | BackendError::SimulationReverted(_)
            | BackendError::Indexer(_)
            | BackendError::Config(_) => StatusCode::BAD_REQUEST,
            BackendError::Persistence(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}
