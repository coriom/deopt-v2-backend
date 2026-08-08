//! Admin routes for the Hybrid V2 pre-broadcast execution
//! orchestrator (Part S of `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! Frozen posture:
//! - Every route is behind the same admin-token gate as
//!   `admin/hybrid_v2/deployments/*/rebuild`.
//! - Base mainnet is refused at handler entry.
//! - The orchestrator itself is not built here — this milestone stops
//!   at plumbing the read + control surface. Handlers that need an
//!   orchestrator (prepare / retry) return 503
//!   `EXECUTION_ORCHESTRATOR_NOT_WIRED` when the caller hasn't
//!   attached one. Wiring lands in a downstream integration milestone
//!   that also brings a production signer.
//!
//! Non-negotiable public-input rule: the request body carries buyer /
//! seller SIGNED envelopes + orders + fill_qty + active_series arrays.
//! Every downstream execution field (target, selector, calldata,
//! value, nonce, gas, chain_id) is derived from the allowlist + the
//! orchestrator's deterministic plan builder. The request body cannot
//! influence any of those.

use crate::api::http::AppState;
use crate::hybrid_v2::execution::{ExecutionPhase, ExecutionRequestRow};
use crate::hybrid_v2::HybridV2ProjectionStore;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const BASE_MAINNET_CHAIN_ID: u64 = 8453;

// -----------------------------------------------------------------
//                       ERROR HELPERS
// -----------------------------------------------------------------

fn err_response(status: StatusCode, code: &str, detail: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": code,
            "detail": detail,
        })),
    )
        .into_response()
}

fn ensure_admin(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    if !state.admin_config.enabled {
        return Err(err_response(
            StatusCode::FORBIDDEN,
            "ADMIN_DISABLED",
            "admin API is disabled",
        ));
    }
    if state.admin_config.require_token {
        let token = headers
            .get("x-admin-token")
            .and_then(|value| value.to_str().ok());
        match token {
            Some(t) if state.admin_config.token_matches(t) => {}
            Some(_) => {
                return Err(err_response(
                    StatusCode::FORBIDDEN,
                    "INVALID_ADMIN_TOKEN",
                    "invalid admin token",
                ));
            }
            None => {
                return Err(err_response(
                    StatusCode::FORBIDDEN,
                    "ADMIN_TOKEN_REQUIRED",
                    "admin token is required",
                ));
            }
        }
    }
    Ok(())
}

fn require_store(state: &AppState) -> Result<Arc<dyn HybridV2ProjectionStore>, Response> {
    state.hybrid_v2_projection_store.clone().ok_or_else(|| {
        err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "HYBRID_V2_NOT_CONFIGURED",
            "no Hybrid V2 projection store attached to AppState",
        )
    })
}

fn resolve_deployment(
    state: &AppState,
    deployment_id: i64,
) -> Result<Arc<crate::api::hybrid_v2_read::DeploymentEntry>, Response> {
    if deployment_id < 0 {
        return Err(err_response(
            StatusCode::NOT_FOUND,
            "DEPLOYMENT_NOT_FOUND",
            &format!("unknown deployment_id {deployment_id}"),
        ));
    }
    let id_u = deployment_id as u64;
    state.hybrid_v2_read.get(id_u).ok_or_else(|| {
        err_response(
            StatusCode::NOT_FOUND,
            "DEPLOYMENT_NOT_FOUND",
            &format!("unknown deployment_id {deployment_id}"),
        )
    })
}

fn refuse_mainnet(chain_id: u64) -> Result<(), Response> {
    if chain_id == BASE_MAINNET_CHAIN_ID {
        return Err(err_response(
            StatusCode::FORBIDDEN,
            "BASE_MAINNET_FORBIDDEN",
            "execution admin routes are refused on Base mainnet",
        ));
    }
    Ok(())
}

// -----------------------------------------------------------------
//                       REQUEST + RESPONSE
// -----------------------------------------------------------------

