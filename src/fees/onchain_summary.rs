//! V2D-E: shared on-chain fee summarization across the V1 and V2 fee event
//! models.
//!
//! The backend now indexes two fee event families:
//!
//! - **V1** — `TradingFeeCharged` (positive fee only, emitted by
//!   `MarginEngine` for backward compatibility).
//! - **V2** — `FeeChargedV2` (positive fee) and `FeeRebatedV2`
//!   (signed-ppm rebate credited to the maker), emitted by `FeesManagerV2`.
//!
//! Both lifecycle views and the `/admin/fees/onchain` endpoint summarize
//! the same set of events through the helpers in this module, so the
//! double-counting policy is enforced in exactly one place.
//!
//! ## Double-counting policy
//!
//! When a single transaction emits both a V1 `TradingFeeCharged` log and a
//! V2 `FeeChargedV2` log for the same fee flow, the V1 log is a
//! compatibility breadcrumb (the V1 event is still emitted by the engine
//! to keep historical indexers working). The V2 log is the source of
//! truth for the V2 fee path.
//!
//! Therefore:
//!
//! - `event_model = "v1"`  → V1 events drive the totals.
//! - `event_model = "v2"`  → V2 charged/rebated events drive the totals.
//! - `event_model = "mixed"` → V2 takes precedence (`source_priority = "v2"`);
//!   V1 events remain visible in the event list but contribute zero to
//!   the totals to avoid double-counting.
//!
//! No transactions are submitted, no broadcast is invoked, no rows are
//! mutated by this module — it is pure read-only aggregation.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::options::lifecycle::LifecycleFees;
use crate::options::OptionExecutionEvent;

/// Snapshot of a single fee event after decoding into a uniform shape.
#[derive(Clone, Debug)]
pub struct NormalizedFeeEvent {
    pub model: FeeEventModel,
    pub event_id: uuid::Uuid,
    pub tx_hash: String,
    pub log_index: u64,
    pub block_number: u64,
    pub contract_address: String,
    pub trader: String,
    pub recipient: String,
    pub side: SideKind,
    pub option_id: Option<String>,
    pub settlement_asset: Option<String>,
    /// Positive when the trader paid the protocol (V1 fee or V2 charged).
    pub charged_amount: u128,
    /// Positive when the trader received a rebate (V2 rebated). Always
    /// zero for V1.
    pub rebate_amount: u128,
    /// Raw `appliedFee` string (V1) or `feeAmount`/`rebateAmount` (V2).
    pub amount_str: Option<String>,
    /// For V2 events: signed ppm rate (positive for charge, negative for
    /// rebate). `None` for V1 events.
    pub ppm: Option<i64>,
    /// For V2 events: V2 product (`"option"` / `"perp"`). `None` for V1.
    pub product_kind: Option<String>,
    /// For V2 events: V2 flow (`"orderbook"` / `"rfq"`). `None` for V1.
    pub flow_kind: Option<String>,
    /// For V2 events: the basis amount (premium for OPTION, notional for
    /// PERP) the ppm rate was applied to. Kept as the decoded uint string
    /// so callers can choose how to render it. `None` for V1 events.
    pub basis_amount: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeeEventModel {
    V1Charged,
    V2Charged,
    V2Rebated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SideKind {
    Maker,
    Taker,
    Unknown,
}

impl SideKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Maker => "maker",
            Self::Taker => "taker",
            Self::Unknown => "unknown",
        }
    }
}

/// Decoded snapshot grouped per fee event family.
#[derive(Clone, Debug, Default)]
pub struct NormalizedFees {
    pub v1_charged: Vec<NormalizedFeeEvent>,
    pub v2_charged: Vec<NormalizedFeeEvent>,
    pub v2_rebated: Vec<NormalizedFeeEvent>,
}

impl NormalizedFees {
    fn total_count(&self) -> usize {
        self.v1_charged.len() + self.v2_charged.len() + self.v2_rebated.len()
    }
}

/// Decoded totals and per-axis breakdowns for a selected event model.
///
/// `by_trader`, `by_recipient`, and `by_side` only accumulate **positive
/// fees**. Rebates are tracked separately on `rebated_total` and
/// `rebated_by_trader` so the aggregate maps stay `u128` (no signed
/// arithmetic), and the lifecycle/admin views can compute net fees by
/// subtraction.
#[derive(Clone, Debug, Default)]
pub struct AggregatedFees {
    pub event_model: &'static str,
    pub source_priority: &'static str,
    pub trading_fee_event_count: u64,
    pub fee_charged_v2_count: u64,
    pub fee_rebated_v2_count: u64,
    pub charged_total: u128,
    pub rebated_total: u128,
    pub by_trader: BTreeMap<String, u128>,
    pub by_recipient: BTreeMap<String, u128>,
    pub by_side: BTreeMap<String, u128>,
    pub rebated_by_trader: BTreeMap<String, u128>,
}

impl AggregatedFees {
    pub fn net_protocol_fee(&self) -> u128 {
        self.charged_total.saturating_sub(self.rebated_total)
    }
}

/// Walk a slice of indexed events and bucket them by fee model.
pub fn normalize_fee_events(events: &[OptionExecutionEvent]) -> NormalizedFees {
    let mut out = NormalizedFees::default();
    for event in events {
        match event.event_name.as_str() {
            "TradingFeeCharged" => {
                if let Some(normalized) = normalize_v1_charged(event) {
                    out.v1_charged.push(normalized);
                }
            }
            "FeeChargedV2" => {
                if let Some(normalized) = normalize_v2_charged(event) {
                    out.v2_charged.push(normalized);
                }
            }
            "FeeRebatedV2" => {
                if let Some(normalized) = normalize_v2_rebated(event) {
                    out.v2_rebated.push(normalized);
                }
            }
            _ => {}
        }
    }
    out
}

