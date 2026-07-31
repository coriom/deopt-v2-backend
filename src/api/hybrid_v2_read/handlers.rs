//! Public read-API handlers.
//!
//! Each handler:
//! 1. resolves the target deployment (implicit when only one is configured,
//!    otherwise via `deployment_id` query/path),
//! 2. acquires a read lock on the runtime,
//! 3. builds the requested view via `HybridV2QueryRepository`,
//! 4. attaches `CanonicalityMetadata`,
//! 5. enforces the hard-readiness gate.
//!
//! No mutations. No signing. No chain writes.

use crate::api::hybrid_v2_read::cursor::{
    filter_hash as compute_filter_hash, Cursor, CursorBinding,
};
use crate::api::hybrid_v2_read::errors::{ApiError, ApiErrorCode};
use crate::api::hybrid_v2_read::history::{
    build_history, sort_desc, take_page_after, HistoryDirection, HistoryEvent, HistoryFilter,
};
use crate::api::hybrid_v2_read::metadata::{
    hard_readiness_failure, now_ms, CanonicalityLevel, CanonicalityMetadata, ConsistencyMode,
};
use crate::api::hybrid_v2_read::serialization::{parse_address, parse_bytes32};
use crate::api::hybrid_v2_read::state::{DeploymentEntry, HybridV2ApiState};
use crate::hybrid_v2::reducer::{
    EscapeStateRow, ExecutionCompletion, FeeEventRow, MatchedExecutionRow, OrderLifecycleRow,
};
use crate::hybrid_v2::repository::{
    CollateralBalanceRow, HybridV2QueryRepository, PageCursor as RepoPageCursor, ReservationRow,
    SubaccountRow, MAX_PAGE_LIMIT,
};
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

fn build_metadata(
    entry: &DeploymentEntry,
    consistency: ConsistencyMode,
) -> Result<CanonicalityMetadata, ApiError> {
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let cursor = runtime.cursor().clone();
    let observed = runtime.metrics().observed_block;
    let finalized = runtime.metrics().finalized_block;
    let reconciliation = runtime.metrics().reconciliation_status.to_string();
    let readiness = runtime.readiness().clone();
    Ok(CanonicalityMetadata::from_runtime(
        entry.deployment_id,
        entry.manifest.chain_id,
        &entry.manifest.manifest_hash,
        &cursor,
        observed,
        finalized,
        &reconciliation,
        &readiness,
        consistency,
        now_ms(),
    ))
}

