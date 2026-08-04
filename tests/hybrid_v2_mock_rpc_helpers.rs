//! Mock JSON-RPC harness for BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE tests.
//!
//! Spins up a local axum server on `127.0.0.1:0` that responds
//! deterministically to a curated set of methods. All received requests
//! are captured into a shared `Arc<Mutex<Vec<Call>>>` for assertion.
//!
//! The handler REJECTS every state-changing method with HTTP 500 —
//! any test that accidentally issues a broadcast method will fail
//! loudly, providing the static safety check for the "no write
//! surface" invariant.

#![allow(dead_code)]

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Json;
use axum::Router;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Snapshot of one RPC call received by the mock.
#[derive(Debug, Clone)]
pub struct Call {
    pub method: String,
    pub params: Value,
}

/// A block record served by the mock, matching the shape of an
/// eth_getBlockByNumber response for a header-only request.
#[derive(Debug, Clone)]
pub struct MockBlock {
    pub number: u64,
    pub hash: String,
    pub parent_hash: String,
    pub timestamp: u64,
    /// Every log served by `eth_getLogs` for this block.
    pub logs: Vec<MockLog>,
}

#[derive(Debug, Clone)]
pub struct MockLog {
    pub address: String,
    pub block_hash: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u32,
    /// 32-byte topics, 0x-prefixed.
    pub topics: Vec<String>,
    /// Hex-encoded data, 0x-prefixed.
    pub data: String,
}

impl MockLog {
    pub fn to_json(&self) -> Value {
        let topics: Vec<Value> = self.topics.iter().map(|t| json!(t)).collect();
        json!({
            "address": self.address,
            "blockHash": self.block_hash,
            "blockNumber": format!("0x{:x}", self.block_number),
            "transactionHash": self.tx_hash,
            "transactionIndex": "0x0",
            "logIndex": format!("0x{:x}", self.log_index),
            "topics": topics,
            "data": self.data,
            "removed": false,
        })
    }
}

impl MockBlock {
    pub fn header_json(&self) -> Value {
        json!({
            "number": format!("0x{:x}", self.number),
            "hash": self.hash,
            "parentHash": self.parent_hash,
            "timestamp": format!("0x{:x}", self.timestamp),
        })
    }
}

/// Mutable state driving the mock's responses. Shared between the
/// handler and the test controller.
#[derive(Debug)]
pub struct MockState {
    pub chain_id: u64,
    pub head: u64,
    pub finalized: Option<u64>,
    /// When false, respond with an RPC error to `eth_getBlockByNumber("finalized", ...)`.
    pub finalized_tag_supported: bool,
    /// When false, respond with an RPC error to `eth_getBlockByNumber("safe", ...)`.
    pub safe_tag_supported: bool,
    pub blocks_by_number: BTreeMap<u64, MockBlock>,
    pub blocks_by_hash: BTreeMap<String, MockBlock>,
    pub calls: Vec<Call>,
    /// Simulated 429s on the next N requests (any method).
    pub simulate_429_remaining: u32,
    /// Simulated 500s / other status codes on the next N requests.
    pub simulate_status_remaining: u32,
    pub simulate_status: u16,
    /// Simulated malformed JSON on the next N requests.
    pub simulate_malformed_remaining: u32,
    /// If Some, respond to eth_getLogs with a Deterministic RPC error
    /// code (not retryable) on the next call.
    pub simulate_next_rpc_error: Option<(i64, String)>,
    pub prohibited_calls_seen: HashSet<String>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            chain_id: 84532,
            head: 0,
            finalized: None,
            finalized_tag_supported: true,
            safe_tag_supported: true,
            blocks_by_number: BTreeMap::new(),
            blocks_by_hash: BTreeMap::new(),
            calls: Vec::new(),
            simulate_429_remaining: 0,
            simulate_status_remaining: 0,
            simulate_status: 500,
            simulate_malformed_remaining: 0,
            simulate_next_rpc_error: None,
            prohibited_calls_seen: HashSet::new(),
        }
    }
}