fn normalize_v1_charged(event: &OptionExecutionEvent) -> Option<NormalizedFeeEvent> {
    let decoded = event.decoded.clone().unwrap_or(Value::Null);
    let applied_fee_str = decoded
        .get("appliedFee")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let charged = applied_fee_str
        .as_deref()
        .and_then(|value| value.parse::<u128>().ok())
        .unwrap_or(0);
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
    let side = match decoded.get("isMaker").and_then(Value::as_bool) {
        Some(true) => SideKind::Maker,
        Some(false) => SideKind::Taker,
        None => SideKind::Unknown,
    };
    let settlement_asset = decoded
        .get("settlementAsset")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some(NormalizedFeeEvent {
        model: FeeEventModel::V1Charged,
        event_id: event.id,
        tx_hash: event.tx_hash.to_ascii_lowercase(),
        log_index: event.log_index,
        block_number: event.block_number,
        contract_address: event.contract_address.to_ascii_lowercase(),
        trader,
        recipient,
        side,
        option_id: event.option_id.clone(),
        settlement_asset,
        charged_amount: charged,
        rebate_amount: 0,
        amount_str: applied_fee_str,
        ppm: None,
        product_kind: None,
        flow_kind: None,
        basis_amount: None,
    })
}

fn normalize_v2_charged(event: &OptionExecutionEvent) -> Option<NormalizedFeeEvent> {
    let decoded = event.decoded.clone().unwrap_or(Value::Null);
    let amount_str = decoded
        .get("feeAmount")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let charged = amount_str
        .as_deref()
        .and_then(|value| value.parse::<u128>().ok())
        .unwrap_or(0);
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
    let recipient = decoded
        .get("recipient")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    let side = match decoded.get("isMaker").and_then(Value::as_bool) {
        Some(true) => SideKind::Maker,
        Some(false) => SideKind::Taker,
        None => SideKind::Unknown,
    };
    let settlement_asset = decoded
        .get("settlementAsset")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let ppm = decoded.get("feePpm").and_then(Value::as_i64);
    let product_kind = decoded
        .get("productKind")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let flow_kind = decoded
        .get("flowKind")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let basis_amount = decoded
        .get("basisAmount")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some(NormalizedFeeEvent {
        model: FeeEventModel::V2Charged,
        event_id: event.id,
        tx_hash: event.tx_hash.to_ascii_lowercase(),
        log_index: event.log_index,
        block_number: event.block_number,
        contract_address: event.contract_address.to_ascii_lowercase(),
        trader,
        recipient,
        side,
        option_id: event.option_id.clone(),
        settlement_asset,
        charged_amount: charged,
        rebate_amount: 0,
        amount_str,
        ppm,
        product_kind,
        flow_kind,
        basis_amount,
    })
}

fn normalize_v2_rebated(event: &OptionExecutionEvent) -> Option<NormalizedFeeEvent> {
    let decoded = event.decoded.clone().unwrap_or(Value::Null);
    let amount_str = decoded
        .get("rebateAmount")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let rebate = amount_str
        .as_deref()
        .and_then(|value| value.parse::<u128>().ok())
        .unwrap_or(0);
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
    let recipient = decoded
        .get("recipient")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    let settlement_asset = decoded
        .get("settlementAsset")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let ppm = decoded.get("rebatePpm").and_then(Value::as_i64);
    let product_kind = decoded
        .get("productKind")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let flow_kind = decoded
        .get("flowKind")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let basis_amount = decoded
        .get("basisAmount")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Some(NormalizedFeeEvent {
        model: FeeEventModel::V2Rebated,
        event_id: event.id,
        tx_hash: event.tx_hash.to_ascii_lowercase(),
        log_index: event.log_index,
        block_number: event.block_number,
        contract_address: event.contract_address.to_ascii_lowercase(),
        trader,
        recipient,
        side: SideKind::Maker,
        option_id: event.option_id.clone(),
        settlement_asset,
        charged_amount: 0,
        rebate_amount: rebate,
        amount_str,
        ppm,
        product_kind,
        flow_kind,
        basis_amount,
    })
}

/// Decide which event model contributes to totals for this set of events.
///
/// - `"v1"` when only V1 events are present.
/// - `"v2"` when only V2 events are present.
/// - `"mixed"` when both are present. V2 still drives the totals — V1 events
///   become compatibility evidence and contribute zero to the totals to
///   avoid double counting.
/// - `"none"` when neither family is present.
pub fn classify_event_model(normalized: &NormalizedFees) -> &'static str {
    let has_v1 = !normalized.v1_charged.is_empty();
    let has_v2 = !normalized.v2_charged.is_empty() || !normalized.v2_rebated.is_empty();
    match (has_v1, has_v2) {
        (false, false) => "none",
        (true, false) => "v1",
        (false, true) => "v2",
        (true, true) => "mixed",
    }
}

pub fn aggregate(normalized: &NormalizedFees) -> AggregatedFees {
    let model = classify_event_model(normalized);
    let mut aggregated = AggregatedFees {
        event_model: model,
        source_priority: if model == "mixed" { "v2" } else { "" },
        trading_fee_event_count: normalized.v1_charged.len() as u64,
        fee_charged_v2_count: normalized.v2_charged.len() as u64,
        fee_rebated_v2_count: normalized.v2_rebated.len() as u64,
        ..AggregatedFees::default()
    };

    // Choose which charged/rebated events drive totals; the others remain
    // visible in the event list but contribute zero so V1 + V2 do not get
    // counted twice for the same fee flow.
    let charging_events: &[NormalizedFeeEvent] = match model {
        "v1" => &normalized.v1_charged,
        "v2" | "mixed" => &normalized.v2_charged,
        _ => &[],
    };
    let rebating_events: &[NormalizedFeeEvent] = match model {
        "v2" | "mixed" => &normalized.v2_rebated,
        _ => &[],
    };

    for entry in charging_events {
        aggregated.charged_total = aggregated
            .charged_total
            .saturating_add(entry.charged_amount);
        accumulate_charge(&mut aggregated, entry);
    }
    for entry in rebating_events {
        aggregated.rebated_total = aggregated.rebated_total.saturating_add(entry.rebate_amount);
        accumulate_rebate(&mut aggregated, entry);
    }
    aggregated
}

