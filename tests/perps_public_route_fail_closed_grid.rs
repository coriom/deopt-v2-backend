//! PERPS-FRONTEND-TICKET-ENABLEMENT-V1 — table-driven fail-closed grid.
//!
//! One central regression pin covering **every public Perps mutation
//! route** under the default backend configuration
//! (`PERPS_PUBLIC_TRADING_ENABLED=false` implied by
//! `AppState::new`). Every route must return `503 SERVICE_UNAVAILABLE`
//! with an error body that mentions "perp" so a downstream
//! frontend/CLI can classify it as `PerpsNotLive`.
//!
//! Coverage:
//!   * 7 legacy routes (`/orders`, `/orders/:id`, `/rfqs`,
//!     `/rfqs/:rfq_id/quotes`, `/rfqs/:rfq_id/accept/:quote_id`,
//!     `/rfqs/:rfq_id/cancel`,
//!     `/execution-intents/:intent_id/signatures`) which are
//!     permanently fail-closed and unaffected by the opt-in flag.
//!   * 2 new flag-gated routes (`POST /perps/orders`,
//!     `DELETE /perps/orders/:order_id`) which return `PerpsNotLive`
//!     when the flag is off (default).
//!
//! Total: **9 mutation routes**. This test file is intentionally the
//! last-line-of-defence: if a milestone accidentally unlocks any
//! Perps mutation route without touching this file, this test fails
//! immediately.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use tower::ServiceExt;

fn app_router() -> axum::Router {
    router(AppState::new(EngineState::with_default_markets()))
}

async fn body_lower(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).to_ascii_lowercase()
}

struct MutationRoute {
    method: &'static str,
    path: &'static str,
    body_json: Option<&'static str>,
    label: &'static str,
}

const PERPS_MUTATION_ROUTES: &[MutationRoute] = &[
    // Legacy Perps mutation routes — permanently fail-closed. The
    // request bodies below are Options-shape because that is the DTO
    // the current handlers expect, but the handlers reject at entry
    // before parsing.
    MutationRoute {
        method: "POST",
        path: "/orders",
        body_json: Some(
            r#"{
                "market_id": 1,
                "account": "0x0000000000000000000000000000000000000abc",
                "side": "buy",
                "price_1e8": "1000",
                "size_1e8": "10",
                "time_in_force": "gtc",
                "reduce_only": false,
                "post_only": false,
                "client_order_id": null,
                "nonce": 0,
                "deadline_ms": 0,
                "signature": "0x"
            }"#,
        ),
        label: "POST /orders (legacy)",
    },
    MutationRoute {
        method: "DELETE",
        path: "/orders/some-order-id",
        body_json: None,
        label: "DELETE /orders/:id (legacy)",
    },
    MutationRoute {
        method: "POST",
        path: "/rfqs",
        body_json: Some(r#"{}"#),
        label: "POST /rfqs (legacy)",
    },
    MutationRoute {
        method: "POST",
        path: "/rfqs/00000000-0000-0000-0000-000000000000/quotes",
        body_json: Some(r#"{}"#),
        label: "POST /rfqs/:rfq_id/quotes (legacy)",
    },
    MutationRoute {
        method: "POST",
        path:
            "/rfqs/00000000-0000-0000-0000-000000000000/accept/00000000-0000-0000-0000-000000000000",
        body_json: Some(r#"{}"#),
        label: "POST /rfqs/:rfq_id/accept/:quote_id (legacy)",
    },
    MutationRoute {
        method: "POST",
        path: "/rfqs/00000000-0000-0000-0000-000000000000/cancel",
        body_json: Some(r#"{}"#),
        label: "POST /rfqs/:rfq_id/cancel (legacy)",
    },
    MutationRoute {
        method: "POST",
        path: "/execution-intents/00000000-0000-0000-0000-000000000000/signatures",
        body_json: Some(r#"{}"#),
        label: "POST /execution-intents/:intent_id/signatures (legacy)",
    },
    // New flag-gated Perps mutation routes — fail-closed under the
    // default `PERPS_PUBLIC_TRADING_ENABLED=false`.
    MutationRoute {
        method: "POST",
        path: "/perps/orders",
        body_json: Some(
            r#"{
                "market_id": "ETH-PERP",
                "account": "0x0000000000000000000000000000000000000abc",
                "side": "buy",
                "price_1e8": "300000000000",
                "size_1e8": "100000000",
                "time_in_force": "gtc",
                "post_only": false,
                "reduce_only": false,
                "isolated_margin_1e8": "30000000000",
                "client_order_id": null
            }"#,
        ),
        label: "POST /perps/orders (opt-in flag: off)",
    },
    MutationRoute {
        method: "DELETE",
        path: "/perps/orders/00000000-0000-0000-0000-000000000000?account=0xabc",
        body_json: None,
        label: "DELETE /perps/orders/:id (opt-in flag: off)",
    },
];

#[tokio::test]
async fn every_perps_mutation_route_fails_closed_under_default_config() {
    for route in PERPS_MUTATION_ROUTES {
        let mut req = Request::builder().method(route.method).uri(route.path);
        if route.body_json.is_some() {
            req = req.header("content-type", "application/json");
        }
        let body = match route.body_json {
            Some(json) => Body::from(json),
            None => Body::empty(),
        };
        let response = app_router()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap_or_else(|e| panic!("route {} request failed: {e}", route.label));
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "route {} must return 503; got {:?}",
            route.label,
            response.status()
        );
        let body = body_lower(response).await;
        assert!(
            body.contains("perp") || body.contains("not live"),
            "route {} body must mention perp/not live; got `{body}`",
            route.label
        );
    }
}

// =====================================================================
// Readiness JSON must report `perps_public_routes` as fail-closed by
// default. This pin ensures the ops-observable state stays honest.
// =====================================================================

#[tokio::test]
async fn readiness_json_reports_perps_public_routes_fail_closed_by_default() {
    let response = app_router()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8_lossy(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .to_string();
    // Field name + status must be present as JSON substrings. The
    // check body is small enough that a substring check is stable
    // against reorderings.
    assert!(
        body.contains("perps_public_routes"),
        "readiness JSON must include `perps_public_routes` field: {body}"
    );
    assert!(
        body.contains("fail_closed"),
        "readiness JSON must report `fail_closed` for the default flag: {body}"
    );
}
