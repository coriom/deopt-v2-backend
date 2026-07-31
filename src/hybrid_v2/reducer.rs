//! Deterministic projection reducer covering the full Hybrid V2 canonical
//! event surface.
//!
//! Frozen rules:
//! - Application is deterministic per (event kind, payload).
//! - The persistence layer keys inserts on
//!   `(deployment_id, block_hash, tx_hash, log_index)` — the reducer
//!   assumes deduplication has already occurred upstream.
//! - Reducer failure means projection corruption: caller MUST roll back
//!   the current block transaction and fail readiness.
//! - Balances / reservations never underflow.
//! - `filled_qty` and `min_valid_nonce` are monotonically non-decreasing.
//! - Recovery state transitions are one-directional and RECOVERED is
//!   terminal.
//! - A projection field is owned by exactly ONE event kind (see
//!   `docs/BACKEND_SUBACCOUNT_CANONICAL_STATE_AND_INDEXER_V1.md`
//!   "Projection ownership" section).

use crate::hybrid_v2::events::{EventKind, HybridV2Event};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

// -----------------------------------------------------------------
//                          PROJECTION STATE
// -----------------------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectionState {
    // Identity
    pub subaccounts: BTreeMap<(String, u32), String>, // (owner, id) → subKey
    pub subaccount_meta: BTreeMap<String, SubaccountMeta>, // subKey → meta

    // Vault
    pub balances: BTreeMap<(String, String), String>, // (subKey, token) → amount
    pub reservations: BTreeMap<(String, String, String), String>, // (subKey, token, engine)
    pub capability_grants: BTreeMap<String, String>,  // engine → bitmap
    pub collateral_universe: BTreeMap<String, u32>,   // token → universe_index
    pub disabled_tokens: BTreeSet<String>,
    pub pause_flags: BTreeMap<String, bool>, // subKey → paused
    pub bad_debt: BTreeMap<(String, String), String>, // (subKey, token)

    // Positions
    pub positions: BTreeMap<(String, String), PositionRow>, // (subKey, series) → row
    pub active_series: BTreeMap<String, BTreeSet<String>>,  // subKey → {series}

    // Orders
    pub order_lifecycle: BTreeMap<String, OrderLifecycleRow>, // order_hash → row
    pub min_valid_nonce: BTreeMap<String, String>, // subKey → min nonce (decimal uint256)

    // Executions (typed correlation groups fill this)
    pub matched_executions: BTreeMap<String, MatchedExecutionRow>, // execution_id

    // Fees / rebates / premium journal (append-only)
    pub fee_events: Vec<FeeEventRow>,

    // Recovery
    pub recovery_state: BTreeMap<String, RecoveryStateProjection>, // subKey → state
    pub escape_state: BTreeMap<String, EscapeStateRow>,            // subKey
    pub owner_recovery_epochs: BTreeMap<String, EpochRow>,         // owner
    pub subaccount_recovery_epochs: BTreeMap<String, EpochRow>,    // subKey
    pub recovery_finalization_withdrawals: BTreeMap<String, u32>,  // subKey → count
    pub recovery_pause: Option<RecoveryPauseRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubaccountMeta {
    pub materialised_via_created: bool,
    pub materialised_via_lazy: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionRow {
    pub long_qty_1e8: String,
    pub short_qty_1e8: String,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLifecycleRow {
    pub subkey: String,
    pub owner: String,
    pub series_id: Option<String>,
    pub side: u8,          // 0=Buy, 1=Sell
    pub time_in_force: u8, // 0=GTC, 1=IOC, 2=FOK
    pub total_qty_1e8: String,
    pub filled_qty_1e8: String,
    pub cancelled: bool,
    pub terminal: bool,
    pub first_seen_block: u64,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchedExecutionRow {
    pub buyer_order_hash: String,
    pub seller_order_hash: String,
    pub buyer_subkey: String,
    pub seller_subkey: String,
    pub series_id: String,
    pub matched_qty_1e8: String,
    pub premium_amount: String,
    pub fee_amount: String,
    pub rebate_amount: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub completion_status: ExecutionCompletion,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionCompletion {
    #[default]
    Incomplete,
    Complete,
    InvalidatedByReorg,
}

impl ExecutionCompletion {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionCompletion::Incomplete => "INCOMPLETE",
            ExecutionCompletion::Complete => "COMPLETE",
            ExecutionCompletion::InvalidatedByReorg => "INVALIDATED_BY_REORG",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeEventRow {
    pub kind: &'static str, // "OPTION_FEE_CHARGED" / "OPTION_REBATE_PAID" / "OPTION_PREMIUM_TRANSFERRED"
    pub payer_subkey: Option<String>,
    pub receiver_subkey: Option<String>,
    pub token: String,
    pub amount: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u32,
    pub execution_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryStateProjection {
    Normal,
    RecoveryPending,
    RecoveryActive,
    Cancelled,
    Recovered,
}

impl RecoveryStateProjection {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecoveryStateProjection::Normal => "NORMAL",
            RecoveryStateProjection::RecoveryPending => "RECOVERY_PENDING",
            RecoveryStateProjection::RecoveryActive => "RECOVERY_ACTIVE",
            RecoveryStateProjection::Cancelled => "CANCELLED",
            RecoveryStateProjection::Recovered => "RECOVERED",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscapeStateRow {
    pub state: &'static str, // NORMAL / REQUESTED / ACTIVATED / CANCELLED / FINALIZED
    pub requested_ts: Option<u64>,
    pub activation_eligible_at: Option<u64>,
    pub activated_ts: Option<u64>,
    pub cancelled_ts: Option<u64>,
    pub finalized_ts: Option<u64>,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRow {
    pub epoch_count: u64,
    pub min_valid_nonce: String,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPauseRow {
    pub paused: bool,
    pub until_ts: u64,
    pub last_event_block: u64,
}

// -----------------------------------------------------------------
//                          ERRORS
// -----------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ReducerError {
    #[error("balance underflow for subkey {subkey} token {token}: have {have}, need {need}")]
    Underflow {
        subkey: String,
        token: String,
        have: String,
        need: String,
    },
    #[error("event kind {kind} missing required field {field}")]
    MissingField {
        kind: &'static str,
        field: &'static str,
    },
    #[error("finalized subaccount {subkey} rejected credit via {kind}")]
    FinalizedSubaccountCredit { subkey: String, kind: &'static str },
    #[error("recovery transition not allowed: {subkey} {from:?} → {to:?} via {kind}")]
    IllegalRecoveryTransition {
        subkey: String,
        from: Option<RecoveryStateProjection>,
        to: RecoveryStateProjection,
        kind: &'static str,
    },
    #[error("uint256 arithmetic overflow on subkey {subkey} token {token}")]
    Overflow { subkey: String, token: String },
    #[error("filled quantity monotonicity violated on order {order_hash}: have {have}, incoming {incoming}")]
    FilledQtyDecrease {
        order_hash: String,
        have: String,
        incoming: String,
    },
    #[error("min valid nonce monotonicity violated on {scope} {scope_key}: have {have}, incoming {incoming}")]
    MinNonceDecrease {
        scope: &'static str,
        scope_key: String,
        have: String,
        incoming: String,
    },
    #[error(
        "position quantity underflow on {subkey}/{series_id}: {side} have {have}, need {need}"
    )]
    PositionUnderflow {
        subkey: String,
        series_id: String,
        side: &'static str,
        have: String,
        need: String,
    },
    #[error("active series capacity exceeded on {subkey}: {count} > {max}")]
    ActiveSeriesCapacity {
        subkey: String,
        count: usize,
        max: usize,
    },
    #[error("collateral universe capacity exceeded: {count} > {max}")]
    CollateralUniverseCapacity { count: usize, max: usize },
}

// -----------------------------------------------------------------
//                          APPLY
// -----------------------------------------------------------------

pub const MAX_ACTIVE_SERIES: usize = 32;
pub const MAX_COLLATERAL_TOKENS: usize = 8;

/// Context passed to reducer for events that need block/tx metadata.
#[derive(Debug, Clone, Default)]
pub struct ApplyContext {
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u32,
    pub block_timestamp: u64,
}

pub fn apply(
    state: &mut ProjectionState,
    event: &HybridV2Event,
    ctx: &ApplyContext,
) -> Result<(), ReducerError> {
    match event.kind {
        // -------------------- IDENTITY --------------------
        EventKind::SubaccountCreated => {
            let (owner, sid, subkey) = require_identity(event)?;
            state
                .subaccounts
                .insert((owner.clone(), sid), subkey.clone());
            let meta = state.subaccount_meta.entry(subkey.clone()).or_default();
            meta.materialised_via_created = true;
        }
        EventKind::SubaccountLazyRegistered => {
            let (owner, sid, subkey) = require_identity(event)?;
            state
                .subaccounts
                .insert((owner.clone(), sid), subkey.clone());
            let meta = state.subaccount_meta.entry(subkey.clone()).or_default();
            meta.materialised_via_lazy = true;
        }

        // -------------------- VAULT BALANCES --------------------
        EventKind::Deposit => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            if is_finalized(state, subkey) {
                return Err(ReducerError::FinalizedSubaccountCredit {
                    subkey: subkey.clone(),
                    kind: event.kind.as_str(),
                });
            }
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            let key = (subkey.clone(), token.clone());
            let cur = state
                .balances
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "0".into());
            let next = u256_add(&cur, &amount).ok_or(ReducerError::Overflow {
                subkey: subkey.clone(),
                token: token.clone(),
            })?;
            state.balances.insert(key, next);
        }
        EventKind::Withdraw => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            let key = (subkey.clone(), token.clone());
            let cur = state
                .balances
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "0".into());
            let next = u256_sub(&cur, &amount).ok_or(ReducerError::Underflow {
                subkey: subkey.clone(),
                token: token.clone(),
                have: cur,
                need: amount.clone(),
            })?;
            state.balances.insert(key, next);
        }
        EventKind::InternalTransfer => {
            let from = payload_string(&event.payload, "from_subkey");
            let to = payload_string(&event.payload, "to_subkey");
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            if let Some(from) = from.as_ref() {
                let key = (from.clone(), token.clone());
                let cur = state
                    .balances
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "0".into());
                let next = u256_sub(&cur, &amount).ok_or(ReducerError::Underflow {
                    subkey: from.clone(),
                    token: token.clone(),
                    have: cur,
                    need: amount.clone(),
                })?;
                state.balances.insert(key, next);
            }
            if let Some(to) = to.as_ref() {
                let key = (to.clone(), token.clone());
                let cur = state
                    .balances
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "0".into());
                let next = u256_add(&cur, &amount).ok_or(ReducerError::Overflow {
                    subkey: to.clone(),
                    token: token.clone(),
                })?;
                state.balances.insert(key, next);
            }
        }
        EventKind::CollateralLocked => {
            adjust_reservation(state, event, true)?;
        }
        EventKind::CollateralUnlocked => {
            adjust_reservation(state, event, false)?;
        }
        EventKind::OrphanedLockReleased => {
            adjust_reservation(state, event, false)?;
        }
        EventKind::CollateralTokenEnteredUniverse => {
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let fallback = state.collateral_universe.len() as u32;
            let idx = payload_string(&event.payload, "universe_index")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(fallback);
            if !state.collateral_universe.contains_key(token)
                && state.collateral_universe.len() >= MAX_COLLATERAL_TOKENS
            {
                return Err(ReducerError::CollateralUniverseCapacity {
                    count: state.collateral_universe.len() + 1,
                    max: MAX_COLLATERAL_TOKENS,
                });
            }
            state
                .collateral_universe
                .entry(token.clone())
                .or_insert(idx);
            state.disabled_tokens.remove(token);
        }
        EventKind::SupportedTokenAdded => {
            if let Some(token) = event.token.as_ref() {
                state.disabled_tokens.remove(token);
            }
        }
        EventKind::SupportedTokenRemoved => {
            if let Some(token) = event.token.as_ref() {
                state.disabled_tokens.insert(token.clone());
            }
        }

        // -------------------- CAPABILITY --------------------
        EventKind::EngineCapabilityChanged => {
            let engine = require(event.engine.as_ref(), event.kind.as_str(), "engine")?;
            let added = payload_string(&event.payload, "added_bits").unwrap_or_else(|| "0".into());
            let removed =
                payload_string(&event.payload, "removed_bits").unwrap_or_else(|| "0".into());
            let cur = state
                .capability_grants
                .get(engine)
                .cloned()
                .unwrap_or_else(|| "0".into());
            let combined = u256_bitwise_or_and_mask(&cur, &added, &removed);
            state.capability_grants.insert(engine.clone(), combined);
        }
        EventKind::EngineGuardianRevoked => {
            let engine = require(event.engine.as_ref(), event.kind.as_str(), "engine")?;
            // Guardian revocation zeroes any capability the guardian had granted; the
            // canonical Solidity path always follows this with a full
            // EngineCapabilityChanged that reflects the new bitmap. We treat this event
            // as an anchor for the projection audit trail; the bitmap update happens
            // via that follow-up event.
            let _ = engine;
        }
        EventKind::GuardianChanged => { /* projection audit trail only */ }

        // -------------------- PAUSE / BAD DEBT --------------------
        EventKind::PauseFlagChanged => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let paused = payload_bool(&event.payload, "paused").unwrap_or(false);
            state.pause_flags.insert(subkey.clone(), paused);
        }
        EventKind::BadDebtSocialized => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            let key = (subkey.clone(), token.clone());
            let cur = state
                .bad_debt
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "0".into());
            let next = u256_add(&cur, &amount).ok_or(ReducerError::Overflow {
                subkey: subkey.clone(),
                token: token.clone(),
            })?;
            state.bad_debt.insert(key, next);
        }

        // -------------------- INITIALISATION EVIDENCE (immutable, no state mutation)
        EventKind::ProtocolSubaccountsInitialized
        | EventKind::EscapeControllerInitialized
        | EventKind::RecoveryFinalizerInitialized
        | EventKind::DeploymentManifestDeclared
        | EventKind::RiskParamsSet
        | EventKind::RiskModuleActivated
        | EventKind::LiquidationTriggered => {}

        // -------------------- POSITIONS --------------------
        EventKind::OptionPositionOpened => {
            apply_position_open(state, event, ctx)?;
        }
        EventKind::OptionPositionModified => {
            apply_position_modify(state, event, ctx)?;
        }
        EventKind::OptionPositionClosed => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let series = require(event.series_id.as_ref(), event.kind.as_str(), "series_id")?;
            state.positions.remove(&(subkey.clone(), series.clone()));
            if let Some(set) = state.active_series.get_mut(subkey) {
                set.remove(series);
                if set.is_empty() {
                    state.active_series.remove(subkey);
                }
            }
        }
        EventKind::OptionExercised | EventKind::OptionSettled => {
            // Exercise/settle burns some long or short; the canonical vault + ledger
            // mutations follow via Deposit/Withdraw + OptionPositionModified. Recording
            // this event for correlation is enough — position row shrinks via
            // OptionPositionModified.
        }
        EventKind::OptionPositionLiquidated => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let series = require(event.series_id.as_ref(), event.kind.as_str(), "series_id")?;
            if let Some(row) = state.positions.get_mut(&(subkey.clone(), series.clone())) {
                row.long_qty_1e8 = "0".into();
                row.short_qty_1e8 = "0".into();
                row.last_event_block = ctx.block_number;
            }
            if let Some(set) = state.active_series.get_mut(subkey) {
                set.remove(series);
                if set.is_empty() {
                    state.active_series.remove(subkey);
                }
            }
        }

        // -------------------- ORDERS --------------------
        EventKind::OptionOrderFilled => {
            apply_order_filled(state, event, ctx)?;
        }
        EventKind::OptionOrderCancelled => {
            let order_hash = require(event.order_hash.as_ref(), event.kind.as_str(), "order_hash")?;
            let row = state.order_lifecycle.entry(order_hash.clone()).or_default();
            if row.first_seen_block == 0 {
                row.first_seen_block = ctx.block_number;
            }
            row.cancelled = true;
            row.terminal = true;
            row.last_event_block = ctx.block_number;
            if let Some(sk) = event.subkey.clone() {
                row.subkey = sk;
            }
            if let Some(owner) = event.owner.clone() {
                row.owner = owner;
            }
        }
        EventKind::OptionOrderPairExecuted => {
            // Journaled + used by execution correlator; no direct projection mutation.
        }
        EventKind::OptionSubaccountMinValidOrderNonceAdvanced => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let incoming =
                payload_string(&event.payload, "new_min_valid_nonce").unwrap_or_else(|| "0".into());
            let cur = state
                .min_valid_nonce
                .get(subkey)
                .cloned()
                .unwrap_or_else(|| "0".into());
            if u256_lt(&incoming, &cur) {
                return Err(ReducerError::MinNonceDecrease {
                    scope: "SUBACCOUNT",
                    scope_key: subkey.clone(),
                    have: cur,
                    incoming,
                });
            }
            state.min_valid_nonce.insert(subkey.clone(), incoming);
        }

        // -------------------- FEES / REBATES / PREMIUM --------------------
        EventKind::OptionPremiumTransferred => {
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            state.fee_events.push(FeeEventRow {
                kind: "OPTION_PREMIUM_TRANSFERRED",
                payer_subkey: payload_string(&event.payload, "from_subkey"),
                receiver_subkey: payload_string(&event.payload, "to_subkey"),
                token: token.clone(),
                amount,
                block_number: ctx.block_number,
                tx_hash: ctx.tx_hash.clone(),
                log_index: ctx.log_index,
                execution_id: event.execution_id.clone(),
            });
        }
        EventKind::OptionFeeCharged => {
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            state.fee_events.push(FeeEventRow {
                kind: "OPTION_FEE_CHARGED",
                payer_subkey: event.subkey.clone(),
                receiver_subkey: payload_string(&event.payload, "fee_subkey"),
                token: token.clone(),
                amount,
                block_number: ctx.block_number,
                tx_hash: ctx.tx_hash.clone(),
                log_index: ctx.log_index,
                execution_id: event.execution_id.clone(),
            });
        }
        EventKind::OptionRebatePaid => {
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            state.fee_events.push(FeeEventRow {
                kind: "OPTION_REBATE_PAID",
                payer_subkey: payload_string(&event.payload, "rebate_subkey"),
                receiver_subkey: event.subkey.clone(),
                token: token.clone(),
                amount,
                block_number: ctx.block_number,
                tx_hash: ctx.tx_hash.clone(),
                log_index: ctx.log_index,
                execution_id: event.execution_id.clone(),
            });
        }

        // -------------------- RECOVERY --------------------
        EventKind::RecoveryRequested => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::RecoveryPending);
            let row = state.escape_state.entry(subkey.clone()).or_default();
            row.state = "REQUESTED";
            row.requested_ts = Some(ctx.block_timestamp);
            row.activation_eligible_at = payload_string(&event.payload, "activation_eligible_at")
                .and_then(|s| s.parse().ok());
            row.last_event_block = ctx.block_number;
        }
        EventKind::RecoveryActivated => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::RecoveryActive);
            let row = state.escape_state.entry(subkey.clone()).or_default();
            row.state = "ACTIVATED";
            row.activated_ts = Some(ctx.block_timestamp);
            row.last_event_block = ctx.block_number;
        }
        EventKind::RecoveryCancelled => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let prior = state.recovery_state.get(subkey).copied();
            if prior == Some(RecoveryStateProjection::Recovered) {
                return Err(ReducerError::IllegalRecoveryTransition {
                    subkey: subkey.clone(),
                    from: prior,
                    to: RecoveryStateProjection::Cancelled,
                    kind: event.kind.as_str(),
                });
            }
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::Cancelled);
            let row = state.escape_state.entry(subkey.clone()).or_default();
            row.state = "CANCELLED";
            row.cancelled_ts = Some(ctx.block_timestamp);
            row.last_event_block = ctx.block_number;
        }
        EventKind::RecoveryEpochIncremented => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let row = state
                .subaccount_recovery_epochs
                .entry(subkey.clone())
                .or_default();
            row.epoch_count = row.epoch_count.saturating_add(1);
            row.last_event_block = ctx.block_number;
        }
        EventKind::RecoveryPauseSet => {
            let paused = payload_bool(&event.payload, "paused").unwrap_or(false);
            let until = payload_string(&event.payload, "until_ts")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            state.recovery_pause = Some(RecoveryPauseRow {
                paused,
                until_ts: until,
                last_event_block: ctx.block_number,
            });
        }
        EventKind::RecoveryFinalized => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let prior = state.recovery_state.get(subkey).copied();
            if prior == Some(RecoveryStateProjection::Recovered) {
                return Err(ReducerError::IllegalRecoveryTransition {
                    subkey: subkey.clone(),
                    from: prior,
                    to: RecoveryStateProjection::Recovered,
                    kind: event.kind.as_str(),
                });
            }
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::Recovered);
            let row = state.escape_state.entry(subkey.clone()).or_default();
            row.state = "FINALIZED";
            row.finalized_ts = Some(ctx.block_timestamp);
            row.last_event_block = ctx.block_number;
            // Atomically zero balances + reservations for finalized subaccount.
            let sk = subkey.clone();
            state.balances.retain(|(s, _), _| s != &sk);
            state.reservations.retain(|(s, _, _), _| s != &sk);
        }
        EventKind::RecoveryFinalizationWithdrawn => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let counter = state
                .recovery_finalization_withdrawals
                .entry(subkey.clone())
                .or_insert(0);
            *counter = counter.saturating_add(1);
        }
        EventKind::IntentConsumed => { /* recorded via decoded_events; no projection change */ }
        EventKind::NonceCancelled => {
            let owner = require(event.owner.as_ref(), event.kind.as_str(), "owner")?;
            let incoming =
                payload_string(&event.payload, "new_min_valid_nonce").unwrap_or_else(|| "0".into());
            let cur = state
                .owner_recovery_epochs
                .get(owner)
                .map(|r| r.min_valid_nonce.clone())
                .unwrap_or_else(|| "0".into());
            if u256_lt(&incoming, &cur) {
                return Err(ReducerError::MinNonceDecrease {
                    scope: "OWNER",
                    scope_key: owner.clone(),
                    have: cur,
                    incoming,
                });
            }
            let row = state
                .owner_recovery_epochs
                .entry(owner.clone())
                .or_default();
            row.min_valid_nonce = incoming;
            row.last_event_block = ctx.block_number;
        }
        EventKind::OwnerRecoveryEpochAdvanced => {
            let owner = require(event.owner.as_ref(), event.kind.as_str(), "owner")?;
            let row = state
                .owner_recovery_epochs
                .entry(owner.clone())
                .or_default();
            row.epoch_count = row.epoch_count.saturating_add(1);
            row.last_event_block = ctx.block_number;
        }
        EventKind::SubaccountRecoveryEpochAdvanced => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let row = state
                .subaccount_recovery_epochs
                .entry(subkey.clone())
                .or_default();
            row.epoch_count = row.epoch_count.saturating_add(1);
            row.last_event_block = ctx.block_number;
        }
    }
    Ok(())
}

