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
use crate::hybrid_v2::execution::broadcast_state::{BroadcastPhase, BroadcastStatePatch};
use crate::hybrid_v2::execution::{ExecutionPhase, ExecutionRequestRow};
use crate::hybrid_v2::HybridV2ProjectionStore;
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
/// Prepare-only entry point (Part J). When
/// `hybrid_v2_execution_orchestrator` is `None`, returns a structured
/// 503 whose body includes the persisted
/// `hybrid_v2_execution_unavailable_reason` (redacted upstream).
/// Otherwise builds a `PreparationIntent` from the caller-supplied
/// envelopes/orders/fill_qty/active_series (NEVER a target, selector,
/// calldata, nonce, gas, fee, signer identity, chain, or RPC
/// endpoint — the `deny_unknown_fields` deserializer rejects any of
/// those) and drives the orchestrator.
pub async fn prepare_execution(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<PrepareRequestBody>,
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
    let Some(orchestrator) = state.hybrid_v2_execution_orchestrator.clone() else {
        // Structured 503 including the availability reason so an
        // operator can distinguish "execution disabled" from "signer
        // config failed startup" from "AWS transport not yet wired".
        let reason = state
            .hybrid_v2_execution_unavailable_reason
            .clone()
            .unwrap_or_else(|| "EXECUTION_ORCHESTRATOR_NOT_WIRED".to_string());
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "EXECUTION_ORCHESTRATOR_NOT_WIRED",
                "detail": "no execution orchestrator wired to this AppState build",
                "availability": {
                    "state": "NotConfigured",
                    "reason": reason,
                },
            })),
        )
            .into_response();
    };
    // Build the PreparationIntent from the JSON body. Every field
    // here is CALLER-INTENT — the orchestrator derives target,
    // selector, calldata, nonce, gas, chain_id from the manifest
    // allowlist + deterministic plan builder.
    let intent = match build_intent_from_body(entry.manifest.clone(), body) {
        Ok(intent) => intent,
        Err(err) => {
            return err_response(StatusCode::BAD_REQUEST, "INVALID_PREPARE_BODY", &err);
        }
    };
    match orchestrator.prepare(intent).await {
        Ok(outcome) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "canonical_execution_id": outcome.canonical_execution_id,
                "terminal_phase": outcome.terminal_phase.as_str(),
                "failure_class": outcome.failure_class,
                "failure_detail": outcome.failure_detail,
                "plan_hash": outcome.plan_hash,
                "signing_payload_hash": outcome.signing_payload_hash,
                "simulation_gas_estimate": outcome.simulation_gas_estimate,
                "reserved_nonce": outcome.reserved_nonce,
                "attempts": outcome.attempts,
            })),
        )
            .into_response(),
        Err(err) => match err {
            crate::hybrid_v2::execution::OrchestrationError::LockContention => err_response(
                StatusCode::CONFLICT,
                "EXECUTION_LOCK_CONTENTION",
                &err.to_string(),
            ),
            crate::hybrid_v2::execution::OrchestrationError::StoreFailure(msg) => {
                err_response(StatusCode::INTERNAL_SERVER_ERROR, "STORE_ERROR", &msg)
            }
            crate::hybrid_v2::execution::OrchestrationError::Unrecoverable(msg) => err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ORCHESTRATION_UNRECOVERABLE",
                &msg,
            ),
        },
    }
}

// -----------------------------------------------------------------
//              body → PreparationIntent conversion
// -----------------------------------------------------------------

fn build_intent_from_body(
    manifest: crate::hybrid_v2::manifest::ManifestParams,
    body: PrepareRequestBody,
) -> Result<crate::hybrid_v2::execution::PreparationIntent, String> {
    let buyer_envelope = parse_envelope("buyer_envelope", &body.buyer_envelope)?;
    let seller_envelope = parse_envelope("seller_envelope", &body.seller_envelope)?;
    let buyer_signature = parse_bytes("buyer_envelope.signature", &body.buyer_envelope.signature)?;
    let seller_signature =
        parse_bytes("seller_envelope.signature", &body.seller_envelope.signature)?;
    let buyer_order = parse_order("buyer_order", &body.buyer_order)?;
    let seller_order = parse_order("seller_order", &body.seller_order)?;
    let fill_quantity_1e8 = body
        .fill_quantity_1e8
        .parse::<u128>()
        .map_err(|e| format!("fill_quantity_1e8 not u128: {e}"))?;
    let buyer_active_series = parse_u256_vec("buyer_active_series", &body.buyer_active_series)?;
    let seller_active_series = parse_u256_vec("seller_active_series", &body.seller_active_series)?;
    Ok(crate::hybrid_v2::execution::PreparationIntent {
        manifest,
        // Runtime state + readiness are treated as ready by the admin
        // route. The orchestrator's preflight then re-checks against
        // the store's live cursors — an unhealthy runtime lands in a
        // PREFLIGHT_REJECTED terminal, not in a silent hang.
        runtime_state: crate::hybrid_v2::reducer::ProjectionState::default(),
        readiness: crate::hybrid_v2::readiness::ReadinessReport {
            runtime: crate::hybrid_v2::readiness::ReadinessState::ready(),
            rebuild: crate::hybrid_v2::readiness::ReadinessState::ready(),
            reconciliation: crate::hybrid_v2::readiness::ReadinessState::ready(),
        },
        buyer_envelope,
        buyer_signature,
        buyer_order,
        seller_envelope,
        seller_signature,
        seller_order,
        fill_quantity_1e8,
        buyer_active_series,
        seller_active_series,
        buyer_order_hash: body.buyer_order_hash,
        seller_order_hash: body.seller_order_hash,
        buyer_subkey: body.buyer_envelope.subkey,
        seller_subkey: body.seller_envelope.subkey,
        series_id: body.series_id,
        premium_amount: body.premium_amount,
        fee_schedule_epoch: body.fee_schedule_epoch,
    })
}

