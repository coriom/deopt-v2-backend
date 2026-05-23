use crate::error::{BackendError, Result};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionConfirmationConfig {
    pub enabled: bool,
    pub poll_interval_ms: u64,
    pub finality_blocks: u64,
    pub batch_size: u32,
    pub require_rpc: bool,
    pub rpc_url: Option<String>,
}

impl OptionConfirmationConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            poll_interval_ms: 15_000,
            finality_blocks: 3,
            batch_size: 25,
            require_rpc: true,
            rpc_url: None,
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.batch_size == 0 {
            return Err(BackendError::Config(
                "OPTION_CONFIRMATION_BATCH_SIZE must be greater than zero".to_string(),
            ));
        }
        if self.poll_interval_ms == 0 {
            return Err(BackendError::Config(
                "OPTION_CONFIRMATION_POLL_INTERVAL_MS must be greater than zero".to_string(),
            ));
        }
        if !persistence_enabled {
            return Err(BackendError::Config(
                "option confirmation worker requires persistence enabled".to_string(),
            ));
        }
        if self.require_rpc && self.rpc_url.is_none() {
            return Err(BackendError::Config(
                "RPC_URL is required when OPTION_CONFIRMATION_WORKER_ENABLED=true and OPTION_CONFIRMATION_REQUIRE_RPC=true".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionConfirmationOutcome {
    /// Worker is disabled — no work performed.
    Disabled,
    /// No pending option execution transactions found.
    NoPending,
    /// Receipt missing on-chain; left pending.
    ReceiptMissing,
    /// Receipt exists but finality threshold has not been reached yet.
    NotFinalized,
    /// Finalized as a successful, mined trade.
    MinedSuccess,
    /// Finalized as a failed, mined trade (status=0).
    MinedFailed,
    /// RPC error observed; left pending and recorded.
    ReceiptError,
}

impl OptionConfirmationOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NoPending => "no_pending",
            Self::ReceiptMissing => "receipt_missing",
            Self::NotFinalized => "not_finalized",
            Self::MinedSuccess => "mined_success",
            Self::MinedFailed => "mined_failed",
            Self::ReceiptError => "receipt_error",
        }
    }

    pub fn is_finalized(self) -> bool {
        matches!(self, Self::MinedSuccess | Self::MinedFailed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionConfirmationDecision {
    pub transaction_id: String,
    pub tx_hash: Option<String>,
    pub outcome: OptionConfirmationOutcome,
    pub receipt_status: Option<u64>,
    pub block_number: Option<u64>,
    pub current_block_number: Option<u64>,
    pub finality_blocks: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionConfirmationTickResult {
    pub enabled: bool,
    pub batch_size: u32,
    pub finality_blocks: u64,
    pub current_block_number: Option<u64>,
    pub decisions: Vec<OptionConfirmationDecision>,
}

/// Spawn the background option execution confirmation worker.
///
/// The worker polls `service::confirm_pending_option_execution_transactions` every
/// `poll_interval_ms`. It is a no-op when the config is disabled.
pub fn spawn_option_confirmation_worker(state: crate::api::AppState) {
    if !state.option_confirmation_config.enabled {
        tracing::info!("option confirmation worker disabled");
        return;
    }
    let poll_interval_ms = state.option_confirmation_config.poll_interval_ms;
    let rpc_url = match state.option_confirmation_config.rpc_url.clone() {
        Some(url) => url,
        None => {
            tracing::warn!("option confirmation worker enabled without RPC_URL; refusing to spawn");
            return;
        }
    };
    let provider = crate::execution::HttpJsonRpcProvider::new(rpc_url);
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(poll_interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match crate::options::service::confirm_pending_option_execution_transactions(
                &state, &provider,
            )
            .await
            {
                Ok(result) => {
                    if !result.decisions.is_empty() {
                        tracing::info!(
                            batch = result.decisions.len(),
                            current_block = ?result.current_block_number,
                            "option confirmation worker tick"
                        );
                    }
                }
                Err(error) => tracing::warn!(%error, "option confirmation worker tick failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_default_passes_validate() {
        OptionConfirmationConfig::disabled()
            .validate_startup(true)
            .unwrap();
    }

    #[test]
    fn enabled_requires_persistence() {
        let mut cfg = OptionConfirmationConfig::disabled();
        cfg.enabled = true;
        cfg.rpc_url = Some("https://example.invalid".to_string());
        let error = cfg.validate_startup(false).unwrap_err();
        assert!(error
            .to_string()
            .contains("option confirmation worker requires persistence"));
    }

    #[test]
    fn enabled_with_require_rpc_requires_rpc_url() {
        let mut cfg = OptionConfirmationConfig::disabled();
        cfg.enabled = true;
        cfg.require_rpc = true;
        let error = cfg.validate_startup(true).unwrap_err();
        assert!(error
            .to_string()
            .contains("OPTION_CONFIRMATION_WORKER_ENABLED=true"));
    }

    #[test]
    fn enabled_rejects_zero_batch_size() {
        let mut cfg = OptionConfirmationConfig::disabled();
        cfg.enabled = true;
        cfg.batch_size = 0;
        let error = cfg.validate_startup(true).unwrap_err();
        assert!(error.to_string().contains("OPTION_CONFIRMATION_BATCH_SIZE"));
    }

    #[test]
    fn enabled_rejects_zero_poll_interval() {
        let mut cfg = OptionConfirmationConfig::disabled();
        cfg.enabled = true;
        cfg.poll_interval_ms = 0;
        cfg.rpc_url = Some("https://example.invalid".to_string());
        let error = cfg.validate_startup(true).unwrap_err();
        assert!(error
            .to_string()
            .contains("OPTION_CONFIRMATION_POLL_INTERVAL_MS"));
    }
}
