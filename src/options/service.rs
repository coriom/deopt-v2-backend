use super::series_id::{option_series_id, OptionSeriesIdInput};
use super::{
    OptionFill, OptionFillFilter, OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderId,
    OptionOrderStatus, OptionOrderbookLevel, OptionOrderbookSnapshot, OptionRfqFill, OptionRfqId,
    OptionRfqQuote, OptionRfqQuoteId, OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus,
    OptionSeries, OptionSeriesFilter, OptionSeriesId, OptionSeriesSource, OptionSeriesStatus,
};
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::signing::eip712::parse_evm_address;
use crate::signing::signature::validate_signature_shape;
use crate::types::{now_ms, AccountId, OrderId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOptionSeriesInput {
    pub underlying: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: Price1e8,
    pub is_call: bool,
    pub contract_size_1e8: Option<Size1e8>,
    pub onchain_product_id: Option<String>,
    pub onchain_series_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionOrderInput {
    pub option_series_id: OptionSeriesId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub time_in_force: TimeInForce,
    pub client_order_id: Option<String>,
    pub nonce: Option<u64>,
    pub deadline_ms: Option<TimestampMs>,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionOrderOutcome {
    pub order: OptionOrder,
    pub fills: Vec<OptionFill>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOptionRfqInput {
    pub taker: AccountId,
    pub option_series_id: OptionSeriesId,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub limit_price_1e8: Option<Price1e8>,
    pub ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionRfqQuoteInput {
    pub mm_account: AccountId,
    pub session_id: Option<String>,
    pub client_quote_id: Option<String>,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub quote_ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptOptionRfqQuoteOutcome {
    pub rfq: OptionRfqRequest,
    pub quote: OptionRfqQuote,
    pub fill: OptionRfqFill,
}

pub async fn create_option_series(
    state: &AppState,
    input: CreateOptionSeriesInput,
) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    if !state.options_config.allow_manual_series {
        return Err(BackendError::InvalidOptionSeriesState(
            "manual option series creation is disabled".to_string(),
        ));
    }

    let now = now_ms();
    let now_sec = now_sec(now)?;
    let contract_size_1e8 = input
        .contract_size_1e8
        .unwrap_or(state.options_config.default_contract_size_1e8);
    validate_assets(&[
        ("underlying", &input.underlying),
        ("base_asset", &input.base_asset),
        ("quote_asset", &input.quote_asset),
        ("settlement_asset", &input.settlement_asset),
    ])?;
    if input.expiry <= now_sec {
        return Err(BackendError::InvalidOptionSeriesState(
            "option series expiry must be in the future".to_string(),
        ));
    }
    if input.strike_1e8 == 0 {
        return Err(BackendError::InvalidFixedPoint {
            field: "strike_1e8".to_string(),
            reason: "must be greater than zero".to_string(),
        });
    }
    if contract_size_1e8 == 0 {
        return Err(BackendError::InvalidFixedPoint {
            field: "contract_size_1e8".to_string(),
            reason: "must be greater than zero".to_string(),
        });
    }

    let underlying = trim_asset(input.underlying);
    let base_asset = trim_asset(input.base_asset);
    let quote_asset = trim_asset(input.quote_asset);
    let settlement_asset = trim_asset(input.settlement_asset);
    let option_series_id = option_series_id(OptionSeriesIdInput {
        underlying: &underlying,
        base_asset: &base_asset,
        quote_asset: &quote_asset,
        settlement_asset: &settlement_asset,
        expiry: input.expiry,
        strike_1e8: input.strike_1e8,
        is_call: input.is_call,
        contract_size_1e8,
    });

    if let Some(existing) = get_option_series_optional(state, &option_series_id).await? {
        return Ok(existing);
    }

    let series = OptionSeries {
        option_series_id,
        underlying,
        base_asset,
        quote_asset,
        settlement_asset,
        expiry: input.expiry,
        strike_1e8: input.strike_1e8,
        is_call: input.is_call,
        contract_size_1e8,
        status: OptionSeriesStatus::Active,
        source: OptionSeriesSource::Manual,
        onchain_product_id: input.onchain_product_id,
        onchain_series_id: input.onchain_series_id,
        created_at_ms: now,
        updated_at_ms: now,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_option_series(&series).await?;
        Ok(repository
            .get_option_series(&series.option_series_id)
            .await?
            .unwrap_or(series))
    } else {
        Ok(state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .insert_series(series))
    }
}

pub async fn list_option_series(
    state: &AppState,
    filter: OptionSeriesFilter,
) -> Result<Vec<OptionSeries>> {
    ensure_enabled(state)?;
    let now_sec = now_sec(now_ms())?;
    if let Some(repository) = state.repository.clone() {
        return Ok(repository
            .list_option_series()
            .await?
            .into_iter()
            .filter(|series| filter.matches(series, now_sec))
            .collect());
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_series(&filter, now_sec))
}

pub async fn get_option_series(state: &AppState, option_series_id: &str) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    get_option_series_optional(state, option_series_id)
        .await?
        .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))
}

pub async fn disable_option_series(
    state: &AppState,
    option_series_id: &str,
) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    let now = now_ms();
    if let Some(repository) = state.repository.clone() {
        return repository
            .disable_option_series(option_series_id, now)
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .disable_series(option_series_id, now)
}

pub async fn get_option_orderbook(
    state: &AppState,
    option_series_id: OptionSeriesId,
) -> Result<OptionOrderbookSnapshot> {
    let series = get_option_series(state, &option_series_id).await?;
    let orders = open_option_orders_for_series(state, &option_series_id).await?;
    Ok(OptionOrderbookSnapshot {
        option_series_id,
        status: series.effective_status(now_sec(now_ms())?),
        bids: aggregate_levels(&orders, Side::Buy),
        asks: aggregate_levels(&orders, Side::Sell),
    })
}

pub async fn submit_option_order(
    state: &AppState,
    input: SubmitOptionOrderInput,
) -> Result<SubmitOptionOrderOutcome> {
    ensure_enabled(state)?;
    validate_account(&input.account)?;
    if input.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if input.time_in_force != TimeInForce::Gtc {
        return Err(BackendError::UnsupportedTimeInForce(format!(
            "{:?}",
            input.time_in_force
        )));
    }
    if let Some(deadline_ms) = input.deadline_ms {
        if now_ms() >= deadline_ms {
            return Err(BackendError::DeadlineExpired);
        }
    }
    if let Some(signature) = &input.signature {
        validate_signature_shape(signature)?;
    }

    let series = get_option_series(state, &input.option_series_id).await?;
    if series.effective_status(now_sec(now_ms())?) != OptionSeriesStatus::Active {
        return Err(BackendError::InvalidOptionOrderState(
            "option series is not active".to_string(),
        ));
    }

    let now = now_ms();
    let order = OptionOrder {
        order_id: OrderId::new(),
        option_series_id: input.option_series_id,
        account: input.account,
        side: input.side,
        price_1e8: input.price_1e8,
        size_1e8: input.size_1e8,
        remaining_size_1e8: input.size_1e8,
        time_in_force: input.time_in_force,
        client_order_id: input.client_order_id,
        nonce: input.nonce,
        deadline_ms: input.deadline_ms,
        signature: input.signature,
        status: OptionOrderStatus::Open,
        created_at_ms: now,
        updated_at_ms: now,
    };

    if let Some(repository) = state.repository.clone() {
        let (order, fills) = repository.submit_option_order_and_match(order, now).await?;
        return Ok(SubmitOptionOrderOutcome { order, fills });
    }

    let (order, fills) = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .submit_order_and_match(order, now)?;
    Ok(SubmitOptionOrderOutcome { order, fills })
}

pub async fn list_option_orders(
    state: &AppState,
    filter: OptionOrderFilter,
) -> Result<Vec<OptionOrder>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return Ok(repository
            .list_option_orders()
            .await?
            .into_iter()
            .filter(|order| filter.matches(order))
            .collect());
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_orders(&filter))
}

