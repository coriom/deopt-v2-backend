use super::schedule::{resolve_rates_from_volume, FeeProduct, ResolvedFeeRates, RATE_DENOMINATOR};
use super::types::{
    FeeEvent, FeeFlowType, FeeMarketType, FeeSourceType, FeeStatus, RebateAccrual, RebateStatus,
    VolumeBucket,
};
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::indexer::IndexedPerpTrade;
use crate::mm::permissions::list_permission_accounts;
use crate::options::{
    OptionExecutionEvent, OptionFill, OptionRfqFill, OptionRfqQuote, OptionSeries,
};
use crate::types::{AccountId, MarketId, Price1e8, Size1e8, TimestampMs};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::BTreeMap;

const ONE_1E8: u128 = 100_000_000;
const ROLLING_VOLUME_DAYS: i64 = 28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FeeParticipantRole {
    Maker,
    Taker,
}

impl FeeParticipantRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Maker => "maker",
            Self::Taker => "taker",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RebateEligibilityKind {
    OptionOrderFill,
    OptionRfqFill,
    PerpOrderbook,
    PerpRfq,
}

#[allow(clippy::too_many_arguments)]
struct FeeEventInput<'a> {
    source_type: FeeSourceType,
    source_id: String,
    market_type: FeeMarketType,
    flow_type: FeeFlowType,
    product: FeeProduct,
    market_id: Option<MarketId>,
    option_series_id: Option<String>,
    maker: &'a AccountId,
    taker: &'a AccountId,
    payer: &'a AccountId,
    role: FeeParticipantRole,
    notional_1e8: u128,
    premium_notional_1e8: u128,
    rebate_kind: Option<RebateEligibilityKind>,
    created_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmedPerpTradeFeeInput {
    pub source_id: String,
    pub flow_type: FeeFlowType,
    pub market_id: MarketId,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub execution_price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub buyer_is_maker: bool,
    pub created_at_ms: TimestampMs,
}

pub async fn record_option_order_fills(state: &AppState, fills: &[OptionFill]) -> Result<()> {
    if !state.fees_config.enabled {
        return Ok(());
    }

    for fill in fills {
        let series = load_option_series(state, &fill.option_series_id).await?;
        let notional_1e8 = option_underlying_notional_1e8(&series, fill.size_1e8)?;
        let premium_notional_1e8 = premium_notional_1e8(fill.price_1e8, fill.size_1e8)?;
        let (maker, taker) = option_order_maker_taker(fill);
        record_participant_fee(
            state,
            FeeEventInput {
                source_type: FeeSourceType::OptionOrderFill,
                source_id: fill.fill_id.to_string(),
                market_type: FeeMarketType::Option,
                flow_type: FeeFlowType::Orderbook,
                product: FeeProduct::OptionOrderbook,
                market_id: None,
                option_series_id: Some(fill.option_series_id.clone()),
                maker,
                taker,
                payer: maker,
                role: FeeParticipantRole::Maker,
                notional_1e8,
                premium_notional_1e8,
                rebate_kind: Some(RebateEligibilityKind::OptionOrderFill),
                created_at_ms: fill.created_at_ms,
            },
        )
        .await?;
        record_participant_fee(
            state,
            FeeEventInput {
                source_type: FeeSourceType::OptionOrderFill,
                source_id: fill.fill_id.to_string(),
                market_type: FeeMarketType::Option,
                flow_type: FeeFlowType::Orderbook,
                product: FeeProduct::OptionOrderbook,
                market_id: None,
                option_series_id: Some(fill.option_series_id.clone()),
                maker,
                taker,
                payer: taker,
                role: FeeParticipantRole::Taker,
                notional_1e8,
                premium_notional_1e8,
                rebate_kind: None,
                created_at_ms: fill.created_at_ms,
            },
        )
        .await?;
    }
    Ok(())
}

pub async fn record_option_rfq_fill(
    state: &AppState,
    fill: &OptionRfqFill,
    _quote: &OptionRfqQuote,
) -> Result<()> {
    if !state.fees_config.enabled {
        return Ok(());
    }

    let series = load_option_series(state, &fill.option_series_id).await?;
    let notional_1e8 = option_underlying_notional_1e8(&series, fill.size_1e8)?;
    let premium_notional_1e8 = premium_notional_1e8(fill.price_1e8, fill.size_1e8)?;
    record_participant_fee(
        state,
        FeeEventInput {
            source_type: FeeSourceType::OptionRfqFill,
            source_id: fill.fill_id.to_string(),
            market_type: FeeMarketType::Option,
            flow_type: FeeFlowType::Rfq,
            product: FeeProduct::OptionRfq,
            market_id: None,
            option_series_id: Some(fill.option_series_id.clone()),
            maker: &fill.mm_account,
            taker: &fill.taker,
            payer: &fill.mm_account,
            role: FeeParticipantRole::Maker,
            notional_1e8,
            premium_notional_1e8,
            rebate_kind: Some(RebateEligibilityKind::OptionRfqFill),
            created_at_ms: fill.created_at_ms,
        },
    )
    .await?;
    record_participant_fee(
        state,
        FeeEventInput {
            source_type: FeeSourceType::OptionRfqFill,
            source_id: fill.fill_id.to_string(),
            market_type: FeeMarketType::Option,
            flow_type: FeeFlowType::Rfq,
            product: FeeProduct::OptionRfq,
            market_id: None,
            option_series_id: Some(fill.option_series_id.clone()),
            maker: &fill.mm_account,
            taker: &fill.taker,
            payer: &fill.taker,
            role: FeeParticipantRole::Taker,
            notional_1e8,
            premium_notional_1e8,
            rebate_kind: None,
            created_at_ms: fill.created_at_ms,
        },
    )
    .await
}

