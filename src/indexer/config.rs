use crate::error::{BackendError, Result};
use crate::types::AccountId;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexerConfig {
    pub enabled: bool,
    pub start_block: u64,
    pub poll_interval_ms: u64,
    pub max_block_range: u64,
    pub require_persistence: bool,
    pub rpc_url: Option<String>,
    pub perp_matching_engine_address: AccountId,
}

impl IndexerConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            start_block: 0,
            poll_interval_ms: 3_000,
            max_block_range: 500,
            require_persistence: true,
            rpc_url: None,
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000000",
            ),
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.max_block_range == 0 {
            return Err(BackendError::Config(
                "INDEXER_MAX_BLOCK_RANGE must be greater than zero".to_string(),
            ));
        }
        if self.enabled && self.rpc_url.is_none() {
            return Err(BackendError::Config(
                "RPC_URL is required when INDEXER_ENABLED=true".to_string(),
            ));
        }
        if self.enabled && self.require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "indexer requires persistence enabled".to_string(),
            ));
        }
        Ok(())
    }

    pub fn status(&self, last_indexed_block: u64) -> IndexerConfigStatus {
        IndexerConfigStatus {
            indexer_enabled: self.enabled,
            rpc_configured: self.rpc_url.is_some(),
            persistence_required: self.require_persistence,
            last_indexed_block,
            target_contract: self.perp_matching_engine_address.0.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IndexerConfigStatus {
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
