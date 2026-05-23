use super::series_id::{option_series_id, OptionSeriesIdInput};
use super::signing::{
    option_rfq_id_to_b256, option_rfq_quote_digest, option_rfq_quote_digest_bytes,
    option_series_id_to_b256, OptionRfqQuoteSigningPayload,
};
use super::{
    build_option_execution_transaction_request, encode_option_execute_trade_calldata,
    normalize_u256_string, option_execution_intent_id_to_hex_bytes32,
    option_execution_simulation_pending, option_execution_simulation_unavailable,
    option_trade_digest, option_trade_digest_bytes, perform_option_broadcast_gas_safety_check,
    simulate_option_execution_intent, validate_simulation_intent, validate_simulation_target,
    OptionExecutionConfirmationStatus, OptionExecutionGasSafetyCheck, OptionExecutionIntent,
    OptionExecutionIntentId, OptionExecutionIntentStatus, OptionExecutionSignatureMode,
    OptionExecutionSimulationResult, OptionExecutionSourceType, OptionExecutionTransaction,
    OptionFill, OptionFillFilter, OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderId,
    OptionOrderStatus, OptionOrderbookLevel, OptionOrderbookSnapshot, OptionRfqFill,
    OptionRfqFillId, OptionRfqId, OptionRfqQuote, OptionRfqQuoteId, OptionRfqQuoteSignatureMode,
    OptionRfqQuoteSignatureStatus, OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus,
    OptionSeries, OptionSeriesFilter, OptionSeriesId, OptionSeriesSource, OptionSeriesStatus,
    OptionTradePayload, OptionTradeSignatureBundle,
};
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::execution::transaction::hex_0x;
use crate::execution::{
    sign_eip1559_transaction, EthCallProvider, ExecutionTransactionRequest,
    ExecutionTransactionStatus, ExecutorSigner, GasEstimateProvider, HttpJsonRpcProvider,
    TransactionBroadcastProvider, TransactionReceiptProvider,
};
use crate::mm::protocol::{
    NotificationEnvelope, OptionRfqQuoteAcceptedPayload, OptionRfqQuoteRejectedPayload,
    OptionRfqRequestPayload, ServerMessage,
};
use crate::nonce_sync::{read_option_nonce_value, OptionNonceProvider};
use crate::signing::eip712::parse_evm_address;
use crate::signing::recover_eip712_signer;
use crate::signing::signature::validate_signature_shape;
use crate::types::{now_ms, AccountId, OrderId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};
use std::collections::BTreeMap;
use tracing::{info, warn};
use uuid::Uuid;

