use super::{QuoteId, RfqConfig, RfqId, RfqQuote, RfqQuoteStatus, RfqRequest, RfqStatus};
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::execution::{intent_id_to_hex_bytes32, ExecutionIntent, ExecutionIntentStatus};
use crate::nonce_sync::read_perp_nonce;
use crate::signing::eip712::parse_evm_address;
use crate::types::{now_ms, AccountId, MarketId, OrderId, Price1e8, Side, Size1e8, TimestampMs};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateRfqInput {
    pub taker: AccountId,
    pub market_id: MarketId,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub limit_price_1e8: Option<Price1e8>,
    pub ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitQuoteInput {
    pub rfq_id: RfqId,
    pub mm_account: AccountId,
    pub session_id: Option<String>,
    pub client_quote_id: Option<String>,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub quote_ttl_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptQuoteOutcome {
    pub rfq_id: RfqId,
    pub quote_id: QuoteId,
    pub status: RfqStatus,
    pub execution_intent_id: Uuid,
    pub onchain_intent_id: String,
}

pub async fn create_rfq(state: &AppState, input: CreateRfqInput) -> Result<RfqRequest> {
    ensure_enabled(&state.rfq_config)?;
    validate_account(&input.taker)?;
    validate_market(state, input.market_id)?;
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if input.limit_price_1e8 == Some(0) {
        return Err(BackendError::ZeroPrice);
    }
    let now = now_ms();
    let ttl_ms = input
        .ttl_ms
        .unwrap_or(state.rfq_config.default_ttl_ms)
        .min(state.rfq_config.max_ttl_ms);
    if ttl_ms == 0 {
        return Err(BackendError::InvalidRfqState(
            "RFQ ttl_ms must be nonzero".to_string(),
        ));
    }
    let expires_at_ms = checked_expiry(now, ttl_ms)?;
    let rfq = RfqRequest {
        rfq_id: Uuid::new_v4(),
        taker: input.taker,
        market_id: input.market_id,
        side: input.side,
        size_1e8: input.size_1e8,
        limit_price_1e8: input.limit_price_1e8,
        status: RfqStatus::Open,
        created_at_ms: now,
        expires_at_ms,
        accepted_quote_id: None,
        execution_intent_id: None,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_rfq(&rfq).await?;
    } else {
        state
            .rfq_store
            .lock()
            .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
            .insert_rfq(rfq.clone());
    }
    Ok(rfq)
}

pub async fn list_rfqs(state: &AppState) -> Result<Vec<RfqRequest>> {
    ensure_enabled(&state.rfq_config)?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_rfqs().await;
    }
    Ok(state
        .rfq_store
        .lock()
        .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
        .list_rfqs())
}

pub async fn get_rfq(state: &AppState, rfq_id: RfqId) -> Result<RfqRequest> {
    ensure_enabled(&state.rfq_config)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_rfq(rfq_id)
            .await?
            .ok_or(BackendError::InvalidRfqId);
    }
    state
        .rfq_store
        .lock()
        .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
        .get_rfq(rfq_id)
        .ok_or(BackendError::InvalidRfqId)
}

pub async fn submit_quote(state: &AppState, input: SubmitQuoteInput) -> Result<RfqQuote> {
    ensure_enabled(&state.rfq_config)?;
    validate_account(&input.mm_account)?;
    if input.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if input.quote_ttl_ms < state.rfq_config.min_quote_ttl_ms {
        return Err(BackendError::InvalidRfqQuoteState(
            "quote_ttl_ms is below RFQ_MIN_QUOTE_TTL_MS".to_string(),
        ));
    }

    let rfq = get_rfq(state, input.rfq_id).await?;
    let now = now_ms();
    validate_open_rfq(&rfq, now)?;
    if input.size_1e8 > rfq.size_1e8 {
        return Err(BackendError::InvalidRfqQuoteState(
            "quote size exceeds RFQ size".to_string(),
        ));
    }

    let current_quote_count = quote_count(state, input.rfq_id).await?;
    if current_quote_count >= state.rfq_config.max_quotes_per_rfq {
        return Err(BackendError::InvalidRfqQuoteState(
            "RFQ quote limit reached".to_string(),
        ));
    }

    let quote_ttl_ms = input.quote_ttl_ms.min(state.rfq_config.max_quote_ttl_ms);
    let expires_at_ms = checked_expiry(now, quote_ttl_ms)?.min(rfq.expires_at_ms);
    let quote = RfqQuote {
        quote_id: Uuid::new_v4(),
        rfq_id: input.rfq_id,
        mm_account: input.mm_account,
        session_id: input.session_id,
        client_quote_id: input.client_quote_id,
        price_1e8: input.price_1e8,
        size_1e8: input.size_1e8,
        status: RfqQuoteStatus::Active,
        created_at_ms: now,
        expires_at_ms,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_rfq_quote(&quote).await?;
    } else {
        state
            .rfq_store
            .lock()
            .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
            .insert_quote(quote.clone());
    }
    Ok(quote)
}