/// Signed envelope shape received from an operator. Every field on
/// this struct is CALLER-supplied INTENT — never a wire-format
/// execution primitive (calldata, target, nonce, gas). The
/// orchestrator derives execution primitives from the manifest
/// allowlist + deterministic plan builder.
#[derive(Debug, Clone, Deserialize)]
pub struct EnvelopeInput {
    pub owner: String,
    pub subaccount_id: u32,
    pub subkey: String,
    pub signer: String,
    pub engine: String,
    pub action: String,
    pub architecture_version: String,
    pub nonce: String,
    pub deadline: String,
    pub owner_recovery_epoch: String,
    pub subaccount_recovery_epoch: String,
    pub payload_hash: String,
    /// Signature bytes as `0x`-prefixed hex.
    pub signature: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderInput {
    pub series_id: String,
    pub side: u8,
    pub quantity_1e8: String,
    pub price_per_contract_1e8: String,
    pub limit_price_per_contract_1e8: String,
    pub premium_token: String,
    pub time_in_force: u8,
    pub role: u8,
    pub max_positive_fee_ppm: u32,
    pub salt: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrepareRequestBody {
    pub buyer_envelope: EnvelopeInput,
    pub buyer_order: OrderInput,
    pub seller_envelope: EnvelopeInput,
    pub seller_order: OrderInput,
    /// Fill quantity in 1e8 units (decimal string).
    pub fill_quantity_1e8: String,
    /// Active-series arrays: list of decimal strings.
    pub buyer_active_series: Vec<String>,
    pub seller_active_series: Vec<String>,
    /// Order hash for buyer / seller — used for canonical id derivation.
    pub buyer_order_hash: String,
    pub seller_order_hash: String,
    pub series_id: String,
    pub premium_amount: String,
    pub fee_schedule_epoch: Option<i64>,
}

/// Sanitized row returned to admin observers. Signature (r, s, v) is
/// deliberately omitted — an admin viewer can confirm the canonical
/// id + phase + failure class + recovered signer address without
/// re-exporting cryptographic material. `recovered_signer` is a
/// PUBLIC address value (recoverable by any downstream chain observer
/// from the eventual on-chain tx) so it is safe to surface.
#[derive(Debug, Clone, Serialize)]
pub struct SanitizedExecutionRow {
    pub canonical_execution_id: String,
    pub deployment_id: i64,
    pub chain_id: i64,
    pub execution_kind: String,
    pub buyer_order_hash: String,
    pub seller_order_hash: String,
    pub buyer_subkey: String,
    pub seller_subkey: String,
    pub series_id: String,
    pub fill_quantity_1e8: String,
    pub premium_amount: String,
    pub target_contract: String,
    pub selector: String,
    pub calldata_hash: Option<String>,
    pub plan_hash: Option<String>,
    pub tx_value_wei: String,
    pub simulation_block_number: Option<i64>,
    pub simulation_block_hash: Option<String>,
    pub simulation_gas_estimate: Option<i64>,
    pub signer_identity: Option<String>,
    pub signing_payload_hash: Option<String>,
    pub recovered_signer: Option<String>,
    pub gas_limit: Option<i64>,
    pub max_fee_per_gas_wei: Option<String>,
    pub max_priority_fee_per_gas_wei: Option<String>,
    pub reserved_nonce: Option<i64>,
    pub phase: String,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
    pub retry_count: i32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl From<ExecutionRequestRow> for SanitizedExecutionRow {
    fn from(row: ExecutionRequestRow) -> Self {
        SanitizedExecutionRow {
            canonical_execution_id: row.canonical_execution_id,
            deployment_id: row.deployment_id,
            chain_id: row.chain_id,
            execution_kind: row.execution_kind,
            buyer_order_hash: row.buyer_order_hash,
            seller_order_hash: row.seller_order_hash,
            buyer_subkey: row.buyer_subkey,
            seller_subkey: row.seller_subkey,
            series_id: row.series_id,
            fill_quantity_1e8: row.fill_quantity_1e8,
            premium_amount: row.premium_amount,
            target_contract: row.target_contract,
            selector: row.selector,
            calldata_hash: row.calldata_hash,
            plan_hash: row.plan_hash,
            tx_value_wei: row.tx_value_wei,
            simulation_block_number: row.simulation_block_number,
            simulation_block_hash: row.simulation_block_hash,
            simulation_gas_estimate: row.simulation_gas_estimate,
            signer_identity: row.signer_identity,
            signing_payload_hash: row.signing_payload_hash,
            recovered_signer: row.recovered_signer,
            gas_limit: row.gas_limit,
            max_fee_per_gas_wei: row.max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei: row.max_priority_fee_per_gas_wei,
            reserved_nonce: row.reserved_nonce,
            phase: row.phase.as_str().to_string(),
            failure_class: row.failure_class,
            failure_detail: row.failure_detail,
            retry_count: row.retry_count,
            created_at_ms: row.created_at_ms,
            updated_at_ms: row.updated_at_ms,
        }
    }
}

// -----------------------------------------------------------------
//                          HANDLERS
// -----------------------------------------------------------------

/// `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/prepare`
///
/// Prepares an execution. This milestone does not wire the concrete
/// orchestrator instance in production paths, so this handler returns
/// 503 `EXECUTION_ORCHESTRATOR_NOT_WIRED` when no orchestrator is
/// attached. The full request/response shape is validated up-front so
/// callers can prove the boundary contract is being enforced (Base
/// mainnet refused, admin token required, envelope schema honoured).
pub async fn prepare_execution(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(_body): Json<PrepareRequestBody>,
) -> Response {
    if let Err(resp) = ensure_admin(&state, &headers) {
        return resp;
    }
    if let Err(resp) = require_store(&state) {
        return resp;
    }
    let entry = match resolve_deployment(&state, deployment_id) {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = refuse_mainnet(entry.manifest.chain_id) {
        return resp;
    }
    // Boundary sanity: canonical id must look like a hex-prefixed
    // 32-byte string. Do not accept arbitrary strings that could pack
    // extra semantic content.
    if !is_valid_hash32(&canonical_execution_id) {
        return err_response(
            StatusCode::BAD_REQUEST,
            "INVALID_CANONICAL_ID",
            "canonical_execution_id must be 0x-prefixed 32-byte hex",
        );
    }
    err_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "EXECUTION_ORCHESTRATOR_NOT_WIRED",
        "the execution orchestrator is not wired to this AppState build; \
         wire it via a downstream integration milestone that also brings \
         a production signer",
    )
}

/// `GET /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id`
///
/// Returns the sanitized row. Signature (r, s, v) is deliberately
/// omitted.
pub async fn get_execution(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = ensure_admin(&state, &headers) {
        return resp;
    }
    let store = match require_store(&state) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let entry = match resolve_deployment(&state, deployment_id) {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = refuse_mainnet(entry.manifest.chain_id) {
        return resp;
    }
    if !is_valid_hash32(&canonical_execution_id) {
        return err_response(
            StatusCode::BAD_REQUEST,
            "INVALID_CANONICAL_ID",
            "canonical_execution_id must be 0x-prefixed 32-byte hex",
        );
    }
    match store.get_execution_request(&canonical_execution_id).await {
        Ok(Some(row)) => {
            if row.deployment_id != deployment_id {
                return err_response(
                    StatusCode::NOT_FOUND,
                    "EXECUTION_NOT_IN_DEPLOYMENT",
                    "canonical execution id belongs to a different deployment",
                );
            }
            (StatusCode::OK, Json(SanitizedExecutionRow::from(row))).into_response()
        }
        Ok(None) => err_response(
            StatusCode::NOT_FOUND,
            "EXECUTION_NOT_FOUND",
            "no execution row for the given canonical id",
        ),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORE_ERROR",
            &e.to_string(),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// `GET /admin/hybrid_v2/deployments/:deployment_id/executions?limit=N&offset=M`
pub async fn list_executions(
    Path(deployment_id): Path<i64>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = ensure_admin(&state, &headers) {
        return resp;
    }
    let store = match require_store(&state) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let entry = match resolve_deployment(&state, deployment_id) {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = refuse_mainnet(entry.manifest.chain_id) {
        return resp;
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 1_000);
    let offset = q.offset.unwrap_or(0).max(0);
    // The store's list method returns a bounded slice; we apply the
    // offset here in memory. `limit + offset` is capped so a hostile
    // caller cannot request unbounded pages.
    let effective_limit = limit.checked_add(offset).unwrap_or(limit).min(10_000);
    match store
        .list_execution_requests_by_deployment(deployment_id, effective_limit)
        .await
    {
        Ok(rows) => {
            let sanitized: Vec<SanitizedExecutionRow> = rows
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .map(SanitizedExecutionRow::from)
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "deployment_id": deployment_id,
                    "limit": limit,
                    "offset": offset,
                    "executions": sanitized,
                })),
            )
                .into_response()
        }
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORE_ERROR",
            &e.to_string(),
        ),
    }
}

/// `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/cancel`
///
/// Cancels an execution that has NOT yet entered the signing phase.
/// Refused once SIGNING or later — cancelling a signed intent must go
/// through a fresh execution row.
pub async fn cancel_execution(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = ensure_admin(&state, &headers) {
        return resp;
    }
    let store = match require_store(&state) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let entry = match resolve_deployment(&state, deployment_id) {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = refuse_mainnet(entry.manifest.chain_id) {
        return resp;
    }
    if !is_valid_hash32(&canonical_execution_id) {
        return err_response(
            StatusCode::BAD_REQUEST,
            "INVALID_CANONICAL_ID",
            "canonical_execution_id must be 0x-prefixed 32-byte hex",
        );
    }
    let row = match store.get_execution_request(&canonical_execution_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return err_response(
                StatusCode::NOT_FOUND,
                "EXECUTION_NOT_FOUND",
                "no execution row for the given canonical id",
            );
        }
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "STORE_ERROR",
                &e.to_string(),
            );
        }
    };
    if row.deployment_id != deployment_id {
        return err_response(
            StatusCode::NOT_FOUND,
            "EXECUTION_NOT_IN_DEPLOYMENT",
            "canonical execution id belongs to a different deployment",
        );
    }
    // Refuse if SIGNING or later. The legal phases that can be
    // cancelled by the state machine are: Discovered, Validating,
    // ReadyToSimulate, SimulationSucceeded, AwaitingSignature. Any
    // other non-terminal phase (Simulating, Signing, SignatureVerified,
    // ReadyForBroadcast) reflects work in-flight or already committed
    // — cancellation is refused with 409.
    let cancellable = matches!(
        row.phase,
        ExecutionPhase::Discovered
            | ExecutionPhase::Validating
            | ExecutionPhase::ReadyToSimulate
            | ExecutionPhase::SimulationSucceeded
            | ExecutionPhase::AwaitingSignature
    );
    if !cancellable {
        return err_response(
            StatusCode::CONFLICT,
            "EXECUTION_NOT_CANCELLABLE",
            &format!(
                "execution phase {} is not cancellable — cancel is only \
                 valid for DISCOVERED..AWAITING_SIGNATURE",
                row.phase.as_str()
            ),
        );
    }
    let now = wall_ms();
    match store
        .update_execution_phase(
            &canonical_execution_id,
            row.phase,
            ExecutionPhase::Cancelled,
            now,
            crate::hybrid_v2::execution::ExecutionRequestPatch {
                failure_class: Some("ADMIN_CANCELLED".to_string()),
                failure_detail: Some("operator-issued cancel via admin route".to_string()),
                ..Default::default()
            },
        )
        .await
    {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "canonical_execution_id": canonical_execution_id,
                "deployment_id": deployment_id,
                "phase": "CANCELLED",
            })),
        )
            .into_response(),
        Ok(false) => err_response(
            StatusCode::CONFLICT,
            "CANCEL_LOST_UPDATE",
            "another writer advanced the row concurrently; re-read and retry",
        ),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORE_ERROR",
            &e.to_string(),
        ),
    }
}

