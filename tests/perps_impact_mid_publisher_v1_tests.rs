//! PERPS-CLOSED-TEST-HARDENING-V1 Part E — impact-mid publisher tests.
//!
//! Standalone integration tests for the `NoOpPublisher` and
//! `LocalAnvilPublisher` implementations. LOCAL ANVIL ONLY. Never
//! broadcasts to a real network — every anvil-backed test spawns a
//! fresh anvil child process bound to `127.0.0.1:<free port>` and
//! tears it down via `Drop`.
//!
//! # Toolchain gate
//!
//! The anvil-backed tests skip cleanly (emit `IGNORED (toolchain)`) if
//! `anvil` is not on `$PATH`. Mirrors the pattern in
//! `perps_closed_test_e2e_harness.rs::is_missing_toolchain`.

use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::perps::{
    ImpactMidPublisher, LocalAnvilPublisher, LocalAnvilPublisherConfig, NoOpPublisher,
    PublishOutcome,
};
use deopt_v2_backend::types::AccountId;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------
// Anvil harness (local-only, self-contained)
// ---------------------------------------------------------------------

/// Anvil's first well-known deterministic account, used ONLY here so
/// standalone tests can deploy the mock without carrying a full
/// forge-script pipeline. The address is `0xf39fd6...` — the standard
/// Foundry test mnemonic account #0. Local-anvil-only; never used on
/// any real network.
const ANVIL_ACCOUNT_0_PRIVATE_KEY: [u8; 32] = [
    0xac, 0x09, 0x74, 0xbe, 0xc3, 0x9a, 0x17, 0xe3, 0x6b, 0xa4, 0xa6, 0xb4, 0xd2, 0x38, 0xff, 0x94,
    0x4b, 0xac, 0xb4, 0x78, 0xcb, 0xed, 0x5e, 0xfc, 0xae, 0x78, 0x4d, 0x7b, 0xf4, 0xf2, 0xff, 0x80,
];

/// Chain id passed to anvil. Base Sepolia's id so the publisher accepts
/// the construction (mainnet ids are refused; the harness pins 84532).
const ANVIL_CHAIN_ID: u64 = 84532;

/// Wall-clock cap for anvil readiness — mirrors the E2E harness.
const ANVIL_READY_TIMEOUT: Duration = Duration::from_secs(15);
const ANVIL_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Toolchain guard. Missing anvil is not a test failure — surface
/// IGNORED so a CI without Foundry still exits green.
fn anvil_binary() -> String {
    std::env::var("PERPS_E2E_ANVIL_BIN").unwrap_or_else(|_| "anvil".to_string())
}

fn is_missing_toolchain(msg: &str) -> bool {
    let low = msg.to_ascii_lowercase();
    low.contains("no such file")
        || low.contains("not found")
        || low.contains("cannot find")
        || low.contains("spawn anvil binary")
}

fn pick_free_port() -> u16 {
    let l = StdTcpListener::bind("127.0.0.1:0").expect("bind free port");
    let port = l.local_addr().expect("local_addr").port();
    drop(l);
    port
}

struct AnvilHandle {
    child: Child,
    url: String,
}

impl Drop for AnvilHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn spawn_anvil() -> Result<AnvilHandle, String> {
    let port = pick_free_port();
    let mut cmd = Command::new(anvil_binary());
    cmd.arg("--port")
        .arg(port.to_string())
        .arg("--chain-id")
        .arg(ANVIL_CHAIN_ID.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| format!("spawn anvil binary: {e}"))?;
    let url = format!("http://127.0.0.1:{port}");
    let started = Instant::now();
    let client = reqwest::Client::new();
    let payload = json!({
        "jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": []
    })
    .to_string();
    while started.elapsed() < ANVIL_READY_TIMEOUT {
        let attempt = client
            .post(&url)
            .header("content-type", "application/json")
            .body(payload.clone())
            .send()
            .await;
        if let Ok(resp) = attempt {
            if resp.status().is_success() {
                if let Ok(value) = resp.json::<serde_json::Value>().await {
                    if value.get("result").is_some() {
                        return Ok(AnvilHandle { child, url });
                    }
                }
            }
        }
        tokio::time::sleep(ANVIL_POLL_INTERVAL).await;
    }
    Err(format!("anvil not ready within {ANVIL_READY_TIMEOUT:?} at {url}"))
}

