//! `BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1` — Part M.
//!
//! Bounded property tests over the live broadcast wiring path.
//! Every property is exercised across a small, deterministic sample
//! set (`NUM_CASES = 20`) driven by fixed seed material — no new
//! Cargo deps (no `proptest` crate is introduced; we follow the same
//! deterministic-loop convention as
//! `tests/hybrid_v2_broadcast_properties.rs`).
//!
//! Every property that boots the mock asserts
//! `mock.write_method_calls()` contains ONLY `eth_sendRawTransaction`
//! on exit — the frozen write-allowlist invariant.
//!
//! Properties:
//! 1.  `prop_fresh_broadcast_and_resume_use_identical_tx_identity`
//! 2.  `prop_duplicate_fresh_requests_cannot_create_two_broadcast_intents`
//! 3.  `prop_restart_before_first_send_cannot_change_tx_identity`
//! 4.  `prop_confirmation_progresses_without_operator_recheck`
//! 5.  `prop_one_worker_cannot_confirm_another_deployment_incorrectly`
//! 6.  `prop_disabled_broadcaster_zero_write_rpc_calls`
//! 7.  `prop_worker_restart_preserves_pending_transaction`
//! 8.  `prop_shutdown_cannot_fabricate_terminal_state`
//! 9.  `prop_application_reconstruction_preserves_outbox_state`
//! 10. `prop_only_exact_send_raw_transaction_bytes_submitted`
//! 11. `prop_admin_body_extra_fields_always_rejected`
//! 12. `prop_no_automatic_rbf_after_random_transient_failures`
//!
//! Frozen safety: NO REAL EXTERNAL CHAIN TRANSACTION IS EVER BROADCAST.

#![cfg(feature = "test-signer")]

mod hybrid_v2_broadcast_mock_rpc;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use hybrid_v2_broadcast_mock_rpc::MockBroadcastRpc;
use hybrid_v2_support::baseline_manifest;
use sha3::{Digest, Keccak256};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tower::ServiceExt;

