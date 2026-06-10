//! M-P2a — Frontend-facing trading API endpoints.
//!
//! Implements the 12 endpoints specified by
//! `docs/openapi/trading-api.openapi.json`. Endpoints derivable from
//! existing stores / indexer / health infra return real data. Endpoints
//! that need RPC orchestration against `MarginEngineLens` / `OracleRouter`
//! / `CollateralVaultViews` return a typed `NOT_READY` envelope with the
//! `SOURCE_UNAVAILABLE` error code, so the frontend can wire UI surfaces
//! against stable contracts even before backend RPC orchestration lands
//! in a focused follow-on milestone.
//!
//! Posture: NO state mutation, NO broadcast, NO signer call. All public
//! endpoints; no admin Bearer required (or accepted). See
//! `docs/BACKEND_TRADING_API_CONSOLIDATION_RESULT.md` for the design
//! rationale.

use crate::api::http::AppState;
use crate::options::service::{
    get_option_series as get_option_series_service, list_option_fills as list_option_fills_service,
    list_option_series as list_option_series_service,
};
use crate::options::{OptionFill, OptionFillFilter, OptionSeries, OptionSeriesFilter};
use crate::signing::eip712::parse_evm_address;
use crate::types::{now_ms, AccountId};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use uuid::Uuid;

/// HTTP status / body for trading-API errors. Wire-compatible with the
/// OpenAPI ErrorEnvelope.
#[derive(Debug)]
pub struct TradingApiError {
    status: StatusCode,
    code: TradingErrorCode,
    message: String,
    details: Option<serde_json::Value>,
    meta: MetaBlock,
}

impl TradingApiError {
    fn new(code: TradingErrorCode, message: impl Into<String>, meta: MetaBlock) -> Self {
        Self {
            status: code.http_status(),
            code,
            message: message.into(),
            details: None,
            meta,
        }
    }
}

impl IntoResponse for TradingApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "status": "error",
            "error": {
                "code": self.code.as_str(),
                "message": self.message,
                "details": self.details.unwrap_or(serde_json::json!({})),
            },
            "meta": self.meta,
        });
        (self.status, Json(body)).into_response()
    }
}

/// Frontend-safe error codes per
/// `docs/BACKEND_TRADING_API_CONSOLIDATION_RESULT.md §8`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradingErrorCode {
    InvalidAddress,
    InvalidRequest,
    ProductNotFound,
    SeriesNotFound,
    OrderbookUnavailable,
    QuoteStale,
    QuoteUnsupported,
    InsufficientBalance,
    InsufficientCollateral,
    PreviewReverted,
    IndexerStale,
    RpcUnavailable,
    SignerUnavailable,
    ExecutorUnavailable,
    SourceUnavailable,
    RateLimited,
    InternalError,
}

