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
    create_user_initiated_execution_intent_from_quote,
    get_option_series as get_option_series_service, list_option_fills as list_option_fills_service,
    list_option_series as list_option_series_service,
};
use crate::options::{
    OptionFill, OptionFillFilter, OptionOrderFilter, OptionSeries, OptionSeriesFilter,
};
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

// FRONTEND-BACKEND-HISTORY-V1 — richer tabbed history payload.
//
// Every column documented in the V1 brief is encoded as `Option<String>`
// (or `Option<u64>` / `Option<i64>` for natural numbers) on a single
// item type so the wire shape stays stable across tabs. Fields not
// applicable to a given tab are emitted as JSON `null`.
//
// Source coverage today:
//   - `trades`        → `option_fills` rows where the address is buyer or seller
//   - `orders`        → `option_orders` rows where the address is the order owner
//   - `transactions`  → `option_execution_intents` rows where the address is buyer or seller
//   - `settlement` / `funding` / `interest` / `liquidations` → empty arrays
//     (no source data exists yet — perps + settlement events are not in
//     scope for this V1 milestone)
#[derive(Clone, Debug, Default, Serialize)]
pub struct HistoryV2Item {
    pub time_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pnl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fees: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settlement_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidation_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penalty: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryV2Data {
    pub address: String,
    pub chain: String,
    pub chain_id: u64,
    pub range: String,
    pub tab: String,
    pub page: u32,
    pub page_size: u32,
    pub total_records: usize,
    pub items: Vec<HistoryV2Item>,
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

// M-P2b typed response payloads. Optional fields use `Option<String>` so
// the JSON output is `null` when the value is not yet wired (the
// `warnings` array carries the structured reason).

#[derive(Clone, Debug, Serialize)]
pub struct FeeBreakdown {
    pub ppm_signed: i64,
    pub amount: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuotePreviewData {
    pub series_id: String,
    pub side: String,
    pub size: String,
    pub price_1e8: String,
    pub premium: String,
    pub buyer_fee: FeeBreakdown,
    pub seller_fee: FeeBreakdown,
    pub settlement_asset: String,
    pub oracle_mark_1e8: Option<String>,
    pub im_impact: Option<String>,
    pub free_collateral_after: Option<String>,
    pub quote_expires_at_ms: i64,
    pub position_size_after: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Position {
    pub series_id: String,
    pub size: String,
    pub side: &'static str,
    pub avg_entry_price_1e8: Option<String>,
    pub mark_price_1e8: Option<String>,
    pub unrealised_pnl: Option<String>,
    pub im_contribution: Option<String>,
    pub mm_contribution: Option<String>,
    pub is_exercisable: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PositionsData {
    pub address: String,
    pub positions: Vec<Position>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PortfolioData {
    pub address: String,
    pub equity: Option<String>,
    pub im: Option<String>,
    pub mm: Option<String>,
    pub free_collateral: Option<String>,
    pub total_notional: Option<String>,
    pub open_positions_count: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BalanceRow {
    pub token: String,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub balance: String,
    pub balance_with_yield: Option<String>,
    pub strategy_assets_preview: Option<String>,
    pub is_collateral_active: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BalancesData {
    pub address: String,
    pub balances: Vec<BalanceRow>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExercisePreviewData {
    pub series_id: String,
    pub account: String,
    pub is_already_settled: bool,
    pub can_settle: bool,
    pub pnl: String,
    pub payable_from_settlement_sink: Option<String>,
    pub insurance_preview: Option<String>,
    pub collectible_from_trader_preview: Option<String>,
    pub residual_bad_debt_preview: Option<String>,
}

// ---------------------------------------------------------------------
// M-P2f — Create-intent request / response DTOs (B7 close)
//
// Public/user-wallet endpoint. **No admin Bearer.** Mints an
// `OptionExecutionIntent` from caller-supplied trade parameters; the
// user explicitly clicks "Sign" afterwards to drive the existing
// signing-payload / signature-submit flow. NEVER signs. NEVER
// broadcasts. NEVER calls the signer / AWS / KMS.
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct CreateExecutionIntentRequest {
    pub series_id: String,
    pub side: String,
    pub size_1e8: String,
    pub price_1e8: String,
    pub buyer: Option<String>,
    pub seller: Option<String>,
    /// Optional. When supplied, the backend may resolve buyer + seller
    /// from a previously-issued RFQ quote. As of M-P2f this is a
    /// forward-compatibility hook only: the body's explicit buyer and
    /// seller fields are mandatory and the quote_id is ignored. A future
    /// milestone (BACKEND-QUOTE-COUNTERPARTY-RESOLVER) will activate it.
    pub quote_id: Option<String>,
    /// Optional client-side correlation id. Echoed back in the
    /// response `meta.request_id` if supplied; otherwise the backend
    /// generates a fresh uuid.
    pub client_request_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateExecutionIntentData {
    pub intent_id: String,
    pub request_id: String,
    pub status: String,
    /// `true` iff the user can immediately call
    /// `GET /options/execution-intents/:intent_id/signing-payload`.
    pub signing_payload_available: bool,
    /// Next step the UI should take. One of:
    ///   * `"request_signing_payload"` — happy path.
    ///   * `"pending_backend_support"` — partial; the UI should
    ///     surface an amber notice.
    pub next_step: &'static str,
    /// Optional deadline beyond which the intent will be rejected at
    /// broadcast. Null when the backend has not yet wired a deadline.
    pub expires_at_ms: Option<i64>,
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
    let provider = rpc_provider_from_state(&state);
    series_details_impl(&state, &series_id, provider.as_ref()).await
}

pub(crate) async fn series_details_impl<P>(
    state: &AppState,
    series_id: &str,
    provider: Option<&P>,
) -> Result<Json<Envelope<SeriesDetailData>>, TradingApiError>
where
    P: crate::execution::rpc::EthCallProvider,
{
    let series = get_option_series_service(state, series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(state, "db"),
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
        list_option_fills_service(state, filter)
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

    // M-P2e — read-only OracleRouter.getPriceSafe(underlying, settlement)
    // when configured AND the RPC provider is available. Surface stale /
    // missing-feed reverts as a structured ORACLE_UNAVAILABLE warning.
    let mut warnings: Vec<Warning> = Vec::new();
    let mut oracle_mark_1e8: Option<String> = None;
    let oracle_configured = state.trading_views.oracle_router_address.is_some();
    let provider_configured = provider.is_some();
    if oracle_configured && provider_configured {
        let p = provider.expect("checked");
        let underlying_addr = address_from_account_str(&series.underlying);
        let settlement_addr = address_from_account_str(&series.settlement_asset);
        match (underlying_addr, settlement_addr) {
            (Some(base), Some(quote)) => {
                let from = &state.execution_config.executor_from_address;
                match crate::api::trading_views::try_get_oracle_price_safe(
                    &state.trading_views,
                    from,
                    base,
                    quote,
                    p,
                )
                .await
                {
                    Ok(Some(price)) => {
                        oracle_mark_1e8 = Some(price.to_string());
                    }
                    Ok(None) => warnings.push(warning_config_missing(
                        "oracle_router_address vanished mid-read; oracle_mark_1e8 unavailable.",
                    )),
                    Err(e) => warnings.push(warning_oracle_unavailable(&format!(
                        "OracleRouter.getPriceSafe failed: {}",
                        sanitise_rpc_err(&e)
                    ))),
                }
            }
            _ => warnings.push(warning_source_unavailable(
                "underlying or settlement_asset is not a valid EVM address; oracle skipped.",
            )),
        }
    } else {
        if !oracle_configured {
            warnings.push(warning_config_missing(
                "OPTION_ORACLE_ROUTER_ADDRESS not configured; oracle_mark_1e8 unavailable.",
            ));
        }
        if !provider_configured {
            warnings.push(warning_rpc_unavailable(
                "RPC provider not configured; oracle_mark_1e8 unavailable.",
            ));
        }
    }

    let status: &'static str = if oracle_mark_1e8.is_some() {
        "ok"
    } else {
        "partial"
    };

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
        oracle_mark_1e8,
    };
    Ok(Json(Envelope {
        status,
        data,
        warnings,
        meta: MetaBlock::new(state, if status == "ok" { "rpc" } else { "db" }),
    }))
}

fn side_str(s: crate::types::Side) -> &'static str {
    use crate::types::Side;
    match s {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn parse_side(value: &str) -> Option<crate::types::Side> {
    match value {
        "buy" => Some(crate::types::Side::Buy),
        "sell" => Some(crate::types::Side::Sell),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// M-P2f — POST /options/execution-intents (B7 close)
// ---------------------------------------------------------------------

/// Public/user-wallet endpoint that mints an
/// `OptionExecutionIntent` from caller-supplied trade params. The
/// returned `intent_id` is consumed by the existing
/// signing-payload / signature-submit / tx-status endpoints.
///
/// **No admin Bearer.** **No signer call.** **No broadcast.**
/// **No AWS / KMS call.** The handler delegates to
/// `create_user_initiated_execution_intent_from_quote` in the
/// service layer, which writes only to the existing
/// `option_execution_intents` table (the same table the M-P2a/M-P3b
/// flow has always written to).
///
/// ## Response shape
///
/// The body is **flat** (not the standard `Envelope<T>` shape) to
/// match the frontend client `createExecutionIntent` decoder which
/// looks for `intent_id` on the top-level object. The fields are
/// captured by `CreateExecutionIntentData`.
///
/// ## Error mapping
///
/// | Condition | TradingErrorCode | HTTP |
/// |---|---|---|
/// | side ∉ {buy,sell} | InvalidRequest | 400 |
/// | size_1e8 not parseable / zero | InvalidRequest | 400 |
/// | price_1e8 not parseable / zero | InvalidRequest | 400 |
/// | buyer or seller missing | InvalidRequest | 400 |
/// | buyer / seller malformed address | InvalidAddress | 400 |
/// | series_id unknown | SeriesNotFound | 404 |
/// | series not Active | QuoteUnsupported | 400 |
/// | buyer == seller (self-trade) | InvalidRequest | 400 |
/// | options disabled | InternalError | 500 |
/// | options not configured for execution | InternalError | 500 |
pub async fn create_execution_intent(
    State(state): State<AppState>,
    Json(req): Json<CreateExecutionIntentRequest>,
) -> Result<Json<CreateExecutionIntentData>, TradingApiError> {
    let side = parse_side(&req.side).ok_or_else(|| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "side must be 'buy' or 'sell'",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    let size_1e8: u128 = req.size_1e8.parse().map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "size_1e8 must be a non-negative integer",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    if size_1e8 == 0 {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "size_1e8 must be positive",
            MetaBlock::new(&state, "validation"),
        ));
    }
    let price_1e8: u128 = req.price_1e8.parse().map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "price_1e8 must be a non-negative integer",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    if price_1e8 == 0 {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "price_1e8 must be positive",
            MetaBlock::new(&state, "validation"),
        ));
    }
    let buyer_raw = req.buyer.as_deref().ok_or_else(|| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "buyer address is required (counterparty resolver not yet wired)",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    let seller_raw = req.seller.as_deref().ok_or_else(|| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "seller address is required (counterparty resolver not yet wired)",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    let buyer = parse_address_or_400(&state, buyer_raw)?;
    let seller = parse_address_or_400(&state, seller_raw)?;
    if buyer.0.eq_ignore_ascii_case(&seller.0) {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "buyer and seller must differ",
            MetaBlock::new(&state, "validation"),
        ));
    }

    let series = get_option_series_service(&state, &req.series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(&state, "db"),
            )
        })?;
    if !matches!(series.status, crate::options::OptionSeriesStatus::Active) {
        return Err(TradingApiError::new(
            TradingErrorCode::QuoteUnsupported,
            "Series is not Active",
            MetaBlock::new(&state, "db"),
        ));
    }

    let intent = create_user_initiated_execution_intent_from_quote(
        &state, &series, buyer, seller, side, size_1e8, price_1e8,
    )
    .await
    .map_err(|err| {
        // Map service-layer BackendError to a public TradingErrorCode.
        // Internal-shape errors collapse to InternalError WITHOUT
        // surfacing the raw message (the message can carry config
        // hints that aren't appropriate for a public endpoint).
        use crate::error::BackendError;
        let (code, message) = match &err {
            BackendError::ZeroSize | BackendError::ZeroPrice => {
                (TradingErrorCode::InvalidRequest, err.to_string())
            }
            BackendError::SelfTrade => (
                TradingErrorCode::InvalidRequest,
                "buyer and seller must differ".to_string(),
            ),
            BackendError::MalformedAccountAddress => (
                TradingErrorCode::InvalidAddress,
                "address shape invalid".to_string(),
            ),
            BackendError::InvalidOptionSeriesState(reason)
            | BackendError::InvalidOptionExecutionIntentState(reason) => {
                (TradingErrorCode::InvalidRequest, reason.clone())
            }
            BackendError::OptionsDisabled => (
                TradingErrorCode::InternalError,
                "options service not enabled".to_string(),
            ),
            _ => (
                TradingErrorCode::InternalError,
                "intent creation failed".to_string(),
            ),
        };
        TradingApiError::new(code, message, MetaBlock::new(&state, "intent_service"))
    })?;

    let request_id = req
        .client_request_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    Ok(Json(CreateExecutionIntentData {
        intent_id: intent.intent_id.to_string(),
        request_id,
        status: intent.status.as_str().to_string(),
        signing_payload_available: matches!(
            intent.status,
            crate::options::OptionExecutionIntentStatus::SignaturesRequired
                | crate::options::OptionExecutionIntentStatus::SignaturesReady
                | crate::options::OptionExecutionIntentStatus::CalldataReady
        ),
        next_step: "request_signing_payload",
        expires_at_ms: None,
    }))
}

#[derive(Debug, Deserialize)]
pub struct QuotePreviewQuery {
    pub series_id: String,
    pub side: String,
    pub size: String,
    pub price_1e8: Option<String>,
    pub account: Option<String>,
}

// ---------------------------------------------------------------------
// M-P2b — Quote / Exercise / Close previews
//
// Posture: PARTIAL implementations. The preview math here derives from
// the option-series store + a deterministic fee-ppm assumption
// (FM_V2.takerFee defaults to ≤ MAX_TAKER_FEE_PPM = 1000). Full on-chain
// MarginEngineLens.previewTradeFees + OracleRouter.getFeed orchestration
// is tracked for M-P2c. Until then, every preview attaches a
// structured `partial_preview` warning so the frontend renders an
// "approximate" badge and the operator can audit the gap.
// ---------------------------------------------------------------------

/// Default taker-fee ppm used in the deterministic partial preview.
/// Matches the conservative end of the FM_V2 surface
/// (`MAX_TAKER_FEE_PPM = 1000`); the on-chain on-chain refinement
/// landing in M-P2c may produce a tighter value per account tier.
const PARTIAL_PREVIEW_TAKER_PPM: i64 = 100;

fn warning_partial_preview() -> Warning {
    Warning {
        code: "PARTIAL_PREVIEW".to_string(),
        message: "Preview is a deterministic approximation. Full \
                  on-chain MarginEngineLens.previewTradeFees + \
                  OracleRouter.getFeed orchestration lands in M-P2c."
            .to_string(),
        details: serde_json::json!({
            "assumed_taker_ppm": PARTIAL_PREVIEW_TAKER_PPM,
        }),
    }
}

fn warning_source_unavailable(reason: &str) -> Warning {
    Warning {
        code: "SOURCE_UNAVAILABLE_FIELD".to_string(),
        message: reason.to_string(),
        details: serde_json::json!({}),
    }
}

pub async fn quote_preview(
    State(state): State<AppState>,
    Query(query): Query<QuotePreviewQuery>,
) -> Result<Json<Envelope<QuotePreviewData>>, TradingApiError> {
    let provider = rpc_provider_from_state(&state);
    quote_preview_impl(&state, query, provider.as_ref()).await
}

pub(crate) async fn quote_preview_impl<P>(
    state: &AppState,
    query: QuotePreviewQuery,
    provider: Option<&P>,
) -> Result<Json<Envelope<QuotePreviewData>>, TradingApiError>
where
    P: crate::execution::rpc::EthCallProvider,
{
    if !["buy", "sell"].contains(&query.side.as_str()) {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "side must be 'buy' or 'sell'",
            MetaBlock::new(state, "validation"),
        ));
    }
    let size: u128 = query.size.parse().map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "size must be a non-negative integer",
            MetaBlock::new(state, "validation"),
        )
    })?;
    if let Some(addr) = query.account.as_deref() {
        parse_address_or_400(state, addr)?;
    }
    let series = get_option_series_service(state, &query.series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(state, "db"),
            )
        })?;
    // Series must be quotable.
    if !matches!(series.status, crate::options::OptionSeriesStatus::Active) {
        return Err(TradingApiError::new(
            TradingErrorCode::QuoteUnsupported,
            "Series is not Active",
            MetaBlock::new(state, "db"),
        ));
    }
    // Reference price: use the explicit price_1e8 if provided, else the
    // series strike as a deterministic placeholder. The "approximate"
    // warning declares the assumption.
    let price_1e8: u128 = match query.price_1e8.as_deref() {
        Some(p) => p.parse().map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::InvalidRequest,
                "price_1e8 must be a non-negative integer",
                MetaBlock::new(state, "validation"),
            )
        })?,
        None => series.strike_1e8,
    };
    let premium = size.saturating_mul(price_1e8);
    let fee_amount = premium.saturating_mul(PARTIAL_PREVIEW_TAKER_PPM as u128) / 1_000_000u128;
    let quote_expires_at_ms = now_ms() + 20_000; // 20s freshness window.

    // M-P2e — read-only oracle mark when configured.
    let mut warnings: Vec<Warning> = Vec::new();
    let mut oracle_mark_1e8: Option<String> = None;
    if state.trading_views.oracle_router_address.is_some() {
        if let Some(p) = provider {
            let underlying_addr = address_from_account_str(&series.underlying);
            let settlement_addr = address_from_account_str(&series.settlement_asset);
            if let (Some(base), Some(quote)) = (underlying_addr, settlement_addr) {
                match crate::api::trading_views::try_get_oracle_price_safe(
                    &state.trading_views,
                    &state.execution_config.executor_from_address,
                    base,
                    quote,
                    p,
                )
                .await
                {
                    Ok(Some(mark)) => oracle_mark_1e8 = Some(mark.to_string()),
                    Ok(None) => {}
                    Err(e) => warnings.push(warning_oracle_unavailable(&format!(
                        "OracleRouter.getPriceSafe failed: {}",
                        sanitise_rpc_err(&e)
                    ))),
                }
            }
        }
    }
    warnings.push(warning_partial_preview());
    if oracle_mark_1e8.is_none() {
        warnings.push(warning_oracle_unavailable(
            "oracle_mark_1e8 unavailable (OracleRouter address or RPC not configured).",
        ));
    }

    Ok(Json(Envelope {
        // quote_preview is intentionally always "partial" — premium /
        // fee math here is a deterministic approximation. Even when the
        // oracle is wired the math doesn't switch to "ok" because
        // PARTIAL_PREVIEW_TAKER_PPM is still a hard-coded placeholder
        // (real previewTradeFees decoding lands in a follow-on).
        status: "partial",
        data: QuotePreviewData {
            series_id: series.option_series_id.to_string(),
            side: query.side.clone(),
            size: size.to_string(),
            price_1e8: price_1e8.to_string(),
            premium: premium.to_string(),
            buyer_fee: FeeBreakdown {
                ppm_signed: PARTIAL_PREVIEW_TAKER_PPM,
                amount: fee_amount.to_string(),
            },
            seller_fee: FeeBreakdown {
                ppm_signed: PARTIAL_PREVIEW_TAKER_PPM,
                amount: fee_amount.to_string(),
            },
            settlement_asset: series.settlement_asset.clone(),
            oracle_mark_1e8,
            im_impact: None,
            free_collateral_after: None,
            quote_expires_at_ms,
            position_size_after: None,
        },
        warnings,
        meta: MetaBlock::new(state, "db"),
    }))
}