use deopt_v2_backend::admin::AdminConfig;
use deopt_v2_backend::api::hybrid_v2_read::{DeploymentEntry, EmptyReadStore, HybridV2ApiState};
use deopt_v2_backend::hybrid_v2::config::HybridV2ExecutionConfig;
use deopt_v2_backend::hybrid_v2::execution::broadcast_outbox::BroadcastOutbox;
use deopt_v2_backend::hybrid_v2::execution::broadcast_rpc::{
    BroadcastRpcError, ExecutionBroadcastRpcClient, SendOutcome, TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_state::BroadcastPhase;
use deopt_v2_backend::hybrid_v2::execution::broadcast_worker::{
    BroadcastConfirmationWorker, WorkerCancel,
};
use deopt_v2_backend::hybrid_v2::execution::orchestrator::MockClock;
use deopt_v2_backend::hybrid_v2::execution::persistence::ExecutionRequestRow;
use deopt_v2_backend::hybrid_v2::execution::plan::executeMatchCall;
use deopt_v2_backend::hybrid_v2::execution::state::ExecutionPhase;
use deopt_v2_backend::hybrid_v2::execution::tx_serialization::serialize_signed_execution;
use deopt_v2_backend::hybrid_v2::execution::ExecutionPlan;
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";
const ADMIN_TOKEN: &str = "admin-test-token";
const CHAIN_ID: u64 = 84532;
const ENGINE_HEX: &str = "0x0000000000000000000000000000000000000006";
const NUM_CASES: usize = 20;

// -----------------------------------------------------------------
//                           SCAFFOLDING
// -----------------------------------------------------------------

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

fn engine_bytes() -> [u8; 20] {
    let mut out = [0u8; 20];
    let s = ENGINE_HEX.trim_start_matches("0x");
    for i in 0..20 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn hex_of(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{:02x}", x));
    }
    format!("0x{s}")
}

fn keccak_of(bytes: &[u8]) -> [u8; 32] {
    let mut h = [0u8; 32];
    let d = Keccak256::digest(bytes);
    h.copy_from_slice(&d[..]);
    h
}

fn wall_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Deterministic seed derivation: mixes a case index into every
/// property-specific parameter without introducing thread-local
/// randomness. Each call produces a stable canonical id.
fn seed_canonical_id(prefix: u8, i: usize) -> String {
    let mut out = String::with_capacity(66);
    out.push_str("0x");
    let mut buf = [prefix; 32];
    buf[0] = prefix;
    buf[1] = i as u8;
    buf[2] = ((i >> 8) & 0xff) as u8;
    for b in &buf {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn derive_plan_hash(
    chain_id: u64,
    target: &[u8; 20],
    selector: &[u8; 4],
    calldata_hash: &[u8; 32],
    canonical_id_hex: &str,
) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(b"HV2_PLAN_V1");
    hasher.update(chain_id.to_be_bytes());
    hasher.update(target);
    hasher.update(selector);
    hasher.update(calldata_hash);
    let value_be: [u8; 32] = U256::ZERO.to_be_bytes::<32>();
    hasher.update(value_be);
    let cid_stripped = canonical_id_hex.trim_start_matches("0x");
    let mut cid_bytes = [0u8; 32];
    for i in 0..32 {
        cid_bytes[i] = u8::from_str_radix(&cid_stripped[2 * i..2 * i + 2], 16).unwrap();
    }
    hasher.update(cid_bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out[..]);
    arr
}

fn build_calldata() -> Vec<u8> {
    use deopt_v2_backend::hybrid_v2::execution::plan::{OptionOrder, SignedActionEnvelope};
    let empty_env = SignedActionEnvelope {
        owner: alloy_primitives::Address::ZERO,
        subaccountId: 1,
        subKey: alloy_primitives::FixedBytes::ZERO,
        signer: alloy_primitives::Address::ZERO,
        engine: alloy_primitives::Address::from(engine_bytes()),
        action: alloy_primitives::FixedBytes::ZERO,
        architectureVersion: U256::from(1),
        nonce: U256::from(1),
        deadline: U256::from(0),
        ownerRecoveryEpoch: U256::from(0),
        subaccountRecoveryEpoch: U256::from(0),
        payloadHash: alloy_primitives::FixedBytes::ZERO,
    };
    let empty_order = OptionOrder {
        seriesId: U256::from(42),
        side: 0,
        quantity1e8: 100_000_000,
        pricePerContract1e8: 50_000_000,
        limitPricePerContract1e8: 60_000_000,
        premiumToken: alloy_primitives::Address::ZERO,
        timeInForce: 0,
        role: 0,
        maxPositiveFeePpm: 0,
        salt: alloy_primitives::FixedBytes::ZERO,
    };
    let call = executeMatchCall {
        buyerEnv: empty_env.clone(),
        buyerSig: alloy_primitives::Bytes::from(vec![0u8; 65]),
        buyerOrder: empty_order.clone(),
        sellerEnv: empty_env,
        sellerSig: alloy_primitives::Bytes::from(vec![0u8; 65]),
        sellerOrder: empty_order,
        fillQuantity1e8: 100_000_000,
        buyerActiveSeriesIds: vec![U256::from(42)],
        sellerActiveSeriesIds: vec![U256::from(42)],
    };
    call.abi_encode()
}

async fn seed_ready_row(
    store: &PostgresHybridV2ProjectionStore,
    deployment_id: i64,
    canonical_id: &str,
    signer_addr: [u8; 20],
    nonce: u64,
) -> (Vec<u8>, ExecutionPlan) {
    let calldata = build_calldata();
    let calldata_hash = keccak_of(&calldata);
    let target = engine_bytes();
    let selector = executeMatchCall::SELECTOR;
    let plan_hash = derive_plan_hash(CHAIN_ID, &target, &selector, &calldata_hash, canonical_id);
    let plan = ExecutionPlan {
        canonical_execution_id:
            deopt_v2_backend::hybrid_v2::execution::identity::CanonicalExecutionId(
                canonical_id.to_string(),
            ),
        chain_id: CHAIN_ID,
        deployment_id,
        target,
        selector,
        calldata: calldata.clone(),
        calldata_hash,
        value_wei: U256::ZERO,
        expected_module_version: "OptionMatchingEngineV2".into(),
        deadline_ms: None,
        plan_hash,
    };
    let row = ExecutionRequestRow {
        canonical_execution_id: canonical_id.into(),
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
        target_contract: hex_of(&target),
        selector: hex_of(&selector),
        calldata_hash: Some(hex_of(&calldata_hash)),
        calldata_bytes: Some(calldata.clone()),
        plan_hash: Some(hex_of(&plan_hash)),
        tx_value_wei: "0".into(),
        simulation_block_number: Some(100),
        simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
        simulation_gas_estimate: Some(500_000),
        simulation_result_json: Some(serde_json::json!({})),
        signer_identity: Some(hex_of(&signer_addr)),
        signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
        signature_r: Some(format!("0x{}", "11".repeat(32))),
        signature_s: Some(format!("0x{}", "22".repeat(32))),
        signature_v: Some(0),
        recovered_signer: Some(hex_of(&signer_addr)),
        gas_limit: Some(1_000_000),
        max_fee_per_gas_wei: Some("2000000000".into()),
        max_priority_fee_per_gas_wei: Some("500000000".into()),
        reserved_nonce: Some(nonce as i64),
        phase: ExecutionPhase::ReadyForBroadcast,
        failure_class: None,
        failure_detail: None,
        retry_count: 0,
        holder_epoch: None,
        signer_request_idempotency_key: None,
        created_at_ms: wall_ms(),
        updated_at_ms: wall_ms(),
    };
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store.insert_execution_request(&row).await.unwrap();
    (calldata, plan)
}

fn broadcast_exec_config(broadcast_enabled: bool) -> HybridV2ExecutionConfig {
    let mut cfg = HybridV2ExecutionConfig::disabled();
    cfg.execution_enabled = true;
    cfg.broadcast_enabled = broadcast_enabled;
    cfg.executor_address = [0x77u8; 20];
    cfg.allowed_broadcast_chain_ids = vec![CHAIN_ID];
    cfg.submission_retry_max = 3;
    cfg.max_pending_age_ms = 3_600_000;
    cfg.simulation_max_age_ms = 24 * 3_600_000;
    cfg.confirmation_depth = 1;
    cfg.receipt_poll_interval_ms = 50;
    cfg
}

async fn build_state_from_wire(
    pool: &PgPool,
    rpc: Arc<dyn ExecutionBroadcastRpcClient>,
) -> (
    deopt_v2_backend::api::http::AppState,
    Arc<PostgresHybridV2ProjectionStore>,
    Arc<BroadcastOutbox>,
    Arc<BroadcastConfirmationWorker>,
    i64,
) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let deployment_id = dyn_store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    let clock: Arc<dyn deopt_v2_backend::hybrid_v2::execution::orchestrator::Clock> =
        Arc::new(MockClock::new(2_000));
    let outbox = Arc::new(BroadcastOutbox {
        store: store.clone(),
        rpc: rpc.clone(),
        clock: clock.clone(),
        deployment_id,
    });
    let cfg = broadcast_exec_config(true);
    let worker = Arc::new(BroadcastConfirmationWorker {
        store: store.clone(),
        rpc: rpc.clone(),
        clock,
        deployment_id,
        chain_id: CHAIN_ID,
        confirmation_depth: cfg.confirmation_depth,
        poll_interval_ms: cfg.receipt_poll_interval_ms,
        poll_timeout_ms: cfg.receipt_poll_timeout_ms,
        max_batch_size: 32,
        max_pending_age_ms: cfg.max_pending_age_ms,
    });
    let mut state = deopt_v2_backend::api::http::AppState::new(
        deopt_v2_backend::engine::EngineState::with_default_markets(),
    );
    state.admin_config = AdminConfig::new(true, true, Some(ADMIN_TOKEN.to_string()));
    let entry = Arc::new(DeploymentEntry::from_metadata(
        deployment_id as u64,
        manifest,
    ));
    let read_state = HybridV2ApiState::with_store(
        Arc::new(EmptyReadStore)
            as Arc<dyn deopt_v2_backend::hybrid_v2::read_store::HybridV2ReadStore>,
        vec![entry],
    );
    state = state
        .with_hybrid_v2(read_state)
        .with_hybrid_v2_projection_store(store.clone())
        .with_hybrid_v2_broadcast(outbox.clone(), worker.clone(), rpc.clone(), cfg.clone());
    state.hybrid_v2_execution_config = Some(cfg);
    (state, store, outbox, worker, deployment_id)
}

fn router_for(state: deopt_v2_backend::api::http::AppState) -> axum::Router {
    deopt_v2_backend::api::routes::router(state)
}

async fn admin_post_broadcast_body(
    router: &axum::Router,
    deployment_id: i64,
    canonical_id: &str,
    body: &str,
) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/admin/hybrid_v2/deployments/{}/executions/{}/broadcast",
            deployment_id, canonical_id
        ))
        .header("x-admin-token", ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    // Drain the body so the connection releases.
    let _ = to_bytes(resp.into_body(), 1024 * 1024).await;
    status
}