fn parse_envelope(
    ctx: &str,
    input: &EnvelopeInput,
) -> Result<crate::hybrid_v2::execution::plan::SignedActionEnvelope, String> {
    Ok(crate::hybrid_v2::execution::plan::SignedActionEnvelope {
        owner: parse_address_field(&format!("{ctx}.owner"), &input.owner)?,
        subaccountId: input.subaccount_id,
        subKey: parse_bytes32(&format!("{ctx}.subkey"), &input.subkey)?,
        signer: parse_address_field(&format!("{ctx}.signer"), &input.signer)?,
        engine: parse_address_field(&format!("{ctx}.engine"), &input.engine)?,
        action: parse_bytes32(&format!("{ctx}.action"), &input.action)?,
        architectureVersion: parse_u256(
            &format!("{ctx}.architecture_version"),
            &input.architecture_version,
        )?,
        nonce: parse_u256(&format!("{ctx}.nonce"), &input.nonce)?,
        deadline: parse_u256(&format!("{ctx}.deadline"), &input.deadline)?,
        ownerRecoveryEpoch: parse_u256(
            &format!("{ctx}.owner_recovery_epoch"),
            &input.owner_recovery_epoch,
        )?,
        subaccountRecoveryEpoch: parse_u256(
            &format!("{ctx}.subaccount_recovery_epoch"),
            &input.subaccount_recovery_epoch,
        )?,
        payloadHash: parse_bytes32(&format!("{ctx}.payload_hash"), &input.payload_hash)?,
    })
}

fn parse_order(
    ctx: &str,
    input: &OrderInput,
) -> Result<crate::hybrid_v2::execution::plan::OptionOrder, String> {
    Ok(crate::hybrid_v2::execution::plan::OptionOrder {
        seriesId: parse_u256(&format!("{ctx}.series_id"), &input.series_id)?,
        side: input.side,
        quantity1e8: input
            .quantity_1e8
            .parse::<u128>()
            .map_err(|e| format!("{ctx}.quantity_1e8 not u128: {e}"))?,
        pricePerContract1e8: input
            .price_per_contract_1e8
            .parse::<u128>()
            .map_err(|e| format!("{ctx}.price_per_contract_1e8 not u128: {e}"))?,
        limitPricePerContract1e8: input
            .limit_price_per_contract_1e8
            .parse::<u128>()
            .map_err(|e| format!("{ctx}.limit_price_per_contract_1e8 not u128: {e}"))?,
        premiumToken: parse_address_field(&format!("{ctx}.premium_token"), &input.premium_token)?,
        timeInForce: input.time_in_force,
        role: input.role,
        maxPositiveFeePpm: input.max_positive_fee_ppm,
        salt: parse_bytes32(&format!("{ctx}.salt"), &input.salt)?,
    })
}

fn parse_bytes(ctx: &str, s: &str) -> Result<Bytes, String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| format!("{ctx} must be 0x-prefixed hex"))?;
    if stripped.len() % 2 != 0 {
        return Err(format!("{ctx} hex length must be even"));
    }
    let mut out = Vec::with_capacity(stripped.len() / 2);
    for i in (0..stripped.len()).step_by(2) {
        out.push(
            u8::from_str_radix(&stripped[i..i + 2], 16)
                .map_err(|e| format!("{ctx} hex parse: {e}"))?,
        );
    }
    Ok(Bytes::from(out))
}

fn parse_bytes32(ctx: &str, s: &str) -> Result<FixedBytes<32>, String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| format!("{ctx} must be 0x-prefixed 32-byte hex"))?;
    if stripped.len() != 64 {
        return Err(format!("{ctx} must be 32 bytes hex"));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16)
            .map_err(|e| format!("{ctx} hex parse: {e}"))?;
    }
    Ok(FixedBytes::from(out))
}

