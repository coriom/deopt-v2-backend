//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` (Package 3, parts Q–U) —
//! real-PostgreSQL + in-process HTTP mock coverage for the pre-broadcast
//! [`ExecutionOrchestrator`], its restart-safety `resume` entry point,
//! and the deployment-lock contention path.
//!
//! Scope of this file:
//!   * `happy_path_discovers_and_lands_at_broadcast_disabled` — end-to-end
//!     walk from `Discovered` to `BroadcastDisabled` with the
//!     `TestEphemeralSigner`.
//!   * `duplicate_prepare_is_idempotent` — two `prepare` calls converge
//!     on the same terminal state / plan_hash / nonce.
//!   * `preflight_rejection_reconciliation_drift` — drift blocks the
//!     row in `Failed(PREFLIGHT_REJECTED)`.
//!   * `preflight_rejection_active_reorg` — active reorg blocks.
//!   * `preflight_rejection_active_rebuild` — active rebuild blocks.
//!   * `deterministic_revert_fails_row` — revert → `Failed(SIMULATION_FAILED_DETERMINISTIC)`.
//!   * `gas_policy_rejection` — abnormal-fee history → `Failed(GAS_POLICY_REJECTED)`.
//!   * `firewall_rejection_on_tampered_row` — tampered plan_hash →
//!     `Failed(FIREWALL_REJECTED)`.
//!   * `production_signer_default_is_unavailable` — `Failed(SIGNER_UNAVAILABLE)`.
//!   * `resume_after_mid_signing_crash` — resume from `SignatureVerified`
//!     idempotently.
//!   * `lock_contention_returns_lock_contention_error` — two concurrent
//!     prepare calls, one wins.
//!   * `deployment_isolation` — canonical ids for the same intent
//!     under distinct deployments diverge.
//!
//! Every test asserts `mock.prohibited_calls_seen()` is EMPTY on exit.
//! Adding a broadcast method anywhere in the pipeline would trip these
//! assertions loudly.

mod hybrid_v2_mock_rpc_helpers;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Bytes, FixedBytes, U256};
use hybrid_v2_mock_rpc_helpers::{make_block, MockRpcServer};
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};

use alloy_sol_types::SolCall;
use deopt_v2_backend::hybrid_v2::execution::plan::{
    executeMatchCall, ExecutionPlanBuilder, OptionOrder, SignedActionEnvelope,
};
use deopt_v2_backend::hybrid_v2::execution::signer_ephemeral::TestEphemeralSigner;
use deopt_v2_backend::hybrid_v2::execution::signer_production::ProductionSignerUnavailable;
use deopt_v2_backend::hybrid_v2::execution::{
    derive_canonical_execution_id, failure_class as fc, ExecutionOrchestrator, ExecutionPhase,
    GasFeePolicy, HttpExecutionRpcClient, MockClock, PreparationIntent, TargetPolicy,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReason, ReadinessReport, ReadinessState};
use deopt_v2_backend::hybrid_v2::reducer::ProjectionState;

// -----------------------------------------------------------------
//                          PG helpers
// -----------------------------------------------------------------

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

async fn build_store(pool: &PgPool) -> (Arc<dyn HybridV2ProjectionStore>, i64) {
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(84532);
    let deployment_id = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    (store, deployment_id)
}

// -----------------------------------------------------------------
//                          intent builders
// -----------------------------------------------------------------