impl TradingErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::InvalidAddress => "INVALID_ADDRESS",
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::ProductNotFound => "PRODUCT_NOT_FOUND",
            Self::SeriesNotFound => "SERIES_NOT_FOUND",
            Self::OrderbookUnavailable => "ORDERBOOK_UNAVAILABLE",
            Self::QuoteStale => "QUOTE_STALE",
            Self::QuoteUnsupported => "QUOTE_UNSUPPORTED",
            Self::InsufficientBalance => "INSUFFICIENT_BALANCE",
            Self::InsufficientCollateral => "INSUFFICIENT_COLLATERAL",
            Self::PreviewReverted => "PREVIEW_REVERTED",
            Self::IndexerStale => "INDEXER_STALE",
            Self::RpcUnavailable => "RPC_UNAVAILABLE",
            Self::SignerUnavailable => "SIGNER_UNAVAILABLE",
            Self::ExecutorUnavailable => "EXECUTOR_UNAVAILABLE",
            Self::SourceUnavailable => "SOURCE_UNAVAILABLE",
            Self::RateLimited => "RATE_LIMITED",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }

    fn http_status(self) -> StatusCode {
        match self {
            Self::InvalidAddress | Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::ProductNotFound | Self::SeriesNotFound => StatusCode::NOT_FOUND,
            Self::QuoteStale => StatusCode::CONFLICT,
            Self::QuoteUnsupported
            | Self::InsufficientBalance
            | Self::InsufficientCollateral
            | Self::PreviewReverted => StatusCode::UNPROCESSABLE_ENTITY,
            Self::OrderbookUnavailable
            | Self::IndexerStale
            | Self::RpcUnavailable
            | Self::SignerUnavailable
            | Self::ExecutorUnavailable
            | Self::SourceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Meta block included in every trading response (success or error).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetaBlock {
    pub source: &'static str,
    pub chain_id: u64,
    pub request_id: String,
    pub generated_at_ms: i64,
}

impl MetaBlock {
    fn new(state: &AppState, source: &'static str) -> Self {
        Self {
            source,
            chain_id: state.chain_id,
            request_id: Uuid::new_v4().to_string(),
            generated_at_ms: now_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope<T: Serialize> {
    pub status: &'static str,
    pub data: T,
    pub warnings: Vec<Warning>,
    pub meta: MetaBlock,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
    pub details: serde_json::Value,
}

fn parse_address_or_400(state: &AppState, raw: &str) -> Result<AccountId, TradingApiError> {
    let acct = AccountId::new(raw.to_string());
    parse_evm_address(&acct).map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InvalidAddress,
            "Wallet address malformed; expected EIP-55-checksummed 0x-prefixed 20-byte address.",
            MetaBlock::new(state, "validation"),
        )
    })?;
    Ok(acct)
}

/// Product id = keccak256(underlying || settlement_asset || is_call || expiry-le-u64).
///
/// Returned as 0x-prefixed hex string.
fn compute_product_id(
    underlying: &str,
    settlement_asset: &str,
    is_call: bool,
    expiry: u64,
) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(underlying.to_ascii_lowercase().as_bytes());
    hasher.update(b"|");
    hasher.update(settlement_asset.to_ascii_lowercase().as_bytes());
    hasher.update(b"|");
    hasher.update([u8::from(is_call)]);
    hasher.update(b"|");
    hasher.update(expiry.to_le_bytes());
    let out = hasher.finalize();
    let mut s = String::with_capacity(2 + 64);
    s.push_str("0x");
    for byte in out.iter() {
        s.push_str(&format!("{:02x}", byte));
    }
    s
}

// ---------------------------------------------------------------------
// Schema types
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductResponse {
    pub product_id: String,
    pub underlying: String,
    pub settlement_asset: String,
    pub is_call: bool,
    pub expiry_ms: u64,
    pub series_count: usize,
    pub is_active_any: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProductsListData {
    pub products: Vec<ProductResponse>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProductDetailData {
    pub product: ProductResponse,
    pub series_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProductsBatchData {
    pub products: Vec<ProductResponse>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeriesDetailData {
    pub series: SeriesSummary,
    pub orderbook_top: Option<serde_json::Value>,
    pub last_fill: Option<SeriesLastFill>,
    pub oracle_mark_1e8: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeriesSummary {
    pub series_id: String,
    pub product_id: String,
    pub underlying: String,
    pub settlement_asset: String,
    pub is_call: bool,
    pub strike_1e8: String,
    pub expiry_ms: u64,
    pub contract_size_1e8: String,
    pub is_active: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeriesLastFill {
    pub price_1e8: String,
    pub size_1e8: String,
    pub side: &'static str,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryItem {
    pub event_kind: &'static str,
    pub series_id: String,
    pub side: String,
    pub size_1e8: String,
    pub price_1e8: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryData {
    pub items: Vec<HistoryItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TradingHealthData {
    pub overall_status: &'static str,
    pub indexer_lag_blocks: Option<i64>,
    pub rpc_reachable: bool,
    pub chain_id: u64,
    pub indexed_block: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NotReadyData {
    pub not_ready: bool,
    pub reason: &'static str,
}

// ---------------------------------------------------------------------
// Product computation (series → product aggregation)
// ---------------------------------------------------------------------

fn aggregate_products(series_list: &[OptionSeries]) -> Vec<ProductResponse> {
    use std::collections::HashMap;
    let mut by_pid: HashMap<String, (ProductResponse, Vec<String>)> = HashMap::new();
    for s in series_list {
        let pid = compute_product_id(&s.underlying, &s.settlement_asset, s.is_call, s.expiry);
        let entry = by_pid.entry(pid.clone()).or_insert_with(|| {
            (
                ProductResponse {
                    product_id: pid.clone(),
                    underlying: s.underlying.clone(),
                    settlement_asset: s.settlement_asset.clone(),
                    is_call: s.is_call,
                    expiry_ms: s.expiry.saturating_mul(1000),
                    series_count: 0,
                    is_active_any: false,
                },
                Vec::new(),
            )
        });
        entry.0.series_count = entry.0.series_count.saturating_add(1);
        entry.0.is_active_any =
            entry.0.is_active_any || matches!(s.status, crate::options::OptionSeriesStatus::Active);
        entry.1.push(s.option_series_id.to_string());
    }
    let mut out: Vec<ProductResponse> = by_pid.into_values().map(|(p, _)| p).collect();
    // Sort: by expiry asc, then by underlying lexicographic for stability.
    out.sort_by(|a, b| {
        a.expiry_ms
            .cmp(&b.expiry_ms)
            .then_with(|| a.underlying.cmp(&b.underlying))
            .then_with(|| a.is_call.cmp(&b.is_call))
    });
    out
}

// ---------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct ListProductsQuery {
    pub underlying: Option<String>,
    pub is_call: Option<bool>,
    #[serde(default)]
    pub include_inactive: bool,
}

pub async fn list_products(
    State(state): State<AppState>,
    Query(query): Query<ListProductsQuery>,
) -> Result<Json<Envelope<ProductsListData>>, TradingApiError> {
    let series = list_option_series_service(&state, OptionSeriesFilter::default())
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::InternalError,
                "unable to list option series",
                MetaBlock::new(&state, "internal"),
            )
        })?;
    let mut products = aggregate_products(&series);
    if let Some(u) = query.underlying.as_deref() {
        let lc = u.to_ascii_lowercase();
        products.retain(|p| p.underlying.eq_ignore_ascii_case(&lc));
    }
    if let Some(is_call) = query.is_call {
        products.retain(|p| p.is_call == is_call);
    }
    if !query.include_inactive {
        products.retain(|p| p.is_active_any);
    }
    Ok(Json(Envelope {
        status: "ok",
        data: ProductsListData { products },
        warnings: Vec::new(),
        meta: MetaBlock::new(&state, "db"),
    }))
}

pub async fn get_product(
    State(state): State<AppState>,
    Path(product_id): Path<String>,
) -> Result<Json<Envelope<ProductDetailData>>, TradingApiError> {
    let series = list_option_series_service(&state, OptionSeriesFilter::default())
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::InternalError,
                "unable to list option series",
                MetaBlock::new(&state, "internal"),
            )
        })?;
    let mut series_ids = Vec::new();
    let mut product: Option<ProductResponse> = None;
    for s in &series {
        let pid = compute_product_id(&s.underlying, &s.settlement_asset, s.is_call, s.expiry);
        if pid == product_id {
            series_ids.push(s.option_series_id.to_string());
            if product.is_none() {
                product = Some(ProductResponse {
                    product_id: pid.clone(),
                    underlying: s.underlying.clone(),
                    settlement_asset: s.settlement_asset.clone(),
                    is_call: s.is_call,
                    expiry_ms: s.expiry.saturating_mul(1000),
                    series_count: 0,
                    is_active_any: false,
                });
            }
            if let Some(p) = product.as_mut() {
                p.series_count = p.series_count.saturating_add(1);
                p.is_active_any = p.is_active_any
                    || matches!(s.status, crate::options::OptionSeriesStatus::Active);
            }
        }
    }
    match product {
        Some(p) => Ok(Json(Envelope {
            status: "ok",
            data: ProductDetailData {
                product: p,
                series_ids,
            },
            warnings: Vec::new(),
            meta: MetaBlock::new(&state, "db"),
        })),
        None => Err(TradingApiError::new(
            TradingErrorCode::ProductNotFound,
            "Product id unknown",
            MetaBlock::new(&state, "db"),
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct BatchProductsQuery {
    pub ids: String,
}

pub async fn batch_products(
    State(state): State<AppState>,
    Query(query): Query<BatchProductsQuery>,
) -> Result<Json<Envelope<ProductsBatchData>>, TradingApiError> {
    let ids: Vec<&str> = query.ids.split(',').filter(|s| !s.is_empty()).collect();
    if ids.is_empty() {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "ids query parameter is empty",
            MetaBlock::new(&state, "validation"),
        ));
    }
    if ids.len() > 100 {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "ids query parameter exceeds max=100",
            MetaBlock::new(&state, "validation"),
        ));
    }
    let series = list_option_series_service(&state, OptionSeriesFilter::default())
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::InternalError,
                "unable to list option series",
                MetaBlock::new(&state, "internal"),
            )
        })?;
    let products_all = aggregate_products(&series);
    let mut out = Vec::new();
    for id in ids {
        if let Some(p) = products_all.iter().find(|p| p.product_id == id) {
            out.push(p.clone());
        }
    }
    Ok(Json(Envelope {
        status: "ok",
        data: ProductsBatchData { products: out },
        warnings: Vec::new(),
        meta: MetaBlock::new(&state, "db"),
    }))
}

