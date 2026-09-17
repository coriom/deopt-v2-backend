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
    /// PERPS_BASE_SEPOLIA_CLOSED_TEST_RUNTIME_ARMING_AND_ACCOUNTING_V1
    /// — explicit closed-test one-intent arming gate. Default false.
    /// When false, the broadcast worker MUST NOT initiate a NEW real
    /// on-chain broadcast for any intent. When true, exactly one
    /// intent (`perps_closed_test_broadcast_intent_id`) is eligible;
    /// all others are refused. Independent of
    /// `broadcast_ready()` — infrastructure may be ready while the
    /// gate remains disarmed.
    pub perps_closed_test_broadcast_armed: bool,
    /// The exact UUID that is eligible when
    /// `perps_closed_test_broadcast_armed=true`. Any other intent id
    /// is rejected. `armed=true && intent_id=None` fails startup.
    pub perps_closed_test_broadcast_intent_id: Option<uuid::Uuid>,
    /// PERPS_BASE_SEPOLIA_CLOSED_TEST_RUNTIME_ARMING_AND_ACCOUNTING_V1
    /// — pre-send drift cap in bps applied against
    /// `PerpEngine.getMarkPrice(marketId)` immediately before signing
    /// the executor transaction. Default 100 bps. Bounded [1, 500].
    /// Fails closed on oracle error / mark==0 / drift > cap /
    /// deadline expired.
    pub perps_closed_test_max_drift_bps: u32,
    /// PERPS_BASE_SEPOLIA_CLOSED_TEST_RUNTIME_ARMING_AND_ACCOUNTING_V1
    /// — minimum remaining lifetime (seconds) between `now` and
    /// `intent.deadline` at pre-send time. Default 900 s. Bounded
    /// [60, 86_400].
    pub perps_closed_test_min_deadline_remaining_sec: u64,
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
            perps_closed_test_broadcast_armed: false,
            perps_closed_test_broadcast_intent_id: None,
            perps_closed_test_max_drift_bps: 100,
            perps_closed_test_min_deadline_remaining_sec: 900,
        }
    }

    /// Whether the closed-test one-intent gate accepts this UUID for a
    /// NEW broadcast. False if the gate is disarmed, if no UUID is
    /// configured, or if the id does not match. Case-sensitive Uuid
    /// equality (canonical hyphenated form).
    pub fn broadcast_armed_for(&self, intent_id: &uuid::Uuid) -> bool {
        self.perps_closed_test_broadcast_armed
            && self.perps_closed_test_broadcast_intent_id.as_ref() == Some(intent_id)
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.execution_enabled && !persistence_enabled {
            return Err(BackendError::Config(
                "executor requires persistence enabled".to_string(),
            ));
        }
        // PERPS_BASE_SEPOLIA_CLOSED_TEST_RUNTIME_ARMING_AND_ACCOUNTING_V1
        // Arming gate startup consistency: armed=true requires the
        // exact one-intent UUID to be configured. armed=false with a
        // configured UUID is allowed (operator staged the value ahead
        // of the arming toggle).
        if self.perps_closed_test_broadcast_armed
            && self.perps_closed_test_broadcast_intent_id.is_none()
        {
            return Err(BackendError::Config(
                "PERPS_CLOSED_TEST_BROADCAST_ARMED=true requires \
                 PERPS_CLOSED_TEST_BROADCAST_INTENT_ID=<uuid>"
                    .to_string(),
            ));
        }
        if !(1..=500).contains(&self.perps_closed_test_max_drift_bps) {
            return Err(BackendError::Config(format!(
                "PERPS_CLOSED_TEST_MAX_DRIFT_BPS={} out of bounds [1..=500]",
                self.perps_closed_test_max_drift_bps
            )));
        }
        if !(60..=86_400).contains(&self.perps_closed_test_min_deadline_remaining_sec) {
            return Err(BackendError::Config(format!(
                "PERPS_CLOSED_TEST_MIN_DEADLINE_REMAINING_SEC={} out of bounds [60..=86400]",
                self.perps_closed_test_min_deadline_remaining_sec
            )));
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

    // ================================================================
    // PERPS_BASE_SEPOLIA_CLOSED_TEST_RUNTIME_ARMING_AND_ACCOUNTING_V1
    // ================================================================

    fn armable_sepolia_base() -> ExecutionConfig {
        let mut cfg = sepolia_local_base();
        cfg.executor_allow_local_signer = true;
        cfg
    }

    #[test]
    fn arming_default_is_disarmed() {
        let cfg = ExecutionConfig::disabled();
        assert!(!cfg.perps_closed_test_broadcast_armed);
        assert!(cfg.perps_closed_test_broadcast_intent_id.is_none());
        assert!(!cfg.broadcast_armed_for(&uuid::Uuid::nil()));
    }

    #[test]
    fn arming_armed_without_intent_id_refuses_startup() {
        let mut cfg = armable_sepolia_base();
        cfg.perps_closed_test_broadcast_armed = true;
        cfg.perps_closed_test_broadcast_intent_id = None;
        let err = cfg
            .validate_startup(true)
            .expect_err("armed without intent id must refuse");
        assert!(err
            .to_string()
            .contains("PERPS_CLOSED_TEST_BROADCAST_ARMED=true requires"));
    }

    #[test]
    fn arming_armed_with_intent_id_accepts() {
        let mut cfg = armable_sepolia_base();
        cfg.perps_closed_test_broadcast_armed = true;
        let id = uuid::Uuid::from_u128(0xdead);
        cfg.perps_closed_test_broadcast_intent_id = Some(id);
        cfg.validate_startup(true)
            .expect("armed + intent id must start");
        assert!(cfg.broadcast_armed_for(&id));
    }

    #[test]
    fn arming_disarmed_ignores_configured_intent_id() {
        let mut cfg = armable_sepolia_base();
        let id = uuid::Uuid::from_u128(0xbeef);
        cfg.perps_closed_test_broadcast_intent_id = Some(id);
        // armed=false, id present: must start; broadcast_armed_for=false
        cfg.validate_startup(true)
            .expect("disarmed with configured id must start");
        assert!(!cfg.broadcast_armed_for(&id));
    }

    #[test]
    fn arming_armed_but_wrong_uuid_returns_false() {
        let mut cfg = armable_sepolia_base();
        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        cfg.perps_closed_test_broadcast_armed = true;
        cfg.perps_closed_test_broadcast_intent_id = Some(a);
        assert!(cfg.broadcast_armed_for(&a));
        assert!(!cfg.broadcast_armed_for(&b));
    }

    #[test]
    fn drift_cap_out_of_bounds_refuses_startup() {
        let mut cfg = armable_sepolia_base();
        cfg.perps_closed_test_max_drift_bps = 0;
        let err = cfg.validate_startup(true).expect_err("0 bps must refuse");
        assert!(err.to_string().contains("PERPS_CLOSED_TEST_MAX_DRIFT_BPS"));
        cfg.perps_closed_test_max_drift_bps = 501;
        let err = cfg.validate_startup(true).expect_err("501 bps must refuse");
        assert!(err.to_string().contains("PERPS_CLOSED_TEST_MAX_DRIFT_BPS"));
    }

    #[test]
    fn drift_cap_valid_bounds_accept() {
        let mut cfg = armable_sepolia_base();
        for bps in [1u32, 50, 100, 500] {
            cfg.perps_closed_test_max_drift_bps = bps;
            cfg.validate_startup(true)
                .unwrap_or_else(|e| panic!("bps={bps} rejected: {e}"));
        }
    }

    #[test]
    fn deadline_remaining_out_of_bounds_refuses_startup() {
        let mut cfg = armable_sepolia_base();
        cfg.perps_closed_test_min_deadline_remaining_sec = 59;
        let err = cfg.validate_startup(true).expect_err("59s must refuse");
        assert!(err
            .to_string()
            .contains("PERPS_CLOSED_TEST_MIN_DEADLINE_REMAINING_SEC"));
        cfg.perps_closed_test_min_deadline_remaining_sec = 86_401;
        let err = cfg.validate_startup(true).expect_err("86401s must refuse");
        assert!(err
            .to_string()
            .contains("PERPS_CLOSED_TEST_MIN_DEADLINE_REMAINING_SEC"));
    }

    #[test]
    fn deadline_remaining_valid_bounds_accept() {
        let mut cfg = armable_sepolia_base();
        for sec in [60u64, 900, 3600, 86_400] {
            cfg.perps_closed_test_min_deadline_remaining_sec = sec;
            cfg.validate_startup(true)
                .unwrap_or_else(|e| panic!("sec={sec} rejected: {e}"));
        }
    }
}