pub async fn record_indexed_perp_trade_fees(
    state: &AppState,
    trade: &IndexedPerpTrade,
    flow_type: FeeFlowType,
) -> Result<()> {
    let market_id = parse_u64_field(&trade.market_id, "indexed perp trade market_id")?;
    let size_1e8 = parse_u128_field(&trade.size_delta_1e8, "indexed perp trade size_delta_1e8")?;
    let execution_price_1e8 = parse_u128_field(
        &trade.execution_price_1e8,
        "indexed perp trade execution_price_1e8",
    )?;

    record_perp_trade_fees(
        state,
        &ConfirmedPerpTradeFeeInput {
            source_id: trade.event_id.clone(),
            flow_type,
            market_id,
            buyer: AccountId::new(trade.buyer.clone()),
            seller: AccountId::new(trade.seller.clone()),
            execution_price_1e8,
            size_1e8,
            buyer_is_maker: trade.buyer_is_maker,
            created_at_ms: trade.created_at_ms,
        },
    )
    .await
}

pub async fn record_perp_trade_fees(
    state: &AppState,
    trade: &ConfirmedPerpTradeFeeInput,
) -> Result<()> {
    if !state.fees_config.enabled {
        return Ok(());
    }

    let notional_1e8 = perp_notional_1e8(trade.execution_price_1e8, trade.size_1e8)?;
    let (maker, taker) = perp_maker_taker(&trade.buyer, &trade.seller, trade.buyer_is_maker);
    let product = match trade.flow_type {
        FeeFlowType::Orderbook => FeeProduct::PerpOrderbook,
        FeeFlowType::Rfq => FeeProduct::PerpRfq,
    };
    let rebate_kind = match trade.flow_type {
        FeeFlowType::Orderbook => RebateEligibilityKind::PerpOrderbook,
        FeeFlowType::Rfq => RebateEligibilityKind::PerpRfq,
    };

    record_participant_fee(
        state,
        FeeEventInput {
            source_type: FeeSourceType::PerpTrade,
            source_id: trade.source_id.clone(),
            market_type: FeeMarketType::Perp,
            flow_type: trade.flow_type,
            product,
            market_id: Some(trade.market_id),
            option_series_id: None,
            maker,
            taker,
            payer: maker,
            role: FeeParticipantRole::Maker,
            notional_1e8,
            premium_notional_1e8: notional_1e8,
            rebate_kind: Some(rebate_kind),
            created_at_ms: trade.created_at_ms,
        },
    )
    .await?;
    record_participant_fee(
        state,
        FeeEventInput {
            source_type: FeeSourceType::PerpTrade,
            source_id: trade.source_id.clone(),
            market_type: FeeMarketType::Perp,
            flow_type: trade.flow_type,
            product,
            market_id: Some(trade.market_id),
            option_series_id: None,
            maker,
            taker,
            payer: taker,
            role: FeeParticipantRole::Taker,
            notional_1e8,
            premium_notional_1e8: notional_1e8,
            rebate_kind: None,
            created_at_ms: trade.created_at_ms,
        },
    )
    .await
}

pub async fn admin_fee_summary(state: &AppState) -> Result<Value> {
    let ledger_summary = if let Some(repository) = state.repository.clone() {
        repository.admin_fee_summary().await?
    } else {
        summary_to_json(
            state
                .fees_store
                .lock()
                .map_err(|_| fee_store_lock())?
                .summary()?,
        )
    };
    Ok(serde_json::json!({
        "enabled": state.fees_config.enabled,
        "require_persistence": state.fees_config.require_persistence,
        "rebates_enabled": state.fees_config.rebates_enabled,
        "persistence_enabled": state.persistence_enabled,
        "rate_unit": {
            "name": "micro_bps",
            "micro_bps_per_bps": super::MICRO_BPS_PER_BPS.to_string(),
            "denominator": RATE_DENOMINATOR.to_string()
        },
        "option_fee_basis": state.fees_config.option_fee_basis.as_str(),
        "option_premium_cap_pct": state.fees_config.option_premium_cap_pct,
        "default_fee_asset": state.fees_config.default_fee_asset.clone(),
        "protocol_fee_recipient": state.fees_config.protocol_fee_recipient.clone(),
        "schedule": super::launch_fee_schedule(),
        "ledger": ledger_summary
    }))
}

pub async fn admin_fee_events(state: &AppState, limit: u32) -> Result<Vec<Value>> {
    let limit = limit.min(100);
    if let Some(repository) = state.repository.clone() {
        return repository.admin_recent_fee_events(limit).await;
    }
    Ok(state
        .fees_store
        .lock()
        .map_err(|_| fee_store_lock())?
        .list_fee_events(limit as usize)
        .into_iter()
        .map(fee_event_to_json)
        .collect())
}