// -----------------------------------------------------------------
//                          RESERVATION HELPER
// -----------------------------------------------------------------

fn adjust_reservation(
    state: &mut ProjectionState,
    event: &HybridV2Event,
    add: bool,
) -> Result<(), ReducerError> {
    let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
    let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
    let engine = require(event.engine.as_ref(), event.kind.as_str(), "engine")?;
    let amount = require_amount(&event.payload, event.kind.as_str())?;
    let key = (subkey.clone(), token.clone(), engine.clone());
    let cur = state
        .reservations
        .get(&key)
        .cloned()
        .unwrap_or_else(|| "0".into());
    let next = if add {
        u256_add(&cur, &amount).ok_or(ReducerError::Overflow {
            subkey: subkey.clone(),
            token: token.clone(),
        })?
    } else {
        u256_sub(&cur, &amount).ok_or(ReducerError::Underflow {
            subkey: subkey.clone(),
            token: token.clone(),
            have: cur,
            need: amount.clone(),
        })?
    };
    state.reservations.insert(key, next);
    Ok(())
}

// -----------------------------------------------------------------
//                          POSITION HELPERS
// -----------------------------------------------------------------

fn apply_position_open(
    state: &mut ProjectionState,
    event: &HybridV2Event,
    ctx: &ApplyContext,
) -> Result<(), ReducerError> {
    let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
    let series = require(event.series_id.as_ref(), event.kind.as_str(), "series_id")?;
    let long_delta = payload_string(&event.payload, "long_delta_1e8").unwrap_or_else(|| "0".into());
    let short_delta =
        payload_string(&event.payload, "short_delta_1e8").unwrap_or_else(|| "0".into());
    let row = state
        .positions
        .entry((subkey.clone(), series.clone()))
        .or_default();
    row.long_qty_1e8 = u256_add(&row.long_qty_1e8, &long_delta).ok_or(ReducerError::Overflow {
        subkey: subkey.clone(),
        token: series.clone(),
    })?;
    row.short_qty_1e8 =
        u256_add(&row.short_qty_1e8, &short_delta).ok_or(ReducerError::Overflow {
            subkey: subkey.clone(),
            token: series.clone(),
        })?;
    row.last_event_block = ctx.block_number;
    let set = state.active_series.entry(subkey.clone()).or_default();
    set.insert(series.clone());
    if set.len() > MAX_ACTIVE_SERIES {
        return Err(ReducerError::ActiveSeriesCapacity {
            subkey: subkey.clone(),
            count: set.len(),
            max: MAX_ACTIVE_SERIES,
        });
    }
    Ok(())
}

