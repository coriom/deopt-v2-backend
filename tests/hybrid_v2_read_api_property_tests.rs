//! Bounded property + reorg tests for the Hybrid V2 read API.

mod hybrid_v2_support;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use deopt_v2_backend::api::hybrid_v2_read::{
    build_hybrid_v2_read_router, DeploymentEntry, HybridV2ApiState,
};
use deopt_v2_backend::hybrid_v2::chain_source::InMemoryChainSource;
use deopt_v2_backend::hybrid_v2::runtime::IndexerRuntime;
use hybrid_v2_support::{
    baseline_manifest, block, deposit_log, pad_address, subaccount_created_log,
};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
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

fn build_frozen_state(n_blocks: u64) -> Arc<DeploymentEntry> {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    for i in 1..=n_blocks {
        source.push(block(
            i,
            &format!("0xb{:04x}", i),
            &if i == 1 {
                "0xb0000".to_string()
            } else {
                format!("0xb{:04x}", i - 1)
            },
            1000 + i * 12,
            vec![
                subaccount_created_log(&manifest, "0xa1", i as u32, &format!("0xf{:04x}", i)),
                deposit_log(
                    &manifest,
                    &format!("0xf{:04x}", i),
                    "0xa1",
                    i as u32,
                    "0xef",
                    "100",
                ),
            ],
        ));
    }
    let mut runtime = IndexerRuntime::new(1, manifest);
    while runtime.tick(&source).unwrap() {}
    Arc::new(DeploymentEntry::new(runtime))
}

#[tokio::test]
async fn pagination_frozen_dataset_yields_every_item_once() {
    let entry = build_frozen_state(20);
    let state = HybridV2ApiState::new(vec![entry]);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages < 100, "runaway pagination");
        let uri = if let Some(c) = &cursor {
            format!(
                "/accounts/{}/hybrid-v2/history?limit=5&cursor={}",
                owner_hex, c
            )
        } else {
            format!("/accounts/{}/hybrid-v2/history?limit=5", owner_hex)
        };
        let (status, body) = get_body(app.clone(), &uri).await;
        assert_eq!(status, StatusCode::OK);
        let items = body["data"].as_array().unwrap();
        if items.is_empty() {
            break;
        }
        for item in items {
            let id = item["event_id"].as_str().unwrap().to_string();
            assert!(seen.insert(id.clone()), "duplicate: {}", id);
        }
        cursor = body["next_cursor"].as_str().map(String::from);
        if cursor.is_none() {
            break;
        }
        if items.len() < 5 {
            break;
        }
    }
    // 20 subaccount_created + 20 deposit = 40 events for owner 0xa1.
    assert_eq!(seen.len(), 40);
}

#[tokio::test]
async fn filtered_pagination_equals_pagination_over_filtered_set() {
    let entry = build_frozen_state(10);
    let state = HybridV2ApiState::new(vec![entry]);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    // All DEPOSIT events
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = if let Some(c) = &cursor {
            format!(
                "/accounts/{}/hybrid-v2/history?families=DEPOSIT&limit=3&cursor={}",
                owner_hex, c
            )
        } else {
            format!(
                "/accounts/{}/hybrid-v2/history?families=DEPOSIT&limit=3",
                owner_hex
            )
        };
        let (status, body) = get_body(app.clone(), &uri).await;
        assert_eq!(status, StatusCode::OK);
        let items = body["data"].as_array().unwrap();
        for item in items {
            all.push(item["event_id"].as_str().unwrap().to_string());
        }
        cursor = body["next_cursor"].as_str().map(String::from);
        if items.len() < 3 || cursor.is_none() {
            break;
        }
    }
    assert_eq!(all.len(), 10);
    // No duplicates.
    let set: HashSet<_> = all.iter().collect();
    assert_eq!(set.len(), all.len());
}

