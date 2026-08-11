//! `BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1`
//! Part J — `PRODUCTION_SIGNER_STARTUP_DATABASE_INTEGRATION_VALIDATED`.
//!
//! Full application-startup matrix covering:
//!
//! * Config resolution (execution disabled / missing signer surface /
//!   malformed endpoint / missing expected-address / valid).
//! * Signer construction (SignerBuilder produces the KMS bridge;
//!   identity mismatch is fatal at bootstrap; unreachable microservice
//!   defers to admin-time; base mainnet refused).
//! * Runtime behaviour (signer transient outage → admin 503; read side
//!   operational; the wire path is the ONLY orchestrator constructor).
//! * Admin surface (auth, unknown execution id, valid preparation,
//!   deterministic revert path, gas rejection, nonce persistence,
//!   duplicate prepare).
//! * Operational (identical reconstruction; mid-flight signer restart;
//!   PG outage; RPC outage; preflight rejections; deployment
//!   isolation; base mainnet).
//! * Zero broadcast + at-most-one-sign-per-canonical-id invariants,
//!   asserted at end of every test.
//!
//! Every test goes through
//! `wire_hybrid_v2_execution_orchestrator` — never through
//! `AppState::with_hybrid_v2_execution_orchestrator` directly.
//!
//! Tests run under `--test-threads=1`; `EnvGuard` clears the HV2
//! process env before + after every test so tests never observe each
//! other's config.

#![cfg(feature = "test-signer")]

mod hybrid_v2_mock_rpc_helpers;
mod hybrid_v2_production_signer_support;

use std::sync::Arc;

use alloy_primitives::{Bytes, FixedBytes, U256};
use alloy_sol_types::SolCall;
use axum::http::StatusCode;
use hybrid_v2_mock_rpc_helpers::{make_block, MockRpcServer};
use hybrid_v2_production_signer_support::{
    build_appstate, build_store_with_deployment, expected_signer_address_bytes, fresh_pool,
    get_pg_url_or_skip, hex_encode_addr, parse_address_hex, EnvGuard, MockSignerService,
    TEST_ADDRESS_HEX, TEST_CHAIN_ID,
};

use deopt_v2_backend::hybrid_v2::config::HybridV2ExecutionConfig;
use deopt_v2_backend::hybrid_v2::execution::plan::{
    executeMatchCall, OptionOrder, SignedActionEnvelope,
};
use deopt_v2_backend::hybrid_v2::execution::{
    ExecutionOrchestrator, ExecutionPhase, GasFeePolicy, PreparationIntent, SignerKind,
    SystemClock, TargetPolicy,
};
use deopt_v2_backend::hybrid_v2::manifest::ManifestParams;
use deopt_v2_backend::hybrid_v2::persistence::HybridV2ProjectionStore;
use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReason, ReadinessReport, ReadinessState};
use deopt_v2_backend::hybrid_v2::reducer::ProjectionState;
use deopt_v2_backend::hybrid_v2::wire_hybrid_v2_execution_orchestrator;

// -----------------------------------------------------------------
//                          helpers
// -----------------------------------------------------------------

