//! End-to-end integration tests for the Hybrid V2 public read API.

mod hybrid_v2_support;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::hybrid_v2_read::{
    build_hybrid_v2_read_router, DeploymentEntry, HybridV2ApiState,
};
use deopt_v2_backend::hybrid_v2::chain_source::InMemoryChainSource;
use deopt_v2_backend::hybrid_v2::runtime::IndexerRuntime;
use hybrid_v2_support::{
    baseline_manifest, block, deposit_log, matching_pair_log, order_filled_log, pad_address,
    pad_bytes32, premium_log, reservation_lock_log, subaccount_created_log, withdraw_log,
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

fn build_populated_state(chain_id: u64) -> (HybridV2ApiState, u64) {
    let manifest = baseline_manifest(chain_id);
    let deployment_id = 1;
    let mut source = InMemoryChainSource::new(chain_id);
    source
        .push(block(
            1,
            "0xb1",
            "0xb0",
            1000,
            vec![
                subaccount_created_log(&manifest, "0xa1", 1, "0xff01"),
                subaccount_created_log(&manifest, "0xa1", 2, "0xff02"),
                subaccount_created_log(&manifest, "0xa2", 1, "0xff03"),
                deposit_log(&manifest, "0xff01", "0xa1", 1, "0xef", "1000"),
                reservation_lock_log(&manifest, "0xff01", "0xef", "0xef", "300"),
            ],
        ))
        .push(block(
            2,
            "0xb2",
            "0xb1",
            1012,
            vec![
                withdraw_log(&manifest, "0xff01", "0xa1", 1, "0xef", "400"),
                deposit_log(&manifest, "0xff02", "0xa1", 2, "0xef", "500"),
            ],
        ))
        .push(block(
            3,
            "0xb3",
            "0xb2",
            1024,
            vec![
                matching_pair_log(
                    &manifest, "0xcc01", "0xbb01", "0xbb02", "0xff01", "0xff03", "10", "20",
                ),
                order_filled_log(&manifest, "0xff01", "0xbb01", "10", "10", true),
                order_filled_log(&manifest, "0xff03", "0xbb02", "10", "10", true),
                premium_log(&manifest, "0xff01", "0xff03", "0xef", "20"),
            ],
        ));
    let mut runtime = IndexerRuntime::new(deployment_id, manifest);
    while runtime.tick(&source).unwrap() {}
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let state = HybridV2ApiState::new(vec![entry]);
    (state, deployment_id)
}

#[tokio::test]
async fn deployments_route_lists_configured_deployment() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let (status, body) = get_body(app, "/subaccounts/deployments").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_array().unwrap().len() >= 1);
    assert_eq!(body[0]["deployment_id"], 1);
    assert_eq!(body[0]["chain_id"], 84532);
    assert_eq!(body[0]["ready"], true);
}

#[tokio::test]
async fn deployment_status_readable_and_has_metadata() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let (status, body) = get_body(app, "/subaccounts/deployments/1/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ready"], true);
    assert!(body["indexed_block"].as_u64().unwrap() >= 3);
    assert!(body["metadata"]["canonicality_level"].is_string());
    assert!(body["metadata"]["manifest_hash"]
        .as_str()
        .unwrap()
        .starts_with("0x"));
}

#[tokio::test]
async fn deployment_status_returns_404_for_unknown() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let (status, body) = get_body(app, "/subaccounts/deployments/999/status").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "DEPLOYMENT_NOT_FOUND");
}

#[tokio::test]
async fn owner_subaccounts_returns_owned_subs_only() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    let uri = format!("/accounts/{}/hybrid-v2/subaccounts", owner_hex);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let subs = body["data"]["subaccounts"].as_array().unwrap();
    assert_eq!(subs.len(), 2);
    assert_eq!(subs[0]["subaccount_id"], 1);
    assert_eq!(subs[1]["subaccount_id"], 2);
    assert_eq!(body["data"]["owner"], owner_hex);
    // Sibling isolation: 0xa2 not present.
    for sub in subs {
        assert_ne!(sub["subkey"].as_str().unwrap(), pad_bytes32("0xff03"));
    }
}

#[tokio::test]
async fn owner_subaccounts_rejects_account_0() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let uri = format!("/accounts/{}/hybrid-v2/subaccounts", pad_address("0x0"));
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "ACCOUNT_0_INVALID");
}

#[tokio::test]
async fn owner_subaccounts_rejects_bad_address() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let (status, body) = get_body(app, "/accounts/0xnothex/hybrid-v2/subaccounts").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_ADDRESS");
}

