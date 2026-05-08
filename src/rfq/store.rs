use super::{QuoteId, RfqId, RfqQuote, RfqQuoteStatus, RfqRequest, RfqStatus};
use crate::error::{BackendError, Result};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct RfqStore {
    rfqs: HashMap<RfqId, RfqRequest>,
    quotes: HashMap<QuoteId, RfqQuote>,
}

impl RfqStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_rfq(&mut self, rfq: RfqRequest) {
        self.rfqs.insert(rfq.rfq_id, rfq);
    }

    pub fn list_rfqs(&self) -> Vec<RfqRequest> {
        let mut rfqs = self.rfqs.values().cloned().collect::<Vec<_>>();
        rfqs.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.rfq_id.cmp(&right.rfq_id))
        });
        rfqs
    }

    pub fn get_rfq(&self, rfq_id: RfqId) -> Option<RfqRequest> {
        self.rfqs.get(&rfq_id).cloned()
    }

    pub fn insert_quote(&mut self, quote: RfqQuote) {
        self.quotes.insert(quote.quote_id, quote);
    }

    pub fn quote_count(&self, rfq_id: RfqId) -> usize {
        self.quotes
            .values()
            .filter(|quote| quote.rfq_id == rfq_id)
            .count()
    }

    pub fn list_quotes(&self, rfq_id: RfqId) -> Vec<RfqQuote> {
        let mut quotes = self
            .quotes
            .values()
            .filter(|quote| quote.rfq_id == rfq_id)
            .cloned()
            .collect::<Vec<_>>();
        quotes.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.quote_id.cmp(&right.quote_id))
        });
        quotes
    }

    pub fn get_quote(&self, quote_id: QuoteId) -> Option<RfqQuote> {
        self.quotes.get(&quote_id).cloned()
    }

    pub fn accept_quote(
        &mut self,
        rfq_id: RfqId,
        quote_id: QuoteId,
        execution_intent_id: Uuid,
    ) -> Result<()> {
        let rfq = self
            .rfqs
            .get_mut(&rfq_id)
            .ok_or(BackendError::InvalidRfqId)?;
        if rfq.status != RfqStatus::Open {
            return Err(BackendError::InvalidRfqState(format!(
                "RFQ is {}",
                rfq.status.as_str()
            )));
        }
        let quote = self
            .quotes
            .get_mut(&quote_id)
            .ok_or(BackendError::InvalidRfqQuoteId)?;
        if quote.rfq_id != rfq_id {
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

        rfq.status = RfqStatus::Accepted;
        rfq.accepted_quote_id = Some(quote_id);
        rfq.execution_intent_id = Some(execution_intent_id);
        quote.status = RfqQuoteStatus::Accepted;

        for other in self.quotes.values_mut() {
            if other.rfq_id == rfq_id
                && other.quote_id != quote_id
                && other.status == RfqQuoteStatus::Active
            {
                other.status = RfqQuoteStatus::Rejected;
            }
        }
        Ok(())
    }

    pub fn cancel_rfq(&mut self, rfq_id: RfqId) -> Result<RfqRequest> {
        let rfq = self
            .rfqs
            .get_mut(&rfq_id)
            .ok_or(BackendError::InvalidRfqId)?;
        if rfq.status == RfqStatus::Accepted {
            return Err(BackendError::InvalidRfqState(
                "accepted RFQ cannot be cancelled".to_string(),
            ));
        }
        rfq.status = RfqStatus::Cancelled;
        for quote in self.quotes.values_mut() {
            if quote.rfq_id == rfq_id && quote.status == RfqQuoteStatus::Active {
                quote.status = RfqQuoteStatus::Cancelled;
            }
        }
        Ok(rfq.clone())
    }
}
