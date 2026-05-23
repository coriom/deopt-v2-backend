use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::types::now_ms;
use serde::Serialize;
use std::collections::BTreeMap;
use uuid::Uuid;

use super::{
    OptionExecutionEvent, OptionExecutionIntent, OptionExecutionReconciliation,
    OptionExecutionTransaction, OptionReconciliationStatus,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionReconciliationConfig {
    pub enabled: bool,
    pub poll_interval_ms: u64,
    pub batch_size: u32,
    pub require_events: bool,
    pub require_rpc: bool,
    pub strict: bool,
    pub rpc_url: Option<String>,
}

impl OptionReconciliationConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            poll_interval_ms: 15_000,
            batch_size: 25,
            require_events: true,
            require_rpc: true,
            strict: true,
            rpc_url: None,
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.poll_interval_ms == 0 {
            return Err(BackendError::Config(
                "OPTION_RECONCILIATION_POLL_INTERVAL_MS must be greater than zero".to_string(),
            ));
        }
        if self.batch_size == 0 {
            return Err(BackendError::Config(
                "OPTION_RECONCILIATION_BATCH_SIZE must be greater than zero".to_string(),
            ));
        }
        if !persistence_enabled {
            return Err(BackendError::Config(
                "option reconciliation worker requires persistence enabled".to_string(),
            ));
        }
        if self.require_rpc && self.rpc_url.is_none() {
            return Err(BackendError::Config(
                "RPC_URL is required when OPTION_RECONCILIATION_WORKER_ENABLED=true and OPTION_RECONCILIATION_REQUIRE_RPC=true".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionReconciliationDecision {
    pub transaction_id: String,
    pub intent_id: Uuid,
    pub tx_hash: String,
    pub status: OptionReconciliationStatus,
    pub mismatch_reason: Option<String>,
    pub missing_required: Option<String>,
    pub decoded_event_count: u64,
    pub trade_executed_event_id: Option<Uuid>,
    pub margin_trade_event_id: Option<Uuid>,
    pub trading_fee_event_count: u64,
    pub internal_transfer_event_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionReconciliationTickResult {
    pub enabled: bool,
    pub batch_size: u32,
    pub strict: bool,
    pub require_events: bool,
    pub require_rpc: bool,
    pub considered: u64,
    pub reconciled: u64,
    pub partially_reconciled: u64,
    pub reconciliation_failed: u64,
    pub missing_events: u64,
    pub skipped: u64,
    pub decisions: Vec<OptionReconciliationDecision>,
}

/// Run one tick of the option execution reconciliation worker.
///
/// For every confirmed `mined_success` option execution transaction that has
/// no reconciliation row yet, fetch the linked intent and the indexed events
/// keyed by `tx_hash`, then assert event-level invariants:
///
/// - `OptionTradeExecuted` decoded fields match the persisted intent
///   (`onchain_intent_id`, `buyer`, `seller`, `option_id`, `quantity`,
///   `premium_per_contract`).
/// - `TradeExecuted` (margin engine) is observed when present and consistent
///   with the same buyer/seller/option/quantity/premium tuple.
/// - `TradingFeeCharged` and `CollateralVault` `InternalTransfer` rows are
///   counted and recorded for evidence.
///
/// Strict mode (`OPTION_RECONCILIATION_STRICT=true`) returns
/// `ReconciliationFailed` on any mismatch and `MissingEvents` when the
/// `OptionTradeExecuted` row is absent. Non-strict mode degrades to
/// `PartiallyReconciled` so operators can re-run the indexer without
/// rolling back terminal state.
///
/// Never broadcasts, signs, creates new intents or transactions, or touches
/// generic `execution_transactions`.
pub async fn reconcile_option_executions(
    state: &AppState,
) -> Result<OptionReconciliationTickResult> {
    let config = state.option_reconciliation_config.clone();
    if !config.enabled {
        return Ok(empty_tick_result(&config));
    }

    let pending = list_confirmed_unreconciled_option_transactions(state, config.batch_size).await?;
    let mut decisions = Vec::with_capacity(pending.len());
    let mut counts = ReconciliationOutcomeCounts::default();
    let total_considered = pending.len() as u64;

    for transaction in pending {
        let now = now_ms();
        let Some(tx_hash) = transaction.tx_hash.clone() else {
            counts.skipped += 1;
            continue;
        };

        let intent = match get_option_execution_intent(state, transaction.intent_id).await? {
            Some(intent) => intent,
            None => {
                let decision = build_skip_decision(&transaction, &tx_hash, "intent not found");
                counts.skipped += 1;
                decisions.push(decision);
                continue;
            }
        };

        let events = list_option_execution_events_for_tx(state, &tx_hash).await?;
        let outcome = evaluate_reconciliation(
            state.chain_id,
            &config,
            &intent,
            &transaction,
            &tx_hash,
            &events,
        );
        match outcome.row.status {
            OptionReconciliationStatus::Reconciled => counts.reconciled += 1,
            OptionReconciliationStatus::PartiallyReconciled => counts.partially_reconciled += 1,
            OptionReconciliationStatus::ReconciliationFailed => counts.reconciliation_failed += 1,
            OptionReconciliationStatus::MissingEvents => counts.missing_events += 1,
            OptionReconciliationStatus::Skipped => counts.skipped += 1,
        }

        persist_option_execution_reconciliation(state, &outcome.row, now).await?;
        decisions.push(outcome.decision);
    }

    let result = OptionReconciliationTickResult {
        enabled: true,
        batch_size: config.batch_size,
        strict: config.strict,
        require_events: config.require_events,
        require_rpc: config.require_rpc,
        considered: total_considered,
        reconciled: counts.reconciled,
        partially_reconciled: counts.partially_reconciled,
        reconciliation_failed: counts.reconciliation_failed,
        missing_events: counts.missing_events,
        skipped: counts.skipped,
        decisions,
    };
    if let Ok(mut slot) = state.option_reconciliation_last_tick.lock() {
        *slot = Some(result.clone());
    }
    Ok(result)
}

#[derive(Default)]
struct ReconciliationOutcomeCounts {
    reconciled: u64,
    partially_reconciled: u64,
    reconciliation_failed: u64,
    missing_events: u64,
    skipped: u64,
}

struct EvaluatedReconciliation {
    row: OptionExecutionReconciliation,
    decision: OptionReconciliationDecision,
}

fn empty_tick_result(config: &OptionReconciliationConfig) -> OptionReconciliationTickResult {
    OptionReconciliationTickResult {
        enabled: false,
        batch_size: config.batch_size,
        strict: config.strict,
        require_events: config.require_events,
        require_rpc: config.require_rpc,
        considered: 0,
        reconciled: 0,
        partially_reconciled: 0,
        reconciliation_failed: 0,
        missing_events: 0,
        skipped: 0,
        decisions: Vec::new(),
    }
}

fn build_skip_decision(
    transaction: &OptionExecutionTransaction,
    tx_hash: &str,
    reason: &str,
) -> OptionReconciliationDecision {
    OptionReconciliationDecision {
        transaction_id: transaction.transaction_id.clone(),
        intent_id: transaction.intent_id,
        tx_hash: tx_hash.to_string(),
        status: OptionReconciliationStatus::Skipped,
        mismatch_reason: Some(reason.to_string()),
        missing_required: None,
        decoded_event_count: 0,
        trade_executed_event_id: None,
        margin_trade_event_id: None,
        trading_fee_event_count: 0,
        internal_transfer_event_count: 0,
    }
}

fn evaluate_reconciliation(
    chain_id: u64,
    config: &OptionReconciliationConfig,
    intent: &OptionExecutionIntent,
    transaction: &OptionExecutionTransaction,
    tx_hash: &str,
    events: &[OptionExecutionEvent],
) -> EvaluatedReconciliation {
    let now = now_ms();
    let trade_executed = events
        .iter()
        .find(|event| event.event_name == "OptionTradeExecuted");
    let margin_trade = events
        .iter()
        .find(|event| event.event_name == "TradeExecuted");
    let trading_fee_events = events
        .iter()
        .filter(|event| event.event_name == "TradingFeeCharged")
        .collect::<Vec<_>>();
    let internal_transfer_events = events
        .iter()
        .filter(|event| event.event_name == "InternalTransfer")
        .collect::<Vec<_>>();

    let mut details = serde_json::Map::new();
    details.insert(
        "event_count_by_name".to_string(),
        serde_json::to_value(event_count_by_name(events)).unwrap_or(serde_json::Value::Null),
    );
    details.insert(
        "trading_fee_events".to_string(),
        serde_json::Value::Array(
            trading_fee_events
                .iter()
                .map(|event| trading_fee_evidence(event))
                .collect(),
        ),
    );
    details.insert(
        "internal_transfer_events".to_string(),
        serde_json::Value::Array(
            internal_transfer_events
                .iter()
                .map(|event| internal_transfer_evidence(event))
                .collect(),
        ),
    );
    if let Some(event) = margin_trade {
        details.insert("margin_trade".to_string(), margin_trade_evidence(event));
    }
    details.insert("intent".to_string(), intent_evidence(intent));

    let mut mismatch_reasons: Vec<String> = Vec::new();
    let mut missing_required: Option<String> = None;

    let trade_executed_id = trade_executed.map(|event| event.id);
    let margin_trade_id = margin_trade.map(|event| event.id);

    if let Some(event) = trade_executed {
        if let Some(reason) = check_trade_executed(intent, event) {
            mismatch_reasons.push(reason);
        }
    } else if config.require_events {
        missing_required = Some("OptionTradeExecuted".to_string());
    }

    if let Some(event) = margin_trade {
        if let Some(reason) = check_margin_trade_executed(intent, event) {
            mismatch_reasons.push(reason);
        }
    }

    let status = decide_status(
        config,
        trade_executed.is_some(),
        missing_required.as_deref(),
        &mismatch_reasons,
    );

    if !mismatch_reasons.is_empty() {
        details.insert(
            "mismatch_reasons".to_string(),
            serde_json::Value::Array(
                mismatch_reasons
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(missing) = missing_required.as_deref() {
        details.insert(
            "missing_required".to_string(),
            serde_json::Value::String(missing.to_string()),
        );
    }

    let mismatch_reason = if mismatch_reasons.is_empty() {
        None
    } else {
        Some(mismatch_reasons.join("; "))
    };

    let row = OptionExecutionReconciliation {
        id: Uuid::new_v4(),
        intent_id: intent.intent_id,
        onchain_intent_id: intent.onchain_intent_id.clone(),
        option_execution_transaction_id: transaction.transaction_id.clone(),
        tx_hash: tx_hash.to_string(),
        chain_id,
        status,
        strict: config.strict,
        requires_events: config.require_events,
        trade_executed_event_id: trade_executed_id,
        margin_trade_event_id: margin_trade_id,
        trading_fee_event_count: trading_fee_events.len() as u64,
        internal_transfer_event_count: internal_transfer_events.len() as u64,
        decoded_event_count: events.len() as u64,
        mismatch_reason: mismatch_reason.clone(),
        missing_required: missing_required.clone(),
        details: serde_json::Value::Object(details),
        reconciled_at_ms: now,
        created_at_ms: now,
        updated_at_ms: now,
    };
    let decision = OptionReconciliationDecision {
        transaction_id: transaction.transaction_id.clone(),
        intent_id: intent.intent_id,
        tx_hash: tx_hash.to_string(),
        status,
        mismatch_reason,
        missing_required,
        decoded_event_count: events.len() as u64,
        trade_executed_event_id: trade_executed_id,
        margin_trade_event_id: margin_trade_id,
        trading_fee_event_count: trading_fee_events.len() as u64,
        internal_transfer_event_count: internal_transfer_events.len() as u64,
    };
    EvaluatedReconciliation { row, decision }
}

fn decide_status(
    config: &OptionReconciliationConfig,
    has_trade_executed: bool,
    missing_required: Option<&str>,
    mismatch_reasons: &[String],
) -> OptionReconciliationStatus {
    if missing_required.is_some() {
        return OptionReconciliationStatus::MissingEvents;
    }
    if !mismatch_reasons.is_empty() {
        return OptionReconciliationStatus::ReconciliationFailed;
    }
    if has_trade_executed {
        return OptionReconciliationStatus::Reconciled;
    }
    if config.strict && config.require_events {
        return OptionReconciliationStatus::MissingEvents;
    }
    OptionReconciliationStatus::PartiallyReconciled
}

fn check_trade_executed(
    intent: &OptionExecutionIntent,
    event: &OptionExecutionEvent,
) -> Option<String> {
    let mut reasons = Vec::new();
    if let Some(onchain) = event.onchain_intent_id.as_deref() {
        if !onchain.eq_ignore_ascii_case(&intent.onchain_intent_id) {
            reasons.push(format!(
                "OptionTradeExecuted.onchain_intent_id={} does not match intent.onchain_intent_id={}",
                onchain, intent.onchain_intent_id
            ));
        }
    } else {
        reasons.push("OptionTradeExecuted missing onchain_intent_id".to_string());
    }
    if !matches_address(event.buyer.as_deref(), &intent.buyer.0) {
        reasons.push(format!(
            "OptionTradeExecuted.buyer={} does not match intent.buyer={}",
            event.buyer.clone().unwrap_or_default(),
            intent.buyer.0
        ));
    }
    if !matches_address(event.seller.as_deref(), &intent.seller.0) {
        reasons.push(format!(
            "OptionTradeExecuted.seller={} does not match intent.seller={}",
            event.seller.clone().unwrap_or_default(),
            intent.seller.0
        ));
    }
    if !matches_optional_string(event.option_id.as_deref(), &intent.onchain_option_id) {
        reasons.push(format!(
            "OptionTradeExecuted.option_id={} does not match intent.onchain_option_id={}",
            event.option_id.clone().unwrap_or_default(),
            intent.onchain_option_id
        ));
    }
    if !matches_u128(
        event.quantity_contracts.as_deref(),
        intent.quantity_contracts,
    ) {
        reasons.push(format!(
            "OptionTradeExecuted.quantity={} does not match intent.quantity_contracts={}",
            event.quantity_contracts.clone().unwrap_or_default(),
            intent.quantity_contracts
        ));
    }
    if !matches_u128(
        event.premium_per_contract_native.as_deref(),
        intent.premium_per_contract_native,
    ) {
        reasons.push(format!(
            "OptionTradeExecuted.premium_per_contract={} does not match intent.premium_per_contract_native={}",
            event.premium_per_contract_native.clone().unwrap_or_default(),
            intent.premium_per_contract_native
        ));
    }
    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    }
}

fn check_margin_trade_executed(
    intent: &OptionExecutionIntent,
    event: &OptionExecutionEvent,
) -> Option<String> {
    let mut reasons = Vec::new();
    if !matches_address(event.buyer.as_deref(), &intent.buyer.0) {
        reasons.push(format!(
            "TradeExecuted.buyer={} does not match intent.buyer={}",
            event.buyer.clone().unwrap_or_default(),
            intent.buyer.0
        ));
    }
    if !matches_address(event.seller.as_deref(), &intent.seller.0) {
        reasons.push(format!(
            "TradeExecuted.seller={} does not match intent.seller={}",
            event.seller.clone().unwrap_or_default(),
            intent.seller.0
        ));
    }
    if !matches_optional_string(event.option_id.as_deref(), &intent.onchain_option_id) {
        reasons.push(format!(
            "TradeExecuted.option_id={} does not match intent.onchain_option_id={}",
            event.option_id.clone().unwrap_or_default(),
            intent.onchain_option_id
        ));
    }
    if !matches_u128(
        event.quantity_contracts.as_deref(),
        intent.quantity_contracts,
    ) {
        reasons.push(format!(
            "TradeExecuted.quantity={} does not match intent.quantity_contracts={}",
            event.quantity_contracts.clone().unwrap_or_default(),
            intent.quantity_contracts
        ));
    }
    if !matches_u128(
        event.premium_per_contract_native.as_deref(),
        intent.premium_per_contract_native,
    ) {
        reasons.push(format!(
            "TradeExecuted.price={} does not match intent.premium_per_contract_native={}",
            event
                .premium_per_contract_native
                .clone()
                .unwrap_or_default(),
            intent.premium_per_contract_native
        ));
    }
    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    }
}

fn matches_address(observed: Option<&str>, expected: &str) -> bool {
    observed
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn matches_optional_string(observed: Option<&str>, expected: &str) -> bool {
    observed.map(|value| value == expected).unwrap_or(false)
}

fn matches_u128(observed: Option<&str>, expected: u128) -> bool {
    observed
        .map(|value| {
            value
                .parse::<u128>()
                .map(|parsed| parsed == expected)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn event_count_by_name(events: &[OptionExecutionEvent]) -> BTreeMap<String, u64> {
    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    for event in events {
        *out.entry(event.event_name.clone()).or_default() += 1;
    }
    out
}

fn trading_fee_evidence(event: &OptionExecutionEvent) -> serde_json::Value {
    serde_json::json!({
        "event_id": event.id,
        "log_index": event.log_index,
        "block_number": event.block_number,
        "trader": event.account,
        "option_id": event.option_id,
        "applied_fee": event
            .decoded
            .as_ref()
            .and_then(|value| value.get("appliedFee").cloned())
            .unwrap_or(serde_json::Value::Null),
        "is_maker": event
            .decoded
            .as_ref()
            .and_then(|value| value.get("isMaker").cloned())
            .unwrap_or(serde_json::Value::Null),
    })
}

fn internal_transfer_evidence(event: &OptionExecutionEvent) -> serde_json::Value {
    serde_json::json!({
        "event_id": event.id,
        "log_index": event.log_index,
        "block_number": event.block_number,
        "from": event.decoded
            .as_ref()
            .and_then(|value| value.get("from").cloned())
            .unwrap_or(serde_json::Value::Null),
        "to": event.decoded
            .as_ref()
            .and_then(|value| value.get("to").cloned())
            .unwrap_or(serde_json::Value::Null),
        "amount": event.premium_per_contract_native,
        "token": event.decoded
            .as_ref()
            .and_then(|value| value.get("token").cloned())
            .unwrap_or(serde_json::Value::Null),
    })
}

fn margin_trade_evidence(event: &OptionExecutionEvent) -> serde_json::Value {
    serde_json::json!({
        "event_id": event.id,
        "log_index": event.log_index,
        "block_number": event.block_number,
        "buyer": event.buyer,
        "seller": event.seller,
        "option_id": event.option_id,
        "quantity": event.quantity_contracts,
        "price": event.premium_per_contract_native,
    })
}

fn intent_evidence(intent: &OptionExecutionIntent) -> serde_json::Value {
    serde_json::json!({
        "intent_id": intent.intent_id,
        "onchain_intent_id": intent.onchain_intent_id,
        "buyer": intent.buyer.0,
        "seller": intent.seller.0,
        "option_id": intent.onchain_option_id,
        "quantity_contracts": intent.quantity_contracts.to_string(),
        "premium_per_contract_native": intent.premium_per_contract_native.to_string(),
    })
}

async fn list_confirmed_unreconciled_option_transactions(
    state: &AppState,
    limit: u32,
) -> Result<Vec<OptionExecutionTransaction>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .list_confirmed_unreconciled_option_execution_transactions(limit)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_confirmed_unreconciled_option_execution_transactions(limit))
}

async fn get_option_execution_intent(
    state: &AppState,
    intent_id: Uuid,
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

async fn list_option_execution_events_for_tx(
    state: &AppState,
    tx_hash: &str,
) -> Result<Vec<OptionExecutionEvent>> {
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

async fn persist_option_execution_reconciliation(
    state: &AppState,
    row: &OptionExecutionReconciliation,
    updated_at_ms: i64,
) -> Result<()> {
    if let Some(repository) = state.repository.clone() {
        repository
            .upsert_option_execution_reconciliation(row, updated_at_ms)
            .await?;
        return Ok(());
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .upsert_option_execution_reconciliation(row.clone(), updated_at_ms);
    Ok(())
}

pub async fn summarize_option_execution_reconciliations(
    state: &AppState,
) -> Result<BTreeMap<String, u64>> {
    let mut counts: BTreeMap<String, u64> = default_reconciliation_counts();
    let stored = if let Some(repository) = state.repository.clone() {
        repository
            .summarize_option_execution_reconciliations()
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .summarize_option_execution_reconciliations()
    };
    for (status, count) in stored {
        counts.insert(status, count);
    }
    Ok(counts)
}

fn default_reconciliation_counts() -> BTreeMap<String, u64> {
    BTreeMap::from([
        ("reconciled".to_string(), 0),
        ("partially_reconciled".to_string(), 0),
        ("reconciliation_failed".to_string(), 0),
        ("missing_events".to_string(), 0),
        ("skipped".to_string(), 0),
    ])
}

/// Spawn the background option execution reconciliation worker.
pub fn spawn_option_reconciliation_worker(state: AppState) -> Option<tokio::task::JoinHandle<()>> {
    if !state.option_reconciliation_config.enabled {
        tracing::info!("option reconciliation worker disabled");
        return None;
    }
    let poll_interval_ms = state.option_reconciliation_config.poll_interval_ms;
    Some(tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(poll_interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match reconcile_option_executions(&state).await {
                Ok(result) => {
                    if !result.decisions.is_empty() {
                        tracing::info!(
                            considered = result.considered,
                            reconciled = result.reconciled,
                            failed = result.reconciliation_failed,
                            missing = result.missing_events,
                            "option reconciliation worker tick"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "option reconciliation worker tick failed");
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineState;
    use crate::execution::ExecutionTransactionStatus;
    use crate::options::{
        OptionExecutionConfirmationStatus, OptionExecutionIntent, OptionExecutionIntentStatus,
        OptionExecutionSimulationStatus, OptionExecutionSourceType, OptionExecutionTransaction,
        OptionsConfig,
    };
    use crate::types::AccountId;

    #[test]
    fn disabled_default_passes_validate() {
        OptionReconciliationConfig::disabled()
            .validate_startup(true)
            .unwrap();
    }

    #[test]
    fn enabled_requires_persistence() {
        let mut cfg = OptionReconciliationConfig::disabled();
        cfg.enabled = true;
        cfg.rpc_url = Some("https://example.invalid".to_string());
        let error = cfg.validate_startup(false).unwrap_err();
        assert!(error
            .to_string()
            .contains("option reconciliation worker requires persistence"));
    }

    #[test]
    fn enabled_with_require_rpc_requires_rpc_url() {
        let mut cfg = OptionReconciliationConfig::disabled();
        cfg.enabled = true;
        cfg.require_rpc = true;
        let error = cfg.validate_startup(true).unwrap_err();
        assert!(error
            .to_string()
            .contains("OPTION_RECONCILIATION_WORKER_ENABLED=true"));
    }

    #[test]
    fn enabled_rejects_zero_batch_size() {
        let mut cfg = OptionReconciliationConfig::disabled();
        cfg.enabled = true;
        cfg.batch_size = 0;
        cfg.rpc_url = Some("https://example.invalid".to_string());
        let error = cfg.validate_startup(true).unwrap_err();
        assert!(error
            .to_string()
            .contains("OPTION_RECONCILIATION_BATCH_SIZE"));
    }

    #[test]
    fn enabled_rejects_zero_poll_interval() {
        let mut cfg = OptionReconciliationConfig::disabled();
        cfg.enabled = true;
        cfg.poll_interval_ms = 0;
        cfg.rpc_url = Some("https://example.invalid".to_string());
        let error = cfg.validate_startup(true).unwrap_err();
        assert!(error
            .to_string()
            .contains("OPTION_RECONCILIATION_POLL_INTERVAL_MS"));
    }

    #[tokio::test]
    async fn worker_disabled_does_nothing() {
        let state = state_with_reconciliation(false, true, true);
        let result = reconcile_option_executions(&state).await.unwrap();
        assert!(!result.enabled);
        assert_eq!(result.considered, 0);
        assert!(result.decisions.is_empty());
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_event_marks_missing_events_in_strict_mode() {
        let state = state_with_reconciliation(true, true, true);
        let (_intent, tx) = insert_confirmed_intent_and_tx(&state);

        let result = reconcile_option_executions(&state).await.unwrap();

        assert_eq!(result.considered, 1);
        assert_eq!(result.missing_events, 1);
        assert_eq!(result.reconciled, 0);
        let stored = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_reconciliations(10);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, OptionReconciliationStatus::MissingEvents);
        assert_eq!(
            stored[0].missing_required.as_deref(),
            Some("OptionTradeExecuted")
        );
        assert_eq!(stored[0].option_execution_transaction_id, tx.transaction_id);
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn matching_option_trade_executed_reconciles() {
        let state = state_with_reconciliation(true, true, true);
        let (intent, tx) = insert_confirmed_intent_and_tx(&state);
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        insert_event(
            &state,
            margin_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );
        for log_index in 5..7 {
            insert_event(
                &state,
                trading_fee_event(&intent, &tx.tx_hash.clone().unwrap(), log_index),
            );
        }
        for log_index in 10..12 {
            insert_event(
                &state,
                internal_transfer_event(&tx.tx_hash.clone().unwrap(), log_index),
            );
        }

        let result = reconcile_option_executions(&state).await.unwrap();

        assert_eq!(result.considered, 1);
        assert_eq!(result.reconciled, 1);
        let stored = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_reconciliations(10);
        assert_eq!(stored.len(), 1);
        let row = &stored[0];
        assert_eq!(row.status, OptionReconciliationStatus::Reconciled);
        assert_eq!(row.trading_fee_event_count, 2);
        assert_eq!(row.internal_transfer_event_count, 2);
        assert!(row.trade_executed_event_id.is_some());
        assert!(row.margin_trade_event_id.is_some());
        assert!(row.mismatch_reason.is_none());
        assert!(row.missing_required.is_none());
        let trading_fee_evidence = row
            .details
            .get("trading_fee_events")
            .and_then(|value| value.as_array())
            .map(|values| values.len())
            .unwrap_or(0);
        assert_eq!(trading_fee_evidence, 2);
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mismatched_buyer_or_seller_marks_reconciliation_failed() {
        let state = state_with_reconciliation(true, true, true);
        let (intent, tx) = insert_confirmed_intent_and_tx(&state);
        let mut event = option_trade_event(&intent, &tx.tx_hash.clone().unwrap());
        event.buyer = Some("0x00000000000000000000000000000000000000ab".to_string());
        insert_event(&state, event);

        let result = reconcile_option_executions(&state).await.unwrap();

        assert_eq!(result.reconciliation_failed, 1);
        let stored = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_reconciliations(10);
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].status,
            OptionReconciliationStatus::ReconciliationFailed
        );
        assert!(stored[0]
            .mismatch_reason
            .as_deref()
            .unwrap_or_default()
            .contains("buyer"));
    }

    #[tokio::test]
    async fn mismatched_option_id_marks_reconciliation_failed() {
        let state = state_with_reconciliation(true, true, true);
        let (intent, tx) = insert_confirmed_intent_and_tx(&state);
        let mut event = option_trade_event(&intent, &tx.tx_hash.clone().unwrap());
        event.option_id = Some("999".to_string());
        insert_event(&state, event);

        let result = reconcile_option_executions(&state).await.unwrap();

        assert_eq!(result.reconciliation_failed, 1);
        let stored = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_reconciliations(10);
        assert!(stored[0]
            .mismatch_reason
            .as_deref()
            .unwrap_or_default()
            .contains("option_id"));
    }

    #[tokio::test]
    async fn mismatched_quantity_marks_reconciliation_failed() {
        let state = state_with_reconciliation(true, true, true);
        let (intent, tx) = insert_confirmed_intent_and_tx(&state);
        let mut event = option_trade_event(&intent, &tx.tx_hash.clone().unwrap());
        event.quantity_contracts = Some("9".to_string());
        insert_event(&state, event);

        let result = reconcile_option_executions(&state).await.unwrap();

        assert_eq!(result.reconciliation_failed, 1);
        let stored = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_reconciliations(10);
        assert!(stored[0]
            .mismatch_reason
            .as_deref()
            .unwrap_or_default()
            .contains("quantity"));
    }

    #[tokio::test]
    async fn idempotent_reruns_overwrite_same_row() {
        let state = state_with_reconciliation(true, true, true);
        let (intent, tx) = insert_confirmed_intent_and_tx(&state);
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );

        reconcile_option_executions(&state).await.unwrap();
        reconcile_option_executions(&state).await.unwrap();

        let stored = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_reconciliations(10);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, OptionReconciliationStatus::Reconciled);
        // Second tick should consider the same transaction again — confirmed unreconciled
        // returns it only when no terminal row exists, but a Reconciled row stays terminal.
        // Our non-strict store filter excludes already-reconciled transactions on the
        // second pass so the considered counter resets to 0.
        let last = state
            .option_reconciliation_last_tick
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        assert_eq!(last.considered, 0);
    }

    #[tokio::test]
    async fn no_broadcast_or_generic_execution_rows_touched() {
        let state = state_with_reconciliation(true, true, true);
        let (intent, tx) = insert_confirmed_intent_and_tx(&state);
        insert_event(
            &state,
            option_trade_event(&intent, &tx.tx_hash.clone().unwrap()),
        );

        reconcile_option_executions(&state).await.unwrap();

        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
        assert!(state.engine.lock().unwrap().execution_intents().is_empty());
    }

    fn state_with_reconciliation(enabled: bool, require_events: bool, strict: bool) -> AppState {
        let mut options = OptionsConfig::enabled_in_memory_for_tests();
        options.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        let mut state = AppState::with_options_config(EngineState::with_default_markets(), options);
        state.option_reconciliation_config = OptionReconciliationConfig {
            enabled,
            poll_interval_ms: 15_000,
            batch_size: 25,
            require_events,
            require_rpc: false,
            strict,
            rpc_url: None,
        };
        state
    }

    fn insert_confirmed_intent_and_tx(
        state: &AppState,
    ) -> (OptionExecutionIntent, OptionExecutionTransaction) {
        let intent = option_intent();
        let mut transaction = OptionExecutionTransaction {
            transaction_id: "option-tx-1".to_string(),
            intent_id: intent.intent_id,
            onchain_intent_id: Some(intent.onchain_intent_id.clone()),
            from: AccountId::new("0x00000000000000000000000000000000000000c0"),
            to: AccountId::new("0x00000000000000000000000000000000000000ee"),
            calldata: "0x1234".to_string(),
            value_wei: "0".to_string(),
            gas_limit: Some(1_500_000),
            tx_hash: Some(
                "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125".to_string(),
            ),
            status: ExecutionTransactionStatus::Submitted,
            error: None,
            estimated_gas: None,
            required_gas: None,
            simulation_gas_limit: None,
            broadcast_gas_limit: None,
            gas_safety_bps: None,
            gas_check_status: None,
            gas_check_error: None,
            confirmation_status: Some(OptionExecutionConfirmationStatus::MinedSuccess),
            confirmed_at_ms: Some(10),
            confirmed_block_number: Some(41_856_964),
            receipt_status: Some(1),
            confirmation_error: None,
            gas_used: None,
            effective_gas_price: None,
            cumulative_gas_used: None,
            receipt_block_hash: None,
            receipt_transaction_index: None,
            receipt_observed_at_ms: None,
            created_at_ms: 2,
            updated_at_ms: 2,
        };
        // Force-disable the "submitted" exclusivity check so the store accepts the
        // pre-baked terminal row used by tests.
        transaction.status = ExecutionTransactionStatus::Submitted;
        let inserted = {
            let mut store = state.options_store.lock().unwrap();
            let inserted_intent = store.insert_option_execution_intent(intent);
            // Insert the transaction by bypassing the submitted-exclusivity check —
            // tests rely on having a single mined-success row already persisted.
            store.test_insert_option_execution_transaction(transaction.clone());
            (inserted_intent, transaction)
        };
        inserted
    }

    fn option_intent() -> OptionExecutionIntent {
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
            calldata: Some("0x12345678".to_string()),
            status: OptionExecutionIntentStatus::BroadcastConfirmed,
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
            event_signature: crate::options::OPTION_TRADE_EXECUTED_SIGNATURE.to_string(),
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
            account: Some(intent.buyer.0.clone()),
            option_id: Some(intent.onchain_option_id.clone()),
            quantity_contracts: None,
            premium_per_contract_native: Some("6".to_string()),
            raw_topics: serde_json::Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(serde_json::json!({"appliedFee": "6", "isMaker": false})),
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

    fn insert_event(state: &AppState, event: OptionExecutionEvent) {
        state
            .options_store
            .lock()
            .unwrap()
            .persist_option_execution_events_and_cursor("option_events_test", &[event], 0, 1);
    }
}
