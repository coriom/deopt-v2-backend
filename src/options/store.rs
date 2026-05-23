use super::{
    OptionEventIndexerState, OptionExecutionConfirmationStatus, OptionExecutionEvent,
    OptionExecutionEventLink, OptionExecutionIntent, OptionExecutionIntentId,
    OptionExecutionIntentStatus, OptionExecutionReceiptCost, OptionExecutionSimulationResult,
    OptionExecutionSourceType, OptionExecutionTransaction, OptionFill, OptionFillFilter,
    OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderId, OptionOrderStatus, OptionRfqFill,
    OptionRfqId, OptionRfqQuote, OptionRfqQuoteId, OptionRfqQuoteStatus, OptionRfqRequest,
    OptionRfqStatus, OptionSeries, OptionSeriesFilter, OptionSeriesId, OptionSeriesStatus,
};
use crate::error::{BackendError, Result};
use crate::execution::ExecutionTransactionStatus;
use crate::types::{Side, TimestampMs};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct OptionSeriesStore {
    series: HashMap<OptionSeriesId, OptionSeries>,
    orders: HashMap<OptionOrderId, OptionOrder>,
    fills: HashMap<OptionFillId, OptionFill>,
    option_rfqs: HashMap<OptionRfqId, OptionRfqRequest>,
    option_rfq_quotes: HashMap<OptionRfqQuoteId, OptionRfqQuote>,
    option_rfq_fills: HashMap<Uuid, OptionRfqFill>,
    option_execution_intents: HashMap<OptionExecutionIntentId, OptionExecutionIntent>,
    option_execution_intents_by_source: HashMap<(String, String), OptionExecutionIntentId>,
    option_execution_transactions: HashMap<String, OptionExecutionTransaction>,
    option_execution_events: HashMap<(u64, String, u64), OptionExecutionEvent>,
    option_event_indexer_states: HashMap<String, OptionEventIndexerState>,
}