pub async fn get_option_order(state: &AppState, order_id: OptionOrderId) -> Result<OptionOrder> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_order(order_id)
            .await?
            .ok_or(BackendError::InvalidOptionOrderId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_order(order_id)
        .ok_or(BackendError::InvalidOptionOrderId)
}

pub async fn cancel_option_order(state: &AppState, order_id: OptionOrderId) -> Result<OptionOrder> {
    ensure_enabled(state)?;
    let now = now_ms();
    if let Some(repository) = state.repository.clone() {
        return repository.cancel_option_order(order_id, now).await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .cancel_order(order_id, now)
}

pub async fn list_option_fills(
    state: &AppState,
    filter: OptionFillFilter,
) -> Result<Vec<OptionFill>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return Ok(repository
            .list_option_fills()
            .await?
            .into_iter()
            .filter(|fill| filter.matches(fill))
            .collect());
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_fills(&filter))
}

pub async fn get_option_fill(state: &AppState, fill_id: OptionFillId) -> Result<OptionFill> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_fill(fill_id)
            .await?
            .ok_or(BackendError::InvalidOptionFillId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_fill(fill_id)
        .ok_or(BackendError::InvalidOptionFillId)
}

pub async fn get_option_order_fills(
    state: &AppState,
    order_id: OptionOrderId,
) -> Result<Vec<OptionFill>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository.option_fills_for_order(order_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .fills_for_order(order_id))
}

