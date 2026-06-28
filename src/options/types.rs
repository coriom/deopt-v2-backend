use crate::error::{BackendError, Result};
use crate::execution::ExecutionTransactionStatus;
use crate::signing::Eip712Domain;
use crate::types::{AccountId, OrderId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

pub type OptionSeriesId = String;
pub type OptionOrderId = OrderId;
pub type OptionFillId = Uuid;
pub type OptionRfqId = Uuid;
pub type OptionRfqQuoteId = Uuid;
pub type OptionRfqFillId = Uuid;
pub type OptionExecutionIntentId = Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionsConfig {
    pub enabled: bool,
    pub require_persistence: bool,
    pub allow_manual_series: bool,
    pub sync_onchain_registry: bool,
    pub default_contract_size_1e8: Size1e8,
    pub rfq_enabled: bool,
    pub rfq_require_persistence: bool,
    pub rfq_default_ttl_ms: u64,
    pub rfq_max_ttl_ms: u64,
    pub rfq_min_quote_ttl_ms: u64,
    pub rfq_max_quote_ttl_ms: u64,
    pub rfq_max_quotes_per_rfq: usize,
    pub rfq_quote_signature_mode: OptionRfqQuoteSignatureMode,
    pub rfq_eip712_domain: Eip712Domain,
    pub execution_enabled: bool,
    pub execution_require_persistence: bool,
    pub matching_engine_address: AccountId,
    pub execution_signature_mode: OptionExecutionSignatureMode,
    pub execution_eip712_domain: Eip712Domain,
    pub execution_default_settlement_decimals: u32,
    pub execution_simulation_enabled: bool,
    pub execution_require_rpc_for_simulation: bool,
    pub execution_simulation_gas_limit: u64,
    pub execution_simulation_from: Option<AccountId>,
    pub execution_simulation_rpc_url: Option<String>,
    pub execution_broadcast_enabled: bool,
    pub execution_require_simulation_ok: bool,
    pub execution_broadcast_gas_limit: u64,
    pub execution_gas_safety_bps: u32,
}

pub const OPTION_EXECUTION_GAS_SAFETY_BPS_MIN: u32 = 10_000;
pub const OPTION_EXECUTION_GAS_SAFETY_BPS_MAX: u32 = 50_000;
pub const OPTION_EXECUTION_GAS_SAFETY_BPS_DEFAULT: u32 = 12_500;

impl OptionsConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
            allow_manual_series: true,
            sync_onchain_registry: false,
            default_contract_size_1e8: 100_000_000,
            rfq_enabled: false,
            rfq_require_persistence: true,
            rfq_default_ttl_ms: 5_000,
            rfq_max_ttl_ms: 30_000,
            rfq_min_quote_ttl_ms: 500,
            rfq_max_quote_ttl_ms: 10_000,
            rfq_max_quotes_per_rfq: 50,
            rfq_quote_signature_mode: OptionRfqQuoteSignatureMode::Disabled,
            rfq_eip712_domain: Eip712Domain {
                name: "DeOptV2OptionRFQ".to_string(),
                version: "1".to_string(),
                chain_id: 84532,
                verifying_contract: AccountId::new("0x0000000000000000000000000000000000000000"),
            },
            execution_enabled: false,
            execution_require_persistence: true,
            matching_engine_address: AccountId::new(""),
            execution_signature_mode: OptionExecutionSignatureMode::Disabled,
            execution_eip712_domain: Eip712Domain {
                name: "DeOptV2-OptionMatchingEngine".to_string(),
                version: "1".to_string(),
                chain_id: 84532,
                verifying_contract: AccountId::new(""),
            },
            execution_default_settlement_decimals: 6,
            execution_simulation_enabled: false,
            execution_require_rpc_for_simulation: true,
            execution_simulation_gas_limit: 0,
            execution_simulation_from: None,
            execution_simulation_rpc_url: None,
            execution_broadcast_enabled: false,
            execution_require_simulation_ok: true,
            execution_broadcast_gas_limit: 0,
            execution_gas_safety_bps: OPTION_EXECUTION_GAS_SAFETY_BPS_DEFAULT,
        }
    }

    pub fn enabled_in_memory_for_tests() -> Self {
        Self {
            enabled: true,
            require_persistence: false,
            ..Self::disabled()
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.enabled && self.require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "Options require persistence enabled when OPTIONS_REQUIRE_PERSISTENCE=true"
                    .to_string(),
            ));
        }
        if self.default_contract_size_1e8 == 0 {
            return Err(BackendError::Config(
                "OPTIONS_DEFAULT_CONTRACT_SIZE_1E8 must be nonzero".to_string(),
            ));
        }
        if self.rfq_enabled && self.rfq_require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "Option RFQ requires persistence enabled when OPTION_RFQ_REQUIRE_PERSISTENCE=true"
                    .to_string(),
            ));
        }
        if self.rfq_default_ttl_ms == 0 || self.rfq_max_ttl_ms == 0 {
            return Err(BackendError::Config(
                "Option RFQ TTL bounds must be nonzero".to_string(),
            ));
        }
        if self.rfq_default_ttl_ms > self.rfq_max_ttl_ms {
            return Err(BackendError::Config(
                "OPTION_RFQ_DEFAULT_TTL_MS must be <= OPTION_RFQ_MAX_TTL_MS".to_string(),
            ));
        }
        if self.rfq_min_quote_ttl_ms == 0 || self.rfq_max_quote_ttl_ms < self.rfq_min_quote_ttl_ms {
            return Err(BackendError::Config(
                "Option RFQ quote TTL bounds are invalid".to_string(),
            ));
        }
        if self.rfq_max_quotes_per_rfq == 0 {
            return Err(BackendError::Config(
                "OPTION_RFQ_MAX_QUOTES_PER_RFQ must be nonzero".to_string(),
            ));
        }
        if self.execution_enabled && !self.enabled {
            return Err(BackendError::Config(
                "Option execution requires OPTIONS_ENABLED=true".to_string(),
            ));
        }
        if self.execution_enabled && self.execution_require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "Option execution requires persistence enabled when OPTION_EXECUTION_REQUIRE_PERSISTENCE=true".to_string(),
            ));
        }
        if self.execution_enabled {
            crate::signing::eip712::parse_evm_address(&self.matching_engine_address)
                .map_err(|_| {
                    BackendError::Config(
                        "OPTION_MATCHING_ENGINE_ADDRESS must be a valid address when OPTION_EXECUTION_ENABLED=true".to_string(),
                    )
                })?;
            if self
                .matching_engine_address
                .0
                .eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
            {
                return Err(BackendError::Config(
                    "OPTION_MATCHING_ENGINE_ADDRESS must be nonzero when OPTION_EXECUTION_ENABLED=true"
                        .to_string(),
                ));
            }
            if self.execution_eip712_domain.verifying_contract.0 != self.matching_engine_address.0 {
                return Err(BackendError::Config(
                    "OPTION_EXECUTION_EIP712 verifying contract must match OPTION_MATCHING_ENGINE_ADDRESS".to_string(),
                ));
            }
        }
        if self.execution_default_settlement_decimals > 38 {
            return Err(BackendError::Config(
                "OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS must be <= 38".to_string(),
            ));
        }
        if self.execution_simulation_enabled && !self.enabled {
            return Err(BackendError::Config(
                "Option execution simulation requires OPTIONS_ENABLED=true".to_string(),
            ));
        }
        if self.execution_broadcast_enabled && !self.enabled {
            return Err(BackendError::Config(
                "Option execution broadcast requires OPTIONS_ENABLED=true".to_string(),
            ));
        }
        if self.execution_broadcast_enabled && !self.execution_enabled {
            return Err(BackendError::Config(
                "Option execution broadcast requires OPTION_EXECUTION_ENABLED=true".to_string(),
            ));
        }
        if self.execution_simulation_enabled
            && self.execution_require_rpc_for_simulation
            && self.execution_simulation_rpc_url.is_none()
        {
            return Err(BackendError::Config(
                "RPC_URL is required when OPTION_EXECUTION_SIMULATION_ENABLED=true and OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true".to_string(),
            ));
        }
        if let Some(from) = self.execution_simulation_from.as_ref() {
            crate::signing::eip712::parse_evm_address(from).map_err(|_| {
                BackendError::Config(
                    "OPTION_EXECUTION_SIMULATION_FROM must be a valid address".to_string(),
                )
            })?;
        }
        if self.execution_gas_safety_bps < OPTION_EXECUTION_GAS_SAFETY_BPS_MIN {
            return Err(BackendError::Config(format!(
                "OPTION_EXECUTION_GAS_SAFETY_BPS must be >= {OPTION_EXECUTION_GAS_SAFETY_BPS_MIN} (no safety margin below 100%)"
            )));
        }
        if self.execution_gas_safety_bps > OPTION_EXECUTION_GAS_SAFETY_BPS_MAX {
            return Err(BackendError::Config(format!(
                "OPTION_EXECUTION_GAS_SAFETY_BPS must be <= {OPTION_EXECUTION_GAS_SAFETY_BPS_MAX}"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionRfqQuoteSignatureMode {
    #[default]
    Disabled,
    Strict,
}

impl FromStr for OptionRfqQuoteSignatureMode {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "strict" => Ok(Self::Strict),
            other => Err(BackendError::Config(format!(
                "invalid OPTION_RFQ_QUOTE_SIGNATURE_MODE: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionSignatureMode {
    #[default]
    Disabled,
    Strict,
}

impl FromStr for OptionExecutionSignatureMode {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "strict" => Ok(Self::Strict),
            other => Err(BackendError::Config(format!(
                "invalid OPTION_EXECUTION_SIGNATURE_MODE: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionSourceType {
    OptionOrderbookFill,
    OptionRfqFill,
}

impl OptionExecutionSourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptionOrderbookFill => "option_orderbook_fill",
            Self::OptionRfqFill => "option_rfq_fill",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "option_orderbook_fill" => Ok(Self::OptionOrderbookFill),
            "option_rfq_fill" => Ok(Self::OptionRfqFill),
            other => Err(BackendError::Persistence(format!(
                "invalid option execution source type: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionIntentStatus {
    Pending,
    SignaturesRequired,
    SignaturesReady,
    CalldataReady,
    SimulationReady,
    SimulationOk,
    SimulationFailed,
    BroadcastSubmitted,
    BroadcastConfirmed,
    BroadcastReverted,
    BroadcastFailed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionSimulationStatus {
    SimulationPending,
    SimulationOk,
    SimulationFailed,
    SimulationUnavailable,
}

impl OptionExecutionSimulationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SimulationPending => "simulation_pending",
            Self::SimulationOk => "simulation_ok",
            Self::SimulationFailed => "simulation_failed",
            Self::SimulationUnavailable => "simulation_unavailable",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "simulation_pending" => Ok(Self::SimulationPending),
            "simulation_ok" => Ok(Self::SimulationOk),
            "simulation_failed" => Ok(Self::SimulationFailed),
            "simulation_unavailable" => Ok(Self::SimulationUnavailable),
            other => Err(BackendError::Persistence(format!(
                "invalid option execution simulation status: {other}"
            ))),
        }
    }
}

impl OptionExecutionIntentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::SignaturesRequired => "signatures_required",
            Self::SignaturesReady => "signatures_ready",
            Self::CalldataReady => "calldata_ready",
            Self::SimulationReady => "simulation_ready",
            Self::SimulationOk => "simulation_ok",
            Self::SimulationFailed => "simulation_failed",
            Self::BroadcastSubmitted => "broadcast_submitted",
            Self::BroadcastConfirmed => "broadcast_confirmed",
            Self::BroadcastReverted => "broadcast_reverted",
            Self::BroadcastFailed => "broadcast_failed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "signatures_required" => Ok(Self::SignaturesRequired),
            "signatures_ready" => Ok(Self::SignaturesReady),
            "calldata_ready" => Ok(Self::CalldataReady),
            "simulation_ready" => Ok(Self::SimulationReady),
            "simulation_ok" => Ok(Self::SimulationOk),
            "simulation_failed" => Ok(Self::SimulationFailed),
            "broadcast_submitted" => Ok(Self::BroadcastSubmitted),
            "broadcast_confirmed" => Ok(Self::BroadcastConfirmed),
            "broadcast_reverted" => Ok(Self::BroadcastReverted),
            "broadcast_failed" => Ok(Self::BroadcastFailed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => Err(BackendError::Persistence(format!(
                "invalid option execution intent status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionExecutionSimulationResult {
    pub intent_id: OptionExecutionIntentId,
    pub simulation_status: OptionExecutionSimulationStatus,
    pub block_number: Option<u64>,
    pub error: Option<String>,
    pub revert_data: Option<String>,
    pub revert_selector: Option<String>,
    pub simulated_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionExecutionIntent {
    pub intent_id: OptionExecutionIntentId,
    pub onchain_intent_id: String,
    pub source_type: OptionExecutionSourceType,
    pub source_id: String,
    pub option_series_id: OptionSeriesId,
    pub onchain_option_id: String,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub underlying: AccountId,
    pub settlement_asset: AccountId,
    pub expiry: u64,
    pub strike_1e8: Price1e8,
    pub is_call: bool,
    pub contract_size_1e8: Size1e8,
    pub quantity_contracts: u128,
    pub source_size_1e8: Size1e8,
    pub source_price_1e8: Price1e8,
    pub premium_per_contract_native: u128,
    pub buyer_is_maker: bool,
    pub buyer_nonce: Option<u128>,
    pub seller_nonce: Option<u128>,
    pub deadline: u64,
    pub buyer_signature: Option<String>,
    pub seller_signature: Option<String>,
    pub calldata: Option<String>,
    pub status: OptionExecutionIntentStatus,
    pub error: Option<String>,
    pub simulation_status: Option<OptionExecutionSimulationStatus>,
    pub simulation_error: Option<String>,
    pub simulation_block_number: Option<u64>,
    pub simulation_revert_data: Option<String>,
    pub simulation_revert_selector: Option<String>,
    pub simulated_at_ms: Option<TimestampMs>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionExecutionTransaction {
    pub transaction_id: String,
    pub intent_id: OptionExecutionIntentId,
    pub onchain_intent_id: Option<String>,
    pub from: AccountId,
    pub to: AccountId,
    pub calldata: String,
    pub value_wei: String,
    pub gas_limit: Option<u64>,
    pub tx_hash: Option<String>,
    pub status: ExecutionTransactionStatus,
    pub error: Option<String>,
    pub estimated_gas: Option<u64>,
    pub required_gas: Option<u64>,
    pub simulation_gas_limit: Option<u64>,
    pub broadcast_gas_limit: Option<u64>,
    pub gas_safety_bps: Option<u32>,
    pub gas_check_status: Option<OptionExecutionGasCheckStatus>,
    pub gas_check_error: Option<String>,
    pub confirmation_status: Option<OptionExecutionConfirmationStatus>,
    pub confirmed_at_ms: Option<TimestampMs>,
    pub confirmed_block_number: Option<u64>,
    pub receipt_status: Option<u64>,
    pub confirmation_error: Option<String>,
    pub gas_used: Option<u64>,
    pub effective_gas_price: Option<String>,
    pub cumulative_gas_used: Option<u64>,
    pub receipt_block_hash: Option<String>,
    pub receipt_transaction_index: Option<u64>,
    pub receipt_observed_at_ms: Option<TimestampMs>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

/// Receipt-cost bundle persisted on `option_execution_transactions` rows when
/// `eth_getTransactionReceipt` returns the corresponding fields. All fields
/// are optional; the worker and manual confirmation paths persist whichever
/// values the receipt provider actually populates.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct OptionExecutionReceiptCost {
    pub gas_used: Option<u64>,
    pub effective_gas_price: Option<String>,
    pub cumulative_gas_used: Option<u64>,
    pub block_hash: Option<String>,
    pub transaction_index: Option<u64>,
    pub observed_at_ms: Option<TimestampMs>,
}

impl OptionExecutionReceiptCost {
    pub fn is_empty(&self) -> bool {
        self.gas_used.is_none()
            && self.effective_gas_price.is_none()
            && self.cumulative_gas_used.is_none()
            && self.block_hash.is_none()
            && self.transaction_index.is_none()
            && self.observed_at_ms.is_none()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionExecutionEvent {
    pub id: Uuid,
    pub chain_id: u64,
    pub contract_address: String,
    pub tx_hash: String,
    pub log_index: u64,
    pub block_number: u64,
    pub block_hash: Option<String>,
    pub event_name: String,
    pub event_signature: String,
    pub intent_id: Option<Uuid>,
    pub onchain_intent_id: Option<String>,
    pub option_execution_transaction_id: Option<String>,
    pub buyer: Option<String>,
    pub seller: Option<String>,
    pub account: Option<String>,
    pub option_id: Option<String>,
    pub quantity_contracts: Option<String>,
    pub premium_per_contract_native: Option<String>,
    pub raw_topics: serde_json::Value,
    pub raw_data: String,
    pub decoded: Option<serde_json::Value>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

impl OptionExecutionEvent {
    pub fn idempotency_key(&self) -> (u64, String, u64) {
        (
            self.chain_id,
            self.tx_hash.to_ascii_lowercase(),
            self.log_index,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionEventIndexerState {
    pub id: String,
    pub last_indexed_block: u64,
    pub updated_at_ms: TimestampMs,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionExecutionEventLink {
    pub intent_id: Option<Uuid>,
    pub option_execution_transaction_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionReconciliationStatus {
    Reconciled,
    PartiallyReconciled,
    ReconciliationFailed,
    MissingEvents,
    Skipped,
}

impl OptionReconciliationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reconciled => "reconciled",
            Self::PartiallyReconciled => "partially_reconciled",
            Self::ReconciliationFailed => "reconciliation_failed",
            Self::MissingEvents => "missing_events",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "reconciled" => Ok(Self::Reconciled),
            "partially_reconciled" => Ok(Self::PartiallyReconciled),
            "reconciliation_failed" => Ok(Self::ReconciliationFailed),
            "missing_events" => Ok(Self::MissingEvents),
            "skipped" => Ok(Self::Skipped),
            other => Err(BackendError::Persistence(format!(
                "invalid option reconciliation status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionExecutionReconciliation {
    pub id: Uuid,
    pub intent_id: OptionExecutionIntentId,
    pub onchain_intent_id: String,
    pub option_execution_transaction_id: String,
    pub tx_hash: String,
    pub chain_id: u64,
    pub status: OptionReconciliationStatus,
    pub strict: bool,
    pub requires_events: bool,
    pub trade_executed_event_id: Option<Uuid>,
    pub margin_trade_event_id: Option<Uuid>,
    pub trading_fee_event_count: u64,
    pub internal_transfer_event_count: u64,
    pub decoded_event_count: u64,
    pub mismatch_reason: Option<String>,
    pub missing_required: Option<String>,
    pub details: serde_json::Value,
    pub reconciled_at_ms: TimestampMs,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionConfirmationStatus {
    Pending,
    MinedSuccess,
    /// V1T legacy status for `eth_getTransactionReceipt.status == 0`.
    /// New code paths emit `MinedFailed` instead; `MinedReverted` is retained
    /// for backward compatibility with V1T-era persisted rows.
    MinedReverted,
    MinedFailed,
    ReceiptMissing,
    ReceiptError,
}

impl OptionExecutionConfirmationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::MinedSuccess => "mined_success",
            Self::MinedReverted => "mined_reverted",
            Self::MinedFailed => "mined_failed",
            Self::ReceiptMissing => "receipt_missing",
            Self::ReceiptError => "receipt_error",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "mined_success" => Ok(Self::MinedSuccess),
            "mined_reverted" => Ok(Self::MinedReverted),
            "mined_failed" => Ok(Self::MinedFailed),
            "receipt_missing" => Ok(Self::ReceiptMissing),
            "receipt_error" => Ok(Self::ReceiptError),
            other => Err(BackendError::Persistence(format!(
                "invalid option execution confirmation status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionGasCheckStatus {
    Skipped,
    Ok,
    EstimateFailed,
    BroadcastCapTooLow,
    BelowSafetyMargin,
    UncappedBroadcastRejected,
}

impl OptionExecutionGasCheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Ok => "ok",
            Self::EstimateFailed => "estimate_failed",
            Self::BroadcastCapTooLow => "broadcast_cap_too_low",
            Self::BelowSafetyMargin => "below_safety_margin",
            Self::UncappedBroadcastRejected => "uncapped_broadcast_rejected",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "skipped" => Ok(Self::Skipped),
            "ok" => Ok(Self::Ok),
            "estimate_failed" => Ok(Self::EstimateFailed),
            "broadcast_cap_too_low" => Ok(Self::BroadcastCapTooLow),
            "below_safety_margin" => Ok(Self::BelowSafetyMargin),
            "uncapped_broadcast_rejected" => Ok(Self::UncappedBroadcastRejected),
            other => Err(BackendError::Persistence(format!(
                "invalid option execution gas check status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionSeriesStatus {
    Active,
    Expired,
    Disabled,
}

impl OptionSeriesStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "disabled" => Ok(Self::Disabled),
            other => Err(BackendError::Persistence(format!(
                "invalid option series status: {other}"
            ))),
        }
    }
}

impl FromStr for OptionSeriesStatus {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "disabled" => Ok(Self::Disabled),
            other => Err(BackendError::Config(format!(
                "invalid option series status filter: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionSeriesSource {
    Manual,
    Onchain,
}

impl OptionSeriesSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Onchain => "onchain",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "manual" => Ok(Self::Manual),
            "onchain" => Ok(Self::Onchain),
            other => Err(BackendError::Persistence(format!(
                "invalid option series source: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionSeries {
    pub option_series_id: OptionSeriesId,
    pub underlying: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: Price1e8,
    pub is_call: bool,
    pub contract_size_1e8: Size1e8,
    pub status: OptionSeriesStatus,
    pub source: OptionSeriesSource,
    pub onchain_product_id: Option<String>,
    pub onchain_series_id: Option<String>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

impl OptionSeries {
    pub fn effective_status(&self, now_sec: u64) -> OptionSeriesStatus {
        if self.status == OptionSeriesStatus::Active && now_sec >= self.expiry {
            OptionSeriesStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionSeriesFilter {
    pub underlying: Option<String>,
    pub expiry: Option<u64>,
    pub is_call: Option<bool>,
    pub status: Option<OptionSeriesStatus>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionOrderStatus {
    Open,
    PartiallyFilled,
    Cancelled,
    Filled,
    Rejected,
    Expired,
}

impl OptionOrderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::PartiallyFilled => "partially_filled",
            Self::Cancelled => "cancelled",
            Self::Filled => "filled",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "partially_filled" => Ok(Self::PartiallyFilled),
            "cancelled" => Ok(Self::Cancelled),
            "filled" => Ok(Self::Filled),
            "rejected" => Ok(Self::Rejected),
            "expired" => Ok(Self::Expired),
            other => Err(BackendError::Persistence(format!(
                "invalid option order status: {other}"
            ))),
        }
    }
}

impl FromStr for OptionOrderStatus {
    type Err = BackendError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "partially_filled" => Ok(Self::PartiallyFilled),
            "cancelled" => Ok(Self::Cancelled),
            "filled" => Ok(Self::Filled),
            "rejected" => Ok(Self::Rejected),
            "expired" => Ok(Self::Expired),
            other => Err(BackendError::Config(format!(
                "invalid option order status filter: {other}"
            ))),
        }
    }
}

impl OptionOrderStatus {
    pub fn is_live(self) -> bool {
        matches!(self, Self::Open | Self::PartiallyFilled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionOrder {
    pub order_id: OptionOrderId,
    pub option_series_id: OptionSeriesId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub remaining_size_1e8: Size1e8,
    pub time_in_force: TimeInForce,
    #[serde(default)]
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub nonce: Option<u64>,
    pub deadline_ms: Option<TimestampMs>,
    pub signature: Option<String>,
    pub status: OptionOrderStatus,
    // HISTORY-V2-TERMINAL-REASONS-V1: optional persisted reason stamped
    // at terminal transitions where the cause is known (user cancel,
    // IOC remainder). NULL for live rows and for pre-existing rows from
    // before migration 0030.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason_source: Option<String>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

// HISTORY-V2-TERMINAL-REASONS-V1: stable token table for the two
// terminal causes the backend can persist honestly today. Anything
// not in this list is a pre-persistence rejection (PostOnlyWouldMatch,
// FokNotFillable, matching rejections) and never reaches the DB.
pub mod terminal_reason {
    pub const USER_CANCELLED: &str = "user_cancelled";
    pub const IOC_REMAINDER_CANCELLED: &str = "ioc_remainder_cancelled";

    pub const SOURCE_USER: &str = "user";
    pub const SOURCE_TIF_POLICY: &str = "tif_policy";
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionFill {
    pub fill_id: OptionFillId,
    pub option_series_id: OptionSeriesId,
    pub buy_order_id: OptionOrderId,
    pub sell_order_id: OptionOrderId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub maker_order_id: OptionOrderId,
    pub taker_order_id: OptionOrderId,
    pub taker_side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub created_at_ms: TimestampMs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionRfqStatus {
    Open,
    Expired,
    Accepted,
    Cancelled,
    Failed,
}

impl OptionRfqStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Expired => "expired",
            Self::Accepted => "accepted",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "expired" => Ok(Self::Expired),
            "accepted" => Ok(Self::Accepted),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => Err(BackendError::Persistence(format!(
                "invalid option RFQ status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionRfqQuoteStatus {
    Active,
    Expired,
    Accepted,
    Rejected,
    Cancelled,
}

impl OptionRfqQuoteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "expired" => Ok(Self::Expired),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(BackendError::Persistence(format!(
                "invalid option RFQ quote status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionRfqQuoteSignatureStatus {
    NotRequired,
    Verified,
    Missing,
    Invalid,
    SignerMismatch,
}

impl OptionRfqQuoteSignatureStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Verified => "verified",
            Self::Missing => "missing",
            Self::Invalid => "invalid",
            Self::SignerMismatch => "signer_mismatch",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "not_required" => Ok(Self::NotRequired),
            "verified" => Ok(Self::Verified),
            "missing" => Ok(Self::Missing),
            "invalid" => Ok(Self::Invalid),
            "signer_mismatch" => Ok(Self::SignerMismatch),
            other => Err(BackendError::Persistence(format!(
                "invalid option RFQ quote signature status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionRfqRequest {
    pub option_rfq_id: OptionRfqId,
    pub taker: AccountId,
    pub option_series_id: OptionSeriesId,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub limit_price_1e8: Option<Price1e8>,
    pub status: OptionRfqStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub accepted_quote_id: Option<OptionRfqQuoteId>,
    pub option_fill_id: Option<OptionRfqFillId>,
}

impl OptionRfqRequest {
    pub fn effective_status(&self, now_ms: TimestampMs) -> OptionRfqStatus {
        if self.status == OptionRfqStatus::Open && now_ms >= self.expires_at_ms {
            OptionRfqStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionRfqQuote {
    pub quote_id: OptionRfqQuoteId,
    pub option_rfq_id: OptionRfqId,
    pub mm_account: AccountId,
    pub session_id: Option<String>,
    pub client_quote_id: Option<String>,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub status: OptionRfqQuoteStatus,
    pub created_at_ms: TimestampMs,
    pub expires_at_ms: TimestampMs,
    pub signature: Option<String>,
    pub quote_digest: Option<String>,
    pub quote_nonce: Option<String>,
    pub signature_status: OptionRfqQuoteSignatureStatus,
    pub recovered_signer: Option<AccountId>,
}

impl OptionRfqQuote {
    pub fn effective_status(&self, now_ms: TimestampMs) -> OptionRfqQuoteStatus {
        if self.status == OptionRfqQuoteStatus::Active && now_ms >= self.expires_at_ms {
            OptionRfqQuoteStatus::Expired
        } else {
            self.status
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptionRfqFill {
    pub fill_id: OptionRfqFillId,
    pub option_rfq_id: OptionRfqId,
    pub quote_id: OptionRfqQuoteId,
    pub option_series_id: OptionSeriesId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub taker: AccountId,
    pub mm_account: AccountId,
    pub taker_side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub created_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionOrderFilter {
    pub option_series_id: Option<OptionSeriesId>,
    pub account: Option<AccountId>,
    pub status: Option<OptionOrderStatus>,
    pub side: Option<Side>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptionFillFilter {
    pub option_series_id: Option<OptionSeriesId>,
    pub account: Option<AccountId>,
    pub order_id: Option<OptionOrderId>,
}

impl OptionFillFilter {
    pub fn matches(&self, fill: &OptionFill) -> bool {
        if let Some(option_series_id) = &self.option_series_id {
            if &fill.option_series_id != option_series_id {
                return false;
            }
        }
        if let Some(account) = &self.account {
            if !fill.buyer.0.eq_ignore_ascii_case(&account.0)
                && !fill.seller.0.eq_ignore_ascii_case(&account.0)
            {
                return false;
            }
        }
        if let Some(order_id) = self.order_id {
            if fill.buy_order_id != order_id
                && fill.sell_order_id != order_id
                && fill.maker_order_id != order_id
                && fill.taker_order_id != order_id
            {
                return false;
            }
        }
        true
    }
}

impl OptionOrderFilter {
    pub fn matches(&self, order: &OptionOrder) -> bool {
        if let Some(option_series_id) = &self.option_series_id {
            if &order.option_series_id != option_series_id {
                return false;
            }
        }
        if let Some(account) = &self.account {
            if !order.account.0.eq_ignore_ascii_case(&account.0) {
                return false;
            }
        }
        if let Some(status) = self.status {
            if order.status != status {
                return false;
            }
        }
        if let Some(side) = self.side {
            if order.side != side {
                return false;
            }
        }
        true
    }
}

impl OptionSeriesFilter {
    pub fn matches(&self, series: &OptionSeries, now_sec: u64) -> bool {
        if let Some(underlying) = &self.underlying {
            if !series.underlying.eq_ignore_ascii_case(underlying) {
                return false;
            }
        }
        if let Some(expiry) = self.expiry {
            if series.expiry != expiry {
                return false;
            }
        }
        if let Some(is_call) = self.is_call {
            if series.is_call != is_call {
                return false;
            }
        }
        if let Some(status) = self.status {
            if series.effective_status(now_sec) != status {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionOrderbookSnapshot {
    pub option_series_id: OptionSeriesId,
    pub status: OptionSeriesStatus,
    pub bids: Vec<OptionOrderbookLevel>,
    pub asks: Vec<OptionOrderbookLevel>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionOrderbookLevel {
    pub price_1e8: String,
    pub size_1e8: String,
}
