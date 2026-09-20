//! PERPS_V2_BACKEND_ANVIL_LIVE_E2E_INTEGRATION_V1 — real V2 backend
//! + Anvil + PostgreSQL integration coverage.
//!
//! This binary owns BOTH:
//!
//!   1. A first-class V2 spawn harness (`V2E2eEnv`) — spawns Anvil,
//!      invokes `DeployPerpsV2E2E` via `forge script`, parses the
//!      structured manifest, initialises a fresh PgRepository, and
//!      constructs an `AppState` wired to the real deployed V2 stack
//!      with `PERPS_ACTIVE_ENGINE_VERSION=v2`.
//!
//!   2. The end-to-end non-broadcast lifecycle test surface:
//!      preflight → HTTP prepare → PG persistence → active-version
//!      flip durability → real cosign → real eth_call simulation →
//!      independent calldata decode → zero-send proofs. Plus the five
//!      REAL negative preflight scenarios (migration-open, clearing
//!      identity mismatch, clearing floor, executor auth, PME↔engine
//!      linkage).
//!
//! # Hard rules
//!
//! * NO Base Sepolia write. NO public-chain deployment. NO
//!   `sendRawTransaction`. NO executor arming. NO real trader
//!   keystore. NO LocalKeystore. Anvil + local PostgreSQL only.
//!
//! # PG env gate
//!
//! When `PERPS_CLOSED_TEST_E2E_PG_URL` is unset every test emits an
//! `IGNORED (PG url not provided)` marker and returns. Same pattern
//! as `perps_closed_test_e2e_harness.rs` / `perps_cosign_route_pg_integration.rs`.

#![allow(dead_code)]

use axum::Router;
use deopt_v2_backend::api::perps_cosign::{
    CosignTradeRequest, CosignTradeResponse, PrepareTradeRequest, PrepareTradeResponse,
};
use deopt_v2_backend::api::{router, AppState};
use deopt_v2_backend::db::PgRepository;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::execution::rpc::HttpJsonRpcProvider;
use deopt_v2_backend::execution::v2_readiness::{
    v2_preflight_check, MigrationState, RpcV2EngineReader, RpcV2MatchingEngineReader,
    RpcV2VaultReader, V2PreflightDenial, V2PreflightOutcome,
};
use deopt_v2_backend::execution::{
    decode_execute_trade_v2_calldata, encode_execute_trade_v2_calldata, execute_trade_v2_selector,
    perp_trade_v2_digest_bytes, ExecutionConfig, ExecutionIntentStatus, PerpTradeDomain,
    PerpTradePayload, PerpTradeSignatureBundle, PerpsProtocolVersion,
};
use deopt_v2_backend::perps::PerpsReadConfig;
use deopt_v2_backend::signing::eip712::keccak256;
use deopt_v2_backend::types::AccountId;
use k256::ecdsa::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::{json, Value as JsonValue};
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------
// Constants — mirror perps_closed_test_e2e_harness.rs where possible so
// developers reason about ONE anvil/backend spawn model.
// ---------------------------------------------------------------------

/// Base-Sepolia chain id. Backend `validate_startup` compares
/// `executor_chain_id` against `rpc.chain_id()`; the anvil node is
/// spawned with this id so the config is accepted. The anvil node is
/// still fully ephemeral and NEVER touches real Base Sepolia.
const HARNESS_CHAIN_ID: u64 = 84532;

const SPAWN_BUDGET: Duration = Duration::from_secs(120);
const ANVIL_READY_TIMEOUT: Duration = Duration::from_secs(30);
const BACKEND_READY_TIMEOUT: Duration = Duration::from_secs(15);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// PG env gate. Shared with the V1 closed-test harness so ONE local
/// PG URL drives every integration suite in this repo.
const PG_ENV_VAR: &str = "PERPS_CLOSED_TEST_E2E_PG_URL";

/// Base-Sepolia-parity A/B fixture: A holds +1_000_000 (long), B holds
/// -1_000_000 (short). Mirrors `DeployPerpsV2E2E`'s seeded positions
/// and the live Base Sepolia state.
const CANDIDATE_SIZE_1E8: u128 = 1_000_000;

/// $2468.31 in 1e8 scale — matches the `DeployPerpsV2E2E`
/// `DEFAULT_PRICE_1E8` constant AND the live Base Sepolia mark.
const CANDIDATE_PRICE_1E8: u128 = 246_831_000_000;

/// 1M mUSDC at 6-decimal scale — matches
/// `DeployPerpsV2E2E::DEFAULT_CLEARING_FUND_RAW`.
const CLEARING_FUND_RAW: u128 = 1_000_000_000_000;

/// Bounded-check floor used by the positive test.  Set well below the
/// seeded 1M raw clearing balance so preflight passes and the negative
/// test can independently flip it above.
const POSITIVE_CLEARING_FLOOR_RAW: u128 = 500_000_000_000;

/// Path to the on-disk sol repository (relative to the backend crate
/// root — standard sibling checkout layout).
const SOL_REPO_RELATIVE_PATH: &str = "../deopt-v2-sol";

const MARKET_ID: u128 = 1;

// ---------------------------------------------------------------------
// Harness types
// ---------------------------------------------------------------------

#[derive(Debug)]
pub struct HarnessError(pub String);

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HarnessError: {}", self.0)
    }
}

impl std::error::Error for HarnessError {}

fn err<S: Into<String>>(s: S) -> HarnessError {
    HarnessError(s.into())
}

/// Every field the `DeployPerpsV2E2E._writeManifest` script emits.
/// Kept 1:1 with the 21-field structured manifest so any Sol-side
/// drift surfaces as a compile-time mismatch.
#[derive(Clone, Debug)]
pub struct V2DeployedAddresses {
    pub chain_id: u64,
    pub market_id: u128,
    pub initial_price_1e8: u128,
    pub clearing_fund_raw: u128,
    pub sealed: bool,
    pub seal_hash: [u8; 32],
    pub deployer: String,
    pub usdc: String,
    pub weth: String,
    pub primary_source: String,
    pub secondary_source: String,
    pub oracle_router: String,
    pub vault: String,
    pub perp_market_registry: String,
    pub perp_engine_v2: String,
    pub perp_matching_engine_v2: String,
    pub perp_clearing_account_v2: String,
    pub risk: String,
    pub trader_a: String,
    pub trader_b: String,
    pub executor: String,
}

/// One local V2 test wallet. Trader A / B / executor keys are
/// generated fresh per spawn (OS randomness) and remain in-process
/// only; they are NEVER logged or serialised.
pub struct V2Wallet {
    pub address: String,
    pub private_key_hex: String,
    pub signer: SigningKey,
}

impl V2Wallet {
    fn generate() -> Self {
        let mut bytes = [0u8; 32];
        loop {
            OsRng.fill_bytes(&mut bytes);
            if let Ok(signer) = SigningKey::from_bytes(&bytes.into()) {
                let address = evm_address_from_signing_key(&signer);
                return Self {
                    address,
                    private_key_hex: to_hex_0x(&bytes),
                    signer,
                };
            }
        }
    }
}

/// Ephemeral wallets for the V2 stack. `trader_a` / `trader_b` sign
/// EIP-712 digests; `executor` is the caller address recorded on
/// `PerpMatchingEngineV2.setExecutor` at deploy time. `deployer`
/// funds the deploy + optional privileged mutations
/// (`setExecutor` toggles for the negative test).
pub struct V2Wallets {
    pub deployer: V2Wallet,
    pub trader_a: V2Wallet,
    pub trader_b: V2Wallet,
    pub executor: V2Wallet,
}

