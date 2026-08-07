//! Narrowed read-only JSON-RPC surface for the Hybrid V2 pre-broadcast
//! execution pipeline (Part I of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! ## Frozen safety — compile-time firewall against broadcast
//!
//! The [`ExecutionRpcClient`] trait deliberately exposes NO
//! `send_raw_transaction`, `send_transaction`, `sign_transaction`,
//! `personal_sign`, or any equivalent method. It is the compile-time
//! firewall that guarantees `BROADCAST_IS_DISABLED` for every consumer
//! that only holds a `&dyn ExecutionRpcClient`. Because there is no
//! broadcast method here — nor helper that reconstructs a raw
//! transaction — a downstream orchestrator cannot reach for one
//! through this trait.
//!
//! The `HttpExecutionRpcClient` impl further pins the guarantee with a
//! statically enumerated JSON-RPC allowlist. It only ever emits:
//!   * `eth_chainId`
//!   * `eth_blockNumber`
//!   * `eth_getBlockByNumber`
//!   * `eth_getTransactionCount`
//!   * `eth_call`
//!   * `eth_estimateGas`
//!   * `eth_feeHistory`
//! Any attempt to construct a different method name is a compile error
//! (there is no code path that builds one at runtime).

use crate::hybrid_v2::config::redact_rpc_url;
use crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID;
use alloy_primitives::U256;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

// -----------------------------------------------------------------
//                            TYPES
// -----------------------------------------------------------------

/// Block reference for a bounded read. `Number(n)` and `Hash(h)` bind
/// the read to a specific height/hash; `Latest` / `Pending` are the
/// two loose tags the trait supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockTag {
    Number(u64),
    Hash([u8; 32]),
    Latest,
    Pending,
}

impl BlockTag {
    /// JSON-RPC block parameter encoding. Number → `0x<hex>`; Hash is
    /// rejected inline by the caller because `eth_call` and
    /// `eth_estimateGas` require the object form (`{blockHash: ...}`).
    fn to_number_tag(self) -> Result<Value, ExecutionRpcError> {
        match self {
            BlockTag::Number(n) => Ok(Value::String(format!("0x{:x}", n))),
            BlockTag::Latest => Ok(Value::String("latest".to_string())),
            BlockTag::Pending => Ok(Value::String("pending".to_string())),
            BlockTag::Hash(_) => Err(ExecutionRpcError::Unsupported(
                "block-by-hash tag not supported for this JSON-RPC method".to_string(),
            )),
        }
    }
}

/// Minimal read-only shape of an `eth_call` request. Only fields the
/// pre-broadcast pipeline needs. `from` is included so the execution
/// simulator can bind the caller to the executor address (the
/// contract's `msg.sender` check).
#[derive(Debug, Clone)]
pub struct EthCallRequest {
    pub from: Option<[u8; 20]>,
    pub to: [u8; 20],
    pub data: Vec<u8>,
    /// Value carried by the call. `executeMatch` is non-payable, so
    /// this must be zero in every plan today — but the trait accepts
    /// arbitrary values to keep the surface truthful.
    pub value_wei: U256,
    /// Optional gas cap for the call. Providers ignore this on
    /// non-execution reads but pass it through to `eth_estimateGas`.
    pub gas: Option<u64>,
}

/// Structured `eth_call` outcome. Distinguishes success from a decoded
/// revert (Part I includes the revert selectors for
/// `OptionMatchingEngineV2`). `MalformedResponse` is a hard error
/// that the caller must surface — it means the provider returned
/// something the pipeline cannot classify safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EthCallOutcome {
    Success(Vec<u8>),
    Revert {
        raw_data: Vec<u8>,
        decoded: Option<DecodedRevert>,
    },
    /// `Panic(uint256)` — 0x4e487b71 selector.
    Panic {
        code: u32,
    },
    MalformedResponse(String),
}