fn require_ready_canonical(entry: &DeploymentEntry) -> Result<(), ApiError> {
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    if let Some(code) = hard_readiness_failure(runtime.readiness()) {
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
    let mut out = Vec::new();
    for entry in state.list() {
        let ready = entry
            .runtime
            .read()
            .map(|r| r.readiness().ready)
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
    // Status is always readable — never blocked by the hard-readiness gate.
    let metadata = build_metadata(&entry, ConsistencyMode::Indexed)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let readiness = runtime.readiness().clone();
    let metrics = runtime.metrics().clone();
    Ok(Json(DeploymentStatusResponse {
        deployment_id: entry.deployment_id,
        chain_id: entry.manifest.chain_id,
        manifest_hash: entry.manifest.manifest_hash.clone(),
        ready: readiness.ready,
        ready_reason: readiness.reason.as_ref().map(|r| format!("{:?}", r)),
        indexed_block: runtime.cursor().indexed_head_block,
        indexed_block_hash: runtime.cursor().indexed_head_hash.clone(),
        finalized_block: metrics.finalized_block,
        observed_head_block: metrics.observed_block,
        indexer_lag: metrics.lag,
        decode_failures: metrics.decode_failures,
        projection_failures: metrics.projection_failures,
        reorg_count: metrics.reorg_count,
        max_reorg_depth_seen: metrics.max_reorg_depth_seen,
        rebuild_status: metrics.rebuild_status.to_string(),
        reconciliation_status: metrics.reconciliation_status.to_string(),
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
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let repo = HybridV2QueryRepository::new(
        entry.deployment_id,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        runtime
            .readiness()
            .reason
            .clone()
            .map(|r| format!("{:?}", r)),
    );
    let subs: Vec<SubaccountRow> = repo.subaccounts_by_owner(&owner);
    let mut rows = Vec::with_capacity(subs.len());
    for sub in subs {
        let meta = runtime
            .projection()
            .subaccount_meta
            .get(&sub.subkey)
            .copied()
            .unwrap_or_default();
        let recovery = repo.recovery_state(&sub.subkey);
        let balance_token_count = runtime
            .projection()
            .balances
            .iter()
            .filter(|((sk, _), _)| sk.eq_ignore_ascii_case(&sub.subkey))
            .count();
        let reservation_count = runtime
            .projection()
            .reservations
            .iter()
            .filter(|((sk, _, _), _)| sk.eq_ignore_ascii_case(&sub.subkey))
            .count();
        let active_series_count = runtime
            .projection()
            .active_series
            .get(&sub.subkey)
            .map(|s| s.len())
            .unwrap_or(0);
        rows.push(OwnerSubaccountRow {
            subaccount_id: sub.subaccount_id,
            subkey: sub.subkey.clone(),
            materialised_via_created: meta.materialised_via_created,
            materialised_via_lazy: meta.materialised_via_lazy,
            recovery_state: recovery.as_str().to_string(),
            finalized: matches!(
                recovery,
                crate::hybrid_v2::reducer::RecoveryStateProjection::Recovered
            ),
            balance_token_count,
            reservation_count,
            active_series_count,
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
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let repo = HybridV2QueryRepository::new(
        entry.deployment_id,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        None,
    );
    let details = repo
        .subaccount_details(&subkey)
        .ok_or_else(|| ApiError::new(ApiErrorCode::SubaccountNotFound, "subkey not found"))?;
    let meta = runtime
        .projection()
        .subaccount_meta
        .get(&subkey)
        .copied()
        .unwrap_or_default();
    let recovery = repo.recovery_state(&subkey);
    let balance_token_count = runtime
        .projection()
        .balances
        .iter()
        .filter(|((sk, _), _)| sk.eq_ignore_ascii_case(&subkey))
        .count();
    let reservation_count = runtime
        .projection()
        .reservations
        .iter()
        .filter(|((sk, _, _), _)| sk.eq_ignore_ascii_case(&subkey))
        .count();
    let active_series_count = runtime
        .projection()
        .active_series
        .get(&subkey)
        .map(|s| s.len())
        .unwrap_or(0);
    let open_order_count = runtime
        .projection()
        .order_lifecycle
        .values()
        .filter(|row| row.subkey.eq_ignore_ascii_case(&subkey) && !row.terminal && !row.cancelled)
        .count();
    let escape_state = repo.escape_state(&subkey);
    let owner_epoch = runtime
        .projection()
        .owner_recovery_epochs
        .get(&details.owner)
        .map(|r| r.epoch_count)
        .unwrap_or(0);
    let subaccount_epoch = runtime
        .projection()
        .subaccount_recovery_epochs
        .get(&subkey)
        .map(|r| r.epoch_count)
        .unwrap_or(0);
    Ok(Json(WithMeta::new(
        SubaccountSummary {
            subkey: details.subkey.clone(),
            owner: details.owner.clone(),
            subaccount_id: details.subaccount_id,
            materialised_via_created: meta.materialised_via_created,
            materialised_via_lazy: meta.materialised_via_lazy,
            recovery_state: recovery.as_str().to_string(),
            finalized: matches!(
                recovery,
                crate::hybrid_v2::reducer::RecoveryStateProjection::Recovered
            ),
            balance_token_count,
            reservation_count,
            active_series_count,
            open_order_count,
            escape_state,
            owner_recovery_epoch: owner_epoch,
            subaccount_recovery_epoch: subaccount_epoch,
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
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let repo = HybridV2QueryRepository::new(
        entry.deployment_id,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        None,
    );
    let balances: Vec<CollateralBalanceRow> = repo.collateral_balances(&subkey);
    let reservations: Vec<ReservationRow> = repo.reservations(&subkey);
    let mut rows: Vec<CollateralRow> = balances
        .into_iter()
        .map(|b| {
            let aggregate = reservations
                .iter()
                .filter(|r| r.token.eq_ignore_ascii_case(&b.token))
                .fold("0".to_string(), |acc, r| {
                    crate::hybrid_v2::reducer::u256_add(&acc, &r.reserved).unwrap_or(acc)
                });
            let available = crate::hybrid_v2::reducer::u256_sub(&b.balance, &aggregate)
                .unwrap_or_else(|| "0".to_string());
            let universe_index = runtime
                .projection()
                .collateral_universe
                .get(&b.token)
                .copied();
            let enabled = !runtime.projection().disabled_tokens.contains(&b.token);
            CollateralRow {
                token: b.token.clone(),
                universe_index,
                enabled,
                balance: b.balance.clone(),
                aggregate_reserved: aggregate,
                available,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.token.cmp(&b.token));
    Ok(Json(WithMeta::new(rows, metadata)))
}

pub async fn get_subaccount_reservations(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<ReservationRow>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let repo = HybridV2QueryRepository::new(
        entry.deployment_id,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        None,
    );
    let mut rows = repo.reservations(&subkey);
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
    let entry = resolve_deployment(&state, q.deployment_id)?;
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let repo = HybridV2QueryRepository::new(
        entry.deployment_id,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        None,
    );
    let active_set = runtime
        .projection()
        .active_series
        .get(&subkey)
        .cloned()
        .unwrap_or_default();
    let mut rows: Vec<PositionResponseRow> = repo
        .active_positions(&subkey)
        .into_iter()
        .map(|(series, row)| PositionResponseRow {
            series_id: series.clone(),
            long_qty_1e8: row.long_qty_1e8,
            short_qty_1e8: row.short_qty_1e8,
            active: active_set.contains(&series),
            last_event_block: row.last_event_block,
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
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let mut rows: Vec<OrderRow> = runtime
        .projection()
        .order_lifecycle
        .iter()
        .filter(|(_, row)| row.subkey.eq_ignore_ascii_case(&subkey))
        .map(|(hash, row)| {
            let remaining =
                crate::hybrid_v2::reducer::u256_sub(&row.total_qty_1e8, &row.filled_qty_1e8)
                    .unwrap_or_else(|| "0".to_string());
            OrderRow {
                order_hash: hash.clone(),
                owner: row.owner.clone(),
                subkey: row.subkey.clone(),
                series_id: row.series_id.clone(),
                side: row.side,
                time_in_force: row.time_in_force,
                total_qty_1e8: row.total_qty_1e8.clone(),
                filled_qty_1e8: row.filled_qty_1e8.clone(),
                remaining_qty_1e8: remaining,
                cancelled: row.cancelled,
                terminal: row.terminal,
                first_seen_block: row.first_seen_block,
                last_event_block: row.last_event_block,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.order_hash.cmp(&b.order_hash));
    rows.truncate(limit);
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
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let repo = HybridV2QueryRepository::new(
        entry.deployment_id,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        None,
    );
    let row = repo
        .order_lifecycle(&oh)
        .ok_or_else(|| ApiError::new(ApiErrorCode::OrderNotFound, "order not found"))?;
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
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    // Public API returns only Complete groups.
    let mut rows: Vec<(String, MatchedExecutionRow)> = runtime
        .projection()
        .matched_executions
        .iter()
        .filter(|(_, r)| {
            r.completion_status == ExecutionCompletion::Complete
                && (r.buyer_subkey.eq_ignore_ascii_case(&subkey)
                    || r.seller_subkey.eq_ignore_ascii_case(&subkey))
        })
        .map(|(id, row)| (id.clone(), row.clone()))
        .collect();
    rows.sort_by(|a, b| (b.1.block_number, &b.0).cmp(&(a.1.block_number, &a.0)));
    rows.truncate(limit);
    Ok(Json(WithMeta::new(rows, metadata)))
}

// -----------------------------------------------------------------
//                          FEES
// -----------------------------------------------------------------

pub async fn get_subaccount_fees(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(q): Query<BasePageQuery>,
) -> Result<Json<WithMeta<Vec<FeeEventRow>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let limit = bounded_limit(q.limit)?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let mut rows: Vec<FeeEventRow> = runtime
        .projection()
        .fee_events
        .iter()
        .filter(|row| {
            row.payer_subkey
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case(&subkey))
                == Some(true)
                || row
                    .receiver_subkey
                    .as_deref()
                    .map(|s| s.eq_ignore_ascii_case(&subkey))
                    == Some(true)
        })
        .cloned()
        .collect();
    rows.sort_by(|a, b| (b.block_number, b.log_index).cmp(&(a.block_number, a.log_index)));
    rows.truncate(limit);
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
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let repo = HybridV2QueryRepository::new(
        entry.deployment_id,
        runtime.projection(),
        runtime.cursor(),
        runtime.readiness().ready,
        None,
    );
    let details = repo
        .subaccount_details(&subkey)
        .ok_or_else(|| ApiError::new(ApiErrorCode::SubaccountNotFound, "subkey not found"))?;
    let recovery = repo.recovery_state(&subkey);
    let escape = repo.escape_state(&subkey);
    let owner_epoch = runtime
        .projection()
        .owner_recovery_epochs
        .get(&details.owner)
        .map(|r| r.epoch_count)
        .unwrap_or(0);
    let subaccount_epoch = runtime
        .projection()
        .subaccount_recovery_epochs
        .get(&subkey)
        .map(|r| r.epoch_count)
        .unwrap_or(0);
    let owner_min_nonce = runtime
        .projection()
        .owner_recovery_epochs
        .get(&details.owner)
        .map(|r| r.min_valid_nonce.clone())
        .unwrap_or_else(|| "0".into());
    let withdrawn = runtime
        .projection()
        .recovery_finalization_withdrawals
        .get(&subkey)
        .copied()
        .unwrap_or(0);
    Ok(Json(WithMeta::new(
        RecoveryResponse {
            subkey: details.subkey.clone(),
            recovery_state: recovery.as_str().to_string(),
            finalized: matches!(
                recovery,
                crate::hybrid_v2::reducer::RecoveryStateProjection::Recovered
            ),
            owner_recovery_epoch: owner_epoch,
            subaccount_recovery_epoch: subaccount_epoch,
            owner_min_valid_nonce: owner_min_nonce,
            escape_state: escape,
            finalization_withdrawal_count: withdrawn,
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

pub async fn get_history(
    State(state): State<HybridV2ApiState>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<WithMeta<Vec<HistoryEvent>>>, ApiError> {
    let consistency = parse_consistency(q.consistency.as_ref())?;
    let limit = bounded_limit(q.limit)?;
    let entry = resolve_deployment(&state, q.deployment_id)?;
    require_ready_canonical(&entry)?;
    let metadata = build_metadata(&entry, consistency)?;
    let runtime = entry
        .runtime
        .read()
        .map_err(|_| ApiError::new(ApiErrorCode::InternalInconsistency, "runtime lock poisoned"))?;
    let filter = q.build_filter()?;
    let filter_h = compute_filter_hash(&filter.to_stable_json());
    let binding = CursorBinding {
        deployment_id: entry.deployment_id,
        consistency,
        filter_hash: filter_h.clone(),
        indexed_head_hash: runtime.cursor().indexed_head_hash.clone(),
    };
    let last_key = if let Some(raw) = q.cursor.as_ref() {
        let cursor = Cursor::decode(raw)?;
        cursor.verify(&binding)?;
        Some(cursor.last_key)
    } else {
        None
    };
    let finalized_block = runtime.metrics().finalized_block;
    let mut all = build_history(&runtime, entry.manifest.chain_id, finalized_block, &filter);
    sort_desc(&mut all);
    let page = take_page_after(&all, last_key.as_deref(), limit);
    let next = page.last().map(|last| {
        Cursor {
            version: 1,
            binding: binding.clone(),
            last_key: last.event_id.clone(),
        }
        .encode()
    });
    Ok(Json(WithMeta::new(page, metadata).with_cursor(next)))
}

pub async fn get_subaccount_history(
    State(state): State<HybridV2ApiState>,
    Path(subkey_raw): Path<String>,
    Query(mut q): Query<HistoryQuery>,
) -> Result<Json<WithMeta<Vec<HistoryEvent>>>, ApiError> {
    let subkey = parse_bytes32(&subkey_raw)?;
    q.subkey = Some(subkey);
    get_history(State(state), Query(q)).await
}

pub async fn get_owner_history(
    State(state): State<HybridV2ApiState>,
    Path(owner_raw): Path<String>,
    Query(mut q): Query<HistoryQuery>,
) -> Result<Json<WithMeta<Vec<HistoryEvent>>>, ApiError> {
    let owner = parse_address(&owner_raw)?;
    q.owner = Some(owner);
    get_history(State(state), Query(q)).await
}