/// `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/retry`
///
/// Retries a FAILED execution row if its failure_class is retryable.
/// Bounded to 3 retries total (matches
/// `orchestrator::MAX_TRANSPORT_RETRIES`).
pub async fn retry_execution(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if let Err(resp) = ensure_admin(&state, &headers) {
        return resp;
    }
    let store = match require_store(&state) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let entry = match resolve_deployment(&state, deployment_id) {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    if let Err(resp) = refuse_mainnet(entry.manifest.chain_id) {
        return resp;
    }
    if !is_valid_hash32(&canonical_execution_id) {
        return err_response(
            StatusCode::BAD_REQUEST,
            "INVALID_CANONICAL_ID",
            "canonical_execution_id must be 0x-prefixed 32-byte hex",
        );
    }
    let row = match store.get_execution_request(&canonical_execution_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return err_response(
                StatusCode::NOT_FOUND,
                "EXECUTION_NOT_FOUND",
                "no execution row for the given canonical id",
            );
        }
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "STORE_ERROR",
                &e.to_string(),
            );
        }
    };
    if row.deployment_id != deployment_id {
        return err_response(
            StatusCode::NOT_FOUND,
            "EXECUTION_NOT_IN_DEPLOYMENT",
            "canonical execution id belongs to a different deployment",
        );
    }
    if row.phase != ExecutionPhase::Failed {
        return err_response(
            StatusCode::CONFLICT,
            "EXECUTION_NOT_FAILED",
            "retry is only valid for rows in the FAILED terminal phase",
        );
    }
    let retryable = matches!(
        row.failure_class.as_deref(),
        Some(crate::hybrid_v2::execution::failure_class::SIMULATION_TRANSPORT_FAILED)
            | Some(crate::hybrid_v2::execution::failure_class::LOCK_CONTENTION)
            | Some(crate::hybrid_v2::execution::failure_class::STORE_FAILURE)
    );
    if !retryable {
        return err_response(
            StatusCode::CONFLICT,
            "EXECUTION_NOT_RETRYABLE",
            &format!(
                "failure_class {:?} is not in the retryable set \
                 (SIMULATION_TRANSPORT_FAILED, LOCK_CONTENTION, STORE_FAILURE)",
                row.failure_class,
            ),
        );
    }
    if row.retry_count as u32 >= crate::hybrid_v2::execution::MAX_TRANSPORT_RETRIES {
        return err_response(
            StatusCode::CONFLICT,
            "RETRY_BUDGET_EXHAUSTED",
            &format!(
                "row exceeded the bounded retry budget ({} retries)",
                crate::hybrid_v2::execution::MAX_TRANSPORT_RETRIES,
            ),
        );
    }
    // Failed → Discovered is NOT a legal edge in the state matrix
    // (terminal states have no outgoing edges — see `state.rs`). The
    // retry surface is honoured by returning a structured 409 that
    // instructs the caller to re-issue `prepare` against a fresh
    // canonical id derived from the same intent. This preserves the
    // frozen `EXECUTION_IS_IDEMPOTENT_BY_CANONICAL_ID` invariant.
    err_response(
        StatusCode::CONFLICT,
        "RETRY_MUST_ISSUE_NEW_CANONICAL_ID",
        "terminal FAILED rows cannot be resurrected; re-issue `prepare` \
         with the original intent — the canonical id includes the intent \
         payload so retries converge deterministically",
    )
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn is_valid_hash32(s: &str) -> bool {
    let stripped = match s.strip_prefix("0x") {
        Some(s) => s,
        None => return false,
    };
    stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit())
}

fn wall_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