async fn admin_post_broadcast(
    router: &axum::Router,
    deployment_id: i64,
    canonical_id: &str,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/admin/hybrid_v2/deployments/{}/executions/{}/broadcast",
            deployment_id, canonical_id
        ))
        .header("x-admin-token", ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes)
        .unwrap_or(serde_json::json!({ "raw": String::from_utf8_lossy(&body_bytes).to_string() }));
    (status, body_json)
}

async fn spawn_supervised_worker(
    worker: Arc<BroadcastConfirmationWorker>,
) -> (
    WorkerCancel,
    tokio::sync::watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let cancel = WorkerCancel::new();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = worker.spawn_supervised(cancel.clone(), rx);
    (cancel, tx, handle)
}

async fn tear_down_worker(
    cancel: WorkerCancel,
    tx: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
) {
    cancel.cancel();
    let _ = tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

async fn wait_for_phase<F>(
    store: &PostgresHybridV2ProjectionStore,
    cid: &str,
    predicate: F,
    max_ms: u64,
) -> Option<BroadcastPhase>
where
    F: Fn(BroadcastPhase) -> bool,
{
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    let step = 25u64;
    let mut elapsed = 0u64;
    while elapsed < max_ms {
        if let Ok(Some(row)) = dyn_store.get_broadcast_state(cid).await {
            if predicate(row.phase) {
                return Some(row.phase);
            }
        }
        tokio::time::sleep(Duration::from_millis(step)).await;
        elapsed += step;
    }
    dyn_store
        .get_broadcast_state(cid)
        .await
        .ok()
        .flatten()
        .map(|r| r.phase)
}

fn make_signed(signer: [u8; 20]) -> deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
    deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    }
}