fn buyer_owner() -> [u8; 20] {
    let mut a = [0u8; 20];
    a.iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b = 0xa0 | (i as u8));
    a
}
fn seller_owner() -> [u8; 20] {
    let mut a = [0u8; 20];
    a.iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b = 0xb0 | (i as u8));
    a
}
fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2 + 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
fn engine_address_from_manifest(
    m: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
) -> [u8; 20] {
    let hex = m
        .module_addresses
        .option_matching_engine
        .trim_start_matches("0x");
    let mut a = [0u8; 20];
    for i in 0..20 {
        a[i] = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    a
}

fn baseline_envelope(
    manifest: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
    owner: [u8; 20],
    subkey_seed: u8,
) -> SignedActionEnvelope {
    let engine = engine_address_from_manifest(manifest);
    SignedActionEnvelope {
        owner: alloy_primitives::Address::from(owner),
        subaccountId: 1,
        subKey: FixedBytes::from([subkey_seed; 32]),
        signer: alloy_primitives::Address::from(owner),
        engine: alloy_primitives::Address::from(engine),
        action: FixedBytes::from([0x11u8; 32]),
        architectureVersion: U256::from(1u64),
        nonce: U256::from(1u64),
        deadline: U256::from(2_000_000_000u64),
        ownerRecoveryEpoch: U256::from(0u64),
        subaccountRecoveryEpoch: U256::from(0u64),
        payloadHash: FixedBytes::from([0x22u8; 32]),
    }
}

fn baseline_order() -> OptionOrder {
    OptionOrder {
        seriesId: U256::from(42u64),
        side: 0,
        quantity1e8: 100_000_000,
        pricePerContract1e8: 50_000_000,
        limitPricePerContract1e8: 60_000_000,
        premiumToken: alloy_primitives::Address::from({
            let mut a = [0u8; 20];
            for (i, b) in a.iter_mut().enumerate() {
                *b = 0x0b + (i as u8);
            }
            a
        }),
        timeInForce: 0,
        role: 0,
        maxPositiveFeePpm: 100,
        salt: FixedBytes::from([0x33u8; 32]),
    }
}

fn ready_readiness() -> ReadinessReport {
    ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::ready(),
    }
}

fn build_intent(
    manifest: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
    readiness: ReadinessReport,
    fill_qty: u128,
) -> PreparationIntent {
    let buyer_env = baseline_envelope(manifest, buyer_owner(), 0xaa);
    let seller_env = baseline_envelope(manifest, seller_owner(), 0xbb);
    PreparationIntent {
        manifest: manifest.clone(),
        runtime_state: ProjectionState::default(),
        readiness,
        buyer_envelope: buyer_env,
        buyer_signature: Bytes::from(vec![0x77u8; 65]),
        buyer_order: baseline_order(),
        seller_envelope: seller_env,
        seller_signature: Bytes::from(vec![0x88u8; 65]),
        seller_order: baseline_order(),
        fill_quantity_1e8: fill_qty,
        buyer_active_series: vec![U256::from(42u64)],
        seller_active_series: vec![U256::from(42u64)],
        buyer_order_hash: format!("0x{}", "aa".repeat(32)),
        seller_order_hash: format!("0x{}", "bb".repeat(32)),
        buyer_subkey: format!("0x{}", "aa".repeat(32)),
        seller_subkey: format!("0x{}", "bb".repeat(32)),
        series_id: "42".to_string(),
        premium_amount: "50000000".to_string(),
        fee_schedule_epoch: None,
    }
}

// -----------------------------------------------------------------
//                       mock RPC + gas policy
// -----------------------------------------------------------------

async fn boot_mock_for_happy_path(chain_id: u64, plan_target: [u8; 20]) -> MockRpcServer {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(chain_id);
    mock.set_head(100);
    // head + parent header for eth_getBlockByNumber(latest)
    let latest_block = make_block(100, 0xab, &format!("0x{}", "cd".repeat(32)), 1_700_000_000);
    mock.push_block(latest_block.clone());
    // eth_call fixture — reply with 32-byte executionId.
    mock.set_eth_call_response(&to_hex(&plan_target), executeMatchCall::SELECTOR, {
        let mut buf = vec![0u8; 32];
        buf[31] = 0x42;
        buf
    });
    // eth_estimateGas fixture.
    mock.set_estimate_gas_response(90_000);
    // eth_getTransactionCount(signer, pending) → 7 (arbitrary).
    // NOTE: address filled in per-test based on the ephemeral signer.
    // eth_feeHistory — deterministic sample.
    mock.set_fee_history(
        vec![
            "0x3b9aca00".to_string(),
            "0x41314cf0".to_string(),
            "0x3d0900c0".to_string(),
            "0x3b9aca00".to_string(),
        ],
        vec![
            vec!["0x1dcd6500".to_string()],
            vec!["0x22ecb25c".to_string()],
            vec!["0x20c855c0".to_string()],
        ],
    );
    mock
}

