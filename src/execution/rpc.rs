use crate::confirmation::ConfirmationReceipt;
use crate::error::{BackendError, Result};
use crate::execution::revert::diagnostics_from_rpc_error;
use crate::types::AccountId;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

pub type RpcFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EthCallRequest {
    pub from: AccountId,
    pub to: AccountId,
    pub data: Vec<u8>,
    pub value: u128,
    pub gas_limit: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EthCallSuccess {
    pub block_number: Option<u64>,
    pub output: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EstimateGasRequest {
    pub from: AccountId,
    pub to: AccountId,
    pub data: Vec<u8>,
    pub value: u128,
}

pub trait EthCallProvider: Clone + Send + Sync {
    fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess>;
}

pub trait GasEstimateProvider: Clone + Send + Sync {
    fn estimate_gas(&self, request: EstimateGasRequest) -> RpcFuture<'_, u64>;
}

pub trait TransactionBroadcastProvider: Clone + Send + Sync {
    fn chain_id(&self) -> RpcFuture<'_, u64>;
    fn transaction_count(&self, address: AccountId) -> RpcFuture<'_, u64>;
    fn send_raw_transaction(&self, raw_transaction: String) -> RpcFuture<'_, String>;
}

pub trait TransactionReceiptProvider: Clone + Send + Sync {
    fn block_number(&self) -> RpcFuture<'_, u64>;
    fn transaction_receipt(&self, tx_hash: String) -> RpcFuture<'_, Option<ConfirmationReceipt>>;
}

pub trait EthLogsProvider: Clone + Send + Sync {
    fn block_number(&self) -> RpcFuture<'_, u64>;
    fn get_logs(&self, filter: EthGetLogsFilter) -> RpcFuture<'_, Vec<crate::indexer::EthLog>>;
}

/// Minimal `eth_getBalance` surface for the broadcast policy data provider.
/// Returns the native (wei) balance of `address` at the latest block.
pub trait EthBalanceProvider: Clone + Send + Sync {
    fn eth_get_balance(&self, address: AccountId) -> RpcFuture<'_, u128>;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EthGetLogsFilter {
    pub from_block: String,
    pub to_block: String,
    pub address: String,
    pub topics: Vec<String>,
}

#[derive(Clone)]
pub struct HttpJsonRpcProvider {
    rpc_url: String,
    client: reqwest::Client,
}

impl HttpJsonRpcProvider {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

impl TransactionBroadcastProvider for HttpJsonRpcProvider {
    fn chain_id(&self) -> RpcFuture<'_, u64> {
        Box::pin(async move {
            let response: JsonRpcResponse<String> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_chainId",
                    params: Vec::<serde_json::Value>::new(),
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Simulation(error.message));
            }
            let result = response.result.ok_or_else(|| {
                BackendError::Simulation("eth_chainId returned no result".to_string())
            })?;
            parse_hex_quantity_u64(&result)
        })
    }

    fn transaction_count(&self, address: AccountId) -> RpcFuture<'_, u64> {
        Box::pin(async move {
            let response: JsonRpcResponse<String> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_getTransactionCount",
                    params: (address.0, "pending"),
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Simulation(error.message));
            }
            let result = response.result.ok_or_else(|| {
                BackendError::Simulation("eth_getTransactionCount returned no result".to_string())
            })?;
            parse_hex_quantity_u64(&result)
        })
    }

    fn send_raw_transaction(&self, raw_transaction: String) -> RpcFuture<'_, String> {
        Box::pin(async move {
            let response: JsonRpcResponse<String> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_sendRawTransaction",
                    params: [raw_transaction],
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Simulation(error.message));
            }
            let tx_hash = response.result.ok_or_else(|| {
                BackendError::Simulation("eth_sendRawTransaction returned no result".to_string())
            })?;
            validate_tx_hash(&tx_hash)?;
            Ok(tx_hash.to_ascii_lowercase())
        })
    }
}

impl EthCallProvider for HttpJsonRpcProvider {
    fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
        Box::pin(async move {
            let block_number = TransactionReceiptProvider::block_number(self).await.ok();
            let response: JsonRpcResponse<String> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_call",
                    params: (
                        EthCallParams {
                            from: request.from.0,
                            to: request.to.0,
                            data: hex_0x(&request.data),
                            value: hex_quantity_u128(request.value),
                            gas: request.gas_limit.map(hex_quantity_u64),
                        },
                        "latest",
                    ),
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;

            if let Some(error) = response.error {
                return Err(BackendError::SimulationReverted(Box::new(
                    diagnostics_from_rpc_error(&error.message, error.data.as_ref()),
                )));
            }
            let result = response.result.ok_or_else(|| {
                BackendError::Simulation("eth_call returned no result".to_string())
            })?;

            Ok(EthCallSuccess {
                block_number,
                output: decode_hex_bytes(&result)?,
            })
        })
    }
}

impl GasEstimateProvider for HttpJsonRpcProvider {
    fn estimate_gas(&self, request: EstimateGasRequest) -> RpcFuture<'_, u64> {
        Box::pin(async move {
            let response: JsonRpcResponse<String> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_estimateGas",
                    params: (
                        EstimateGasParams {
                            from: request.from.0,
                            to: request.to.0,
                            data: hex_0x(&request.data),
                            value: hex_quantity_u128(request.value),
                        },
                        "latest",
                    ),
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;

            if let Some(error) = response.error {
                return Err(BackendError::SimulationReverted(Box::new(
                    diagnostics_from_rpc_error(&error.message, error.data.as_ref()),
                )));
            }
            let result = response.result.ok_or_else(|| {
                BackendError::Simulation("eth_estimateGas returned no result".to_string())
            })?;
            parse_hex_quantity_u64(&result)
        })
    }
}

