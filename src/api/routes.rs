use super::AppState;
use crate::api::dto::{
    ApiEngineEvent, ApiExecutionIntent, SubmitOrderRequest, SubmitOrderResponse,
};
use crate::db::PgRepository;
use crate::engine::{EngineCommand, EngineEvent};
use crate::error::{BackendError, Result as BackendResult};
use crate::execution::{
    perp_trade_digest, simulate_execution_intent, ExecutionIntentStatus, Executor,
    HttpJsonRpcProvider, PerpTradeDomain, PerpTradePayload, SimulationResult,
    StoredTradeSignatures, TradeSignatureStatus, PERP_TRADE_TYPE,
};
use crate::signing::{SignatureVerifier, SignedOrder};
use crate::types::{now_ms, MarketId, NewOrder, OrderId};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/markets", get(markets))
        .route("/orderbook/:market_id", get(orderbook))
        .route("/orders", post(submit_order))
        .route("/orders/:order_id", delete(cancel_order))
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
        .route("/executor/tick", post(executor_tick))
        .route(
            "/executor/simulate/:intent_id",
            post(simulate_executor_intent),
        )
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

async fn submit_order(
    State(state): State<AppState>,
    Json(request): Json<SubmitOrderRequest>,
) -> Result<Json<SubmitOrderResponse>, ApiError> {
    let signed_order = request.into_signed_order()?;
    validate_deadline(signed_order.deadline_ms)?;
    SignatureVerifier::verify(
        &signed_order,
        state.signature_verification_mode,
        &state.eip712_domain,
    )?;

    {
        let engine = state.engine.lock().map_err(|_| ApiError::internal())?;
        if !engine.has_market(signed_order.market_id) {
            return Err(BackendError::UnknownMarket(signed_order.market_id).into());
        }
    }

    if let Some(repository) = state.repository.clone() {
        return submit_order_persistent(state, repository, signed_order).await;
    }

    {
        let mut nonces = state.nonces.lock().map_err(|_| ApiError::internal())?;
        nonces.reserve(&signed_order.account, signed_order.nonce)?;
    }

    let mut engine = state.engine.lock().map_err(|_| ApiError::internal())?;
    let events = engine.process(EngineCommand::SubmitOrder(NewOrder::from(signed_order)))?;
    let status = if events
        .iter()
        .any(|event| matches!(event, EngineEvent::OrderRejected { .. }))
    {
        "rejected"
    } else {
        "accepted"
    };
    let order_id = first_order_id(&events);
    let execution_intents = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::ExecutionIntentCreated { intent } => {
                Some(ApiExecutionIntent::from(intent.clone()))
            }
            _ => None,
        })
        .collect();
    let events = events.into_iter().map(ApiEngineEvent::from).collect();

    Ok(Json(SubmitOrderResponse {
        status: status.to_string(),
        order_id,
        events,
        execution_intents,
    }))
}

async fn submit_order_persistent(
    state: AppState,
    repository: PgRepository,
    signed_order: SignedOrder,
) -> Result<Json<SubmitOrderResponse>, ApiError> {
    let mut tx = repository.begin().await?;
    repository
        .insert_nonce_tx(&mut tx, &signed_order.account, signed_order.nonce, now_ms())
        .await?;

    let events = {
        let mut engine = state.engine.lock().map_err(|_| ApiError::internal())?;
        engine.process(EngineCommand::SubmitOrder(NewOrder::from(
            signed_order.clone(),
        )))?
    };

    repository
        .persist_submission_tx(&mut tx, &signed_order, &events)
        .await?;
    tx.commit()
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

    Ok(Json(response_from_events(events)))
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
    let events = {
        let mut engine = state.engine.lock().map_err(|_| ApiError::internal())?;
        engine.process(EngineCommand::CancelOrder { order_id })?
    };
    if let Some(repository) = state.repository.clone() {
        repository.persist_engine_events(&events).await?;
    }
    let Some(event) = events.into_iter().next() else {
        return Err(ApiError::internal());
    };
    Ok(Json(CancelOrderResponse {
        status: "cancelled".to_string(),
        event: event.into(),
    }))
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
    submitted: bool,
    confirmed: bool,
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

    Ok(Json(SimulationResponse {
        intent_id,
        simulation_status: result.status,
        block_number: result.block_number,
        error: result.error,
        submitted: false,
        confirmed: false,
    }))
}

fn response_from_events(events: Vec<EngineEvent>) -> SubmitOrderResponse {
    let status = if events
        .iter()
        .any(|event| matches!(event, EngineEvent::OrderRejected { .. }))
    {
        "rejected"
    } else {
        "accepted"
    };
    let order_id = first_order_id(&events);
    let execution_intents = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::ExecutionIntentCreated { intent } => {
                Some(ApiExecutionIntent::from(intent.clone()))
            }
            _ => None,
        })
        .collect();
    let events = events.into_iter().map(ApiEngineEvent::from).collect();

    SubmitOrderResponse {
        status: status.to_string(),
        order_id,
        events,
        execution_intents,
    }
}

fn first_order_id(events: &[EngineEvent]) -> Option<OrderId> {
    events.iter().find_map(|event| match event {
        EngineEvent::OrderAccepted { order } => Some(order.order_id),
        EngineEvent::OrderRejected { order_id, .. } => Some(*order_id),
        EngineEvent::OrderCancelled { order } => Some(order.order_id),
        EngineEvent::OrderPartiallyFilled { order } => Some(order.order_id),
        EngineEvent::OrderFilled { order } => Some(order.order_id),
        EngineEvent::TradeMatched { .. } | EngineEvent::ExecutionIntentCreated { .. } => None,
    })
}

fn validate_deadline(deadline_ms: i64) -> BackendResult<()> {
    if deadline_ms <= now_ms() {
        return Err(BackendError::DeadlineExpired);
    }
    Ok(())
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

fn perp_trade_type_fields() -> Vec<SigningPayloadTypeField> {
    let _ = PERP_TRADE_TYPE;
    vec![
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
}

impl From<BackendError> for ApiError {
    fn from(value: BackendError) -> Self {
        let status = match value {
            BackendError::InvalidOrderId => StatusCode::BAD_REQUEST,
            BackendError::InvalidExecutionIntentId => StatusCode::NOT_FOUND,
            BackendError::OrderNotFound(_) | BackendError::OrderNotOpen(_) => StatusCode::NOT_FOUND,
            BackendError::InvalidFixedPoint { .. } => StatusCode::BAD_REQUEST,
            BackendError::DeadlineExpired
            | BackendError::InvalidNonce
            | BackendError::NonceAlreadyUsed
            | BackendError::MalformedSignature
            | BackendError::MissingTradeSignatures
            | BackendError::MissingExecutionMetadata(_)
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
