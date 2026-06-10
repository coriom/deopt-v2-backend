use crate::error::{BackendError, Result};
use crate::execution::remote_signer::{
    SignerBackendKind, ANVIL_CHAIN_ID, BASE_SEPOLIA_CHAIN_ID, MAINNET_CHAIN_ID,
};
use crate::execution::signer::ExecutorSigner;
use crate::types::AccountId;
use serde::Serialize;
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct PrivateKeySecret(String);

impl PrivateKeySecret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PrivateKeySecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionConfig {
    pub execution_enabled: bool,
    pub dry_run: bool,
    pub poll_interval_ms: u64,
    pub max_batch_size: u32,
    pub real_broadcast_enabled: bool,
    pub executor_private_key: Option<PrivateKeySecret>,
    pub executor_chain_id: u64,
    pub max_gas_limit: u64,
    pub max_fee_per_gas_wei: Option<String>,
    pub max_priority_fee_per_gas_wei: Option<String>,
    pub require_simulation_ok: bool,
    pub simulation_enabled: bool,
    pub simulation_requires_persistence: bool,
    pub rpc_url: Option<String>,
    pub executor_from_address: AccountId,
    pub perp_matching_engine_address: AccountId,
    pub perp_engine_address: AccountId,
    /// V2F-P: optional address of the OLD (stranded) PerpEngine.
    /// Used solely by the `deopt_perp_fee_charged_v2_total{consumer="old"}`
    /// observability metric and the related alert; never used to route
    /// broadcast traffic. `None` means "OLD address not configured" and
    /// any unmatched consumer is bucketed as `"unknown"`.
    pub old_perp_engine_address: Option<AccountId>,
    /// Selected signer backend. `LocalDev` wraps the in-process
    /// `ExecutorSigner::from_private_key`; `Remote` routes through the
    /// signer microservice mTLS client. Mainnet (`chain_id == 8453`)
    /// REFUSES `LocalDev` at startup per
    /// `MAINNET_BE_SIGNER_SERVICE_DESIGN.md §5.4`.
    pub backend_signer_mode: SignerBackendKind,
    /// Endpoint URL of the signer microservice. REQUIRED on mainnet.
    pub backend_signer_endpoint: Option<String>,
    /// Explicit operator opt-in to allow the `LocalDev` signer on
    /// testnets (`chain_id ∈ {84532, …}`). `chain_id == 31337` (anvil)
    /// is granted implicit local-dev privilege independent of this flag.
    pub executor_allow_local_signer: bool,
    /// Optional pluggable-provider kind used when
    /// `backend_signer_mode == Remote`. `None` means the
    /// production `RemoteSignerClient` continues to use its
    /// `UnimplementedTransport` default — fail-closed; no signing
    /// capability. A configured value names the vendor adapter the
    /// operator intends to wire in. `Mock` is REFUSED on mainnet
    /// (`chain_id == 8453`) per
    /// `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §3` and
    /// `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`.
    pub backend_signer_provider: Option<crate::execution::signer_adapters::SignerProviderKind>,
    /// Per-request timeout (milliseconds) applied by the remote
    /// signer adapter to vendor calls (AWS KMS / GCP KMS / Turnkey /
    /// etc.). Default 2500 ms per
    /// `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §2.10`.
    /// Range enforced at env load: 100..=30000. Adapter MUST treat a
    /// missing value as the default and MUST NOT retry sign requests
    /// on timeout (re-sign risks duplicate intent submission with a
    /// different nonce).
    pub backend_signer_timeout_ms: u32,
}

