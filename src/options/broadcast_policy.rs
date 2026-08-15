//! `should_broadcast` — pre-broadcast policy gate.
//!
//! Implements `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8` and the Cluster 4
//! launch invariant verifier (`MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3`).
//!
//! The gate is **pure**: no I/O, no clock read, no chain call. All inputs are
//! supplied through `BroadcastContext`. Side effects (status update, structured
//! log, persistence) live in the call site, not the policy.
//!
//! Closes gap-list C-4 / W-3; provides the launch-invariant verifier sweep
//! required by AUDIT-EXT Q-34.
//!
//! Hard rule: this module never signs, never broadcasts, never mutates chain.

use crate::execution::ExecutionConfig;
use crate::options::types::{
    OptionExecutionIntent, OptionExecutionIntentStatus, OptionExecutionSimulationStatus,
    OptionExecutionSourceType, OptionsConfig,
};
use crate::types::AccountId;
use serde::Serialize;

// Chain-id mode mapping (Base).
pub const MAINNET_CHAIN_ID: u64 = 8453;
pub const SEPOLIA_CHAIN_ID: u64 = 84532;

/// Deployment-mode awareness. Mainnet is fail-closed by default; Sepolia is
/// fee-only by construction and tolerates configurations that mainnet refuses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BroadcastMode {
    Mainnet,
    Sepolia,
}

impl BroadcastMode {
    pub fn from_chain_id(chain_id: u64) -> Self {
        match chain_id {
            SEPOLIA_CHAIN_ID => Self::Sepolia,
            _ => Self::Mainnet,
        }
    }

    pub fn is_mainnet(self) -> bool {
        matches!(self, Self::Mainnet)
    }
}

/// Structured allow/deny result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ShouldBroadcastDecision {
    Approve(ApprovalReason),
    Reject(RejectReason),
}

impl ShouldBroadcastDecision {
    pub fn is_approve(&self) -> bool {
        matches!(self, Self::Approve(_))
    }

    pub fn is_reject(&self) -> bool {
        matches!(self, Self::Reject(_))
    }

    /// Stable machine-readable error code for the reject case; `None` for approve.
    pub fn error_code(&self) -> Option<&'static str> {
        match self {
            Self::Approve(_) => None,
            Self::Reject(reason) => Some(reason.code()),
        }
    }

    /// Non-sensitive human-readable description suitable for logs / API.
    pub fn message(&self) -> String {
        match self {
            Self::Approve(reason) => format!("approve:{}", reason.code()),
            Self::Reject(reason) => format!("policy:{}:{}", reason.code(), reason.detail()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalReason {
    Profitable,
    AtCost,
    Subsidisable(SubsidyReason),
}

impl ApprovalReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Profitable => "profitable",
            Self::AtCost => "at_cost",
            Self::Subsidisable(_) => "subsidisable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsidyReason {
    PromotionalLaunch,
    LiquidityBootstrap,
}

/// Structured rejection reason. Codes are stable strings consumed by metrics,
/// alerts, and API clients; details carry non-sensitive context only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum RejectReason {
    Dupe,
    BuyerSigMissing,
    SellerSigMissing,
    Expired { now_ms: i64, deadline_ms: i64 },
    NonceUnsynced,
    ProductUnlisted,
    StaleRm { age_ms: u64, max_age_ms: u64 },
    OmePaused,
    BeNotExec,
    BeLowBal { balance_wei: u128, floor_wei: u128 },
    NoBuyerMargin,
    NoSellerMargin,
    SimRevert(String),
    GasCap { gas_units: u64, cap: u64 },
    Wash,
    QuotaBreach,
    AttackPattern,
    NoEconContent,
    RebateBudget { required: u128, available: u128 },
    RebateReserve { required: u128, available: u128 },
    NegativeEffectivePpm { tier: u8 },
    ChainIdMismatch { expected: u64, actual: u64 },
    SelectorNotAllowed(String),
    TargetNotAllowed,
    SimulationNotOk,
    CalldataMissing,
    InvalidState(String),
    LiquidationOutOfScope,
    PolicyInternal(String),
}

impl RejectReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Dupe => "dupe",
            Self::BuyerSigMissing => "buyer-sig-missing",
            Self::SellerSigMissing => "seller-sig-missing",
            Self::Expired { .. } => "expired",
            Self::NonceUnsynced => "nonce-unsynced",
            Self::ProductUnlisted => "product-unlisted",
            Self::StaleRm { .. } => "stale-rm",
            Self::OmePaused => "ome-paused",
            Self::BeNotExec => "be-not-exec",
            Self::BeLowBal { .. } => "be-low-bal",
            Self::NoBuyerMargin => "no-buyer-margin",
            Self::NoSellerMargin => "no-seller-margin",
            Self::SimRevert(_) => "sim-revert",
            Self::GasCap { .. } => "gas-cap",
            Self::Wash => "wash",
            Self::QuotaBreach => "quota-breach",
            Self::AttackPattern => "attack-pattern",
            Self::NoEconContent => "no-econ-content",
            Self::RebateBudget { .. } => "rebate-budget",
            Self::RebateReserve { .. } => "rebate-reserve",
            Self::NegativeEffectivePpm { .. } => "negative-effective-ppm",
            Self::ChainIdMismatch { .. } => "chain-id-mismatch",
            Self::SelectorNotAllowed(_) => "selector-not-allowed",
            Self::TargetNotAllowed => "target-not-allowed",
            Self::SimulationNotOk => "simulation-not-ok",
            Self::CalldataMissing => "calldata-missing",
            Self::InvalidState(_) => "invalid-state",
            Self::LiquidationOutOfScope => "liquidation-out-of-scope",
            Self::PolicyInternal(_) => "policy-internal",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Dupe
            | Self::BuyerSigMissing
            | Self::SellerSigMissing
            | Self::NonceUnsynced
            | Self::ProductUnlisted
            | Self::OmePaused
            | Self::BeNotExec
            | Self::NoBuyerMargin
            | Self::NoSellerMargin
            | Self::Wash
            | Self::QuotaBreach
            | Self::AttackPattern
            | Self::NoEconContent
            | Self::TargetNotAllowed
            | Self::SimulationNotOk
            | Self::CalldataMissing
            | Self::LiquidationOutOfScope => String::new(),
            Self::Expired {
                now_ms,
                deadline_ms,
            } => format!("now={now_ms},deadline={deadline_ms}"),
            Self::StaleRm { age_ms, max_age_ms } => format!("age={age_ms},max={max_age_ms}"),
            Self::BeLowBal {
                balance_wei,
                floor_wei,
            } => format!("balance={balance_wei},floor={floor_wei}"),
            Self::SimRevert(msg) => msg.clone(),
            Self::GasCap { gas_units, cap } => format!("gas={gas_units},cap={cap}"),
            Self::RebateBudget {
                required,
                available,
            } => format!("required={required},available={available}"),
            Self::RebateReserve {
                required,
                available,
            } => format!("required={required},available={available}"),
            Self::NegativeEffectivePpm { tier } => format!("tier={tier}"),
            Self::ChainIdMismatch { expected, actual } => {
                format!("expected={expected},actual={actual}")
            }
            Self::SelectorNotAllowed(selector) => selector.clone(),
            Self::InvalidState(msg) => msg.clone(),
            Self::PolicyInternal(msg) => msg.clone(),
        }
    }
}