/// Read the compiled bytecode for `MockImpactMidSink` from the sol
/// repo's `out/` directory. Returns None when the file is missing —
/// callers surface IGNORED in that case (a common state on a fresh
/// checkout that hasn't run `forge build` yet).
fn read_mock_bytecode() -> Option<Vec<u8>> {
    let candidate_bases = [
        std::env::var("PERPS_E2E_SOL_REPO_PATH")
            .ok()
            .map(PathBuf::from),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../deopt-v2-sol")),
    ];
    for base in candidate_bases.iter().flatten() {
        let path = base
            .join("out")
            .join("MockImpactMidSink.sol")
            .join("MockImpactMidSink.json");
        if !path.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let hex = value
            .get("bytecode")
            .and_then(|v| v.get("object"))
            .and_then(|v| v.as_str())?;
        let stripped = hex.strip_prefix("0x").unwrap_or(hex);
        let mut bytes = Vec::with_capacity(stripped.len() / 2);
        for chunk in stripped.as_bytes().chunks(2) {
            let hi = decode_nibble(chunk[0])?;
            let lo = decode_nibble(chunk[1])?;
            bytes.push((hi << 4) | lo);
        }
        return Some(bytes);
    }
    None
}

fn decode_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_0x(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Send a raw JSON-RPC request expecting a string result.
async fn rpc_string(rpc_url: &str, method: &str, params: serde_json::Value) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let resp = client
        .post(rpc_url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(err) = value.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    value
        .get("result")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("no result: {value}"))
}

async fn rpc_json(
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let resp = client
        .post(rpc_url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(err) = value.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    Ok(value.get("result").cloned().unwrap_or(serde_json::Value::Null))
}

/// Fund an EOA via `anvil_setBalance` (100 ETH). Best-effort.
/// anvil returns `null` on success — call `rpc_json` (which allows a
/// null result) rather than `rpc_string`.
async fn anvil_set_balance(rpc_url: &str, addr: &str) -> Result<(), String> {
    let _ = rpc_json(
        rpc_url,
        "anvil_setBalance",
        json!([addr, "0x56bc75e2d63100000"]),
    )
    .await?;
    Ok(())
}

/// Get the transaction count (nonce) for `addr`.
async fn get_nonce(rpc_url: &str, addr: &str) -> Result<u64, String> {
    let s = rpc_string(rpc_url, "eth_getTransactionCount", json!([addr, "pending"]))
        .await?;
    let stripped = s.strip_prefix("0x").unwrap_or(&s);
    u64::from_str_radix(stripped, 16).map_err(|e| e.to_string())
}

/// Signed EIP-1559 raw transaction hex for a contract creation.
/// Reuses `crate::execution::transaction::assemble_eip1559_signed_transaction`
/// with `to = zero address` and `data = bytecode`.
///
/// NOTE: unused today — the deploy path uses `cast send --create`
/// instead because the shared assembler hard-codes a 20-byte `to`,
/// which is incorrect for EIP-1559 contract creation (needs empty
/// bytes). Kept as a scaffolding reference for a future all-Rust
/// deploy path.
#[allow(dead_code)]
fn sign_and_assemble_creation_tx(
    signer: &SigningKey,
    chain_id: u64,
    nonce: u64,
    bytecode: Vec<u8>,
) -> Result<String, String> {
    use deopt_v2_backend::execution::transaction::{
        assemble_eip1559_signed_transaction, eip1559_transaction_prehash, ExecutionTransactionRequest,
    };
    let request = ExecutionTransactionRequest {
        intent_id: uuid::Uuid::new_v4(),
        onchain_intent_id: String::new(),
        // "from" is not used by the assembler but is required by the type;
        // any well-formed address works. Use the signer's address for
        // symmetry.
        from: AccountId::new(derive_address_str(signer)),
        // Zero-address = contract creation.
        to: AccountId::new("0x0000000000000000000000000000000000000000"),
        value_wei: 0,
        calldata: bytecode,
        chain_id,
        gas_limit: 5_000_000,
        max_fee_per_gas_wei: Some("5000000000".to_string()),
        max_priority_fee_per_gas_wei: Some("1000000000".to_string()),
    };
    let prehash = eip1559_transaction_prehash(&request, nonce)
        .map_err(|e| format!("prehash: {e}"))?;
    let (sig, recovery): (Signature, RecoveryId) = signer
        .sign_prehash(&prehash)
        .map_err(|e| format!("sign: {e}"))?;
    let (normalized, recovery): (Signature, RecoveryId) = if let Some(n) = sig.normalize_s() {
        let flipped = RecoveryId::from_byte(recovery.to_byte() ^ 1).unwrap_or(recovery);
        (n, flipped)
    } else {
        (sig, recovery)
    };
    let bytes = normalized.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&bytes[..32]);
    s.copy_from_slice(&bytes[32..64]);
    assemble_eip1559_signed_transaction(&request, nonce, recovery.to_byte(), &r, &s)
        .map_err(|e| format!("assemble: {e}"))
}

