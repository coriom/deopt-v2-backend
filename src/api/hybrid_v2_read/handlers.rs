//! Public read-API handlers.
//!
//! `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1` posture:
//!
//! Every data-bearing handler depends exclusively on
//! `Arc<dyn HybridV2ReadStore>` via `state.store()`. No handler reads
//! from `ProjectionState`, `IndexerRuntime`, or any runtime lock. Under
//! production wiring the store is `PostgresHybridV2ReadStore` and there
//! is NO runtime-memory fallback. Store failures fail closed via
//! `ApiError::from(ReadStoreError)`.
//!
//! Per-handler shape:
//! 1. request parsing + strict address/hash validation,
//! 2. cursor decoding + binding verification,
//! 3. store-scoped canonical read (`store.get_deployment_status` + the
//!    handler's data method),
//! 4. hard-readiness gate driven by the persisted `ready` flag,
//! 5. response transformation with exact wire compatibility,
//! 6. next-cursor encoding.
//!
//! No mutations. No signing. No chain writes.

use crate::api::hybrid_v2_read::cursor::{
    filter_hash as compute_filter_hash, Cursor, CursorBinding,
};
use crate::api::hybrid_v2_read::errors::{ApiError, ApiErrorCode};
use crate::api::hybrid_v2_read::history::{
    HistoryDirection, HistoryEvent, HistoryEventPayload, HistoryFilter,
};
use crate::api::hybrid_v2_read::metadata::{
    build_metadata_from_status, hard_readiness_failure_from_status, now_ms, CanonicalityMetadata,
    ConsistencyMode,
};
use crate::api::hybrid_v2_read::serialization::{parse_address, parse_bytes32};
use crate::api::hybrid_v2_read::state::{DeploymentEntry, HybridV2ApiState};
use crate::hybrid_v2::read_store::{
    filter_stable_hash, DeploymentStatusRecord, HistoryConsistency, HistoryCursorKey,
    HistoryPageAnchor, HistoryRecord, HistoryScope, HybridV2ReadStore, PageAnchor,
};
use crate::hybrid_v2::reducer::{
    EscapeStateRow, ExecutionCompletion, FeeEventRow, MatchedExecutionRow, OrderLifecycleRow,
};
use crate::hybrid_v2::repository::MAX_PAGE_LIMIT;
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// -----------------------------------------------------------------
//                          RESPONSE WRAPPERS
// -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithMeta<T: Serialize> {
    pub data: T,
    pub metadata: CanonicalityMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl<T: Serialize> WithMeta<T> {
    pub fn new(data: T, metadata: CanonicalityMetadata) -> Self {
        Self {
            data,
            metadata,
            next_cursor: None,
        }
    }
    pub fn with_cursor(mut self, cursor: Option<String>) -> Self {
        self.next_cursor = cursor;
        self
    }
}

// -----------------------------------------------------------------
//                          COMMON QUERY
// -----------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct BasePageQuery {
    pub deployment_id: Option<u64>,
    pub consistency: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

fn resolve_deployment(
    state: &HybridV2ApiState,
    explicit: Option<u64>,
) -> Result<Arc<DeploymentEntry>, ApiError> {
    if let Some(id) = explicit {
        return state
            .get(id)
            .ok_or_else(|| ApiError::new(ApiErrorCode::DeploymentNotFound, "unknown deployment"));
    }
    state.resolve_single().ok_or_else(|| {
        ApiError::new(
            ApiErrorCode::InvalidDeployment,
            "deployment_id is required when multiple deployments are configured",
        )
    })
}

fn parse_consistency(s: Option<&String>) -> Result<ConsistencyMode, ApiError> {
    match s {
        None => Ok(ConsistencyMode::Indexed),
        Some(raw) => ConsistencyMode::parse(raw).ok_or_else(|| {
            ApiError::new(
                ApiErrorCode::UnsupportedConsistency,
                format!("unsupported consistency '{}'", raw),
            )
        }),
    }
}

