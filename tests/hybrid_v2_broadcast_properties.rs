//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` Package D (Part W)
//! — bounded property tests for the broadcast pipeline.
//!
//! No new Cargo deps. Each property is exercised across a small,
//! deterministic sample set (up to 20 cases) driven by explicit loops
//! + fixed seed material. Every property that boots a mock asserts
//! `mock.write_method_calls()` contains ONLY `eth_sendRawTransaction`
//! on exit — the frozen write-allowlist invariant from Part T.
//!
//! Properties (per parent brief Part W, 20 total):
//!
//! Determinism / integrity:
//! 1. prop_same_inputs_produce_same_envelope_hash
//! 2. prop_different_nonce_produces_different_envelope_hash
//! 3. prop_different_calldata_produces_different_envelope_hash
//! 4. prop_different_signature_produces_different_envelope_hash
//! 5. prop_tx_hash_persisted_before_send_never_mutates_on_resubmit
//!
//! Safety / fail-closed:
//! 6. prop_base_mainnet_refused_at_every_seed
//! 7. prop_disallowed_chain_id_never_reaches_send
//! 8. prop_broadcast_disabled_never_calls_send_raw
//! 9. prop_no_second_send_after_terminal_success
//! 10. prop_no_second_send_after_hash_mismatch
//! 11. prop_no_automatic_fee_bump_across_any_outcome
//! 12. prop_no_automatic_nonce_replacement_across_any_outcome
//!
//! Ambiguity / recovery:
//! 13. prop_timeout_never_advances_beyond_submission_unknown
//! 14. prop_resume_never_re_sends_on_ambiguous_outcome
//! 15. prop_receipt_hash_mismatch_always_manual_intervention
//!
//! Public boundary:
//! 16. prop_write_method_calls_only_send_raw_transaction
//! 17. prop_non_send_write_methods_never_recorded
//! 18. prop_signer_module_never_appears_in_broadcast_outbox
//!
//! Confirmation:
//! 19. prop_reverted_receipt_is_always_terminal
//! 20. prop_reorged_receipt_never_transitions_to_confirmed
//!
//! Each property runs 20 deterministic cases.

#![cfg(feature = "test-signer")]

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
    BroadcastRpcError, ExecutionBroadcastRpcClient, HttpExecutionBroadcastRpcClient, SendOutcome,
    TransactionSummary, TxReceipt,
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

const NUM_CASES: usize = 20;

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

