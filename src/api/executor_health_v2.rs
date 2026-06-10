//! Executor health endpoint v2 — non-sensitive JSON summary of executor,
//! signer, policy gate, live-provider config, chain-state, economic, and
//! observability status for admin / frontend / operator consumers.
//!
//! Endpoint path: `GET /executor/health/v2`.
//!
//! ## Design contract
//!
//! * Read-only. Reads only in-memory snapshots (typed config +
//!   `BroadcastObservability::snapshot`). No RPC probes, no DB writes, no
//!   transactions. Always returns HTTP 200; partial data is reported as
//!   `null` plus an entry in `not_tracked_yet`.
//! * Secret-safe. Never serialises private keys, RPC URLs with embedded
//!   tokens, admin tokens, `DATABASE_URL`, API keys, webhook URLs, or
//!   provider credentials. Public contract addresses (OME / PFV / CV /
//!   FM_V2 / BE) MAY appear because they are already part of the
//!   public configuration surface and emitted by `/admin/config` and
//!   `/metrics`.
//! * Conservative status logic. `overall_status` ∈ {green, yellow, red}
//!   with a flat `reasons` list. Red conditions are pinned to
//!   custody-policy hard stops (mainnet local-signer attempt, R5 drift,
//!   OME paused, BE not executor, mainnet env-key sit). Yellow flags
//!   missing-but-required configuration and "no observations yet" gaps.

use crate::api::AppState;
use crate::execution::remote_signer::{SignerBackendKind, MAINNET_CHAIN_ID};
use crate::options::BroadcastObservabilitySnapshot;
use crate::types::{now_ms, AccountId};
use serde::Serialize;
use std::collections::BTreeMap;

/// Top-level executor health v2 response. All fields are non-sensitive
/// and safe for admin UI / frontend / operator consumption.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutorHealthV2Response {
    pub service: ServiceBlock,
    pub execution_flags: ExecutionFlagsBlock,
    pub signer: SignerBlock,
    pub policy_gate: PolicyGateBlock,
    pub live_provider_config: LiveProviderConfigBlock,
    pub chain_state_last_seen: ChainStateLastSeenBlock,
    pub economics_last_seen: EconomicsLastSeenBlock,
    pub r5: R5Block,
    pub recent_policy_decisions: RecentPolicyDecisionsBlock,
    pub recent_signer_events: RecentSignerEventsBlock,
    pub observability: ObservabilityBlock,
    pub warnings: Vec<String>,
    pub hard_stops: Vec<String>,
    pub not_tracked_yet: Vec<String>,
    pub overall_status: HealthStatus,
    pub reasons: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Green,
    Yellow,
    Red,
}

