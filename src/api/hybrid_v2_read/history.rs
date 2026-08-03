//! Normalized tagged history model.
//!
//! Every history item carries block/tx/log identity + typed payload. The
//! projection layer emits events into `ProjectionState.fee_events`,
//! `matched_executions`, etc. — this module reconstructs a chronological
//! typed view by walking `IndexerRuntime.raw_logs` (canonical only) and
//! matching each canonical log against a projection-derived family label.

use crate::api::hybrid_v2_read::serialization::to_lower_hex;
use crate::hybrid_v2::decoder::{decode_log_with, CanonicalRawLog};
use crate::hybrid_v2::events::EventKind;
use crate::hybrid_v2::reducer::ProjectionState;
use crate::hybrid_v2::runtime::IndexerRuntime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HistoryDirection {
    Inbound,
    Outbound,
    Internal,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HistoryEventPayload {
    SubaccountCreated {
        owner: String,
        subaccount_id: u32,
    },
    SubaccountLazyRegistered {
        owner: String,
        subaccount_id: u32,
    },
    Deposit {
        token: String,
        amount: String,
    },
    Withdraw {
        token: String,
        amount: String,
    },
    InternalTransfer {
        token: String,
        amount: String,
        from_subkey: String,
        to_subkey: String,
    },
    ReservationIncrease {
        token: String,
        engine: String,
        amount: String,
    },
    ReservationRelease {
        token: String,
        engine: String,
        amount: String,
    },
    OrphanedLockReleased {
        token: String,
        engine: String,
        amount: String,
    },
    OptionOrderFilled {
        order_hash: String,
        filled_delta_1e8: String,
        terminal: bool,
    },
    OptionOrderCancelled {
        order_hash: String,
    },
    MatchedExecution {
        execution_id: String,
        buyer_order_hash: String,
        seller_order_hash: String,
        matched_qty_1e8: String,
    },
    PremiumTransferred {
        token: String,
        amount: String,
        from_subkey: String,
        to_subkey: String,
    },
    FeeCharged {
        token: String,
        amount: String,
        fee_subkey: String,
    },
    RebatePaid {
        token: String,
        amount: String,
        rebate_subkey: String,
    },
    MinValidNonceAdvanced {
        new_min_valid_nonce: String,
    },
    RecoveryRequested,
    RecoveryActivated,
    RecoveryCancelled,
    RecoveryFinalized,
    RecoveryFinalizationWithdrawn {
        token: String,
        amount: String,
    },
    RecoveryEpochIncremented,
    OwnerRecoveryEpochAdvanced,
    SubaccountRecoveryEpochAdvanced,
    PauseFlagChanged {
        paused: bool,
    },
    BadDebtSocialized {
        token: String,
        amount: String,
    },
    CapabilityEvent {
        kind: String,
        engine: Option<String>,
    },
    RiskEvent {
        kind: String,
    },
    Manifest {
        kind: String,
    },
    Other {
        kind: String,
    },
}

impl HistoryEventPayload {
    pub fn family_label(&self) -> &'static str {
        use HistoryEventPayload::*;
        match self {
            SubaccountCreated { .. } => "SUBACCOUNT_CREATED",
            SubaccountLazyRegistered { .. } => "SUBACCOUNT_LAZY_REGISTERED",
            Deposit { .. } => "DEPOSIT",
            Withdraw { .. } => "WITHDRAW",
            InternalTransfer { .. } => "INTERNAL_TRANSFER",
            ReservationIncrease { .. } => "RESERVATION_INCREASE",
            ReservationRelease { .. } => "RESERVATION_RELEASE",
            OrphanedLockReleased { .. } => "ORPHANED_LOCK_RELEASED",
            OptionOrderFilled { .. } => "OPTION_ORDER_FILLED",
            OptionOrderCancelled { .. } => "OPTION_ORDER_CANCELLED",
            MatchedExecution { .. } => "MATCHED_EXECUTION",
            PremiumTransferred { .. } => "PREMIUM_TRANSFERRED",
            FeeCharged { .. } => "FEE_CHARGED",
            RebatePaid { .. } => "REBATE_PAID",
            MinValidNonceAdvanced { .. } => "MIN_VALID_NONCE_ADVANCED",
            RecoveryRequested => "RECOVERY_REQUESTED",
            RecoveryActivated => "RECOVERY_ACTIVATED",
            RecoveryCancelled => "RECOVERY_CANCELLED",
            RecoveryFinalized => "RECOVERY_FINALIZED",
            RecoveryFinalizationWithdrawn { .. } => "RECOVERY_FINALIZATION_WITHDRAWN",
            RecoveryEpochIncremented => "RECOVERY_EPOCH_INCREMENTED",
            OwnerRecoveryEpochAdvanced => "OWNER_RECOVERY_EPOCH_ADVANCED",
            SubaccountRecoveryEpochAdvanced => "SUBACCOUNT_RECOVERY_EPOCH_ADVANCED",
            PauseFlagChanged { .. } => "PAUSE_FLAG_CHANGED",
            BadDebtSocialized { .. } => "BAD_DEBT_SOCIALIZED",
            CapabilityEvent { .. } => "CAPABILITY_EVENT",
            RiskEvent { .. } => "RISK_EVENT",
            Manifest { .. } => "MANIFEST",
            Other { .. } => "OTHER",
        }
    }
}

