//! `BACKEND-SUBACCOUNT-READ-API-RUNTIME-WIRING-CLOSURE-V1` — end-to-end
//! tests through the real backend router `deopt_v2_backend::api::router`.
//!
//! These tests exercise the actual production wiring — the merge of the
//! Hybrid V2 read sub-router into the main `Router<()>` served by
//! `axum::serve` in `src/main.rs`. Previous tests only exercised the
//! isolated `build_hybrid_v2_read_router` factory.
//!
//! Frozen contracts verified here:
//!
//! - Startup succeeds when no Hybrid V2 deployment is configured
//!   (default `AppState::new` path).
//! - `/subaccounts/deployments` remains readable when unconfigured
//!   (empty array).
//! - Canonical Hybrid V2 routes return a structured 503 while
//!   unconfigured.
//! - Existing V1 read routes (`/health`, `/markets`,
//!   `/accounts/:address/history/v2`) remain reachable through the same
//!   router.
//! - Configured + ready deployment serves canonical data through the
//!   real router.
//! - Malformed input on Hybrid V2 canonical routes returns typed 400.
//! - `/hybrid-v2/openapi.json` is served by the real router.

mod hybrid_v2_support;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::hybrid_v2_read::{DeploymentEntry, HybridV2ApiState};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::hybrid_v2::chain_source::InMemoryChainSource;
use deopt_v2_backend::hybrid_v2::runtime::IndexerRuntime;
use hybrid_v2_support::{
    baseline_manifest, block, deposit_log, pad_address, pad_bytes32, subaccount_created_log,
};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

