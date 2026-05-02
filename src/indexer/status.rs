use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IndexerStatus {
    #[serde(rename = "indexerEnabled")]
    pub indexer_enabled: bool,
    #[serde(rename = "rpcConfigured")]
    pub rpc_configured: bool,
    #[serde(rename = "persistenceRequired")]
    pub persistence_required: bool,
    #[serde(rename = "lastIndexedBlock")]
    pub last_indexed_block: u64,
    #[serde(rename = "targetContract")]
    pub target_contract: String,
}