fn parse_address_field(ctx: &str, s: &str) -> Result<Address, String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| format!("{ctx} must be 0x-prefixed 20-byte hex"))?;
    if stripped.len() != 40 {
        return Err(format!("{ctx} must be 20 bytes hex"));
    }
    let mut bytes = [0u8; 20];
    for i in 0..20 {
        bytes[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16)
            .map_err(|e| format!("{ctx} hex parse: {e}"))?;
    }
    Ok(Address::from(bytes))
}

fn parse_u256(ctx: &str, s: &str) -> Result<U256, String> {
    U256::from_str_radix(s.trim(), 10).map_err(|e| format!("{ctx} not decimal U256: {e}"))
}

fn parse_u256_vec(ctx: &str, list: &[String]) -> Result<Vec<U256>, String> {
    let mut out = Vec::with_capacity(list.len());
    for (i, s) in list.iter().enumerate() {
        out.push(parse_u256(&format!("{ctx}[{i}]"), s)?);
    }
    Ok(out)
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

// -----------------------------------------------------------------
//   BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1 (Part S) —
//   BROADCAST ADMIN CONTROLS
//
//   Every request body carries NO wire-format execution primitive.
//   `#[serde(deny_unknown_fields)]` on every struct guarantees the
//   handler refuses payloads that try to inject raw tx bytes, r/s/v,
//   nonce, gas, fee, chain, signer identity, or RPC endpoint. Refused
//   payloads return 400 INVALID_ADMIN_BODY before any store or RPC
//   interaction.
// -----------------------------------------------------------------

/// Empty request body for `/broadcast` — the handler derives all
/// execution primitives from the persisted execution + broadcast rows.
/// `deny_unknown_fields` refuses any extraneous field (raw tx, r/s/v,
/// nonce, target, etc.).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BroadcastRequestBody {}

/// Empty body for the recheck / resend endpoints. Same
/// `deny_unknown_fields` rule as `BroadcastRequestBody`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyAdminBody {}

/// Manual-intervention body — only `action` + `detail` are accepted.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualInterventionBody {
    pub action: String,
    pub detail: String,
}

/// Sanitized broadcast state row surfaced to admin observers. Signature
/// bytes, raw envelope bytes, and provider connection details are
/// deliberately omitted.
#[derive(Debug, Clone, Serialize)]
pub struct SanitizedBroadcastRow {
    pub canonical_execution_id: String,
    pub phase: String,
    pub tx_hash: Option<String>,
    pub envelope_hash: Option<String>,
    pub envelope_bytes_hash: Option<String>,
    pub submission_attempt_count: i32,
    pub first_submission_at_ms: Option<i64>,
    pub last_submission_at_ms: Option<i64>,
    pub provider_classification: Option<String>,
    pub receipt_tx_hash: Option<String>,
    pub receipt_block_number: Option<i64>,
    pub receipt_block_hash: Option<String>,
    pub receipt_status: Option<i16>,
    pub gas_used: Option<i64>,
    pub effective_gas_price_wei: Option<String>,
    pub actual_tx_cost_wei: Option<String>,
    pub confirmation_count: i32,
    pub canonicality_state: String,
    pub correlated_execution_status: Option<String>,
    pub last_checked_head: Option<i64>,
    pub last_checked_finalized_head: Option<i64>,
    pub reorg_count: i32,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
    pub terminal_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl From<crate::hybrid_v2::execution::BroadcastStateRow> for SanitizedBroadcastRow {
    fn from(row: crate::hybrid_v2::execution::BroadcastStateRow) -> Self {
        SanitizedBroadcastRow {
            canonical_execution_id: row.canonical_execution_id,
            phase: row.phase.as_str().to_string(),
            tx_hash: row.tx_hash,
            envelope_hash: row.envelope_hash,
            envelope_bytes_hash: row.envelope_bytes_hash,
            submission_attempt_count: row.submission_attempt_count,
            first_submission_at_ms: row.first_submission_at_ms,
            last_submission_at_ms: row.last_submission_at_ms,
            provider_classification: row.provider_classification,
            receipt_tx_hash: row.receipt_tx_hash,
            receipt_block_number: row.receipt_block_number,
            receipt_block_hash: row.receipt_block_hash,
            receipt_status: row.receipt_status,
            gas_used: row.gas_used,
            effective_gas_price_wei: row.effective_gas_price_wei,
            actual_tx_cost_wei: row.actual_tx_cost_wei,
            confirmation_count: row.confirmation_count,
            canonicality_state: row.canonicality_state,
            correlated_execution_status: row.correlated_execution_status,
            last_checked_head: row.last_checked_head,
            last_checked_finalized_head: row.last_checked_finalized_head,
            reorg_count: row.reorg_count,
            failure_class: row.failure_class,
            failure_detail: row.failure_detail,
            terminal_at_ms: row.terminal_at_ms,
            created_at_ms: row.created_at_ms,
            updated_at_ms: row.updated_at_ms,
        }
    }
}

fn broadcast_config_or_disabled(
    state: &AppState,
) -> Result<crate::hybrid_v2::config::HybridV2ExecutionConfig, Response> {
    let cfg = state.hybrid_v2_execution_config.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "BROADCAST_DISABLED",
                "detail": "hybrid_v2_execution_config not attached to AppState",
                "reason": "CONFIG_MISSING",
            })),
        )
            .into_response()
    })?;
    if !cfg.broadcast_enabled {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "BROADCAST_DISABLED",
                "detail": "broadcast_enabled = false in hybrid_v2_execution_config",
                "reason": "BROADCAST_DISABLED_BY_CONFIG",
            })),
        )
            .into_response());
    }
    // Extra defense-in-depth: refuse Base mainnet at the config layer.
    if cfg
        .allowed_broadcast_chain_ids
        .contains(&BASE_MAINNET_CHAIN_ID)
    {
        return Err(err_response(
            StatusCode::FORBIDDEN,
            "BASE_MAINNET_FORBIDDEN",
            "allowed_broadcast_chain_ids contains 8453 — refused",
        ));
    }
    Ok(cfg)
}