/// Knobs the fixture exposes to negative-scenario tests. Every
/// negative scenario except migration-open uses the default sealed
/// fixture; setting `seal_migration = false` deploys the fixture with
/// migration OPEN (matches the `PERPS_V2_E2E_SEAL_MIGRATION=false`
/// solidity-side envvar).
#[derive(Clone, Debug)]
pub struct V2SpawnOpts {
    pub seal_migration: bool,
    /// Configured clearing-account balance floor (raw base-units). The
    /// positive test uses `POSITIVE_CLEARING_FLOOR_RAW`; negatives may
    /// override via `V2E2eEnv::with_clearing_min_balance_raw`.
    pub clearing_min_balance_raw: u128,
}

impl Default for V2SpawnOpts {
    fn default() -> Self {
        Self {
            seal_migration: true,
            clearing_min_balance_raw: POSITIVE_CLEARING_FLOOR_RAW,
        }
    }
}

/// Handle to a running local `anvil`. `Drop` kills the process.
pub struct AnvilProcess {
    child: Child,
    pub url: String,
    pub chain_id: u64,
}

impl Drop for AnvilProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One live V2 environment: Anvil + deployed V2 stack + backend +
/// PgRepository + wallets.
pub struct V2E2eEnv {
    pub anvil: AnvilProcess,
    pub anvil_url: String,
    pub chain_id: u64,
    pub pg_url: String,
    pub backend_url: String,
    pub contracts: V2DeployedAddresses,
    pub wallets: V2Wallets,
    pub state: Arc<AppState>,
    pub http: reqwest::Client,
    pub repository: PgRepository,
    manifest_path: PathBuf,
    backend_task: Option<JoinHandle<()>>,
    opts: V2SpawnOpts,
}

impl V2E2eEnv {
    /// End-to-end spawn: anvil → forge-deploy V2 → migrate PG →
    /// build V2 AppState → axum backend → readiness poll. Total
    /// wall-clock < SPAWN_BUDGET.
    pub async fn spawn(opts: V2SpawnOpts) -> Result<Self, HarnessError> {
        let started = Instant::now();

        // (1) Anvil.
        let anvil_port = pick_free_port().map_err(|e| err(format!("pick_free_port: {e}")))?;
        let deployer = V2Wallet::generate();
        let trader_a = V2Wallet::generate();
        let trader_b = V2Wallet::generate();
        let executor = V2Wallet::generate();
        let anvil = spawn_anvil(
            anvil_port,
            HARNESS_CHAIN_ID,
            &[
                &deployer.address,
                &trader_a.address,
                &trader_b.address,
                &executor.address,
            ],
        )
        .await
        .map_err(|e| err(format!("spawn anvil: {e}")))?;

        // (2) Forge deploy.
        let manifest_path = write_temp_path("perps_v2_e2e_manifest.json")
            .map_err(|e| err(format!("temp path: {e}")))?;
        run_forge_deploy_v2(
            &anvil.url,
            &deployer.private_key_hex,
            &trader_a.address,
            &trader_b.address,
            &executor.address,
            &manifest_path,
            opts.seal_migration,
        )
        .await
        .map_err(|e| err(format!("forge deploy: {e}")))?;
        let contracts =
            read_v2_manifest(&manifest_path).map_err(|e| err(format!("read manifest: {e}")))?;
        // Structural sanity: manifest must agree with the anvil chain
        // id AND the harness-generated addresses (case-insensitive).
        if contracts.chain_id != HARNESS_CHAIN_ID {
            return Err(err(format!(
                "manifest chainId {} != HARNESS_CHAIN_ID {}",
                contracts.chain_id, HARNESS_CHAIN_ID
            )));
        }
        if !contracts.trader_a.eq_ignore_ascii_case(&trader_a.address)
            || !contracts.trader_b.eq_ignore_ascii_case(&trader_b.address)
            || !contracts.executor.eq_ignore_ascii_case(&executor.address)
        {
            return Err(err(
                "manifest trader/executor addresses do not match harness-generated wallets"
                    .to_string(),
            ));
        }

        // (3) PG migrations.
        let pg_url = std::env::var(PG_ENV_VAR).unwrap_or_default();
        if pg_url.is_empty() {
            return Err(err(format!(
                "{PG_ENV_VAR} unset — this integration binary requires PG"
            )));
        }
        ensure_migrated(&pg_url)
            .await
            .map_err(|e| err(format!("pg migrate: {e}")))?;
        let repository = PgRepository::connect(&pg_url)
            .await
            .map_err(|e| err(format!("pg connect: {e}")))?;

        // (4) AppState wired for V2.
        let state = build_v2_app_state(
            &contracts,
            &anvil.url,
            &trader_a,
            &trader_b,
            &executor,
            repository.clone(),
            opts.clearing_min_balance_raw,
        );
        let state_arc = Arc::new(state.clone());

        // (5) Backend.
        let (backend_url, backend_task) = spawn_backend(state.clone())
            .await
            .map_err(|e| err(format!("spawn backend: {e}")))?;

        if started.elapsed() > SPAWN_BUDGET {
            return Err(err(format!(
                "spawn budget exceeded: elapsed={:?} > budget={:?}",
                started.elapsed(),
                SPAWN_BUDGET
            )));
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| err(format!("reqwest client: {e}")))?;

        let anvil_url = anvil.url.clone();
        Ok(Self {
            anvil,
            anvil_url,
            chain_id: HARNESS_CHAIN_ID,
            pg_url,
            backend_url,
            contracts,
            wallets: V2Wallets {
                deployer,
                trader_a,
                trader_b,
                executor,
            },
            state: state_arc,
            http,
            repository,
            manifest_path,
            backend_task: Some(backend_task),
            opts,
        })
    }

    pub async fn shutdown(mut self) -> Result<(), HarnessError> {
        if let Some(task) = self.backend_task.take() {
            task.abort();
        }
        // Best-effort: unlink the manifest.
        let _ = std::fs::remove_file(&self.manifest_path);
        Ok(())
    }

    /// Build a fresh JSON-RPC provider bound to the anvil node. Used
    /// by both the preflight readers and the simulation route.
    pub fn rpc(&self) -> HttpJsonRpcProvider {
        HttpJsonRpcProvider::new(self.anvil_url.clone())
    }