/// Summarize the on-chain `TradingFeeCharged` evidence indexed for one or
/// more option execution transactions. This treats the on-chain log as
/// the source of truth for live fees and never depends on the backend
/// fee ledger being enabled.
///
/// `tx_hash` filters to a single broadcast transaction. When `None`, the
/// most recent indexed `TradingFeeCharged` events (up to `limit`) are
/// returned. The backend ledger status is reported explicitly per
/// transaction; a disabled or empty ledger is *not* an error.
pub async fn admin_onchain_fees(
    state: &AppState,
    tx_hash: Option<&str>,
    limit: u32,
) -> Result<Value> {
    let limit = limit.clamp(1, 200);
    let events = match tx_hash {
        Some(hash) => load_events_for_tx_hash(state, hash).await?,
        None => load_recent_trading_fee_events(state, limit).await?,
    };
    let trading_fee_events: Vec<&OptionExecutionEvent> = events
        .iter()
        .filter(|event| event.event_name == "TradingFeeCharged")
        .collect();

    let backend_ledger_enabled = state.fees_config.enabled;
    let mut by_tx: BTreeMap<String, OnchainFeeTxSummary> = BTreeMap::new();
    let mut overall_total: u128 = 0;
    let mut by_recipient: BTreeMap<String, u128> = BTreeMap::new();
    let mut by_trader: BTreeMap<String, u128> = BTreeMap::new();
    let mut by_side: BTreeMap<String, u128> = BTreeMap::new();
    let mut event_payloads: Vec<Value> = Vec::with_capacity(trading_fee_events.len());

    for event in &trading_fee_events {
        let decoded = event.decoded.clone().unwrap_or(Value::Null);
        let applied_fee_str = decoded
            .get("appliedFee")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let applied_fee = applied_fee_str
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
        let is_maker_value = decoded.get("isMaker").cloned().unwrap_or(Value::Null);
        let side_key = match is_maker_value.as_bool() {
            Some(true) => "maker",
            Some(false) => "taker",
            None => "unknown",
        }
        .to_string();
        let settlement_asset = decoded
            .get("settlementAsset")
            .and_then(|value| value.as_str())
            .map(str::to_string);

        overall_total = overall_total.saturating_add(applied_fee);
        *by_recipient.entry(recipient.clone()).or_default() += applied_fee;
        *by_trader.entry(trader.clone()).or_default() += applied_fee;
        *by_side.entry(side_key.clone()).or_default() += applied_fee;

        let tx_summary = by_tx.entry(event.tx_hash.to_ascii_lowercase()).or_default();
        tx_summary.trading_fee_event_count += 1;
        tx_summary.observed_total = tx_summary.observed_total.saturating_add(applied_fee);
        *tx_summary
            .by_recipient
            .entry(recipient.clone())
            .or_default() += applied_fee;
        *tx_summary.by_trader.entry(trader.clone()).or_default() += applied_fee;
        *tx_summary.by_side.entry(side_key.clone()).or_default() += applied_fee;

        event_payloads.push(serde_json::json!({
            "event_id": event.id,
            "chain_id": event.chain_id,
            "tx_hash": event.tx_hash,
            "log_index": event.log_index,
            "block_number": event.block_number,
            "source_contract": event.contract_address,
            "trader": trader,
            "recipient": recipient,
            "applied_fee": applied_fee_str,
            "is_maker": is_maker_value,
            "side": side_key,
            "option_id": event.option_id,
            "settlement_asset": settlement_asset,
        }));
    }

    let txs_view: Vec<Value> = by_tx
        .into_iter()
        .map(|(tx, summary)| summary.into_value(tx))
        .collect();

    Ok(serde_json::json!({
        "source_of_truth": "onchain",
        "backend_ledger_enabled": backend_ledger_enabled,
        "backend_ledger_status": if backend_ledger_enabled { "enabled" } else { "disabled" },
        "filter": {
            "tx_hash": tx_hash,
            "limit": limit,
        },
        "trading_fee_event_count": trading_fee_events.len() as u64,
        "observed_total": overall_total.to_string(),
        "by_trader": amounts_to_strings(&by_trader),
        "by_recipient": amounts_to_strings(&by_recipient),
        "by_side": amounts_to_strings(&by_side),
        "reconciliation_status": if trading_fee_events.is_empty() {
            "no_onchain_events"
        } else {
            "onchain_observed"
        },
        "transactions": txs_view,
        "events": event_payloads,
    }))
}

#[derive(Default)]
struct OnchainFeeTxSummary {
    trading_fee_event_count: u64,
    observed_total: u128,
    by_recipient: BTreeMap<String, u128>,
    by_trader: BTreeMap<String, u128>,
    by_side: BTreeMap<String, u128>,
}

impl OnchainFeeTxSummary {
    fn into_value(self, tx_hash: String) -> Value {
        serde_json::json!({
            "tx_hash": tx_hash,
            "trading_fee_event_count": self.trading_fee_event_count,
            "observed_total": self.observed_total.to_string(),
            "by_recipient": amounts_to_strings(&self.by_recipient),
            "by_trader": amounts_to_strings(&self.by_trader),
            "by_side": amounts_to_strings(&self.by_side),
        })
    }
}

fn amounts_to_strings(map: &BTreeMap<String, u128>) -> BTreeMap<String, String> {
    map.iter()
        .map(|(key, value)| (key.clone(), value.to_string()))
        .collect()
}