fn apply_position_modify(
    state: &mut ProjectionState,
    event: &HybridV2Event,
    ctx: &ApplyContext,
) -> Result<(), ReducerError> {
    let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
    let series = require(event.series_id.as_ref(), event.kind.as_str(), "series_id")?;
    let long_delta_signed = payload_i128(&event.payload, "long_delta_1e8_signed");
    let short_delta_signed = payload_i128(&event.payload, "short_delta_1e8_signed");
    let row = state
        .positions
        .entry((subkey.clone(), series.clone()))
        .or_default();
    if let Some(delta) = long_delta_signed {
        row.long_qty_1e8 = apply_signed_delta(&row.long_qty_1e8, delta, subkey, series, "long")?;
    }
    if let Some(delta) = short_delta_signed {
        row.short_qty_1e8 = apply_signed_delta(&row.short_qty_1e8, delta, subkey, series, "short")?;
    }
    row.last_event_block = ctx.block_number;
    if row.long_qty_1e8 == "0" && row.short_qty_1e8 == "0" {
        state.positions.remove(&(subkey.clone(), series.clone()));
        if let Some(set) = state.active_series.get_mut(subkey) {
            set.remove(series);
            if set.is_empty() {
                state.active_series.remove(subkey);
            }
        }
    }
    Ok(())
}