pub async fn account_positions(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Envelope<PositionsData>>, TradingApiError> {
    let provider = rpc_provider_from_state(&state);
    account_positions_impl(&state, &address, provider.as_ref()).await
}

pub(crate) async fn account_positions_impl<P>(
    state: &AppState,
    address: &str,
    provider: Option<&P>,
) -> Result<Json<Envelope<PositionsData>>, TradingApiError>
where
    P: crate::execution::rpc::EthCallProvider,
{
    let acct = parse_address_or_400(state, address)?;
    // Aggregate net position per series from the OptionFill store.
    let fills = list_option_fills_service(
        state,
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
            MetaBlock::new(state, "internal"),
        )
    })?;
    // For each series, compute signed net size (positive = long; negative = short).
    use std::collections::HashMap;
    let mut net: HashMap<String, (i128, u128, u128)> = HashMap::new();
    // (signed_size_1e8, sum_premium_for_avg_entry, total_abs_size_for_avg_entry)
    for f in &fills {
        let is_long = f.buyer == acct;
        let signed_delta: i128 = if is_long {
            f.size_1e8 as i128
        } else {
            -(f.size_1e8 as i128)
        };
        let entry = net
            .entry(f.option_series_id.to_string())
            .or_insert((0, 0, 0));
        entry.0 = entry.0.saturating_add(signed_delta);
        entry.1 = entry
            .1
            .saturating_add(f.size_1e8.saturating_mul(f.price_1e8));
        entry.2 = entry.2.saturating_add(f.size_1e8);
    }
    let mut positions = Vec::with_capacity(net.len());
    for (series_id, (signed, premium_sum, size_sum)) in net {
        if signed == 0 {
            continue;
        }
        let (side, abs_size) = if signed > 0 {
            ("long", signed as u128)
        } else {
            ("short", (-signed) as u128)
        };
        let avg_entry_price_1e8 = premium_sum.checked_div(size_sum).map(|v| v.to_string());
        positions.push(Position {
            series_id,
            size: abs_size.to_string(),
            side,
            avg_entry_price_1e8,
            mark_price_1e8: None,
            unrealised_pnl: None,
            im_contribution: None,
            mm_contribution: None,
            is_exercisable: None,
        });
    }
    positions.sort_by(|a, b| a.series_id.cmp(&b.series_id));

    // M-P2e — per-position oracle mark when oracle is configured. We
    // call OracleRouter.getPriceSafe(underlying, settlement) for each
    // distinct (underlying, settlement) pair. The on-chain function
    // reverts on stale feeds; we map any revert to a structured
    // warning + leave that series' mark as None.
    let mut warnings: Vec<Warning> = Vec::new();
    let oracle_configured = state.trading_views.oracle_router_address.is_some();
    let provider_configured = provider.is_some();
    let mut any_mark_resolved = false;
    if oracle_configured && provider_configured {
        let p = provider.expect("checked");
        let from = &state.execution_config.executor_from_address;
        use std::collections::HashMap;
        let mut mark_cache: HashMap<(String, String), Option<String>> = HashMap::new();
        for pos in positions.iter_mut() {
            // Look up series metadata to obtain underlying/settlement.
            let series = match get_option_series_service(state, &pos.series_id).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let key = (series.underlying.clone(), series.settlement_asset.clone());
            let cached = mark_cache.get(&key).cloned();
            let resolved = if let Some(c) = cached {
                c
            } else {
                let underlying_addr = address_from_account_str(&series.underlying);
                let settlement_addr = address_from_account_str(&series.settlement_asset);
                let result = match (underlying_addr, settlement_addr) {
                    (Some(base), Some(quote)) => {
                        match crate::api::trading_views::try_get_oracle_price_safe(
                            &state.trading_views,
                            from,
                            base,
                            quote,
                            p,
                        )
                        .await
                        {
                            Ok(Some(mark)) => Some(mark.to_string()),
                            Ok(None) => None,
                            Err(_) => None,
                        }
                    }
                    _ => None,
                };
                mark_cache.insert(key, result.clone());
                result
            };
            if resolved.is_some() {
                any_mark_resolved = true;
            }
            pos.mark_price_1e8 = resolved;
        }
        if !any_mark_resolved && !positions.is_empty() {
            warnings.push(warning_oracle_unavailable(
                "OracleRouter.getPriceSafe failed or returned no marks for any open series.",
            ));
        }
    } else {
        if !oracle_configured {
            warnings.push(warning_config_missing(
                "OPTION_ORACLE_ROUTER_ADDRESS not configured; mark_price_1e8 unavailable.",
            ));
        }
        if !provider_configured {
            warnings.push(warning_rpc_unavailable(
                "RPC provider not configured; mark_price_1e8 unavailable.",
            ));
        }
    }
    if !any_mark_resolved {
        warnings.push(warning_source_unavailable(
            "unrealised_pnl / IM / MM contribution not wired without per-series mark + per-position lens read.",
        ));
    }

    let status: &'static str = if any_mark_resolved && !positions.is_empty() {
        "ok"
    } else {
        "partial"
    };

    Ok(Json(Envelope {
        status,
        data: PositionsData {
            address: address.to_string(),
            positions,
        },
        warnings,
        meta: MetaBlock::new(state, if status == "ok" { "rpc" } else { "db" }),
    }))
}

pub async fn account_portfolio(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Envelope<PortfolioData>>, TradingApiError> {
    let provider = rpc_provider_from_state(&state);
    account_portfolio_impl(&state, &address, provider.as_ref()).await
}

pub(crate) async fn account_portfolio_impl<P>(
    state: &AppState,
    address: &str,
    provider: Option<&P>,
) -> Result<Json<Envelope<PortfolioData>>, TradingApiError>
where
    P: crate::execution::rpc::EthCallProvider,
{
    let acct = parse_address_or_400(state, address)?;
    let fills = list_option_fills_service(
        state,
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
            MetaBlock::new(state, "internal"),
        )
    })?;
    use std::collections::HashMap;
    let mut net_per_series: HashMap<String, i128> = HashMap::new();
    let mut total_notional: u128 = 0;
    for f in &fills {
        let is_long = f.buyer == acct;
        let signed_delta: i128 = if is_long {
            f.size_1e8 as i128
        } else {
            -(f.size_1e8 as i128)
        };
        *net_per_series
            .entry(f.option_series_id.to_string())
            .or_insert(0) += signed_delta;
        total_notional = total_notional.saturating_add(f.size_1e8.saturating_mul(f.price_1e8));
    }
    let open_positions_count = net_per_series.values().filter(|&&v| v != 0).count() as u32;

    // M-P2e — read MarginEngineLens.getAccountState when configured.
    let mut warnings: Vec<Warning> = Vec::new();
    let mut equity: Option<String> = None;
    let mut im: Option<String> = None;
    let mut mm: Option<String> = None;
    let mut free_collateral: Option<String> = None;

    let lens_configured = state.trading_views.margin_engine_lens_address.is_some();
    let me_configured = state.trading_views.margin_engine_address.is_some();
    let provider_configured = provider.is_some();
    if lens_configured && me_configured && provider_configured {
        let p = provider.expect("checked");
        let me_acct = state
            .trading_views
            .margin_engine_address
            .as_ref()
            .expect("checked");
        let me_addr = address_from_account_str(&me_acct.0);
        let trader_addr = address_from_account_str(&acct.0);
        match (me_addr, trader_addr) {
            (Some(me), Some(trader)) => {
                let from = &state.execution_config.executor_from_address;
                match crate::api::trading_views::try_get_account_state(
                    &state.trading_views,
                    from,
                    me,
                    trader,
                    p,
                )
                .await
                {
                    Ok(Some(bytes)) => match decode_account_state(&bytes) {
                        Ok(decoded) => {
                            equity = Some(decoded.equity_base.to_string());
                            mm = Some(decoded.maintenance_margin_base.to_string());
                            im = Some(decoded.initial_margin_base.to_string());
                            free_collateral = Some(decoded.free_collateral_base.to_string());
                        }
                        Err(e) => warnings.push(warning_account_state_unavailable(&format!(
                            "AccountState decode failed: {}",
                            sanitise_rpc_err(&e)
                        ))),
                    },
                    Ok(None) => warnings.push(warning_config_missing(
                        "margin_engine_lens_address vanished mid-read.",
                    )),
                    Err(e) => warnings.push(warning_account_state_unavailable(&format!(
                        "MarginEngineLens.getAccountState failed: {}",
                        sanitise_rpc_err(&e)
                    ))),
                }
            }
            _ => warnings.push(warning_source_unavailable(
                "margin engine or trader address is not a valid EVM address.",
            )),
        }
    } else {
        let mut reasons: Vec<&str> = Vec::new();
        if !lens_configured {
            reasons.push("OPTION_MARGIN_ENGINE_LENS_ADDRESS");
        }
        if !me_configured {
            reasons.push("OPTION_MARGIN_ENGINE_ADDRESS");
        }
        if !provider_configured {
            reasons.push("EXECUTION_RPC_URL");
        }
        warnings.push(warning_config_missing(&format!(
            "AccountState read requires: {}.",
            reasons.join(", ")
        )));
    }

    let status: &'static str =
        if equity.is_some() && im.is_some() && mm.is_some() && free_collateral.is_some() {
            "ok"
        } else {
            "partial"
        };
    Ok(Json(Envelope {
        status,
        data: PortfolioData {
            address: address.to_string(),
            equity,
            im,
            mm,
            free_collateral,
            total_notional: Some(total_notional.to_string()),
            open_positions_count: Some(open_positions_count),
        },
        warnings,
        meta: MetaBlock::new(state, if status == "ok" { "rpc" } else { "db" }),
    }))
}