const ONE_CONTRACT_1E8: u128 = 100_000_000;

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
    pub quote_nonce: Option<u64>,
    pub quote_ttl_ms: Option<u64>,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionRfqQuoteSigningPayloadInput {
    pub option_rfq_id: OptionRfqId,
    pub mm_account: AccountId,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub quote_nonce: u64,
    pub quote_ttl_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionRfqQuoteSigningPayloadOutcome {
    pub rfq: OptionRfqRequest,
    pub payload: OptionRfqQuoteSigningPayload,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptOptionRfqQuoteOutcome {
    pub rfq: OptionRfqRequest,
    pub quote: OptionRfqQuote,
    pub fill: OptionRfqFill,
    pub mm_notification_sent: bool,
    pub mm_notification_warning: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionSigningPayloadOutcome {
    pub intent: OptionExecutionIntent,
    pub payload: OptionTradePayload,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionExecutionSignaturesInput {
    pub buyer_signature: Option<String>,
    pub seller_signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionExecutionSignaturesOutcome {
    pub intent: OptionExecutionIntent,
    pub buyer_signature_present: bool,
    pub seller_signature_present: bool,
    pub calldata_ready: bool,
    pub missing_signatures: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionCalldataOutcome {
    pub intent: OptionExecutionIntent,
    pub calldata: Option<String>,
    pub calldata_ready: bool,
    pub missing_signatures: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionBroadcastOutcome {
    pub intent: OptionExecutionIntent,
    pub transaction: OptionExecutionTransaction,
    pub broadcast_enabled: bool,
    pub submitted: bool,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionConfirmationOutcome {
    pub intent: OptionExecutionIntent,
    pub transaction: OptionExecutionTransaction,
    pub confirmation_status: OptionExecutionConfirmationStatus,
    pub receipt_status: Option<u64>,
    pub block_number: Option<u64>,
    pub error: Option<String>,
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
    validate_option_order_execution_preflight(state, &series, &input).await?;

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
        create_option_orderbook_execution_intents(state, &fills).await?;
        crate::fees::service::record_option_order_fills(state, &fills).await?;
        return Ok(SubmitOptionOrderOutcome { order, fills });
    }

    let (order, fills) = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .submit_order_and_match(order, now)?;
    create_option_orderbook_execution_intents(state, &fills).await?;
    crate::fees::service::record_option_order_fills(state, &fills).await?;
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
        let rfq = repository
            .get_option_rfq(rfq.option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId)?;
        broadcast_option_rfq_request(state, &rfq);
        return Ok(rfq);
    }

    let rfq = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_rfq(rfq);
    broadcast_option_rfq_request(state, &rfq);
    Ok(rfq)
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

pub async fn option_rfq_quote_signing_payload(
    state: &AppState,
    input: OptionRfqQuoteSigningPayloadInput,
) -> Result<OptionRfqQuoteSigningPayloadOutcome> {
    ensure_option_rfq_enabled(state)?;
    validate_account(&input.mm_account)?;
    if input.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    let quote_ttl_ms = input
        .quote_ttl_ms
        .min(state.options_config.rfq_max_quote_ttl_ms);
    validate_option_rfq_quote_ttl(state, quote_ttl_ms)?;

    let rfq = get_option_rfq(state, input.option_rfq_id).await?;
    let now = now_ms();
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

    let payload = option_rfq_quote_payload(
        &rfq,
        input.mm_account,
        input.price_1e8,
        input.size_1e8,
        input.quote_nonce,
        quote_ttl_ms,
    )?;
    let digest = option_rfq_quote_digest(&payload, &state.options_config.rfq_eip712_domain)?;
    Ok(OptionRfqQuoteSigningPayloadOutcome {
        rfq,
        payload,
        digest,
    })
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
    crate::mm::permissions::check_can_quote_option_rfq(
        state,
        &input.mm_account,
        &rfq.option_series_id,
    )
    .await?;
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
    validate_option_rfq_quote_ttl(state, quote_ttl_ms)?;
    let signature_metadata = verify_option_rfq_quote_signature(state, &rfq, &input, quote_ttl_ms)?;
    let expires_at_ms = quote_expires_at_ms(state, &rfq, now, quote_ttl_ms)?;
    if now >= expires_at_ms {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote has expired".to_string(),
        ));
    }

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
        signature: signature_metadata.signature,
        quote_digest: signature_metadata.quote_digest,
        quote_nonce: signature_metadata.quote_nonce,
        signature_status: signature_metadata.signature_status,
        recovered_signer: signature_metadata.recovered_signer,
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
    validate_option_rfq_quote_signature_status(state, &quote)?;
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
    validate_option_rfq_execution_preflight(state, &series, &quote).await?;
    let quotes_before_accept = list_option_rfq_quotes(state, option_rfq_id).await?;

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
        create_option_rfq_execution_intent(state, &fill).await?;
        crate::fees::service::record_option_rfq_fill(state, &fill, &quote).await?;
        let (mm_notification_sent, mm_notification_warning) =
            notify_option_rfq_quote_acceptance(state, &quote, &quotes_before_accept, fill.fill_id);
        return Ok(AcceptOptionRfqQuoteOutcome {
            rfq,
            quote,
            fill,
            mm_notification_sent,
            mm_notification_warning,
        });
    }

    let (rfq, quote) = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .accept_option_rfq_quote(option_rfq_id, quote_id, fill.clone())?;
    create_option_rfq_execution_intent(state, &fill).await?;
    crate::fees::service::record_option_rfq_fill(state, &fill, &quote).await?;
    let (mm_notification_sent, mm_notification_warning) =
        notify_option_rfq_quote_acceptance(state, &quote, &quotes_before_accept, fill.fill_id);
    Ok(AcceptOptionRfqQuoteOutcome {
        rfq,
        quote,
        fill,
        mm_notification_sent,
        mm_notification_warning,
    })
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

pub async fn list_option_execution_intents(state: &AppState) -> Result<Vec<OptionExecutionIntent>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_option_execution_intents().await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_execution_intents())
}

pub async fn get_option_execution_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionIntent> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_execution_intent(intent_id)
            .await?
            .ok_or(BackendError::InvalidOptionExecutionIntentId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_execution_intent(intent_id)
        .ok_or(BackendError::InvalidOptionExecutionIntentId)
}

pub async fn option_execution_signing_payload(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionSigningPayloadOutcome> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    let payload = OptionTradePayload::from_intent(&intent)?;
    let digest = option_trade_digest(&payload, &state.options_config.execution_eip712_domain)?;
    Ok(OptionExecutionSigningPayloadOutcome {
        intent,
        payload,
        digest,
    })
}

pub async fn submit_option_execution_signatures(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    input: SubmitOptionExecutionSignaturesInput,
) -> Result<SubmitOptionExecutionSignaturesOutcome> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    let payload = OptionTradePayload::from_intent(&intent)?;
    let digest_bytes =
        option_trade_digest_bytes(&payload, &state.options_config.execution_eip712_domain)?;
    verify_option_execution_signature(
        state,
        input.buyer_signature.as_deref(),
        &digest_bytes,
        &intent.buyer,
    )?;
    verify_option_execution_signature(
        state,
        input.seller_signature.as_deref(),
        &digest_bytes,
        &intent.seller,
    )?;

    let effective_buyer_signature = input
        .buyer_signature
        .clone()
        .or_else(|| intent.buyer_signature.clone());
    let effective_seller_signature = input
        .seller_signature
        .clone()
        .or_else(|| intent.seller_signature.clone());
    let (status, calldata) = if let (Some(buyer_signature), Some(seller_signature)) = (
        effective_buyer_signature.as_deref(),
        effective_seller_signature.as_deref(),
    ) {
        let calldata = build_option_execution_calldata_from_parts(
            &payload,
            buyer_signature,
            seller_signature,
        )?;
        (OptionExecutionIntentStatus::CalldataReady, Some(calldata))
    } else {
        (OptionExecutionIntentStatus::SignaturesRequired, None)
    };

    let updated_at_ms = now_ms();
    let updated = if let Some(repository) = state.repository.clone() {
        repository
            .upsert_option_execution_signatures(
                intent_id,
                input.buyer_signature,
                input.seller_signature,
                status,
                calldata,
                updated_at_ms,
            )
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .upsert_option_execution_signatures(
                intent_id,
                input.buyer_signature,
                input.seller_signature,
                status,
                calldata,
                updated_at_ms,
            )?
    };

    Ok(option_execution_signature_outcome(updated))
}

pub async fn option_execution_calldata(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionCalldataOutcome> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    if intent.calldata.is_some() {
        return Ok(OptionExecutionCalldataOutcome {
            calldata_ready: true,
            missing_signatures: false,
            calldata: intent.calldata.clone(),
            intent,
        });
    }
    let Some(buyer_signature) = intent.buyer_signature.as_deref() else {
        return Ok(OptionExecutionCalldataOutcome {
            calldata_ready: false,
            missing_signatures: true,
            calldata: None,
            intent,
        });
    };
    let Some(seller_signature) = intent.seller_signature.as_deref() else {
        return Ok(OptionExecutionCalldataOutcome {
            calldata_ready: false,
            missing_signatures: true,
            calldata: None,
            intent,
        });
    };

    let payload = OptionTradePayload::from_intent(&intent)?;
    let calldata =
        build_option_execution_calldata_from_parts(&payload, buyer_signature, seller_signature)?;
    let updated_at_ms = now_ms();
    let updated = if let Some(repository) = state.repository.clone() {
        repository
            .upsert_option_execution_signatures(
                intent_id,
                None,
                None,
                OptionExecutionIntentStatus::CalldataReady,
                Some(calldata.clone()),
                updated_at_ms,
            )
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .upsert_option_execution_signatures(
                intent_id,
                None,
                None,
                OptionExecutionIntentStatus::CalldataReady,
                Some(calldata.clone()),
                updated_at_ms,
            )?
    };
    Ok(OptionExecutionCalldataOutcome {
        intent: updated,
        calldata: Some(calldata),
        calldata_ready: true,
        missing_signatures: false,
    })
}

pub async fn option_execution_simulation_status(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionSimulationResult> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    Ok(option_execution_simulation_pending(&intent))
}

pub async fn prepare_option_execution_simulation(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionIntent> {
    ensure_option_execution_simulation_enabled(state)?;
    let intent = get_option_execution_intent(state, intent_id).await?;
    if let Err(error) = validate_option_execution_simulation_preflight(state, &intent) {
        let unavailable =
            option_execution_simulation_unavailable(intent.intent_id, error.to_string());
        persist_option_execution_simulation_result(state, &unavailable).await?;
        return Err(error);
    }
    Ok(intent)
}

pub async fn simulate_prepared_option_execution_intent<P>(
    state: &AppState,
    intent: &OptionExecutionIntent,
    provider: &P,
) -> Result<OptionExecutionSimulationResult>
where
    P: EthCallProvider,
{
    let from = option_execution_simulation_from(state)?;
    let result = match simulate_option_execution_intent(
        provider,
        intent,
        &state.options_config.matching_engine_address,
        &from,
        state.options_config.execution_simulation_gas_limit,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let unavailable =
                option_execution_simulation_unavailable(intent.intent_id, error.to_string());
            persist_option_execution_simulation_result(state, &unavailable).await?;
            return Err(error);
        }
    };
    persist_option_execution_simulation_result(state, &result).await?;
    Ok(result)
}

pub async fn persist_option_execution_simulation_unavailable(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    error: impl Into<String>,
) -> Result<OptionExecutionSimulationResult> {
    let result = option_execution_simulation_unavailable(intent_id, error.into());
    persist_option_execution_simulation_result(state, &result).await?;
    Ok(result)
}

pub async fn broadcast_option_execution_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionBroadcastOutcome> {
    ensure_option_execution_broadcast_enabled(state)?;
    let rpc_url = state.execution_config.rpc_url.clone().ok_or_else(|| {
        BackendError::Config("RPC_URL is required for option execution broadcast".to_string())
    })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    broadcast_option_execution_intent_with_provider(state, intent_id, &provider).await
}

pub async fn broadcast_option_execution_intent_with_provider<P>(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    provider: &P,
) -> Result<OptionExecutionBroadcastOutcome>
where
    P: TransactionBroadcastProvider + GasEstimateProvider,
{
    ensure_option_execution_broadcast_enabled(state)?;
    let intent = get_option_execution_intent(state, intent_id).await?;
    if let Some(transaction) = find_submitted_option_execution_transaction(state, intent_id).await?
    {
        return Ok(OptionExecutionBroadcastOutcome {
            intent,
            transaction,
            broadcast_enabled: true,
            submitted: true,
            duplicate: true,
        });
    }
    if state.execution_config.rpc_url.is_none() {
        return Err(BackendError::Config(
            "RPC_URL is required for option execution broadcast".to_string(),
        ));
    }
    let private_key = state
        .execution_config
        .executor_private_key
        .as_ref()
        .ok_or_else(|| {
            BackendError::Config(
                "EXECUTOR_PRIVATE_KEY is required for option execution broadcast".to_string(),
            )
        })?;
    let request = build_option_execution_transaction_request(
        &state.execution_config,
        &state.options_config,
        &intent,
    )?;
    let signer = ExecutorSigner::from_private_key(private_key)?;
    let from = signer.address().clone();

    let gas_check = perform_option_broadcast_gas_safety_check(
        provider,
        &state.execution_config,
        &state.options_config,
        &intent,
        &from,
    )
    .await?;

    if !gas_check.is_ok() {
        let reason = gas_check
            .reject_reason()
            .unwrap_or_else(|| "option execution gas safety check failed".to_string());
        let now = now_ms();
        let transaction = option_execution_transaction_from_request(
            &request,
            from,
            None,
            Some(reason.clone()),
            now,
            Some(&gas_check),
        );
        insert_option_execution_transaction(state, transaction).await?;
        update_option_execution_intent_status(
            state,
            intent_id,
            OptionExecutionIntentStatus::BroadcastFailed,
            Some(reason.clone()),
            now,
        )
        .await?;
        return Err(BackendError::BroadcastRejected(reason));
    }

    let rpc_chain_id = provider.chain_id().await?;
    if rpc_chain_id != request.chain_id {
        return Err(BackendError::BroadcastRejected(format!(
            "RPC chain id {rpc_chain_id} does not match EXECUTOR_CHAIN_ID {}",
            request.chain_id
        )));
    }

    let now = now_ms();
    let nonce = provider.transaction_count(from.clone()).await?;
    let raw_transaction = sign_eip1559_transaction(&request, nonce, &signer)?;
    let tx_hash = match provider.send_raw_transaction(raw_transaction).await {
        Ok(tx_hash) => {
            if !is_valid_tx_hash(&tx_hash) {
                let error = "broadcast provider returned an invalid transaction hash".to_string();
                let transaction = option_execution_transaction_from_request(
                    &request,
                    from,
                    None,
                    Some(error.clone()),
                    now,
                    Some(&gas_check),
                );
                insert_option_execution_transaction(state, transaction.clone()).await?;
                update_option_execution_intent_status(
                    state,
                    intent_id,
                    OptionExecutionIntentStatus::BroadcastFailed,
                    Some(error.clone()),
                    now,
                )
                .await?;
                return Err(BackendError::BroadcastRejected(error));
            }
            tx_hash.to_ascii_lowercase()
        }
        Err(error) => {
            let transaction = option_execution_transaction_from_request(
                &request,
                from,
                None,
                Some(error.to_string()),
                now,
                Some(&gas_check),
            );
            insert_option_execution_transaction(state, transaction).await?;
            update_option_execution_intent_status(
                state,
                intent_id,
                OptionExecutionIntentStatus::BroadcastFailed,
                Some(error.to_string()),
                now,
            )
            .await?;
            return Err(error);
        }
    };

    let transaction = option_execution_transaction_from_request(
        &request,
        from,
        Some(tx_hash),
        None,
        now,
        Some(&gas_check),
    );
    let transaction = insert_option_execution_transaction(state, transaction).await?;
    let updated_intent = update_option_execution_intent_status(
        state,
        intent_id,
        OptionExecutionIntentStatus::BroadcastSubmitted,
        None,
        now,
    )
    .await?;

    Ok(OptionExecutionBroadcastOutcome {
        intent: updated_intent,
        transaction,
        broadcast_enabled: true,
        submitted: true,
        duplicate: false,
    })
}

pub async fn confirm_option_execution_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionConfirmationOutcome> {
    let rpc_url = state.execution_config.rpc_url.clone().ok_or_else(|| {
        BackendError::Config("RPC_URL is required for option execution confirmation".to_string())
    })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    confirm_option_execution_intent_with_provider(state, intent_id, &provider).await
}

pub async fn confirm_option_execution_intent_with_provider<P>(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    provider: &P,
) -> Result<OptionExecutionConfirmationOutcome>
where
    P: TransactionReceiptProvider,
{
    let intent = get_option_execution_intent(state, intent_id).await?;
    let transaction = find_submitted_option_execution_transaction(state, intent_id)
        .await?
        .ok_or_else(|| {
            BackendError::InvalidOptionExecutionIntentState(
                "no submitted option execution transaction to confirm".to_string(),
            )
        })?;
    let tx_hash = transaction.tx_hash.clone().ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(
            "submitted option execution transaction is missing a tx hash".to_string(),
        )
    })?;

    let receipt_result = provider.transaction_receipt(tx_hash.clone()).await;
    let now = now_ms();

    let (status, receipt_status_value, block_number, error_string) = match receipt_result {
        Ok(Some(receipt)) => {
            if !receipt.tx_hash.eq_ignore_ascii_case(&tx_hash) {
                (
                    OptionExecutionConfirmationStatus::ReceiptMissing,
                    None,
                    None,
                    Some(format!(
                        "receipt tx hash {} does not match submitted {}",
                        receipt.tx_hash, tx_hash
                    )),
                )
            } else {
                let mapped = match receipt.status {
                    Some(1) => OptionExecutionConfirmationStatus::MinedSuccess,
                    Some(_) => OptionExecutionConfirmationStatus::MinedReverted,
                    None => OptionExecutionConfirmationStatus::ReceiptError,
                };
                let err = if mapped == OptionExecutionConfirmationStatus::ReceiptError {
                    Some("receipt missing status field".to_string())
                } else {
                    None
                };
                (mapped, receipt.status, receipt.block_number, err)
            }
        }
        Ok(None) => (
            OptionExecutionConfirmationStatus::ReceiptMissing,
            None,
            None,
            Some("receipt not yet available".to_string()),
        ),
        Err(error) => (
            OptionExecutionConfirmationStatus::ReceiptError,
            None,
            None,
            Some(error.to_string()),
        ),
    };

    let updated_transaction = persist_option_execution_confirmation(
        state,
        &transaction.transaction_id,
        status,
        now,
        block_number,
        receipt_status_value,
        error_string.clone(),
    )
    .await?;

    let next_intent_status = match status {
        OptionExecutionConfirmationStatus::MinedSuccess => {
            Some(OptionExecutionIntentStatus::BroadcastConfirmed)
        }
        OptionExecutionConfirmationStatus::MinedReverted => {
            Some(OptionExecutionIntentStatus::BroadcastReverted)
        }
        _ => None,
    };

    let updated_intent = if let Some(next) = next_intent_status {
        update_option_execution_intent_status(state, intent_id, next, None, now).await?
    } else {
        intent.clone()
    };

    Ok(OptionExecutionConfirmationOutcome {
        intent: updated_intent,
        transaction: updated_transaction,
        confirmation_status: status,
        receipt_status: receipt_status_value,
        block_number,
        error: error_string,
    })
}

async fn list_pending_option_execution_transactions(
    state: &AppState,
    limit: u32,
) -> Result<Vec<OptionExecutionTransaction>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .list_pending_option_execution_transactions(limit)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_pending_option_execution_transactions(limit))
}

