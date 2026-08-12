//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` Package B (Parts
//! J–P) — real-PostgreSQL coverage for the broadcast lifecycle:
//! idempotent resume, same-byte resend, nonce conflict investigation,
//! receipt polling, canonicality verification, confirmation depth,
//! indexer correlation, and the final confirmation rule.
//!
//! Every test uses a deterministic in-process
//! [`MockBroadcastRpc`] — NO REAL EXTERNAL CHAIN TRANSACTION IS EVER
//! BROADCAST. The mock records every `send_raw_transaction` invocation
//! so tests can assert `write_method_calls() == ["eth_sendRawTransaction"]`.

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
use deopt_v2_backend::hybrid_v2::execution::broadcast_worker::{
    BroadcastConfirmationWorker, CanonicalityStatus,
};
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
            panic!("{} required but no PG URL provided", REQUIRE_ENV);
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
        calldata_bytes: None,
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

// -----------------------------------------------------------------
//                             TESTS
// -----------------------------------------------------------------

// -------------------- Part J — idempotency + resend --------------

#[tokio::test]
async fn resume_after_submission_unknown_finds_tx_and_advances() {
    let Some(url) = get_pg_url_or_skip("resume_after_submission_unknown_finds_tx_and_advances")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a0".repeat(32));
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

    // First: submit hits Timeout -> SubmissionUnknown.
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
    let outcome = outbox
        .submit(
            row.clone(),
            plan.clone(),
            signed.clone(),
            signer,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
            make_readiness_ready(),
            &fw,
        )
        .await
        .expect("submit");
    assert_eq!(outcome.phase, BroadcastPhase::SubmissionUnknown);

    // Then: resume finds the tx in the mempool -> Pending.
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
    assert_eq!(resumed.phase, BroadcastPhase::Pending);
    // Only one write invocation (the initial submit).
    assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
}

#[tokio::test]
async fn resume_after_submission_unknown_past_budget_escalates() {
    let Some(url) = get_pg_url_or_skip("resume_after_submission_unknown_past_budget_escalates")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a1".repeat(32));
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

    // Advance the clock past max_pending_age_ms.
    clock.set(10_000_000);
    let resumed = outbox
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
    assert_eq!(
        resumed.failure_class.as_deref(),
        Some(bfc::TRANSPORT_AMBIGUOUS)
    );
}