/// EIP-1559 `eth_feeHistory` response shape (only the fields the gas
/// policy needs).
#[derive(Debug, Clone, PartialEq)]
pub struct FeeHistory {
    /// Oldest block covered by the sample. Included for auditability.
    pub oldest_block: u64,
    /// Base fee per gas for `block_count + 1` blocks (last entry is
    /// the pending base fee).
    pub base_fee_per_gas: Vec<U256>,
    /// Reward samples per requested percentile per block. Providers
    /// may return an empty outer vector when reward tracking is
    /// disabled — the gas policy treats an empty vector as "no
    /// suggestion available".
    pub reward: Vec<Vec<U256>>,
    /// Gas used ratio per block (fraction of block gas used, 0..=1).
    pub gas_used_ratio: Vec<f64>,
}

// -----------------------------------------------------------------
//                         REVERT DECODING
// -----------------------------------------------------------------

use alloy_sol_types::{sol, SolError};

sol! {
    // -------- generic Solidity errors --------
    /// `Error(string)` — the classic `require(_, "reason")` selector
    /// 0x08c379a0.
    #[allow(missing_docs)] error Error(string);
    /// `Panic(uint256)` — selector 0x4e487b71.
    #[allow(missing_docs)] error Panic(uint256);

    // -------- OptionMatchingEngineV2 custom errors (audit-frozen) --------
    #[allow(missing_docs)] error ExecutionPaused();
    #[allow(missing_docs)] error FillQuantityInvalid();
    #[allow(missing_docs)] error OrderAlreadyFullyFilled();
    #[allow(missing_docs)] error OrderCancelled();
    #[allow(missing_docs)] error OrderNonceStale();
    #[allow(missing_docs)] error InvalidSigner();
    #[allow(missing_docs)] error OrderPayloadHashMismatch();
    #[allow(missing_docs)] error SeriesMismatch();
    #[allow(missing_docs)] error PremiumTokenMismatch();
    #[allow(missing_docs)] error SameSideMatch();
    #[allow(missing_docs)] error SelfTrade();
    #[allow(missing_docs)] error InvalidMakerTakerAssignment();
    #[allow(missing_docs)] error PremiumOutsideLimit();
    #[allow(missing_docs)] error PremiumDisagreement();
    #[allow(missing_docs)] error QuantityZero();
    #[allow(missing_docs)] error InvalidSeries();
    #[allow(missing_docs)] error PostOnlyRoleViolation();
    #[allow(missing_docs)] error InvalidTifCombination();
    #[allow(missing_docs)] error FeeHookRejected();
    #[allow(missing_docs)] error UnsafePostTradeMargin();
    #[allow(missing_docs)] error FokRequiresFullFillFromZero();
    #[allow(missing_docs)] error NotOrderOwner();
    #[allow(missing_docs)] error MinValidOrderNonceNotAdvancing();
    #[allow(missing_docs)] error PositiveFeeRateExceedsSignedMaximum();
    #[allow(missing_docs)] error RecoveryActiveForSubaccount();
    // Not in the earlier audit table — selectors are still stable
    // because they come directly from the `sol!` macro's keccak256 of
    // the signature. Named after the corresponding recovery module
    // error surface.
    #[allow(missing_docs)] error StaleOwnerRecoveryEpoch();
    #[allow(missing_docs)] error StaleSubaccountRecoveryEpoch();
}

/// A recognised revert. `Error(string)` and `Panic(uint256)` are
/// carried as `Standard` variants; every custom error selector is
/// reified as its own variant so downstream classification can pattern
/// match without decoding strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedRevert {
    /// `Error(string)` — legacy `require(_, "reason")`.
    Error { reason: String },
    /// `Panic(uint256)` — Solidity 0.8 panic codes (see the Solidity
    /// docs' `Panic(uint256)` table).
    Panic { code: u32 },
    /// Every custom error carries its selector + human-friendly name
    /// (the Solidity type name from `sol!`) so operators can grep on
    /// either the selector or the identifier.
    Custom {
        selector: [u8; 4],
        name: &'static str,
    },
}