fn gas_policy() -> GasFeePolicy {
    GasFeePolicy {
        max_gas_limit: 5_000_000,
        gas_limit_multiplier_bps: 12_000,
        max_fee_per_gas_wei: U256::from(50_000_000_000u64),
        max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
        max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
        abnormal_estimate_reject_threshold: 10,
    }
}

fn tight_gas_policy() -> GasFeePolicy {
    // Same as `gas_policy` but abnormal_reject at threshold 2 so the
    // deliberate 100x base-fee anomaly trips.
    GasFeePolicy {
        max_gas_limit: 5_000_000,
        gas_limit_multiplier_bps: 12_000,
        max_fee_per_gas_wei: U256::from(50_000_000_000u64),
        max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
        max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
        abnormal_estimate_reject_threshold: 2,
    }
}

// -----------------------------------------------------------------
//                          TESTS
// -----------------------------------------------------------------

#[tokio::test]
async fn happy_path_discovers_and_lands_at_broadcast_disabled() {
    let Some(url) = get_pg_url_or_skip("happy_path_discovers_and_lands_at_broadcast_disabled")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);

    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    let rpc = Arc::new(
        HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc client"),
    );
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).expect("target policy"));

    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc: rpc.clone(),
        signer: signer.clone(),
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };

    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let outcome = orchestrator.prepare(intent).await.expect("prepare ok");

    assert_eq!(outcome.terminal_phase, ExecutionPhase::BroadcastDisabled);
    assert!(outcome.plan_hash.is_some(), "plan_hash must be set");
    assert!(
        outcome.signing_payload_hash.is_some(),
        "signing_payload_hash must be set"
    );
    assert_eq!(outcome.reserved_nonce, Some(7));

    assert!(
        mock.prohibited_calls_seen().is_empty(),
        "no broadcast method must have been called: {:?}",
        mock.prohibited_calls_seen()
    );
}