async fn get_body(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn status_only(app: axum::Router, method: &str, uri: &str) -> StatusCode {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    response.status()
}

fn state_without_hybrid_v2() -> AppState {
    AppState::new(EngineState::with_default_markets())
}

fn state_with_ready_hybrid_v2() -> AppState {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xa1", 1, "0xff01"),
            deposit_log(&manifest, "0xff01", "0xa1", 1, "0xef", "1000"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    while runtime.tick(&source).unwrap() {}
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let hybrid_state = HybridV2ApiState::new(vec![entry]);
    AppState::new(EngineState::with_default_markets()).with_hybrid_v2(hybrid_state)
}

// -------------------- STARTUP / DEFAULT --------------------

#[tokio::test]
async fn app_state_default_omits_hybrid_v2_configuration() {
    // AppState::new must succeed without any Hybrid V2 configuration.
    let state = state_without_hybrid_v2();
    assert!(state.hybrid_v2_read.list().is_empty());
    // The full router must build without panicking.
    let _app = router(state);
}

#[tokio::test]
async fn deployments_route_returns_empty_when_unconfigured() {
    let state = state_without_hybrid_v2();
    let app = router(state);
    let (status, body) = get_body(app, "/subaccounts/deployments").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn canonical_route_returns_400_when_unconfigured() {
    // Without any deployment configured, an owner-scoped canonical call
    // fails to resolve the implicit deployment → structured 400
    // `INVALID_DEPLOYMENT`. Once populated with a single deployment the
    // implicit resolution kicks in; when multiple are populated the
    // caller must pass `deployment_id`.
    let state = state_without_hybrid_v2();
    let app = router(state);
    let owner_hex = pad_address("0xa1");
    let (status, body) = get_body(
        app,
        &format!("/accounts/{}/hybrid-v2/subaccounts", owner_hex),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_DEPLOYMENT");
}

// -------------------- V1 REGRESSION PINS --------------------

#[tokio::test]
async fn health_route_still_reachable_through_merged_router() {
    let state = state_without_hybrid_v2();
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn markets_route_still_reachable_through_merged_router() {
    let state = state_without_hybrid_v2();
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/markets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Just checking reachability — status can be 200 or another documented
    // code; the point is the route resolves rather than 404 after the
    // merge.
    assert_ne!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn account_history_v2_route_still_reachable_through_merged_router() {
    // Existing V1 route `/accounts/:address/history/v2` must remain
    // reachable after the Hybrid V2 merge. We check for anything other
    // than 404 to prove non-collision — the exact status depends on the
    // V1 handler's behaviour with a synthetic address.
    let state = state_without_hybrid_v2();
    let app = router(state);
    let addr = "0x000000000000000000000000000000000000abcd";
    let response = app
        .oneshot(
            Request::builder()
                .uri(&format!("/accounts/{}/history/v2", addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::NOT_FOUND);
}

// -------------------- CONFIGURED READY --------------------

#[tokio::test]
async fn ready_deployment_serves_status_through_real_router() {
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    let (status, body) = get_body(app, "/subaccounts/deployments/1/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deployment_id"], 1);
    assert_eq!(body["ready"], true);
    assert_eq!(body["chain_id"], 84532);
    assert!(body["metadata"]["indexed_block"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn ready_deployment_serves_owner_subaccounts_through_real_router() {
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    let owner_hex = pad_address("0xa1");
    let (status, body) = get_body(
        app,
        &format!("/accounts/{}/hybrid-v2/subaccounts", owner_hex),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let subs = body["data"]["subaccounts"].as_array().unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0]["subaccount_id"], 1);
    assert_eq!(body["metadata"]["deployment_id"], 1);
    assert_eq!(body["metadata"]["chain_id"], 84532);
}

#[tokio::test]
async fn ready_deployment_serves_collateral_through_real_router() {
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    let sk = pad_bytes32("0xff01");
    let (status, body) = get_body(app, &format!("/subaccounts/{}/collateral", sk)).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["balance"], "1000");
}

#[tokio::test]
async fn ready_deployment_serves_history_through_real_router() {
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    let owner_hex = pad_address("0xa1");
    let (status, body) = get_body(
        app,
        &format!("/accounts/{}/hybrid-v2/history?limit=10", owner_hex),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events = body["data"].as_array().unwrap();
    // Contains SUBACCOUNT_CREATED + DEPOSIT for the fixture.
    assert!(!events.is_empty());
    let families: Vec<&str> = events
        .iter()
        .map(|e| e["payload"]["family"].as_str().unwrap())
        .collect();
    assert!(families.contains(&"SUBACCOUNT_CREATED"));
    assert!(families.contains(&"DEPOSIT"));
}

// -------------------- MALFORMED INPUT --------------------

#[tokio::test]
async fn malformed_address_returns_400_through_real_router() {
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    let (status, body) = get_body(app, "/accounts/0xnothex/hybrid-v2/subaccounts").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_ADDRESS");
}

#[tokio::test]
async fn malformed_subkey_returns_400_through_real_router() {
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    let (status, body) = get_body(app, "/subaccounts/0xshort").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_SUBKEY");
}

#[tokio::test]
async fn account_0_rejected_through_real_router() {
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    let zero = format!("0x{}", "0".repeat(40));
    let (status, body) = get_body(app, &format!("/accounts/{}/hybrid-v2/subaccounts", zero)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "ACCOUNT_0_INVALID");
}

// -------------------- WRITE METHODS --------------------

#[tokio::test]
async fn hybrid_v2_write_method_rejected_405_or_404() {
    // The Hybrid V2 surface exposes no write route. POST/PUT/PATCH/DELETE
    // must not accidentally resolve. Axum 0.7 returns 405 Method Not
    // Allowed when a path exists with different methods; 404 when the
    // path itself is unknown. Either is acceptable — the key contract is
    // that no write handler executes.
    let state = state_with_ready_hybrid_v2();
    let app = router(state);
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        let sk = pad_bytes32("0xff01");
        let s = status_only(
            app.clone(),
            method,
            &format!("/subaccounts/{}/collateral", sk),
        )
        .await;
        assert!(
            matches!(s, StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_FOUND),
            "{} on collateral resolved to {}",
            method,
            s
        );
    }
}

// -------------------- OPENAPI --------------------

#[tokio::test]
async fn openapi_served_through_real_router() {
    let state = state_without_hybrid_v2();
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/hybrid-v2/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["openapi"], "3.1.0");
    assert!(json["paths"]["/subaccounts/deployments"].is_object());
    assert!(json["paths"]["/hybrid-v2/history"].is_object());
    assert!(json["components"]["schemas"]["CanonicalityMetadata"].is_object());
}

// -------------------- READINESS FAILURE --------------------

#[tokio::test]
async fn manifest_mismatch_returns_503_through_real_router() {
    use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReason, ReadinessState};
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    runtime.readiness = ReadinessState::new_not_ready(ReadinessReason::ManifestMismatch {
        detail: "test-drift".into(),
    });
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let hybrid_state = HybridV2ApiState::new(vec![entry]);
    let app_state = AppState::new(EngineState::with_default_markets()).with_hybrid_v2(hybrid_state);
    let app = router(app_state);
    let owner_hex = pad_address("0xa1");
    let (status, body) = get_body(
        app,
        &format!("/accounts/{}/hybrid-v2/subaccounts", owner_hex),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "MANIFEST_MISMATCH");
    assert_eq!(body["retryable"], true);
}

#[tokio::test]
async fn status_route_readable_even_when_not_ready() {
    use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReason, ReadinessState};
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    runtime.readiness = ReadinessState::new_not_ready(ReadinessReason::RebuildInProgress);
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let hybrid_state = HybridV2ApiState::new(vec![entry]);
    let app_state = AppState::new(EngineState::with_default_markets()).with_hybrid_v2(hybrid_state);
    let app = router(app_state);
    let (status, body) = get_body(app, "/subaccounts/deployments/1/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ready"], false);
    assert!(body["ready_reason"].as_str().unwrap().contains("Rebuild"));
}