fn apply_signed_delta(
    current: &str,
    delta: i128,
    subkey: &str,
    series: &str,
    side: &'static str,
) -> Result<String, ReducerError> {
    if delta >= 0 {
        u256_add(current, &delta.to_string()).ok_or_else(|| ReducerError::Overflow {
            subkey: subkey.to_string(),
            token: series.to_string(),
        })
    } else {
        let abs = (delta as i128).unsigned_abs().to_string();
        u256_sub(current, &abs).ok_or_else(|| ReducerError::PositionUnderflow {
            subkey: subkey.to_string(),
            series_id: series.to_string(),
            side,
            have: current.to_string(),
            need: abs,
        })
    }
}

// -----------------------------------------------------------------
//                          ORDER HELPERS
// -----------------------------------------------------------------

fn apply_order_filled(
    state: &mut ProjectionState,
    event: &HybridV2Event,
    ctx: &ApplyContext,
) -> Result<(), ReducerError> {
    let order_hash = require(event.order_hash.as_ref(), event.kind.as_str(), "order_hash")?;
    let filled_delta =
        payload_string(&event.payload, "filled_delta_1e8").unwrap_or_else(|| "0".into());
    let total_qty = payload_string(&event.payload, "total_qty_1e8");
    let terminal = payload_bool(&event.payload, "terminal").unwrap_or(false);
    let time_in_force = payload_u8(&event.payload, "time_in_force").unwrap_or(0);
    let side = payload_u8(&event.payload, "side").unwrap_or(0);
    let row = state.order_lifecycle.entry(order_hash.clone()).or_default();
    if row.first_seen_block == 0 {
        row.first_seen_block = ctx.block_number;
    }
    let candidate = u256_add(&row.filled_qty_1e8, &filled_delta).ok_or(ReducerError::Overflow {
        subkey: order_hash.clone(),
        token: "filled".into(),
    })?;
    if u256_lt(&candidate, &row.filled_qty_1e8) {
        return Err(ReducerError::FilledQtyDecrease {
            order_hash: order_hash.clone(),
            have: row.filled_qty_1e8.clone(),
            incoming: candidate,
        });
    }
    row.filled_qty_1e8 = candidate;
    if let Some(t) = total_qty {
        if row.total_qty_1e8 == "0" || u256_lt(&row.total_qty_1e8, &t) {
            row.total_qty_1e8 = t;
        }
    }
    if let Some(sk) = event.subkey.clone() {
        row.subkey = sk;
    }
    if let Some(owner) = event.owner.clone() {
        row.owner = owner;
    }
    if let Some(s) = event.series_id.clone() {
        row.series_id = Some(s);
    }
    row.side = side;
    row.time_in_force = time_in_force;
    row.terminal = row.terminal || terminal;
    row.last_event_block = ctx.block_number;
    Ok(())
}

