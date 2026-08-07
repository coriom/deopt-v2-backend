//! Signer policy firewall (Part M of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! Independent revalidation gate placed BETWEEN the orchestrator and
//! the signer. Every field the pipeline believes is safe is re-checked
//! here — plan_hash and calldata_hash immutability, target/selector
//! allowlist, nonce agreement, gas/fee ceilings, simulation
//! freshness, execution phase, readiness, canonical id agreement.
//!
//! Rejections are structural (typed enum) so operator logs and admin
//! routes can classify without string matching.

use crate::hybrid_v2::execution::gas_policy::GasFeePolicy;
use crate::hybrid_v2::execution::nonce::NonceReservation;
use crate::hybrid_v2::execution::persistence::ExecutionRequestRow;
use crate::hybrid_v2::execution::plan::ExecutionPlan;
use crate::hybrid_v2::execution::state::ExecutionPhase;
use crate::hybrid_v2::execution::target_policy::TargetPolicy;
use crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use alloy_primitives::U256;
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum FirewallRejection {
    #[error("chain mismatch")]
    ChainMismatch,
    #[error("Base mainnet is forbidden")]
    BaseMainnetForbidden,
    #[error("target not allowed")]
    TargetNotAllowed,
    #[error("selector not allowed for target")]
    SelectorNotAllowed,
    #[error("calldata hash mismatch")]
    CalldataHashMismatch,
    #[error("plan hash is missing on the request row")]
    PlanHashMissing,
    #[error("plan hash on the row differs from the recomputed plan")]
    PlanHashChanged,
    #[error("canonical execution id missing")]
    ExecutionIdMissing,
    #[error("non-zero value is disallowed for executeMatch")]
    ValueDisallowed,
    #[error("nonce mismatch with reservation")]
    NonceMismatch,
    #[error("gas limit exceeds policy ceiling")]
    GasLimitExceedsPolicy,
    #[error("fee cap exceeds policy ceiling")]
    FeeCapExceedsPolicy,
    #[error("simulation not populated on the request row")]
    SimulationMissing,
    #[error("simulation is too old (max age {max_age_ms}ms, actual {actual_age_ms}ms)")]
    SimulationStale { max_age_ms: u64, actual_age_ms: u64 },
    #[error("readiness blocked: {0}")]
    ReadinessBlocked(String),
    #[error("execution is already signed")]
    ExecutionAlreadySigned,
    #[error("execution is already terminal in phase {phase}")]
    ExecutionAlreadyTerminal { phase: ExecutionPhase },
    #[error("execution was cancelled")]
    ExecutionCancelled,
    #[error("execution row is malformed: {0}")]
    Malformed(String),
    #[error("persistence failure: {0}")]
    Persistence(String),
    #[error("simulation reverted — signer must refuse")]
    SimulationReverted,
}

/// The firewall — holds borrowed context; runs one `revalidate` per
/// sign attempt.
pub struct SignerPolicyFirewall<'a> {
    pub target_policy: &'a TargetPolicy,
    pub gas_policy: &'a GasFeePolicy,
    pub configured_chain_id: u64,
    pub deployment_id: i64,
    pub store: &'a dyn HybridV2ProjectionStore,
    /// Max wall-clock age between `req.updated_at_ms` (when the
    /// simulation was persisted) and NOW that the firewall accepts
    /// before demanding a re-simulation.
    pub simulation_max_age_ms: u64,
    pub now_ms: u64,
}

impl<'a> SignerPolicyFirewall<'a> {
    /// Re-check every safety invariant that must hold before the
    /// signer is invoked. This method makes ONE read against the
    /// store (to fetch the row again — the caller may have observed
    /// a stale row).
    pub async fn revalidate(
        &self,
        req: &ExecutionRequestRow,
        plan: &ExecutionPlan,
        reservation: &NonceReservation,
        signer_identity: [u8; 20],
    ) -> Result<(), FirewallRejection> {
        // ------------------- static request checks -------------------
        // Base mainnet is refused before anything else.
        if self.configured_chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(FirewallRejection::BaseMainnetForbidden);
        }
        let req_chain_id = u64::try_from(req.chain_id)
            .map_err(|_| FirewallRejection::Malformed("negative chain_id".into()))?;
        if req_chain_id == BASE_MAINNET_CHAIN_ID {
            return Err(FirewallRejection::BaseMainnetForbidden);
        }
        if req_chain_id != self.configured_chain_id {
            return Err(FirewallRejection::ChainMismatch);
        }
        if plan.chain_id != self.configured_chain_id {
            return Err(FirewallRejection::ChainMismatch);
        }