pub async fn account_balances(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<Json<Envelope<BalancesData>>, TradingApiError> {
    // Production path: build an HttpJsonRpcProvider from
    // `state.execution_config.rpc_url` when configured; otherwise route
    // straight to the M-P2b partial fallback.
    let provider = state
        .execution_config
        .rpc_url
        .as_ref()
        .map(|url| crate::execution::rpc::HttpJsonRpcProvider::new(url.clone()));
    account_balances_impl(&state, &address, provider.as_ref()).await
}

pub(crate) async fn account_balances_impl<P>(
    state: &AppState,
    address: &str,
    provider: Option<&P>,
) -> Result<Json<Envelope<BalancesData>>, TradingApiError>
where
    P: crate::execution::rpc::EthCallProvider,
{
    let acct = parse_address_or_400(state, address)?;
    let from = &state.execution_config.executor_from_address;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut balances: Vec<BalanceRow> = Vec::new();
    let mut all_ok = true;

    // Need both the views address and the underlying CV address to read
    // anything. If either is missing, fall through to the partial-empty
    // fallback (matches M-P2b behaviour).
    let views_configured = state.trading_views.collateral_vault_views_address.is_some();
    let cv_configured = state.trading_views.collateral_vault_address.is_some();
    let provider_configured = provider.is_some();

    if views_configured && cv_configured && provider_configured {
        let provider = provider.expect("checked above");
        match crate::api::trading_views::try_get_collateral_tokens(
            &state.trading_views,
            from,
            provider,
        )
        .await
        {
            Ok(Some(tokens)) => {
                for token in tokens {
                    match crate::api::trading_views::try_get_balance(
                        &state.trading_views,
                        from,
                        &acct,
                        token,
                        provider,
                    )
                    .await
                    {
                        Ok(Some(bal)) => balances.push(BalanceRow {
                            token: format!("{token:#x}"),
                            symbol: None,
                            decimals: None,
                            balance: bal.to_string(),
                            balance_with_yield: None,
                            strategy_assets_preview: None,
                            is_collateral_active: None,
                        }),
                        Ok(None) => {
                            // CV address removed mid-read; treat as configuration gap.
                            all_ok = false;
                            warnings.push(warning_source_unavailable(
                                "collateral_vault_address not configured; per-token balance unreachable.",
                            ));
                            break;
                        }
                        Err(e) => {
                            all_ok = false;
                            warnings.push(Warning {
                                code: "RPC_UNAVAILABLE".to_string(),
                                message: format!(
                                    "balance read failed for token {token:#x}: {}",
                                    sanitise_rpc_err(&e)
                                ),
                                details: serde_json::json!({}),
                            });
                            break;
                        }
                    }
                }
            }
            Ok(None) => {
                all_ok = false;
                warnings.push(warning_source_unavailable(
                    "collateral_vault_views_address not configured; token list unreachable.",
                ));
            }
            Err(e) => {
                all_ok = false;
                warnings.push(Warning {
                    code: "RPC_UNAVAILABLE".to_string(),
                    message: format!(
                        "collateral token list read failed: {}",
                        sanitise_rpc_err(&e)
                    ),
                    details: serde_json::json!({}),
                });
            }
        }
    } else {
        all_ok = false;
        let mut reasons = Vec::new();
        if !views_configured {
            reasons.push("CollateralVaultViews address");
        }
        if !cv_configured {
            reasons.push("CollateralVault address");
        }
        if !provider_configured {
            reasons.push("RPC provider (EXECUTION RPC_URL)");
        }
        warnings.push(warning_source_unavailable(&format!(
            "Per-token balances require: {}. Falling back to empty list.",
            reasons.join(", ")
        )));
    }

    let status: &'static str = if all_ok && !balances.is_empty() {
        "ok"
    } else {
        "partial"
    };
    Ok(Json(Envelope {
        status,
        data: BalancesData {
            address: address.to_string(),
            balances,
        },
        warnings,
        meta: MetaBlock::new(state, if all_ok { "rpc" } else { "db" }),
    }))
}

// ---------------------------------------------------------------------
// M-P2e — internal helpers for read-only RPC orchestration
// ---------------------------------------------------------------------

/// Build an `Option<HttpJsonRpcProvider>` from `state.execution_config.rpc_url`.
/// Returns `None` when no RPC URL is configured — handlers then take the
/// partial-data path with structured warnings.
fn rpc_provider_from_state(state: &AppState) -> Option<crate::execution::rpc::HttpJsonRpcProvider> {
    state
        .execution_config
        .rpc_url
        .as_ref()
        .map(|url| crate::execution::rpc::HttpJsonRpcProvider::new(url.clone()))
}

/// Parse an `AccountId` hex string into an `alloy` `Address`.
/// Returns `None` for malformed input.
pub(crate) fn address_from_account_str(s: &str) -> Option<alloy_primitives::Address> {
    crate::api::trading_views::address_from_account(&AccountId::new(s.to_string()))
}

/// Decode the AccountState tuple returned by
/// `MarginEngineLens.getAccountState`. The on-chain shape is verified
/// against `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/MarginEngineLens.abi.json`:
///
/// ```text
/// (int256 equityBase, uint256 maintenanceMarginBase, uint256 initialMarginBase,
///  int256 freeCollateralBase, uint256 marginRatioBps, uint256 openSeriesCount,
///  uint256 totalShortOpenContracts, bool liquidatable)
/// ```
#[derive(Clone, Debug)]
struct DecodedAccountState {
    equity_base: alloy_primitives::I256,
    maintenance_margin_base: alloy_primitives::U256,
    initial_margin_base: alloy_primitives::U256,
    free_collateral_base: alloy_primitives::I256,
}

fn decode_account_state(bytes: &[u8]) -> Result<DecodedAccountState, String> {
    use alloy_primitives::{I256, U256};
    use alloy_sol_types::sol_data;
    use alloy_sol_types::SolType;
    type Tup = (
        sol_data::Int<256>,
        sol_data::Uint<256>,
        sol_data::Uint<256>,
        sol_data::Int<256>,
        sol_data::Uint<256>,
        sol_data::Uint<256>,
        sol_data::Uint<256>,
        sol_data::Bool,
    );
    let (equity, mm, im, free, _ratio, _count, _short, _liq): (
        I256,
        U256,
        U256,
        I256,
        U256,
        U256,
        U256,
        bool,
    ) = Tup::abi_decode(bytes, true).map_err(|e| format!("account state decode: {e}"))?;
    Ok(DecodedAccountState {
        equity_base: equity,
        maintenance_margin_base: mm,
        initial_margin_base: im,
        free_collateral_base: free,
    })
}

/// Decode the SettlementPreview tuple returned by
/// `MarginEngineLens.previewAccountSettlement`. The on-chain shape is
/// verified against `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/MarginEngineLens.abi.json`:
///
/// ```text
/// (int256 pnl, uint256 grossAmount, uint256 collectibleAmount,
///  uint256 payableFromSettlementSink, uint256 insurancePreview,
///  uint256 residualBadDebtPreview, bool isSettled, bool canSettle)
/// ```
#[derive(Clone, Debug)]
struct DecodedSettlementPreview {
    pnl: alloy_primitives::I256,
    collectible_amount: alloy_primitives::U256,
    payable_from_settlement_sink: alloy_primitives::U256,
    insurance_preview: alloy_primitives::U256,
    residual_bad_debt_preview: alloy_primitives::U256,
    is_settled: bool,
    can_settle: bool,
}

fn decode_settlement_preview(bytes: &[u8]) -> Result<DecodedSettlementPreview, String> {
    use alloy_primitives::{I256, U256};
    use alloy_sol_types::sol_data;
    use alloy_sol_types::SolType;
    type Tup = (
        sol_data::Int<256>,
        sol_data::Uint<256>,
        sol_data::Uint<256>,
        sol_data::Uint<256>,
        sol_data::Uint<256>,
        sol_data::Uint<256>,
        sol_data::Bool,
        sol_data::Bool,
    );
    let (pnl, _gross, collectible, payable, insurance, residual, is_settled, can_settle): (
        I256,
        U256,
        U256,
        U256,
        U256,
        U256,
        bool,
        bool,
    ) = Tup::abi_decode(bytes, true).map_err(|e| format!("settlement preview decode: {e}"))?;
    Ok(DecodedSettlementPreview {
        pnl,
        collectible_amount: collectible,
        payable_from_settlement_sink: payable,
        insurance_preview: insurance,
        residual_bad_debt_preview: residual,
        is_settled,
        can_settle,
    })
}

fn warning_rpc_unavailable(reason: &str) -> Warning {
    Warning {
        code: "RPC_UNAVAILABLE".to_string(),
        message: reason.to_string(),
        details: serde_json::json!({}),
    }
}

fn warning_oracle_unavailable(reason: &str) -> Warning {
    Warning {
        code: "ORACLE_UNAVAILABLE".to_string(),
        message: reason.to_string(),
        details: serde_json::json!({}),
    }
}

fn warning_config_missing(reason: &str) -> Warning {
    Warning {
        code: "CONFIG_MISSING".to_string(),
        message: reason.to_string(),
        details: serde_json::json!({}),
    }
}

fn warning_account_state_unavailable(reason: &str) -> Warning {
    Warning {
        code: "ACCOUNT_STATE_UNAVAILABLE".to_string(),
        message: reason.to_string(),
        details: serde_json::json!({}),
    }
}

fn warning_settlement_preview_unavailable(reason: &str) -> Warning {
    Warning {
        code: "SETTLEMENT_PREVIEW_UNAVAILABLE".to_string(),
        message: reason.to_string(),
        details: serde_json::json!({}),
    }
}

/// Sanitise RPC-side error messages so the response envelope never
/// exposes a raw RPC URL or a provider's internal trace. We strip
/// anything that looks like an HTTP(S) URL.
fn sanitise_rpc_err(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == 'h' {
            // Speculatively match http:// or https://
            let rest: String = chars.clone().take(7).collect();
            if rest.starts_with("ttp://") || rest.starts_with("ttps://") {
                // Skip until whitespace / end.
                while let Some(&nx) = chars.peek() {
                    if nx.is_whitespace() {
                        break;
                    }
                    chars.next();
                }
                out.push_str("<url-redacted>");
                continue;
            }
        }
        out.push(c);
    }
    // Cap length to 200 chars; truncate noisy provider dumps.
    if out.len() > 200 {
        out.truncate(200);
        out.push('…');
    }
    out
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

// ---------------------------------------------------------------------
// FRONTEND-BACKEND-HISTORY-V1 — tabbed, paginated, range-scoped history
// ---------------------------------------------------------------------

/// Tabs surfaced by the V1 frontend history page. The four `_empty`
/// variants intentionally return zero rows today; they are kept in the
/// API so the frontend tab order is stable as perps + settlement land.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryTab {
    Trades,
    Transactions,
    Orders,
    Settlement,
    Funding,
    Interest,
    Liquidations,
}