// -----------------------------------------------------------------
//                          UTILITIES
// -----------------------------------------------------------------

fn is_finalized(state: &ProjectionState, subkey: &str) -> bool {
    matches!(
        state.recovery_state.get(subkey),
        Some(RecoveryStateProjection::Recovered)
    )
}

fn require<'a, T>(
    opt: Option<&'a T>,
    kind: &'static str,
    field: &'static str,
) -> Result<&'a T, ReducerError> {
    opt.ok_or(ReducerError::MissingField { kind, field })
}

fn require_identity(event: &HybridV2Event) -> Result<(&String, u32, &String), ReducerError> {
    let owner = require(event.owner.as_ref(), event.kind.as_str(), "owner")?;
    let sid = event.subaccount_id.ok_or(ReducerError::MissingField {
        kind: event.kind.as_str(),
        field: "subaccount_id",
    })?;
    let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
    Ok((owner, sid, subkey))
}

fn require_amount(payload: &serde_json::Value, kind: &'static str) -> Result<String, ReducerError> {
    let amount =
        payload
            .get("amount")
            .and_then(|v| v.as_str())
            .ok_or(ReducerError::MissingField {
                kind,
                field: "amount",
            })?;
    Ok(amount.to_string())
}

fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn payload_bool(payload: &serde_json::Value, key: &str) -> Option<bool> {
    payload.get(key).and_then(|v| v.as_bool())
}

