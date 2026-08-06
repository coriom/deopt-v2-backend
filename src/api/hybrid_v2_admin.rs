//! `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1`
//! — mounted operator recovery controls for Hybrid V2.
//!
//! Frozen posture:
//! - Every route is behind the same admin-token gate as
//!   `admin/options/events/tick` (see `ensure_admin_access`).
//! - Base mainnet (`chain_id = 8453`) is refused at handler entry —
//!   never depend on downstream layers to enforce this.
//! - No route triggers a synchronous chain-write; every action is a
//!   persistence-only state-machine mutation (rebuild-op row insert /
//!   operation lock acquire) or an in-place reconciliation invocation.
//! - Handlers refuse when `state.hybrid_v2_projection_store` is `None`
//!   with 503 `HYBRID_V2_NOT_CONFIGURED`.
//!
//! Boundary:
//! - Input: authenticated operator triggers a rebuild or reconciliation
//!   for a specific `deployment_id`.
//! - Output: JSON body describing the enqueued operation (or a 409 on
//!   contention). No chain state observed here.

use crate::api::http::AppState;
use crate::hybrid_v2::rebuild_operations::{
    RebuildConfig, RebuildMode, RebuildOperationsService, RebuildOutcome,
};
use crate::hybrid_v2::HybridV2ProjectionStore;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const BASE_MAINNET_CHAIN_ID: u64 = 8453;

// -----------------------------------------------------------------
//                       REQUEST + RESPONSE
// -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RebuildRequestBody {
    /// `"JOURNAL_REPLAY"` (safe: verify only) or `"FRESH_CHAIN"` (Mode-2).
    #[serde(default)]
    pub mode: Option<String>,
    /// Explicit opt-in for the auto-rematerialization drift branch.
    /// Default false — drift escalates to
    /// `MANUAL_INTERVENTION_REQUIRED` without touching any live table.
    #[serde(default)]
    pub auto_rematerialize: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RebuildResponse {
    pub deployment_id: i64,
    pub rebuild_epoch: i64,
    pub outcome: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReconcileResponse {
    pub deployment_id: i64,
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LatestOperationResponse {
    pub rebuild: Option<serde_json::Value>,
    pub reconciliation: Option<serde_json::Value>,
    pub reorg_recovery: Option<serde_json::Value>,
}

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
            "operator recovery routes are refused on Base mainnet",
        ));
    }
    Ok(())
}

// -----------------------------------------------------------------
//                       HANDLERS
// -----------------------------------------------------------------

pub async fn request_rebuild(
    Path(deployment_id): Path<i64>,
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<RebuildRequestBody>,
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
    let mode = match body.mode.as_deref().unwrap_or("JOURNAL_REPLAY") {
        "JOURNAL_REPLAY" => RebuildMode::JournalReplay,
        "FRESH_CHAIN" => RebuildMode::FreshChain,
        other => {
            return err_response(
                StatusCode::BAD_REQUEST,
                "INVALID_MODE",
                &format!("mode must be JOURNAL_REPLAY or FRESH_CHAIN, got {other}"),
            );
        }
    };
    if matches!(mode, RebuildMode::FreshChain) {
        return err_response(
            StatusCode::NOT_IMPLEMENTED,
            "FRESH_CHAIN_NOT_ROUTED",
            "FRESH_CHAIN mode requires an operator-supplied ingestion 
             closure; call the service directly via the CLI instead",
        );
    }
    let mut config = RebuildConfig::default();
    if let Some(auto) = body.auto_rematerialize {
        config.auto_rematerialize = auto;
    }
    let service = RebuildOperationsService::new(deployment_id, config);
    let manifest = entry.manifest.clone();
    match service.rebuild_from_journal(&store, &manifest).await {
        Ok(RebuildOutcome::NothingToDo {
            epoch,
            events_replayed,
        }) => (
            StatusCode::OK,
            Json(RebuildResponse {
                deployment_id,
                rebuild_epoch: epoch,
                outcome: "NOTHING_TO_DO".to_string(),
                detail: Some(format!("replayed {events_replayed} events")),
            }),
        )
            .into_response(),
        Ok(RebuildOutcome::Rebuilt {
            epoch,
            events_replayed,
            executions_correlated,
        }) => (
            StatusCode::OK,
            Json(RebuildResponse {
                deployment_id,
                rebuild_epoch: epoch,
                outcome: "REBUILT".to_string(),
                detail: Some(format!(
                    "replayed {events_replayed} events, {executions_correlated} executions"
                )),
            }),
        )
            .into_response(),
        Ok(RebuildOutcome::ManualInterventionRequired { epoch, reason }) => (
            StatusCode::CONFLICT,
            Json(RebuildResponse {
                deployment_id,
                rebuild_epoch: epoch,
                outcome: "MANUAL_INTERVENTION_REQUIRED".to_string(),
                detail: Some(reason),
            }),
        )
            .into_response(),
        Ok(RebuildOutcome::RetryableFailure { epoch, reason }) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(RebuildResponse {
                deployment_id,
                rebuild_epoch: epoch,
                outcome: "RETRYABLE_FAILURE".to_string(),
                detail: Some(reason),
            }),
        )
            .into_response(),
        Err(crate::hybrid_v2::rebuild_operations::RebuildError::LockContention {
            deployment_id: _,
            kind: _,
        }) => err_response(
            StatusCode::CONFLICT,
            "OPERATION_LOCK_CONTENTION",
            "another operation (rebuild / reorg / reconciliation) holds 
             the deployment lock",
        ),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "REBUILD_ERROR",
            &e.to_string(),
        ),
    }
}

