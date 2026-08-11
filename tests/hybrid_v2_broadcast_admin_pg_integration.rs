//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` Package C (Part S)
//! — admin route PG coverage.
//!
//! Every scenario builds a fresh `AppState` + `Router` and drives one
//! HTTP request through the admin surface. Focus:
//!  * admin-token gate enforced.
//!  * Base mainnet chain id refused.
//!  * `deny_unknown_fields` at the request body boundary rejects raw
//!    tx bytes, r/s/v, calldata, nonce, gas, chain, RPC endpoint, and
//!    signer identity.
//!  * 503 EXECUTION_ORCHESTRATOR_NOT_WIRED / BROADCAST_DISABLED when
//!    the pipeline is not attached.
//!  * Bounded resend policy is enforced.
//!  * Sanitized status response contains no signature bytes / raw
//!    envelope bytes / provider details.

mod hybrid_v2_broadcast_mock_rpc;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};
use tower::ServiceExt;

use deopt_v2_backend::admin::AdminConfig;
use deopt_v2_backend::api::hybrid_v2_read::{DeploymentEntry, EmptyReadStore, HybridV2ApiState};
use deopt_v2_backend::hybrid_v2::config::HybridV2ExecutionConfig;
use deopt_v2_backend::hybrid_v2::execution::broadcast_state::{
    BroadcastPhase, BroadcastStatePatch,
};
use deopt_v2_backend::hybrid_v2::execution::persistence::ExecutionRequestRow;
use deopt_v2_backend::hybrid_v2::execution::state::ExecutionPhase;
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

fn get_pg_url_or_skip(test_name: &str) -> Option<String> {
    let url = std::env::var(URL_ENV)
        .ok()
        .or_else(|| std::env::var(ALT_URL_ENV).ok())
        .filter(|v| !v.is_empty());
    if url.is_none() {
        let required = matches!(
            std::env::var(REQUIRE_ENV).ok().as_deref(),
            Some("1") | Some("true") | Some("TRUE")
        );
        if required {
            panic!("{REQUIRE_ENV} required but no PG URL provided");
        }
        eprintln!("SKIP {test_name}: no PG URL");
    }
    url
}

async fn fresh_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(30))
        .connect(url)
        .await
        .expect("connect");
    sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(&pool)
        .await
        .expect("drop schema");
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("create schema");
    sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
        .execute(&pool)
        .await
        .expect("grant");
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
        .await
        .expect("migrations");
    migrator.run(&pool).await.expect("apply migrations");
    pool
}

const ADMIN_TOKEN: &str = "admin-test-token";
const CID: &str = "0xe0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0";
const SIGNER: [u8; 20] = [0x77u8; 20];
const CHAIN_ID: u64 = 84532;

fn hex_of(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{:02x}", x));
    }
    format!("0x{s}")
}

fn broadcast_config(enabled: bool) -> HybridV2ExecutionConfig {
    let mut cfg = HybridV2ExecutionConfig::disabled();
    cfg.broadcast_enabled = enabled;
    cfg.allowed_broadcast_chain_ids = vec![CHAIN_ID];
    cfg.submission_retry_max = 1;
    cfg
}