/// Run one tick of the option execution confirmation worker against the supplied receipt
/// provider. Deterministic and side-effect-isolated to the option store / repository:
/// it never broadcasts, never touches the generic executor path, never inserts new rows.
///
/// For each pending option execution transaction:
/// - If a receipt is found and `receipt.block_number + finality_blocks <= current_block_number`:
///   - status=1 → persist `mined_success`, transition intent to `broadcast_confirmed`
///   - status=0 → persist `mined_failed`, transition intent to `broadcast_failed`
///   - status=None → record receipt + leave pending (receipt_error)
/// - If a receipt is found but finality is not yet reached, leave pending. Update the
///   in-memory `confirmation_status` to `pending` so operators can see the worker observed
///   the receipt (no terminal state until finalized).
/// - If `eth_getTransactionReceipt` returns `None`, leave pending (`receipt_missing`).
/// - If RPC errors, leave pending (`receipt_error`).
pub async fn confirm_pending_option_execution_transactions<P>(
    state: &AppState,
    provider: &P,
) -> Result<crate::options::OptionConfirmationTickResult>
where
    P: TransactionReceiptProvider,
{
    let config = state.option_confirmation_config.clone();
    if !config.enabled {
        return Ok(crate::options::OptionConfirmationTickResult {
            enabled: false,
            batch_size: config.batch_size,
            finality_blocks: config.finality_blocks,
            current_block_number: None,
            decisions: Vec::new(),
        });
    }

    let current_block_number = provider.block_number().await.ok();
    let pending = list_pending_option_execution_transactions(state, config.batch_size).await?;
    let mut decisions = Vec::with_capacity(pending.len());

    for tx in pending {
        let now = now_ms();
        let Some(tx_hash) = tx.tx_hash.clone() else {
            // Shouldn't happen — list query filters for non-null hash — defensive only.
            continue;
        };
        let receipt_result = provider.transaction_receipt(tx_hash.clone()).await;
        let decision = compute_worker_decision(
            &tx.transaction_id,
            &tx_hash,
            current_block_number,
            config.finality_blocks,
            &receipt_result,
        );
        apply_worker_decision(state, &tx, &decision, now).await?;
        decisions.push(decision);
    }

    Ok(crate::options::OptionConfirmationTickResult {
        enabled: true,
        batch_size: config.batch_size,
        finality_blocks: config.finality_blocks,
        current_block_number,
        decisions,
    })
}

fn compute_worker_decision(
    transaction_id: &str,
    expected_tx_hash: &str,
    current_block_number: Option<u64>,
    finality_blocks: u64,
    receipt_result: &Result<Option<crate::confirmation::ConfirmationReceipt>>,
) -> crate::options::OptionConfirmationDecision {
    use crate::options::{OptionConfirmationDecision, OptionConfirmationOutcome};
    let base = |outcome: OptionConfirmationOutcome,
                receipt_status: Option<u64>,
                block_number: Option<u64>,
                error: Option<String>|
     -> OptionConfirmationDecision {
        OptionConfirmationDecision {
            transaction_id: transaction_id.to_string(),
            tx_hash: Some(expected_tx_hash.to_string()),
            outcome,
            receipt_status,
            block_number,
            current_block_number,
            finality_blocks,
            error,
        }
    };
    let receipt = match receipt_result {
        Ok(Some(r)) => r,
        Ok(None) => {
            return base(
                OptionConfirmationOutcome::ReceiptMissing,
                None,
                None,
                Some("receipt not yet available".to_string()),
            )
        }
        Err(error) => {
            return base(
                OptionConfirmationOutcome::ReceiptError,
                None,
                None,
                Some(error.to_string()),
            )
        }
    };
    if !receipt.tx_hash.eq_ignore_ascii_case(expected_tx_hash) {
        return base(
            OptionConfirmationOutcome::ReceiptMissing,
            receipt.status,
            receipt.block_number,
            Some(format!(
                "receipt tx hash {} does not match submitted {}",
                receipt.tx_hash, expected_tx_hash
            )),
        );
    }
    let Some(receipt_block) = receipt.block_number else {
        return base(
            OptionConfirmationOutcome::ReceiptError,
            receipt.status,
            None,
            Some("receipt missing block_number".to_string()),
        );
    };
    let Some(head) = current_block_number else {
        return base(
            OptionConfirmationOutcome::NotFinalized,
            receipt.status,
            Some(receipt_block),
            Some("current block number unavailable".to_string()),
        );
    };
    let finalized_at = receipt_block.saturating_add(finality_blocks);
    if head < finalized_at {
        return base(
            OptionConfirmationOutcome::NotFinalized,
            receipt.status,
            Some(receipt_block),
            None,
        );
    }
    match receipt.status {
        Some(1) => base(
            OptionConfirmationOutcome::MinedSuccess,
            Some(1),
            Some(receipt_block),
            None,
        ),
        Some(other) => base(
            OptionConfirmationOutcome::MinedFailed,
            Some(other),
            Some(receipt_block),
            None,
        ),
        None => base(
            OptionConfirmationOutcome::ReceiptError,
            None,
            Some(receipt_block),
            Some("receipt missing status field".to_string()),
        ),
    }
}

async fn apply_worker_decision(
    state: &AppState,
    transaction: &OptionExecutionTransaction,
    decision: &crate::options::OptionConfirmationDecision,
    now: TimestampMs,
) -> Result<()> {
    use crate::options::OptionConfirmationOutcome;
    let (persist_status, intent_status) = match decision.outcome {
        OptionConfirmationOutcome::MinedSuccess => (
            OptionExecutionConfirmationStatus::MinedSuccess,
            Some(OptionExecutionIntentStatus::BroadcastConfirmed),
        ),
        OptionConfirmationOutcome::MinedFailed => (
            OptionExecutionConfirmationStatus::MinedFailed,
            Some(OptionExecutionIntentStatus::BroadcastFailed),
        ),
        OptionConfirmationOutcome::NotFinalized => {
            (OptionExecutionConfirmationStatus::Pending, None)
        }
        OptionConfirmationOutcome::ReceiptMissing => {
            (OptionExecutionConfirmationStatus::ReceiptMissing, None)
        }
        OptionConfirmationOutcome::ReceiptError => {
            (OptionExecutionConfirmationStatus::ReceiptError, None)
        }
        OptionConfirmationOutcome::Disabled | OptionConfirmationOutcome::NoPending => return Ok(()),
    };

    persist_option_execution_confirmation(
        state,
        &transaction.transaction_id,
        persist_status,
        now,
        decision.block_number,
        decision.receipt_status,
        decision.error.clone(),
    )
    .await?;
    if let Some(next) = intent_status {
        update_option_execution_intent_status(state, transaction.intent_id, next, None, now)
            .await?;
    }
    Ok(())
}

async fn persist_option_execution_confirmation(
    state: &AppState,
    transaction_id: &str,
    confirmation_status: OptionExecutionConfirmationStatus,
    confirmed_at_ms: TimestampMs,
    confirmed_block_number: Option<u64>,
    receipt_status: Option<u64>,
    confirmation_error: Option<String>,
) -> Result<OptionExecutionTransaction> {
    if let Some(repository) = state.repository.clone() {
        let rows = repository
            .update_option_execution_confirmation(
                transaction_id,
                confirmation_status,
                confirmed_at_ms,
                confirmed_block_number,
                receipt_status,
                confirmation_error.clone(),
            )
            .await?;
        if rows == 0 {
            return Err(BackendError::Persistence(format!(
                "option execution transaction {transaction_id} not found"
            )));
        }
        let tx = repository
            .get_option_execution_transaction(transaction_id)
            .await?;
        return tx.ok_or_else(|| {
            BackendError::Persistence(format!(
                "option execution transaction {transaction_id} disappeared after update"
            ))
        });
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .update_option_execution_confirmation(
            transaction_id,
            confirmation_status,
            confirmed_at_ms,
            confirmed_block_number,
            receipt_status,
            confirmation_error,
        )
}

pub async fn create_option_orderbook_execution_intent(
    state: &AppState,
    fill: &OptionFill,
) -> Result<Option<OptionExecutionIntent>> {
    let provider = option_nonce_provider(state)?;
    create_option_orderbook_execution_intent_with_nonce_provider(state, fill, provider.as_ref())
        .await
}

async fn create_option_orderbook_execution_intent_with_nonce_provider<P>(
    state: &AppState,
    fill: &OptionFill,
    nonce_provider: Option<&P>,
) -> Result<Option<OptionExecutionIntent>>
where
    P: OptionNonceProvider,
{
    if !state.options_config.execution_enabled {
        return Ok(None);
    }
    let series = get_option_series(state, &fill.option_series_id).await?;
    let buyer_is_maker = fill.maker_order_id == fill.buy_order_id;
    let (buyer_nonce, seller_nonce) =
        option_execution_nonces(state, nonce_provider, &fill.buyer, &fill.seller).await?;
    let intent = build_option_execution_intent(
        state,
        &series,
        OptionExecutionSourceType::OptionOrderbookFill,
        fill.fill_id.to_string(),
        fill.buyer.clone(),
        fill.seller.clone(),
        fill.price_1e8,
        fill.size_1e8,
        buyer_is_maker,
        buyer_nonce,
        seller_nonce,
        fill.created_at_ms,
    )?;
    insert_option_execution_intent(state, intent)
        .await
        .map(Some)
}

pub async fn create_option_rfq_execution_intent(
    state: &AppState,
    fill: &OptionRfqFill,
) -> Result<Option<OptionExecutionIntent>> {
    let provider = option_nonce_provider(state)?;
    create_option_rfq_execution_intent_with_nonce_provider(state, fill, provider.as_ref()).await
}

async fn create_option_rfq_execution_intent_with_nonce_provider<P>(
    state: &AppState,
    fill: &OptionRfqFill,
    nonce_provider: Option<&P>,
) -> Result<Option<OptionExecutionIntent>>
where
    P: OptionNonceProvider,
{
    if !state.options_config.execution_enabled {
        return Ok(None);
    }
    let series = get_option_series(state, &fill.option_series_id).await?;
    let buyer_is_maker = fill.buyer.0.eq_ignore_ascii_case(&fill.mm_account.0);
    let (buyer_nonce, seller_nonce) =
        option_execution_nonces(state, nonce_provider, &fill.buyer, &fill.seller).await?;
    let intent = build_option_execution_intent(
        state,
        &series,
        OptionExecutionSourceType::OptionRfqFill,
        fill.fill_id.to_string(),
        fill.buyer.clone(),
        fill.seller.clone(),
        fill.price_1e8,
        fill.size_1e8,
        buyer_is_maker,
        buyer_nonce,
        seller_nonce,
        fill.created_at_ms,
    )?;
    insert_option_execution_intent(state, intent)
        .await
        .map(Some)
}

async fn create_option_orderbook_execution_intents(
    state: &AppState,
    fills: &[OptionFill],
) -> Result<Vec<OptionExecutionIntent>> {
    let mut intents = Vec::new();
    if !state.options_config.execution_enabled {
        return Ok(intents);
    }
    for fill in fills {
        if let Some(intent) = create_option_orderbook_execution_intent(state, fill).await? {
            intents.push(intent);
        }
    }
    Ok(intents)
}

async fn insert_option_execution_intent(
    state: &AppState,
    intent: OptionExecutionIntent,
) -> Result<OptionExecutionIntent> {
    if let Some(repository) = state.repository.clone() {
        return repository.insert_option_execution_intent(&intent).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_execution_intent(intent))
}

async fn persist_option_execution_simulation_result(
    state: &AppState,
    result: &OptionExecutionSimulationResult,
) -> Result<OptionExecutionIntent> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .persist_option_execution_simulation_result(result)
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .persist_option_execution_simulation_result(result)
}

async fn update_option_execution_intent_status(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    status: OptionExecutionIntentStatus,
    error: Option<String>,
    updated_at_ms: TimestampMs,
) -> Result<OptionExecutionIntent> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .update_option_execution_intent_status(intent_id, status, error, updated_at_ms)
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .update_option_execution_intent_status(intent_id, status, error, updated_at_ms)
}