    /// Run the full V2 preflight against the live Anvil V2 stack.
    /// `min_balance_raw` overrides the AppState default (used by the
    /// clearing-floor negative test).
    pub async fn run_v2_preflight_with(
        &self,
        engine_addr: &AccountId,
        pme_addr: &AccountId,
        clearing_addr: &AccountId,
        min_balance_raw: u128,
    ) -> deopt_v2_backend::execution::v2_readiness::V2PreflightReport {
        let rpc = self.rpc();
        let engine_reader = RpcV2EngineReader::new(rpc.clone(), engine_addr.clone());
        let pme_reader = RpcV2MatchingEngineReader::new(rpc.clone(), pme_addr.clone());
        let vault_reader =
            RpcV2VaultReader::new(rpc.clone(), AccountId::new(self.contracts.vault.clone()));

        let mut config = self.state.execution_config.clone();
        config.perp_engine_v2_address = Some(engine_addr.clone());
        config.perp_matching_engine_v2_address = Some(pme_addr.clone());
        config.perp_clearing_account_v2_address = Some(clearing_addr.clone());
        config.perps_v2_clearing_min_balance_raw = min_balance_raw;

        v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &AccountId::new(self.contracts.executor.clone()),
            &AccountId::new(self.contracts.usdc.clone()),
            &engine_reader,
            &pme_reader,
            &vault_reader,
        )
        .await
    }

    /// Convenience: run preflight against the deployed V2 addresses
    /// with the AppState-configured min balance floor.
    pub async fn run_v2_preflight(
        &self,
    ) -> deopt_v2_backend::execution::v2_readiness::V2PreflightReport {
        self.run_v2_preflight_with(
            &AccountId::new(self.contracts.perp_engine_v2.clone()),
            &AccountId::new(self.contracts.perp_matching_engine_v2.clone()),
            &AccountId::new(self.contracts.perp_clearing_account_v2.clone()),
            self.opts.clearing_min_balance_raw,
        )
        .await
    }

    /// Real HTTP `POST /perps/closed-test/trades/prepare`.
    pub async fn http_prepare(
        &self,
        req: &PrepareTradeRequest,
    ) -> Result<PrepareTradeResponse, HarnessError> {
        let resp = self
            .http
            .post(format!(
                "{}/perps/closed-test/trades/prepare",
                self.backend_url
            ))
            .json(req)
            .send()
            .await
            .map_err(|e| err(format!("http prepare send: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| err(format!("http prepare body: {e}")))?;
        if !status.is_success() {
            return Err(err(format!("http prepare non-2xx {status}: {body}")));
        }
        serde_json::from_str::<PrepareTradeResponse>(&body)
            .map_err(|e| err(format!("http prepare parse: {e} — body={body}")))
    }

    /// Real HTTP `POST /perps/closed-test/trades/{uuid}/cosign`.
    pub async fn http_cosign(
        &self,
        uuid: uuid::Uuid,
        buyer_sig: &str,
        seller_sig: &str,
    ) -> Result<CosignTradeResponse, HarnessError> {
        let body = CosignTradeRequest {
            buyer_signature: buyer_sig.to_string(),
            seller_signature: seller_sig.to_string(),
        };
        let resp = self
            .http
            .post(format!(
                "{}/perps/closed-test/trades/{uuid}/cosign",
                self.backend_url
            ))
            .json(&body)
            .send()
            .await
            .map_err(|e| err(format!("http cosign send: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| err(format!("http cosign body: {e}")))?;
        if !status.is_success() {
            return Err(err(format!("http cosign non-2xx {status}: {text}")));
        }
        serde_json::from_str::<CosignTradeResponse>(&text)
            .map_err(|e| err(format!("http cosign parse: {e} — body={text}")))
    }

    /// Real HTTP `POST /executor/simulate/{intent_id}` — returns the
    /// raw JSON so callers can prove `simulation_status` /
    /// `revert_data` / `revert_selector` explicitly.
    pub async fn http_simulate(&self, uuid: uuid::Uuid) -> Result<JsonValue, HarnessError> {
        let resp = self
            .http
            .post(format!("{}/executor/simulate/{uuid}", self.backend_url))
            .send()
            .await
            .map_err(|e| err(format!("http simulate send: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| err(format!("http simulate body: {e}")))?;
        if !status.is_success() {
            return Err(err(format!("http simulate non-2xx {status}: {text}")));
        }
        serde_json::from_str(&text)
            .map_err(|e| err(format!("http simulate parse: {e} — body={text}")))
    }

    /// Cast `PerpMatchingEngineV2.setExecutor(runtime, enabled)` via
    /// the deployer key. Used by the executor-auth negative test.
    pub async fn cast_set_executor(&self, enabled: bool) -> Result<(), HarnessError> {
        let selector = keccak256(b"setExecutor(address,bool)");
        let mut data = String::from("0x");
        for b in &selector[..4] {
            data.push_str(&format!("{b:02x}"));
        }
        data.push_str(&pad_address(&self.contracts.executor));
        data.push_str(&pad_bool(enabled));
        run_cast_send(
            &self.anvil_url,
            &self.wallets.deployer.private_key_hex,
            &self.contracts.perp_matching_engine_v2,
            &data,
        )
        .await
    }

    /// Read the executor Anvil nonce (`eth_getTransactionCount`). Used
    /// by the zero-send proof.
    pub async fn executor_anvil_nonce(&self) -> Result<u64, HarnessError> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionCount",
            "params": [self.wallets.executor.address.clone(), "latest"]
        });
        let resp = self
            .http
            .post(&self.anvil_url)
            .header("content-type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .map_err(|e| err(format!("nonce rpc send: {e}")))?;
        let json_val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| err(format!("nonce rpc json: {e}")))?;
        let hex = json_val
            .get("result")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err(format!("nonce rpc missing result: {json_val}")))?;
        let trimmed = hex.trim_start_matches("0x");
        u64::from_str_radix(if trimmed.is_empty() { "0" } else { trimmed }, 16)
            .map_err(|e| err(format!("nonce parse {hex}: {e}")))
    }
}

// ---------------------------------------------------------------------
// Low-level harness helpers. Duplicated (small) from the V1 closed-test
// harness on purpose so the V2 harness has zero cross-binary coupling.
// ---------------------------------------------------------------------

fn pick_free_port() -> std::io::Result<u16> {
    let listener = StdTcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn evm_address_from_signing_key(key: &SigningKey) -> String {
    let verifying: &VerifyingKey = key.verifying_key();
    let encoded = verifying.to_encoded_point(false);
    let hash = keccak256(&encoded.as_bytes()[1..]);
    let mut out = String::from("0x");
    for b in &hash[12..] {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn to_hex_0x(bytes: &[u8]) -> String {
    let mut s = String::from("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn pad_address(addr: &str) -> String {
    let hex = addr.strip_prefix("0x").unwrap_or(addr);
    format!("{:0>64}", hex.to_ascii_lowercase())
}

fn pad_bool(v: bool) -> String {
    if v {
        format!("{:0>64}", "1")
    } else {
        format!("{:0>64}", "0")
    }
}

fn write_temp_path(name: &str) -> std::io::Result<PathBuf> {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    let suffix = to_hex_0x(&bytes);
    let mut path = std::env::temp_dir();
    path.push(format!("{}_{}", suffix.trim_start_matches("0x"), name));
    Ok(path)
}

fn anvil_binary() -> String {
    std::env::var("PERPS_E2E_ANVIL_BIN").unwrap_or_else(|_| "anvil".to_string())
}

fn forge_binary() -> String {
    std::env::var("PERPS_E2E_FORGE_BIN").unwrap_or_else(|_| "forge".to_string())
}

fn cast_binary() -> String {
    std::env::var("PERPS_E2E_CAST_BIN").unwrap_or_else(|_| "cast".to_string())
}

fn sol_repo_path() -> Result<PathBuf, HarnessError> {
    if let Ok(p) = std::env::var("PERPS_E2E_SOL_REPO_PATH") {
        return Ok(PathBuf::from(p));
    }
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = crate_dir.join(SOL_REPO_RELATIVE_PATH);
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(err(format!(
        "sol repo not found. Set PERPS_E2E_SOL_REPO_PATH or place the sol \
         checkout at {}",
        candidate.display()
    )))
}

fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

async fn spawn_anvil(
    port: u16,
    chain_id: u64,
    fund_addresses: &[&str],
) -> Result<AnvilProcess, HarnessError> {
    let mut cmd = Command::new(anvil_binary());
    cmd.arg("--port")
        .arg(port.to_string())
        .arg("--chain-id")
        .arg(chain_id.to_string())
        .arg("--accounts")
        .arg("0")
        // PerpEngineV2 (with its inherited storage/admin/views/trading
        // stack) exceeds the vanilla EIP-170 24kB ceiling. This is a
        // TEST-ONLY concession — mainnet Base Sepolia deployments
        // remain subject to the real 24kB limit. Backend never reads
        // this flag; it is passed to anvil directly.
        .arg("--disable-code-size-limit")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| err(format!("spawn anvil binary: {e}")))?;

    let url = format!("http://127.0.0.1:{port}");
    if let Err(e) = poll_anvil_ready(&url).await {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }

    // Fund every provided address with 10k ETH so the deployer can
    // broadcast + the traders/executor can (unused) sign anything.
    let client = reqwest::Client::new();
    for addr in fund_addresses {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "anvil_setBalance",
            "params": [addr, "0x21e19e0c9bab2400000"]
        });
        if let Err(e) = client
            .post(&url)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err(format!("anvil_setBalance({addr}): {e}")));
        }
    }

    match child.try_wait() {
        Ok(Some(status)) => Err(err(format!("anvil exited early: {status:?}"))),
        Ok(None) => Ok(AnvilProcess {
            child,
            url,
            chain_id,
        }),
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(err(format!("anvil try_wait: {e}")))
        }
    }
}

async fn poll_anvil_ready(url: &str) -> Result<(), HarnessError> {
    let client = reqwest::Client::new();
    let started = Instant::now();
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_blockNumber",
        "params": []
    })
    .to_string();
    loop {
        if started.elapsed() > ANVIL_READY_TIMEOUT {
            return Err(err(format!(
                "anvil not ready within {ANVIL_READY_TIMEOUT:?} at {url}"
            )));
        }
        if let Ok(resp) = client
            .post(url)
            .header("content-type", "application/json")
            .body(payload.clone())
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(value) = resp.json::<serde_json::Value>().await {
                    if value.get("result").is_some() {
                        return Ok(());
                    }
                }
            }
        }
        tokio::time::sleep(READY_POLL_INTERVAL).await;
    }
}

