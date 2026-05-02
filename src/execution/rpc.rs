use crate::error::{BackendError, Result};
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EthCallSuccess {
    pub block_number: Option<u64>,
    pub output: Vec<u8>,
}

pub trait EthCallProvider: Clone + Send + Sync {
    fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess>;
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

impl EthCallProvider for HttpJsonRpcProvider {
    fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
        Box::pin(async move {
            let block_number = self.block_number().await.ok();
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
                return Err(BackendError::Simulation(error.message));
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

impl HttpJsonRpcProvider {
    async fn block_number(&self) -> Result<u64> {
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
}

#[derive(Clone, Debug, Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Clone, Debug, Deserialize)]
struct JsonRpcError {
    message: String,
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

fn parse_hex_quantity_u64(value: &str) -> Result<u64> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| BackendError::Simulation("invalid hex quantity".to_string()))?;
    u64::from_str_radix(hex, 16)
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
