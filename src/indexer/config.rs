use crate::error::{BackendError, Result};
use crate::types::AccountId;
use serde::Serialize;

/// Indexer configuration. Note that `perp_matching_engine_address`
/// is the deployed V1 PerpMatchingEngine (`PME`) address. As of
/// PERPS_V2_BACKEND_RECONCILIATION_V1 §12–14, the indexer can
/// ALSO observe the V2 PME (`perp_matching_engine_v2_address`,
/// optional). Because V1 and V2 emit BYTE-IDENTICAL
/// `TradeExecuted` signatures (see §6/§23), the emitter address is
/// the sole generation boundary — a single indexer pass must
/// filter by BOTH addresses so both event streams are captured
/// and each incoming log's `protocol_version` is derived from its
/// emitter, never from a runtime config value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexerConfig {
    pub enabled: bool,
    pub start_block: u64,
    pub poll_interval_ms: u64,
    pub max_block_range: u64,
    pub require_persistence: bool,
    pub rpc_url: Option<String>,
    pub perp_matching_engine_address: AccountId,
    /// PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — optional V2 PME
    /// emitter. When populated the indexer filters
    /// `eth_getLogs.address` by BOTH the V1 and V2 emitters and
    /// tags each incoming log with the correct
    /// `PerpsProtocolVersion` derived from its `log.address`.
    pub perp_matching_engine_v2_address: Option<AccountId>,
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
            perp_matching_engine_v2_address: None,
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
            target_contract_v2: self
                .perp_matching_engine_v2_address
                .as_ref()
                .map(|addr| addr.0.clone()),
        }
    }

    /// PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — return the flat
    /// address list the indexer must filter `eth_getLogs` by. The
    /// V1 address is always present; the V2 address is included
    /// only when configured. Addresses are lower-cased for
    /// wire-format stability across RPC providers.
    pub fn perp_matching_engine_addresses(&self) -> Vec<String> {
        let mut out = vec![self.perp_matching_engine_address.0.to_ascii_lowercase()];
        if let Some(v2) = self.perp_matching_engine_v2_address.as_ref() {
            let v2_lower = v2.0.to_ascii_lowercase();
            if v2_lower != out[0] {
                out.push(v2_lower);
            }
        }
        out
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
    /// PERPS_V2_BACKEND_RECONCILIATION_V1 §12 — the optional V2
    /// PerpMatchingEngine emitter address the indexer is
    /// additionally filtering. `None` when the operator has not
    /// configured a V2 stack.
    #[serde(rename = "targetContractV2", skip_serializing_if = "Option::is_none")]
    pub target_contract_v2: Option<String>,
}