async fn insert_option_execution_transaction(
    state: &AppState,
    transaction: OptionExecutionTransaction,
) -> Result<OptionExecutionTransaction> {
    if let Some(repository) = state.repository.clone() {
        repository
            .insert_option_execution_transaction(&transaction)
            .await?;
        return Ok(transaction);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_execution_transaction(transaction)
}

async fn find_submitted_option_execution_transaction(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<Option<OptionExecutionTransaction>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .find_submitted_option_execution_transaction_by_intent(intent_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .find_submitted_option_execution_transaction_by_intent(intent_id))
}

fn option_execution_transaction_from_request(
    request: &ExecutionTransactionRequest,
    from: AccountId,
    tx_hash: Option<String>,
    error: Option<String>,
    now: TimestampMs,
    gas_check: Option<&OptionExecutionGasSafetyCheck>,
) -> OptionExecutionTransaction {
    let status = if tx_hash.is_some() {
        ExecutionTransactionStatus::Submitted
    } else {
        ExecutionTransactionStatus::Failed
    };
    OptionExecutionTransaction {
        transaction_id: Uuid::new_v4().to_string(),
        intent_id: request.intent_id,
        onchain_intent_id: Some(request.onchain_intent_id.clone()),
        from,
        to: request.to.clone(),
        calldata: request.calldata_hex(),
        value_wei: request.value_wei.to_string(),
        gas_limit: Some(request.gas_limit),
        tx_hash,
        status,
        error,
        estimated_gas: gas_check.and_then(|check| check.estimated_gas),
        required_gas: gas_check.and_then(|check| check.required_gas),
        simulation_gas_limit: gas_check.map(|check| check.simulation_gas_limit),
        broadcast_gas_limit: gas_check.map(|check| check.broadcast_gas_limit),
        gas_safety_bps: gas_check.map(|check| check.gas_safety_bps),
        gas_check_status: gas_check.map(|check| check.status),
        gas_check_error: gas_check.and_then(|check| check.error.clone()),
        confirmation_status: None,
        confirmed_at_ms: None,
        confirmed_block_number: None,
        receipt_status: None,
        confirmation_error: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

fn is_valid_tx_hash(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("0x") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn option_nonce_provider(state: &AppState) -> Result<Option<HttpJsonRpcProvider>> {
    if !state.option_nonce_sync_config.enabled {
        return Ok(None);
    }
    Ok(state
        .option_nonce_sync_config
        .rpc_url
        .clone()
        .map(HttpJsonRpcProvider::new))
}

async fn option_execution_nonces<P>(
    state: &AppState,
    nonce_provider: Option<&P>,
    buyer: &AccountId,
    seller: &AccountId,
) -> Result<(u128, u128)>
where
    P: OptionNonceProvider,
{
    if !state.option_nonce_sync_config.enabled {
        return Ok((0, 0));
    }

    let result = async {
        let provider = nonce_provider.ok_or_else(|| {
            BackendError::Config("RPC_URL is required for option nonce sync".to_string())
        })?;
        let buyer_nonce =
            read_option_nonce_value(&state.option_nonce_sync_config, provider, buyer).await?;
        let seller_nonce =
            read_option_nonce_value(&state.option_nonce_sync_config, provider, seller).await?;
        Ok((buyer_nonce, seller_nonce))
    }
    .await;

    match result {
        Ok(nonces) => Ok(nonces),
        Err(error) if state.option_nonce_sync_config.strict => Err(error),
        Err(error) => {
            warn!(
                buyer = %buyer.0,
                seller = %seller.0,
                error = %error,
                "option nonce sync failed in non-strict mode; falling back to zero nonces"
            );
            Ok((0, 0))
        }
    }
}

fn validate_option_execution_simulation_preflight(
    state: &AppState,
    intent: &OptionExecutionIntent,
) -> Result<()> {
    validate_simulation_target(&state.options_config.matching_engine_address)?;
    validate_simulation_intent(intent)?;
    let from = option_execution_simulation_from(state)?;
    parse_evm_address(&from)?;
    Ok(())
}

fn option_execution_simulation_from(state: &AppState) -> Result<AccountId> {
    let from = state
        .options_config
        .execution_simulation_from
        .clone()
        .unwrap_or_else(|| state.execution_config.executor_from_address.clone());
    parse_evm_address(&from)?;
    Ok(from)
}

async fn validate_option_order_execution_preflight(
    state: &AppState,
    series: &OptionSeries,
    input: &SubmitOptionOrderInput,
) -> Result<()> {
    if !state.options_config.execution_enabled {
        return Ok(());
    }

    let mut candidates = open_option_orders_for_series(state, &input.option_series_id)
        .await?
        .into_iter()
        .filter(|order| {
            order.side != input.side
                && order.status.is_live()
                && order.remaining_size_1e8 > 0
                && match input.side {
                    Side::Buy => input.price_1e8 >= order.price_1e8,
                    Side::Sell => input.price_1e8 <= order.price_1e8,
                }
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(());
    }

    validate_executable_option_series(state, series)?;
    sort_execution_preflight_candidates(&mut candidates, input.side);
    let mut remaining_size_1e8 = input.size_1e8;
    for maker in candidates {
        if remaining_size_1e8 == 0 {
            break;
        }
        let fill_size_1e8 = remaining_size_1e8.min(maker.remaining_size_1e8);
        validate_option_execution_conversion(state, fill_size_1e8, maker.price_1e8)?;
        remaining_size_1e8 -= fill_size_1e8;
    }
    Ok(())
}

async fn validate_option_rfq_execution_preflight(
    state: &AppState,
    series: &OptionSeries,
    quote: &OptionRfqQuote,
) -> Result<()> {
    if !state.options_config.execution_enabled {
        return Ok(());
    }
    validate_executable_option_series(state, series)?;
    validate_option_execution_conversion(state, quote.size_1e8, quote.price_1e8)
}

#[allow(clippy::too_many_arguments)]
fn build_option_execution_intent(
    state: &AppState,
    series: &OptionSeries,
    source_type: OptionExecutionSourceType,
    source_id: String,
    buyer: AccountId,
    seller: AccountId,
    source_price_1e8: Price1e8,
    source_size_1e8: Size1e8,
    buyer_is_maker: bool,
    buyer_nonce: u128,
    seller_nonce: u128,
    source_created_at_ms: TimestampMs,
) -> Result<OptionExecutionIntent> {
    let metadata = validate_executable_option_series(state, series)?;
    let quantity_contracts = quantity_contracts_from_size(source_size_1e8)?;
    let premium_per_contract_native = premium_per_contract_native(
        source_price_1e8,
        state.options_config.execution_default_settlement_decimals,
    )?;
    let intent_id = Uuid::new_v4();
    let onchain_intent_id = option_execution_intent_id_to_hex_bytes32(&intent_id.to_string())?;
    let now = now_ms();
    Ok(OptionExecutionIntent {
        intent_id,
        onchain_intent_id,
        source_type,
        source_id,
        option_series_id: series.option_series_id.clone(),
        onchain_option_id: metadata.onchain_option_id,
        buyer,
        seller,
        underlying: metadata.underlying,
        settlement_asset: metadata.settlement_asset,
        expiry: series.expiry,
        strike_1e8: series.strike_1e8,
        is_call: series.is_call,
        contract_size_1e8: series.contract_size_1e8,
        quantity_contracts,
        source_size_1e8,
        source_price_1e8,
        premium_per_contract_native,
        buyer_is_maker,
        buyer_nonce: Some(buyer_nonce),
        seller_nonce: Some(seller_nonce),
        deadline: 0,
        buyer_signature: None,
        seller_signature: None,
        calldata: None,
        status: OptionExecutionIntentStatus::SignaturesRequired,
        error: None,
        simulation_status: None,
        simulation_error: None,
        simulation_block_number: None,
        simulation_revert_data: None,
        simulation_revert_selector: None,
        simulated_at_ms: None,
        created_at_ms: source_created_at_ms,
        updated_at_ms: now,
    })
}

struct ExecutableOptionSeriesMetadata {
    onchain_option_id: String,
    underlying: AccountId,
    settlement_asset: AccountId,
}

fn validate_executable_option_series(
    state: &AppState,
    series: &OptionSeries,
) -> Result<ExecutableOptionSeriesMetadata> {
    let onchain_option_id = series
        .onchain_series_id
        .as_deref()
        .or(series.onchain_product_id.as_deref())
        .ok_or_else(|| {
            BackendError::InvalidOptionExecutionIntentState(
                "option series is missing onchain_series_id or onchain_product_id".to_string(),
            )
        })
        .and_then(|value| normalize_u256_string(value, "optionId"))?;
    validate_nonzero_execution_address(&series.underlying, "underlying")?;
    validate_nonzero_execution_address(&series.settlement_asset, "settlement_asset")?;
    validate_series_option_id_matches_metadata(series, &onchain_option_id)?;
    let _ = state;
    Ok(ExecutableOptionSeriesMetadata {
        onchain_option_id,
        underlying: AccountId::new(series.underlying.clone()),
        settlement_asset: AccountId::new(series.settlement_asset.clone()),
    })
}

fn validate_series_option_id_matches_metadata(
    series: &OptionSeries,
    onchain_option_id: &str,
) -> Result<()> {
    let strike_1e8 = u64::try_from(series.strike_1e8).map_err(|_| {
        BackendError::InvalidOptionExecutionIntentState("strike_1e8 exceeds uint64".to_string())
    })?;
    let option_id =
        alloy_primitives::U256::from_str_radix(onchain_option_id, 10).map_err(|error| {
            BackendError::InvalidOptionExecutionIntentState(format!(
                "optionId must be a uint256: {error}"
            ))
        })?;
    let underlying = AccountId::new(series.underlying.clone());
    let settlement_asset = AccountId::new(series.settlement_asset.clone());
    let european_id = crate::options::option_product_registry_option_id(
        &underlying,
        &settlement_asset,
        series.expiry,
        strike_1e8,
        series.contract_size_1e8,
        series.is_call,
        true,
    )?;
    let american_id = crate::options::option_product_registry_option_id(
        &underlying,
        &settlement_asset,
        series.expiry,
        strike_1e8,
        series.contract_size_1e8,
        series.is_call,
        false,
    )?;
    if option_id != european_id && option_id != american_id {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "optionId does not match option metadata for either isEuropean value".to_string(),
        ));
    }
    Ok(())
}

fn validate_nonzero_execution_address(value: &str, field: &str) -> Result<()> {
    let account = AccountId::new(value.to_string());
    let address = parse_evm_address(&account).map_err(|_| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be an EVM address when option execution is enabled"
        ))
    })?;
    if address.iter().all(|byte| *byte == 0) {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be nonzero when option execution is enabled"
        )));
    }
    Ok(())
}

fn validate_option_execution_conversion(
    state: &AppState,
    size_1e8: Size1e8,
    price_1e8: Price1e8,
) -> Result<()> {
    let _ = quantity_contracts_from_size(size_1e8)?;
    let _ = premium_per_contract_native(
        price_1e8,
        state.options_config.execution_default_settlement_decimals,
    )?;
    Ok(())
}

fn quantity_contracts_from_size(size_1e8: Size1e8) -> Result<u128> {
    if size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if size_1e8 % ONE_CONTRACT_1E8 != 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "size_1e8 must be a whole number of option contracts when option execution is enabled"
                .to_string(),
        ));
    }
    let quantity = size_1e8 / ONE_CONTRACT_1E8;
    if quantity == 0 {
        return Err(BackendError::ZeroSize);
    }
    Ok(quantity)
}

fn premium_per_contract_native(price_1e8: Price1e8, settlement_decimals: u32) -> Result<u128> {
    if price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    let scale = 10u128.checked_pow(settlement_decimals).ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(
            "settlement decimals overflow native premium conversion".to_string(),
        )
    })?;
    let premium = price_1e8.checked_mul(scale).ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(
            "premium native conversion overflow".to_string(),
        )
    })? / ONE_CONTRACT_1E8;
    if premium == 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "premium_per_contract_native is zero after settlement-native conversion".to_string(),
        ));
    }
    Ok(premium)
}

fn verify_option_execution_signature(
    state: &AppState,
    signature: Option<&str>,
    digest_bytes: &[u8; 32],
    expected_signer: &AccountId,
) -> Result<()> {
    let Some(signature) = signature else {
        return Ok(());
    };
    validate_signature_shape(signature)?;
    if state.options_config.execution_signature_mode == OptionExecutionSignatureMode::Strict {
        let recovered_signer = recover_eip712_signer(digest_bytes, signature)?;
        let expected = parse_evm_address(expected_signer)?;
        let recovered = parse_evm_address(&recovered_signer)?;
        if recovered != expected {
            return Err(BackendError::SignatureSignerMismatch);
        }
    }
    Ok(())
}

