//! Read-only EVM JSON-RPC `ChainSource` for the Hybrid V2 indexer.
//!
//! Frozen policy — this module MUST NOT:
//! - accept or store a signer, private key, or wallet handle,
//! - emit `eth_sendTransaction`, `eth_sendRawTransaction`, `eth_sign`,
//!   `personal_*`, `wallet_*`, or `eth_accounts`,
//! - print, log, or embed the raw RPC endpoint URL (which may embed a
//!   provider API key on the path). Only the redacted `<scheme>://<host>`
//!   form is exposed via `Debug` / errors / tracing.
//!
//! Allowed JSON-RPC methods:
//! - `eth_chainId`
//! - `eth_blockNumber`
//! - `eth_getBlockByNumber`
//! - `eth_getBlockByHash`
//! - `eth_getLogs`
//! - `eth_call` (block-bound, allowlisted target address + selector)
//!
//! Retry policy: bounded exponential backoff on transport failures,
//! HTTP 429, and HTTP 5xx. Deterministic JSON-RPC error responses
//! (with a numeric error code) are surfaced immediately — retrying
//! against a provider that already returned an error is unlikely to
//! help and burns rate-limit budget.

use crate::hybrid_v2::chain_source::{ChainSource, ChainSourceError, RawBlock};
use crate::hybrid_v2::config::redact_rpc_url;
use crate::hybrid_v2::decoder::CanonicalRawLog;
use crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

/// Static configuration for the JSON-RPC chain source. Held by the
/// source and never mutated once constructed.
#[derive(Clone)]
pub struct RpcSourceConfig {
    /// Full JSON-RPC endpoint URL. Never printed verbatim.
    pub endpoint: String,
    /// Expected chain id — validated at startup against the provider.
    pub chain_id: u64,
    /// Per-request timeout applied via reqwest's Client builder.
    pub timeout: Duration,
    /// Max retries on transient failures (transport, 429, 5xx, timeout).
    pub max_retries: u32,
    /// Base backoff (multiplied by `2^attempt`, capped at 30s).
    pub retry_backoff: Duration,
    /// Cap on log entries returned per `eth_getLogs` call.
    pub max_logs_per_range: u32,
    /// Confirmation depth used as the fallback finality tag when the
    /// provider rejects `finalized` and `safe`.
    pub confirmation_depth: u64,
}

impl std::fmt::Debug for RpcSourceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcSourceConfig")
            .field("endpoint", &redact_rpc_url(&self.endpoint))
            .field("chain_id", &self.chain_id)
            .field("timeout_ms", &self.timeout.as_millis())
            .field("max_retries", &self.max_retries)
            .field("retry_backoff_ms", &self.retry_backoff.as_millis())
            .field("max_logs_per_range", &self.max_logs_per_range)
            .field("confirmation_depth", &self.confirmation_depth)
            .finish()
    }
}

/// Which finality tag the provider accepts. Probed once and cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalityTag {
    Finalized,
    Safe,
    ConfirmationDepth,
}

/// The live, read-only JSON-RPC chain source.
pub struct RpcHybridV2ChainSource {
    client: reqwest::Client,
    endpoint: String,
    chain_id_expected: u64,
    /// Emitter addresses filtered on `eth_getLogs`, lowercase, 0x-prefixed.
    emitters: Vec<String>,
    max_retries: u32,
    retry_backoff: Duration,
    max_logs_per_range: u32,
    confirmation_depth: u64,
    finality_tag: OnceCell<FinalityTag>,
}

impl std::fmt::Debug for RpcHybridV2ChainSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcHybridV2ChainSource")
            .field("endpoint", &redact_rpc_url(&self.endpoint))
            .field("chain_id_expected", &self.chain_id_expected)
            .field("emitters_count", &self.emitters.len())
            .field("max_retries", &self.max_retries)
            .field("max_logs_per_range", &self.max_logs_per_range)
            .finish()
    }
}

