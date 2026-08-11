//! Broadcast policy firewall (Part H of
//! `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! Placed BETWEEN the outbox and `send_raw_transaction`. Every check
//! that must hold at send time is re-verified here from primary
//! sources (the persisted row, the plan, the envelope, the readiness
//! report) — DO NOT trust an earlier verdict. Rejections are structural
//! so operators can classify without string matching.
//!
//! Frozen safety:
//! - Refuses when broadcast config is disabled OR chain id not in the
//!   allowlist OR chain id == Base mainnet.
//! - Refuses when the recovered signer on file does not match the
//!   expected_signer wired at startup.
//! - Refuses when the local envelope hash does not equal the plan's
//!   derived tx hash.
//! - Refuses when target / selector are not on the frozen allowlist.
//! - Refuses when simulation is missing or older than `simulation_max_age_ms`.
//! - Refuses when gas / fee ceilings exceed the gas policy.
//! - Refuses when the underlying execution row is already
//!   submitted / confirmed / cancelled / reverted.

use crate::hybrid_v2::execution::broadcast_rpc::{BroadcastRpcError, ExecutionBroadcastRpcClient};
use crate::hybrid_v2::execution::broadcast_state::BroadcastPhase;
use crate::hybrid_v2::execution::gas_policy::GasFeePolicy;
use crate::hybrid_v2::execution::persistence::ExecutionRequestRow;
use crate::hybrid_v2::execution::plan::ExecutionPlan;
use crate::hybrid_v2::execution::signer::SignedTx;
use crate::hybrid_v2::execution::state::ExecutionPhase;
use crate::hybrid_v2::execution::target_policy::TargetPolicy;
use crate::hybrid_v2::execution::tx_serialization::SignedExecutionEnvelope;
use crate::hybrid_v2::manifest::BASE_MAINNET_CHAIN_ID;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::readiness::ReadinessReport;
use alloy_primitives::U256;
use thiserror::Error;

/// Structural rejections produced by [`BroadcastPolicyFirewall::revalidate_before_send`].
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum BroadcastFirewallRejection {
    #[error("broadcast config disabled")]
    BroadcastConfigDisabled,
    #[error("chain not allowed: {chain_id}")]
    ChainNotAllowed { chain_id: u64 },
    #[error("Base mainnet forbidden")]
    BaseMainnetForbidden,
    #[error("execution phase not broadcastable ({0})")]
    ExecutionPhaseNotBroadcastable(String),
    #[error("plan mismatch")]
    PlanMismatch,
    #[error("signature not verified on execution row")]
    SignatureNotVerified,
    #[error("expected signer mismatch")]
    ExpectedSignerMismatch,
    #[error("local tx hash mismatch")]
    LocalTxHashMismatch,
    #[error("target not allowed")]
    TargetNotAllowed,
    #[error("selector not allowed")]
    SelectorNotAllowed,
    #[error("nonce mismatch")]
    NonceMismatch,
    #[error("simulation missing")]
    SimulationMissing,
    #[error("simulation stale (age {age_ms}ms)")]
    SimulationStale { age_ms: u64 },
    #[error("gas exceeds policy")]
    GasExceedsPolicy,
    #[error("fee exceeds policy")]
    FeeExceedsPolicy,
    #[error("readiness blocked: {0}")]
    ReadinessBlocked(String),
    #[error("execution already submitted or confirmed")]
    ExecutionAlreadySubmittedOrConfirmed,
    #[error("execution already reverted")]
    ExecutionAlreadyReverted,
    #[error("tx hash already observed on chain")]
    TxHashAlreadyObservedOnChain,
    #[error("execution cancelled")]
    ExecutionCancelled,
    #[error("row malformed: {0}")]
    Malformed(String),
    #[error("persistence failure: {0}")]
    Persistence(String),
    #[error("broadcast rpc failure: {0}")]
    RpcFailure(String),
}