fn bounded_limit(requested: Option<usize>) -> Result<usize, ApiError> {
    let n = requested.unwrap_or(100).max(1);
    if n > MAX_PAGE_LIMIT {
        return Err(ApiError::new(
            ApiErrorCode::PageLimitExceeded,
            format!("limit {} exceeds max {}", n, MAX_PAGE_LIMIT),
        ));
    }
    Ok(n)
}

async fn load_status(
    store: &Arc<dyn HybridV2ReadStore>,
    deployment_id: u64,
) -> Result<DeploymentStatusRecord, ApiError> {
    store
        .get_deployment_status(deployment_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::new(
                ApiErrorCode::DeploymentNotFound,
                "deployment status unavailable",
            )
        })
}

fn enforce_readiness(status: &DeploymentStatusRecord) -> Result<(), ApiError> {
    if let Some(code) =
        hard_readiness_failure_from_status(status.ready, status.ready_reason.as_deref())
    {
        let err_code = match code {
            "MANIFEST_MISMATCH" => ApiErrorCode::ManifestMismatch,
            "RECONCILIATION_DRIFT" => ApiErrorCode::ReconciliationDrift,
            _ => ApiErrorCode::IndexerNotReady,
        };
        return Err(ApiError::new(
            err_code,
            format!("hybrid v2 indexer not serving canonical data: {}", code),
        ));
    }
    Ok(())
}

fn build_metadata(
    entry: &DeploymentEntry,
    status: &DeploymentStatusRecord,
    consistency: ConsistencyMode,
) -> CanonicalityMetadata {
    build_metadata_from_status(
        entry.deployment_id,
        entry.manifest.chain_id,
        &entry.manifest.manifest_hash,
        status,
        consistency,
        now_ms(),
    )
}

// -----------------------------------------------------------------
//                          DEPLOYMENT + STATUS
// -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct DeploymentListRow {
    pub deployment_id: u64,
    pub chain_id: u64,
    pub manifest_hash: String,
    pub manifest_address: String,
    pub deployment_version: u16,
    pub activation_status: String,
    pub max_collateral_tokens: u16,
    pub max_active_series: u32,
    pub ready: bool,
}

pub async fn get_deployments(
    State(state): State<HybridV2ApiState>,
) -> Result<Json<Vec<DeploymentListRow>>, ApiError> {
    let store = state.store();
    let mut out = Vec::new();
    for entry in state.list() {
        // Store call resolves the canonical `ready` flag; when a
        // deployment is registered but has no persisted status yet,
        // the row reports `ready = false`.
        let ready = store
            .get_deployment_status(entry.deployment_id)
            .await
            .map_err(ApiError::from)?
            .map(|s| s.ready)
            .unwrap_or(false);
        out.push(DeploymentListRow {
            deployment_id: entry.deployment_id,
            chain_id: entry.manifest.chain_id,
            manifest_hash: entry.manifest.manifest_hash.clone(),
            manifest_address: entry.manifest.manifest_address.clone(),
            deployment_version: entry.manifest.deployment_version,
            activation_status: format!("{:?}", entry.manifest.activation_status).to_uppercase(),
            max_collateral_tokens: entry.manifest.max_collateral_tokens,
            max_active_series: entry.manifest.max_active_series,
            ready,
        });
    }
    Ok(Json(out))
}

#[derive(Debug, Serialize)]
pub struct DeploymentStatusResponse {
    pub deployment_id: u64,
    pub chain_id: u64,
    pub manifest_hash: String,
    pub ready: bool,
    pub ready_reason: Option<String>,
    pub indexed_block: u64,
    pub indexed_block_hash: String,
    pub finalized_block: u64,
    pub observed_head_block: u64,
    pub indexer_lag: u64,
    pub decode_failures: u64,
    pub projection_failures: u64,
    pub reorg_count: u64,
    pub max_reorg_depth_seen: u64,
    pub rebuild_status: String,
    pub reconciliation_status: String,
    pub metadata: CanonicalityMetadata,
}

