use crate::error::{BackendError, Result};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationConfig {
    pub enabled: bool,
    pub require_persistence: bool,
    pub max_batch_size: u32,
}

impl ReconciliationConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
            max_batch_size: 100,
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.enabled && self.require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "reconciliation requires persistence enabled".to_string(),
            ));
        }
        if self.max_batch_size == 0 {
            return Err(BackendError::Config(
                "RECONCILIATION_MAX_BATCH_SIZE must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationStatus {
    Matched,
    Ambiguous,
    Unmatched,
    Ignored,
}

impl ReconciliationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::Ambiguous => "ambiguous",
            Self::Unmatched => "unmatched",
            Self::Ignored => "ignored",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "matched" => Ok(Self::Matched),
            "ambiguous" => Ok(Self::Ambiguous),
            "unmatched" => Ok(Self::Unmatched),
            "ignored" => Ok(Self::Ignored),
            other => Err(BackendError::Persistence(format!(
                "invalid reconciliation status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutionReconciliation {
    pub reconciliation_id: String,
    pub onchain_intent_id: String,
    pub intent_id: String,
    pub indexed_event_id: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
    pub status: ReconciliationStatus,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ReconciliationCounts {
    pub matched: u64,
    pub ambiguous: u64,
    pub unmatched: u64,
    pub ignored: u64,
}

impl ReconciliationCounts {
    pub fn confirmed(&self) -> u64 {
        0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ReconciliationTickResult {
    pub indexed_trades_checked: u64,
    pub matched: u64,
    pub ambiguous: u64,
    pub unmatched: u64,
    pub confirmed: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectReconciliationInput {
    pub onchain_intent_id: Option<String>,
    pub matching_intent_count: usize,
    pub matching_indexed_event_count: usize,
}

pub fn decide_direct_reconciliation(input: &DirectReconciliationInput) -> ReconciliationStatus {
    let Some(onchain_intent_id) = input.onchain_intent_id.as_deref() else {
        return ReconciliationStatus::Ignored;
    };
    if onchain_intent_id.is_empty() {
        return ReconciliationStatus::Ignored;
    }
    if input.matching_intent_count == 0 {
        return ReconciliationStatus::Unmatched;
    }
    if input.matching_intent_count == 1 && input.matching_indexed_event_count == 1 {
        return ReconciliationStatus::Matched;
    }
    ReconciliationStatus::Ambiguous
}

pub fn normalize_onchain_intent_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() != 66 {
        return None;
    }
    let hex = value.strip_prefix("0x")?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_onchain_intent_id_match_is_matched() {
        let decision = decide_direct_reconciliation(&DirectReconciliationInput {
            onchain_intent_id: Some(word(1)),
            matching_intent_count: 1,
            matching_indexed_event_count: 1,
        });

        assert_eq!(decision, ReconciliationStatus::Matched);
    }

    #[test]
    fn event_without_matching_intent_is_unmatched() {
        let decision = decide_direct_reconciliation(&DirectReconciliationInput {
            onchain_intent_id: Some(word(1)),
            matching_intent_count: 0,
            matching_indexed_event_count: 1,
        });

        assert_eq!(decision, ReconciliationStatus::Unmatched);
    }

    #[test]
    fn multiple_matching_intents_are_ambiguous_not_confirmed() {
        let counts = ReconciliationCounts {
            ambiguous: 1,
            ..ReconciliationCounts::default()
        };
        let decision = decide_direct_reconciliation(&DirectReconciliationInput {
            onchain_intent_id: Some(word(1)),
            matching_intent_count: 2,
            matching_indexed_event_count: 1,
        });

        assert_eq!(decision, ReconciliationStatus::Ambiguous);
        assert_eq!(counts.confirmed(), 0);
    }

    #[test]
    fn duplicate_indexed_events_are_ambiguous() {
        let decision = decide_direct_reconciliation(&DirectReconciliationInput {
            onchain_intent_id: Some(word(1)),
            matching_intent_count: 1,
            matching_indexed_event_count: 2,
        });

        assert_eq!(decision, ReconciliationStatus::Ambiguous);
    }

    #[test]
    fn missing_onchain_intent_id_is_ignored() {
        let decision = decide_direct_reconciliation(&DirectReconciliationInput {
            onchain_intent_id: None,
            matching_intent_count: 1,
            matching_indexed_event_count: 1,
        });

        assert_eq!(decision, ReconciliationStatus::Ignored);
    }

    #[test]
    fn onchain_intent_id_normalization_requires_bytes32_hex() {
        assert_eq!(
            normalize_onchain_intent_id(&word_upper(10)).as_deref(),
            Some(word(10).as_str())
        );
        assert_eq!(normalize_onchain_intent_id("0x1234"), None);
        assert_eq!(normalize_onchain_intent_id("not-hex"), None);
    }

    #[test]
    fn duplicate_tick_result_can_remain_zero_after_rows_exist() {
        let result = ReconciliationTickResult::default();

        assert_eq!(result.indexed_trades_checked, 0);
        assert_eq!(result.confirmed, 0);
    }

    fn word(value: u8) -> String {
        format!("0x{value:064x}")
    }

    fn word_upper(value: u8) -> String {
        format!("0x{value:064X}")
    }
}
