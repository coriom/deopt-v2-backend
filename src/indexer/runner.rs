use super::config::IndexerConfig;
use super::decoder::{
    classify_emitter, decode_trade_executed_log_for_emitter, hex_quantity, parse_hex_quantity,
    trade_executed_topic0,
};
use super::events::EthLog;
use crate::chain_runtime::ChainRuntimeHandle;
use crate::db::PgRepository;
use crate::error::{BackendError, Result};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

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

/// PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — `eth_getLogs` filter
/// where `address` may carry ONE or MANY emitter addresses. Every
/// mainstream Ethereum JSON-RPC implementation (geth, erigon,
/// nethermind, reth, anvil) accepts either a single 0x-hex address
/// string or a `Vec<String>` here. Serialized with the
/// `untagged` variant so a single-address filter keeps the same
/// wire shape as before this milestone (single V1 PME) and a
/// multi-address filter (V1 + V2) transparently uses the array
/// form.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum EthGetLogsAddress {
    Single(String),
    Many(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EthGetLogsFilter {
    pub from_block: String,
    pub to_block: String,
    pub address: EthGetLogsAddress,
    pub topics: Vec<String>,
}

impl EthGetLogsAddress {
    pub fn from_slice(addrs: &[String]) -> Self {
        match addrs.len() {
            0 | 1 => Self::Single(addrs.first().cloned().unwrap_or_default()),
            _ => Self::Many(addrs.to_vec()),
        }
    }
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
        // DEOPT_MULTICHAIN_SCHEMA_HARDENING_V1 — cursor lookup is
        // now `(chain_id, name)`-scoped. V1 has exactly one chain
        // runtime (Base Sepolia via `v1_default`).
        let chain_id = ChainRuntimeHandle::v1_default().chain_id();
        let last_indexed_block = self
            .repository
            .get_indexer_cursor(chain_id, PERP_MATCHING_ENGINE_CURSOR)
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

        // PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — filter by BOTH
        // the V1 PME emitter and the optional V2 PME emitter in a
        // single RPC call. Each log's `protocol_version` is derived
        // from its emitter address (via `classify_emitter`) — a
        // log whose address matches neither is a defense-in-depth
        // signal that the RPC returned an out-of-filter row; we
        // fail closed and skip it rather than silently defaulting.
        let addresses = self.config.perp_matching_engine_addresses();
        let v1_pme = self.config.perp_matching_engine_address.0.clone();
        let v2_pme_opt = self
            .config
            .perp_matching_engine_v2_address
            .as_ref()
            .map(|addr| addr.0.clone());

        let logs = self
            .provider
            .get_logs(EthGetLogsFilter {
                from_block: hex_quantity(from_block),
                to_block: hex_quantity(to_block),
                address: EthGetLogsAddress::from_slice(&addresses),
                topics: vec![trade_executed_topic0()],
            })
            .await?;
        let logs_found = logs.len();
        let mut trades = Vec::with_capacity(logs.len());
        for log in &logs {
            let Some(version) =
                classify_emitter(&log.address, &v1_pme, v2_pme_opt.as_deref())
            else {
                // Emitter matches neither configured PME. Fail closed
                // for THIS log; do not misclassify. The next tick
                // will re-run and the RPC will either fix its filter
                // or the log will remain skipped — either way, no
                // cross-generation attribution.
                warn!(
                    emitter = %log.address,
                    tx_hash = ?log.transaction_hash,
                    log_index = ?log.log_index,
                    "indexer: TradeExecuted emitter matches neither V1 nor V2 PME — skipping"
                );
                continue;
            };
            trades.push(decode_trade_executed_log_for_emitter(log, version)?);
        }
        let events_indexed = self
            .repository
            .persist_indexed_perp_trades_and_cursor(
                chain_id,
                PERP_MATCHING_ENGINE_CURSOR,
                &trades,
                to_block,
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AccountId;
    use serde_json::json;

    // PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — the multi-address
    // JSON-RPC filter must serialize to a single string when only
    // one address is present (preserving the pre-milestone wire
    // shape) and to an array when both are configured.
    #[test]
    fn eth_get_logs_address_serializes_single_as_string() {
        let addr = EthGetLogsAddress::Single(
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        );
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(
            v,
            json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            "single-address filter must serialize as a bare string \
             for wire compatibility with pre-multi-emitter deployments"
        );
    }

    #[test]
    fn eth_get_logs_address_serializes_many_as_array() {
        let addr = EthGetLogsAddress::Many(vec![
            "0xaaaa".to_string(),
            "0xbbbb".to_string(),
        ]);
        let v = serde_json::to_value(&addr).unwrap();
        assert_eq!(v, json!(["0xaaaa", "0xbbbb"]));
    }

    #[test]
    fn from_slice_picks_single_variant_when_len_le_1() {
        assert!(matches!(
            EthGetLogsAddress::from_slice(&[]),
            EthGetLogsAddress::Single(_)
        ));
        assert!(matches!(
            EthGetLogsAddress::from_slice(&["0xaa".to_string()]),
            EthGetLogsAddress::Single(_)
        ));
        assert!(matches!(
            EthGetLogsAddress::from_slice(&["0xaa".to_string(), "0xbb".to_string()]),
            EthGetLogsAddress::Many(_)
        ));
    }

    // PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — the address list the
    // indexer will feed into `eth_getLogs` must include both V1 and
    // V2 emitters when V2 is configured, and MUST NOT include the
    // V2 address when unconfigured (V1-only deployments).
    #[test]
    fn perp_matching_engine_addresses_v1_only_yields_single() {
        let cfg = IndexerConfig {
            enabled: true,
            start_block: 0,
            poll_interval_ms: 3_000,
            max_block_range: 500,
            require_persistence: false,
            rpc_url: Some("http://mock".to_string()),
            perp_matching_engine_address: AccountId::new(
                "0xAAAABBBBAAAABBBBAAAABBBBAAAABBBBAAAABBBB".to_string(),
            ),
            perp_matching_engine_v2_address: None,
        };
        let addrs = cfg.perp_matching_engine_addresses();
        assert_eq!(
            addrs,
            vec!["0xaaaabbbbaaaabbbbaaaabbbbaaaabbbbaaaabbbb".to_string()]
        );
    }

    #[test]
    fn perp_matching_engine_addresses_v1_plus_v2_yields_both() {
        let cfg = IndexerConfig {
            enabled: true,
            start_block: 0,
            poll_interval_ms: 3_000,
            max_block_range: 500,
            require_persistence: false,
            rpc_url: Some("http://mock".to_string()),
            perp_matching_engine_address: AccountId::new(
                "0xAAAABBBBAAAABBBBAAAABBBBAAAABBBBAAAABBBB".to_string(),
            ),
            perp_matching_engine_v2_address: Some(AccountId::new(
                "0x1111111111111111111111111111111111111111".to_string(),
            )),
        };
        let addrs = cfg.perp_matching_engine_addresses();
        assert_eq!(addrs.len(), 2);
        assert!(addrs.contains(&"0xaaaabbbbaaaabbbbaaaabbbbaaaabbbbaaaabbbb".to_string()));
        assert!(addrs.contains(&"0x1111111111111111111111111111111111111111".to_string()));
    }

    // PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — an operator that
    // (mis)configures V1 == V2 must not produce a duplicate entry
    // in the filter list (some RPC providers reject duplicate
    // addresses; more importantly a duplicate would suggest a
    // misconfiguration that we silently masked). The multi-address
    // helper de-duplicates.
    #[test]
    fn perp_matching_engine_addresses_dedupes_v1_equals_v2() {
        let cfg = IndexerConfig {
            enabled: true,
            start_block: 0,
            poll_interval_ms: 3_000,
            max_block_range: 500,
            require_persistence: false,
            rpc_url: Some("http://mock".to_string()),
            perp_matching_engine_address: AccountId::new(
                "0xAAAA000000000000000000000000000000000000".to_string(),
            ),
            perp_matching_engine_v2_address: Some(AccountId::new(
                "0xaaaa000000000000000000000000000000000000".to_string(),
            )),
        };
        let addrs = cfg.perp_matching_engine_addresses();
        assert_eq!(addrs.len(), 1, "V1==V2 must not duplicate the filter list");
    }
}