/// Simulation outcome surface required by the policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationSummary {
    pub status: OptionExecutionSimulationStatus,
    pub revert_reason: Option<String>,
    pub gas_units: u64,
}

/// Fee/rebate split computed against the FeesManagerV2 model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeSplitSummary {
    pub gross_fee_revenue: u128,
    pub total_rebate_outflow: u128,
    pub net_protocol_revenue: i128,
    pub effective_maker_ppm: i64,
    pub effective_taker_ppm: i64,
    pub asset: AccountId,
    pub tier: u8,
}

impl FeeSplitSummary {
    pub fn empty(asset: AccountId) -> Self {
        Self {
            gross_fee_revenue: 0,
            total_rebate_outflow: 0,
            net_protocol_revenue: 0,
            effective_maker_ppm: 0,
            effective_taker_ppm: 0,
            asset,
            tier: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct SubsidyBudgetView {
    pub remaining_native: i128,
    pub eligible: bool,
    pub reason: Option<SubsidyReason>,
}

#[derive(Clone, Debug)]
pub struct BroadcastContext<'a> {
    pub chain_id: u64,
    pub now_ms: i64,
    pub mode: BroadcastMode,
    pub options_config: &'a OptionsConfig,
    pub execution_config: &'a ExecutionConfig,
    pub be_address: &'a AccountId,
    pub be_balance_wei: u128,
    pub fund_floor_wei: u128,
    pub ome_paused: bool,
    pub ome_is_executor: bool,
    pub buyer_has_margin: bool,
    pub seller_has_margin: bool,
    pub product_listed: bool,
    pub rm_snapshot_age_ms: u64,
    pub rm_snapshot_max_age_ms: u64,
    pub dedupe_hit: bool,
    pub allowed_target: &'a AccountId,
    pub allowed_selectors: &'a [String],
    pub call_selector: String,
    pub call_target: AccountId,
    pub simulation: SimulationSummary,
    /// When `false`, §8 steps 4 (econ-content + negative-ppm), 5
    /// (rebate-solvency), and 7 (PnL decision states) are skipped — the
    /// boundary-only checks still fire (sigs, status, calldata, deadline,
    /// nonces, target/selector allowlist, wash, chain-id, sim status,
    /// chain-state). Caller sets this to `true` once fee_split + on-chain
    /// rebate budget/reserve are wired (follow-on track).
    pub econ_data_available: bool,
    pub fee_split: FeeSplitSummary,
    pub rebate_budget_asset: u128,
    pub rebate_reserve_asset: u128,
    pub gas_units: u64,
    pub hard_gas_cap: u64,
    pub gas_cost_native: i128,
    pub pnl_floor_native: i128,
    pub safety_margin_bps: u32,
    pub subsidy_budget: SubsidyBudgetView,
}

/// Apply the §8 pseudocode in order, returning the first reject reason or an
/// approval bucket.
pub fn should_broadcast(
    intent: &OptionExecutionIntent,
    context: &BroadcastContext<'_>,
) -> ShouldBroadcastDecision {
    // ---- §8 step 0 — pre-flight static checks ----
    if context.chain_id != context.execution_config.executor_chain_id {
        return ShouldBroadcastDecision::Reject(RejectReason::ChainIdMismatch {
            expected: context.execution_config.executor_chain_id,
            actual: context.chain_id,
        });
    }
    if context.dedupe_hit {
        return ShouldBroadcastDecision::Reject(RejectReason::Dupe);
    }
    if intent.buyer_signature.is_none() {
        return ShouldBroadcastDecision::Reject(RejectReason::BuyerSigMissing);
    }
    if intent.seller_signature.is_none() {
        return ShouldBroadcastDecision::Reject(RejectReason::SellerSigMissing);
    }
    match intent.status {
        OptionExecutionIntentStatus::CalldataReady
        | OptionExecutionIntentStatus::SimulationReady
        | OptionExecutionIntentStatus::SimulationOk => {}
        other => {
            return ShouldBroadcastDecision::Reject(RejectReason::InvalidState(
                other.as_str().to_string(),
            ));
        }
    }
    match intent.calldata.as_deref() {
        None | Some("") | Some("0x") => {
            return ShouldBroadcastDecision::Reject(RejectReason::CalldataMissing);
        }
        Some(_) => {}
    }
    if intent.deadline as i64 != 0 && (intent.deadline as i64) * 1_000 < context.now_ms {
        return ShouldBroadcastDecision::Reject(RejectReason::Expired {
            now_ms: context.now_ms,
            deadline_ms: (intent.deadline as i64) * 1_000,
        });
    }
    if intent.buyer_nonce.is_none() || intent.seller_nonce.is_none() {
        return ShouldBroadcastDecision::Reject(RejectReason::NonceUnsynced);
    }
    if !context.product_listed {
        return ShouldBroadcastDecision::Reject(RejectReason::ProductUnlisted);
    }
    if context.rm_snapshot_age_ms > context.rm_snapshot_max_age_ms {
        return ShouldBroadcastDecision::Reject(RejectReason::StaleRm {
            age_ms: context.rm_snapshot_age_ms,
            max_age_ms: context.rm_snapshot_max_age_ms,
        });
    }

    // Target + selector allowlists.
    if !addresses_equal(&context.call_target, context.allowed_target) {
        return ShouldBroadcastDecision::Reject(RejectReason::TargetNotAllowed);
    }
    let selector_normalized = normalize_selector(&context.call_selector);
    if !context
        .allowed_selectors
        .iter()
        .any(|allowed| normalize_selector(allowed) == selector_normalized)
    {
        return ShouldBroadcastDecision::Reject(RejectReason::SelectorNotAllowed(
            selector_normalized,
        ));
    }

    // Source-type allowlist (orderbook / RFQ; perp not supported here).
    match intent.source_type {
        OptionExecutionSourceType::OptionOrderbookFill
        | OptionExecutionSourceType::OptionRfqFill => {}
    }

    // ---- §8 step 1 — NEW_OME live state ----
    if context.ome_paused {
        return ShouldBroadcastDecision::Reject(RejectReason::OmePaused);
    }
    if !context.ome_is_executor {
        return ShouldBroadcastDecision::Reject(RejectReason::BeNotExec);
    }
    if context.be_balance_wei < context.fund_floor_wei {
        return ShouldBroadcastDecision::Reject(RejectReason::BeLowBal {
            balance_wei: context.be_balance_wei,
            floor_wei: context.fund_floor_wei,
        });
    }

    // ---- §8 step 2 — margin / product guards ----
    if !context.buyer_has_margin {
        return ShouldBroadcastDecision::Reject(RejectReason::NoBuyerMargin);
    }
    if !context.seller_has_margin {
        return ShouldBroadcastDecision::Reject(RejectReason::NoSellerMargin);
    }

    // ---- §8 step 3 — simulate ----
    match context.simulation.status {
        OptionExecutionSimulationStatus::SimulationOk => {}
        OptionExecutionSimulationStatus::SimulationFailed => {
            let reason = context
                .simulation
                .revert_reason
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            return ShouldBroadcastDecision::Reject(RejectReason::SimRevert(reason));
        }
        OptionExecutionSimulationStatus::SimulationPending
        | OptionExecutionSimulationStatus::SimulationUnavailable => {
            return ShouldBroadcastDecision::Reject(RejectReason::SimulationNotOk);
        }
    }
    if context.simulation.gas_units > context.hard_gas_cap {
        return ShouldBroadcastDecision::Reject(RejectReason::GasCap {
            gas_units: context.simulation.gas_units,
            cap: context.hard_gas_cap,
        });
    }

    // ---- §8 step 6 — anti-griefing (boundary-side; runs in every mode) ----
    if addresses_equal(&intent.buyer, &intent.seller) {
        return ShouldBroadcastDecision::Reject(RejectReason::Wash);
    }

    // ---- §8 steps 4 / 5 / 7 — economic gates ----
    //
    // Boundary mode: `econ_data_available == false` — fee_split + on-chain
    // rebate budget/reserve not yet wired. Approve on field-level pass; the
    // Cluster 4 launch invariant is still verified out-of-band via
    // `verify_launch_invariant` (operator sweep + startup hook).
    if !context.econ_data_available {
        return ShouldBroadcastDecision::Approve(ApprovalReason::Profitable);
    }

    // ---- §8 step 4 — fee / rebate computation ----
    if context.fee_split.gross_fee_revenue == 0 && context.fee_split.total_rebate_outflow == 0 {
        return ShouldBroadcastDecision::Reject(RejectReason::NoEconContent);
    }
    if context.mode.is_mainnet()
        && (context.fee_split.effective_maker_ppm < 0 || context.fee_split.effective_taker_ppm < 0)
    {
        return ShouldBroadcastDecision::Reject(RejectReason::NegativeEffectivePpm {
            tier: context.fee_split.tier,
        });
    }

    // ---- §8 step 5 — rebate solvency (HARD GATE — Cluster 4 primary teeth) ----
    if context.fee_split.total_rebate_outflow > 0 {
        if context.rebate_budget_asset < context.fee_split.total_rebate_outflow {
            return ShouldBroadcastDecision::Reject(RejectReason::RebateBudget {
                required: context.fee_split.total_rebate_outflow,
                available: context.rebate_budget_asset,
            });
        }
        if context.rebate_reserve_asset < context.fee_split.total_rebate_outflow {
            return ShouldBroadcastDecision::Reject(RejectReason::RebateReserve {
                required: context.fee_split.total_rebate_outflow,
                available: context.rebate_reserve_asset,
            });
        }
    }

    // ---- §8 step 7 — gas cost in asset terms + decision states ----
    let expected_pnl = context
        .fee_split
        .net_protocol_revenue
        .saturating_sub(context.gas_cost_native);

    if expected_pnl >= context.pnl_floor_native {
        return ShouldBroadcastDecision::Approve(ApprovalReason::Profitable);
    }

    let safety_margin = i128::from(context.safety_margin_bps);
    let at_cost_threshold = context.gas_cost_native.saturating_mul(safety_margin) / 10_000;
    if context.fee_split.net_protocol_revenue >= at_cost_threshold {
        return ShouldBroadcastDecision::Approve(ApprovalReason::AtCost);
    }

    let gap = context
        .gas_cost_native
        .saturating_sub(context.fee_split.net_protocol_revenue);
    if context.subsidy_budget.eligible && context.subsidy_budget.remaining_native >= gap {
        let reason = context
            .subsidy_budget
            .reason
            .clone()
            .unwrap_or(SubsidyReason::PromotionalLaunch);
        return ShouldBroadcastDecision::Approve(ApprovalReason::Subsidisable(reason));
    }

    ShouldBroadcastDecision::Reject(RejectReason::NoEconContent)
}

// ----------- Launch invariant verifier -----------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductKind {
    Option,
    Future,
    Perp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeFlow {
    Orderbook,
    Rfq,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveFeeProfile {
    pub tier: u8,
    pub product: ProductKind,
    pub flow: FeeFlow,
    pub maker_ppm: i64,
    pub taker_ppm: i64,
    pub maker_discount_ppm: u64,
    pub taker_discount_ppm: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EffectiveProfileEntry {
    pub tier: u8,
    pub product: ProductKind,
    pub flow: FeeFlow,
    pub effective_maker_ppm: i64,
    pub effective_taker_ppm: i64,
    pub non_negative: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LaunchInvariantReport {
    pub all_profiles_non_negative: bool,
    pub profiles: Vec<EffectiveProfileEntry>,
    pub rebate_reserve: u128,
    pub rebate_reserve_zero: bool,
    pub overall_pass: bool,
}

/// Compute effective maker/taker ppm after applying the per-flow discount and
/// classify each profile against the Cluster 4 non-negative invariant.
pub fn verify_launch_invariant(
    profiles: &[ActiveFeeProfile],
    rebate_reserve_asset: u128,
    mode: BroadcastMode,
) -> LaunchInvariantReport {
    let entries: Vec<EffectiveProfileEntry> = profiles
        .iter()
        .map(|p| {
            let effective_maker = apply_discount(p.maker_ppm, p.maker_discount_ppm);
            let effective_taker = apply_discount(p.taker_ppm, p.taker_discount_ppm);
            EffectiveProfileEntry {
                tier: p.tier,
                product: p.product,
                flow: p.flow,
                effective_maker_ppm: effective_maker,
                effective_taker_ppm: effective_taker,
                non_negative: effective_maker >= 0 && effective_taker >= 0,
            }
        })
        .collect();

    let all_non_negative = entries.iter().all(|e| e.non_negative);
    let reserve_zero = rebate_reserve_asset == 0;
    let overall_pass = match mode {
        BroadcastMode::Mainnet => all_non_negative && reserve_zero,
        BroadcastMode::Sepolia => all_non_negative,
    };

    LaunchInvariantReport {
        all_profiles_non_negative: all_non_negative,
        profiles: entries,
        rebate_reserve: rebate_reserve_asset,
        rebate_reserve_zero: reserve_zero,
        overall_pass,
    }
}

/// Clamp discount to [0, 1_000_000] then subtract from the base ppm.
/// Cluster 4 launch invariant requires effective_ppm >= 0; a discount that
/// would push it negative is reported as `non_negative=false`.
fn apply_discount(base_ppm: i64, discount_ppm: u64) -> i64 {
    let discount = discount_ppm.min(1_000_000) as i64;
    base_ppm.saturating_mul(1_000_000_i64.saturating_sub(discount)) / 1_000_000
}

// ----------- helpers -----------

fn addresses_equal(a: &AccountId, b: &AccountId) -> bool {
    a.0.eq_ignore_ascii_case(&b.0)
}

fn normalize_selector(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::config::ExecutionConfig;
    use crate::options::types::{OptionExecutionIntentStatus, OptionExecutionSourceType};
    use crate::types::{AccountId, Price1e8, Size1e8};
    use uuid::Uuid;

    const OPTION_TARGET: &str = "0x000000000000000000000000000000000000beef";
    const BUYER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SELLER: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const BE: &str = "0xcccccccccccccccccccccccccccccccccccccccc";
    const ASSET: &str = "0x6eae407f5640b006fac9965182e238582a3b412e";
    const TRADE_SELECTOR: &str = "executeTrade((bytes32,address,address,uint256,address,address,uint64,uint64,bool,uint128,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)";

    fn options_config() -> OptionsConfig {
        OptionsConfig::disabled()
    }

    fn execution_config(chain_id: u64) -> ExecutionConfig {
        let mut cfg = ExecutionConfig::disabled();
        cfg.executor_chain_id = chain_id;
        cfg
    }

    fn allowed_selectors() -> Vec<String> {
        vec![TRADE_SELECTOR.to_string()]
    }

    fn approve_fee_split() -> FeeSplitSummary {
        FeeSplitSummary {
            gross_fee_revenue: 3_000,
            total_rebate_outflow: 0,
            net_protocol_revenue: 3_000,
            effective_maker_ppm: 50,
            effective_taker_ppm: 100,
            asset: AccountId::new(ASSET),
            tier: 0,
        }
    }

    fn happy_intent() -> OptionExecutionIntent {
        OptionExecutionIntent {
            intent_id: Uuid::nil(),
            onchain_intent_id: "0x01".to_string(),
            source_type: OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-1".to_string(),
            option_series_id: "series-1".to_string(),
            onchain_option_id: "0x02".to_string(),
            buyer: AccountId::new(BUYER),
            seller: AccountId::new(SELLER),
            underlying: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            settlement_asset: AccountId::new(ASSET),
            expiry: 9_999_999_999,
            strike_1e8: 100_000_000 as Price1e8,
            is_call: true,
            contract_size_1e8: 100_000_000 as Size1e8,
            quantity_contracts: 1,
            source_size_1e8: 100_000_000 as Size1e8,
            source_price_1e8: 100_000_000 as Price1e8,
            premium_per_contract_native: 1_000_000,
            buyer_is_maker: true,
            buyer_nonce: Some(0),
            seller_nonce: Some(0),
            deadline: 9_999_999_999,
            buyer_signature: Some("0x01".to_string()),
            seller_signature: Some("0x02".to_string()),
            calldata: Some("0xdeadbeef".to_string()),
            status: OptionExecutionIntentStatus::CalldataReady,
            error: None,
            simulation_status: Some(OptionExecutionSimulationStatus::SimulationOk),
            simulation_error: None,
            simulation_block_number: Some(1),
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: Some(1),
            canonical_execution_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn happy_ctx<'a>(
        options: &'a OptionsConfig,
        exec: &'a ExecutionConfig,
        be: &'a AccountId,
        target: &'a AccountId,
        selectors: &'a [String],
    ) -> BroadcastContext<'a> {
        BroadcastContext {
            chain_id: SEPOLIA_CHAIN_ID,
            now_ms: 1_000_000,
            mode: BroadcastMode::Sepolia,
            options_config: options,
            execution_config: exec,
            be_address: be,
            be_balance_wei: 1_000_000_000_000_000_000,
            fund_floor_wei: 100_000_000_000_000_000,
            ome_paused: false,
            ome_is_executor: true,
            buyer_has_margin: true,
            seller_has_margin: true,
            product_listed: true,
            rm_snapshot_age_ms: 100,
            rm_snapshot_max_age_ms: 5_000,
            dedupe_hit: false,
            allowed_target: target,
            allowed_selectors: selectors,
            call_selector: TRADE_SELECTOR.to_string(),
            call_target: AccountId::new(OPTION_TARGET),
            simulation: SimulationSummary {
                status: OptionExecutionSimulationStatus::SimulationOk,
                revert_reason: None,
                gas_units: 300_000,
            },
            econ_data_available: true,
            fee_split: approve_fee_split(),
            rebate_budget_asset: 0,
            rebate_reserve_asset: 0,
            gas_units: 300_000,
            hard_gas_cap: 5_000_000,
            gas_cost_native: 100,
            pnl_floor_native: 500,
            safety_margin_bps: 11_000,
            subsidy_budget: SubsidyBudgetView::default(),
        }
    }

    // ---- Test 1 — happy path ----
    #[test]
    fn approves_profitable_orderbook_intent() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(
            decision,
            ShouldBroadcastDecision::Approve(ApprovalReason::Profitable)
        );
        assert_eq!(decision.error_code(), None);
    }

    // ---- Test 2 — wash detection ----
    #[test]
    fn rejects_same_buyer_and_seller_as_wash() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.seller = intent.buyer.clone();
        let ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("wash"));
    }

    // ---- Test 3 — sim revert ----
    #[test]
    fn rejects_simulation_revert() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.simulation = SimulationSummary {
            status: OptionExecutionSimulationStatus::SimulationFailed,
            revert_reason: Some("OME: nonce".to_string()),
            gas_units: 1,
        };
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("sim-revert"));
        assert!(decision.message().contains("OME: nonce"));
    }