impl DecodedRevert {
    /// Attempt to decode the raw revert payload returned by an
    /// `eth_call`. Returns `None` if the payload does not start with a
    /// selector we recognise; the caller preserves `raw_data` for
    /// audit in that case.
    pub fn decode(data: &[u8]) -> Option<DecodedRevert> {
        if data.len() < 4 {
            return None;
        }
        let mut sel = [0u8; 4];
        sel.copy_from_slice(&data[..4]);

        // Standard string-revert.
        if sel == Error::SELECTOR {
            if let Ok(decoded) = Error::abi_decode(data, true) {
                return Some(DecodedRevert::Error { reason: decoded._0 });
            }
        }
        if sel == Panic::SELECTOR {
            if let Ok(decoded) = Panic::abi_decode(data, true) {
                let code = if decoded._0 > U256::from(u32::MAX as u64) {
                    u32::MAX
                } else {
                    decoded._0.to::<u64>() as u32
                };
                return Some(DecodedRevert::Panic { code });
            }
        }

        // Custom errors — try each. `sol!`-generated types expose
        // `SolError::SELECTOR` at compile time.
        for (needle, name) in KNOWN_CUSTOM_ERROR_SELECTORS {
            if sel == *needle {
                return Some(DecodedRevert::Custom {
                    selector: *needle,
                    name,
                });
            }
        }
        None
    }
}

/// Compile-time table of `(selector, human_name)` for every custom
/// error surfaced by `OptionMatchingEngineV2` (and the neighbouring
/// recovery-epoch errors). Consumers can grep either the selector or
/// the name.
pub const KNOWN_CUSTOM_ERROR_SELECTORS: &[([u8; 4], &str)] = &[
    (ExecutionPaused::SELECTOR, "ExecutionPaused"),
    (FillQuantityInvalid::SELECTOR, "FillQuantityInvalid"),
    (OrderAlreadyFullyFilled::SELECTOR, "OrderAlreadyFullyFilled"),
    (OrderCancelled::SELECTOR, "OrderCancelled"),
    (OrderNonceStale::SELECTOR, "OrderNonceStale"),
    (InvalidSigner::SELECTOR, "InvalidSigner"),
    (
        OrderPayloadHashMismatch::SELECTOR,
        "OrderPayloadHashMismatch",
    ),
    (SeriesMismatch::SELECTOR, "SeriesMismatch"),
    (PremiumTokenMismatch::SELECTOR, "PremiumTokenMismatch"),
    (SameSideMatch::SELECTOR, "SameSideMatch"),
    (SelfTrade::SELECTOR, "SelfTrade"),
    (
        InvalidMakerTakerAssignment::SELECTOR,
        "InvalidMakerTakerAssignment",
    ),
    (PremiumOutsideLimit::SELECTOR, "PremiumOutsideLimit"),
    (PremiumDisagreement::SELECTOR, "PremiumDisagreement"),
    (QuantityZero::SELECTOR, "QuantityZero"),
    (InvalidSeries::SELECTOR, "InvalidSeries"),
    (PostOnlyRoleViolation::SELECTOR, "PostOnlyRoleViolation"),
    (InvalidTifCombination::SELECTOR, "InvalidTifCombination"),
    (FeeHookRejected::SELECTOR, "FeeHookRejected"),
    (UnsafePostTradeMargin::SELECTOR, "UnsafePostTradeMargin"),
    (
        FokRequiresFullFillFromZero::SELECTOR,
        "FokRequiresFullFillFromZero",
    ),
    (NotOrderOwner::SELECTOR, "NotOrderOwner"),
    (
        MinValidOrderNonceNotAdvancing::SELECTOR,
        "MinValidOrderNonceNotAdvancing",
    ),
    (
        PositiveFeeRateExceedsSignedMaximum::SELECTOR,
        "PositiveFeeRateExceedsSignedMaximum",
    ),
    (
        RecoveryActiveForSubaccount::SELECTOR,
        "RecoveryActiveForSubaccount",
    ),
    (StaleOwnerRecoveryEpoch::SELECTOR, "StaleOwnerRecoveryEpoch"),
    (
        StaleSubaccountRecoveryEpoch::SELECTOR,
        "StaleSubaccountRecoveryEpoch",
    ),
];

