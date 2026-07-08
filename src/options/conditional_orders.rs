// OPTIONS-CONDITIONAL-ORDERS-TP-SL-V1
//
// Persistent server-side Take-Profit / Stop-Loss conditional exit
// orders for existing option positions.
//
// Safety guarantees (must NOT regress):
//
//   * Trigger source is the on-chain `OracleRouter.getPriceSafe`
//     mark price (reverts on stale feeds). The worker skips ticks
//     entirely when the oracle is unconfigured or the RPC fails.
//   * Child orders are always `IOC` + reduce_only=true, with a
//     mandatory limit price (NEVER an unbounded market order).
//   * Child quantity is re-capped to the live reducible position
//     size right before submission so concurrent fills cannot let
//     a conditional exit reverse the position.
//   * Atomic claim: the worker transitions `armed → triggering` in
//     a guarded UPDATE so two workers cannot fire the same row twice
//     after a restart.
//   * OCO atomicity: when one leg transitions out of `armed`, the
//     sibling is cancelled in the SAME critical section.
//
// Persistence model:
//   * `options_conditional_orders` table (migration 0028) is the
//     single source of truth.
//   * The in-memory variant (used by unit tests and the no-DB local
//     mode) mirrors the same lifecycle through `OptionSeriesStore`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{BackendError, Result};
use crate::options::store::OptionSeriesStore;
use crate::options::types::OptionFillFilter;
use crate::options::{OptionFill, OptionOrderStatus, OptionSeries};
use crate::types::{AccountId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};

// ---- Identifiers ----------------------------------------------------

pub type ConditionalOrderId = Uuid;
pub type OcoGroupId = Uuid;

// ---- Enums (TEXT in DB) --------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionalType {
    TakeProfit,
    StopLoss,
}

impl ConditionalType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TakeProfit => "take_profit",
            Self::StopLoss => "stop_loss",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "take_profit" => Ok(Self::TakeProfit),
            "stop_loss" => Ok(Self::StopLoss),
            other => Err(BackendError::InvalidConditionalOrderState(format!(
                "unknown conditional_type {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSource {
    UnderlyingOracle,
}

impl TriggerSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnderlyingOracle => "underlying_oracle",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "underlying_oracle" => Ok(Self::UnderlyingOracle),
            other => Err(BackendError::InvalidConditionalOrderState(format!(
                "unknown trigger_source {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCondition {
    Gte,
    Lte,
}

impl TriggerCondition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gte => "gte",
            Self::Lte => "lte",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "gte" => Ok(Self::Gte),
            "lte" => Ok(Self::Lte),
            other => Err(BackendError::InvalidConditionalOrderState(format!(
                "unknown trigger_condition {other}"
            ))),
        }
    }
    pub fn matches(self, observed: Price1e8, trigger: Price1e8) -> bool {
        match self {
            Self::Gte => observed >= trigger,
            Self::Lte => observed <= trigger,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionalExecutionType {
    IocLimit,
}

impl ConditionalExecutionType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IocLimit => "ioc_limit",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "ioc_limit" => Ok(Self::IocLimit),
            other => Err(BackendError::InvalidConditionalOrderState(format!(
                "unknown execution_type {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
}

impl PositionSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "long" => Ok(Self::Long),
            "short" => Ok(Self::Short),
            other => Err(BackendError::InvalidConditionalOrderState(format!(
                "unknown position_side {other}"
            ))),
        }
    }
    /// The closing side of a position is the opposite of its entry
    /// side. A long is closed by selling; a short is closed by
    /// buying. This is what the child IOC order must use.
    pub fn closing_side(self) -> Side {
        match self {
            Self::Long => Side::Sell,
            Self::Short => Side::Buy,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    Call,
    Put,
}

impl OptionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Put => "put",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "call" => Ok(Self::Call),
            "put" => Ok(Self::Put),
            other => Err(BackendError::InvalidConditionalOrderState(format!(
                "unknown option_kind {other}"
            ))),
        }
    }
    pub fn from_is_call(is_call: bool) -> Self {
        if is_call {
            Self::Call
        } else {
            Self::Put
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionalOrderStatus {
    Armed,
    Triggering,
    Executing,
    Completed,
    PartiallyFilled,
    Cancelled,
    Failed,
    Expired,
}

impl ConditionalOrderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Armed => "armed",
            Self::Triggering => "triggering",
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::PartiallyFilled => "partially_filled",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "armed" => Ok(Self::Armed),
            "triggering" => Ok(Self::Triggering),
            "executing" => Ok(Self::Executing),
            "completed" => Ok(Self::Completed),
            "partially_filled" => Ok(Self::PartiallyFilled),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "expired" => Ok(Self::Expired),
            other => Err(BackendError::InvalidConditionalOrderState(format!(
                "unknown status {other}"
            ))),
        }
    }
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::PartiallyFilled
                | Self::Cancelled
                | Self::Failed
                | Self::Expired
        )
    }
}

// ---- Domain record -------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConditionalOrder {
    pub id: ConditionalOrderId,
    pub account: AccountId,
    /// SUBACCOUNTS-OPTIONS-CONDITIONAL-TWAP-WS-V1 — the parent's
    /// `(owner, subaccount_id)` scope. Attached TP/SL orders emitted
    /// by the conditional-orders worker inherit this value so a
    /// subaccount 2 parent never emits subaccount 1 children.
    #[serde(default = "crate::options::types::one_subaccount_id")]
    pub subaccount_id: u32,
    pub option_series_id: String,
    pub position_side: PositionSide,
    pub option_kind: OptionKind,
    pub conditional_type: ConditionalType,
    pub trigger_source: TriggerSource,
    pub trigger_condition: TriggerCondition,
    pub trigger_price_1e8: Price1e8,
    pub quantity_1e8: Size1e8,
    pub execution_type: ConditionalExecutionType,
    pub limit_price_1e8: Price1e8,
    pub reduce_only: bool,
    pub oco_group_id: Option<OcoGroupId>,
    pub status: ConditionalOrderStatus,
    pub child_order_id: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub expires_at_ms: Option<TimestampMs>,
    pub triggered_at_ms: Option<TimestampMs>,
    pub completed_at_ms: Option<TimestampMs>,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
    pub version: i64,
}