pub async fn get_deployment_status(
    State(state): State<HybridV2ApiState>,
    Path(deployment_id): Path<u64>,
) -> Result<Json<DeploymentStatusResponse>, ApiError> {
    let entry = state
        .get(deployment_id)
        .ok_or_else(|| ApiError::new(ApiErrorCode::DeploymentNotFound, "unknown deployment"))?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    // Status is always readable — never blocked by the hard-readiness gate.
    let metadata = build_metadata(&entry, &status, ConsistencyMode::Indexed);
    let lag = status
        .observed_head_block
        .saturating_sub(status.indexed_head_block);
    Ok(Json(DeploymentStatusResponse {
        deployment_id: entry.deployment_id,
        chain_id: entry.manifest.chain_id,
        manifest_hash: entry.manifest.manifest_hash.clone(),
        ready: status.ready,
        ready_reason: status.ready_reason.clone(),
        indexed_block: status.indexed_head_block,
        indexed_block_hash: status.indexed_head_hash.clone(),
        finalized_block: status.finalized_head_block,
        observed_head_block: status.observed_head_block,
        indexer_lag: lag,
        decode_failures: status.decode_failures,
        projection_failures: status.projection_failures,
        reorg_count: status.reorg_count,
        max_reorg_depth_seen: status.max_reorg_depth_seen,
        rebuild_status: status
            .rebuild_status
            .clone()
            .unwrap_or_else(|| "NORMAL".into()),
        reconciliation_status: status
            .reconciliation_status
            .clone()
            .unwrap_or_else(|| "NORMAL".into()),
        metadata,
    }))
}

// -----------------------------------------------------------------
//                          OWNER / SUBACCOUNT
// -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OwnerSubaccountsResponse {
    pub owner: String,
    pub subaccounts: Vec<OwnerSubaccountRow>,
}

#[derive(Debug, Serialize)]
pub struct OwnerSubaccountRow {
    pub subaccount_id: u32,
    pub subkey: String,
    pub materialised_via_created: bool,
    pub materialised_via_lazy: bool,
    pub recovery_state: String,
    pub finalized: bool,
    pub balance_token_count: usize,
    pub reservation_count: usize,
    pub active_series_count: usize,
}