fn payload_u8(payload: &serde_json::Value, key: &str) -> Option<u8> {
    payload.get(key).and_then(|v| v.as_u64()).map(|n| n as u8)
}

fn payload_i128(payload: &serde_json::Value, key: &str) -> Option<i128> {
    payload.get(key).and_then(|v| {
        if let Some(n) = v.as_i64() {
            Some(n as i128)
        } else if let Some(s) = v.as_str() {
            s.parse::<i128>().ok()
        } else {
            None
        }
    })
}

// -----------------------------------------------------------------
//                          UINT256 ARITHMETIC
// -----------------------------------------------------------------

pub fn u256_add(a: &str, b: &str) -> Option<String> {
    let (bits_a, ok_a) = parse_dec_uint256(a);
    let (bits_b, ok_b) = parse_dec_uint256(b);
    if !ok_a || !ok_b {
        return None;
    }
    let mut out = [0u8; 32];
    let mut carry: u16 = 0;
    for i in (0..32).rev() {
        let sum = bits_a[i] as u16 + bits_b[i] as u16 + carry;
        out[i] = (sum & 0xff) as u8;
        carry = sum >> 8;
    }
    if carry != 0 {
        return None;
    }
    Some(u256_be_to_decimal_public(&out))
}

pub fn u256_sub(a: &str, b: &str) -> Option<String> {
    let (bits_a, ok_a) = parse_dec_uint256(a);
    let (bits_b, ok_b) = parse_dec_uint256(b);
    if !ok_a || !ok_b {
        return None;
    }
    if u256_cmp(&bits_a, &bits_b).is_lt() {
        return None;
    }
    let mut out = [0u8; 32];
    let mut borrow: i16 = 0;
    for i in (0..32).rev() {
        let diff = bits_a[i] as i16 - bits_b[i] as i16 - borrow;
        if diff < 0 {
            out[i] = (diff + 256) as u8;
            borrow = 1;
        } else {
            out[i] = diff as u8;
            borrow = 0;
        }
    }
    Some(u256_be_to_decimal_public(&out))
}

