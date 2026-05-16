use super::schedule::{resolve_rates_from_volume, FeeProduct, ResolvedFeeRates, RATE_DENOMINATOR};
use super::types::{
    FeeEvent, FeeFlowType, FeeMarketType, FeeSourceType, FeeStatus, RebateAccrual, RebateStatus,
    VolumeBucket,
};
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::mm::permissions::list_permission_accounts;
use crate::options::{OptionFill, OptionRfqFill, OptionRfqQuote, OptionSeries};
use crate::types::{AccountId, Price1e8, Size1e8, TimestampMs};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

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
}

#[allow(clippy::too_many_arguments)]
struct FeeEventInput<'a> {
    source_type: FeeSourceType,
    source_id: String,
    flow_type: FeeFlowType,
    product: FeeProduct,
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
                flow_type: FeeFlowType::Orderbook,
                product: FeeProduct::OptionOrderbook,
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
                flow_type: FeeFlowType::Orderbook,
                product: FeeProduct::OptionOrderbook,
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
            flow_type: FeeFlowType::Rfq,
            product: FeeProduct::OptionRfq,
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
            flow_type: FeeFlowType::Rfq,
            product: FeeProduct::OptionRfq,
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
    let rolling_volume_1e8 = account_rolling_volume_since(
        state,
        input.payer,
        FeeMarketType::Option,
        start_day.as_str(),
    )
    .await?;
    let rates = resolve_rates_from_volume(input.product, rolling_volume_1e8);
    let (fee_rate_micro_bps, rebate_rate_micro_bps) =
        participant_rates(state, input.payer, input.role, rates, input.rebate_kind).await?;
    let fee_amount_1e8 = option_capped_amount_1e8(
        input.notional_1e8,
        input.premium_notional_1e8,
        fee_rate_micro_bps,
        state.fees_config.option_premium_cap_pct,
    )?;
    let rebate_amount_1e8 = option_capped_amount_1e8(
        input.notional_1e8,
        input.premium_notional_1e8,
        rebate_rate_micro_bps,
        state.fees_config.option_premium_cap_pct,
    )?;
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
        market_type: FeeMarketType::Option,
        flow_type: input.flow_type,
        market_id: None,
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
        FeeMarketType::Option,
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
        "market_type_counts": summary.market_type_counts
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
    use crate::options::{OptionSeriesSource, OptionSeriesStatus};

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
}