async fn load_events_for_tx_hash(
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

async fn load_recent_trading_fee_events(
    state: &AppState,
    limit: u32,
) -> Result<Vec<OptionExecutionEvent>> {
    let request_limit = limit.saturating_mul(8).min(2_000).max(limit);
    let events = if let Some(repository) = state.repository.clone() {
        repository
            .list_option_execution_events(request_limit)
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .list_option_execution_events(request_limit)
    };
    let mut filtered: Vec<OptionExecutionEvent> = events
        .into_iter()
        .filter(|event| event.event_name == "TradingFeeCharged")
        .collect();
    filtered.truncate(limit as usize);
    Ok(filtered)
}

pub async fn admin_fee_volumes(state: &AppState, account: Option<AccountId>) -> Result<Vec<Value>> {
    if let Some(repository) = state.repository.clone() {
        return repository.admin_fee_volumes(account.as_ref()).await;
    }
    Ok(state
        .fees_store
        .lock()
        .map_err(|_| fee_store_lock())?
        .list_volume_buckets(account.as_ref())
        .into_iter()
        .map(volume_bucket_to_json)
        .collect())
}

pub async fn admin_fee_rebates(state: &AppState, account: Option<AccountId>) -> Result<Vec<Value>> {
    if let Some(repository) = state.repository.clone() {
        return repository.admin_fee_rebates(account.as_ref()).await;
    }
    Ok(state
        .fees_store
        .lock()
        .map_err(|_| fee_store_lock())?
        .list_rebate_accruals(account.as_ref())
        .into_iter()
        .map(rebate_accrual_to_json)
        .collect())
}

#[allow(clippy::too_many_arguments)]
async fn record_participant_fee(state: &AppState, input: FeeEventInput<'_>) -> Result<()> {
    let bucket_day = bucket_day(input.created_at_ms)?;
    let start_day = rolling_start_bucket_day(input.created_at_ms)?;
    let rolling_volume_1e8 =
        account_rolling_volume_since(state, input.payer, input.market_type, start_day.as_str())
            .await?;
    let rates = resolve_rates_from_volume(input.product, rolling_volume_1e8);
    let (fee_rate_micro_bps, rebate_rate_micro_bps) =
        participant_rates(state, input.payer, input.role, rates, input.rebate_kind).await?;
    let fee_amount_1e8 = fee_amount_for_input(state, &input, fee_rate_micro_bps)?;
    let rebate_amount_1e8 = fee_amount_for_input(state, &input, rebate_rate_micro_bps)?;
    let protocol_amount_1e8 = fee_amount_1e8;
    let event = FeeEvent {
        fee_event_id: fee_event_id(
            input.source_type,
            input.source_id.as_str(),
            input.role,
            input.payer,
        ),
        source_type: input.source_type,
        source_id: input.source_id,
        market_type: input.market_type,
        flow_type: input.flow_type,
        market_id: input.market_id,
        option_series_id: input.option_series_id,
        maker: Some(input.maker.clone()),
        taker: Some(input.taker.clone()),
        payer: input.payer.clone(),
        recipient: state.fees_config.protocol_fee_recipient.clone(),
        fee_asset: state.fees_config.default_fee_asset.clone(),
        notional_1e8: input.notional_1e8,
        fee_rate_micro_bps,
        fee_amount_1e8,
        rebate_rate_micro_bps,
        rebate_amount_1e8,
        protocol_amount_1e8,
        status: FeeStatus::Accrued,
        created_at_ms: input.created_at_ms,
    };
    let inserted = insert_fee_event(state, &event).await?;
    if !inserted {
        return Ok(());
    }

    let (maker_delta_1e8, taker_delta_1e8) = match input.role {
        FeeParticipantRole::Maker => (input.notional_1e8, 0),
        FeeParticipantRole::Taker => (0, input.notional_1e8),
    };
    upsert_volume_delta(
        state,
        input.payer,
        bucket_day.as_str(),
        input.market_type,
        maker_delta_1e8,
        taker_delta_1e8,
        input.created_at_ms,
    )
    .await?;

    if rebate_amount_1e8 > 0 {
        let rebate = RebateAccrual {
            rebate_id: format!("rebate-{}", event.fee_event_id),
            fee_event_id: event.fee_event_id.clone(),
            account: input.payer.clone(),
            source_type: event.source_type,
            source_id: event.source_id.clone(),
            rebate_asset: event.fee_asset.clone(),
            rebate_amount_1e8,
            status: RebateStatus::Accrued,
            created_at_ms: event.created_at_ms,
        };
        insert_rebate_accrual(state, &rebate).await?;
    }
    Ok(())
}

async fn participant_rates(
    state: &AppState,
    payer: &AccountId,
    role: FeeParticipantRole,
    rates: ResolvedFeeRates,
    rebate_kind: Option<RebateEligibilityKind>,
) -> Result<(u64, u64)> {
    match role {
        FeeParticipantRole::Maker => {
            let rebate_eligible = match rebate_kind {
                Some(kind) => maker_rebate_eligible(state, payer, kind).await?,
                None => false,
            };
            Ok((
                rates.maker_fee_micro_bps,
                if rebate_eligible {
                    rates.maker_rebate_micro_bps
                } else {
                    0
                },
            ))
        }
        FeeParticipantRole::Taker => Ok((rates.taker_fee_micro_bps, 0)),
    }
}

async fn maker_rebate_eligible(
    state: &AppState,
    maker: &AccountId,
    kind: RebateEligibilityKind,
) -> Result<bool> {
    if !state.fees_config.rebates_enabled || !state.mm_permissions_config.enabled {
        return Ok(false);
    }
    let Some(account) = list_permission_accounts(state)
        .await?
        .into_iter()
        .find(|account| account.mm_account.0.eq_ignore_ascii_case(maker.0.as_str()))
    else {
        return Ok(false);
    };
    if !account.enabled {
        return Ok(false);
    }
    Ok(match kind {
        RebateEligibilityKind::OptionRfqFill => account.can_quote_option_rfq,
        RebateEligibilityKind::OptionOrderFill => {
            account.can_submit_option_orders || account.can_quote_option_rfq
        }
        RebateEligibilityKind::PerpOrderbook => account.can_submit_perp_orders,
        RebateEligibilityKind::PerpRfq => account.can_quote_perp_rfq,
    })
}

async fn insert_fee_event(state: &AppState, event: &FeeEvent) -> Result<bool> {
    if let Some(repository) = state.repository.clone() {
        return repository.insert_fee_event(event).await;
    }
    state
        .fees_store
        .lock()
        .map_err(|_| fee_store_lock())?
        .insert_fee_event(event.clone())
}

#[allow(clippy::too_many_arguments)]
async fn upsert_volume_delta(
    state: &AppState,
    account: &AccountId,
    bucket_day: &str,
    market_type: FeeMarketType,
    maker_delta_1e8: u128,
    taker_delta_1e8: u128,
    updated_at_ms: TimestampMs,
) -> Result<VolumeBucket> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .upsert_fee_volume_delta(
                account,
                bucket_day,
                market_type,
                maker_delta_1e8,
                taker_delta_1e8,
                updated_at_ms,
            )
            .await;
    }
    state
        .fees_store
        .lock()
        .map_err(|_| fee_store_lock())?
        .upsert_volume_delta(
            account,
            bucket_day,
            market_type,
            maker_delta_1e8,
            taker_delta_1e8,
            updated_at_ms,
        )
}