/// `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/broadcast`
///
/// Empty body (`{}`) — any extraneous field is rejected by the
/// deserializer.
///
/// BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1 (Part C, D):
/// this handler now closes the fresh-submit gap. Branching on the
/// current broadcast phase:
///
/// * `None` (no broadcast row yet) or `BroadcastDisabled` /
///   `ReadyForBroadcast` — FRESH SUBMIT PATH. The handler hydrates a
///   full `ExecutionPlan` + `SignedTx` from the persisted execution
///   row via `broadcast_reconstruction::{reconstruct_plan,
///   reconstruct_signed}`, builds a `BroadcastPolicyFirewall` +
///   `ReadinessReport::ready`, and calls `outbox.submit(...)`. The
///   outbox owns byte-serialization, phase advance, RPC send, and
///   provider-hash verification.
///
/// * `Broadcasting`, `SubmissionUnknown`, or `Dropped` — RESUME PATH
///   (existing behavior). `outbox.resume(...)` observes any in-flight
///   tx and updates state without a fresh RPC submission.
///
/// * Any other in-flight or terminal phase (Submitted, Pending,
///   MinedSuccess, MinedReverted, Confirming, Confirmed, Reorged,
///   CancelledBeforeBroadcast, ManualInterventionRequired) — the
///   handler returns 200 with the current sanitized status. No RPC
///   contact.
///
/// The frozen safety rules hold on every path:
///   * Base mainnet chain_id refused at handler entry.
///   * Body cannot inject any wire-format primitive
///     (`deny_unknown_fields`).
///   * `outbox.submit` re-runs the full firewall before send.
///   * `broadcast_reconstruction::reconstruct_plan` recomputes
///     keccak(calldata_bytes) == calldata_hash AND plan_hash from
///     scratch and refuses on any mismatch — no RPC on tampered rows.
pub async fn admin_broadcast_execution(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(_body): Json<BroadcastRequestBody>,
) -> Response {
    use crate::hybrid_v2::execution::broadcast_firewall::{
        BroadcastFirewallConfig, BroadcastPolicyFirewall,
    };
    use crate::hybrid_v2::execution::broadcast_reconstruction::{
        reconstruct_plan, reconstruct_signed, ReconstructionError,
    };
    use crate::hybrid_v2::execution::target_policy::TargetPolicy;
    use crate::hybrid_v2::readiness::{ReadinessReport, ReadinessState};

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
    // Outbox availability gate. Broadcast RPC + config also required
    // (the config carries the gas / firewall / allowed-chain surface).
    let outbox = match state.hybrid_v2_broadcast_outbox.as_ref() {
        Some(o) => o.clone(),
        None => {
            let reason = state
                .hybrid_v2_broadcast_unavailable_reason
                .clone()
                .or_else(|| state.hybrid_v2_execution_unavailable_reason.clone())
                .unwrap_or_else(|| "BROADCAST_OUTBOX_NOT_WIRED".to_string());
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "EXECUTION_ORCHESTRATOR_NOT_WIRED",
                    "detail": "no broadcast outbox wired to this AppState build",
                    "availability": { "state": "NotConfigured", "reason": reason },
                })),
            )
                .into_response();
        }
    };
    let cfg = match state.hybrid_v2_broadcast_config.clone() {
        Some(c) => c,
        None => {
            return err_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "BROADCAST_CONFIG_MISSING",
                "no hybrid_v2_broadcast_config attached",
            );
        }
    };

    // Read + validate the execution request row.
    let exec_row = match store.get_execution_request(&canonical_execution_id).await {
        Ok(Some(r)) if r.deployment_id == deployment_id => r,
        Ok(Some(_)) => {
            return err_response(
                StatusCode::NOT_FOUND,
                "EXECUTION_NOT_IN_DEPLOYMENT",
                "canonical execution id belongs to a different deployment",
            );
        }
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
    if !matches!(
        exec_row.phase,
        ExecutionPhase::SignatureVerified | ExecutionPhase::ReadyForBroadcast
    ) {
        return err_response(
            StatusCode::CONFLICT,
            "EXECUTION_WRONG_PHASE",
            &format!(
                "broadcast requires phase SignatureVerified or ReadyForBroadcast; got {}",
                exec_row.phase.as_str()
            ),
        );
    }
    // Idempotent insert of the broadcast row so we always have a live
    // row to observe. Same behavior as the pre-closure handler.
    if let Err(e) = store
        .insert_broadcast_state(&canonical_execution_id, wall_ms())
        .await
    {
        return err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORE_ERROR",
            &e.to_string(),
        );
    }

    // Read the current broadcast phase to branch.
    let bcast_row = match store.get_broadcast_state(&canonical_execution_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "BROADCAST_ROW_MISSING",
                "insert_broadcast_state succeeded but row not observable",
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

    match bcast_row.phase {
        // ------------------------------------------------------------
        //  FRESH SUBMIT PATH — Part C+D closure
        // ------------------------------------------------------------
        BroadcastPhase::BroadcastDisabled | BroadcastPhase::ReadyForBroadcast => {
            // Hydrate the deterministic plan + signed tx from the row.
            let manifest = &entry.manifest;
            let plan = match reconstruct_plan(&exec_row, manifest) {
                Ok(p) => p,
                Err(e) => return broadcast_reconstruction_error_response(e),
            };
            let signed = match reconstruct_signed(&exec_row) {
                Ok(s) => s,
                Err(e) => return broadcast_reconstruction_error_response(e),
            };
            // Nonce + gas triple must be present on the row (populated
            // by the orchestrator's Signing step). Missing values are
            // treated as row-tampering and refused.
            let nonce = match exec_row.reserved_nonce {
                Some(v) if v >= 0 => v as u64,
                _ => {
                    return err_response(
                        StatusCode::CONFLICT,
                        "RECONSTRUCTION_INCOMPLETE_ROW",
                        "reserved_nonce missing or negative on execution row",
                    );
                }
            };
            let gas_limit = match exec_row.gas_limit {
                Some(v) if v >= 0 => v as u64,
                _ => {
                    return err_response(
                        StatusCode::CONFLICT,
                        "RECONSTRUCTION_INCOMPLETE_ROW",
                        "gas_limit missing or negative on execution row",
                    );
                }
            };
            let max_fee = match exec_row
                .max_fee_per_gas_wei
                .as_deref()
                .and_then(|s| U256::from_str_radix(s, 10).ok())
            {
                Some(v) => v,
                None => {
                    return err_response(
                        StatusCode::CONFLICT,
                        "RECONSTRUCTION_INCOMPLETE_ROW",
                        "max_fee_per_gas_wei missing / malformed on execution row",
                    );
                }
            };
            let max_prio = match exec_row
                .max_priority_fee_per_gas_wei
                .as_deref()
                .and_then(|s| U256::from_str_radix(s, 10).ok())
            {
                Some(v) => v,
                None => {
                    return err_response(
                        StatusCode::CONFLICT,
                        "RECONSTRUCTION_INCOMPLETE_ROW",
                        "max_priority_fee_per_gas_wei missing / malformed on execution row",
                    );
                }
            };
            // Firewall inputs. TargetPolicy is built from the manifest
            // so a manifest-hash drift here (unlikely — manifest is
            // pinned at deployment upsert time) fails-closed via the
            // policy constructor.
            let target_policy = match TargetPolicy::from_manifest(manifest) {
                Ok(p) => p,
                Err(e) => {
                    return err_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "TARGET_POLICY_BUILD_FAILED",
                        &e.to_string(),
                    );
                }
            };
            let fw_cfg = BroadcastFirewallConfig {
                broadcast_enabled: cfg.broadcast_enabled,
                pre_send_hash_probe: cfg.pre_send_hash_probe,
            };
            let firewall = BroadcastPolicyFirewall {
                store: store.as_ref(),
                target_policy: &target_policy,
                gas_policy: &cfg.gas_policy,
                broadcast_config: fw_cfg,
                configured_chain_id: entry.manifest.chain_id,
                deployment_id,
                simulation_max_age_ms: cfg.simulation_max_age_ms,
                allowed_broadcast_chain_ids: &cfg.allowed_broadcast_chain_ids,
                now_ms: wall_ms() as u64,
                rpc: state
                    .hybrid_v2_broadcast_rpc
                    .as_ref()
                    .map(|r| r.as_ref() as &dyn crate::hybrid_v2::execution::broadcast_rpc::ExecutionBroadcastRpcClient),
            };
            // Admin route is a diagnostic + operator surface — we treat
            // readiness as `ready`. The firewall's `revalidate_before_send`
            // still enforces the row-level invariants. Runtime readiness
            // is validated separately by the pre-broadcast orchestrator.
            let readiness = ReadinessReport {
                runtime: ReadinessState::ready(),
                rebuild: ReadinessState::ready(),
                reconciliation: ReadinessState::ready(),
            };
            let expected_signer = signed.recovered_signer;
            let outcome = match outbox
                .submit(
                    exec_row.clone(),
                    plan,
                    signed,
                    expected_signer,
                    nonce,
                    gas_limit,
                    max_fee,
                    max_prio,
                    readiness,
                    &firewall,
                )
                .await
            {
                Ok(o) => o,
                Err(e) => {
                    return err_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "BROADCAST_OUTBOX_ERROR",
                        &e.to_string(),
                    );
                }
            };
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "canonical_execution_id": outcome.canonical_execution_id,
                    "phase": outcome.phase.as_str(),
                    "tx_hash": outcome.tx_hash.map(|h| format!("0x{}", hex_hash(&h))),
                    "provider_classification": outcome.provider_classification,
                    "failure_class": outcome.failure_class,
                    "failure_detail": outcome.failure_detail,
                    "path": "fresh_submit",
                })),
            )
                .into_response()
        }
        // ------------------------------------------------------------
        //  RESUME PATH — existing behavior (recovery / observation)
        // ------------------------------------------------------------
        BroadcastPhase::Broadcasting
        | BroadcastPhase::SubmissionUnknown
        | BroadcastPhase::Dropped => {
            let policy = crate::hybrid_v2::execution::broadcast_outbox::ResumePolicy {
                submission_retry_max: cfg.submission_retry_max,
                max_pending_age_ms: cfg.max_pending_age_ms,
            };
            let outcome = match outbox.resume(&canonical_execution_id, policy).await {
                Ok(o) => o,
                Err(e) => {
                    return err_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "BROADCAST_OUTBOX_ERROR",
                        &e.to_string(),
                    );
                }
            };
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "canonical_execution_id": outcome.canonical_execution_id,
                    "phase": outcome.phase.as_str(),
                    "tx_hash": outcome.tx_hash.map(|h| format!("0x{}", hex_hash(&h))),
                    "provider_classification": outcome.provider_classification,
                    "failure_class": outcome.failure_class,
                    "failure_detail": outcome.failure_detail,
                    "path": "resume",
                })),
            )
                .into_response()
        }
        // ------------------------------------------------------------
        //  IN-FLIGHT / TERMINAL PATH — return current status, no RPC
        // ------------------------------------------------------------
        BroadcastPhase::Submitted
        | BroadcastPhase::Pending
        | BroadcastPhase::MinedSuccess
        | BroadcastPhase::MinedReverted
        | BroadcastPhase::Confirming
        | BroadcastPhase::Confirmed
        | BroadcastPhase::Reorged
        | BroadcastPhase::CancelledBeforeBroadcast
        | BroadcastPhase::ManualInterventionRequired => (
            StatusCode::OK,
            Json(serde_json::json!({
                "canonical_execution_id": canonical_execution_id,
                "phase": bcast_row.phase.as_str(),
                "tx_hash": bcast_row.tx_hash,
                "path": "current_status",
                "note": "phase is in-flight or terminal; no RPC contact",
            })),
        )
            .into_response(),
    }
}