/// One row in the normalized history feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub event_id: String,
    pub deployment_id: u64,
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub tx_hash: String,
    pub tx_index: u32,
    pub log_index: u32,
    pub timestamp_ms: u64,
    pub finalized: bool,
    pub direction: HistoryDirection,
    pub owner: Option<String>,
    pub subaccount_id: Option<u32>,
    pub subkey: Option<String>,
    pub related_order_hash: Option<String>,
    pub related_execution_id: Option<String>,
    pub payload: HistoryEventPayload,
}

/// Filter for history queries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryFilter {
    pub deployment_id: Option<u64>,
    pub owner: Option<String>,
    pub subkey: Option<String>,
    pub subaccount_id: Option<u32>,
    pub families: Vec<String>,
    pub token: Option<String>,
    pub series_id: Option<String>,
    pub order_hash: Option<String>,
    pub execution_id: Option<String>,
    pub direction: Option<HistoryDirection>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub from_ts_ms: Option<u64>,
    pub to_ts_ms: Option<u64>,
}

impl HistoryFilter {
    pub fn to_stable_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

/// Build the normalized history for a runtime.
pub fn build_history(
    runtime: &IndexerRuntime,
    chain_id: u64,
    finalized_block: u64,
    filter: &HistoryFilter,
) -> Vec<HistoryEvent> {
    let mut out = Vec::new();
    // Precompute a subKey -> (owner, subaccount_id) reverse lookup from the
    // projection so we can attach identity to events that only carry subKey
    // in the log payload.
    let sk_lookup = subkey_lookup(runtime.projection());
    for log in runtime.raw_logs.iter().filter(|l| l.is_canonical) {
        if let Some(from) = filter.from_block {
            if log.block_number < from {
                continue;
            }
        }
        if let Some(to) = filter.to_block {
            if log.block_number > to {
                continue;
            }
        }
        let raw = CanonicalRawLog {
            emitter: log.emitter.clone(),
            topics: log.topics.clone(),
            data: log.data.clone(),
        };
        let Ok(event) = decode_log_with(&raw, None, runtime.manifest.event_version) else {
            continue;
        };
        let (owner, sid, subkey) = derive_identity(&event, &sk_lookup);
        let payload = to_payload(&event);
        let direction = classify_direction(&event, filter, subkey.as_deref());
        // Ownership scoping filters.
        if let Some(f_owner) = filter.owner.as_ref() {
            if owner.as_deref().map(|o| o.eq_ignore_ascii_case(f_owner)) != Some(true) {
                continue;
            }
        }
        if let Some(f_sk) = filter.subkey.as_ref() {
            if subkey.as_deref().map(|s| s.eq_ignore_ascii_case(f_sk)) != Some(true) {
                continue;
            }
        }
        if let Some(f_sid) = filter.subaccount_id {
            if sid != Some(f_sid) {
                continue;
            }
        }
        if !filter.families.is_empty() {
            let label = payload.family_label();
            if !filter
                .families
                .iter()
                .any(|f| f.eq_ignore_ascii_case(label))
            {
                continue;
            }
        }
        if let Some(t) = filter.token.as_ref() {
            let matches = matches!(&payload,
                HistoryEventPayload::Deposit { token, .. }
                | HistoryEventPayload::Withdraw { token, .. }
                | HistoryEventPayload::InternalTransfer { token, .. }
                | HistoryEventPayload::ReservationIncrease { token, .. }
                | HistoryEventPayload::ReservationRelease { token, .. }
                | HistoryEventPayload::OrphanedLockReleased { token, .. }
                | HistoryEventPayload::PremiumTransferred { token, .. }
                | HistoryEventPayload::FeeCharged { token, .. }
                | HistoryEventPayload::RebatePaid { token, .. }
                | HistoryEventPayload::RecoveryFinalizationWithdrawn { token, .. }
                | HistoryEventPayload::BadDebtSocialized { token, .. }
                if token.eq_ignore_ascii_case(t));
            if !matches {
                continue;
            }
        }
        if let Some(oh_filter) = filter.order_hash.as_ref() {
            let matches = matches!(&payload,
                HistoryEventPayload::OptionOrderFilled { order_hash, .. }
                | HistoryEventPayload::OptionOrderCancelled { order_hash, .. }
                if order_hash.eq_ignore_ascii_case(oh_filter));
            if !matches {
                continue;
            }
        }
        if let Some(exec_filter) = filter.execution_id.as_ref() {
            let matches = matches!(&payload,
                HistoryEventPayload::MatchedExecution { execution_id, .. }
                if execution_id.eq_ignore_ascii_case(exec_filter));
            if !matches {
                continue;
            }
        }
        if let Some(dir) = filter.direction {
            if direction != dir {
                continue;
            }
        }
        let event_id = format!("{}:{}:{}", log.block_hash, log.tx_hash, log.log_index);
        out.push(HistoryEvent {
            event_id,
            deployment_id: runtime.deployment_id,
            chain_id,
            block_number: log.block_number,
            block_hash: log.block_hash.clone(),
            tx_hash: log.tx_hash.clone(),
            tx_index: log.tx_index,
            log_index: log.log_index,
            timestamp_ms: log.block_timestamp * 1000,
            finalized: log.block_number <= finalized_block,
            direction,
            owner: owner.map(|s| to_lower_hex(&s)),
            subaccount_id: sid,
            subkey: subkey.map(|s| to_lower_hex(&s)),
            related_order_hash: extract_order_hash(&payload),
            related_execution_id: extract_execution_id(&payload),
            payload,
        });
    }
    out
}

fn subkey_lookup(state: &ProjectionState) -> std::collections::BTreeMap<String, (String, u32)> {
    let mut out = std::collections::BTreeMap::new();
    for ((owner, sid), subkey) in state.subaccounts.iter() {
        out.insert(subkey.to_ascii_lowercase(), (owner.clone(), *sid));
    }
    out
}

fn derive_identity(
    event: &crate::hybrid_v2::events::HybridV2Event,
    sk_lookup: &std::collections::BTreeMap<String, (String, u32)>,
) -> (Option<String>, Option<u32>, Option<String>) {
    let subkey = event.subkey.clone();
    let mut owner = event.owner.clone();
    let mut sid = event.subaccount_id;
    if owner.is_none() || sid.is_none() {
        if let Some(sk) = subkey.as_ref() {
            if let Some((o, s)) = sk_lookup.get(&sk.to_ascii_lowercase()) {
                if owner.is_none() {
                    owner = Some(o.clone());
                }
                if sid.is_none() {
                    sid = Some(*s);
                }
            }
        }
    }
    (owner, sid, subkey)
}

fn classify_direction(
    event: &crate::hybrid_v2::events::HybridV2Event,
    filter: &HistoryFilter,
    subkey: Option<&str>,
) -> HistoryDirection {
    match event.kind {
        EventKind::Deposit
        | EventKind::OptionRebatePaid
        | EventKind::CollateralUnlocked
        | EventKind::OrphanedLockReleased
        | EventKind::RecoveryFinalizationWithdrawn => HistoryDirection::Inbound,
        EventKind::Withdraw
        | EventKind::OptionFeeCharged
        | EventKind::CollateralLocked
        | EventKind::BadDebtSocialized => HistoryDirection::Outbound,
        EventKind::InternalTransfer | EventKind::OptionPremiumTransferred => {
            let from = event.payload.get("from_subkey").and_then(|v| v.as_str());
            let to = event.payload.get("to_subkey").and_then(|v| v.as_str());
            let target = filter.subkey.as_deref().or(subkey);
            match target {
                Some(t) if to.map(|x| x.eq_ignore_ascii_case(t)) == Some(true) => {
                    HistoryDirection::Inbound
                }
                Some(t) if from.map(|x| x.eq_ignore_ascii_case(t)) == Some(true) => {
                    HistoryDirection::Outbound
                }
                _ => HistoryDirection::Internal,
            }
        }
        _ => HistoryDirection::Metadata,
    }
}

fn to_payload(event: &crate::hybrid_v2::events::HybridV2Event) -> HistoryEventPayload {
    use HistoryEventPayload as P;
    let amount = event
        .payload
        .get("amount")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    match event.kind {
        EventKind::SubaccountCreated => P::SubaccountCreated {
            owner: event.owner.clone().unwrap_or_default(),
            subaccount_id: event.subaccount_id.unwrap_or(0),
        },
        EventKind::SubaccountLazyRegistered => P::SubaccountLazyRegistered {
            owner: event.owner.clone().unwrap_or_default(),
            subaccount_id: event.subaccount_id.unwrap_or(0),
        },
        EventKind::Deposit => P::Deposit {
            token: event.token.clone().unwrap_or_default(),
            amount,
        },
        EventKind::Withdraw => P::Withdraw {
            token: event.token.clone().unwrap_or_default(),
            amount,
        },
        EventKind::InternalTransfer => P::InternalTransfer {
            token: event.token.clone().unwrap_or_default(),
            amount,
            from_subkey: event
                .payload
                .get("from_subkey")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            to_subkey: event
                .payload
                .get("to_subkey")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        EventKind::CollateralLocked => P::ReservationIncrease {
            token: event.token.clone().unwrap_or_default(),
            engine: event.engine.clone().unwrap_or_default(),
            amount,
        },
        EventKind::CollateralUnlocked => P::ReservationRelease {
            token: event.token.clone().unwrap_or_default(),
            engine: event.engine.clone().unwrap_or_default(),
            amount,
        },
        EventKind::OrphanedLockReleased => P::OrphanedLockReleased {
            token: event.token.clone().unwrap_or_default(),
            engine: event.engine.clone().unwrap_or_default(),
            amount,
        },
        EventKind::OptionOrderFilled => P::OptionOrderFilled {
            order_hash: event.order_hash.clone().unwrap_or_default(),
            filled_delta_1e8: event
                .payload
                .get("filled_delta_1e8")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            terminal: event
                .payload
                .get("terminal")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        EventKind::OptionOrderCancelled => P::OptionOrderCancelled {
            order_hash: event.order_hash.clone().unwrap_or_default(),
        },
        EventKind::OptionOrderPairExecuted => P::MatchedExecution {
            execution_id: event.execution_id.clone().unwrap_or_default(),
            buyer_order_hash: event
                .payload
                .get("buyer_order_hash")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            seller_order_hash: event
                .payload
                .get("seller_order_hash")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            matched_qty_1e8: event
                .payload
                .get("matched_qty_1e8")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
        },
        EventKind::OptionPremiumTransferred => P::PremiumTransferred {
            token: event.token.clone().unwrap_or_default(),
            amount,
            from_subkey: event
                .payload
                .get("from_subkey")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            to_subkey: event
                .payload
                .get("to_subkey")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        EventKind::OptionFeeCharged => P::FeeCharged {
            token: event.token.clone().unwrap_or_default(),
            amount,
            fee_subkey: event
                .payload
                .get("fee_subkey")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        EventKind::OptionRebatePaid => P::RebatePaid {
            token: event.token.clone().unwrap_or_default(),
            amount,
            rebate_subkey: event
                .payload
                .get("rebate_subkey")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        EventKind::OptionSubaccountMinValidOrderNonceAdvanced => P::MinValidNonceAdvanced {
            new_min_valid_nonce: event
                .payload
                .get("new_min_valid_nonce")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
        },
        EventKind::RecoveryRequested => P::RecoveryRequested,
        EventKind::RecoveryActivated => P::RecoveryActivated,
        EventKind::RecoveryCancelled => P::RecoveryCancelled,
        EventKind::RecoveryFinalized => P::RecoveryFinalized,
        EventKind::RecoveryFinalizationWithdrawn => P::RecoveryFinalizationWithdrawn {
            token: event.token.clone().unwrap_or_default(),
            amount,
        },
        EventKind::RecoveryEpochIncremented => P::RecoveryEpochIncremented,
        EventKind::OwnerRecoveryEpochAdvanced => P::OwnerRecoveryEpochAdvanced,
        EventKind::SubaccountRecoveryEpochAdvanced => P::SubaccountRecoveryEpochAdvanced,
        EventKind::PauseFlagChanged => P::PauseFlagChanged {
            paused: event
                .payload
                .get("paused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        EventKind::BadDebtSocialized => P::BadDebtSocialized {
            token: event.token.clone().unwrap_or_default(),
            amount,
        },
        EventKind::EngineCapabilityChanged
        | EventKind::EngineGuardianRevoked
        | EventKind::GuardianChanged => P::CapabilityEvent {
            kind: event.kind.as_str().to_string(),
            engine: event.engine.clone(),
        },
        EventKind::RiskParamsSet
        | EventKind::RiskModuleActivated
        | EventKind::LiquidationTriggered => P::RiskEvent {
            kind: event.kind.as_str().to_string(),
        },
        EventKind::DeploymentManifestDeclared
        | EventKind::ProtocolSubaccountsInitialized
        | EventKind::EscapeControllerInitialized
        | EventKind::RecoveryFinalizerInitialized
        | EventKind::CollateralTokenEnteredUniverse
        | EventKind::SupportedTokenAdded
        | EventKind::SupportedTokenRemoved
        | EventKind::RecoveryPauseSet
        | EventKind::IntentConsumed
        | EventKind::NonceCancelled => P::Manifest {
            kind: event.kind.as_str().to_string(),
        },
        // Every other typed kind rendered generically.
        _ => P::Other {
            kind: event.kind.as_str().to_string(),
        },
    }
}

fn extract_order_hash(p: &HistoryEventPayload) -> Option<String> {
    match p {
        HistoryEventPayload::OptionOrderFilled { order_hash, .. }
        | HistoryEventPayload::OptionOrderCancelled { order_hash, .. } => Some(order_hash.clone()),
        HistoryEventPayload::MatchedExecution {
            buyer_order_hash, ..
        } => Some(buyer_order_hash.clone()),
        _ => None,
    }
}

fn extract_execution_id(p: &HistoryEventPayload) -> Option<String> {
    match p {
        HistoryEventPayload::MatchedExecution { execution_id, .. } => Some(execution_id.clone()),
        _ => None,
    }
}

/// Deterministic ordering: block DESC, tx_index DESC, log_index DESC.
/// Then descending event_id as tie-breaker.
pub fn sort_desc(events: &mut Vec<HistoryEvent>) {
    events.sort_by(|a, b| {
        (b.block_number, b.tx_index, b.log_index, &b.event_id).cmp(&(
            a.block_number,
            a.tx_index,
            a.log_index,
            &a.event_id,
        ))
    });
}

pub fn take_page_after(
    events: &[HistoryEvent],
    last_key: Option<&str>,
    limit: usize,
) -> Vec<HistoryEvent> {
    let mut skipping = last_key.is_some();
    let mut out = Vec::with_capacity(limit);
    for e in events {
        if skipping {
            if e.event_id.as_str() == last_key.unwrap() {
                skipping = false;
            }
            continue;
        }
        out.push(e.clone());
        if out.len() >= limit {
            break;
        }
    }
    out
}