pub async fn get_owner_subaccounts(
    State(state): State<HybridV2ApiState>,
    Path(owner_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<OwnerSubaccountsResponse>>, ApiError> {
    let owner = parse_address(&owner_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let subs = store
        .list_subaccounts_by_owner(entry.deployment_id, &owner)
        .await
        .map_err(ApiError::from)?;
    let mut rows = Vec::with_capacity(subs.len());
    for sub in subs {
        // Per-subaccount summary fills in `materialised_via` + counts
        // + recovery state. N here is small in practice (few
        // subaccounts per owner).
        let summary = store
            .get_subaccount_summary(entry.deployment_id, &sub.subkey)
            .await
            .map_err(ApiError::from)?;
        let (mat_c, mat_l, recov, finalized, btc, rc, asc) = match summary {
            Some(s) => (
                s.materialised_via_created,
                s.materialised_via_lazy,
                s.recovery_state,
                s.finalized,
                s.balance_token_count,
                s.reservation_count,
                s.active_series_count,
            ),
            None => (false, false, "NORMAL".to_string(), false, 0, 0, 0),
        };
        rows.push(OwnerSubaccountRow {
            subaccount_id: sub.subaccount_id,
            subkey: sub.subkey.clone(),
            materialised_via_created: mat_c,
            materialised_via_lazy: mat_l,
            recovery_state: recov,
            finalized,
            balance_token_count: btc,
            reservation_count: rc,
            active_series_count: asc,
        });
    }
    rows.sort_by_key(|r| r.subaccount_id);
    let _ = q.cursor;
    Ok(Json(WithMeta::new(
        OwnerSubaccountsResponse {
            owner: owner.clone(),
            subaccounts: rows,
        },
        metadata,
    )))
}

#[derive(Debug, Serialize)]
pub struct SubaccountSummary {
    pub subkey: String,
    pub owner: String,
    pub subaccount_id: u32,
    pub materialised_via_created: bool,
    pub materialised_via_lazy: bool,
    pub recovery_state: String,
    pub finalized: bool,
    pub balance_token_count: usize,
    pub reservation_count: usize,
    pub active_series_count: usize,
    pub open_order_count: usize,
    pub escape_state: Option<EscapeStateRow>,
    pub owner_recovery_epoch: u64,
    pub subaccount_recovery_epoch: u64,
}

pub async fn get_subaccount_summary(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<SubaccountSummary>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let summary = store
        .get_subaccount_summary(entry.deployment_id, &subkey)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::new(ApiErrorCode::SubaccountNotFound, "subkey not found"))?;
    Ok(Json(WithMeta::new(
        SubaccountSummary {
            subkey: summary.identity.subkey.clone(),
            owner: summary.identity.owner.clone(),
            subaccount_id: summary.identity.subaccount_id,
            materialised_via_created: summary.materialised_via_created,
            materialised_via_lazy: summary.materialised_via_lazy,
            recovery_state: summary.recovery_state,
            finalized: summary.finalized,
            balance_token_count: summary.balance_token_count,
            reservation_count: summary.reservation_count,
            active_series_count: summary.active_series_count,
            open_order_count: summary.open_order_count,
            escape_state: summary.escape,
            owner_recovery_epoch: summary.owner_recovery_epoch,
            subaccount_recovery_epoch: summary.subaccount_recovery_epoch,
        },
        metadata,
    )))
}

// -----------------------------------------------------------------
//                          COLLATERAL / RESERVATIONS
// -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CollateralRow {
    pub token: String,
    pub universe_index: Option<u32>,
    pub enabled: bool,
    pub balance: String,
    pub aggregate_reserved: String,
    pub available: String,
}

pub async fn get_subaccount_collateral(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<CollateralRow>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let records = store
        .list_collateral(entry.deployment_id, &subkey)
        .await
        .map_err(ApiError::from)?;
    let mut rows: Vec<CollateralRow> = records
        .into_iter()
        .map(|r| CollateralRow {
            token: r.token,
            universe_index: r.universe_index,
            enabled: r.enabled,
            balance: r.balance,
            aggregate_reserved: r.aggregate_reserved,
            available: r.available,
        })
        .collect();
    rows.sort_by(|a, b| a.token.cmp(&b.token));
    Ok(Json(WithMeta::new(rows, metadata)))
}

#[derive(Debug, Serialize)]
pub struct ReservationRowResponse {
    pub token: String,
    pub engine: String,
    pub reserved: String,
}

pub async fn get_subaccount_reservations(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<ReservationRowResponse>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let records = store
        .list_reservations(entry.deployment_id, &subkey)
        .await
        .map_err(ApiError::from)?;
    let mut rows: Vec<ReservationRowResponse> = records
        .into_iter()
        .map(|r| ReservationRowResponse {
            token: r.token,
            engine: r.engine,
            reserved: r.reserved,
        })
        .collect();
    rows.sort_by(|a, b| (&a.token, &a.engine).cmp(&(&b.token, &b.engine)));
    Ok(Json(WithMeta::new(rows, metadata)))
}

// -----------------------------------------------------------------
//                          POSITIONS
// -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PositionResponseRow {
    pub series_id: String,
    pub long_qty_1e8: String,
    pub short_qty_1e8: String,
    pub active: bool,
    pub last_event_block: u64,
}