async fn run_forge_deploy_v2(
    anvil_url: &str,
    deployer_key_hex: &str,
    trader_a: &str,
    trader_b: &str,
    executor: &str,
    manifest_path: &PathBuf,
    seal_migration: bool,
) -> Result<(), HarnessError> {
    let sol_repo = sol_repo_path()?;
    let seal_flag = if seal_migration { "true" } else { "false" };
    let mut cmd = Command::new(forge_binary());
    cmd.current_dir(&sol_repo)
        .arg("script")
        .arg("script/DeployPerpsE2E.s.sol:DeployPerpsV2E2E")
        .arg("--rpc-url")
        .arg(anvil_url)
        .arg("--broadcast")
        // PerpEngineV2's runtime bytecode (with its full inheritance
        // chain) exceeds EIP-170's 24kB ceiling by ~3.5kB. Anvil is
        // spawned with `--disable-code-size-limit`; forge script
        // enforces the same ceiling independently, so we mirror the
        // flag here. Test-only concession — mainnet deploys remain
        // subject to EIP-170.
        .arg("--disable-code-size-limit")
        .arg("--slow")
        .env("PERPS_V2_E2E_DEPLOY_ENABLED", "true")
        .env("DEPLOYER_PRIVATE_KEY", deployer_key_hex)
        .env("PERPS_V2_E2E_TRADER_A", trader_a)
        .env("PERPS_V2_E2E_TRADER_B", trader_b)
        .env("PERPS_V2_E2E_EXECUTOR", executor)
        .env(
            "PERPS_V2_E2E_MANIFEST_PATH",
            manifest_path.to_string_lossy().to_string(),
        )
        .env("PERPS_V2_E2E_SEAL_MIGRATION", seal_flag)
        .env(
            "PERPS_V2_E2E_INITIAL_PRICE_1E8",
            CANDIDATE_PRICE_1E8.to_string(),
        )
        .env(
            "PERPS_V2_E2E_CLEARING_FUND_RAW",
            CLEARING_FUND_RAW.to_string(),
        )
        .env("FOUNDRY_DISABLE_NIGHTLY_WARNING", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = tokio::task::spawn_blocking(move || cmd.output())
        .await
        .map_err(|e| err(format!("forge spawn_blocking: {e}")))?
        .map_err(|e| err(format!("forge exec: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(err(format!(
            "forge script failed (status={:?})\n--- stderr (tail) ---\n{}\n--- stdout (tail) ---\n{}",
            output.status,
            tail_lines(&stderr, 40),
            tail_lines(&stdout, 40)
        )));
    }
    if !manifest_path.exists() {
        return Err(err(format!(
            "forge script succeeded but manifest not written at {}",
            manifest_path.display()
        )));
    }
    Ok(())
}

async fn run_cast_send(
    anvil_url: &str,
    deployer_key_hex: &str,
    contract: &str,
    calldata: &str,
) -> Result<(), HarnessError> {
    let anvil_url = anvil_url.to_string();
    let deployer_key = deployer_key_hex.to_string();
    let contract = contract.to_string();
    let calldata = calldata.to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(cast_binary())
            .arg("send")
            .arg("--rpc-url")
            .arg(anvil_url)
            .arg("--private-key")
            .arg(deployer_key)
            .arg(contract)
            .arg(calldata)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .map_err(|e| err(format!("cast spawn_blocking: {e}")))?
    .map_err(|e| err(format!("cast exec: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(err(format!(
            "cast send failed (status={:?}): {}",
            output.status,
            tail_lines(&stderr, 10)
        )));
    }
    Ok(())
}

fn read_v2_manifest(path: &PathBuf) -> Result<V2DeployedAddresses, HarnessError> {
    let raw = std::fs::read_to_string(path).map_err(|e| err(format!("read manifest: {e}")))?;
    let value: JsonValue =
        serde_json::from_str(&raw).map_err(|e| err(format!("parse manifest json: {e}")))?;
    let addr = |key: &str| -> Result<String, HarnessError> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase())
            .ok_or_else(|| err(format!("manifest missing {key}")))
    };
    let uint_u128 = |key: &str| -> Result<u128, HarnessError> {
        value
            .get(key)
            .and_then(|v| v.as_u64())
            .map(|u| u as u128)
            .ok_or_else(|| err(format!("manifest missing {key}")))
    };
    let uint_u64 = |key: &str| -> Result<u64, HarnessError> {
        value
            .get(key)
            .and_then(|v| v.as_u64())
            .ok_or_else(|| err(format!("manifest missing {key}")))
    };
    let bool_v = |key: &str| -> Result<bool, HarnessError> {
        value
            .get(key)
            .and_then(|v| v.as_bool())
            .ok_or_else(|| err(format!("manifest missing {key}")))
    };
    let bytes32 = |key: &str| -> Result<[u8; 32], HarnessError> {
        let s = value
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| err(format!("manifest missing {key}")))?;
        let hex = s.strip_prefix("0x").unwrap_or(s);
        if hex.len() != 64 {
            return Err(err(format!(
                "manifest {key} not a 32-byte hex value: len={}",
                hex.len()
            )));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let byte_str = std::str::from_utf8(chunk)
                .map_err(|_| err(format!("manifest {key} non-utf8 hex")))?;
            out[i] = u8::from_str_radix(byte_str, 16)
                .map_err(|e| err(format!("manifest {key} bad hex: {e}")))?;
        }
        Ok(out)
    };
    Ok(V2DeployedAddresses {
        chain_id: uint_u64("chainId")?,
        market_id: uint_u128("marketId")?,
        initial_price_1e8: uint_u128("initialPrice1e8")?,
        clearing_fund_raw: uint_u128("clearingFundRaw")?,
        sealed: bool_v("sealed")?,
        seal_hash: bytes32("sealHash")?,
        deployer: addr("deployer")?,
        usdc: addr("usdc")?,
        weth: addr("weth")?,
        primary_source: addr("primarySource")?,
        secondary_source: addr("secondarySource")?,
        oracle_router: addr("oracleRouter")?,
        vault: addr("vault")?,
        perp_market_registry: addr("perpMarketRegistry")?,
        perp_engine_v2: addr("perpEngineV2")?,
        perp_matching_engine_v2: addr("perpMatchingEngineV2")?,
        perp_clearing_account_v2: addr("perpClearingAccountV2")?,
        risk: addr("risk")?,
        trader_a: addr("traderA")?,
        trader_b: addr("traderB")?,
        executor: addr("executor")?,
    })
}