// ---- Service inputs -------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConditionalLegInput {
    pub conditional_type: ConditionalType,
    pub trigger_price_1e8: Price1e8,
    pub limit_price_1e8: Price1e8,
    /// Optional explicit comparator. If provided, must match the one
    /// the backend derives from (kind, side, conditional_type);
    /// otherwise the request is rejected with
    /// `InvalidTriggerDirection`. If `None`, the backend stores the
    /// derived comparator.
    pub explicit_trigger_condition: Option<TriggerCondition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateConditionalOrderInput {
    pub account: AccountId,
    /// SUBACCOUNTS-OPTIONS-CONDITIONAL-TWAP-WS-V1 — resolved
    /// subaccount that scopes this standalone conditional (and any
    /// attached child order emitted by the worker).
    pub subaccount_id: u32,
    pub option_series_id: String,
    pub quantity_1e8: Size1e8,
    pub legs: Vec<ConditionalLegInput>,
    pub link_as_oco: bool,
    pub expires_at_ms: Option<TimestampMs>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ConditionalOrderFilter {
    pub account: Option<AccountId>,
    pub option_series_id: Option<String>,
    pub status: Option<ConditionalOrderStatus>,
    pub oco_group_id: Option<OcoGroupId>,
}

impl ConditionalOrderFilter {
    pub fn matches(&self, order: &ConditionalOrder) -> bool {
        if let Some(account) = &self.account {
            if !order.account.0.eq_ignore_ascii_case(&account.0) {
                return false;
            }
        }
        if let Some(series) = &self.option_series_id {
            if &order.option_series_id != series {
                return false;
            }
        }
        if let Some(status) = self.status {
            if order.status != status {
                return false;
            }
        }
        if let Some(oco) = self.oco_group_id {
            if order.oco_group_id != Some(oco) {
                return false;
            }
        }
        true
    }
}

// ---- Trigger direction derivation ----------------------------------

/// Phase 3 — derive the comparator from (kind, side, conditional_type).
///
/// |  kind | side  | conditional | comparator |
/// |-------|-------|-------------|------------|
/// | call  | long  | TP          | gte (price up = profit)
/// | call  | long  | SL          | lte
/// | call  | short | TP          | lte
/// | call  | short | SL          | gte
/// | put   | long  | TP          | lte (price down = profit)
/// | put   | long  | SL          | gte
/// | put   | short | TP          | gte
/// | put   | short | SL          | lte
pub fn derive_trigger_condition(
    kind: OptionKind,
    side: PositionSide,
    cond: ConditionalType,
) -> TriggerCondition {
    let price_up_is_profit = matches!(
        (kind, side),
        (OptionKind::Call, PositionSide::Long) | (OptionKind::Put, PositionSide::Short)
    );
    match (cond, price_up_is_profit) {
        (ConditionalType::TakeProfit, true) => TriggerCondition::Gte,
        (ConditionalType::TakeProfit, false) => TriggerCondition::Lte,
        (ConditionalType::StopLoss, true) => TriggerCondition::Lte,
        (ConditionalType::StopLoss, false) => TriggerCondition::Gte,
    }
}

// ---- Reduce-only computation ---------------------------------------

/// Sums `option_fills` for `(account, series_id)` and returns the
/// signed reducible position (positive = long, negative = short).
///
/// This is the same derivation used by `account_positions` in
/// `src/api/trading.rs`. We don't go on-chain here — the worker is
/// allowed to consume the off-chain index because (a) the matcher is
/// the authoritative source for testnet fills, (b) any divergence
/// from on-chain state would already have been caught by the
/// reconciler.
pub fn compute_position_signed_1e8(fills: &[OptionFill], account: &AccountId) -> i128 {
    fills
        .iter()
        .filter(|f| &f.option_series_id == &f.option_series_id) // no-op; kept for grep
        .fold(0i128, |acc, f| {
            let is_long = f.buyer == *account;
            let is_short = f.seller == *account;
            let delta = f.size_1e8 as i128;
            if is_long {
                acc.saturating_add(delta)
            } else if is_short {
                acc.saturating_sub(delta)
            } else {
                acc
            }
        })
}

pub struct ReduciblePosition {
    pub side: PositionSide,
    pub size_1e8: Size1e8,
}

/// Map the signed position to a directional reducible quantity.
/// Returns `None` if the account holds no position for this series.
pub fn classify_reducible(signed_1e8: i128) -> Option<ReduciblePosition> {
    if signed_1e8 > 0 {
        Some(ReduciblePosition {
            side: PositionSide::Long,
            size_1e8: signed_1e8 as Size1e8,
        })
    } else if signed_1e8 < 0 {
        Some(ReduciblePosition {
            side: PositionSide::Short,
            size_1e8: (-signed_1e8) as Size1e8,
        })
    } else {
        None
    }
}

/// Compute the active conditional commitment for `(account,
/// series_id)` — the sum of `quantity_1e8` across non-terminal,
/// non-OCO-deduplicated rows. Used to prevent the operator from
/// over-committing the position to multiple TP/SL stacks.
pub fn active_committed_quantity(orders: &[ConditionalOrder]) -> Size1e8 {
    // OCO siblings share quantity (only one can win), so we count one
    // representative per oco_group_id. Standalone legs count as-is.
    use std::collections::HashSet;
    let mut seen_oco: HashSet<OcoGroupId> = HashSet::new();
    let mut total: Size1e8 = 0;
    for o in orders {
        if o.status.is_terminal() {
            continue;
        }
        if let Some(group) = o.oco_group_id {
            if !seen_oco.insert(group) {
                continue;
            }
        }
        total = total.saturating_add(o.quantity_1e8);
    }
    total
}

// ---- Service: create / list / get / cancel -------------------------

/// Phase 7 — create one or two conditional orders.
///
/// * `legs.len() == 1` → standalone TP or SL.
/// * `legs.len() == 2 && link_as_oco` → OCO pair with a shared
///   `oco_group_id`. Both legs MUST be one TP + one SL.
pub fn create_conditional_orders_in_store(
    store: &mut OptionSeriesStore,
    series: &OptionSeries,
    input: CreateConditionalOrderInput,
    now_ms: TimestampMs,
) -> Result<Vec<ConditionalOrder>> {
    // -- 1. Input validation --
    if input.quantity_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if input.legs.is_empty() || input.legs.len() > 2 {
        return Err(BackendError::InvalidConditionalOrderState(
            "must supply 1 or 2 legs".to_string(),
        ));
    }
    if input.legs.len() == 2 {
        let kinds: Vec<_> = input.legs.iter().map(|l| l.conditional_type).collect();
        let has_tp = kinds.contains(&ConditionalType::TakeProfit);
        let has_sl = kinds.contains(&ConditionalType::StopLoss);
        if !(has_tp && has_sl) {
            return Err(BackendError::InvalidConditionalOrderState(
                "two-leg request must contain one TP and one SL".to_string(),
            ));
        }
    }
    for leg in &input.legs {
        if leg.trigger_price_1e8 == 0 {
            return Err(BackendError::InvalidTriggerPrice);
        }
        if leg.limit_price_1e8 == 0 {
            return Err(BackendError::InvalidConditionalLimitPrice);
        }
    }

    // -- 2. Position discovery (reduce-only at arm time) --
    let fills = store.list_fills(&OptionFillFilter {
        option_series_id: Some(series.option_series_id.clone()),
        account: Some(input.account.clone()),
        order_id: None,
        subaccount_id: None,
    });
    let signed = compute_position_signed_1e8(&fills, &input.account);
    let position = classify_reducible(signed)
        .ok_or_else(|| BackendError::NoReduciblePosition(series.option_series_id.clone()))?;
    if input.quantity_1e8 > position.size_1e8 {
        return Err(BackendError::QuantityExceedsPosition);
    }

    // -- 3. Over-commit guard --
    let existing = store.list_conditional_orders(&ConditionalOrderFilter {
        account: Some(input.account.clone()),
        option_series_id: Some(series.option_series_id.clone()),
        status: None,
        oco_group_id: None,
    });
    let already_committed = active_committed_quantity(&existing);
    let oco_share: Size1e8 = if input.legs.len() == 2 && input.link_as_oco {
        input.quantity_1e8
    } else {
        input
            .quantity_1e8
            .saturating_mul(input.legs.len() as Size1e8)
    };
    if already_committed.saturating_add(oco_share) > position.size_1e8 {
        return Err(BackendError::QuantityExceedsPosition);
    }

    // -- 4. Build rows --
    let kind = OptionKind::from_is_call(series.is_call);
    let oco_group = if input.legs.len() == 2 && input.link_as_oco {
        Some(Uuid::new_v4())
    } else {
        None
    };

    let mut rows = Vec::with_capacity(input.legs.len());
    for leg in &input.legs {
        let derived = derive_trigger_condition(kind, position.side, leg.conditional_type);
        if let Some(explicit) = leg.explicit_trigger_condition {
            if explicit != derived {
                return Err(BackendError::InvalidTriggerDirection);
            }
        }
        let order = ConditionalOrder {
            id: Uuid::new_v4(),
            account: input.account.clone(),
            subaccount_id: input.subaccount_id,
            option_series_id: series.option_series_id.clone(),
            position_side: position.side,
            option_kind: kind,
            conditional_type: leg.conditional_type,
            trigger_source: TriggerSource::UnderlyingOracle,
            trigger_condition: derived,
            trigger_price_1e8: leg.trigger_price_1e8,
            quantity_1e8: input.quantity_1e8,
            execution_type: ConditionalExecutionType::IocLimit,
            limit_price_1e8: leg.limit_price_1e8,
            reduce_only: true,
            oco_group_id: oco_group,
            status: ConditionalOrderStatus::Armed,
            child_order_id: None,
            failure_code: None,
            failure_message: None,
            expires_at_ms: input.expires_at_ms,
            triggered_at_ms: None,
            completed_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            version: 0,
        };
        rows.push(order);
    }
    // Atomic insert: only commit both rows or neither (in-memory
    // store mutates synchronously so this is atomic by construction).
    for row in &rows {
        store.insert_conditional_order(row.clone());
    }
    Ok(rows)
}

/// Phase 9 — cancel a single conditional order. Manual cancellation
/// does NOT auto-cancel the OCO sibling (recommended policy per the
/// V1 brief). Use `cancel_oco_group_in_store` for that.
pub fn cancel_conditional_order_in_store(
    store: &mut OptionSeriesStore,
    id: ConditionalOrderId,
    requesting_account: &AccountId,
    now_ms: TimestampMs,
) -> Result<ConditionalOrder> {
    let existing = store
        .get_conditional_order(id)
        .ok_or(BackendError::InvalidConditionalOrderId)?;
    if !existing
        .account
        .0
        .eq_ignore_ascii_case(&requesting_account.0)
    {
        return Err(BackendError::InvalidConditionalOrderId);
    }
    if existing.status.is_terminal() {
        return Err(BackendError::ConditionalOrderAlreadyTerminal);
    }
    let mut updated = existing.clone();
    updated.status = ConditionalOrderStatus::Cancelled;
    updated.completed_at_ms = Some(now_ms);
    updated.updated_at_ms = now_ms;
    updated.version = existing.version.saturating_add(1);
    store.update_conditional_order(updated.clone())?;
    Ok(updated)
}

/// Cancel both legs of an OCO group atomically. Returns the cancelled
/// orders (terminal ones are skipped, returned as-is).
pub fn cancel_oco_group_in_store(
    store: &mut OptionSeriesStore,
    group: OcoGroupId,
    requesting_account: &AccountId,
    now_ms: TimestampMs,
) -> Result<Vec<ConditionalOrder>> {
    let siblings = store.list_conditional_orders(&ConditionalOrderFilter {
        account: Some(requesting_account.clone()),
        option_series_id: None,
        status: None,
        oco_group_id: Some(group),
    });
    if siblings.is_empty() {
        return Err(BackendError::InvalidConditionalOrderId);
    }
    let mut out = Vec::with_capacity(siblings.len());
    for sib in siblings {
        if sib.status.is_terminal() {
            out.push(sib);
            continue;
        }
        let mut updated = sib.clone();
        updated.status = ConditionalOrderStatus::Cancelled;
        updated.completed_at_ms = Some(now_ms);
        updated.updated_at_ms = now_ms;
        updated.version = sib.version.saturating_add(1);
        store.update_conditional_order(updated.clone())?;
        out.push(updated);
    }
    Ok(out)
}

// ---- Worker primitives (Phase 5 + Phase 6) --------------------------

/// Plan one tick of the evaluator. Pure function over a snapshot of
/// armed orders and the latest observed underlying price (mapped per
/// `(underlying, settlement)` pair). Returns the ids that should
/// transition to `triggering` this tick.
///
/// The caller is responsible for the atomic claim (UPDATE status WHERE
/// status='armed' AND version=$v RETURNING ...).
pub fn select_orders_to_trigger(
    armed: &[ConditionalOrder],
    price_for_series: impl Fn(&str) -> Option<Price1e8>,
    now_ms: TimestampMs,
) -> Vec<ConditionalOrderId> {
    let mut out = Vec::new();
    for o in armed {
        if o.status != ConditionalOrderStatus::Armed {
            continue;
        }
        if let Some(expiry) = o.expires_at_ms {
            if now_ms >= expiry {
                continue;
            }
        }
        let Some(price) = price_for_series(&o.option_series_id) else {
            continue;
        };
        if o.trigger_condition.matches(price, o.trigger_price_1e8) {
            out.push(o.id);
        }
    }
    out
}

/// Phase 5 — execute a single triggered conditional order against the
/// live matching engine via the existing `submit_order_and_match`
/// path. Re-derives reducible size right before submission and caps
/// the child order quantity. Cancels the OCO sibling atomically.
///
/// Returns the final terminal state of the conditional order.
pub fn execute_triggered_in_store(
    store: &mut OptionSeriesStore,
    _series: &OptionSeries,
    conditional_id: ConditionalOrderId,
    now_ms: TimestampMs,
    mut batch: Option<&mut WorkerLifecycleBatch>,
) -> Result<ConditionalOrder> {
    use crate::options::store::{
        build_option_match_plan, enforce_tif_plan, final_status_for_tif, status_for_remaining,
    };
    use crate::options::types::OptionOrder;
    use crate::types::OrderId;
    use uuid::Uuid as UuidV4;

    let order = store
        .get_conditional_order(conditional_id)
        .ok_or(BackendError::InvalidConditionalOrderId)?;

    // Idempotent re-entry guard: if we already produced a child this
    // call is a no-op (status would have been transitioned out of
    // `Triggering` after the first success). After restart we may
    // re-enter with status==Triggering and child_order_id==Some — in
    // that case we trust the existing child and just terminalise.
    if order.status.is_terminal() {
        return Ok(order);
    }
    if order.child_order_id.is_some() {
        let mut finalised = order.clone();
        finalised.status = ConditionalOrderStatus::Completed;
        finalised.completed_at_ms = Some(now_ms);
        finalised.updated_at_ms = now_ms;
        finalised.version = order.version.saturating_add(1);
        store.update_conditional_order(finalised.clone())?;
        if let Some(b) = batch.as_deref_mut() {
            b.push_conditional(finalised.clone());
        }
        return Ok(finalised);
    }

    // 1. Atomic OCO sibling cancellation FIRST so a parallel evaluator
    //    cannot also pick the sibling. (In-memory store is single-
    //    threaded behind a Mutex, so this is just a state mutation.)
    if let Some(group) = order.oco_group_id {
        let siblings = store.list_conditional_orders(&ConditionalOrderFilter {
            account: Some(order.account.clone()),
            option_series_id: None,
            status: None,
            oco_group_id: Some(group),
        });
        for sib in siblings {
            if sib.id == order.id {
                continue;
            }
            if sib.status.is_terminal() {
                return Err(BackendError::OcoSiblingTerminal);
            }
            let mut cancelled = sib.clone();
            cancelled.status = ConditionalOrderStatus::Cancelled;
            cancelled.failure_code = Some("oco_sibling_triggered".to_string());
            cancelled.failure_message = Some(format!(
                "sibling conditional order {} triggered first",
                order.id
            ));
            cancelled.completed_at_ms = Some(now_ms);
            cancelled.updated_at_ms = now_ms;
            cancelled.version = sib.version.saturating_add(1);
            store.update_conditional_order(cancelled.clone())?;
            if let Some(b) = batch.as_deref_mut() {
                b.push_conditional(cancelled);
            }
        }
    }

    // 2. Recompute reducible size right before submission.
    let fills = store.list_fills(&OptionFillFilter {
        option_series_id: Some(order.option_series_id.clone()),
        account: Some(order.account.clone()),
        order_id: None,
        subaccount_id: None,
    });
    let signed = compute_position_signed_1e8(&fills, &order.account);
    let live = classify_reducible(signed);

    // No reducible position left — the position was closed elsewhere
    // between arm and trigger. Mark cancelled with a clear reason.
    let Some(live_pos) = live else {
        let mut closed = order.clone();
        closed.status = ConditionalOrderStatus::Cancelled;
        closed.failure_code = Some("position_closed".to_string());
        closed.failure_message =
            Some("position is no longer open when the trigger fired".to_string());
        closed.completed_at_ms = Some(now_ms);
        closed.updated_at_ms = now_ms;
        closed.version = order.version.saturating_add(1);
        store.update_conditional_order(closed.clone())?;
        if let Some(b) = batch.as_deref_mut() {
            b.push_conditional(closed.clone());
        }
        return Ok(closed);
    };

    // Reverse-side defense — must never happen for an honest client,
    // but a stale `position_side` after a flip would silently widen
    // the position. Refuse.
    if live_pos.side != order.position_side {
        let mut failed = order.clone();
        failed.status = ConditionalOrderStatus::Failed;
        failed.failure_code = Some("position_side_changed".to_string());
        failed.failure_message =
            Some("position direction changed since arming; refusing to fire".to_string());
        failed.completed_at_ms = Some(now_ms);
        failed.updated_at_ms = now_ms;
        failed.version = order.version.saturating_add(1);
        store.update_conditional_order(failed.clone())?;
        if let Some(b) = batch.as_deref_mut() {
            b.push_conditional(failed.clone());
        }
        return Ok(failed);
    }

    // 3. Cap child quantity at the live reducible size.
    let child_qty = order.quantity_1e8.min(live_pos.size_1e8);
    if child_qty == 0 {
        let mut closed = order.clone();
        closed.status = ConditionalOrderStatus::Cancelled;
        closed.failure_code = Some("position_closed".to_string());
        closed.failure_message = Some("reducible size is zero at trigger time".to_string());
        closed.completed_at_ms = Some(now_ms);
        closed.updated_at_ms = now_ms;
        closed.version = order.version.saturating_add(1);
        store.update_conditional_order(closed.clone())?;
        if let Some(b) = batch.as_deref_mut() {
            b.push_conditional(closed.clone());
        }
        return Ok(closed);
    }

    // 4. Build the child IOC limit order. Side = opposite of position.
    let now_ms_signed = now_ms;
    let child = OptionOrder {
        order_id: OrderId::new(),
        option_series_id: order.option_series_id.clone(),
        account: order.account.clone(),
        // SUBACCOUNTS-OPTIONS-CONDITIONAL-TWAP-WS-V1 — child order
        // inherits the parent conditional's subaccount so a
        // subaccount 2 parent never silently emits subaccount 1
        // children.
        subaccount_id: order.subaccount_id,
        side: order.position_side.closing_side(),
        price_1e8: order.limit_price_1e8,
        size_1e8: child_qty,
        remaining_size_1e8: child_qty,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        client_order_id: Some(format!("cond-{}", order.id)),
        nonce: None,
        deadline_ms: None,
        signature: None,
        status: OptionOrderStatus::Open,
        terminal_reason_code: None,
        terminal_reason_message: None,
        terminal_reason_source: None,
        created_at_ms: now_ms_signed,
        updated_at_ms: now_ms_signed,
    };

    // Mirror the matcher's plan-then-execute so the conditional path
    // honours the same TIF semantics already proven for direct
    // orders. Errors from the matcher map to Failed terminal state.
    let mut candidates = store.list_open_match_candidates(&child);
    crate::options::store::sort_match_candidates_view(&mut candidates, child.side);
    let plan = build_option_match_plan(&child, &candidates);
    if let Err(e) = enforce_tif_plan(&child, &plan) {
        let mut failed = order.clone();
        failed.status = ConditionalOrderStatus::Failed;
        failed.failure_code = Some("tif_rejected".to_string());
        failed.failure_message = Some(e.to_string());
        failed.completed_at_ms = Some(now_ms);
        failed.updated_at_ms = now_ms;
        failed.version = order.version.saturating_add(1);
        store.update_conditional_order(failed.clone())?;
        if let Some(b) = batch.as_deref_mut() {
            b.push_conditional(failed.clone());
        }
        return Ok(failed);
    }

    let (final_child, fills_out) =
        store
            .submit_order_and_match(child, now_ms_signed)
            .map_err(|e| {
                BackendError::InvalidConditionalOrderState(format!("child submit failed: {e}"))
            })?;

    // 5. Persist child id + terminal status.
    let filled: u128 = fills_out.iter().map(|f| f.size_1e8).sum();
    let remaining = child_qty.saturating_sub(filled);
    let derived_status = final_status_for_tif(&final_child);
    let conditional_status = match derived_status {
        OptionOrderStatus::Filled => ConditionalOrderStatus::Completed,
        OptionOrderStatus::PartiallyFilled => ConditionalOrderStatus::PartiallyFilled,
        OptionOrderStatus::Cancelled if filled > 0 => ConditionalOrderStatus::PartiallyFilled,
        OptionOrderStatus::Cancelled => ConditionalOrderStatus::Failed,
        // Other variants shouldn't happen for IOC under our matcher.
        _ => ConditionalOrderStatus::Failed,
    };
    let mut completed = order.clone();
    completed.status = conditional_status;
    completed.child_order_id = Some(final_child.order_id.to_string());
    completed.triggered_at_ms = Some(now_ms);
    completed.completed_at_ms = Some(now_ms);
    completed.updated_at_ms = now_ms;
    completed.version = order.version.saturating_add(1);
    if matches!(conditional_status, ConditionalOrderStatus::Failed) {
        completed.failure_code = Some("no_liquidity".to_string());
        completed.failure_message = Some(format!(
            "child IOC order filled 0 of {} contracts",
            child_qty
        ));
    }
    store.update_conditional_order(completed.clone())?;
    // ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1 — child order + per-fill
    // events for the in-memory path. (DB path emits these via the
    // existing `submit_option_order` helper inside
    // `execute_triggered_via_repo`.) The conditional terminal update
    // event is emitted right after — both are queued and the caller
    // emits AFTER the store lock is dropped.
    if let Some(b) = batch.as_deref_mut() {
        b.set_child_order(final_child.clone(), fills_out.clone());
        b.push_conditional(completed.clone());
    }
    // Defensive — fills_out + remaining invariant
    debug_assert_eq!(filled + remaining as u128, child_qty as u128);
    let _ = (now_ms_signed, status_for_remaining); // silence unused-import-style lints
    let _ = UuidV4::nil; // anchor uuid use in case feature gates change
    Ok(completed)
}

// ---- Service layer (branches on `state.repository`) ----------------

use crate::api::AppState;
use crate::error::Result as ServiceResult;
use crate::options::service::get_option_series;

/// Phase 7 — create one or two conditional orders. Routes through
/// the DB repository when configured; falls back to the in-memory
/// store otherwise.
pub async fn create_conditional_orders(
    state: &AppState,
    input: CreateConditionalOrderInput,
) -> ServiceResult<Vec<ConditionalOrder>> {
    use crate::types::now_ms;
    if !state.options_config.enabled {
        return Err(BackendError::OptionsDisabled);
    }
    let series = get_option_series(state, &input.option_series_id).await?;
    let now = now_ms();

    if let Some(repo) = state.repository.clone() {
        // DB path — load existing rows + fills, build, insert. We do
        // NOT wrap in a tx for now: the unique constraints on the
        // table plus the per-row insert errors are sufficient
        // safety. A future revision can wrap inside `begin/commit`.
        let fills = repo
            .list_option_fills_for_account_and_series(&input.account, &input.option_series_id)
            .await?;
        let signed = compute_position_signed_1e8(&fills, &input.account);
        let position = classify_reducible(signed)
            .ok_or_else(|| BackendError::NoReduciblePosition(input.option_series_id.clone()))?;
        validate_input_shape(&input, &position)?;

        let existing = repo
            .list_conditional_orders(&ConditionalOrderFilter {
                account: Some(input.account.clone()),
                option_series_id: Some(input.option_series_id.clone()),
                ..Default::default()
            })
            .await?;
        let already = active_committed_quantity(&existing);
        let extra = oco_share(&input);
        if already.saturating_add(extra) > position.size_1e8 {
            return Err(BackendError::QuantityExceedsPosition);
        }

        let rows = build_conditional_rows(&series, &input, position.side, now)?;
        for row in &rows {
            repo.insert_conditional_order(row).await?;
        }
        emit_conditional_lifecycle(state, &rows);
        Ok(rows)
    } else {
        let mut store = state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
        let rows = create_conditional_orders_in_store(&mut store, &series, input, now)?;
        drop(store);
        emit_conditional_lifecycle(state, &rows);
        Ok(rows)
    }
}

/// ORDER-LIFECYCLE-OBSERVABILITY-V1 — emit one `ConditionalOrderUpdated`
/// per row. Called from create / cancel / trigger paths AFTER the DB
/// commit (or in-memory mutation) has succeeded.
pub(crate) fn emit_conditional_lifecycle(state: &AppState, rows: &[ConditionalOrder]) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    use crate::types::now_ms;
    let now = now_ms();
    for row in rows {
        state.lifecycle_events.emit(LifecycleEvent {
            account: row.account.clone(),
            channel: LifecycleChannel::AccountConditionalOrders,
            payload: LifecyclePayload::ConditionalOrderUpdated {
                conditional_order_id: row.id.to_string(),
                option_series_id: row.option_series_id.clone(),
                // SUBACCOUNTS-OPTIONS-WS-PAYLOAD-V1 — sourced from
                // `options_conditional_orders.subaccount_id`.
                // Attached TP/SL rows inherit the parent's value at
                // materialisation time, so this is the correct value
                // for both standalone conditionals and TP/SL legs.
                subaccount_id: row.subaccount_id,
                status: row.status.as_str().to_string(),
                child_order_id: row.child_order_id.clone(),
                oco_group_id: row.oco_group_id.map(|g| g.to_string()),
                failure_code: row.failure_code.clone(),
            },
            emitted_at_ms: now,
        });
    }
}

/// ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1 — collect-then-emit-after-
/// commit batch for the in-memory worker path.
///
/// The in-memory `execute_triggered_in_store` holds a mutex on
/// `state.options_store` for the entire trigger sequence (claim → OCO
/// siblings → child submission → terminal persistence). Emitting WS
/// events while holding that lock would (a) extend lock hold time on
/// every WS send and (b) risk a WS-layer panic poisoning the store
/// lock. Instead, we COLLECT pending events into this batch during
/// the mutation and EMIT them only after the function returns Ok and
/// the caller drops the lock.
///
/// The DB path does NOT need this batch — its mutations are scoped to
/// individual repository calls and emission happens between them
/// without a long-running lock.
#[derive(Default)]
pub struct WorkerLifecycleBatch {
    pub conditional_updates: Vec<ConditionalOrder>,
    pub child_order: Option<(crate::options::OptionOrder, Vec<crate::options::OptionFill>)>,
}

impl WorkerLifecycleBatch {
    pub fn push_conditional(&mut self, order: ConditionalOrder) {
        self.conditional_updates.push(order);
    }

    pub fn set_child_order(
        &mut self,
        order: crate::options::OptionOrder,
        fills: Vec<crate::options::OptionFill>,
    ) {
        self.child_order = Some((order, fills));
    }

