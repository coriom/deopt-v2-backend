use super::AppState;
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
use crate::indexer::{Indexer, IndexerStatus, IndexerTickResult};
use crate::nonce_sync::{read_perp_nonce, PerpNonceResponse};
use crate::options::service::{
    accept_option_rfq_quote as accept_option_rfq_quote_service,
    cancel_option_order as cancel_option_order_service,
    cancel_option_rfq as cancel_option_rfq_service, create_option_rfq as create_option_rfq_service,
    create_option_series as create_option_series_service,
    disable_option_series as disable_option_series_service,
    get_option_fill as get_option_fill_service, get_option_order as get_option_order_service,
    get_option_order_fills as get_option_order_fills_service,
    get_option_orderbook as get_option_orderbook_service, get_option_rfq as get_option_rfq_service,
    get_option_series as get_option_series_service, list_option_fills as list_option_fills_service,
    list_option_orders as list_option_orders_service,
    list_option_rfq_quotes as list_option_rfq_quotes_service,
    list_option_rfqs as list_option_rfqs_service, list_option_series as list_option_series_service,
    submit_option_order as submit_option_order_service,
    submit_option_rfq_quote as submit_option_rfq_quote_service, CreateOptionRfqInput,
    CreateOptionSeriesInput, SubmitOptionOrderInput, SubmitOptionRfqQuoteInput,
};
use crate::options::{
    OptionFill, OptionFillFilter, OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderStatus,
    OptionOrderbookSnapshot, OptionRfqFill, OptionRfqId, OptionRfqQuote, OptionRfqQuoteId,
    OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus, OptionSeries, OptionSeriesFilter,
    OptionSeriesStatus,
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
    quote_ttl_ms: Option<u64>,
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
        }
    }
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
            quote_ttl_ms: request.quote_ttl_ms,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::DecodedRevertError;

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
            BackendError::InvalidOptionRfqId | BackendError::InvalidOptionRfqQuoteId => {
                StatusCode::NOT_FOUND
            }
            BackendError::OrderNotFound(_) | BackendError::OrderNotOpen(_) => StatusCode::NOT_FOUND,
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
            | BackendError::InvalidOptionRfqState(_)
            | BackendError::InvalidOptionRfqQuoteState(_)
            | BackendError::PerpNonceSyncDisabled
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