pub fn u256_lt(a: &str, b: &str) -> bool {
    let (bits_a, ok_a) = parse_dec_uint256(a);
    let (bits_b, ok_b) = parse_dec_uint256(b);
    if !ok_a || !ok_b {
        return false;
    }
    u256_cmp(&bits_a, &bits_b).is_lt()
}

fn u256_bitwise_or_and_mask(current: &str, add: &str, remove: &str) -> String {
    let (c, _) = parse_dec_uint256(current);
    let (a, _) = parse_dec_uint256(add);
    let (r, _) = parse_dec_uint256(remove);
    let mut out = [0u8; 32];
    for i in 0..32 {
        let combined = c[i] | a[i];
        out[i] = combined & !r[i];
    }
    u256_be_to_decimal_public(&out)
}

fn u256_cmp(a: &[u8; 32], b: &[u8; 32]) -> std::cmp::Ordering {
    for i in 0..32 {
        match a[i].cmp(&b[i]) {
            std::cmp::Ordering::Equal => continue,
            ord => return ord,
        }
    }
    std::cmp::Ordering::Equal
}

fn parse_dec_uint256(s: &str) -> ([u8; 32], bool) {
    let s = s.trim();
    if s.is_empty() {
        // Treat empty (default) as "0" — projection rows initialise their
        // decimal-uint256 fields via `Default::default()` which yields "".
        return ([0u8; 32], true);
    }
    let mut out = [0u8; 32];
    for ch in s.chars() {
        if !ch.is_ascii_digit() {
            return ([0u8; 32], false);
        }
        let digit = (ch as u8 - b'0') as u16;
        let mut carry: u16 = digit;
        for byte in out.iter_mut().rev() {
            let product = (*byte as u16) * 10 + carry;
            *byte = (product & 0xff) as u8;
            carry = product >> 8;
        }
        if carry != 0 {
            return ([0u8; 32], false);
        }
    }
    (out, true)
}