impl OptionSeriesStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_series(&mut self, series: OptionSeries) -> OptionSeries {
        if let Some(existing) = self.series.get(&series.option_series_id) {
            return existing.clone();
        }
        self.series
            .insert(series.option_series_id.clone(), series.clone());
        series
    }

    pub fn list_series(&self, filter: &OptionSeriesFilter, now_sec: u64) -> Vec<OptionSeries> {
        let mut series = self
            .series
            .values()
            .filter(|series| filter.matches(series, now_sec))
            .cloned()
            .collect::<Vec<_>>();
        series.sort_by(|left, right| {
            left.expiry
                .cmp(&right.expiry)
                .then_with(|| left.strike_1e8.cmp(&right.strike_1e8))
                .then_with(|| left.option_series_id.cmp(&right.option_series_id))
        });
        series
    }

    pub fn get_series(&self, option_series_id: &str) -> Option<OptionSeries> {
        self.series.get(option_series_id).cloned()
    }

    pub fn disable_series(
        &mut self,
        option_series_id: &str,
        updated_at_ms: i64,
    ) -> Result<OptionSeries> {
        let series = self
            .series
            .get_mut(option_series_id)
            .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))?;
        series.status = OptionSeriesStatus::Disabled;
        series.updated_at_ms = updated_at_ms;
        Ok(series.clone())
    }

    pub fn insert_order(&mut self, order: OptionOrder) -> Result<OptionOrder> {
        if self.has_open_client_order(&order.account.0, order.client_order_id.as_deref()) {
            return Err(BackendError::InvalidOptionOrderState(
                "duplicate open client_order_id for option account".to_string(),
            ));
        }
        self.orders.insert(order.order_id, order.clone());
        Ok(order)
    }

    pub fn submit_order_and_match(
        &mut self,
        mut incoming: OptionOrder,
        updated_at_ms: TimestampMs,
    ) -> Result<(OptionOrder, Vec<OptionFill>)> {
        if self.has_open_client_order(&incoming.account.0, incoming.client_order_id.as_deref()) {
            return Err(BackendError::InvalidOptionOrderState(
                "duplicate open client_order_id for option account".to_string(),
            ));
        }

        let mut makers = self
            .orders
            .values()
            .filter(|order| can_match(&incoming, order))
            .cloned()
            .collect::<Vec<_>>();
        sort_match_candidates(&mut makers, incoming.side);

        let mut fills = Vec::new();
        for maker_snapshot in makers {
            if incoming.remaining_size_1e8 == 0 {
                break;
            }
            let Some(maker) = self.orders.get_mut(&maker_snapshot.order_id) else {
                continue;
            };
            if !can_match(&incoming, maker) {
                continue;
            }

            let fill_size = incoming.remaining_size_1e8.min(maker.remaining_size_1e8);
            if fill_size == 0 {
                continue;
            }

            let fill = option_fill_from_match(&incoming, maker, fill_size, updated_at_ms);
            incoming.remaining_size_1e8 -= fill_size;
            maker.remaining_size_1e8 -= fill_size;
            maker.status = status_for_remaining(maker.size_1e8, maker.remaining_size_1e8);
            maker.updated_at_ms = updated_at_ms;
            self.fills.insert(fill.fill_id, fill.clone());
            fills.push(fill);
        }

        incoming.status = status_for_remaining(incoming.size_1e8, incoming.remaining_size_1e8);
        incoming.updated_at_ms = updated_at_ms;
        self.orders.insert(incoming.order_id, incoming.clone());
        Ok((incoming, fills))
    }

    pub fn list_orders(&self, filter: &OptionOrderFilter) -> Vec<OptionOrder> {
        let mut orders = self
            .orders
            .values()
            .filter(|order| filter.matches(order))
            .cloned()
            .collect::<Vec<_>>();
        orders.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.order_id.cmp(&right.order_id))
        });
        orders
    }

    pub fn get_order(&self, order_id: OptionOrderId) -> Option<OptionOrder> {
        self.orders.get(&order_id).cloned()
    }

    pub fn cancel_order(
        &mut self,
        order_id: OptionOrderId,
        updated_at_ms: i64,
    ) -> Result<OptionOrder> {
        let order = self
            .orders
            .get_mut(&order_id)
            .ok_or(BackendError::InvalidOptionOrderId)?;
        if !order.status.is_live() {
            return Err(BackendError::InvalidOptionOrderState(format!(
                "option order is {}",
                order.status.as_str()
            )));
        }
        order.status = OptionOrderStatus::Cancelled;
        order.updated_at_ms = updated_at_ms;
        Ok(order.clone())
    }

    pub fn open_orders_for_series(&self, option_series_id: &str) -> Vec<OptionOrder> {
        self.orders
            .values()
            .filter(|order| {
                order.option_series_id == option_series_id
                    && order.status.is_live()
                    && (order.side == Side::Buy || order.side == Side::Sell)
            })
            .cloned()
            .collect()
    }

    pub fn list_fills(&self, filter: &OptionFillFilter) -> Vec<OptionFill> {
        let mut fills = self
            .fills
            .values()
            .filter(|fill| filter.matches(fill))
            .cloned()
            .collect::<Vec<_>>();
        fills.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.fill_id.cmp(&right.fill_id))
        });
        fills
    }

    pub fn get_fill(&self, fill_id: OptionFillId) -> Option<OptionFill> {
        self.fills.get(&fill_id).cloned()
    }

    pub fn fills_for_order(&self, order_id: OptionOrderId) -> Vec<OptionFill> {
        self.list_fills(&OptionFillFilter {
            order_id: Some(order_id),
            ..OptionFillFilter::default()
        })
    }

    pub fn insert_option_rfq(&mut self, rfq: OptionRfqRequest) -> OptionRfqRequest {
        self.option_rfqs.insert(rfq.option_rfq_id, rfq.clone());
        rfq
    }

    pub fn list_option_rfqs(&self) -> Vec<OptionRfqRequest> {
        let mut rfqs = self.option_rfqs.values().cloned().collect::<Vec<_>>();
        rfqs.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.option_rfq_id.cmp(&right.option_rfq_id))
        });
        rfqs
    }

    pub fn get_option_rfq(&self, option_rfq_id: OptionRfqId) -> Option<OptionRfqRequest> {
        self.option_rfqs.get(&option_rfq_id).cloned()
    }

    pub fn insert_option_rfq_quote(&mut self, quote: OptionRfqQuote) -> Result<OptionRfqQuote> {
        if self.has_duplicate_option_rfq_client_quote(
            quote.option_rfq_id,
            &quote.mm_account.0,
            quote.client_quote_id.as_deref(),
        ) {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "duplicate client_quote_id for option RFQ and MM account".to_string(),
            ));
        }
        self.option_rfq_quotes.insert(quote.quote_id, quote.clone());
        Ok(quote)
    }

    pub fn count_option_rfq_quotes(&self, option_rfq_id: OptionRfqId) -> usize {
        self.option_rfq_quotes
            .values()
            .filter(|quote| quote.option_rfq_id == option_rfq_id)
            .count()
    }

    pub fn list_option_rfq_quotes(&self, option_rfq_id: OptionRfqId) -> Vec<OptionRfqQuote> {
        let mut quotes = self
            .option_rfq_quotes
            .values()
            .filter(|quote| quote.option_rfq_id == option_rfq_id)
            .cloned()
            .collect::<Vec<_>>();
        quotes.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.quote_id.cmp(&right.quote_id))
        });
        quotes
    }

    pub fn get_option_rfq_quote(&self, quote_id: OptionRfqQuoteId) -> Option<OptionRfqQuote> {
        self.option_rfq_quotes.get(&quote_id).cloned()
    }

    pub fn accept_option_rfq_quote(
        &mut self,
        option_rfq_id: OptionRfqId,
        quote_id: OptionRfqQuoteId,
        fill: OptionRfqFill,
    ) -> Result<(OptionRfqRequest, OptionRfqQuote)> {
        let rfq = self
            .option_rfqs
            .get_mut(&option_rfq_id)
            .ok_or(BackendError::InvalidOptionRfqId)?;
        if rfq.status != OptionRfqStatus::Open || rfq.accepted_quote_id.is_some() {
            return Err(BackendError::InvalidOptionRfqState(
                "option RFQ is no longer open".to_string(),
            ));
        }

        let quote = self
            .option_rfq_quotes
            .get_mut(&quote_id)
            .ok_or(BackendError::InvalidOptionRfqQuoteId)?;
        if quote.option_rfq_id != option_rfq_id || quote.status != OptionRfqQuoteStatus::Active {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "option RFQ quote is no longer active".to_string(),
            ));
        }

        rfq.status = OptionRfqStatus::Accepted;
        rfq.accepted_quote_id = Some(quote_id);
        rfq.option_fill_id = Some(fill.fill_id);
        quote.status = OptionRfqQuoteStatus::Accepted;

        for competing in self.option_rfq_quotes.values_mut() {
            if competing.option_rfq_id == option_rfq_id
                && competing.quote_id != quote_id
                && competing.status == OptionRfqQuoteStatus::Active
            {
                competing.status = OptionRfqQuoteStatus::Rejected;
            }
        }

        self.option_rfq_fills.insert(fill.fill_id, fill);
        let rfq = self
            .option_rfqs
            .get(&option_rfq_id)
            .ok_or(BackendError::InvalidOptionRfqId)?
            .clone();
        let quote = self
            .option_rfq_quotes
            .get(&quote_id)
            .ok_or(BackendError::InvalidOptionRfqQuoteId)?
            .clone();
        Ok((rfq, quote))
    }

    pub fn get_option_rfq_fill(&self, fill_id: Uuid) -> Option<OptionRfqFill> {
        self.option_rfq_fills.get(&fill_id).cloned()
    }

    pub fn list_option_rfq_fills(&self) -> Vec<OptionRfqFill> {
        let mut fills = self.option_rfq_fills.values().cloned().collect::<Vec<_>>();
        fills.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.fill_id.cmp(&right.fill_id))
        });
        fills
    }

    pub fn insert_option_execution_intent(
        &mut self,
        intent: OptionExecutionIntent,
    ) -> OptionExecutionIntent {
        let source_key = option_execution_source_key(intent.source_type, &intent.source_id);
        if let Some(existing_id) = self.option_execution_intents_by_source.get(&source_key) {
            if let Some(existing) = self.option_execution_intents.get(existing_id) {
                return existing.clone();
            }
        }
        self.option_execution_intents_by_source
            .insert(source_key, intent.intent_id);
        self.option_execution_intents
            .insert(intent.intent_id, intent.clone());
        intent
    }

    pub fn list_option_execution_intents(&self) -> Vec<OptionExecutionIntent> {
        let mut intents = self
            .option_execution_intents
            .values()
            .cloned()
            .collect::<Vec<_>>();
        intents.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.intent_id.cmp(&right.intent_id))
        });
        intents
    }

    pub fn get_option_execution_intent(
        &self,
        intent_id: OptionExecutionIntentId,
    ) -> Option<OptionExecutionIntent> {
        self.option_execution_intents.get(&intent_id).cloned()
    }

    pub fn get_option_execution_intent_by_source(
        &self,
        source_type: OptionExecutionSourceType,
        source_id: &str,
    ) -> Option<OptionExecutionIntent> {
        let source_key = option_execution_source_key(source_type, source_id);
        self.option_execution_intents_by_source
            .get(&source_key)
            .and_then(|intent_id| self.option_execution_intents.get(intent_id))
            .cloned()
    }

    pub fn upsert_option_execution_signatures(
        &mut self,
        intent_id: OptionExecutionIntentId,
        buyer_signature: Option<String>,
        seller_signature: Option<String>,
        status: OptionExecutionIntentStatus,
        calldata: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<OptionExecutionIntent> {
        let intent = self
            .option_execution_intents
            .get_mut(&intent_id)
            .ok_or(BackendError::InvalidOptionExecutionIntentId)?;
        if buyer_signature.is_some() {
            intent.buyer_signature = buyer_signature;
        }
        if seller_signature.is_some() {
            intent.seller_signature = seller_signature;
        }
        if calldata.is_some() {
            intent.calldata = calldata;
        }
        clear_option_execution_simulation(intent);
        intent.status = status;
        intent.updated_at_ms = updated_at_ms;
        Ok(intent.clone())
    }

    pub fn persist_option_execution_simulation_result(
        &mut self,
        result: &OptionExecutionSimulationResult,
    ) -> Result<OptionExecutionIntent> {
        let intent = self
            .option_execution_intents
            .get_mut(&result.intent_id)
            .ok_or(BackendError::InvalidOptionExecutionIntentId)?;
        intent.simulation_status = Some(result.simulation_status);
        intent.simulation_error = result.error.clone();
        intent.simulation_block_number = result.block_number;
        intent.simulation_revert_data = result.revert_data.clone();
        intent.simulation_revert_selector = result.revert_selector.clone();
        intent.simulated_at_ms = Some(result.simulated_at_ms);
        intent.updated_at_ms = result.simulated_at_ms;
        Ok(intent.clone())
    }

    pub fn update_option_execution_intent_status(
        &mut self,
        intent_id: OptionExecutionIntentId,
        status: OptionExecutionIntentStatus,
        error: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<OptionExecutionIntent> {
        let intent = self
            .option_execution_intents
            .get_mut(&intent_id)
            .ok_or(BackendError::InvalidOptionExecutionIntentId)?;
        intent.status = status;
        intent.error = error;
        intent.updated_at_ms = updated_at_ms;
        Ok(intent.clone())
    }

    pub fn insert_option_execution_transaction(
        &mut self,
        transaction: OptionExecutionTransaction,
    ) -> Result<OptionExecutionTransaction> {
        if transaction.status == ExecutionTransactionStatus::Submitted
            && self
                .find_submitted_option_execution_transaction_by_intent(transaction.intent_id)
                .is_some()
        {
            return Err(BackendError::BroadcastRejected(
                "option execution intent already has a submitted transaction".to_string(),
            ));
        }
        self.option_execution_transactions
            .insert(transaction.transaction_id.clone(), transaction.clone());
        Ok(transaction)
    }

    pub fn find_submitted_option_execution_transaction_by_intent(
        &self,
        intent_id: OptionExecutionIntentId,
    ) -> Option<OptionExecutionTransaction> {
        let mut transactions = self
            .option_execution_transactions
            .values()
            .filter(|transaction| {
                transaction.intent_id == intent_id
                    && transaction.status == ExecutionTransactionStatus::Submitted
            })
            .cloned()
            .collect::<Vec<_>>();
        transactions.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| right.transaction_id.cmp(&left.transaction_id))
        });
        transactions.into_iter().next()
    }

    pub fn list_pending_option_execution_transactions(
        &self,
        limit: u32,
    ) -> Vec<OptionExecutionTransaction> {
        let mut transactions = self
            .option_execution_transactions
            .values()
            .filter(|tx| {
                tx.status == ExecutionTransactionStatus::Submitted
                    && tx
                        .tx_hash
                        .as_deref()
                        .map(|hash| !hash.is_empty())
                        .unwrap_or(false)
                    && matches!(
                        tx.confirmation_status,
                        None | Some(OptionExecutionConfirmationStatus::Pending)
                            | Some(OptionExecutionConfirmationStatus::ReceiptMissing)
                            | Some(OptionExecutionConfirmationStatus::ReceiptError)
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        transactions.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.transaction_id.cmp(&right.transaction_id))
        });
        transactions.truncate(limit as usize);
        transactions
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_option_execution_confirmation(
        &mut self,
        transaction_id: &str,
        confirmation_status: OptionExecutionConfirmationStatus,
        confirmed_at_ms: TimestampMs,
        confirmed_block_number: Option<u64>,
        receipt_status: Option<u64>,
        confirmation_error: Option<String>,
        receipt_cost: &OptionExecutionReceiptCost,
    ) -> Result<OptionExecutionTransaction> {
        let transaction = self
            .option_execution_transactions
            .get_mut(transaction_id)
            .ok_or_else(|| {
                BackendError::Persistence(format!(
                    "option execution transaction {transaction_id} not found"
                ))
            })?;
        transaction.confirmation_status = Some(confirmation_status);
        transaction.confirmed_at_ms = Some(confirmed_at_ms);
        transaction.confirmed_block_number = confirmed_block_number;
        transaction.receipt_status = receipt_status;
        transaction.confirmation_error = confirmation_error;
        if let Some(value) = receipt_cost.gas_used {
            transaction.gas_used = Some(value);
        }
        if let Some(value) = receipt_cost.effective_gas_price.clone() {
            transaction.effective_gas_price = Some(value);
        }
        if let Some(value) = receipt_cost.cumulative_gas_used {
            transaction.cumulative_gas_used = Some(value);
        }
        if let Some(value) = receipt_cost.block_hash.clone() {
            transaction.receipt_block_hash = Some(value);
        }
        if let Some(value) = receipt_cost.transaction_index {
            transaction.receipt_transaction_index = Some(value);
        }
        if let Some(value) = receipt_cost.observed_at_ms {
            transaction.receipt_observed_at_ms = Some(value);
        }
        transaction.updated_at_ms = confirmed_at_ms;
        Ok(transaction.clone())
    }

    /// Aggregate counts by `confirmation_status` (NULL bucketed as "pending").
    pub fn summarize_option_execution_confirmations(&self) -> Vec<(String, u64)> {
        let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for tx in self.option_execution_transactions.values() {
            let key = match tx.confirmation_status {
                Some(status) => status.as_str().to_string(),
                None => "pending".to_string(),
            };
            *counts.entry(key).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    pub fn find_option_execution_event_link(
        &self,
        tx_hash: &str,
        onchain_intent_id: Option<&str>,
    ) -> OptionExecutionEventLink {
        if !tx_hash.is_empty() {
            let mut transactions = self
                .option_execution_transactions
                .values()
                .filter(|transaction| {
                    transaction
                        .tx_hash
                        .as_deref()
                        .map(|hash| hash.eq_ignore_ascii_case(tx_hash))
                        .unwrap_or(false)
                })
                .cloned()
                .collect::<Vec<_>>();
            transactions.sort_by(|left, right| {
                right
                    .created_at_ms
                    .cmp(&left.created_at_ms)
                    .then_with(|| right.transaction_id.cmp(&left.transaction_id))
            });
            if let Some(transaction) = transactions.into_iter().next() {
                return OptionExecutionEventLink {
                    intent_id: Some(transaction.intent_id),
                    option_execution_transaction_id: Some(transaction.transaction_id),
                };
            }
        }

        if let Some(onchain_intent_id) = onchain_intent_id {
            let mut transactions = self
                .option_execution_transactions
                .values()
                .filter(|transaction| {
                    transaction
                        .onchain_intent_id
                        .as_deref()
                        .map(|value| value.eq_ignore_ascii_case(onchain_intent_id))
                        .unwrap_or(false)
                })
                .cloned()
                .collect::<Vec<_>>();
            transactions.sort_by(|left, right| {
                right
                    .created_at_ms
                    .cmp(&left.created_at_ms)
                    .then_with(|| right.transaction_id.cmp(&left.transaction_id))
            });
            if let Some(transaction) = transactions.into_iter().next() {
                return OptionExecutionEventLink {
                    intent_id: Some(transaction.intent_id),
                    option_execution_transaction_id: Some(transaction.transaction_id),
                };
            }

            let mut intents = self
                .option_execution_intents
                .values()
                .filter(|intent| {
                    intent
                        .onchain_intent_id
                        .eq_ignore_ascii_case(onchain_intent_id)
                })
                .cloned()
                .collect::<Vec<_>>();
            intents.sort_by(|left, right| {
                right
                    .created_at_ms
                    .cmp(&left.created_at_ms)
                    .then_with(|| right.intent_id.cmp(&left.intent_id))
            });
            if let Some(intent) = intents.into_iter().next() {
                return OptionExecutionEventLink {
                    intent_id: Some(intent.intent_id),
                    option_execution_transaction_id: None,
                };
            }
        }

        OptionExecutionEventLink::default()
    }

    pub fn get_option_event_indexer_state(&self, id: &str) -> Option<OptionEventIndexerState> {
        self.option_event_indexer_states.get(id).cloned()
    }

    pub fn persist_option_execution_events_and_cursor(
        &mut self,
        state_id: &str,
        events: &[OptionExecutionEvent],
        last_indexed_block: u64,
        updated_at_ms: i64,
    ) -> u64 {
        let mut inserted = 0u64;
        for event in events {
            let key = event.idempotency_key();
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.option_execution_events.entry(key)
            {
                entry.insert(event.clone());
                inserted += 1;
            }
        }
        self.option_event_indexer_states.insert(
            state_id.to_string(),
            OptionEventIndexerState {
                id: state_id.to_string(),
                last_indexed_block,
                updated_at_ms,
                last_error: None,
            },
        );
        inserted
    }

    pub fn summarize_option_execution_events(&self) -> Vec<(String, u64)> {
        let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for event in self.option_execution_events.values() {
            *counts.entry(event.event_name.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    pub fn list_option_execution_events(&self, limit: u32) -> Vec<OptionExecutionEvent> {
        let mut events = self
            .option_execution_events
            .values()
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            right
                .block_number
                .cmp(&left.block_number)
                .then_with(|| right.log_index.cmp(&left.log_index))
                .then_with(|| right.id.cmp(&left.id))
        });
        events.truncate(limit as usize);
        events
    }

    pub fn option_execution_transactions_for_intent(
        &self,
        intent_id: OptionExecutionIntentId,
    ) -> Vec<OptionExecutionTransaction> {
        let mut transactions = self
            .option_execution_transactions
            .values()
            .filter(|transaction| transaction.intent_id == intent_id)
            .cloned()
            .collect::<Vec<_>>();
        transactions.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| right.transaction_id.cmp(&left.transaction_id))
        });
        transactions
    }

    pub fn cancel_option_rfq(&mut self, option_rfq_id: OptionRfqId) -> Result<OptionRfqRequest> {
        let rfq = self
            .option_rfqs
            .get_mut(&option_rfq_id)
            .ok_or(BackendError::InvalidOptionRfqId)?;
        if rfq.status == OptionRfqStatus::Accepted {
            return Err(BackendError::InvalidOptionRfqState(
                "accepted option RFQ cannot be cancelled".to_string(),
            ));
        }
        rfq.status = OptionRfqStatus::Cancelled;
        for quote in self.option_rfq_quotes.values_mut() {
            if quote.option_rfq_id == option_rfq_id && quote.status == OptionRfqQuoteStatus::Active
            {
                quote.status = OptionRfqQuoteStatus::Cancelled;
            }
        }
        Ok(rfq.clone())
    }

    fn has_open_client_order(&self, account: &str, client_order_id: Option<&str>) -> bool {
        let Some(client_order_id) = client_order_id else {
            return false;
        };
        self.orders.values().any(|order| {
            order.status.is_live()
                && order.account.0.eq_ignore_ascii_case(account)
                && order.client_order_id.as_deref() == Some(client_order_id)
        })
    }

    fn has_duplicate_option_rfq_client_quote(
        &self,
        option_rfq_id: OptionRfqId,
        mm_account: &str,
        client_quote_id: Option<&str>,
    ) -> bool {
        let Some(client_quote_id) = client_quote_id else {
            return false;
        };
        self.option_rfq_quotes.values().any(|quote| {
            quote.option_rfq_id == option_rfq_id
                && quote.mm_account.0.eq_ignore_ascii_case(mm_account)
                && quote.client_quote_id.as_deref() == Some(client_quote_id)
        })
    }
}