pub async fn get_subaccount_positions(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<PositionResponseRow>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let limit = bounded_limit(q.limit)?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let page = store
        .list_positions(
            entry.deployment_id,
            &subkey,
            &PageAnchor::first(limit).map_err(ApiError::from)?,
        )
        .await
        .map_err(ApiError::from)?;
    let mut rows: Vec<PositionResponseRow> = page
        .items
        .into_iter()
        .map(|p| PositionResponseRow {
            series_id: p.series_id,
            long_qty_1e8: p.long_qty_1e8,
            short_qty_1e8: p.short_qty_1e8,
            active: p.active,
            last_event_block: p.last_event_block,
        })
        .collect();
    rows.sort_by(|a, b| a.series_id.cmp(&b.series_id));
    Ok(Json(WithMeta::new(rows, metadata)))
}

// -----------------------------------------------------------------
//                          ORDERS
// -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OrderRow {
    pub order_hash: String,
    pub owner: String,
    pub subkey: String,
    pub series_id: Option<String>,
    pub side: u8,
    pub time_in_force: u8,
    pub total_qty_1e8: String,
    pub filled_qty_1e8: String,
    pub remaining_qty_1e8: String,
    pub cancelled: bool,
    pub terminal: bool,
    pub first_seen_block: u64,
    pub last_event_block: u64,
}

pub async fn get_subaccount_orders(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<OrderRow>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let limit = bounded_limit(q.limit)?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let page = store
        .list_orders(
            entry.deployment_id,
            &subkey,
            &PageAnchor::first(limit).map_err(ApiError::from)?,
        )
        .await
        .map_err(ApiError::from)?;
    let mut rows: Vec<OrderRow> = page
        .items
        .into_iter()
        .map(|r| OrderRow {
            order_hash: r.order_hash,
            owner: r.owner,
            subkey: r.subkey,
            series_id: r.series_id,
            side: r.side,
            time_in_force: r.time_in_force,
            total_qty_1e8: r.total_qty_1e8,
            filled_qty_1e8: r.filled_qty_1e8,
            remaining_qty_1e8: r.remaining_qty_1e8,
            cancelled: r.cancelled,
            terminal: r.terminal,
            first_seen_block: r.first_seen_block,
            last_event_block: r.last_event_block,
        })
        .collect();
    rows.sort_by(|a, b| a.order_hash.cmp(&b.order_hash));
    Ok(Json(WithMeta::new(rows, metadata)))
}

pub async fn get_order_lifecycle(
    State(state): State<HybridV2ApiState>,
    Path(order_hash): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<OrderLifecycleRow>>, ApiError> {
    let oh = parse_bytes32(&order_hash)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let record = store
        .get_order(entry.deployment_id, &oh)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::new(ApiErrorCode::OrderNotFound, "order not found"))?;
    let row = OrderLifecycleRow {
        subkey: record.subkey,
        owner: record.owner,
        series_id: record.series_id,
        side: record.side,
        time_in_force: record.time_in_force,
        total_qty_1e8: record.total_qty_1e8,
        filled_qty_1e8: record.filled_qty_1e8,
        cancelled: record.cancelled,
        terminal: record.terminal,
        first_seen_block: record.first_seen_block,
        last_event_block: record.last_event_block,
    };
    Ok(Json(WithMeta::new(row, metadata)))
}

// -----------------------------------------------------------------
//                          EXECUTIONS
// -----------------------------------------------------------------