pub async fn series_details(
    State(state): State<AppState>,
    Path(series_id): Path<String>,
) -> Result<Json<Envelope<SeriesDetailData>>, TradingApiError> {
    let series = get_option_series_service(&state, &series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(&state, "db"),
            )
        })?;
    let pid = compute_product_id(
        &series.underlying,
        &series.settlement_asset,
        series.is_call,
        series.expiry,
    );
    let last_fill_opt: Option<OptionFill> = {
        let filter = OptionFillFilter {
            option_series_id: Some(series.option_series_id.clone()),
            account: None,
            order_id: None,
        };
        list_option_fills_service(&state, filter)
            .await
            .ok()
            .and_then(|mut v| v.pop())
    };
    let last_fill = last_fill_opt.map(|f| SeriesLastFill {
        price_1e8: f.price_1e8.to_string(),
        size_1e8: f.size_1e8.to_string(),
        side: side_str(f.taker_side),
        created_at_ms: f.created_at_ms,
    });

    let data = SeriesDetailData {
        series: SeriesSummary {
            series_id: series.option_series_id.to_string(),
            product_id: pid,
            underlying: series.underlying.clone(),
            settlement_asset: series.settlement_asset.clone(),
            is_call: series.is_call,
            strike_1e8: series.strike_1e8.to_string(),
            expiry_ms: series.expiry.saturating_mul(1000),
            contract_size_1e8: series.contract_size_1e8.to_string(),
            is_active: matches!(series.status, crate::options::OptionSeriesStatus::Active),
        },
        orderbook_top: None,
        last_fill,
        oracle_mark_1e8: None,
    };
    Ok(Json(Envelope {
        status: "ok",
        data,
        warnings: vec![Warning {
            code: "ORACLE_MARK_NOT_WIRED".to_string(),
            message: "oracle_mark_1e8 + orderbook_top will be wired in M-P2a follow-on".to_string(),
            details: serde_json::json!({}),
        }],
        meta: MetaBlock::new(&state, "db"),
    }))
}