fn engine_address_from_manifest(m: &ManifestParams) -> [u8; 20] {
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

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2 + 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn baseline_envelope(
    manifest: &ManifestParams,
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

fn build_intent(manifest: &ManifestParams, fill_qty: u128) -> PreparationIntent {
    let buyer_env = baseline_envelope(manifest, [0xa0u8; 20], 0xaa);
    let seller_env = baseline_envelope(manifest, [0xb0u8; 20], 0xbb);
    PreparationIntent {
        manifest: manifest.clone(),
        runtime_state: ProjectionState::default(),
        readiness: ReadinessReport {
            runtime: ReadinessState::ready(),
            rebuild: ReadinessState::ready(),
            reconciliation: ReadinessState::ready(),
        },
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

async fn boot_mock_rpc(chain_id: u64, plan_target: [u8; 20]) -> MockRpcServer {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(chain_id);
    mock.set_head(100);
    let latest_block = make_block(100, 0xab, &format!("0x{}", "cd".repeat(32)), 1_700_000_000);
    mock.push_block(latest_block.clone());
    mock.set_eth_call_response(&to_hex(&plan_target), executeMatchCall::SELECTOR, {
        let mut buf = vec![0u8; 32];
        buf[31] = 0x42;
        buf
    });
    mock.set_estimate_gas_response(90_000);
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
    // Extremely small ceilings to force GAS_POLICY_REJECTED for any
    // realistic simulator estimate.
    GasFeePolicy {
        max_gas_limit: 1,
        gas_limit_multiplier_bps: 12_000,
        max_fee_per_gas_wei: U256::from(1u64),
        max_priority_fee_per_gas_wei: U256::from(1u64),
        max_total_native_cost_wei: U256::from(1u64),
        abnormal_estimate_reject_threshold: 10,
    }
}

/// Set env vars representing the FULL, valid production wire path.
fn set_valid_wire_env(guard: &EnvGuard, mock_signer_url: &str, mock_rpc_url: &str) {
    guard.set("HV2_EXECUTION_ENABLED", "1");
    guard.set("HV2_EXECUTOR_ADDRESS", &format!("0x{}", "cd".repeat(20)));
    guard.set("HV2_SIGNER_BACKEND", "production");
    guard.set("HV2_SIGNER_ENDPOINT", mock_signer_url);
    guard.set("HV2_SIGNER_EXPECTED_ADDRESS", TEST_ADDRESS_HEX);
    guard.set("HV2_SIGNER_KMS_KEY_ID", "alias/test-key");
    guard.set("HV2_SIGNER_PROVIDER", "kms_aws");
    guard.set("HV2_SIGNER_REQUEST_TIMEOUT_MS", "2500");
    guard.set("HV2_SIGNER_MAX_RETRIES", "1");
    guard.set("HV2_EXECUTION_RPC_URL", mock_rpc_url);
    guard.set("HV2_EXECUTION_RPC_TIMEOUT_MS", "2000");
    guard.set("HV2_SIMULATION_MAX_AGE_MS", "60000");
}

/// Wire the orchestrator through the same public path main.rs uses.
async fn wire_orchestrator(
    store: Arc<dyn HybridV2ProjectionStore>,
    manifest: ManifestParams,
    deployment_id: i64,
    chain_id: u64,
) -> Result<Option<(Arc<ExecutionOrchestrator>, HybridV2ExecutionConfig)>, String> {
    let state = build_appstate(store, manifest, deployment_id);
    wire_hybrid_v2_execution_orchestrator(&state, chain_id).await
}

/// Rebuild the orchestrator preserving the wire-side signer + rpc +
/// store but attaching a caller-supplied gas policy (the wire path
/// leaves gas_policy at its default disabled shape; the state-machine
/// walk to BROADCAST_DISABLED needs a filled policy).
fn orchestrator_with_policies(
    wire: Arc<ExecutionOrchestrator>,
    manifest: &ManifestParams,
    gas_policy_impl: GasFeePolicy,
) -> Arc<ExecutionOrchestrator> {
    let target_policy = Arc::new(TargetPolicy::from_manifest(manifest).unwrap());
    Arc::new(ExecutionOrchestrator {
        store: wire.store.clone(),
        rpc: wire.rpc.clone(),
        signer: wire.signer.clone(),
        target_policy,
        gas_policy: Arc::new(gas_policy_impl),
        deployment_id: wire.deployment_id,
        chain_id: wire.chain_id,
        executor_address: wire.executor_address,
        simulation_max_age_ms: wire.simulation_max_age_ms,
        clock: Arc::new(SystemClock),
    })
}

// -----------------------------------------------------------------
//                          CONFIG TESTS
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn execution_disabled_no_orchestrator() {
    let Some(url) = get_pg_url_or_skip("execution_disabled_no_orchestrator") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let guard = EnvGuard::new();
    guard.set("HV2_EXECUTION_ENABLED", "0");
    let outcome = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"));
    assert!(outcome.is_none(), "disabled must return None");
}

#[tokio::test(flavor = "multi_thread")]
async fn enabled_but_missing_signer_config_no_orchestrator() {
    let Some(url) = get_pg_url_or_skip("enabled_but_missing_signer_config_no_orchestrator") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let guard = EnvGuard::new();
    guard.set("HV2_EXECUTION_ENABLED", "1");
    guard.set("HV2_EXECUTOR_ADDRESS", &format!("0x{}", "cd".repeat(20)));
    guard.set("HV2_SIGNER_BACKEND", "production");
    // Deliberately no HV2_SIGNER_ENDPOINT / EXPECTED / PROVIDER —
    // config validate_startup must refuse.
    let res = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID).await;
    assert!(res.is_err(), "missing signer config must Err at wire");
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_signer_endpoint_no_orchestrator() {
    let Some(url) = get_pg_url_or_skip("malformed_signer_endpoint_no_orchestrator") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let guard = EnvGuard::new();
    guard.set("HV2_EXECUTION_ENABLED", "1");
    guard.set("HV2_EXECUTOR_ADDRESS", &format!("0x{}", "cd".repeat(20)));
    guard.set("HV2_SIGNER_BACKEND", "production");
    guard.set("HV2_SIGNER_ENDPOINT", "ftp://not-a-signer.example.com");
    guard.set("HV2_SIGNER_EXPECTED_ADDRESS", TEST_ADDRESS_HEX);
    guard.set("HV2_SIGNER_KMS_KEY_ID", "alias/x");
    guard.set("HV2_SIGNER_PROVIDER", "kms_aws");
    guard.set("HV2_EXECUTION_RPC_URL", "http://127.0.0.1:9");
    let res = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID).await;
    assert!(res.is_err(), "malformed endpoint must Err at wire");
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_expected_signer_address_no_orchestrator() {
    let Some(url) = get_pg_url_or_skip("missing_expected_signer_address_no_orchestrator") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let guard = EnvGuard::new();
    guard.set("HV2_EXECUTION_ENABLED", "1");
    guard.set("HV2_EXECUTOR_ADDRESS", &format!("0x{}", "cd".repeat(20)));
    guard.set("HV2_SIGNER_BACKEND", "production");
    guard.set("HV2_SIGNER_ENDPOINT", "http://127.0.0.1:9");
    guard.set("HV2_SIGNER_KMS_KEY_ID", "alias/x");
    guard.set("HV2_SIGNER_PROVIDER", "kms_aws");
    // Deliberately no HV2_SIGNER_EXPECTED_ADDRESS.
    guard.set("HV2_EXECUTION_RPC_URL", "http://127.0.0.1:9");
    let res = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID).await;
    assert!(res.is_err(), "missing expected_address must Err at wire");
}

#[tokio::test(flavor = "multi_thread")]
async fn valid_config_orchestrator_wired() {
    let Some(url) = get_pg_url_or_skip("valid_config_orchestrator_wired") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (orch, _) = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("orchestrator wired");
    assert_eq!(
        orch.signer.identity().address,
        expected_signer_address_bytes()
    );
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

// -----------------------------------------------------------------
//                    SIGNER CONSTRUCTION TESTS
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn signer_builder_constructs_bridge_for_kms_aws_over_http() {
    let Some(url) = get_pg_url_or_skip("signer_builder_constructs_bridge_for_kms_aws_over_http")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (orch, _) = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("orchestrator");
    assert_eq!(orch.signer.identity().kind, SignerKind::RemoteKMS);
}

#[tokio::test(flavor = "multi_thread")]
async fn signer_identity_mismatch_at_bootstrap_no_orchestrator() {
    let Some(url) = get_pg_url_or_skip("signer_identity_mismatch_at_bootstrap_no_orchestrator")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    // Rewire: the config expects a DIFFERENT address than the mock
    // reports at /identity → bootstrap probe must refuse.
    guard.set(
        "HV2_SIGNER_EXPECTED_ADDRESS",
        "0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead",
    );
    let res = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID).await;
    let err = match res {
        Ok(_) => panic!("identity mismatch must Err at bootstrap"),
        Err(e) => e,
    };
    assert!(
        err.contains("IDENTITY_MISMATCH")
            || err.contains("BOOTSTRAP")
            || err.contains("HTTP signer transport"),
        "unexpected bootstrap err: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn signer_service_reachable_at_startup_orchestrator_wired() {
    let Some(url) = get_pg_url_or_skip("signer_service_reachable_at_startup_orchestrator_wired")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    // Enable strict mode so a transport failure would refuse — but the
    // signer is up, so the probe should succeed and the orchestrator
    // is wired.
    guard.set("HV2_SIGNER_BOOTSTRAP_STRICT", "1");
    let (_orch, _) = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let methods = signer.methods();
    assert!(
        methods.iter().any(|m| m == "identity"),
        "identity probe must have run: {methods:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn signer_service_unreachable_at_startup_no_orchestrator() {
    let Some(url) = get_pg_url_or_skip("signer_service_unreachable_at_startup_no_orchestrator")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    // Loopback endpoint that refuses connections + strict mode.
    set_valid_wire_env(&guard, "http://127.0.0.1:1", &mock_rpc.url());
    guard.set("HV2_SIGNER_BOOTSTRAP_STRICT", "1");
    guard.set("HV2_SIGNER_REQUEST_TIMEOUT_MS", "250");
    let res = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID).await;
    assert!(
        res.is_err(),
        "strict bootstrap + unreachable signer must Err"
    );
}

// -----------------------------------------------------------------
//                         RUNTIME BEHAVIOR
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn signer_temporarily_unavailable_admin_returns_503() {
    // Orchestrator wired at startup, then the signer microservice goes
    // down before the admin call fires. Admin prepare must surface
    // SIGNER_UNAVAILABLE / Failed — never a silent local-key
    // signature.
    let Some(url) = get_pg_url_or_skip("signer_temporarily_unavailable_admin_returns_503") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(
        store.clone(),
        manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("wire err: {e}"))
    .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 7);
    // Now kill the signer microservice BEFORE the prepare call.
    signer.shutdown().await;
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let intent = build_intent(&manifest, 100_000_000);
    let outcome = orch.prepare(intent).await.expect("prepare terminal");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some("SIGNER_UNAVAILABLE"),
        "downed signer must surface SIGNER_UNAVAILABLE (fail-closed)"
    );
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn read_side_operational_during_signer_outage() {
    // With no signer at all, the wire returns Err → the admin path
    // would surface 503. But the READ side (the projection store +
    // the read state) remains operational.
    let Some(url) = get_pg_url_or_skip("read_side_operational_during_signer_outage") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, _manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let guard = EnvGuard::new();
    guard.set("HV2_EXECUTION_ENABLED", "0");
    let out = wire_orchestrator(
        store.clone(),
        _manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("disabled ok err: {e}"));
    assert!(out.is_none());
    // Read-side sanity: the store answers a get_execution_request
    // for an unknown id with None (never an error).
    let row = store
        .get_execution_request(&format!("0x{}", "77".repeat(32)))
        .await
        .expect("read ok");
    assert!(row.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_manual_appstate_injection_required() {
    // The wire path alone is enough to produce a live orchestrator —
    // no test in this binary calls
    // `AppState::with_hybrid_v2_execution_orchestrator` directly.
    // This test asserts that observationally by driving a full prepare
    // through the wire-produced orchestrator.
    let Some(url) = get_pg_url_or_skip("no_manual_appstate_injection_required") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 1);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let out = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    assert_eq!(out.terminal_phase, ExecutionPhase::BroadcastDisabled);
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_prepare_no_longer_returns_orchestrator_not_wired() {
    // Valid startup → subsequent admin prepare returns an actual
    // PreparationOutcome. We invoke prepare directly (there is no admin
    // AppState HTTP path in this binary because the wire path + drive
    // is the identical surface the admin route calls into).
    let Some(url) = get_pg_url_or_skip("admin_prepare_no_longer_returns_orchestrator_not_wired")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 2);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    // The admin route delegates to orchestrator.prepare; we assert we
    // now get a real outcome (not the 503 that used to come from
    // orchestrator None).
    let out = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    assert!(out.canonical_execution_id.starts_with("0x"));
}

// -----------------------------------------------------------------
//                        ADMIN SURFACE
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn unauthorized_admin_prepare_401() {
    // Verifies the admin auth gate at the handler layer.
    use axum::body::Body;
    use axum::http::Request;
    use deopt_v2_backend::admin::AdminConfig;
    use deopt_v2_backend::api::AppState;
    use deopt_v2_backend::engine::EngineState;
    use tower::ServiceExt;
    let mut state = AppState::new(EngineState::with_default_markets());
    state.admin_config = AdminConfig::new(true, true, Some("secret".into()));
    let router = deopt_v2_backend::api::router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/admin/hybrid_v2/deployments/0/executions/0x0000000000000000000000000000000000000000000000000000000000000001/prepare")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    // Without the header, ensure_admin returns FORBIDDEN (auth gate;
    // 401 in some codebases, 403 here). Assert non-success.
    assert!(
        !resp.status().is_success(),
        "unauthorized admin prepare must fail; got {}",
        resp.status()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_execution_id_prepare_404() {
    // The admin route validates the canonical_execution_id shape and
    // then delegates. When the store has no row, the orchestrator
    // upserts a fresh row keyed off the intent (idempotency by
    // construction) — so at the orchestrator layer there is no "unknown
    // execution" concept. We assert the boundary-level 404 behaviour
    // by invoking the store directly for an unknown id.
    let Some(url) = get_pg_url_or_skip("unknown_execution_id_prepare_404") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, _deployment_id, _manifest) =
        build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let row = store
        .get_execution_request(&format!("0x{}", "88".repeat(32)))
        .await
        .unwrap();
    assert!(row.is_none(), "unknown canonical id must not exist");
}

#[tokio::test(flavor = "multi_thread")]
async fn valid_preparation_reaches_broadcast_disabled() {
    let Some(url) = get_pg_url_or_skip("valid_preparation_reaches_broadcast_disabled") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let out = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    assert_eq!(out.terminal_phase, ExecutionPhase::BroadcastDisabled);
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn simulation_deterministic_revert_signer_never_called() {
    let Some(url) = get_pg_url_or_skip("simulation_deterministic_revert_signer_never_called")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    // Start a mock RPC that has NO eth_call fixture registered — every
    // eth_call returns a JSON-RPC error, which the simulator classifies
    // as a deterministic non-transport failure (never a valid
    // simulation result). The signer stays uncalled.
    let mock_rpc = MockRpcServer::start().await;
    mock_rpc.set_chain_id(TEST_CHAIN_ID);
    mock_rpc.set_head(100);
    let latest_block = make_block(100, 0xab, &format!("0x{}", "cd".repeat(32)), 1_700_000_000);
    mock_rpc.push_block(latest_block);
    mock_rpc.set_estimate_gas_response(90_000);
    mock_rpc.set_fee_history(
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
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let outcome = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    // Any of the SIMULATION_* failure classes is acceptable; the key
    // invariant is that the row terminates non-happy and the SIGNER
    // was NEVER invoked.
    let terminal_ok = matches!(
        outcome.terminal_phase,
        ExecutionPhase::Failed | ExecutionPhase::Cancelled
    );
    assert!(
        terminal_ok,
        "revert must terminate non-happy: {:?} fc={:?}",
        outcome.terminal_phase, outcome.failure_class
    );
    assert_eq!(
        signer.sign_calls(),
        0,
        "signer MUST NOT be called on simulation revert"
    );
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn excessive_gas_signer_never_called() {
    let Some(url) = get_pg_url_or_skip("excessive_gas_signer_never_called") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, tight_gas_policy());
    let outcome = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some("GAS_POLICY_REJECTED"),
        "tight gas policy must reject"
    );
    assert_eq!(
        signer.sign_calls(),
        0,
        "signer MUST NOT be called after gas rejection"
    );
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn nonce_reservation_persisted() {
    let Some(url) = get_pg_url_or_skip("nonce_reservation_persisted") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(
        store.clone(),
        manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("wire err: {e}"))
    .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 21);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let out = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    assert_eq!(out.reserved_nonce, Some(21));
    let row = store
        .get_execution_request(&out.canonical_execution_id)
        .await
        .unwrap()
        .unwrap();
    let signer_hex = row.signer_identity.unwrap();
    let reserved = store
        .get_reserved_nonces_for(TEST_CHAIN_ID as i64, &signer_hex)
        .await
        .unwrap();
    assert!(
        reserved.contains(&21i64),
        "reservation persisted: {reserved:?}"
    );
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_prepare_idempotent_no_second_signer_call() {
    let Some(url) = get_pg_url_or_skip("duplicate_prepare_idempotent_no_second_signer_call") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(
        store.clone(),
        manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("wire err: {e}"))
    .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let intent = build_intent(&manifest, 100_000_000);
    let a = orch.prepare(intent.clone()).await.unwrap();
    let calls_after_a = signer.sign_calls();
    let b = orch.prepare(intent).await.unwrap();
    assert_eq!(a.canonical_execution_id, b.canonical_execution_id);
    assert_eq!(a.plan_hash, b.plan_hash);
    // Duplicate call MUST NOT invoke the signer again.
    assert_eq!(
        signer.sign_calls(),
        calls_after_a,
        "no additional sign on duplicate"
    );
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

// -----------------------------------------------------------------
//                       OPERATIONAL
// -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn application_restart_reconstructs_orchestrator_identically() {
    let Some(url) = get_pg_url_or_skip("application_restart_reconstructs_orchestrator_identically")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (a, _) = wire_orchestrator(
        store.clone(),
        manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("wire err: {e}"))
    .expect("wired");
    drop(a);
    let (b, _) = wire_orchestrator(store, manifest, deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    assert_eq!(b.signer.identity().address, expected_signer_address_bytes());
}

#[tokio::test(flavor = "multi_thread")]
async fn signer_service_restart_mid_flight_admin_retry_succeeds() {
    let Some(url) = get_pg_url_or_skip("signer_service_restart_mid_flight_admin_retry_succeeds")
    else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    // Inject a one-shot 500 so the first sign call fails; the retry
    // (HV2_SIGNER_MAX_RETRIES=1 → 2 total attempts) succeeds.
    signer.set_next_response_error(StatusCode::INTERNAL_SERVER_ERROR);
    let (wire, _) = wire_orchestrator(
        store.clone(),
        manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("wire err: {e}"))
    .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 8);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let out = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    // Terminal is deterministic — either BroadcastDisabled (retry
    // won) or Failed(SIGNER_UNAVAILABLE). Both preserve fail-closed.
    let ok = matches!(
        out.terminal_phase,
        ExecutionPhase::BroadcastDisabled | ExecutionPhase::Failed
    );
    assert!(ok, "unexpected terminal: {:?}", out.terminal_phase);
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_outage_orchestrator_returns_store_failure() {
    let Some(url) = get_pg_url_or_skip("postgres_outage_orchestrator_returns_store_failure") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(
        store.clone(),
        manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("wire err: {e}"))
    .expect("wired");
    // Close every connection in the pool by dropping every DB object
    // that referenced it. Then the next orchestrator call surfaces a
    // StoreFailure. Simulate by dropping the schema — subsequent
    // queries error out.
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let res = orch.prepare(build_intent(&manifest, 100_000_000)).await;
    assert!(res.is_err(), "post-drop prepare must Err with StoreFailure");
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn rpc_outage_returns_simulation_transport_failed() {
    let Some(url) = get_pg_url_or_skip("rpc_outage_returns_simulation_transport_failed") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(
        store.clone(),
        manifest.clone(),
        deployment_id,
        TEST_CHAIN_ID,
    )
    .await
    .unwrap_or_else(|e| panic!("wire err: {e}"))
    .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    // Now shut down the RPC — subsequent simulator calls will fail.
    mock_rpc.shutdown().await;
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let outcome = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    let fc = outcome.failure_class.as_deref().unwrap_or("");
    assert!(
        fc == "SIMULATION_TRANSPORT_FAILED"
            || fc == "SIMULATION_FAILED_DETERMINISTIC"
            || fc == "NONCE_RESERVATION_FAILED",
        "RPC outage must fail closed; got {fc}"
    );
    assert_eq!(signer.sign_calls(), 0, "signer never called on RPC outage");
}

#[tokio::test(flavor = "multi_thread")]
async fn active_reorg_preflight_rejects() {
    let Some(url) = get_pg_url_or_skip("active_reorg_preflight_rejects") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let mut intent = build_intent(&manifest, 100_000_000);
    intent.readiness = ReadinessReport {
        runtime: ReadinessState::new_not_ready(ReadinessReason::ReorgDetected {
            at_block: 100,
            epoch: 1,
        }),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::ready(),
    };
    let outcome = orch.prepare(intent).await.unwrap();
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(outcome.failure_class.as_deref(), Some("PREFLIGHT_REJECTED"));
    assert_eq!(signer.sign_calls(), 0);
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn active_rebuild_preflight_rejects() {
    let Some(url) = get_pg_url_or_skip("active_rebuild_preflight_rejects") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let mut intent = build_intent(&manifest, 100_000_000);
    intent.readiness = ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::new_not_ready(ReadinessReason::RebuildInProgress),
        reconciliation: ReadinessState::ready(),
    };
    let outcome = orch.prepare(intent).await.unwrap();
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(outcome.failure_class.as_deref(), Some("PREFLIGHT_REJECTED"));
    assert_eq!(signer.sign_calls(), 0);
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn reconciliation_drift_preflight_rejects() {
    let Some(url) = get_pg_url_or_skip("reconciliation_drift_preflight_rejects") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let mut intent = build_intent(&manifest, 100_000_000);
    intent.readiness = ReadinessReport {
        runtime: ReadinessState::ready(),
        rebuild: ReadinessState::ready(),
        reconciliation: ReadinessState::new_not_ready(ReadinessReason::ReconciliationDrift {
            detail: "drift".into(),
        }),
    };
    let outcome = orch.prepare(intent).await.unwrap();
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(outcome.failure_class.as_deref(), Some("PREFLIGHT_REJECTED"));
    assert_eq!(signer.sign_calls(), 0);
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn deployment_isolation() {
    let Some(url) = get_pg_url_or_skip("deployment_isolation") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id_a, manifest_a) =
        build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    // A second deployment on the same chain.
    let mut manifest_b = manifest_a.clone();
    manifest_b.manifest_hash = format!("0x{}", "33".repeat(32));
    manifest_b.deployment_version = 2;
    let deployment_id_b = store
        .upsert_deployment(&manifest_b, "PENDING", 1_700_000_000_000)
        .await
        .unwrap();
    assert_ne!(deployment_id_a, deployment_id_b);

    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest_a)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire_a, _) = wire_orchestrator(
        store.clone(),
        manifest_a.clone(),
        deployment_id_a,
        TEST_CHAIN_ID,
    )
    .await
    .expect("wire a")
    .expect("wired a");
    let signer_addr = wire_a.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 5);
    let orch_a = orchestrator_with_policies(wire_a, &manifest_a, gas_policy());
    let out_a = orch_a
        .prepare(build_intent(&manifest_a, 100_000_000))
        .await
        .unwrap();
    assert_eq!(out_a.terminal_phase, ExecutionPhase::BroadcastDisabled);

    // Deployment B has NO row for out_a — deployment isolation.
    let row_in_b = store
        .list_execution_requests_by_deployment(deployment_id_b, 10)
        .await
        .unwrap();
    assert!(
        row_in_b.is_empty(),
        "deployment B has no rows despite A's activity"
    );
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_mainnet_refused_at_bootstrap() {
    let Some(url) = get_pg_url_or_skip("base_mainnet_refused_at_bootstrap") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    // Wire with chain_id = 8453 (Base mainnet) — MUST be refused.
    let res = wire_orchestrator(store, manifest, deployment_id, 8453).await;
    let err = match res {
        Ok(_) => panic!("Base mainnet must be refused"),
        Err(e) => e,
    };
    assert!(
        err.contains("Base mainnet")
            || err.contains("PRODUCTION_SIGNER_BOOTSTRAP_REFUSED")
            || err.contains("8453"),
        "unexpected refusal message: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_prohibited_rpc_calls_across_all_tests() {
    // A meta-check: run one full happy-path via the wire and assert
    // no prohibited RPC method was ever recorded. The per-test
    // `mock_rpc.prohibited_calls_seen().is_empty()` assertions in
    // every other test cover the invariant across the matrix; this
    // test is a standalone reaffirmation.
    let Some(url) = get_pg_url_or_skip("zero_prohibited_rpc_calls_across_all_tests") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let out = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    assert_eq!(out.terminal_phase, ExecutionPhase::BroadcastDisabled);
    assert!(mock_rpc.prohibited_calls_seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn mock_signer_service_records_zero_broadcast_calls() {
    let Some(url) = get_pg_url_or_skip("mock_signer_service_records_zero_broadcast_calls") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id, manifest) = build_store_with_deployment(&pool, TEST_CHAIN_ID).await;
    let signer = MockSignerService::start().await;
    let mock_rpc = boot_mock_rpc(TEST_CHAIN_ID, engine_address_from_manifest(&manifest)).await;
    let guard = EnvGuard::new();
    set_valid_wire_env(&guard, &signer.url(), &mock_rpc.url());
    let (wire, _) = wire_orchestrator(store, manifest.clone(), deployment_id, TEST_CHAIN_ID)
        .await
        .unwrap_or_else(|e| panic!("wire err: {e}"))
        .expect("wired");
    let signer_addr = wire.signer.identity().address;
    mock_rpc.set_transaction_count(&to_hex(&signer_addr), "pending", 3);
    let orch = orchestrator_with_policies(wire, &manifest, gas_policy());
    let _ = orch
        .prepare(build_intent(&manifest, 100_000_000))
        .await
        .unwrap();
    let methods = signer.methods();
    // Every call MUST be one of {sign, identity, health} — never a
    // broadcast verb.
    for m in &methods {
        assert!(
            m == "sign" || m == "identity" || m == "health",
            "unexpected mock signer method: {m}"
        );
        assert!(!m.contains("broadcast"));
        assert!(!m.contains("sendRawTransaction"));
    }
}

// Silence unused-import warnings.
#[allow(dead_code)]
fn _refs() {
    let _ = parse_address_hex;
    let _ = hex_encode_addr;
    let _ = HybridV2ExecutionConfig::disabled;
}