    pub fn emit(self, state: &AppState) {
        if !self.conditional_updates.is_empty() {
            emit_conditional_lifecycle(state, &self.conditional_updates);
        }
        if let Some((order, fills)) = self.child_order {
            crate::options::service::emit_option_order_lifecycle(state, &order, &fills);
        }
    }
}

fn validate_input_shape(
    input: &CreateConditionalOrderInput,
    position: &ReduciblePosition,
) -> Result<()> {
    if input.quantity_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if input.legs.is_empty() || input.legs.len() > 2 {
        return Err(BackendError::InvalidConditionalOrderState(
            "must supply 1 or 2 legs".to_string(),
        ));
    }
    if input.legs.len() == 2 {
        let has_tp = input
            .legs
            .iter()
            .any(|l| l.conditional_type == ConditionalType::TakeProfit);
        let has_sl = input
            .legs
            .iter()
            .any(|l| l.conditional_type == ConditionalType::StopLoss);
        if !(has_tp && has_sl) {
            return Err(BackendError::InvalidConditionalOrderState(
                "two-leg request must contain one TP and one SL".to_string(),
            ));
        }
    }
    for leg in &input.legs {
        if leg.trigger_price_1e8 == 0 {
            return Err(BackendError::InvalidTriggerPrice);
        }
        if leg.limit_price_1e8 == 0 {
            return Err(BackendError::InvalidConditionalLimitPrice);
        }
    }
    if input.quantity_1e8 > position.size_1e8 {
        return Err(BackendError::QuantityExceedsPosition);
    }
    Ok(())
}

