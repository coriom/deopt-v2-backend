use crate::error::{BackendError, Result};
use crate::types::AccountId;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionConfig {
    pub execution_enabled: bool,
    pub dry_run: bool,
    pub poll_interval_ms: u64,
    pub max_batch_size: u32,
    pub simulation_enabled: bool,
    pub simulation_requires_persistence: bool,
    pub rpc_url: Option<String>,
    pub executor_from_address: AccountId,
    pub perp_matching_engine_address: AccountId,
    pub perp_engine_address: AccountId,
}

impl ExecutionConfig {
    pub fn disabled() -> Self {
        Self {
            execution_enabled: false,
            dry_run: true,
            poll_interval_ms: 1_000,
            max_batch_size: 10,
            simulation_enabled: false,
            simulation_requires_persistence: true,
            rpc_url: None,
            executor_from_address: AccountId::new("0x0000000000000000000000000000000000000000"),
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000000",
            ),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.execution_enabled {
            if !self.dry_run {
                return Err(BackendError::Config(
                    "real on-chain execution is not implemented yet; set EXECUTOR_DRY_RUN=true"
                        .to_string(),
                ));
            }
            if !persistence_enabled {
                return Err(BackendError::Config(
                    "executor requires persistence enabled".to_string(),
                ));
            }
        }
        if self.max_batch_size == 0 {
            return Err(BackendError::Config(
                "EXECUTOR_MAX_BATCH_SIZE must be greater than zero".to_string(),
            ));
        }
        if self.simulation_enabled && self.rpc_url.is_none() {
            return Err(BackendError::Config(
                "RPC_URL is required when SIMULATION_ENABLED=true".to_string(),
            ));
        }
        if self.simulation_enabled && self.simulation_requires_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "simulation requires persistence enabled".to_string(),
            ));
        }
        Ok(())
    }

    pub fn status(&self) -> ExecutionStatus {
        ExecutionStatus {
            execution_enabled: self.execution_enabled,
            dry_run: self.dry_run,
            real_broadcast_enabled: false,
            persistence_required: true,
            simulation_enabled: self.simulation_enabled,
            simulation_requires_persistence: self.simulation_requires_persistence,
            rpc_configured: self.rpc_url.is_some(),
            broadcast_enabled: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionStatus {
    #[serde(rename = "executionEnabled")]
    pub execution_enabled: bool,
    #[serde(rename = "dryRun")]
    pub dry_run: bool,
    #[serde(rename = "realBroadcastEnabled")]
    pub real_broadcast_enabled: bool,
    #[serde(rename = "persistenceRequired")]
    pub persistence_required: bool,
    #[serde(rename = "simulationEnabled")]
    pub simulation_enabled: bool,
    #[serde(rename = "simulationRequiresPersistence")]
    pub simulation_requires_persistence: bool,
    #[serde(rename = "rpcConfigured")]
    pub rpc_configured: bool,
    #[serde(rename = "broadcastEnabled")]
    pub broadcast_enabled: bool,
}