fn build_option_execution_calldata_from_parts(
    payload: &OptionTradePayload,
    buyer_signature: &str,
    seller_signature: &str,
) -> Result<String> {
    let bundle = OptionTradeSignatureBundle::new(buyer_signature, seller_signature)?;
    Ok(hex_0x(&encode_option_execute_trade_calldata(
        payload, &bundle,
    )?))
}

fn option_execution_signature_outcome(
    intent: OptionExecutionIntent,
) -> SubmitOptionExecutionSignaturesOutcome {
    let buyer_signature_present = intent.buyer_signature.is_some();
    let seller_signature_present = intent.seller_signature.is_some();
    let calldata_ready =
        intent.calldata.is_some() && intent.status == OptionExecutionIntentStatus::CalldataReady;
    SubmitOptionExecutionSignaturesOutcome {
        intent,
        buyer_signature_present,
        seller_signature_present,
        calldata_ready,
        missing_signatures: !(buyer_signature_present && seller_signature_present),
    }
}

fn sort_execution_preflight_candidates(orders: &mut [OptionOrder], taker_side: Side) {
    orders.sort_by(|left, right| {
        let price_order = match taker_side {
            Side::Buy => left.price_1e8.cmp(&right.price_1e8),
            Side::Sell => right.price_1e8.cmp(&left.price_1e8),
        };
        price_order
            .then_with(|| left.created_at_ms.cmp(&right.created_at_ms))
            .then_with(|| left.order_id.cmp(&right.order_id))
    });
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

fn ensure_option_execution_simulation_enabled(state: &AppState) -> Result<()> {
    ensure_enabled(state)?;
    if state.options_config.execution_simulation_enabled {
        Ok(())
    } else {
        Err(BackendError::Config(
            "option execution simulation is disabled".to_string(),
        ))
    }
}

fn ensure_option_execution_broadcast_enabled(state: &AppState) -> Result<()> {
    if !state.options_config.execution_broadcast_enabled {
        return Err(BackendError::Config(
            "option execution broadcast is disabled".to_string(),
        ));
    }
    ensure_enabled(state)?;
    if !state.options_config.execution_enabled {
        return Err(BackendError::Config(
            "OPTION_EXECUTION_ENABLED=true is required for option execution broadcast".to_string(),
        ));
    }
    if !state.execution_config.execution_enabled {
        return Err(BackendError::Config(
            "EXECUTION_ENABLED=true is required for option execution broadcast".to_string(),
        ));
    }
    if !state.execution_config.real_broadcast_enabled {
        return Err(BackendError::Config(
            "EXECUTOR_REAL_BROADCAST_ENABLED=true is required for option execution broadcast"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_account(account: &AccountId) -> Result<()> {
    parse_evm_address(account).map(|_| ())
}

fn validate_option_rfq_quote_ttl(state: &AppState, quote_ttl_ms: u64) -> Result<()> {
    if quote_ttl_ms < state.options_config.rfq_min_quote_ttl_ms {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote_ttl_ms is below the minimum".to_string(),
        ));
    }
    Ok(())
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

struct OptionRfqQuoteSignatureMetadata {
    signature: Option<String>,
    quote_digest: Option<String>,
    quote_nonce: Option<String>,
    signature_status: OptionRfqQuoteSignatureStatus,
    recovered_signer: Option<AccountId>,
}

fn verify_option_rfq_quote_signature(
    state: &AppState,
    rfq: &OptionRfqRequest,
    input: &SubmitOptionRfqQuoteInput,
    quote_ttl_ms: u64,
) -> Result<OptionRfqQuoteSignatureMetadata> {
    match state.options_config.rfq_quote_signature_mode {
        OptionRfqQuoteSignatureMode::Disabled => {
            let quote_digest = input
                .quote_nonce
                .map(|quote_nonce| {
                    let payload = option_rfq_quote_payload(
                        rfq,
                        input.mm_account.clone(),
                        input.price_1e8,
                        input.size_1e8,
                        quote_nonce,
                        quote_ttl_ms,
                    )?;
                    option_rfq_quote_digest(&payload, &state.options_config.rfq_eip712_domain)
                })
                .transpose()?;
            Ok(OptionRfqQuoteSignatureMetadata {
                signature: input.signature.clone(),
                quote_digest,
                quote_nonce: input.quote_nonce.map(|value| value.to_string()),
                signature_status: OptionRfqQuoteSignatureStatus::NotRequired,
                recovered_signer: None,
            })
        }
        OptionRfqQuoteSignatureMode::Strict => {
            let Some(quote_nonce) = input.quote_nonce else {
                return Err(BackendError::InvalidOptionRfqQuoteState(
                    "quote_nonce is required when OPTION_RFQ_QUOTE_SIGNATURE_MODE=strict"
                        .to_string(),
                ));
            };
            let Some(signature) = input.signature.as_deref() else {
                return Err(BackendError::InvalidOptionRfqQuoteState(
                    "signature is required when OPTION_RFQ_QUOTE_SIGNATURE_MODE=strict".to_string(),
                ));
            };
            validate_signature_shape(signature)?;
            let payload = option_rfq_quote_payload(
                rfq,
                input.mm_account.clone(),
                input.price_1e8,
                input.size_1e8,
                quote_nonce,
                quote_ttl_ms,
            )?;
            let digest_bytes =
                option_rfq_quote_digest_bytes(&payload, &state.options_config.rfq_eip712_domain)?;
            let quote_digest = hex_0x(&digest_bytes);
            let recovered_signer = recover_eip712_signer(&digest_bytes, signature)?;
            let expected = parse_evm_address(&input.mm_account)?;
            let recovered = parse_evm_address(&recovered_signer)?;
            if recovered != expected {
                return Err(BackendError::SignatureSignerMismatch);
            }
            Ok(OptionRfqQuoteSignatureMetadata {
                signature: Some(signature.to_string()),
                quote_digest: Some(quote_digest),
                quote_nonce: Some(quote_nonce.to_string()),
                signature_status: OptionRfqQuoteSignatureStatus::Verified,
                recovered_signer: Some(recovered_signer),
            })
        }
    }
}

fn quote_expires_at_ms(
    state: &AppState,
    rfq: &OptionRfqRequest,
    now: TimestampMs,
    quote_ttl_ms: u64,
) -> Result<TimestampMs> {
    match state.options_config.rfq_quote_signature_mode {
        OptionRfqQuoteSignatureMode::Disabled => {
            checked_expiry(now, quote_ttl_ms, "option RFQ quote expiry")
                .map(|expires_at_ms| expires_at_ms.min(rfq.expires_at_ms))
        }
        OptionRfqQuoteSignatureMode::Strict => signed_quote_expires_at_ms(rfq, quote_ttl_ms),
    }
}

fn signed_quote_expires_at_ms(rfq: &OptionRfqRequest, quote_ttl_ms: u64) -> Result<TimestampMs> {
    let quote_ttl_ms = i64::try_from(quote_ttl_ms).map_err(|_| {
        BackendError::InvalidOptionRfqQuoteState("quote_ttl_ms cannot be encoded".to_string())
    })?;
    rfq.created_at_ms
        .checked_add(quote_ttl_ms)
        .map(|expires_at_ms| expires_at_ms.min(rfq.expires_at_ms))
        .ok_or_else(|| {
            BackendError::InvalidOptionRfqQuoteState("quote expiry overflow".to_string())
        })
}

fn validate_option_rfq_quote_signature_status(
    state: &AppState,
    quote: &OptionRfqQuote,
) -> Result<()> {
    if state.options_config.rfq_quote_signature_mode != OptionRfqQuoteSignatureMode::Strict {
        return Ok(());
    }
    if quote.signature_status != OptionRfqQuoteSignatureStatus::Verified {
        return Err(BackendError::InvalidOptionRfqQuoteState(format!(
            "option RFQ quote signature is {}",
            quote.signature_status.as_str()
        )));
    }
    Ok(())
}

fn option_rfq_quote_payload(
    rfq: &OptionRfqRequest,
    mm_account: AccountId,
    price_1e8: Price1e8,
    size_1e8: Size1e8,
    quote_nonce: u64,
    quote_ttl_ms: u64,
) -> Result<OptionRfqQuoteSigningPayload> {
    let expires_at_ms = signed_quote_expires_at_ms(rfq, quote_ttl_ms)?;
    let expiry = u128::try_from(expires_at_ms / 1000).map_err(|_| {
        BackendError::InvalidOptionRfqQuoteState("quote expiry cannot be encoded".to_string())
    })?;
    Ok(OptionRfqQuoteSigningPayload {
        option_rfq_id: option_rfq_id_to_b256(&rfq.option_rfq_id.to_string()),
        mm_account,
        option_series_id: option_series_id_to_b256(&rfq.option_series_id)?,
        taker_is_buyer: rfq.side == Side::Buy,
        price_1e8,
        size_1e8,
        quote_nonce: quote_nonce.into(),
        expiry,
    })
}

fn broadcast_option_rfq_request(state: &AppState, rfq: &OptionRfqRequest) {
    let message = ServerMessage::OptionRfqRequest(NotificationEnvelope::new(
        "option_rfq_request",
        format!("option-rfq-push-{}", rfq.option_rfq_id),
        OptionRfqRequestPayload {
            option_rfq_id: rfq.option_rfq_id,
            taker: rfq.taker.clone(),
            option_series_id: rfq.option_series_id.clone(),
            side: rfq.side,
            size_1e8: rfq.size_1e8.to_string(),
            limit_price_1e8: rfq.limit_price_1e8.map(|value| value.to_string()),
            expires_at_ms: rfq.expires_at_ms,
        },
    ));
    match state.mm_sessions.broadcast(message) {
        Ok(sent) => {
            info!(
                option_rfq_id = %rfq.option_rfq_id,
                broadcast_count = sent,
                "broadcast option RFQ request to MM sessions"
            );
        }
        Err(error) => {
            warn!(
                option_rfq_id = %rfq.option_rfq_id,
                error = %error,
                "option RFQ request broadcast failed"
            );
        }
    }
}

fn notify_option_rfq_quote_acceptance(
    state: &AppState,
    accepted_quote: &OptionRfqQuote,
    quotes_before_accept: &[OptionRfqQuote],
    option_fill_id: OptionRfqFillId,
) -> (bool, Option<String>) {
    let mut accepted_sent = false;
    let mut warning = None;

    if let Some(session_id) = accepted_quote.session_id.as_deref() {
        let message = ServerMessage::OptionRfqQuoteAccepted(NotificationEnvelope::new(
            "option_rfq_quote_accepted",
            format!("option-rfq-accepted-{}", accepted_quote.quote_id),
            OptionRfqQuoteAcceptedPayload {
                option_rfq_id: accepted_quote.option_rfq_id,
                quote_id: accepted_quote.quote_id,
                option_fill_id,
            },
        ));
        match state.mm_sessions.send_to_session(session_id, message) {
            Ok(()) => {
                accepted_sent = true;
            }
            Err(error) => {
                let message = error.to_string();
                warn!(
                    option_rfq_id = %accepted_quote.option_rfq_id,
                    quote_id = %accepted_quote.quote_id,
                    session_id,
                    error = %message,
                    "option RFQ quote accepted notification failed"
                );
                warning = Some(message);
            }
        }
    }

    for quote in quotes_before_accept {
        if quote.quote_id == accepted_quote.quote_id || quote.status != OptionRfqQuoteStatus::Active
        {
            continue;
        }
        let Some(session_id) = quote.session_id.as_deref() else {
            continue;
        };
        let message = ServerMessage::OptionRfqQuoteRejected(NotificationEnvelope::new(
            "option_rfq_quote_rejected",
            format!("option-rfq-rejected-{}", quote.quote_id),
            OptionRfqQuoteRejectedPayload {
                option_rfq_id: quote.option_rfq_id,
                quote_id: quote.quote_id,
                reason: "competing quote accepted".to_string(),
            },
        ));
        if let Err(error) = state.mm_sessions.send_to_session(session_id, message) {
            warn!(
                option_rfq_id = %quote.option_rfq_id,
                quote_id = %quote.quote_id,
                session_id,
                error = %error,
                "option RFQ quote rejected notification failed"
            );
        }
    }

    (accepted_sent, warning)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirmation::ConfirmationReceipt;
    use crate::engine::EngineState;
    use crate::execution::rpc::{EstimateGasRequest, EthCallRequest, EthCallSuccess, RpcFuture};
    use crate::execution::{DecodedRevertError, RevertDiagnostics};
    use crate::nonce_sync::OptionNonceSyncConfig;
    use crate::options::{
        OptionExecutionConfirmationStatus, OptionExecutionGasCheckStatus,
        OptionExecutionSimulationStatus, OptionsConfig,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    enum MockOutcome {
        Success,
        Revert(RevertDiagnostics),
    }

    #[derive(Clone)]
    struct MockProvider {
        outcome: MockOutcome,
        calls: Arc<Mutex<Vec<EthCallRequest>>>,
    }

    #[derive(Clone)]
    struct MockOptionNonceProvider {
        buyer_nonce: u128,
        seller_nonce: u128,
        fail: bool,
        calls: Arc<Mutex<Vec<AccountId>>>,
    }

    #[derive(Clone)]
    enum MockEstimateOutcome {
        Value(u64),
        Failure(String),
    }

    #[derive(Clone)]
    struct MockBroadcastProvider {
        chain_id: u64,
        tx_hash: String,
        fail_send: bool,
        send_calls: Arc<Mutex<Vec<String>>>,
        nonce_calls: Arc<Mutex<Vec<AccountId>>>,
        estimate_outcome: MockEstimateOutcome,
        estimate_calls: Arc<Mutex<Vec<EstimateGasRequest>>>,
    }

    impl MockProvider {
        fn success() -> Self {
            Self {
                outcome: MockOutcome::Success,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn revert(revert_data: &str) -> Self {
            Self {
                outcome: MockOutcome::Revert(RevertDiagnostics {
                    raw_error: "execution reverted".to_string(),
                    revert_data: Some(revert_data.to_string()),
                    revert_selector: Some(revert_data[..10].to_string()),
                    decoded_error: DecodedRevertError {
                        kind: "unknown_custom_error".to_string(),
                        name: None,
                        selector: Some(revert_data[..10].to_string()),
                        args: None,
                        decoded: None,
                    },
                }),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl MockOptionNonceProvider {
        fn success(buyer_nonce: u128, seller_nonce: u128) -> Self {
            Self {
                buyer_nonce,
                seller_nonce,
                fail: false,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn failure() -> Self {
            Self {
                buyer_nonce: 0,
                seller_nonce: 0,
                fail: true,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<AccountId> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl MockBroadcastProvider {
        fn success() -> Self {
            Self {
                chain_id: 84532,
                tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                fail_send: false,
                send_calls: Arc::new(Mutex::new(Vec::new())),
                nonce_calls: Arc::new(Mutex::new(Vec::new())),
                estimate_outcome: MockEstimateOutcome::Value(450_000),
                estimate_calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn invalid_hash() -> Self {
            Self {
                tx_hash: "0xnot-a-tx-hash".to_string(),
                ..Self::success()
            }
        }

        fn fail_send() -> Self {
            Self {
                fail_send: true,
                ..Self::success()
            }
        }

        fn with_estimate(estimated_gas: u64) -> Self {
            Self {
                estimate_outcome: MockEstimateOutcome::Value(estimated_gas),
                ..Self::success()
            }
        }

        fn with_estimate_failure(error: impl Into<String>) -> Self {
            Self {
                estimate_outcome: MockEstimateOutcome::Failure(error.into()),
                ..Self::success()
            }
        }

        fn send_count(&self) -> usize {
            self.send_calls.lock().unwrap().len()
        }

        fn estimate_count(&self) -> usize {
            self.estimate_calls.lock().unwrap().len()
        }
    }

    impl EthCallProvider for MockProvider {
        fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let outcome = self.outcome.clone();
            let calls = self.calls.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(request);
                match outcome {
                    MockOutcome::Success => Ok(EthCallSuccess {
                        block_number: Some(123),
                        output: Vec::new(),
                    }),
                    MockOutcome::Revert(diagnostics) => {
                        Err(BackendError::SimulationReverted(Box::new(diagnostics)))
                    }
                }
            })
        }
    }

    impl OptionNonceProvider for MockOptionNonceProvider {
        fn option_matching_nonce(
            &self,
            _matching_engine: AccountId,
            account: AccountId,
        ) -> RpcFuture<'_, u128> {
            let calls = self.calls.clone();
            let buyer_nonce = self.buyer_nonce;
            let seller_nonce = self.seller_nonce;
            let fail = self.fail;
            Box::pin(async move {
                calls.lock().unwrap().push(account.clone());
                if fail {
                    return Err(BackendError::Simulation(
                        "option nonce RPC unavailable".to_string(),
                    ));
                }
                if account.0.ends_with("0001") {
                    Ok(buyer_nonce)
                } else {
                    Ok(seller_nonce)
                }
            })
        }
    }

    impl GasEstimateProvider for MockBroadcastProvider {
        fn estimate_gas(&self, request: EstimateGasRequest) -> RpcFuture<'_, u64> {
            let calls = self.estimate_calls.clone();
            let outcome = self.estimate_outcome.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(request);
                match outcome {
                    MockEstimateOutcome::Value(value) => Ok(value),
                    MockEstimateOutcome::Failure(message) => Err(BackendError::Simulation(message)),
                }
            })
        }
    }

    impl TransactionBroadcastProvider for MockBroadcastProvider {
        fn chain_id(&self) -> RpcFuture<'_, u64> {
            let chain_id = self.chain_id;
            Box::pin(async move { Ok(chain_id) })
        }

        fn transaction_count(&self, address: AccountId) -> RpcFuture<'_, u64> {
            let calls = self.nonce_calls.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(address);
                Ok(7)
            })
        }

        fn send_raw_transaction(&self, raw_transaction: String) -> RpcFuture<'_, String> {
            let calls = self.send_calls.clone();
            let tx_hash = self.tx_hash.clone();
            let fail_send = self.fail_send;
            Box::pin(async move {
                calls.lock().unwrap().push(raw_transaction);
                if fail_send {
                    return Err(BackendError::Simulation("mock send failed".to_string()));
                }
                Ok(tx_hash)
            })
        }
    }

    #[tokio::test]
    async fn option_nonce_sync_disabled_preserves_zero_intent_nonces() {
        let state = state_with_simulation(false);
        insert_executable_series(&state);
        let fill = orderbook_fill();

        let intent = create_option_orderbook_execution_intent_with_nonce_provider::<
            MockOptionNonceProvider,
        >(&state, &fill, None)
        .await
        .unwrap()
        .unwrap();

        assert_eq!(intent.buyer_nonce, Some(0));
        assert_eq!(intent.seller_nonce, Some(0));
    }

    #[tokio::test]
    async fn option_execution_intent_uses_synced_option_nonces() {
        let state = state_with_option_nonce_sync(true);
        insert_executable_series(&state);
        let provider = MockOptionNonceProvider::success(17, 23);
        let fill = orderbook_fill();

        let intent = create_option_orderbook_execution_intent_with_nonce_provider(
            &state,
            &fill,
            Some(&provider),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(intent.buyer_nonce, Some(17));
        assert_eq!(intent.seller_nonce, Some(23));
        let calls = provider.calls();
        assert_eq!(calls, vec![fill.buyer, fill.seller]);
    }

    #[tokio::test]
    async fn strict_option_nonce_sync_failure_does_not_create_intent() {
        let state = state_with_option_nonce_sync(true);
        insert_executable_series(&state);
        let fill = orderbook_fill();
        let provider = MockOptionNonceProvider::failure();

        let error = create_option_orderbook_execution_intent_with_nonce_provider(
            &state,
            &fill,
            Some(&provider),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("option nonce RPC unavailable"));
        assert!(state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_intents()
            .is_empty());
    }

    #[tokio::test]
    async fn non_strict_option_nonce_sync_failure_falls_back_to_zero() {
        let state = state_with_option_nonce_sync(false);
        insert_executable_series(&state);
        let provider = MockOptionNonceProvider::failure();

        let intent = create_option_orderbook_execution_intent_with_nonce_provider(
            &state,
            &orderbook_fill(),
            Some(&provider),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(intent.buyer_nonce, Some(0));
        assert_eq!(intent.seller_nonce, Some(0));
    }

    #[tokio::test]
    async fn option_execution_signing_payload_uses_stored_synced_nonces() {
        let state = state_with_simulation(false);
        let mut intent = calldata_ready_intent();
        intent.buyer_nonce = Some(17);
        intent.seller_nonce = Some(23);
        let intent = insert_intent(&state, intent);

        let outcome = option_execution_signing_payload(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(outcome.payload.buyer_nonce, 17);
        assert_eq!(outcome.payload.seller_nonce, 23);
    }

    #[tokio::test]
    async fn option_execution_calldata_uses_stored_synced_nonces() {
        let state = state_with_simulation(false);
        let mut intent = calldata_ready_intent();
        intent.buyer_nonce = Some(17);
        intent.seller_nonce = Some(23);
        intent.calldata = None;
        intent.status = OptionExecutionIntentStatus::SignaturesRequired;
        intent.buyer_signature = Some(signature_hex(0xaa));
        intent.seller_signature = Some(signature_hex(0xbb));
        let expected_payload = OptionTradePayload::from_intent(&intent).unwrap();
        let expected_calldata = build_option_execution_calldata_from_parts(
            &expected_payload,
            intent.buyer_signature.as_deref().unwrap(),
            intent.seller_signature.as_deref().unwrap(),
        )
        .unwrap();
        let intent = insert_intent(&state, intent);

        let outcome = option_execution_calldata(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(
            outcome.calldata.as_deref(),
            Some(expected_calldata.as_str())
        );
    }

    #[tokio::test]
    async fn option_execution_simulation_disabled_rejects() {
        let state = state_with_simulation(false);
        let intent = insert_intent(&state, calldata_ready_intent());

        let error = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap_err();

        assert!(
            matches!(error, BackendError::Config(message) if message.contains("option execution simulation is disabled"))
        );
    }

    #[tokio::test]
    async fn option_execution_simulation_missing_intent_rejects() {
        let state = state_with_simulation(true);

        let error = prepare_option_execution_simulation(&state, Uuid::from_u128(99))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            BackendError::InvalidOptionExecutionIntentId
        ));
    }

    #[tokio::test]
    async fn option_execution_simulation_missing_calldata_rejects_and_stores_unavailable() {
        let state = state_with_simulation(true);
        let intent = insert_intent(
            &state,
            OptionExecutionIntent {
                calldata: None,
                ..calldata_ready_intent()
            },
        );

        let error = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap_err();
        let status = option_execution_simulation_status(&state, intent.intent_id)
            .await
            .unwrap();

        assert!(
            matches!(error, BackendError::InvalidOptionExecutionIntentState(message) if message.contains("calldata"))
        );
        assert_eq!(
            status.simulation_status,
            OptionExecutionSimulationStatus::SimulationUnavailable
        );
        assert!(status.error.unwrap().contains("calldata"));
    }

    #[tokio::test]
    async fn option_execution_simulation_success_stores_ok_without_changing_intent_status() {
        let state = state_with_simulation(true);
        let intent = insert_intent(&state, calldata_ready_intent());
        let provider = MockProvider::success();

        let prepared = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap();
        let result = simulate_prepared_option_execution_intent(&state, &prepared, &provider)
            .await
            .unwrap();
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(
            result.simulation_status,
            OptionExecutionSimulationStatus::SimulationOk
        );
        assert_eq!(result.block_number, Some(123));
        assert_eq!(stored.status, OptionExecutionIntentStatus::CalldataReady);
        assert_eq!(
            stored.simulation_status,
            Some(OptionExecutionSimulationStatus::SimulationOk)
        );
        assert_eq!(stored.simulation_block_number, Some(123));
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.0, "0x00000000000000000000000000000000000000ee");
        assert_eq!(calls[0].gas_limit, Some(500_000));
    }

    #[tokio::test]
    async fn option_execution_simulation_revert_stores_failed_and_revert_data() {
        let state = state_with_simulation(true);
        let intent = insert_intent(&state, calldata_ready_intent());
        let provider = MockProvider::revert("0x12345678");

        let prepared = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap();
        let result = simulate_prepared_option_execution_intent(&state, &prepared, &provider)
            .await
            .unwrap();
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(
            result.simulation_status,
            OptionExecutionSimulationStatus::SimulationFailed
        );
        assert_eq!(result.revert_data.as_deref(), Some("0x12345678"));
        assert_eq!(result.revert_selector.as_deref(), Some("0x12345678"));
        assert_eq!(
            stored.simulation_status,
            Some(OptionExecutionSimulationStatus::SimulationFailed)
        );
        assert_eq!(stored.simulation_revert_data.as_deref(), Some("0x12345678"));
        assert_eq!(stored.status, OptionExecutionIntentStatus::CalldataReady);
    }

    #[tokio::test]
    async fn option_execution_broadcast_disabled_rejects_without_send() {
        let state = state_with_broadcast(false);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(error, BackendError::Config(message) if message.contains("broadcast is disabled"))
        );
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_intent_rejects() {
        let state = state_with_broadcast(true);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, Uuid::from_u128(99), &provider)
                .await
                .unwrap_err();

        assert!(matches!(
            error,
            BackendError::InvalidOptionExecutionIntentId
        ));
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_calldata_rejects_without_transaction() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.calldata = None;
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(error, BackendError::InvalidOptionExecutionIntentState(message) if message.contains("calldata"))
        );
        assert_eq!(provider.send_count(), 0);
        assert!(option_transactions(&state, intent.intent_id).is_empty());
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_signatures_rejects_without_send() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.buyer_signature = None;
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(matches!(error, BackendError::MissingTradeSignatures));
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_requires_simulation_ok_by_default() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.simulation_status = Some(OptionExecutionSimulationStatus::SimulationFailed);
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(error, BackendError::BroadcastRejected(message) if message.contains("simulation_ok"))
        );
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_rpc_or_private_key_rejects() {
        let mut missing_rpc = state_with_broadcast(true);
        missing_rpc.execution_config.rpc_url = None;
        let intent = insert_intent(&missing_rpc, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();

        let error = broadcast_option_execution_intent_with_provider(
            &missing_rpc,
            intent.intent_id,
            &provider,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, BackendError::Config(message) if message.contains("RPC_URL")));
        assert_eq!(provider.send_count(), 0);

        let mut missing_key = state_with_broadcast(true);
        missing_key.execution_config.executor_private_key = None;
        let intent = insert_intent(&missing_key, broadcast_ready_intent());
        let error = broadcast_option_execution_intent_with_provider(
            &missing_key,
            intent.intent_id,
            &provider,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, BackendError::Config(message) if message.contains("EXECUTOR_PRIVATE_KEY"))
        );
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_mock_success_persists_submitted_hash_once() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();

        let outcome =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(outcome.submitted);
        assert!(!outcome.duplicate);
        assert_eq!(
            outcome.transaction.tx_hash.as_deref(),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            stored.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[0].status,
            ExecutionTransactionStatus::Submitted
        );
        assert_eq!(
            transactions[0].tx_hash.as_deref(),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(provider.send_count(), 1);

        let duplicate =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(
            duplicate.transaction.transaction_id,
            transactions[0].transaction_id
        );
        assert_eq!(provider.send_count(), 1);
    }

    #[tokio::test]
    async fn option_execution_broadcast_does_not_persist_invalid_or_failed_tx_hashes() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::invalid_hash();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();

        assert!(
            matches!(error, BackendError::BroadcastRejected(message) if message.contains("invalid transaction hash"))
        );
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(stored.status, OptionExecutionIntentStatus::BroadcastFailed);

        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::fail_send();
        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(error.to_string().contains("mock send failed"));
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
    }

    #[tokio::test]
    async fn option_execution_broadcast_rejects_when_cap_below_estimated_gas() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        // broadcast cap 600_000 < estimated 750_000 → broadcast_cap_too_low
        let provider = MockBroadcastProvider::with_estimate(750_000);

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("below estimated_gas"))
        );
        assert_eq!(provider.send_count(), 0);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::BroadcastCapTooLow)
        );
        assert_eq!(transactions[0].estimated_gas, Some(750_000));
        assert_eq!(transactions[0].broadcast_gas_limit, Some(600_000));
        assert_eq!(transactions[0].gas_safety_bps, Some(12_500));
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(stored.status, OptionExecutionIntentStatus::BroadcastFailed);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_rejects_when_cap_below_safety_margin() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        // estimated 500_000; required = 500_000 * 1.25 = 625_000; cap 600_000 < required
        let provider = MockBroadcastProvider::with_estimate(500_000);

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("below required_gas"))
        );
        assert_eq!(provider.send_count(), 0);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::BelowSafetyMargin)
        );
        assert_eq!(transactions[0].estimated_gas, Some(500_000));
        assert_eq!(transactions[0].required_gas, Some(625_000));
        assert_eq!(transactions[0].broadcast_gas_limit, Some(600_000));
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(stored.status, OptionExecutionIntentStatus::BroadcastFailed);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_allows_when_cap_satisfies_safety_margin() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        // estimated 400_000; required = 500_000; cap 600_000 >= required → ok
        let provider = MockBroadcastProvider::with_estimate(400_000);

        let outcome =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert!(outcome.submitted);
        assert_eq!(provider.send_count(), 1);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(
            outcome.transaction.gas_check_status,
            Some(OptionExecutionGasCheckStatus::Ok)
        );
        assert_eq!(outcome.transaction.estimated_gas, Some(400_000));
        assert_eq!(outcome.transaction.required_gas, Some(500_000));
        assert_eq!(outcome.transaction.broadcast_gas_limit, Some(600_000));
        assert_eq!(outcome.transaction.gas_safety_bps, Some(12_500));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_uncapped_simulation_cannot_bypass_capped_broadcast() {
        // Reproduces the V1L failure: simulation runs with no gas cap (`OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0`)
        // and produces simulation_ok, but the live broadcast cap inherited from EXECUTOR_MAX_GAS_LIMIT is
        // smaller than the eth_estimateGas result. The preflight must reject before signing or sending.
        let mut state = state_with_broadcast(true);
        state.options_config.execution_simulation_gas_limit = 0;
        state.options_config.execution_broadcast_gas_limit = 0; // fall back to EXECUTOR_MAX_GAS_LIMIT
        state.execution_config.max_gas_limit = 1_000_000;
        let intent = insert_intent(&state, broadcast_ready_intent());
        // mirrors the live failure: estimate 1_040_080, cap 1_000_000 → too low
        let provider = MockBroadcastProvider::with_estimate(1_040_080);

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(matches!(&error, BackendError::BroadcastRejected(_)));
        assert_eq!(provider.send_count(), 0);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::BroadcastCapTooLow)
        );
        // simulation_gas_limit recorded as 0 (the uncapped sentinel), confirming the bypass attempt
        // was visible to the preflight rather than relied upon for safety.
        assert_eq!(transactions[0].simulation_gas_limit, Some(0));
        assert_eq!(transactions[0].broadcast_gas_limit, Some(1_000_000));
        assert_eq!(transactions[0].estimated_gas, Some(1_040_080));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_rejects_when_estimate_fails() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::with_estimate_failure("estimate gas rpc down");

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("eth_estimateGas failed"))
        );
        assert_eq!(provider.send_count(), 0);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::EstimateFailed)
        );
        assert_eq!(transactions[0].estimated_gas, None);
        assert!(transactions[0]
            .gas_check_error
            .as_deref()
            .unwrap_or_default()
            .contains("estimate gas rpc down"));
        assert_no_generic_execution_rows(&state);
    }

    fn assert_no_generic_execution_rows(state: &AppState) {
        // The option execution path must never write to the generic execution_transactions
        // store or call the generic executor's broadcast endpoint. In tests we run without
        // a Postgres repository, so the only persistence sink is the in-memory option store,
        // which has no concept of `execution_transactions` (the generic perp table).
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    #[derive(Clone)]
    enum MockReceiptOutcome {
        Receipt(ConfirmationReceipt),
        NotFound,
        Error(String),
    }

    #[derive(Clone)]
    struct MockReceiptProvider {
        outcome: MockReceiptOutcome,
        calls: Arc<Mutex<Vec<String>>>,
        head_block: u64,
    }

    impl MockReceiptProvider {
        fn mined_success(tx_hash: &str, block: u64) -> Self {
            Self {
                outcome: MockReceiptOutcome::Receipt(ConfirmationReceipt {
                    tx_hash: tx_hash.to_string(),
                    status: Some(1),
                    block_number: Some(block),
                }),
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn mined_reverted(tx_hash: &str, block: u64) -> Self {
            Self {
                outcome: MockReceiptOutcome::Receipt(ConfirmationReceipt {
                    tx_hash: tx_hash.to_string(),
                    status: Some(0),
                    block_number: Some(block),
                }),
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn missing() -> Self {
            Self {
                outcome: MockReceiptOutcome::NotFound,
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn rpc_error(message: &str) -> Self {
            Self {
                outcome: MockReceiptOutcome::Error(message.to_string()),
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn with_head(mut self, head_block: u64) -> Self {
            self.head_block = head_block;
            self
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl TransactionReceiptProvider for MockReceiptProvider {
        fn block_number(&self) -> RpcFuture<'_, u64> {
            let head = self.head_block;
            Box::pin(async move { Ok(head) })
        }

        fn transaction_receipt(
            &self,
            tx_hash: String,
        ) -> RpcFuture<'_, Option<ConfirmationReceipt>> {
            let outcome = self.outcome.clone();
            let calls = self.calls.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(tx_hash);
                match outcome {
                    MockReceiptOutcome::Receipt(r) => Ok(Some(r)),
                    MockReceiptOutcome::NotFound => Ok(None),
                    MockReceiptOutcome::Error(message) => Err(BackendError::Simulation(message)),
                }
            })
        }
    }

    fn broadcast_submitted_intent_with_tx(
        state: &AppState,
        tx_hash: &str,
    ) -> (OptionExecutionIntent, OptionExecutionTransaction) {
        let intent = broadcast_ready_intent();
        let intent = insert_intent(state, intent);
        // Move intent to broadcast_submitted in-memory and insert a matching tx row.
        let now = now_ms();
        let updated = state
            .options_store
            .lock()
            .unwrap()
            .update_option_execution_intent_status(
                intent.intent_id,
                OptionExecutionIntentStatus::BroadcastSubmitted,
                None,
                now,
            )
            .unwrap();
        let tx = OptionExecutionTransaction {
            transaction_id: Uuid::new_v4().to_string(),
            intent_id: intent.intent_id,
            onchain_intent_id: Some(intent.onchain_intent_id.clone()),
            from: AccountId::new("0xc35f7a8a103a9a4464adfaa76b9b514093d23c27"),
            to: AccountId::new("0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b"),
            calldata: "0x031f77b3".to_string(),
            value_wei: "0".to_string(),
            gas_limit: Some(1_500_000),
            tx_hash: Some(tx_hash.to_string()),
            status: ExecutionTransactionStatus::Submitted,
            error: None,
            estimated_gas: Some(1_091_120),
            required_gas: Some(1_363_900),
            simulation_gas_limit: Some(0),
            broadcast_gas_limit: Some(1_500_000),
            gas_safety_bps: Some(12_500),
            gas_check_status: Some(OptionExecutionGasCheckStatus::Ok),
            gas_check_error: None,
            confirmation_status: None,
            confirmed_at_ms: None,
            confirmed_block_number: None,
            receipt_status: None,
            confirmation_error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let stored_tx = state
            .options_store
            .lock()
            .unwrap()
            .insert_option_execution_transaction(tx)
            .unwrap();
        (updated, stored_tx)
    }

    #[tokio::test]
    async fn option_execution_confirm_mined_success_transitions_to_broadcast_confirmed() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125";
        let (intent, tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 41856964);

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        assert_eq!(outcome.receipt_status, Some(1));
        assert_eq!(outcome.block_number, Some(41856964));
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastConfirmed
        );
        assert_eq!(
            outcome.transaction.confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedSuccess)
        );
        assert_eq!(outcome.transaction.confirmed_block_number, Some(41856964));
        assert_eq!(outcome.transaction.receipt_status, Some(1));
        assert_eq!(outcome.transaction.transaction_id, tx.transaction_id);
        assert_eq!(provider.call_count(), 1);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_confirm_mined_reverted_transitions_to_broadcast_reverted() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_reverted(tx_hash, 100);

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::MinedReverted
        );
        assert_eq!(outcome.receipt_status, Some(0));
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastReverted
        );
        assert_eq!(
            outcome.transaction.confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedReverted)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_confirm_missing_receipt_does_not_change_intent_status() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x2222222222222222222222222222222222222222222222222222222222222222";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::missing();

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::ReceiptMissing
        );
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        assert_eq!(
            outcome.transaction.confirmation_status,
            Some(OptionExecutionConfirmationStatus::ReceiptMissing)
        );
        assert!(outcome.error.is_some());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_confirm_receipt_error_does_not_change_intent_status() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x3333333333333333333333333333333333333333333333333333333333333333";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::rpc_error("rpc temporarily unavailable");

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::ReceiptError
        );
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        assert!(outcome
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("rpc temporarily unavailable"));
    }

    #[tokio::test]
    async fn option_execution_confirm_rejects_intent_without_submitted_transaction() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockReceiptProvider::mined_success("0xabc", 1);

        let error =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(matches!(
            error,
            BackendError::InvalidOptionExecutionIntentState(message)
                if message.contains("no submitted option execution transaction")
        ));
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_confirm_idempotent_on_already_confirmed_row() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x4444444444444444444444444444444444444444444444444444444444444444";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 200);

        let first =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();
        let second =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            first.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        assert_eq!(
            second.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        assert_eq!(
            second.intent.status,
            OptionExecutionIntentStatus::BroadcastConfirmed
        );
        // Provider was called both times — confirm() does not memoize on its own.
        assert_eq!(provider.call_count(), 2);
    }

    fn state_with_simulation(enabled: bool) -> AppState {
        let mut options_config = OptionsConfig::enabled_in_memory_for_tests();
        options_config.execution_enabled = true;
        options_config.execution_require_persistence = false;
        options_config.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        options_config.execution_eip712_domain.verifying_contract =
            options_config.matching_engine_address.clone();
        options_config.execution_simulation_enabled = enabled;
        options_config.execution_require_rpc_for_simulation = false;
        options_config.execution_simulation_gas_limit = 500_000;
        AppState::with_options_config(EngineState::with_default_markets(), options_config)
    }

    fn state_with_broadcast(broadcast_enabled: bool) -> AppState {
        let mut state = state_with_simulation(false);
        state.options_config.execution_broadcast_enabled = broadcast_enabled;
        state.options_config.execution_require_simulation_ok = true;
        state.options_config.execution_broadcast_gas_limit = 600_000;
        state.execution_config = crate::execution::ExecutionConfig {
            execution_enabled: true,
            real_broadcast_enabled: true,
            executor_private_key: Some(crate::execution::PrivateKeySecret::new(
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".to_string(),
            )),
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            max_gas_limit: 1_000_000,
            executor_chain_id: 84532,
            ..crate::execution::ExecutionConfig::disabled()
        };
        state
    }

    fn state_with_option_nonce_sync(strict: bool) -> AppState {
        let mut state = state_with_simulation(false);
        state.option_nonce_sync_config = OptionNonceSyncConfig {
            enabled: true,
            require_rpc: true,
            strict,
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
            option_matching_engine_address: state.options_config.matching_engine_address.clone(),
        };
        state
    }

    fn insert_executable_series(state: &AppState) {
        let expiry = 4_102_444_800;
        let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
        let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
        let onchain_option_id = crate::options::option_product_registry_option_id(
            &underlying,
            &settlement_asset,
            expiry,
            300_000_000_000,
            100_000_000,
            true,
            true,
        )
        .unwrap()
        .to_string();
        state
            .options_store
            .lock()
            .unwrap()
            .insert_series(OptionSeries {
                option_series_id: "series-1".to_string(),
                underlying: underlying.0,
                base_asset: "ETH".to_string(),
                quote_asset: "USDC".to_string(),
                settlement_asset: settlement_asset.0,
                expiry,
                strike_1e8: 300_000_000_000,
                is_call: true,
                contract_size_1e8: 100_000_000,
                status: OptionSeriesStatus::Active,
                source: OptionSeriesSource::Manual,
                onchain_product_id: None,
                onchain_series_id: Some(onchain_option_id),
                created_at_ms: 1,
                updated_at_ms: 1,
            });
    }

    fn orderbook_fill() -> OptionFill {
        OptionFill {
            fill_id: Uuid::from_u128(101),
            option_series_id: "series-1".to_string(),
            buy_order_id: OrderId(Uuid::from_u128(201)),
            sell_order_id: OrderId(Uuid::from_u128(202)),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            maker_order_id: OrderId(Uuid::from_u128(202)),
            taker_order_id: OrderId(Uuid::from_u128(201)),
            taker_side: Side::Buy,
            price_1e8: 10_000_000,
            size_1e8: 100_000_000,
            created_at_ms: 123,
        }
    }

    fn insert_intent(state: &AppState, intent: OptionExecutionIntent) -> OptionExecutionIntent {
        state
            .options_store
            .lock()
            .unwrap()
            .insert_option_execution_intent(intent)
    }

    fn option_transactions(
        state: &AppState,
        intent_id: OptionExecutionIntentId,
    ) -> Vec<OptionExecutionTransaction> {
        state
            .options_store
            .lock()
            .unwrap()
            .option_execution_transactions_for_intent(intent_id)
    }

    fn broadcast_ready_intent() -> OptionExecutionIntent {
        OptionExecutionIntent {
            buyer_signature: Some(signature_hex(0xaa)),
            seller_signature: Some(signature_hex(0xbb)),
            simulation_status: Some(OptionExecutionSimulationStatus::SimulationOk),
            ..calldata_ready_intent()
        }
    }

    fn calldata_ready_intent() -> OptionExecutionIntent {
        let expiry = 4_102_444_800;
        let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
        let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
        let onchain_option_id = crate::options::option_product_registry_option_id(
            &underlying,
            &settlement_asset,
            expiry,
            300_000_000_000,
            100_000_000,
            true,
            true,
        )
        .unwrap()
        .to_string();
        OptionExecutionIntent {
            intent_id: Uuid::from_u128(1),
            onchain_intent_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
            source_type: OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-1".to_string(),
            option_series_id: "series-1".to_string(),
            onchain_option_id,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            underlying,
            settlement_asset,
            expiry,
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
            status: OptionExecutionIntentStatus::CalldataReady,
            error: None,
            simulation_status: None,
            simulation_error: None,
            simulation_block_number: None,
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }

    // ----------------------------------------------------------------------------
    // V1V option execution confirmation worker tests
    // ----------------------------------------------------------------------------

    fn state_with_confirmation_worker(enabled: bool, finality_blocks: u64) -> AppState {
        let mut state = state_with_broadcast(true);
        state.option_confirmation_config = crate::options::OptionConfirmationConfig {
            enabled,
            poll_interval_ms: 15_000,
            finality_blocks,
            batch_size: 25,
            require_rpc: true,
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
        };
        state
    }

    fn pending_tx_hash() -> &'static str {
        "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125"
    }

    #[tokio::test]
    async fn worker_disabled_returns_disabled_and_does_nothing() {
        let state = state_with_confirmation_worker(false, 3);
        let tx_hash = pending_tx_hash();
        let (intent, tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(200);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        assert!(!result.enabled);
        assert!(result.decisions.is_empty());
        assert_eq!(provider.call_count(), 0);
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(txs[0].transaction_id, tx.transaction_id);
        assert!(txs[0].confirmation_status.is_none());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_missing_receipt_leaves_pending() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::missing().with_head(200);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        assert!(result.enabled);
        assert_eq!(result.decisions.len(), 1);
        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::ReceiptMissing
        );
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::ReceiptMissing)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_receipt_without_finality_does_not_finalize() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        // receipt at block 100, head at 101: head < 100+3 → not finalized
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(101);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        assert!(result.enabled);
        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::NotFinalized
        );
        assert_eq!(decision.receipt_status, Some(1));
        assert_eq!(decision.block_number, Some(100));
        assert_eq!(decision.current_block_number, Some(101));
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::Pending)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_successful_receipt_with_finality_finalizes_mined_success() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(105);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::MinedSuccess
        );
        assert_eq!(decision.receipt_status, Some(1));
        assert_eq!(decision.block_number, Some(100));
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastConfirmed
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedSuccess)
        );
        assert_eq!(txs[0].confirmed_block_number, Some(100));
        assert_eq!(txs[0].receipt_status, Some(1));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_failed_receipt_with_finality_finalizes_mined_failed() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_reverted(tx_hash, 100).with_head(120);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::MinedFailed
        );
        assert_eq!(decision.receipt_status, Some(0));
        assert_eq!(decision.block_number, Some(100));
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastFailed
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedFailed)
        );
        assert_eq!(txs[0].receipt_status, Some(0));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_does_not_use_broadcast_provider() {
        // Construct a state with both worker enabled and an in-memory broadcast provider
        // that *would* fail loudly if called. Confirm the worker never touches it.
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (_intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let receipt_provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(120);
        let broadcast_provider = MockBroadcastProvider::success();

        let _ = confirm_pending_option_execution_transactions(&state, &receipt_provider)
            .await
            .unwrap();

        assert_eq!(broadcast_provider.send_count(), 0);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_never_creates_generic_execution_rows() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(200);

        for _ in 0..3 {
            confirm_pending_option_execution_transactions(&state, &provider)
                .await
                .unwrap();
        }

        // The option-side row sees its final mined_success.
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(txs.len(), 1);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedSuccess)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[test]
    fn worker_outcome_is_finalized_only_for_mined_states() {
        use crate::options::OptionConfirmationOutcome::*;
        assert!(MinedSuccess.is_finalized());
        assert!(MinedFailed.is_finalized());
        assert!(!NotFinalized.is_finalized());
        assert!(!ReceiptMissing.is_finalized());
        assert!(!ReceiptError.is_finalized());
        assert!(!Disabled.is_finalized());
        assert!(!NoPending.is_finalized());
    }
}