async fn insert_rebate_accrual(state: &AppState, rebate: &RebateAccrual) -> Result<bool> {
    if let Some(repository) = state.repository.clone() {
        return repository.insert_rebate_accrual(rebate).await;
    }
    state
        .fees_store
        .lock()
        .map_err(|_| fee_store_lock())?
        .insert_rebate_accrual(rebate.clone())
}

async fn account_rolling_volume_since(
    state: &AppState,
    account: &AccountId,
    market_type: FeeMarketType,
    start_bucket_day: &str,
) -> Result<u128> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .fee_rolling_volume_since(account, market_type, start_bucket_day)
            .await;
    }
    state
        .fees_store
        .lock()
        .map_err(|_| fee_store_lock())?
        .rolling_volume_since(account, market_type, start_bucket_day)
}

fn option_order_maker_taker(fill: &OptionFill) -> (&AccountId, &AccountId) {
    if fill.maker_order_id == fill.buy_order_id {
        (&fill.buyer, &fill.seller)
    } else {
        (&fill.seller, &fill.buyer)
    }
}

pub fn perp_maker_taker<'a>(
    buyer: &'a AccountId,
    seller: &'a AccountId,
    buyer_is_maker: bool,
) -> (&'a AccountId, &'a AccountId) {
    if buyer_is_maker {
        (buyer, seller)
    } else {
        (seller, buyer)
    }
}

pub fn perp_notional_1e8(price_1e8: Price1e8, size_1e8: Size1e8) -> Result<u128> {
    mul_div_1e8(price_1e8, size_1e8, "perp notional")
}

pub fn premium_notional_1e8(price_1e8: Price1e8, size_1e8: Size1e8) -> Result<u128> {
    mul_div_1e8(price_1e8, size_1e8, "option premium notional")
}

pub fn option_underlying_notional_1e8(series: &OptionSeries, size_1e8: Size1e8) -> Result<u128> {
    let strike_times_size = mul_div_1e8(series.strike_1e8, size_1e8, "option strike notional")?;
    mul_div_1e8(
        strike_times_size,
        series.contract_size_1e8,
        "option underlying notional",
    )
}

pub fn option_capped_amount_1e8(
    underlying_notional_1e8: u128,
    premium_notional_1e8: u128,
    rate_micro_bps: u64,
    premium_cap_pct: u64,
) -> Result<u128> {
    let underlying_amount = amount_from_rate(underlying_notional_1e8, rate_micro_bps)?;
    let premium_cap = premium_notional_1e8
        .checked_mul(u128::from(premium_cap_pct))
        .ok_or_else(|| BackendError::Config("option premium fee cap overflow".to_string()))?
        / 100;
    Ok(underlying_amount.min(premium_cap))
}

pub fn amount_from_rate(notional_1e8: u128, rate_micro_bps: u64) -> Result<u128> {
    notional_1e8
        .checked_mul(u128::from(rate_micro_bps))
        .ok_or_else(|| BackendError::Config("fee amount overflow".to_string()))
        .map(|value| value / RATE_DENOMINATOR)
}

fn fee_amount_for_input(
    state: &AppState,
    input: &FeeEventInput<'_>,
    rate_micro_bps: u64,
) -> Result<u128> {
    match input.market_type {
        FeeMarketType::Option => option_capped_amount_1e8(
            input.notional_1e8,
            input.premium_notional_1e8,
            rate_micro_bps,
            state.fees_config.option_premium_cap_pct,
        ),
        FeeMarketType::Perp => amount_from_rate(input.notional_1e8, rate_micro_bps),
    }
}

fn parse_u128_field(value: &str, field: &str) -> Result<u128> {
    value
        .parse()
        .map_err(|error| BackendError::Persistence(format!("invalid {field}: {error}")))
}

fn parse_u64_field(value: &str, field: &str) -> Result<u64> {
    value
        .parse()
        .map_err(|error| BackendError::Persistence(format!("invalid {field}: {error}")))
}

fn mul_div_1e8(left: u128, right: u128, context: &str) -> Result<u128> {
    left.checked_mul(right)
        .ok_or_else(|| BackendError::Config(format!("{context} overflow")))?
        .checked_div(ONE_1E8)
        .ok_or_else(|| BackendError::Config(format!("{context} division failed")))
}