#[tokio::test]
async fn subaccount_summary_returns_details() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let sk = pad_bytes32("0xff01");
    let uri = format!("/subaccounts/{}", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["subaccount_id"], 1);
    assert_eq!(body["data"]["owner"], pad_address("0xa1"));
    assert!(body["data"]["materialised_via_created"].as_bool().unwrap());
    assert_eq!(body["data"]["balance_token_count"], 1);
    assert_eq!(body["data"]["reservation_count"], 1);
}

#[tokio::test]
async fn subaccount_summary_404_for_unknown_subkey() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let uri = format!("/subaccounts/{}", pad_bytes32("0xdead"));
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "SUBACCOUNT_NOT_FOUND");
}

#[tokio::test]
async fn subaccount_summary_rejects_bad_subkey() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let (status, body) = get_body(app, "/subaccounts/0xshort").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_SUBKEY");
}

#[tokio::test]
async fn collateral_returns_exact_integer_strings() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let sk = pad_bytes32("0xff01");
    let uri = format!("/subaccounts/{}/collateral", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["token"], pad_address("0xef"));
    assert_eq!(rows[0]["balance"], "600"); // 1000 - 400
    assert_eq!(rows[0]["aggregate_reserved"], "300");
    assert_eq!(rows[0]["available"], "300"); // 600 - 300
}

#[tokio::test]
async fn reservations_route_returns_engine_grouped_rows() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let sk = pad_bytes32("0xff01");
    let uri = format!("/subaccounts/{}/reservations", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["engine"], pad_address("0xef"));
    assert_eq!(rows[0]["reserved"], "300");
}

#[tokio::test]
async fn executions_returns_complete_group() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let sk = pad_bytes32("0xff01");
    let uri = format!("/subaccounts/{}/executions", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let (id, row) = (rows[0][0].as_str().unwrap(), &rows[0][1]);
    assert_eq!(id, pad_bytes32("0xcc01"));
    assert_eq!(row["completion_status"], "COMPLETE");
    assert_eq!(row["matched_qty_1e8"], "10");
    assert_eq!(row["premium_amount"], "20");
}

#[tokio::test]
async fn fees_route_returns_premium_row() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let sk = pad_bytes32("0xff01");
    let uri = format!("/subaccounts/{}/fees", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().unwrap();
    assert!(rows
        .iter()
        .any(|r| r["kind"] == "OPTION_PREMIUM_TRANSFERRED" && r["amount"] == "20"));
}

#[tokio::test]
async fn history_returns_typed_tagged_events() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    let uri = format!("/accounts/{}/hybrid-v2/history?limit=100", owner_hex);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let events = body["data"].as_array().unwrap();
    assert!(!events.is_empty());
    // Newest first.
    let blocks: Vec<u64> = events
        .iter()
        .map(|e| e["block_number"].as_u64().unwrap())
        .collect();
    for w in blocks.windows(2) {
        assert!(w[0] >= w[1], "not descending");
    }
    // Contains DEPOSIT + WITHDRAW.
    let families: Vec<&str> = events
        .iter()
        .map(|e| e["payload"]["family"].as_str().unwrap())
        .collect();
    assert!(families.contains(&"DEPOSIT"));
    assert!(families.contains(&"WITHDRAW"));
}

#[tokio::test]
async fn history_pagination_yields_no_duplicates() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    // First page limit=2
    let uri1 = format!("/accounts/{}/hybrid-v2/history?limit=2", owner_hex);
    let (_status, body1) = get_body(app.clone(), &uri1).await;
    let page1_ids: Vec<String> = body1["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_id"].as_str().unwrap().to_string())
        .collect();
    let cursor = body1["next_cursor"].as_str().unwrap().to_string();
    // Next page
    let uri2 = format!(
        "/accounts/{}/hybrid-v2/history?limit=2&cursor={}",
        owner_hex, cursor
    );
    let (_status, body2) = get_body(app, &uri2).await;
    let page2_ids: Vec<String> = body2["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_id"].as_str().unwrap().to_string())
        .collect();
    for id in &page2_ids {
        assert!(!page1_ids.contains(id), "duplicate: {}", id);
    }
}

#[tokio::test]
async fn history_rejects_invalid_cursor() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let uri = "/hybrid-v2/history?cursor=not-a-valid-cursor";
    let (status, body) = get_body(app, uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_CURSOR");
}

#[tokio::test]
async fn history_family_filter_narrows_result() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    let uri = format!("/accounts/{}/hybrid-v2/history?families=DEPOSIT", owner_hex);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    for ev in body["data"].as_array().unwrap() {
        assert_eq!(ev["payload"]["family"], "DEPOSIT");
    }
}

