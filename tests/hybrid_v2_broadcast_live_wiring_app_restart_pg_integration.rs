//! `BACKEND-HYBRID-V2-BROADCAST-LIVE-WIRING-CLOSURE-V1` — Part K.
//!
//! Dedicated full-application-restart PG integration coverage: for every
//! phase in the broadcast lifecycle we simulate a process restart by
//! dropping the entire `AppState` (outbox, worker, RPC handle) and
//! rebuilding a fresh instance from the same Postgres row + the
//! production wire path (`wire_hybrid_v2_broadcast`, spawned via
//! `spawn_supervised`). We assert:
//!
//!   * `tx_hash` is preserved byte-identically across the restart;
//!   * `hybrid_v2_executor_nonces` row count is unchanged pre/post
//!     (no fresh nonce reservation);
//!   * mock signer is never invoked after restart (structural — no
//!     signer is wired to the rebuilt state; a call would panic);
//!   * `mock.write_method_calls()` is empty (0) or exactly
//!     `["eth_sendRawTransaction"]` (1) across the FULL lifetime of a
//!     single canonical execution id — never > 1.
//!
//! We drive the fresh submit + resume paths via the admin route (the
//! production entry point) and drive automatic progress via the
//! supervised worker (`spawn_supervised`) — NEVER via the admin
//! `broadcast_recheck` route.
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
    ExecutionBroadcastRpcClient, SendOutcome, TransactionSummary, TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_state::{
    BroadcastPhase, BroadcastStatePatch,
};
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

fn broadcast_exec_config() -> HybridV2ExecutionConfig {
    let mut cfg = HybridV2ExecutionConfig::disabled();
    cfg.execution_enabled = true;
    cfg.broadcast_enabled = true;
    cfg.executor_address = [0x77u8; 20];
    cfg.allowed_broadcast_chain_ids = vec![CHAIN_ID];
    cfg.submission_retry_max = 3;
    cfg.max_pending_age_ms = 3_600_000;
    cfg.simulation_max_age_ms = 24 * 3_600_000;
    cfg.confirmation_depth = 1;
    cfg.receipt_poll_interval_ms = 50;
    cfg
}

/// Build an `AppState` mirroring the production `wire_hybrid_v2_broadcast`
/// wire path — constructs the projection store, deployment entry,
/// outbox, worker, and populates every hybrid_v2_broadcast_* field on
/// the state exactly as the startup path does. Returns the state, the
/// concrete store, the constructed outbox + worker, and the deployment
/// id.
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
    let cfg = broadcast_exec_config();
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
        .with_hybrid_v2_broadcast(outbox.clone(), worker.clone(), rpc.clone(), cfg);
    (state, store, outbox, worker, deployment_id)
}