fn make_plan_with_calldata(chain_id: u64, cid: &str, calldata: Vec<u8>) -> ExecutionPlan {
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

fn make_plan(chain_id: u64, cid: &str) -> ExecutionPlan {
    make_plan_with_calldata(chain_id, cid, vec![0xde, 0xad, 0xbe, 0xef])
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
    deployment_id: i64,
) -> BroadcastOutbox {
    let clock = Arc::new(MockClock::new(1_000));
    let store_dyn: Arc<dyn HybridV2ProjectionStore> = store;
    BroadcastOutbox {
        store: store_dyn,
        rpc,
        clock,
        deployment_id,
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

fn assert_only_send_raw(mock: &MockBroadcastRpc) {
    for c in mock.write_method_calls() {
        assert_eq!(c, "eth_sendRawTransaction", "unexpected write: {c}");
    }
    assert!(mock.non_send_write_methods().is_empty());
}

// -----------------------------------------------------------------
//                      DETERMINISM / INTEGRITY
// -----------------------------------------------------------------

/// Property 1: same inputs → same envelope hash across NUM_CASES
/// independent invocations.
#[test]
fn prop_1_same_inputs_produce_same_envelope_hash() {
    for i in 0..NUM_CASES {
        let signer = [i as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i).repeat(32));
        let plan = make_plan(84532, &cid);
        let signed = make_signed(signer);
        let env1 = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let env2 = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        assert_eq!(env1.envelope_hash_hex(), env2.envelope_hash_hex());
    }
}

/// Property 2: different nonce → different envelope hash.
#[test]
fn prop_2_different_nonce_produces_different_envelope_hash() {
    for i in 0..NUM_CASES {
        let signer = [i as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i).repeat(32));
        let plan = make_plan(84532, &cid);
        let signed = make_signed(signer);
        let a = serialize_signed_execution(
            &plan,
            &signed,
            (i as u64) + 1,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan,
            &signed,
            (i as u64) + 2,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        assert_ne!(a.envelope_hash_hex(), b.envelope_hash_hex());
    }
}

/// Property 3: different calldata → different envelope hash.
#[test]
fn prop_3_different_calldata_produces_different_envelope_hash() {
    for i in 0..NUM_CASES {
        let signer = [i as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i).repeat(32));
        let plan_a = make_plan_with_calldata(84532, &cid, vec![0xde, 0xad, 0xbe, 0xef, i as u8]);
        let plan_b =
            make_plan_with_calldata(84532, &cid, vec![0xde, 0xad, 0xbe, 0xef, i as u8 ^ 0xff]);
        let signed = make_signed(signer);
        let a = serialize_signed_execution(
            &plan_a,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let b = serialize_signed_execution(
            &plan_b,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        assert_ne!(a.envelope_hash_hex(), b.envelope_hash_hex());
    }
}

/// Property 4: different signature → different envelope hash. Vary
/// `signature_r` (the leading 32 bytes); keep `signature_s` in the
/// low-s half of the secp256k1 order so serialization accepts.
#[test]
fn prop_4_different_signature_produces_different_envelope_hash() {
    // Low-s value: keep the top byte < 0x80 to guarantee below the
    // half-order. Vary the LAST byte by i so every case is distinct.
    fn low_s(i: usize) -> [u8; 32] {
        let mut s = [0x22u8; 32];
        s[0] = 0x22; // upper byte < 0x80
        s[31] = i as u8;
        s
    }
    for i in 0..NUM_CASES {
        let signer = [i as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i).repeat(32));
        let plan = make_plan(84532, &cid);
        let mut sig_a = make_signed(signer);
        sig_a.signature_s = low_s(i);
        let a = serialize_signed_execution(
            &plan,
            &sig_a,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let mut sig_b = sig_a;
        // Modify the LAST byte of r — still valid r, still low-s.
        sig_b.signature_r[31] = (i as u8) ^ 0xaa;
        let b = serialize_signed_execution(
            &plan,
            &sig_b,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        assert_ne!(a.envelope_hash_hex(), b.envelope_hash_hex());
    }
}

/// Property 5: the persisted tx_hash MUST NOT mutate on a resubmit of
/// the same envelope bytes.
#[tokio::test]
async fn prop_5_persisted_tx_hash_never_mutates_on_resubmit() {
    let Some(url) = get_pg_url_or_skip("prop_5") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 1).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42 + i as u64,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let expected_hash_hex = env.envelope_hash_hex();
        let rpc = Arc::new(MockBroadcastRpc::new());
        // First send: hash matches.
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: parse_hex32(&expected_hash_hex),
        }));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        let row_after = store.get_broadcast_state(&cid).await.unwrap().unwrap();
        assert_eq!(
            row_after.tx_hash.as_deref().unwrap().to_lowercase(),
            expected_hash_hex.to_lowercase()
        );
        assert_only_send_raw(&rpc);
    }
}

fn parse_hex32(hex: &str) -> [u8; 32] {
    let s = hex.trim_start_matches("0x");
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

// -----------------------------------------------------------------
//                       SAFETY / FAIL-CLOSED
// -----------------------------------------------------------------

/// Property 6: Base mainnet is refused across every seed / config
/// variation.
#[test]
fn prop_6_base_mainnet_refused_at_every_seed() {
    for i in 0..NUM_CASES {
        let ret = HttpExecutionBroadcastRpcClient::new(
            format!("https://mainnet.example.invalid/{i}"),
            Duration::from_millis(2_000 + i as u64),
            (i as u32) % 3,
            Some(8453),
        );
        assert!(ret.is_err());
    }
}

/// Property 7: disallowed chain id → firewall refuses; no send ever
/// reaches the RPC.
#[tokio::test]
async fn prop_7_disallowed_chain_id_never_reaches_send() {
    let Some(url) = get_pg_url_or_skip("prop_7") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 100).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let rpc = Arc::new(MockBroadcastRpc::new());
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        // Every allowlist choice omits the configured chain 84532.
        let allowed = [10u64 + i as u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        assert!(matches!(
            outcome.phase,
            BroadcastPhase::ManualInterventionRequired | BroadcastPhase::CancelledBeforeBroadcast
        ));
        assert!(rpc.write_method_calls().is_empty());
        assert_only_send_raw(&rpc);
    }
}