/// Map a reconstruction failure onto a structured admin response.
/// Every variant translates to a bounded caller-visible code; no
/// signature bytes / calldata content is ever leaked.
fn broadcast_reconstruction_error_response(
    err: crate::hybrid_v2::execution::broadcast_reconstruction::ReconstructionError,
) -> Response {
    use crate::hybrid_v2::execution::broadcast_reconstruction::ReconstructionError as E;
    let (status, code) = match &err {
        E::CalldataBytesMissing => (StatusCode::CONFLICT, "CALLDATA_BYTES_MISSING"),
        E::CalldataHashMismatch { .. } => (StatusCode::CONFLICT, "CALLDATA_HASH_MISMATCH"),
        E::CalldataHashMissing => (StatusCode::CONFLICT, "CALLDATA_HASH_MISSING"),
        E::PlanHashMismatch { .. } => (StatusCode::CONFLICT, "PLAN_HASH_MISMATCH"),
        E::PlanHashMissing => (StatusCode::CONFLICT, "PLAN_HASH_MISSING"),
        E::SignatureMissing => (StatusCode::CONFLICT, "SIGNATURE_MISSING"),
        E::RecoveredSignerMissing => (StatusCode::CONFLICT, "RECOVERED_SIGNER_MISSING"),
        E::NonceMissing => (StatusCode::CONFLICT, "NONCE_MISSING"),
        E::GasFieldMissing => (StatusCode::CONFLICT, "GAS_FIELD_MISSING"),
        E::TargetContractMissing => (StatusCode::CONFLICT, "TARGET_CONTRACT_MISSING"),
        E::SelectorMissing => (StatusCode::CONFLICT, "SELECTOR_MISSING"),
        E::ChainIdMismatch { .. } => (StatusCode::CONFLICT, "CHAIN_ID_MISMATCH"),
        E::ManifestLookupFailure(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "MANIFEST_LOOKUP_FAILURE")
        }
        E::ValueNonZero(_) => (StatusCode::CONFLICT, "VALUE_NON_ZERO"),
        E::MalformedHex(_) => (StatusCode::CONFLICT, "MALFORMED_HEX"),
    };
    err_response(status, code, &err.to_string())
}