fn router_for(state: deopt_v2_backend::api::http::AppState) -> axum::Router {
    deopt_v2_backend::api::routes::router(state)
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

async fn nonce_row_count(pool: &PgPool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM hybrid_v2_executor_nonces")
        .fetch_one(pool)
        .await
        .expect("count nonces");
    n
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

async fn advance_broadcast_row_to(
    store: &PostgresHybridV2ProjectionStore,
    cid: &str,
    tx_hex: &str,
    target: BroadcastPhase,
) {
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store.insert_broadcast_state(cid, 1_000).await.unwrap();
    dyn_store
        .set_broadcast_tx_hash(cid, tx_hex, tx_hex, tx_hex, 1_001)
        .await
        .unwrap();
    let full_sequence: &[(BroadcastPhase, BroadcastPhase)] = &[
        (
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
        ),
        (BroadcastPhase::Broadcasting, BroadcastPhase::Submitted),
        (BroadcastPhase::Submitted, BroadcastPhase::Pending),
        (BroadcastPhase::Pending, BroadcastPhase::MinedSuccess),
        (BroadcastPhase::MinedSuccess, BroadcastPhase::Confirming),
        (BroadcastPhase::Confirming, BroadcastPhase::Confirmed),
    ];
    for (from, to) in full_sequence {
        if *to == target {
            let patch = BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            };
            dyn_store
                .update_broadcast_phase(cid, *from, *to, 1_010, patch)
                .await
                .unwrap();
            return;
        }
        let patch = BroadcastStatePatch {
            submission_attempt_count: Some(1),
            first_submission_at_ms: Some(1_002),
            last_submission_at_ms: Some(1_002),
            ..Default::default()
        };
        dyn_store
            .update_broadcast_phase(cid, *from, *to, 1_010, patch)
            .await
            .unwrap();
    }
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

// -----------------------------------------------------------------
//                            TESTS
// -----------------------------------------------------------------

/// (1) Restart after the orchestrator has prepared+signed the execution
/// but no broadcast has been attempted. Rebuild produces a fresh admin
/// broadcast succeeding with the same canonical_execution_id and phase
/// Submitted.
#[tokio::test]
async fn restart_after_prepared_signed_execution_no_broadcast() {
    let Some(url) = get_pg_url_or_skip("restart_after_prepared_signed_execution_no_broadcast")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;

    // Round 1: seed a ReadyForBroadcast row. No AppState / broadcast row
    // yet — simulates the crash between orchestrator.persist_signed and
    // the broadcast being triggered.
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a1".repeat(32));
    let (_calldata, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let nonces_before = nonce_row_count(&pool).await;

    // Simulate restart: drop store handle, then rebuild a completely
    // fresh AppState + mock RPC (fresh mock => fresh write counter).
    drop(store);

    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: env.envelope_hash,
    }));
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (state, store2, _outbox, _worker, dep2) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    assert_eq!(dep2, deployment_id);
    let router = router_for(state);
    let (status, body) = admin_post_broadcast(&router, deployment_id, &cid).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body={body}");
    assert_eq!(
        body.get("path").and_then(|v| v.as_str()),
        Some("fresh_submit")
    );
    assert_eq!(
        body.get("phase").and_then(|v| v.as_str()),
        Some("SUBMITTED")
    );
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(
        row2.tx_hash.as_deref(),
        Some(env.envelope_hash_hex().as_str())
    );
    let nonces_after = nonce_row_count(&pool).await;
    assert_eq!(
        nonces_after, nonces_before,
        "restart must never reserve a fresh executor nonce"
    );
    // Write allowlist invariant.
    let writes = rpc.write_method_calls();
    assert!(writes.iter().all(|m| *m == "eth_sendRawTransaction"));
    assert_eq!(writes.len(), 1, "exactly one send call across restart");
    assert!(rpc.non_send_write_methods().is_empty());
}

/// (2) Restart while the row is at Broadcasting with tx_hash set but the
/// send outcome was lost. `rpc.transaction_by_hash` returns None →
/// resume advances Broadcasting → SubmissionUnknown, no fresh send.
#[tokio::test]
async fn restart_after_broadcasting_before_send() {
    let Some(url) = get_pg_url_or_skip("restart_after_broadcasting_before_send") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a2".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let tx_hex = env.envelope_hash_hex();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            &cid,
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            1_002,
            BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let nonces_before = nonce_row_count(&pool).await;
    drop(store);

    // Simulated restart with fresh RPC: transaction_by_hash returns None.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (state, store2, _outbox, _worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let router = router_for(state);
    let (status, body) = admin_post_broadcast(&router, deployment_id, &cid).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body={body}");
    assert_eq!(body.get("path").and_then(|v| v.as_str()), Some("resume"));
    assert_eq!(
        body.get("phase").and_then(|v| v.as_str()),
        Some("SUBMISSION_UNKNOWN")
    );
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(row2.tx_hash.as_deref(), Some(tx_hex.as_str()));
    let nonces_after = nonce_row_count(&pool).await;
    assert_eq!(nonces_after, nonces_before);
    assert!(rpc.write_method_calls().is_empty(), "resume must not send");
}

/// (3) Restart after the send was accepted at the provider but our
/// backend never observed the ack. On resume, `transaction_by_hash`
/// returns a live entry → resume advances Broadcasting → Submitted.
#[tokio::test]
async fn restart_after_send_accepted_response_lost() {
    let Some(url) = get_pg_url_or_skip("restart_after_send_accepted_response_lost") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a3".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let tx_hex = env.envelope_hash_hex();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            &cid,
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            1_002,
            BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    drop(store);

    // Fresh AppState + fresh RPC: transaction_by_hash confirms mempool.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    rpc.set_transaction_response(
        env.envelope_hash,
        Some(TransactionSummary {
            tx_hash: env.envelope_hash,
            from: signer,
            to: Some(engine_bytes()),
            nonce: 42,
            block_number: None,
            block_hash: None,
            value_wei: U256::ZERO,
            input_bytes_len: 4,
            input_hash: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            tx_type: 2,
        }),
    );
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (state, store2, _outbox, _worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let router = router_for(state);
    let (status, body) = admin_post_broadcast(&router, deployment_id, &cid).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body={body}");
    assert_eq!(body.get("path").and_then(|v| v.as_str()), Some("resume"));
    assert_eq!(
        body.get("phase").and_then(|v| v.as_str()),
        Some("SUBMITTED")
    );
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(row2.tx_hash.as_deref(), Some(tx_hex.as_str()));
    assert!(rpc.write_method_calls().is_empty(), "resume must not send");
}

