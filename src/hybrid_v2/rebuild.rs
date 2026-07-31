//! DB-loss rebuild service.
//!
//! Runs an in-memory rebuild of every canonical projection from the raw
//! canonical journal alone. Callers wire this into a persistence-backed
//! service that:
//! 1. Selects one explicit deployment.
//! 2. Validates its manifest (via `ManifestValidator`).
//! 3. Acquires the existing repository rebuild lock.
//! 4. Clears rebuildable projection tables (raw journal preserved).
//! 5. Streams raw logs into `RebuildService::replay_all`.
//! 6. Diffs against reconciler; publishes readiness only after
//!    convergence.

use crate::hybrid_v2::correlation::ExecutionCorrelator;
use crate::hybrid_v2::decoder::{decode_log_with, CanonicalRawLog};
use crate::hybrid_v2::reducer::{apply, ApplyContext, ProjectionState};
use crate::hybrid_v2::runtime::JournaledLog;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RebuildOutcome {
    pub events_replayed: u64,
    pub decode_failures: u64,
    pub projection_failures: u64,
    pub final_head_block: u64,
}

pub struct RebuildService {
    pub expected_event_version: u16,
}

impl RebuildService {
    pub fn new(expected_event_version: u16) -> Self {
        Self {
            expected_event_version,
        }
    }

    /// Deterministically rebuild `state` by replaying the canonical
    /// portion of `journal` in canonical order (block, then log_index).
    pub fn replay_all(
        &self,
        journal: &[JournaledLog],
    ) -> (ProjectionState, ExecutionCorrelator, RebuildOutcome) {
        let mut state = ProjectionState::default();
        let mut correlator = ExecutionCorrelator::new();
        let mut outcome = RebuildOutcome {
            events_replayed: 0,
            decode_failures: 0,
            projection_failures: 0,
            final_head_block: 0,
        };
        // Sort canonical logs by (block_number, log_index).
        let mut logs: Vec<&JournaledLog> = journal.iter().filter(|l| l.is_canonical).collect();
        logs.sort_by(|a, b| (a.block_number, a.log_index).cmp(&(b.block_number, b.log_index)));
        let mut last_tx: Option<String> = None;
        for log in logs {
            let ctx = ApplyContext {
                block_number: log.block_number,
                tx_hash: log.tx_hash.clone(),
                log_index: log.log_index,
                block_timestamp: log.block_timestamp,
            };
            if let Some(prev) = last_tx.as_ref() {
                if prev != &ctx.tx_hash {
                    correlator.flush_transaction(prev, &mut state);
                }
            }
            last_tx = Some(ctx.tx_hash.clone());
            let raw = CanonicalRawLog {
                emitter: log.emitter.clone(),
                topics: log.topics.clone(),
                data: log.data.clone(),
            };
            let event = match decode_log_with(&raw, None, self.expected_event_version) {
                Ok(e) => e,
                Err(_) => {
                    outcome.decode_failures += 1;
                    continue;
                }
            };
            if apply(&mut state, &event, &ctx).is_err() {
                outcome.projection_failures += 1;
                continue;
            }
            correlator.observe(&event, &ctx, &mut state);
            outcome.events_replayed += 1;
            outcome.final_head_block = log.block_number;
        }
        if let Some(tx) = last_tx {
            correlator.flush_transaction(&tx, &mut state);
        }
        (state, correlator, outcome)
    }
}