pub async fn list_quotes(state: &AppState, rfq_id: RfqId) -> Result<Vec<RfqQuote>> {
    ensure_enabled(&state.rfq_config)?;
    get_rfq(state, rfq_id).await?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_rfq_quotes(rfq_id).await;
    }
    Ok(state
        .rfq_store
        .lock()
        .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
        .list_quotes(rfq_id))
}

pub async fn accept_quote(
    state: &AppState,
    rfq_id: RfqId,
    quote_id: QuoteId,
) -> Result<AcceptQuoteOutcome> {
    ensure_enabled(&state.rfq_config)?;
    let rfq = get_rfq(state, rfq_id).await?;
    let quote = get_quote(state, quote_id).await?;
    let now = now_ms();
    validate_acceptance(&rfq, &quote, now)?;

    let (buyer, seller, buyer_is_maker) = rfq_execution_sides(&rfq, &quote);
    let (buyer_nonce, seller_nonce) = reserve_trade_nonces(state, &buyer, &seller).await?;
    let intent = ExecutionIntent {
        intent_id: Uuid::new_v4(),
        market_id: rfq.market_id,
        buyer,
        seller,
        price_1e8: quote.price_1e8,
        size_1e8: quote.size_1e8,
        buy_order_id: OrderId::new(),
        sell_order_id: OrderId::new(),
        buyer_is_maker: Some(buyer_is_maker),
        buyer_nonce: Some(buyer_nonce),
        seller_nonce: Some(seller_nonce),
        deadline_ms: Some(rfq.expires_at_ms.min(quote.expires_at_ms)),
        created_at_ms: now,
        status: ExecutionIntentStatus::Pending,
    };
    let onchain_intent_id = intent_id_to_hex_bytes32(&intent.intent_id.to_string())?;

    if let Some(repository) = state.repository.clone() {
        repository
            .accept_rfq_quote_and_insert_intent(rfq_id, quote_id, &intent, now)
            .await?;
    } else {
        {
            let mut store = state
                .rfq_store
                .lock()
                .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?;
            store.accept_quote(rfq_id, quote_id, intent.intent_id)?;
        }
        state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?
            .push_execution_intent(intent.clone());
    }

    Ok(AcceptQuoteOutcome {
        rfq_id,
        quote_id,
        status: RfqStatus::Accepted,
        execution_intent_id: intent.intent_id,
        onchain_intent_id,
    })
}

pub async fn cancel_rfq(state: &AppState, rfq_id: RfqId) -> Result<RfqRequest> {
    ensure_enabled(&state.rfq_config)?;
    if let Some(repository) = state.repository.clone() {
        return repository.cancel_rfq(rfq_id, now_ms()).await;
    }
    state
        .rfq_store
        .lock()
        .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
        .cancel_rfq(rfq_id)
}

fn ensure_enabled(config: &RfqConfig) -> Result<()> {
    if config.enabled {
        Ok(())
    } else {
        Err(BackendError::RfqDisabled)
    }
}

fn validate_account(account: &AccountId) -> Result<()> {
    parse_evm_address(account).map(|_| ())
}

fn validate_market(state: &AppState, market_id: MarketId) -> Result<()> {
    let engine = state
        .engine
        .lock()
        .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
    if engine.has_market(market_id) {
        Ok(())
    } else {
        Err(BackendError::UnknownMarket(market_id))
    }
}

fn validate_open_rfq(rfq: &RfqRequest, now: TimestampMs) -> Result<()> {
    if rfq.status != RfqStatus::Open {
        return Err(BackendError::InvalidRfqState(format!(
            "RFQ is {}",
            rfq.status.as_str()
        )));
    }
    if now >= rfq.expires_at_ms {
        return Err(BackendError::InvalidRfqState("RFQ has expired".to_string()));
    }
    Ok(())
}