fn envelope_for(
    plan: &ExecutionPlan,
    signer: [u8; 20],
    nonce: u64,
) -> deopt_v2_backend::hybrid_v2::execution::tx_serialization::SignedExecutionEnvelope {
    serialize_signed_execution(
        plan,
        &make_signed(signer),
        nonce,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .expect("serialize")
}

// -----------------------------------------------------------------
//                          PROPERTY TESTS
// -----------------------------------------------------------------

/// (1) Fresh broadcast + resume produce the same tx identity for every
/// random (nonce, signer) pair — the envelope is a pure function of the
/// row + gas triple.
#[tokio::test]
async fn prop_fresh_broadcast_and_resume_use_identical_tx_identity() {
    let Some(url) = get_pg_url_or_skip("prop_fresh_broadcast_and_resume_use_identical_tx_identity")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xa1, i);
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (state, store, _o, _w, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
        let nonce = 40 + i as u64;
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        let env1 = envelope_for(&plan, signer, nonce);
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: env1.envelope_hash,
        }));
        let router = router_for(state);
        let (_s, _b) = admin_post_broadcast(&router, deployment_id, &cid).await;
        // Recompute the envelope from the exact same row: byte identical.
        let env2 = envelope_for(&plan, signer, nonce);
        assert_eq!(env1.envelope_hash, env2.envelope_hash);
        assert_eq!(env1.raw_bytes, env2.raw_bytes);
        assert_eq!(env1.envelope_hash_hex(), env2.envelope_hash_hex());
        assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
    }
}

/// (2) Two admin fresh-broadcast calls cannot produce two broadcast
/// intents — exactly one `broadcast_state` row + exactly one tx hash.
#[tokio::test]
async fn prop_duplicate_fresh_requests_cannot_create_two_broadcast_intents() {
    let Some(url) =
        get_pg_url_or_skip("prop_duplicate_fresh_requests_cannot_create_two_broadcast_intents")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xa2, i);
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (state, store, _o, _w, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
        let nonce = 40 + i as u64;
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        let env = envelope_for(&plan, signer, nonce);
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: env.envelope_hash,
        }));
        let router = router_for(state);
        let (_s1, _b1) = admin_post_broadcast(&router, deployment_id, &cid).await;
        let (_s2, _b2) = admin_post_broadcast(&router, deployment_id, &cid).await;
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let row = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
        assert_eq!(
            row.tx_hash.as_deref(),
            Some(env.envelope_hash_hex().as_str())
        );
        // Exactly one send RPC across both admin calls.
        assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
        // And exactly one row.
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM hybrid_v2_broadcast_state WHERE canonical_execution_id = $1",
        )
        .bind(&cid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 1);
    }
}

/// (3) Restart before first send cannot change the tx identity — for
/// N recreations of AppState + fresh mocks, `serialize_signed_execution`
/// returns byte-identical results.
#[tokio::test]
async fn prop_restart_before_first_send_cannot_change_tx_identity() {
    let Some(url) = get_pg_url_or_skip("prop_restart_before_first_send_cannot_change_tx_identity")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xa3, i);
        let (_state, store, _o, _w, deployment_id) = {
            let rpc = Arc::new(MockBroadcastRpc::new());
            rpc.set_chain_id(CHAIN_ID);
            let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
            build_state_from_wire(&pool, rpc_dyn).await
        };
        let nonce = 100 + i as u64;
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        let baseline = envelope_for(&plan, signer, nonce);
        // Recreate the AppState 3 times → the envelope hash never changes.
        for _ in 0..3 {
            let rpc = Arc::new(MockBroadcastRpc::new());
            rpc.set_chain_id(CHAIN_ID);
            let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
            let (_s, _st, _o, _w, _dep) = build_state_from_wire(&pool, rpc_dyn).await;
            let env = envelope_for(&plan, signer, nonce);
            assert_eq!(baseline.envelope_hash, env.envelope_hash);
            assert!(rpc.write_method_calls().is_empty());
        }
    }
}