impl RpcHybridV2ChainSource {
    /// Construct a live RPC chain source. Fails when the URL is
    /// malformed or the reqwest client cannot be built.
    pub fn new(config: RpcSourceConfig, emitters: Vec<String>) -> Result<Self, ChainSourceError> {
        if !(config.endpoint.starts_with("http://") || config.endpoint.starts_with("https://")) {
            return Err(ChainSourceError::Unsupported(
                "endpoint scheme not supported (only http/https)".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| ChainSourceError::Transport(format!("client build: {e}")))?;
        let emitters = emitters
            .into_iter()
            .map(|e| e.to_ascii_lowercase())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        Ok(Self {
            client,
            endpoint: config.endpoint,
            chain_id_expected: config.chain_id,
            emitters,
            max_retries: config.max_retries,
            retry_backoff: config.retry_backoff,
            max_logs_per_range: config.max_logs_per_range,
            confirmation_depth: config.confirmation_depth,
            finality_tag: OnceCell::new(),
        })
    }

    /// Explicit startup validation: call `eth_chainId`, compare against
    /// the expected chain id, and refuse Base mainnet unconditionally.
    pub async fn validate_chain_identity(&self) -> Result<u64, ChainSourceError> {
        let observed = self.rpc_chain_id().await?;
        if observed == BASE_MAINNET_CHAIN_ID {
            return Err(ChainSourceError::Unsupported(format!(
                "Base mainnet (chain_id={observed}) is not authorised for Hybrid V2"
            )));
        }
        if observed != self.chain_id_expected {
            return Err(ChainSourceError::Unsupported(format!(
                "provider chain_id={observed} does not match expected={}",
                self.chain_id_expected
            )));
        }
        Ok(observed)
    }

    /// Redacted host string, safe to include in log statements.
    pub fn redacted_endpoint(&self) -> String {
        redact_rpc_url(&self.endpoint)
    }

    // ---------------- INTERNAL JSON-RPC PLUMBING ----------------

    async fn call(&self, method: &str, params: Value) -> Result<Value, ChainSourceError> {
        // Defence-in-depth: refuse ever to send a state-changing method
        // from this source, regardless of caller intent.
        if is_prohibited_method(method) {
            return Err(ChainSourceError::Unsupported(format!(
                "method {method} is not allowed in read-only Hybrid V2 chain source"
            )));
        }
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let mut attempt: u32 = 0;
        loop {
            let outcome = self.attempt_once(&body).await;
            match outcome {
                Ok(v) => return Ok(v),
                Err(err) if !is_retryable(&err) => return Err(err),
                Err(err) => {
                    if attempt >= self.max_retries {
                        return Err(err);
                    }
                    let delay = compute_backoff(self.retry_backoff, attempt);
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn attempt_once(&self, body: &Value) -> Result<Value, ChainSourceError> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(body)
            .send()
            .await
            .map_err(map_reqwest_send_err)?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ChainSourceError::RateLimited);
        }
        if status.is_server_error() {
            return Err(ChainSourceError::ServerError {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            return Err(ChainSourceError::Transport(format!(
                "unexpected http status {}",
                status.as_u16()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|e| ChainSourceError::Malformed(format!("json decode: {e}")))?;
        // JSON-RPC error object → deterministic error, do not retry.
        if let Some(err) = payload.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32000);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("<no message>")
                .to_string();
            return Err(ChainSourceError::RpcError { code, message });
        }
        payload
            .get("result")
            .cloned()
            .ok_or_else(|| ChainSourceError::Malformed("missing `result` field".to_string()))
    }

    async fn rpc_chain_id(&self) -> Result<u64, ChainSourceError> {
        let v = self.call("eth_chainId", json!([])).await?;
        parse_u64_hex(&v, "eth_chainId")
    }

    async fn rpc_block_number(&self) -> Result<u64, ChainSourceError> {
        let v = self.call("eth_blockNumber", json!([])).await?;
        parse_u64_hex(&v, "eth_blockNumber")
    }

    async fn rpc_finalized_block_number(&self) -> Result<u64, ChainSourceError> {
        // Probe once and cache the tag choice.
        if let Some(tag) = self.finality_tag.get().copied() {
            return self.finalized_via(tag).await;
        }
        // Try finalized first.
        if let Ok(v) = self.finalized_via(FinalityTag::Finalized).await {
            let _ = self.finality_tag.set(FinalityTag::Finalized);
            return Ok(v);
        }
        // Then safe.
        if let Ok(v) = self.finalized_via(FinalityTag::Safe).await {
            let _ = self.finality_tag.set(FinalityTag::Safe);
            return Ok(v);
        }
        // Fall back to confirmation depth off the head.
        let _ = self.finality_tag.set(FinalityTag::ConfirmationDepth);
        self.finalized_via(FinalityTag::ConfirmationDepth).await
    }

    async fn finalized_via(&self, tag: FinalityTag) -> Result<u64, ChainSourceError> {
        match tag {
            FinalityTag::Finalized => {
                let v = self
                    .call("eth_getBlockByNumber", json!(["finalized", false]))
                    .await?;
                extract_block_number(&v)
            }
            FinalityTag::Safe => {
                let v = self
                    .call("eth_getBlockByNumber", json!(["safe", false]))
                    .await?;
                extract_block_number(&v)
            }
            FinalityTag::ConfirmationDepth => {
                let head = self.rpc_block_number().await?;
                Ok(head.saturating_sub(self.confirmation_depth))
            }
        }
    }

    /// Read-only `eth_call` against a Hybrid V2 module contract.
    ///
    /// Strict boundary enforced INSIDE this method:
    /// - `to` must be a well-formed 20-byte 0x-prefixed hex address.
    /// - `data` must be at least 4 bytes (a function selector).
    /// - The 4-byte selector must appear in `allowed_selectors` for the
    ///   normalised target address; if the map is empty for a target the
    ///   call is REJECTED (the caller wires the allowlist).
    /// - `block` may be `Number(n)` (encoded as `"0x{n:x}"`) or
    ///   `Latest`. `Hash(_)` is unsupported by all providers we target
    ///   so it is REJECTED here (returns `Unsupported`).
    ///
    /// Retry policy identical to every other allowed method.
    ///
    /// Returns the raw hex-decoded response bytes.
    pub async fn eth_call(
        &self,
        to: &str,
        data: &[u8],
        block: BlockRef,
        allowed_selectors: &std::collections::HashMap<[u8; 20], std::collections::HashSet<[u8; 4]>>,
    ) -> Result<Vec<u8>, ChainSourceError> {
        if !is_address_hex(to) {
            return Err(ChainSourceError::Malformed(format!(
                "eth_call `to` is not a 20-byte hex address"
            )));
        }
        if data.len() < 4 {
            return Err(ChainSourceError::Malformed(
                "eth_call `data` shorter than 4-byte selector".to_string(),
            ));
        }
        let mut selector = [0u8; 4];
        selector.copy_from_slice(&data[0..4]);
        let normalised_to = normalise_address(to);
        let to_bytes = address_hex_to_bytes(&normalised_to)
            .map_err(|e| ChainSourceError::Malformed(format!("eth_call `to` decode: {e}")))?;
        let Some(allowed) = allowed_selectors.get(&to_bytes) else {
            return Err(ChainSourceError::Unsupported(format!(
                "eth_call target {normalised_to} is not in the module allowlist"
            )));
        };
        if !allowed.contains(&selector) {
            return Err(ChainSourceError::Unsupported(format!(
                "eth_call selector 0x{:02x}{:02x}{:02x}{:02x} is not allowed for target {}",
                selector[0], selector[1], selector[2], selector[3], normalised_to,
            )));
        }
        let block_param = match block {
            BlockRef::Number(n) => Value::String(format!("0x{:x}", n)),
            BlockRef::Latest => Value::String("latest".to_string()),
        };
        let params = json!([
            {
                "to": normalised_to,
                "data": format!("0x{}", hex_encode(data)),
            },
            block_param,
        ]);
        let v = self.call("eth_call", params).await?;
        let s = v.as_str().ok_or_else(|| {
            ChainSourceError::Malformed("eth_call response not a string".to_string())
        })?;
        parse_hex_bytes(s)
            .map_err(|e| ChainSourceError::Malformed(format!("eth_call hex decode: {e}")))
    }

    async fn fetch_block_header(
        &self,
        selector: BlockSelector<'_>,
    ) -> Result<Option<BlockHeader>, ChainSourceError> {
        let (method, params) = match selector {
            BlockSelector::Number(n) => {
                ("eth_getBlockByNumber", json!([format!("0x{:x}", n), false]))
            }
            BlockSelector::Hash(h) => ("eth_getBlockByHash", json!([h, false])),
        };
        let v = self.call(method, params).await?;
        if v.is_null() {
            return Ok(None);
        }
        parse_block_header(&v).map(Some)
    }

    async fn fetch_logs_for_block(
        &self,
        block_number: u64,
        expected_hash: &str,
    ) -> Result<Vec<JsonRpcLog>, ChainSourceError> {
        let mut params_obj = serde_json::Map::new();
        let hex_block = format!("0x{:x}", block_number);
        params_obj.insert("fromBlock".to_string(), Value::String(hex_block.clone()));
        params_obj.insert("toBlock".to_string(), Value::String(hex_block));
        if !self.emitters.is_empty() {
            params_obj.insert(
                "address".to_string(),
                Value::Array(
                    self.emitters
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
        }
        let v = self
            .call("eth_getLogs", json!([Value::Object(params_obj)]))
            .await?;
        let arr = v
            .as_array()
            .ok_or_else(|| ChainSourceError::Malformed("eth_getLogs did not return array".into()))?
            .clone();
        if (arr.len() as u64) > self.max_logs_per_range as u64 {
            return Err(ChainSourceError::Malformed(format!(
                "eth_getLogs returned {} entries, exceeds bound {}",
                arr.len(),
                self.max_logs_per_range
            )));
        }
        // Parse + filter by block hash, dedupe by (blockHash, txHash, logIndex),
        // sort by (blockNumber, logIndex).
        let mut parsed: Vec<JsonRpcLog> = Vec::with_capacity(arr.len());
        for entry in arr.into_iter() {
            let log = parse_log(&entry)?;
            if !hash_eq(&log.block_hash, expected_hash) {
                return Err(ChainSourceError::Malformed(format!(
                    "eth_getLogs entry blockHash {} does not match expected {} for block {}",
                    log.block_hash, expected_hash, block_number
                )));
            }
            parsed.push(log);
        }
        parsed.sort_by(|a, b| {
            a.block_number
                .cmp(&b.block_number)
                .then(a.log_index.cmp(&b.log_index))
        });
        parsed.dedup_by(|a, b| {
            hash_eq(&a.block_hash, &b.block_hash)
                && hash_eq(&a.tx_hash, &b.tx_hash)
                && a.log_index == b.log_index
        });
        Ok(parsed)
    }
}

enum BlockSelector<'a> {
    Number(u64),
    Hash(&'a str),
}

/// Which block a read-only view is bound to. `Number(n)` produces a
/// deterministic snapshot at height `n`; `Latest` is only appropriate
/// for probe calls where the caller does not need block-coherent state.
#[derive(Debug, Clone, Copy)]
pub enum BlockRef {
    Number(u64),
    Latest,
}

#[async_trait]
impl ChainSource for RpcHybridV2ChainSource {
    async fn chain_id(&self) -> Result<u64, ChainSourceError> {
        self.rpc_chain_id().await
    }

    async fn head_block_number(&self) -> Result<u64, ChainSourceError> {
        self.rpc_block_number().await
    }

    async fn finalized_block_number(&self) -> Result<u64, ChainSourceError> {
        self.rpc_finalized_block_number().await
    }

    async fn block_at(&self, number: u64) -> Result<Option<RawBlock>, ChainSourceError> {
        let Some(header) = self
            .fetch_block_header(BlockSelector::Number(number))
            .await?
        else {
            return Ok(None);
        };
        let logs = self
            .fetch_logs_for_block(header.number, &header.hash)
            .await?;
        Ok(Some(assemble_raw_block(header, logs)))
    }

    async fn block_by_hash(&self, hash: &str) -> Result<Option<RawBlock>, ChainSourceError> {
        let Some(header) = self.fetch_block_header(BlockSelector::Hash(hash)).await? else {
            return Ok(None);
        };
        let logs = self
            .fetch_logs_for_block(header.number, &header.hash)
            .await?;
        Ok(Some(assemble_raw_block(header, logs)))
    }
}

fn assemble_raw_block(header: BlockHeader, logs: Vec<JsonRpcLog>) -> RawBlock {
    let canonical_logs = logs
        .into_iter()
        .map(|l| CanonicalRawLog {
            emitter: l.address.to_ascii_lowercase(),
            topics: l.topics,
            data: l.data,
        })
        .collect();
    RawBlock {
        number: header.number,
        hash: header.hash,
        parent_hash: header.parent_hash,
        timestamp: header.timestamp,
        logs: canonical_logs,
    }
}

// ---------------- SUPPORT ----------------

struct BlockHeader {
    number: u64,
    hash: String,
    parent_hash: String,
    timestamp: u64,
}

struct JsonRpcLog {
    address: String,
    block_hash: String,
    block_number: u64,
    tx_hash: String,
    log_index: u32,
    topics: Vec<[u8; 32]>,
    data: Vec<u8>,
}

fn parse_block_header(v: &Value) -> Result<BlockHeader, ChainSourceError> {
    let number = v
        .get("number")
        .and_then(|n| n.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("block missing `number`".into()))?;
    let number = parse_hex_u64(number)
        .map_err(|e| ChainSourceError::Malformed(format!("block.number: {e}")))?;
    let hash = v
        .get("hash")
        .and_then(|n| n.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("block missing `hash`".into()))?
        .to_string();
    if !is_bytes32_hex(&hash) {
        return Err(ChainSourceError::Malformed(format!(
            "block.hash is not a 32-byte hex string"
        )));
    }
    let parent_hash = v
        .get("parentHash")
        .and_then(|n| n.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("block missing `parentHash`".into()))?
        .to_string();
    if !is_bytes32_hex(&parent_hash) {
        return Err(ChainSourceError::Malformed(
            "block.parentHash is not a 32-byte hex string".to_string(),
        ));
    }
    let timestamp = v
        .get("timestamp")
        .and_then(|n| n.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("block missing `timestamp`".into()))?;
    let timestamp = parse_hex_u64(timestamp)
        .map_err(|e| ChainSourceError::Malformed(format!("block.timestamp: {e}")))?;
    Ok(BlockHeader {
        number,
        hash,
        parent_hash,
        timestamp,
    })
}

fn extract_block_number(v: &Value) -> Result<u64, ChainSourceError> {
    if v.is_null() {
        return Err(ChainSourceError::Unsupported(
            "finality tag returned null block".to_string(),
        ));
    }
    let n = v
        .get("number")
        .and_then(|n| n.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("finality block missing `number`".into()))?;
    parse_hex_u64(n).map_err(|e| ChainSourceError::Malformed(format!("finality: {e}")))
}

fn parse_log(entry: &Value) -> Result<JsonRpcLog, ChainSourceError> {
    let address = entry
        .get("address")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("log missing `address`".into()))?
        .to_string();
    if !is_address_hex(&address) {
        return Err(ChainSourceError::Malformed(
            "log.address not a 20-byte hex string".to_string(),
        ));
    }
    let block_hash = entry
        .get("blockHash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("log missing `blockHash`".into()))?
        .to_string();
    if !is_bytes32_hex(&block_hash) {
        return Err(ChainSourceError::Malformed(
            "log.blockHash not 32-byte hex".to_string(),
        ));
    }
    let block_number = entry
        .get("blockNumber")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("log missing `blockNumber`".into()))?;
    let block_number = parse_hex_u64(block_number)
        .map_err(|e| ChainSourceError::Malformed(format!("log.blockNumber: {e}")))?;
    let tx_hash = entry
        .get("transactionHash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("log missing `transactionHash`".into()))?
        .to_string();
    if !is_bytes32_hex(&tx_hash) {
        return Err(ChainSourceError::Malformed(
            "log.transactionHash not 32-byte hex".to_string(),
        ));
    }
    let log_index = entry
        .get("logIndex")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("log missing `logIndex`".into()))?;
    let log_index = parse_hex_u64(log_index)
        .map_err(|e| ChainSourceError::Malformed(format!("log.logIndex: {e}")))?
        as u32;
    let topics_val = entry
        .get("topics")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ChainSourceError::Malformed("log missing `topics`".into()))?;
    let mut topics: Vec<[u8; 32]> = Vec::with_capacity(topics_val.len());
    for t in topics_val {
        let s = t
            .as_str()
            .ok_or_else(|| ChainSourceError::Malformed("topic not string".into()))?;
        let bytes =
            parse_bytes32_hex(s).map_err(|e| ChainSourceError::Malformed(format!("topic: {e}")))?;
        topics.push(bytes);
    }
    let data_str = entry
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChainSourceError::Malformed("log missing `data`".into()))?;
    let data =
        parse_hex_bytes(data_str).map_err(|e| ChainSourceError::Malformed(format!("data: {e}")))?;
    Ok(JsonRpcLog {
        address,
        block_hash,
        block_number,
        tx_hash,
        log_index,
        topics,
        data,
    })
}

fn parse_u64_hex(v: &Value, ctx: &str) -> Result<u64, ChainSourceError> {
    let s = v
        .as_str()
        .ok_or_else(|| ChainSourceError::Malformed(format!("{ctx} not a string")))?;
    parse_hex_u64(s).map_err(|e| ChainSourceError::Malformed(format!("{ctx}: {e}")))
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let stripped = s
        .strip_prefix("0x")
        .ok_or_else(|| format!("expected 0x-prefixed hex, got {} chars", s.len().min(64)))?;
    if stripped.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(stripped, 16).map_err(|e| format!("hex parse: {e}"))
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    let stripped = s
        .strip_prefix("0x")
        .ok_or_else(|| "expected 0x-prefixed hex".to_string())?;
    if stripped.is_empty() {
        return Ok(Vec::new());
    }
    if stripped.len() % 2 != 0 {
        return Err("odd hex length".to_string());
    }
    let mut out = Vec::with_capacity(stripped.len() / 2);
    for i in (0..stripped.len()).step_by(2) {
        let byte = u8::from_str_radix(&stripped[i..i + 2], 16)
            .map_err(|e| format!("hex byte parse: {e}"))?;
        out.push(byte);
    }
    Ok(out)
}

fn parse_bytes32_hex(s: &str) -> Result<[u8; 32], String> {
    let bytes = parse_hex_bytes(s)?;
    if bytes.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn is_bytes32_hex(s: &str) -> bool {
    if let Some(stripped) = s.strip_prefix("0x") {
        stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit())
    } else {
        false
    }
}

fn is_address_hex(s: &str) -> bool {
    if let Some(stripped) = s.strip_prefix("0x") {
        stripped.len() == 40 && stripped.chars().all(|c| c.is_ascii_hexdigit())
    } else {
        false
    }
}

fn hash_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn normalise_address(addr: &str) -> String {
    let lower = addr.trim().to_ascii_lowercase();
    if lower.starts_with("0x") {
        lower
    } else {
        format!("0x{lower}")
    }
}

fn address_hex_to_bytes(addr: &str) -> Result<[u8; 20], String> {
    let bytes = parse_hex_bytes(addr)?;
    if bytes.len() != 20 {
        return Err(format!("expected 20 bytes, got {}", bytes.len()));
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn map_reqwest_send_err(err: reqwest::Error) -> ChainSourceError {
    if err.is_timeout() {
        ChainSourceError::Timeout
    } else if err.is_connect() || err.is_request() {
        ChainSourceError::Transport(format!("{}", err.without_url()))
    } else {
        ChainSourceError::Transport(format!("{}", err.without_url()))
    }
}

fn is_retryable(err: &ChainSourceError) -> bool {
    matches!(
        err,
        ChainSourceError::Transport(_)
            | ChainSourceError::Timeout
            | ChainSourceError::RateLimited
            | ChainSourceError::ServerError { .. }
    )
}

fn compute_backoff(base: Duration, attempt: u32) -> Duration {
    let cap = Duration::from_secs(30);
    // 2^attempt without panicking on overflow.
    let factor: u64 = 1u64 << attempt.min(20);
    let ms = base.as_millis().saturating_mul(factor as u128);
    let ms_u64 = ms.min(cap.as_millis()) as u64;
    Duration::from_millis(ms_u64)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_u64_roundtrips() {
        assert_eq!(parse_hex_u64("0x0").unwrap(), 0);
        assert_eq!(parse_hex_u64("0x1a").unwrap(), 26);
        assert_eq!(parse_hex_u64("0x14a34").unwrap(), 84_532);
        assert!(parse_hex_u64("14a34").is_err());
    }

    #[test]
    fn parse_hex_bytes_odd_length_rejected() {
        assert!(parse_hex_bytes("0xabc").is_err());
        assert_eq!(parse_hex_bytes("0x").unwrap(), Vec::<u8>::new());
        assert_eq!(parse_hex_bytes("0xdead").unwrap(), vec![0xde, 0xad]);
    }

    #[test]
    fn parse_bytes32_hex_enforces_exact_length() {
        assert!(parse_bytes32_hex("0xdead").is_err());
        let hex = format!("0x{}", "ff".repeat(32));
        assert_eq!(parse_bytes32_hex(&hex).unwrap(), [0xff; 32]);
    }

    #[test]
    fn is_bytes32_hex_rejects_non_hex() {
        assert!(is_bytes32_hex(&format!("0x{}", "0".repeat(64))));
        assert!(!is_bytes32_hex("0x00"));
        assert!(!is_bytes32_hex(&format!("0x{}z", "0".repeat(63))));
    }

    #[test]
    fn compute_backoff_caps_at_30s() {
        let base = Duration::from_millis(1_000);
        // 2^5 = 32, so 1000*32 = 32s > 30s cap.
        assert_eq!(compute_backoff(base, 5), Duration::from_secs(30));
        assert_eq!(compute_backoff(base, 0), Duration::from_millis(1_000));
        assert_eq!(compute_backoff(base, 2), Duration::from_millis(4_000));
    }

    #[test]
    fn is_retryable_matches_transient_only() {
        assert!(is_retryable(&ChainSourceError::Timeout));
        assert!(is_retryable(&ChainSourceError::RateLimited));
        assert!(is_retryable(&ChainSourceError::ServerError { status: 502 }));
        assert!(is_retryable(&ChainSourceError::Transport("x".into())));
        assert!(!is_retryable(&ChainSourceError::RpcError {
            code: -32602,
            message: "bad".into()
        }));
        assert!(!is_retryable(&ChainSourceError::Malformed("x".into())));
        assert!(!is_retryable(&ChainSourceError::Unsupported("x".into())));
    }

    #[test]
    fn prohibited_method_set_covers_write_surface() {
        for m in [
            "eth_sendTransaction",
            "eth_sendRawTransaction",
            "eth_sign",
            "eth_signTransaction",
            "eth_accounts",
            "personal_sign",
            "personal_sendTransaction",
            "wallet_switchEthereumChain",
        ] {
            assert!(is_prohibited_method(m), "must be prohibited: {m}");
        }
        for m in [
            "eth_chainId",
            "eth_blockNumber",
            "eth_getBlockByNumber",
            "eth_getBlockByHash",
            "eth_getLogs",
            "eth_call",
        ] {
            assert!(!is_prohibited_method(m), "must be allowed: {m}");
        }
    }

    #[tokio::test]
    async fn eth_call_rejects_bad_target() {
        // Use a dummy endpoint — the source will construct fine but never
        // send a request because validation trips first.
        let cfg = RpcSourceConfig {
            endpoint: "http://127.0.0.1:1/".to_string(),
            chain_id: 84532,
            timeout: Duration::from_millis(50),
            max_retries: 0,
            retry_backoff: Duration::from_millis(1),
            max_logs_per_range: 1000,
            confirmation_depth: 12,
        };
        let src = RpcHybridV2ChainSource::new(cfg, vec![]).expect("construct");
        let allowlist = std::collections::HashMap::new();
        // Non-hex `to` is rejected before any allowlist lookup.
        let err = src
            .eth_call("garbage", &[0x1u8, 2, 3, 4], BlockRef::Latest, &allowlist)
            .await
            .expect_err("must reject non-address");
        assert!(matches!(err, ChainSourceError::Malformed(_)));
    }

    #[tokio::test]
    async fn eth_call_rejects_short_data() {
        let cfg = RpcSourceConfig {
            endpoint: "http://127.0.0.1:1/".to_string(),
            chain_id: 84532,
            timeout: Duration::from_millis(50),
            max_retries: 0,
            retry_backoff: Duration::from_millis(1),
            max_logs_per_range: 1000,
            confirmation_depth: 12,
        };
        let src = RpcHybridV2ChainSource::new(cfg, vec![]).expect("construct");
        let allowlist = std::collections::HashMap::new();
        let addr = format!("0x{}", "aa".repeat(20));
        let err = src
            .eth_call(&addr, &[0x1u8, 2, 3], BlockRef::Latest, &allowlist)
            .await
            .expect_err("must reject short data");
        assert!(matches!(err, ChainSourceError::Malformed(_)));
    }

    #[tokio::test]
    async fn eth_call_rejects_unlisted_target() {
        let cfg = RpcSourceConfig {
            endpoint: "http://127.0.0.1:1/".to_string(),
            chain_id: 84532,
            timeout: Duration::from_millis(50),
            max_retries: 0,
            retry_backoff: Duration::from_millis(1),
            max_logs_per_range: 1000,
            confirmation_depth: 12,
        };
        let src = RpcHybridV2ChainSource::new(cfg, vec![]).expect("construct");
        let allowlist = std::collections::HashMap::new();
        let addr = format!("0x{}", "aa".repeat(20));
        let err = src
            .eth_call(&addr, &[0x1u8, 2, 3, 4], BlockRef::Latest, &allowlist)
            .await
            .expect_err("must reject unlisted target");
        assert!(matches!(err, ChainSourceError::Unsupported(_)));
    }

    #[tokio::test]
    async fn eth_call_rejects_unlisted_selector() {
        let cfg = RpcSourceConfig {
            endpoint: "http://127.0.0.1:1/".to_string(),
            chain_id: 84532,
            timeout: Duration::from_millis(50),
            max_retries: 0,
            retry_backoff: Duration::from_millis(1),
            max_logs_per_range: 1000,
            confirmation_depth: 12,
        };
        let src = RpcHybridV2ChainSource::new(cfg, vec![]).expect("construct");
        let addr = format!("0x{}", "aa".repeat(20));
        let target_bytes = address_hex_to_bytes(&addr).unwrap();
        let mut allowed_sel = std::collections::HashSet::new();
        allowed_sel.insert([0xde, 0xad, 0xbe, 0xef]);
        let mut allowlist = std::collections::HashMap::new();
        allowlist.insert(target_bytes, allowed_sel);
        let err = src
            .eth_call(&addr, &[0x1u8, 2, 3, 4], BlockRef::Latest, &allowlist)
            .await
            .expect_err("must reject unlisted selector");
        assert!(matches!(err, ChainSourceError::Unsupported(_)));
    }

    #[test]
    fn new_rejects_non_http_scheme() {
        let cfg = RpcSourceConfig {
            endpoint: "ws://foo/bar".to_string(),
            chain_id: 84532,
            timeout: Duration::from_secs(1),
            max_retries: 0,
            retry_backoff: Duration::from_millis(50),
            max_logs_per_range: 1000,
            confirmation_depth: 12,
        };
        assert!(RpcHybridV2ChainSource::new(cfg, vec![]).is_err());
    }
}