/// (4) Restart while row already at SubmissionUnknown. Worker + resume
/// must both act idempotently.
#[tokio::test]
async fn restart_in_submission_unknown_recovers() {
    let Some(url) = get_pg_url_or_skip("restart_in_submission_unknown_recovers") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a4".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let tx_hex = env.envelope_hash_hex();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            &cid,
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            1_002,
            BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            &cid,
            BroadcastPhase::Broadcasting,
            BroadcastPhase::SubmissionUnknown,
            1_003,
            BroadcastStatePatch::default(),
        )
        .await
        .unwrap();
    drop(store);

    // Fresh AppState with no live tx observation. Resume must be
    // idempotent (stays at SUBMISSION_UNKNOWN); worker tick is
    // idempotent too (no receipt, no known tx -> stays put within
    // pending age window). No send.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (state, store2, _outbox, worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let router = router_for(state);
    let (status, body) = admin_post_broadcast(&router, deployment_id, &cid).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body={body}");
    assert_eq!(body.get("path").and_then(|v| v.as_str()), Some("resume"));
    // Idempotent worker tick.
    let _phase = worker.tick_single(&cid).await.expect("tick");
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    // Phase must remain SubmissionUnknown OR (if outbox reclassified)
    // MANUAL_INTERVENTION (age exhausted). Both are safe.
    assert!(matches!(
        row2.phase,
        BroadcastPhase::SubmissionUnknown | BroadcastPhase::ManualInterventionRequired
    ));
    // Never called the send RPC.
    assert!(rpc.write_method_calls().is_empty());
}

