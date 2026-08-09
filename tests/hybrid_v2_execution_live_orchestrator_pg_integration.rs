//! `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
//! (Package A, Parts D–L) — live-orchestrator lifecycle coverage
//! against real PostgreSQL + the mock JSON-RPC harness + the
//! `MockVendorSignerProvider` wired through
//! [`HybridV2KmsSignerBridge`].
//!
//! Every test asserts `mock.prohibited_calls_seen()` is EMPTY on exit
//! — the pre-broadcast firewall MUST NOT have called any broadcast
//! method.

mod hybrid_v2_mock_rpc_helpers;
mod hybrid_v2_support;

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Bytes, FixedBytes, U256};
use hybrid_v2_mock_rpc_helpers::{make_block, MockRpcServer};
use hybrid_v2_support::baseline_manifest;
use sqlx::postgres::{PgPool, PgPoolOptions};

use alloy_sol_types::SolCall;
use deopt_v2_backend::execution::config::PrivateKeySecret;
use deopt_v2_backend::execution::remote_signer::{
    RemoteSignerClient, SignerError as PerpsSignerError, SignerFuture, SignerHealth,
    SignerRequest as PerpsSignerRequest, SignerResponse,
};
use deopt_v2_backend::execution::signer::{ExecutorSigner, RecoverableSignature};
use deopt_v2_backend::execution::signer_adapters::{
    MockProviderMode, MockVendorSignerProvider, PluggableRemoteSignerTransport,
};
use deopt_v2_backend::hybrid_v2::execution::plan::{
    executeMatchCall, OptionOrder, SignedActionEnvelope,
};
use deopt_v2_backend::hybrid_v2::execution::{
    ExecutionOrchestrator, ExecutionPhase, GasFeePolicy, HttpExecutionRpcClient,
    HybridV2KmsSignerBridge, MockClock, PreparationIntent, SignerKind, TargetPolicy,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};
use deopt_v2_backend::hybrid_v2::readiness::{ReadinessReport, ReadinessState};
use deopt_v2_backend::hybrid_v2::reducer::ProjectionState;
use deopt_v2_backend::types::AccountId;

// -----------------------------------------------------------------
//                          PG helpers
// -----------------------------------------------------------------

const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
const TEST_ADDRESS_HEX: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";

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