fn accumulate_charge(aggregated: &mut AggregatedFees, entry: &NormalizedFeeEvent) {
    let amount = entry.charged_amount;
    if amount == 0 {
        return;
    }
    let trader = aggregated
        .by_trader
        .entry(entry.trader.clone())
        .or_default();
    *trader = trader.saturating_add(amount);
    let recipient = aggregated
        .by_recipient
        .entry(entry.recipient.clone())
        .or_default();
    *recipient = recipient.saturating_add(amount);
    let side = aggregated
        .by_side
        .entry(entry.side.as_str().to_string())
        .or_default();
    *side = side.saturating_add(amount);
}

fn accumulate_rebate(aggregated: &mut AggregatedFees, entry: &NormalizedFeeEvent) {
    let amount = entry.rebate_amount;
    if amount == 0 {
        return;
    }
    let rebated = aggregated
        .rebated_by_trader
        .entry(entry.trader.clone())
        .or_default();
    *rebated = rebated.saturating_add(amount);
}

fn amounts_to_strings(map: &BTreeMap<String, u128>) -> BTreeMap<String, String> {
    map.iter()
        .map(|(key, value)| (key.clone(), value.to_string()))
        .collect()
}

/// Build the lifecycle fee view for a single intent's indexed events.
pub fn summarize_fees_for_lifecycle(
    events: &[OptionExecutionEvent],
    backend_ledger_status: &str,
) -> LifecycleFees {
    let normalized = normalize_fee_events(events);
    let aggregated = aggregate(&normalized);
    let reconciliation_status = if normalized.total_count() == 0 {
        "no_onchain_events"
    } else {
        "onchain_observed"
    };
    let event_payloads: Vec<Value> = collect_event_payloads(&normalized);
    let by_recipient_strings = amounts_to_strings(&aggregated.by_recipient);
    LifecycleFees {
        source_of_truth: "onchain".to_string(),
        event_model: aggregated.event_model.to_string(),
        source_priority: aggregated.source_priority.to_string(),
        trading_fee_event_count: aggregated.trading_fee_event_count,
        fee_charged_v2_count: aggregated.fee_charged_v2_count,
        fee_rebated_v2_count: aggregated.fee_rebated_v2_count,
        events: event_payloads,
        observed_total: aggregated.charged_total.to_string(),
        observed_total_charged: aggregated.charged_total.to_string(),
        observed_total_rebated: aggregated.rebated_total.to_string(),
        net_protocol_fee: aggregated.net_protocol_fee().to_string(),
        by_trader: amounts_to_strings(&aggregated.by_trader),
        rebated_by_trader: amounts_to_strings(&aggregated.rebated_by_trader),
        by_recipient: by_recipient_strings.clone(),
        by_side: amounts_to_strings(&aggregated.by_side),
        total_by_recipient: by_recipient_strings,
        backend_ledger_status: backend_ledger_status.to_string(),
        reconciliation_status: reconciliation_status.to_string(),
    }
}

/// Render the per-event payloads surfaced through `LifecycleFees::events`
/// and `/admin/fees/onchain.events`.
pub fn collect_event_payloads(normalized: &NormalizedFees) -> Vec<Value> {
    let mut payloads: Vec<Value> = Vec::with_capacity(normalized.total_count());
    for entry in &normalized.v1_charged {
        payloads.push(serde_json::json!({
            "event_id": entry.event_id,
            "event_model": "v1",
            "event_name": "TradingFeeCharged",
            "tx_hash": entry.tx_hash,
            "log_index": entry.log_index,
            "block_number": entry.block_number,
            "source_contract": entry.contract_address,
            "trader": entry.trader,
            "recipient": entry.recipient,
            "applied_fee": entry.amount_str,
            "fee_amount": entry.amount_str,
            "rebate_amount": null,
            "is_maker": maybe_bool(entry.side),
            "side": entry.side.as_str(),
            "option_id": entry.option_id,
            "settlement_asset": entry.settlement_asset,
        }));
    }
    for entry in &normalized.v2_charged {
        payloads.push(serde_json::json!({
            "event_id": entry.event_id,
            "event_model": "v2",
            "event_name": "FeeChargedV2",
            "tx_hash": entry.tx_hash,
            "log_index": entry.log_index,
            "block_number": entry.block_number,
            "source_contract": entry.contract_address,
            "trader": entry.trader,
            "recipient": entry.recipient,
            "applied_fee": entry.amount_str,
            "fee_amount": entry.amount_str,
            "rebate_amount": null,
            "basis_amount": entry.basis_amount,
            "is_maker": maybe_bool(entry.side),
            "side": entry.side.as_str(),
            "fee_ppm": entry.ppm,
            "product_kind": entry.product_kind,
            "flow_kind": entry.flow_kind,
            "option_id": entry.option_id,
            "settlement_asset": entry.settlement_asset,
        }));
    }
    for entry in &normalized.v2_rebated {
        payloads.push(serde_json::json!({
            "event_id": entry.event_id,
            "event_model": "v2",
            "event_name": "FeeRebatedV2",
            "tx_hash": entry.tx_hash,
            "log_index": entry.log_index,
            "block_number": entry.block_number,
            "source_contract": entry.contract_address,
            "trader": entry.trader,
            "recipient": entry.recipient,
            "applied_fee": null,
            "fee_amount": null,
            "rebate_amount": entry.amount_str,
            "basis_amount": entry.basis_amount,
            "is_maker": true,
            "side": entry.side.as_str(),
            "rebate_ppm": entry.ppm,
            "product_kind": entry.product_kind,
            "flow_kind": entry.flow_kind,
            "option_id": entry.option_id,
            "settlement_asset": entry.settlement_asset,
        }));
    }
    payloads
}