fn build_v2_app_state(
    contracts: &V2DeployedAddresses,
    anvil_url: &str,
    _trader_a: &V2Wallet,
    _trader_b: &V2Wallet,
    executor: &V2Wallet,
    repository: PgRepository,
    clearing_min_balance_raw: u128,
) -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());

    // PerpsReadConfig.
    let mut cfg = PerpsReadConfig::enabled_in_memory_for_tests();
    cfg.chain_id = HARNESS_CHAIN_ID;
    cfg.rpc_url = Some(anvil_url.to_string());
    cfg.market_registry_address = Some(AccountId::new(contracts.perp_market_registry.clone()));
    cfg.oracle_router_address = Some(AccountId::new(contracts.oracle_router.clone()));
    if let Some(eth) = cfg.markets.iter_mut().find(|m| m.symbol == "ETH-PERP") {
        eth.base_asset_address = AccountId::new(contracts.weth.clone());
        eth.quote_asset_address = AccountId::new(contracts.usdc.clone());
        eth.onchain_market_id = contracts.market_id as u64;
    }
    state.perps_read_config = cfg;

    // Execution config — active=V2.
    let mut exec = ExecutionConfig::disabled();
    exec.executor_chain_id = HARNESS_CHAIN_ID;
    exec.rpc_url = Some(anvil_url.to_string());
    exec.executor_from_address = AccountId::new(contracts.executor.clone());
    // V1 fallback addresses MUST be non-zero AND distinct from V2 to
    // pass `validate_startup`. Deployer works — it's a plain EOA that
    // will never be called against as a contract.
    let v1_placeholder = AccountId::new(contracts.deployer.clone());
    exec.perp_matching_engine_address = v1_placeholder.clone();
    exec.perp_engine_address = v1_placeholder;
    exec.perp_engine_v2_address = Some(AccountId::new(contracts.perp_engine_v2.clone()));
    exec.perp_matching_engine_v2_address =
        Some(AccountId::new(contracts.perp_matching_engine_v2.clone()));
    exec.perp_clearing_account_v2_address =
        Some(AccountId::new(contracts.perp_clearing_account_v2.clone()));
    exec.perps_active_engine_version = PerpsProtocolVersion::V2;
    exec.perps_v2_clearing_min_balance_raw = clearing_min_balance_raw;
    exec.simulation_enabled = true;
    exec.simulation_requires_persistence = true;
    exec.require_simulation_ok = true;
    exec.real_broadcast_enabled = false; // hard NO for this milestone
    state.execution_config = exec;

    // Closed-test flag + allowlist (traders A/B).
    state.perps_closed_test_enabled = true;
    state.perps_public_trading_enabled = false;
    state.perps_closed_test_allowlist = vec![
        AccountId::new(contracts.trader_a.clone()),
        AccountId::new(contracts.trader_b.clone()),
    ];

    state.repository = Some(repository);
    state.persistence_enabled = true;
    state.database_configured = true;
    state.chain_id = HARNESS_CHAIN_ID;

    // Silence "executor unused" — the address is baked into
    // executor_from_address above; the argument keeps the fn signature
    // symmetric with the wallets struct.
    let _ = executor;

    state
}

