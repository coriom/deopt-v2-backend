use super::{
    terminal_reason, OptionEventIndexerState, OptionExecutionConfirmationStatus,
    OptionExecutionEvent, OptionExecutionEventLink, OptionExecutionIntent, OptionExecutionIntentId,
    OptionExecutionIntentStatus, OptionExecutionReceiptCost, OptionExecutionReconciliation,
    OptionExecutionSimulationResult, OptionExecutionSourceType, OptionExecutionTransaction,
    OptionFill, OptionFillFilter, OptionFillId, OptionOrder, OptionOrderFilter, OptionOrderId,
    OptionOrderStatus, OptionRfqFill, OptionRfqId, OptionRfqQuote, OptionRfqQuoteId,
    OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus, OptionSeries, OptionSeriesFilter,
    OptionSeriesId, OptionSeriesStatus,
};
use crate::error::{BackendError, Result};
use crate::execution::ExecutionTransactionStatus;
use crate::types::{Price1e8, Side, TimeInForce, TimestampMs};
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
    option_execution_reconciliations: HashMap<String, OptionExecutionReconciliation>,
    // OPTIONS-CONDITIONAL-ORDERS-TP-SL-V1
    conditional_orders: HashMap<uuid::Uuid, crate::options::conditional_orders::ConditionalOrder>,
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

    /// Test-only helper used by `api::trading::tests` to seed
    /// `OptionFill` rows directly. The production path inserts fills
    /// via `submit_order_and_match`; this helper bypasses the matching
    /// engine so unit tests can pin a known set of participants.
    #[cfg(test)]
    pub fn insert_fill_for_test(&mut self, fill: OptionFill) {
        self.fills.insert(fill.fill_id, fill);
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

        // Snapshot compatible resting orders in price–time priority,
        // then build a deterministic match plan WITHOUT mutating state.
        let mut candidates = self
            .orders
            .values()
            .filter(|order| can_match(&incoming, order))
            .cloned()
            .collect::<Vec<_>>();
        sort_match_candidates(&mut candidates, incoming.side);
        let plan = build_option_match_plan(&incoming, &candidates);

        // Validate TIF + post-only. Errors here mean ZERO mutation: the
        // taker is never inserted, no fills, no maker updates.
        enforce_tif_plan(&incoming, &plan)?;

        // Execute the planned legs. The single-threaded Mutex-guarded
        // store means the snapshot is still authoritative — but we
        // re-check each maker's remaining size defensively.
        let mut fills = Vec::with_capacity(plan.legs.len());
        for leg in &plan.legs {
            let Some(maker) = self.orders.get_mut(&leg.maker_id) else {
                continue;
            };
            if maker.remaining_size_1e8 < leg.fill_size_1e8 {
                continue;
            }
            let fill = option_fill_from_match(&incoming, maker, leg.fill_size_1e8, updated_at_ms);
            incoming.remaining_size_1e8 -= leg.fill_size_1e8;
            maker.remaining_size_1e8 -= leg.fill_size_1e8;
            maker.status = status_for_remaining(maker.size_1e8, maker.remaining_size_1e8);
            maker.updated_at_ms = updated_at_ms;
            self.fills.insert(fill.fill_id, fill.clone());
            fills.push(fill);
        }

        incoming.updated_at_ms = updated_at_ms;
        incoming.status = final_status_for_tif(&incoming);
        stamp_insert_terminal_reason(&mut incoming);
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
        order.terminal_reason_code = Some(terminal_reason::USER_CANCELLED.to_string());
        order.terminal_reason_message = None;
        order.terminal_reason_source = Some(terminal_reason::SOURCE_USER.to_string());
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

    // ---- OPTIONS-CONDITIONAL-ORDERS-TP-SL-V1 helpers ----

    /// Insert a freshly-built conditional order (caller has already
    /// run the reduce-only + OCO validations).
    pub fn insert_conditional_order(
        &mut self,
        order: crate::options::conditional_orders::ConditionalOrder,
    ) {
        self.conditional_orders.insert(order.id, order);
    }

    pub fn get_conditional_order(
        &self,
        id: uuid::Uuid,
    ) -> Option<crate::options::conditional_orders::ConditionalOrder> {
        self.conditional_orders.get(&id).cloned()
    }

    pub fn list_conditional_orders(
        &self,
        filter: &crate::options::conditional_orders::ConditionalOrderFilter,
    ) -> Vec<crate::options::conditional_orders::ConditionalOrder> {
        let mut out: Vec<_> = self
            .conditional_orders
            .values()
            .filter(|o| filter.matches(o))
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            a.created_at_ms
                .cmp(&b.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    /// Optimistic-lock update: the caller bumped `version`; we trust
    /// it here because the in-memory store is single-threaded behind
    /// the AppState mutex. The DB-backed mirror enforces `version`
    /// via a WHERE clause.
    pub fn update_conditional_order(
        &mut self,
        order: crate::options::conditional_orders::ConditionalOrder,
    ) -> Result<()> {
        if !self.conditional_orders.contains_key(&order.id) {
            return Err(BackendError::InvalidConditionalOrderId);
        }
        self.conditional_orders.insert(order.id, order);
        Ok(())
    }

    /// Snapshot of resting orders on the OPPOSITE side of `taker`
    /// that are price-compatible. Mirrors the filter in
    /// `submit_order_and_match` so the worker can build the same
    /// match plan deterministically.
    pub fn list_open_match_candidates(&self, taker: &OptionOrder) -> Vec<OptionOrder> {
        self.orders
            .values()
            .filter(|order| can_match(taker, order))
            .cloned()
            .collect()
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

    pub fn summarize_option_execution_events_by_contract_address(&self) -> Vec<(String, u64)> {
        let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for event in self.option_execution_events.values() {
            *counts
                .entry(event.contract_address.to_ascii_lowercase())
                .or_default() += 1;
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

    /// V2F-P: in-memory mirror of
    /// `PgRepository::admin_perp_fee_v2_consumer_counts`.
    ///
    /// Returns the count of PERP `FeeChargedV2` events grouped by the
    /// lowercased decoded `consumer` topic. Rebates and OPTION-flavoured
    /// FeeChargedV2 events are excluded.
    pub fn perp_fee_v2_consumer_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.fee_v2_consumer_counts_for_product("FeeChargedV2", "perp")
    }

    /// V2F-Q: in-memory mirror of
    /// `PgRepository::admin_perp_fee_v2_rebated_consumer_counts`.
    ///
    /// Returns the count of PERP `FeeRebatedV2` events grouped by the
    /// lowercased decoded `consumer` topic. `FeeChargedV2` rows and
    /// OPTION-flavoured `FeeRebatedV2` rows are excluded.
    pub fn perp_fee_v2_rebated_consumer_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.fee_v2_consumer_counts_for_product("FeeRebatedV2", "perp")
    }

    /// V2G-F: in-memory mirror of
    /// `PgRepository::admin_option_fee_v2_consumer_counts`.
    ///
    /// Returns the count of OPTION `FeeChargedV2` events grouped by the
    /// lowercased decoded `consumer` topic. Rebates and PERP-flavoured
    /// `FeeChargedV2` events are excluded.
    pub fn option_fee_v2_consumer_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.fee_v2_consumer_counts_for_product("FeeChargedV2", "option")
    }

    /// V2G-F: in-memory mirror of
    /// `PgRepository::admin_option_fee_v2_rebated_consumer_counts`.
    ///
    /// Returns the count of OPTION `FeeRebatedV2` events grouped by the
    /// lowercased decoded `consumer` topic. `FeeChargedV2` rows and
    /// PERP-flavoured `FeeRebatedV2` rows are excluded.
    pub fn option_fee_v2_rebated_consumer_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.fee_v2_consumer_counts_for_product("FeeRebatedV2", "option")
    }

    fn fee_v2_consumer_counts_for_product(
        &self,
        event_name: &str,
        product_kind: &str,
    ) -> std::collections::BTreeMap<String, u64> {
        let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for event in self.option_execution_events.values() {
            if event.event_name != event_name {
                continue;
            }
            let Some(decoded) = event.decoded.as_ref() else {
                continue;
            };
            if decoded.get("productKind").and_then(|value| value.as_str()) != Some(product_kind) {
                continue;
            }
            let consumer = decoded
                .get("consumer")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            *counts.entry(consumer).or_default() += 1;
        }
        counts
    }

    /// V2G-F: derived FeesManagerV2 rebate budget per settlement asset,
    /// computed by summing indexed `RebateBudgetFunded` (credit) minus
    /// `RebateBudgetSpent` and `RebateBudgetWithdrawn` (debits). Asset
    /// addresses are lowercased; amounts are clamped to a non-negative
    /// `u64` (the on-chain contract prevents net-negative; if the
    /// indexer is briefly behind a withdraw the gauge floors at 0
    /// rather than wrap-underflowing).
    ///
    /// Mirrors `PgRepository::admin_fees_manager_v2_rebate_budget_by_asset`.
    /// Feeds the `deopt_fees_manager_v2_rebate_budget_native{asset=...}`
    /// gauge and the `FeesManagerV2RebateBudgetLow` Prometheus alert.
    pub fn fees_manager_v2_rebate_budget_by_asset(
        &self,
    ) -> std::collections::BTreeMap<String, u64> {
        let mut net: std::collections::BTreeMap<String, i128> = std::collections::BTreeMap::new();
        for event in self.option_execution_events.values() {
            let Some(decoded) = event.decoded.as_ref() else {
                continue;
            };
            let Some(asset) = decoded.get("settlementAsset").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(amount_str) = decoded.get("amount").and_then(|v| v.as_str()) else {
                continue;
            };
            let Ok(amount) = amount_str.parse::<i128>() else {
                continue;
            };
            let asset_lc = asset.to_ascii_lowercase();
            let entry = net.entry(asset_lc).or_default();
            match event.event_name.as_str() {
                "RebateBudgetFunded" => *entry = entry.saturating_add(amount),
                "RebateBudgetSpent" | "RebateBudgetWithdrawn" => {
                    *entry = entry.saturating_sub(amount)
                }
                _ => {}
            }
        }
        net.into_iter()
            .map(|(asset, value)| {
                let clamped = if value < 0 { 0u64 } else { value as u64 };
                (asset, clamped)
            })
            .collect()
    }

    pub fn list_option_execution_events_by_tx_hash(
        &self,
        tx_hash: &str,
    ) -> Vec<OptionExecutionEvent> {
        let mut events = self
            .option_execution_events
            .values()
            .filter(|event| event.tx_hash.eq_ignore_ascii_case(tx_hash))
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            left.block_number
                .cmp(&right.block_number)
                .then_with(|| left.log_index.cmp(&right.log_index))
                .then_with(|| left.id.cmp(&right.id))
        });
        events
    }

    /// List confirmed (`mined_success`) option execution transactions whose
    /// existing reconciliation row (if any) has a non-terminal status. A
    /// non-terminal status is anything other than
    /// [`OptionReconciliationStatus::Reconciled`]; specifically
    /// `missing_events`, `partially_reconciled`, `reconciliation_failed`, and
    /// `skipped` are all eligible for re-evaluation once new evidence
    /// (typically backfilled `option_execution_events`) becomes available.
    ///
    /// Once a row reaches `reconciled`, it is filtered out and the worker
    /// will never touch it again.
    pub fn list_confirmed_unreconciled_option_execution_transactions(
        &self,
        limit: u32,
        include_reconciled_without_state_checks: bool,
    ) -> Vec<OptionExecutionTransaction> {
        let mut transactions = self
            .option_execution_transactions
            .values()
            .filter(|tx| {
                tx.tx_hash
                    .as_deref()
                    .map(|hash| !hash.is_empty())
                    .unwrap_or(false)
                    && matches!(
                        tx.confirmation_status,
                        Some(OptionExecutionConfirmationStatus::MinedSuccess)
                    )
                    && self
                        .option_execution_reconciliations
                        .get(&tx.transaction_id)
                        .map(|row| {
                            row.status != crate::options::OptionReconciliationStatus::Reconciled
                                || (include_reconciled_without_state_checks
                                    && row.details.get("state_checks").is_none())
                        })
                        .unwrap_or(true)
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

    pub fn upsert_option_execution_reconciliation(
        &mut self,
        row: OptionExecutionReconciliation,
        updated_at_ms: i64,
    ) {
        let mut row = row;
        if let Some(existing) = self
            .option_execution_reconciliations
            .get(&row.option_execution_transaction_id)
        {
            row.id = existing.id;
            row.created_at_ms = existing.created_at_ms;
        }
        row.updated_at_ms = updated_at_ms;
        self.option_execution_reconciliations
            .insert(row.option_execution_transaction_id.clone(), row);
    }

    pub fn list_option_execution_reconciliations(
        &self,
        limit: u32,
    ) -> Vec<OptionExecutionReconciliation> {
        let mut rows = self
            .option_execution_reconciliations
            .values()
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            right
                .reconciled_at_ms
                .cmp(&left.reconciled_at_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        rows.truncate(limit as usize);
        rows
    }

    pub fn summarize_option_execution_reconciliations(&self) -> Vec<(String, u64)> {
        let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        for row in self.option_execution_reconciliations.values() {
            *counts.entry(row.status.as_str().to_string()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    pub fn get_option_execution_reconciliation_by_transaction_id(
        &self,
        transaction_id: &str,
    ) -> Option<OptionExecutionReconciliation> {
        self.option_execution_reconciliations
            .get(transaction_id)
            .cloned()
    }

    /// Test-only path that inserts an option execution transaction even when a
    /// submitted row already exists for the same intent. Production code paths
    /// continue to use [`insert_option_execution_transaction`].
    #[cfg(test)]
    pub fn test_insert_option_execution_transaction(
        &mut self,
        transaction: OptionExecutionTransaction,
    ) {
        self.option_execution_transactions
            .insert(transaction.transaction_id.clone(), transaction);
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

    /// In-memory mirror of
    /// [`PgRepository::list_recent_option_execution_transactions`].
    /// Each tuple is `(tx, source_type)` — `source_type` is sourced
    /// from the parent intent (if present) so the unified
    /// `/executor/transactions` list endpoint reports the same
    /// discriminator string DB consumers would see (`"option_orderbook_fill"`
    /// or `"option_rfq_fill"`). Orphaned rows (no intent) carry
    /// `source_type = None`.
    pub fn list_recent_option_execution_transactions(
        &self,
        limit: u32,
    ) -> Vec<(OptionExecutionTransaction, Option<String>)> {
        let mut transactions: Vec<OptionExecutionTransaction> = self
            .option_execution_transactions
            .values()
            .cloned()
            .collect();
        transactions.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| right.transaction_id.cmp(&left.transaction_id))
        });
        transactions
            .into_iter()
            .take(limit as usize)
            .map(|tx| {
                let source_type = self
                    .get_option_execution_intent(tx.intent_id)
                    .map(|intent| intent.source_type.as_str().to_string());
                (tx, source_type)
            })
            .collect()
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

/// Public re-export of the price-time priority sort so the conditional
/// orders worker can reuse it on a borrowed snapshot when planning a
/// child IOC order.
pub fn sort_match_candidates_view(orders: &mut [OptionOrder], taker_side: Side) {
    sort_match_candidates(orders, taker_side)
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

/// Deterministic per-maker plan entry. Built from a snapshot of resting
/// orders; never mutates state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OptionMatchLeg {
    pub maker_id: OptionOrderId,
    pub maker_price_1e8: Price1e8,
    pub fill_size_1e8: u128,
}

/// Full match plan for an incoming taker. `total_fill_size_1e8` is the
/// sum of all leg sizes — used by FOK to decide whether the order is
/// fully fillable before any mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OptionMatchPlan {
    pub legs: Vec<OptionMatchLeg>,
    pub total_fill_size_1e8: u128,
}

/// Compose a match plan from a *sorted* list of compatible resting
/// orders (price–time priority). Caller is responsible for filtering /
/// sorting; this only walks the candidates in order until the incoming
/// taker is exhausted.
pub(crate) fn build_option_match_plan(
    incoming: &OptionOrder,
    candidates: &[OptionOrder],
) -> OptionMatchPlan {
    let mut legs = Vec::new();
    let mut remaining = incoming.size_1e8;
    let mut total: u128 = 0;
    for maker in candidates {
        if remaining == 0 {
            break;
        }
        let fill_size = remaining.min(maker.remaining_size_1e8);
        if fill_size == 0 {
            continue;
        }
        legs.push(OptionMatchLeg {
            maker_id: maker.order_id,
            maker_price_1e8: maker.price_1e8,
            fill_size_1e8: fill_size,
        });
        remaining -= fill_size;
        total += fill_size;
    }
    OptionMatchPlan {
        legs,
        total_fill_size_1e8: total,
    }
}

/// Validate the planned match against TIF + post-only constraints.
/// Runs BEFORE any mutation so FOK/post-only rejections leave the book
/// untouched.
pub(crate) fn enforce_tif_plan(order: &OptionOrder, plan: &OptionMatchPlan) -> Result<()> {
    if order.post_only && !plan.legs.is_empty() {
        return Err(BackendError::PostOnlyWouldMatch);
    }
    if matches!(order.time_in_force, TimeInForce::Fok) && plan.total_fill_size_1e8 < order.size_1e8
    {
        return Err(BackendError::FokNotFillable);
    }
    Ok(())
}

/// Final status of the taker once the planned legs have been applied.
/// Honours TIF semantics: GTC rests, IOC cancels remainder, FOK is
/// always fully filled when it reaches this point (validated earlier).
pub(crate) fn final_status_for_tif(order: &OptionOrder) -> OptionOrderStatus {
    let base = status_for_remaining(order.size_1e8, order.remaining_size_1e8);
    match order.time_in_force {
        TimeInForce::Gtc => base,
        TimeInForce::Ioc => match base {
            OptionOrderStatus::Open | OptionOrderStatus::PartiallyFilled => {
                OptionOrderStatus::Cancelled
            }
            other => other,
        },
        TimeInForce::Fok => base,
    }
}

/// HISTORY-V2-TERMINAL-REASONS-V1 — stamp the persisted reason on the
/// staged taker right before insert. The only insert-time terminal
/// reason today is `ioc_remainder_cancelled` (IOC that did not fully
/// fill). FOK rejection and post-only rejection are pre-persistence
/// (`enforce_tif_plan` errors before any row is written) and never
/// reach this point.
pub(crate) fn stamp_insert_terminal_reason(order: &mut OptionOrder) {
    if matches!(order.status, OptionOrderStatus::Cancelled)
        && matches!(order.time_in_force, TimeInForce::Ioc)
        && order.remaining_size_1e8 > 0
    {
        order.terminal_reason_code = Some(terminal_reason::IOC_REMAINDER_CANCELLED.to_string());
        order.terminal_reason_message = None;
        order.terminal_reason_source = Some(terminal_reason::SOURCE_TIF_POLICY.to_string());
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