fn maybe_bool(side: SideKind) -> Value {
    match side {
        SideKind::Maker => Value::Bool(true),
        SideKind::Taker => Value::Bool(false),
        SideKind::Unknown => Value::Null,
    }
}

/// Per-transaction summary used by the admin `/admin/fees/onchain`
/// endpoint. Each entry keeps its own per-tx counts and aggregations so
/// callers can compute drift signals downstream.
#[derive(Default, Debug)]
pub struct OnchainFeeTxSummary {
    pub event_model: &'static str,
    pub source_priority: &'static str,
    pub trading_fee_event_count: u64,
    pub fee_charged_v2_count: u64,
    pub fee_rebated_v2_count: u64,
    pub observed_total_charged: u128,
    pub observed_total_rebated: u128,
    pub by_recipient: BTreeMap<String, u128>,
    pub by_trader: BTreeMap<String, u128>,
    pub by_side: BTreeMap<String, u128>,
    pub rebated_by_trader: BTreeMap<String, u128>,
}

impl OnchainFeeTxSummary {
    fn from_aggregated(aggregated: AggregatedFees) -> Self {
        Self {
            event_model: aggregated.event_model,
            source_priority: aggregated.source_priority,
            trading_fee_event_count: aggregated.trading_fee_event_count,
            fee_charged_v2_count: aggregated.fee_charged_v2_count,
            fee_rebated_v2_count: aggregated.fee_rebated_v2_count,
            observed_total_charged: aggregated.charged_total,
            observed_total_rebated: aggregated.rebated_total,
            by_recipient: aggregated.by_recipient,
            by_trader: aggregated.by_trader,
            by_side: aggregated.by_side,
            rebated_by_trader: aggregated.rebated_by_trader,
        }
    }

    pub fn net_protocol_fee(&self) -> u128 {
        self.observed_total_charged
            .saturating_sub(self.observed_total_rebated)
    }

    pub fn into_value(self, tx_hash: String) -> Value {
        serde_json::json!({
            "tx_hash": tx_hash,
            "event_model": self.event_model,
            "source_priority": self.source_priority,
            "trading_fee_event_count": self.trading_fee_event_count,
            "fee_charged_v2_count": self.fee_charged_v2_count,
            "fee_rebated_v2_count": self.fee_rebated_v2_count,
            "observed_total": self.observed_total_charged.to_string(),
            "observed_total_charged": self.observed_total_charged.to_string(),
            "observed_total_rebated": self.observed_total_rebated.to_string(),
            "net_protocol_fee": self.net_protocol_fee().to_string(),
            "by_recipient": amounts_to_strings(&self.by_recipient),
            "by_trader": amounts_to_strings(&self.by_trader),
            "by_side": amounts_to_strings(&self.by_side),
            "rebated_by_trader": amounts_to_strings(&self.rebated_by_trader),
        })
    }
}

/// Build per-transaction summaries for the admin `/admin/fees/onchain`
/// endpoint. Splits events by `tx_hash`, then applies the
/// per-tx event-model classification (so one V2 tx and one V1 tx in the
/// same response window each get the correct model).
pub fn summarize_admin_onchain(events: &[OptionExecutionEvent]) -> AdminOnchainSummary {
    let normalized = normalize_fee_events(events);
    let overall = aggregate(&normalized);

    // Group events by tx_hash, preserving insertion order via BTreeMap.
    let mut per_tx_events: BTreeMap<String, NormalizedFees> = BTreeMap::new();
    for entry in &normalized.v1_charged {
        per_tx_events
            .entry(entry.tx_hash.clone())
            .or_default()
            .v1_charged
            .push(entry.clone());
    }
    for entry in &normalized.v2_charged {
        per_tx_events
            .entry(entry.tx_hash.clone())
            .or_default()
            .v2_charged
            .push(entry.clone());
    }
    for entry in &normalized.v2_rebated {
        per_tx_events
            .entry(entry.tx_hash.clone())
            .or_default()
            .v2_rebated
            .push(entry.clone());
    }

    let mut per_tx: Vec<(String, OnchainFeeTxSummary)> = per_tx_events
        .into_iter()
        .map(|(tx_hash, tx_events)| {
            let tx_aggregated = aggregate(&tx_events);
            (tx_hash, OnchainFeeTxSummary::from_aggregated(tx_aggregated))
        })
        .collect();
    per_tx.sort_by(|left, right| left.0.cmp(&right.0));

    AdminOnchainSummary {
        normalized,
        overall,
        per_tx,
    }
}

/// Output of `summarize_admin_onchain`: the overall aggregate plus the
/// per-tx breakdown the admin endpoint serializes.
pub struct AdminOnchainSummary {
    pub normalized: NormalizedFees,
    pub overall: AggregatedFees,
    pub per_tx: Vec<(String, OnchainFeeTxSummary)>,
}