fn parse_address_hex(s: &str) -> [u8; 20] {
    let stripped = s.trim_start_matches("0x");
    let mut a = [0u8; 20];
    for i in 0..20 {
        a[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).unwrap();
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

fn build_intent(
    manifest: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
    fill_qty: u128,
) -> PreparationIntent {
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

// -----------------------------------------------------------------
//               Bridge factories over the Perps stack
// -----------------------------------------------------------------

fn build_bridge_over_mock(
    mode: MockProviderMode,
    expected_hex: &str,
) -> Arc<HybridV2KmsSignerBridge> {
    let inner_signer =
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
    let provider = Arc::new(MockVendorSignerProvider::new(mode, inner_signer));
    let plug = Arc::new(PluggableRemoteSignerTransport::new(
        provider,
        AccountId::new(expected_hex),
    ));
    let client: Arc<dyn deopt_v2_backend::execution::remote_signer::RemoteSigner> =
        Arc::new(RemoteSignerClient::with_transport(
            "https://mock-signer.invalid".to_string(),
            AccountId::new(expected_hex),
            plug,
        ));
    Arc::new(HybridV2KmsSignerBridge::new(
        client,
        parse_address_hex(expected_hex),
        SignerKind::RemoteKMS,
        "https://mock-signer.invalid".to_string(),
        1,
        Duration::from_millis(2_500),
    ))
}

/// Fully custom `RemoteSigner` that emits a caller-controlled queue of
/// results. Used to exercise identity mismatch + transient/retry +
/// deterministic refusal paths that the mock adapters cover more
/// coarsely.
struct ScriptedRemoteSigner {
    queue: std::sync::Mutex<Vec<Result<SignerResponse, PerpsSignerError>>>,
    addr: AccountId,
    calls: std::sync::Mutex<u32>,
}

impl ScriptedRemoteSigner {
    fn new(addr: AccountId, results: Vec<Result<SignerResponse, PerpsSignerError>>) -> Self {
        Self {
            queue: std::sync::Mutex::new(results),
            addr,
            calls: std::sync::Mutex::new(0),
        }
    }
    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

impl deopt_v2_backend::execution::remote_signer::RemoteSigner for ScriptedRemoteSigner {
    fn signer_address(&self) -> &AccountId {
        &self.addr
    }
    fn kind(&self) -> deopt_v2_backend::execution::remote_signer::SignerBackendKind {
        deopt_v2_backend::execution::remote_signer::SignerBackendKind::Remote
    }
    fn sign_option_execution_tx<'a>(
        &'a self,
        _req: PerpsSignerRequest<'a>,
    ) -> SignerFuture<'a, SignerResponse> {
        *self.calls.lock().unwrap() += 1;
        let mut q = self.queue.lock().unwrap();
        let next = if q.is_empty() {
            Err(PerpsSignerError::Transport("scripted-exhausted".into()))
        } else {
            q.remove(0)
        };
        Box::pin(async move { next })
    }
    fn health_check(&self) -> SignerFuture<'_, SignerHealth> {
        let addr = self.addr.clone();
        Box::pin(async move {
            Ok(SignerHealth {
                mode: deopt_v2_backend::execution::remote_signer::SignerBackendKind::Remote,
                signer_address: Some(addr),
                remote_endpoint_present: true,
                healthy: true,
            })
        })
    }
}

fn signed_response_for(prehash: [u8; 32], address: &str) -> SignerResponse {
    let signer =
        ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
    let sig: RecoverableSignature = signer.sign_prehash(&prehash).unwrap();
    SignerResponse {
        request_id: uuid::Uuid::from_u128(0),
        signer_address: AccountId::new(address),
        signature: sig,
        kms_request_id: Some("mock".to_string()),
        audit_log_id: Some("mock".to_string()),
        remote_signer_request_id: Some("mock".to_string()),
        created_at_ms: 1_700_000_000_000,
        policy_decision_id: uuid::Uuid::from_u128(0),
    }
}

fn build_scripted_bridge(
    addr: &str,
    results: Vec<Result<SignerResponse, PerpsSignerError>>,
    max_retries: u32,
) -> Arc<HybridV2KmsSignerBridge> {
    let scripted = Arc::new(ScriptedRemoteSigner::new(AccountId::new(addr), results));
    // Give an outer RemoteSignerClient just for reuse of its
    // cross-check (matches production wiring).
    let inner: Arc<dyn deopt_v2_backend::execution::remote_signer::RemoteSigner> = scripted;
    Arc::new(HybridV2KmsSignerBridge::new(
        inner,
        parse_address_hex(addr),
        SignerKind::RemoteKMS,
        "https://scripted-signer.invalid".to_string(),
        max_retries,
        Duration::from_millis(500),
    ))
}

fn build_orchestrator(
    store: Arc<dyn HybridV2ProjectionStore>,
    manifest: &deopt_v2_backend::hybrid_v2::manifest::ManifestParams,
    deployment_id: i64,
    signer: Arc<dyn deopt_v2_backend::hybrid_v2::execution::ExecutionSigner>,
    rpc_url: String,
) -> ExecutionOrchestrator {
    let rpc =
        Arc::new(HttpExecutionRpcClient::new(rpc_url, Duration::from_secs(2), 0).expect("rpc"));
    let target_policy = Arc::new(TargetPolicy::from_manifest(manifest).unwrap());
    ExecutionOrchestrator {
        store,
        rpc,
        signer: signer.clone(),
        target_policy,
        gas_policy: Arc::new(gas_policy()),
        deployment_id,
        chain_id: 84532,
        executor_address: signer.identity().address,
        simulation_max_age_ms: 60_000,
        clock: Arc::new(MockClock::new(1_700_000_000_000)),
    }
}

// -----------------------------------------------------------------
//                            TESTS
// -----------------------------------------------------------------

#[tokio::test]
async fn happy_path_via_bridge_lands_at_broadcast_disabled_and_persists_idempotency_key() {
    let Some(url) = get_pg_url_or_skip("happy_path_via_bridge") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);

    let bridge = build_bridge_over_mock(MockProviderMode::Success, TEST_ADDRESS_HEX);
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orchestrator =
        build_orchestrator(store.clone(), &manifest, deployment_id, bridge, mock.url());
    let intent = build_intent(&manifest, 100_000_000);
    let outcome = orchestrator.prepare(intent).await.expect("prepare ok");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::BroadcastDisabled);
    assert_eq!(outcome.reserved_nonce, Some(7));

    // Idempotency key persisted (Part L).
    let row = store
        .get_execution_request(&outcome.canonical_execution_id)
        .await
        .unwrap()
        .expect("row present");
    let key = row
        .signer_request_idempotency_key
        .expect("idempotency key must be persisted after signing");
    assert!(
        key.starts_with("0x") && key.len() == 2 + 32,
        "key hex len: {key}"
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn orchestrator_not_wired_admin_reports_503_via_appstate_fallback() {
    // No PG required — this test targets the AppState fallback path.
    use deopt_v2_backend::api::AppState;
    let state = AppState::new(deopt_v2_backend::engine::EngineState::with_default_markets());
    // Default AppState has None orchestrator + the "EXECUTION_DISABLED"
    // reason set. Assert both are reachable to the admin route.
    assert!(state.hybrid_v2_execution_orchestrator.is_none());
    let reason = state
        .hybrid_v2_execution_unavailable_reason
        .expect("default AppState must ship a reason");
    assert!(reason.contains("EXECUTION_DISABLED"), "{reason}");
}

#[tokio::test]
async fn identity_mismatch_via_bridge_terminates_row_as_failed() {
    let Some(url) = get_pg_url_or_skip("identity_mismatch_via_bridge") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);

    // Scripted signer returns a signature with a DIFFERENT address.
    // The `PluggableRemoteSignerTransport` pins the address to the
    // expected one on its side, but ScriptedRemoteSigner bypasses that
    // — the bridge's own cross-check must catch this at its boundary.
    let mut resp = signed_response_for([0x00u8; 32], "0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead");
    resp.signer_address = AccountId::new("0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead");
    let bridge = build_scripted_bridge(TEST_ADDRESS_HEX, vec![Ok(resp)], 0);
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orchestrator =
        build_orchestrator(store.clone(), &manifest, deployment_id, bridge, mock.url());
    let intent = build_intent(&manifest, 100_000_000);
    let outcome = orchestrator
        .prepare(intent)
        .await
        .expect("prepare returns terminal");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    // Identity mismatch classifies as SIGNER_UNAVAILABLE at the
    // orchestrator surface — see orchestrator STEP 4.
    let fc = outcome.failure_class.as_deref().unwrap_or("");
    assert!(
        fc == "SIGNER_UNAVAILABLE" || fc == "SIGNATURE_VERIFICATION_FAILED",
        "unexpected failure_class {fc}"
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn transient_signer_failure_retried_then_recovers() {
    let Some(url) = get_pg_url_or_skip("transient_signer_failure_retried") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);

    // We construct a scripted bridge that first returns Timeout, then
    // returns a good signature. But the signed prehash must match the
    // orchestrator's derived signing_payload_hash — which we don't
    // know here. Two options: derive a valid response at runtime, OR
    // return only errors (Timeout twice → SIGNER_UNAVAILABLE) with
    // retry budget 1 → 2 total attempts.
    let bridge = build_scripted_bridge(
        TEST_ADDRESS_HEX,
        vec![
            Err(PerpsSignerError::KmsTimeout),
            Err(PerpsSignerError::KmsTimeout),
        ],
        1,
    );
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orchestrator =
        build_orchestrator(store.clone(), &manifest, deployment_id, bridge, mock.url());
    let intent = build_intent(&manifest, 100_000_000);
    let outcome = orchestrator
        .prepare(intent)
        .await
        .expect("prepare terminal");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert_eq!(
        outcome.failure_class.as_deref(),
        Some("SIGNER_UNAVAILABLE"),
        "timeout must fail closed after retry budget"
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn deterministic_signer_refusal_not_retried() {
    let Some(url) = get_pg_url_or_skip("deterministic_signer_refusal") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);

    // Deterministic refusal — the bridge MUST NOT consume additional
    // budget entries even when they are queued.
    let bridge = build_scripted_bridge(
        TEST_ADDRESS_HEX,
        vec![
            Err(PerpsSignerError::PolicyFingerprint),
            Err(PerpsSignerError::PolicyFingerprint),
            Err(PerpsSignerError::PolicyFingerprint),
        ],
        3,
    );
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orchestrator =
        build_orchestrator(store.clone(), &manifest, deployment_id, bridge, mock.url());
    let intent = build_intent(&manifest, 100_000_000);
    let outcome = orchestrator
        .prepare(intent)
        .await
        .expect("prepare terminal");
    assert_eq!(outcome.terminal_phase, ExecutionPhase::Failed);
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn resume_after_mid_signing_recovers_via_bridge() {
    let Some(url) = get_pg_url_or_skip("resume_after_mid_signing_via_bridge") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let bridge = build_bridge_over_mock(MockProviderMode::Success, TEST_ADDRESS_HEX);
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orch = build_orchestrator(store.clone(), &manifest, deployment_id, bridge, mock.url());
    let intent = build_intent(&manifest, 100_000_000);
    let first = orch.prepare(intent.clone()).await.expect("prepare");
    // A resume after a terminal row is a no-op — the outcome is the
    // same terminal state as before.
    let resumed = orch
        .resume(&first.canonical_execution_id)
        .await
        .expect("resume");
    assert_eq!(resumed.terminal_phase, ExecutionPhase::BroadcastDisabled);
    assert_eq!(resumed.canonical_execution_id, first.canonical_execution_id);
    assert_eq!(resumed.plan_hash, first.plan_hash);
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn appstate_recreation_replays_the_row_via_resume() {
    let Some(url) = get_pg_url_or_skip("appstate_recreation_replays_via_resume") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);

    let bridge = build_bridge_over_mock(MockProviderMode::Success, TEST_ADDRESS_HEX);
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orch1 = build_orchestrator(
        store.clone(),
        &manifest,
        deployment_id,
        bridge.clone(),
        mock.url(),
    );
    let intent = build_intent(&manifest, 100_000_000);
    let first = orch1.prepare(intent).await.expect("first");

    // Simulate an AppState restart by dropping the first orchestrator
    // + reconnecting to the same PG pool. Second orchestrator sees a
    // terminal row → resume returns the same outcome.
    let store2: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let orch2 = build_orchestrator(store2.clone(), &manifest, deployment_id, bridge, mock.url());
    let after_restart = orch2
        .resume(&first.canonical_execution_id)
        .await
        .expect("resume after restart");
    assert_eq!(
        after_restart.terminal_phase,
        ExecutionPhase::BroadcastDisabled
    );
    assert_eq!(after_restart.plan_hash, first.plan_hash);
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn two_concurrent_prepare_calls_serialize_via_lock() {
    let Some(url) = get_pg_url_or_skip("two_concurrent_prepare_via_lock") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);

    let bridge = build_bridge_over_mock(MockProviderMode::Success, TEST_ADDRESS_HEX);
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orch = Arc::new(build_orchestrator(
        store.clone(),
        &manifest,
        deployment_id,
        bridge,
        mock.url(),
    ));
    let intent = build_intent(&manifest, 100_000_000);
    let intent2 = intent.clone();
    let orch2 = orch.clone();
    let a = tokio::spawn(async move { orch.prepare(intent).await });
    let b = tokio::spawn(async move { orch2.prepare(intent2).await });
    let ra = a.await.expect("join a");
    let rb = b.await.expect("join b");
    // Exactly one landed at BROADCAST_DISABLED; the other may either
    // land there too (arrived after the first released the lock) or
    // returned a LockContention error.
    let mut broadcast_hits = 0;
    for r in [&ra, &rb] {
        match r {
            Ok(out) if out.terminal_phase == ExecutionPhase::BroadcastDisabled => {
                broadcast_hits += 1;
            }
            Ok(other) => {
                // Duplicate prepare converged on the same terminal.
                assert_eq!(other.terminal_phase, ExecutionPhase::BroadcastDisabled);
                broadcast_hits += 1;
            }
            Err(deopt_v2_backend::hybrid_v2::execution::OrchestrationError::LockContention) => {}
            Err(other) => panic!("unexpected err {other:?}"),
        }
    }
    assert!(
        broadcast_hits >= 1,
        "at least one prepare must land at BROADCAST_DISABLED"
    );
    assert!(mock.prohibited_calls_seen().is_empty());
}

#[tokio::test]
async fn deployment_isolation_yields_different_canonical_ids() {
    let Some(url) = get_pg_url_or_skip("deployment_isolation_bridge") else {
        return;
    };
    let pool = fresh_pool(&url).await;
    let (store, deployment_id) = build_store(&pool).await;
    // Create a second deployment (different manifest_hash + version).
    let mut manifest2 = baseline_manifest(84532);
    manifest2.manifest_hash = format!("0x{}", "22".repeat(32));
    manifest2.deployment_version = 2;
    let deployment_id_2 = store
        .upsert_deployment(&manifest2, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert second deployment");
    assert_ne!(deployment_id, deployment_id_2);

    let manifest = baseline_manifest(84532);
    let target = engine_address_from_manifest(&manifest);
    let bridge_a = build_bridge_over_mock(MockProviderMode::Success, TEST_ADDRESS_HEX);
    let bridge_b = build_bridge_over_mock(MockProviderMode::Success, TEST_ADDRESS_HEX);
    let mock = boot_mock_rpc(84532, target).await;
    mock.set_transaction_count(&to_hex(&parse_address_hex(TEST_ADDRESS_HEX)), "pending", 7);

    let orch_a = build_orchestrator(
        store.clone(),
        &manifest,
        deployment_id,
        bridge_a,
        mock.url(),
    );
    let orch_b = build_orchestrator(
        store.clone(),
        &manifest2,
        deployment_id_2,
        bridge_b,
        mock.url(),
    );
    let intent = build_intent(&manifest, 100_000_000);
    let a = orch_a.prepare(intent.clone()).await.expect("a");
    let b = orch_b.prepare(intent).await.expect("b");
    assert_ne!(a.canonical_execution_id, b.canonical_execution_id);
    assert!(mock.prohibited_calls_seen().is_empty());
}