pub struct MockRpcServer {
    pub state: Arc<Mutex<MockState>>,
    pub addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl MockRpcServer {
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(MockState::default()));
        let router = Router::new()
            .route("/", post(handle_rpc))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let server = axum::serve(listener, router).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            let _ = server.await;
        });
        Self {
            state,
            addr,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    pub fn set_chain_id(&self, chain_id: u64) {
        self.state.lock().unwrap().chain_id = chain_id;
    }

    pub fn set_head(&self, head: u64) {
        self.state.lock().unwrap().head = head;
    }

    pub fn set_finalized(&self, block: u64) {
        self.state.lock().unwrap().finalized = Some(block);
    }

    pub fn set_finalized_tag_supported(&self, supported: bool) {
        self.state.lock().unwrap().finalized_tag_supported = supported;
    }

    pub fn set_safe_tag_supported(&self, supported: bool) {
        self.state.lock().unwrap().safe_tag_supported = supported;
    }

    pub fn push_block(&self, block: MockBlock) {
        let mut st = self.state.lock().unwrap();
        st.blocks_by_number.insert(block.number, block.clone());
        st.blocks_by_hash.insert(block.hash.clone(), block);
    }

    pub fn simulate_status(&self, next_n: u32, status: u16) {
        let mut st = self.state.lock().unwrap();
        st.simulate_status_remaining = next_n;
        st.simulate_status = status;
    }

    pub fn simulate_rate_limit(&self, next_n: u32) {
        self.state.lock().unwrap().simulate_429_remaining = next_n;
    }

    pub fn simulate_malformed(&self, next_n: u32) {
        self.state.lock().unwrap().simulate_malformed_remaining = next_n;
    }

    pub fn simulate_next_rpc_error(&self, code: i64, message: impl Into<String>) {
        self.state.lock().unwrap().simulate_next_rpc_error = Some((code, message.into()));
    }

    pub fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn prohibited_calls(&self) -> HashSet<String> {
        self.state.lock().unwrap().prohibited_calls_seen.clone()
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        }
    }
}

impl Drop for MockRpcServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

async fn handle_rpc(
    State(state): State<Arc<Mutex<MockState>>>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let (method, params) = extract_call(&body);
    // Refuse write methods hard. This is the static safety check.
    if is_prohibited_method(&method) {
        state
            .lock()
            .unwrap()
            .prohibited_calls_seen
            .insert(method.clone());
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "jsonrpc": "2.0", "id": body.get("id").cloned().unwrap_or(json!(1)),
                "error": {"code": -32601, "message": format!("method {method} forbidden in mock")}
            })),
        );
    }
    // Record.
    {
        let mut st = state.lock().unwrap();
        st.calls.push(Call {
            method: method.clone(),
            params: params.clone(),
        });
        if st.simulate_429_remaining > 0 {
            st.simulate_429_remaining -= 1;
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "too many requests"})),
            );
        }
        if st.simulate_status_remaining > 0 {
            st.simulate_status_remaining -= 1;
            let code = StatusCode::from_u16(st.simulate_status)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            return (code, Json(json!({"error": "injected status"})));
        }
        if st.simulate_malformed_remaining > 0 {
            st.simulate_malformed_remaining -= 1;
            // Return a non-JSON payload — but axum's Json response
            // sends JSON. Emulate by returning a body that lacks a
            // `result` field and lacks `error`.
            return (
                StatusCode::OK,
                Json(json!({"jsonrpc": "2.0", "id": 1, "junk": true})),
            );
        }
    }
    // Dispatch.
    let response = dispatch(state.clone(), &method, &params, body.get("id").cloned());
    (StatusCode::OK, Json(response))
}

fn extract_call(body: &Value) -> (String, Value) {
    let method = body
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string();
    let params = body.get("params").cloned().unwrap_or(json!([]));
    (method, params)
}

