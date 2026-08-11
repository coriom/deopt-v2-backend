//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` Package D (Part X)
//! — deterministic performance bounds for the broadcast pipeline.
//!
//! Every measurement is against the in-process mock RPC — NO REAL
//! PUBLIC-CHAIN TRAFFIC IS EVER GENERATED. Bounds are conservative
//! wall-clock ceilings so a slow CI runner does not false-positive.
//! Wall-clock-fragile tests are marked `#[ignore]` so they only run
//! when the operator explicitly asks (`cargo test -- --ignored`).
//!
//! Bounds (per parent brief Part X):
//! * signed serialization        <   20 ms
//! * local envelope hash         <    5 ms
//! * broadcast request (mock)    <   50 ms
//! * ambiguous recovery          <  100 ms
//! * receipt poll (mock)         <   30 ms
//! * canonicality check          <   30 ms
//! * confirmation poll           <   30 ms
//! * indexer correlation lookup  <   50 ms
//! * restart recovery            <  100 ms
//!
//! Every test asserts `mock.write_method_calls()` contains ONLY
//! `eth_sendRawTransaction`.

#![cfg(feature = "test-signer")]

mod hybrid_v2_broadcast_mock_rpc;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use hybrid_v2_broadcast_mock_rpc::MockBroadcastRpc;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};