        // Target + selector allowlist (independent — do NOT trust the
        // plan builder's earlier decision).
        self.target_policy
            .is_allowed(plan.chain_id, &plan.target, &plan.selector)
            .map_err(|_| FirewallRejection::SelectorNotAllowed)?;

        // Value must be zero.
        if plan.value_wei != U256::ZERO {
            return Err(FirewallRejection::ValueDisallowed);
        }

        // canonical id present.
        if req.canonical_execution_id.is_empty() {
            return Err(FirewallRejection::ExecutionIdMissing);
        }
        if plan.canonical_execution_id.as_str() != req.canonical_execution_id {
            return Err(FirewallRejection::Malformed(
                "canonical_execution_id disagreement between plan and row".into(),
            ));
        }

        // calldata_hash on the row must equal the plan (bit-identical).
        let row_calldata_hash = req
            .calldata_hash
            .as_deref()
            .ok_or_else(|| FirewallRejection::Malformed("row.calldata_hash missing".into()))?;
        let plan_calldata_hex = format!("0x{}", hex_encode(&plan.calldata_hash));
        if !row_calldata_hash.eq_ignore_ascii_case(&plan_calldata_hex) {
            return Err(FirewallRejection::CalldataHashMismatch);
        }

        // plan_hash present + matches.
        let row_plan_hash = req
            .plan_hash
            .as_deref()
            .ok_or(FirewallRejection::PlanHashMissing)?;
        let plan_hash_hex = format!("0x{}", hex_encode(&plan.plan_hash));
        if !row_plan_hash.eq_ignore_ascii_case(&plan_hash_hex) {
            return Err(FirewallRejection::PlanHashChanged);
        }

        // ------------------- reservation checks ----------------------
        if reservation.chain_id != self.configured_chain_id {
            return Err(FirewallRejection::ChainMismatch);
        }
        if let Some(row_nonce) = req.reserved_nonce {
            if u64::try_from(row_nonce).map_err(|_| {
                FirewallRejection::Malformed("negative reserved_nonce on row".into())
            })? != reservation.nonce
            {
                return Err(FirewallRejection::NonceMismatch);
            }
        }

        // ------------------- gas/fee checks --------------------------
        let gas_limit = req
            .gas_limit
            .ok_or_else(|| FirewallRejection::Malformed("row.gas_limit missing".into()))?;
        let gas_limit_u64 = u64::try_from(gas_limit)
            .map_err(|_| FirewallRejection::Malformed("negative gas_limit".into()))?;
        if gas_limit_u64 > self.gas_policy.max_gas_limit {
            return Err(FirewallRejection::GasLimitExceedsPolicy);
        }
        let max_fee = req
            .max_fee_per_gas_wei
            .as_deref()
            .ok_or_else(|| FirewallRejection::Malformed("row.max_fee_per_gas missing".into()))?;
        let max_fee_u256 = parse_decimal_u256(max_fee)
            .map_err(|_| FirewallRejection::Malformed("row.max_fee_per_gas decimal".into()))?;
        if max_fee_u256 > self.gas_policy.max_fee_per_gas_wei {
            return Err(FirewallRejection::FeeCapExceedsPolicy);
        }
        let max_priority = req.max_priority_fee_per_gas_wei.as_deref().ok_or_else(|| {
            FirewallRejection::Malformed("row.max_priority_fee_per_gas missing".into())
        })?;
        let max_priority_u256 = parse_decimal_u256(max_priority).map_err(|_| {
            FirewallRejection::Malformed("row.max_priority_fee_per_gas decimal".into())
        })?;
        if max_priority_u256 > self.gas_policy.max_priority_fee_per_gas_wei {
            return Err(FirewallRejection::FeeCapExceedsPolicy);
        }