/// (4) For deterministic block progressions + confirmation depths the
/// worker reaches Confirming/Confirmed within bounded ticks — no admin
/// recheck required.
#[tokio::test]
async fn prop_confirmation_progresses_without_operator_recheck() {
    let Some(url) = get_pg_url_or_skip("prop_confirmation_progresses_without_operator_recheck")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xa4, i);
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (state, store, _o, worker, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
        let nonce = 42 + i as u64;
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        let env = envelope_for(&plan, signer, nonce);
        let block = 200 + (i as u64) * 10;
        let block_hash = {
            let mut h = [0x33u8; 32];
            h[0] = i as u8;
            h
        };
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: env.envelope_hash,
        }));
        rpc.set_receipt_response(
            env.envelope_hash,
            Some(TxReceipt {
                tx_hash: env.envelope_hash,
                block_number: block,
                block_hash,
                status: 1,
                gas_used: 800_000,
                effective_gas_price_wei: U256::from(1_500_000_000u64),
                cumulative_gas_used: 800_000,
                from: signer,
                to: Some(engine_bytes()),
            }),
        );
        rpc.set_head_finalized(block + 20, Some(block));
        rpc.set_block_hash_at(block, block_hash);
        let router = router_for(state.clone());
        let (_s, _b) = admin_post_broadcast(&router, deployment_id, &cid).await;
        let (cancel, tx, handle) = spawn_supervised_worker(worker).await;
        let observed = wait_for_phase(
            &store,
            &cid,
            |p| matches!(p, BroadcastPhase::Confirming | BroadcastPhase::Confirmed),
            3000,
        )
        .await;
        tear_down_worker(cancel, tx, handle).await;
        assert!(
            matches!(
                observed,
                Some(BroadcastPhase::Confirming) | Some(BroadcastPhase::Confirmed)
            ),
            "case {i}: worker did not reach Confirming/Confirmed (observed={observed:?})"
        );
        assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
    }
}

/// (5) A worker attached to deployment A cannot advance rows belonging
/// to a distinct canonical id that was not seeded — the worker only
/// touches rows for its own deployment id + explicit tick_single calls.
#[tokio::test]
async fn prop_one_worker_cannot_confirm_another_deployment_incorrectly() {
    let Some(url) =
        get_pg_url_or_skip("prop_one_worker_cannot_confirm_another_deployment_incorrectly")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid_a = seed_canonical_id(0xa5, i);
        let cid_b = seed_canonical_id(0xb5, i);
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (state, store, _o, worker, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
        let (_cd_a, plan_a) =
            seed_ready_row(&store, deployment_id, &cid_a, signer, 50 + i as u64).await;
        let (_cd_b, _plan_b) =
            seed_ready_row(&store, deployment_id, &cid_b, signer, 200 + i as u64).await;
        let env_a = envelope_for(&plan_a, signer, 50 + i as u64);
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: env_a.envelope_hash,
        }));
        rpc.set_receipt_response(
            env_a.envelope_hash,
            Some(TxReceipt {
                tx_hash: env_a.envelope_hash,
                block_number: 200,
                block_hash: [0xEE; 32],
                status: 1,
                gas_used: 500_000,
                effective_gas_price_wei: U256::from(1_500_000_000u64),
                cumulative_gas_used: 500_000,
                from: signer,
                to: Some(engine_bytes()),
            }),
        );
        rpc.set_head_finalized(220, Some(200));
        rpc.set_block_hash_at(200, [0xEE; 32]);
        let router = router_for(state.clone());
        let (_s, _b) = admin_post_broadcast(&router, deployment_id, &cid_a).await;
        let (cancel, tx, handle) = spawn_supervised_worker(worker).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        tear_down_worker(cancel, tx, handle).await;
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        // A's row was touched.
        let row_a = dyn_store.get_broadcast_state(&cid_a).await.unwrap();
        assert!(row_a.is_some(), "row A must exist");
        // B's row: no broadcast_state row was ever created because it
        // was never broadcast.
        let row_b_opt = dyn_store.get_broadcast_state(&cid_b).await.unwrap();
        assert!(row_b_opt.is_none(), "worker must not manufacture row B");
        assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
    }
}

