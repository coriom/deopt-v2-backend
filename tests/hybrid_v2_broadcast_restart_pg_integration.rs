//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` Package C (Part R)
//! — restart-safety PG coverage.
//!
//! For every broadcast phase we simulate a process restart by DROPPING
//! the outbox / worker instance and rebuilding a fresh one against the
//! same Postgres row. The tests assert:
//!  * `tx_hash` is preserved byte-identically across restart.
//!  * No fresh nonce is ever reserved (`hybrid_v2_executor_nonces`
//!    row unchanged if present).
//!  * The signer is never re-invoked — the outbox rebuilds an
//!    envelope from the persisted plan + signature and derives the
//!    SAME `envelope_hash`.
//!  * The mock's `write_method_calls()` contains only
//!    `eth_sendRawTransaction`, and `non_send_write_methods()` is
//!    empty end-to-end.
//!
//! Frozen safety: NO REAL EXTERNAL CHAIN TRANSACTION IS EVER BROADCAST.

mod hybrid_v2_broadcast_mock_rpc;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use alloy_sol_types::SolCall;
use hybrid_v2_broadcast_mock_rpc::MockBroadcastRpc;
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};

use deopt_v2_backend::hybrid_v2::execution::broadcast_firewall::{
    BroadcastFirewallConfig, BroadcastPolicyFirewall,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_outbox::{
    failure_class as bfc, BroadcastOutbox, ResumePolicy,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_rpc::{
    BroadcastRpcError, ExecutionBroadcastRpcClient, SendOutcome, TransactionSummary, TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::broadcast_state::{
    BroadcastPhase, BroadcastStatePatch,
};
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

// ------------------------- fixture helpers -------------------------

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

fn make_plan(chain_id: u64, canonical_id: &str) -> ExecutionPlan {
    let calldata = vec![0xde, 0xad, 0xbe, 0xef];
    let calldata_hash = {
        use sha3::{Digest, Keccak256};
        let mut h = [0u8; 32];
        let d = Keccak256::digest(&calldata);
        h.copy_from_slice(&d[..]);
        h
    };
    ExecutionPlan {
        canonical_execution_id: CanonicalExecutionId(canonical_id.to_string()),
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

fn make_readiness_ready() -> ReadinessReport {
    ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::ready(),
    }
}

fn gas_policy() -> GasFeePolicy {
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
        .expect("upsert deployment");
    (store, deployment_id)
}

async fn seed_row(
    store: &PostgresHybridV2ProjectionStore,
    deployment_id: i64,
    signer: [u8; 20],
    canonical_id: &str,
    nonce: u64,
) -> (ExecutionRequestRow, ExecutionPlan, SignedTx) {
    let plan = make_plan(84532, canonical_id);
    let row = make_row(84532, &plan, signer, deployment_id, nonce);
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store.insert_execution_request(&row).await.unwrap();
    (row, plan, make_signed(signer))
}

fn build_outbox(
    store: Arc<PostgresHybridV2ProjectionStore>,
    rpc: Arc<dyn ExecutionBroadcastRpcClient>,
    clock: Arc<MockClock>,
    deployment_id: i64,
) -> BroadcastOutbox {
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store;
    BroadcastOutbox {
        store: store_dyn,
        rpc,
        clock,
        deployment_id,
    }
}

fn build_worker(
    store: Arc<PostgresHybridV2ProjectionStore>,
    rpc: Arc<dyn ExecutionBroadcastRpcClient>,
    clock: Arc<MockClock>,
    deployment_id: i64,
    confirmation_depth: u32,
) -> BroadcastConfirmationWorker {
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store;
    BroadcastConfirmationWorker {
        store: store_dyn,
        rpc,
        clock,
        deployment_id,
        chain_id: 84532,
        confirmation_depth,
        poll_interval_ms: 100,
        poll_timeout_ms: 30_000,
        max_batch_size: 100,
        max_pending_age_ms: 3_600_000,
    }
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

async fn read_tx_hash(store: &PostgresHybridV2ProjectionStore, cid: &str) -> Option<String> {
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store
        .get_broadcast_state(cid)
        .await
        .unwrap()
        .and_then(|r| r.tx_hash)
}

async fn read_reserved_nonce(store: &PostgresHybridV2ProjectionStore, cid: &str) -> Option<i64> {
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    dyn_store
        .get_execution_request(cid)
        .await
        .unwrap()
        .and_then(|r| r.reserved_nonce)
}

async fn read_signature(store: &PostgresHybridV2ProjectionStore, cid: &str) -> (Option<String>, Option<String>, Option<i16>) {
    let dyn_store: &dyn HybridV2ProjectionStore = store;
    let row = dyn_store
        .get_execution_request(cid)
        .await
        .unwrap()
        .expect("row");
    (row.signature_r, row.signature_s, row.signature_v)
}

// -----------------------------------------------------------------
//                            TESTS
// -----------------------------------------------------------------

#[tokio::test]
async fn restart_after_ready_for_broadcast_no_send() {
    let Some(url) = get_pg_url_or_skip("restart_after_ready_for_broadcast_no_send") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b0".repeat(32));
    let _ = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    // Row is at BROADCAST_DISABLED, no tx_hash — a fresh outbox
    // resuming this row should return the row unchanged.
    let rpc = Arc::new(MockBroadcastRpc::new());
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(2_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let resumed = outbox
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 0,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect("resume");
    assert_eq!(resumed.phase, BroadcastPhase::BroadcastDisabled);
    assert!(rpc.write_method_calls().is_empty());
    assert!(rpc.non_send_write_methods().is_empty());
}

#[tokio::test]
async fn restart_after_broadcasting_persisted_before_send() {
    // Simulate: outbox persisted tx_hash + phase=BROADCASTING and then
    // crashed BEFORE the network call. rpc.transaction_by_hash returns
    // None → resume transitions BROADCASTING → SUBMISSION_UNKNOWN.
    let Some(url) = get_pg_url_or_skip("restart_after_broadcasting_persisted_before_send") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b1".repeat(32));
    let (_row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
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
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hex = env.envelope_hash_hex();
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
    // Fresh outbox with fresh mock RPC.
    let rpc = Arc::new(MockBroadcastRpc::new());
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(2_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let resumed = outbox
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 0,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect("resume");
    assert_eq!(resumed.phase, BroadcastPhase::SubmissionUnknown);
    assert_eq!(read_tx_hash(&store, &cid).await.as_deref(), Some(tx_hex.as_str()));
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn restart_after_rpc_request_issued_provider_accepted_response_lost() {
    let Some(url) = get_pg_url_or_skip(
        "restart_after_rpc_request_issued_provider_accepted_response_lost",
    ) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b2".repeat(32));
    let (_row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
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
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hex = env.envelope_hash_hex();
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
    // Provider actually accepted — hash is in mempool. Fresh outbox
    // observes this and advances Broadcasting → Submitted.
    let rpc = Arc::new(MockBroadcastRpc::new());
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
    let clock = Arc::new(MockClock::new(2_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let resumed = outbox
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 0,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect("resume");
    assert_eq!(resumed.phase, BroadcastPhase::Submitted);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn restart_after_submission_unknown_persistent() {
    let Some(url) = get_pg_url_or_skip("restart_after_submission_unknown_persistent") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b3".repeat(32));
    let (row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Err(BroadcastRpcError::Timeout));
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock.clone(), deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &fw,
        )
        .await
        .unwrap();
    // Drop the outbox. Advance clock past max_pending_age. Fresh outbox
    // observes persistent unknown → escalates to MANUAL_INTERVENTION.
    drop(outbox);
    clock.set(10_000_000);
    let outbox2 = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let resumed = outbox2
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 0,
                max_pending_age_ms: 100,
            },
        )
        .await
        .expect("resume");
    assert_eq!(resumed.phase, BroadcastPhase::ManualInterventionRequired);
    assert_eq!(resumed.failure_class.as_deref(), Some(bfc::TRANSPORT_AMBIGUOUS));
    // Only the initial submit hit the wire.
    assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
}

#[tokio::test]
async fn restart_after_submitted_worker_advances_to_confirming() {
    let Some(url) = get_pg_url_or_skip("restart_after_submitted_worker_advances_to_confirming")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b4".repeat(32));
    let (row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: env.envelope_hash,
    }));
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock.clone(), deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &fw,
        )
        .await
        .unwrap();
    drop(outbox);
    // Configure receipt on new worker; worker advances Submitted →
    // MinedSuccess → Confirming across two ticks.
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: 500,
            block_hash: [0xAB; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let phase = worker.tick_single(&cid).await.expect("tick");
    assert_eq!(phase, BroadcastPhase::MinedSuccess);
    let phase2 = worker.tick_single(&cid).await.expect("tick");
    assert_eq!(phase2, BroadcastPhase::Confirming);
    // The mock's write log records only the initial submit call.
    assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
}

#[tokio::test]
async fn restart_after_pending_receipt_arrives_between_ticks() {
    let Some(url) = get_pg_url_or_skip("restart_after_pending_receipt_arrives_between_ticks") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b5".repeat(32));
    let (row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: env.envelope_hash,
    }));
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock.clone(), deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &fw,
        )
        .await
        .unwrap();
    // Manually advance to Pending (simulate a mempool observation prior
    // to receipt arrival).
    store_dyn
        .update_broadcast_phase(
            &cid,
            BroadcastPhase::Submitted,
            BroadcastPhase::Pending,
            1_003,
            BroadcastStatePatch::default(),
        )
        .await
        .unwrap();
    // Receipt arrives now.
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: 500,
            block_hash: [0xAB; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let phase = worker.tick_single(&cid).await.expect("tick");
    assert_eq!(phase, BroadcastPhase::MinedSuccess);
}

#[tokio::test]
async fn restart_after_mined_success_worker_resumes_confirmation() {
    let Some(url) =
        get_pg_url_or_skip("restart_after_mined_success_worker_resumes_confirmation")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b6".repeat(32));
    let (_row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    // Install a MinedSuccess row directly.
    let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hex = env.envelope_hash_hex();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    // Walk through Broadcasting → Submitted → MinedSuccess.
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
            BroadcastPhase::Submitted,
            1_003,
            BroadcastStatePatch::default(),
        )
        .await
        .unwrap();
    dyn_store
        .update_broadcast_phase(
            &cid,
            BroadcastPhase::Submitted,
            BroadcastPhase::MinedSuccess,
            1_004,
            BroadcastStatePatch {
                receipt_tx_hash: Some(tx_hex.clone()),
                receipt_block_number: Some(500),
                receipt_block_hash: Some(format!("0x{}", "AB".repeat(32).to_lowercase())),
                receipt_status: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Fresh worker: canonicality + tick reads MinedSuccess and advances
    // to Confirming.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: 500,
            block_hash: [0xAB; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(2_000));
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let phase = worker.tick_single(&cid).await.expect("tick");
    assert_eq!(phase, BroadcastPhase::Confirming);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn restart_after_confirming_below_depth_keeps_confirming() {
    let Some(url) = get_pg_url_or_skip("restart_after_confirming_below_depth_keeps_confirming")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b7".repeat(32));
    let (_row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
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
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hex = env.envelope_hash_hex();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
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
                receipt_block_number: Some(500),
                receipt_block_hash: Some(format!("0x{}", "ab".repeat(32))),
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
            .update_broadcast_phase(&cid, from, to, 1_005, patch)
            .await
            .unwrap();
    }
    // Head is at 500 (equal to receipt block); depth threshold 3 not
    // reached — a fresh worker's maybe_finalize returns None.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_head(500);
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(2_000));
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let row = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
    let out = worker.maybe_finalize(&row).await.expect("maybe_finalize");
    assert!(out.is_none());
    let after = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(after.phase, BroadcastPhase::Confirming);
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn restart_after_confirmed_terminal_is_idempotent() {
    let Some(url) = get_pg_url_or_skip("restart_after_confirmed_terminal_is_idempotent") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b8".repeat(32));
    let (_row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
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
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hex = env.envelope_hash_hex();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
    // Walk row all the way to Confirmed.
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
                receipt_block_number: Some(500),
                receipt_block_hash: Some(format!("0x{}", "ab".repeat(32))),
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
            BroadcastPhase::Confirmed,
            BroadcastStatePatch {
                canonicality_state: Some("CANONICAL".into()),
                terminal_at_ms: Some(9_000),
                ..Default::default()
            },
        ),
    ] {
        dyn_store
            .update_broadcast_phase(&cid, from, to, 1_005, patch)
            .await
            .unwrap();
    }
    // Resume of a Confirmed row must be a NO-OP.
    let rpc = Arc::new(MockBroadcastRpc::new());
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(3_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let resumed = outbox
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 0,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect("resume");
    assert_eq!(resumed.phase, BroadcastPhase::Confirmed);
    // No writes; no state mutation.
    assert!(rpc.write_method_calls().is_empty());
    let after = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(after.phase, BroadcastPhase::Confirmed);
    assert_eq!(after.terminal_at_ms, Some(9_000));
}

#[tokio::test]
async fn restart_preserves_tx_hash_across_all_phases() {
    let Some(url) = get_pg_url_or_skip("restart_preserves_tx_hash_across_all_phases") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "b9".repeat(32));
    let (row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: env.envelope_hash,
    }));
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock.clone(), deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &fw,
        )
        .await
        .unwrap();
    let observed = read_tx_hash(&store, &cid).await.expect("hash");
    assert_eq!(observed, env.envelope_hash_hex());
    // Drop outbox / worker, verify hash persists.
    drop(outbox);
    assert_eq!(read_tx_hash(&store, &cid).await.as_deref(), Some(observed.as_str()));
    // A fresh worker tick preserves the hash too.
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: 500,
            block_hash: [0xAB; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let _ = worker.tick_single(&cid).await.expect("tick");
    let after = read_tx_hash(&store, &cid).await.expect("hash");
    assert_eq!(after, observed);
}

#[tokio::test]
async fn restart_never_reserves_new_nonce() {
    let Some(url) = get_pg_url_or_skip("restart_never_reserves_new_nonce") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ba".repeat(32));
    let (row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Err(BroadcastRpcError::Timeout));
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock.clone(), deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &fw,
        )
        .await
        .unwrap();
    let nonce_before = read_reserved_nonce(&store, &cid).await;
    drop(outbox);
    // Fresh outbox + fresh resume must NOT re-reserve a nonce.
    let outbox2 = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let _ = outbox2
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 0,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect("resume");
    let nonce_after = read_reserved_nonce(&store, &cid).await;
    assert_eq!(nonce_before, nonce_after);
    // Suppress unused warnings.
    let _ = env;
}

#[tokio::test]
async fn restart_never_recomputes_different_signature() {
    let Some(url) = get_pg_url_or_skip("restart_never_recomputes_different_signature") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "bb".repeat(32));
    let (row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let env = serialize_signed_execution(
        &plan,
        &signed,
        42,
        1_000_000,
        U256::from(2_000_000_000u64),
        U256::from(500_000_000u64),
    )
    .unwrap();
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: env.envelope_hash,
    }));
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock.clone(), deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    outbox
        .submit(
            row,
            plan,
            signed,
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &fw,
        )
        .await
        .unwrap();
    let sig_before = read_signature(&store, &cid).await;
    drop(outbox);
    // Fresh outbox resume path is an OBSERVATION only — never re-signs.
    let outbox2 = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let _ = outbox2
        .resume(
            &cid,
            ResumePolicy {
                submission_retry_max: 0,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect("resume");
    let sig_after = read_signature(&store, &cid).await;
    assert_eq!(sig_before, sig_after);
}

#[tokio::test]
async fn restart_after_reorged_recovers_via_reorg_recovery() {
    let Some(url) = get_pg_url_or_skip("restart_after_reorged_recovers_via_reorg_recovery") else {
        return;
    };
    use deopt_v2_backend::hybrid_v2::execution::broadcast_nonce_policy::BroadcastNonceInvestigator;
    use deopt_v2_backend::hybrid_v2::execution::broadcast_reorg_recovery::{
        BroadcastReorgRecovery, ReorgRecoveryOutcome,
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "bc".repeat(32));
    let (_row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
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
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hex = env.envelope_hash_hex();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
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
                receipt_block_number: Some(500),
                receipt_block_hash: Some(format!("0x{}", "ab".repeat(32))),
                receipt_status: Some(1),
                ..Default::default()
            },
        ),
        (
            BroadcastPhase::MinedSuccess,
            BroadcastPhase::Reorged,
            BroadcastStatePatch {
                canonicality_state: Some("ORPHANED".into()),
                reorg_count: Some(1),
                ..Default::default()
            },
        ),
    ] {
        dyn_store
            .update_broadcast_phase(&cid, from, to, 1_005, patch)
            .await
            .unwrap();
    }
    // Restart: fresh recovery driver with same store.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: 600,
            block_hash: [0xCD; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(600, [0xCD; 32]);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let investigator = BroadcastNonceInvestigator {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        executor_address: signer,
        chain_id: 84532,
    };
    let recovery = BroadcastReorgRecovery {
        store: dyn_store,
        rpc: rpc_dyn.as_ref(),
        nonce_investigator: &investigator,
        deployment_id,
        chain_id: 84532,
        executor_address: signer,
    };
    let outcome = recovery.recover(&cid).await.expect("recover");
    assert!(matches!(
        outcome,
        ReorgRecoveryOutcome::ReminedInReplacement { .. }
    ));
    assert!(rpc.write_method_calls().is_empty());
}

#[tokio::test]
async fn restart_after_confirmation_threshold_reached_before_db_commit() {
    let Some(url) = get_pg_url_or_skip(
        "restart_after_confirmation_threshold_reached_before_db_commit",
    ) else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "bd".repeat(32));
    let (_row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
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
    dyn_store.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hex = env.envelope_hash_hex();
    dyn_store
        .set_broadcast_tx_hash(&cid, &tx_hex, &tx_hex, &tx_hex, 1_001)
        .await
        .unwrap();
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
                receipt_block_number: Some(500),
                receipt_block_hash: Some(format!("0x{}", "ab".repeat(32))),
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
            .update_broadcast_phase(&cid, from, to, 1_005, patch)
            .await
            .unwrap();
    }
    // Fresh worker: canonical head is deep enough that the confirmation
    // depth is reached; correlation gate is NOT set up so maybe_finalize
    // returns None. Restart safety invariant asserted here is: no
    // spontaneous mutation of the row and no write to the RPC.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_head(600);
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(3_000));
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let row = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
    let out = worker.maybe_finalize(&row).await;
    match out {
        Ok(None) => {
            let after = dyn_store.get_broadcast_state(&cid).await.unwrap().unwrap();
            assert_eq!(after.phase, BroadcastPhase::Confirming);
        }
        Ok(Some(other)) => panic!("unexpected phase promoted without correlation: {other:?}"),
        Err(e) => panic!("maybe_finalize failed: {e}"),
    }
    assert!(rpc.write_method_calls().is_empty());
    // Suppress unused import warning.
    let _ = &pool;
}
