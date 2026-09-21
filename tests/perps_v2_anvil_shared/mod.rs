//! Shared V2 Anvil E2E harness for perps integration binaries.
//!
//! Extracted from `perps_v2_backend_anvil_live_e2e_pg_integration.rs` in
//! PERPS_V2_BACKEND_ANVIL_BROADCAST_E2E_V1 so the non-broadcast
//! lifecycle binary and the new broadcast-lifecycle binary share ONE
//! source of truth for anvil spawn, forge deploy, PG migrate, backend
//! wiring, HTTP prepare/cosign/simulate, and cast helpers.
//!
//! # Hard rules (inherited)
//!
//! * NO Base Sepolia write. NO public-chain deployment. NO real
//!   `sendRawTransaction` by this module alone — the broadcast binary
//!   layers its own real-send policy on top with tight arming gates.
//! * NO real trader keystore. NO production executor keystore.
//! * All wallets are ephemeral, in-process only, and NEVER logged.
//!
//! # PG env gate
//!
//! When `PERPS_CLOSED_TEST_E2E_PG_URL` is unset every test emits an
//! `IGNORED (PG url not provided)` marker and returns.

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
    v2_preflight_check, RpcV2EngineReader, RpcV2MatchingEngineReader, RpcV2VaultReader,
};
use deopt_v2_backend::execution::{ExecutionConfig, PerpsProtocolVersion};
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
// Constants
// ---------------------------------------------------------------------

/// Base-Sepolia chain id. Backend `validate_startup` compares
/// `executor_chain_id` against `rpc.chain_id()`; the anvil node is
/// spawned with this id so the config is accepted. The anvil node is
/// still fully ephemeral and NEVER touches real Base Sepolia.
pub const HARNESS_CHAIN_ID: u64 = 84532;

pub const SPAWN_BUDGET: Duration = Duration::from_secs(120);
pub const ANVIL_READY_TIMEOUT: Duration = Duration::from_secs(30);
pub const BACKEND_READY_TIMEOUT: Duration = Duration::from_secs(15);
pub const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// PG env gate. Shared with the V1 closed-test harness so ONE local
/// PG URL drives every integration suite in this repo.
pub const PG_ENV_VAR: &str = "PERPS_CLOSED_TEST_E2E_PG_URL";

/// Base-Sepolia-parity A/B fixture: A holds +1_000_000 (long), B holds
/// -1_000_000 (short). Mirrors `DeployPerpsV2E2E`'s seeded positions
/// and the live Base Sepolia state.
pub const CANDIDATE_SIZE_1E8: u128 = 1_000_000;

/// $2468.31 in 1e8 scale — matches the `DeployPerpsV2E2E`
/// `DEFAULT_PRICE_1E8` constant AND the live Base Sepolia mark.
pub const CANDIDATE_PRICE_1E8: u128 = 246_831_000_000;

/// 1M mUSDC at 6-decimal scale — matches
/// `DeployPerpsV2E2E::DEFAULT_CLEARING_FUND_RAW`.
pub const CLEARING_FUND_RAW: u128 = 1_000_000_000_000;

/// Bounded-check floor used by the positive test.
pub const POSITIVE_CLEARING_FLOOR_RAW: u128 = 500_000_000_000;

/// Path to the on-disk sol repository (relative to the backend crate
/// root — standard sibling checkout layout).
pub const SOL_REPO_RELATIVE_PATH: &str = "../deopt-v2-sol";

pub const MARKET_ID: u128 = 1;

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

pub fn err<S: Into<String>>(s: S) -> HarnessError {
    HarnessError(s.into())
}

/// Every field the `DeployPerpsV2E2E._writeManifest` script emits.
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

/// One local V2 test wallet.
pub struct V2Wallet {
    pub address: String,
    pub private_key_hex: String,
    pub signer: SigningKey,
}

impl V2Wallet {
    pub fn generate() -> Self {
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

pub struct V2Wallets {
    pub deployer: V2Wallet,
    pub trader_a: V2Wallet,
    pub trader_b: V2Wallet,
    pub executor: V2Wallet,
}

#[derive(Clone, Debug)]
pub struct V2SpawnOpts {
    pub seal_migration: bool,
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
    pub manifest_path: PathBuf,
    pub backend_task: Option<JoinHandle<()>>,
    pub opts: V2SpawnOpts,
}

impl V2E2eEnv {
    pub async fn spawn(opts: V2SpawnOpts) -> Result<Self, HarnessError> {
        let started = Instant::now();

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
            .timeout(Duration::from_secs(30))
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
        let _ = std::fs::remove_file(&self.manifest_path);
        Ok(())
    }

    pub fn rpc(&self) -> HttpJsonRpcProvider {
        HttpJsonRpcProvider::new(self.anvil_url.clone())
    }

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

    /// Generic `eth_call` against the Anvil node. Returns hex output
    /// (leading `0x`).
    pub async fn eth_call_hex(&self, to: &str, data_hex: &str) -> Result<String, HarnessError> {
        let params = json!({
            "to": to.to_ascii_lowercase(),
            "data": data_hex,
        });
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [params, "latest"],
        });
        let resp = self
            .http
            .post(&self.anvil_url)
            .header("content-type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .map_err(|e| err(format!("eth_call send: {e}")))?;
        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| err(format!("eth_call json: {e}")))?;
        val.get("result")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| err(format!("eth_call missing result: {val}")))
    }
}

