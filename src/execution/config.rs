use crate::error::{BackendError, Result};
use crate::types::AccountId;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionConfig {
    pub execution_enabled: bool,
    pub dry_run: bool,
    pub poll_interval_ms: u64,
    pub max_batch_size: u32,
    pub rpc_url: Option<String>,
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
            rpc_url: None,
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000000",
            ),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if !self.execution_enabled {
            return Ok(());
        }
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
        if self.max_batch_size == 0 {
            return Err(BackendError::Config(
                "EXECUTOR_MAX_BATCH_SIZE must be greater than zero".to_string(),
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
}