fn validate_acceptance(rfq: &RfqRequest, quote: &RfqQuote, now: TimestampMs) -> Result<()> {
    validate_open_rfq(rfq, now)?;
    if quote.rfq_id != rfq.rfq_id {
        return Err(BackendError::InvalidRfqQuoteState(
            "quote does not belong to RFQ".to_string(),
        ));
    }
    if quote.status != RfqQuoteStatus::Active {
        return Err(BackendError::InvalidRfqQuoteState(format!(
            "quote is {}",
            quote.status.as_str()
        )));
    }
    if now >= quote.expires_at_ms {
        return Err(BackendError::InvalidRfqQuoteState(
            "quote has expired".to_string(),
        ));
    }
    if quote.size_1e8 == 0 || quote.size_1e8 > rfq.size_1e8 {
        return Err(BackendError::InvalidRfqQuoteState(
            "quote size is incompatible with RFQ".to_string(),
        ));
    }
    if !price_satisfies_limit(rfq.side, rfq.limit_price_1e8, quote.price_1e8) {
        return Err(BackendError::InvalidRfqQuoteState(
            "quote price is beyond taker limit".to_string(),
        ));
    }
    Ok(())
}

fn price_satisfies_limit(side: Side, limit: Option<Price1e8>, price: Price1e8) -> bool {
    match (side, limit) {
        (_, None) => true,
        (Side::Buy, Some(limit)) => price <= limit,
        (Side::Sell, Some(limit)) => price >= limit,
    }
}

pub fn rfq_execution_sides(rfq: &RfqRequest, quote: &RfqQuote) -> (AccountId, AccountId, bool) {
    match rfq.side {
        Side::Buy => (rfq.taker.clone(), quote.mm_account.clone(), false),
        Side::Sell => (quote.mm_account.clone(), rfq.taker.clone(), true),
    }
}

async fn reserve_trade_nonces(
    state: &AppState,
    buyer: &AccountId,
    seller: &AccountId,
) -> Result<(u64, u64)> {
    if state.perp_nonce_sync_config.enabled && state.perp_nonce_sync_config.strict {
        let rpc_url = state
            .perp_nonce_sync_config
            .rpc_url
            .clone()
            .ok_or_else(|| {
                BackendError::Config("RPC_URL is required for perp nonce sync".to_string())
            })?;
        let provider = crate::execution::HttpJsonRpcProvider::new(rpc_url);
        let buyer_nonce = read_perp_nonce(&state.perp_nonce_sync_config, &provider, buyer)
            .await?
            .nonce;
        let seller_nonce = read_perp_nonce(&state.perp_nonce_sync_config, &provider, seller)
            .await?
            .nonce;
        if state.repository.is_some() {
            return Ok((buyer_nonce, seller_nonce));
        }
        return reserve_explicit_trade_nonces(state, buyer, buyer_nonce, seller, seller_nonce)
            .await;
    }

    if let Some(repository) = state.repository.clone() {
        return repository.next_trade_nonces(buyer, seller).await;
    }

    let mut nonces = state
        .nonces
        .lock()
        .map_err(|_| BackendError::Config("nonce lock poisoned".to_string()))?;
    let buyer_nonce = nonces.reserve_next(buyer)?;
    let seller_nonce = nonces.reserve_next(seller)?;
    Ok((buyer_nonce, seller_nonce))
}

async fn reserve_explicit_trade_nonces(
    state: &AppState,
    buyer: &AccountId,
    buyer_nonce: u64,
    seller: &AccountId,
    seller_nonce: u64,
) -> Result<(u64, u64)> {
    if let Some(repository) = state.repository.clone() {
        repository
            .reserve_trade_nonces(buyer, buyer_nonce, seller, seller_nonce, now_ms())
            .await?;
    } else {
        let mut nonces = state
            .nonces
            .lock()
            .map_err(|_| BackendError::Config("nonce lock poisoned".to_string()))?;
        nonces.reserve(buyer, buyer_nonce)?;
        nonces.reserve(seller, seller_nonce)?;
    }
    Ok((buyer_nonce, seller_nonce))
}

async fn get_quote(state: &AppState, quote_id: QuoteId) -> Result<RfqQuote> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_rfq_quote(quote_id)
            .await?
            .ok_or(BackendError::InvalidRfqQuoteId);
    }
    state
        .rfq_store
        .lock()
        .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
        .get_quote(quote_id)
        .ok_or(BackendError::InvalidRfqQuoteId)
}

async fn quote_count(state: &AppState, rfq_id: RfqId) -> Result<usize> {
    if let Some(repository) = state.repository.clone() {
        return repository.count_rfq_quotes(rfq_id).await;
    }
    Ok(state
        .rfq_store
        .lock()
        .map_err(|_| BackendError::Config("RFQ store lock poisoned".to_string()))?
        .quote_count(rfq_id))
}

fn checked_expiry(now: TimestampMs, ttl_ms: u64) -> Result<TimestampMs> {
    let ttl_ms = i64::try_from(ttl_ms)
        .map_err(|_| BackendError::InvalidRfqState("ttl_ms exceeds i64".to_string()))?;
    now.checked_add(ttl_ms)
        .ok_or_else(|| BackendError::InvalidRfqState("expires_at_ms overflow".to_string()))
}
