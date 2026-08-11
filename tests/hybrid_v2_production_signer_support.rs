//! `BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1`
//! Shared support module for the production-signer startup PG suites
//! (Parts H + J). Provides:
//!
//! * `MockSignerService` — a full-fidelity axum microservice with
//!   fault-injection knobs (mirrors the harness in
//!   `tests/hybrid_v2_production_signer_http_e2e.rs`, extracted here
//!   for reuse across the restart + full-matrix binaries).
//! * `fresh_pool` / `build_store_with_deployment` — PG bootstrapping
//!   helpers that reset the schema per test and upsert a Sepolia
//!   manifest.
//! * `EnvGuard` — RAII helper that sets a curated set of
//!   `HV2_*` env vars for the duration of a test and clears them on
//!   drop. Every restart / matrix test uses this so tests never leak
//!   env into each other under `--test-threads=1`.
//! * `AppStateFactory` — builder that constructs an `AppState` with
//!   the manifest + projection store + deployment entry attached,
//!   ready to be fed into `wire_hybrid_v2_execution_orchestrator`.
//!
//! Every helper in this module is `#[allow(dead_code)]` because
//! individual binaries only pull a subset. The frozen safety posture
//! is enforced at the module level: no helper here issues a broadcast
//! RPC or embeds a mainnet key.

#![cfg(feature = "test-signer")]
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::postgres::{PgPool, PgPoolOptions};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use deopt_v2_backend::api::hybrid_v2_read::{DeploymentEntry, HybridV2ApiState};
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::config::PrivateKeySecret;
use deopt_v2_backend::execution::signer::ExecutorSigner;
use deopt_v2_backend::hybrid_v2::manifest::{
    ActivationStatus, ManifestModuleAddresses, ManifestParams,
};
use deopt_v2_backend::hybrid_v2::persistence::{
    HybridV2ProjectionStore, PostgresHybridV2ProjectionStore,
};

pub const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
pub const TEST_ADDRESS_HEX: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";
pub const TEST_CHAIN_ID: u64 = 84532;
pub const URL_ENV: &str = "HYBRID_V2_PG_TEST_DATABASE_URL";
pub const ALT_URL_ENV: &str = "PG_INTEGRATION_URL";
pub const REQUIRE_ENV: &str = "DEOPT_REQUIRE_PG_INTEGRATION";

// -----------------------------------------------------------------
//                        PG helpers
// -----------------------------------------------------------------