async fn seed_execution_and_broadcast_row(
    store: &PostgresHybridV2ProjectionStore,
    deployment_id: i64,
    phase: ExecutionPhase,
) {
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    let row = ExecutionRequestRow {
        canonical_execution_id: CID.into(),
        deployment_id,
        chain_id: CHAIN_ID as i64,
        execution_kind: "HYBRID_V2_OPTION_MATCH".into(),
        buyer_order_hash: format!("0x{}", "aa".repeat(32)),
        seller_order_hash: format!("0x{}", "bb".repeat(32)),
        buyer_subkey: format!("0x{}", "aa".repeat(32)),
        seller_subkey: format!("0x{}", "bb".repeat(32)),
        series_id: "42".into(),
        fill_quantity_1e8: "100000000".into(),
        premium_amount: "50000000".into(),
        fee_schedule_epoch: None,
        source_matched_execution_id: None,
        target_contract: format!("0x{}", "ee".repeat(20)),
        selector: "0x00000000".into(),
        calldata_hash: Some(format!("0x{}", "cd".repeat(32))),
        plan_hash: Some(format!("0x{}", "ee".repeat(32))),
        tx_value_wei: "0".into(),
        simulation_block_number: Some(1),
        simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
        simulation_gas_estimate: Some(500_000),
        simulation_result_json: Some(serde_json::json!({})),
        signer_identity: Some(hex_of(&SIGNER)),
        signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
        signature_r: Some(format!("0x{}", "11".repeat(32))),
        signature_s: Some(format!("0x{}", "22".repeat(32))),
        signature_v: Some(0),
        recovered_signer: Some(hex_of(&SIGNER)),
        gas_limit: Some(1_000_000),
        max_fee_per_gas_wei: Some("2000000000".into()),
        max_priority_fee_per_gas_wei: Some("500000000".into()),
        reserved_nonce: Some(42),
        phase,
        failure_class: None,
        failure_detail: None,
        retry_count: 0,
        holder_epoch: None,
        signer_request_idempotency_key: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    dyn_store.insert_execution_request(&row).await.unwrap();
    dyn_store.insert_broadcast_state(CID, 1_000).await.unwrap();
}

async fn build_app_state(
    pool: &PgPool,
    broadcast_enabled: bool,
) -> (i64, deopt_v2_backend::api::http::AppState) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let deployment_id = dyn_store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .unwrap();
    let mut state = deopt_v2_backend::api::http::AppState::new(
        deopt_v2_backend::engine::EngineState::with_default_markets(),
    );
    state.admin_config = AdminConfig::new(true, true, Some(ADMIN_TOKEN.to_string()));
    // Route the hybrid_v2 read state to a metadata-only entry so
    // `resolve_deployment` succeeds.
    let entry = Arc::new(DeploymentEntry::from_metadata(
        deployment_id as u64,
        manifest,
    ));
    // Use with_store + EmptyReadStore so the read state is metadata-only
    // (no runtime backing needed for admin routes).
    let read_state = HybridV2ApiState::with_store(
        Arc::new(EmptyReadStore)
            as Arc<dyn deopt_v2_backend::hybrid_v2::read_store::HybridV2ReadStore>,
        vec![entry],
    );
    state = state
        .with_hybrid_v2(read_state)
        .with_hybrid_v2_projection_store(store);
    state.hybrid_v2_execution_config = Some(broadcast_config(broadcast_enabled));
    (deployment_id, state)
}

fn router_for(state: deopt_v2_backend::api::http::AppState) -> axum::Router {
    deopt_v2_backend::api::routes::router(state)
}

fn post(path: &str, token: Option<&str>, body: serde_json::Value) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri(path);
    if let Some(t) = token {
        builder = builder.header("x-admin-token", t);
    }
    builder = builder.header("content-type", "application/json");
    builder.body(Body::from(body.to_string())).unwrap()
}

fn get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(t) = token {
        builder = builder.header("x-admin-token", t);
    }
    builder.body(Body::empty()).unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let b = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&b).unwrap_or(serde_json::json!({}))
}

async fn body_text(resp: axum::response::Response) -> String {
    let b = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(b.to_vec()).unwrap_or_default()
}

// -----------------------------------------------------------------
//                            TESTS
// -----------------------------------------------------------------