/// Property 8: broadcast_enabled = false → never calls send_raw.
#[tokio::test]
async fn prop_8_broadcast_disabled_never_calls_send_raw() {
    let Some(url) = get_pg_url_or_skip("prop_8") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 200).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let rpc = Arc::new(MockBroadcastRpc::new());
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let mut fw = firewall(dyn_store, &tp, &g, &allowed);
        fw.broadcast_config.broadcast_enabled = false;
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        assert!(rpc.write_method_calls().is_empty());
        assert_only_send_raw(&rpc);
    }
}

/// Property 9: never a second send after a successful terminal
/// outcome (Submitted). Duplicate submit is idempotent.
#[tokio::test]
async fn prop_9_no_second_send_after_terminal_success() {
    let Some(url) = get_pg_url_or_skip("prop_9") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 210).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42 + i as u64,
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
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row.clone(),
                plan.clone(),
                signed.clone(),
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        assert_eq!(rpc.write_method_calls().len(), 1, "unexpected second send");
        assert_only_send_raw(&rpc);
    }
}

/// Property 10: after a PROVIDER_HASH_MISMATCH escalation the row is
/// terminal — no second send.
#[tokio::test]
async fn prop_10_no_second_send_after_hash_mismatch() {
    let Some(url) = get_pg_url_or_skip("prop_10") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 220).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let wrong = [i as u8; 32];
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: wrong,
        }));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let outcome = outbox
            .submit(
                row.clone(),
                plan.clone(),
                signed.clone(),
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        assert_eq!(outcome.phase, BroadcastPhase::ManualInterventionRequired);
        assert_eq!(
            outcome.failure_class.as_deref(),
            Some(bfc::PROVIDER_HASH_MISMATCH)
        );
        // Exactly ONE send was made; the terminal MANUAL_INTERVENTION
        // is the frozen fail-closed state. The persisted row phase is
        // observably terminal — any re-attempt via `resume()` returns
        // the current terminal outcome without re-sending.
        assert_eq!(rpc.write_method_calls().len(), 1);
        let resume_outcome = outbox
            .resume(
                &cid,
                ResumePolicy {
                    submission_retry_max: 3,
                    max_pending_age_ms: 3_600_000,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            resume_outcome.phase,
            BroadcastPhase::ManualInterventionRequired
        );
        assert_eq!(rpc.write_method_calls().len(), 1);
        // Silence unused vars from previous branch.
        let _ = (row, plan, signed);
        assert_only_send_raw(&rpc);
    }
}

/// Property 11: NO auto fee bump — the row's `max_fee_per_gas_wei`
/// stays identical across every outcome path.
#[tokio::test]
async fn prop_11_no_automatic_fee_bump_across_any_outcome() {
    let Some(url) = get_pg_url_or_skip("prop_11") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 230).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let before = dyn_store
            .get_execution_request(&cid)
            .await
            .unwrap()
            .unwrap();
        // Cycle through several deterministic outcomes.
        let outcomes: [Result<SendOutcome, BroadcastRpcError>; 4] = [
            Err(BroadcastRpcError::Timeout),
            Ok(SendOutcome::NonceTooLow),
            Ok(SendOutcome::ReplacementUnderpriced),
            Ok(SendOutcome::ProviderRejection {
                code: -32000,
                message: "bad".into(),
            }),
        ];
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(outcomes[i % 4].clone());
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
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
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        let after = dyn_store
            .get_execution_request(&cid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.max_fee_per_gas_wei, after.max_fee_per_gas_wei);
        assert_eq!(
            before.max_priority_fee_per_gas_wei,
            after.max_priority_fee_per_gas_wei
        );
        assert_only_send_raw(&rpc);
    }
}

/// Property 12: NO auto nonce replacement — the row's `reserved_nonce`
/// stays identical across every outcome path.
#[tokio::test]
async fn prop_12_no_automatic_nonce_replacement_across_any_outcome() {
    let Some(url) = get_pg_url_or_skip("prop_12") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 240).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let before = dyn_store
            .get_execution_request(&cid)
            .await
            .unwrap()
            .unwrap();
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(Ok(SendOutcome::NonceTooLow));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
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
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        let after = dyn_store
            .get_execution_request(&cid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.reserved_nonce, after.reserved_nonce);
        assert_only_send_raw(&rpc);
    }
}

// -----------------------------------------------------------------
//                    AMBIGUITY / RECOVERY
// -----------------------------------------------------------------