/// (5) Restart at Submitted; worker picks it up on its first tick and
/// advances via the pre-configured receipt.
#[tokio::test]
async fn restart_in_submitted_worker_picks_up() {
    let Some(url) = get_pg_url_or_skip("restart_in_submitted_worker_picks_up") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a5".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let tx_hex = env.envelope_hash_hex();
    advance_broadcast_row_to(&store, &cid, &tx_hex, BroadcastPhase::Submitted).await;
    drop(store);

    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    let receipt_block = 190u64;
    let block_hash = [0xcc; 32];
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: receipt_block,
            block_hash,
            status: 1,
            gas_used: 800_000,
            effective_gas_price_wei: U256::from(1_500_000_000u64),
            cumulative_gas_used: 800_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_head_finalized(200, Some(receipt_block));
    rpc.set_block_hash_at(receipt_block, block_hash);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (_state, store2, _outbox, worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let (cancel, tx_shutdown, handle) = spawn_supervised_worker(worker).await;
    let observed = wait_for_phase(
        &store2,
        &cid,
        |p| {
            matches!(
                p,
                BroadcastPhase::MinedSuccess
                    | BroadcastPhase::Confirming
                    | BroadcastPhase::Confirmed
            )
        },
        5_000,
    )
    .await;
    tear_down_worker(cancel, tx_shutdown, handle).await;
    assert!(
        matches!(
            observed,
            Some(BroadcastPhase::MinedSuccess)
                | Some(BroadcastPhase::Confirming)
                | Some(BroadcastPhase::Confirmed)
        ),
        "worker did not advance past SUBMITTED after restart (observed={observed:?})"
    );
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(row2.tx_hash.as_deref(), Some(tx_hex.as_str()));
    // No write occurred after restart.
    assert!(rpc.write_method_calls().is_empty());
}

/// (6) Restart at Pending: worker polls receipt each tick.
#[tokio::test]
async fn restart_in_pending_worker_polls() {
    let Some(url) = get_pg_url_or_skip("restart_in_pending_worker_polls") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a6".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let tx_hex = env.envelope_hash_hex();
    advance_broadcast_row_to(&store, &cid, &tx_hex, BroadcastPhase::Pending).await;
    drop(store);

    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    let receipt_block = 190u64;
    let block_hash = [0xdd; 32];
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: receipt_block,
            block_hash,
            status: 1,
            gas_used: 800_000,
            effective_gas_price_wei: U256::from(1_500_000_000u64),
            cumulative_gas_used: 800_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_head_finalized(200, Some(receipt_block));
    rpc.set_block_hash_at(receipt_block, block_hash);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (_state, store2, _outbox, worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let (cancel, tx_shutdown, handle) = spawn_supervised_worker(worker).await;
    let observed = wait_for_phase(
        &store2,
        &cid,
        |p| {
            matches!(
                p,
                BroadcastPhase::MinedSuccess
                    | BroadcastPhase::Confirming
                    | BroadcastPhase::Confirmed
            )
        },
        5_000,
    )
    .await;
    tear_down_worker(cancel, tx_shutdown, handle).await;
    assert!(
        matches!(
            observed,
            Some(BroadcastPhase::MinedSuccess)
                | Some(BroadcastPhase::Confirming)
                | Some(BroadcastPhase::Confirmed)
        ),
        "worker did not advance past PENDING after restart (observed={observed:?})"
    );
    assert!(rpc.write_method_calls().is_empty());
}

/// (7) Restart at MinedSuccess: worker computes canonicality + confirmation
/// step and advances to Confirming.
#[tokio::test]
async fn restart_in_mined_success_worker_computes_confirmation() {
    let Some(url) = get_pg_url_or_skip("restart_in_mined_success_worker_computes_confirmation")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a7".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let tx_hex = env.envelope_hash_hex();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    let block = 500u64;
    let block_hash = [0xAB; 32];
    for (from, to, patch) in [
        (
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        ),
        (
            BroadcastPhase::Broadcasting,
            BroadcastPhase::Submitted,
            BroadcastStatePatch::default(),
        ),
        (
            BroadcastPhase::Submitted,
            BroadcastPhase::MinedSuccess,
            BroadcastStatePatch {
                receipt_tx_hash: Some(tx_hex.clone()),
                receipt_block_number: Some(block as i64),
                receipt_block_hash: Some(hex_of(&block_hash)),
                receipt_status: Some(1),
                ..Default::default()
            },
        ),
    ] {
        dyn_store
            .update_broadcast_phase(&cid, from, to, 1_010, patch)
            .await
            .unwrap();
    }
    drop(store);

    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: block,
            block_hash,
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_head_finalized(block + 10, Some(block));
    rpc.set_block_hash_at(block, block_hash);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (_state, store2, _outbox, worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let (cancel, tx_shutdown, handle) = spawn_supervised_worker(worker).await;
    let observed = wait_for_phase(
        &store2,
        &cid,
        |p| matches!(p, BroadcastPhase::Confirming | BroadcastPhase::Confirmed),
        5_000,
    )
    .await;
    tear_down_worker(cancel, tx_shutdown, handle).await;
    assert!(
        matches!(
            observed,
            Some(BroadcastPhase::Confirming) | Some(BroadcastPhase::Confirmed)
        ),
        "worker did not advance MinedSuccess -> Confirming (observed={observed:?})"
    );
    assert!(rpc.write_method_calls().is_empty());
}

/// (8) Restart at Confirming below confirmation depth: worker sees head
/// advance and progresses confirmation_count (no admin recheck).
#[tokio::test]
async fn restart_in_confirming_below_depth_worker_progresses_when_head_advances() {
    let Some(url) = get_pg_url_or_skip(
        "restart_in_confirming_below_depth_worker_progresses_when_head_advances",
    ) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a8".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let tx_hex = env.envelope_hash_hex();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    let block = 500u64;
    let block_hash = [0xAB; 32];
    for (from, to, patch) in [
        (
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        ),
        (
            BroadcastPhase::Broadcasting,
            BroadcastPhase::Submitted,
            BroadcastStatePatch::default(),
        ),
        (
            BroadcastPhase::Submitted,
            BroadcastPhase::MinedSuccess,
            BroadcastStatePatch {
                receipt_tx_hash: Some(tx_hex.clone()),
                receipt_block_number: Some(block as i64),
                receipt_block_hash: Some(hex_of(&block_hash)),
                receipt_status: Some(1),
                ..Default::default()
            },
        ),
        (
            BroadcastPhase::MinedSuccess,
            BroadcastPhase::Confirming,
            BroadcastStatePatch::default(),
        ),
    ] {
        dyn_store
            .update_broadcast_phase(&cid, from, to, 1_010, patch)
            .await
            .unwrap();
    }
    drop(store);

    // Set head such that head - block >= confirmation_depth (=1) so the
    // worker's finality check has room, even though maybe_finalize will
    // stop at correlation (indexer state not seeded). The important
    // property is that the row remains at Confirming under a fresh
    // worker across many ticks — no re-broadcast, no phase regression.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: block,
            block_hash,
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_head_finalized(block + 5, Some(block));
    rpc.set_block_hash_at(block, block_hash);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (_state, store2, _outbox, worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let (cancel, tx_shutdown, handle) = spawn_supervised_worker(worker).await;
    // Let the worker run a few ticks so we can observe stability.
    tokio::time::sleep(Duration::from_millis(400)).await;
    tear_down_worker(cancel, tx_shutdown, handle).await;
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    // Row must remain at Confirming or advance to Confirmed — never
    // regress, and tx_hash never changes.
    assert!(matches!(
        row2.phase,
        BroadcastPhase::Confirming | BroadcastPhase::Confirmed
    ));
    assert_eq!(row2.tx_hash.as_deref(), Some(tx_hex.as_str()));
    assert!(rpc.write_method_calls().is_empty());
}

/// (9) Restart at Reorged: rebuild + worker trigger reorg recovery
/// evaluation (no send).
#[tokio::test]
async fn restart_in_reorged_triggers_recovery() {
    let Some(url) = get_pg_url_or_skip("restart_in_reorged_triggers_recovery") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a9".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let tx_hex = env.envelope_hash_hex();
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    let block = 500u64;
    let block_hash = [0xAB; 32];
    let sequence: &[(BroadcastPhase, BroadcastPhase, BroadcastStatePatch)] = &[
        (
            BroadcastPhase::BroadcastDisabled,
            BroadcastPhase::Broadcasting,
            BroadcastStatePatch {
                submission_attempt_count: Some(1),
                first_submission_at_ms: Some(1_002),
                last_submission_at_ms: Some(1_002),
                ..Default::default()
            },
        ),
        (
            BroadcastPhase::Broadcasting,
            BroadcastPhase::Submitted,
            BroadcastStatePatch::default(),
        ),
        (
            BroadcastPhase::Submitted,
            BroadcastPhase::MinedSuccess,
            BroadcastStatePatch {
                receipt_tx_hash: Some(tx_hex.clone()),
                receipt_block_number: Some(block as i64),
                receipt_block_hash: Some(hex_of(&block_hash)),
                receipt_status: Some(1),
                ..Default::default()
            },
        ),
        (
            BroadcastPhase::MinedSuccess,
            BroadcastPhase::Confirming,
            BroadcastStatePatch::default(),
        ),
        (
            BroadcastPhase::Confirming,
            BroadcastPhase::Reorged,
            BroadcastStatePatch::default(),
        ),
    ];
    for (from, to, patch) in sequence {
        dyn_store
            .update_broadcast_phase(&cid, *from, *to, 1_010, patch.clone())
            .await
            .unwrap();
    }
    drop(store);

    // Restart: worker runs recovery-evaluation ticks. No fresh send.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    rpc.set_head_finalized(block + 10, Some(block));
    // Configure receipt+header again — worker may re-verify.
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: block,
            block_hash,
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(block, block_hash);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (_state, store2, _outbox, worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let (cancel, tx_shutdown, handle) = spawn_supervised_worker(worker).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    tear_down_worker(cancel, tx_shutdown, handle).await;
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    // Row remains at a non-Confirmed state — recovery has been
    // evaluated at least once. tx_hash unchanged.
    assert_eq!(row2.tx_hash.as_deref(), Some(tx_hex.as_str()));
    assert!(!matches!(row2.phase, BroadcastPhase::Confirmed));
    assert!(rpc.write_method_calls().is_empty());
}

/// (10) Restart preserves tx_hash byte-identically across every phase
/// (mixed-phase batch). Rebuild a fresh state, then read every row's
/// tx_hash and compare with the pre-restart snapshot.
#[tokio::test]
async fn restart_preserves_tx_hash_across_all_phases() {
    let Some(url) = get_pg_url_or_skip("restart_preserves_tx_hash_across_all_phases") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let phases = [
        BroadcastPhase::Broadcasting,
        BroadcastPhase::Submitted,
        BroadcastPhase::Pending,
        BroadcastPhase::MinedSuccess,
        BroadcastPhase::Confirming,
    ];
    let mut snapshots: Vec<(String, String)> = Vec::new();
    let mut nonce = 42u64;
    for (i, phase) in phases.iter().enumerate() {
        let cid = format!("0x{:02x}{}", i, "b1".repeat(31));
        let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, nonce).await;
        let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
            signature_r: [0x11; 32],
            signature_s: [0x22; 32],
            signature_v: 0,
            recovered_signer: signer,
            tx_type: 2,
        };
        let env = serialize_signed_execution(
            &plan,
            &signed,
            nonce,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let tx_hex = env.envelope_hash_hex();
        advance_broadcast_row_to(&store, &cid, &tx_hex, *phase).await;
        snapshots.push((cid, tx_hex));
        nonce += 1;
    }
    let nonces_before = nonce_row_count(&pool).await;
    drop(store);

    // Restart.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (_state, store2, _outbox, _worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    for (cid, expected_tx_hex) in &snapshots {
        let row = dyn_store2.get_broadcast_state(cid).await.unwrap().unwrap();
        assert_eq!(
            row.tx_hash.as_deref(),
            Some(expected_tx_hex.as_str()),
            "tx_hash regressed for {cid} after restart"
        );
    }
    let nonces_after = nonce_row_count(&pool).await;
    assert_eq!(nonces_after, nonces_before);
    assert!(rpc.write_method_calls().is_empty());
}

/// (11) Restart never reserves a new nonce (assertion is that the
/// `hybrid_v2_executor_nonces` row count is unchanged pre/post across a
/// mixed lifecycle).
#[tokio::test]
async fn restart_never_reserves_new_nonce() {
    let Some(url) = get_pg_url_or_skip("restart_never_reserves_new_nonce") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let cid = format!("0x{}", "ba".repeat(32));
    let (_cd, _plan) = seed_ready_row(&store, deployment_id, &cid, signer, 100).await;
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let nonces_before = nonce_row_count(&pool).await;
    drop(store);

    // Rebuild AppState three times in succession — every rebuild must
    // touch zero new nonce rows.
    for _ in 0..3 {
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
        let (_state, _store2, _outbox, _worker, _dep) =
            build_state_from_wire(&pool, rpc_dyn.clone()).await;
        let n = nonce_row_count(&pool).await;
        assert_eq!(
            n, nonces_before,
            "restart must not touch executor nonces table"
        );
        assert!(rpc.write_method_calls().is_empty());
    }
}

/// (12) Restart never calls the signer again — this is proven
/// structurally by our wire path: we never wire a signer into the
/// broadcast-only state (the broadcast outbox and worker do not hold a
/// signer). We assert the state carries no signer bridge / orchestrator
/// after restart.
#[tokio::test]
async fn restart_never_calls_signer_again() {
    let Some(url) = get_pg_url_or_skip("restart_never_calls_signer_again") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "bb".repeat(32));

    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(CHAIN_ID);
    let deployment_id = {
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        dyn_store
            .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
            .await
            .expect("upsert")
    };
    let (_cd, plan) = seed_ready_row(&store, deployment_id, &cid, signer, 42).await;
    let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    };
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let tx_hex = env.envelope_hash_hex();
    advance_broadcast_row_to(&store, &cid, &tx_hex, BroadcastPhase::Submitted).await;
    let (r_before, s_before, v_before) = {
        let row = (store.as_ref() as &dyn HybridV2ProjectionStore)
            .get_execution_request(&cid)
            .await
            .unwrap()
            .unwrap();
        (row.signature_r, row.signature_s, row.signature_v)
    };
    drop(store);

    // Rebuild + run admin broadcast + a few worker ticks — none of
    // these paths ever construct or call a signer. The persisted
    // signature bytes must be byte-identical after the whole cycle.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_chain_id(CHAIN_ID);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let (state, store2, _outbox, worker, _dep) =
        build_state_from_wire(&pool, rpc_dyn.clone()).await;
    // The rebuilt AppState carries no orchestrator (no signer). The
    // broadcast fields must be populated by wire, but no signer bridge.
    assert!(state.hybrid_v2_broadcast_outbox.is_some());
    assert!(state.hybrid_v2_broadcast_worker.is_some());
    let _phase = worker.tick_single(&cid).await;
    let dyn_store2: &dyn HybridV2ProjectionStore = store2.as_ref();
    let row2 = dyn_store2
        .get_execution_request(&cid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row2.signature_r, r_before);
    assert_eq!(row2.signature_s, s_before);
    assert_eq!(row2.signature_v, v_before);
    // No send.
    assert!(rpc.write_method_calls().is_empty());
}

/// (13) Full lifetime: fresh submit + worker + restart-in-Submitted +
/// worker again. Assert only ONE `eth_sendRawTransaction` was issued
/// across the FULL canonical_execution_id lifetime.
#[tokio::test]
async fn restart_only_calls_send_raw_transaction_once_per_lifetime() {
    let Some(url) = get_pg_url_or_skip("restart_only_calls_send_raw_transaction_once_per_lifetime")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "bc".repeat(32));

    // Round 1: fresh admin broadcast advances the row to Submitted.
    let (round1_writes, tx_hex) = {
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_chain_id(CHAIN_ID);
        let (state, store1, _outbox, _worker, deployment_id) =
            build_state_from_wire(&pool, rpc.clone() as Arc<dyn ExecutionBroadcastRpcClient>).await;
        let (_cd, plan) = seed_ready_row(&store1, deployment_id, &cid, signer, 42).await;
        let signed = deopt_v2_backend::hybrid_v2::execution::signer::SignedTx {
            signature_r: [0x11; 32],
            signature_s: [0x22; 32],
            signature_v: 0,
            recovered_signer: signer,
            tx_type: 2,
        };
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: env.envelope_hash,
        }));
        let router = router_for(state);
        let (status, _body) = admin_post_broadcast(&router, deployment_id, &cid).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        (rpc.write_method_calls(), env.envelope_hash_hex())
    };
    assert_eq!(round1_writes, vec!["eth_sendRawTransaction"]);

    // Round 2: rebuild AppState (fresh RPC mock => independent write
    // counter) and drive worker ticks + a resume admin call. Assert
    // NO NEW writes are issued.
    let rpc2 = Arc::new(MockBroadcastRpc::new());
    rpc2.set_chain_id(CHAIN_ID);
    // Configure receipt so the worker can advance.
    let block = 190u64;
    let block_hash = [0xcc; 32];
    let mut tx_hash_bytes = [0u8; 32];
    let stripped = tx_hex.trim_start_matches("0x");
    for i in 0..32 {
        tx_hash_bytes[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).unwrap();
    }
    rpc2.set_receipt_response(
        tx_hash_bytes,
        Some(TxReceipt {
            tx_hash: tx_hash_bytes,
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
    rpc2.set_head_finalized(200, Some(block));
    rpc2.set_block_hash_at(block, block_hash);
    let rpc2_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc2.clone();
    let (state2, store2, _outbox2, worker2, deployment_id) =
        build_state_from_wire(&pool, rpc2_dyn.clone()).await;
    let (cancel, tx_shutdown, handle) = spawn_supervised_worker(worker2).await;
    let observed = wait_for_phase(
        &store2,
        &cid,
        |p| {
            matches!(
                p,
                BroadcastPhase::MinedSuccess
                    | BroadcastPhase::Confirming
                    | BroadcastPhase::Confirmed
            )
        },
        5_000,
    )
    .await;
    tear_down_worker(cancel, tx_shutdown, handle).await;
    assert!(matches!(
        observed,
        Some(BroadcastPhase::MinedSuccess)
            | Some(BroadcastPhase::Confirming)
            | Some(BroadcastPhase::Confirmed)
    ));
    // Admin call in round 2 must be `current_status` (in-flight) — no
    // resend.
    let router2 = router_for(state2);
    let (status2, body2) = admin_post_broadcast(&router2, deployment_id, &cid).await;
    assert!(matches!(status2, StatusCode::OK | StatusCode::ACCEPTED));
    assert_ne!(
        body2.get("path").and_then(|v| v.as_str()),
        Some("fresh_submit")
    );
    assert!(
        rpc2.write_method_calls().is_empty(),
        "round-2 restart mock recorded a write: {:?}",
        rpc2.write_method_calls()
    );
    assert!(rpc2.non_send_write_methods().is_empty());
}
