use crate::api::dto::{ApiEngineEvent, ApiExecutionIntent, SubmitOrderResponse};
use crate::api::AppState;
use crate::db::PgRepository;
use crate::engine::{EngineCommand, EngineEvent};
use crate::error::{BackendError, Result};
use crate::execution::HttpJsonRpcProvider;
use crate::nonce_sync::validate_order_perp_nonce;
use crate::signing::{SignatureVerifier, SignedOrder};
use crate::types::{now_ms, AccountId, MarketId, NewOrder, Order, OrderId, TimestampMs};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOrderOutcome {
    pub status: String,
    pub order_id: Option<OrderId>,
    pub client_order_id: Option<String>,
    pub resting: bool,
    pub matched_intents: Vec<String>,
    pub events: Vec<EngineEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelOrderInput {
    pub account: Option<AccountId>,
    pub market_id: Option<MarketId>,
    pub order_id: Option<OrderId>,
    pub client_order_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelOrderOutcome {
    pub order_id: OrderId,
    pub client_order_id: Option<String>,
    pub events: Vec<EngineEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelRestingFilter {
    pub account: AccountId,
    pub market_id: Option<MarketId>,
    pub client_order_ids: Option<BTreeSet<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelRestingOutcome {
    pub cancelled: Vec<CancelOrderOutcome>,
}

pub async fn submit_signed_order(
    state: &AppState,
    signed_order: SignedOrder,
) -> Result<SubmitOrderOutcome> {
    validate_signed_order(state, &signed_order).await?;

    if let Some(repository) = state.repository.clone() {
        return submit_order_persistent(state, repository, signed_order).await;
    }

    {
        let mut nonces = state
            .nonces
            .lock()
            .map_err(|_| BackendError::Config("nonce lock poisoned".to_string()))?;
        nonces.reserve(&signed_order.account, signed_order.nonce)?;
    }

    let events = {
        let mut engine = state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
        engine.process(EngineCommand::SubmitOrder(NewOrder::from(
            signed_order.clone(),
        )))?
    };
    Ok(outcome_from_events(signed_order.client_order_id, events))
}

pub async fn cancel_order(state: &AppState, input: CancelOrderInput) -> Result<CancelOrderOutcome> {
    let order_id = resolve_cancel_order_id(state, &input)?;
    let events = {
        let mut engine = state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
        engine.process(EngineCommand::CancelOrder { order_id })?
    };
    if let Some(repository) = state.repository.clone() {
        repository.persist_engine_events(&events).await?;
    }

    let client_order_id = cancelled_client_order_id(&events).or(input.client_order_id);
    Ok(CancelOrderOutcome {
        order_id,
        client_order_id,
        events,
    })
}

pub async fn cancel_resting_orders(
    state: &AppState,
    filter: CancelRestingFilter,
) -> Result<CancelRestingOutcome> {
    let order_ids = {
        let engine = state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
        engine
            .resting_orders()
            .into_iter()
            .filter(|order| order.account == filter.account)
            .filter(|order| match filter.market_id {
                Some(market_id) => order.market_id == market_id,
                None => true,
            })
            .filter(|order| match filter.client_order_ids.as_ref() {
                Some(ids) => order
                    .client_order_id
                    .as_ref()
                    .is_some_and(|id| ids.contains(id)),
                None => true,
            })
            .map(|order| order.order_id)
            .collect::<Vec<_>>()
    };

    let mut all_events = Vec::new();
    let mut cancelled = Vec::with_capacity(order_ids.len());
    {
        let mut engine = state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
        for order_id in order_ids {
            match engine.process(EngineCommand::CancelOrder { order_id }) {
                Ok(events) => {
                    let client_order_id = cancelled_client_order_id(&events);
                    cancelled.push(CancelOrderOutcome {
                        order_id,
                        client_order_id,
                        events: events.clone(),
                    });
                    all_events.extend(events);
                }
                Err(BackendError::OrderNotFound(_) | BackendError::OrderNotOpen(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }

    if !all_events.is_empty() {
        if let Some(repository) = state.repository.clone() {
            repository.persist_engine_events(&all_events).await?;
        }
    }

    Ok(CancelRestingOutcome { cancelled })
}

pub fn submit_response_from_events(events: Vec<EngineEvent>) -> SubmitOrderResponse {
    let status = if events
        .iter()
        .any(|event| matches!(event, EngineEvent::OrderRejected { .. }))
    {
        "rejected"
    } else {
        "accepted"
    };
    let order_id = first_order_id(&events);
    let execution_intents = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::ExecutionIntentCreated { intent } => {
                Some(ApiExecutionIntent::from(intent.clone()))
            }
            _ => None,
        })
        .collect();
    let events = events.into_iter().map(ApiEngineEvent::from).collect();

    SubmitOrderResponse {
        status: status.to_string(),
        order_id,
        events,
        execution_intents,
    }
}

async fn validate_signed_order(state: &AppState, signed_order: &SignedOrder) -> Result<()> {
    validate_deadline(signed_order.deadline_ms)?;
    SignatureVerifier::verify(
        signed_order,
        state.signature_verification_mode,
        &state.eip712_domain,
    )?;

    {
        let engine = state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
        if !engine.has_market(signed_order.market_id) {
            return Err(BackendError::UnknownMarket(signed_order.market_id));
        }
    }

    validate_perp_nonce_before_local_reserve(state, signed_order).await
}

async fn submit_order_persistent(
    state: &AppState,
    repository: PgRepository,
    signed_order: SignedOrder,
) -> Result<SubmitOrderOutcome> {
    let mut tx = repository.begin().await?;
    repository
        .insert_nonce_tx(&mut tx, &signed_order.account, signed_order.nonce, now_ms())
        .await?;

    let events = {
        let mut engine = state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;
        engine.process(EngineCommand::SubmitOrder(NewOrder::from(
            signed_order.clone(),
        )))?
    };

    repository
        .persist_submission_tx(&mut tx, &signed_order, &events)
        .await?;
    tx.commit()
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

    Ok(outcome_from_events(signed_order.client_order_id, events))
}

fn resolve_cancel_order_id(state: &AppState, input: &CancelOrderInput) -> Result<OrderId> {
    let engine = state
        .engine
        .lock()
        .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?;

    if let Some(order_id) = input.order_id {
        let order = engine
            .order(order_id)
            .ok_or(BackendError::OrderNotFound(order_id))?;
        validate_cancel_ownership(&order, input)?;
        return Ok(order_id);
    }

    let client_order_id = input
        .client_order_id
        .as_deref()
        .ok_or(BackendError::InvalidOrderId)?;
    let order = engine
        .resting_orders()
        .into_iter()
        .find(|order| {
            order.client_order_id.as_deref() == Some(client_order_id)
                && match input.market_id {
                    Some(market_id) => order.market_id == market_id,
                    None => true,
                }
                && match input.account.as_ref() {
                    Some(account) => &order.account == account,
                    None => true,
                }
        })
        .ok_or_else(|| BackendError::Config("client order id is not open".to_string()))?;
    validate_cancel_ownership(&order, input)?;
    Ok(order.order_id)
}

fn validate_cancel_ownership(order: &Order, input: &CancelOrderInput) -> Result<()> {
    if let Some(account) = input.account.as_ref() {
        if &order.account != account {
            return Err(BackendError::Config(
                "order is not owned by this account".to_string(),
            ));
        }
    }
    if let Some(market_id) = input.market_id {
        if order.market_id != market_id {
            return Err(BackendError::Config(
                "order is not in requested market".to_string(),
            ));
        }
    }
    Ok(())
}

async fn validate_perp_nonce_before_local_reserve(
    state: &AppState,
    signed_order: &SignedOrder,
) -> Result<()> {
    if !state.perp_nonce_sync_config.enabled || !state.perp_nonce_sync_config.strict {
        return Ok(());
    }
    let rpc_url = state
        .perp_nonce_sync_config
        .rpc_url
        .clone()
        .ok_or_else(|| {
            BackendError::Config("RPC_URL is required for perp nonce sync".to_string())
        })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    validate_order_perp_nonce(
        &state.perp_nonce_sync_config,
        &provider,
        &signed_order.account,
        signed_order.nonce,
    )
    .await
}

fn outcome_from_events(
    client_order_id: Option<String>,
    events: Vec<EngineEvent>,
) -> SubmitOrderOutcome {
    let status = if events
        .iter()
        .any(|event| matches!(event, EngineEvent::OrderRejected { .. }))
    {
        "rejected"
    } else {
        "accepted"
    };
    let order_id = first_order_id(&events);
    let resting = events.iter().any(|event| match event {
        EngineEvent::OrderAccepted { order } | EngineEvent::OrderPartiallyFilled { order } => {
            order.remaining_size_1e8 > 0
        }
        _ => false,
    });
    let matched_intents = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::ExecutionIntentCreated { intent } => Some(intent.intent_id.to_string()),
            _ => None,
        })
        .collect();

    SubmitOrderOutcome {
        status: status.to_string(),
        order_id,
        client_order_id,
        resting,
        matched_intents,
        events,
    }
}

fn first_order_id(events: &[EngineEvent]) -> Option<OrderId> {
    events.iter().find_map(|event| match event {
        EngineEvent::OrderAccepted { order } => Some(order.order_id),
        EngineEvent::OrderRejected { order_id, .. } => Some(*order_id),
        EngineEvent::OrderCancelled { order } => Some(order.order_id),
        EngineEvent::OrderPartiallyFilled { order } => Some(order.order_id),
        EngineEvent::OrderFilled { order } => Some(order.order_id),
        EngineEvent::TradeMatched { .. } | EngineEvent::ExecutionIntentCreated { .. } => None,
    })
}

fn cancelled_client_order_id(events: &[EngineEvent]) -> Option<String> {
    events.iter().find_map(|event| match event {
        EngineEvent::OrderCancelled { order } => order.client_order_id.clone(),
        _ => None,
    })
}

fn validate_deadline(deadline_ms: TimestampMs) -> Result<()> {
    if deadline_ms <= now_ms() {
        return Err(BackendError::DeadlineExpired);
    }
    Ok(())
}