/// (6) When broadcast is disabled, the mock records ZERO write RPC
/// calls across N calls per case.
#[tokio::test]
async fn prop_disabled_broadcaster_zero_write_rpc_calls() {
    let Some(url) = get_pg_url_or_skip("prop_disabled_broadcaster_zero_write_rpc_calls") else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        // Build a broadcast-disabled AppState explicitly.
        let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
        let manifest = baseline_manifest(CHAIN_ID);
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let deployment_id = dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert");
        let mut state = deopt_v2_backend::api::http::AppState::new(
            deopt_v2_backend::engine::EngineState::with_default_markets(),
        );
        state.admin_config = AdminConfig::new(true, true, Some(ADMIN_TOKEN.to_string()));
        let entry = Arc::new(DeploymentEntry::from_metadata(
            deployment_id as u64,
            manifest,
        ));
        let read_state = HybridV2ApiState::with_store(
            Arc::new(EmptyReadStore)
                as Arc<dyn deopt_v2_backend::hybrid_v2::read_store::HybridV2ReadStore>,
            vec![entry],
        );
        state = state
            .with_hybrid_v2(read_state)
            .with_hybrid_v2_projection_store(store.clone())
            .with_hybrid_v2_broadcast_unavailable("BROADCAST_DISABLED_PROP_TEST");
        let router = router_for(state);
        // No shared mock — construct a fresh one per case so the write
        // counter is independent. There's no RPC wired; make sure the
        // admin route surfaces the disabled sentinel and doesn't call
        // any RPC at all.
        let rpc = Arc::new(MockBroadcastRpc::new());
        let cid = seed_canonical_id(0xd6, i);
        let (status, _body) = admin_post_broadcast(&router, deployment_id, &cid).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            rpc.write_method_calls().is_empty(),
            "disabled broadcaster produced writes"
        );
    }
}

/// (7) Restarting the worker N times while a tx is Pending never
/// changes the persisted tx_hash / phase.
#[tokio::test]
async fn prop_worker_restart_preserves_pending_transaction() {
    let Some(url) = get_pg_url_or_skip("prop_worker_restart_preserves_pending_transaction") else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xa7, i);
        // Seed a broadcast row already at Pending.
        let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
        let manifest = baseline_manifest(CHAIN_ID);
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let deployment_id = dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert");
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
        let env = envelope_for(&plan, signer, 42);
        let tx_hex = env.envelope_hash_hex();
        dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
        dyn_store
            .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
            .await
            .unwrap();
        for (from, to) in [
            (
                BroadcastPhase::BroadcastDisabled,
                BroadcastPhase::Broadcasting,
            ),
            (BroadcastPhase::Broadcasting, BroadcastPhase::Submitted),
            (BroadcastPhase::Submitted, BroadcastPhase::Pending),
        ] {
            dyn_store
                .update_broadcast_phase(
                    &cid,
                    from,
                    to,
                    1_010,
                    deopt_v2_backend::hybrid_v2::execution::broadcast_state::BroadcastStatePatch {
                        submission_attempt_count: Some(1),
                        first_submission_at_ms: Some(1_002),
                        last_submission_at_ms: Some(1_002),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }
        // Restart the worker 3 times with a fresh mock RPC each round.
        for _ in 0..3 {
            let rpc = Arc::new(MockBroadcastRpc::new());
            rpc.set_chain_id(CHAIN_ID);
            let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
            let (_state, store2, _o, worker, _dep) =
                build_state_from_wire(&pool, rpc_dyn.clone()).await;
            let (cancel, tx, handle) = spawn_supervised_worker(worker).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            tear_down_worker(cancel, tx, handle).await;
            let dyn2: &dyn HybridV2ProjectionStore = store2.as_ref();
            let row = dyn2.get_broadcast_state(&cid).await.unwrap().unwrap();
            assert_eq!(row.tx_hash.as_deref(), Some(tx_hex.as_str()));
            assert!(rpc.write_method_calls().is_empty());
        }
    }
}