pub async fn get_subaccount_executions(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<(String, MatchedExecutionRow)>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let limit = bounded_limit(q.limit)?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let page = store
        .list_completed_executions(
            entry.deployment_id,
            Some(&subkey),
            &PageAnchor::first(limit).map_err(ApiError::from)?,
        )
        .await
        .map_err(ApiError::from)?;
    let mut rows: Vec<(String, MatchedExecutionRow)> = page
        .items
        .into_iter()
        .map(|rec| {
            (
                rec.execution_id,
                MatchedExecutionRow {
                    buyer_order_hash: rec.buyer_order_hash,
                    seller_order_hash: rec.seller_order_hash,
                    buyer_subkey: rec.buyer_subkey,
                    seller_subkey: rec.seller_subkey,
                    series_id: rec.series_id,
                    matched_qty_1e8: rec.matched_qty_1e8,
                    premium_amount: rec.premium_amount,
                    fee_amount: rec.fee_amount,
                    rebate_amount: rec.rebate_amount,
                    block_number: rec.block_number,
                    tx_hash: rec.tx_hash,
                    // Store filter guarantees COMPLETE.
                    completion_status: ExecutionCompletion::Complete,
                },
            )
        })
        .collect();
    rows.sort_by(|a, b| (b.1.block_number, &b.0).cmp(&(a.1.block_number, &a.0)));
    Ok(Json(WithMeta::new(rows, metadata)))
}

// -----------------------------------------------------------------
//                          FEES
// -----------------------------------------------------------------

fn fee_kind_static(kind: &str) -> &'static str {
    match kind {
        "OPTION_FEE_CHARGED" => "OPTION_FEE_CHARGED",
        "OPTION_REBATE_PAID" => "OPTION_REBATE_PAID",
        "OPTION_PREMIUM_TRANSFERRED" => "OPTION_PREMIUM_TRANSFERRED",
        _ => "OTHER",
    }
}

pub async fn get_subaccount_fees(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<FeeEventRow>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let limit = bounded_limit(q.limit)?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let page = store
        .list_fees(
            entry.deployment_id,
            &subkey,
            &PageAnchor::first(limit).map_err(ApiError::from)?,
        )
        .await
        .map_err(ApiError::from)?;
    let mut rows: Vec<FeeEventRow> = page
        .items
        .into_iter()
        .map(|r| FeeEventRow {
            kind: fee_kind_static(&r.kind),
            payer_subkey: r.payer_subkey,
            receiver_subkey: r.receiver_subkey,
            token: r.token,
            amount: r.amount,
            block_number: r.block_number,
            tx_hash: r.tx_hash,
            log_index: r.log_index,
            execution_id: r.execution_id,
        })
        .collect();
    rows.sort_by(|a, b| (b.block_number, b.log_index).cmp(&(a.block_number, a.log_index)));
    Ok(Json(WithMeta::new(rows, metadata)))
}

// -----------------------------------------------------------------
//                          RECOVERY
// -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RecoveryResponse {
    pub subkey: String,
    pub recovery_state: String,
    pub finalized: bool,
    pub owner_recovery_epoch: u64,
    pub subaccount_recovery_epoch: u64,
    pub owner_min_valid_nonce: String,
    pub escape_state: Option<EscapeStateRow>,
    pub finalization_withdrawal_count: u32,
}

pub async fn get_subaccount_recovery(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<RecoveryResponse>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let record = store
        .get_recovery(entry.deployment_id, &subkey)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::new(ApiErrorCode::SubaccountNotFound, "subkey not found"))?;
    Ok(Json(WithMeta::new(
        RecoveryResponse {
            subkey: record.subkey,
            recovery_state: record.recovery_state,
            finalized: record.finalized,
            owner_recovery_epoch: record.owner_recovery_epoch,
            subaccount_recovery_epoch: record.subaccount_recovery_epoch,
            owner_min_valid_nonce: record.owner_min_valid_nonce,
            escape_state: record.escape,
            finalization_withdrawal_count: record.withdrawal_count,
        },
        metadata,
    )))
}

// -----------------------------------------------------------------
//                          HISTORY
// -----------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryQuery {
    pub deployment_id: Option<u64>,
    pub consistency: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub owner: Option<String>,
    pub subkey: Option<String>,
    pub subaccount_id: Option<u32>,
    #[serde(default)]
    pub families: Option<String>, // comma-separated
    pub token: Option<String>,
    pub series_id: Option<String>,
    pub order_hash: Option<String>,
    pub execution_id: Option<String>,
    pub direction: Option<String>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
}