async fn spawn_backend(state: AppState) -> Result<(String, JoinHandle<()>), HarnessError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| err(format!("bind backend: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| err(format!("backend local_addr: {e}")))?;
    let app: Router = router(state);
    let url = format!("http://{addr}");
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    poll_backend_ready(&url).await?;
    Ok((url, task))
}

async fn poll_backend_ready(url: &str) -> Result<(), HarnessError> {
    let client = reqwest::Client::new();
    let started = Instant::now();
    loop {
        if started.elapsed() > BACKEND_READY_TIMEOUT {
            return Err(err(format!(
                "backend not ready within {BACKEND_READY_TIMEOUT:?} at {url}"
            )));
        }
        if let Ok(resp) = client.get(format!("{url}/health")).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(READY_POLL_INTERVAL).await;
    }
}

static MIGRATED: OnceCell<()> = OnceCell::const_new();

async fn ensure_migrated(url: &str) -> Result<(), HarnessError> {
    MIGRATED
        .get_or_try_init(|| async {
            let repo = PgRepository::connect(url)
                .await
                .map_err(|e| err(format!("pg connect for migration: {e}")))?;
            repo.run_migrations()
                .await
                .map_err(|e| err(format!("pg run_migrations: {e}")))?;
            Ok::<(), HarnessError>(())
        })
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------
// EIP-712 signing helper. Uses the standard {27,28} v convention that
// the backend `cosign_verify_core_for_version` accepts.
// ---------------------------------------------------------------------

fn sign_digest_65(key: &SigningKey, digest: &[u8; 32]) -> String {
    let (sig, rec_id) = key.sign_prehash_recoverable(digest).unwrap();
    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();
    let mut out = String::from("0x");
    for b in r.iter().chain(s.iter()) {
        out.push_str(&format!("{b:02x}"));
    }
    let v = 27u8 + rec_id.to_byte();
    out.push_str(&format!("{v:02x}"));
    out
}

fn hex_no_prefix(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn decode_hex_bytes(s: &str) -> Vec<u8> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    (0..stripped.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&stripped[i..i + 2], 16).unwrap())
        .collect()
}

// ---------------------------------------------------------------------
// Fixture — build the close-candidate `PrepareTradeRequest`.
//
// buyer = trader B (short, closing his -1_000_000)
// seller = trader A (long, closing his +1_000_000)
// sizeDelta1e8 = 1_000_000
// buyerIsMaker = true
// bounds: exec ± 1%   → NON-TRIVIAL, min <= exec <= max.
// ---------------------------------------------------------------------

fn build_close_candidate(env: &V2E2eEnv) -> PrepareTradeRequest {
    let exec = CANDIDATE_PRICE_1E8;
    let one_pct = exec / 100;
    PrepareTradeRequest {
        buyer: env.wallets.trader_b.address.clone(),
        seller: env.wallets.trader_a.address.clone(),
        market_id: MARKET_ID.to_string(),
        size_delta_1e8: CANDIDATE_SIZE_1E8.to_string(),
        buyer_is_maker: true,
        max_execution_price_1e8: Some((exec + one_pct).to_string()),
        min_execution_price_1e8: Some((exec - one_pct).to_string()),
    }
}

fn pg_url_or_ignore(label: &str) -> Option<String> {
    match std::env::var(PG_ENV_VAR) {
        Ok(url) if !url.is_empty() => Some(url),
        _ => {
            eprintln!(
                "IGNORED [{label}] ({PG_ENV_VAR} not provided). \
                 Set PERPS_CLOSED_TEST_E2E_PG_URL=postgres://user:pass@host/db to run."
            );
            None
        }
    }
}

fn is_missing_toolchain(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("no such file or directory")
        || m.contains("spawn anvil binary")
        || m.contains("cast exec")
        || m.contains("forge exec")
        || m.contains("permission denied")
}

/// Spawn or bail with `IGNORED` on a missing local toolchain. Every
/// integration test in this file funnels through this so a CI runner
/// without foundry installed produces `IGNORED` rather than a hard
/// panic. Callers pass a scenario label for the log line.
async fn spawn_or_ignore(label: &str, opts: V2SpawnOpts) -> Option<V2E2eEnv> {
    match V2E2eEnv::spawn(opts).await {
        Ok(env) => Some(env),
        Err(e) => {
            if is_missing_toolchain(&e.0) {
                eprintln!(
                    "IGNORED [{label}] (toolchain not available: {}). \
                     Install foundry (anvil + forge + cast) and re-run.",
                    e.0
                );
                None
            } else {
                panic!("V2E2eEnv::spawn failed for [{label}]: {e}");
            }
        }
    }
}

// =====================================================================
// TEST 1 — POSITIVE lifecycle (readiness → prepare → PG → cosign →
// simulation_ok → zero-send + decoded-calldata proofs).
// =====================================================================

#[tokio::test]
async fn v2_backend_anvil_live_positive_lifecycle() {
    let label = "v2_positive_lifecycle";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };

    // Deployment sanity — deployed local addresses (redacted-safe: no
    // secrets, only public contract addresses).
    eprintln!(
        "V2E2E deployed addresses:\n\
         vault={}\n perpEngineV2={}\n perpMatchingEngineV2={}\n \
         perpClearingAccountV2={}\n oracleRouter={}\n \
         perpMarketRegistry={}\n usdc={}\n executor={}",
        env.contracts.vault,
        env.contracts.perp_engine_v2,
        env.contracts.perp_matching_engine_v2,
        env.contracts.perp_clearing_account_v2,
        env.contracts.oracle_router,
        env.contracts.perp_market_registry,
        env.contracts.usdc,
        env.contracts.executor
    );

    // ── (A) Preflight — Ready ─────────────────────────────────────
    let preflight = env.run_v2_preflight().await;
    match &preflight.outcome {
        V2PreflightOutcome::Ready => {}
        V2PreflightOutcome::Denied(d) => {
            panic!(
                "expected Ready; got Denied({:?}). report={:?}",
                d, preflight
            )
        }
    }
    assert_eq!(preflight.migration_state, Some(MigrationState::Sealed));
    assert!(
        preflight
            .migration_snapshot_hash
            .expect("snapshot hash present")
            != [0u8; 32]
    );
    assert_eq!(preflight.pme_is_executor, Some(true));
    assert_eq!(preflight.pme_paused, Some(false));
    assert!(
        preflight.clearing_balance_raw.expect("balance present") >= POSITIVE_CLEARING_FLOOR_RAW
    );

    // ── (B) HTTP prepare (real route, non-trivial bounds) ────────
    let request = build_close_candidate(&env);
    let expected_max_bound: u128 = request
        .max_execution_price_1e8
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    let expected_min_bound: u128 = request
        .min_execution_price_1e8
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    assert!(expected_max_bound > 0);
    assert!(expected_min_bound > 0);
    let prepared = env.http_prepare(&request).await.expect("http prepare");
    let uuid = uuid::Uuid::parse_str(&prepared.uuid).expect("prepared uuid");

    // Prepare-response asserts.
    let domain = prepared
        .typed_data
        .get("domain")
        .expect("typedData.domain")
        .clone();
    assert_eq!(
        domain.get("name").and_then(|v| v.as_str()),
        Some("DeOptV2-PerpMatchingEngine")
    );
    assert_eq!(domain.get("version").and_then(|v| v.as_str()), Some("2"));
    assert_eq!(
        domain
            .get("verifyingContract")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase()),
        Some(env.contracts.perp_matching_engine_v2.clone())
    );
    let msg = prepared
        .typed_data
        .get("message")
        .expect("typedData.message");
    // All 12 fields present.
    for field in [
        "intentId",
        "buyer",
        "seller",
        "marketId",
        "sizeDelta1e8",
        "executionPrice1e8",
        "maxExecutionPrice1e8",
        "minExecutionPrice1e8",
        "buyerIsMaker",
        "buyerNonce",
        "sellerNonce",
        "deadline",
    ] {
        assert!(
            msg.get(field).is_some(),
            "typedData.message missing field {field}: {}",
            prepared.typed_data
        );
    }
    // Bounds non-trivial + ordered.
    assert_ne!(prepared.trade.max_execution_price_1e8, "0");
    assert_ne!(prepared.trade.min_execution_price_1e8, "0");
    let max_bound: u128 = prepared.trade.max_execution_price_1e8.parse().unwrap();
    let min_bound: u128 = prepared.trade.min_execution_price_1e8.parse().unwrap();
    let exec_price: u128 = prepared.trade.execution_price_1e8.parse().unwrap();
    assert!(min_bound <= exec_price && exec_price <= max_bound);
    assert_eq!(max_bound, expected_max_bound);
    assert_eq!(min_bound, expected_min_bound);

    // ── (C) Reload from Postgres and prove all 12 fields survive ─
    // NOTE: The prepare HTTP handler already persisted the intent
    // (see `perps_closed_test_prepare_trade`); we're the second
    // reader of the row and prove PG-durability by round-trip.
    let intent = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("pg reload")
        .expect("row exists");
    assert_eq!(intent.protocol_version, PerpsProtocolVersion::V2);
    assert_eq!(intent.status, ExecutionIntentStatus::Pending);
    assert_eq!(intent.buyer_nonce, Some(0));
    assert_eq!(intent.seller_nonce, Some(0));
    assert_eq!(u128::from(intent.size_1e8), CANDIDATE_SIZE_1E8);
    assert_eq!(u128::from(intent.price_1e8), CANDIDATE_PRICE_1E8);
    assert_eq!(intent.max_execution_price_1e8, max_bound);
    assert_eq!(intent.min_execution_price_1e8, min_bound);
    assert!(intent.buyer_is_maker == Some(true));

    // Reload domain from the intent's PERSISTED version — never the
    // runtime active version. This is what cosign will do server-side.
    let v2_verifying = AccountId::new(env.contracts.perp_matching_engine_v2.clone());
    let reload_domain =
        PerpTradeDomain::for_version(intent.protocol_version, env.chain_id, v2_verifying.clone());
    assert_eq!(reload_domain.version, "2");

    let payload_after: PerpTradePayload = intent.perp_trade_payload().expect("payload reconstruct");
    // 12-field equality against the prepare-response typed data.
    assert_eq!(
        format!("0x{}", hex_no_prefix(payload_after.intent_id.as_slice())),
        prepared.intent_id_hex
    );
    assert_eq!(
        payload_after.buyer.0.to_ascii_lowercase(),
        env.wallets.trader_b.address
    );
    assert_eq!(
        payload_after.seller.0.to_ascii_lowercase(),
        env.wallets.trader_a.address
    );
    assert_eq!(payload_after.market_id, MARKET_ID);
    assert_eq!(payload_after.size_delta_1e8, CANDIDATE_SIZE_1E8);
    assert_eq!(payload_after.execution_price_1e8, CANDIDATE_PRICE_1E8);
    assert_eq!(payload_after.max_execution_price_1e8, max_bound);
    assert_eq!(payload_after.min_execution_price_1e8, min_bound);
    assert!(payload_after.buyer_is_maker);
    assert_eq!(payload_after.buyer_nonce, 0);
    assert_eq!(payload_after.seller_nonce, 0);

    // Digest equality — pre-persistence vs post-reload MUST match
    // byte-for-byte or downstream signatures are refused.
    let digest_before = decode_hex_bytes(&prepared.digest);
    assert_eq!(digest_before.len(), 32);
    let digest_after =
        perp_trade_v2_digest_bytes(&payload_after, &reload_domain).expect("digest_after");
    assert_eq!(
        digest_before.as_slice(),
        digest_after.as_slice(),
        "V2 digest must survive PG round-trip byte-for-byte"
    );

    // ── (D) Active-version flip durability ───────────────────────
    // Temporarily flip runtime active version to V1, prove the
    // persisted V2 intent is still reconstructed as a V2 payload +
    // still hashes to the same digest (persisted version wins).
    let mut flipped = (*env.state).clone();
    flipped.execution_config.perps_active_engine_version = PerpsProtocolVersion::V1;
    // The reload still uses `intent.protocol_version` (V2), so the
    // domain built with `for_version(intent.protocol_version, ...)`
    // MUST target the V2 PME — not the runtime "active" V1 fallback.
    let intent_after_flip = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("reload during flip")
        .expect("row present during flip");
    assert_eq!(
        intent_after_flip.protocol_version,
        PerpsProtocolVersion::V2,
        "persisted intent's protocol_version must be immutable under runtime flip"
    );
    let flip_verifying = flipped
        .execution_config
        .perp_matching_engine_address_for(intent_after_flip.protocol_version)
        .expect("verifying for V2")
        .clone();
    let flip_domain = PerpTradeDomain::for_version(
        intent_after_flip.protocol_version,
        flipped.perps_read_config.chain_id,
        flip_verifying,
    );
    let payload_after_flip = intent_after_flip
        .perp_trade_payload()
        .expect("payload during flip");
    let digest_after_flip =
        perp_trade_v2_digest_bytes(&payload_after_flip, &flip_domain).expect("digest during flip");
    assert_eq!(
        digest_after_flip.as_slice(),
        digest_before.as_slice(),
        "runtime active-version flip MUST NOT alter the digest of an already-persisted V2 intent"
    );
    // The bounds MUST also be unchanged.
    assert_eq!(payload_after_flip.max_execution_price_1e8, max_bound);
    assert_eq!(payload_after_flip.min_execution_price_1e8, min_bound);

    // ── (E) Ephemeral signing with local trader keys ─────────────
    let digest_bytes: [u8; 32] = digest_before
        .as_slice()
        .try_into()
        .expect("digest is 32 bytes");
    let buyer_sig = sign_digest_65(&env.wallets.trader_b.signer, &digest_bytes);
    let seller_sig = sign_digest_65(&env.wallets.trader_a.signer, &digest_bytes);
    // Local recovery sanity — buyer_sig recovers trader_b, seller_sig
    // recovers trader_a. Uses the backend's canonical recover fn to
    // catch any encoding drift.
    let recovered_buyer =
        deopt_v2_backend::signing::recover_eip712_signer(&digest_bytes, &buyer_sig)
            .expect("recover buyer");
    assert_eq!(
        recovered_buyer.0.to_ascii_lowercase(),
        env.wallets.trader_b.address
    );
    let recovered_seller =
        deopt_v2_backend::signing::recover_eip712_signer(&digest_bytes, &seller_sig)
            .expect("recover seller");
    assert_eq!(
        recovered_seller.0.to_ascii_lowercase(),
        env.wallets.trader_a.address
    );

    // ── (F) Real HTTP cosign after PG reload ─────────────────────
    let cosign_response = env
        .http_cosign(uuid, &buyer_sig, &seller_sig)
        .await
        .expect("http cosign");
    assert!(cosign_response.calldata_ready);
    let signatures = env
        .repository
        .get_execution_intent_signatures(uuid)
        .await
        .expect("signatures reload");
    assert!(signatures.buyer_signature_present());
    assert!(signatures.seller_signature_present());

    let intent_after_cosign = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("reload after cosign")
        .expect("row present after cosign");
    assert_eq!(
        intent_after_cosign.status,
        ExecutionIntentStatus::CalldataReady
    );

    // ── (G) Preflight (again — still Ready) ──────────────────────
    let preflight_2 = env.run_v2_preflight().await;
    assert!(
        preflight_2.outcome.is_ready(),
        "preflight 2: {preflight_2:?}"
    );

    // Record pre-simulation state for the zero-send proof.
    let nonce_before = env.executor_anvil_nonce().await.expect("executor nonce");

    // ── (H) Real HTTP simulate → simulation_ok ───────────────────
    let sim = env.http_simulate(uuid).await.expect("http simulate");
    assert_eq!(
        sim.get("simulation_status").and_then(|v| v.as_str()),
        Some("simulation_ok"),
        "sim body={sim}"
    );
    assert!(sim.get("error").map(|v| v.is_null()).unwrap_or(true));
    assert!(sim.get("revert_data").map(|v| v.is_null()).unwrap_or(true));
    assert!(sim
        .get("revert_selector")
        .map(|v| v.is_null())
        .unwrap_or(true));

    // Exactly one row in the intent's simulation-status advancement.
    let intent_final = env
        .repository
        .get_execution_intent(uuid)
        .await
        .expect("reload final")
        .expect("row present final");
    assert_eq!(intent_final.status, ExecutionIntentStatus::SimulationOk);

    // ── (I) Independent V2 calldata decode ───────────────────────
    // Reconstruct calldata using the persisted payload + persisted
    // signatures and prove every field byte-matches the persisted
    // intent — no second source of truth.
    let stored_signatures = signatures.clone();
    let bundle = PerpTradeSignatureBundle::new(
        stored_signatures.buyer_sig.as_deref().unwrap(),
        stored_signatures.seller_sig.as_deref().unwrap(),
    )
    .expect("bundle build");
    let calldata =
        encode_execute_trade_v2_calldata(&payload_after, &bundle).expect("v2 calldata encode");
    // Selector byte-for-byte match against the canonical V2 selector.
    assert_eq!(&calldata[..4], &execute_trade_v2_selector()[..]);

    let (decoded_tuple, _decoded_buyer_sig, _decoded_seller_sig) =
        decode_execute_trade_v2_calldata(&calldata).expect("v2 calldata decode");

    // `decoded_tuple` is an alloy `sol!` struct — Solidity-style
    // camelCase field names. Every field MUST byte-match the persisted
    // canonical `PerpTradePayload` recovered from PG.
    assert_eq!(decoded_tuple.intentId, payload_after.intent_id);
    let decoded_buyer_addr = format!("0x{}", hex_no_prefix(decoded_tuple.buyer.as_slice()));
    let decoded_seller_addr = format!("0x{}", hex_no_prefix(decoded_tuple.seller.as_slice()));
    assert_eq!(
        decoded_buyer_addr.to_ascii_lowercase(),
        payload_after.buyer.0.to_ascii_lowercase()
    );
    assert_eq!(
        decoded_seller_addr.to_ascii_lowercase(),
        payload_after.seller.0.to_ascii_lowercase()
    );
    assert_eq!(
        u128::try_from(decoded_tuple.marketId).expect("marketId fits u128"),
        payload_after.market_id
    );
    assert_eq!(decoded_tuple.sizeDelta1e8, payload_after.size_delta_1e8);
    assert_eq!(
        decoded_tuple.executionPrice1e8,
        payload_after.execution_price_1e8
    );
    assert_eq!(
        decoded_tuple.maxExecutionPrice1e8,
        payload_after.max_execution_price_1e8
    );
    assert_eq!(
        decoded_tuple.minExecutionPrice1e8,
        payload_after.min_execution_price_1e8
    );
    assert_eq!(decoded_tuple.buyerIsMaker, payload_after.buyer_is_maker);
    assert_eq!(
        u128::try_from(decoded_tuple.buyerNonce).expect("buyerNonce fits u128"),
        payload_after.buyer_nonce
    );
    assert_eq!(
        u128::try_from(decoded_tuple.sellerNonce).expect("sellerNonce fits u128"),
        payload_after.seller_nonce
    );
    assert_eq!(
        u128::try_from(decoded_tuple.deadline).expect("deadline fits u128"),
        payload_after.deadline
    );

    // ── (J) Zero-send proof ──────────────────────────────────────
    let nonce_after = env.executor_anvil_nonce().await.expect("post-nonce");
    assert_eq!(
        nonce_before, nonce_after,
        "executor Anvil nonce MUST NOT change after eth_call simulation"
    );

    // Broadcast count and execution-transaction count MUST be zero.
    let submitted = env
        .repository
        .find_submitted_transaction_by_intent(uuid)
        .await
        .expect("submitted lookup");
    assert!(
        submitted.is_none(),
        "no execution_transactions row expected after simulation-only lifecycle"
    );

    env.shutdown().await.expect("clean shutdown");
    eprintln!("V2E2E_POSITIVE_LIFECYCLE_OK");
}

