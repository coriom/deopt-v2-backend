use super::AppState;
use crate::engine::{EngineCommand, EngineEvent};
use crate::error::BackendError;
use crate::types::{AccountId, MarketId, NewOrder, OrderId, Price1e8, Side, Size1e8, TimeInForce};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct SubmitOrderRequest {
    pub market_id: MarketId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub time_in_force: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    pub client_order_id: Option<String>,
}

impl From<SubmitOrderRequest> for NewOrder {
    fn from(value: SubmitOrderRequest) -> Self {
        Self {
            market_id: value.market_id,
            account: value.account,
            side: value.side,
            price_1e8: value.price_1e8,
            size_1e8: value.size_1e8,
            time_in_force: value.time_in_force,
            reduce_only: value.reduce_only,
            post_only: value.post_only,
            client_order_id: value.client_order_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubmitOrderResponse {
    pub status: String,
    pub order_id: Option<OrderId>,
    pub events: Vec<EngineEvent>,
    pub execution_intents: Vec<crate::execution::ExecutionIntent>,
}

async fn submit_order(
    State(state): State<AppState>,
    Json(request): Json<SubmitOrderRequest>,
) -> Result<Json<SubmitOrderResponse>, ApiError> {
    let mut engine = state.engine.lock().map_err(|_| ApiError::internal())?;
    let events = engine.process(EngineCommand::SubmitOrder(request.into()))?;
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
            EngineEvent::ExecutionIntentCreated { intent } => Some(intent.clone()),
            _ => None,
        })
        .collect();

    Ok(Json(SubmitOrderResponse {
        status: status.to_string(),
        order_id,
        events,
        execution_intents,
    }))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CancelOrderResponse {
    status: String,
    event: EngineEvent,
}

async fn cancel_order(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> Result<Json<CancelOrderResponse>, ApiError> {
    let order_id = OrderId::from_str(&order_id).map_err(|_| BackendError::InvalidOrderId)?;
    let mut engine = state.engine.lock().map_err(|_| ApiError::internal())?;
    let events = engine.process(EngineCommand::CancelOrder { order_id })?;
    let Some(event) = events.into_iter().next() else {
        return Err(ApiError::internal());
    };
    Ok(Json(CancelOrderResponse {
        status: "cancelled".to_string(),
        event,
    }))
}

async fn execution_intents(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::execution::ExecutionIntent>>, ApiError> {
    let engine = state.engine.lock().map_err(|_| ApiError::internal())?;
    Ok(Json(engine.execution_intents()))
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
            BackendError::ZeroPrice
            | BackendError::ZeroSize
            | BackendError::PostOnlyWouldMatch
            | BackendError::SelfTrade
            | BackendError::UnsupportedTimeInForce(_)
            | BackendError::UnsupportedCommand(_)
            | BackendError::Config(_) => StatusCode::BAD_REQUEST,
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