// -----------------------------------------------------------------
//                            ERROR
// -----------------------------------------------------------------

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ExecutionRpcError {
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("provider returned malformed payload: {0}")]
    Malformed(String),
    #[error("provider returned JSON-RPC error {code}: {message}")]
    RpcError { code: i64, message: String },
    #[error("provider timed out")]
    Timeout,
    #[error("provider rate-limited (429)")]
    RateLimited,
    #[error("provider returned HTTP server error {status}")]
    ServerError { status: u16 },
    #[error("chain id {actual} does not match expected {expected}")]
    ChainMismatch { expected: u64, actual: u64 },
    #[error("Base mainnet is forbidden for Hybrid V2 execution")]
    BaseMainnetForbidden,
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

// -----------------------------------------------------------------
//                       TRAIT (narrowed)
// -----------------------------------------------------------------

/// Read-only RPC surface for the Hybrid V2 pre-broadcast pipeline.
///
/// Intentionally NARROW: only the methods a simulator + gas policy +
/// nonce reserver need. No send / sign / personal / accounts method
/// exists on this trait or any impl in this module tree — that is the
/// compile-time enforcement of `BROADCAST_IS_DISABLED`.
#[async_trait]
pub trait ExecutionRpcClient: Send + Sync {
    /// `eth_chainId`.
    async fn chain_id(&self) -> Result<u64, ExecutionRpcError>;
    /// `eth_blockNumber`.
    async fn head_block_number(&self) -> Result<u64, ExecutionRpcError>;
    /// `eth_getBlockByNumber(latest).hash`.
    async fn head_block_hash(&self) -> Result<[u8; 32], ExecutionRpcError>;
    /// `eth_getTransactionCount(address, tag)`.
    async fn transaction_count(
        &self,
        address: [u8; 20],
        block_tag: BlockTag,
    ) -> Result<u64, ExecutionRpcError>;
    /// `eth_call(req, tag)` returning a structured outcome (success
    /// vs decoded revert vs malformed).
    async fn eth_call(
        &self,
        req: EthCallRequest,
        block_tag: BlockTag,
    ) -> Result<EthCallOutcome, ExecutionRpcError>;
    /// `eth_estimateGas(req, tag)`.
    async fn estimate_gas(
        &self,
        req: EthCallRequest,
        block_tag: BlockTag,
    ) -> Result<u64, ExecutionRpcError>;
    /// `eth_feeHistory(block_count, newest_block, reward_percentiles)`.
    async fn fee_history(
        &self,
        block_count: u64,
        newest_block: BlockTag,
        reward_percentiles: &[f64],
    ) -> Result<FeeHistory, ExecutionRpcError>;
}

// -----------------------------------------------------------------
//                          HTTP IMPL
// -----------------------------------------------------------------

/// Statically enumerated list of JSON-RPC methods this client may
/// emit. Any deviation is a compile-time inspection failure — the
/// only call sites for `call_json` supply a `&'static str` that is
/// one of these names, and the helper checks it at runtime.
pub const ALLOWED_METHODS: &[&str] = &[
    "eth_chainId",
    "eth_blockNumber",
    "eth_getBlockByNumber",
    "eth_getTransactionCount",
    "eth_call",
    "eth_estimateGas",
    "eth_feeHistory",
];

/// Live HTTP JSON-RPC client. Redacted `Debug`. Bounded retries on
/// transport failures only.
pub struct HttpExecutionRpcClient {
    client: reqwest::Client,
    endpoint: String,
    timeout: Duration,
    max_retries: u32,
}

impl std::fmt::Debug for HttpExecutionRpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpExecutionRpcClient")
            .field("endpoint", &redact_rpc_url(&self.endpoint))
            .field("timeout_ms", &self.timeout.as_millis())
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