    // ---- Test 4 — gas cap ----
    #[test]
    fn rejects_gas_units_over_cap() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.simulation.gas_units = 10_000_000;
        ctx.hard_gas_cap = 5_000_000;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("gas-cap"));
    }

    // ---- Test 5 — zero economic content ----
    #[test]
    fn rejects_zero_econ_content() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.fee_split = FeeSplitSummary {
            gross_fee_revenue: 0,
            total_rebate_outflow: 0,
            net_protocol_revenue: 0,
            effective_maker_ppm: 0,
            effective_taker_ppm: 0,
            asset: AccountId::new(ASSET),
            tier: 0,
        };
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("no-econ-content"));
    }

    // ---- Test 6 — rebate-solvency hard gate (Cluster 4 primary teeth) ----
    #[test]
    fn rejects_rebate_positive_when_reserve_zero() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.fee_split.total_rebate_outflow = 100;
        ctx.fee_split.gross_fee_revenue = 100;
        ctx.fee_split.net_protocol_revenue = 0;
        ctx.rebate_budget_asset = 1_000;
        ctx.rebate_reserve_asset = 0;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("rebate-reserve"));
    }

    #[test]
    fn rejects_rebate_positive_when_budget_insufficient() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.fee_split.total_rebate_outflow = 1_000;
        ctx.fee_split.gross_fee_revenue = 1_000;
        ctx.fee_split.net_protocol_revenue = 0;
        ctx.rebate_budget_asset = 100;
        ctx.rebate_reserve_asset = 100_000;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("rebate-budget"));
    }

    // ---- Test 7 — fee-only profile with reserve != 0 (approve) ----
    #[test]
    fn approves_fee_only_intent_even_when_reserve_nonzero() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.rebate_reserve_asset = 1_000_000;
        let decision = should_broadcast(&intent, &ctx);
        assert!(decision.is_approve());
    }

    // ---- Test 8 — BE balance below floor ----
    #[test]
    fn rejects_be_balance_below_floor() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.be_balance_wei = 1;
        ctx.fund_floor_wei = 100_000_000_000_000_000;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("be-low-bal"));
    }

    // ---- Test 9 — OME paused ----
    #[test]
    fn rejects_when_ome_paused() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.ome_paused = true;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("ome-paused"));
    }

    // ---- Test 10 — OME isExecutor false ----
    #[test]
    fn rejects_when_be_not_executor() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.ome_is_executor = false;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("be-not-exec"));
    }

    // ---- Test 11 — expired deadline ----
    #[test]
    fn rejects_expired_deadline() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.deadline = 1;
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.now_ms = 9_999_999_999_000;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("expired"));
    }

    // ---- Test 12 — stale RM snapshot ----
    #[test]
    fn rejects_stale_rm_snapshot() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.rm_snapshot_age_ms = 60_000;
        ctx.rm_snapshot_max_age_ms = 5_000;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("stale-rm"));
    }

    // ---- Test 13 — dedupe cache hit ----
    #[test]
    fn rejects_dedupe_hit() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.dedupe_hit = true;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("dupe"));
    }

    // ---- Test 14 — at-cost approval ----
    #[test]
    fn approves_at_cost_when_revenue_covers_gas_with_margin() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.fee_split.net_protocol_revenue = 110;
        ctx.gas_cost_native = 100;
        ctx.pnl_floor_native = 500;
        ctx.safety_margin_bps = 11_000;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(
            decision,
            ShouldBroadcastDecision::Approve(ApprovalReason::AtCost)
        );
    }

    // ---- Test 15 — subsidisable approval ----
    #[test]
    fn approves_subsidisable_when_budget_covers_gap() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.fee_split.net_protocol_revenue = 10;
        ctx.gas_cost_native = 100;
        ctx.pnl_floor_native = 500;
        ctx.safety_margin_bps = 11_000;
        ctx.subsidy_budget = SubsidyBudgetView {
            remaining_native: 1_000,
            eligible: true,
            reason: Some(SubsidyReason::LiquidityBootstrap),
        };
        let decision = should_broadcast(&intent, &ctx);
        match decision {
            ShouldBroadcastDecision::Approve(ApprovalReason::Subsidisable(reason)) => {
                assert_eq!(reason, SubsidyReason::LiquidityBootstrap);
            }
            other => panic!("expected subsidisable, got {other:?}"),
        }
    }

    // ---- Negative-effective-ppm mainnet hard gate ----
    #[test]
    fn rejects_negative_effective_ppm_on_mainnet() {
        let options = options_config();
        let exec = execution_config(MAINNET_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.chain_id = MAINNET_CHAIN_ID;
        ctx.mode = BroadcastMode::Mainnet;
        ctx.fee_split.effective_maker_ppm = -10;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("negative-effective-ppm"));
    }

    // ---- Chain-id mismatch ----
    #[test]
    fn rejects_chain_id_mismatch() {
        let options = options_config();
        let exec = execution_config(MAINNET_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.chain_id = SEPOLIA_CHAIN_ID;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("chain-id-mismatch"));
    }

    // ---- Target / selector allowlists ----
    #[test]
    fn rejects_unknown_target() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.call_target = AccountId::new("0x0000000000000000000000000000000000000666");
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("target-not-allowed"));
    }

    #[test]
    fn rejects_unknown_selector() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.call_selector = "transfer(address,uint256)".to_string();
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("selector-not-allowed"));
    }

    #[test]
    fn rejects_missing_buyer_signature() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.buyer_signature = None;
        let ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("buyer-sig-missing"));
    }

    #[test]
    fn rejects_missing_seller_signature() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.seller_signature = None;
        let ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("seller-sig-missing"));
    }

    #[test]
    fn rejects_missing_calldata() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.calldata = Some("0x".to_string());
        let ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("calldata-missing"));
    }

    #[test]
    fn rejects_invalid_state_machine_status() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.status = OptionExecutionIntentStatus::Pending;
        let ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("invalid-state"));
    }

    #[test]
    fn rejects_unsynced_nonces() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.seller_nonce = None;
        let ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("nonce-unsynced"));
    }

    #[test]
    fn boundary_mode_skips_econ_checks_and_approves() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.econ_data_available = false;
        ctx.fee_split = FeeSplitSummary::empty(AccountId::new(ASSET));
        ctx.rebate_reserve_asset = 0;
        ctx.rebate_budget_asset = 0;
        let decision = should_broadcast(&intent, &ctx);
        assert!(
            decision.is_approve(),
            "boundary mode must approve when field-level checks pass; got {decision:?}"
        );
    }

    #[test]
    fn boundary_mode_still_enforces_wash_check() {
        let options = options_config();
        let exec = execution_config(SEPOLIA_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let mut intent = happy_intent();
        intent.seller = intent.buyer.clone();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.econ_data_available = false;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("wash"));
    }

    #[test]
    fn boundary_mode_still_enforces_chain_id_mismatch() {
        let options = options_config();
        let exec = execution_config(MAINNET_CHAIN_ID);
        let be = AccountId::new(BE);
        let target = AccountId::new(OPTION_TARGET);
        let selectors = allowed_selectors();
        let intent = happy_intent();
        let mut ctx = happy_ctx(&options, &exec, &be, &target, &selectors);
        ctx.econ_data_available = false;
        ctx.chain_id = SEPOLIA_CHAIN_ID;
        let decision = should_broadcast(&intent, &ctx);
        assert_eq!(decision.error_code(), Some("chain-id-mismatch"));
    }

    // ----- launch invariant verifier -----

    fn fee_only_profiles() -> Vec<ActiveFeeProfile> {
        vec![ActiveFeeProfile {
            tier: 0,
            product: ProductKind::Option,
            flow: FeeFlow::Orderbook,
            maker_ppm: 50,
            taker_ppm: 100,
            maker_discount_ppm: 0,
            taker_discount_ppm: 0,
        }]
    }

    // ---- Test 16 — all non-negative + reserve == 0 ----
    #[test]
    fn launch_invariant_passes_when_fee_only_and_reserve_zero() {
        let report = verify_launch_invariant(&fee_only_profiles(), 0, BroadcastMode::Mainnet);
        assert!(report.all_profiles_non_negative);
        assert!(report.rebate_reserve_zero);
        assert!(report.overall_pass);
        assert_eq!(report.profiles.len(), 1);
        assert!(report.profiles[0].non_negative);
    }

    // ---- Test 17 — one effective negative profile ----
    #[test]
    fn launch_invariant_fails_when_any_profile_effective_negative() {
        let mut profiles = fee_only_profiles();
        profiles.push(ActiveFeeProfile {
            tier: 1,
            product: ProductKind::Option,
            flow: FeeFlow::Rfq,
            maker_ppm: -5,
            taker_ppm: 100,
            maker_discount_ppm: 0,
            taker_discount_ppm: 0,
        });
        let report = verify_launch_invariant(&profiles, 0, BroadcastMode::Mainnet);
        assert!(!report.all_profiles_non_negative);
        assert!(!report.overall_pass);
        assert!(!report.profiles[1].non_negative);
        assert!(report.profiles[0].non_negative);
    }

    // ---- Test 18 — reserve > 0 fails mainnet launch invariant ----
    #[test]
    fn launch_invariant_fails_when_reserve_nonzero_on_mainnet() {
        let report = verify_launch_invariant(&fee_only_profiles(), 1, BroadcastMode::Mainnet);
        assert!(report.all_profiles_non_negative);
        assert!(!report.rebate_reserve_zero);
        assert!(!report.overall_pass);
    }

    #[test]
    fn launch_invariant_passes_on_sepolia_even_when_reserve_nonzero() {
        let report = verify_launch_invariant(&fee_only_profiles(), 1, BroadcastMode::Sepolia);
        assert!(report.overall_pass);
    }

    // ---- Test 19 — RFQ discount edge case ----
    #[test]
    fn rfq_discount_clamped_at_1m_ppm() {
        let profiles = vec![ActiveFeeProfile {
            tier: 0,
            product: ProductKind::Option,
            flow: FeeFlow::Rfq,
            maker_ppm: 50,
            taker_ppm: 100,
            maker_discount_ppm: 1_000_001,
            taker_discount_ppm: 0,
        }];
        let report = verify_launch_invariant(&profiles, 0, BroadcastMode::Mainnet);
        assert!(report.profiles[0].non_negative);
        assert_eq!(report.profiles[0].effective_maker_ppm, 0);
        assert_eq!(report.profiles[0].effective_taker_ppm, 100);
        assert!(report.overall_pass);
    }

    #[test]
    fn discount_pushing_negative_is_flagged() {
        // maker_ppm > 0 cannot go negative because clamp caps discount at 1m;
        // negativity can only originate from base_ppm < 0.
        let profiles = vec![ActiveFeeProfile {
            tier: 2,
            product: ProductKind::Option,
            flow: FeeFlow::Orderbook,
            maker_ppm: -5,
            taker_ppm: 100,
            maker_discount_ppm: 0,
            taker_discount_ppm: 0,
        }];
        let report = verify_launch_invariant(&profiles, 0, BroadcastMode::Mainnet);
        assert!(!report.profiles[0].non_negative);
        assert!(!report.overall_pass);
    }
}
