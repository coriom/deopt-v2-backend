//! Axum router factory for the Hybrid V2 public read API.
//!
//! This router is self-contained: it takes a `HybridV2ApiState` and returns
//! an `axum::Router` that can be `nested` under any prefix in the top-level
//! backend router (default: `/hybrid-v2`).
//!
//! Every route in this module is a READ. No mutations, no signing, no
//! transaction submission.

use crate::api::hybrid_v2_read::handlers::*;
use crate::api::hybrid_v2_read::openapi::openapi_spec_json;
use crate::api::hybrid_v2_read::state::HybridV2ApiState;
use axum::routing::get;
use axum::Router;

pub fn build_hybrid_v2_read_router(state: HybridV2ApiState) -> Router {
    Router::new()
        // Deployment + status
        .route("/subaccounts/deployments", get(get_deployments))
        .route(
            "/subaccounts/deployments/:deployment_id/status",
            get(get_deployment_status),
        )
        // Owner discovery
        .route(
            "/accounts/:owner/hybrid-v2/subaccounts",
            get(get_owner_subaccounts),
        )
        .route("/accounts/:owner/hybrid-v2/history", get(get_owner_history))
        // Subaccount
        .route("/subaccounts/:subkey", get(get_subaccount_summary))
        .route(
            "/subaccounts/:subkey/collateral",
            get(get_subaccount_collateral),
        )
        .route(
            "/subaccounts/:subkey/reservations",
            get(get_subaccount_reservations),
        )
        .route(
            "/subaccounts/:subkey/positions",
            get(get_subaccount_positions),
        )
        .route("/subaccounts/:subkey/orders", get(get_subaccount_orders))
        .route(
            "/subaccounts/:subkey/executions",
            get(get_subaccount_executions),
        )
        .route("/subaccounts/:subkey/fees", get(get_subaccount_fees))
        .route(
            "/subaccounts/:subkey/recovery",
            get(get_subaccount_recovery),
        )
        .route("/subaccounts/:subkey/history", get(get_subaccount_history))
        // Order
        .route("/hybrid-v2/orders/:order_hash", get(get_order_lifecycle))
        // History
        .route("/hybrid-v2/history", get(get_history))
        // OpenAPI 3.1 spec
        .route("/hybrid-v2/openapi.json", get(get_openapi))
        .with_state(state)
}

async fn get_openapi() -> axum::response::Response {
    use axum::http::header::CONTENT_TYPE;
    use axum::http::StatusCode;
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json")],
        openapi_spec_json(),
    )
        .into_response()
}

use axum::response::IntoResponse;