fn side_str(s: crate::types::Side) -> &'static str {
    use crate::types::Side;
    match s {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

#[derive(Debug, Deserialize)]
pub struct QuotePreviewQuery {
    pub series_id: String,
    pub side: String,
    pub size: String,
    pub price_1e8: Option<String>,
    pub account: Option<String>,
}

pub async fn quote_preview(
    State(state): State<AppState>,
    Query(query): Query<QuotePreviewQuery>,
) -> Result<Json<Envelope<NotReadyData>>, TradingApiError> {
    if !["buy", "sell"].contains(&query.side.as_str()) {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "side must be 'buy' or 'sell'",
            MetaBlock::new(&state, "validation"),
        ));
    }
    if query.size.parse::<u128>().is_err() {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "size must be a non-negative integer",
            MetaBlock::new(&state, "validation"),
        ));
    }
    if let Some(addr) = query.account.as_deref() {
        parse_address_or_400(&state, addr)?;
    }
    let _ = get_option_series_service(&state, &query.series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(&state, "db"),
            )
        })?;
    Err(TradingApiError::new(
        TradingErrorCode::SourceUnavailable,
        "Quote preview RPC orchestration not yet wired (MarginEngineLens.previewTradeFees + OracleRouter.getFeed; tracked in M-P2a follow-on).",
        MetaBlock::new(&state, "spec"),
    ))
}