fn fee_event_id(
    source_type: FeeSourceType,
    source_id: &str,
    role: FeeParticipantRole,
    payer: &AccountId,
) -> String {
    format!(
        "fee-{}-{}-{}-{}",
        source_type.as_str(),
        source_id,
        role.as_str(),
        payer.0.to_ascii_lowercase()
    )
}

fn bucket_day(timestamp_ms: TimestampMs) -> Result<String> {
    let datetime = DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| BackendError::Config("fee bucket timestamp is out of range".to_string()))?;
    Ok(datetime.format("%Y-%m-%d").to_string())
}

fn rolling_start_bucket_day(timestamp_ms: TimestampMs) -> Result<String> {
    let datetime = DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| BackendError::Config("fee bucket timestamp is out of range".to_string()))?;
    let start = datetime
        .checked_sub_signed(Duration::days(ROLLING_VOLUME_DAYS))
        .ok_or_else(|| {
            BackendError::Config("fee rolling window timestamp underflow".to_string())
        })?;
    Ok(start.format("%Y-%m-%d").to_string())
}

async fn load_option_series(state: &AppState, option_series_id: &str) -> Result<OptionSeries> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_series(option_series_id)
            .await?
            .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()));
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_series(option_series_id)
        .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))
}

fn summary_to_json(summary: super::types::FeeLedgerSummary) -> Value {
    serde_json::json!({
        "event_count": summary.event_count,
        "fee_total_1e8": summary.fee_total_1e8.to_string(),
        "rebate_total_1e8": summary.rebate_total_1e8.to_string(),
        "protocol_total_1e8": summary.protocol_total_1e8.to_string(),
        "status_counts": summary.status_counts,
        "source_type_counts": summary.source_type_counts,
        "market_type_counts": summary.market_type_counts,
        "flow_type_counts": summary.flow_type_counts
    })
}

pub fn fee_event_to_json(event: FeeEvent) -> Value {
    serde_json::json!({
        "fee_event_id": event.fee_event_id,
        "source_type": event.source_type.as_str(),
        "source_id": event.source_id,
        "market_type": event.market_type.as_str(),
        "flow_type": event.flow_type.as_str(),
        "market_id": event.market_id,
        "option_series_id": event.option_series_id,
        "maker": event.maker,
        "taker": event.taker,
        "payer": event.payer,
        "recipient": event.recipient,
        "fee_asset": event.fee_asset,
        "notional_1e8": event.notional_1e8.to_string(),
        "fee_rate_micro_bps": event.fee_rate_micro_bps,
        "fee_amount_1e8": event.fee_amount_1e8.to_string(),
        "rebate_rate_micro_bps": event.rebate_rate_micro_bps,
        "rebate_amount_1e8": event.rebate_amount_1e8.to_string(),
        "protocol_amount_1e8": event.protocol_amount_1e8.to_string(),
        "status": event.status.as_str(),
        "created_at_ms": event.created_at_ms
    })
}

pub fn volume_bucket_to_json(bucket: VolumeBucket) -> Value {
    serde_json::json!({
        "bucket_id": bucket.bucket_id,
        "account": bucket.account,
        "bucket_day": bucket.bucket_day,
        "market_type": bucket.market_type.as_str(),
        "maker_volume_1e8": bucket.maker_volume_1e8.to_string(),
        "taker_volume_1e8": bucket.taker_volume_1e8.to_string(),
        "total_volume_1e8": bucket.total_volume_1e8.to_string(),
        "updated_at_ms": bucket.updated_at_ms
    })
}

pub fn rebate_accrual_to_json(rebate: RebateAccrual) -> Value {
    serde_json::json!({
        "rebate_id": rebate.rebate_id,
        "fee_event_id": rebate.fee_event_id,
        "account": rebate.account,
        "source_type": rebate.source_type.as_str(),
        "source_id": rebate.source_id,
        "rebate_asset": rebate.rebate_asset,
        "rebate_amount_1e8": rebate.rebate_amount_1e8.to_string(),
        "status": rebate.status.as_str(),
        "created_at_ms": rebate.created_at_ms
    })
}