pub async fn create_option_rfq(
    state: &AppState,
    input: CreateOptionRfqInput,
) -> Result<OptionRfqRequest> {
    ensure_option_rfq_enabled(state)?;
    validate_account(&input.taker)?;
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if input.limit_price_1e8 == Some(0) {
        return Err(BackendError::ZeroPrice);
    }

    let series = get_option_series(state, &input.option_series_id).await?;
    if series.effective_status(now_sec(now_ms())?) != OptionSeriesStatus::Active {
        return Err(BackendError::InvalidOptionRfqState(
            "option series is not active".to_string(),
        ));
    }

    let ttl_ms = input
        .ttl_ms
        .unwrap_or(state.options_config.rfq_default_ttl_ms);
    if ttl_ms == 0 {
        return Err(BackendError::InvalidOptionRfqState(
            "option RFQ ttl_ms must be greater than zero".to_string(),
        ));
    }
    let ttl_ms = ttl_ms.min(state.options_config.rfq_max_ttl_ms);
    let now = now_ms();
    let expires_at_ms = checked_expiry(now, ttl_ms, "option RFQ expiry")?;
    let rfq = OptionRfqRequest {
        option_rfq_id: Uuid::new_v4(),
        taker: input.taker,
        option_series_id: input.option_series_id,
        side: input.side,
        size_1e8: input.size_1e8,
        limit_price_1e8: input.limit_price_1e8,
        status: OptionRfqStatus::Open,
        created_at_ms: now,
        expires_at_ms,
        accepted_quote_id: None,
        option_fill_id: None,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_option_rfq(&rfq).await?;
        return repository
            .get_option_rfq(rfq.option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId);
    }

    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_rfq(rfq))
}

pub async fn list_option_rfqs(state: &AppState) -> Result<Vec<OptionRfqRequest>> {
    ensure_option_rfq_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_option_rfqs().await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_rfqs())
}

pub async fn get_option_rfq(
    state: &AppState,
    option_rfq_id: OptionRfqId,
) -> Result<OptionRfqRequest> {
    ensure_option_rfq_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_rfq(option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_rfq(option_rfq_id)
        .ok_or(BackendError::InvalidOptionRfqId)
}

pub async fn submit_option_rfq_quote(
    state: &AppState,
    option_rfq_id: OptionRfqId,
    input: SubmitOptionRfqQuoteInput,
) -> Result<OptionRfqQuote> {
    ensure_option_rfq_enabled(state)?;
    validate_account(&input.mm_account)?;
    if input.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }

    let now = now_ms();
    let rfq = get_option_rfq(state, option_rfq_id).await?;
    if rfq.effective_status(now) != OptionRfqStatus::Open {
        return Err(BackendError::InvalidOptionRfqState(
            "option RFQ is not open".to_string(),
        ));
    }
    if input.size_1e8 > rfq.size_1e8 {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote size exceeds requested size".to_string(),
        ));
    }
    let existing_quote_count = count_option_rfq_quotes(state, option_rfq_id).await?;
    if existing_quote_count >= state.options_config.rfq_max_quotes_per_rfq {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote limit reached".to_string(),
        ));
    }

    let quote_ttl_ms = input
        .quote_ttl_ms
        .unwrap_or(state.options_config.rfq_max_quote_ttl_ms)
        .min(state.options_config.rfq_max_quote_ttl_ms);
    if quote_ttl_ms < state.options_config.rfq_min_quote_ttl_ms {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote_ttl_ms is below the minimum".to_string(),
        ));
    }
    let expires_at_ms =
        checked_expiry(now, quote_ttl_ms, "option RFQ quote expiry")?.min(rfq.expires_at_ms);

    let quote = OptionRfqQuote {
        quote_id: Uuid::new_v4(),
        option_rfq_id,
        mm_account: input.mm_account,
        session_id: input.session_id,
        client_quote_id: input.client_quote_id,
        price_1e8: input.price_1e8,
        size_1e8: input.size_1e8,
        status: OptionRfqQuoteStatus::Active,
        created_at_ms: now,
        expires_at_ms,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_option_rfq_quote(&quote).await?;
        return repository
            .get_option_rfq_quote(quote.quote_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqQuoteId);
    }

    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_rfq_quote(quote)
}