pub fn get_pg_url_or_skip(test_name: &str) -> Option<String> {
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

pub async fn fresh_pool(url: &str) -> PgPool {
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

pub fn baseline_manifest(chain_id: u64) -> ManifestParams {
    ManifestParams {
        chain_id,
        manifest_address: "0x000000000000000000000000000000000000d001".into(),
        manifest_hash: "0x1111111111111111111111111111111111111111111111111111111111111111".into(),
        module_addresses_hash: "0x2222222222222222222222222222222222222222222222222222222222222222"
            .into(),
        critical_config_hash: "0x3333333333333333333333333333333333333333333333333333333333333333"
            .into(),
        architecture_version: 1,
        storage_version: 1,
        event_version: 1,
        deployment_version: 1,
        manifest_schema_version: 1,
        environment_tag: "0x6c6f63616c00000000000000000000000000000000000000000000000000000".into(),
        deployer: "0x000000000000000000000000000000000000dead".into(),
        deployment_block: 100,
        deployment_timestamp: 1_700_000_000,
        module_addresses: ManifestModuleAddresses {
            subaccount_registry: "0x0000000000000000000000000000000000000001".into(),
            collateral_vault: "0x0000000000000000000000000000000000000002".into(),
            options_positions_ledger: "0x0000000000000000000000000000000000000003".into(),
            risk_module: "0x0000000000000000000000000000000000000004".into(),
            margin_engine: "0x0000000000000000000000000000000000000005".into(),
            option_matching_engine: "0x0000000000000000000000000000000000000006".into(),
            escape_controller: "0x0000000000000000000000000000000000000007".into(),
            recovery_finalizer: "0x0000000000000000000000000000000000000008".into(),
            oracle_adapter: "0x0000000000000000000000000000000000000009".into(),
            options_risk_provider: "0x000000000000000000000000000000000000000a".into(),
            quote_token: "0x000000000000000000000000000000000000000b".into(),
            fees_manager_v2: None,
            option_execution_fee_adapter: None,
            protocol_timelock: None,
            governance: Some("0x00000000000000000000000000000000000000a1".into()),
            guardian: Some("0x00000000000000000000000000000000000000a2".into()),
        },
        protocol_fee_subkey: "0xaa00000000000000000000000000000000000000000000000000000000000001"
            .into(),
        rebate_budget_subkey: "0xaa00000000000000000000000000000000000000000000000000000000000002"
            .into(),
        insurance_fund_subkey: "0xaa00000000000000000000000000000000000000000000000000000000000003"
            .into(),
        max_collateral_tokens: 8,
        max_active_series: 32,
        all_capabilities_mask: "65535".into(),
        recovery_activation_delay_seconds: 3600,
        recovery_pause_max_duration_blocks: 100_800,
        activation_status: ActivationStatus::Active,
    }
}

pub async fn build_store_with_deployment(
    pool: &PgPool,
    chain_id: u64,
) -> (Arc<dyn HybridV2ProjectionStore>, i64, ManifestParams) {
    let store: Arc<dyn HybridV2ProjectionStore> =
        Arc::new(PostgresHybridV2ProjectionStore::new(pool.clone()));
    let manifest = baseline_manifest(chain_id);
    let deployment_id = store
        .upsert_deployment(&manifest, "PENDING", 1_700_000_000_000)
        .await
        .expect("upsert deployment");
    (store, deployment_id, manifest)
}

pub fn parse_address_hex(s: &str) -> [u8; 20] {
    let stripped = s.trim_start_matches("0x").trim_start_matches("0X");
    let mut a = [0u8; 20];
    for i in 0..20 {
        a[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).unwrap();
    }
    a
}

pub fn expected_signer_address_bytes() -> [u8; 20] {
    parse_address_hex(TEST_ADDRESS_HEX)
}

pub fn hex_encode_addr(bytes: &[u8; 20]) -> String {
    let mut s = String::with_capacity(42);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// -----------------------------------------------------------------
//                    AppState + wire path builder
// -----------------------------------------------------------------

/// Attach the projection store + a deployment entry keyed on the
/// same manifest so `wire_hybrid_v2_execution_orchestrator` can find
/// both the projection store and a `deployment_id` when it inspects
/// `AppState::hybrid_v2_read.list()`.
pub fn build_appstate(
    store: Arc<dyn HybridV2ProjectionStore>,
    manifest: ManifestParams,
    deployment_id: i64,
) -> AppState {
    let entry = Arc::new(DeploymentEntry::from_metadata(
        deployment_id as u64,
        manifest.clone(),
    ));
    let hybrid_v2_read = HybridV2ApiState::with_store(
        std::sync::Arc::new(deopt_v2_backend::api::hybrid_v2_read::EmptyReadStore) as _,
        vec![entry],
    );
    AppState::new(EngineState::with_default_markets())
        .with_hybrid_v2(hybrid_v2_read)
        .with_hybrid_v2_projection_store(store)
        .with_hybrid_v2_manifest(manifest)
}

// -----------------------------------------------------------------
//                     env var scoping (EnvGuard)
// -----------------------------------------------------------------

/// Curated list of env vars this milestone reads. `EnvGuard` clears
/// them on drop so `--test-threads=1` execution never leaks state.
pub const HV2_ENV_VARS: &[&str] = &[
    "HV2_EXECUTION_ENABLED",
    "HV2_EXECUTOR_ADDRESS",
    "HV2_SIGNER_BACKEND",
    "HV2_SIGNER_ENDPOINT",
    "HV2_SIGNER_EXPECTED_ADDRESS",
    "HV2_SIGNER_KMS_KEY_ID",
    "HV2_SIGNER_PROVIDER",
    "HV2_SIGNER_REQUEST_TIMEOUT_MS",
    "HV2_SIGNER_MAX_RETRIES",
    "HV2_SIGNER_AUTH_REFERENCE",
    "HV2_SIGNER_MTLS_CERT_PATH",
    "HV2_SIGNER_MTLS_KEY_PATH",
    "HV2_SIGNER_ROOT_CA_PATH",
    "HV2_SIGNER_BOOTSTRAP_STRICT",
    "HV2_EXECUTION_RPC_URL",
    "HYBRID_V2_RPC_URL",
    "HV2_EXECUTION_RPC_TIMEOUT_MS",
    "HV2_SIMULATION_MAX_AGE_MS",
];

pub struct EnvGuard;

impl EnvGuard {
    /// Wipe every known HV2 var before the test runs. Callers use
    /// `set` to add per-test values and the drop impl wipes them
    /// again.
    pub fn new() -> Self {
        for k in HV2_ENV_VARS {
            std::env::remove_var(k);
        }
        Self
    }

    pub fn set(&self, key: &str, value: &str) {
        std::env::set_var(key, value);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for k in HV2_ENV_VARS {
            std::env::remove_var(k);
        }
    }
}

// -----------------------------------------------------------------
//                     Mock signer microservice
// -----------------------------------------------------------------

#[derive(Clone)]
pub struct MockSignerState {
    signer: Arc<ExecutorSigner>,
    signer_address_hex: String,
    chain_id: Arc<Mutex<u64>>,
    seen_methods: Arc<Mutex<Vec<String>>>,
    reject_auth: Arc<Mutex<bool>>,
    force_wrong_signer: Arc<Mutex<bool>>,
    next_response_error: Arc<Mutex<Option<StatusCode>>>,
    next_response_delay_ms: Arc<Mutex<Option<u64>>>,
    malformed_signature: Arc<Mutex<bool>>,
    sign_call_count: Arc<Mutex<u32>>,
    /// If `Some(x)`, force the sign body's `chain_id` refusal at
    /// this value (defence-in-depth against Base mainnet). Never
    /// consulted by tests directly — the mock refuses `chain_id ==
    /// 8453` unconditionally.
    _reserved: Arc<Mutex<Option<u64>>>,
}

impl MockSignerState {
    fn new() -> Self {
        let signer =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
        Self {
            signer: Arc::new(signer),
            signer_address_hex: TEST_ADDRESS_HEX.to_string(),
            chain_id: Arc::new(Mutex::new(TEST_CHAIN_ID)),
            seen_methods: Arc::new(Mutex::new(Vec::new())),
            reject_auth: Arc::new(Mutex::new(false)),
            force_wrong_signer: Arc::new(Mutex::new(false)),
            next_response_error: Arc::new(Mutex::new(None)),
            next_response_delay_ms: Arc::new(Mutex::new(None)),
            malformed_signature: Arc::new(Mutex::new(false)),
            sign_call_count: Arc::new(Mutex::new(0)),
            _reserved: Arc::new(Mutex::new(None)),
        }
    }

    fn record(&self, m: &str) {
        self.seen_methods.lock().unwrap().push(m.to_string());
        if m == "sign" {
            *self.sign_call_count.lock().unwrap() += 1;
        }
    }
}

/// Wire-visible handle to the mock signer microservice.
pub struct MockSignerService {
    state: MockSignerState,
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl MockSignerService {
    pub async fn start() -> Self {
        let state = MockSignerState::new();
        let router = Router::new()
            .route("/hybrid_v2/sign", post(handle_sign))
            .route("/hybrid_v2/identity", get(handle_identity))
            .route("/hybrid_v2/health", get(handle_health))
            .with_state(state.clone());
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = TcpListener::bind(addr).await.unwrap();
        let bound = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        Self {
            state,
            addr: bound,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn sign_endpoint(&self) -> String {
        format!("http://{}/hybrid_v2/sign", self.addr)
    }

    pub fn methods(&self) -> Vec<String> {
        self.state.seen_methods.lock().unwrap().clone()
    }

    pub fn sign_calls(&self) -> u32 {
        *self.state.sign_call_count.lock().unwrap()
    }

    pub fn set_next_response_error(&self, status: StatusCode) {
        *self.state.next_response_error.lock().unwrap() = Some(status);
    }

    pub fn set_next_response_delay(&self, delay: Duration) {
        *self.state.next_response_delay_ms.lock().unwrap() = Some(delay.as_millis() as u64);
    }

    pub fn set_reject_auth(&self, reject: bool) {
        *self.state.reject_auth.lock().unwrap() = reject;
    }

    pub fn set_wrong_signer(&self, wrong: bool) {
        *self.state.force_wrong_signer.lock().unwrap() = wrong;
    }

    pub fn set_malformed_signature(&self, malformed: bool) {
        *self.state.malformed_signature.lock().unwrap() = malformed;
    }

    pub fn set_reported_chain_id(&self, chain_id: u64) {
        *self.state.chain_id.lock().unwrap() = chain_id;
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        }
    }
}

impl Drop for MockSignerService {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

#[derive(Debug, Deserialize)]
struct SignBody {
    chain_id: u64,
    #[serde(default)]
    nonce: u64,
    digest: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    value_wei_hex: Option<String>,
    #[serde(default)]
    gas_limit: Option<u64>,
    #[serde(default)]
    max_fee_per_gas_hex: Option<String>,
    #[serde(default)]
    max_priority_fee_per_gas_hex: Option<String>,
    #[serde(default)]
    tx_type: Option<u8>,
    expected_signer: String,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    policy_decision_id: Option<String>,
    #[serde(default)]
    fingerprint: Option<String>,
}

#[derive(Debug, Serialize)]
struct SignResponseBody {
    signature_r: String,
    signature_s: String,
    signature_v: u8,
    recovered_signer: String,
}

async fn handle_sign(
    State(state): State<MockSignerState>,
    Json(body): Json<SignBody>,
) -> Result<Json<SignResponseBody>, StatusCode> {
    state.record("sign");
    // Extract every flag with a NARROW lock scope (avoid holding a
    // MutexGuard across the await point — the compiler otherwise
    // flags the handler as non-Send).
    let next_err = {
        let mut g = state.next_response_error.lock().unwrap();
        g.take()
    };
    if let Some(status) = next_err {
        return Err(status);
    }
    let delay = {
        let mut g = state.next_response_delay_ms.lock().unwrap();
        g.take()
    };
    if let Some(delay_ms) = delay {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    let reject_auth = *state.reject_auth.lock().unwrap();
    if reject_auth {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if body.chain_id == 8453 {
        return Err(StatusCode::FORBIDDEN);
    }
    if body.expected_signer.to_ascii_lowercase() != state.signer_address_hex {
        return Err(StatusCode::CONFLICT);
    }
    let digest = parse_hex_32(&body.digest).ok_or(StatusCode::BAD_REQUEST)?;
    let sig = state
        .signer
        .sign_prehash(&digest)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let force_wrong = *state.force_wrong_signer.lock().unwrap();
    let malformed = *state.malformed_signature.lock().unwrap();
    let addr = if force_wrong {
        "0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead".to_string()
    } else {
        state.signer_address_hex.clone()
    };
    let (v, r_hex, s_hex) = if malformed {
        (
            99,
            format!("0x{}", "zz".repeat(32)),
            format!("0x{}", "zz".repeat(32)),
        )
    } else {
        (
            sig.y_parity,
            format!("0x{}", hex_encode(&sig.r)),
            format!("0x{}", hex_encode(&sig.s)),
        )
    };
    Ok(Json(SignResponseBody {
        signature_r: r_hex,
        signature_s: s_hex,
        signature_v: v,
        recovered_signer: addr,
    }))
}

async fn handle_identity(State(state): State<MockSignerState>) -> Json<serde_json::Value> {
    state.record("identity");
    let chain_id = { *state.chain_id.lock().unwrap() };
    Json(json!({
        "signer_address": state.signer_address_hex,
        "chain_id": chain_id,
    }))
}

async fn handle_health(State(state): State<MockSignerState>) -> Json<serde_json::Value> {
    state.record("health");
    let chain_id = { *state.chain_id.lock().unwrap() };
    Json(json!({
        "healthy": true,
        "signer_address": state.signer_address_hex,
        "chain_id": chain_id,
    }))
}

fn parse_hex_32(s: &str) -> Option<[u8; 32]> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    if stripped.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