fn fee_store_lock() -> BackendError {
    BackendError::Config("fee ledger store lock poisoned".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineState;
    use crate::fees::FeesConfig;
    use crate::mm::{MmAccountPermissions, MmPermissionsConfig};
    use crate::options::{OptionSeriesSource, OptionSeriesStatus};

    const CREATED_AT_MS: TimestampMs = 1_700_000_000_000;
    const VOLUME_25M_1E8: u128 = 25_000_000 * ONE_1E8;

    fn account_one() -> AccountId {
        AccountId::new("0x0000000000000000000000000000000000000001")
    }

    fn account_two() -> AccountId {
        AccountId::new("0x0000000000000000000000000000000000000002")
    }

    fn fee_state(rebates_enabled: bool) -> AppState {
        let mut state = AppState::new(EngineState::with_default_markets());
        let mut fees = FeesConfig::enabled_in_memory_for_tests();
        fees.rebates_enabled = rebates_enabled;
        state.fees_config = fees;
        state
    }

    fn confirmed_perp_trade(
        source_id: &str,
        flow_type: FeeFlowType,
        buyer_is_maker: bool,
    ) -> ConfirmedPerpTradeFeeInput {
        ConfirmedPerpTradeFeeInput {
            source_id: source_id.to_string(),
            flow_type,
            market_id: 1,
            buyer: account_one(),
            seller: account_two(),
            execution_price_1e8: 3_000 * ONE_1E8,
            size_1e8: 2 * ONE_1E8,
            buyer_is_maker,
            created_at_ms: CREATED_AT_MS,
        }
    }

    fn seed_perp_volume(
        state: &AppState,
        account: &AccountId,
        maker_volume_1e8: u128,
        taker_volume_1e8: u128,
    ) {
        let bucket_day = bucket_day(CREATED_AT_MS).unwrap();
        state
            .fees_store
            .lock()
            .unwrap()
            .upsert_volume_delta(
                account,
                &bucket_day,
                FeeMarketType::Perp,
                maker_volume_1e8,
                taker_volume_1e8,
                CREATED_AT_MS,
            )
            .unwrap();
    }

    fn enable_perp_mm_permissions(
        state: &mut AppState,
        account: AccountId,
        can_submit_perp_orders: bool,
        can_quote_perp_rfq: bool,
    ) {
        state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
        state
            .mm_permissions
            .lock()
            .unwrap()
            .upsert_account(MmAccountPermissions {
                mm_account: account,
                enabled: true,
                label: Some("perp-mm".to_string()),
                can_submit_perp_orders,
                can_quote_perp_rfq,
                can_quote_option_rfq: false,
                can_submit_option_orders: false,
                created_at_ms: 1,
                updated_at_ms: 1,
            });
    }

    fn test_series() -> OptionSeries {
        OptionSeries {
            option_series_id: "series-1".to_string(),
            underlying: "ETH".to_string(),
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry: 2_000_000_000,
            strike_1e8: 2_000 * ONE_1E8,
            is_call: true,
            contract_size_1e8: ONE_1E8,
            status: OptionSeriesStatus::Active,
            source: OptionSeriesSource::Manual,
            onchain_product_id: None,
            onchain_series_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn option_notional_uses_strike_size_and_contract_size() {
        let series = test_series();

        let notional = option_underlying_notional_1e8(&series, 2 * ONE_1E8).unwrap();
        let premium = premium_notional_1e8(50 * ONE_1E8, 2 * ONE_1E8).unwrap();

        assert_eq!(notional, 4_000 * ONE_1E8);
        assert_eq!(premium, 100 * ONE_1E8);
    }

    #[test]
    fn option_fee_cap_uses_premium_cap() {
        let underlying = 1_000_000 * ONE_1E8;
        let premium = 10 * ONE_1E8;
        let amount = option_capped_amount_1e8(underlying, premium, 25_000, 10).unwrap();

        assert_eq!(amount, ONE_1E8);
    }

    #[test]
    fn perp_notional_uses_price_times_size() {
        let notional = perp_notional_1e8(3_000 * ONE_1E8, 2 * ONE_1E8).unwrap();

        assert_eq!(notional, 6_000 * ONE_1E8);
    }

    #[test]
    fn buyer_is_maker_classifies_buyer_as_maker() {
        let buyer = account_one();
        let seller = account_two();
        let (maker, taker) = perp_maker_taker(&buyer, &seller, true);

        assert_eq!(maker, &buyer);
        assert_eq!(taker, &seller);
    }

    #[test]
    fn buyer_is_taker_classifies_seller_as_maker() {
        let buyer = account_one();
        let seller = account_two();
        let (maker, taker) = perp_maker_taker(&buyer, &seller, false);

        assert_eq!(maker, &seller);
        assert_eq!(taker, &buyer);
    }

    #[tokio::test]
    async fn confirmed_perp_trade_records_taker_fee_and_perp_volumes() {
        let state = fee_state(false);
        let trade = confirmed_perp_trade("0xtx:1", FeeFlowType::Orderbook, false);

        record_perp_trade_fees(&state, &trade).await.unwrap();

        let store = state.fees_store.lock().unwrap();
        let events = store.list_fee_events(10);
        assert_eq!(events.len(), 2);
        let maker_event = events
            .iter()
            .find(|event| event.payer == account_two())
            .unwrap();
        let taker_event = events
            .iter()
            .find(|event| event.payer == account_one())
            .unwrap();
        assert_eq!(maker_event.source_type.as_str(), "perp_trade");
        assert_eq!(maker_event.market_type, FeeMarketType::Perp);
        assert_eq!(maker_event.flow_type, FeeFlowType::Orderbook);
        assert_eq!(maker_event.market_id, Some(1));
        assert_eq!(maker_event.maker.as_ref(), Some(&account_two()));
        assert_eq!(maker_event.taker.as_ref(), Some(&account_one()));
        assert_eq!(maker_event.notional_1e8, 6_000 * ONE_1E8);
        assert_eq!(maker_event.fee_rate_micro_bps, 5_000);
        assert_eq!(maker_event.fee_amount_1e8, 30_000_000);
        assert_eq!(taker_event.fee_rate_micro_bps, 30_000);
        assert_eq!(taker_event.fee_amount_1e8, 180_000_000);

        let volumes = store.list_volume_buckets(None);
        assert_eq!(volumes.len(), 2);
        let maker_volume = volumes
            .iter()
            .find(|bucket| bucket.account == account_two())
            .unwrap();
        let taker_volume = volumes
            .iter()
            .find(|bucket| bucket.account == account_one())
            .unwrap();
        assert_eq!(maker_volume.market_type, FeeMarketType::Perp);
        assert_eq!(maker_volume.maker_volume_1e8, 6_000 * ONE_1E8);
        assert_eq!(maker_volume.taker_volume_1e8, 0);
        assert_eq!(taker_volume.market_type, FeeMarketType::Perp);
        assert_eq!(taker_volume.maker_volume_1e8, 0);
        assert_eq!(taker_volume.taker_volume_1e8, 6_000 * ONE_1E8);
    }

    #[tokio::test]
    async fn permissioned_perp_mm_maker_rebate_accrues() {
        let mut state = fee_state(true);
        enable_perp_mm_permissions(&mut state, account_two(), true, false);
        seed_perp_volume(&state, &account_two(), VOLUME_25M_1E8, 0);
        let trade = confirmed_perp_trade("0xtx:2", FeeFlowType::Orderbook, false);

        record_perp_trade_fees(&state, &trade).await.unwrap();

        let store = state.fees_store.lock().unwrap();
        let events = store.list_fee_events(10);
        let maker_event = events
            .iter()
            .find(|event| event.payer == account_two() && event.source_id == "0xtx:2")
            .unwrap();
        assert_eq!(maker_event.fee_rate_micro_bps, 0);
        assert_eq!(maker_event.fee_amount_1e8, 0);
        assert_eq!(maker_event.rebate_rate_micro_bps, 10_000);
        assert_eq!(maker_event.rebate_amount_1e8, 60_000_000);
        let rebates = store.list_rebate_accruals(Some(&account_two()));
        assert_eq!(rebates.len(), 1);
        assert_eq!(rebates[0].fee_event_id, maker_event.fee_event_id);
        assert_eq!(rebates[0].rebate_amount_1e8, 60_000_000);
    }

    #[tokio::test]
    async fn permissioned_perp_rfq_maker_rebate_uses_rfq_capability() {
        let mut state = fee_state(true);
        enable_perp_mm_permissions(&mut state, account_two(), false, true);
        seed_perp_volume(&state, &account_two(), VOLUME_25M_1E8, 0);
        let trade = confirmed_perp_trade("0xtx:2-rfq", FeeFlowType::Rfq, false);

        record_perp_trade_fees(&state, &trade).await.unwrap();

        let store = state.fees_store.lock().unwrap();
        let maker_event = store
            .list_fee_events(10)
            .into_iter()
            .find(|event| event.payer == account_two() && event.source_id == "0xtx:2-rfq")
            .unwrap();
        assert_eq!(maker_event.flow_type, FeeFlowType::Rfq);
        assert_eq!(maker_event.fee_rate_micro_bps, 0);
        assert_eq!(maker_event.rebate_rate_micro_bps, 10_000);
        assert_eq!(maker_event.rebate_amount_1e8, 60_000_000);
        assert_eq!(store.list_rebate_accruals(Some(&account_two())).len(), 1);
    }

    #[tokio::test]
    async fn no_perp_rebate_when_rebates_disabled() {
        let mut state = fee_state(false);
        enable_perp_mm_permissions(&mut state, account_two(), true, false);
        seed_perp_volume(&state, &account_two(), VOLUME_25M_1E8, 0);
        let trade = confirmed_perp_trade("0xtx:3", FeeFlowType::Orderbook, false);

        record_perp_trade_fees(&state, &trade).await.unwrap();

        let store = state.fees_store.lock().unwrap();
        let maker_event = store
            .list_fee_events(10)
            .into_iter()
            .find(|event| event.payer == account_two() && event.source_id == "0xtx:3")
            .unwrap();
        assert_eq!(maker_event.rebate_rate_micro_bps, 0);
        assert_eq!(maker_event.rebate_amount_1e8, 0);
        assert!(store.list_rebate_accruals(None).is_empty());
    }

    #[tokio::test]
    async fn no_perp_rebate_when_maker_is_not_permissioned() {
        let mut state = fee_state(true);
        state.mm_permissions_config = MmPermissionsConfig::enabled_in_memory_for_tests();
        seed_perp_volume(&state, &account_two(), VOLUME_25M_1E8, 0);
        let trade = confirmed_perp_trade("0xtx:4", FeeFlowType::Orderbook, false);

        record_perp_trade_fees(&state, &trade).await.unwrap();

        let store = state.fees_store.lock().unwrap();
        let maker_event = store
            .list_fee_events(10)
            .into_iter()
            .find(|event| event.payer == account_two() && event.source_id == "0xtx:4")
            .unwrap();
        assert_eq!(maker_event.rebate_rate_micro_bps, 0);
        assert_eq!(maker_event.rebate_amount_1e8, 0);
        assert!(store.list_rebate_accruals(None).is_empty());
    }

    #[tokio::test]
    async fn repeated_perp_source_is_idempotent() {
        let state = fee_state(false);
        let trade = confirmed_perp_trade("0xtx:5", FeeFlowType::Orderbook, false);

        record_perp_trade_fees(&state, &trade).await.unwrap();
        record_perp_trade_fees(&state, &trade).await.unwrap();

        let store = state.fees_store.lock().unwrap();
        assert_eq!(store.list_fee_events(10).len(), 2);
        let volumes = store.list_volume_buckets(None);
        let maker_volume = volumes
            .iter()
            .find(|bucket| bucket.account == account_two())
            .unwrap();
        let taker_volume = volumes
            .iter()
            .find(|bucket| bucket.account == account_one())
            .unwrap();
        assert_eq!(maker_volume.maker_volume_1e8, 6_000 * ONE_1E8);
        assert_eq!(taker_volume.taker_volume_1e8, 6_000 * ONE_1E8);
    }
}