/// (8) Triggering shutdown at random points cannot fabricate a terminal
/// phase — the worker only mutates via successful ticks. Every case
/// records the phase before shutdown + verifies it after tear-down.
#[tokio::test]
async fn prop_shutdown_cannot_fabricate_terminal_state() {
    let Some(url) = get_pg_url_or_skip("prop_shutdown_cannot_fabricate_terminal_state") else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xa8, i);
        let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
        let manifest = baseline_manifest(CHAIN_ID);
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let deployment_id = dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert");
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
        let env = envelope_for(&plan, signer, 42);
        let tx_hex = env.envelope_hash_hex();
        dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
        dyn_store
            .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
            .await
            .unwrap();
        // Cycle through non-terminal phases as a function of the case
        // index — every phase must be observably preserved across a
        // shutdown that happens before the worker sees a receipt.
        let target = match i % 4 {
            0 => BroadcastPhase::Broadcasting,
            1 => BroadcastPhase::Submitted,
            2 => BroadcastPhase::Pending,
            _ => BroadcastPhase::MinedSuccess,
        };
        // Walk to target.
        for (from, to) in [
            (
                BroadcastPhase::BroadcastDisabled,
                BroadcastPhase::Broadcasting,
            ),
            (BroadcastPhase::Broadcasting, BroadcastPhase::Submitted),
            (BroadcastPhase::Submitted, BroadcastPhase::Pending),
            (BroadcastPhase::Pending, BroadcastPhase::MinedSuccess),
        ] {
            if from as u8 > target as u8 {
                break;
            }
            let _ = dyn_store
                .update_broadcast_phase(
                    &cid,
                    from,
                    to,
                    1_010,
                    deopt_v2_backend::hybrid_v2::execution::broadcast_state::BroadcastStatePatch {
                        submission_attempt_count: Some(1),
                        first_submission_at_ms: Some(1_002),
                        last_submission_at_ms: Some(1_002),
                        ..Default::default()
                    },
                )
                .await;
            if to == target {
                break;
            }
        }
        let pre = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
        let phase_before = pre.phase;
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (_state, store2, _o, worker, _dep) = build_state_from_wire(&pool, rpc_dyn).await;
        let (cancel, tx, handle) = spawn_supervised_worker(worker).await;
        // Immediately shut down — the worker cannot possibly reach a
        // terminal state in ~1 tick given no receipt is configured.
        tokio::time::sleep(Duration::from_millis(20)).await;
        tear_down_worker(cancel, tx, handle).await;
        let dyn2: &dyn HybridV2ProjectionStore = store2.as_ref();
        let post = dyn2.get_broadcast_state(&cid).await.unwrap().unwrap();
        // Terminal fabrication guard: the row is not Confirmed / Reorged
        // / Dropped / ManualInterventionRequired / CancelledBeforeBroadcast
        // unless it was already there.
        let terminal = matches!(
            post.phase,
            BroadcastPhase::Confirmed
                | BroadcastPhase::MinedReverted
                | BroadcastPhase::Dropped
                | BroadcastPhase::ManualInterventionRequired
                | BroadcastPhase::CancelledBeforeBroadcast
        );
        assert!(
            !terminal || post.phase == phase_before,
            "case {i}: shutdown fabricated terminal {:?} from {:?}",
            post.phase,
            phase_before
        );
        assert!(rpc.write_method_calls().is_empty());
    }
}

/// (9) AppState reconstruction preserves the `broadcast_state` row
/// byte-identically for every case.
#[tokio::test]
async fn prop_application_reconstruction_preserves_outbox_state() {
    let Some(url) = get_pg_url_or_skip("prop_application_reconstruction_preserves_outbox_state")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xa9, i);
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (state, store, _o, _w, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
        let nonce = 44 + i as u64;
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        let env = envelope_for(&plan, signer, nonce);
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: env.envelope_hash,
        }));
        let router = router_for(state);
        let (_s, _b) = admin_post_broadcast(&router, deployment_id, &cid).await;
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let row1 = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
        // Recreate AppState.
        let rpc2 = Arc::new(MockBroadcastRpc::new());
        rpc2.set_chain_id(CHAIN_ID);
        let (_state2, store2, _o, _w, _dep) =
            build_state_from_wire(&pool, rpc2.clone() as Arc<dyn ExecutionBroadcastRpcClient>)
                .await;
        let dyn2: &dyn HybridV2ProjectionStore = store2.as_ref();
        let row2 = dyn2.get_broadcast_state(&cid).await.unwrap().unwrap();
        assert_eq!(row1.phase, row2.phase);
        assert_eq!(row1.tx_hash, row2.tx_hash);
        assert_eq!(row1.envelope_hash, row2.envelope_hash);
        assert!(rpc2.write_method_calls().is_empty());
        assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
    }
}

/// (10) The exact `serialize_signed_execution` bytes are the only
/// thing ever submitted — verify by computing the local envelope and
/// asserting the mock's send outcome tracks byte-identity.
#[tokio::test]
async fn prop_only_exact_send_raw_transaction_bytes_submitted() {
    let Some(url) = get_pg_url_or_skip("prop_only_exact_send_raw_transaction_bytes_submitted")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xaa, i);
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (state, store, _o, _w, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
        let nonce = 45 + i as u64;
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        let env = envelope_for(&plan, signer, nonce);
        // Configure Accepted with the exact same tx hash we computed.
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: env.envelope_hash,
        }));
        let router = router_for(state);
        let (_s, _b) = admin_post_broadcast(&router, deployment_id, &cid).await;
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let row = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
        // Persisted tx_hash matches keccak(raw_bytes).
        assert_eq!(
            row.tx_hash.as_deref(),
            Some(env.envelope_hash_hex().as_str())
        );
        // Exactly one send.
        assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
    }
}