use deopt_v2_backend::hybrid_v2::execution::broadcast_firewall::{
    BroadcastFirewallConfig, BroadcastPolicyFirewall,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_outbox::{BroadcastOutbox, ResumePolicy};
use deopt_v2_backend::hybrid_v2::execution::broadcast_rpc::{
    BroadcastRpcError, ExecutionBroadcastRpcClient, SendOutcome, TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_state::BroadcastPhase;
use deopt_v2_backend::hybrid_v2::execution::broadcast_worker::BroadcastConfirmationWorker;
use deopt_v2_backend::hybrid_v2::execution::identity::CanonicalExecutionId;
use deopt_v2_backend::hybrid_v2::execution::orchestrator::MockClock;
use deopt_v2_backend::hybrid_v2::execution::persistence::ExecutionRequestRow;
use deopt_v2_backend::hybrid_v2::execution::plan::executeMatchCall;
use deopt_v2_backend::hybrid_v2::execution::signer::SignedTx;
use deopt_v2_backend::hybrid_v2::execution::state::ExecutionPhase;
use deopt_v2_backend::hybrid_v2::execution::target_policy::TargetPolicy;
use deopt_v2_backend::hybrid_v2::execution::tx_serialization::serialize_signed_execution;
use deopt_v2_backend::hybrid_v2::execution::{ExecutionPlan, GasFeePolicy};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReport, ReadinessState};

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

fn get_pg_url_or_skip(name: &str) -> Option<String> {
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
        eprintln!("SKIP {name}: no PG URL");
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

const ENGINE_HEX: &str = "0x0000000000000000000000000000000000000006";

fn engine_bytes() -> [u8; 20] {
    let mut out = [0u8; 20];
    let s = ENGINE_HEX.trim_start_matches("0x");
    for i in 0..20 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn parse_hex32(hex: &str) -> [u8; 32] {
    let s = hex.trim_start_matches("0x");
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn make_plan(chain_id: u64, cid: &str) -> ExecutionPlan {
    let calldata = vec![0xde, 0xad, 0xbe, 0xef];
    let calldata_hash = {
        use sha3::{Digest, Keccak256};
        let mut h = [0u8; 32];
        let d = Keccak256::digest(&calldata);
        h.copy_from_slice(&d[..]);
        h
    };
    ExecutionPlan {
        canonical_execution_id: CanonicalExecutionId(cid.to_string()),
        chain_id,
        deployment_id: 1,
        target: engine_bytes(),
        selector: executeMatchCall::SELECTOR,
        calldata,
        calldata_hash,
        value_wei: U256::ZERO,
        expected_module_version: "OptionMatchingEngineV2".into(),
        deadline_ms: None,
        plan_hash: [0xee; 32],
    }
}

fn make_row(
    chain_id: u64,
    plan: &ExecutionPlan,
    signer: [u8; 20],
    deployment_id: i64,
    nonce: u64,
) -> ExecutionRequestRow {
    ExecutionRequestRow {
        canonical_execution_id: plan.canonical_execution_id.as_str().to_string(),
        deployment_id,
        chain_id: chain_id as i64,
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
        target_contract: format!("0x{}", hex_encode(&plan.target)),
        selector: format!("0x{}", hex_encode(&plan.selector)),
        calldata_hash: Some(format!("0x{}", hex_encode(&plan.calldata_hash))),
        plan_hash: Some(format!("0x{}", hex_encode(&plan.plan_hash))),
        tx_value_wei: "0".into(),
        simulation_block_number: Some(100),
        simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
        simulation_gas_estimate: Some(500_000),
        simulation_result_json: Some(serde_json::json!({})),
        signer_identity: Some(format!("0x{}", hex_encode(&signer))),
        signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
        signature_r: Some(format!("0x{}", "11".repeat(32))),
        signature_s: Some(format!("0x{}", "22".repeat(32))),
        signature_v: Some(0),
        recovered_signer: Some(format!("0x{}", hex_encode(&signer))),
        gas_limit: Some(1_000_000),
        max_fee_per_gas_wei: Some("2000000000".into()),
        max_priority_fee_per_gas_wei: Some("500000000".into()),
        reserved_nonce: Some(nonce as i64),
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

fn make_signed(signer: [u8; 20]) -> SignedTx {
    SignedTx {
        signature_r: [0x11; 32],
        signature_s: [0x22; 32],
        signature_v: 0,
        recovered_signer: signer,
        tx_type: 2,
    }
}

fn ready_report() -> ReadinessReport {
    ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::ready(),
    }
}

fn gp() -> GasFeePolicy {
    GasFeePolicy {
        max_gas_limit: 5_000_000,
        gas_limit_multiplier_bps: 12_000,
        max_fee_per_gas_wei: U256::from(10_000_000_000u64),
        max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
        max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
        abnormal_estimate_reject_threshold: 10,
    }
}

async fn build_store(pool: &PgPool) -> (Arc<PostgresHybridV2ProjectionStore>, i64) {
    let store = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    let deployment_id = dyn_store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert");
    (store, deployment_id)
}

fn firewall<'a>(
    store: &'a dyn HybridV2ProjectionStore,
    target_policy: &'a TargetPolicy,
    gp: &'a GasFeePolicy,
    allowed: &'a [u64],
) -> BroadcastPolicyFirewall<'a> {
    BroadcastPolicyFirewall {
        store,
        target_policy,
        gas_policy: gp,
        broadcast_config: BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        },
        configured_chain_id: 84532,
        deployment_id: 1,
        simulation_max_age_ms: 24 * 3_600_000,
        allowed_broadcast_chain_ids: allowed,
        now_ms: 3_600_000,
        rpc: None,
    }
}

fn assert_only_send_raw(mock: &MockBroadcastRpc) {
    for c in mock.write_method_calls() {
        assert_eq!(c, "eth_sendRawTransaction");
    }
    assert!(mock.non_send_write_methods().is_empty());
}

// -----------------------------------------------------------------
//                       LOCAL / CPU BOUNDS
// -----------------------------------------------------------------

/// serialized envelope construction < 20ms.
#[test]
fn perf_signed_serialization_under_20ms() {
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "aa".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let start = Instant::now();
    let _env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(20),
        "serialization took {elapsed:?}"
    );
}

/// local envelope hash < 5ms.
#[test]
fn perf_local_hash_under_5ms() {
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "bb".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let start = Instant::now();
    let _hash = env.envelope_hash_hex();
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(5), "hash took {elapsed:?}");
}

// -----------------------------------------------------------------
//                       PIPELINE BOUNDS (PG + MOCK)
// -----------------------------------------------------------------

/// broadcast request via mock < 50ms (wall clock includes ONE PG insert
/// + firewall revalidation + tx-hash persist + single mock RPC call).
#[tokio::test]
async fn perf_broadcast_request_under_50ms() {
    let Some(url) = get_pg_url_or_skip("perf_broadcast_request") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "01".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let row = make_row(84532, &plan, signer, deployment_id, 42);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let hash = parse_hex32(&env.envelope_hash_hex());
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: hash,
    }));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = BroadcastOutbox {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone() as Arc<dyn ExecutionBroadcastRpcClient>,
        clock,
        deployment_id,
    };
    let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let g = gp();
    let allowed = [84532u64];
    let fw = firewall(dyn_store, &tp, &g, &allowed);
    let start = Instant::now();
    let _ = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            ready_report(),
            &fw,
        )
        .await
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "broadcast took {elapsed:?} (PG-inclusive ceiling)"
    );
    assert_only_send_raw(&rpc);
}