fn option_execution_source_key(
    source_type: OptionExecutionSourceType,
    source_id: &str,
) -> (String, String) {
    (source_type.as_str().to_string(), source_id.to_string())
}

fn clear_option_execution_simulation(intent: &mut OptionExecutionIntent) {
    intent.simulation_status = None;
    intent.simulation_error = None;
    intent.simulation_block_number = None;
    intent.simulation_revert_data = None;
    intent.simulation_revert_selector = None;
    intent.simulated_at_ms = None;
}

fn can_match(incoming: &OptionOrder, resting: &OptionOrder) -> bool {
    incoming.option_series_id == resting.option_series_id
        && incoming.side != resting.side
        && resting.status.is_live()
        && resting.remaining_size_1e8 > 0
        && match incoming.side {
            Side::Buy => incoming.price_1e8 >= resting.price_1e8,
            Side::Sell => incoming.price_1e8 <= resting.price_1e8,
        }
}

fn sort_match_candidates(orders: &mut [OptionOrder], taker_side: Side) {
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

pub(crate) fn status_for_remaining(size_1e8: u128, remaining_size_1e8: u128) -> OptionOrderStatus {
    if remaining_size_1e8 == 0 {
        OptionOrderStatus::Filled
    } else if remaining_size_1e8 < size_1e8 {
        OptionOrderStatus::PartiallyFilled
    } else {
        OptionOrderStatus::Open
    }
}

fn option_fill_from_match(
    incoming: &OptionOrder,
    maker: &OptionOrder,
    size_1e8: u128,
    created_at_ms: TimestampMs,
) -> OptionFill {
    let (buy_order, sell_order) = match incoming.side {
        Side::Buy => (incoming, maker),
        Side::Sell => (maker, incoming),
    };
    OptionFill {
        fill_id: Uuid::new_v4(),
        option_series_id: incoming.option_series_id.clone(),
        buy_order_id: buy_order.order_id,
        sell_order_id: sell_order.order_id,
        buyer: buy_order.account.clone(),
        seller: sell_order.account.clone(),
        maker_order_id: maker.order_id,
        taker_order_id: incoming.order_id,
        taker_side: incoming.side,
        price_1e8: maker.price_1e8,
        size_1e8,
        created_at_ms,
    }
}
