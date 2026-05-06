use crate::error::{BackendError, Result};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmationConfig {
    pub enabled: bool,
    pub require_persistence: bool,
    pub required_blocks: u64,
    pub max_batch_size: u32,
    pub require_reconciliation: bool,
    pub rpc_url: Option<String>,
}

impl ConfirmationConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
            required_blocks: 2,
            max_batch_size: 50,
            require_reconciliation: true,
            rpc_url: None,
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.enabled && self.rpc_url.is_none() {
            return Err(BackendError::Config(
                "RPC_URL is required when CONFIRMATION_ENABLED=true".to_string(),
            ));
        }
        if self.enabled && self.require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "confirmation requires persistence enabled".to_string(),
            ));
        }
        if self.enabled && !self.require_reconciliation {
            return Err(BackendError::Config(
                "CONFIRMATION_REQUIRE_RECONCILIATION must be true".to_string(),
            ));
        }
        if self.max_batch_size == 0 {
            return Err(BackendError::Config(
                "CONFIRMATION_MAX_BATCH_SIZE must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    Pending,
    Confirmed,
    Failed,
    NotFinalized,
    MissingReceipt,
    MissingReconciliation,
    MissingIndexedEvent,
    ReceiptFailed,
}

impl ConfirmationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::NotFinalized => "not_finalized",
            Self::MissingReceipt => "missing_receipt",
            Self::MissingReconciliation => "missing_reconciliation",
            Self::MissingIndexedEvent => "missing_indexed_event",
            Self::ReceiptFailed => "receipt_failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "confirmed" => Ok(Self::Confirmed),
            "failed" => Ok(Self::Failed),
            "not_finalized" => Ok(Self::NotFinalized),
            "missing_receipt" => Ok(Self::MissingReceipt),
            "missing_reconciliation" => Ok(Self::MissingReconciliation),
            "missing_indexed_event" => Ok(Self::MissingIndexedEvent),
            "receipt_failed" => Ok(Self::ReceiptFailed),
            other => Err(BackendError::Persistence(format!(
                "invalid confirmation status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmationReceipt {
    pub tx_hash: String,
    pub status: Option<u64>,
    pub block_number: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmationDecisionInput {
    pub tx_hash: Option<String>,
    pub receipt: Option<ConfirmationReceipt>,
    pub current_block_number: Option<u64>,
    pub required_confirmations: u64,
    pub indexed_event_found: bool,
    pub reconciliation_matched: bool,
    pub require_reconciliation: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfirmationDecision {
    pub confirmation_status: ConfirmationStatus,
    pub confirmed: bool,
    pub receipt_status: Option<u64>,
    pub receipt_block_number: Option<u64>,
    pub current_block_number: Option<u64>,
    pub required_confirmations: u64,
    pub indexed_event_found: bool,
    pub reconciliation_matched: bool,
    pub reason: Option<String>,
}

pub fn decide_confirmation(input: &ConfirmationDecisionInput) -> ConfirmationDecision {
    let base = |confirmation_status, reason: &str| ConfirmationDecision {
        confirmation_status,
        confirmed: false,
        receipt_status: input.receipt.as_ref().and_then(|receipt| receipt.status),
        receipt_block_number: input
            .receipt
            .as_ref()
            .and_then(|receipt| receipt.block_number),
        current_block_number: input.current_block_number,
        required_confirmations: input.required_confirmations,
        indexed_event_found: input.indexed_event_found,
        reconciliation_matched: input.reconciliation_matched,
        reason: Some(reason.to_string()),
    };

    let Some(expected_tx_hash) = input.tx_hash.as_deref() else {
        return base(
            ConfirmationStatus::Failed,
            "submitted transaction hash not found",
        );
    };
    let Some(receipt) = input.receipt.as_ref() else {
        return base(
            ConfirmationStatus::MissingReceipt,
            "transaction receipt not found",
        );
    };
    if !receipt.tx_hash.eq_ignore_ascii_case(expected_tx_hash) {
        return base(
            ConfirmationStatus::MissingReceipt,
            "receipt transaction hash does not match submitted transaction",
        );
    }
    if receipt.status != Some(1) {
        return base(
            ConfirmationStatus::ReceiptFailed,
            "receipt status is not success",
        );
    }
    let Some(receipt_block_number) = receipt.block_number else {
        return base(
            ConfirmationStatus::MissingReceipt,
            "receipt block number not found",
        );
    };
    let Some(current_block_number) = input.current_block_number else {
        return base(
            ConfirmationStatus::NotFinalized,
            "current block number not available",
        );
    };
    let finalized_at = receipt_block_number.saturating_add(input.required_confirmations);
    if current_block_number < finalized_at {
        return base(
            ConfirmationStatus::NotFinalized,
            "receipt block has fewer than required confirmations",
        );
    }
    if !input.indexed_event_found {
        return base(
            ConfirmationStatus::MissingIndexedEvent,
            "indexed TradeExecuted event not found for transaction and onchain intent",
        );
    }
    if !input.reconciliation_matched {
        return base(
            ConfirmationStatus::MissingReconciliation,
            "matched reconciliation not found",
        );
    }

    ConfirmationDecision {
        confirmation_status: ConfirmationStatus::Confirmed,
        confirmed: true,
        receipt_status: receipt.status,
        receipt_block_number: receipt.block_number,
        current_block_number: input.current_block_number,
        required_confirmations: input.required_confirmations,
        indexed_event_found: input.indexed_event_found,
        reconciliation_matched: input.reconciliation_matched,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_receipt_is_missing_receipt() {
        let decision = decide_confirmation(&input(None, 12, true, true));

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::MissingReceipt
        );
        assert!(!decision.confirmed);
    }

    #[test]
    fn receipt_status_zero_is_receipt_failed() {
        let decision =
            decide_confirmation(&input(Some(receipt(Some(0), Some(10))), 12, true, true));

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::ReceiptFailed
        );
        assert!(!decision.confirmed);
    }

    #[test]
    fn insufficient_confirmations_are_not_finalized() {
        let decision =
            decide_confirmation(&input(Some(receipt(Some(1), Some(10))), 11, true, true));

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::NotFinalized
        );
        assert!(!decision.confirmed);
    }

    #[test]
    fn missing_indexed_event_blocks_confirmation() {
        let decision =
            decide_confirmation(&input(Some(receipt(Some(1), Some(10))), 12, false, true));

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::MissingIndexedEvent
        );
        assert!(!decision.confirmed);
    }

    #[test]
    fn missing_reconciliation_blocks_confirmation() {
        let decision =
            decide_confirmation(&input(Some(receipt(Some(1), Some(10))), 12, true, false));

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::MissingReconciliation
        );
        assert!(!decision.confirmed);
    }

    #[test]
    fn wrong_tx_hash_is_not_treated_as_confirmation() {
        let mut receipt = receipt(Some(1), Some(10));
        receipt.tx_hash = format!("0x{:064x}", 2);
        let decision = decide_confirmation(&input(Some(receipt), 12, true, true));

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::MissingReceipt
        );
        assert!(!decision.confirmed);
    }

    #[test]
    fn wrong_tx_indexed_event_is_missing_indexed_event() {
        let decision =
            decide_confirmation(&input(Some(receipt(Some(1), Some(10))), 12, false, true));

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::MissingIndexedEvent
        );
    }

    #[test]
    fn all_conditions_true_are_confirmed() {
        let decision =
            decide_confirmation(&input(Some(receipt(Some(1), Some(10))), 12, true, true));

        assert_eq!(decision.confirmation_status, ConfirmationStatus::Confirmed);
        assert!(decision.confirmed);
        assert_eq!(decision.reason, None);
    }

    #[test]
    fn repeated_success_decision_is_stable() {
        let input = input(Some(receipt(Some(1), Some(10))), 12, true, true);

        let first = decide_confirmation(&input);
        let second = decide_confirmation(&input);

        assert_eq!(first, second);
        assert!(second.confirmed);
    }

    #[test]
    fn reconciliation_cannot_be_disabled_by_decision_input() {
        let mut input = input(Some(receipt(Some(1), Some(10))), 12, true, false);
        input.require_reconciliation = false;

        let decision = decide_confirmation(&input);

        assert_eq!(
            decision.confirmation_status,
            ConfirmationStatus::MissingReconciliation
        );
        assert!(!decision.confirmed);
    }

    fn input(
        receipt: Option<ConfirmationReceipt>,
        current_block_number: u64,
        indexed_event_found: bool,
        reconciliation_matched: bool,
    ) -> ConfirmationDecisionInput {
        ConfirmationDecisionInput {
            tx_hash: Some(format!("0x{:064x}", 1)),
            receipt,
            current_block_number: Some(current_block_number),
            required_confirmations: 2,
            indexed_event_found,
            reconciliation_matched,
            require_reconciliation: true,
        }
    }

    fn receipt(status: Option<u64>, block_number: Option<u64>) -> ConfirmationReceipt {
        ConfirmationReceipt {
            tx_hash: format!("0x{:064x}", 1),
            status,
            block_number,
        }
    }
}