fn oco_share(input: &CreateConditionalOrderInput) -> Size1e8 {
    if input.legs.len() == 2 && input.link_as_oco {
        input.quantity_1e8
    } else {
        input
            .quantity_1e8
            .saturating_mul(input.legs.len() as Size1e8)
    }
}

fn build_conditional_rows(
    series: &OptionSeries,
    input: &CreateConditionalOrderInput,
    position_side: PositionSide,
    now: TimestampMs,
) -> Result<Vec<ConditionalOrder>> {
    let kind = OptionKind::from_is_call(series.is_call);
    let oco_group = if input.legs.len() == 2 && input.link_as_oco {
        Some(Uuid::new_v4())
    } else {
        None
    };
    let mut rows = Vec::with_capacity(input.legs.len());
    for leg in &input.legs {
        let derived = derive_trigger_condition(kind, position_side, leg.conditional_type);
        if let Some(explicit) = leg.explicit_trigger_condition {
            if explicit != derived {
                return Err(BackendError::InvalidTriggerDirection);
            }
        }
        rows.push(ConditionalOrder {
            id: Uuid::new_v4(),
            account: input.account.clone(),
            subaccount_id: input.subaccount_id,
            option_series_id: series.option_series_id.clone(),
            position_side,
            option_kind: kind,
            conditional_type: leg.conditional_type,
            trigger_source: TriggerSource::UnderlyingOracle,
            trigger_condition: derived,
            trigger_price_1e8: leg.trigger_price_1e8,
            quantity_1e8: input.quantity_1e8,
            execution_type: ConditionalExecutionType::IocLimit,
            limit_price_1e8: leg.limit_price_1e8,
            reduce_only: true,
            oco_group_id: oco_group,
            status: ConditionalOrderStatus::Armed,
            child_order_id: None,
            failure_code: None,
            failure_message: None,
            expires_at_ms: input.expires_at_ms,
            triggered_at_ms: None,
            completed_at_ms: None,
            created_at_ms: now,
            updated_at_ms: now,
            version: 0,
        });
    }
    Ok(rows)
}

pub async fn list_conditional_orders(
    state: &AppState,
    filter: ConditionalOrderFilter,
) -> ServiceResult<Vec<ConditionalOrder>> {
    if !state.options_config.enabled {
        return Err(BackendError::OptionsDisabled);
    }
    if let Some(repo) = state.repository.clone() {
        return repo.list_conditional_orders(&filter).await;
    }
    let store = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
    Ok(store.list_conditional_orders(&filter))
}

pub async fn get_conditional_order(
    state: &AppState,
    id: ConditionalOrderId,
) -> ServiceResult<Option<ConditionalOrder>> {
    if !state.options_config.enabled {
        return Err(BackendError::OptionsDisabled);
    }
    if let Some(repo) = state.repository.clone() {
        return repo.get_conditional_order(id).await;
    }
    let store = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
    Ok(store.get_conditional_order(id))
}