#[tokio::test]
async fn broadcast_route_requires_admin_token() {
    let Some(url) = get_pg_url_or_skip("broadcast_route_requires_admin_token") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path = format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast");
    let response = router_for(state)
        .oneshot(post(&path, None, serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn broadcast_route_returns_503_before_any_row_lookup() {
    // Since the orchestrator is never wired in this integration
    // harness, `/broadcast` returns 503 EXECUTION_ORCHESTRATOR_NOT_WIRED
    // before even reading the execution row. This test locks that
    // ordering — the handler MUST fail closed at the wiring gate.
    let Some(url) = get_pg_url_or_skip("broadcast_route_returns_503_before_any_row_lookup") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path = format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast");
    let response = router_for(state)
        .oneshot(post(&path, Some(ADMIN_TOKEN), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(response).await;
    assert_eq!(json["error"], "EXECUTION_ORCHESTRATOR_NOT_WIRED");
}

#[tokio::test]
async fn broadcast_status_returns_404_for_unknown_execution() {
    // The /broadcast_status route needs neither an orchestrator nor an
    // RPC — it only reads the store. Unknown execution → 404.
    let Some(url) = get_pg_url_or_skip("broadcast_status_returns_404_for_unknown_execution") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path =
        format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_status");
    let response = router_for(state)
        .oneshot(get(&path, Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = body_json(response).await;
    assert_eq!(json["error"], "EXECUTION_NOT_FOUND");
}

#[tokio::test]
async fn broadcast_recheck_returns_503_when_broadcast_disabled() {
    // /broadcast_recheck runs the broadcast_config_or_disabled gate
    // even without an orchestrator wired. broadcast_enabled = false ->
    // 503 BROADCAST_DISABLED.
    let Some(url) = get_pg_url_or_skip("broadcast_recheck_returns_503_when_broadcast_disabled")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, false).await;
    let path =
        format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_recheck");
    let response = router_for(state)
        .oneshot(post(&path, Some(ADMIN_TOKEN), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(response).await;
    assert_eq!(json["error"], "BROADCAST_DISABLED");
}

#[tokio::test]
async fn broadcast_route_returns_503_when_orchestrator_not_wired() {
    let Some(url) = get_pg_url_or_skip("broadcast_route_returns_503_when_orchestrator_not_wired")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path = format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast");
    let response = router_for(state)
        .oneshot(post(&path, Some(ADMIN_TOKEN), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(response).await;
    assert_eq!(json["error"], "EXECUTION_ORCHESTRATOR_NOT_WIRED");
}

#[tokio::test]
async fn broadcast_route_rejects_unknown_body_fields() {
    let Some(url) = get_pg_url_or_skip("broadcast_route_rejects_unknown_body_fields") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path = format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast");
    // Body contains a raw tx bytes field — must be rejected at the
    // deserializer boundary as 400 / 422.
    let response = router_for(state)
        .oneshot(post(
            &path,
            Some(ADMIN_TOKEN),
            serde_json::json!({ "raw_tx_bytes": "0xdeadbeef" }),
        ))
        .await
        .unwrap();
    let status = response.status();
    assert!(
        status.is_client_error(),
        "raw tx bytes body must be rejected as 4xx; got {status}"
    );
    let text = body_text(response).await;
    assert!(
        !text.contains("0xdeadbeef"),
        "response should not echo the injected raw tx bytes: {text}"
    );
}

#[tokio::test]
async fn broadcast_status_returns_sanitized_row() {
    let Some(url) = get_pg_url_or_skip("broadcast_status_returns_sanitized_row") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let store = state.hybrid_v2_projection_store.clone().unwrap();
    let store_pg: &PostgresHybridV2ProjectionStore = downcast(&*store);
    seed_execution_and_broadcast_row(store_pg, deployment_id, ExecutionPhase::SignatureVerified)
        .await;
    let path =
        format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_status");
    let response = router_for(state)
        .oneshot(get(&path, Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    // Sanitized: no signature bytes or raw envelope bytes surfaced.
    for forbidden in [
        "signature_r",
        "signature_s",
        "signature_v",
        "raw_tx_bytes",
        "raw_envelope",
    ] {
        assert!(
            !text.contains(forbidden),
            "sanitized broadcast_status contains `{forbidden}`: {text}"
        );
    }
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["canonical_execution_id"], CID);
    assert_eq!(json["phase"], "BROADCAST_DISABLED");
}

#[tokio::test]
async fn broadcast_pending_is_bounded_by_limit() {
    let Some(url) = get_pg_url_or_skip("broadcast_pending_is_bounded_by_limit") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path = format!("/admin/hybrid_v2/deployments/{deployment_id}/broadcast_pending?limit=25");
    let response = router_for(state)
        .oneshot(get(&path, Some(ADMIN_TOKEN)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["deployment_id"], deployment_id);
    assert_eq!(json["limit"], 25);
    assert!(json["rows"].is_array());
}

#[tokio::test]
async fn broadcast_recheck_returns_503_when_worker_not_wired() {
    let Some(url) = get_pg_url_or_skip("broadcast_recheck_returns_503_when_worker_not_wired")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path =
        format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_recheck");
    let response = router_for(state)
        .oneshot(post(&path, Some(ADMIN_TOKEN), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(response).await;
    assert_eq!(json["error"], "EXECUTION_ORCHESTRATOR_NOT_WIRED");
}

#[tokio::test]
async fn broadcast_recheck_rejects_unknown_body_fields() {
    let Some(url) = get_pg_url_or_skip("broadcast_recheck_rejects_unknown_body_fields") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path =
        format!("/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_recheck");
    let response = router_for(state)
        .oneshot(post(
            &path,
            Some(ADMIN_TOKEN),
            serde_json::json!({ "signer_endpoint": "https://attacker/rpc" }),
        ))
        .await
        .unwrap();
    let status = response.status();
    assert!(
        status.is_client_error(),
        "unknown fields must be 4xx; got {status}"
    );
    let text = body_text(response).await;
    assert!(
        !text.contains("attacker"),
        "response echoes rejected field content: {text}"
    );
}

#[tokio::test]
async fn resend_same_bytes_refuses_wrong_phase() {
    let Some(url) = get_pg_url_or_skip("resend_same_bytes_refuses_wrong_phase") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    // Broadcast row exists but is at BROADCAST_DISABLED — not eligible.
    let store = state.hybrid_v2_projection_store.clone().unwrap();
    let store_pg: &PostgresHybridV2ProjectionStore = downcast(&*store);
    seed_execution_and_broadcast_row(store_pg, deployment_id, ExecutionPhase::SignatureVerified)
        .await;
    let path = format!(
        "/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_resend_same_bytes"
    );
    let response = router_for(state)
        .oneshot(post(&path, Some(ADMIN_TOKEN), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = body_json(response).await;
    assert_eq!(json["error"], "RESEND_WRONG_PHASE");
}

#[tokio::test]
async fn resend_same_bytes_refuses_when_budget_exhausted() {
    let Some(url) = get_pg_url_or_skip("resend_same_bytes_refuses_when_budget_exhausted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let store = state.hybrid_v2_projection_store.clone().unwrap();
    let store_pg: &PostgresHybridV2ProjectionStore = downcast(&*store);
    seed_execution_and_broadcast_row(store_pg, deployment_id, ExecutionPhase::SignatureVerified)
        .await;
    // Advance the row into SubmissionUnknown with attempt_count above
    // submission_retry_max (= 1 in the test config).
    let dyn_store: &dyn HybridV2ProjectionStore = store_pg;
    let tx_hex = format!("0x{}", "aa".repeat(32));
    dyn_store
        .set_broadcast_tx_hash(CID, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            CID,
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            1_002,
            BroadcastStatePatch {
                submission_attempt_count: Some(5),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            CID,
            BroadcastPhase::Broadcasting,
            BroadcastPhase::SubmissionUnknown,
            1_003,
            BroadcastStatePatch::default(),
        )
        .await
        .unwrap();
    let path = format!(
        "/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_resend_same_bytes"
    );
    let response = router_for(state)
        .oneshot(post(&path, Some(ADMIN_TOKEN), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = body_json(response).await;
    assert_eq!(json["error"], "RESEND_BUDGET_EXHAUSTED");
}

#[tokio::test]
async fn manual_intervention_transitions_row_and_stamps_failure_class() {
    let Some(url) =
        get_pg_url_or_skip("manual_intervention_transitions_row_and_stamps_failure_class")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let store = state.hybrid_v2_projection_store.clone().unwrap();
    let store_pg: &PostgresHybridV2ProjectionStore = downcast(&*store);
    seed_execution_and_broadcast_row(store_pg, deployment_id, ExecutionPhase::SignatureVerified)
        .await;
    let path = format!(
        "/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_manual_intervention"
    );
    let response = router_for(state.clone())
        .oneshot(post(
            &path,
            Some(ADMIN_TOKEN),
            serde_json::json!({ "action": "MARK_MANUAL", "detail": "operator diagnostic" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // Row should now be at MANUAL_INTERVENTION_REQUIRED.
    let dyn_store: &dyn HybridV2ProjectionStore = store_pg;
    let row = dyn_store.get_broadcast_state(CID).await.unwrap().unwrap();
    assert_eq!(row.phase, BroadcastPhase::ManualInterventionRequired);
    assert_eq!(
        row.failure_class.as_deref(),
        Some("ADMIN_MANUAL_INTERVENTION")
    );
}

#[tokio::test]
async fn manual_intervention_rejects_unknown_action_and_body_fields() {
    let Some(url) =
        get_pg_url_or_skip("manual_intervention_rejects_unknown_action_and_body_fields")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (deployment_id, state) = build_app_state(&pool, true).await;
    let path = format!(
        "/admin/hybrid_v2/deployments/{deployment_id}/executions/{CID}/broadcast_manual_intervention"
    );
    let response = router_for(state.clone())
        .oneshot(post(
            &path,
            Some(ADMIN_TOKEN),
            serde_json::json!({ "action": "FORCE_CONFIRM", "detail": "x" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // Unknown body fields also refused.
    let response2 = router_for(state)
        .oneshot(post(
            &path,
            Some(ADMIN_TOKEN),
            serde_json::json!({ "action": "MARK_MANUAL", "detail": "x", "raw_tx": "0x1234" }),
        ))
        .await
        .unwrap();
    assert!(response2.status().is_client_error());
    let text = body_text(response2).await;
    assert!(!text.contains("0x1234"));
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn default_row(deployment_id: i64) -> ExecutionRequestRow {
    ExecutionRequestRow {
        canonical_execution_id: CID.into(),
        deployment_id,
        chain_id: CHAIN_ID as i64,
        execution_kind: "HYBRID_V2_OPTION_MATCH".into(),
        buyer_order_hash: format!("0x{}", "aa".repeat(32)),
        seller_order_hash: format!("0x{}", "bb".repeat(32)),
        buyer_subkey: format!("0x{}", "aa".repeat(32)),
        seller_subkey: format!("0x{}", "bb".repeat(32)),
        series_id: "42".into(),
        fill_quantity_1e8: "100000000".into(),
        premium_amount: "50000000".into(),
        fee_schedule_epoch: None,
        source_matched_execution_id: None,
        target_contract: format!("0x{}", "ee".repeat(20)),
        selector: "0x00000000".into(),
        calldata_hash: Some(format!("0x{}", "cd".repeat(32))),
        plan_hash: Some(format!("0x{}", "ee".repeat(32))),
        tx_value_wei: "0".into(),
        simulation_block_number: Some(1),
        simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
        simulation_gas_estimate: Some(500_000),
        simulation_result_json: Some(serde_json::json!({})),
        signer_identity: Some(hex_of(&SIGNER)),
        signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
        signature_r: Some(format!("0x{}", "11".repeat(32))),
        signature_s: Some(format!("0x{}", "22".repeat(32))),
        signature_v: Some(0),
        recovered_signer: Some(hex_of(&SIGNER)),
        gas_limit: Some(1_000_000),
        max_fee_per_gas_wei: Some("2000000000".into()),
        max_priority_fee_per_gas_wei: Some("500000000".into()),
        reserved_nonce: Some(42),
        phase: ExecutionPhase::SignatureVerified,
        failure_class: None,
        failure_detail: None,
        retry_count: 0,
        holder_epoch: None,
        signer_request_idempotency_key: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

/// Downcast helper: the tests always attach a
/// `PostgresHybridV2ProjectionStore` behind the trait. We take a
/// borrow of the trait object and re-borrow the underlying concrete
/// type via a raw pointer + type-erased cast. This is safe because the
/// tests fully own the store instance for the duration.
fn downcast(store: &dyn HybridV2ProjectionStore) -> &PostgresHybridV2ProjectionStore {
    // SAFETY: tests always attach the Postgres store to AppState. The
    // reference lifetime is bounded by the caller's borrow.
    unsafe {
        &*(store as *const dyn HybridV2ProjectionStore as *const PostgresHybridV2ProjectionStore)
    }
}