// =====================================================================
// NEGATIVES — five REAL preflight refusal branches. Each spawns a
// dedicated env (or reuses the sealed default with a config mutation).
// None broadcasts.
// =====================================================================

// -------------------- N1. Migration OPEN --------------------

#[tokio::test]
async fn v2_negative_migration_open_denies_preflight() {
    let label = "v2_negative_migration_open";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let opts = V2SpawnOpts {
        seal_migration: false,
        ..V2SpawnOpts::default()
    };
    let Some(env) = spawn_or_ignore(label, opts).await else {
        return;
    };
    let report = env.run_v2_preflight().await;
    match &report.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::MigrationOpen) => {}
        other => panic!("expected MigrationOpen; got {other:?}. report={report:?}"),
    }
    env.shutdown().await.expect("clean shutdown");
}

// -------------------- N2. Clearing identity mismatch --------

#[tokio::test]
async fn v2_negative_clearing_account_mismatch_denies_preflight() {
    let label = "v2_negative_clearing_mismatch";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };
    // Bogus clearing address distinct from the real one: reuse the
    // deployer EOA — it's a valid (non-zero) address that has no
    // relationship to `PerpEngineV2.clearingAccount()`.
    let bogus_clearing = AccountId::new(env.contracts.deployer.clone());
    let report = env
        .run_v2_preflight_with(
            &AccountId::new(env.contracts.perp_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &bogus_clearing,
            env.opts.clearing_min_balance_raw,
        )
        .await;
    match &report.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::ClearingAccountMismatch { .. }) => {}
        other => panic!("expected ClearingAccountMismatch; got {other:?}. report={report:?}"),
    }
    env.shutdown().await.expect("clean shutdown");
}

