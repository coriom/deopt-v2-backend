//! Admin-only lifecycle aggregation for option execution intents.
//!
//! V1Z adds a single read-only entry point that stitches together every
//! persisted artefact for one `intent_id`: the intent itself, its
//! signature/simulation/calldata state, the broadcast transaction, the
//! receipt/confirmation fields, the indexed events, and the
//! reconciliation row. It never writes, broadcasts, or calls RPC; it
//! never queries `execution_transactions`. It exists so an operator can
//! verify the full V1S-through-V1Y pipeline with a single GET.

use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::execution::ExecutionTransactionStatus;
use serde::Serialize;
use std::collections::BTreeMap;
use uuid::Uuid;

use super::{
    OptionExecutionConfirmationStatus, OptionExecutionEvent, OptionExecutionIntent,
    OptionExecutionIntentId, OptionExecutionIntentStatus, OptionExecutionReconciliation,
    OptionExecutionSimulationStatus, OptionExecutionTransaction, OptionReconciliationStatus,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionLifecycleStage {
    IntentCreated,
    SignaturesReady,
    CalldataReady,
    SimulationOk,
    BroadcastSubmitted,
    MinedSuccess,
    EventsIndexed,
    Reconciled,
    Failed,
}

impl OptionExecutionLifecycleStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IntentCreated => "intent_created",
            Self::SignaturesReady => "signatures_ready",
            Self::CalldataReady => "calldata_ready",
            Self::SimulationOk => "simulation_ok",
            Self::BroadcastSubmitted => "broadcast_submitted",
            Self::MinedSuccess => "mined_success",
            Self::EventsIndexed => "events_indexed",
            Self::Reconciled => "reconciled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionExecutionLifecycleHealth {
    pub stage: OptionExecutionLifecycleStage,
    pub is_terminal_success: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionExecutionLifecycle {
    pub intent_id: OptionExecutionIntentId,
    pub status: OptionExecutionIntentStatus,
    pub source: LifecycleSource,
    pub trade: LifecycleTrade,
    pub metadata: LifecycleMetadata,
    pub signatures: LifecycleSignatures,
    pub simulation: LifecycleSimulation,
    pub calldata: LifecycleCalldata,
    pub broadcast: Option<LifecycleBroadcast>,
    pub confirmation: Option<LifecycleConfirmation>,
    pub events: LifecycleEvents,
    pub fees: LifecycleFees,
    pub transfers: LifecycleTransfers,
    pub reconciliation: Option<LifecycleReconciliation>,
    pub state_checks: Option<LifecycleStateChecks>,
    pub health: OptionExecutionLifecycleHealth,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleSource {
    pub source_type: String,
    pub source_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleTrade {
    pub buyer: String,
    pub seller: String,
    pub option_id: String,
    pub quantity_contracts: String,
    pub premium_per_contract_native: String,
    pub buyer_is_maker: bool,
    pub onchain_intent_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleMetadata {
    pub underlying: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: u128,
    pub contract_size_1e8: u128,
    pub is_call: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleSignatures {
    pub buyer_signature_present: bool,
    pub seller_signature_present: bool,
    pub signature_mode: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleSimulation {
    pub status: Option<OptionExecutionSimulationStatus>,
    pub block_number: Option<u64>,
    pub simulated_at_ms: Option<i64>,
    pub error: Option<String>,
    pub revert_selector: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleCalldata {
    pub present: bool,
    pub selector: Option<String>,
    /// Length in hex characters (after the `0x` prefix). A 4-byte selector
    /// + N ABI words is `8 + 64 * N` hex chars.
    pub hex_length: usize,
    /// Length in raw bytes.
    pub byte_length: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleBroadcast {
    pub transaction_id: String,
    pub tx_hash: Option<String>,
    pub status: ExecutionTransactionStatus,
    pub from: String,
    pub to: String,
    pub gas_limit: Option<u64>,
    pub gas_check_status: Option<String>,
    pub gas_check_error: Option<String>,
    pub estimated_gas: Option<u64>,
    pub required_gas: Option<u64>,
    pub simulation_gas_limit: Option<u64>,
    pub broadcast_gas_limit: Option<u64>,
    pub gas_safety_bps: Option<u32>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleConfirmation {
    pub confirmation_status: OptionExecutionConfirmationStatus,
    pub receipt_status: Option<u64>,
    pub confirmed_block_number: Option<u64>,
    pub confirmed_at_ms: Option<i64>,
    pub confirmation_error: Option<String>,
    pub gas_used: Option<u64>,
    pub effective_gas_price: Option<String>,
    pub cumulative_gas_used: Option<u64>,
    pub receipt_block_hash: Option<String>,
    pub receipt_transaction_index: Option<u64>,
    pub receipt_observed_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LifecycleEvents {
    pub total: u64,
    pub counts_by_event_name: BTreeMap<String, u64>,
    pub counts_by_contract_address: BTreeMap<String, u64>,
    pub recent: Vec<OptionExecutionEvent>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LifecycleFees {
    /// Always `"onchain"` while live trades exist: the `TradingFeeCharged`
    /// log from `MarginEngine.applyTrade` is the canonical record of what
    /// the protocol charged. Backend fee ledger entries are informational
    /// and may legitimately be missing for purely on-chain reconciliation.
    pub source_of_truth: String,
    pub trading_fee_event_count: u64,
    pub events: Vec<serde_json::Value>,
    /// Sum of `appliedFee` across every observed `TradingFeeCharged` log on
    /// this transaction, as a base-10 string (settlement-asset native units).
    pub observed_total: String,
    /// Per-trader sum of `appliedFee` (trader = decoded `trader` field,
    /// falls back to the event's `account` column for older indexed rows).
    pub by_trader: BTreeMap<String, String>,
    /// Per-recipient sum of `appliedFee`. Same shape as the legacy
    /// `total_by_recipient` map and kept in sync with it.
    pub by_recipient: BTreeMap<String, String>,
    /// Per-side sum of `appliedFee` keyed by `"maker"` / `"taker"`
    /// (or `"unknown"` when `isMaker` is absent).
    pub by_side: BTreeMap<String, String>,
    /// Backward-compatible alias of `by_recipient` retained for clients
    /// still consuming the V1Z field name.
    pub total_by_recipient: BTreeMap<String, String>,
    /// Explicit status describing whether the backend ledger (the off-chain
    /// `fee_events` table populated by `fees::service`) has any matching
    /// rows for this intent's `source_id`. `"disabled"` when the fee module
    /// is off; `"missing_or_disabled"` when enabled but no row matches the
    /// intent's source; `"present"` when at least one row is keyed to the
    /// same source. A missing ledger row is never an error here — the
    /// on-chain event is the source of truth.
    pub backend_ledger_status: String,
    /// Indicates whether on-chain fee events have been observed:
    /// `"onchain_observed"` when at least one `TradingFeeCharged` event
    /// was indexed for this tx, `"no_onchain_events"` otherwise.
    pub reconciliation_status: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LifecycleTransfers {
    pub internal_transfer_count: u64,
    pub events: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleReconciliation {
    pub id: Uuid,
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
    pub reconciled_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LifecycleStateChecks {
    pub state_check_status: String,
    pub nonce_check_status: String,
    pub position_check_status: String,
    pub vault_check_status: String,
    pub strict: bool,
    pub details: serde_json::Value,
}

/// Aggregate the persisted lifecycle for a single option execution intent.
///
/// Read-only: queries `option_execution_intents`,
/// `option_execution_transactions`, `option_execution_events`, and
/// `option_execution_reconciliations`. Returns
/// [`BackendError::InvalidOptionExecutionIntentId`] when no intent row
/// matches; the route layer turns that into HTTP 404.
pub async fn get_option_execution_lifecycle(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionLifecycle> {
    let intent = load_intent(state, intent_id)
        .await?
        .ok_or(BackendError::InvalidOptionExecutionIntentId)?;
    let transactions = load_transactions_for_intent(state, intent_id).await?;
    let broadcast = transactions.first().cloned();
    let confirmation = broadcast
        .as_ref()
        .and_then(|tx| tx.confirmation_status.map(|status| (tx.clone(), status)));

    let mut events: Vec<OptionExecutionEvent> = Vec::new();
    if let Some(tx) = broadcast.as_ref() {
        if let Some(hash) = tx.tx_hash.as_deref() {
            if !hash.is_empty() {
                events = load_events_for_tx(state, hash).await?;
            }
        }
    }

    let reconciliation = if let Some(tx) = broadcast.as_ref() {
        load_reconciliation(state, &tx.transaction_id).await?
    } else {
        None
    };

    let calldata = build_calldata_view(&intent);
    let signatures_view = build_signatures_view(state, &intent);
    let trade_view = LifecycleTrade {
        buyer: intent.buyer.0.clone(),
        seller: intent.seller.0.clone(),
        option_id: intent.onchain_option_id.clone(),
        quantity_contracts: intent.quantity_contracts.to_string(),
        premium_per_contract_native: intent.premium_per_contract_native.to_string(),
        buyer_is_maker: intent.buyer_is_maker,
        onchain_intent_id: intent.onchain_intent_id.clone(),
    };
    let metadata_view = LifecycleMetadata {
        underlying: intent.underlying.0.clone(),
        settlement_asset: intent.settlement_asset.0.clone(),
        expiry: intent.expiry,
        strike_1e8: intent.strike_1e8,
        contract_size_1e8: intent.contract_size_1e8,
        is_call: intent.is_call,
    };
    let simulation_view = LifecycleSimulation {
        status: intent.simulation_status,
        block_number: intent.simulation_block_number,
        simulated_at_ms: intent.simulated_at_ms,
        error: intent.simulation_error.clone(),
        revert_selector: intent.simulation_revert_selector.clone(),
    };
    let source_view = LifecycleSource {
        source_type: intent.source_type.as_str().to_string(),
        source_id: intent.source_id.clone(),
    };

    let events_view = build_events_view(&events);
    let backend_ledger_status = resolve_backend_ledger_status(state, &intent).await?;
    let fees_view = build_fees_view(&events, &backend_ledger_status);
    let transfers_view = build_transfers_view(&events);

    let broadcast_view = broadcast.as_ref().map(|tx| LifecycleBroadcast {
        transaction_id: tx.transaction_id.clone(),
        tx_hash: tx.tx_hash.clone(),
        status: tx.status,
        from: tx.from.0.clone(),
        to: tx.to.0.clone(),
        gas_limit: tx.gas_limit,
        gas_check_status: tx
            .gas_check_status
            .map(|status| status.as_str().to_string()),
        gas_check_error: tx.gas_check_error.clone(),
        estimated_gas: tx.estimated_gas,
        required_gas: tx.required_gas,
        simulation_gas_limit: tx.simulation_gas_limit,
        broadcast_gas_limit: tx.broadcast_gas_limit,
        gas_safety_bps: tx.gas_safety_bps,
        created_at_ms: tx.created_at_ms,
        updated_at_ms: tx.updated_at_ms,
    });

    let confirmation_view = confirmation
        .as_ref()
        .map(|(tx, status)| LifecycleConfirmation {
            confirmation_status: *status,
            receipt_status: tx.receipt_status,
            confirmed_block_number: tx.confirmed_block_number,
            confirmed_at_ms: tx.confirmed_at_ms,
            confirmation_error: tx.confirmation_error.clone(),
            gas_used: tx.gas_used,
            effective_gas_price: tx.effective_gas_price.clone(),
            cumulative_gas_used: tx.cumulative_gas_used,
            receipt_block_hash: tx.receipt_block_hash.clone(),
            receipt_transaction_index: tx.receipt_transaction_index,
            receipt_observed_at_ms: tx.receipt_observed_at_ms,
        });

    let reconciliation_view = reconciliation.as_ref().map(|row| LifecycleReconciliation {
        id: row.id,
        status: row.status,
        strict: row.strict,
        requires_events: row.requires_events,
        trade_executed_event_id: row.trade_executed_event_id,
        margin_trade_event_id: row.margin_trade_event_id,
        trading_fee_event_count: row.trading_fee_event_count,
        internal_transfer_event_count: row.internal_transfer_event_count,
        decoded_event_count: row.decoded_event_count,
        mismatch_reason: row.mismatch_reason.clone(),
        missing_required: row.missing_required.clone(),
        reconciled_at_ms: row.reconciled_at_ms,
        updated_at_ms: row.updated_at_ms,
    });
    let state_checks_view = reconciliation.as_ref().and_then(build_state_checks_view);

    let health = compute_health(
        &intent,
        broadcast.as_ref(),
        confirmation.as_ref().map(|(_, status)| *status),
        events.len() as u64,
        reconciliation.as_ref(),
        state_checks_view.as_ref(),
        &calldata,
    );

    Ok(OptionExecutionLifecycle {
        intent_id,
        status: intent.status,
        source: source_view,
        trade: trade_view,
        metadata: metadata_view,
        signatures: signatures_view,
        simulation: simulation_view,
        calldata,
        broadcast: broadcast_view,
        confirmation: confirmation_view,
        events: events_view,
        fees: fees_view,
        transfers: transfers_view,
        reconciliation: reconciliation_view,
        state_checks: state_checks_view,
        health,
    })
}

async fn load_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<Option<OptionExecutionIntent>> {
    if let Some(repository) = state.repository.clone() {
        return repository.get_option_execution_intent(intent_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_execution_intent(intent_id))
}

async fn load_transactions_for_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<Vec<OptionExecutionTransaction>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_execution_transactions_for_intent(intent_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .option_execution_transactions_for_intent(intent_id))
}

async fn load_events_for_tx(state: &AppState, tx_hash: &str) -> Result<Vec<OptionExecutionEvent>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .list_option_execution_events_by_tx_hash(tx_hash)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_execution_events_by_tx_hash(tx_hash))
}

async fn load_reconciliation(
    state: &AppState,
    transaction_id: &str,
) -> Result<Option<OptionExecutionReconciliation>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_execution_reconciliation_by_transaction_id(transaction_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_execution_reconciliation_by_transaction_id(transaction_id))
}

fn build_signatures_view(state: &AppState, intent: &OptionExecutionIntent) -> LifecycleSignatures {
    LifecycleSignatures {
        buyer_signature_present: intent
            .buyer_signature
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false),
        seller_signature_present: intent
            .seller_signature
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false),
        signature_mode: format!("{:?}", state.options_config.execution_signature_mode)
            .to_ascii_lowercase(),
    }
}

fn build_calldata_view(intent: &OptionExecutionIntent) -> LifecycleCalldata {
    match intent.calldata.as_deref().filter(|value| !value.is_empty()) {
        None => LifecycleCalldata {
            present: false,
            selector: None,
            hex_length: 0,
            byte_length: 0,
        },
        Some(calldata) => {
            let hex_body = calldata.strip_prefix("0x").unwrap_or(calldata);
            let hex_length = hex_body.len();
            let byte_length = hex_length / 2;
            let selector = if hex_length >= 8 {
                Some(format!("0x{}", &hex_body[..8].to_ascii_lowercase()))
            } else {
                None
            };
            LifecycleCalldata {
                present: true,
                selector,
                hex_length,
                byte_length,
            }
        }
    }
}

fn build_events_view(events: &[OptionExecutionEvent]) -> LifecycleEvents {
    let mut counts_by_event_name: BTreeMap<String, u64> = BTreeMap::new();
    let mut counts_by_contract_address: BTreeMap<String, u64> = BTreeMap::new();
    for event in events {
        *counts_by_event_name
            .entry(event.event_name.clone())
            .or_default() += 1;
        *counts_by_contract_address
            .entry(event.contract_address.to_ascii_lowercase())
            .or_default() += 1;
    }
    let mut recent = events.to_vec();
    recent.sort_by(|left, right| {
        right
            .block_number
            .cmp(&left.block_number)
            .then_with(|| right.log_index.cmp(&left.log_index))
            .then_with(|| right.id.cmp(&left.id))
    });
    recent.truncate(20);
    LifecycleEvents {
        total: events.len() as u64,
        counts_by_event_name,
        counts_by_contract_address,
        recent,
    }
}

fn build_fees_view(events: &[OptionExecutionEvent], backend_ledger_status: &str) -> LifecycleFees {
    let mut by_recipient: BTreeMap<String, u128> = BTreeMap::new();
    let mut by_trader: BTreeMap<String, u128> = BTreeMap::new();
    let mut by_side: BTreeMap<String, u128> = BTreeMap::new();
    let mut observed_total: u128 = 0;
    let mut fee_events: Vec<serde_json::Value> = Vec::new();
    let mut trading_fee_event_count: u64 = 0;
    for event in events {
        if event.event_name != "TradingFeeCharged" {
            continue;
        }
        trading_fee_event_count += 1;
        let decoded = event.decoded.clone().unwrap_or(serde_json::Value::Null);
        let applied_fee_str = decoded
            .get("appliedFee")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let recipient = decoded
            .get("recipient")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "unknown".to_string());
        let trader = decoded
            .get("trader")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .or_else(|| {
                event
                    .account
                    .as_ref()
                    .map(|value| value.to_ascii_lowercase())
            })
            .unwrap_or_else(|| "unknown".to_string());
        let is_maker_value = decoded
            .get("isMaker")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let side_key = match is_maker_value.as_bool() {
            Some(true) => "maker",
            Some(false) => "taker",
            None => "unknown",
        };
        if let Some(value) = applied_fee_str.as_deref() {
            if let Ok(parsed) = value.parse::<u128>() {
                observed_total = observed_total.saturating_add(parsed);
                let entry = by_recipient.entry(recipient.clone()).or_default();
                *entry = entry.saturating_add(parsed);
                let trader_entry = by_trader.entry(trader.clone()).or_default();
                *trader_entry = trader_entry.saturating_add(parsed);
                let side_entry = by_side.entry(side_key.to_string()).or_default();
                *side_entry = side_entry.saturating_add(parsed);
            }
        }
        fee_events.push(serde_json::json!({
            "event_id": event.id,
            "log_index": event.log_index,
            "block_number": event.block_number,
            "trader": trader,
            "recipient": recipient,
            "applied_fee": applied_fee_str,
            "is_maker": is_maker_value,
            "side": side_key,
            "option_id": event.option_id,
            "settlement_asset": decoded.get("settlementAsset").cloned().unwrap_or(serde_json::Value::Null),
        }));
    }
    let recipient_strings: BTreeMap<String, String> = by_recipient
        .into_iter()
        .map(|(key, value)| (key, value.to_string()))
        .collect();
    let trader_strings: BTreeMap<String, String> = by_trader
        .into_iter()
        .map(|(key, value)| (key, value.to_string()))
        .collect();
    let side_strings: BTreeMap<String, String> = by_side
        .into_iter()
        .map(|(key, value)| (key, value.to_string()))
        .collect();
    let reconciliation_status = if trading_fee_event_count == 0 {
        "no_onchain_events"
    } else {
        "onchain_observed"
    };
    LifecycleFees {
        source_of_truth: "onchain".to_string(),
        trading_fee_event_count,
        events: fee_events,
        observed_total: observed_total.to_string(),
        by_trader: trader_strings,
        by_recipient: recipient_strings.clone(),
        by_side: side_strings,
        total_by_recipient: recipient_strings,
        backend_ledger_status: backend_ledger_status.to_string(),
        reconciliation_status: reconciliation_status.to_string(),
    }
}

async fn resolve_backend_ledger_status(
    state: &AppState,
    intent: &OptionExecutionIntent,
) -> Result<String> {
    if !state.fees_config.enabled {
        return Ok("disabled".to_string());
    }
    let fee_source_type = match intent.source_type {
        crate::options::OptionExecutionSourceType::OptionOrderbookFill => "option_order_fill",
        crate::options::OptionExecutionSourceType::OptionRfqFill => "option_rfq_fill",
    };
    let matches =
        count_backend_fee_events(state, fee_source_type, intent.source_id.as_str()).await?;
    if matches == 0 {
        Ok("missing_or_disabled".to_string())
    } else {
        Ok("present".to_string())
    }
}

async fn count_backend_fee_events(
    state: &AppState,
    source_type: &str,
    source_id: &str,
) -> Result<u64> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .admin_count_fee_events_by_source(source_type, source_id)
            .await;
    }
    let store = state
        .fees_store
        .lock()
        .map_err(|_| BackendError::Config("fee ledger store lock poisoned".to_string()))?;
    Ok(store.count_fee_events_by_source(source_type, source_id))
}

fn build_transfers_view(events: &[OptionExecutionEvent]) -> LifecycleTransfers {
    let mut internal_transfer_count: u64 = 0;
    let mut transfer_events: Vec<serde_json::Value> = Vec::new();
    for event in events {
        if event.event_name != "InternalTransfer" {
            continue;
        }
        internal_transfer_count += 1;
        let decoded = event.decoded.clone().unwrap_or(serde_json::Value::Null);
        transfer_events.push(serde_json::json!({
            "event_id": event.id,
            "log_index": event.log_index,
            "block_number": event.block_number,
            "from": decoded.get("from").cloned().unwrap_or(serde_json::Value::Null),
            "to": decoded.get("to").cloned().unwrap_or(serde_json::Value::Null),
            "amount": event.premium_per_contract_native,
            "token": decoded.get("token").cloned().unwrap_or(serde_json::Value::Null),
        }));
    }
    LifecycleTransfers {
        internal_transfer_count,
        events: transfer_events,
    }
}

fn build_state_checks_view(row: &OptionExecutionReconciliation) -> Option<LifecycleStateChecks> {
    let details = row.details.get("state_checks")?.clone();
    let state_check_status = details
        .get("overall_status")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let nonce_check_status = details
        .get("nonce_check_status")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let position_check_status = details
        .get("position_check_status")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let vault_check_status = details
        .get("vault_check_status")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let strict = details
        .get("strict")
        .and_then(|value| value.as_bool())
        .unwrap_or(row.strict);
    Some(LifecycleStateChecks {
        state_check_status,
        nonce_check_status,
        position_check_status,
        vault_check_status,
        strict,
        details,
    })
}

fn compute_health(
    intent: &OptionExecutionIntent,
    broadcast: Option<&OptionExecutionTransaction>,
    confirmation_status: Option<OptionExecutionConfirmationStatus>,
    event_count: u64,
    reconciliation: Option<&OptionExecutionReconciliation>,
    state_checks: Option<&LifecycleStateChecks>,
    calldata: &LifecycleCalldata,
) -> OptionExecutionLifecycleHealth {
    let mut warnings: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut stage = OptionExecutionLifecycleStage::IntentCreated;
    let mut is_terminal_success = false;

    // Walk forward through the pipeline. Failure short-circuits to Failed.
    if matches!(intent.status, OptionExecutionIntentStatus::Failed) {
        errors.push(format!("intent.status = {}", intent.status.as_str()));
        return OptionExecutionLifecycleHealth {
            stage: OptionExecutionLifecycleStage::Failed,
            is_terminal_success: false,
            warnings,
            errors,
        };
    }
    if matches!(intent.status, OptionExecutionIntentStatus::Cancelled) {
        warnings.push("intent.status = cancelled".to_string());
        return OptionExecutionLifecycleHealth {
            stage: OptionExecutionLifecycleStage::Failed,
            is_terminal_success: false,
            warnings,
            errors,
        };
    }
    if matches!(intent.status, OptionExecutionIntentStatus::BroadcastFailed) {
        errors.push("intent.status = broadcast_failed".to_string());
        return OptionExecutionLifecycleHealth {
            stage: OptionExecutionLifecycleStage::Failed,
            is_terminal_success: false,
            warnings,
            errors,
        };
    }

    let buyer_signed = intent
        .buyer_signature
        .as_deref()
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    let seller_signed = intent
        .seller_signature
        .as_deref()
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    if buyer_signed && seller_signed {
        stage = OptionExecutionLifecycleStage::SignaturesReady;
    }
    if calldata.present {
        stage = OptionExecutionLifecycleStage::CalldataReady;
    }
    if matches!(
        intent.simulation_status,
        Some(OptionExecutionSimulationStatus::SimulationOk)
    ) {
        stage = OptionExecutionLifecycleStage::SimulationOk;
    }
    if matches!(
        intent.simulation_status,
        Some(OptionExecutionSimulationStatus::SimulationFailed)
    ) {
        warnings.push("simulation failed".to_string());
    }

    let Some(tx) = broadcast else {
        return OptionExecutionLifecycleHealth {
            stage,
            is_terminal_success: false,
            warnings,
            errors,
        };
    };
    stage = OptionExecutionLifecycleStage::BroadcastSubmitted;
    if matches!(
        intent.status,
        OptionExecutionIntentStatus::BroadcastReverted
            | OptionExecutionIntentStatus::BroadcastFailed
    ) {
        errors.push(format!("intent.status = {}", intent.status.as_str()));
        return OptionExecutionLifecycleHealth {
            stage: OptionExecutionLifecycleStage::Failed,
            is_terminal_success: false,
            warnings,
            errors,
        };
    }
    if matches!(tx.status, ExecutionTransactionStatus::Failed) {
        errors.push("transaction.status = failed".to_string());
        return OptionExecutionLifecycleHealth {
            stage: OptionExecutionLifecycleStage::Failed,
            is_terminal_success: false,
            warnings,
            errors,
        };
    }

    match confirmation_status {
        Some(OptionExecutionConfirmationStatus::MinedSuccess) => {
            stage = OptionExecutionLifecycleStage::MinedSuccess;
        }
        Some(OptionExecutionConfirmationStatus::MinedFailed)
        | Some(OptionExecutionConfirmationStatus::MinedReverted) => {
            errors.push(format!(
                "confirmation_status = {}",
                confirmation_status.unwrap().as_str()
            ));
            return OptionExecutionLifecycleHealth {
                stage: OptionExecutionLifecycleStage::Failed,
                is_terminal_success: false,
                warnings,
                errors,
            };
        }
        Some(OptionExecutionConfirmationStatus::ReceiptMissing)
        | Some(OptionExecutionConfirmationStatus::ReceiptError) => {
            warnings.push(format!(
                "confirmation_status = {}",
                confirmation_status.unwrap().as_str()
            ));
        }
        Some(OptionExecutionConfirmationStatus::Pending) | None => {}
    }

    if matches!(stage, OptionExecutionLifecycleStage::MinedSuccess) {
        if event_count == 0 {
            warnings.push("missing_indexed_events".to_string());
        } else {
            stage = OptionExecutionLifecycleStage::EventsIndexed;
            if reconciliation.is_none() {
                warnings.push("not_reconciled".to_string());
            }
        }
    }

    if let Some(row) = reconciliation {
        match row.status {
            OptionReconciliationStatus::Reconciled => {
                stage = OptionExecutionLifecycleStage::Reconciled;
                is_terminal_success = true;
            }
            OptionReconciliationStatus::PartiallyReconciled => {
                warnings.push("reconciliation_partial".to_string());
            }
            OptionReconciliationStatus::MissingEvents => {
                warnings.push("reconciliation_missing_events".to_string());
            }
            OptionReconciliationStatus::ReconciliationFailed => {
                errors.push(format!(
                    "reconciliation_failed: {}",
                    row.mismatch_reason.clone().unwrap_or_default()
                ));
            }
            OptionReconciliationStatus::Skipped => {
                warnings.push("reconciliation_skipped".to_string());
            }
        }
    }

    if let Some(checks) = state_checks {
        match checks.state_check_status.as_str() {
            "ok" => {}
            "failed" => {
                let message = "state_checks_failed".to_string();
                if checks.strict {
                    errors.push(message);
                    stage = OptionExecutionLifecycleStage::Failed;
                    is_terminal_success = false;
                } else {
                    warnings.push(message);
                }
            }
            "skipped" => warnings.push("state_checks_skipped".to_string()),
            "warning" => warnings.push("state_checks_warning".to_string()),
            other => warnings.push(format!("state_checks_{other}")),
        }
    }

    OptionExecutionLifecycleHealth {
        stage,
        is_terminal_success,
        warnings,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineState;
    use crate::execution::ExecutionTransactionStatus;
    use crate::options::{
        OptionExecutionConfirmationStatus, OptionExecutionIntent, OptionExecutionIntentStatus,
        OptionExecutionSimulationStatus, OptionExecutionSourceType, OptionExecutionTransaction,
        OptionsConfig, OPTION_TRADE_EXECUTED_SIGNATURE,
    };
    use crate::types::AccountId;

    #[tokio::test]
    async fn unknown_intent_returns_invalid_intent_id_error() {
        let state = fresh_state();
        let err = get_option_execution_lifecycle(&state, Uuid::from_u128(999))
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidOptionExecutionIntentId));
    }

    #[tokio::test]
    async fn intent_with_no_tx_reports_pre_broadcast_stage() {
        let state = fresh_state();
        let intent = signed_calldata_ready_intent();
        let intent_id = intent.intent_id;
        state
            .options_store
            .lock()
            .unwrap()
            .insert_option_execution_intent(intent);

        let view = get_option_execution_lifecycle(&state, intent_id)
            .await
            .unwrap();

        assert_eq!(view.intent_id, intent_id);
        assert!(view.broadcast.is_none());
        assert!(view.confirmation.is_none());
        assert!(view.reconciliation.is_none());
        assert_eq!(view.events.total, 0);
        assert!(view.calldata.present);
        assert_eq!(view.calldata.selector.as_deref(), Some("0x031f77b3"));
        assert_eq!(view.calldata.byte_length, 4);
        assert_eq!(
            view.health.stage,
            OptionExecutionLifecycleStage::SimulationOk
        );
        assert!(!view.health.is_terminal_success);
    }

    #[tokio::test]
    async fn mined_success_without_events_warns_missing_events() {
        let state = fresh_state();
        let (intent, _) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert!(view.broadcast.is_some());
        let confirmation = view.confirmation.unwrap();
        assert_eq!(
            confirmation.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        assert_eq!(view.events.total, 0);
        assert_eq!(
            view.health.stage,
            OptionExecutionLifecycleStage::MinedSuccess
        );
        assert!(!view.health.is_terminal_success);
        assert!(view
            .health
            .warnings
            .iter()
            .any(|warning| warning == "missing_indexed_events"));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn indexed_events_without_reconciliation_warns_not_reconciled() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        insert_event(
            &state,
            trading_fee_event(&intent, &tx.tx_hash.clone().unwrap(), 5),
        );
        insert_event(
            &state,
            internal_transfer_event(&tx.tx_hash.clone().unwrap(), 6),
        );

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(view.events.total, 3);
        assert_eq!(view.fees.trading_fee_event_count, 1);
        assert_eq!(view.transfers.internal_transfer_count, 1);
        assert_eq!(
            view.health.stage,
            OptionExecutionLifecycleStage::EventsIndexed
        );
        assert!(view
            .health
            .warnings
            .iter()
            .any(|warning| warning == "not_reconciled"));
        assert!(!view.health.is_terminal_success);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn reconciled_terminal_success() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        insert_event(
            &state,
            margin_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        insert_event(
            &state,
            trading_fee_event(&intent, &tx.tx_hash.clone().unwrap(), 5),
        );
        insert_reconciliation(
            &state,
            &intent,
            &tx,
            OptionReconciliationStatus::Reconciled,
            Some(Uuid::from_u128(100)),
            Some(Uuid::from_u128(101)),
        );

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        let reconciliation = view.reconciliation.unwrap();
        assert_eq!(
            reconciliation.status,
            OptionReconciliationStatus::Reconciled
        );
        assert_eq!(view.health.stage, OptionExecutionLifecycleStage::Reconciled);
        assert!(view.health.is_terminal_success);
        assert!(view.health.warnings.is_empty());
        assert!(view.health.errors.is_empty());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn reconciliation_missing_events_surfaces_warning() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        insert_reconciliation(
            &state,
            &intent,
            &tx,
            OptionReconciliationStatus::MissingEvents,
            None,
            None,
        );

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        let reconciliation = view.reconciliation.unwrap();
        assert_eq!(
            reconciliation.status,
            OptionReconciliationStatus::MissingEvents
        );
        assert!(!view.health.is_terminal_success);
        assert!(view
            .health
            .warnings
            .iter()
            .any(|warning| warning == "reconciliation_missing_events"));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn reconciliation_failed_surfaces_error() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        let mut row = build_reconciliation(
            &intent,
            &tx,
            OptionReconciliationStatus::ReconciliationFailed,
            None,
            None,
        );
        row.mismatch_reason = Some("buyer mismatch".to_string());
        state
            .options_store
            .lock()
            .unwrap()
            .upsert_option_execution_reconciliation(row, 99);

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(
            view.reconciliation.as_ref().unwrap().status,
            OptionReconciliationStatus::ReconciliationFailed
        );
        assert!(view
            .health
            .errors
            .iter()
            .any(|error| error.starts_with("reconciliation_failed")));
        assert!(!view.health.is_terminal_success);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn lifecycle_includes_state_checks() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        let mut row = build_reconciliation(
            &intent,
            &tx,
            OptionReconciliationStatus::Reconciled,
            Some(Uuid::from_u128(100)),
            None,
        );
        row.details = state_check_details("ok", false);
        state
            .options_store
            .lock()
            .unwrap()
            .upsert_option_execution_reconciliation(row, 99);

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        let checks = view.state_checks.unwrap();
        assert_eq!(checks.state_check_status, "ok");
        assert_eq!(checks.nonce_check_status, "ok");
        assert_eq!(checks.position_check_status, "ok");
        assert!(view.health.warnings.is_empty());
        assert!(view.health.errors.is_empty());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn lifecycle_health_warns_on_non_strict_state_check_mismatch() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        let mut row = build_reconciliation(
            &intent,
            &tx,
            OptionReconciliationStatus::Reconciled,
            Some(Uuid::from_u128(100)),
            None,
        );
        row.details = state_check_details("failed", false);
        state
            .options_store
            .lock()
            .unwrap()
            .upsert_option_execution_reconciliation(row, 99);

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(view.health.stage, OptionExecutionLifecycleStage::Reconciled);
        assert!(view.health.is_terminal_success);
        assert!(view
            .health
            .warnings
            .iter()
            .any(|warning| warning == "state_checks_failed"));
        assert!(view.health.errors.is_empty());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn mined_failed_marks_health_failed() {
        let state = fresh_state();
        let (intent, _) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedFailed,
            OptionExecutionIntentStatus::BroadcastReverted,
        );

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(view.health.stage, OptionExecutionLifecycleStage::Failed);
        assert!(view.health.is_terminal_success.eq(&false));
        assert!(!view.health.errors.is_empty());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn events_summary_buckets_by_name_and_address() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        let tx_hash = tx.tx_hash.clone().unwrap();
        insert_event(&state, option_trade_event(&intent, &tx_hash));
        insert_event(&state, margin_trade_event(&intent, &tx_hash));
        insert_event(&state, trading_fee_event(&intent, &tx_hash, 4));
        insert_event(&state, trading_fee_event(&intent, &tx_hash, 5));
        insert_event(&state, internal_transfer_event(&tx_hash, 6));
        insert_event(&state, internal_transfer_event(&tx_hash, 7));
        insert_event(&state, internal_transfer_event(&tx_hash, 8));

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(view.events.total, 7);
        assert_eq!(view.events.counts_by_event_name["OptionTradeExecuted"], 1);
        assert_eq!(view.events.counts_by_event_name["TradeExecuted"], 1);
        assert_eq!(view.events.counts_by_event_name["TradingFeeCharged"], 2);
        assert_eq!(view.events.counts_by_event_name["InternalTransfer"], 3);
        assert!(view
            .events
            .counts_by_contract_address
            .contains_key("0x00000000000000000000000000000000000000ee"));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn lifecycle_aggregator_does_not_touch_broadcast_or_generic_paths() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        insert_reconciliation(
            &state,
            &intent,
            &tx,
            OptionReconciliationStatus::Reconciled,
            Some(Uuid::from_u128(100)),
            None,
        );

        let _ = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
        assert!(state.engine.lock().unwrap().execution_intents().is_empty());
    }

    fn fresh_state() -> AppState {
        let mut options = OptionsConfig::enabled_in_memory_for_tests();
        options.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        AppState::with_options_config(EngineState::with_default_markets(), options)
    }

    fn assert_no_generic_execution_rows(state: &AppState) {
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    fn insert_intent_and_tx(
        state: &AppState,
        confirmation_status: OptionExecutionConfirmationStatus,
        intent_status: OptionExecutionIntentStatus,
    ) -> (OptionExecutionIntent, OptionExecutionTransaction) {
        let mut intent = signed_calldata_ready_intent();
        intent.status = intent_status;
        let intent_clone = intent.clone();
        let tx = OptionExecutionTransaction {
            transaction_id: "option-tx-1".to_string(),
            intent_id: intent.intent_id,
            onchain_intent_id: Some(intent.onchain_intent_id.clone()),
            from: AccountId::new("0x00000000000000000000000000000000000000c0"),
            to: AccountId::new("0x00000000000000000000000000000000000000ee"),
            calldata: intent.calldata.clone().unwrap_or_default(),
            value_wei: "0".to_string(),
            gas_limit: Some(1_500_000),
            tx_hash: Some(
                "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125".to_string(),
            ),
            status: ExecutionTransactionStatus::Submitted,
            error: None,
            estimated_gas: Some(1_091_120),
            required_gas: Some(1_363_900),
            simulation_gas_limit: Some(1_500_000),
            broadcast_gas_limit: Some(1_500_000),
            gas_safety_bps: Some(12_500),
            gas_check_status: None,
            gas_check_error: None,
            confirmation_status: Some(confirmation_status),
            confirmed_at_ms: Some(10),
            confirmed_block_number: Some(41_856_964),
            receipt_status: match confirmation_status {
                OptionExecutionConfirmationStatus::MinedSuccess => Some(1),
                OptionExecutionConfirmationStatus::MinedFailed
                | OptionExecutionConfirmationStatus::MinedReverted => Some(0),
                _ => None,
            },
            confirmation_error: None,
            gas_used: Some(1_057_772),
            effective_gas_price: Some("6000000".to_string()),
            cumulative_gas_used: Some(1_672_948),
            receipt_block_hash: Some(
                "0x53d62c21ecbe462e2868e216b4655474de0d2b7b832f15ab6e72b216fb1f3853".to_string(),
            ),
            receipt_transaction_index: Some(5),
            receipt_observed_at_ms: Some(20),
            created_at_ms: 2,
            updated_at_ms: 2,
        };
        let inserted_tx = {
            let mut store = state.options_store.lock().unwrap();
            store.insert_option_execution_intent(intent_clone);
            store.test_insert_option_execution_transaction(tx.clone());
            tx
        };
        (intent, inserted_tx)
    }

    fn insert_event(state: &AppState, event: OptionExecutionEvent) {
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor("lifecycle_test", &[event], 0, 1);
    }

    fn insert_reconciliation(
        state: &AppState,
        intent: &OptionExecutionIntent,
        tx: &OptionExecutionTransaction,
        status: OptionReconciliationStatus,
        trade_event_id: Option<Uuid>,
        margin_event_id: Option<Uuid>,
    ) {
        let row = build_reconciliation(intent, tx, status, trade_event_id, margin_event_id);
        state
            .options_store
            .lock()
            .unwrap()
            .upsert_option_execution_reconciliation(row, 99);
    }

    fn build_reconciliation(
        intent: &OptionExecutionIntent,
        tx: &OptionExecutionTransaction,
        status: OptionReconciliationStatus,
        trade_event_id: Option<Uuid>,
        margin_event_id: Option<Uuid>,
    ) -> OptionExecutionReconciliation {
        OptionExecutionReconciliation {
            id: Uuid::from_u128(500),
            intent_id: intent.intent_id,
            onchain_intent_id: intent.onchain_intent_id.clone(),
            option_execution_transaction_id: tx.transaction_id.clone(),
            tx_hash: tx.tx_hash.clone().unwrap_or_default(),
            chain_id: 84532,
            status,
            strict: true,
            requires_events: true,
            trade_executed_event_id: trade_event_id,
            margin_trade_event_id: margin_event_id,
            trading_fee_event_count: 0,
            internal_transfer_event_count: 0,
            decoded_event_count: 0,
            mismatch_reason: None,
            missing_required: None,
            details: serde_json::json!({}),
            reconciled_at_ms: 50,
            created_at_ms: 50,
            updated_at_ms: 50,
        }
    }

    fn state_check_details(overall_status: &str, strict: bool) -> serde_json::Value {
        let failed = overall_status != "ok";
        serde_json::json!({
            "state_checks": {
                "enabled": true,
                "checked_at_ms": 99,
                "strict": strict,
                "require_rpc": true,
                "overall_status": overall_status,
                "nonce_check_status": if failed { "failed" } else { "ok" },
                "position_check_status": "ok",
                "vault_check_status": "skipped",
                "buyer_nonce": {
                    "expected_min": "1",
                    "actual": if failed { "0" } else { "1" },
                    "status": if failed { "failed" } else { "ok" }
                }
            }
        })
    }

    fn signed_calldata_ready_intent() -> OptionExecutionIntent {
        OptionExecutionIntent {
            intent_id: Uuid::from_u128(1),
            onchain_intent_id: "0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5"
                .to_string(),
            source_type: OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-1".to_string(),
            option_series_id: "series-1".to_string(),
            onchain_option_id: "7".to_string(),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            underlying: AccountId::new("0x0000000000000000000000000000000000000010"),
            settlement_asset: AccountId::new("0x0000000000000000000000000000000000000020"),
            expiry: 4_102_444_800,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: 100_000_000,
            quantity_contracts: 1,
            source_size_1e8: 100_000_000,
            source_price_1e8: 10_000_000,
            premium_per_contract_native: 10_000,
            buyer_is_maker: false,
            buyer_nonce: Some(0),
            seller_nonce: Some(0),
            deadline: 0,
            buyer_signature: Some("0x01".to_string()),
            seller_signature: Some("0x02".to_string()),
            calldata: Some("0x031f77b3".to_string()),
            status: OptionExecutionIntentStatus::SimulationOk,
            error: None,
            simulation_status: Some(OptionExecutionSimulationStatus::SimulationOk),
            simulation_error: None,
            simulation_block_number: Some(41_856_962),
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: Some(1),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn option_trade_event(intent: &OptionExecutionIntent, tx_hash: &str) -> OptionExecutionEvent {
        OptionExecutionEvent {
            id: Uuid::from_u128(100),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000ee".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index: 1,
            block_number: 41_856_964,
            block_hash: None,
            event_name: "OptionTradeExecuted".to_string(),
            event_signature: OPTION_TRADE_EXECUTED_SIGNATURE.to_string(),
            intent_id: Some(intent.intent_id),
            onchain_intent_id: Some(intent.onchain_intent_id.clone()),
            option_execution_transaction_id: None,
            buyer: Some(intent.buyer.0.clone()),
            seller: Some(intent.seller.0.clone()),
            account: None,
            option_id: Some(intent.onchain_option_id.clone()),
            quantity_contracts: Some(intent.quantity_contracts.to_string()),
            premium_per_contract_native: Some(intent.premium_per_contract_native.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({"buyerIsMaker": false})),
            created_at_ms: 5,
            updated_at_ms: 5,
        }
    }

    fn margin_trade_event(intent: &OptionExecutionIntent, tx_hash: &str) -> OptionExecutionEvent {
        OptionExecutionEvent {
            id: Uuid::from_u128(101),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000aa".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index: 2,
            block_number: 41_856_964,
            block_hash: None,
            event_name: "TradeExecuted".to_string(),
            event_signature: "TradeExecuted".to_string(),
            intent_id: Some(intent.intent_id),
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: Some(intent.buyer.0.clone()),
            seller: Some(intent.seller.0.clone()),
            account: None,
            option_id: Some(intent.onchain_option_id.clone()),
            quantity_contracts: Some(intent.quantity_contracts.to_string()),
            premium_per_contract_native: Some(intent.premium_per_contract_native.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: None,
            created_at_ms: 5,
            updated_at_ms: 5,
        }
    }

    fn trading_fee_event(
        intent: &OptionExecutionIntent,
        tx_hash: &str,
        log_index: u64,
    ) -> OptionExecutionEvent {
        trading_fee_event_for(
            intent,
            tx_hash,
            log_index,
            intent.buyer.0.as_str(),
            6,
            false,
        )
    }

    fn trading_fee_event_for(
        intent: &OptionExecutionIntent,
        tx_hash: &str,
        log_index: u64,
        trader: &str,
        applied_fee: u128,
        is_maker: bool,
    ) -> OptionExecutionEvent {
        OptionExecutionEvent {
            id: Uuid::from_u128(200 + log_index as u128),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000aa".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 41_856_964,
            block_hash: None,
            event_name: "TradingFeeCharged".to_string(),
            event_signature: "TradingFeeCharged".to_string(),
            intent_id: Some(intent.intent_id),
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some(trader.to_string()),
            option_id: Some(intent.onchain_option_id.clone()),
            quantity_contracts: None,
            premium_per_contract_native: Some(applied_fee.to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "trader": trader,
                "appliedFee": applied_fee.to_string(),
                "isMaker": is_maker,
                "recipient": "0x009f3849df0d4f2547cfb72cc3e7500",
                "settlementAsset": "0x0000000000000000000000000000000000000020"
            })),
            created_at_ms: 5,
            updated_at_ms: 5,
        }
    }

    fn internal_transfer_event(tx_hash: &str, log_index: u64) -> OptionExecutionEvent {
        OptionExecutionEvent {
            id: Uuid::from_u128(300 + log_index as u128),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000bb".to_string(),
            tx_hash: tx_hash.to_string(),
            log_index,
            block_number: 41_856_964,
            block_hash: None,
            event_name: "InternalTransfer".to_string(),
            event_signature: "InternalTransfer".to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: Some("0x0000000000000000000000000000000000000001".to_string()),
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: Some("10000".to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({
                "from": "0x0000000000000000000000000000000000000001",
                "to": "0x0000000000000000000000000000000000000002",
                "token": "0x0000000000000000000000000000000000000020"
            })),
            created_at_ms: 5,
            updated_at_ms: 5,
        }
    }

    #[tokio::test]
    async fn fees_view_uses_onchain_source_of_truth_with_v1s_split() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        let tx_hash = tx.tx_hash.clone().unwrap();
        // V1S split: buyer (taker) charged 6, seller (maker) charged 4.
        insert_event(
            &state,
            trading_fee_event_for(&intent, &tx_hash, 4, intent.buyer.0.as_str(), 6, false),
        );
        insert_event(
            &state,
            trading_fee_event_for(&intent, &tx_hash, 5, intent.seller.0.as_str(), 4, true),
        );

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(view.fees.source_of_truth, "onchain");
        assert_eq!(view.fees.trading_fee_event_count, 2);
        assert_eq!(view.fees.observed_total, "10");
        assert_eq!(view.fees.reconciliation_status, "onchain_observed");
        assert_eq!(view.fees.backend_ledger_status, "disabled");
        assert_eq!(
            view.fees.by_trader[&intent.buyer.0.to_ascii_lowercase()],
            "6".to_string()
        );
        assert_eq!(
            view.fees.by_trader[&intent.seller.0.to_ascii_lowercase()],
            "4".to_string()
        );
        assert_eq!(view.fees.by_side["taker"], "6".to_string());
        assert_eq!(view.fees.by_side["maker"], "4".to_string());
        assert_eq!(
            view.fees.by_recipient["0x009f3849df0d4f2547cfb72cc3e7500"],
            "10".to_string()
        );
        // Backwards-compatible alias.
        assert_eq!(
            view.fees.total_by_recipient["0x009f3849df0d4f2547cfb72cc3e7500"],
            "10".to_string()
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn fees_view_reports_no_onchain_events_when_only_unrelated_logs_indexed() {
        let state = fresh_state();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        let tx_hash = tx.tx_hash.clone().unwrap();
        insert_event(&state, internal_transfer_event(&tx_hash, 6));

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(view.fees.source_of_truth, "onchain");
        assert_eq!(view.fees.trading_fee_event_count, 0);
        assert_eq!(view.fees.observed_total, "0");
        assert_eq!(view.fees.reconciliation_status, "no_onchain_events");
        assert!(view.fees.by_trader.is_empty());
        assert!(view.fees.by_recipient.is_empty());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn fees_view_reports_missing_or_disabled_when_ledger_enabled_but_empty() {
        let mut state = fresh_state();
        // Enable the fee ledger without inserting any matching ledger rows.
        // The on-chain summary must still succeed and explicitly mark the
        // ledger as missing — not fail the lifecycle aggregation.
        state.fees_config = crate::fees::FeesConfig::enabled_in_memory_for_tests();
        let (intent, tx) = insert_intent_and_tx(
            &state,
            OptionExecutionConfirmationStatus::MinedSuccess,
            OptionExecutionIntentStatus::BroadcastConfirmed,
        );
        let tx_hash = tx.tx_hash.clone().unwrap();
        insert_event(
            &state,
            trading_fee_event_for(&intent, &tx_hash, 4, intent.buyer.0.as_str(), 6, false),
        );

        let view = get_option_execution_lifecycle(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(view.fees.source_of_truth, "onchain");
        assert_eq!(view.fees.trading_fee_event_count, 1);
        assert_eq!(view.fees.observed_total, "6");
        assert_eq!(view.fees.backend_ledger_status, "missing_or_disabled");
        assert_eq!(view.fees.reconciliation_status, "onchain_observed");
        assert_no_generic_execution_rows(&state);
    }
}