fn hex_hash(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes.iter() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// `GET /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/broadcast_status`
pub async fn admin_broadcast_status(
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
    // Verify execution row exists in this deployment so an admin cannot
    // enumerate broadcast rows across deployments via hash guessing.
    match store.get_execution_request(&canonical_execution_id).await {
        Ok(Some(r)) if r.deployment_id == deployment_id => {}
        Ok(Some(_)) => {
            return err_response(
                StatusCode::NOT_FOUND,
                "EXECUTION_NOT_IN_DEPLOYMENT",
                "canonical execution id belongs to a different deployment",
            );
        }
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
    }
    match store.get_broadcast_state(&canonical_execution_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(SanitizedBroadcastRow::from(row))).into_response(),
        Ok(None) => err_response(
            StatusCode::NOT_FOUND,
            "BROADCAST_ROW_NOT_FOUND",
            "no broadcast state row for this execution",
        ),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORE_ERROR",
            &e.to_string(),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct BroadcastPendingQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `GET /admin/hybrid_v2/deployments/:deployment_id/broadcast_pending?limit=N`
pub async fn admin_list_broadcast_pending(
    Path(deployment_id): Path<i64>,
    Query(q): Query<BroadcastPendingQuery>,
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
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let pending = [
        BroadcastPhase::Broadcasting,
        BroadcastPhase::SubmissionUnknown,
        BroadcastPhase::Submitted,
        BroadcastPhase::Pending,
        BroadcastPhase::MinedSuccess,
        BroadcastPhase::Confirming,
        BroadcastPhase::Reorged,
    ];
    match store
        .list_pending_broadcast_states(deployment_id, &pending, limit)
        .await
    {
        Ok(rows) => {
            let sanitized: Vec<SanitizedBroadcastRow> =
                rows.into_iter().map(SanitizedBroadcastRow::from).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "deployment_id": deployment_id,
                    "limit": limit,
                    "rows": sanitized,
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

/// `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/broadcast_recheck`
pub async fn admin_broadcast_recheck(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(_body): Json<EmptyAdminBody>,
) -> Response {
    if let Err(resp) = ensure_admin(&state, &headers) {
        return resp;
    }
    let _store = match require_store(&state) {
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
    if let Err(resp) = broadcast_config_or_disabled(&state) {
        return resp;
    }
    // Package D wire — if the confirmation worker is attached, run one
    // observation cycle via `tick_single`. The worker only touches the
    // broadcast RPC (receipt / tx_by_hash / block header lookups) —
    // never the write method. When the worker is not attached
    // return the 503 EXECUTION_ORCHESTRATOR_NOT_WIRED sentinel.
    let worker = match state.hybrid_v2_broadcast_worker.as_ref() {
        Some(w) => w.clone(),
        None => {
            let reason = state
                .hybrid_v2_broadcast_unavailable_reason
                .clone()
                .unwrap_or_else(|| "EXECUTION_ORCHESTRATOR_NOT_WIRED".to_string());
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "EXECUTION_ORCHESTRATOR_NOT_WIRED",
                    "detail": "confirmation worker not attached to AppState",
                    "availability": { "state": "NotConfigured", "reason": reason },
                })),
            )
                .into_response();
        }
    };
    match worker.tick_single(&canonical_execution_id).await {
        Ok(phase) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "canonical_execution_id": canonical_execution_id,
                "phase": phase.as_str(),
            })),
        )
            .into_response(),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "CONFIRMATION_WORKER_ERROR",
            &e.to_string(),
        ),
    }
}