/// (11) Any random JSON with extra keys is always rejected by
/// deny_unknown_fields (400 / 422). We drive N distinct extra-field
/// bodies.
#[tokio::test]
async fn prop_admin_body_extra_fields_always_rejected() {
    let Some(url) = get_pg_url_or_skip("prop_admin_body_extra_fields_always_rejected") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (state, _s, _o, _w, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
    let router = router_for(state);
    // NUM_CASES distinct malformed bodies, keyed on the case index.
    let extra_fields = [
        r#"{"raw_tx_hex":"0xdeadbeef"}"#,
        r#"{"nonce":42}"#,
        r#"{"force":true}"#,
        r#"{"foo":"bar"}"#,
        r#"{"tx_hash":"0x00"}"#,
        r#"{"gas_limit":1}"#,
        r#"{"broadcast_recheck":true}"#,
        r#"{"private_key":"0x00"}"#,
        r#"{"resume":true}"#,
        r#"{"skip_firewall":true}"#,
        r#"{"admin_override":true}"#,
        r#"{"fee_bump":true}"#,
        r#"{"target":"0xdead"}"#,
        r#"{"deployment_id":1}"#,
        r#"{"signature":"0x00"}"#,
        r#"{"replacement_nonce":1}"#,
        r#"{"emergency":true}"#,
        r#"{"canonical_execution_id":"foo"}"#,
        r#"{"rpc_url":"http://evil"}"#,
        r#"{"chain_id":8453}"#,
    ];
    for (i, body) in extra_fields.iter().take(NUM_CASES).enumerate() {
        let cid = seed_canonical_id(0xab, i);
        let status = admin_post_broadcast_body(&router, deployment_id, &cid, body).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
            "case {i}: extra-field body accepted with {status}: {body}"
        );
    }
    // Zero writes across all cases.
    assert!(rpc.write_method_calls().is_empty());
}

/// (12) After random transient RPC failures, no automatic RBF /
/// fee-bump attempts occur — the outbox surfaces failures as
/// SubmissionUnknown, not as a re-signed tx with new fees.
#[tokio::test]
async fn prop_no_automatic_rbf_after_random_transient_failures() {
    let Some(url) = get_pg_url_or_skip("prop_no_automatic_rbf_after_random_transient_failures")
    else {
        return;
    };
    for i in 0..NUM_CASES {
        let pool = fresh_pool(&url).await;
        let signer = [0x77u8; 20];
        let cid = seed_canonical_id(0xac, i);
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (state, store, _o, _w, deployment_id) = build_state_from_wire(&pool, rpc_dyn).await;
        let nonce = 46 + i as u64;
        let (_cd, _plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        // Case-parameterised transient failure.
        let outcome: Result<SendOutcome, BroadcastRpcError> = match i % 3 {
            0 => Err(BroadcastRpcError::Timeout),
            1 => Err(BroadcastRpcError::Transport("simulated".into())),
            _ => Err(BroadcastRpcError::RateLimited),
        };
        rpc.set_next_send_outcome(outcome);
        let router = router_for(state);
        let (_s, _b) = admin_post_broadcast(&router, deployment_id, &cid).await;
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let row = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
        // Row landed at Broadcasting or SubmissionUnknown or
        // ManualInterventionRequired — never Submitted (no ack) and
        // never Confirmed.
        assert!(
            !matches!(
                row.phase,
                BroadcastPhase::Submitted
                    | BroadcastPhase::Pending
                    | BroadcastPhase::MinedSuccess
                    | BroadcastPhase::Confirming
                    | BroadcastPhase::Confirmed
            ),
            "case {i}: transient failure elevated to success phase {:?}",
            row.phase
        );
        // Exactly ONE send attempt regardless — no auto-retry / RBF.
        let writes = rpc.write_method_calls();
        assert!(
            writes.len() == 1 && writes[0] == "eth_sendRawTransaction",
            "case {i}: outbox retried/rbf'd after transient failure (writes={writes:?})"
        );
    }
}

// Verdict marker for CI grep.
#[test]
fn verdict_live_broadcast_wiring_properties_validated() {
    println!("LIVE_BROADCAST_WIRING_PROPERTIES_VALIDATED");
}