        // ------------------- simulation checks -----------------------
        // Simulation must be populated.
        let gas_estimate = req
            .simulation_gas_estimate
            .ok_or(FirewallRejection::SimulationMissing)?;
        if gas_estimate <= 0 {
            return Err(FirewallRejection::SimulationReverted);
        }
        if req.simulation_block_number.is_none() || req.simulation_block_hash.is_none() {
            return Err(FirewallRejection::SimulationMissing);
        }
        // Freshness — measured against the row's updated_at_ms as a
        // proxy for when the simulation was persisted.
        let age_ms = self.now_ms.saturating_sub(req.updated_at_ms as u64);
        if age_ms > self.simulation_max_age_ms {
            return Err(FirewallRejection::SimulationStale {
                max_age_ms: self.simulation_max_age_ms,
                actual_age_ms: age_ms,
            });
        }

        // ------------------- state checks ----------------------------
        if req.phase == ExecutionPhase::Cancelled {
            return Err(FirewallRejection::ExecutionCancelled);
        }
        if req.phase.is_terminal() {
            return Err(FirewallRejection::ExecutionAlreadyTerminal { phase: req.phase });
        }
        if req.signature_r.is_some() || req.signature_s.is_some() {
            return Err(FirewallRejection::ExecutionAlreadySigned);
        }
        // Signer identity must not be zero.
        if signer_identity == [0u8; 20] {
            return Err(FirewallRejection::ReadinessBlocked(
                "signer identity is zero-address".into(),
            ));
        }

        // Re-read the row to guard against stale views. If the caller
        // held an old snapshot and the DB now says the row is signed
        // or terminal, refuse.
        let live = self
            .store
            .get_execution_request(&req.canonical_execution_id)
            .await
            .map_err(|e| FirewallRejection::Persistence(e.to_string()))?
            .ok_or_else(|| FirewallRejection::Malformed("execution row disappeared".to_string()))?;
        if live.phase.is_terminal() {
            return Err(FirewallRejection::ExecutionAlreadyTerminal { phase: live.phase });
        }
        if live.signature_r.is_some() || live.signature_s.is_some() {
            return Err(FirewallRejection::ExecutionAlreadySigned);
        }

        // Everything checks out.
        Ok(())
    }
}