impl AdminOnchainSummary {
    pub fn into_admin_json(
        self,
        tx_hash_filter: Option<&str>,
        limit: u32,
        backend_ledger_enabled: bool,
    ) -> Value {
        let event_payloads = collect_event_payloads(&self.normalized);
        let txs: Vec<Value> = self
            .per_tx
            .into_iter()
            .map(|(tx_hash, summary)| summary.into_value(tx_hash))
            .collect();
        let reconciliation_status = if self.normalized.total_count() == 0 {
            "no_onchain_events"
        } else {
            "onchain_observed"
        };

        serde_json::json!({
            "source_of_truth": "onchain",
            "event_model": self.overall.event_model,
            "source_priority": self.overall.source_priority,
            "backend_ledger_enabled": backend_ledger_enabled,
            "backend_ledger_status": if backend_ledger_enabled { "enabled" } else { "disabled" },
            "filter": {
                "tx_hash": tx_hash_filter,
                "limit": limit,
            },
            "trading_fee_event_count": self.overall.trading_fee_event_count,
            "fee_charged_v2_count": self.overall.fee_charged_v2_count,
            "fee_rebated_v2_count": self.overall.fee_rebated_v2_count,
            "observed_total": self.overall.charged_total.to_string(),
            "observed_total_charged": self.overall.charged_total.to_string(),
            "observed_total_rebated": self.overall.rebated_total.to_string(),
            "net_protocol_fee": self.overall.net_protocol_fee().to_string(),
            "by_trader": amounts_to_strings(&self.overall.by_trader),
            "by_recipient": amounts_to_strings(&self.overall.by_recipient),
            "by_side": amounts_to_strings(&self.overall.by_side),
            "rebated_by_trader": amounts_to_strings(&self.overall.rebated_by_trader),
            "reconciliation_status": reconciliation_status,
            "transactions": txs,
            "events": event_payloads,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::OptionExecutionEvent;
    use uuid::Uuid;

    fn make_event(name: &str, log_index: u64, decoded: Value) -> OptionExecutionEvent {
        OptionExecutionEvent {
            id: Uuid::from_u128(900 + log_index as u128),
            chain_id: 84532,
            contract_address: "0x00000000000000000000000000000000000000aa".to_string(),
            tx_hash: "0xtx".to_string(),
            log_index,
            block_number: 1_000,
            block_hash: None,
            event_name: name.to_string(),
            event_signature: name.to_string(),
            intent_id: None,
            onchain_intent_id: None,
            option_execution_transaction_id: None,
            buyer: None,
            seller: None,
            account: None,
            option_id: None,
            quantity_contracts: None,
            premium_per_contract_native: None,
            raw_topics: Value::Array(Vec::new()),
            raw_data: "0x".to_string(),
            decoded: Some(decoded),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn v1_event(
        log_index: u64,
        trader: &str,
        applied_fee: u128,
        is_maker: bool,
    ) -> OptionExecutionEvent {
        make_event(
            "TradingFeeCharged",
            log_index,
            serde_json::json!({
                "trader": trader,
                "recipient": "0x009f38440f058d095b61e0e2ee7fabdf05be7500",
                "settlementAsset": "0x0000000000000000000000000000000000000020",
                "isMaker": is_maker,
                "appliedFee": applied_fee.to_string(),
            }),
        )
    }

    fn v2_charged_event(
        log_index: u64,
        trader: &str,
        fee_amount: u128,
        fee_ppm: i64,
        is_maker: bool,
    ) -> OptionExecutionEvent {
        make_event(
            "FeeChargedV2",
            log_index,
            serde_json::json!({
                "consumer": "0x00000000000000000000000000000000000000aa",
                "trader": trader,
                "recipient": "0x009f38440f058d095b61e0e2ee7fabdf05be7500",
                "settlementAsset": "0x0000000000000000000000000000000000000020",
                "productKind": "option",
                "flowKind": "orderbook",
                "isMaker": is_maker,
                "feePpm": fee_ppm,
                "basisAmount": "10000",
                "feeAmount": fee_amount.to_string(),
            }),
        )
    }

    fn v2_rebated_event(
        log_index: u64,
        trader: &str,
        rebate_amount: u128,
        rebate_ppm: i64,
    ) -> OptionExecutionEvent {
        make_event(
            "FeeRebatedV2",
            log_index,
            serde_json::json!({
                "consumer": "0x00000000000000000000000000000000000000aa",
                "trader": trader,
                "recipient": trader,
                "settlementAsset": "0x0000000000000000000000000000000000000020",
                "productKind": "option",
                "flowKind": "orderbook",
                "rebatePpm": rebate_ppm,
                "basisAmount": "10000",
                "rebateAmount": rebate_amount.to_string(),
            }),
        )
    }

    #[test]
    fn empty_events_classify_as_none() {
        let normalized = normalize_fee_events(&[]);
        let aggregated = aggregate(&normalized);
        assert_eq!(aggregated.event_model, "none");
        assert_eq!(aggregated.charged_total, 0);
        assert_eq!(aggregated.rebated_total, 0);
    }

    #[test]
    fn v1_only_classifies_as_v1_and_sums_applied_fees() {
        let events = [
            v1_event(1, "0xc0a76c2a00000000000000000000000000000000", 6, false),
            v1_event(2, "0xbaf0976a00000000000000000000000000000000", 4, true),
        ];
        let normalized = normalize_fee_events(&events);
        let aggregated = aggregate(&normalized);
        assert_eq!(aggregated.event_model, "v1");
        assert_eq!(aggregated.charged_total, 10);
        assert_eq!(aggregated.rebated_total, 0);
        assert_eq!(aggregated.trading_fee_event_count, 2);
        assert_eq!(aggregated.fee_charged_v2_count, 0);
        assert_eq!(aggregated.by_side.get("taker").copied(), Some(6));
        assert_eq!(aggregated.by_side.get("maker").copied(), Some(4));
    }

    #[test]
    fn v2_only_classifies_as_v2_and_uses_v2_totals() {
        let events = [
            v2_charged_event(
                1,
                "0xc0a76c2a00000000000000000000000000000000",
                25,
                250,
                false,
            ),
            v2_rebated_event(2, "0xbaf0976a00000000000000000000000000000000", 5, -50),
        ];
        let normalized = normalize_fee_events(&events);
        let aggregated = aggregate(&normalized);
        assert_eq!(aggregated.event_model, "v2");
        assert_eq!(aggregated.charged_total, 25);
        assert_eq!(aggregated.rebated_total, 5);
        assert_eq!(aggregated.net_protocol_fee(), 20);
        assert_eq!(aggregated.fee_charged_v2_count, 1);
        assert_eq!(aggregated.fee_rebated_v2_count, 1);
        // Rebated maker does not show up in by_trader (charges-only), but
        // does appear in rebated_by_trader.
        assert!(!aggregated
            .by_trader
            .contains_key("0xbaf0976a00000000000000000000000000000000"));
        assert_eq!(
            aggregated
                .rebated_by_trader
                .get("0xbaf0976a00000000000000000000000000000000")
                .copied(),
            Some(5)
        );
        assert_eq!(aggregated.by_side.get("taker").copied(), Some(25));
    }

    #[test]
    fn mixed_v1_and_v2_does_not_double_count() {
        // Same fee flow: V1 compatibility event + V2 source-of-truth event.
        // The double-counting policy says V2 totals win.
        let events = [
            v1_event(1, "0xc0a76c2a00000000000000000000000000000000", 6, false),
            v2_charged_event(
                2,
                "0xc0a76c2a00000000000000000000000000000000",
                25,
                250,
                false,
            ),
        ];
        let normalized = normalize_fee_events(&events);
        let aggregated = aggregate(&normalized);
        assert_eq!(aggregated.event_model, "mixed");
        assert_eq!(aggregated.source_priority, "v2");
        assert_eq!(aggregated.charged_total, 25);
        assert_eq!(aggregated.trading_fee_event_count, 1);
        assert_eq!(aggregated.fee_charged_v2_count, 1);
    }

    /// V2E-I: `basisAmount` must be normalized off the decoded payload so
    /// admin and lifecycle JSON can surface it next to `feeAmount`.
    #[test]
    fn v2_charged_normalizes_basis_amount() {
        let events = [v2_charged_event(
            1,
            "0xc0a76c2a00000000000000000000000000000000",
            13,
            250,
            false,
        )];
        let normalized = normalize_fee_events(&events);
        assert_eq!(normalized.v2_charged.len(), 1);
        assert_eq!(
            normalized.v2_charged[0].basis_amount.as_deref(),
            Some("10000")
        );
    }

    #[test]
    fn v2_rebated_normalizes_basis_amount() {
        let events = [v2_rebated_event(
            1,
            "0xbaf0976a00000000000000000000000000000000",
            5,
            -50,
        )];
        let normalized = normalize_fee_events(&events);
        assert_eq!(normalized.v2_rebated.len(), 1);
        assert_eq!(
            normalized.v2_rebated[0].basis_amount.as_deref(),
            Some("10000")
        );
    }

    #[test]
    fn v1_event_has_no_basis_amount() {
        let events = [v1_event(
            1,
            "0xc0a76c2a00000000000000000000000000000000",
            6,
            false,
        )];
        let normalized = normalize_fee_events(&events);
        assert_eq!(normalized.v1_charged.len(), 1);
        assert!(normalized.v1_charged[0].basis_amount.is_none());
    }

    /// V2E-I: `collect_event_payloads` must surface `basis_amount` on V2
    /// entries (charged and rebated) and omit/null it on V1 entries.
    #[test]
    fn collect_event_payloads_surfaces_basis_amount_for_v2() {
        let events = [
            v1_event(1, "0xc0a76c2a00000000000000000000000000000000", 6, false),
            v2_charged_event(
                2,
                "0xc0a76c2a00000000000000000000000000000000",
                13,
                250,
                false,
            ),
            v2_rebated_event(3, "0xbaf0976a00000000000000000000000000000000", 5, -50),
        ];
        let normalized = normalize_fee_events(&events);
        let payloads = collect_event_payloads(&normalized);
        assert_eq!(payloads.len(), 3);

        let v1 = payloads
            .iter()
            .find(|payload| payload["event_name"] == "TradingFeeCharged")
            .expect("v1 payload");
        // V1 does not carry a basis amount; the field is null on V1 entries.
        assert!(v1.get("basis_amount").map(Value::is_null).unwrap_or(true));

        let v2_charged = payloads
            .iter()
            .find(|payload| payload["event_name"] == "FeeChargedV2")
            .expect("v2 charged payload");
        assert_eq!(v2_charged["basis_amount"], "10000");
        assert_eq!(v2_charged["fee_amount"], "13");
        assert_eq!(v2_charged["fee_ppm"], 250);

        let v2_rebated = payloads
            .iter()
            .find(|payload| payload["event_name"] == "FeeRebatedV2")
            .expect("v2 rebated payload");
        assert_eq!(v2_rebated["basis_amount"], "10000");
        assert_eq!(v2_rebated["rebate_amount"], "5");
        assert_eq!(v2_rebated["rebate_ppm"], -50);
    }

    /// V2F-N: reproduces the V2F-LM PERP fee smoke transaction
    /// (`0x400acedf…ff63a79a`, block 42188599) — two `FeeChargedV2` events
    /// emitted by `FeesManagerV2` when `PerpEngineV2` called `chargeFee`.
    /// Asserts that:
    ///
    /// - both PERP events carry `productKind = "perp"` and `flowKind =
    ///   "orderbook"` through to the per-event payloads.
    /// - the per-event `basisAmount = "30"` (native notional, not premium)
    ///   is preserved next to `feeAmount` and `feePpm`.
    /// - the aggregator buckets 1 + 1 native mUSDC into `charged_total = 2`,
    ///   no rebate, no double-counting, no V1 fallback.
    /// - per-trader and per-side totals split the V2F-LM taker leg
    ///   (`0x8B94…9A34`, fee=1) from the maker leg (`0x475F…BC0C`, fee=1).
    #[test]
    fn v2f_lm_perp_payloads_expose_perp_product_kind_and_totals() {
        let buyer_taker = "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34";
        let seller_maker = "0x475fe397fa56884952d350aa9ee1c3946964bc0c";
        let recipient = "0x009f38440f058d095b61e0e2ee7fabdf05be7500";
        let settlement_asset = "0x6eae407f5640b006fac9965182e238582a3b412e";

        let events = [
            make_event(
                "FeeChargedV2",
                117,
                serde_json::json!({
                    "consumer": "0xc6c592100723fe0c66343a16e95ec34cc0c2141c",
                    "trader": buyer_taker,
                    "recipient": recipient,
                    "settlementAsset": settlement_asset,
                    "productKind": "perp",
                    "productKindRaw": 1,
                    "flowKind": "orderbook",
                    "flowKindRaw": 0,
                    "isMaker": false,
                    "feePpm": 300,
                    "basisAmount": "30",
                    "feeAmount": "1",
                }),
            ),
            make_event(
                "FeeChargedV2",
                118,
                serde_json::json!({
                    "consumer": "0xc6c592100723fe0c66343a16e95ec34cc0c2141c",
                    "trader": seller_maker,
                    "recipient": recipient,
                    "settlementAsset": settlement_asset,
                    "productKind": "perp",
                    "productKindRaw": 1,
                    "flowKind": "orderbook",
                    "flowKindRaw": 0,
                    "isMaker": true,
                    "feePpm": 50,
                    "basisAmount": "30",
                    "feeAmount": "1",
                }),
            ),
        ];

        let normalized = normalize_fee_events(&events);
        let aggregated = aggregate(&normalized);

        assert_eq!(aggregated.event_model, "v2");
        assert_eq!(aggregated.source_priority, "");
        assert_eq!(aggregated.fee_charged_v2_count, 2);
        assert_eq!(aggregated.fee_rebated_v2_count, 0);
        assert_eq!(aggregated.trading_fee_event_count, 0);
        assert_eq!(aggregated.charged_total, 2);
        assert_eq!(aggregated.rebated_total, 0);
        assert_eq!(aggregated.net_protocol_fee(), 2);
        assert_eq!(aggregated.by_side.get("taker").copied(), Some(1));
        assert_eq!(aggregated.by_side.get("maker").copied(), Some(1));
        assert_eq!(aggregated.by_trader.get(buyer_taker).copied(), Some(1));
        assert_eq!(aggregated.by_trader.get(seller_maker).copied(), Some(1));

        // Per-event payloads must preserve productKind=perp and
        // basisAmount=30 so admin/observability surfaces the V2F-LM economics.
        let payloads = collect_event_payloads(&normalized);
        assert_eq!(payloads.len(), 2);
        for payload in &payloads {
            assert_eq!(payload["event_model"], "v2");
            assert_eq!(payload["event_name"], "FeeChargedV2");
            assert_eq!(payload["product_kind"], "perp");
            assert_eq!(payload["flow_kind"], "orderbook");
            assert_eq!(payload["basis_amount"], "30");
            assert_eq!(payload["fee_amount"], "1");
        }
    }

    /// V2F-N: a mixed V1+V2 PERP transaction must resolve to
    /// `source_priority = "v2"` and only count V2 totals, even though a V1
    /// `TradingFeeCharged` compatibility log appears alongside. Guards
    /// against double-counting when PerpEngineV2 still emits the V1
    /// breadcrumb during the bridging window.
    #[test]
    fn v2f_lm_mixed_v1_v2_perp_does_not_double_count() {
        let buyer_taker = "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34";
        let events = [
            v1_event(115, buyer_taker, 1, false),
            make_event(
                "FeeChargedV2",
                117,
                serde_json::json!({
                    "consumer": "0xc6c592100723fe0c66343a16e95ec34cc0c2141c",
                    "trader": buyer_taker,
                    "recipient": "0x009f38440f058d095b61e0e2ee7fabdf05be7500",
                    "settlementAsset": "0x6eae407f5640b006fac9965182e238582a3b412e",
                    "productKind": "perp",
                    "productKindRaw": 1,
                    "flowKind": "orderbook",
                    "flowKindRaw": 0,
                    "isMaker": false,
                    "feePpm": 300,
                    "basisAmount": "30",
                    "feeAmount": "1",
                }),
            ),
        ];

        let normalized = normalize_fee_events(&events);
        let aggregated = aggregate(&normalized);

        assert_eq!(aggregated.event_model, "mixed");
        assert_eq!(aggregated.source_priority, "v2");
        // Must be 1 (V2 wins), not 2 (V1 + V2 double-counted).
        assert_eq!(aggregated.charged_total, 1);
        assert_eq!(aggregated.fee_charged_v2_count, 1);
        assert_eq!(aggregated.trading_fee_event_count, 1);
    }

    /// V2E-I regression: reproduces the V2E-G live trade payload shape
    /// (`premium=50_000`, taker=13, maker=3) and asserts both `FeeChargedV2`
    /// payloads expose `basis_amount = "50000"`, the mixed model still
    /// resolves to `source_priority = "v2"`, and per-tx totals match the
    /// live values (`observed_total_charged = 16`).
    #[test]
    fn v2e_g_payloads_expose_basis_amount() {
        let buyer = "0xc0a76c2a6c6b70c0b065a05e64417886416cc976";
        let seller = "0xbaf0976a00a0dcc84df5b15d927695c8b014b1c3";
        let events = [
            v1_event(117, buyer, 13, false),
            v1_event(124, seller, 3, true),
            // Re-shape v2_charged_event with the V2E-G basis (50_000) by
            // building the JSON directly; the helper hard-codes basis=10_000.
            make_event(
                "FeeChargedV2",
                111,
                serde_json::json!({
                    "consumer": "0x287cef479be5889eefca847f9e73c860898f48cc",
                    "trader": buyer,
                    "recipient": "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588",
                    "settlementAsset": "0x6eae407f5640b006fac9965182e238582a3b412e",
                    "productKind": "option",
                    "flowKind": "orderbook",
                    "isMaker": false,
                    "feePpm": 250,
                    "basisAmount": "50000",
                    "feeAmount": "13",
                }),
            ),
            make_event(
                "FeeChargedV2",
                118,
                serde_json::json!({
                    "consumer": "0x287cef479be5889eefca847f9e73c860898f48cc",
                    "trader": seller,
                    "recipient": "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588",
                    "settlementAsset": "0x6eae407f5640b006fac9965182e238582a3b412e",
                    "productKind": "option",
                    "flowKind": "orderbook",
                    "isMaker": true,
                    "feePpm": 50,
                    "basisAmount": "50000",
                    "feeAmount": "3",
                }),
            ),
        ];

        let normalized = normalize_fee_events(&events);
        let aggregated = aggregate(&normalized);
        assert_eq!(aggregated.event_model, "mixed");
        assert_eq!(aggregated.source_priority, "v2");
        assert_eq!(aggregated.charged_total, 16);
        assert_eq!(aggregated.rebated_total, 0);
        assert_eq!(aggregated.fee_charged_v2_count, 2);
        assert_eq!(aggregated.fee_rebated_v2_count, 0);
        assert_eq!(aggregated.trading_fee_event_count, 2);

        let payloads = collect_event_payloads(&normalized);
        let v2_payloads: Vec<&Value> = payloads
            .iter()
            .filter(|payload| payload["event_name"] == "FeeChargedV2")
            .collect();
        assert_eq!(v2_payloads.len(), 2);
        for payload in v2_payloads {
            assert_eq!(payload["basis_amount"], "50000");
        }

        // Lifecycle wrapper still surfaces the V2 totals + carries the
        // per-event basis_amount through.
        let lifecycle = summarize_fees_for_lifecycle(&events, "disabled");
        assert_eq!(lifecycle.event_model, "mixed");
        assert_eq!(lifecycle.source_priority, "v2");
        assert_eq!(lifecycle.observed_total_charged, "16");
        let lifecycle_v2_with_basis = lifecycle
            .events
            .iter()
            .filter(|event| {
                event["event_name"] == "FeeChargedV2" && event["basis_amount"] == "50000"
            })
            .count();
        assert_eq!(lifecycle_v2_with_basis, 2);
    }

    /// V2G-N — backend decode readiness pin for the RFQ flow.
    ///
    /// The `flow_kind` field is parsed as a free-form `Option<String>`
    /// from the indexer-decoded `flowKind` topic, with no enum
    /// constraint. The instant the first real `FeeChargedV2{flowKind=1}`
    /// (RFQ) event lands, the backend already surfaces it correctly:
    /// per-event payloads carry `flow_kind = "rfq"` and the aggregator
    /// counts the leg under the right rebate/charge bucket. This test
    /// pins both invariants against a synthetic OPTION RFQ trade
    /// (Tier 2 taker @ basis 1_000_000, applied 94 ppm — the V2G-N
    /// canonical Tier 2 taker discount).
    ///
    /// Cardinality contract: even after this test runs, the
    /// `consumer="rfq"` label NEVER appears on any of the V2 fee
    /// metrics — that bucket carries the `consumer` (engine) label,
    /// not the flow. RFQ shows up only in admin payloads.
    #[test]
    fn v2g_n_indexer_decodes_option_rfq_flow_kind_verbatim() {
        let taker = "0x77ca9dd6ccce2d692fb23877a2db7178807b0020";
        let maker = "0x290bd12c93e467bf51c51f5273d35bddb19e9274";
        let recipient = "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588";
        let settlement_asset = "0x6eae407f5640b006fac9965182e238582a3b412e";
        let consumer = "0x287cef479be5889eefca847f9e73c860898f48cc";

        // Synthetic OPTION RFQ taker (Tier 2): basis = 1_000_000,
        // takerPpm = 125 with a 25% RFQ taker discount applied at the
        // contract layer → 94 ppm → feeAmount = ceil(1_000_000 * 94 / 1e6) = 94.
        let charged = make_event(
            "FeeChargedV2",
            7,
            serde_json::json!({
                "consumer": consumer,
                "trader": taker,
                "recipient": recipient,
                "settlementAsset": settlement_asset,
                "productKind": "option",
                "productKindRaw": 0,
                "flowKind": "rfq",
                "flowKindRaw": 1,
                "isMaker": false,
                "feePpm": 94,
                "basisAmount": "1000000",
                "feeAmount": "94",
            }),
        );
        // Synthetic OPTION RFQ maker rebate (Tier 2): makerPpm = -10
        // (unchanged by Design-Option-A — RFQ discount never amplifies
        // negative ppm). rebateAmount = floor(1_000_000 * 10 / 1e6) = 10.
        let rebated = make_event(
            "FeeRebatedV2",
            8,
            serde_json::json!({
                "consumer": consumer,
                "trader": maker,
                "recipient": maker,
                "settlementAsset": settlement_asset,
                "productKind": "option",
                "productKindRaw": 0,
                "flowKind": "rfq",
                "flowKindRaw": 1,
                "rebatePpm": -10,
                "basisAmount": "1000000",
                "rebateAmount": "10",
            }),
        );

        let normalized = normalize_fee_events(&[charged, rebated]);
        let aggregated = aggregate(&normalized);

        assert_eq!(aggregated.event_model, "v2");
        assert_eq!(aggregated.fee_charged_v2_count, 1);
        assert_eq!(aggregated.fee_rebated_v2_count, 1);
        assert_eq!(aggregated.charged_total, 94);
        assert_eq!(aggregated.rebated_total, 10);
        assert_eq!(aggregated.net_protocol_fee(), 84);

        // Per-event payloads carry `flow_kind = "rfq"` verbatim. No
        // enum filtering — the backend is RFQ-ready out of the box.
        let payloads = collect_event_payloads(&normalized);
        assert_eq!(payloads.len(), 2);
        for payload in &payloads {
            assert_eq!(payload["product_kind"], "option");
            assert_eq!(payload["flow_kind"], "rfq");
        }
    }
}