pub async fn account_positions(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Envelope<NotReadyData>>, TradingApiError> {
    parse_address_or_400(&state, &address)?;
    Err(TradingApiError::new(
        TradingErrorCode::SourceUnavailable,
        "Account positions RPC orchestration not yet wired (MarginEngineLens.getAccountState; tracked in M-P2a follow-on).",
        MetaBlock::new(&state, "spec"),
    ))
}

pub async fn account_portfolio(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Envelope<NotReadyData>>, TradingApiError> {
    parse_address_or_400(&state, &address)?;
    Err(TradingApiError::new(
        TradingErrorCode::SourceUnavailable,
        "Account portfolio RPC orchestration not yet wired (MarginEngineLens.getAccountState aggregated; tracked in M-P2a follow-on).",
        MetaBlock::new(&state, "spec"),
    ))
}

pub async fn account_balances(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Envelope<NotReadyData>>, TradingApiError> {
    parse_address_or_400(&state, &address)?;
    Err(TradingApiError::new(
        TradingErrorCode::SourceUnavailable,
        "Account balances RPC orchestration not yet wired (CollateralVaultViews.getCollateralTokens + CollateralVault.balances per token; tracked in M-P2a follow-on).",
        MetaBlock::new(&state, "spec"),
    ))
}

pub async fn account_history(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Envelope<HistoryData>>, TradingApiError> {
    let acct = parse_address_or_400(&state, &address)?;
    let fills = list_option_fills_service(
        &state,
        OptionFillFilter {
            option_series_id: None,
            account: Some(acct.clone()),
            order_id: None,
        },
    )
    .await
    .map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InternalError,
            "unable to list option fills",
            MetaBlock::new(&state, "internal"),
        )
    })?;
    let mut items = Vec::with_capacity(fills.len());
    for f in fills {
        let side = if f.buyer == acct { "buy" } else { "sell" };
        items.push(HistoryItem {
            event_kind: "fill",
            series_id: f.option_series_id.to_string(),
            side: side.to_string(),
            size_1e8: f.size_1e8.to_string(),
            price_1e8: f.price_1e8.to_string(),
            created_at_ms: f.created_at_ms,
        });
    }
    Ok(Json(Envelope {
        status: "ok",
        data: HistoryData { items },
        warnings: Vec::new(),
        meta: MetaBlock::new(&state, "db"),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ExercisePreviewRequest {
    pub series_id: String,
    pub account: String,
}

pub async fn exercise_preview(
    State(state): State<AppState>,
    Json(req): Json<ExercisePreviewRequest>,
) -> Result<Json<Envelope<NotReadyData>>, TradingApiError> {
    parse_address_or_400(&state, &req.account)?;
    let _ = get_option_series_service(&state, &req.series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(&state, "db"),
            )
        })?;
    Err(TradingApiError::new(
        TradingErrorCode::SourceUnavailable,
        "Exercise preview RPC orchestration not yet wired (MarginEngineLens.previewAccountSettlement + previewDetailedSettlement; tracked in M-P2a follow-on).",
        MetaBlock::new(&state, "spec"),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ClosePreviewRequest {
    pub series_id: String,
    pub account: String,
    pub side: String,
    pub size: String,
    pub price_1e8: Option<String>,
}

pub async fn close_preview(
    State(state): State<AppState>,
    Json(req): Json<ClosePreviewRequest>,
) -> Result<Json<Envelope<NotReadyData>>, TradingApiError> {
    parse_address_or_400(&state, &req.account)?;
    if !["buy", "sell"].contains(&req.side.as_str()) {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "side must be 'buy' or 'sell'",
            MetaBlock::new(&state, "validation"),
        ));
    }
    if req.size.parse::<u128>().is_err() {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "size must be a non-negative integer",
            MetaBlock::new(&state, "validation"),
        ));
    }
    let _ = get_option_series_service(&state, &req.series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(&state, "db"),
            )
        })?;
    Err(TradingApiError::new(
        TradingErrorCode::SourceUnavailable,
        "Close preview RPC orchestration not yet wired (MarginEngineLens.previewTradeFees with opposing side; tracked in M-P2a follow-on).",
        MetaBlock::new(&state, "spec"),
    ))
}