impl ExecutionConfig {
    pub fn disabled() -> Self {
        Self {
            execution_enabled: false,
            dry_run: true,
            poll_interval_ms: 1_000,
            max_batch_size: 10,
            real_broadcast_enabled: false,
            executor_private_key: None,
            executor_chain_id: 84532,
            max_gas_limit: 1_000_000,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            require_simulation_ok: true,
            simulation_enabled: false,
            simulation_requires_persistence: true,
            rpc_url: None,
            executor_from_address: AccountId::new("0x0000000000000000000000000000000000000000"),
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000000",
            ),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
            old_perp_engine_address: None,
            backend_signer_mode: SignerBackendKind::LocalDev,
            backend_signer_endpoint: None,
            executor_allow_local_signer: false,
            backend_signer_provider: None,
            backend_signer_timeout_ms: 2500,
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.execution_enabled && !persistence_enabled {
            return Err(BackendError::Config(
                "executor requires persistence enabled".to_string(),
            ));
        }
        if self.max_batch_size == 0 {
            return Err(BackendError::Config(
                "EXECUTOR_MAX_BATCH_SIZE must be greater than zero".to_string(),
            ));
        }
        if self.max_gas_limit == 0 {
            return Err(BackendError::Config(
                "EXECUTOR_MAX_GAS_LIMIT must be greater than zero".to_string(),
            ));
        }
        if self.executor_chain_id == 0 {
            return Err(BackendError::Config(
                "EXECUTOR_CHAIN_ID must be greater than zero".to_string(),
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
        if self.real_broadcast_enabled {
            if !persistence_enabled {
                return Err(BackendError::Config(
                    "real broadcast requires persistence enabled".to_string(),
                ));
            }
            // Mainnet hard refusal: env-keyed signing is forbidden by custody
            // policy §6 BE-5; the canonical check sits here so the process
            // refuses to start before any code path can dereference the key.
            if self.executor_chain_id == MAINNET_CHAIN_ID && self.executor_private_key.is_some() {
                return Err(BackendError::Config(
                    "EXECUTOR_PRIVATE_KEY must NOT be set on mainnet (chain_id=8453); use BACKEND_SIGNER_MODE=remote per MAINNET_BE_SIGNER_SERVICE_DESIGN §5.4".to_string(),
                ));
            }
            // Validate the signer backend choice for this chain id.
            self.validate_signer_backend()?;

            match self.backend_signer_mode {
                SignerBackendKind::LocalDev => {
                    let Some(private_key) = self.executor_private_key.as_ref() else {
                        return Err(BackendError::Config(
                            "EXECUTOR_PRIVATE_KEY is required when BACKEND_SIGNER_MODE=local_dev"
                                .to_string(),
                        ));
                    };
                    ExecutorSigner::from_private_key(private_key)?;
                }
                SignerBackendKind::Remote => {
                    if self
                        .backend_signer_endpoint
                        .as_deref()
                        .unwrap_or("")
                        .is_empty()
                    {
                        return Err(BackendError::Config(
                            "BACKEND_SIGNER_ENDPOINT is required when BACKEND_SIGNER_MODE=remote"
                                .to_string(),
                        ));
                    }
                }
            }

            if self.rpc_url.is_none() {
                return Err(BackendError::Config(
                    "RPC_URL is required when EXECUTOR_REAL_BROADCAST_ENABLED=true".to_string(),
                ));
            }
            if self.max_fee_per_gas_wei.is_none() || self.max_priority_fee_per_gas_wei.is_none() {
                return Err(BackendError::Config(
                    "EXECUTOR_MAX_FEE_PER_GAS_WEI and EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI are required when EXECUTOR_REAL_BROADCAST_ENABLED=true"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Enforce the signer-mode ↔ chain-id matrix from
    /// `MAINNET_BE_SIGNER_SERVICE_DESIGN.md §5.3 + §5.4`:
    ///
    /// | chain_id           | LocalDev                                                       | Remote                                  |
    /// |--------------------|----------------------------------------------------------------|-----------------------------------------|
    /// | 8453 (mainnet)     | REFUSED                                                        | required; endpoint MUST be set          |
    /// | 84532 (sepolia) etc| allowed only with `EXECUTOR_ALLOW_LOCAL_SIGNER=true`           | allowed; endpoint MUST be set if chosen |
    /// | 31337 (anvil)      | allowed unconditionally                                        | allowed; endpoint MUST be set if chosen |
    ///
    /// No fallback to LocalDev is granted in any code path — the broadcast
    /// site additionally refuses a `LocalDev` signer at runtime on chain id
    /// 8453 as defence-in-depth.
    pub fn validate_signer_backend(&self) -> Result<()> {
        match self.backend_signer_mode {
            SignerBackendKind::LocalDev => {
                if self.executor_chain_id == MAINNET_CHAIN_ID {
                    return Err(BackendError::Config(
                        "BACKEND_SIGNER_MODE=local_dev is REFUSED on mainnet (chain_id=8453); use BACKEND_SIGNER_MODE=remote".to_string(),
                    ));
                }
                let testnet_allowed =
                    self.executor_chain_id == ANVIL_CHAIN_ID || self.executor_allow_local_signer;
                if !testnet_allowed {
                    return Err(BackendError::Config(format!(
                        "BACKEND_SIGNER_MODE=local_dev on chain_id={} requires EXECUTOR_ALLOW_LOCAL_SIGNER=true (anvil chain_id={} is exempt)",
                        self.executor_chain_id, ANVIL_CHAIN_ID
                    )));
                }
                let _ = BASE_SEPOLIA_CHAIN_ID; // referenced by docs; suppress unused warning
            }
            SignerBackendKind::Remote => {
                if self
                    .backend_signer_endpoint
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
                {
                    return Err(BackendError::Config(
                        "BACKEND_SIGNER_ENDPOINT is required when BACKEND_SIGNER_MODE=remote"
                            .to_string(),
                    ));
                }
                // Mainnet hard-refuses the `Mock` pluggable provider —
                // defence-in-depth on top of the runtime guard in
                // `signer_adapters::PluggableRemoteSignerTransport`'s
                // health-check `healthy` flag.
                if self.executor_chain_id == MAINNET_CHAIN_ID
                    && matches!(
                        self.backend_signer_provider,
                        Some(crate::execution::signer_adapters::SignerProviderKind::Mock)
                    )
                {
                    return Err(BackendError::Config(
                        "BACKEND_REMOTE_SIGNER_PROVIDER=mock is REFUSED on mainnet (chain_id=8453); \
                         configure an operational vendor adapter".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn status(&self) -> ExecutionStatus {
        ExecutionStatus {
            execution_enabled: self.execution_enabled,
            dry_run: self.dry_run,
            real_broadcast_enabled: self.real_broadcast_enabled,
            persistence_required: true,
            simulation_enabled: self.simulation_enabled,
            simulation_requires_persistence: self.simulation_requires_persistence,
            rpc_configured: self.rpc_url.is_some(),
            broadcast_enabled: self.real_broadcast_enabled,
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

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    fn mainnet_remote_base() -> ExecutionConfig {
        ExecutionConfig {
            execution_enabled: true,
            dry_run: false,
            real_broadcast_enabled: true,
            executor_chain_id: MAINNET_CHAIN_ID,
            rpc_url: Some("https://example.invalid".to_string()),
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            backend_signer_mode: SignerBackendKind::Remote,
            backend_signer_endpoint: Some("https://signer.invalid".to_string()),
            ..ExecutionConfig::disabled()
        }
    }

    fn sepolia_local_base() -> ExecutionConfig {
        ExecutionConfig {
            execution_enabled: true,
            dry_run: false,
            real_broadcast_enabled: true,
            executor_chain_id: BASE_SEPOLIA_CHAIN_ID,
            executor_private_key: Some(PrivateKeySecret::new(TEST_KEY.to_string())),
            rpc_url: Some("https://example.invalid".to_string()),
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            backend_signer_mode: SignerBackendKind::LocalDev,
            executor_allow_local_signer: true,
            ..ExecutionConfig::disabled()
        }
    }

    #[test]
    fn mainnet_with_executor_private_key_refuses_startup() {
        let mut cfg = mainnet_remote_base();
        cfg.executor_private_key = Some(PrivateKeySecret::new(TEST_KEY.to_string()));
        let err = cfg
            .validate_startup(true)
            .expect_err("mainnet env-key must refuse startup");
        let msg = err.to_string();
        assert!(msg.contains("EXECUTOR_PRIVATE_KEY must NOT be set on mainnet"));
        // never echo the secret in the error string.
        assert!(!msg.contains("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"));
    }

    #[test]
    fn mainnet_with_local_signer_mode_refuses_startup() {
        let mut cfg = mainnet_remote_base();
        cfg.executor_private_key = None;
        cfg.backend_signer_mode = SignerBackendKind::LocalDev;
        let err = cfg
            .validate_startup(true)
            .expect_err("local-dev on mainnet must refuse");
        assert!(err.to_string().contains("local_dev is REFUSED on mainnet"));
    }

    #[test]
    fn mainnet_with_remote_signer_endpoint_allowed() {
        let cfg = mainnet_remote_base();
        cfg.validate_startup(true)
            .expect("mainnet remote-signer config must start");
    }

    #[test]
    fn mainnet_remote_mode_without_endpoint_refuses_startup() {
        let mut cfg = mainnet_remote_base();
        cfg.backend_signer_endpoint = None;
        let err = cfg
            .validate_startup(true)
            .expect_err("remote mode requires endpoint");
        assert!(err
            .to_string()
            .contains("BACKEND_SIGNER_ENDPOINT is required"));
    }

    #[test]
    fn sepolia_local_signer_without_allow_flag_refuses_startup() {
        let mut cfg = sepolia_local_base();
        cfg.executor_allow_local_signer = false;
        let err = cfg
            .validate_startup(true)
            .expect_err("sepolia local-signer without explicit allow must refuse");
        assert!(err.to_string().contains("EXECUTOR_ALLOW_LOCAL_SIGNER=true"));
    }

    #[test]
    fn sepolia_local_signer_with_allow_flag_allowed() {
        let cfg = sepolia_local_base();
        cfg.validate_startup(true)
            .expect("sepolia + allow + local signer must start");
    }

    #[test]
    fn anvil_local_signer_allowed_unconditionally() {
        let mut cfg = sepolia_local_base();
        cfg.executor_chain_id = ANVIL_CHAIN_ID;
        cfg.executor_allow_local_signer = false;
        cfg.validate_startup(true)
            .expect("anvil chain-id must allow local-signer by default");
    }

    #[test]
    fn mainnet_with_mock_pluggable_provider_refuses_startup() {
        let mut cfg = mainnet_remote_base();
        cfg.backend_signer_provider =
            Some(crate::execution::signer_adapters::SignerProviderKind::Mock);
        let err = cfg
            .validate_startup(true)
            .expect_err("mainnet must refuse mock provider");
        assert!(err
            .to_string()
            .contains("BACKEND_REMOTE_SIGNER_PROVIDER=mock is REFUSED on mainnet"));
    }

    #[test]
    fn mainnet_with_operational_provider_kind_passes() {
        let mut cfg = mainnet_remote_base();
        cfg.backend_signer_provider =
            Some(crate::execution::signer_adapters::SignerProviderKind::AwsKms);
        cfg.validate_startup(true)
            .expect("mainnet with operational provider must pass");
    }

    #[test]
    fn mainnet_without_provider_passes_but_runtime_remains_fail_closed() {
        // Default `backend_signer_provider = None` keeps
        // `RemoteSignerClient` on its `UnimplementedTransport`
        // default. Startup MAY pass (the operator may stage the
        // adapter wiring in a follow-on); runtime broadcast attempts
        // still fail with `SignerError::Transport(...)`.
        let cfg = mainnet_remote_base();
        assert!(cfg.backend_signer_provider.is_none());
        cfg.validate_startup(true)
            .expect("mainnet without provider must still pass startup; runtime is fail-closed");
    }

    #[test]
    fn sepolia_with_mock_provider_allowed() {
        let mut cfg = ExecutionConfig {
            execution_enabled: true,
            dry_run: false,
            real_broadcast_enabled: true,
            executor_chain_id: BASE_SEPOLIA_CHAIN_ID,
            rpc_url: Some("https://example.invalid".to_string()),
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            backend_signer_mode: SignerBackendKind::Remote,
            backend_signer_endpoint: Some("https://signer.invalid".to_string()),
            ..ExecutionConfig::disabled()
        };
        cfg.backend_signer_provider =
            Some(crate::execution::signer_adapters::SignerProviderKind::Mock);
        cfg.validate_startup(true)
            .expect("Sepolia + remote + mock allowed");
    }
}
