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

// -----------------------------------------------------------------
//  BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1 (Part T) — public
//  boundary audit. Verdict: `EXECUTION_PUBLIC_BOUNDARY_VALIDATED`.
//  Every route in this module is `get(...)`; no public route accepts
//  execution mutations (no calldata / target / value / nonce / gas /
//  chain_id ingress). The static regression below reads this source
//  file at test time and fails loudly if any non-GET verb sneaks in.
// -----------------------------------------------------------------
// AUDIT-BOUNDARY-STRIP-BELOW

#[cfg(test)]
mod public_boundary_audit {
    /// Guardrail — the public read router must only mount GET
    /// handlers. Any change that lands a POST/PUT/PATCH/DELETE here is
    /// a public-boundary violation for the pre-broadcast execution
    /// pipeline. See `PART T — Public boundary` in the SIGNER-AND-
    /// EXECUTION-V1 audit surface.
    ///
    /// The scan reads the router source, strips this test module (so
    /// the assertion strings don't self-trip), and asserts no forbidden
    /// verb tokens remain.
    #[test]
    fn no_mutating_verbs_on_public_router() {
        let source = include_str!("router.rs");
        // Strip everything from the audit-marker onward — the marker is
        // the sentinel comment placed immediately before this test
        // module, so the pre-marker slice contains only production
        // route mounts.
        let marker = "// AUDIT-BOUNDARY-STRIP-BELOW";
        let scanned = match source.find(marker) {
            Some(idx) => &source[..idx],
            None => source,
        };
        // Build the forbidden-verb list from concatenated char slices
        // so this test's own source doesn't contain the literal token
        // (which would slip past the marker if a future edit moved the
        // marker).
        let forbidden_full = [
            format!("routing::{}", "post"),
            format!("routing::{}", "put"),
            format!("routing::{}", "patch"),
            format!("routing::{}", "delete"),
        ];
        for verb in &forbidden_full {
            assert!(
                !scanned.contains(verb.as_str()),
                "public read router leaked a mutating verb `{verb}` — HV2 execution boundary regression"
            );
        }
        // Shorthand form: `, post(` / ` post(` etc. Constructed
        // dynamically for the same reason.
        for verb_name in ["post", "put", "patch", "delete"] {
            let shorthand_a = format!(" {verb_name}(");
            let shorthand_b = format!(", {verb_name}(");
            assert!(
                !scanned.contains(shorthand_a.as_str()),
                "public read router leaked shorthand `{shorthand_a}` — HV2 execution boundary regression"
            );
            assert!(
                !scanned.contains(shorthand_b.as_str()),
                "public read router leaked shorthand `{shorthand_b}` — HV2 execution boundary regression"
            );
        }
    }
}