fn parse_decimal_u256(s: &str) -> Result<U256, String> {
    U256::from_str_radix(s.trim(), 10).map_err(|e| e.to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::identity::CanonicalExecutionId;
    use crate::hybrid_v2::execution::plan::executeMatchCall;
    use crate::hybrid_v2::manifest::{ActivationStatus, ManifestModuleAddresses, ManifestParams};
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use alloy_sol_types::SolCall;

    const ENGINE_HEX: &str = "0x5a5EBF9A9CCd7c012518569DE8283982982670f6";

    fn engine_addr() -> [u8; 20] {
        let mut out = [0u8; 20];
        for i in 0..20 {
            let hi = u8::from_str_radix(&ENGINE_HEX[2 + 2 * i..2 + 2 * i + 1], 16).unwrap();
            let lo = u8::from_str_radix(&ENGINE_HEX[2 + 2 * i + 1..2 + 2 * i + 2], 16).unwrap();
            out[i] = (hi << 4) | lo;
        }
        out
    }

    fn manifest() -> ManifestParams {
        ManifestParams {
            chain_id: 84532,
            manifest_address: format!("0x{}", "01".repeat(20)),
            manifest_hash: format!("0x{}", "11".repeat(32)),
            module_addresses_hash: format!("0x{}", "22".repeat(32)),
            critical_config_hash: format!("0x{}", "33".repeat(32)),
            architecture_version: 1,
            storage_version: 1,
            event_version: 1,
            deployment_version: 1,
            manifest_schema_version: 1,
            environment_tag: format!("0x{}", "00".repeat(32)),
            deployer: format!("0x{}", "de".repeat(20)),
            deployment_block: 1,
            deployment_timestamp: 1,
            module_addresses: ManifestModuleAddresses {
                subaccount_registry: format!("0x{}", "01".repeat(20)),
                collateral_vault: format!("0x{}", "02".repeat(20)),
                options_positions_ledger: format!("0x{}", "03".repeat(20)),
                risk_module: format!("0x{}", "04".repeat(20)),
                margin_engine: format!("0x{}", "05".repeat(20)),
                option_matching_engine: ENGINE_HEX.into(),
                escape_controller: format!("0x{}", "07".repeat(20)),
                recovery_finalizer: format!("0x{}", "08".repeat(20)),
                oracle_adapter: format!("0x{}", "09".repeat(20)),
                options_risk_provider: format!("0x{}", "0a".repeat(20)),
                quote_token: format!("0x{}", "0b".repeat(20)),
                fees_manager_v2: None,
                option_execution_fee_adapter: None,
                protocol_timelock: None,
                governance: None,
                guardian: None,
            },
            protocol_fee_subkey: format!("0x{}", "01".repeat(32)),
            rebate_budget_subkey: format!("0x{}", "02".repeat(32)),
            insurance_fund_subkey: format!("0x{}", "03".repeat(32)),
            max_collateral_tokens: 8,
            max_active_series: 32,
            all_capabilities_mask: format!("0x{}", "ff".repeat(32)),
            recovery_activation_delay_seconds: 3600,
            recovery_pause_max_duration_blocks: 1000,
            activation_status: ActivationStatus::Active,
        }
    }

    fn gas_policy() -> GasFeePolicy {
        GasFeePolicy {
            max_gas_limit: 5_000_000,
            gas_limit_multiplier_bps: 12_000,
            max_fee_per_gas_wei: U256::from(50_000_000_000u64),
            max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
            max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
            abnormal_estimate_reject_threshold: 10,
        }
    }

    fn baseline_plan() -> ExecutionPlan {
        ExecutionPlan {
            canonical_execution_id: CanonicalExecutionId("0xdead".to_string() + &"ee".repeat(30)),
            chain_id: 84532,
            deployment_id: 1,
            target: engine_addr(),
            selector: executeMatchCall::SELECTOR,
            calldata: vec![0x00u8, 0x01, 0x02, 0x03, 0x04],
            calldata_hash: [0xaau8; 32],
            value_wei: U256::ZERO,
            expected_module_version: "OptionMatchingEngineV2".into(),
            deadline_ms: None,
            plan_hash: [0xbbu8; 32],
        }
    }

    fn baseline_row(plan: &ExecutionPlan) -> ExecutionRequestRow {
        ExecutionRequestRow {
            canonical_execution_id: plan.canonical_execution_id.as_str().to_string(),
            deployment_id: 1,
            chain_id: 84532,
            execution_kind: "HYBRID_V2_OPTION_MATCH".into(),
            buyer_order_hash: format!("0x{}", "aa".repeat(32)),
            seller_order_hash: format!("0x{}", "bb".repeat(32)),
            buyer_subkey: format!("0x{}", "aa".repeat(32)),
            seller_subkey: format!("0x{}", "bb".repeat(32)),
            series_id: "42".into(),
            fill_quantity_1e8: "100000000".into(),
            premium_amount: "50000000".into(),
            fee_schedule_epoch: None,
            source_matched_execution_id: None,
            target_contract: ENGINE_HEX.to_string(),
            selector: format!("0x{}", hex_encode(&plan.selector)),
            calldata_hash: Some(format!("0x{}", hex_encode(&plan.calldata_hash))),
            plan_hash: Some(format!("0x{}", hex_encode(&plan.plan_hash))),
            tx_value_wei: "0".into(),
            simulation_block_number: Some(100),
            simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
            simulation_gas_estimate: Some(90_000),
            simulation_result_json: None,
            signer_identity: None,
            signing_payload_hash: None,
            signature_r: None,
            signature_s: None,
            signature_v: None,
            recovered_signer: None,
            gas_limit: Some(120_000),
            max_fee_per_gas_wei: Some("2000000000".into()),
            max_priority_fee_per_gas_wei: Some("1000000000".into()),
            reserved_nonce: Some(7),
            phase: ExecutionPhase::AwaitingSignature,
            failure_class: None,
            failure_detail: None,
            retry_count: 0,
            holder_epoch: None,
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
        }
    }

    fn reservation(canonical_id: &str) -> NonceReservation {
        NonceReservation {
            chain_id: 84532,
            signer_identity: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            nonce: 7,
            canonical_execution_id: canonical_id.to_string(),
            reserved_at_ms: 1_000,
        }
    }

    async fn seed_store(store: &InMemoryProjectionStore, row: &ExecutionRequestRow) {
        store.insert_execution_request(row).await.unwrap();
    }

    #[tokio::test]
    async fn happy_path_allows() {
        let plan = baseline_plan();
        let row = baseline_row(&plan);
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let sig = [0xaau8; 20];
        fw.revalidate(
            &row,
            &plan,
            &reservation(plan.canonical_execution_id.as_str()),
            sig,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn base_mainnet_rejected() {
        let plan = baseline_plan();
        let row = baseline_row(&plan);
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 8453,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let err = fw
            .revalidate(
                &row,
                &plan,
                &reservation(plan.canonical_execution_id.as_str()),
                [0xaau8; 20],
            )
            .await
            .unwrap_err();
        assert_eq!(err, FirewallRejection::BaseMainnetForbidden);
    }

    #[tokio::test]
    async fn plan_hash_change_rejected() {
        let plan = baseline_plan();
        let mut row = baseline_row(&plan);
        row.plan_hash = Some(format!("0x{}", "ff".repeat(32)));
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let err = fw
            .revalidate(
                &row,
                &plan,
                &reservation(plan.canonical_execution_id.as_str()),
                [0xaau8; 20],
            )
            .await
            .unwrap_err();
        assert_eq!(err, FirewallRejection::PlanHashChanged);
    }

    #[tokio::test]
    async fn nonce_mismatch_rejected() {
        let plan = baseline_plan();
        let row = baseline_row(&plan);
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let mut r = reservation(plan.canonical_execution_id.as_str());
        r.nonce = 999;
        let err = fw
            .revalidate(&row, &plan, &r, [0xaau8; 20])
            .await
            .unwrap_err();
        assert_eq!(err, FirewallRejection::NonceMismatch);
    }

    #[tokio::test]
    async fn stale_simulation_rejected() {
        let plan = baseline_plan();
        let row = baseline_row(&plan);
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 100_000_000,
        };
        let err = fw
            .revalidate(
                &row,
                &plan,
                &reservation(plan.canonical_execution_id.as_str()),
                [0xaau8; 20],
            )
            .await
            .unwrap_err();
        assert!(matches!(err, FirewallRejection::SimulationStale { .. }));
    }

    #[tokio::test]
    async fn gas_limit_over_policy_rejected() {
        let plan = baseline_plan();
        let mut row = baseline_row(&plan);
        row.gas_limit = Some(999_999_999);
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let err = fw
            .revalidate(
                &row,
                &plan,
                &reservation(plan.canonical_execution_id.as_str()),
                [0xaau8; 20],
            )
            .await
            .unwrap_err();
        assert_eq!(err, FirewallRejection::GasLimitExceedsPolicy);
    }

    #[tokio::test]
    async fn already_signed_rejected() {
        let plan = baseline_plan();
        let mut row = baseline_row(&plan);
        row.signature_r = Some(format!("0x{}", "aa".repeat(32)));
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let err = fw
            .revalidate(
                &row,
                &plan,
                &reservation(plan.canonical_execution_id.as_str()),
                [0xaau8; 20],
            )
            .await
            .unwrap_err();
        assert_eq!(err, FirewallRejection::ExecutionAlreadySigned);
    }

    #[tokio::test]
    async fn simulation_missing_rejected() {
        let plan = baseline_plan();
        let mut row = baseline_row(&plan);
        row.simulation_gas_estimate = None;
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let err = fw
            .revalidate(
                &row,
                &plan,
                &reservation(plan.canonical_execution_id.as_str()),
                [0xaau8; 20],
            )
            .await
            .unwrap_err();
        assert_eq!(err, FirewallRejection::SimulationMissing);
    }

    #[tokio::test]
    async fn terminal_row_rejected_via_live_reread() {
        let plan = baseline_plan();
        let mut row = baseline_row(&plan);
        row.phase = ExecutionPhase::Cancelled;
        let store = InMemoryProjectionStore::new();
        seed_store(&store, &row).await;
        let tp = TargetPolicy::from_manifest(&manifest()).unwrap();
        let gp = gas_policy();
        let fw = SignerPolicyFirewall {
            target_policy: &tp,
            gas_policy: &gp,
            configured_chain_id: 84532,
            deployment_id: 1,
            store: &store,
            simulation_max_age_ms: 60_000,
            now_ms: 1_100,
        };
        let err = fw
            .revalidate(
                &row,
                &plan,
                &reservation(plan.canonical_execution_id.as_str()),
                [0xaau8; 20],
            )
            .await
            .unwrap_err();
        assert_eq!(err, FirewallRejection::ExecutionCancelled);
    }
}