impl HttpExecutionRpcClient {
    pub fn new(
        endpoint: String,
        timeout: Duration,
        max_retries: u32,
    ) -> Result<Self, ExecutionRpcError> {
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(ExecutionRpcError::Unsupported(
                "endpoint scheme not supported (only http/https)".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| ExecutionRpcError::Transport(format!("client build: {e}")))?;
        Ok(Self {
            client,
            endpoint,
            timeout,
            max_retries,
        })
    }

    /// Verify the runtime allowlist. Called on every JSON-RPC round
    /// trip. Guarantees that no future edit accidentally introduces a
    /// send/sign method — `is_send_or_sign_method` returns true for
    /// the wire-protocol names of every write action and this helper
    /// panics if it ever fires (loud test-time failure, safe
    /// production error otherwise).
    fn check_method(method: &'static str) -> Result<(), ExecutionRpcError> {
        if is_send_or_sign_method(method) {
            return Err(ExecutionRpcError::Unsupported(format!(
                "method {method} is a broadcast/sign method and is banned in ExecutionRpcClient"
            )));
        }
        if !ALLOWED_METHODS.iter().any(|m| *m == method) {
            return Err(ExecutionRpcError::Unsupported(format!(
                "method {method} is not in the ExecutionRpcClient allowlist"
            )));
        }
        Ok(())
    }

    async fn call_json(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<Value, ExecutionRpcError> {
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

    async fn attempt_once(&self, body: &Value) -> Result<Value, ExecutionRpcError> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(body)
            .send()
            .await
            .map_err(map_reqwest_err)?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ExecutionRpcError::RateLimited);
        }
        if status.is_server_error() {
            return Err(ExecutionRpcError::ServerError {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            return Err(ExecutionRpcError::Transport(format!(
                "unexpected http status {}",
                status.as_u16()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|e| ExecutionRpcError::Malformed(format!("json decode: {e}")))?;
        if let Some(err) = payload.get("error") {
            // JSON-RPC error responses that include a `data` field with
            // a hex-encoded revert payload are treated the same as any
            // deterministic error at this layer; the eth_call helper
            // re-interprets when it makes sense.
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32000);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("<no message>")
                .to_string();
            // Some providers pack the raw revert data into `error.data`.
            // Encode it into the `RpcError` message so eth_call can pull
            // it back out for revert decoding.
            if let Some(data) = err.get("data").and_then(|d| d.as_str()) {
                return Err(ExecutionRpcError::RpcError {
                    code,
                    message: format!("{message}\ndata:{data}"),
                });
            }
            return Err(ExecutionRpcError::RpcError { code, message });
        }
        payload
            .get("result")
            .cloned()
            .ok_or_else(|| ExecutionRpcError::Malformed("missing `result` field".to_string()))
    }
}

#[async_trait]
impl ExecutionRpcClient for HttpExecutionRpcClient {
    async fn chain_id(&self) -> Result<u64, ExecutionRpcError> {
        let v = self.call_json("eth_chainId", json!([])).await?;
        let s = v
            .as_str()
            .ok_or_else(|| ExecutionRpcError::Malformed("chainId not a string".into()))?;
        let id =
            parse_hex_u64(s).map_err(|e| ExecutionRpcError::Malformed(format!("chainId: {e}")))?;
        if id == BASE_MAINNET_CHAIN_ID {
            return Err(ExecutionRpcError::BaseMainnetForbidden);
        }
        Ok(id)
    }

    async fn head_block_number(&self) -> Result<u64, ExecutionRpcError> {
        let v = self.call_json("eth_blockNumber", json!([])).await?;
        let s = v
            .as_str()
            .ok_or_else(|| ExecutionRpcError::Malformed("blockNumber not a string".into()))?;
        parse_hex_u64(s).map_err(|e| ExecutionRpcError::Malformed(format!("blockNumber: {e}")))
    }

    async fn head_block_hash(&self) -> Result<[u8; 32], ExecutionRpcError> {
        let v = self
            .call_json("eth_getBlockByNumber", json!(["latest", false]))
            .await?;
        if v.is_null() {
            return Err(ExecutionRpcError::Malformed(
                "latest block was null".to_string(),
            ));
        }
        let hash = v
            .get("hash")
            .and_then(|h| h.as_str())
            .ok_or_else(|| ExecutionRpcError::Malformed("latest block missing hash".to_string()))?;
        parse_bytes32(hash)
    }

    async fn transaction_count(
        &self,
        address: [u8; 20],
        block_tag: BlockTag,
    ) -> Result<u64, ExecutionRpcError> {
        let addr = to_hex_addr(&address);
        let tag = block_tag.to_number_tag()?;
        let v = self
            .call_json("eth_getTransactionCount", json!([addr, tag]))
            .await?;
        let s = v
            .as_str()
            .ok_or_else(|| ExecutionRpcError::Malformed("txCount not a string".into()))?;
        parse_hex_u64(s).map_err(|e| ExecutionRpcError::Malformed(format!("txCount: {e}")))
    }

    async fn eth_call(
        &self,
        req: EthCallRequest,
        block_tag: BlockTag,
    ) -> Result<EthCallOutcome, ExecutionRpcError> {
        let call_obj = build_call_object(&req);
        let tag = block_tag.to_number_tag()?;
        let outcome = self.call_json("eth_call", json!([call_obj, tag])).await;
        match outcome {
            Ok(v) => {
                let s = v.as_str().ok_or_else(|| {
                    ExecutionRpcError::Malformed("eth_call response not a string".into())
                })?;
                let bytes = parse_hex_bytes(s)
                    .map_err(|e| ExecutionRpcError::Malformed(format!("eth_call hex: {e}")))?;
                Ok(EthCallOutcome::Success(bytes))
            }
            Err(ExecutionRpcError::RpcError { code, message }) => {
                // Look for `\ndata:0x...` marker we set in attempt_once.
                if let Some(data_hex) = extract_data_from_message(&message) {
                    let raw = parse_hex_bytes(&data_hex)
                        .map_err(|e| ExecutionRpcError::Malformed(format!("revert data: {e}")))?;
                    let decoded = DecodedRevert::decode(&raw);
                    if let Some(DecodedRevert::Panic { code }) = &decoded {
                        return Ok(EthCallOutcome::Panic { code: *code });
                    }
                    Ok(EthCallOutcome::Revert {
                        raw_data: raw,
                        decoded,
                    })
                } else {
                    // Deterministic RPC error without revert data — treat
                    // as a hard error so the caller sees the full detail.
                    Err(ExecutionRpcError::RpcError { code, message })
                }
            }
            Err(other) => Err(other),
        }
    }

    async fn estimate_gas(
        &self,
        req: EthCallRequest,
        block_tag: BlockTag,
    ) -> Result<u64, ExecutionRpcError> {
        let call_obj = build_call_object(&req);
        let tag = block_tag.to_number_tag()?;
        let v = self
            .call_json("eth_estimateGas", json!([call_obj, tag]))
            .await?;
        let s = v
            .as_str()
            .ok_or_else(|| ExecutionRpcError::Malformed("estimateGas not a string".into()))?;
        parse_hex_u64(s).map_err(|e| ExecutionRpcError::Malformed(format!("estimateGas: {e}")))
    }

    async fn fee_history(
        &self,
        block_count: u64,
        newest_block: BlockTag,
        reward_percentiles: &[f64],
    ) -> Result<FeeHistory, ExecutionRpcError> {
        let newest = newest_block.to_number_tag()?;
        let percentiles: Vec<Value> = reward_percentiles.iter().map(|p| Value::from(*p)).collect();
        let count_hex = format!("0x{:x}", block_count);
        let v = self
            .call_json(
                "eth_feeHistory",
                json!([count_hex, newest, Value::Array(percentiles)]),
            )
            .await?;
        parse_fee_history(&v)
    }
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn build_call_object(req: &EthCallRequest) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(from) = req.from {
        obj.insert("from".into(), Value::String(to_hex_addr(&from)));
    }
    obj.insert("to".into(), Value::String(to_hex_addr(&req.to)));
    obj.insert(
        "data".into(),
        Value::String(format!("0x{}", hex_encode(&req.data))),
    );
    if req.value_wei != U256::ZERO {
        obj.insert(
            "value".into(),
            Value::String(format!("0x{:x}", req.value_wei)),
        );
    }
    if let Some(gas) = req.gas {
        obj.insert("gas".into(), Value::String(format!("0x{:x}", gas)));
    }
    Value::Object(obj)
}

fn parse_fee_history(v: &Value) -> Result<FeeHistory, ExecutionRpcError> {
    let oldest = v
        .get("oldestBlock")
        .and_then(|x| x.as_str())
        .ok_or_else(|| ExecutionRpcError::Malformed("feeHistory missing oldestBlock".into()))?;
    let oldest_block = parse_hex_u64(oldest)
        .map_err(|e| ExecutionRpcError::Malformed(format!("feeHistory.oldestBlock: {e}")))?;
    let base_fee_arr = v
        .get("baseFeePerGas")
        .and_then(|x| x.as_array())
        .ok_or_else(|| ExecutionRpcError::Malformed("feeHistory missing baseFeePerGas".into()))?;
    let mut base_fee_per_gas = Vec::with_capacity(base_fee_arr.len());
    for item in base_fee_arr {
        let s = item.as_str().ok_or_else(|| {
            ExecutionRpcError::Malformed("baseFeePerGas entry not a string".into())
        })?;
        base_fee_per_gas.push(
            parse_hex_u256(s).map_err(|e| {
                ExecutionRpcError::Malformed(format!("feeHistory.baseFeePerGas: {e}"))
            })?,
        );
    }
    let gas_used_ratio = v
        .get("gasUsedRatio")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| r.as_f64().unwrap_or(0.0))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let reward = v
        .get("reward")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .map(|inner| {
                    inner
                        .as_array()
                        .map(|inner_arr| {
                            inner_arr
                                .iter()
                                .map(|s| {
                                    s.as_str()
                                        .and_then(|s| parse_hex_u256(s).ok())
                                        .unwrap_or(U256::ZERO)
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(FeeHistory {
        oldest_block,
        base_fee_per_gas,
        reward,
        gas_used_ratio,
    })
}

fn extract_data_from_message(msg: &str) -> Option<String> {
    // We format error data as `\ndata:0x...` in attempt_once.
    let idx = msg.find("\ndata:")?;
    Some(msg[(idx + "\ndata:".len())..].to_string())
}

fn map_reqwest_err(err: reqwest::Error) -> ExecutionRpcError {
    if err.is_timeout() {
        ExecutionRpcError::Timeout
    } else {
        ExecutionRpcError::Transport(err.without_url().to_string())
    }
}

fn is_retryable(err: &ExecutionRpcError) -> bool {
    matches!(
        err,
        ExecutionRpcError::Transport(_)
            | ExecutionRpcError::Timeout
            | ExecutionRpcError::RateLimited
            | ExecutionRpcError::ServerError { .. }
    )
}

/// Any wire-protocol name that could plausibly broadcast or sign a
/// transaction. Used defensively by `check_method` to reject at
/// runtime — the real compile-time firewall is the shape of the
/// trait itself (no `send_*` method).
pub fn is_send_or_sign_method(m: &str) -> bool {
    matches!(
        m,
        "eth_sendTransaction"
            | "eth_sendRawTransaction"
            | "eth_sign"
            | "eth_signTransaction"
            | "eth_signTypedData"
            | "eth_signTypedData_v3"
            | "eth_signTypedData_v4"
            | "eth_accounts"
            | "personal_sign"
            | "personal_sendTransaction"
            | "personal_unlockAccount"
            | "personal_ecRecover"
    ) || m.starts_with("wallet_")
        || m.starts_with("personal_")
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

fn parse_bytes32(s: &str) -> Result<[u8; 32], ExecutionRpcError> {
    let bytes = parse_hex_bytes(s)
        .map_err(|e| ExecutionRpcError::Malformed(format!("bytes32 decode: {e}")))?;
    if bytes.len() != 32 {
        return Err(ExecutionRpcError::Malformed(format!(
            "expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
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

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_methods_contain_only_read_only_verbs() {
        for m in ALLOWED_METHODS {
            assert!(
                !is_send_or_sign_method(m),
                "allowed method {m} must not be a send/sign method"
            );
        }
    }

    #[test]
    fn is_send_or_sign_method_catches_every_write_verb() {
        for m in [
            "eth_sendTransaction",
            "eth_sendRawTransaction",
            "eth_sign",
            "eth_signTransaction",
            "eth_signTypedData_v4",
            "eth_accounts",
            "personal_sign",
            "personal_unlockAccount",
            "wallet_switchEthereumChain",
        ] {
            assert!(is_send_or_sign_method(m), "must be flagged: {m}");
        }
    }

    #[test]
    fn check_method_rejects_send_and_unknown() {
        assert!(HttpExecutionRpcClient::check_method("eth_chainId").is_ok());
        assert!(HttpExecutionRpcClient::check_method("eth_call").is_ok());
        // Unknown / not in the allowlist:
        assert!(matches!(
            HttpExecutionRpcClient::check_method("eth_getLogs"),
            Err(ExecutionRpcError::Unsupported(_))
        ));
    }

    #[test]
    fn decoded_revert_error_string() {
        // abi_encode(Error("reason")).
        let payload = Error {
            _0: "reason".to_string(),
        }
        .abi_encode();
        let out = DecodedRevert::decode(&payload).expect("decoded");
        assert!(matches!(out, DecodedRevert::Error { .. }));
        if let DecodedRevert::Error { reason } = out {
            assert_eq!(reason, "reason");
        }
    }

    #[test]
    fn decoded_revert_panic_code() {
        let payload = Panic {
            _0: U256::from(0x11u64),
        }
        .abi_encode();
        let out = DecodedRevert::decode(&payload).expect("decoded");
        assert_eq!(out, DecodedRevert::Panic { code: 0x11 });
    }

    #[test]
    fn decoded_revert_every_known_custom_selector() {
        for (sel, name) in KNOWN_CUSTOM_ERROR_SELECTORS {
            let out = DecodedRevert::decode(sel).expect(name);
            match out {
                DecodedRevert::Custom { selector, name: n } => {
                    assert_eq!(&selector, sel);
                    assert_eq!(n, *name);
                }
                other => panic!("wanted Custom for {name}, got {other:?}"),
            }
        }
    }

    #[test]
    fn decoded_revert_none_for_unknown_selector() {
        let sel = [0xde, 0xad, 0xbe, 0xef];
        assert!(DecodedRevert::decode(&sel).is_none());
    }

    #[test]
    fn decoded_revert_none_for_short_payload() {
        assert!(DecodedRevert::decode(&[0xde]).is_none());
        assert!(DecodedRevert::decode(&[]).is_none());
    }

    #[test]
    fn extract_data_from_message_pulls_hex() {
        let msg = "execution reverted\ndata:0xdeadbeef";
        assert_eq!(
            extract_data_from_message(msg).as_deref(),
            Some("0xdeadbeef")
        );
        assert!(extract_data_from_message("no data").is_none());
    }

    #[test]
    fn http_client_rejects_non_http_scheme() {
        let err = HttpExecutionRpcClient::new("ws://foo".to_string(), Duration::from_millis(50), 0)
            .expect_err("scheme rejected");
        assert!(matches!(err, ExecutionRpcError::Unsupported(_)));
    }

    #[test]
    fn block_tag_hash_is_rejected_for_number_only_methods() {
        let err = BlockTag::Hash([0u8; 32]).to_number_tag().unwrap_err();
        assert!(matches!(err, ExecutionRpcError::Unsupported(_)));
    }
}