impl TransactionReceiptProvider for HttpJsonRpcProvider {
    fn block_number(&self) -> RpcFuture<'_, u64> {
        Box::pin(async move {
            let response: JsonRpcResponse<String> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_blockNumber",
                    params: Vec::<serde_json::Value>::new(),
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Simulation(error.message));
            }
            let result = response.result.ok_or_else(|| {
                BackendError::Simulation("eth_blockNumber returned no result".to_string())
            })?;
            parse_hex_quantity_u64(&result)
        })
    }

    fn transaction_receipt(&self, tx_hash: String) -> RpcFuture<'_, Option<ConfirmationReceipt>> {
        Box::pin(async move {
            validate_tx_hash(&tx_hash)?;
            let response: JsonRpcResponse<Option<EthTransactionReceipt>> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_getTransactionReceipt",
                    params: [tx_hash],
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Simulation(error.message));
            }
            response
                .result
                .flatten()
                .map(ConfirmationReceipt::try_from)
                .transpose()
        })
    }
}

impl EthBalanceProvider for HttpJsonRpcProvider {
    fn eth_get_balance(&self, address: AccountId) -> RpcFuture<'_, u128> {
        Box::pin(async move {
            let response: JsonRpcResponse<String> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_getBalance",
                    params: (address.0, "latest"),
                })
                .send()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Simulation(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Simulation(error.message));
            }
            let result = response.result.ok_or_else(|| {
                BackendError::Simulation("eth_getBalance returned no result".to_string())
            })?;
            parse_hex_quantity_u128(&result)
        })
    }
}

impl EthLogsProvider for HttpJsonRpcProvider {
    fn block_number(&self) -> RpcFuture<'_, u64> {
        TransactionReceiptProvider::block_number(self)
    }

    fn get_logs(&self, filter: EthGetLogsFilter) -> RpcFuture<'_, Vec<crate::indexer::EthLog>> {
        Box::pin(async move {
            let response: JsonRpcResponse<Vec<crate::indexer::EthLog>> = self
                .client
                .post(&self.rpc_url)
                .json(&JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_getLogs",
                    params: [filter],
                })
                .send()
                .await
                .map_err(|error| BackendError::Indexer(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Indexer(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Indexer(error.message));
            }
            response
                .result
                .ok_or_else(|| BackendError::Indexer("eth_getLogs returned no result".to_string()))
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct JsonRpcRequest<P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: P,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EthCallParams {
    from: String,
    to: String,
    data: String,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    gas: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EstimateGasParams {
    from: String,
    to: String,
    data: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Clone, Debug, Deserialize)]
struct JsonRpcError {
    message: String,
    data: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EthTransactionReceipt {
    transaction_hash: String,
    block_number: Option<String>,
    status: Option<String>,
    gas_used: Option<String>,
    effective_gas_price: Option<String>,
    cumulative_gas_used: Option<String>,
    block_hash: Option<String>,
    transaction_index: Option<String>,
}

impl TryFrom<EthTransactionReceipt> for ConfirmationReceipt {
    type Error = BackendError;

    fn try_from(value: EthTransactionReceipt) -> Result<Self> {
        validate_tx_hash(&value.transaction_hash)?;
        Ok(Self {
            tx_hash: value.transaction_hash.to_ascii_lowercase(),
            status: value
                .status
                .as_deref()
                .map(parse_hex_quantity_u64)
                .transpose()?,
            block_number: value
                .block_number
                .as_deref()
                .map(parse_hex_quantity_u64)
                .transpose()?,
            gas_used: value
                .gas_used
                .as_deref()
                .map(parse_hex_quantity_u64)
                .transpose()?,
            effective_gas_price: value.effective_gas_price,
            cumulative_gas_used: value
                .cumulative_gas_used
                .as_deref()
                .map(parse_hex_quantity_u64)
                .transpose()?,
            block_hash: value.block_hash,
            transaction_index: value
                .transaction_index
                .as_deref()
                .map(parse_hex_quantity_u64)
                .transpose()?,
        })
    }
}

fn hex_0x(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(2 + bytes.len() * 2);
    encoded.push_str("0x");
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_quantity_u128(value: u128) -> String {
    format!("0x{value:x}")
}

fn hex_quantity_u64(value: u64) -> String {
    format!("0x{value:x}")
}

fn parse_hex_quantity_u64(value: &str) -> Result<u64> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| BackendError::Simulation("invalid hex quantity".to_string()))?;
    u64::from_str_radix(hex, 16)
        .map_err(|error| BackendError::Simulation(format!("invalid hex quantity: {error}")))
}

fn parse_hex_quantity_u128(value: &str) -> Result<u128> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| BackendError::Simulation("invalid hex quantity".to_string()))?;
    u128::from_str_radix(hex, 16)
        .map_err(|error| BackendError::Simulation(format!("invalid hex quantity: {error}")))
}

fn decode_hex_bytes(value: &str) -> Result<Vec<u8>> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| BackendError::Simulation("invalid hex bytes".to_string()))?;
    if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::Simulation("invalid hex bytes".to_string()));
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for index in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[index..index + 2], 16)
            .map_err(|error| BackendError::Simulation(format!("invalid hex bytes: {error}")))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn validate_tx_hash(value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(BackendError::Simulation(
            "invalid transaction hash".to_string(),
        ));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::Simulation(
            "invalid transaction hash".to_string(),
        ));
    }
    Ok(())
}
