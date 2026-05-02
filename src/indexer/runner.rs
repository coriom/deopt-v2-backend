use super::config::IndexerConfig;
use super::decoder::{
    decode_trade_executed_log, hex_quantity, parse_hex_quantity, trade_executed_topic0,
};
use super::events::EthLog;
use crate::db::PgRepository;
use crate::error::{BackendError, Result};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

pub const PERP_MATCHING_ENGINE_CURSOR: &str = "perp_matching_engine";

pub type RpcFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait IndexerRpcProvider: Clone + Send + Sync {
    fn block_number(&self) -> RpcFuture<'_, u64>;
    fn get_logs(&self, filter: EthGetLogsFilter) -> RpcFuture<'_, Vec<EthLog>>;
}

#[derive(Clone)]
pub struct HttpIndexerRpcProvider {
    rpc_url: String,
    client: reqwest::Client,
}

impl HttpIndexerRpcProvider {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

impl IndexerRpcProvider for HttpIndexerRpcProvider {
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
                .map_err(|error| BackendError::Indexer(error.to_string()))?
                .json()
                .await
                .map_err(|error| BackendError::Indexer(error.to_string()))?;
            if let Some(error) = response.error {
                return Err(BackendError::Indexer(error.message));
            }
            let result = response.result.ok_or_else(|| {
                BackendError::Indexer("eth_blockNumber returned no result".to_string())
            })?;
            parse_hex_quantity(&result)
        })
    }

    fn get_logs(&self, filter: EthGetLogsFilter) -> RpcFuture<'_, Vec<EthLog>> {
        Box::pin(async move {
            let response: JsonRpcResponse<Vec<EthLog>> = self
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EthGetLogsFilter {
    pub from_block: String,
    pub to_block: String,
    pub address: String,
    pub topics: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct JsonRpcRequest<P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: P,
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

#[derive(Clone)]
pub struct Indexer<P> {
    config: IndexerConfig,
    repository: PgRepository,
    provider: P,
}

impl Indexer<HttpIndexerRpcProvider> {
    pub fn from_config_and_repository(
        config: IndexerConfig,
        repository: PgRepository,
    ) -> Result<Self> {
        let rpc_url = config
            .rpc_url
            .clone()
            .ok_or_else(|| BackendError::Config("RPC_URL is required for indexer".to_string()))?;
        Ok(Self::new(
            config,
            repository,
            HttpIndexerRpcProvider::new(rpc_url),
        ))
    }
}

impl<P> Indexer<P>
where
    P: IndexerRpcProvider,
{
    pub fn new(config: IndexerConfig, repository: PgRepository, provider: P) -> Self {
        Self {
            config,
            repository,
            provider,
        }
    }

    pub async fn tick(&self) -> Result<IndexerTickResult> {
        if !self.config.enabled {
            return Err(BackendError::Config("indexer is disabled".to_string()));
        }

        let current_block = self.provider.block_number().await?;
        let last_indexed_block = self
            .repository
            .get_indexer_cursor(PERP_MATCHING_ENGINE_CURSOR)
            .await?
            .unwrap_or(self.config.start_block);
        let from_block = last_indexed_block.saturating_add(1);
        if from_block > current_block {
            return Ok(IndexerTickResult {
                from_block,
                to_block: last_indexed_block,
                logs_found: 0,
                events_indexed: 0,
                cursor_updated: false,
            });
        }
        let range_end = from_block
            .saturating_add(self.config.max_block_range)
            .saturating_sub(1);
        let to_block = current_block.min(range_end);

        let logs = self
            .provider
            .get_logs(EthGetLogsFilter {
                from_block: hex_quantity(from_block),
                to_block: hex_quantity(to_block),
                address: self.config.perp_matching_engine_address.0.clone(),
                topics: vec![trade_executed_topic0()],
            })
            .await?;
        let logs_found = logs.len();
        let trades = logs
            .iter()
            .map(decode_trade_executed_log)
            .collect::<Result<Vec<_>>>()?;
        let events_indexed = self
            .repository
            .persist_indexed_perp_trades_and_cursor(PERP_MATCHING_ENGINE_CURSOR, &trades, to_block)
            .await?;

        info!(
            from_block,
            to_block, logs_found, events_indexed, "indexer persisted TradeExecuted events"
        );

        Ok(IndexerTickResult {
            from_block,
            to_block,
            logs_found,
            events_indexed,
            cursor_updated: true,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct IndexerTickResult {
    #[serde(rename = "from_block")]
    pub from_block: u64,
    #[serde(rename = "to_block")]
    pub to_block: u64,
    #[serde(rename = "logs_found")]
    pub logs_found: usize,
    #[serde(rename = "events_indexed")]
    pub events_indexed: u64,
    #[serde(rename = "cursor_updated")]
    pub cursor_updated: bool,
}

pub fn spawn_indexer(
    indexer: Indexer<HttpIndexerRpcProvider>,
    poll_interval_ms: u64,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(error) = indexer.tick().await {
                error!(%error, "indexer tick failed");
            }
            sleep(Duration::from_millis(poll_interval_ms)).await;
        }
    })
}
