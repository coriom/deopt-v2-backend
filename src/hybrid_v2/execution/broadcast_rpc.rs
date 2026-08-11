//! Narrow JSON-RPC broadcast client for the Hybrid V2 submission
//! pipeline (Part G of
//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! ## Design
//!
//! - `ExecutionBroadcastRpcClient` is a NEW trait — deliberately not a
//!   superset of `ExecutionRpcClient`. The two clients share the same
//!   JSON-RPC transport shape but have distinct responsibilities and
//!   distinct method allowlists.
//! - The ONLY write method is `send_raw_transaction`. Every other
//!   method is read-only (chain id, head, tx / receipt / block lookup,
//!   nonce count).
//! - `HttpExecutionBroadcastRpcClient` statically allowlists JSON-RPC
//!   method names. `check_method` panics-loud + errors-safe on any
//!   method not in the allowlist. Because every call site passes a
//!   `&'static str`, drift becomes an immediate CI failure.
//! - Base mainnet (chain_id 8453) is refused by `chain_id()` as a
//!   defense-in-depth guard: even if the operator misconfigures the
//!   RPC URL, the first `eth_chainId` reply that says `0x2105` yields
//!   `BroadcastRpcError::BaseMainnetForbidden`.

use crate::hybrid_v2::config::redact_rpc_url;
use crate::hybrid_v2::execution::rpc::BlockTag;
use crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID;
use alloy_primitives::U256;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

// -----------------------------------------------------------------
//                             TYPES
// -----------------------------------------------------------------

/// Minimal shape of a transaction returned by `eth_getTransactionByHash`.
/// Contains only the fields the broadcast pipeline uses for recovery
/// after a `SUBMISSION_UNKNOWN` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionSummary {
    pub tx_hash: [u8; 32],
    pub from: [u8; 20],
    /// Contract creation transactions have `to = None`. Hybrid V2
    /// executions always target OptionMatchingEngineV2, so `None` in
    /// this pipeline is a data-integrity error at the caller.
    pub to: Option<[u8; 20]>,
    pub nonce: u64,
    /// `None` when the tx is still in the mempool.
    pub block_number: Option<u64>,
    /// `None` when the tx is still in the mempool.
    pub block_hash: Option<[u8; 32]>,
    pub value_wei: U256,
    pub input_bytes_len: usize,
    /// The provider-reported input hash (keccak256 over the calldata
    /// bytes). Optional — some providers omit input from the summary
    /// object; we recompute against local calldata whenever needed.
    pub input_hash: Option<[u8; 32]>,
    /// EIP-1559 max_fee_per_gas — Some for type 2 txs.
    pub max_fee_per_gas: Option<U256>,
    /// EIP-1559 max_priority_fee_per_gas — Some for type 2 txs.
    pub max_priority_fee_per_gas: Option<U256>,
    /// `0` legacy, `1` EIP-2930, `2` EIP-1559.
    pub tx_type: u8,
}

/// Receipt shape used by the confirmation watcher. Only the fields the
/// broadcast pipeline needs — no logs (Part K correlates logs via the
/// indexer path, not through this trait).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxReceipt {
    pub tx_hash: [u8; 32],
    pub block_number: u64,
    pub block_hash: [u8; 32],
    /// `0` = reverted, `1` = success. Providers that omit `status`
    /// pre-Byzantium are rejected by the parser (status is mandatory
    /// on every chain the broadcast pipeline targets).
    pub status: u8,
    pub gas_used: u64,
    pub effective_gas_price_wei: U256,
    pub cumulative_gas_used: u64,
    pub from: [u8; 20],
    pub to: Option<[u8; 20]>,
}

/// Minimal EVM block header used by the confirmation watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    pub number: u64,
    pub hash: [u8; 32],
    pub parent_hash: [u8; 32],
    pub timestamp: u64,
}