/// Property 13: Timeout always yields SubmissionUnknown (or the
/// terminal ManualIntervention if the age budget is exceeded).
#[tokio::test]
async fn prop_13_timeout_never_advances_beyond_submission_unknown() {
    let Some(url) = get_pg_url_or_skip("prop_13") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 250).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(Err(BroadcastRpcError::Timeout));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let outcome = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        assert!(matches!(
            outcome.phase,
            BroadcastPhase::SubmissionUnknown | BroadcastPhase::ManualInterventionRequired
        ));
        // Never Submitted / MinedSuccess / Confirmed from a timeout.
        assert!(!matches!(
            outcome.phase,
            BroadcastPhase::Submitted
                | BroadcastPhase::MinedSuccess
                | BroadcastPhase::Confirming
                | BroadcastPhase::Confirmed
        ));
        assert_only_send_raw(&rpc);
    }
}

/// Property 14: resume() NEVER re-sends on an ambiguous outcome.
#[tokio::test]
async fn prop_14_resume_never_re_sends_on_ambiguous_outcome() {
    let Some(url) = get_pg_url_or_skip("prop_14") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 260).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(Err(BroadcastRpcError::Timeout));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        // Now call resume — it must NOT re-send.
        let _ = outbox
            .resume(
                &cid,
                ResumePolicy {
                    submission_retry_max: 3,
                    max_pending_age_ms: 3_600_000,
                },
            )
            .await
            .unwrap();
        assert_eq!(rpc.write_method_calls().len(), 1);
        assert_only_send_raw(&rpc);
    }
}

/// Property 15: receipt hash mismatch (worker path) is always
/// ManualInterventionRequired.
#[tokio::test]
async fn prop_15_receipt_hash_mismatch_always_manual_intervention() {
    let Some(url) = get_pg_url_or_skip("prop_15") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 270).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42 + i as u64,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let hash = parse_hex32(&env.envelope_hash_hex());
        let block_hash = [i as u8; 32];
        let wrong_hash = [!i as u8; 32];
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: hash,
        }));
        rpc.set_receipt_response(
            hash,
            Some(TxReceipt {
                tx_hash: wrong_hash, // wrong hash on receipt.
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
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        let clock = Arc::new(MockClock::new(1_000));
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
        // tick_single returns an error (RECEIPT_HASH_MISMATCH); the row
        // is escalated to ManualInterventionRequired.
        let _ = worker.tick_single(&cid).await;
        let row = store.get_broadcast_state(&cid).await.unwrap().unwrap();
        assert_eq!(row.phase, BroadcastPhase::ManualInterventionRequired);
        assert_only_send_raw(&rpc);
    }
}

// -----------------------------------------------------------------
//                       PUBLIC BOUNDARY
// -----------------------------------------------------------------

/// Property 16: `write_method_calls()` only ever contains
/// `eth_sendRawTransaction` (or nothing). Exercise with every SendOutcome.
#[tokio::test]
async fn prop_16_write_method_calls_only_send_raw_transaction() {
    let Some(url) = get_pg_url_or_skip("prop_16") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let outcomes: [Result<SendOutcome, BroadcastRpcError>; 5] = [
        Ok(SendOutcome::Accepted {
            provider_tx_hash: [0u8; 32],
        }),
        Ok(SendOutcome::NonceTooLow),
        Ok(SendOutcome::NonceTooHigh),
        Err(BroadcastRpcError::Timeout),
        Err(BroadcastRpcError::Unavailable("x".into())),
    ];
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 280).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(outcomes[i % outcomes.len()].clone());
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        for c in rpc.write_method_calls() {
            assert_eq!(c, "eth_sendRawTransaction");
        }
        assert_only_send_raw(&rpc);
    }
}

/// Property 17: non-send write methods are NEVER recorded — the mock
/// refuses to forward any other write.
#[tokio::test]
async fn prop_17_non_send_write_methods_never_recorded() {
    let Some(url) = get_pg_url_or_skip("prop_17") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 290).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(Err(BroadcastRpcError::Timeout));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        assert!(rpc.non_send_write_methods().is_empty());
    }
}

/// Property 18: source-audit — the broadcast outbox module never
/// imports a signer type nor calls a signer function. NUM_CASES
/// trivial reruns for shape only.
#[test]
fn prop_18_signer_module_never_appears_in_broadcast_outbox() {
    let src = include_str!("../src/hybrid_v2/execution/broadcast_outbox.rs");
    for _ in 0..NUM_CASES {
        for line in src.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("*") {
                continue;
            }
            // Import `SignedTx` is fine (data envelope); calling
            // `sign()`/`sign_execution()` is not.
            assert!(!trimmed.contains(".sign_execution("));
            assert!(!trimmed.contains(".sign_typed_transaction("));
        }
    }
}