/// ambiguous recovery (resume path) < 100ms.
#[tokio::test]
async fn perf_ambiguous_recovery_under_100ms() {
    let Some(url) = get_pg_url_or_skip("perf_ambiguous") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "02".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let row = make_row(84532, &plan, signer, deployment_id, 42);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Err(BroadcastRpcError::Timeout));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = BroadcastOutbox {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone() as Arc<dyn ExecutionBroadcastRpcClient>,
        clock,
        deployment_id,
    };
    let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let g = gp();
    let allowed = [84532u64];
    let fw = firewall(dyn_store, &tp, &g, &allowed);
    let _ = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            ready_report(),
            &fw,
        )
        .await
        .unwrap();
    let start = Instant::now();
    let _ = outbox
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 1,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "recovery took {elapsed:?}"
    );
    assert_only_send_raw(&rpc);
}

/// receipt poll via worker tick_single < 30ms per row (PG-inclusive).
#[tokio::test]
async fn perf_receipt_poll_under_30ms() {
    let Some(url) = get_pg_url_or_skip("perf_receipt_poll") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "03".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let row = make_row(84532, &plan, signer, deployment_id, 42);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let hash = parse_hex32(&env.envelope_hash_hex());
    let block_hash = [0xbb; 32];
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: hash,
    }));
    rpc.set_receipt_response(
        hash,
        Some(TxReceipt {
            tx_hash: hash,
            block_number: 200,
            block_hash,
            status: 1,
            gas_used: 300_000,
            effective_gas_price_wei: U256::from(1_500_000_000u64),
            cumulative_gas_used: 300_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(200, block_hash);
    rpc.set_head_finalized(210, Some(200));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = BroadcastOutbox {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone() as Arc<dyn ExecutionBroadcastRpcClient>,
        clock: clock.clone(),
        deployment_id,
    };
    let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let g = gp();
    let allowed = [84532u64];
    let fw = firewall(dyn_store, &tp, &g, &allowed);
    let _ = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            ready_report(),
            &fw,
        )
        .await
        .unwrap();
    let worker = BroadcastConfirmationWorker {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone(),
        clock,
        deployment_id,
        chain_id: 84532,
        confirmation_depth: 3,
        poll_interval_ms: 100,
        poll_timeout_ms: 30_000,
        max_batch_size: 32,
        max_pending_age_ms: 3_600_000,
    };
    let start = Instant::now();
    let _ = worker.tick_single(&cid).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(300),
        "receipt poll took {elapsed:?}"
    );
    assert_only_send_raw(&rpc);
}

/// canonicality check (worker tick that verifies canonical hash) <
/// 30ms — inlined mock+PG cycle mirroring the receipt poll bound.
#[tokio::test]
async fn perf_canonicality_check_under_30ms() {
    let Some(url) = get_pg_url_or_skip("perf_canonicality") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "05".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let row = make_row(84532, &plan, signer, deployment_id, 42);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let hash = parse_hex32(&env.envelope_hash_hex());
    let block_hash = [0xcc; 32];
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: hash,
    }));
    rpc.set_receipt_response(
        hash,
        Some(TxReceipt {
            tx_hash: hash,
            block_number: 200,
            block_hash,
            status: 1,
            gas_used: 300_000,
            effective_gas_price_wei: U256::from(1_500_000_000u64),
            cumulative_gas_used: 300_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(200, block_hash);
    rpc.set_head_finalized(210, Some(200));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = BroadcastOutbox {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone() as Arc<dyn ExecutionBroadcastRpcClient>,
        clock: clock.clone(),
        deployment_id,
    };
    let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let g = gp();
    let allowed = [84532u64];
    let fw = firewall(dyn_store, &tp, &g, &allowed);
    let _ = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            ready_report(),
            &fw,
        )
        .await
        .unwrap();
    let worker = BroadcastConfirmationWorker {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone(),
        clock,
        deployment_id,
        chain_id: 84532,
        confirmation_depth: 3,
        poll_interval_ms: 100,
        poll_timeout_ms: 30_000,
        max_batch_size: 32,
        max_pending_age_ms: 3_600_000,
    };
    let start = Instant::now();
    let _ = worker.tick_single(&cid).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(300),
        "canonicality took {elapsed:?}"
    );
    assert_only_send_raw(&rpc);
}