#[tokio::test]
async fn reorg_stale_cursor_returns_409_conflict() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source
        .push(block(
            1,
            "0xb1",
            "0xb0",
            1000,
            vec![
                subaccount_created_log(&manifest, "0xa1", 1, "0xf1"),
                deposit_log(&manifest, "0xf1", "0xa1", 1, "0xef", "100"),
            ],
        ))
        .push(block(
            2,
            "0xb2a",
            "0xb1",
            1012,
            vec![deposit_log(&manifest, "0xf1", "0xa1", 1, "0xef", "50")],
        ));
    let mut runtime = IndexerRuntime::new(1, manifest.clone());
    while runtime.tick(&source).unwrap() {}
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let state = HybridV2ApiState::new(vec![entry.clone()]);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    // Get initial cursor
    let (_status, body) = get_body(
        app.clone(),
        &format!("/accounts/{}/hybrid-v2/history?limit=2", owner_hex),
    )
    .await;
    let cursor = body["next_cursor"].as_str().unwrap().to_string();
    // Force a reorg on the source and re-tick the runtime.
    source.reorg_from(
        2,
        vec![block(
            2,
            "0xb2b",
            "0xb1",
            1012,
            vec![deposit_log(&manifest, "0xf1", "0xa1", 1, "0xef", "77")],
        )],
    );
    source.push(block(
        3,
        "0xb3",
        "0xb2b",
        1024,
        vec![deposit_log(&manifest, "0xf1", "0xa1", 1, "0xef", "9")],
    ));
    {
        let rt_lock = entry
            .runtime
            .as_ref()
            .expect("runtime-backed entry must carry a runtime handle")
            .clone();
        let mut rt = rt_lock.write().unwrap();
        while rt.tick(&source).unwrap() {}
    }
    // Now the old cursor is stale — bound to hash 0xb2 which is no longer canonical.
    let uri = format!(
        "/accounts/{}/hybrid-v2/history?limit=2&cursor={}",
        owner_hex, cursor
    );
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "STALE_CURSOR");
    assert!(body["retryable"].as_bool().unwrap());
}

#[tokio::test]
async fn owner_wide_history_never_leaks_sibling_events() {
    let manifest = baseline_manifest(84532);
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xa1", 1, "0xf1"),
            subaccount_created_log(&manifest, "0xa2", 1, "0xf2"),
            deposit_log(&manifest, "0xf1", "0xa1", 1, "0xef", "100"),
            deposit_log(&manifest, "0xf2", "0xa2", 1, "0xef", "999"),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    runtime.tick(&source).unwrap();
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let state = HybridV2ApiState::new(vec![entry]);
    let app = build_hybrid_v2_read_router(state);
    let owner_hex = pad_address("0xa1");
    let (status, body) = get_body(
        app,
        &format!("/accounts/{}/hybrid-v2/history?limit=100", owner_hex),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for ev in body["data"].as_array().unwrap() {
        let owner = ev["owner"].as_str().unwrap_or("");
        assert_eq!(owner, pad_address("0xa1"), "leaked sibling event: {}", ev);
    }
}

#[tokio::test]
async fn integer_serialization_roundtrips_large_uint() {
    let manifest = baseline_manifest(84532);
    let big = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    let mut source = InMemoryChainSource::new(84532);
    source.push(block(
        1,
        "0xb1",
        "0xb0",
        1000,
        vec![
            subaccount_created_log(&manifest, "0xa1", 1, "0xf1"),
            deposit_log(&manifest, "0xf1", "0xa1", 1, "0xef", big),
        ],
    ));
    let mut runtime = IndexerRuntime::new(1, manifest);
    runtime.tick(&source).unwrap();
    let entry = Arc::new(DeploymentEntry::new(runtime));
    let state = HybridV2ApiState::new(vec![entry]);
    let app = build_hybrid_v2_read_router(state);
    let sk = format!("0x{:0>64}", "f1");
    let uri = format!("/subaccounts/{}/collateral", sk);
    let (status, body) = get_body(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows[0]["balance"], big);
}

/// Silence unused import when this module is compiled but the RwLock helper
/// isn't otherwise referenced.
#[allow(dead_code)]
fn _keep_rwlock_import() -> RwLock<()> {
    RwLock::new(())
}