#[tokio::test]
async fn resend_same_bytes_permitted_under_budget() {
    let Some(url) = get_pg_url_or_skip("resend_same_bytes_permitted_under_budget") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a2".repeat(32));
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
            row.clone(),
            plan.clone(),
            signed.clone(),
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

    // Next send outcome: Accepted with matching hash.
    rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
        provider_tx_hash: env.envelope_hash,
    }));
    let resend = outbox
        .resend_same_bytes(
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
            ResumePolicy {
                submission_retry_max: 1, // one extra attempt allowed
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect("resend");
    assert_eq!(resend.phase, BroadcastPhase::Submitted);
    // Two writes: initial submit + one resend. NEVER more.
    assert_eq!(
        rpc.write_method_calls(),
        vec!["eth_sendRawTransaction", "eth_sendRawTransaction"]
    );
}

// -------------------- Part K — nonce conflict --------------------

#[tokio::test]
async fn nonce_too_low_escalates_to_manual_with_investigation() {
    let Some(url) = get_pg_url_or_skip("nonce_too_low_escalates_to_manual_with_investigation")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a3".repeat(32));
    let (row, plan, signed) = seed_row(&store, deployment_id, signer, &cid, 42).await;
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_next_send_outcome(Ok(SendOutcome::NonceTooLow));
    // Pending nonce is 43 > our 42 → DifferentTxConsumedNonce.
    rpc.set_transaction_count(
        signer,
        deopt_v2_backend::hybrid_v2::execution::rpc::BlockTag::Pending,
        43,
    );
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    let outcome = outbox
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
        .expect("submit");
    assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
    let fc = outcome.failure_class.as_deref().unwrap_or("");
    assert!(fc.contains("DIFFERENT_TX_CONSUMED_NONCE"), "got: {fc}");
}

#[tokio::test]
async fn nonce_conflict_investigation_our_tx_mined_returns_specific_class() {
    let Some(url) =
        get_pg_url_or_skip("nonce_conflict_investigation_our_tx_mined_returns_specific_class")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a4".repeat(32));
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
    rpc.set_next_send_outcome(Ok(SendOutcome::NonceTooLow));
    // The investigator: transaction_by_hash returns Some(mined).
    rpc.set_transaction_response(
        env.envelope_hash,
        Some(TransactionSummary {
            tx_hash: env.envelope_hash,
            from: signer,
            to: Some(engine_bytes()),
            nonce: 42,
            block_number: Some(500),
            block_hash: Some([0xAB; 32]),
            value_wei: U256::ZERO,
            input_bytes_len: 4,
            input_hash: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            tx_type: 2,
        }),
    );
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    let outcome = outbox
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
        .expect("submit");
    assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
    let fc = outcome.failure_class.as_deref().unwrap_or("");
    assert!(fc.contains("OUR_TX_MINED"), "got: {fc}");
}

// -------------------- Parts L, M, N — worker --------------------

#[tokio::test]
async fn receipt_polling_pending_to_mined_success_advances_to_confirming() {
    let Some(url) =
        get_pg_url_or_skip("receipt_polling_pending_to_mined_success_advances_to_confirming")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a5".repeat(32));
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

    // Configure the receipt + canonical header + head.
    let receipt_block = 500u64;
    let receipt_hash = [0xAB; 32];
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: receipt_block,
            block_hash: receipt_hash,
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(receipt_block, receipt_hash);
    rpc.set_head(receipt_block + 1);
    let worker = build_worker(
        store.clone(),
        rpc_dyn.clone(),
        clock.clone(),
        deployment_id,
        3,
    );
    // Tick 1: Submitted -> MinedSuccess.
    let phase = worker.tick_single(&cid).await.expect("tick 1");
    assert_eq!(phase, BroadcastPhase::MinedSuccess);
    // Tick 2: MinedSuccess -> Confirming.
    let phase = worker.tick_single(&cid).await.expect("tick 2");
    assert_eq!(phase, BroadcastPhase::Confirming);
}

#[tokio::test]
async fn receipt_polling_status_zero_is_terminal_mined_reverted() {
    let Some(url) = get_pg_url_or_skip("receipt_polling_status_zero_is_terminal_mined_reverted")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a6".repeat(32));
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

    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: 500,
            block_hash: [0xAB; 32],
            status: 0,
            gas_used: 100_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 100_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let phase = worker.tick_single(&cid).await.expect("tick");
    assert_eq!(phase, BroadcastPhase::MinedReverted);
    // Only one write on the wire (initial submit) — never a retry.
    assert_eq!(rpc.write_method_calls(), vec!["eth_sendRawTransaction"]);
}

#[tokio::test]
async fn canonicality_hash_mismatch_transitions_to_reorged() {
    let Some(url) = get_pg_url_or_skip("canonicality_hash_mismatch_transitions_to_reorged") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a7".repeat(32));
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

    // Configure a receipt whose block_hash does not match the canonical
    // header at that block number.
    let receipt_block = 500u64;
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: receipt_block,
            block_hash: [0xCD; 32], // orphaned
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(receipt_block, [0xAB; 32]); // canonical hash
    rpc.set_head(receipt_block + 1);
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let phase = worker.tick_single(&cid).await.expect("tick");
    assert_eq!(phase, BroadcastPhase::Reorged);
}

#[tokio::test]
async fn canonicality_matches_returns_canonical_verdict() {
    let Some(url) = get_pg_url_or_skip("canonicality_matches_returns_canonical_verdict") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let rpc = Arc::new(MockBroadcastRpc::new());
    let rpc_dyn: Arc<dyn ExecutionBroadcastRpcClient> = rpc.clone();
    let clock = Arc::new(MockClock::new(1_000));
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    rpc.set_head(1000);
    rpc.set_block_hash_at(500, [0xAB; 32]);
    let status = worker
        .verify_canonical_receipt(500, [0xAB; 32])
        .await
        .unwrap();
    assert_eq!(status, CanonicalityStatus::Canonical);
}

// -------------------- Parts O + P — correlation + finalization ------

#[tokio::test]
async fn confirming_but_indexer_behind_stays_confirming() {
    let Some(url) = get_pg_url_or_skip("confirming_but_indexer_behind_stays_confirming") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a8".repeat(32));
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
    let receipt_block = 500u64;
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: env.envelope_hash,
            block_number: receipt_block,
            block_hash: [0xAB; 32],
            status: 1,
            gas_used: 400_000,
            effective_gas_price_wei: U256::from(1_000_000_000u64),
            cumulative_gas_used: 400_000,
            from: signer,
            to: Some(engine_bytes()),
        }),
    );
    rpc.set_block_hash_at(receipt_block, [0xAB; 32]);
    rpc.set_head(receipt_block + 10); // depth 10 >> 3
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    // Two ticks: Submitted -> MinedSuccess -> Confirming.
    worker.tick_single(&cid).await.unwrap();
    worker.tick_single(&cid).await.unwrap();
    let row_after = store_dyn.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(row_after.phase, BroadcastPhase::Confirming);
    // maybe_finalize refuses because indexed_head is 0 < 500 (indexer
    // has not reported ANY block).
    let phase = worker.maybe_finalize(&row_after).await.unwrap();
    assert_eq!(phase, None);
}

// -------------------- Frozen safety cross-cutting -----------------