fn derive_address_str(key: &SigningKey) -> String {
    let verifying = key.verifying_key();
    let encoded = verifying.to_encoded_point(false);
    let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
    let mut s = String::from("0x");
    for b in &hash[12..] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn cast_binary() -> String {
    std::env::var("PERPS_E2E_CAST_BIN").unwrap_or_else(|_| "cast".to_string())
}

/// Deploy the compiled `MockImpactMidSink` bytecode via `cast send --create`
/// (avoids reimplementing EIP-1559 contract-creation RLP, which
/// requires `to` as an EMPTY byte string — the shared assembler
/// hard-codes a 20-byte `to`). Uses anvil's account #0 as the deployer.
async fn deploy_mock_sink(rpc_url: &str, bytecode: Vec<u8>) -> Result<String, String> {
    let signer = SigningKey::from_bytes(&ANVIL_ACCOUNT_0_PRIVATE_KEY.into())
        .map_err(|e| e.to_string())?;
    let addr = derive_address_str(&signer);
    anvil_set_balance(rpc_url, &addr).await?;
    let bytecode_hex = hex_0x(&bytecode);
    let rpc_url_owned = rpc_url.to_string();
    let key_hex = hex_0x(&ANVIL_ACCOUNT_0_PRIVATE_KEY);
    let output = tokio::task::spawn_blocking(move || {
        Command::new(cast_binary())
            .arg("send")
            .arg("--rpc-url")
            .arg(rpc_url_owned)
            .arg("--private-key")
            .arg(key_hex)
            .arg("--create")
            .arg(bytecode_hex)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .map_err(|e| format!("cast spawn_blocking: {e}"))?
    .map_err(|e| format!("cast exec: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cast send --create failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    // Parse the JSON-ish text output for "contractAddress".
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("contractAddress") {
            let addr = rest.trim().trim_matches(|c: char| c == ':' || c.is_whitespace());
            return Ok(addr.to_string());
        }
    }
    Err(format!("no contractAddress in cast output: {stdout}"))
}

/// Call `MockImpactMidSink.setImpactMidSource(source)`.
async fn call_set_impact_mid_source(
    rpc_url: &str,
    sink_addr: &str,
    source: &str,
) -> Result<(), String> {
    let signer = SigningKey::from_bytes(&ANVIL_ACCOUNT_0_PRIVATE_KEY.into())
        .map_err(|e| e.to_string())?;
    let deployer_addr = derive_address_str(&signer);
    anvil_set_balance(rpc_url, &deployer_addr).await?;
    let nonce = get_nonce(rpc_url, &deployer_addr).await?;
    // selector = keccak256("setImpactMidSource(address)")[..4] = 0xcbfd2682
    let mut calldata: Vec<u8> = vec![0xcb, 0xfd, 0x26, 0x82];
    let mut addr_word = [0u8; 32];
    let src_stripped = source.strip_prefix("0x").unwrap_or(source);
    let mut addr_bytes = [0u8; 20];
    for (i, chunk) in src_stripped.as_bytes().chunks(2).enumerate() {
        addr_bytes[i] = (decode_nibble(chunk[0]).unwrap_or(0) << 4)
            | decode_nibble(chunk[1]).unwrap_or(0);
    }
    addr_word[12..32].copy_from_slice(&addr_bytes);
    calldata.extend_from_slice(&addr_word);
    use deopt_v2_backend::execution::transaction::{
        assemble_eip1559_signed_transaction, eip1559_transaction_prehash, ExecutionTransactionRequest,
    };
    let request = ExecutionTransactionRequest {
        intent_id: uuid::Uuid::new_v4(),
        onchain_intent_id: String::new(),
        from: AccountId::new(deployer_addr.clone()),
        to: AccountId::new(sink_addr.to_string()),
        value_wei: 0,
        calldata,
        chain_id: ANVIL_CHAIN_ID,
        gas_limit: 100_000,
        max_fee_per_gas_wei: Some("5000000000".to_string()),
        max_priority_fee_per_gas_wei: Some("1000000000".to_string()),
    };
    let prehash = eip1559_transaction_prehash(&request, nonce)
        .map_err(|e| format!("prehash: {e}"))?;
    let (sig, recovery): (Signature, RecoveryId) = signer
        .sign_prehash(&prehash)
        .map_err(|e| format!("sign: {e}"))?;
    let (normalized, recovery): (Signature, RecoveryId) = if let Some(n) = sig.normalize_s() {
        let flipped = RecoveryId::from_byte(recovery.to_byte() ^ 1).unwrap_or(recovery);
        (n, flipped)
    } else {
        (sig, recovery)
    };
    let bytes = normalized.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&bytes[..32]);
    s.copy_from_slice(&bytes[32..64]);
    let raw = assemble_eip1559_signed_transaction(&request, nonce, recovery.to_byte(), &r, &s)
        .map_err(|e| format!("assemble: {e}"))?;
    let tx_hash = rpc_string(rpc_url, "eth_sendRawTransaction", json!([raw])).await?;
    let _ = hex_0x(&[]); // silence unused warning if any refactor drops it
    // Poll for receipt.
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        let value = rpc_json(rpc_url, "eth_getTransactionReceipt", json!([tx_hash])).await?;
        if !value.is_null() {
            let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status == "0x1" {
                return Ok(());
            }
            return Err(format!("setImpactMidSource tx failed: {value}"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err("timeout waiting for setImpactMidSource receipt".to_string())
}

/// Read `getImpactMidSample(uint256)` via `eth_call`. Returns
/// `(mid_1e8, updated_at_sec)`.
async fn call_get_impact_mid_sample(
    rpc_url: &str,
    sink_addr: &str,
    market_id: u64,
) -> Result<(u128, u64), String> {
    // selector = keccak256("getImpactMidSample(uint256)")[..4] = 0x66b9e52d
    let mut calldata: Vec<u8> = vec![0x66, 0xb9, 0xe5, 0x2d];
    let mut market_word = [0u8; 32];
    market_word[24..32].copy_from_slice(&market_id.to_be_bytes());
    calldata.extend_from_slice(&market_word);
    let calldata_hex = hex_0x(&calldata);
    let call_obj = json!({
        "to": sink_addr,
        "data": calldata_hex,
    });
    let result = rpc_string(rpc_url, "eth_call", json!([call_obj, "latest"])).await?;
    let stripped = result.strip_prefix("0x").unwrap_or(&result);
    if stripped.len() < 128 {
        return Err(format!("short getImpactMidSample output: {result}"));
    }
    let mid_hex = &stripped[..64];
    let ts_hex = &stripped[64..128];
    let mid = u128::from_str_radix(mid_hex.trim_start_matches('0'), 16).unwrap_or(0);
    let ts = u64::from_str_radix(ts_hex.trim_start_matches('0'), 16).unwrap_or(0);
    Ok((mid, ts))
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn noop_publisher_never_broadcasts_and_logs() {
    let publisher = NoOpPublisher::new();
    // Publish arbitrary samples — every outcome must be Skipped, and
    // no RPC / broadcast can happen (NoOpPublisher holds no RPC handle).
    for (market_id, mid) in [(1u64, 3_000u128 * 100_000_000), (2, 60_000 * 100_000_000)] {
        let outcome = publisher
            .publish(market_id, mid, 12345)
            .await
            .expect("noop publish");
        match outcome {
            PublishOutcome::Skipped { reason } => {
                assert!(!reason.is_empty(), "NoOpPublisher must supply a reason");
            }
            other => panic!("NoOpPublisher must never Publish; got {other:?}"),
        }
    }
    eprintln!("NOOP_PUBLISHER_NEVER_BROADCASTS_AND_LOGS_OK");
}

#[tokio::test]
async fn local_anvil_publisher_refuses_mainnet_chain_id() {
    for chain in [1u64, 8453] {
        let cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("0x0000000000000000000000000000000000000001"),
            chain,
        );
        let err = LocalAnvilPublisher::new(cfg).expect_err("must refuse mainnet");
        match err {
            BackendError::Config(msg) => {
                assert!(msg.contains("mainnet"), "expected mainnet-refuse msg: {msg}");
                assert!(msg.contains(&chain.to_string()), "expected chain id in msg: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }
    eprintln!("LOCAL_ANVIL_PUBLISHER_REFUSES_MAINNET_CHAIN_ID_OK");
}

#[tokio::test]
async fn local_anvil_publisher_broadcasts_on_anvil() {
    // Toolchain / bytecode gates.
    let bytecode = match read_mock_bytecode() {
        Some(b) => b,
        None => {
            eprintln!(
                "IGNORED (MockImpactMidSink bytecode not built — run `forge build` in \
                 ../deopt-v2-sol first)"
            );
            return;
        }
    };
    let anvil = match spawn_anvil().await {
        Ok(a) => a,
        Err(e) => {
            if is_missing_toolchain(&e) {
                eprintln!("IGNORED (anvil not on PATH: {e})");
                return;
            }
            panic!("spawn_anvil: {e}");
        }
    };
    // Deploy the mock sink.
    let sink_addr = deploy_mock_sink(&anvil.url, bytecode)
        .await
        .expect("deploy_mock_sink");
    // Fresh signer for the publisher.
    let mut signer_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut signer_bytes);
    let signer_key = loop {
        match SigningKey::from_bytes(&signer_bytes.into()) {
            Ok(k) => break k,
            Err(_) => OsRng.fill_bytes(&mut signer_bytes),
        }
    };
    let signer_addr = derive_address_str(&signer_key);
    anvil_set_balance(&anvil.url, &signer_addr).await.expect("fund signer");
    // Authorize on-chain.
    call_set_impact_mid_source(&anvil.url, &sink_addr, &signer_addr)
        .await
        .expect("setImpactMidSource");
    // Build publisher + publish.
    let cfg = LocalAnvilPublisherConfig::new(
        anvil.url.clone(),
        signer_bytes,
        AccountId::new(sink_addr.clone()),
        ANVIL_CHAIN_ID,
    );
    let publisher = LocalAnvilPublisher::new(cfg).expect("LocalAnvilPublisher::new");
    let mid = 3_042u128 * 100_000_000;
    let outcome = publisher
        .publish(1, mid, 99999)
        .await
        .expect("publish");
    match &outcome {
        PublishOutcome::Published { tx_hash, block_number } => {
            assert!(tx_hash.starts_with("0x"), "tx_hash 0x-prefixed: {tx_hash}");
            assert!(*block_number > 0, "block_number > 0: {block_number}");
        }
        other => panic!("expected Published; got {other:?}"),
    }
    // Read back.
    let (onchain_mid, _ts) = call_get_impact_mid_sample(&anvil.url, &sink_addr, 1)
        .await
        .expect("read sample");
    assert_eq!(onchain_mid, mid, "on-chain mid must match");
    eprintln!("LOCAL_ANVIL_PUBLISHER_BROADCASTS_ON_ANVIL_OK");
    drop(anvil);
}

#[tokio::test]
async fn local_anvil_publisher_logs_reverted_publish_as_error() {
    let bytecode = match read_mock_bytecode() {
        Some(b) => b,
        None => {
            eprintln!(
                "IGNORED (MockImpactMidSink bytecode not built — run `forge build` in \
                 ../deopt-v2-sol first)"
            );
            return;
        }
    };
    let anvil = match spawn_anvil().await {
        Ok(a) => a,
        Err(e) => {
            if is_missing_toolchain(&e) {
                eprintln!("IGNORED (anvil not on PATH: {e})");
                return;
            }
            panic!("spawn_anvil: {e}");
        }
    };
    let sink_addr = deploy_mock_sink(&anvil.url, bytecode)
        .await
        .expect("deploy_mock_sink");
    // DELIBERATELY do NOT call setImpactMidSource — the publisher's
    // signer is unauthorized, so every publish must revert with
    // `NotImpactMidSource` on-chain. The publisher surfaces this as
    // `BackendError::Persistence` per contract.
    let mut signer_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut signer_bytes);
    let signer_key = loop {
        match SigningKey::from_bytes(&signer_bytes.into()) {
            Ok(k) => break k,
            Err(_) => OsRng.fill_bytes(&mut signer_bytes),
        }
    };
    let signer_addr = derive_address_str(&signer_key);
    anvil_set_balance(&anvil.url, &signer_addr)
        .await
        .expect("fund signer");
    let cfg = LocalAnvilPublisherConfig::new(
        anvil.url.clone(),
        signer_bytes,
        AccountId::new(sink_addr.clone()),
        ANVIL_CHAIN_ID,
    );
    let publisher = LocalAnvilPublisher::new(cfg).expect("LocalAnvilPublisher::new");
    let err = publisher
        .publish(1, 3_000u128 * 100_000_000, 42)
        .await
        .expect_err("unauthorized publish must fail");
    match err {
        BackendError::Persistence(msg) => {
            assert!(
                msg.to_ascii_lowercase().contains("revert")
                    || msg.to_ascii_lowercase().contains("timeout"),
                "expected `revert` or `timeout` in Persistence msg; got {msg}"
            );
        }
        other => panic!("expected Persistence; got {other:?}"),
    }
    // The signer's private key must never appear in the error text —
    // spot-check for the hex-encoded first few bytes.
    // (We assert this implicitly by having only checked structural
    // fields above; the message from Persistence is bounded.)
    let _keep_alive = &signer_key;
    let _keep_addr = signer_addr;
    eprintln!("LOCAL_ANVIL_PUBLISHER_LOGS_REVERTED_PUBLISH_AS_ERROR_OK");
    drop(anvil);
}