impl HistoryQuery {
    fn build_filter(&self) -> Result<HistoryFilter, ApiError> {
        let direction = match self.direction.as_deref() {
            None => None,
            Some("INBOUND") | Some("inbound") => Some(HistoryDirection::Inbound),
            Some("OUTBOUND") | Some("outbound") => Some(HistoryDirection::Outbound),
            Some("INTERNAL") | Some("internal") => Some(HistoryDirection::Internal),
            Some("METADATA") | Some("metadata") => Some(HistoryDirection::Metadata),
            Some(o) => {
                return Err(ApiError::new(
                    ApiErrorCode::InvalidFilterCombination,
                    format!("unknown direction '{}'", o),
                ))
            }
        };
        Ok(HistoryFilter {
            deployment_id: self.deployment_id,
            owner: self.owner.clone(),
            subkey: self.subkey.clone(),
            subaccount_id: self.subaccount_id,
            families: self
                .families
                .as_deref()
                .map(|s| {
                    s.split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_ascii_uppercase())
                        .collect()
                })
                .unwrap_or_default(),
            token: self.token.clone(),
            series_id: self.series_id.clone(),
            order_hash: self.order_hash.clone(),
            execution_id: self.execution_id.clone(),
            direction,
            from_block: self.from_block,
            to_block: self.to_block,
            from_ts_ms: None,
            to_ts_ms: None,
        })
    }
}

fn record_to_event(r: HistoryRecord) -> HistoryEvent {
    HistoryEvent {
        event_id: r.event_id,
        deployment_id: r.deployment_id,
        chain_id: r.chain_id,
        block_number: r.block_number,
        block_hash: r.block_hash,
        tx_hash: r.tx_hash,
        tx_index: r.tx_index,
        log_index: r.log_index,
        timestamp_ms: r.timestamp_ms,
        finalized: r.finalized,
        direction: r.direction,
        owner: r.owner,
        subaccount_id: r.subaccount_id,
        subkey: r.subkey,
        related_order_hash: r.related_order_hash,
        related_execution_id: r.related_execution_id,
        payload: r.payload,
    }
}

fn history_scope_from_filter(filter: &HistoryFilter) -> HistoryScope {
    if let Some(sk) = filter.subkey.as_ref() {
        HistoryScope::Subaccount { subkey: sk.clone() }
    } else if let Some(o) = filter.owner.as_ref() {
        HistoryScope::Owner { owner: o.clone() }
    } else {
        HistoryScope::Global
    }
}

async fn run_history(
    state: &HybridV2ApiState,
    q: &HistoryQuery,
    scope: HistoryScope,
) -> Result<Json<WithMeta<Vec<HistoryEvent>>>, ApiError> {
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let limit = bounded_limit(q.limit)?;
    let entry = resolve_deployment(state, q.deployment_id)?;
    let store = state.store();
    let status = load_status(&store, entry.deployment_id).await?;
    enforce_readiness(&status)?;
    let metadata = build_metadata(&entry, &status, consistency);
    let filter = q.build_filter()?;

    // Cursor binding hashes.
    let http_filter_hash = compute_filter_hash(&filter.to_stable_json());
    let indexed_head_hash = status.indexed_head_hash.clone();
    let binding = CursorBinding {
        deployment_id: entry.deployment_id,
        consistency,
        filter_hash: http_filter_hash.clone(),
        indexed_head_hash: indexed_head_hash.clone(),
    };
    let after_key = if let Some(raw) = q.cursor.as_ref() {
        let cursor = Cursor::decode(raw)?;
        cursor.verify(&binding)?;
        Some(cursor.last_key)
    } else {
        None
    };

    // Store-boundary anchor.
    let store_filter_hash = filter_stable_hash(&filter);
    let store_consistency = match consistency {
        ConsistencyMode::Indexed => HistoryConsistency::Indexed,
        ConsistencyMode::Finalized => HistoryConsistency::Finalized,
    };
    let after = match after_key {
        None => None,
        Some(raw) => Some(parse_history_cursor_key(&raw)?),
    };
    let page_anchor = HistoryPageAnchor {
        limit,
        after,
        consistency: store_consistency,
        filter_hash: store_filter_hash,
        indexed_head_hash: indexed_head_hash.clone(),
    };
    let page = store
        .query_history(entry.deployment_id, &scope, &filter, &page_anchor)
        .await
        .map_err(ApiError::from)?;

    let events: Vec<HistoryEvent> = page.items.into_iter().map(record_to_event).collect();
    let next = page.next_anchor.and_then(|a| {
        a.after.map(|k| Cursor {
            version: 1,
            binding: binding.clone(),
            last_key: encode_history_cursor_key(&k),
        })
    });
    let encoded = next.as_ref().map(Cursor::encode);
    Ok(Json(WithMeta::new(events, metadata).with_cursor(encoded)))
}

