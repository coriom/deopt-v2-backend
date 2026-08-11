//! Indexer / execution correlation for the Hybrid V2 broadcast pipeline
//! (Part O of `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! The confirmation worker MUST NOT promote a broadcast row to
//! `CONFIRMED` until:
//!   * the indexer has ingested the receipt block, AND
//!   * a matching `hybrid_v2_matched_executions` row exists with
//!     `completion_status = COMPLETE` AND
//!   * that row's `tx_hash` equals our persisted `tx_hash`.
//!
//! Any other outcome is a typed fail-closed state. In particular, if
//! the indexer has passed the block and the expected canonical evidence
//! is missing after `tolerance_ticks`, the outcome is a
//! `Contradiction` — the worker escalates to
//! `MANUAL_INTERVENTION_REQUIRED` with `failure_class = CORRELATION_MISSING`.

use crate::error::Result as BackendResult;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;

/// Structured correlation outcome. Callers advance the phase machine
/// according to the variant; the checker itself is read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrelationOutcome {
    /// Indexer has NOT yet caught up to the receipt block; remain in
    /// `Confirming`.
    IndexerBehind {
        indexed_head: u64,
        receipt_block: u64,
    },
    /// Indexer is at or past the block but no matched execution row has
    /// been observed yet; caller may return here for a bounded number
    /// of tolerance ticks before escalating.
    NotCorrelatedYet,
    /// Matched row exists but its completion status is `INCOMPLETE` —
    /// stay in `Confirming` until it flips to COMPLETE.
    MatchedRowIncomplete,
    /// Matched row exists but was invalidated by a reorg — transition
    /// to `Reorged`.
    MatchedRowInvalidatedByReorg,
    /// Matched row exists AND is COMPLETE AND tx_hash agrees.
    MatchedRowComplete { matched_execution_id: String },
    /// The indexer has moved past the receipt block AND the expected
    /// canonical evidence is missing beyond tolerance. Typed
    /// fail-closed — caller escalates to MANUAL_INTERVENTION.
    Contradiction { detail: String },
}

/// Indexer correlation checker. Borrows the store + deployment id for
/// the duration of a single `check` call. Does NOT mutate state.
pub struct IndexerCorrelationChecker<'a> {
    pub store: &'a dyn HybridV2ProjectionStore,
    pub deployment_id: i64,
}