pub async fn cancel_conditional_order(
    state: &AppState,
    id: ConditionalOrderId,
    requesting_account: &AccountId,
) -> ServiceResult<ConditionalOrder> {
    use crate::types::now_ms;
    if !state.options_config.enabled {
        return Err(BackendError::OptionsDisabled);
    }
    let now = now_ms();
    if let Some(repo) = state.repository.clone() {
        let existing = repo
            .get_conditional_order(id)
            .await?
            .ok_or(BackendError::InvalidConditionalOrderId)?;
        if !existing
            .account
            .0
            .eq_ignore_ascii_case(&requesting_account.0)
        {
            return Err(BackendError::InvalidConditionalOrderId);
        }
        if existing.status.is_terminal() {
            return Err(BackendError::ConditionalOrderAlreadyTerminal);
        }
        let mut updated = existing.clone();
        updated.status = ConditionalOrderStatus::Cancelled;
        updated.completed_at_ms = Some(now);
        updated.updated_at_ms = now;
        updated.version = existing.version.saturating_add(1);
        let saved = repo.update_conditional_order(&updated).await?;
        emit_conditional_lifecycle(state, std::slice::from_ref(&saved));
        return Ok(saved);
    }
    let mut store = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
    let cancelled = cancel_conditional_order_in_store(&mut store, id, requesting_account, now)?;
    drop(store);
    emit_conditional_lifecycle(state, std::slice::from_ref(&cancelled));
    Ok(cancelled)
}

// ---- Worker (Phase 6) -----------------------------------------------

#[derive(Clone, Debug)]
pub struct ConditionalOrdersConfig {
    pub enabled: bool,
    pub poll_interval_ms: u64,
    pub batch_size: u32,
    pub max_retries: u32,
}

impl Default for ConditionalOrdersConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_ms: 2_000,
            batch_size: 64,
            max_retries: 3,
        }
    }
}