fn encode_history_cursor_key(k: &HistoryCursorKey) -> String {
    format!(
        "{}:{}:{}:{}",
        k.block_number, k.tx_index, k.log_index, k.event_id
    )
}

fn parse_history_cursor_key(raw: &str) -> Result<HistoryCursorKey, ApiError> {
    // Preserve backwards compatibility with the pre-refactor cursor
    // format `block_hash:tx_hash:log_index` — if the caller supplies
    // it, we accept it as an event_id anchor with unknown numeric
    // ordering (block_number = 0), and the store falls back to
    // event_id matching. New cursors from this handler use the
    // canonical `block_number:tx_index:log_index:event_id`.
    let parts: Vec<&str> = raw.splitn(4, ':').collect();
    match parts.as_slice() {
        [bn, tx, li, ev] => {
            let block_number: u64 = bn.parse().map_err(|_| {
                ApiError::new(ApiErrorCode::InvalidCursor, "cursor block_number invalid")
            })?;
            let tx_index: u32 = tx.parse().map_err(|_| {
                ApiError::new(ApiErrorCode::InvalidCursor, "cursor tx_index invalid")
            })?;
            let log_index: u32 = li.parse().map_err(|_| {
                ApiError::new(ApiErrorCode::InvalidCursor, "cursor log_index invalid")
            })?;
            Ok(HistoryCursorKey {
                block_number,
                tx_index,
                log_index,
                event_id: ev.to_string(),
            })
        }
        _ => Ok(HistoryCursorKey {
            block_number: 0,
            tx_index: 0,
            log_index: 0,
            event_id: raw.to_string(),
        }),
    }
}

pub async fn get_history(
    State(state): State<HybridV2ApiState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<WithMeta<Vec<HistoryEvent>>>, ApiError> {
    let scope = history_scope_from_filter(&q.build_filter()?);
    run_history(&state, &q, scope).await
}

pub async fn get_subaccount_history(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(mut q): Query<HistoryQuery>,
) -> Result<Json<WithMeta<Vec<HistoryEvent>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    q.subkey = Some(subkey.clone());
    run_history(&state, &q, HistoryScope::Subaccount { subkey }).await
}

pub async fn get_owner_history(
    State(state): State<HybridV2ApiState>,
    Path(owner_raw): Path<String>,
    Query(mut q): Query<HistoryQuery>,
) -> Result<Json<WithMeta<Vec<HistoryEvent>>>, ApiError> {
    let owner = parse_address(&owner_raw)?;
    q.owner = Some(owner.clone());
    run_history(&state, &q, HistoryScope::Owner { owner }).await
}

// Retain the payload family label helper for external callers (openapi
// generator, tests) that expect to render the enum discriminant.
pub fn history_event_family_label(p: &HistoryEventPayload) -> &'static str {
    p.family_label()
}