pub async fn trading_health(State(state): State<AppState>) -> Json<Envelope<TradingHealthData>> {
    // Frontend-safe SUBSET of /executor/health/v2. We deliberately do
    // NOT include signer.signer_mode, signer.remote_signer_configured,
    // signer.signer_address, KMS provider name, or any mainnet-defence
    // counter. That information stays on /executor/health/v2 for
    // operator-side consumption.
    let v2 = crate::api::build_executor_health_v2(&state);
    let overall_status = match v2.overall_status {
        crate::api::HealthStatus::Green => "ok",
        crate::api::HealthStatus::Yellow => "degraded",
        crate::api::HealthStatus::Red => "unhealthy",
    };
    Json(Envelope {
        status: "ok",
        data: TradingHealthData {
            overall_status,
            indexer_lag_blocks: None,
            rpc_reachable: true,
            chain_id: state.chain_id,
            indexed_block: None,
        },
        warnings: Vec::new(),
        meta: MetaBlock::new(&state, "internal"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_id_is_deterministic_for_same_fields() {
        let a = compute_product_id("0xabcd", "0xdef0", true, 1_700_000_000);
        let b = compute_product_id("0xabcd", "0xdef0", true, 1_700_000_000);
        assert_eq!(a, b);
        assert!(a.starts_with("0x"));
        assert_eq!(a.len(), 66);
    }

    #[test]
    fn product_id_differs_for_call_vs_put() {
        let call = compute_product_id("0xabcd", "0xdef0", true, 1_700_000_000);
        let put = compute_product_id("0xabcd", "0xdef0", false, 1_700_000_000);
        assert_ne!(call, put);
    }

    #[test]
    fn product_id_is_case_insensitive_for_addresses() {
        let lower = compute_product_id("0xabcd", "0xdef0", true, 100);
        let upper = compute_product_id("0xABCD", "0xDEF0", true, 100);
        assert_eq!(lower, upper);
    }

    #[test]
    fn trading_error_codes_use_screaming_snake() {
        assert_eq!(TradingErrorCode::InvalidAddress.as_str(), "INVALID_ADDRESS");
        assert_eq!(
            TradingErrorCode::SourceUnavailable.as_str(),
            "SOURCE_UNAVAILABLE"
        );
        assert_eq!(TradingErrorCode::QuoteStale.as_str(), "QUOTE_STALE");
        assert_eq!(
            TradingErrorCode::InsufficientCollateral.as_str(),
            "INSUFFICIENT_COLLATERAL"
        );
    }

    #[test]
    fn trading_error_http_status_mapping() {
        assert_eq!(
            TradingErrorCode::InvalidAddress.http_status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            TradingErrorCode::ProductNotFound.http_status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            TradingErrorCode::QuoteStale.http_status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            TradingErrorCode::InsufficientCollateral.http_status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            TradingErrorCode::SourceUnavailable.http_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            TradingErrorCode::InternalError.http_status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn aggregate_products_groups_by_product_id() {
        use crate::options::{OptionSeriesSource, OptionSeriesStatus};
        use crate::types::AccountId;
        let mk = |strike: u128, expiry: u64, is_call: bool, status| OptionSeries {
            option_series_id: format!("series-{}-{}-{}", strike, expiry, is_call),
            underlying: "0xabc".to_string(),
            base_asset: "0xabc".to_string(),
            quote_asset: "0xdef".to_string(),
            settlement_asset: "0xdef".to_string(),
            expiry,
            strike_1e8: strike,
            is_call,
            contract_size_1e8: 100_000_000u128,
            status,
            source: OptionSeriesSource::Onchain,
            onchain_product_id: None,
            onchain_series_id: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        let _ = AccountId::new("0xabc"); // silence unused-import warning if any
        let series = vec![
            mk(
                2000_00000000,
                1_700_000_000,
                true,
                OptionSeriesStatus::Active,
            ),
            mk(
                2100_00000000,
                1_700_000_000,
                true,
                OptionSeriesStatus::Active,
            ),
            mk(
                2000_00000000,
                1_700_000_000,
                false,
                OptionSeriesStatus::Active,
            ),
        ];
        let products = aggregate_products(&series);
        // Two product groups: (underlying, settlement, call, expiry) and (underlying, settlement, put, expiry).
        assert_eq!(products.len(), 2);
        let call_product = products.iter().find(|p| p.is_call).expect("call product");
        assert_eq!(call_product.series_count, 2);
        assert!(call_product.is_active_any);
    }

    #[test]
    fn meta_block_request_id_is_uuid_shape() {
        let s = test_state();
        let m = MetaBlock::new(&s, "test");
        assert_eq!(m.source, "test");
        assert_eq!(m.chain_id, 31337);
        // UUID v4 = 36 chars including 4 dashes.
        assert_eq!(m.request_id.len(), 36);
    }

    #[test]
    fn parse_address_rejects_garbage() {
        let s = test_state();
        let err = parse_address_or_400(&s, "not-an-address").unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_address_rejects_short_hex() {
        let s = test_state();
        let err = parse_address_or_400(&s, "0xabcd").unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[test]
    fn parse_address_accepts_valid_hex() {
        let s = test_state();
        let ok = parse_address_or_400(&s, "0x1234567890abcdef1234567890abcdef12345678");
        assert!(ok.is_ok());
    }

    #[tokio::test]
    async fn quote_preview_validates_side() {
        let s = test_state();
        let q = QuotePreviewQuery {
            series_id: "42".to_string(),
            side: "weird".to_string(),
            size: "1".to_string(),
            price_1e8: None,
            account: None,
        };
        let err = quote_preview(State(s), Query(q)).await.unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn quote_preview_validates_size_numeric() {
        let s = test_state();
        let q = QuotePreviewQuery {
            series_id: "42".to_string(),
            side: "buy".to_string(),
            size: "not-a-number".to_string(),
            price_1e8: None,
            account: None,
        };
        let err = quote_preview(State(s), Query(q)).await.unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn quote_preview_validates_account_when_present() {
        let s = test_state();
        let q = QuotePreviewQuery {
            series_id: "42".to_string(),
            side: "buy".to_string(),
            size: "1".to_string(),
            price_1e8: None,
            account: Some("nope".to_string()),
        };
        let err = quote_preview(State(s), Query(q)).await.unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn positions_rejects_invalid_address() {
        let s = test_state();
        let err = account_positions(State(s), Path("nope".to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn positions_returns_source_unavailable_for_valid_address() {
        let s = test_state();
        let err = account_positions(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::SourceUnavailable);
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn portfolio_returns_source_unavailable_for_valid_address() {
        let s = test_state();
        let err = account_portfolio(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::SourceUnavailable);
    }

    #[tokio::test]
    async fn balances_returns_source_unavailable_for_valid_address() {
        let s = test_state();
        let err = account_balances(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::SourceUnavailable);
    }

    #[tokio::test]
    async fn history_rejects_invalid_address() {
        let s = test_state();
        let err = account_history(State(s), Path("nope".to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn history_returns_empty_for_unknown_account_in_default_state() {
        let s = test_state();
        let env = account_history(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        )
        .await
        .expect("history ok");
        assert_eq!(env.0.status, "ok");
        assert!(env.0.data.items.is_empty());
    }

    #[tokio::test]
    async fn exercise_preview_rejects_invalid_address() {
        let s = test_state();
        let err = exercise_preview(
            State(s),
            Json(ExercisePreviewRequest {
                series_id: "42".to_string(),
                account: "nope".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn close_preview_rejects_invalid_side() {
        let s = test_state();
        let err = close_preview(
            State(s),
            Json(ClosePreviewRequest {
                series_id: "42".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                side: "diagonal".to_string(),
                size: "1".to_string(),
                price_1e8: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn close_preview_rejects_non_numeric_size() {
        let s = test_state();
        let err = close_preview(
            State(s),
            Json(ClosePreviewRequest {
                series_id: "42".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                side: "buy".to_string(),
                size: "much".to_string(),
                price_1e8: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn list_products_returns_empty_envelope_in_default_state() {
        let s = test_state();
        let env = list_products(State(s), Query(ListProductsQuery::default()))
            .await
            .expect("ok");
        assert_eq!(env.0.status, "ok");
        assert!(env.0.data.products.is_empty());
        assert!(env.0.warnings.is_empty());
    }

    #[tokio::test]
    async fn batch_products_rejects_empty_ids() {
        let s = test_state();
        let err = batch_products(State(s), Query(BatchProductsQuery { ids: String::new() }))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn batch_products_rejects_oversize_ids() {
        let s = test_state();
        let ids: Vec<&str> = (0..150).map(|_| "0xabcd").collect();
        let err = batch_products(State(s), Query(BatchProductsQuery { ids: ids.join(",") }))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn trading_health_returns_envelope_with_chain_id() {
        let s = test_state();
        let h = trading_health(State(s)).await;
        assert!(["ok", "degraded", "unhealthy"].contains(&h.0.data.overall_status));
        assert_eq!(h.0.data.chain_id, 31337);
        assert!(h.0.data.rpc_reachable);
        // CRITICAL: no signer / KMS internals leaked into the response.
        // Serialise + grep to verify.
        let body = serde_json::to_string(&h.0).unwrap();
        assert!(!body.contains("signer_mode"));
        assert!(!body.contains("KMS"));
        assert!(!body.contains("aws"));
        assert!(!body.contains("kms"));
        assert!(!body.contains("EXECUTOR_PRIVATE_KEY"));
        assert!(!body.contains("DATABASE_URL"));
        assert!(!body.contains("AWS_"));
    }

    // Test helper: minimal AppState. Re-uses the same constructor as the
    // executor_health_v2 tests at src/api/executor_health_v2.rs:base_state.
    fn test_state() -> AppState {
        use crate::engine::EngineState;
        let mut state = AppState::new(EngineState::new(Vec::new()));
        state.chain_id = 31337;
        state.network_name = "anvil".to_string();
        state.options_config.enabled = true;
        state
    }
}