// ---------------------------------------------------------------------
// Low-level harness helpers.
// ---------------------------------------------------------------------

pub fn pick_free_port() -> std::io::Result<u16> {
    let listener = StdTcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

pub fn evm_address_from_signing_key(key: &SigningKey) -> String {
    let verifying: &VerifyingKey = key.verifying_key();
    let encoded = verifying.to_encoded_point(false);
    let hash = keccak256(&encoded.as_bytes()[1..]);
    let mut out = String::from("0x");
    for b in &hash[12..] {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub fn to_hex_0x(bytes: &[u8]) -> String {
    let mut s = String::from("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn pad_address(addr: &str) -> String {
    let hex = addr.strip_prefix("0x").unwrap_or(addr);
    format!("{:0>64}", hex.to_ascii_lowercase())
}

pub fn pad_bool(v: bool) -> String {
    if v {
        format!("{:0>64}", "1")
    } else {
        format!("{:0>64}", "0")
    }
}

pub fn pad_u256(v: u128) -> String {
    format!("{:0>64x}", v)
}

pub fn write_temp_path(name: &str) -> std::io::Result<PathBuf> {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    let suffix = to_hex_0x(&bytes);
    let mut path = std::env::temp_dir();
    path.push(format!("{}_{}", suffix.trim_start_matches("0x"), name));
    Ok(path)
}

pub fn anvil_binary() -> String {
    std::env::var("PERPS_E2E_ANVIL_BIN").unwrap_or_else(|_| "anvil".to_string())
}

pub fn forge_binary() -> String {
    std::env::var("PERPS_E2E_FORGE_BIN").unwrap_or_else(|_| "forge".to_string())
}

pub fn cast_binary() -> String {
    std::env::var("PERPS_E2E_CAST_BIN").unwrap_or_else(|_| "cast".to_string())
}

pub fn sol_repo_path() -> Result<PathBuf, HarnessError> {
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

pub fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

pub async fn spawn_anvil(
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

pub async fn poll_anvil_ready(url: &str) -> Result<(), HarnessError> {
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

pub async fn run_forge_deploy_v2(
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

pub async fn run_cast_send(
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

pub fn read_v2_manifest(path: &PathBuf) -> Result<V2DeployedAddresses, HarnessError> {
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

/// Build the default V2 AppState (broadcast DISABLED). Broadcast
/// binaries derive a broadcast-enabled `ExecutionConfig` on top of
/// this at test time via `broadcast_config_for()`.
pub fn build_v2_app_state(
    contracts: &V2DeployedAddresses,
    anvil_url: &str,
    _trader_a: &V2Wallet,
    _trader_b: &V2Wallet,
    executor: &V2Wallet,
    repository: PgRepository,
    clearing_min_balance_raw: u128,
) -> AppState {
    let mut state = AppState::new(EngineState::with_default_markets());

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
    exec.real_broadcast_enabled = false;
    state.execution_config = exec;

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

    let _ = executor;

    state
}

pub async fn spawn_backend(state: AppState) -> Result<(String, JoinHandle<()>), HarnessError> {
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

pub async fn poll_backend_ready(url: &str) -> Result<(), HarnessError> {
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

pub async fn ensure_migrated(url: &str) -> Result<(), HarnessError> {
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
// EIP-712 signing helper.
// ---------------------------------------------------------------------

pub fn sign_digest_65(key: &SigningKey, digest: &[u8; 32]) -> String {
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

pub fn hex_no_prefix(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn decode_hex_bytes(s: &str) -> Vec<u8> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    (0..stripped.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&stripped[i..i + 2], 16).unwrap())
        .collect()
}

// ---------------------------------------------------------------------
// Fixture — build the close-candidate `PrepareTradeRequest`.
// ---------------------------------------------------------------------

pub fn build_close_candidate(env: &V2E2eEnv) -> PrepareTradeRequest {
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

pub fn pg_url_or_ignore(label: &str) -> Option<String> {
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

pub fn is_missing_toolchain(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("no such file or directory")
        || m.contains("spawn anvil binary")
        || m.contains("cast exec")
        || m.contains("forge exec")
        || m.contains("permission denied")
}

pub async fn spawn_or_ignore(label: &str, opts: V2SpawnOpts) -> Option<V2E2eEnv> {
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