/// Configuration values sourced from `HybridV2ExecutionConfig` — kept
/// as a small explicit struct so the firewall does not depend on the
/// full config type (helps callers construct focused test fixtures).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BroadcastFirewallConfig {
    pub broadcast_enabled: bool,
    /// Optional mempool probe before send.
    pub pre_send_hash_probe: bool,
}

/// Broadcast policy firewall. Holds borrowed references — construct
/// one per attempt.
pub struct BroadcastPolicyFirewall<'a> {
    pub store: &'a dyn HybridV2ProjectionStore,
    pub target_policy: &'a TargetPolicy,
    pub gas_policy: &'a GasFeePolicy,
    pub broadcast_config: BroadcastFirewallConfig,
    pub configured_chain_id: u64,
    pub deployment_id: i64,
    pub simulation_max_age_ms: u64,
    pub allowed_broadcast_chain_ids: &'a [u64],
    pub now_ms: u64,
    /// Optional broadcast RPC — required when
    /// `pre_send_hash_probe = true`, otherwise unused.
    pub rpc: Option<&'a dyn ExecutionBroadcastRpcClient>,
}

impl BroadcastPolicyFirewall<'_> {
    /// Revalidate every safety invariant that must hold before the raw
    /// tx is handed to the broadcast RPC. Returns `Ok(())` on pass,
    /// otherwise a structural rejection.
    pub async fn revalidate_before_send(
        &self,
        req: &ExecutionRequestRow,
        plan: &ExecutionPlan,
        signed: &SignedTx,
        envelope: &SignedExecutionEnvelope,
        expected_signer: [u8; 20],
        readiness: &ReadinessReport,
    ) -> Result<(), BroadcastFirewallRejection> {
        // --------------------- config gates ------------------------
        if !self.broadcast_config.broadcast_enabled {
            return Err(BroadcastFirewallRejection::BroadcastConfigDisabled);
        }
        if self.configured_chain_id == BASE_MAINNET_CHAIN_ID
            || plan.chain_id == BASE_MAINNET_CHAIN_ID
            || envelope.chain_id == BASE_MAINNET_CHAIN_ID
        {
            return Err(BroadcastFirewallRejection::BaseMainnetForbidden);
        }
        if !self
            .allowed_broadcast_chain_ids
            .contains(&self.configured_chain_id)
        {
            return Err(BroadcastFirewallRejection::ChainNotAllowed {
                chain_id: self.configured_chain_id,
            });
        }
        let req_chain = u64::try_from(req.chain_id)
            .map_err(|_| BroadcastFirewallRejection::Malformed("negative chain_id".into()))?;
        if req_chain != self.configured_chain_id
            || plan.chain_id != self.configured_chain_id
            || envelope.chain_id != self.configured_chain_id
        {
            return Err(BroadcastFirewallRejection::ChainNotAllowed {
                chain_id: req_chain,
            });
        }

        // --------------------- readiness ----------------------------
        if !readiness.is_ready() {
            let mut reason = String::new();
            if !readiness.runtime.ready {
                reason.push_str("runtime ");
            }
            if !readiness.rebuild.ready {
                reason.push_str("rebuild ");
            }
            if !readiness.reconciliation.ready {
                reason.push_str("reconciliation ");
            }
            return Err(BroadcastFirewallRejection::ReadinessBlocked(
                reason.trim_end().to_string(),
            ));
        }

        // --------------------- execution phase ----------------------
        // Broadcast expects the execution row to be at SignatureVerified
        // or ReadyForBroadcast; anything else is a signal that the
        // caller raced or that the row is already elsewhere.
        match req.phase {
            ExecutionPhase::SignatureVerified | ExecutionPhase::ReadyForBroadcast => {}
            ExecutionPhase::Cancelled => {
                return Err(BroadcastFirewallRejection::ExecutionCancelled);
            }
            other => {
                return Err(BroadcastFirewallRejection::ExecutionPhaseNotBroadcastable(
                    other.to_string(),
                ));
            }
        }

        // --------------------- broadcast phase gate -----------------
        // If a broadcast row exists it must not be at a phase past
        // Ready — otherwise the outbox is racing itself.
        if let Some(state) = self
            .store
            .get_broadcast_state(&req.canonical_execution_id)
            .await
            .map_err(|e| BroadcastFirewallRejection::Persistence(e.to_string()))?
        {
            match state.phase {
                BroadcastPhase::BroadcastDisabled | BroadcastPhase::ReadyForBroadcast => {}
                BroadcastPhase::Submitted
                | BroadcastPhase::Pending
                | BroadcastPhase::MinedSuccess
                | BroadcastPhase::Confirming
                | BroadcastPhase::Confirmed
                | BroadcastPhase::Broadcasting => {
                    return Err(BroadcastFirewallRejection::ExecutionAlreadySubmittedOrConfirmed);
                }
                BroadcastPhase::MinedReverted => {
                    return Err(BroadcastFirewallRejection::ExecutionAlreadyReverted);
                }
                BroadcastPhase::CancelledBeforeBroadcast => {
                    return Err(BroadcastFirewallRejection::ExecutionCancelled);
                }
                BroadcastPhase::ManualInterventionRequired
                | BroadcastPhase::Reorged
                | BroadcastPhase::SubmissionUnknown
                | BroadcastPhase::Dropped => {
                    return Err(BroadcastFirewallRejection::ExecutionPhaseNotBroadcastable(
                        state.phase.to_string(),
                    ));
                }
            }
            // Immutable-hash check: if a tx_hash was persisted earlier,
            // it must equal the current envelope hash. Otherwise the
            // outbox derived a different hash for the same execution.
            if let Some(existing) = state.tx_hash.as_deref() {
                let env_hex = envelope.envelope_hash_hex();
                if !existing.eq_ignore_ascii_case(&env_hex) {
                    return Err(BroadcastFirewallRejection::LocalTxHashMismatch);
                }
            }
        }

        // --------------------- plan agreement -----------------------
        // Row target must match plan; plan target must match envelope;
        // plan value_wei == 0 (executeMatch is non-payable); envelope
        // agrees.
        if plan.value_wei != U256::ZERO || envelope.value_wei != U256::ZERO {
            return Err(BroadcastFirewallRejection::PlanMismatch);
        }
        if envelope.target != plan.target {
            return Err(BroadcastFirewallRejection::PlanMismatch);
        }
        let row_target = parse_address_hex(&req.target_contract)
            .map_err(|e| BroadcastFirewallRejection::Malformed(e))?;
        if row_target != plan.target {
            return Err(BroadcastFirewallRejection::TargetNotAllowed);
        }
        let row_plan_hash = req
            .plan_hash
            .as_deref()
            .ok_or(BroadcastFirewallRejection::PlanMismatch)?;
        let plan_hex = format!("0x{}", hex_encode(&plan.plan_hash));
        if !row_plan_hash.eq_ignore_ascii_case(&plan_hex) {
            return Err(BroadcastFirewallRejection::PlanMismatch);
        }

        // --------------------- target + selector --------------------
        self.target_policy
            .is_allowed(plan.chain_id, &plan.target, &plan.selector)
            .map_err(|e| match e {
                crate::hybrid_v2::execution::target_policy::PolicyError::UnknownTarget {
                    ..
                } => BroadcastFirewallRejection::TargetNotAllowed,
                crate::hybrid_v2::execution::target_policy::PolicyError::UnknownSelector {
                    ..
                } => BroadcastFirewallRejection::SelectorNotAllowed,
                crate::hybrid_v2::execution::target_policy::PolicyError::BaseMainnetForbidden(
                    _,
                ) => BroadcastFirewallRejection::BaseMainnetForbidden,
                other => BroadcastFirewallRejection::Malformed(other.to_string()),
            })?;

        // --------------------- signature surface --------------------
        let sig_r_hex = req
            .signature_r
            .as_deref()
            .ok_or(BroadcastFirewallRejection::SignatureNotVerified)?;
        let sig_s_hex = req
            .signature_s
            .as_deref()
            .ok_or(BroadcastFirewallRejection::SignatureNotVerified)?;
        let sig_v = req
            .signature_v
            .ok_or(BroadcastFirewallRejection::SignatureNotVerified)?;
        let expected_r = format!("0x{}", hex_encode(&signed.signature_r));
        let expected_s = format!("0x{}", hex_encode(&signed.signature_s));
        if !sig_r_hex.eq_ignore_ascii_case(&expected_r)
            || !sig_s_hex.eq_ignore_ascii_case(&expected_s)
            || sig_v as u8 != signed.signature_v
        {
            return Err(BroadcastFirewallRejection::SignatureNotVerified);
        }
        if signed.recovered_signer != expected_signer {
            return Err(BroadcastFirewallRejection::ExpectedSignerMismatch);
        }
        let recovered_hex = req
            .recovered_signer
            .as_deref()
            .ok_or(BroadcastFirewallRejection::ExpectedSignerMismatch)?;
        let expected_addr = format!("0x{}", hex_encode(&expected_signer));
        if !recovered_hex.eq_ignore_ascii_case(&expected_addr) {
            return Err(BroadcastFirewallRejection::ExpectedSignerMismatch);
        }

        // --------------------- gas / fee / nonce ----------------------
        let gas_limit = req
            .gas_limit
            .ok_or_else(|| BroadcastFirewallRejection::Malformed("gas_limit missing".into()))?;
        let gas_u64 = u64::try_from(gas_limit)
            .map_err(|_| BroadcastFirewallRejection::Malformed("negative gas_limit".into()))?;
        if gas_u64 > self.gas_policy.max_gas_limit {
            return Err(BroadcastFirewallRejection::GasExceedsPolicy);
        }
        let max_fee_str = req
            .max_fee_per_gas_wei
            .as_deref()
            .ok_or_else(|| BroadcastFirewallRejection::Malformed("max_fee missing".into()))?;
        let max_fee =
            parse_dec_u256(max_fee_str).map_err(|e| BroadcastFirewallRejection::Malformed(e))?;
        if max_fee > self.gas_policy.max_fee_per_gas_wei {
            return Err(BroadcastFirewallRejection::FeeExceedsPolicy);
        }
        let max_prio_str = req
            .max_priority_fee_per_gas_wei
            .as_deref()
            .ok_or_else(|| BroadcastFirewallRejection::Malformed("max_priority missing".into()))?;
        let max_prio =
            parse_dec_u256(max_prio_str).map_err(|e| BroadcastFirewallRejection::Malformed(e))?;
        if max_prio > self.gas_policy.max_priority_fee_per_gas_wei {
            return Err(BroadcastFirewallRejection::FeeExceedsPolicy);
        }
        let reserved_nonce = req
            .reserved_nonce
            .ok_or(BroadcastFirewallRejection::NonceMismatch)?;
        let reserved_u64 = u64::try_from(reserved_nonce)
            .map_err(|_| BroadcastFirewallRejection::Malformed("negative nonce".into()))?;
        if reserved_u64 != envelope.nonce {
            return Err(BroadcastFirewallRejection::NonceMismatch);
        }

        // --------------------- simulation ---------------------------
        if req.simulation_block_number.is_none()
            || req.simulation_block_hash.is_none()
            || req.simulation_gas_estimate.is_none()
        {
            return Err(BroadcastFirewallRejection::SimulationMissing);
        }
        let updated_at_u64 = req.updated_at_ms as u64;
        let age = self.now_ms.saturating_sub(updated_at_u64);
        if age > self.simulation_max_age_ms {
            return Err(BroadcastFirewallRejection::SimulationStale { age_ms: age });
        }

        // --------------------- optional pre-send probe --------------
        if self.broadcast_config.pre_send_hash_probe {
            let rpc = self.rpc.ok_or_else(|| {
                BroadcastFirewallRejection::RpcFailure(
                    "pre_send_hash_probe requires an rpc client".into(),
                )
            })?;
            match rpc.transaction_by_hash(envelope.envelope_hash).await {
                Ok(Some(_)) => {
                    return Err(BroadcastFirewallRejection::TxHashAlreadyObservedOnChain);
                }
                Ok(None) => {}
                Err(e) => {
                    // Ambiguous — do NOT proceed on an unreadable probe.
                    return match e {
                        BroadcastRpcError::Timeout
                        | BroadcastRpcError::Transport(_)
                        | BroadcastRpcError::Unavailable(_)
                        | BroadcastRpcError::RateLimited => {
                            Err(BroadcastFirewallRejection::RpcFailure(e.to_string()))
                        }
                        other => Err(BroadcastFirewallRejection::RpcFailure(other.to_string())),
                    };
                }
            }
        }

        Ok(())
    }
}