pub async fn list_option_rfq_quotes(
    state: &AppState,
    option_rfq_id: OptionRfqId,
) -> Result<Vec<OptionRfqQuote>> {
    ensure_option_rfq_enabled(state)?;
    let _ = get_option_rfq(state, option_rfq_id).await?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_option_rfq_quotes(option_rfq_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_rfq_quotes(option_rfq_id))
}

pub async fn accept_option_rfq_quote(
    state: &AppState,
    option_rfq_id: OptionRfqId,
    quote_id: OptionRfqQuoteId,
) -> Result<AcceptOptionRfqQuoteOutcome> {
    ensure_option_rfq_enabled(state)?;
    let now = now_ms();
    let rfq = get_option_rfq(state, option_rfq_id).await?;
    if rfq.effective_status(now) != OptionRfqStatus::Open {
        return Err(BackendError::InvalidOptionRfqState(
            "option RFQ is not open".to_string(),
        ));
    }
    let quote = get_option_rfq_quote(state, quote_id).await?;
    if quote.option_rfq_id != option_rfq_id {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote does not belong to RFQ".to_string(),
        ));
    }
    if quote.effective_status(now) != OptionRfqQuoteStatus::Active {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote is not active".to_string(),
        ));
    }
    if quote.size_1e8 == 0 || quote.size_1e8 > rfq.size_1e8 {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote size is invalid".to_string(),
        ));
    }
    if !option_rfq_price_satisfies_limit(rfq.side, rfq.limit_price_1e8, quote.price_1e8) {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote price violates limit".to_string(),
        ));
    }

    let series = get_option_series(state, &rfq.option_series_id).await?;
    if series.effective_status(now_sec(now)?) != OptionSeriesStatus::Active {
        return Err(BackendError::InvalidOptionRfqState(
            "option series is not active".to_string(),
        ));
    }

    let (buyer, seller) = match rfq.side {
        Side::Buy => (rfq.taker.clone(), quote.mm_account.clone()),
        Side::Sell => (quote.mm_account.clone(), rfq.taker.clone()),
    };
    let fill = OptionRfqFill {
        fill_id: Uuid::new_v4(),
        option_rfq_id,
        quote_id,
        option_series_id: rfq.option_series_id.clone(),
        buyer,
        seller,
        taker: rfq.taker.clone(),
        mm_account: quote.mm_account.clone(),
        taker_side: rfq.side,
        price_1e8: quote.price_1e8,
        size_1e8: quote.size_1e8,
        created_at_ms: now,
    };

    if let Some(repository) = state.repository.clone() {
        repository
            .accept_option_rfq_quote_and_insert_fill(option_rfq_id, quote_id, &fill)
            .await?;
        let rfq = repository
            .get_option_rfq(option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId)?;
        let quote = repository
            .get_option_rfq_quote(quote_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqQuoteId)?;
        let fill = repository.get_option_rfq_fill(fill.fill_id).await?.ok_or(
            BackendError::InvalidOptionRfqState("option RFQ fill was not persisted".to_string()),
        )?;
        return Ok(AcceptOptionRfqQuoteOutcome { rfq, quote, fill });
    }

    let (rfq, quote) = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .accept_option_rfq_quote(option_rfq_id, quote_id, fill.clone())?;
    Ok(AcceptOptionRfqQuoteOutcome { rfq, quote, fill })
}