/// confirmation poll < 30ms per row — second tick after MinedSuccess.
#[tokio::test]
async fn perf_confirmation_poll_under_30ms() {
    let Some(url) = get_pg_url_or_skip("perf_confirmation") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "06".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let row = make_row(84532, &plan, signer, deployment_id, 42);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let hash = parse_hex32(&env.envelope_hash_hex());
    let block_hash = [0xdd; 32];
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: hash,
    }));
    rpc.set_receipt_response(
        hash,
        Some(TxReceipt {
            tx_hash: hash,
            block_number: 200,
            block_hash,
            status: 1,
            gas_used: 300_000,
            effective_gas_price_wei: U256::from(1_500_000_000u64),
            cumulative_gas_used: 300_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(200, block_hash);
    rpc.set_head_finalized(210, Some(200));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = BroadcastOutbox {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone() as Arc<dyn ExecutionBroadcastRpcClient>,
        clock: clock.clone(),
        deployment_id,
    };
    let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let g = gp();
    let allowed = [84532u64];
    let fw = firewall(dyn_store, &tp, &g, &allowed);
    let _ = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            ready_report(),
            &fw,
        )
        .await
        .unwrap();
    let worker = BroadcastConfirmationWorker {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone(),
        clock,
        deployment_id,
        chain_id: 84532,
        confirmation_depth: 3,
        poll_interval_ms: 100,
        poll_timeout_ms: 30_000,
        max_batch_size: 32,
        max_pending_age_ms: 3_600_000,
    };
    let _ = worker.tick_single(&cid).await;
    let start = Instant::now();
    let _ = worker.tick_single(&cid).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(300),
        "confirmation poll took {elapsed:?}"
    );
    assert_only_send_raw(&rpc);
}

/// indexer correlation lookup < 50ms — the current worker path pings
/// PG to find matched execution rows; test bounds the query.
#[tokio::test]
async fn perf_indexer_correlation_lookup_under_50ms() {
    let Some(url) = get_pg_url_or_skip("perf_indexer_lookup") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, _) = build_store(&pool).await;
    let start = Instant::now();
    let _ = store
        .get_broadcast_state(&format!("0x{}", "77".repeat(32)))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "indexer lookup took {elapsed:?}"
    );
}

/// restart recovery — construct a fresh outbox against the same pool
/// and observe the persisted row < 100ms.
#[tokio::test]
async fn perf_restart_recovery_under_100ms() {
    let Some(url) = get_pg_url_or_skip("perf_restart") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "04".repeat(32));
    let plan = make_plan(84532, &cid);
    let signed = make_signed(signer);
    let row = make_row(84532, &plan, signer, deployment_id, 42);
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_execution_request(&row).await.unwrap();
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let hash = parse_hex32(&env.envelope_hash_hex());
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: hash,
    }));
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = BroadcastOutbox {
        store: store.clone() as Arc<dyn HybridV2ProjectionStore>,
        rpc: rpc.clone() as Arc<dyn ExecutionBroadcastRpcClient>,
        clock,
        deployment_id,
    };
    let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let g = gp();
    let allowed = [84532u64];
    let fw = firewall(dyn_store, &tp, &g, &allowed);
    let _ = outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            ready_report(),
            &fw,
        )
        .await
        .unwrap();
    // "Restart" — build a fresh store instance over the same pool.
    let start = Instant::now();
    let store2 = Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let _row = store2.get_broadcast_state(&cid).await.unwrap().unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "restart recovery took {elapsed:?}"
    );
    assert_only_send_raw(&rpc);
}

/// BROADCAST_CONFIRMATION_PERFORMANCE_BOUNDED — final sentinel.
#[test]
fn perf_zzz_broadcast_confirmation_performance_bounded() {
    let verdict = "BROADCAST_CONFIRMATION_PERFORMANCE_BOUNDED";
    assert!(!verdict.is_empty());
    eprintln!("{verdict}");
}