// -----------------------------------------------------------------
//                          HELPERS
// -----------------------------------------------------------------

fn parse_address_hex(s: &str) -> Result<[u8; 20], String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| "expected 0x-prefixed hex".to_string())?;
    if stripped.len() != 40 {
        return Err(format!("expected 40 hex chars, got {}", stripped.len()));
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&stripped[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

fn parse_dec_u256(s: &str) -> Result<U256, String> {
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
    use crate::hybrid_v2::execution::tx_serialization::serialize_signed_execution;
    use crate::hybrid_v2::manifest::{ActivationStatus, ManifestModuleAddresses, ManifestParams};
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use crate::hybrid_v2::readiness::{ReadinessReport, ReadinessState};
    use alloy_sol_types::SolCall;

    const ENGINE_HEX: &str = "0x5a5EBF9A9CCd7c012518569DE8283982982670f6";

    fn engine_bytes() -> [u8; 20] {
        parse_address_hex(ENGINE_HEX).unwrap()
    }

    fn make_manifest() -> ManifestParams {
        ManifestParams {
            chain_id: 84532,
            manifest_address: "0x000000000000000000000000000000000000d001".into(),
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

    fn make_plan(chain_id: u64) -> ExecutionPlan {
        let calldata = vec![0xde, 0xad, 0xbe, 0xef];
        let calldata_hash = {
            use sha3::{Digest, Keccak256};
            let mut h = [0u8; 32];
            let d = Keccak256::digest(&calldata);
            h.copy_from_slice(&d[..]);
            h
        };
        ExecutionPlan {
            canonical_execution_id: CanonicalExecutionId(format!("0x{}", "aa".repeat(32))),
            chain_id,
            deployment_id: 1,
            target: engine_bytes(),
            selector: executeMatchCall::SELECTOR,
            calldata,
            calldata_hash,
            value_wei: U256::ZERO,
            expected_module_version: "OptionMatchingEngineV2".into(),
            deadline_ms: None,
            plan_hash: [0xee; 32],
        }
    }

    fn make_row(chain_id: u64, plan: &ExecutionPlan, signer: [u8; 20]) -> ExecutionRequestRow {
        ExecutionRequestRow {
            canonical_execution_id: plan.canonical_execution_id.as_str().to_string(),
            deployment_id: 1,
            chain_id: chain_id as i64,
            execution_kind: "HYBRID_V2_OPTION_MATCH".to_string(),
            buyer_order_hash: format!("0x{}", "aa".repeat(32)),
            seller_order_hash: format!("0x{}", "bb".repeat(32)),
            buyer_subkey: format!("0x{}", "aa".repeat(32)),
            seller_subkey: format!("0x{}", "bb".repeat(32)),
            series_id: "42".to_string(),
            fill_quantity_1e8: "100000000".to_string(),
            premium_amount: "50000000".to_string(),
            fee_schedule_epoch: None,
            source_matched_execution_id: None,
            target_contract: format!("0x{}", hex_encode(&plan.target)),
            selector: format!("0x{}", hex_encode(&plan.selector)),
            calldata_hash: Some(format!("0x{}", hex_encode(&plan.calldata_hash))),
            plan_hash: Some(format!("0x{}", hex_encode(&plan.plan_hash))),
            tx_value_wei: "0".to_string(),
            simulation_block_number: Some(100),
            simulation_block_hash: Some(format!("0x{}", "cc".repeat(32))),
            simulation_gas_estimate: Some(500_000),
            simulation_result_json: Some(serde_json::json!({})),
            signer_identity: Some(format!("0x{}", hex_encode(&signer))),
            signing_payload_hash: Some(format!("0x{}", "ff".repeat(32))),
            signature_r: Some(format!("0x{}", "11".repeat(32))),
            signature_s: Some(format!("0x{}", "22".repeat(32))),
            signature_v: Some(0),
            recovered_signer: Some(format!("0x{}", hex_encode(&signer))),
            gas_limit: Some(1_000_000),
            max_fee_per_gas_wei: Some("2000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("500000000".to_string()),
            reserved_nonce: Some(42),
            phase: ExecutionPhase::SignatureVerified,
            failure_class: None,
            failure_detail: None,
            retry_count: 0,
            holder_epoch: None,
            signer_request_idempotency_key: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn make_signed(signer: [u8; 20]) -> SignedTx {
        SignedTx {
            signature_r: [0x11; 32],
            signature_s: [0x22; 32],
            signature_v: 0,
            recovered_signer: signer,
            tx_type: 2,
        }
    }

    fn make_readiness_ready() -> ReadinessReport {
        ReadinessReport {
            runtime: ReadinessState::ready(),
            rebuild: ReadinessState::ready(),
            reconciliation: ReadinessState::ready(),
        }
    }

    async fn seed(store: &InMemoryProjectionStore, row: &ExecutionRequestRow) {
        store.insert_execution_request(row).await.unwrap();
        store
            .insert_broadcast_state(&row.canonical_execution_id, 1)
            .await
            .unwrap();
    }

    fn gas_policy() -> GasFeePolicy {
        GasFeePolicy {
            max_gas_limit: 5_000_000,
            gas_limit_multiplier_bps: 12_000,
            max_fee_per_gas_wei: U256::from(10_000_000_000u64),
            max_priority_fee_per_gas_wei: U256::from(2_000_000_000u64),
            max_total_native_cost_wei: U256::from(10u64).pow(U256::from(18u64)),
            abnormal_estimate_reject_threshold: 10,
        }
    }

    fn firewall<'a>(
        store: &'a InMemoryProjectionStore,
        target_policy: &'a TargetPolicy,
        gas: &'a GasFeePolicy,
        cfg: BroadcastFirewallConfig,
        chain_id: u64,
        allowed: &'a [u64],
    ) -> BroadcastPolicyFirewall<'a> {
        BroadcastPolicyFirewall {
            store,
            target_policy,
            gas_policy: gas,
            broadcast_config: cfg,
            configured_chain_id: chain_id,
            deployment_id: 1,
            simulation_max_age_ms: 3_600_000,
            allowed_broadcast_chain_ids: allowed,
            now_ms: 1_000,
            rpc: None,
        }
    }

    #[tokio::test]
    async fn happy_path_passes() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let row = make_row(84532, &plan, signer);
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        };
        let allowed = vec![84532u64];
        let fw = firewall(&store, &target_policy, &gp, cfg, 84532, &allowed);
        let signed = make_signed(signer);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let readiness = make_readiness_ready();
        assert!(fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &readiness)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn rejects_when_broadcast_disabled() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let row = make_row(84532, &plan, signer);
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: false,
            pre_send_hash_probe: false,
        };
        let allowed = vec![84532u64];
        let fw = firewall(&store, &target_policy, &gp, cfg, 84532, &allowed);
        let signed = make_signed(signer);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let err = fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &make_readiness_ready())
            .await
            .unwrap_err();
        assert_eq!(err, BroadcastFirewallRejection::BroadcastConfigDisabled);
    }

    #[tokio::test]
    async fn rejects_base_mainnet_chain() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let row = make_row(84532, &plan, signer);
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        };
        let allowed = vec![BASE_MAINNET_CHAIN_ID];
        let fw = firewall(
            &store,
            &target_policy,
            &gp,
            cfg,
            BASE_MAINNET_CHAIN_ID,
            &allowed,
        );
        let signed = make_signed(signer);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let err = fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &make_readiness_ready())
            .await
            .unwrap_err();
        assert_eq!(err, BroadcastFirewallRejection::BaseMainnetForbidden);
    }

    #[tokio::test]
    async fn rejects_when_expected_signer_mismatch() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let row = make_row(84532, &plan, signer);
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        };
        let allowed = vec![84532u64];
        let fw = firewall(&store, &target_policy, &gp, cfg, 84532, &allowed);
        let mut signed = make_signed(signer);
        signed.recovered_signer = [0xAA; 20]; // different signer
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let err = fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &make_readiness_ready())
            .await
            .unwrap_err();
        assert_eq!(err, BroadcastFirewallRejection::ExpectedSignerMismatch);
    }

    #[tokio::test]
    async fn rejects_when_simulation_stale() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let mut row = make_row(84532, &plan, signer);
        row.updated_at_ms = 0;
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        };
        let allowed = vec![84532u64];
        let mut fw = firewall(&store, &target_policy, &gp, cfg, 84532, &allowed);
        fw.simulation_max_age_ms = 100;
        fw.now_ms = 1_000_000;
        let signed = make_signed(signer);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let err = fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &make_readiness_ready())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BroadcastFirewallRejection::SimulationStale { .. }
        ));
    }

    #[tokio::test]
    async fn rejects_gas_exceeds_policy() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let mut row = make_row(84532, &plan, signer);
        row.gas_limit = Some(9_999_999);
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        };
        let allowed = vec![84532u64];
        let fw = firewall(&store, &target_policy, &gp, cfg, 84532, &allowed);
        let signed = make_signed(signer);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            9_999_999,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let err = fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &make_readiness_ready())
            .await
            .unwrap_err();
        assert_eq!(err, BroadcastFirewallRejection::GasExceedsPolicy);
    }

    #[tokio::test]
    async fn rejects_execution_already_submitted() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let row = make_row(84532, &plan, signer);
        seed(&store, &row).await;
        // Force the broadcast row into Submitted.
        store
            .update_broadcast_phase(
                &row.canonical_execution_id,
                BroadcastPhase::BroadcastDisabled,
                BroadcastPhase::Broadcasting,
                2,
                Default::default(),
            )
            .await
            .unwrap();
        store
            .update_broadcast_phase(
                &row.canonical_execution_id,
                BroadcastPhase::Broadcasting,
                BroadcastPhase::Submitted,
                3,
                Default::default(),
            )
            .await
            .unwrap();
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        };
        let allowed = vec![84532u64];
        let fw = firewall(&store, &target_policy, &gp, cfg, 84532, &allowed);
        let signed = make_signed(signer);
        let env = serialize_signed_execution(
            &plan,
            &signed,
            42,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let err = fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &make_readiness_ready())
            .await
            .unwrap_err();
        assert_eq!(
            err,
            BroadcastFirewallRejection::ExecutionAlreadySubmittedOrConfirmed
        );
    }

    #[tokio::test]
    async fn rejects_nonce_mismatch() {
        let store = InMemoryProjectionStore::new();
        let manifest = make_manifest();
        let plan = make_plan(84532);
        let signer = [0x77u8; 20];
        let row = make_row(84532, &plan, signer);
        seed(&store, &row).await;
        let target_policy = TargetPolicy::from_manifest(&manifest).unwrap();
        let gp = gas_policy();
        let cfg = BroadcastFirewallConfig {
            broadcast_enabled: true,
            pre_send_hash_probe: false,
        };
        let allowed = vec![84532u64];
        let fw = firewall(&store, &target_policy, &gp, cfg, 84532, &allowed);
        let signed = make_signed(signer);
        // Envelope carries nonce 99 but row says 42.
        let env = serialize_signed_execution(
            &plan,
            &signed,
            99,
            1_000_000,
            U256::from(2_000_000_000u64),
            U256::from(500_000_000u64),
        )
        .unwrap();
        let err = fw
            .revalidate_before_send(&row, &plan, &signed, &env, signer, &make_readiness_ready())
            .await
            .unwrap_err();
        assert_eq!(err, BroadcastFirewallRejection::NonceMismatch);
    }
}