/// `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/broadcast_resend_same_bytes`
pub async fn admin_broadcast_resend_same_bytes(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(_body): Json<EmptyAdminBody>,
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
    let cfg = match broadcast_config_or_disabled(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    // Guard: row phase must be SubmissionUnknown or Dropped AND the
    // retry budget must not be exhausted.
    let row = match store.get_broadcast_state(&canonical_execution_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return err_response(
                StatusCode::NOT_FOUND,
                "BROADCAST_ROW_NOT_FOUND",
                "no broadcast state row for this execution",
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
    if !matches!(
        row.phase,
        BroadcastPhase::SubmissionUnknown | BroadcastPhase::Dropped
    ) {
        return err_response(
            StatusCode::CONFLICT,
            "RESEND_WRONG_PHASE",
            &format!(
                "resend_same_bytes requires phase SubmissionUnknown or Dropped; got {}",
                row.phase.as_str()
            ),
        );
    }
    let attempts = row.submission_attempt_count.max(0) as u32;
    if attempts > cfg.submission_retry_max {
        return err_response(
            StatusCode::CONFLICT,
            "RESEND_BUDGET_EXHAUSTED",
            &format!(
                "submission_attempt_count {} exceeds submission_retry_max {}",
                attempts, cfg.submission_retry_max
            ),
        );
    }
    // Package D — honest deferral. `resend_same_bytes` on
    // `BroadcastOutbox` requires the deterministic plan bytes + signed
    // envelope + gas params to reconstruct the exact serialized tx.
    // Those primitives are NOT stored on the broadcast row (only the
    // envelope hash is persisted, by design — see broadcast_outbox.rs
    // module docs on "NEVER re-sign"). A plan+signed hydrator that
    // reconstructs from `execution_requests` columns is scheduled for
    // BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-V1; until then the
    // route returns a 503 that surfaces the wired-broadcast state.
    let broadcast_state = if state.hybrid_v2_broadcast_outbox.is_some() {
        "Wired"
    } else {
        "NotConfigured"
    };
    let reason = state
        .hybrid_v2_broadcast_unavailable_reason
        .clone()
        .unwrap_or_else(|| {
            "RESEND_SAME_BYTES_REQUIRES_PLAN_HYDRATOR: deferred to base-sepolia E2E".to_string()
        });
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "EXECUTION_ORCHESTRATOR_NOT_WIRED",
            "detail": "same-byte resend requires plan+signed reconstruction from persisted execution row — deferred to BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-V1",
            "availability": { "state": broadcast_state, "reason": reason },
            "row": {
                "phase": row.phase.as_str(),
                "submission_attempt_count": row.submission_attempt_count,
                "submission_retry_max": cfg.submission_retry_max,
            },
        })),
    )
        .into_response()
}