// -------------------- N3. Clearing floor + recovery --------

#[tokio::test]
async fn v2_negative_clearing_floor_denies_then_lifts() {
    let label = "v2_negative_clearing_floor";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };

    // Read the actual on-chain balance so we can set the floor to
    // exactly `balance + 1` and prove ClearingBalanceBelowFloor.
    let ready = env.run_v2_preflight().await;
    let observed = ready
        .clearing_balance_raw
        .expect("clearing balance observed");
    let too_high_floor = observed + 1;
    let report_denied = env
        .run_v2_preflight_with(
            &AccountId::new(env.contracts.perp_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_clearing_account_v2.clone()),
            too_high_floor,
        )
        .await;
    match &report_denied.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::ClearingBalanceBelowFloor {
            observed: obs,
            floor,
        }) => {
            assert_eq!(*obs, observed);
            assert_eq!(*floor, too_high_floor);
        }
        other => {
            panic!("expected ClearingBalanceBelowFloor; got {other:?}. report={report_denied:?}")
        }
    }

    // Lift the floor to exactly `observed` — recovery MUST report Ready.
    let report_ok = env
        .run_v2_preflight_with(
            &AccountId::new(env.contracts.perp_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_clearing_account_v2.clone()),
            observed,
        )
        .await;
    assert!(
        report_ok.outcome.is_ready(),
        "expected Ready; got {report_ok:?}"
    );
    env.shutdown().await.expect("clean shutdown");
}

// -------------------- N4. Executor auth + recovery ---------

#[tokio::test]
async fn v2_negative_executor_not_authorized_denies_then_lifts() {
    let label = "v2_negative_executor_auth";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };

    // Revoke via the deployer-owned setExecutor path.
    env.cast_set_executor(false)
        .await
        .expect("setExecutor(false)");
    let report_denied = env.run_v2_preflight().await;
    match &report_denied.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::ExecutorNotAuthorized(addr)) => {
            assert_eq!(addr.0.to_ascii_lowercase(), env.contracts.executor);
        }
        other => panic!("expected ExecutorNotAuthorized; got {other:?}. report={report_denied:?}"),
    }

    // Re-authorise and confirm Ready.
    env.cast_set_executor(true)
        .await
        .expect("setExecutor(true) restore");
    let report_ok = env.run_v2_preflight().await;
    assert!(
        report_ok.outcome.is_ready(),
        "expected Ready; got {report_ok:?}"
    );
    env.shutdown().await.expect("clean shutdown");
}

// -------------------- N5. PME↔Engine linkage --------------

#[tokio::test]
async fn v2_negative_pme_engine_linkage_mismatch_denies_preflight() {
    let label = "v2_negative_pme_engine_linkage";
    if pg_url_or_ignore(label).is_none() {
        return;
    }
    let Some(env) = spawn_or_ignore(label, V2SpawnOpts::default()).await else {
        return;
    };
    // Pretend the operator configured a different V2 engine address
    // (deployer EOA). PME.perpEngine() on-chain still points at the
    // real engine, so the linkage check fails.
    let bogus_engine = AccountId::new(env.contracts.deployer.clone());
    let report = env
        .run_v2_preflight_with(
            &bogus_engine,
            &AccountId::new(env.contracts.perp_matching_engine_v2.clone()),
            &AccountId::new(env.contracts.perp_clearing_account_v2.clone()),
            env.opts.clearing_min_balance_raw,
        )
        .await;
    match &report.outcome {
        V2PreflightOutcome::Denied(V2PreflightDenial::PmeEngineLinkageMismatch { .. }) => {}
        // If the reader observed a mismatched clearingAccount FIRST
        // (because we pointed the engine at a non-contract deployer
        // EOA, whose eth_call returns 0x → decode error), the
        // preflight surfaces UpstreamRpcError instead. That's still a
        // fail-closed refusal of the mis-config; accept either.
        V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(_)) => {}
        V2PreflightOutcome::Denied(V2PreflightDenial::ClearingAccountMismatch { .. }) => {}
        other => panic!(
            "expected PmeEngineLinkageMismatch / UpstreamRpcError / ClearingAccountMismatch \
             (fail-closed on bogus engine); got {other:?}. report={report:?}"
        ),
    }
    env.shutdown().await.expect("clean shutdown");
}