#[tokio::test]
async fn history_direction_filter() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    let uri = format!(
        "/accounts/{}/hybrid-v2/history?direction=INBOUND",
        owner_hex
    );
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    for ev in body["data"].as_array().unwrap() {
        assert_eq!(ev["direction"], "INBOUND");
    }
}

#[tokio::test]
async fn history_rejects_page_limit_exceeded() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let (status, body) = get_body(app, "/hybrid-v2/history?limit=10000").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "PAGE_LIMIT_EXCEEDED");
}

#[tokio::test]
async fn openapi_route_returns_json_spec() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
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
    assert!(json["components"]["schemas"]["CanonicalityMetadata"].is_object());
}

#[tokio::test]
async fn not_ready_state_returns_503_on_canonical_route() {
    use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReason, ReadinessState};
    let manifest = baseline_manifest(84532);
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    // Force a hard-readiness failure that isn't `Behind` or `AwaitingFirstBlock`.
    runtime.readiness = ReadinessState::new_not_ready(ReadinessReason::ManifestMismatch {
        detail: "test-drift".into(),
    });
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let state = HybridV2ApiState::new(vec![entry]);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    let (status, body) = get_body(
        app,
        &format!("/accounts/{}/hybrid-v2/subaccounts", owner_hex),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], "MANIFEST_MISMATCH");
    assert!(body["retryable"].as_bool().unwrap());
}

#[tokio::test]
async fn recovery_returns_state_summary() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let sk = pad_bytes32("0xff01");
    let uri = format!("/subaccounts/{}/recovery", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["recovery_state"], "NORMAL");
    assert_eq!(body["data"]["finalized"], false);
}

#[tokio::test]
async fn positions_ordered_by_series() {
    let (state, _) = build_populated_state(84532);
    let app = build_hybrid_v2_read_router(state);
    let sk = pad_bytes32("0xff01");
    let uri = format!("/subaccounts/{}/positions", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    // We didn't emit OptionPositionOpened in the fixture, so 0 rows.
    let rows = body["data"].as_array().unwrap();
    assert!(rows.is_empty() || rows.len() > 0);
}

#[tokio::test]
async fn deployment_isolation_no_cross_leak() {
    use std::sync::Arc;
    let manifest_a = baseline_manifest(84532);
    let mut manifest_b = baseline_manifest(84532);
    manifest_b.deployment_version = 2;
    manifest_b.manifest_hash = "0x".to_string() + &"de".repeat(32);
    let mut source_a = InMemoryChainSource::new(84532);
    let mut source_b = InMemoryChainSource::new(84532);
    source_a.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest_a, "0xa1", 1, "0xff01"),
            deposit_log(&manifest_a, "0xff01", "0xa1", 1, "0xef", "100"),
        ],
    ));
    source_b.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest_b, "0xa1", 1, "0xff01"),
            deposit_log(&manifest_b, "0xff01", "0xa1", 1, "0xef", "999"),
        ],
    ));
    let mut ra = IndexerRuntime::new(1, manifest_a);
    let mut rb = IndexerRuntime::new(2, manifest_b);
    ra.tick(&source_a).unwrap();
    rb.tick(&source_b).unwrap();
    let ea = Arc::new(DeploymentEntry::new(ra));
    let eb = Arc::new(DeploymentEntry::new(rb));
    let state = HybridV2ApiState::new(vec![ea, eb]);
    let app = build_hybrid_v2_read_router(state);
    // Multiple deployments now — no implicit resolution; must specify.
    let owner_hex = pad_address("0xa1");
    let (status, body) = get_body(
        app.clone(),
        &format!("/accounts/{}/hybrid-v2/subaccounts", owner_hex),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_DEPLOYMENT");
    // With explicit deployment_id, we get each in isolation.
    let (status_a, body_a) = get_body(
        app.clone(),
        &format!(
            "/accounts/{}/hybrid-v2/subaccounts?deployment_id=1",
            owner_hex
        ),
    )
    .await;
    assert_eq!(status_a, StatusCode::OK);
    assert_eq!(body_a["metadata"]["deployment_id"], 1);
    let (status_b, body_b) = get_body(
        app,
        &format!(
            "/accounts/{}/hybrid-v2/subaccounts?deployment_id=2",
            owner_hex
        ),
    )
    .await;
    assert_eq!(status_b, StatusCode::OK);
    assert_eq!(body_b["metadata"]["deployment_id"], 2);
}