impl HistoryTab {
    fn as_str(self) -> &'static str {
        match self {
            Self::Trades => "trades",
            Self::Transactions => "transactions",
            Self::Orders => "orders",
            Self::Settlement => "settlement",
            Self::Funding => "funding",
            Self::Interest => "interest",
            Self::Liquidations => "liquidations",
        }
    }

    fn parse(v: &str) -> Option<Self> {
        match v {
            "trades" => Some(Self::Trades),
            "transactions" => Some(Self::Transactions),
            "orders" => Some(Self::Orders),
            "settlement" => Some(Self::Settlement),
            "funding" => Some(Self::Funding),
            "interest" => Some(Self::Interest),
            "liquidations" => Some(Self::Liquidations),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryRange {
    LastDay,
    LastWeek,
    LastMonth,
    LastQuarter,
    All,
}

impl HistoryRange {
    fn as_str(self) -> &'static str {
        match self {
            Self::LastDay => "last_day",
            Self::LastWeek => "last_week",
            Self::LastMonth => "last_month",
            Self::LastQuarter => "last_quarter",
            Self::All => "all",
        }
    }

    fn parse(v: &str) -> Option<Self> {
        match v {
            "last_day" => Some(Self::LastDay),
            "last_week" => Some(Self::LastWeek),
            "last_month" => Some(Self::LastMonth),
            "last_quarter" => Some(Self::LastQuarter),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Inclusive lower bound on `created_at_ms`. `None` for `All`.
    fn since_ms(self, now_ms: i64) -> Option<i64> {
        const DAY: i64 = 24 * 60 * 60 * 1000;
        match self {
            Self::LastDay => Some(now_ms - DAY),
            Self::LastWeek => Some(now_ms - 7 * DAY),
            Self::LastMonth => Some(now_ms - 30 * DAY),
            Self::LastQuarter => Some(now_ms - 90 * DAY),
            Self::All => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct HistoryV2Query {
    pub tab: Option<String>,
    pub range: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

const MAX_HISTORY_PAGE_SIZE: u32 = 10_000;
const DEFAULT_HISTORY_PAGE_SIZE: u32 = 100;

fn chain_name_for(chain_id: u64) -> &'static str {
    match chain_id {
        1 => "ethereum-mainnet",
        8453 => "base-mainnet",
        84532 => "base-sepolia",
        11155111 => "ethereum-sepolia",
        31337 => "anvil",
        _ => "unknown",
    }
}

fn empty_history_envelope(
    state: &AppState,
    address: &str,
    tab: HistoryTab,
    range: HistoryRange,
    page: u32,
    page_size: u32,
) -> Json<Envelope<HistoryV2Data>> {
    Json(Envelope {
        status: "ok",
        data: HistoryV2Data {
            address: address.to_string(),
            chain: chain_name_for(state.chain_id).to_string(),
            chain_id: state.chain_id,
            range: range.as_str().to_string(),
            tab: tab.as_str().to_string(),
            page,
            page_size,
            total_records: 0,
            items: Vec::new(),
        },
        warnings: Vec::new(),
        meta: MetaBlock::new(state, "db"),
    })
}

pub async fn account_history_v2(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(q): Query<HistoryV2Query>,
) -> Result<Json<Envelope<HistoryV2Data>>, TradingApiError> {
    let acct = parse_address_or_400(&state, &address)?;

    let tab_str = q.tab.as_deref().unwrap_or("trades");
    let tab = HistoryTab::parse(tab_str).ok_or_else(|| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "Unknown history tab; expected one of trades/transactions/orders/settlement/funding/interest/liquidations.",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    let range_str = q.range.as_deref().unwrap_or("last_month");
    let range = HistoryRange::parse(range_str).ok_or_else(|| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "Unknown history range; expected one of last_day/last_week/last_month/last_quarter/all.",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q
        .page_size
        .unwrap_or(DEFAULT_HISTORY_PAGE_SIZE)
        .clamp(1, MAX_HISTORY_PAGE_SIZE);

    // Tabs that don't have source data yet return a clean empty page.
    match tab {
        HistoryTab::Settlement
        | HistoryTab::Funding
        | HistoryTab::Interest
        | HistoryTab::Liquidations => {
            return Ok(empty_history_envelope(
                &state, &address, tab, range, page, page_size,
            ));
        }
        _ => {}
    }

    let since = range.since_ms(now_ms());

    let mut items: Vec<HistoryV2Item> = match tab {
        HistoryTab::Trades => trades_rows_for(&state, &acct, since).await?,
        HistoryTab::Orders => orders_rows_for(&state, &acct, since).await?,
        HistoryTab::Transactions => transactions_rows_for(&state, &acct, since).await?,
        _ => Vec::new(),
    };
    items.sort_by(|a, b| b.time_ms.cmp(&a.time_ms));
    let total_records = items.len();

    let start = ((page - 1) as usize).saturating_mul(page_size as usize);
    let end = start.saturating_add(page_size as usize).min(total_records);
    let page_items = if start >= total_records {
        Vec::new()
    } else {
        items[start..end].to_vec()
    };

    Ok(Json(Envelope {
        status: "ok",
        data: HistoryV2Data {
            address: address.clone(),
            chain: chain_name_for(state.chain_id).to_string(),
            chain_id: state.chain_id,
            range: range.as_str().to_string(),
            tab: tab.as_str().to_string(),
            page,
            page_size,
            total_records,
            items: page_items,
        },
        warnings: Vec::new(),
        meta: MetaBlock::new(&state, "db"),
    }))
}

async fn trades_rows_for(
    state: &AppState,
    acct: &AccountId,
    since_ms: Option<i64>,
) -> Result<Vec<HistoryV2Item>, TradingApiError> {
    let fills = list_option_fills_service(
        state,
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
            MetaBlock::new(state, "internal"),
        )
    })?;

    let mut out = Vec::with_capacity(fills.len());
    for f in fills {
        if let Some(min) = since_ms {
            if f.created_at_ms < min {
                continue;
            }
        }
        let is_buyer = f.buyer.0.eq_ignore_ascii_case(&acct.0);
        let side = if is_buyer { "buy" } else { "sell" };
        let role = if (is_buyer && f.maker_order_id == f.buy_order_id)
            || (!is_buyer && f.maker_order_id == f.sell_order_id)
        {
            "maker"
        } else {
            "taker"
        };
        out.push(HistoryV2Item {
            time_ms: f.created_at_ms,
            instrument: Some(f.option_series_id.to_string()),
            side: Some(side.to_string()),
            amount: Some(f.size_1e8.to_string()),
            price: Some(f.price_1e8.to_string()),
            total: None,
            pnl: None,
            fees: None,
            status: Some("filled".to_string()),
            kind: Some("option".to_string()),
            role: Some(role.to_string()),
            tx_hash: None,
            ..HistoryV2Item::default()
        });
    }
    Ok(out)
}

async fn orders_rows_for(
    state: &AppState,
    acct: &AccountId,
    since_ms: Option<i64>,
) -> Result<Vec<HistoryV2Item>, TradingApiError> {
    use crate::options::service::list_option_orders as list_option_orders_service;
    let orders = list_option_orders_service(
        state,
        OptionOrderFilter {
            option_series_id: None,
            account: Some(acct.clone()),
            status: None,
            side: None,
        },
    )
    .await
    .map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InternalError,
            "unable to list option orders",
            MetaBlock::new(state, "internal"),
        )
    })?;

    let mut out = Vec::with_capacity(orders.len());
    for o in orders {
        if let Some(min) = since_ms {
            if o.created_at_ms < min {
                continue;
            }
        }
        let filled_amount_1e8 = o.size_1e8.saturating_sub(o.remaining_size_1e8);
        let side_str = match o.side {
            crate::types::Side::Buy => "buy",
            crate::types::Side::Sell => "sell",
        };
        let tif_str = match o.time_in_force {
            crate::types::TimeInForce::Gtc => "gtc",
            crate::types::TimeInForce::Ioc => "ioc",
            crate::types::TimeInForce::Fok => "fok",
        };
        out.push(HistoryV2Item {
            time_ms: o.created_at_ms,
            instrument: Some(o.option_series_id.to_string()),
            side: Some(side_str.to_string()),
            order_type: Some(tif_str.to_string()),
            amount: Some(o.size_1e8.to_string()),
            limit_price: Some(o.price_1e8.to_string()),
            filled: Some(filled_amount_1e8.to_string()),
            status: Some(o.status.as_str().to_string()),
            role: None,
            tx_hash: None,
            ..HistoryV2Item::default()
        });
    }
    Ok(out)
}

async fn transactions_rows_for(
    state: &AppState,
    acct: &AccountId,
    since_ms: Option<i64>,
) -> Result<Vec<HistoryV2Item>, TradingApiError> {
    use crate::options::service::list_option_execution_intents as list_option_execution_intents_service;
    let intents = list_option_execution_intents_service(state)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::InternalError,
                "unable to list option execution intents",
                MetaBlock::new(state, "internal"),
            )
        })?;

    let chain_label = chain_name_for(state.chain_id).to_string();
    let mut out: Vec<HistoryV2Item> = Vec::new();
    for i in intents {
        let is_buyer = i.buyer.0.eq_ignore_ascii_case(&acct.0);
        let is_seller = i.seller.0.eq_ignore_ascii_case(&acct.0);
        if !is_buyer && !is_seller {
            continue;
        }
        let t = i.updated_at_ms.max(i.created_at_ms);
        if let Some(min) = since_ms {
            if t < min {
                continue;
            }
        }
        out.push(HistoryV2Item {
            time_ms: t,
            instrument: Some(i.option_series_id.to_string()),
            action: Some("option_execute".to_string()),
            asset: Some(i.settlement_asset.0.clone()),
            amount: Some(i.source_size_1e8.to_string()),
            status: Some(i.status.as_str().to_string()),
            tx_hash: None,
            block: None,
            gas: None,
            side: Some(if is_buyer {
                "buy".to_string()
            } else {
                "sell".to_string()
            }),
            kind: Some("option".to_string()),
            ..HistoryV2Item::default()
        });
        // The intent.tx_hash + block + gas live on the linked
        // option_execution_transactions row; that table is not exposed
        // by a service helper yet, so V1 leaves those fields null and
        // a follow-on milestone can backfill from the transactions
        // store without changing the wire shape.
        let _ = chain_label;
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// FRONTEND-BACKEND-LEADERBOARD-V1 — global ranking of accounts by
// trading volume, derived from `option_fills`.
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct LeaderboardItem {
    pub rank: u32,
    pub address: String,
    pub trade_count: u64,
    /// Sum of `size_1e8 * price_1e8 / 1e8` across the participant's
    /// fills, encoded as a base-10 string so the JSON layer never
    /// truncates large values.
    pub volume_1e8: String,
    /// Realized PnL — `null` in V1 (requires settlement-event
    /// indexing). The wire shape reserves the field so a follow-on
    /// milestone can backfill without breaking clients.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realized_pnl_1e8: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LeaderboardData {
    pub chain: String,
    pub chain_id: u64,
    pub range: String,
    pub page: u32,
    pub page_size: u32,
    pub total_records: usize,
    pub items: Vec<LeaderboardItem>,
}

#[derive(Debug, Deserialize)]
pub struct LeaderboardQuery {
    pub range: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn leaderboard(
    State(state): State<AppState>,
    Query(q): Query<LeaderboardQuery>,
) -> Result<Json<Envelope<LeaderboardData>>, TradingApiError> {
    let range_str = q.range.as_deref().unwrap_or("last_month");
    let range = HistoryRange::parse(range_str).ok_or_else(|| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "Unknown leaderboard range; expected one of last_day/last_week/last_month/last_quarter/all.",
            MetaBlock::new(&state, "validation"),
        )
    })?;
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q
        .page_size
        .unwrap_or(DEFAULT_HISTORY_PAGE_SIZE)
        .clamp(1, MAX_HISTORY_PAGE_SIZE);

    let fills = list_option_fills_service(
        &state,
        OptionFillFilter {
            option_series_id: None,
            account: None,
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

    let since = range.since_ms(now_ms());

    // address(lowercased) → (trade_count, volume_1e8 accumulator)
    let mut agg: std::collections::HashMap<String, (u64, u128)> = std::collections::HashMap::new();
    for f in fills {
        if let Some(min) = since {
            if f.created_at_ms < min {
                continue;
            }
        }
        // Notional notional_1e8 = (price_1e8 * size_1e8) / 1e8.
        // Use u128 multiplication then divide; both inputs are u64-ish
        // so this cannot overflow on testnet-scale data.
        let notional = (f.price_1e8 as u128).saturating_mul(f.size_1e8 as u128) / 100_000_000u128;
        let buyer = f.buyer.0.to_ascii_lowercase();
        let seller = f.seller.0.to_ascii_lowercase();
        let entry_b = agg.entry(buyer).or_insert((0, 0));
        entry_b.0 = entry_b.0.saturating_add(1);
        entry_b.1 = entry_b.1.saturating_add(notional);
        let entry_s = agg.entry(seller).or_insert((0, 0));
        entry_s.0 = entry_s.0.saturating_add(1);
        entry_s.1 = entry_s.1.saturating_add(notional);
    }

    let mut ranked: Vec<(String, u64, u128)> = agg
        .into_iter()
        .map(|(addr, (tc, vol))| (addr, tc, vol))
        .collect();
    // Sort by volume desc, then trade_count desc, then address asc for
    // deterministic ties.
    ranked.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.0.cmp(&b.0))
    });

    let total_records = ranked.len();
    let start = ((page - 1) as usize).saturating_mul(page_size as usize);
    let end = start.saturating_add(page_size as usize).min(total_records);
    let page_items: Vec<LeaderboardItem> = if start >= total_records {
        Vec::new()
    } else {
        ranked[start..end]
            .iter()
            .enumerate()
            .map(|(i, (addr, tc, vol))| LeaderboardItem {
                rank: (start + i + 1) as u32,
                address: addr.clone(),
                trade_count: *tc,
                volume_1e8: vol.to_string(),
                realized_pnl_1e8: None,
            })
            .collect()
    };

    Ok(Json(Envelope {
        status: "ok",
        data: LeaderboardData {
            chain: chain_name_for(state.chain_id).to_string(),
            chain_id: state.chain_id,
            range: range.as_str().to_string(),
            page,
            page_size,
            total_records,
            items: page_items,
        },
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
) -> Result<Json<Envelope<ExercisePreviewData>>, TradingApiError> {
    let provider = rpc_provider_from_state(&state);
    exercise_preview_impl(&state, req, provider.as_ref()).await
}

pub(crate) async fn exercise_preview_impl<P>(
    state: &AppState,
    req: ExercisePreviewRequest,
    provider: Option<&P>,
) -> Result<Json<Envelope<ExercisePreviewData>>, TradingApiError>
where
    P: crate::execution::rpc::EthCallProvider,
{
    let acct = parse_address_or_400(state, &req.account)?;
    let series = get_option_series_service(state, &req.series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(state, "db"),
            )
        })?;
    let fills = list_option_fills_service(
        state,
        OptionFillFilter {
            option_series_id: Some(series.option_series_id.clone()),
            account: Some(acct.clone()),
            order_id: None,
        },
    )
    .await
    .map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InternalError,
            "unable to list option fills",
            MetaBlock::new(state, "internal"),
        )
    })?;
    let mut signed_size: i128 = 0;
    for f in &fills {
        let is_long = f.buyer == acct;
        let delta: i128 = if is_long {
            f.size_1e8 as i128
        } else {
            -(f.size_1e8 as i128)
        };
        signed_size = signed_size.saturating_add(delta);
    }
    let now_s = now_ms() / 1000;
    let is_past_expiry = (now_s as u64) >= series.expiry;
    let mut can_settle = is_past_expiry && signed_size != 0;
    let mut is_already_settled = false;
    let mut pnl: String = "0".to_string();
    let mut payable_from_settlement_sink: Option<String> = None;
    let mut insurance_preview: Option<String> = None;
    let mut collectible_from_trader_preview: Option<String> = None;
    let mut residual_bad_debt_preview: Option<String> = None;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut settlement_resolved = false;

    let lens_configured = state.trading_views.margin_engine_lens_address.is_some();
    let me_configured = state.trading_views.margin_engine_address.is_some();
    let provider_configured = provider.is_some();
    if lens_configured && me_configured && provider_configured {
        let p = provider.expect("checked");
        let me_acct = state
            .trading_views
            .margin_engine_address
            .as_ref()
            .expect("checked");
        let me_addr = address_from_account_str(&me_acct.0);
        let trader_addr = address_from_account_str(&acct.0);
        let option_id_u256 = series.onchain_series_id.as_deref().and_then(|s| {
            alloy_primitives::U256::from_str_radix(s.trim_start_matches("0x"), 16).ok()
        });
        match (me_addr, trader_addr, option_id_u256) {
            (Some(me), Some(trader), Some(option_id)) => {
                let from = &state.execution_config.executor_from_address;
                match crate::api::trading_views::try_preview_account_settlement(
                    &state.trading_views,
                    from,
                    me,
                    option_id,
                    trader,
                    p,
                )
                .await
                {
                    Ok(Some(bytes)) => match decode_settlement_preview(&bytes) {
                        Ok(decoded) => {
                            pnl = decoded.pnl.to_string();
                            payable_from_settlement_sink =
                                Some(decoded.payable_from_settlement_sink.to_string());
                            insurance_preview = Some(decoded.insurance_preview.to_string());
                            collectible_from_trader_preview =
                                Some(decoded.collectible_amount.to_string());
                            residual_bad_debt_preview =
                                Some(decoded.residual_bad_debt_preview.to_string());
                            is_already_settled = decoded.is_settled;
                            can_settle = decoded.can_settle;
                            settlement_resolved = true;
                        }
                        Err(e) => warnings.push(warning_settlement_preview_unavailable(&format!(
                            "SettlementPreview decode failed: {}",
                            sanitise_rpc_err(&e)
                        ))),
                    },
                    Ok(None) => warnings.push(warning_config_missing(
                        "margin_engine_lens_address vanished mid-read.",
                    )),
                    Err(e) => warnings.push(warning_settlement_preview_unavailable(&format!(
                        "MarginEngineLens.previewAccountSettlement failed: {}",
                        sanitise_rpc_err(&e)
                    ))),
                }
            }
            (_, _, None) => warnings.push(warning_source_unavailable(
                "series.onchain_series_id is missing or not a valid uint256.",
            )),
            _ => warnings.push(warning_source_unavailable(
                "margin engine or trader address is not a valid EVM address.",
            )),
        }
    } else {
        let mut reasons: Vec<&str> = Vec::new();
        if !lens_configured {
            reasons.push("OPTION_MARGIN_ENGINE_LENS_ADDRESS");
        }
        if !me_configured {
            reasons.push("OPTION_MARGIN_ENGINE_ADDRESS");
        }
        if !provider_configured {
            reasons.push("EXECUTION_RPC_URL");
        }
        warnings.push(warning_config_missing(&format!(
            "Settlement preview requires: {}.",
            reasons.join(", ")
        )));
    }
    if !settlement_resolved {
        warnings.push(warning_partial_preview());
    }

    let status: &'static str = if settlement_resolved { "ok" } else { "partial" };
    Ok(Json(Envelope {
        status,
        data: ExercisePreviewData {
            series_id: series.option_series_id.to_string(),
            account: req.account.clone(),
            is_already_settled,
            can_settle,
            pnl,
            payable_from_settlement_sink,
            insurance_preview,
            collectible_from_trader_preview,
            residual_bad_debt_preview,
        },
        warnings,
        meta: MetaBlock::new(state, if status == "ok" { "rpc" } else { "db" }),
    }))
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
) -> Result<Json<Envelope<QuotePreviewData>>, TradingApiError> {
    let provider = rpc_provider_from_state(&state);
    close_preview_impl(&state, req, provider.as_ref()).await
}

