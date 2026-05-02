use super::AppState;
use crate::api::dto::{
    ApiEngineEvent, ApiExecutionIntent, SubmitOrderRequest, SubmitOrderResponse,
};
use crate::db::PgRepository;
use crate::engine::{EngineCommand, EngineEvent};
use crate::error::{BackendError, Result as BackendResult};
use crate::signing::{SignatureVerifier, SignedOrder};
use crate::types::{now_ms, MarketId, NewOrder, OrderId};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Serialize;
use std::str::FromStr;
use tower_http::trace::TraceLayer;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/markets", get(markets))
        .route("/orderbook/:market_id", get(orderbook))
        .route("/orders", post(submit_order))
        .route("/orders/:order_id", delete(cancel_order))
        .route("/execution-intents", get(execution_intents))
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
            BackendError::OrderNotFound(_) | BackendError::OrderNotOpen(_) => StatusCode::NOT_FOUND,
            BackendError::InvalidFixedPoint { .. } => StatusCode::BAD_REQUEST,
            BackendError::DeadlineExpired
            | BackendError::InvalidNonce
            | BackendError::NonceAlreadyUsed
            | BackendError::MalformedSignature
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