#[tokio::test]
async fn only_send_raw_transaction_is_the_write_method() {
    let Some(url) = get_pg_url_or_skip("only_send_raw_transaction_is_the_write_method") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "a9".repeat(32));
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
    // Configure receipt so the worker exercises the read paths.
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
    rpc.set_head(600);
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    worker.tick_single(&cid).await.unwrap();
    worker.tick_single(&cid).await.unwrap();
    // Every recorded write is eth_sendRawTransaction — one write for
    // the initial submit and NO writes from the worker.
    let writes = rpc.write_method_calls();
    assert_eq!(writes, vec!["eth_sendRawTransaction"]);
}

#[tokio::test]
async fn resend_same_bytes_rejects_when_phase_not_permitted() {
    let Some(url) = get_pg_url_or_skip("resend_same_bytes_rejects_when_phase_not_permitted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "aa".repeat(32));
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
    let outbox = build_outbox(store.clone(), rpc_dyn.clone(), clock, deployment_id);
    let target_policy = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
    let gp = gas_policy();
    let allowed = [84532u64];
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    let fw = firewall(store_dyn, &target_policy, &gp, &allowed);
    outbox
        .submit(
            row.clone(),
            plan.clone(),
            signed.clone(),
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
    // Row is now Submitted → resend must refuse.
    let err = outbox
        .resend_same_bytes(
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
            ResumePolicy {
                submission_retry_max: 3,
                max_pending_age_ms: 3_600_000,
            },
        )
        .await
        .expect_err("must refuse from Submitted");
    let s = err.to_string();
    assert!(s.contains("does not permit"), "got: {s}");
}

#[tokio::test]
async fn worker_no_receipt_advances_submitted_to_pending() {
    let Some(url) = get_pg_url_or_skip("worker_no_receipt_advances_submitted_to_pending") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ab".repeat(32));
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
    // Configure the tx as known in mempool so the worker advances
    // Submitted -> Pending.
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
    let worker = build_worker(store.clone(), rpc_dyn.clone(), clock, deployment_id, 3);
    let phase = worker.tick_single(&cid).await.expect("tick");
    assert_eq!(phase, BroadcastPhase::Pending);
}

#[tokio::test]
async fn worker_receipt_hash_mismatch_escalates_manual() {
    let Some(url) = get_pg_url_or_skip("worker_receipt_hash_mismatch_escalates_manual") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ac".repeat(32));
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
    // Configure a receipt whose tx_hash differs from what we asked for.
    // The mock indexes by the KEY we set, not by the receipt.tx_hash
    // field — set the key to our envelope hash but the payload's
    // tx_hash to something else. This simulates a provider bug.
    rpc.set_receipt_response(
        env.envelope_hash,
        Some(TxReceipt {
            tx_hash: [0xEE; 32], // deliberately different
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
    let res = worker.tick_single(&cid).await;
    assert!(matches!(
        res,
        Err(deopt_v2_backend::hybrid_v2::execution::broadcast_worker::WorkerError::ReceiptTxHashMismatch { .. })
    ));
    let live = store_dyn.get_broadcast_state(&cid).await.unwrap().unwrap();
    assert_eq!(live.phase, BroadcastPhase::ManualInterventionRequired);
    assert_eq!(
        live.failure_class.as_deref(),
        Some(bfc::RECEIPT_HASH_MISMATCH)
    );
}

#[tokio::test]
async fn broadcasting_row_resume_recovers_from_hash_lookup() {
    let Some(url) = get_pg_url_or_skip("broadcasting_row_resume_recovers_from_hash_lookup") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let signer = [0x77u8; 20];
    let cid = format!("0x{}", "ad".repeat(32));
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
    // Simulate a crash mid-send: insert broadcast row + set tx_hash +
    // transition to Broadcasting, then never call the RPC.
    let store_dyn: &dyn HybridV2ProjectionStore = store.as_ref();
    store_dyn.insert_broadcast_state(&cid, 1_000).await.unwrap();
    let tx_hash_hex = env.envelope_hash_hex();
    store_dyn
        .set_broadcast_tx_hash(&cid, &tx_hash_hex, &tx_hash_hex, &tx_hash_hex, 1_001)
        .await
        .unwrap();
    store_dyn
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
    // Configure the RPC: the tx is mined.
    let rpc = Arc::new(MockBroadcastRpc::new());
    rpc.set_transaction_response(
        env.envelope_hash,
        Some(TransactionSummary {
            tx_hash: env.envelope_hash,
            from: signer,
            to: Some(engine_bytes()),
            nonce: 42,
            block_number: Some(500),
            block_hash: Some([0xAB; 32]),
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
    // Broadcasting -> Submitted (Part J semantics — leave receipt
    // observation to the worker).
    assert_eq!(resumed.phase, BroadcastPhase::Submitted);
    // ZERO writes — resume never resigns.
    assert!(rpc.write_method_calls().is_empty());
}