pub(crate) async fn close_preview_impl<P>(
    state: &AppState,
    req: ClosePreviewRequest,
    provider: Option<&P>,
) -> Result<Json<Envelope<QuotePreviewData>>, TradingApiError>
where
    P: crate::execution::rpc::EthCallProvider,
{
    parse_address_or_400(state, &req.account)?;
    if !["buy", "sell"].contains(&req.side.as_str()) {
        return Err(TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "side must be 'buy' or 'sell'",
            MetaBlock::new(state, "validation"),
        ));
    }
    let size: u128 = req.size.parse().map_err(|_| {
        TradingApiError::new(
            TradingErrorCode::InvalidRequest,
            "size must be a non-negative integer",
            MetaBlock::new(state, "validation"),
        )
    })?;
    let series = get_option_series_service(state, &req.series_id)
        .await
        .map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::SeriesNotFound,
                "Series id unknown",
                MetaBlock::new(state, "db"),
            )
        })?;
    if !matches!(series.status, crate::options::OptionSeriesStatus::Active) {
        return Err(TradingApiError::new(
            TradingErrorCode::QuoteUnsupported,
            "Series is not Active",
            MetaBlock::new(state, "db"),
        ));
    }
    let price_1e8: u128 = match req.price_1e8.as_deref() {
        Some(p) => p.parse().map_err(|_| {
            TradingApiError::new(
                TradingErrorCode::InvalidRequest,
                "price_1e8 must be a non-negative integer",
                MetaBlock::new(state, "validation"),
            )
        })?,
        None => series.strike_1e8,
    };
    let premium = size.saturating_mul(price_1e8);
    let fee_amount = premium.saturating_mul(PARTIAL_PREVIEW_TAKER_PPM as u128) / 1_000_000u128;
    let quote_expires_at_ms = now_ms() + 20_000;

    // M-P2e — read-only oracle mark when configured.
    let mut warnings: Vec<Warning> = Vec::new();
    let mut oracle_mark_1e8: Option<String> = None;
    if state.trading_views.oracle_router_address.is_some() {
        if let Some(p) = provider {
            let underlying_addr = address_from_account_str(&series.underlying);
            let settlement_addr = address_from_account_str(&series.settlement_asset);
            if let (Some(base), Some(quote)) = (underlying_addr, settlement_addr) {
                match crate::api::trading_views::try_get_oracle_price_safe(
                    &state.trading_views,
                    &state.execution_config.executor_from_address,
                    base,
                    quote,
                    p,
                )
                .await
                {
                    Ok(Some(mark)) => oracle_mark_1e8 = Some(mark.to_string()),
                    Ok(None) => {}
                    Err(e) => warnings.push(warning_oracle_unavailable(&format!(
                        "OracleRouter.getPriceSafe failed: {}",
                        sanitise_rpc_err(&e)
                    ))),
                }
            }
        }
    }
    warnings.push(warning_partial_preview());
    if oracle_mark_1e8.is_none() {
        warnings.push(warning_oracle_unavailable(
            "oracle_mark_1e8 unavailable (OracleRouter address or RPC not configured).",
        ));
    }

    Ok(Json(Envelope {
        status: "partial",
        data: QuotePreviewData {
            series_id: series.option_series_id.to_string(),
            side: req.side.clone(),
            size: size.to_string(),
            price_1e8: price_1e8.to_string(),
            premium: premium.to_string(),
            buyer_fee: FeeBreakdown {
                ppm_signed: PARTIAL_PREVIEW_TAKER_PPM,
                amount: fee_amount.to_string(),
            },
            seller_fee: FeeBreakdown {
                ppm_signed: PARTIAL_PREVIEW_TAKER_PPM,
                amount: fee_amount.to_string(),
            },
            settlement_asset: series.settlement_asset.clone(),
            oracle_mark_1e8,
            im_impact: None,
            free_collateral_after: None,
            quote_expires_at_ms,
            position_size_after: None,
        },
        warnings,
        meta: MetaBlock::new(state, "db"),
    }))
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

    // M-P2b: positions / portfolio / balances now return partial-real data.

    #[tokio::test]
    async fn positions_returns_empty_partial_for_default_state() {
        let s = test_state();
        let env = account_positions(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        )
        .await
        .expect("partial ok");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.data.positions.is_empty());
        assert_eq!(
            env.0.data.address,
            "0x1234567890abcdef1234567890abcdef12345678"
        );
        assert!(!env.0.warnings.is_empty());
    }

    #[tokio::test]
    async fn portfolio_returns_zero_partial_for_default_state() {
        let s = test_state();
        let env = account_portfolio(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        )
        .await
        .expect("partial ok");
        assert_eq!(env.0.status, "partial");
        assert_eq!(env.0.data.open_positions_count, Some(0));
        assert_eq!(env.0.data.total_notional, Some("0".to_string()));
        // Fields not yet wired remain None.
        assert!(env.0.data.equity.is_none());
        assert!(env.0.data.free_collateral.is_none());
    }

    #[tokio::test]
    async fn balances_returns_empty_partial_for_default_state() {
        let s = test_state();
        let env = account_balances(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        )
        .await
        .expect("partial ok");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.data.balances.is_empty());
        // The SOURCE_UNAVAILABLE warning must be present.
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "SOURCE_UNAVAILABLE_FIELD"));
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

    // FRONTEND-BACKEND-HISTORY-V1 unit tests.
    fn default_history_query() -> HistoryV2Query {
        HistoryV2Query {
            tab: None,
            range: None,
            page: None,
            page_size: None,
        }
    }

    #[tokio::test]
    async fn history_v2_rejects_invalid_address() {
        let s = test_state();
        let err = account_history_v2(
            State(s),
            Path("nope".to_string()),
            Query(default_history_query()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn history_v2_rejects_unknown_tab() {
        let s = test_state();
        let mut q = default_history_query();
        q.tab = Some("garbage".to_string());
        let err = account_history_v2(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            Query(q),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn history_v2_rejects_unknown_range() {
        let s = test_state();
        let mut q = default_history_query();
        q.range = Some("forever".to_string());
        let err = account_history_v2(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            Query(q),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn history_v2_defaults_resolve_cleanly() {
        let s = test_state();
        let env = account_history_v2(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            Query(default_history_query()),
        )
        .await
        .expect("history v2 ok");
        assert_eq!(env.0.status, "ok");
        assert_eq!(env.0.data.tab, "trades");
        assert_eq!(env.0.data.range, "last_month");
        assert_eq!(env.0.data.page, 1);
        assert_eq!(env.0.data.page_size, 100);
        assert_eq!(env.0.data.total_records, 0);
        assert!(env.0.data.items.is_empty());
        assert_eq!(env.0.data.chain, "anvil");
        assert_eq!(env.0.data.chain_id, 31337);
    }

    #[tokio::test]
    async fn history_v2_settlement_funding_interest_liquidations_are_empty() {
        let s = test_state();
        for t in ["settlement", "funding", "interest", "liquidations"] {
            let mut q = default_history_query();
            q.tab = Some(t.to_string());
            let env = account_history_v2(
                State(s.clone()),
                Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
                Query(q),
            )
            .await
            .expect("history v2 ok");
            assert_eq!(env.0.data.tab, t);
            assert_eq!(env.0.data.total_records, 0);
            assert!(env.0.data.items.is_empty());
        }
    }

    #[tokio::test]
    async fn history_v2_page_size_is_clamped_to_max_10000() {
        let s = test_state();
        let mut q = default_history_query();
        q.page_size = Some(99_999);
        let env = account_history_v2(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            Query(q),
        )
        .await
        .expect("history v2 ok");
        assert_eq!(env.0.data.page_size, 10_000);
    }

    #[tokio::test]
    async fn history_v2_page_zero_normalises_to_one() {
        let s = test_state();
        let mut q = default_history_query();
        q.page = Some(0);
        let env = account_history_v2(
            State(s),
            Path("0x1234567890abcdef1234567890abcdef12345678".to_string()),
            Query(q),
        )
        .await
        .expect("history v2 ok");
        assert_eq!(env.0.data.page, 1);
    }

    // FRONTEND-BACKEND-LEADERBOARD-V1 tests.
    fn default_leaderboard_query() -> LeaderboardQuery {
        LeaderboardQuery {
            range: None,
            page: None,
            page_size: None,
        }
    }

    #[tokio::test]
    async fn leaderboard_rejects_unknown_range() {
        let s = test_state();
        let mut q = default_leaderboard_query();
        q.range = Some("forever".to_string());
        let err = leaderboard(State(s), Query(q)).await.unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn leaderboard_defaults_resolve_cleanly_on_empty_state() {
        let s = test_state();
        let env = leaderboard(State(s), Query(default_leaderboard_query()))
            .await
            .expect("leaderboard ok");
        assert_eq!(env.0.status, "ok");
        assert_eq!(env.0.data.range, "last_month");
        assert_eq!(env.0.data.page, 1);
        assert_eq!(env.0.data.page_size, 100);
        assert_eq!(env.0.data.total_records, 0);
        assert!(env.0.data.items.is_empty());
        assert_eq!(env.0.data.chain, "anvil");
        assert_eq!(env.0.data.chain_id, 31337);
    }

    #[tokio::test]
    async fn leaderboard_page_size_is_clamped_to_max_10000() {
        let s = test_state();
        let mut q = default_leaderboard_query();
        q.page_size = Some(99_999);
        let env = leaderboard(State(s), Query(q))
            .await
            .expect("leaderboard ok");
        assert_eq!(env.0.data.page_size, 10_000);
    }

    #[tokio::test]
    async fn leaderboard_page_zero_normalises_to_one() {
        let s = test_state();
        let mut q = default_leaderboard_query();
        q.page = Some(0);
        let env = leaderboard(State(s), Query(q))
            .await
            .expect("leaderboard ok");
        assert_eq!(env.0.data.page, 1);
    }

    #[tokio::test]
    async fn leaderboard_aggregates_buyer_and_seller_volumes() {
        use crate::options::OptionFill;
        use crate::types::{AccountId, OrderId, Side};
        use uuid::Uuid;
        let s = test_state();
        let alice = AccountId::new("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string());
        let bob = AccountId::new("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string());
        let carol = AccountId::new("0xcccccccccccccccccccccccccccccccccccccccc".to_string());
        // Seed two fills: (alice buyer, bob seller) and (carol buyer, alice seller).
        // Both at price=100_00000000 (i.e. $100), size=2_00000000 (i.e. 2.0
        // contracts) → notional = price * size / 1e8 = 200_00000000.
        let now = now_ms();
        let oid = || OrderId(Uuid::new_v4());
        let fills = vec![
            OptionFill {
                fill_id: Uuid::new_v4(),
                option_series_id: "S-1".to_string(),
                buy_order_id: oid(),
                sell_order_id: oid(),
                buyer: alice.clone(),
                seller: bob.clone(),
                maker_order_id: oid(),
                taker_order_id: oid(),
                taker_side: Side::Buy,
                price_1e8: 100_00000000,
                size_1e8: 2_00000000,
                created_at_ms: now,
            },
            OptionFill {
                fill_id: Uuid::new_v4(),
                option_series_id: "S-1".to_string(),
                buy_order_id: oid(),
                sell_order_id: oid(),
                buyer: carol.clone(),
                seller: alice.clone(),
                maker_order_id: oid(),
                taker_order_id: oid(),
                taker_side: Side::Buy,
                price_1e8: 100_00000000,
                size_1e8: 2_00000000,
                created_at_ms: now,
            },
        ];
        // Inject fills directly into the in-memory store. This is the
        // same path the local seed harness uses for trading tests.
        {
            let mut store = s.options_store.lock().expect("lock");
            for f in fills {
                store.insert_fill_for_test(f);
            }
        }
        let env = leaderboard(State(s), Query(default_leaderboard_query()))
            .await
            .expect("leaderboard ok");
        assert_eq!(env.0.data.total_records, 3);
        let items = &env.0.data.items;
        // Alice participated in BOTH fills → 2 trades, volume = 2 × 200_00000000.
        assert_eq!(items[0].rank, 1);
        assert_eq!(items[0].address, alice.0.to_ascii_lowercase());
        assert_eq!(items[0].trade_count, 2);
        assert_eq!(items[0].volume_1e8, "40000000000");
        // Bob and Carol each participated in 1 fill, same notional —
        // tie-broken by address (asc).
        assert_eq!(items[1].rank, 2);
        assert_eq!(items[2].rank, 3);
        for it in &items[1..] {
            assert_eq!(it.trade_count, 1);
            assert_eq!(it.volume_1e8, "20000000000");
        }
        // realized_pnl_1e8 is skipped (None) in V1.
        for it in items {
            assert!(it.realized_pnl_1e8.is_none());
        }
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

    // ---------------------- M-P2b NEW TESTS ----------------------

    #[tokio::test]
    async fn quote_preview_returns_partial_for_unknown_series() {
        let s = test_state();
        // Series 42 doesn't exist in default-state store → SeriesNotFound.
        let err = quote_preview(
            State(s),
            Query(QuotePreviewQuery {
                series_id: "42".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: None,
                account: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::SeriesNotFound);
    }

    #[tokio::test]
    async fn quote_preview_rejects_bad_price() {
        let s = seeded_state(true);
        let err = quote_preview(
            State(s),
            Query(QuotePreviewQuery {
                series_id: "S-1".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: Some("not-a-number".to_string()),
                account: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn quote_preview_returns_partial_for_active_series() {
        let s = seeded_state(true);
        let env = quote_preview(
            State(s),
            Query(QuotePreviewQuery {
                series_id: "S-1".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: None,
                account: None,
            }),
        )
        .await
        .expect("partial ok");
        assert_eq!(env.0.status, "partial");
        assert_eq!(env.0.data.series_id, "S-1");
        assert_eq!(env.0.data.side, "buy");
        assert_eq!(env.0.data.size, "1");
        // ppm matches the documented partial assumption.
        assert_eq!(env.0.data.buyer_fee.ppm_signed, PARTIAL_PREVIEW_TAKER_PPM);
        // PARTIAL_PREVIEW warning present.
        assert!(env.0.warnings.iter().any(|w| w.code == "PARTIAL_PREVIEW"));
    }

    #[tokio::test]
    async fn quote_preview_refuses_inactive_series() {
        let s = seeded_state(false); // inactive
        let err = quote_preview(
            State(s),
            Query(QuotePreviewQuery {
                series_id: "S-1".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: None,
                account: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::QuoteUnsupported);
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn close_preview_returns_partial_for_active_series() {
        let s = seeded_state(true);
        let env = close_preview(
            State(s),
            Json(ClosePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                side: "sell".to_string(),
                size: "1".to_string(),
                price_1e8: None,
            }),
        )
        .await
        .expect("partial ok");
        assert_eq!(env.0.status, "partial");
        assert_eq!(env.0.data.series_id, "S-1");
        assert_eq!(env.0.data.side, "sell");
    }

    #[tokio::test]
    async fn close_preview_refuses_inactive_series() {
        let s = seeded_state(false);
        let err = close_preview(
            State(s),
            Json(ClosePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::QuoteUnsupported);
    }

    #[tokio::test]
    async fn exercise_preview_returns_partial_for_active_series() {
        let s = seeded_state(true);
        let env = exercise_preview(
            State(s),
            Json(ExercisePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            }),
        )
        .await
        .expect("partial ok");
        assert_eq!(env.0.status, "partial");
        assert!(!env.0.data.is_already_settled);
        // Default-state: no fills → signed_size = 0 → can_settle = false.
        assert!(!env.0.data.can_settle);
        assert!(env.0.warnings.iter().any(|w| w.code == "PARTIAL_PREVIEW"));
    }

    #[tokio::test]
    async fn exercise_preview_unknown_series_yields_series_not_found() {
        let s = test_state();
        let err = exercise_preview(
            State(s),
            Json(ExercisePreviewRequest {
                series_id: "missing".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::SeriesNotFound);
    }

    #[tokio::test]
    async fn preview_endpoints_do_not_call_signer_or_broadcast() {
        // Smoke: handlers must not touch the broadcast_observability
        // counters that the signer/executor would increment. We assert
        // pre/post equality.
        let s = seeded_state(true);
        let before = s.broadcast_observability.snapshot();
        let _ = quote_preview(
            State(s.clone()),
            Query(QuotePreviewQuery {
                series_id: "S-1".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: None,
                account: None,
            }),
        )
        .await;
        let _ = close_preview(
            State(s.clone()),
            Json(ClosePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                side: "sell".to_string(),
                size: "1".to_string(),
                price_1e8: None,
            }),
        )
        .await;
        let _ = exercise_preview(
            State(s.clone()),
            Json(ExercisePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            }),
        )
        .await;
        let after = s.broadcast_observability.snapshot();
        // The signer-attempted/success/denied counter family MUST be
        // unchanged — preview handlers must NEVER touch it.
        assert_eq!(
            format!("{:?}", before.signer_attempted_total),
            format!("{:?}", after.signer_attempted_total)
        );
        assert_eq!(
            format!("{:?}", before.signer_success_total),
            format!("{:?}", after.signer_success_total)
        );
        assert_eq!(
            format!("{:?}", before.signer_denied_total),
            format!("{:?}", after.signer_denied_total)
        );
    }

    #[tokio::test]
    async fn response_bodies_do_not_leak_secrets() {
        // For all 6 implemented endpoints, serialize the success body
        // and assert no sensitive strings appear.
        let s = seeded_state(true);
        let addr = "0x1234567890abcdef1234567890abcdef12345678".to_string();
        let positions = account_positions(State(s.clone()), Path(addr.clone()))
            .await
            .expect("partial");
        let portfolio = account_portfolio(State(s.clone()), Path(addr.clone()))
            .await
            .expect("partial");
        let balances = account_balances(State(s.clone()), Path(addr.clone()))
            .await
            .expect("partial");
        let quote = quote_preview(
            State(s.clone()),
            Query(QuotePreviewQuery {
                series_id: "S-1".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: None,
                account: None,
            }),
        )
        .await
        .expect("partial");
        let exercise = exercise_preview(
            State(s.clone()),
            Json(ExercisePreviewRequest {
                series_id: "S-1".to_string(),
                account: addr.clone(),
            }),
        )
        .await
        .expect("partial");
        let close = close_preview(
            State(s.clone()),
            Json(ClosePreviewRequest {
                series_id: "S-1".to_string(),
                account: addr.clone(),
                side: "sell".to_string(),
                size: "1".to_string(),
                price_1e8: None,
            }),
        )
        .await
        .expect("partial");
        for body in [
            serde_json::to_string(&positions.0).unwrap(),
            serde_json::to_string(&portfolio.0).unwrap(),
            serde_json::to_string(&balances.0).unwrap(),
            serde_json::to_string(&quote.0).unwrap(),
            serde_json::to_string(&exercise.0).unwrap(),
            serde_json::to_string(&close.0).unwrap(),
        ] {
            assert!(!body.contains("EXECUTOR_PRIVATE_KEY"), "{body}");
            assert!(!body.contains("DATABASE_URL"), "{body}");
            assert!(!body.contains("AWS_"), "{body}");
            assert!(!body.contains("aws_kms"), "{body}");
            assert!(!body.contains("signer_mode"), "{body}");
        }
    }

    // ----- M-P2d — account_balances_impl with provider injection -----

    use crate::api::trading_views::tests::ProgrammableMockProvider;
    use crate::api::trading_views::TradingViewsConfig;

    #[tokio::test]
    async fn balances_impl_fully_unconfigured_falls_back_to_partial() {
        let s = test_state();
        let env = account_balances_impl::<ProgrammableMockProvider>(
            &s,
            "0x1234567890abcdef1234567890abcdef12345678",
            None,
        )
        .await
        .expect("partial");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.data.balances.is_empty());
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "SOURCE_UNAVAILABLE_FIELD"));
    }

    #[tokio::test]
    async fn balances_impl_invalid_address_rejected() {
        let s = test_state();
        let err = account_balances_impl::<ProgrammableMockProvider>(&s, "nope", None)
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn balances_impl_configured_addresses_no_provider_still_partial() {
        let mut s = test_state();
        s.trading_views = TradingViewsConfig {
            collateral_vault_views_address: Some(AccountId::new(
                "0x0000000000000000000000000000000000000aaa",
            )),
            collateral_vault_address: Some(AccountId::new(
                "0x0000000000000000000000000000000000000bbb",
            )),
            ..Default::default()
        };
        let env = account_balances_impl::<ProgrammableMockProvider>(
            &s,
            "0x1234567890abcdef1234567890abcdef12345678",
            None,
        )
        .await
        .expect("partial");
        assert_eq!(env.0.status, "partial");
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| { w.message.contains("RPC provider") || w.message.contains("RPC_URL") }));
    }

    #[tokio::test]
    async fn balances_impl_configured_returns_ok_when_provider_yields_tokens() {
        use alloy_primitives::{Address, U256};
        use alloy_sol_types::SolValue;
        let mut s = test_state();
        s.trading_views = TradingViewsConfig {
            collateral_vault_views_address: Some(AccountId::new(
                "0x0000000000000000000000000000000000000aaa",
            )),
            collateral_vault_address: Some(AccountId::new(
                "0x0000000000000000000000000000000000000bbb",
            )),
            ..Default::default()
        };
        let tokens: Vec<Address> = vec![Address::from([1u8; 20])];
        let token_list_encoded = SolValue::abi_encode(&tokens);
        let balance_encoded = SolValue::abi_encode(&U256::from(1_000_000u64));
        let mock = ProgrammableMockProvider::new();
        mock.returns([0xb5, 0x8e, 0xb6, 0x3f], token_list_encoded);
        mock.returns([0xc2, 0x3f, 0x00, 0x1f], balance_encoded);
        let env = account_balances_impl(
            &s,
            "0x1234567890abcdef1234567890abcdef12345678",
            Some(&mock),
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "ok");
        assert_eq!(env.0.data.balances.len(), 1);
        assert_eq!(env.0.data.balances[0].balance, "1000000");
        assert!(env.0.warnings.is_empty());
    }

    #[tokio::test]
    async fn balances_impl_rpc_failure_falls_back_to_partial_with_sanitised_warning() {
        let mut s = test_state();
        s.trading_views = TradingViewsConfig {
            collateral_vault_views_address: Some(AccountId::new(
                "0x0000000000000000000000000000000000000aaa",
            )),
            collateral_vault_address: Some(AccountId::new(
                "0x0000000000000000000000000000000000000bbb",
            )),
            ..Default::default()
        };
        let mock = ProgrammableMockProvider::new();
        mock.fails([0xb5, 0x8e, 0xb6, 0x3f]); // Token list fetch fails.
        let env = account_balances_impl(
            &s,
            "0x1234567890abcdef1234567890abcdef12345678",
            Some(&mock),
        )
        .await
        .expect("partial");
        assert_eq!(env.0.status, "partial");
        let w = env
            .0
            .warnings
            .iter()
            .find(|w| w.code == "RPC_UNAVAILABLE")
            .expect("RPC_UNAVAILABLE warning");
        // No raw http(s):// URL in the message.
        assert!(!w.message.contains("http://"));
        assert!(!w.message.contains("https://"));
    }

    #[tokio::test]
    async fn sanitise_rpc_err_strips_urls() {
        let s =
            sanitise_rpc_err("connect failed to http://secret-rpc.example.com:8545/abc more text");
        assert!(s.contains("<url-redacted>"));
        assert!(!s.contains("secret-rpc"));
    }

    #[tokio::test]
    async fn sanitise_rpc_err_strips_https_too() {
        let s =
            sanitise_rpc_err("eth_call rejected at https://provider.example.invalid/v1/key extra");
        assert!(s.contains("<url-redacted>"));
        assert!(!s.contains("provider.example"));
    }

    #[tokio::test]
    async fn sanitise_rpc_err_caps_length() {
        let big = "x".repeat(500);
        let s = sanitise_rpc_err(&big);
        assert!(s.len() <= 203); // 200 + ellipsis bytes
    }

    // -- Test helpers --

    // Minimal AppState. Re-uses the same constructor as the
    // executor_health_v2 tests at src/api/executor_health_v2.rs:base_state.
    fn test_state() -> AppState {
        use crate::engine::EngineState;
        let mut state = AppState::new(EngineState::new(Vec::new()));
        state.chain_id = 31337;
        state.network_name = "anvil".to_string();
        state.options_config.enabled = true;
        state
    }

    // Seeded state with one OptionSeries (id "S-1"; Active or Expired
    // controlled by `active`). Used by preview tests.
    fn seeded_state(active: bool) -> AppState {
        use crate::options::{OptionSeriesSource, OptionSeriesStatus};
        let s = test_state();
        let now_ms_v = now_ms();
        let series = OptionSeries {
            option_series_id: "S-1".to_string(),
            underlying: "0xabc".to_string(),
            base_asset: "0xabc".to_string(),
            quote_asset: "0xdef".to_string(),
            settlement_asset: "0xdef".to_string(),
            expiry: (now_ms_v / 1000) as u64 + 86400,
            strike_1e8: 200_000_000_000u128,
            is_call: true,
            contract_size_1e8: 100_000_000u128,
            status: if active {
                OptionSeriesStatus::Active
            } else {
                OptionSeriesStatus::Expired
            },
            source: OptionSeriesSource::Manual,
            onchain_product_id: None,
            onchain_series_id: None,
            created_at_ms: now_ms_v,
            updated_at_ms: now_ms_v,
        };
        let mut store = s.options_store.lock().unwrap();
        store.insert_series(series);
        drop(store);
        s
    }

    /// M-P2e — seeded state with a series whose underlying / settlement
    /// addresses are real 0x40-hex strings (so the trading_views
    /// helpers can parse them), and whose onchain_series_id is a
    /// valid 0x-prefixed uint256.
    fn seeded_state_with_addresses() -> AppState {
        use crate::options::{OptionSeriesSource, OptionSeriesStatus};
        let s = test_state();
        let now_ms_v = now_ms();
        let series = OptionSeries {
            option_series_id: "S-1".to_string(),
            underlying: "0x1111111111111111111111111111111111111111".to_string(),
            base_asset: "0x1111111111111111111111111111111111111111".to_string(),
            quote_asset: "0x2222222222222222222222222222222222222222".to_string(),
            settlement_asset: "0x2222222222222222222222222222222222222222".to_string(),
            expiry: (now_ms_v / 1000) as u64 + 86400,
            strike_1e8: 200_000_000_000u128,
            is_call: true,
            contract_size_1e8: 100_000_000u128,
            status: OptionSeriesStatus::Active,
            source: OptionSeriesSource::Manual,
            onchain_product_id: None,
            onchain_series_id: Some(format!("0x{:064x}", 7u64)),
            created_at_ms: now_ms_v,
            updated_at_ms: now_ms_v,
        };
        let mut store = s.options_store.lock().unwrap();
        store.insert_series(series);
        drop(store);
        s
    }

    fn tv_config_full() -> TradingViewsConfig {
        TradingViewsConfig {
            margin_engine_lens_address: Some(AccountId::new(
                "0x3333333333333333333333333333333333333333",
            )),
            collateral_vault_views_address: None,
            collateral_vault_address: None,
            oracle_router_address: Some(AccountId::new(
                "0x4444444444444444444444444444444444444444",
            )),
            margin_engine_address: Some(AccountId::new(
                "0x5555555555555555555555555555555555555555",
            )),
        }
    }

    fn encode_account_state(equity: i64, mm: u128, im: u128, free: i64) -> Vec<u8> {
        use alloy_primitives::{I256, U256};
        use alloy_sol_types::SolValue;
        let tup: (I256, U256, U256, I256, U256, U256, U256, bool) = (
            I256::try_from(equity).unwrap(),
            U256::from(mm),
            U256::from(im),
            I256::try_from(free).unwrap(),
            U256::from(0u64),
            U256::from(0u64),
            U256::from(0u64),
            false,
        );
        SolValue::abi_encode(&tup)
    }

    fn encode_settlement_preview(
        pnl: i64,
        collectible: u128,
        payable: u128,
        insurance: u128,
        residual: u128,
        is_settled: bool,
        can_settle: bool,
    ) -> Vec<u8> {
        use alloy_primitives::{I256, U256};
        use alloy_sol_types::SolValue;
        let tup: (I256, U256, U256, U256, U256, U256, bool, bool) = (
            I256::try_from(pnl).unwrap(),
            U256::from(0u64),
            U256::from(collectible),
            U256::from(payable),
            U256::from(insurance),
            U256::from(residual),
            is_settled,
            can_settle,
        );
        SolValue::abi_encode(&tup)
    }

    // ---------------------- M-P2e PHASE 5 TESTS ----------------------

    // ----- series_details_impl -----

    #[tokio::test]
    async fn series_details_impl_no_provider_returns_partial() {
        let s = seeded_state_with_addresses();
        let env = series_details_impl::<ProgrammableMockProvider>(&s, "S-1", None)
            .await
            .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.data.oracle_mark_1e8.is_none());
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "CONFIG_MISSING" || w.code == "RPC_UNAVAILABLE"));
    }

    #[tokio::test]
    async fn series_details_impl_oracle_configured_returns_ok_with_mark() {
        use alloy_primitives::U256;
        use alloy_sol_types::SolValue;
        let mut s = seeded_state_with_addresses();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        let encoded_price = SolValue::abi_encode(&U256::from(3_500_000_000_000u128));
        mock.returns([0x63, 0x85, 0x1e, 0xa3], encoded_price);
        let env = series_details_impl(&s, "S-1", Some(&mock))
            .await
            .expect("ok");
        assert_eq!(env.0.status, "ok");
        assert_eq!(env.0.data.oracle_mark_1e8.as_deref(), Some("3500000000000"));
        assert_eq!(env.0.meta.source, "rpc");
    }

    #[tokio::test]
    async fn series_details_impl_oracle_revert_falls_back_to_partial() {
        let mut s = seeded_state_with_addresses();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        mock.fails([0x63, 0x85, 0x1e, 0xa3]);
        let env = series_details_impl(&s, "S-1", Some(&mock))
            .await
            .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "ORACLE_UNAVAILABLE"));
    }

    #[tokio::test]
    async fn series_details_impl_unknown_series_is_404() {
        let s = test_state();
        let err = series_details_impl::<ProgrammableMockProvider>(&s, "S-UNKNOWN", None)
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::SeriesNotFound);
    }

    // ----- quote_preview_impl -----

    #[tokio::test]
    async fn quote_preview_impl_no_provider_returns_partial_oracle_warning() {
        let s = seeded_state_with_addresses();
        let env = quote_preview_impl::<ProgrammableMockProvider>(
            &s,
            QuotePreviewQuery {
                series_id: "S-1".to_string(),
                side: "buy".to_string(),
                size: "1".to_string(),
                price_1e8: None,
                account: None,
            },
            None,
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.data.oracle_mark_1e8.is_none());
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "ORACLE_UNAVAILABLE"));
    }

    #[tokio::test]
    async fn quote_preview_impl_oracle_configured_populates_oracle_mark() {
        use alloy_primitives::U256;
        use alloy_sol_types::SolValue;
        let mut s = seeded_state_with_addresses();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        mock.returns(
            [0x63, 0x85, 0x1e, 0xa3],
            SolValue::abi_encode(&U256::from(123_456_789_000u128)),
        );
        let env = quote_preview_impl(
            &s,
            QuotePreviewQuery {
                series_id: "S-1".to_string(),
                side: "sell".to_string(),
                size: "5".to_string(),
                price_1e8: None,
                account: None,
            },
            Some(&mock),
        )
        .await
        .expect("ok");
        assert_eq!(env.0.data.oracle_mark_1e8.as_deref(), Some("123456789000"));
    }

    // ----- account_positions_impl -----

    #[tokio::test]
    async fn account_positions_impl_no_oracle_returns_partial_with_config_warning() {
        let s = seeded_state_with_addresses();
        let env = account_positions_impl::<ProgrammableMockProvider>(
            &s,
            "0x1234567890abcdef1234567890abcdef12345678",
            None,
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "CONFIG_MISSING" || w.code == "RPC_UNAVAILABLE"));
    }

    // ----- account_portfolio_impl -----

    #[tokio::test]
    async fn account_portfolio_impl_no_provider_returns_partial_config_missing() {
        let s = test_state();
        let env = account_portfolio_impl::<ProgrammableMockProvider>(
            &s,
            "0x1234567890abcdef1234567890abcdef12345678",
            None,
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.data.equity.is_none());
        assert!(env.0.warnings.iter().any(|w| w.code == "CONFIG_MISSING"));
    }

    #[tokio::test]
    async fn account_portfolio_impl_lens_configured_returns_ok_with_equity() {
        let mut s = test_state();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        let encoded = encode_account_state(1_000_000, 10_000, 25_000, 500_000);
        mock.returns([0xa5, 0x7b, 0xd4, 0xcc], encoded);
        let env = account_portfolio_impl(
            &s,
            "0x6666666666666666666666666666666666666666",
            Some(&mock),
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "ok");
        assert_eq!(env.0.data.equity.as_deref(), Some("1000000"));
        assert_eq!(env.0.data.mm.as_deref(), Some("10000"));
        assert_eq!(env.0.data.im.as_deref(), Some("25000"));
        assert_eq!(env.0.data.free_collateral.as_deref(), Some("500000"));
    }

    #[tokio::test]
    async fn account_portfolio_impl_lens_revert_falls_back_to_partial() {
        let mut s = test_state();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        mock.fails([0xa5, 0x7b, 0xd4, 0xcc]);
        let env = account_portfolio_impl(
            &s,
            "0x6666666666666666666666666666666666666666",
            Some(&mock),
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "ACCOUNT_STATE_UNAVAILABLE"));
    }

    #[tokio::test]
    async fn account_portfolio_impl_invalid_address_rejected() {
        let s = test_state();
        let err = account_portfolio_impl::<ProgrammableMockProvider>(&s, "not-an-address", None)
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    // ----- exercise_preview_impl -----

    #[tokio::test]
    async fn exercise_preview_impl_no_provider_returns_partial() {
        let s = seeded_state_with_addresses();
        let env = exercise_preview_impl::<ProgrammableMockProvider>(
            &s,
            ExercisePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            },
            None,
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.warnings.iter().any(|w| w.code == "CONFIG_MISSING"));
    }

    #[tokio::test]
    async fn exercise_preview_impl_settlement_populates_when_lens_returns_ok() {
        let mut s = seeded_state_with_addresses();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        let encoded = encode_settlement_preview(42_000, 5_000, 3_000, 1_000, 500, false, true);
        mock.returns([0xe8, 0x02, 0x99, 0xc3], encoded);
        let env = exercise_preview_impl(
            &s,
            ExercisePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            },
            Some(&mock),
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "ok");
        assert_eq!(env.0.data.pnl, "42000");
        assert_eq!(
            env.0.data.collectible_from_trader_preview.as_deref(),
            Some("5000")
        );
        assert_eq!(
            env.0.data.payable_from_settlement_sink.as_deref(),
            Some("3000")
        );
        assert_eq!(env.0.data.insurance_preview.as_deref(), Some("1000"));
        assert_eq!(env.0.data.residual_bad_debt_preview.as_deref(), Some("500"));
        assert!(!env.0.data.is_already_settled);
        assert!(env.0.data.can_settle);
    }

    #[tokio::test]
    async fn exercise_preview_impl_lens_failure_falls_back_to_partial() {
        let mut s = seeded_state_with_addresses();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        mock.fails([0xe8, 0x02, 0x99, 0xc3]);
        let env = exercise_preview_impl(
            &s,
            ExercisePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            },
            Some(&mock),
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "SETTLEMENT_PREVIEW_UNAVAILABLE"));
    }

    #[tokio::test]
    async fn exercise_preview_impl_invalid_account_rejected() {
        let s = seeded_state_with_addresses();
        let err = exercise_preview_impl::<ProgrammableMockProvider>(
            &s,
            ExercisePreviewRequest {
                series_id: "S-1".to_string(),
                account: "nope".to_string(),
            },
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    // ----- close_preview_impl -----

    #[tokio::test]
    async fn close_preview_impl_no_provider_returns_partial_oracle_warning() {
        let s = seeded_state_with_addresses();
        let env = close_preview_impl::<ProgrammableMockProvider>(
            &s,
            ClosePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                side: "buy".to_string(),
                size: "2".to_string(),
                price_1e8: None,
            },
            None,
        )
        .await
        .expect("ok");
        assert_eq!(env.0.status, "partial");
        assert!(env.0.data.oracle_mark_1e8.is_none());
        assert!(env
            .0
            .warnings
            .iter()
            .any(|w| w.code == "ORACLE_UNAVAILABLE"));
    }

    #[tokio::test]
    async fn close_preview_impl_oracle_configured_populates_mark() {
        use alloy_primitives::U256;
        use alloy_sol_types::SolValue;
        let mut s = seeded_state_with_addresses();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        mock.returns(
            [0x63, 0x85, 0x1e, 0xa3],
            SolValue::abi_encode(&U256::from(999_999_999u128)),
        );
        let env = close_preview_impl(
            &s,
            ClosePreviewRequest {
                series_id: "S-1".to_string(),
                account: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                side: "sell".to_string(),
                size: "1".to_string(),
                price_1e8: None,
            },
            Some(&mock),
        )
        .await
        .expect("ok");
        assert_eq!(env.0.data.oracle_mark_1e8.as_deref(), Some("999999999"));
    }

    // ----- decoder tests -----

    #[test]
    fn decode_account_state_roundtrips() {
        let encoded = encode_account_state(-100, 200, 300, -400);
        let d = decode_account_state(&encoded).expect("decode ok");
        assert_eq!(d.equity_base.to_string(), "-100");
        assert_eq!(d.maintenance_margin_base.to_string(), "200");
        assert_eq!(d.initial_margin_base.to_string(), "300");
        assert_eq!(d.free_collateral_base.to_string(), "-400");
    }

    #[test]
    fn decode_account_state_rejects_garbage() {
        let out = decode_account_state(&[0u8; 4]);
        assert!(out.is_err());
    }

    #[test]
    fn decode_settlement_preview_roundtrips() {
        let encoded = encode_settlement_preview(-7, 1, 2, 3, 4, true, false);
        let d = decode_settlement_preview(&encoded).expect("decode ok");
        assert_eq!(d.pnl.to_string(), "-7");
        assert_eq!(d.collectible_amount.to_string(), "1");
        assert_eq!(d.payable_from_settlement_sink.to_string(), "2");
        assert_eq!(d.insurance_preview.to_string(), "3");
        assert_eq!(d.residual_bad_debt_preview.to_string(), "4");
        assert!(d.is_settled);
        assert!(!d.can_settle);
    }

    // ----- secret-leak guards -----

    #[tokio::test]
    async fn series_details_response_never_leaks_secrets() {
        let mut s = seeded_state_with_addresses();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        mock.fails([0x63, 0x85, 0x1e, 0xa3]);
        let env = series_details_impl(&s, "S-1", Some(&mock))
            .await
            .expect("ok");
        let body = serde_json::to_string(&env.0).unwrap();
        for forbidden in [
            "EXECUTOR_PRIVATE_KEY",
            "DATABASE_URL",
            "AWS_ACCESS_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "arn:aws:kms",
            "Bearer ",
            "http://",
            "https://",
        ] {
            assert!(
                !body.contains(forbidden),
                "series_details body leaked '{forbidden}': {body}"
            );
        }
    }

    #[tokio::test]
    async fn portfolio_response_never_leaks_secrets_on_rpc_failure() {
        let mut s = test_state();
        s.trading_views = tv_config_full();
        let mock = ProgrammableMockProvider::new();
        mock.fails([0xa5, 0x7b, 0xd4, 0xcc]);
        let env = account_portfolio_impl(
            &s,
            "0x6666666666666666666666666666666666666666",
            Some(&mock),
        )
        .await
        .expect("ok");
        let body = serde_json::to_string(&env.0).unwrap();
        for forbidden in [
            "EXECUTOR_PRIVATE_KEY",
            "DATABASE_URL",
            "AWS_ACCESS_KEY",
            "arn:aws:kms",
            "http://",
            "https://",
        ] {
            assert!(!body.contains(forbidden), "leak: {forbidden}\n{body}");
        }
    }

    // ---------------------- M-P2f CREATE-INTENT TESTS ----------------------

    /// Seeded state with options execution enabled + a series whose
    /// onchain_series_id is the actual `option_product_registry_option_id`
    /// computed against the seeded metadata. The matching engine
    /// address is set to a deterministic non-zero placeholder so the
    /// nonce-sync precheck (disabled by default) doesn't fail.
    fn seeded_state_for_create_intent() -> AppState {
        use crate::options::{OptionSeriesSource, OptionSeriesStatus};
        let mut s = test_state();
        // Enable options execution so the service path is reachable.
        s.options_config.execution_enabled = true;
        s.options_config.execution_require_persistence = false;
        s.options_config.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        s.options_config.execution_eip712_domain.verifying_contract =
            s.options_config.matching_engine_address.clone();

        let now_ms_v = now_ms();
        let underlying = AccountId::new("0x1111111111111111111111111111111111111111");
        let settlement = AccountId::new("0x2222222222222222222222222222222222222222");
        let expiry = (now_ms_v / 1000) as u64 + 86400;
        let strike_1e8: u64 = 200_000_000_000;
        let contract_size_1e8: u128 = 100_000_000;
        let is_call = true;
        // Compute the real option_id (european variant).
        let option_id = crate::options::option_product_registry_option_id(
            &underlying,
            &settlement,
            expiry,
            strike_1e8,
            contract_size_1e8,
            is_call,
            true,
        )
        .expect("option_id compute");
        let series = OptionSeries {
            option_series_id: "S-1".to_string(),
            underlying: underlying.0.clone(),
            base_asset: underlying.0.clone(),
            quote_asset: settlement.0.clone(),
            settlement_asset: settlement.0.clone(),
            expiry,
            strike_1e8: strike_1e8 as u128,
            is_call,
            contract_size_1e8,
            status: OptionSeriesStatus::Active,
            source: OptionSeriesSource::Manual,
            onchain_product_id: None,
            onchain_series_id: Some(option_id.to_string()),
            created_at_ms: now_ms_v,
            updated_at_ms: now_ms_v,
        };
        let mut store = s.options_store.lock().unwrap();
        store.insert_series(series);
        drop(store);
        s
    }

    fn valid_create_request() -> CreateExecutionIntentRequest {
        CreateExecutionIntentRequest {
            series_id: "S-1".to_string(),
            side: "buy".to_string(),
            size_1e8: "100000000".to_string(),
            price_1e8: "10000".to_string(),
            buyer: Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            seller: Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            quote_id: None,
            client_request_id: None,
        }
    }

    #[tokio::test]
    async fn create_intent_success_returns_intent_id_and_request_id() {
        let s = seeded_state_for_create_intent();
        let env = create_execution_intent(State(s), Json(valid_create_request()))
            .await
            .expect("ok");
        let data = &env.0;
        assert!(!data.intent_id.is_empty());
        assert!(!data.request_id.is_empty());
        assert_eq!(data.status, "signatures_required");
        assert!(data.signing_payload_available);
        assert_eq!(data.next_step, "request_signing_payload");
        assert!(data.expires_at_ms.is_none());
    }

    #[tokio::test]
    async fn create_intent_echoes_client_request_id_when_provided() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.client_request_id = Some("my-correlation-id".to_string());
        let env = create_execution_intent(State(s), Json(req))
            .await
            .expect("ok");
        assert_eq!(env.0.request_id, "my-correlation-id");
    }

    #[tokio::test]
    async fn create_intent_rejects_invalid_side() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.side = "diagonal".to_string();
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_intent_rejects_zero_size() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.size_1e8 = "0".to_string();
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn create_intent_rejects_zero_price() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.price_1e8 = "0".to_string();
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn create_intent_rejects_non_numeric_size() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.size_1e8 = "lots".to_string();
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn create_intent_rejects_missing_buyer() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.buyer = None;
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn create_intent_rejects_missing_seller() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.seller = None;
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn create_intent_rejects_malformed_buyer_address() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.buyer = Some("nope".to_string());
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn create_intent_rejects_malformed_seller_address() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.seller = Some("0xZZZ".to_string());
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidAddress);
    }

    #[tokio::test]
    async fn create_intent_rejects_self_trade() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.seller = req.buyer.clone();
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn create_intent_unknown_series_returns_series_not_found() {
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.series_id = "S-DOES-NOT-EXIST".to_string();
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::SeriesNotFound);
    }

    #[tokio::test]
    async fn create_intent_inactive_series_returns_quote_unsupported() {
        use crate::options::OptionSeriesStatus;
        let s = seeded_state_for_create_intent();
        // Force the seeded series into Disabled state via the
        // store's mutating helper (insert_series is no-op when the
        // series_id already exists).
        {
            let mut store = s.options_store.lock().unwrap();
            let _ = OptionSeriesStatus::Disabled;
            store
                .disable_series("S-1", crate::types::now_ms())
                .expect("disable");
        }
        let err = create_execution_intent(State(s), Json(valid_create_request()))
            .await
            .unwrap_err();
        assert_eq!(err.code, TradingErrorCode::QuoteUnsupported);
    }

    #[tokio::test]
    async fn create_intent_persists_intent_via_signing_payload_consumer() {
        // The created intent_id must be visible to the existing
        // get_option_execution_intent / signing-payload endpoints.
        let s = seeded_state_for_create_intent();
        let env = create_execution_intent(State(s.clone()), Json(valid_create_request()))
            .await
            .expect("ok");
        let intent_id = &env.0.intent_id;
        let stored = s
            .options_store
            .lock()
            .unwrap()
            .get_option_execution_intent(Uuid::parse_str(intent_id).expect("uuid"))
            .map(|i: crate::options::OptionExecutionIntent| i.intent_id.to_string());
        assert_eq!(stored.as_deref(), Some(intent_id.as_str()));
    }

    #[tokio::test]
    async fn create_intent_response_body_never_leaks_secrets() {
        let s = seeded_state_for_create_intent();
        let env = create_execution_intent(State(s), Json(valid_create_request()))
            .await
            .expect("ok");
        let body = serde_json::to_string(&env.0).unwrap();
        for forbidden in [
            "EXECUTOR_PRIVATE_KEY",
            "DATABASE_URL",
            "AWS_ACCESS_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "arn:aws:kms",
            "Bearer ",
            "http://",
            "https://",
            "rpc_url",
        ] {
            assert!(!body.contains(forbidden), "leak: {forbidden}\n{body}");
        }
    }

    #[tokio::test]
    async fn create_intent_status_starts_at_signatures_required() {
        // Defence-in-depth: the returned intent must be in the
        // `SignaturesRequired` state, NOT `BroadcastConfirmed` or any
        // other terminal/in-flight state — the user has not signed yet.
        let s = seeded_state_for_create_intent();
        let env = create_execution_intent(State(s.clone()), Json(valid_create_request()))
            .await
            .expect("ok");
        let intent = s
            .options_store
            .lock()
            .unwrap()
            .get_option_execution_intent(Uuid::parse_str(&env.0.intent_id).expect("uuid"))
            .expect("intent stored");
        assert_eq!(
            intent.status,
            crate::options::OptionExecutionIntentStatus::SignaturesRequired
        );
        assert!(intent.buyer_signature.is_none());
        assert!(intent.seller_signature.is_none());
    }

    #[tokio::test]
    async fn create_intent_quote_id_is_accepted_but_ignored() {
        // Forward-compat hook — the caller may supply quote_id; M-P2f
        // does not yet resolve it. Endpoint must succeed when the
        // explicit buyer + seller fields are also supplied.
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.quote_id = Some("RFQ-quote-uuid-placeholder".to_string());
        let env = create_execution_intent(State(s), Json(req))
            .await
            .expect("ok");
        assert!(!env.0.intent_id.is_empty());
    }

    #[tokio::test]
    async fn create_intent_does_not_set_calldata_or_signatures() {
        // The handler must never produce calldata (that's the operator
        // pipeline's job) and must never carry a signature in the
        // returned intent.
        let s = seeded_state_for_create_intent();
        let env = create_execution_intent(State(s.clone()), Json(valid_create_request()))
            .await
            .expect("ok");
        let intent = s
            .options_store
            .lock()
            .unwrap()
            .get_option_execution_intent(Uuid::parse_str(&env.0.intent_id).expect("uuid"))
            .expect("intent stored");
        assert!(intent.calldata.is_none());
        assert!(intent.buyer_signature.is_none());
        assert!(intent.seller_signature.is_none());
        assert!(intent.simulated_at_ms.is_none());
    }

    #[tokio::test]
    async fn create_intent_endpoint_has_no_admin_bearer_requirement() {
        // The handler signature accepts `State(state)` only — no
        // `headers: HeaderMap`, no `AdminIdentity` extractor. A bare
        // call without any auth context produces a successful intent.
        let s = seeded_state_for_create_intent();
        let env = create_execution_intent(State(s), Json(valid_create_request())).await;
        assert!(env.is_ok());
    }

    #[tokio::test]
    async fn create_intent_size_too_large_is_rejected_safely() {
        // Defence-in-depth: a u128 max input must not panic. The
        // service layer catches conversion overflow as an
        // InvalidOptionExecutionIntentState error which maps to
        // InvalidRequest at the public surface.
        let s = seeded_state_for_create_intent();
        let mut req = valid_create_request();
        req.size_1e8 = u128::MAX.to_string();
        let err = create_execution_intent(State(s), Json(req))
            .await
            .unwrap_err();
        // Either InvalidRequest (overflow detected) or InternalError
        // (caught later) — both are acceptable; the contract is that
        // the handler must not panic and must produce a 4xx/5xx
        // response without leaking internals.
        assert!(matches!(
            err.code,
            TradingErrorCode::InvalidRequest | TradingErrorCode::InternalError
        ));
    }

    #[tokio::test]
    async fn create_intent_returns_distinct_intent_ids_per_request() {
        let s = seeded_state_for_create_intent();
        let a = create_execution_intent(State(s.clone()), Json(valid_create_request()))
            .await
            .expect("ok");
        let b = create_execution_intent(State(s), Json(valid_create_request()))
            .await
            .expect("ok");
        assert_ne!(a.0.intent_id, b.0.intent_id);
    }
}