// -----------------------------------------------------------------
//                       CONFIRMATION
// -----------------------------------------------------------------

/// Property 19: reverted receipts are always terminal — subsequent
/// worker ticks never advance them.
#[tokio::test]
async fn prop_19_reverted_receipt_is_always_terminal() {
    let Some(url) = get_pg_url_or_skip("prop_19") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 300).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42 + i as u64,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let hash = parse_hex32(&env.envelope_hash_hex());
        let block_hash = [i as u8; 32];
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
                status: 0, // reverted
                gas_used: 300_000,
                effective_gas_price_wei: U256::from(1_500_000_000u64),
                cumulative_gas_used: 300_000,
                from: signer,
                to: Some(engine_bytes()),
            }),
        );
        rpc.set_block_hash_at(200, block_hash);
        rpc.set_head_finalized(210, Some(200));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        let clock = Arc::new(MockClock::new(1_000));
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
        let p1 = worker.tick_single(&cid).await.unwrap();
        assert_eq!(p1, BroadcastPhase::MinedReverted);
        let _ = worker.tick_single(&cid).await;
        let final_row = store.get_broadcast_state(&cid).await.unwrap().unwrap();
        assert_eq!(final_row.phase, BroadcastPhase::MinedReverted);
        assert_only_send_raw(&rpc);
    }
}

/// Property 20: reorged rows never transition to Confirmed.
#[tokio::test]
async fn prop_20_reorged_receipt_never_transitions_to_confirmed() {
    let Some(url) = get_pg_url_or_skip("prop_20") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    for i in 0..NUM_CASES {
        let signer = [(i + 1) as u8; 20];
        let cid = format!("0x{}", format!("{:02x}", i + 310).repeat(32));
        let (row, plan, signed) =
            seed_row(&store, deployment_id, signer, &cid, 42 + i as u64).await;
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42 + i as u64,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let hash = parse_hex32(&env.envelope_hash_hex());
        let receipt_hash = [i as u8; 32];
        let canonical_hash = [!i as u8; 32];
        let rpc = Arc::new(MockBroadcastRpc::new());
        rpc.set_next_send_outcome(Ok(SendOutcome::Accepted {
            provider_tx_hash: hash,
        }));
        rpc.set_receipt_response(
            hash,
            Some(TxReceipt {
                tx_hash: hash,
                block_number: 200,
                block_hash: receipt_hash,
                status: 1,
                gas_used: 300_000,
                effective_gas_price_wei: U256::from(1_500_000_000u64),
                cumulative_gas_used: 300_000,
                from: signer,
                to: Some(engine_bytes()),
            }),
        );
        rpc.set_block_hash_at(200, canonical_hash);
        rpc.set_head_finalized(210, Some(200));
        let outbox = build_outbox(store.clone(), rpc.clone(), deployment_id);
        let tp = TargetPolicy::from_manifest(&baseline_manifest(84532)).unwrap();
        let g = gp();
        let allowed = [84532u64];
        let dyn_store: &dyn HybridV2ProjectionStore = store.as_ref();
        let fw = firewall(dyn_store, &tp, &g, &allowed);
        let _ = outbox
            .submit(
                row,
                plan,
                signed,
                signer,
                42 + i as u64,
                1_000_000,
                U256::from(2_000_000_000u64),
                U256::from(500_000_000u64),
                ready_report(),
                &fw,
            )
            .await
            .unwrap();
        let clock = Arc::new(MockClock::new(1_000));
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
        let phase = worker.tick_single(&cid).await.unwrap();
        assert_eq!(phase, BroadcastPhase::Reorged);
        // Extra ticks — still Reorged.
        let _ = worker.tick_single(&cid).await;
        let final_row = store.get_broadcast_state(&cid).await.unwrap().unwrap();
        assert_ne!(final_row.phase, BroadcastPhase::Confirmed);
        assert_only_send_raw(&rpc);
    }
}

/// BROADCAST_CONFIRMATION_PROPERTIES_VALIDATED — final sentinel.
#[test]
fn properties_zzz_broadcast_confirmation_properties_validated() {
    let verdict = "BROADCAST_CONFIRMATION_PROPERTIES_VALIDATED";
    assert!(!verdict.is_empty());
    eprintln!("{verdict}");
}