/// Structured outcome of `send_raw_transaction`. Distinguishes benign
/// idempotent replies (`AlreadyKnown`) from deterministic rejections
/// (`NonceTooLow`, `NonceTooHigh`, `ReplacementUnderpriced`) and from
/// generic provider errors carrying a JSON-RPC code + message.
///
/// The outbox maps each variant to a `provider_classification` string
/// and (where applicable) a `failure_class` for observability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Provider accepted the raw tx. `provider_tx_hash` MUST equal the
    /// locally-derived envelope_hash; otherwise the outbox raises
    /// `PROVIDER_HASH_MISMATCH`.
    Accepted { provider_tx_hash: [u8; 32] },
    /// Provider says the tx is already in its mempool (typical idempotent
    /// resubmission). Treated as success in the outbox.
    AlreadyKnown { provider_tx_hash: [u8; 32] },
    /// nonce lower than the account's on-chain nonce — the tx will
    /// never mine. Frozen posture: NO auto-replacement.
    NonceTooLow,
    /// nonce higher than the current on-chain nonce — the tx waits in
    /// the mempool but will not mine until predecessors do.
    NonceTooHigh,
    /// The provider already has a pending tx with the same nonce but a
    /// higher fee. Frozen posture: NO auto-fee-bump.
    ReplacementUnderpriced,
    /// Generic RPC-level rejection. The outbox classifies as
    /// `PROVIDER_REJECTED` and escalates to
    /// `MANUAL_INTERVENTION_REQUIRED`.
    ProviderRejection { code: i64, message: String },
}

/// Broadcast RPC error surface. Distinguishes transport / timeout /
/// unavailable (retryable / ambiguous) from deterministic transport
/// misconfiguration (Malformed, ChainIdMismatch).
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum BroadcastRpcError {
    #[error("provider timed out")]
    Timeout,
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("provider returned malformed payload: {0}")]
    Malformed(String),
    #[error("provider rate-limited (429)")]
    RateLimited,
    #[error("provider unavailable ({0})")]
    Unavailable(String),
    #[error("chain id mismatch: expected {expected}, got {actual}")]
    ChainIdMismatch { expected: u64, actual: u64 },
    #[error("Base mainnet (chain id 8453) is forbidden for Hybrid V2 broadcast")]
    BaseMainnetForbidden,
    #[error("method {0} is not permitted by the broadcast client")]
    MethodNotAllowed(&'static str),
    #[error("provider returned JSON-RPC error {code}: {message}")]
    RpcError { code: i64, message: String },
}

// -----------------------------------------------------------------
//                              TRAIT
// -----------------------------------------------------------------

/// Narrow broadcast RPC surface for Hybrid V2. `send_raw_transaction`
/// is the ONLY write method; every other method is read-only.
#[async_trait]
pub trait ExecutionBroadcastRpcClient: Send + Sync {
    async fn chain_id(&self) -> Result<u64, BroadcastRpcError>;
    async fn head_block_number(&self) -> Result<u64, BroadcastRpcError>;
    /// `finalized_block_number` — returns `None` on chains that do not
    /// expose a `finalized` block tag (some private / test networks).
    async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError>;
    async fn transaction_count(
        &self,
        address: [u8; 20],
        block_tag: BlockTag,
    ) -> Result<u64, BroadcastRpcError>;
    async fn transaction_by_hash(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<TransactionSummary>, BroadcastRpcError>;
    async fn receipt_by_hash(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<TxReceipt>, BroadcastRpcError>;
    async fn block_header_by_number(
        &self,
        number: u64,
    ) -> Result<Option<BlockHeader>, BroadcastRpcError>;
    async fn block_header_by_hash(
        &self,
        hash: [u8; 32],
    ) -> Result<Option<BlockHeader>, BroadcastRpcError>;

    /// ONLY write method — accepts a raw already-signed transaction.
    /// Implementations MUST validate the provider-returned tx hash is
    /// a well-formed 32-byte value; the outbox further verifies it
    /// matches the locally-derived envelope hash.
    async fn send_raw_transaction(
        &self,
        raw_tx_bytes: &[u8],
    ) -> Result<SendOutcome, BroadcastRpcError>;
}

// -----------------------------------------------------------------
//                       HTTP IMPLEMENTATION
// -----------------------------------------------------------------

/// Statically enumerated allowlist of JSON-RPC method names emitted by
/// [`HttpExecutionBroadcastRpcClient`]. The `check_method` helper
/// refuses anything outside this list.
pub const BROADCAST_ALLOWED_METHODS: &[&str] = &[
    "eth_chainId",
    "eth_blockNumber",
    "eth_getTransactionCount",
    "eth_getTransactionByHash",
    "eth_getTransactionReceipt",
    "eth_getBlockByNumber",
    "eth_getBlockByHash",
    "eth_sendRawTransaction",
];

/// Live HTTP JSON-RPC broadcast client.
pub struct HttpExecutionBroadcastRpcClient {
    client: reqwest::Client,
    endpoint: String,
    timeout: Duration,
    max_retries: u32,
    expected_chain_id: Option<u64>,
}

impl std::fmt::Debug for HttpExecutionBroadcastRpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpExecutionBroadcastRpcClient")
            .field("endpoint", &redact_rpc_url(&self.endpoint))
            .field("timeout_ms", &self.timeout.as_millis())
            .field("max_retries", &self.max_retries)
            .field("expected_chain_id", &self.expected_chain_id)
            .finish()
    }
}