impl IndexerCorrelationChecker<'_> {
    /// Perform one correlation cycle for `(canonical_execution_id,
    /// tx_hash, receipt_block)`. `tolerance_exceeded` should be `true`
    /// when the caller has already spent its budget of ticks in the
    /// `NotCorrelatedYet` state — the checker then emits a
    /// `Contradiction` instead.
    pub async fn check(
        &self,
        canonical_execution_id: &str,
        tx_hash: &str,
        receipt_block: u64,
        tolerance_exceeded: bool,
    ) -> BackendResult<CorrelationOutcome> {
        // Step 1: has the indexer caught up?
        let indexed_head = self
            .store
            .indexed_head_block(self.deployment_id)
            .await?
            .unwrap_or(0);
        if indexed_head < receipt_block {
            return Ok(CorrelationOutcome::IndexerBehind {
                indexed_head,
                receipt_block,
            });
        }

        // Step 2: correlate on tx_hash.
        let matched = self
            .store
            .get_matched_execution_by_tx_hash(self.deployment_id, tx_hash)
            .await?;
        match matched {
            None => {
                if tolerance_exceeded {
                    Ok(CorrelationOutcome::Contradiction {
                        detail: format!(
                            "no matched execution row for tx_hash={tx_hash} at receipt_block={receipt_block} \
                             (canonical_execution_id={canonical_execution_id}) after tolerance"
                        ),
                    })
                } else {
                    Ok(CorrelationOutcome::NotCorrelatedYet)
                }
            }
            Some(row) => {
                if !row.tx_hash.eq_ignore_ascii_case(tx_hash) {
                    return Ok(CorrelationOutcome::Contradiction {
                        detail: format!(
                            "matched execution tx_hash disagrees with local tx_hash \
                             (local={tx_hash} indexer={})",
                            row.tx_hash
                        ),
                    });
                }
                match row.completion_status.as_str() {
                    "COMPLETE" => Ok(CorrelationOutcome::MatchedRowComplete {
                        matched_execution_id: row.matched_execution_id,
                    }),
                    "INCOMPLETE" => Ok(CorrelationOutcome::MatchedRowIncomplete),
                    "INVALIDATED_BY_REORG" => Ok(CorrelationOutcome::MatchedRowInvalidatedByReorg),
                    other => Ok(CorrelationOutcome::Contradiction {
                        detail: format!(
                            "unknown completion_status {other} for matched execution \
                             (canonical_execution_id={canonical_execution_id})"
                        ),
                    }),
                }
            }
        }
    }
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use crate::hybrid_v2::reducer::ExecutionCompletion;

    #[tokio::test]
    async fn indexer_behind_when_head_below_receipt() {
        let store = InMemoryProjectionStore::new();
        store.set_indexed_head_for_test(1, 41);
        let checker = IndexerCorrelationChecker {
            store: &store,
            deployment_id: 1,
        };
        let out = checker.check("cid", "0xabc", 42, false).await.unwrap();
        assert_eq!(
            out,
            CorrelationOutcome::IndexerBehind {
                indexed_head: 41,
                receipt_block: 42,
            }
        );
    }

    #[tokio::test]
    async fn not_correlated_yet_when_indexer_caught_up_but_row_missing() {
        let store = InMemoryProjectionStore::new();
        store.set_indexed_head_for_test(1, 100);
        let checker = IndexerCorrelationChecker {
            store: &store,
            deployment_id: 1,
        };
        let out = checker.check("cid", "0xabc", 42, false).await.unwrap();
        assert_eq!(out, CorrelationOutcome::NotCorrelatedYet);
    }

    #[tokio::test]
    async fn contradiction_after_tolerance() {
        let store = InMemoryProjectionStore::new();
        store.set_indexed_head_for_test(1, 100);
        let checker = IndexerCorrelationChecker {
            store: &store,
            deployment_id: 1,
        };
        let out = checker.check("cid", "0xabc", 42, true).await.unwrap();
        assert!(matches!(out, CorrelationOutcome::Contradiction { .. }));
    }

    #[tokio::test]
    async fn matched_row_incomplete_returned() {
        let store = InMemoryProjectionStore::new();
        store.set_indexed_head_for_test(1, 100);
        store.insert_matched_execution_for_test(
            1,
            "exec-1",
            "0xabc",
            42,
            ExecutionCompletion::Incomplete,
        );
        let checker = IndexerCorrelationChecker {
            store: &store,
            deployment_id: 1,
        };
        let out = checker.check("cid", "0xabc", 42, false).await.unwrap();
        assert_eq!(out, CorrelationOutcome::MatchedRowIncomplete);
    }

    #[tokio::test]
    async fn matched_row_invalidated_by_reorg_returned() {
        let store = InMemoryProjectionStore::new();
        store.set_indexed_head_for_test(1, 100);
        store.insert_matched_execution_for_test(
            1,
            "exec-1",
            "0xabc",
            42,
            ExecutionCompletion::InvalidatedByReorg,
        );
        let checker = IndexerCorrelationChecker {
            store: &store,
            deployment_id: 1,
        };
        let out = checker.check("cid", "0xabc", 42, false).await.unwrap();
        assert_eq!(out, CorrelationOutcome::MatchedRowInvalidatedByReorg);
    }

    #[tokio::test]
    async fn matched_row_complete_returns_execution_id() {
        let store = InMemoryProjectionStore::new();
        store.set_indexed_head_for_test(1, 100);
        store.insert_matched_execution_for_test(
            1,
            "exec-1",
            "0xabc",
            42,
            ExecutionCompletion::Complete,
        );
        let checker = IndexerCorrelationChecker {
            store: &store,
            deployment_id: 1,
        };
        let out = checker.check("cid", "0xabc", 42, false).await.unwrap();
        assert_eq!(
            out,
            CorrelationOutcome::MatchedRowComplete {
                matched_execution_id: "exec-1".to_string()
            }
        );
    }
}