pub async fn request_reconciliation(
    Path(deployment_id): Path<i64>,
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
    // BACKEND-HYBRID-V2-CHAIN-VIEW-PROVIDER-AND-RECONCILIATION-TASK-V1
    // Every dependency (provider, runtime, worker config, manifest)
    // must be attached to AppState. Missing pieces surface as
    // deterministic 503 responses so an operator misconfiguration is
    // obvious rather than silently degrading behaviour.
    let Some(provider) = state.hybrid_v2_chain_view_provider.clone() else {
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "RECONCILIATION_PROVIDER_UNAVAILABLE",
            "no production ChainViewProvider is wired for this deployment",
        );
    };
    let Some(runtime) = state.hybrid_v2_runtime.clone() else {
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "RECONCILIATION_RUNTIME_UNAVAILABLE",
            "no indexer runtime handle is wired for this deployment",
        );
    };
    let Some(worker_config) = state.hybrid_v2_reconciliation_worker_config.clone() else {
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "RECONCILIATION_CONFIG_UNAVAILABLE",
            "no reconciliation worker config is attached to AppState",
        );
    };
    // Guardrail: the wired provider must match the requested deployment.
    if worker_config.deployment_id != deployment_id {
        return err_response(
            StatusCode::CONFLICT,
            "RECONCILIATION_DEPLOYMENT_MISMATCH",
            &format!(
                "requested deployment_id={deployment_id} does not match wired provider ({})",
                worker_config.deployment_id,
            ),
        );
    }
    let reconciler = crate::hybrid_v2::chain_view::Reconciler {
        max_pairs_per_batch: worker_config.max_items_per_run as usize,
    };
    let outcome = crate::hybrid_v2::reconciliation_worker::tick_once(
        &runtime,
        provider.as_ref(),
        store.as_ref(),
        &reconciler,
        &worker_config,
    )
    .await;
    match outcome {
        Ok(crate::hybrid_v2::reconciliation_worker::TickOutcome::Ran {
            classification,
            reconciliation_id,
        }) => {
            let status = if classification.is_converged_or_transient() {
                StatusCode::OK
            } else {
                // Drift is not a client error — the request succeeded,
                // and the operator's job is to inspect the persisted
                // classification. Report 200 with the classification
                // string so integrations can decide how to react.
                StatusCode::OK
            };
            (
                status,
                Json(serde_json::json!({
                    "deployment_id": deployment_id,
                    "operation_id": reconciliation_id,
                    "classification": classification.as_str(),
                    "status": "COMPLETED",
                })),
            )
                .into_response()
        }
        Ok(crate::hybrid_v2::reconciliation_worker::TickOutcome::Skipped { reason }) => {
            err_response(
                StatusCode::CONFLICT,
                "RECONCILIATION_SKIPPED",
                &format!("reconciliation skipped: {reason}"),
            )
        }
        Err(detail) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "RECONCILIATION_FAILED",
            &detail,
        ),
    }
}

pub async fn latest_operation(
    Path(deployment_id): Path<i64>,
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
    let rebuild = store
        .read_latest_rebuild_operation(deployment_id)
        .await
        .ok()
        .flatten()
        .map(|op| {
            serde_json::json!({
                "rebuild_epoch": op.rebuild_epoch,
                "mode": op.mode.as_str(),
                "phase": op.phase.as_str(),
                "events_replayed": op.events_replayed,
                "executions_correlated": op.executions_correlated,
                "verification_result": op.verification_result,
                "last_failure_detail": op.last_failure_detail,
                "requested_at_ms": op.requested_at_ms,
                "updated_at_ms": op.updated_at_ms,
                "completed_at_ms": op.completed_at_ms,
            })
        });
    let reconciliation = store
        .read_latest_reconciliation_result(deployment_id)
        .await
        .ok()
        .flatten()
        .map(|rec| {
            serde_json::json!({
                "reconciliation_id": rec.reconciliation_id,
                "ran_at_ms": rec.ran_at_ms,
                "block_number_checked": rec.block_number_checked,
                "classification": rec.classification.as_str(),
                "provider_availability": rec.provider_availability.as_str(),
                "categories_checked": rec.categories_checked,
                "converged_categories": rec.converged_categories,
                "divergent_categories": rec.divergent_categories,
            })
        });
    let reorg_recovery = store
        .read_reorg_recovery(deployment_id)
        .await
        .ok()
        .flatten()
        .map(|r| {
            serde_json::json!({
                "recovery_epoch": r.recovery_epoch,
                "phase": r.phase.as_str(),
                "orphan_depth": r.orphan_depth,
                "updated_at_ms": r.updated_at_ms,
                "completed_at_ms": r.completed_at_ms,
                "last_failure_detail": r.last_failure_detail,
            })
        });
    (
        StatusCode::OK,
        Json(LatestOperationResponse {
            rebuild,
            reconciliation,
            reorg_recovery,
        }),
    )
        .into_response()
}