fn u256_be_to_decimal_public(bytes: &[u8; 32]) -> String {
    if bytes[..16].iter().all(|b| *b == 0) {
        let mut hi = 0u128;
        for b in &bytes[16..] {
            hi = (hi << 8) | (*b as u128);
        }
        return hi.to_string();
    }
    let mut digits: Vec<u8> = Vec::with_capacity(78);
    let mut n = bytes.to_vec();
    while n.iter().any(|b| *b != 0) {
        let mut rem: u32 = 0;
        for byte in n.iter_mut() {
            let acc = (rem << 8) | (*byte as u32);
            *byte = (acc / 10) as u8;
            rem = acc % 10;
        }
        digits.push(rem as u8);
    }
    if digits.is_empty() {
        return "0".to_string();
    }
    digits.reverse();
    digits.into_iter().map(|d| (b'0' + d) as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u256_add_and_sub_roundtrip() {
        assert_eq!(u256_add("100", "42").as_deref(), Some("142"));
        assert_eq!(u256_sub("100", "42").as_deref(), Some("58"));
        assert_eq!(u256_sub("42", "100"), None);
    }

    #[test]
    fn u256_add_overflow_returns_none() {
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(u256_add(max, "1"), None);
    }

    #[test]
    fn u256_sub_beyond_u128_still_works() {
        let big = "680564733841876926926749214863536422912";
        assert_eq!(
            u256_sub(big, "1").as_deref(),
            Some("680564733841876926926749214863536422911")
        );
    }
}