impl HttpExecutionBroadcastRpcClient {
    pub fn new(
        endpoint: String,
        timeout: Duration,
        max_retries: u32,
        expected_chain_id: Option<u64>,
    ) -> Result<Self, BroadcastRpcError> {
        if let Some(cid) = expected_chain_id {
            if cid == BASE_MAINNET_CHAIN_ID {
                return Err(BroadcastRpcError::BaseMainnetForbidden);
            }
        }
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(BroadcastRpcError::Transport(
                "endpoint scheme not supported (only http/https)".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| BroadcastRpcError::Transport(format!("client build: {e}")))?;
        Ok(Self {
            client,
            endpoint,
            timeout,
            max_retries,
            expected_chain_id,
        })
    }

    /// Runtime allowlist guard — every JSON-RPC round trip funnels
    /// through this check. Non-allowlisted methods fail before touching
    /// the wire.
    pub fn check_method(method: &'static str) -> Result<(), BroadcastRpcError> {
        if !BROADCAST_ALLOWED_METHODS.iter().any(|m| *m == method) {
            return Err(BroadcastRpcError::MethodNotAllowed(method));
        }
        Ok(())
    }

    async fn call_json(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<Value, BroadcastRpcError> {
        Self::check_method(method)?;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let mut attempt: u32 = 0;
        loop {
            match self.attempt_once(&body).await {
                Ok(v) => return Ok(v),
                Err(err) if !is_retryable(&err) => return Err(err),
                Err(err) => {
                    if attempt >= self.max_retries {
                        return Err(err);
                    }
                    attempt += 1;
                }
            }
        }
    }

    async fn attempt_once(&self, body: &Value) -> Result<Value, BroadcastRpcError> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(body)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(BroadcastRpcError::RateLimited);
        }
        if status.is_server_error() {
            return Err(BroadcastRpcError::Unavailable(format!(
                "http {}",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(BroadcastRpcError::Transport(format!(
                "unexpected http status {}",
                status.as_u16()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|e| BroadcastRpcError::Malformed(format!("json decode: {e}")))?;
        if let Some(err) = payload.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32000);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("<no message>")
                .to_string();
            return Err(BroadcastRpcError::RpcError { code, message });
        }
        payload
            .get("result")
            .cloned()
            .ok_or_else(|| BroadcastRpcError::Malformed("missing `result` field".to_string()))
    }
}

#[async_trait]
impl ExecutionBroadcastRpcClient for HttpExecutionBroadcastRpcClient {
    async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
        let v = self.call_json("eth_chainId", json!([])).await?;
        let s = v
            .as_str()
            .ok_or_else(|| BroadcastRpcError::Malformed("chainId not a string".into()))?;
        let id =
            parse_hex_u64(s).map_err(|e| BroadcastRpcError::Malformed(format!("chainId: {e}")))?;
        if id == BASE_MAINNET_CHAIN_ID {
            return Err(BroadcastRpcError::BaseMainnetForbidden);
        }
        if let Some(expected) = self.expected_chain_id {
            if expected != id {
                return Err(BroadcastRpcError::ChainIdMismatch {
                    expected,
                    actual: id,
                });
            }
        }
        Ok(id)
    }

    async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
        let v = self.call_json("eth_blockNumber", json!([])).await?;
        let s = v
            .as_str()
            .ok_or_else(|| BroadcastRpcError::Malformed("blockNumber not a string".into()))?;
        parse_hex_u64(s).map_err(|e| BroadcastRpcError::Malformed(format!("blockNumber: {e}")))
    }

    async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
        let outcome = self
            .call_json("eth_getBlockByNumber", json!(["finalized", false]))
            .await;
        match outcome {
            Ok(v) if v.is_null() => Ok(None),
            Ok(v) => {
                let n = v.get("number").and_then(|x| x.as_str()).ok_or_else(|| {
                    BroadcastRpcError::Malformed("finalized block missing number".into())
                })?;
                parse_hex_u64(n)
                    .map(Some)
                    .map_err(|e| BroadcastRpcError::Malformed(format!("finalized number: {e}")))
            }
            // Some networks do not implement the `finalized` tag — treat
            // as "not available" rather than a hard error.
            Err(BroadcastRpcError::RpcError { .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    async fn transaction_count(
        &self,
        address: [u8; 20],
        block_tag: BlockTag,
    ) -> Result<u64, BroadcastRpcError> {
        let addr = to_hex_addr(&address);
        let tag = block_tag_to_json(block_tag)?;
        let v = self
            .call_json("eth_getTransactionCount", json!([addr, tag]))
            .await?;
        let s = v
            .as_str()
            .ok_or_else(|| BroadcastRpcError::Malformed("txCount not a string".into()))?;
        parse_hex_u64(s).map_err(|e| BroadcastRpcError::Malformed(format!("txCount: {e}")))
    }

    async fn transaction_by_hash(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
        let v = self
            .call_json(
                "eth_getTransactionByHash",
                json!([to_hex_bytes32(&tx_hash)]),
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        parse_transaction_summary(&v).map(Some)
    }

    async fn receipt_by_hash(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
        let v = self
            .call_json(
                "eth_getTransactionReceipt",
                json!([to_hex_bytes32(&tx_hash)]),
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        parse_tx_receipt(&v).map(Some)
    }

    async fn block_header_by_number(
        &self,
        number: u64,
    ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
        let v = self
            .call_json(
                "eth_getBlockByNumber",
                json!([format!("0x{:x}", number), false]),
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        parse_block_header(&v).map(Some)
    }

    async fn block_header_by_hash(
        &self,
        hash: [u8; 32],
    ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
        let v = self
            .call_json("eth_getBlockByHash", json!([to_hex_bytes32(&hash), false]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        parse_block_header(&v).map(Some)
    }

    async fn send_raw_transaction(
        &self,
        raw_tx_bytes: &[u8],
    ) -> Result<SendOutcome, BroadcastRpcError> {
        let raw_hex = format!("0x{}", hex_encode(raw_tx_bytes));
        let outcome = self
            .call_json("eth_sendRawTransaction", json!([raw_hex]))
            .await;
        match outcome {
            Ok(v) => {
                let s = v.as_str().ok_or_else(|| {
                    BroadcastRpcError::Malformed("send_raw response not a string".into())
                })?;
                let hash = parse_bytes32(s)?;
                Ok(SendOutcome::Accepted {
                    provider_tx_hash: hash,
                })
            }
            Err(BroadcastRpcError::RpcError { code, message }) => {
                Ok(classify_provider_rejection(code, &message))
            }
            Err(other) => Err(other),
        }
    }
}

// -----------------------------------------------------------------
//                            HELPERS
// -----------------------------------------------------------------

fn classify_provider_rejection(code: i64, message: &str) -> SendOutcome {
    // Providers vary; classify on well-known substrings + code.
    let lower = message.to_ascii_lowercase();
    if lower.contains("already known")
        || lower.contains("already exists")
        || lower.contains("transaction underpriced already")
    // paranoid catch
    {
        // If the provider returned the tx hash in `data`, we'd surface
        // it as Accepted. In practice most providers only emit a
        // string message here — the outbox re-derives the hash locally.
        return SendOutcome::AlreadyKnown {
            provider_tx_hash: [0u8; 32],
        };
    }
    if lower.contains("nonce too low") {
        return SendOutcome::NonceTooLow;
    }
    if lower.contains("nonce too high") {
        return SendOutcome::NonceTooHigh;
    }
    if lower.contains("replacement transaction underpriced")
        || lower.contains("replacement tx underpriced")
    {
        return SendOutcome::ReplacementUnderpriced;
    }
    SendOutcome::ProviderRejection {
        code,
        message: message.to_string(),
    }
}

fn block_tag_to_json(tag: BlockTag) -> Result<Value, BroadcastRpcError> {
    match tag {
        BlockTag::Number(n) => Ok(Value::String(format!("0x{:x}", n))),
        BlockTag::Latest => Ok(Value::String("latest".to_string())),
        BlockTag::Pending => Ok(Value::String("pending".to_string())),
        BlockTag::Hash(_) => Err(BroadcastRpcError::Malformed(
            "block-by-hash tag not supported for this method".into(),
        )),
    }
}

fn parse_transaction_summary(v: &Value) -> Result<TransactionSummary, BroadcastRpcError> {
    let tx_hash = parse_bytes32(v.get("hash").and_then(|x| x.as_str()).unwrap_or_default())?;
    let from = parse_addr(v.get("from").and_then(|x| x.as_str()).unwrap_or_default())?;
    let to = match v.get("to").and_then(|x| x.as_str()) {
        Some(s) if !s.is_empty() && s != "null" => Some(parse_addr(s)?),
        _ => None,
    };
    let nonce = parse_hex_u64(v.get("nonce").and_then(|x| x.as_str()).unwrap_or("0x0"))
        .map_err(|e| BroadcastRpcError::Malformed(format!("nonce: {e}")))?;
    let block_number = match v.get("blockNumber").and_then(|x| x.as_str()) {
        Some(s) if !s.is_empty() => Some(
            parse_hex_u64(s)
                .map_err(|e| BroadcastRpcError::Malformed(format!("blockNumber: {e}")))?,
        ),
        _ => None,
    };
    let block_hash = match v.get("blockHash").and_then(|x| x.as_str()) {
        Some(s) if !s.is_empty() && s != "null" => Some(parse_bytes32(s)?),
        _ => None,
    };
    let value_wei = parse_hex_u256(v.get("value").and_then(|x| x.as_str()).unwrap_or("0x0"))
        .map_err(|e| BroadcastRpcError::Malformed(format!("value: {e}")))?;
    let input_hex = v
        .get("input")
        .and_then(|x| x.as_str())
        .unwrap_or("0x")
        .to_string();
    let input_bytes = parse_hex_bytes(&input_hex)
        .map_err(|e| BroadcastRpcError::Malformed(format!("input: {e}")))?;
    let input_bytes_len = input_bytes.len();
    let input_hash = if input_bytes.is_empty() {
        None
    } else {
        Some(keccak256_of(&input_bytes))
    };
    let max_fee_per_gas = v
        .get("maxFeePerGas")
        .and_then(|x| x.as_str())
        .and_then(|s| parse_hex_u256(s).ok());
    let max_priority_fee_per_gas = v
        .get("maxPriorityFeePerGas")
        .and_then(|x| x.as_str())
        .and_then(|s| parse_hex_u256(s).ok());
    let tx_type = v
        .get("type")
        .and_then(|x| x.as_str())
        .and_then(|s| parse_hex_u64(s).ok())
        .map(|n| n as u8)
        .unwrap_or(0);
    Ok(TransactionSummary {
        tx_hash,
        from,
        to,
        nonce,
        block_number,
        block_hash,
        value_wei,
        input_bytes_len,
        input_hash,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        tx_type,
    })
}

fn parse_tx_receipt(v: &Value) -> Result<TxReceipt, BroadcastRpcError> {
    let tx_hash = parse_bytes32(
        v.get("transactionHash")
            .and_then(|x| x.as_str())
            .unwrap_or_default(),
    )?;
    let block_number = parse_hex_u64(
        v.get("blockNumber")
            .and_then(|x| x.as_str())
            .unwrap_or_default(),
    )
    .map_err(|e| BroadcastRpcError::Malformed(format!("receipt blockNumber: {e}")))?;
    let block_hash = parse_bytes32(
        v.get("blockHash")
            .and_then(|x| x.as_str())
            .unwrap_or_default(),
    )?;
    let status_str = v
        .get("status")
        .and_then(|x| x.as_str())
        .ok_or_else(|| BroadcastRpcError::Malformed("receipt missing status".into()))?;
    let status_u64 = parse_hex_u64(status_str)
        .map_err(|e| BroadcastRpcError::Malformed(format!("status: {e}")))?;
    if status_u64 > 1 {
        return Err(BroadcastRpcError::Malformed(format!(
            "receipt status out of range: {status_u64}"
        )));
    }
    let gas_used = parse_hex_u64(
        v.get("gasUsed")
            .and_then(|x| x.as_str())
            .unwrap_or_default(),
    )
    .map_err(|e| BroadcastRpcError::Malformed(format!("gasUsed: {e}")))?;
    let effective_gas_price_wei = parse_hex_u256(
        v.get("effectiveGasPrice")
            .and_then(|x| x.as_str())
            .unwrap_or("0x0"),
    )
    .map_err(|e| BroadcastRpcError::Malformed(format!("effectiveGasPrice: {e}")))?;
    let cumulative_gas_used = parse_hex_u64(
        v.get("cumulativeGasUsed")
            .and_then(|x| x.as_str())
            .unwrap_or("0x0"),
    )
    .map_err(|e| BroadcastRpcError::Malformed(format!("cumulativeGasUsed: {e}")))?;
    let from = parse_addr(v.get("from").and_then(|x| x.as_str()).unwrap_or_default())?;
    let to = match v.get("to").and_then(|x| x.as_str()) {
        Some(s) if !s.is_empty() && s != "null" => Some(parse_addr(s)?),
        _ => None,
    };
    Ok(TxReceipt {
        tx_hash,
        block_number,
        block_hash,
        status: status_u64 as u8,
        gas_used,
        effective_gas_price_wei,
        cumulative_gas_used,
        from,
        to,
    })
}

fn parse_block_header(v: &Value) -> Result<BlockHeader, BroadcastRpcError> {
    let number = parse_hex_u64(v.get("number").and_then(|x| x.as_str()).unwrap_or_default())
        .map_err(|e| BroadcastRpcError::Malformed(format!("block number: {e}")))?;
    let hash = parse_bytes32(v.get("hash").and_then(|x| x.as_str()).unwrap_or_default())?;
    let parent_hash = parse_bytes32(
        v.get("parentHash")
            .and_then(|x| x.as_str())
            .unwrap_or_default(),
    )?;
    let timestamp = parse_hex_u64(
        v.get("timestamp")
            .and_then(|x| x.as_str())
            .unwrap_or_default(),
    )
    .map_err(|e| BroadcastRpcError::Malformed(format!("timestamp: {e}")))?;
    Ok(BlockHeader {
        number,
        hash,
        parent_hash,
        timestamp,
    })
}

fn map_reqwest_err(err: reqwest::Error) -> BroadcastRpcError {
    if err.is_timeout() {
        BroadcastRpcError::Timeout
    } else {
        BroadcastRpcError::Transport(err.without_url().to_string())
    }
}

fn is_retryable(err: &BroadcastRpcError) -> bool {
    matches!(
        err,
        BroadcastRpcError::Transport(_)
            | BroadcastRpcError::Timeout
            | BroadcastRpcError::RateLimited
            | BroadcastRpcError::Unavailable(_)
    )
}

fn keccak256_of(bytes: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    let mut h = Keccak256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out[..]);
    a
}

fn to_hex_bytes32(bytes: &[u8; 32]) -> String {
    format!("0x{}", hex_encode(bytes))
}

fn to_hex_addr(addr: &[u8; 20]) -> String {
    format!("0x{}", hex_encode(addr))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let stripped = s
        .strip_prefix("0x")
        .ok_or_else(|| "expected 0x-prefixed hex".to_string())?;
    if stripped.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(stripped, 16).map_err(|e| e.to_string())
}

fn parse_hex_u256(s: &str) -> Result<U256, String> {
    let stripped = s
        .strip_prefix("0x")
        .ok_or_else(|| "expected 0x-prefixed hex".to_string())?;
    if stripped.is_empty() {
        return Ok(U256::ZERO);
    }
    U256::from_str_radix(stripped, 16).map_err(|e| e.to_string())
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let stripped = s
        .strip_prefix("0x")
        .ok_or_else(|| "expected 0x-prefixed hex".to_string())?;
    if stripped.is_empty() {
        return Ok(Vec::new());
    }
    if stripped.len() % 2 != 0 {
        return Err("odd length hex".to_string());
    }
    let mut out = Vec::with_capacity(stripped.len() / 2);
    for i in (0..stripped.len()).step_by(2) {
        let byte = u8::from_str_radix(&stripped[i..i + 2], 16).map_err(|e| e.to_string())?;
        out.push(byte);
    }
    Ok(out)
}

fn parse_bytes32(s: &str) -> Result<[u8; 32], BroadcastRpcError> {
    let bytes =
        parse_hex_bytes(s).map_err(|e| BroadcastRpcError::Malformed(format!("bytes32: {e}")))?;
    if bytes.len() != 32 {
        return Err(BroadcastRpcError::Malformed(format!(
            "expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn parse_addr(s: &str) -> Result<[u8; 20], BroadcastRpcError> {
    let bytes =
        parse_hex_bytes(s).map_err(|e| BroadcastRpcError::Malformed(format!("addr: {e}")))?;
    if bytes.len() != 20 {
        return Err(BroadcastRpcError::Malformed(format!(
            "expected 20 bytes, got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    Ok(out)
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_is_read_only_plus_send_raw() {
        for m in BROADCAST_ALLOWED_METHODS {
            // No other write methods allowed.
            let is_write = matches!(*m, "eth_sendRawTransaction");
            let is_read = !is_write;
            if is_read {
                assert_ne!(*m, "eth_sendTransaction");
                assert_ne!(*m, "eth_sign");
                assert_ne!(*m, "eth_signTransaction");
                assert_ne!(*m, "personal_sign");
            }
        }
        // send_raw_transaction is the ONLY write.
        assert!(BROADCAST_ALLOWED_METHODS.contains(&"eth_sendRawTransaction"));
    }

    #[test]
    fn check_method_rejects_non_allowlisted() {
        assert!(HttpExecutionBroadcastRpcClient::check_method("eth_chainId").is_ok());
        assert!(HttpExecutionBroadcastRpcClient::check_method("eth_sendRawTransaction").is_ok());
        assert!(matches!(
            HttpExecutionBroadcastRpcClient::check_method("eth_sendTransaction"),
            Err(BroadcastRpcError::MethodNotAllowed(_))
        ));
        assert!(matches!(
            HttpExecutionBroadcastRpcClient::check_method("eth_sign"),
            Err(BroadcastRpcError::MethodNotAllowed(_))
        ));
        assert!(matches!(
            HttpExecutionBroadcastRpcClient::check_method("eth_getLogs"),
            Err(BroadcastRpcError::MethodNotAllowed(_))
        ));
    }

    #[test]
    fn constructor_refuses_base_mainnet_expected_chain() {
        let err = HttpExecutionBroadcastRpcClient::new(
            "https://mainnet.example/rpc".to_string(),
            Duration::from_millis(1_000),
            0,
            Some(BASE_MAINNET_CHAIN_ID),
        )
        .expect_err("must refuse Base mainnet expected chain");
        assert_eq!(err, BroadcastRpcError::BaseMainnetForbidden);
    }

    #[test]
    fn constructor_refuses_non_http_scheme() {
        let err = HttpExecutionBroadcastRpcClient::new(
            "ws://foo".to_string(),
            Duration::from_millis(50),
            0,
            None,
        )
        .expect_err("scheme rejected");
        assert!(matches!(err, BroadcastRpcError::Transport(_)));
    }

    #[test]
    fn classify_already_known() {
        let out = classify_provider_rejection(-32000, "already known");
        assert!(matches!(out, SendOutcome::AlreadyKnown { .. }));
        let out = classify_provider_rejection(-32000, "ALREADY KNOWN");
        assert!(matches!(out, SendOutcome::AlreadyKnown { .. }));
    }

    #[test]
    fn classify_nonce_too_low() {
        let out = classify_provider_rejection(-32000, "nonce too low");
        assert_eq!(out, SendOutcome::NonceTooLow);
    }

    #[test]
    fn classify_replacement_underpriced() {
        let out = classify_provider_rejection(-32000, "replacement transaction underpriced");
        assert_eq!(out, SendOutcome::ReplacementUnderpriced);
    }

    #[test]
    fn classify_unknown_falls_through_to_generic_rejection() {
        let out = classify_provider_rejection(-32000, "internal error");
        assert!(matches!(out, SendOutcome::ProviderRejection { .. }));
    }

    #[test]
    fn debug_impl_redacts_endpoint() {
        let client = HttpExecutionBroadcastRpcClient::new(
            "https://rpc.example.com/v3/SUPER_SECRET_KEY".to_string(),
            Duration::from_millis(500),
            0,
            Some(84532),
        )
        .expect("constructs");
        let dbg = format!("{client:?}");
        assert!(!dbg.contains("SUPER_SECRET_KEY"), "{dbg}");
        assert!(dbg.contains("rpc.example.com"), "{dbg}");
    }
}