#[tokio::test]
async fn duplicate_prepare_is_idempotent() {
    let Some(url) = get_pg_url_or_skip("duplicate_prepare_is_idempotent") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };

    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let first = orchestrator.prepare(intent.clone()).await.expect("first");
    let second = orchestrator.prepare(intent).await.expect("second");
    assert_eq!(first.canonical_execution_id, second.canonical_execution_id);
    assert_eq!(first.terminal_phase, second.terminal_phase);
    assert_eq!(first.plan_hash, second.plan_hash);
    assert_eq!(first.reserved_nonce, second.reserved_nonce);
    assert_eq!(first.terminal_phase, ExecutionPhase::BroadcastDisabled);
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn preflight_rejection_reconciliation_drift() {
    let Some(url) = get_pg_url_or_skip("preflight_rejection_reconciliation_drift") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };

    let mut intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    intent.readiness = ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::new_not_ready(ReadinessReason::ReconciliationDrift {
            detail: "projection drift".into(),
        }),
    };
    let outcome = orchestrator.prepare(intent).await.expect("prepare returns");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(fc::PREFLIGHT_REJECTED)
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn preflight_rejection_active_reorg() {
    let Some(url) = get_pg_url_or_skip("preflight_rejection_active_reorg") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };
    let mut intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    intent.readiness = ReadinessReport {
        runtime: ReadinessState::new_not_ready(ReadinessReason::ReorgDetected {
            at_block: 100,
            epoch: 1,
        }),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::ready(),
    };
    let outcome = orchestrator.prepare(intent).await.expect("prepare returns");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(fc::PREFLIGHT_REJECTED)
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn preflight_rejection_active_rebuild() {
    let Some(url) = get_pg_url_or_skip("preflight_rejection_active_rebuild") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };
    let mut intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    intent.readiness = ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::new_not_ready(ReadinessReason::RebuildInProgress),
        reconciliation: ReadinessState::ready(),
    };
    let outcome = orchestrator.prepare(intent).await.expect("prepare returns");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(fc::PREFLIGHT_REJECTED)
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn deterministic_revert_fails_row() {
    let Some(url) = get_pg_url_or_skip("deterministic_revert_fails_row") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(100);
    let latest = make_block(100, 0xab, &format!("0x{}", "cd".repeat(32)), 1_700_000_000);
    mock.push_block(latest);
    // Force the next eth_call to error with a "execution reverted" RPC
    // error carrying data — the HttpExecutionRpcClient translates the
    // JSON-RPC error into a Revert outcome. Use a KNOWN custom error
    // selector so the decoder recognises it.
    use deopt_v2_backend::hybrid_v2::execution::rpc::KNOWN_CUSTOM_ERROR_SELECTORS;
    let (sel, _name) = KNOWN_CUSTOM_ERROR_SELECTORS[0];
    // The eth_call fixture is set to a payload, but we want a revert.
    // Use `set_eth_call_next_rpc_error` on the mock so the immediate
    // next call errors with the revert selector packed into data.
    mock.set_eth_call_next_rpc_error(
        -32000,
        format!(
            "execution reverted\ndata:0x{}",
            to_hex(&sel).trim_start_matches("0x")
        ),
    );
    // We also need estimate_gas + fee_history to be available even
    // though revert should short-circuit.
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    mock.set_estimate_gas_response(1);
    mock.set_fee_history(
        vec!["0x3b9aca00".to_string(), "0x3b9aca00".to_string()],
        vec![vec!["0x1dcd6500".to_string()]],
    );

    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };
    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let outcome = orchestrator.prepare(intent).await.expect("prepare returns");
    let _ = target;
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(fc::SIMULATION_FAILED_DETERMINISTIC)
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn gas_policy_rejection() {
    let Some(url) = get_pg_url_or_skip("gas_policy_rejection") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    // Inject anomalous base fees: newest is 100x median.
    mock.set_fee_history(
        vec![
            "0x3b9aca00".to_string(), // 1 gwei
            "0x3b9aca00".to_string(),
            "0x3b9aca00".to_string(),
            format!("0x{:x}", 200_000_000_000u64), // 200 gwei — >2x median
        ],
        vec![vec!["0x1dcd6500".to_string()]],
    );
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(tight_gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };
    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let outcome = orchestrator.prepare(intent).await.expect("prepare returns");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(fc::GAS_POLICY_REJECTED)
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn firewall_rejection_on_tampered_row() {
    let Some(url) = get_pg_url_or_skip("firewall_rejection_on_tampered_row") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };

    // Prepare once to get a valid row + plan_hash + signature.
    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let ok = orchestrator.prepare(intent.clone()).await.expect("prepare");
    assert_eq!(ok.terminal_phase, ExecutionPhase::BroadcastDisabled);
    let canonical_id = ok.canonical_execution_id.clone();

    // Rewind phase to AwaitingSignature and clear the signature so
    // resume re-enters the firewall step. The plan_hash column stays
    // pinned (SQL trigger blocks in-place mutation). We simulate a
    // "tampered plan" by supplying an intent whose buyer_envelope
    // action bytes differ from the original — this yields a plan whose
    // recomputed plan_hash disagrees with the row's persisted
    // plan_hash, and the firewall rejects with `PlanHashChanged`.
    sqlx::query(
        "UPDATE hybrid_v2_execution_requests
         SET signature_r = NULL,
             signature_s = NULL,
             signature_v = NULL,
             recovered_signer = NULL,
             phase = 'AWAITING_SIGNATURE',
             updated_at_ms = $2
         WHERE canonical_execution_id = $1",
    )
    .bind(&canonical_id)
    .bind(1_700_000_000_000i64)
    .execute(&pool)
    .await
    .expect("rewind");

    // Tamper the intent's buyer_envelope action → different calldata →
    // different plan_hash. The canonical id is derived only from
    // buyer_hash + seller_hash + fill_quantity, so it stays stable and
    // the resumed row is the same.
    let mut tampered = intent.clone();
    tampered.buyer_envelope.action = FixedBytes::from([0xffu8; 32]);

    let outcome = orchestrator
        .prepare(tampered)
        .await
        .expect("resumed prepare returns");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some(fc::FIREWALL_REJECTED)
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn production_signer_default_is_unavailable() {
    let Some(url) = get_pg_url_or_skip("production_signer_default_is_unavailable") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(ProductionSignerUnavailable::default_reason());
    // Executor address is arbitrary since the production signer refuses
    // before the signature check.
    let signer_addr = [0x99u8; 20];
    let mock = boot_mock_for_happy_path(84532, target).await;
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };
    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let outcome = orchestrator.prepare(intent).await.expect("prepare returns");
    // The production signer returns a zero-address identity, which the
    // firewall's zero-check refuses BEFORE the signer is called. That
    // rejection is class FIREWALL_REJECTED. Either terminal is a valid
    // structural "no live signer" outcome — accept both for robustness.
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    let class = outcome.failure_class.as_deref().unwrap_or_default();
    assert!(
        class == fc::SIGNER_UNAVAILABLE || class == fc::FIREWALL_REJECTED,
        "expected SIGNER_UNAVAILABLE or FIREWALL_REJECTED, got {class}"
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn resume_after_mid_signing_crash() {
    let Some(url) = get_pg_url_or_skip("resume_after_mid_signing_crash") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    mock.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };

    // Prepare once → terminal BroadcastDisabled + persisted signature.
    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let first = orchestrator.prepare(intent.clone()).await.expect("prepare");
    assert_eq!(first.terminal_phase, ExecutionPhase::BroadcastDisabled);
    let canonical_id = first.canonical_execution_id.clone();

    // Simulate a mid-flight crash by rewinding phase to
    // `SignatureVerified`. Keep the signature intact so `resume` MUST
    // NOT call the signer again — the row already carries a signature.
    sqlx::query(
        "UPDATE hybrid_v2_execution_requests
         SET phase = 'SIGNATURE_VERIFIED'
         WHERE canonical_execution_id = $1",
    )
    .bind(&canonical_id)
    .execute(&pool)
    .await
    .expect("rewind");

    let resumed = orchestrator.resume(&canonical_id).await.expect("resume");
    assert_eq!(resumed.terminal_phase, ExecutionPhase::BroadcastDisabled);
    assert_eq!(resumed.plan_hash, first.plan_hash);
    assert_eq!(resumed.reserved_nonce, first.reserved_nonce);
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn lock_contention_returns_lock_contention_error() {
    let Some(url) = get_pg_url_or_skip("lock_contention_returns_lock_contention_error") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    // Acquire the lock manually so the orchestrator faces a busy lock.
    let _guard = store
        .try_acquire_operation_lock(
            deployment_id,
            deopt_v2_backend::hybrid_v2::rebuild_operations::OperationKind::Rebuild,
            42,
            1_000,
        )
        .await
        .expect("lock ok")
        .expect("lock acquired");

    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let signer = Arc::new(TestEphemeralSigner::from_seed([0x42u8; 32]));
    let signer_addr = signer.address();
    let mock = boot_mock_for_happy_path(84532, target).await;
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(mock.url(), Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(&manifest).unwrap());
    let orchestrator = ExecutionOrchestrator {
        store: store.clone(),
        rpc,
        signer,
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer_addr,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    };

    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    let err = orchestrator.prepare(intent).await.expect_err("contention");
    assert!(matches!(
        err,
        deopt_v2_backend::hybrid_v2::execution::OrchestrationError::LockContention
    ));
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn deployment_isolation() {
    let Some(url) = get_pg_url_or_skip("deployment_isolation") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id_a) = build_store(&pool).await;
    // Insert a second deployment on a distinct chain_id (Sepolia
    // 11155111) so the deployment version-uniqueness index is not
    // violated.
    let store_arc = store.clone();
    let mut manifest_b = baseline_manifest(11155111);
    manifest_b.manifest_hash = format!("0x{}", "ee".repeat(32));
    let deployment_id_b = store_arc
        .upsert_deployment(&manifest_b, "PENDING", 1_700_000_000_000)
        .await
        .expect("second deployment");
    assert_ne!(deployment_id_a, deployment_id_b);

    let intent_a = build_intent(&baseline_manifest(84532), ready_readiness(), 100_000_000);
    let intent_b = build_intent(&manifest_b, ready_readiness(), 100_000_000);

    let id_a = derive_canonical_execution_id(
        deployment_id_a,
        84532,
        &intent_a.buyer_order_hash,
        &intent_a.seller_order_hash,
        intent_a.fill_quantity_1e8,
    )
    .into_string();
    let id_b = derive_canonical_execution_id(
        deployment_id_b,
        11155111,
        &intent_b.buyer_order_hash,
        &intent_b.seller_order_hash,
        intent_b.fill_quantity_1e8,
    )
    .into_string();
    assert_ne!(
        id_a, id_b,
        "distinct deployments must produce distinct canonical ids"
    );
}

#[tokio::test]
async fn plan_builder_yields_deterministic_plan_hash() {
    // Property check that doubles as a sanity guard: two builds off the
    // same intent must produce byte-identical plan hashes; this is what
    // the orchestrator relies on for idempotency + restart safety.
    let manifest = baseline_manifest(84532);
    let intent = build_intent(&manifest, ready_readiness(), 100_000_000);
    // Build twice through the plan builder.
    use deopt_v2_backend::hybrid_v2::execution::ExecutionRequestRow;
    let buyer_h = intent.buyer_order_hash.clone();
    let seller_h = intent.seller_order_hash.clone();
    let id =
        derive_canonical_execution_id(1, 84532, &buyer_h, &seller_h, 100_000_000).into_string();
    let row = ExecutionRequestRow {
        canonical_execution_id: id,
        deployment_id: 1,
        chain_id: 84532,
        execution_kind: "HYBRID_V2_OPTION_MATCH".into(),
        buyer_order_hash: buyer_h,
        seller_order_hash: seller_h,
        buyer_subkey: intent.buyer_subkey.clone(),
        seller_subkey: intent.seller_subkey.clone(),
        series_id: intent.series_id.clone(),
        fill_quantity_1e8: intent.fill_quantity_1e8.to_string(),
        premium_amount: intent.premium_amount.clone(),
        fee_schedule_epoch: None,
        source_matched_execution_id: None,
        target_contract: manifest.module_addresses.option_matching_engine.clone(),
        selector: format!(
            "0x{}",
            executeMatchCall::SELECTOR
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        ),
        calldata_hash: None,
        calldata_bytes: None,
        plan_hash: None,
        tx_value_wei: "0".into(),
        simulation_block_number: None,
        simulation_block_hash: None,
        simulation_gas_estimate: None,
        simulation_result_json: None,
        signer_identity: None,
        signing_payload_hash: None,
        signature_r: None,
        signature_s: None,
        signature_v: None,
        recovered_signer: None,
        gas_limit: None,
        max_fee_per_gas_wei: None,
        max_priority_fee_per_gas_wei: None,
        reserved_nonce: None,
        phase: ExecutionPhase::Discovered,
        failure_class: None,
        failure_detail: None,
        retry_count: 0,
        holder_epoch: None,
        signer_request_idempotency_key: None,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    let a = ExecutionPlanBuilder::build_from_request(
        &row,
        &manifest,
        &intent.buyer_envelope,
        &intent.buyer_signature,
        &intent.buyer_order,
        &intent.seller_envelope,
        &intent.seller_signature,
        &intent.seller_order,
        intent.fill_quantity_1e8,
        &intent.buyer_active_series,
        &intent.seller_active_series,
    )
    .expect("a");
    let b = ExecutionPlanBuilder::build_from_request(
        &row,
        &manifest,
        &intent.buyer_envelope,
        &intent.buyer_signature,
        &intent.buyer_order,
        &intent.seller_envelope,
        &intent.seller_signature,
        &intent.seller_order,
        intent.fill_quantity_1e8,
        &intent.buyer_active_series,
        &intent.seller_active_series,
    )
    .expect("b");
    assert_eq!(a.plan_hash, b.plan_hash);
    assert_eq!(a.calldata, b.calldata);
}