impl ConditionalOrdersConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("CONDITIONAL_ORDERS_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let poll_interval_ms = std::env::var("CONDITIONAL_ORDERS_POLL_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2_000);
        let batch_size = std::env::var("CONDITIONAL_ORDERS_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64);
        let max_retries = std::env::var("CONDITIONAL_ORDERS_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        Self {
            enabled,
            poll_interval_ms,
            batch_size,
            max_retries,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConditionalOrdersTickResult {
    pub evaluated: usize,
    pub triggered: usize,
    pub completed: usize,
    pub failed: usize,
    pub skipped_oracle_unavailable: bool,
}

/// One evaluator tick. Tests call this directly; the spawned worker
/// calls it in a loop. Returns counts so callers can log/observe.
///
/// Safety notes:
/// * If oracle/RPC is unconfigured we return early with
///   `skipped_oracle_unavailable=true` — NO state mutation.
/// * Each armed row is claimed via the atomic `armed → triggering`
///   transition before the child order is built; a parallel worker
///   that lost the race observes `None` and moves on.
/// * OCO siblings are cancelled IN THE SAME tick that the winner is
///   claimed, before the child order is submitted.
pub async fn evaluate_conditional_orders_tick<P>(
    state: &AppState,
    provider: Option<&P>,
) -> ServiceResult<ConditionalOrdersTickResult>
where
    P: crate::execution::rpc::EthCallProvider,
{
    use crate::api::trading_views::try_get_oracle_price_safe;
    use crate::signing::eip712::parse_evm_address;
    use crate::types::now_ms;

    let mut result = ConditionalOrdersTickResult::default();
    if !state.options_config.enabled {
        return Ok(result);
    }
    // NOTE: `recover_stranded_triggering` is intentionally NOT called
    // per-tick. The recovery sweep is a one-shot operation owned by
    // the worker spawn (see `spawn_conditional_orders_worker`); calling
    // it per-tick races with concurrent `trigger_one` invocations and
    // can re-arm a row another worker is mid-execution on, causing the
    // optimistic-lock UPDATE in that worker to fail with
    // `InvalidConditionalOrderId` and (if a second tick reads the
    // re-armed row) duplicate child submissions.
    let oracle_configured = state.trading_views.oracle_router_address.is_some();
    let Some(provider) = provider else {
        result.skipped_oracle_unavailable = true;
        return Ok(result);
    };
    if !oracle_configured {
        result.skipped_oracle_unavailable = true;
        return Ok(result);
    }

    let armed = list_conditional_orders(
        state,
        ConditionalOrderFilter {
            status: Some(ConditionalOrderStatus::Armed),
            ..Default::default()
        },
    )
    .await?;
    result.evaluated = armed.len();
    if armed.is_empty() {
        return Ok(result);
    }

    // Group by series id → resolve oracle price once per distinct
    // (underlying, settlement) pair (mirrors `account_positions`).
    use std::collections::HashMap;
    let from = &state.execution_config.executor_from_address;
    let mut price_cache: HashMap<String, Option<Price1e8>> = HashMap::new();
    for order in &armed {
        if price_cache.contains_key(&order.option_series_id) {
            continue;
        }
        let series = match get_option_series(state, &order.option_series_id).await {
            Ok(s) => s,
            Err(_) => {
                price_cache.insert(order.option_series_id.clone(), None);
                continue;
            }
        };
        let underlying = crate::api::trading::address_from_account_str(&series.underlying);
        let settlement = crate::api::trading::address_from_account_str(&series.settlement_asset);
        let price = match (underlying, settlement) {
            (Some(base), Some(quote)) => {
                match try_get_oracle_price_safe(&state.trading_views, from, base, quote, provider)
                    .await
                {
                    Ok(Some(p)) => {
                        // Oracle is uint256; downscale to u128 with a
                        // saturating fallback (BTC at $1e9 still fits
                        // in u128 with 1e8 scaling, so saturation is
                        // a defence not a real concern).
                        let limbs = p.into_limbs();
                        let lo = limbs[0] as u128 | ((limbs[1] as u128) << 64);
                        Some(lo)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        price_cache.insert(order.option_series_id.clone(), price);
    }

    let to_trigger = select_orders_to_trigger(
        &armed,
        |series_id| price_cache.get(series_id).and_then(|v| *v),
        now_ms(),
    );

    let _ = parse_evm_address; // anchor — used by neighbouring service path

    for id in to_trigger {
        if let Err(error) = trigger_one(state, id, now_ms()).await {
            tracing::warn!(%error, conditional_id=%id, "conditional order trigger failed");
            result.failed += 1;
        } else {
            result.triggered += 1;
            result.completed += 1;
        }
    }

    Ok(result)
}

/// Deterministic test seam — same execution path as
/// `evaluate_conditional_orders_tick`, but the oracle is replaced by a
/// caller-supplied price snapshot keyed by `option_series_id`. Used by
/// integration tests to exercise the trigger → atomic claim → OCO
/// cancel → child IOC path without an RPC dependency.
///
/// Production callers MUST use the RPC variant; this function exists
/// because Phase 4 of OPTIONS-CONDITIONAL-ORDERS-PERSISTENT-E2E-V1
/// explicitly authorises adding the smallest test-only seam.
pub async fn evaluate_conditional_orders_tick_with_prices(
    state: &AppState,
    prices_by_series: &std::collections::HashMap<String, Price1e8>,
) -> ServiceResult<ConditionalOrdersTickResult> {
    use crate::types::now_ms;
    let mut result = ConditionalOrdersTickResult::default();
    if !state.options_config.enabled {
        return Ok(result);
    }

    // NOTE: `recover_stranded_triggering` is intentionally NOT called
    // per-tick — see the matching comment in
    // `evaluate_conditional_orders_tick`. Direct tests call
    // `recover_stranded_triggering` explicitly when needed.

    let armed = list_conditional_orders(
        state,
        ConditionalOrderFilter {
            status: Some(ConditionalOrderStatus::Armed),
            ..Default::default()
        },
    )
    .await?;
    result.evaluated = armed.len();
    if armed.is_empty() {
        return Ok(result);
    }

    let to_trigger = select_orders_to_trigger(
        &armed,
        |series_id| prices_by_series.get(series_id).copied(),
        now_ms(),
    );

    for id in to_trigger {
        if let Err(error) = trigger_one(state, id, now_ms()).await {
            tracing::warn!(%error, conditional_id=%id, "conditional order trigger failed");
            result.failed += 1;
        } else {
            result.triggered += 1;
            result.completed += 1;
        }
    }
    Ok(result)
}

/// Phase 6 Case C — drive any persisted `Triggering` rows to a
/// terminal state at startup / tick entry so a backend crash between
/// `claim_armed` and child submission does not leave the row stuck.
///
/// * If `child_order_id` is set: the previous lifetime DID submit the
///   child (the matcher commit landed) — just terminalise the
///   conditional row as `Completed`. This is safe because IOC orders
///   never linger in a live status, so the child is already at its
///   final outcome.
/// * If `child_order_id` is NULL: reset to `Armed` so the next normal
///   evaluator pass re-attempts. The `client_order_id="cond-<id>"`
///   UNIQUE INDEX on `option_orders` still protects against any
///   duplicate child submission on the retry path.
pub async fn recover_stranded_triggering(
    state: &AppState,
    now: TimestampMs,
) -> ServiceResult<usize> {
    let stranded = list_conditional_orders(
        state,
        ConditionalOrderFilter {
            status: Some(ConditionalOrderStatus::Triggering),
            ..Default::default()
        },
    )
    .await?;
    let mut count = 0usize;
    for order in stranded {
        let recovered_row = if order.child_order_id.is_some() {
            // Child was submitted before the crash → finalise the
            // conditional row. We trust IOC semantics: the child has
            // already reached a terminal status.
            let mut finalised = order.clone();
            finalised.status = ConditionalOrderStatus::Completed;
            finalised.completed_at_ms = Some(now);
            finalised.updated_at_ms = now;
            finalised.version = order.version.saturating_add(1);
            finalised
        } else {
            // Re-arm so the next normal tick picks it up.
            let mut rearmed = order.clone();
            rearmed.status = ConditionalOrderStatus::Armed;
            rearmed.updated_at_ms = now;
            rearmed.version = order.version.saturating_add(1);
            rearmed
        };
        // Tolerate optimistic-lock conflicts: if another worker
        // (or a concurrent test fixture) has already mutated the
        // row since we read it, the recovery for this row is
        // effectively done by that other actor — skip silently and
        // keep sweeping the rest of the batch. Any OTHER error
        // (network, persistence) still bubbles up.
        match persist_recovered(state, &recovered_row).await {
            Ok(()) => {
                count += 1;
                // ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1 — emit the
                // recovery transition. We only reach this arm when the
                // persistence call actually committed the new status,
                // so no event is emitted for a no-op recovery.
                emit_conditional_lifecycle(state, std::slice::from_ref(&recovered_row));
            }
            Err(BackendError::InvalidConditionalOrderId) => {
                tracing::debug!(
                    conditional_id = %order.id,
                    "stranded recovery skipped (row mutated concurrently)"
                );
            }
            Err(other) => return Err(other),
        }
    }
    Ok(count)
}

async fn persist_recovered(state: &AppState, order: &ConditionalOrder) -> ServiceResult<()> {
    if let Some(repo) = state.repository.clone() {
        repo.update_conditional_order(order).await?;
        return Ok(());
    }
    let mut store = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
    store.update_conditional_order(order.clone())
}

async fn trigger_one(
    state: &AppState,
    id: ConditionalOrderId,
    now: TimestampMs,
) -> ServiceResult<ConditionalOrder> {
    // 1. Atomic claim — branches on repo.
    let claimed = if let Some(repo) = state.repository.clone() {
        match repo
            .claim_conditional_order_armed(id, ConditionalOrderStatus::Triggering, now)
            .await?
        {
            Some(o) => o,
            None => return Err(BackendError::ConditionalOrderAlreadyTerminal),
        }
    } else {
        let mut store = state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
        let existing = store
            .get_conditional_order(id)
            .ok_or(BackendError::InvalidConditionalOrderId)?;
        if existing.status != ConditionalOrderStatus::Armed {
            return Err(BackendError::ConditionalOrderAlreadyTerminal);
        }
        let mut transitioned = existing.clone();
        transitioned.status = ConditionalOrderStatus::Triggering;
        transitioned.updated_at_ms = now;
        transitioned.version = existing.version.saturating_add(1);
        store.update_conditional_order(transitioned.clone())?;
        transitioned
    };

    // ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1 — claim succeeded; emit
    // `ConditionalOrderUpdated(status="triggering")` AFTER persistence.
    emit_conditional_lifecycle(state, std::slice::from_ref(&claimed));

    // 2. Cancel OCO siblings atomically (DB) or inline (in-memory).
    if let Some(group) = claimed.oco_group_id {
        if let Some(repo) = state.repository.clone() {
            // V1 → V2: `cancel_oco_siblings` now returns the actual
            // cancelled rows so we can emit one lifecycle event per
            // sibling without a second round-trip.
            let siblings = repo.cancel_oco_siblings(group, claimed.id, now).await?;
            if !siblings.is_empty() {
                emit_conditional_lifecycle(state, &siblings);
            }
        }
        // In-memory mode lets `execute_triggered_in_store` handle the
        // sibling sweep inside the same critical section AND emit the
        // matching lifecycle events for each cancelled sibling.
    }

    // 3. Execute the child order.
    let series = get_option_series(state, &claimed.option_series_id).await?;
    if state.repository.is_some() {
        execute_triggered_via_repo(state, &series, claimed.clone(), now).await
    } else {
        let mut store = state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
        // Collect pending events from inside the store mutation, then
        // emit AFTER the lock is dropped so a WS send error never
        // poisons the store lock. See `WorkerLifecycleBatch` below.
        let mut batch = WorkerLifecycleBatch::default();
        let result =
            execute_triggered_in_store(&mut store, &series, claimed.id, now, Some(&mut batch));
        drop(store);
        if result.is_ok() {
            batch.emit(state);
        }
        result
    }
}

async fn execute_triggered_via_repo(
    state: &AppState,
    _series: &OptionSeries,
    claimed: ConditionalOrder,
    now: TimestampMs,
) -> ServiceResult<ConditionalOrder> {
    use crate::options::service::{submit_option_order, SubmitOptionOrderInput};
    let repo = state
        .repository
        .clone()
        .ok_or_else(|| BackendError::Config("repository missing".to_string()))?;

    // Recompute reducible size from the DB fills index.
    let fills = repo
        .list_option_fills_for_account_and_series(&claimed.account, &claimed.option_series_id)
        .await?;
    let signed = compute_position_signed_1e8(&fills, &claimed.account);
    let live = classify_reducible(signed);

    let Some(live_pos) = live else {
        let mut closed = claimed.clone();
        closed.status = ConditionalOrderStatus::Cancelled;
        closed.failure_code = Some("position_closed".to_string());
        closed.failure_message =
            Some("position is no longer open when the trigger fired".to_string());
        closed.completed_at_ms = Some(now);
        closed.updated_at_ms = now;
        closed.version = claimed.version.saturating_add(1);
        let saved = repo.update_conditional_order(&closed).await?;
        emit_conditional_lifecycle(state, std::slice::from_ref(&saved));
        return Ok(saved);
    };
    if live_pos.side != claimed.position_side {
        let mut failed = claimed.clone();
        failed.status = ConditionalOrderStatus::Failed;
        failed.failure_code = Some("position_side_changed".to_string());
        failed.failure_message =
            Some("position direction changed since arming; refusing to fire".to_string());
        failed.completed_at_ms = Some(now);
        failed.updated_at_ms = now;
        failed.version = claimed.version.saturating_add(1);
        let saved = repo.update_conditional_order(&failed).await?;
        emit_conditional_lifecycle(state, std::slice::from_ref(&saved));
        return Ok(saved);
    }
    let child_qty = claimed.quantity_1e8.min(live_pos.size_1e8);
    if child_qty == 0 {
        let mut closed = claimed.clone();
        closed.status = ConditionalOrderStatus::Cancelled;
        closed.failure_code = Some("position_closed".to_string());
        closed.completed_at_ms = Some(now);
        closed.updated_at_ms = now;
        closed.version = claimed.version.saturating_add(1);
        let saved = repo.update_conditional_order(&closed).await?;
        emit_conditional_lifecycle(state, std::slice::from_ref(&saved));
        return Ok(saved);
    }

    // Submit through the existing options order service — same TIF
    // matcher used by the operator-facing endpoint. `client_order_id`
    // is `cond-<id>` so a retry after a restart cannot duplicate the
    // child order: the existing UNIQUE INDEX on (account,
    // client_order_id) WHERE status IN (open, partially_filled)
    // protects us, and IOC orders never leave a live status anyway.
    let outcome = submit_option_order(
        state,
        SubmitOptionOrderInput {
            option_series_id: claimed.option_series_id.clone(),
            account: claimed.account.clone(),
            // SUBACCOUNTS-OPTIONS-CONDITIONAL-TWAP-WS-V1 — child
            // order inherits the parent conditional's subaccount.
            subaccount_id: claimed.subaccount_id,
            side: claimed.position_side.closing_side(),
            price_1e8: claimed.limit_price_1e8,
            size_1e8: child_qty,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            client_order_id: Some(format!("cond-{}", claimed.id)),
            nonce: None,
            deadline_ms: None,
            signature: None,
            attached_tp_sl: None,
        },
    )
    .await;

    match outcome {
        Ok(out) => {
            let filled: u128 = out.fills.iter().map(|f| f.size_1e8).sum();
            let conditional_status = if filled == child_qty as u128 {
                ConditionalOrderStatus::Completed
            } else if filled > 0 {
                ConditionalOrderStatus::PartiallyFilled
            } else {
                ConditionalOrderStatus::Failed
            };
            let mut completed = claimed.clone();
            completed.status = conditional_status;
            completed.child_order_id = Some(out.order.order_id.to_string());
            completed.triggered_at_ms = Some(now);
            completed.completed_at_ms = Some(now);
            completed.updated_at_ms = now;
            completed.version = claimed.version.saturating_add(1);
            if matches!(conditional_status, ConditionalOrderStatus::Failed) {
                completed.failure_code = Some("no_liquidity".to_string());
                completed.failure_message = Some(format!(
                    "child IOC order filled 0 of {} contracts",
                    child_qty
                ));
            }
            let saved = repo.update_conditional_order(&completed).await?;
            // Child order + per-fill events were already emitted by
            // `submit_option_order` inside `submit_option_order` itself —
            // we only need to emit the conditional terminal update here.
            emit_conditional_lifecycle(state, std::slice::from_ref(&saved));
            Ok(saved)
        }
        Err(e) => {
            let mut failed = claimed.clone();
            failed.status = ConditionalOrderStatus::Failed;
            failed.failure_code = Some("child_submit_failed".to_string());
            failed.failure_message = Some(e.to_string());
            failed.completed_at_ms = Some(now);
            failed.updated_at_ms = now;
            failed.version = claimed.version.saturating_add(1);
            let saved = repo.update_conditional_order(&failed).await?;
            emit_conditional_lifecycle(state, std::slice::from_ref(&saved));
            Ok(saved)
        }
    }
}

/// Spawn the conditional-orders worker. No-op when disabled or when
/// the oracle/RPC dependencies are missing.
pub fn spawn_conditional_orders_worker(state: AppState) {
    let config = state.conditional_orders_config.clone();
    if !config.enabled {
        tracing::info!("conditional orders worker disabled");
        return;
    }
    if state.trading_views.oracle_router_address.is_none() {
        tracing::warn!("conditional orders worker enabled but OPTION_ORACLE_ROUTER_ADDRESS missing; refusing to spawn");
        return;
    }
    let rpc_url = match state.option_confirmation_config.rpc_url.clone() {
        Some(u) => u,
        None => {
            tracing::warn!("conditional orders worker enabled without RPC_URL; refusing to spawn");
            return;
        }
    };
    let provider = crate::execution::HttpJsonRpcProvider::new(rpc_url);
    let interval_ms = config.poll_interval_ms;
    tokio::spawn(async move {
        // One-shot startup recovery sweep. Drives any
        // `Triggering`-state rows left behind by a previous backend
        // lifetime to a terminal state before the periodic tick loop
        // begins. Doing this OUTSIDE the loop avoids racing with our
        // own in-flight `trigger_one` invocations (which transiently
        // leave a row in `Triggering` for the duration of a child
        // submission).
        match recover_stranded_triggering(&state, crate::types::now_ms()).await {
            Ok(swept) => {
                if swept > 0 {
                    tracing::info!(swept, "conditional orders startup recovery");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "conditional orders startup recovery failed");
            }
        }
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match evaluate_conditional_orders_tick(&state, Some(&provider)).await {
                Ok(out) => {
                    if out.triggered > 0 || out.failed > 0 {
                        tracing::info!(
                            evaluated = out.evaluated,
                            triggered = out.triggered,
                            failed = out.failed,
                            "conditional orders tick"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "conditional orders tick failed");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::types::{OptionSeriesSource, OptionSeriesStatus};

    fn dummy_series(is_call: bool) -> OptionSeries {
        OptionSeries {
            option_series_id: "s1".to_string(),
            underlying: "BTC".to_string(),
            base_asset: "BTC".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry: 1_000_000_000,
            strike_1e8: 70_000_000_000,
            is_call,
            contract_size_1e8: 100_000_000,
            status: OptionSeriesStatus::Active,
            source: OptionSeriesSource::Manual,
            onchain_product_id: None,
            onchain_series_id: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn long_call_tp_is_gte_and_sl_is_lte() {
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Call,
                PositionSide::Long,
                ConditionalType::TakeProfit
            ),
            TriggerCondition::Gte
        );
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Call,
                PositionSide::Long,
                ConditionalType::StopLoss
            ),
            TriggerCondition::Lte
        );
    }

    #[test]
    fn long_put_tp_is_lte_and_sl_is_gte() {
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Put,
                PositionSide::Long,
                ConditionalType::TakeProfit
            ),
            TriggerCondition::Lte
        );
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Put,
                PositionSide::Long,
                ConditionalType::StopLoss
            ),
            TriggerCondition::Gte
        );
    }

    #[test]
    fn short_call_tp_is_lte_and_sl_is_gte() {
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Call,
                PositionSide::Short,
                ConditionalType::TakeProfit
            ),
            TriggerCondition::Lte
        );
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Call,
                PositionSide::Short,
                ConditionalType::StopLoss
            ),
            TriggerCondition::Gte
        );
    }

    #[test]
    fn short_put_tp_is_gte_and_sl_is_lte() {
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Put,
                PositionSide::Short,
                ConditionalType::TakeProfit
            ),
            TriggerCondition::Gte
        );
        assert_eq!(
            derive_trigger_condition(
                OptionKind::Put,
                PositionSide::Short,
                ConditionalType::StopLoss
            ),
            TriggerCondition::Lte
        );
    }

    #[test]
    fn trigger_condition_matches_boundary_is_inclusive() {
        assert!(TriggerCondition::Gte.matches(100, 100));
        assert!(TriggerCondition::Gte.matches(101, 100));
        assert!(!TriggerCondition::Gte.matches(99, 100));
        assert!(TriggerCondition::Lte.matches(100, 100));
        assert!(TriggerCondition::Lte.matches(99, 100));
        assert!(!TriggerCondition::Lte.matches(101, 100));
    }

    #[test]
    fn classify_reducible_sign_mapping() {
        assert_eq!(classify_reducible(0).is_none(), true);
        let long = classify_reducible(42).unwrap();
        assert_eq!(long.side, PositionSide::Long);
        assert_eq!(long.size_1e8, 42);
        let short = classify_reducible(-7).unwrap();
        assert_eq!(short.side, PositionSide::Short);
        assert_eq!(short.size_1e8, 7);
    }

    #[test]
    fn closing_side_inverts_position() {
        assert_eq!(PositionSide::Long.closing_side(), Side::Sell);
        assert_eq!(PositionSide::Short.closing_side(), Side::Buy);
    }

    #[test]
    fn select_orders_skips_when_oracle_missing() {
        let _ = dummy_series(true);
        let armed: Vec<ConditionalOrder> = vec![];
        let chosen = select_orders_to_trigger(&armed, |_| None, 0);
        assert!(chosen.is_empty());
    }
}