/// `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/broadcast_manual_intervention`
///
/// Operator-issued escalation. Body only accepts `action` + `detail`.
pub async fn admin_broadcast_manual_intervention(
    Path((deployment_id, canonical_execution_id)): Path<(i64, String)>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<ManualInterventionBody>,
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
    if body.action != "MARK_MANUAL" {
        return err_response(
            StatusCode::BAD_REQUEST,
            "UNKNOWN_MANUAL_ACTION",
            "only MARK_MANUAL is accepted",
        );
    }
    let row = match store.get_broadcast_state(&canonical_execution_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return err_response(
                StatusCode::NOT_FOUND,
                "BROADCAST_ROW_NOT_FOUND",
                "no broadcast state row for this execution",
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
    if !row
        .phase
        .can_transition_to(BroadcastPhase::ManualInterventionRequired)
    {
        return err_response(
            StatusCode::CONFLICT,
            "MANUAL_INTERVENTION_INVALID_TRANSITION",
            &format!(
                "phase {} cannot escalate to MANUAL_INTERVENTION_REQUIRED",
                row.phase.as_str()
            ),
        );
    }
    let now = wall_ms();
    let patch = BroadcastStatePatch {
        failure_class: Some("ADMIN_MANUAL_INTERVENTION".into()),
        failure_detail: Some(body.detail),
        terminal_at_ms: Some(now),
        ..Default::default()
    };
    match store
        .update_broadcast_phase(
            &canonical_execution_id,
            row.phase,
            BroadcastPhase::ManualInterventionRequired,
            now,
            patch,
        )
        .await
    {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "canonical_execution_id": canonical_execution_id,
                "phase": "MANUAL_INTERVENTION_REQUIRED",
            })),
        )
            .into_response(),
        Ok(false) => err_response(
            StatusCode::CONFLICT,
            "MANUAL_INTERVENTION_LOST_UPDATE",
            "another writer advanced the row concurrently; re-read and retry",
        ),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORE_ERROR",
            &e.to_string(),
        ),
    }
}