impl HealthStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Red => "red",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ServiceBlock {
    pub name: &'static str,
    pub ok: bool,
    pub timestamp_ms: i64,
    pub network: String,
    pub chain_id: u64,
    pub persistence_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionFlagsBlock {
    pub execution_enabled: bool,
    pub real_broadcast_enabled: bool,
    pub option_broadcast_enabled: bool,
    pub simulation_enabled: bool,
    pub confirmation_worker_enabled: bool,
    pub nonce_sync_enabled: bool,
    pub policy_gate_enabled: bool,
    pub remote_signer_enabled: bool,
    pub local_signer_allowed: bool,
    pub executor_chain_id: u64,
    pub executor_from_address: Option<String>,
    pub option_matching_engine_address: Option<String>,
    pub perp_matching_engine_address: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SignerBlock {
    pub signer_mode: &'static str,
    pub remote_signer_configured: bool,
    pub signer_address: Option<String>,
    pub last_signer_kind: Option<String>,
    pub last_signer_success_at_ms: Option<i64>,
    pub last_signer_error_code: Option<String>,
    pub local_signer_on_mainnet_refused_total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PolicyGateBlock {
    pub approved_total: u64,
    pub rejected_total: u64,
    pub last_reject_code: Option<String>,
    pub last_reject_source_type: Option<String>,
    pub last_policy_data_failure_type: Option<String>,
    pub econ_data_available_last: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LiveProviderConfigBlock {
    pub protocol_fee_vault_configured: bool,
    pub fees_manager_v2_configured: bool,
    pub collateral_vault_configured: bool,
    pub protocol_fee_vault_address: Option<String>,
    pub fees_manager_v2_address: Option<String>,
    pub collateral_vault_address: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChainStateLastSeenBlock {
    pub be_balance_wei: Option<u128>,
    pub be_balance_floor_wei: Option<u128>,
    pub ome_paused: Option<bool>,
    pub ome_is_executor: Option<bool>,
    pub pfv_fee_balance: Option<u128>,
    pub pfv_rebate_reserve: Option<u128>,
    pub cv_pfv_balance: Option<u128>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EconomicsLastSeenBlock {
    pub fm_v2_rebate_budget: Option<u128>,
    pub effective_maker_ppm: Option<i64>,
    pub effective_taker_ppm: Option<i64>,
    pub econ_data_available_true_total: u64,
    pub econ_data_available_false_total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct R5Block {
    pub drift_zero_last_seen: Option<bool>,
    pub drift_observed_total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecentPolicyDecisionsBlock {
    pub approved_by_source_type: BTreeMap<String, u64>,
    pub rejected_by_code_source_type: Vec<RejectedCount>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RejectedCount {
    pub code: String,
    pub source_type: String,
    pub count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecentSignerEventsBlock {
    pub attempted_by_kind: BTreeMap<String, u64>,
    pub success_by_kind: BTreeMap<String, u64>,
    pub denied_by_code_kind: Vec<DeniedCount>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeniedCount {
    pub code: String,
    pub signer_kind: String,
    pub count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ObservabilityBlock {
    pub fm_v2_decode_failures_total: u64,
    pub fm_v2_rpc_failures_total: u64,
    pub policy_data_failures_total: BTreeMap<String, u64>,
    pub last_dedupe_reason: Option<String>,
    pub last_broadcast_submitted_ms: Option<i64>,
}

/// Build the executor health v2 response from the current [`AppState`]
/// using only in-memory snapshots. Pure / synchronous; safe to call on
/// the request thread without an awaited I/O call.
pub fn build_executor_health_v2(state: &AppState) -> ExecutorHealthV2Response {
    let snap = state.broadcast_observability.snapshot();
    let exec = &state.execution_config;
    let opt_cfg = &state.options_config;
    let indexer_cfg = &state.option_event_indexer_config;

    let service = ServiceBlock {
        name: "deopt-v2-backend",
        ok: true,
        timestamp_ms: now_ms(),
        network: state.network_name.clone(),
        chain_id: state.chain_id,
        persistence_enabled: state.persistence_enabled,
    };

    let execution_flags = ExecutionFlagsBlock {
        execution_enabled: exec.execution_enabled,
        real_broadcast_enabled: exec.real_broadcast_enabled,
        option_broadcast_enabled: opt_cfg.execution_broadcast_enabled,
        simulation_enabled: exec.simulation_enabled,
        confirmation_worker_enabled: state.option_confirmation_config.enabled,
        nonce_sync_enabled: state.option_nonce_sync_config.enabled,
        policy_gate_enabled: opt_cfg.execution_broadcast_enabled,
        remote_signer_enabled: exec.backend_signer_mode == SignerBackendKind::Remote,
        local_signer_allowed: exec.executor_allow_local_signer,
        executor_chain_id: exec.executor_chain_id,
        executor_from_address: account_id_or_none(&exec.executor_from_address),
        option_matching_engine_address: account_id_or_none(&opt_cfg.matching_engine_address),
        perp_matching_engine_address: account_id_or_none(&exec.perp_matching_engine_address),
    };

    let signer = SignerBlock {
        signer_mode: exec.backend_signer_mode.as_str(),
        remote_signer_configured: exec
            .backend_signer_endpoint
            .as_deref()
            .map(|endpoint| !endpoint.is_empty())
            .unwrap_or(false),
        signer_address: account_id_or_none(&exec.executor_from_address),
        last_signer_kind: snap.last_signer_kind.clone(),
        last_signer_success_at_ms: snap.last_broadcast_submitted_ms,
        last_signer_error_code: snap.last_signer_error_code.clone(),
        local_signer_on_mainnet_refused_total: snap.local_signer_on_mainnet_refused_total,
    };

    let policy_gate = PolicyGateBlock {
        approved_total: snap.policy_approved_total.values().sum(),
        rejected_total: snap.policy_rejected_total.values().sum(),
        last_reject_code: snap.last_policy_reject_code.clone(),
        last_reject_source_type: snap.last_reject_source_type.clone(),
        last_policy_data_failure_type: snap.last_policy_data_failure_type.clone(),
        econ_data_available_last: snap.econ_data_available_last,
    };

    let live_provider_config = LiveProviderConfigBlock {
        protocol_fee_vault_configured: indexer_cfg.protocol_fee_vault_address.is_some(),
        fees_manager_v2_configured: indexer_cfg.fees_manager_v2_address.is_some(),
        collateral_vault_configured: !indexer_cfg.collateral_vault_address.0.is_empty(),
        protocol_fee_vault_address: indexer_cfg
            .protocol_fee_vault_address
            .as_ref()
            .and_then(account_id_or_none),
        fees_manager_v2_address: indexer_cfg
            .fees_manager_v2_address
            .as_ref()
            .and_then(account_id_or_none),
        collateral_vault_address: account_id_or_none(&indexer_cfg.collateral_vault_address),
    };

    let chain_state_last_seen = ChainStateLastSeenBlock {
        be_balance_wei: snap.last_be_balance_wei,
        be_balance_floor_wei: snap.last_be_balance_floor_wei,
        ome_paused: snap.last_ome_paused,
        ome_is_executor: snap.last_ome_is_executor,
        pfv_fee_balance: snap.last_pfv_fee_balance,
        pfv_rebate_reserve: snap.last_pfv_rebate_reserve,
        cv_pfv_balance: snap.last_cv_pfv_balance,
    };

    let economics_last_seen = EconomicsLastSeenBlock {
        fm_v2_rebate_budget: snap.last_fm_v2_rebate_budget,
        effective_maker_ppm: snap.last_effective_maker_ppm,
        effective_taker_ppm: snap.last_effective_taker_ppm,
        econ_data_available_true_total: snap.econ_data_available_true_total,
        econ_data_available_false_total: snap.econ_data_available_false_total,
    };

    let r5 = R5Block {
        drift_zero_last_seen: snap.last_r5_drift_zero,
        drift_observed_total: snap.r5_drift_observed_total,
    };

    let recent_policy_decisions = RecentPolicyDecisionsBlock {
        approved_by_source_type: snap.policy_approved_total.clone(),
        rejected_by_code_source_type: snap
            .policy_rejected_total
            .iter()
            .map(|((code, source_type), count)| RejectedCount {
                code: code.clone(),
                source_type: source_type.clone(),
                count: *count,
            })
            .collect(),
    };

    let recent_signer_events = RecentSignerEventsBlock {
        attempted_by_kind: snap.signer_attempted_total.clone(),
        success_by_kind: snap.signer_success_total.clone(),
        denied_by_code_kind: snap
            .signer_denied_total
            .iter()
            .map(|((code, signer_kind), count)| DeniedCount {
                code: code.clone(),
                signer_kind: signer_kind.clone(),
                count: *count,
            })
            .collect(),
    };

    let observability = ObservabilityBlock {
        fm_v2_decode_failures_total: snap.fm_v2_decode_failures_total,
        fm_v2_rpc_failures_total: snap.fm_v2_rpc_failures_total,
        policy_data_failures_total: snap.policy_data_failures_total.clone(),
        last_dedupe_reason: snap.last_dedupe_reason.clone(),
        last_broadcast_submitted_ms: snap.last_broadcast_submitted_ms,
    };

    // BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE closed the
    // last gap; the field lives on `chain_state_last_seen` (the prior
    // `execution_flags.be_balance_floor_wei` label was a docs typo).
    // Every documented health-endpoint field now reports live data.
    let not_tracked_yet: Vec<String> = Vec::new();

    let (overall_status, reasons, hard_stops, warnings) =
        compute_status(state, &snap, &live_provider_config);

    ExecutorHealthV2Response {
        service,
        execution_flags,
        signer,
        policy_gate,
        live_provider_config,
        chain_state_last_seen,
        economics_last_seen,
        r5,
        recent_policy_decisions,
        recent_signer_events,
        observability,
        warnings,
        hard_stops,
        not_tracked_yet,
        overall_status,
        reasons,
    }
}

/// Status logic. Conservative:
///   * Red is reserved for custody-policy hard stops + observed
///     chain-state failures (R5 drift, OME paused, BE not executor).
///   * Yellow flags missing-but-required configuration in real-broadcast
///     mode and "no observations yet" gaps when broadcast is enabled.
///   * Green otherwise.
///
/// Returns `(status, reasons, hard_stops, warnings)`. `hard_stops` carries
/// the subset of red conditions tied to chain-policy hard stops (so
/// operators can wire alerts on the dedicated array); `reasons` carries
/// the full human-readable explanation list across all status levels.
fn compute_status(
    state: &AppState,
    snap: &BroadcastObservabilitySnapshot,
    live: &LiveProviderConfigBlock,
) -> (HealthStatus, Vec<String>, Vec<String>, Vec<String>) {
    let exec = &state.execution_config;
    let mut hard_stops: Vec<String> = Vec::new();
    let mut reasons: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let is_mainnet = exec.executor_chain_id == MAINNET_CHAIN_ID;

    // ---- red: custody-policy hard stops --------------------------------
    if snap.local_signer_on_mainnet_refused_total > 0 {
        hard_stops.push(
            "local-dev signer was attempted on mainnet and refused — investigate config drift"
                .to_string(),
        );
    }
    if is_mainnet && exec.backend_signer_mode == SignerBackendKind::LocalDev {
        hard_stops.push(
            "BACKEND_SIGNER_MODE=local_dev on mainnet (chain_id=8453) — must be `remote`"
                .to_string(),
        );
    }
    if is_mainnet && exec.executor_private_key.is_some() {
        hard_stops.push(
            "EXECUTOR_PRIVATE_KEY is set on mainnet — must be unset; use BACKEND_SIGNER_MODE=remote"
                .to_string(),
        );
    }
    if is_mainnet
        && exec.real_broadcast_enabled
        && exec
            .backend_signer_endpoint
            .as_deref()
            .map(|endpoint| endpoint.is_empty())
            .unwrap_or(true)
    {
        hard_stops.push(
            "mainnet real-broadcast enabled but BACKEND_SIGNER_ENDPOINT is unset".to_string(),
        );
    }

    // ---- red: observed chain-state hard failures -----------------------
    if snap.last_ome_paused == Some(true) {
        hard_stops.push(
            "NEW_OME.paused() observed true on the most recent broadcast attempt".to_string(),
        );
    }
    if snap.last_ome_is_executor == Some(false) {
        hard_stops.push(
            "NEW_OME.isExecutor(BE) observed false on the most recent broadcast attempt"
                .to_string(),
        );
    }
    if snap.last_r5_drift_zero == Some(false) {
        hard_stops.push(
            "R5 invariant drift observed (CV(PFV,asset) != feeBalance + rebateReserve)".to_string(),
        );
    }

    // ---- yellow: missing-but-required configuration --------------------
    if exec.real_broadcast_enabled || state.options_config.execution_broadcast_enabled {
        if !live.protocol_fee_vault_configured {
            warnings.push(
                "PROTOCOL_FEE_VAULT_ADDRESS not configured — PFV reads will be skipped".to_string(),
            );
        }
        if !live.fees_manager_v2_configured {
            warnings.push(
                "FEES_MANAGER_V2 address not configured — quoteFees + rebateBudget reads will be skipped"
                    .to_string(),
            );
        }
        if !live.collateral_vault_configured {
            warnings.push(
                "COLLATERAL_VAULT address not configured — CV(PFV,asset) reads will be skipped"
                    .to_string(),
            );
        }
        if snap.last_be_balance_wei.is_none() {
            warnings.push(
                "no chain-state observations yet — first broadcast attempt has not populated the live-read snapshot"
                    .to_string(),
            );
        }
    }

    // ---- yellow: signer state ------------------------------------------
    if exec.backend_signer_mode == SignerBackendKind::Remote && !is_mainnet {
        warnings.push(
            "remote signer client uses the UnimplementedTransport placeholder — sign attempts will return `signer:transport`"
                .to_string(),
        );
    }
    if snap.fm_v2_rpc_failures_total > 0 {
        warnings.push(format!(
            "FeesManagerV2 RPC failures observed cumulatively: {} (latest gather_inputs reads)",
            snap.fm_v2_rpc_failures_total
        ));
    }
    if snap.fm_v2_decode_failures_total > 0 {
        warnings.push(format!(
            "FeesManagerV2 ABI decode failures observed cumulatively: {}",
            snap.fm_v2_decode_failures_total
        ));
    }

    let status = if !hard_stops.is_empty() {
        for stop in &hard_stops {
            reasons.push(format!("hard_stop: {stop}"));
        }
        for warn in &warnings {
            reasons.push(format!("warning: {warn}"));
        }
        HealthStatus::Red
    } else if !warnings.is_empty() {
        for warn in &warnings {
            reasons.push(format!("warning: {warn}"));
        }
        HealthStatus::Yellow
    } else {
        HealthStatus::Green
    };

    (status, reasons, hard_stops, warnings)
}

/// Return a normalised address string only when `id` is non-empty. Empty
/// `AccountId(String)` values (the disabled-config sentinel) collapse to
/// `None` so the JSON never carries a placeholder like
/// `0x0000…0000` that an operator could mistake for a configured value.
fn account_id_or_none(id: &AccountId) -> Option<String> {
    let trimmed = id.0.trim();
    if trimmed.is_empty() || is_zero_address(trimmed) {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

fn is_zero_address(value: &str) -> bool {
    let normalised = value
        .strip_prefix("0x")
        .unwrap_or(value)
        .to_ascii_lowercase();
    !normalised.is_empty() && normalised.chars().all(|c| c == '0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AppState;
    use crate::engine::EngineState;
    use crate::execution::config::{ExecutionConfig, PrivateKeySecret};
    use crate::execution::remote_signer::{BASE_SEPOLIA_CHAIN_ID, MAINNET_CHAIN_ID};
    use crate::options::OptionsConfig;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    fn base_state() -> AppState {
        AppState::new(EngineState::new(Vec::new()))
    }

    fn mainnet_remote_exec() -> ExecutionConfig {
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
            executor_from_address: AccountId::new("0x00000000000000000000000000000000000000be"),
            ..ExecutionConfig::disabled()
        }
    }

    #[test]
    fn green_when_disabled_and_no_observations() {
        let state = base_state();
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Green);
        assert!(response.hard_stops.is_empty());
        assert!(response.warnings.is_empty());
        assert_eq!(response.service.name, "deopt-v2-backend");
    }

    #[test]
    fn yellow_when_real_broadcast_enabled_but_pfv_unconfigured() {
        let mut state = base_state();
        state.execution_config = mainnet_remote_exec();
        state.execution_config.executor_chain_id = BASE_SEPOLIA_CHAIN_ID;
        state.execution_config.backend_signer_endpoint = Some("https://signer.invalid".to_string());
        state.execution_config.backend_signer_mode = SignerBackendKind::Remote;
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Yellow);
        assert!(response
            .warnings
            .iter()
            .any(|w| w.contains("PROTOCOL_FEE_VAULT_ADDRESS not configured")));
    }

    #[test]
    fn red_when_mainnet_local_dev_signer_seated() {
        let mut state = base_state();
        let mut cfg = mainnet_remote_exec();
        cfg.backend_signer_mode = SignerBackendKind::LocalDev;
        cfg.executor_private_key = Some(PrivateKeySecret::new(TEST_KEY.to_string()));
        state.execution_config = cfg;
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Red);
        assert!(response
            .hard_stops
            .iter()
            .any(|stop| stop.contains("BACKEND_SIGNER_MODE=local_dev on mainnet")));
        assert!(response
            .hard_stops
            .iter()
            .any(|stop| stop.contains("EXECUTOR_PRIVATE_KEY is set on mainnet")));
    }

    #[test]
    fn red_when_ome_paused_observed() {
        let state = base_state();
        state.broadcast_observability.record_inputs_snapshot(
            &crate::options::broadcast_policy_data::BroadcastPolicyInputs {
                ome_paused: Some(true),
                ..Default::default()
            },
        );
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Red);
        assert!(response
            .hard_stops
            .iter()
            .any(|stop| stop.contains("NEW_OME.paused() observed true")));
    }

    #[test]
    fn red_when_be_is_not_executor() {
        let state = base_state();
        state.broadcast_observability.record_inputs_snapshot(
            &crate::options::broadcast_policy_data::BroadcastPolicyInputs {
                ome_is_executor: Some(false),
                ..Default::default()
            },
        );
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Red);
        assert!(response
            .hard_stops
            .iter()
            .any(|stop| stop.contains("isExecutor(BE) observed false")));
    }

    #[test]
    fn red_when_r5_drift_observed() {
        let state = base_state();
        state.broadcast_observability.record_inputs_snapshot(
            &crate::options::broadcast_policy_data::BroadcastPolicyInputs {
                r5_drift_zero: Some(false),
                ..Default::default()
            },
        );
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Red);
        assert!(response
            .hard_stops
            .iter()
            .any(|stop| stop.contains("R5 invariant drift")));
    }

    #[test]
    fn red_when_local_signer_mainnet_refusal_observed() {
        let state = base_state();
        state
            .broadcast_observability
            .record_local_signer_on_mainnet_refused();
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Red);
        assert!(response
            .hard_stops
            .iter()
            .any(|stop| stop.contains("local-dev signer was attempted on mainnet")));
    }

    #[test]
    fn signer_block_reflects_remote_mode() {
        let mut state = base_state();
        state.execution_config = mainnet_remote_exec();
        let response = build_executor_health_v2(&state);
        assert_eq!(response.signer.signer_mode, "remote");
        assert!(response.signer.remote_signer_configured);
        assert_eq!(
            response.signer.signer_address.as_deref(),
            Some("0x00000000000000000000000000000000000000be")
        );
    }

    #[test]
    fn account_id_or_none_collapses_zero_address() {
        let zero = AccountId::new("0x0000000000000000000000000000000000000000");
        assert_eq!(account_id_or_none(&zero), None);
        let empty = AccountId::new("");
        assert_eq!(account_id_or_none(&empty), None);
        let real = AccountId::new("0xAbCd000000000000000000000000000000000001");
        assert_eq!(
            account_id_or_none(&real),
            Some("0xabcd000000000000000000000000000000000001".to_string())
        );
    }

    #[test]
    fn not_tracked_yet_is_empty_after_be_balance_floor_milestone() {
        // BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE closed the
        // final gap. Every documented health-endpoint field now
        // reports live data; `not_tracked_yet` is empty.
        let state = base_state();
        let response = build_executor_health_v2(&state);
        assert!(
            response.not_tracked_yet.is_empty(),
            "not_tracked_yet should be empty; got {:?}",
            response.not_tracked_yet
        );
    }

    #[test]
    fn health_endpoint_surfaces_effective_maker_taker_ppm_singletons() {
        let state = base_state();
        // Initial — None.
        assert_eq!(
            build_executor_health_v2(&state)
                .economics_last_seen
                .effective_maker_ppm,
            None
        );
        assert_eq!(
            build_executor_health_v2(&state)
                .economics_last_seen
                .effective_taker_ppm,
            None
        );
        state
            .broadcast_observability
            .record_effective_fee_ppm(50, 100);
        let response = build_executor_health_v2(&state);
        assert_eq!(response.economics_last_seen.effective_maker_ppm, Some(50));
        assert_eq!(response.economics_last_seen.effective_taker_ppm, Some(100));
    }

    #[test]
    fn health_endpoint_surfaces_be_balance_floor_wei_singleton() {
        let state = base_state();
        // Initial — None.
        assert_eq!(
            build_executor_health_v2(&state)
                .chain_state_last_seen
                .be_balance_floor_wei,
            None
        );
        state
            .broadcast_observability
            .record_be_balance_floor_wei(1_500_000_000_000_000);
        let response = build_executor_health_v2(&state);
        assert_eq!(
            response.chain_state_last_seen.be_balance_floor_wei,
            Some(1_500_000_000_000_000)
        );
    }

    #[test]
    fn health_endpoint_surfaces_zero_be_balance_floor_legitimately() {
        // Sepolia permissive path computes `fund_floor_wei = 0` — that
        // is a valid policy state, not a "fake zero" placeholder. The
        // health endpoint must report the 0 verbatim.
        let state = base_state();
        state.broadcast_observability.record_be_balance_floor_wei(0);
        let response = build_executor_health_v2(&state);
        assert_eq!(response.chain_state_last_seen.be_balance_floor_wei, Some(0));
    }

    #[test]
    fn health_endpoint_surfaces_negative_effective_ppm() {
        // Defence-in-depth pin: i64 representation correctly carries
        // negative effective ppm (rebate-discount profiles). The
        // policy gate already rejects these via the
        // `negative-effective-ppm` reject code on mainnet; the JSON
        // surface only reports — it does not gate.
        let state = base_state();
        state
            .broadcast_observability
            .record_effective_fee_ppm(-25, 30);
        let response = build_executor_health_v2(&state);
        assert_eq!(response.economics_last_seen.effective_maker_ppm, Some(-25));
        assert_eq!(response.economics_last_seen.effective_taker_ppm, Some(30));
    }

    #[test]
    fn health_endpoint_surfaces_last_policy_data_failure_type_singleton() {
        let state = base_state();
        assert_eq!(
            build_executor_health_v2(&state)
                .policy_gate
                .last_policy_data_failure_type,
            None
        );
        state.broadcast_observability.record_policy_data_failure(
            crate::options::broadcast_policy_data::read_type::PFV_REBATE_RESERVE,
        );
        assert_eq!(
            build_executor_health_v2(&state)
                .policy_gate
                .last_policy_data_failure_type
                .as_deref(),
            Some("pfv_rebate_reserve")
        );
        // most-recent overrides earlier
        state.broadcast_observability.record_policy_data_failure(
            crate::options::broadcast_policy_data::read_type::FM_V2_QUOTE_FEES_DECODE,
        );
        assert_eq!(
            build_executor_health_v2(&state)
                .policy_gate
                .last_policy_data_failure_type
                .as_deref(),
            Some("fm_v2_quote_fees_decode")
        );
    }

    #[test]
    fn health_endpoint_surfaces_last_reject_source_type_singleton() {
        let state = base_state();
        state.broadcast_observability.record_policy_rejected(
            "rebate-reserve",
            crate::options::types::OptionExecutionSourceType::OptionRfqFill,
        );
        let response = build_executor_health_v2(&state);
        assert_eq!(
            response.policy_gate.last_reject_source_type.as_deref(),
            Some("rfq")
        );
        assert_eq!(
            response.policy_gate.last_reject_code.as_deref(),
            Some("rebate-reserve")
        );
    }

    #[test]
    fn health_endpoint_surfaces_last_signer_error_code_singleton() {
        let state = base_state();
        state
            .broadcast_observability
            .record_signer_denied("kms-timeout", "remote");
        let response = build_executor_health_v2(&state);
        assert_eq!(
            response.signer.last_signer_error_code.as_deref(),
            Some("kms-timeout")
        );
    }

    #[test]
    fn health_endpoint_surfaces_local_mainnet_refusal_as_signer_error_code() {
        let state = base_state();
        state
            .broadcast_observability
            .record_local_signer_on_mainnet_refused();
        let response = build_executor_health_v2(&state);
        assert_eq!(
            response.signer.last_signer_error_code.as_deref(),
            Some(crate::options::broadcast_observability::LOCAL_MAINNET_REFUSED_CODE)
        );
    }

    #[test]
    fn health_endpoint_surfaces_econ_data_available_last_singleton() {
        let state = base_state();
        // Initially None.
        assert_eq!(
            build_executor_health_v2(&state)
                .policy_gate
                .econ_data_available_last,
            None
        );
        state
            .broadcast_observability
            .record_econ_data_available(true);
        assert_eq!(
            build_executor_health_v2(&state)
                .policy_gate
                .econ_data_available_last,
            Some(true)
        );
        state
            .broadcast_observability
            .record_econ_data_available(false);
        assert_eq!(
            build_executor_health_v2(&state)
                .policy_gate
                .econ_data_available_last,
            Some(false)
        );
    }

    #[test]
    fn last_signer_error_code_does_not_carry_endpoint_url_even_under_pathological_input() {
        // Defence-in-depth pin: even if a caller passed a URL-shaped
        // string into record_signer_denied (which they should not), the
        // sanitiser strips the URL-structural punctuation so the
        // singleton never carries a routable endpoint string. We assert
        // the *shape* (no `://`, no `/`, no `?`, no `=`, no `.`, length
        // cap) — not arbitrary substrings, since alpha-numeric tokens
        // separated by `-` are valid bounded labels.
        let state = base_state();
        state
            .broadcast_observability
            .record_signer_denied("https://signer.invalid/secret-path?token=abc", "remote");
        let response = build_executor_health_v2(&state);
        let code = response
            .signer
            .last_signer_error_code
            .expect("singleton populated");
        assert!(!code.contains("://"), "must not contain URL scheme");
        assert!(!code.contains('/'), "must not contain path separator");
        assert!(!code.contains('?'), "must not contain query separator");
        assert!(!code.contains('='), "must not contain k=v separator");
        assert!(!code.contains('.'), "must not contain dotted host segment");
        assert!(code.len() <= 48, "must be length-bounded");
    }

    #[test]
    fn pfv_fm_v2_cv_configured_booleans_track_indexer_config() {
        let mut state = base_state();
        state.option_event_indexer_config.protocol_fee_vault_address =
            Some(AccountId::new("0x00000000000000000000000000000000000000aa"));
        state.option_event_indexer_config.fees_manager_v2_address =
            Some(AccountId::new("0x00000000000000000000000000000000000000bb"));
        state.option_event_indexer_config.collateral_vault_address =
            AccountId::new("0x00000000000000000000000000000000000000cc");
        let response = build_executor_health_v2(&state);
        assert!(response.live_provider_config.protocol_fee_vault_configured);
        assert!(response.live_provider_config.fees_manager_v2_configured);
        assert!(response.live_provider_config.collateral_vault_configured);
        assert_eq!(
            response
                .live_provider_config
                .protocol_fee_vault_address
                .as_deref(),
            Some("0x00000000000000000000000000000000000000aa")
        );
    }

    #[test]
    fn response_serialises_without_panicking() {
        let state = base_state();
        let response = build_executor_health_v2(&state);
        let json = serde_json::to_string(&response).expect("serialises");
        assert!(json.contains("\"overall_status\":\"green\""));
        assert!(json.contains("\"hard_stops\":[]"));
        assert!(json.contains("\"not_tracked_yet\""));
    }

    #[test]
    fn response_redacts_secret_envelope_strings() {
        // Even when an obvious-secret-looking value sits in the typed
        // config (it should not — defence-in-depth), the response must
        // not surface it. The current schema only exposes addresses +
        // counters + booleans, so this test pins the contract by
        // serialising the response and asserting common secret tokens
        // never appear in the JSON.
        let mut state = base_state();
        state.execution_config = mainnet_remote_exec();
        // Stub a PFV address — a legitimately-public value.
        state.option_event_indexer_config.protocol_fee_vault_address =
            Some(AccountId::new("0x00000000000000000000000000000000000000aa"));
        let response = build_executor_health_v2(&state);
        let json = serde_json::to_string(&response).expect("serialises");
        // Common secret material that must never appear.
        assert!(!json.to_ascii_lowercase().contains("private_key"));
        assert!(!json.to_ascii_lowercase().contains("database_url"));
        assert!(!json.to_ascii_lowercase().contains("rpc_url"));
        assert!(!json.contains(TEST_KEY));
        assert!(!json.contains("https://signer.invalid"));
        assert!(!json.contains("https://example.invalid"));
        assert!(!json.to_ascii_lowercase().contains("admin_token"));
    }

    #[test]
    fn policy_gate_totals_sum_across_source_types() {
        let state = base_state();
        state.broadcast_observability.record_policy_approved(
            crate::options::types::OptionExecutionSourceType::OptionOrderbookFill,
        );
        state.broadcast_observability.record_policy_approved(
            crate::options::types::OptionExecutionSourceType::OptionRfqFill,
        );
        state.broadcast_observability.record_policy_rejected(
            "rebate-reserve",
            crate::options::types::OptionExecutionSourceType::OptionRfqFill,
        );
        let response = build_executor_health_v2(&state);
        assert_eq!(response.policy_gate.approved_total, 2);
        assert_eq!(response.policy_gate.rejected_total, 1);
        assert_eq!(
            response.policy_gate.last_reject_code.as_deref(),
            Some("rebate-reserve")
        );
    }

    #[test]
    fn options_broadcast_enabled_warns_when_pfv_missing_even_without_real_broadcast() {
        let mut state = base_state();
        // Enable option broadcast while leaving the live-provider
        // chain-state addresses unconfigured.
        state.options_config = OptionsConfig {
            enabled: true,
            execution_enabled: true,
            execution_broadcast_enabled: true,
            ..OptionsConfig::disabled()
        };
        let response = build_executor_health_v2(&state);
        assert_eq!(response.overall_status, HealthStatus::Yellow);
        assert!(response
            .warnings
            .iter()
            .any(|w| w.contains("PROTOCOL_FEE_VAULT_ADDRESS not configured")));
    }
}