pub async fn cancel_option_rfq(
    state: &AppState,
    option_rfq_id: OptionRfqId,
) -> Result<OptionRfqRequest> {
    ensure_option_rfq_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository.cancel_option_rfq(option_rfq_id).await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .cancel_option_rfq(option_rfq_id)
}

async fn open_option_orders_for_series(
    state: &AppState,
    option_series_id: &str,
) -> Result<Vec<OptionOrder>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .open_option_orders_for_series(option_series_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .open_orders_for_series(option_series_id))
}

async fn count_option_rfq_quotes(state: &AppState, option_rfq_id: OptionRfqId) -> Result<usize> {
    if let Some(repository) = state.repository.clone() {
        return repository.count_option_rfq_quotes(option_rfq_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .count_option_rfq_quotes(option_rfq_id))
}

async fn get_option_rfq_quote(
    state: &AppState,
    quote_id: OptionRfqQuoteId,
) -> Result<OptionRfqQuote> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_rfq_quote(quote_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqQuoteId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_rfq_quote(quote_id)
        .ok_or(BackendError::InvalidOptionRfqQuoteId)
}

async fn get_option_series_optional(
    state: &AppState,
    option_series_id: &str,
) -> Result<Option<OptionSeries>> {
    if let Some(repository) = state.repository.clone() {
        return repository.get_option_series(option_series_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_series(option_series_id))
}

fn ensure_enabled(state: &AppState) -> Result<()> {
    if state.options_config.enabled {
        Ok(())
    } else {
        Err(BackendError::OptionsDisabled)
    }
}

fn ensure_option_rfq_enabled(state: &AppState) -> Result<()> {
    ensure_enabled(state)?;
    if state.options_config.rfq_enabled {
        Ok(())
    } else {
        Err(BackendError::OptionRfqDisabled)
    }
}

fn validate_account(account: &AccountId) -> Result<()> {
    parse_evm_address(account).map(|_| ())
}

fn validate_assets(fields: &[(&str, &str)]) -> Result<()> {
    for (field, value) in fields {
        if value.trim().is_empty() {
            return Err(BackendError::InvalidOptionSeriesState(format!(
                "{field} must be non-empty"
            )));
        }
    }
    Ok(())
}

fn trim_asset(value: String) -> String {
    value.trim().to_string()
}

fn now_sec(now_ms: TimestampMs) -> Result<u64> {
    u64::try_from(now_ms / 1000)
        .map_err(|_| BackendError::Config("current timestamp cannot be encoded".to_string()))
}

fn checked_expiry(now: TimestampMs, ttl_ms: u64, context: &str) -> Result<TimestampMs> {
    let ttl_ms = i64::try_from(ttl_ms)
        .map_err(|_| BackendError::Config(format!("{context} ttl cannot be encoded")))?;
    now.checked_add(ttl_ms)
        .ok_or_else(|| BackendError::Config(format!("{context} overflow")))
}

fn option_rfq_price_satisfies_limit(
    side: Side,
    limit_price_1e8: Option<Price1e8>,
    price_1e8: Price1e8,
) -> bool {
    match (side, limit_price_1e8) {
        (_, None) => true,
        (Side::Buy, Some(limit)) => price_1e8 <= limit,
        (Side::Sell, Some(limit)) => price_1e8 >= limit,
    }
}

fn aggregate_levels(orders: &[OptionOrder], side: Side) -> Vec<OptionOrderbookLevel> {
    let mut by_price = BTreeMap::<Price1e8, Size1e8>::new();
    for order in orders {
        if order.side == side && order.status.is_live() {
            *by_price.entry(order.price_1e8).or_default() += order.remaining_size_1e8;
        }
    }

    let iter: Box<dyn Iterator<Item = (Price1e8, Size1e8)>> = match side {
        Side::Buy => Box::new(by_price.into_iter().rev()),
        Side::Sell => Box::new(by_price.into_iter()),
    };

    iter.map(|(price_1e8, size_1e8)| OptionOrderbookLevel {
        price_1e8: price_1e8.to_string(),
        size_1e8: size_1e8.to_string(),
    })
    .collect()
}