fn dispatch(
    state: Arc<Mutex<MockState>>,
    method: &str,
    params: &Value,
    id: Option<Value>,
) -> Value {
    let id = id.unwrap_or(json!(1));
    match method {
        "eth_chainId" => {
            let cid = state.lock().unwrap().chain_id;
            json!({"jsonrpc": "2.0", "id": id, "result": format!("0x{:x}", cid)})
        }
        "eth_blockNumber" => {
            let head = state.lock().unwrap().head;
            json!({"jsonrpc": "2.0", "id": id, "result": format!("0x{:x}", head)})
        }
        "eth_getBlockByNumber" => {
            let arr = params.as_array();
            let block_tag = arr
                .and_then(|a| a.get(0))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let st = state.lock().unwrap();
            match block_tag {
                "finalized" => {
                    if !st.finalized_tag_supported {
                        json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": {"code": -32000, "message": "unknown block tag finalized"}
                        })
                    } else if let Some(f) = st.finalized {
                        // Serve the corresponding block if we have it.
                        if let Some(b) = st.blocks_by_number.get(&f) {
                            json!({"jsonrpc": "2.0", "id": id, "result": b.header_json()})
                        } else {
                            json!({
                                "jsonrpc": "2.0", "id": id,
                                "result": {
                                    "number": format!("0x{:x}", f),
                                    "hash": format!("0xF{:0>63x}", f),
                                    "parentHash": format!("0xF{:0>63x}", f.saturating_sub(1)),
                                    "timestamp": format!("0x{:x}", 1000 + f * 12),
                                }
                            })
                        }
                    } else {
                        json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": {"code": -32000, "message": "no finalized block set"}
                        })
                    }
                }
                "safe" => {
                    if !st.safe_tag_supported {
                        json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": {"code": -32000, "message": "unknown block tag safe"}
                        })
                    } else if let Some(f) = st.finalized {
                        json!({
                            "jsonrpc": "2.0", "id": id,
                            "result": {
                                "number": format!("0x{:x}", f),
                                "hash": format!("0xS{:0>63x}", f),
                                "parentHash": format!("0xS{:0>63x}", f.saturating_sub(1)),
                                "timestamp": format!("0x{:x}", 1000 + f * 12),
                            }
                        })
                    } else {
                        json!({
                            "jsonrpc": "2.0", "id": id,
                            "error": {"code": -32000, "message": "no safe block set"}
                        })
                    }
                }
                s if s.starts_with("0x") => {
                    // Parse hex block number.
                    let n = u64::from_str_radix(&s[2..], 16).unwrap_or(u64::MAX);
                    match st.blocks_by_number.get(&n) {
                        Some(b) => {
                            json!({"jsonrpc": "2.0", "id": id, "result": b.header_json()})
                        }
                        None => {
                            json!({"jsonrpc": "2.0", "id": id, "result": Value::Null})
                        }
                    }
                }
                _ => json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32602, "message": "unsupported block tag in mock"}
                }),
            }
        }
        "eth_getBlockByHash" => {
            let arr = params.as_array();
            let hash = arr
                .and_then(|a| a.get(0))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let st = state.lock().unwrap();
            match st.blocks_by_hash.get(hash) {
                Some(b) => json!({"jsonrpc": "2.0", "id": id, "result": b.header_json()}),
                None => json!({"jsonrpc": "2.0", "id": id, "result": Value::Null}),
            }
        }
        "eth_getLogs" => {
            let mut st = state.lock().unwrap();
            if let Some((code, msg)) = st.simulate_next_rpc_error.take() {
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": code, "message": msg}
                });
            }
            let arr = params.as_array();
            let filter = arr.and_then(|a| a.get(0)).cloned().unwrap_or(json!({}));
            let from = filter
                .get("fromBlock")
                .and_then(|v| v.as_str())
                .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0);
            let to = filter
                .get("toBlock")
                .and_then(|v| v.as_str())
                .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(from);
            let addr_filter: Option<HashSet<String>> =
                filter.get("address").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_ascii_lowercase()))
                        .collect()
                });
            let mut out: Vec<Value> = Vec::new();
            for n in from..=to {
                if let Some(b) = st.blocks_by_number.get(&n) {
                    for log in &b.logs {
                        if let Some(f) = &addr_filter {
                            if !f.contains(&log.address.to_ascii_lowercase()) {
                                continue;
                            }
                        }
                        out.push(log.to_json());
                    }
                }
            }
            json!({"jsonrpc": "2.0", "id": id, "result": out})
        }
        other => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": format!("mock: unknown method {other}")}
        }),
    }
}

fn is_prohibited_method(method: &str) -> bool {
    matches!(
        method,
        "eth_sendTransaction"
            | "eth_sendRawTransaction"
            | "eth_sign"
            | "eth_signTransaction"
            | "eth_accounts"
            | "personal_sign"
            | "personal_ecRecover"
            | "personal_unlockAccount"
            | "personal_sendTransaction"
    ) || method.starts_with("wallet_")
        || method.starts_with("personal_")
}

// ---------------- Convenience builders ----------------

pub fn make_block(number: u64, hash_seed: u8, parent: &str, timestamp: u64) -> MockBlock {
    MockBlock {
        number,
        hash: format!("0x{}{:0>62x}", format!("{:02x}", hash_seed), number),
        parent_hash: parent.to_string(),
        timestamp,
        logs: Vec::new(),
    }
}

pub fn make_log(
    block: &MockBlock,
    address: &str,
    log_index: u32,
    topics: Vec<String>,
    data: &str,
) -> MockLog {
    MockLog {
        address: address.to_string(),
        block_hash: block.hash.clone(),
        block_number: block.number,
        tx_hash: format!(
            "0xaa{:0>62x}",
            (block.number << 8) | (log_index as u64 & 0xff)
        ),
        log_index,
        topics,
        data: data.to_string(),
    }
}

pub fn topic_bytes32(seed: u8) -> String {
    format!("0x{}{}", format!("{:02x}", seed).repeat(1), "0".repeat(62))
}

pub fn addr20(seed: u8) -> String {
    format!("0x{}{}", format!("{:02x}", seed).repeat(1), "0".repeat(38))
}
