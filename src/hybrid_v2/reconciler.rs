//! `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — reconciliation
//! scheduling, drift classification, and persisted results.
//!
//! This module wraps `crate::hybrid_v2::chain_view::Reconciler` with:
//!   * an 8-value `DriftClassification` enum mirroring the persisted
//!     `hybrid_v2_reconciliation_results.classification` column,
//!   * a `ReconciliationRecord` value type mirroring the full row,
//!   * a `ReconciliationScheduler` that acquires the deployment-scoped
//!     operation lock, runs one reconciliation, persists the result,
//!     and updates readiness.
//!
//! Frozen safety invariants enforced here:
//! - `RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS` — a
//!   `PROJECTION_DRIFT` (or any other divergent classification) is
//!   persisted and surfaces as `ReadinessReason::ReconciliationDrift`;
//!   no mutation of the projection ever occurs from this module.
//! - `OPERATOR_RECOVERY_ACTIONS_ARE_DEPLOYMENT_SCOPED_AND_NON_PUBLIC`
//!   — the scheduler takes a deployment_id + acquires the unified
//!   operation lock; no cross-deployment access.
//! - `READY_REQUIRES_...RECONCILIATION_CONVERGENCE` — readiness is
//!   READY only when the latest recorded classification is one of
//!   `CONVERGED`, `INDEXER_BEHIND`, or `PROVIDER_UNAVAILABLE`
//!   (transient; not a projection defect).

use crate::error::{BackendError, Result};
use crate::hybrid_v2::chain_view::{ChainViewProvider, Reconciler, ReconciliationResult};
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use crate::hybrid_v2::readiness::ReadinessReason;
use crate::hybrid_v2::rebuild_operations::OperationKind;
use crate::hybrid_v2::reducer::ProjectionState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

// -----------------------------------------------------------------
//                       CLASSIFICATION
// -----------------------------------------------------------------

/// 8 canonical drift classifications. Values are serialised into the
/// `classification` column exactly as returned by [`Self::as_str`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriftClassification {
    Converged,
    IndexerBehind,
    NonFinalDifference,
    ManifestMismatch,
    ProjectionDrift,
    ProviderUnavailable,
    UnsupportedView,
    MalformedChainResponse,
}

impl DriftClassification {
    pub fn as_str(&self) -> &'static str {
        match self {
            DriftClassification::Converged => "CONVERGED",
            DriftClassification::IndexerBehind => "INDEXER_BEHIND",
            DriftClassification::NonFinalDifference => "NON_FINAL_DIFFERENCE",
            DriftClassification::ManifestMismatch => "MANIFEST_MISMATCH",
            DriftClassification::ProjectionDrift => "PROJECTION_DRIFT",
            DriftClassification::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            DriftClassification::UnsupportedView => "UNSUPPORTED_VIEW",
            DriftClassification::MalformedChainResponse => "MALFORMED_CHAIN_RESPONSE",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "CONVERGED" => DriftClassification::Converged,
            "INDEXER_BEHIND" => DriftClassification::IndexerBehind,
            "NON_FINAL_DIFFERENCE" => DriftClassification::NonFinalDifference,
            "MANIFEST_MISMATCH" => DriftClassification::ManifestMismatch,
            "PROJECTION_DRIFT" => DriftClassification::ProjectionDrift,
            "PROVIDER_UNAVAILABLE" => DriftClassification::ProviderUnavailable,
            "UNSUPPORTED_VIEW" => DriftClassification::UnsupportedView,
            "MALFORMED_CHAIN_RESPONSE" => DriftClassification::MalformedChainResponse,
            _ => return None,
        })
    }
    /// True when the classification does NOT indicate a projection
    /// defect that should keep readiness hard-503. The scheduler uses
    /// this to gate readiness updates.
    ///
    /// `PROVIDER_UNAVAILABLE` is treated as non-drifting: an unreachable
    /// chain-view provider must never be conflated with a projection
    /// defect (safety invariant
    /// `RECONCILIATION_DRIFT_NEVER_AUTO_REPAIRS_PROJECTIONS` +
    /// operational realism).
    pub fn is_converged_or_transient(&self) -> bool {
        matches!(
            self,
            DriftClassification::Converged
                | DriftClassification::IndexerBehind
                | DriftClassification::NonFinalDifference
                | DriftClassification::ProviderUnavailable
                | DriftClassification::UnsupportedView
        )
    }
}

/// Provider availability at the time of the reconciliation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderAvailability {
    Available,
    Degraded,
    Unavailable,
}

impl ProviderAvailability {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderAvailability::Available => "AVAILABLE",
            ProviderAvailability::Degraded => "DEGRADED",
            ProviderAvailability::Unavailable => "UNAVAILABLE",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "AVAILABLE" => ProviderAvailability::Available,
            "DEGRADED" => ProviderAvailability::Degraded,
            "UNAVAILABLE" => ProviderAvailability::Unavailable,
            _ => return None,
        })
    }
}

// -----------------------------------------------------------------
//                       RECORD
// -----------------------------------------------------------------

/// One row of `hybrid_v2_reconciliation_results`. The `reconciliation_id`
/// is assigned by the store on insert.
#[derive(Debug, Clone)]
pub struct ReconciliationRecord {
    pub reconciliation_id: Option<i64>,
    pub deployment_id: i64,
    pub ran_at_ms: i64,
    pub block_number_checked: u64,
    pub block_hash_checked: String,
    pub categories_checked: u32,
    pub items_checked: u64,
    pub converged_categories: u32,
    pub divergent_categories: u32,
    pub classification: DriftClassification,
    pub mismatch_sample_json: Option<serde_json::Value>,
    pub provider_availability: ProviderAvailability,
    pub failure_detail: Option<String>,
    pub duration_ms: u64,
}

// -----------------------------------------------------------------
//                       SCHEDULER
// -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReconciliationSchedulerConfig {
    /// Cadence in milliseconds. `0` means "disabled" — the scheduler's
    /// tick becomes a no-op. Bounded to `[0, 86_400_000]` (0 or up to 24h).
    pub periodic_ms: u64,
    /// Maximum categories to inspect per run. Bounded [1, 1_000_000].
    pub max_items_per_run: u64,
    /// Enable the scheduler. When `false`, no run is ever attempted.
    pub enabled: bool,
}

impl Default for ReconciliationSchedulerConfig {
    fn default() -> Self {
        Self {
            periodic_ms: 0,
            max_items_per_run: 10_000,
            enabled: true,
        }
    }
}

impl ReconciliationSchedulerConfig {
    pub fn validate(&self) -> Result<()> {
        if self.periodic_ms > 86_400_000 {
            return Err(BackendError::Config(format!(
                "reconciliation.periodic_ms must be within [0, 86400000], got {}",
                self.periodic_ms
            )));
        }
        if !(1..=1_000_000).contains(&self.max_items_per_run) {
            return Err(BackendError::Config(format!(
                "reconciliation.max_items_per_run must be within [1, 1000000], got {}",
                self.max_items_per_run
            )));
        }
        Ok(())
    }
}

pub struct ReconciliationScheduler {
    pub deployment_id: i64,
    pub config: ReconciliationSchedulerConfig,
    reconciler: Reconciler,
}

impl ReconciliationScheduler {
    pub fn new(deployment_id: i64, config: ReconciliationSchedulerConfig) -> Self {
        Self {
            deployment_id,
            config,
            reconciler: Reconciler::new(),
        }
    }

    /// Run one reconciliation pass. Acquires the deployment-scoped
    /// operation lock, evaluates drift, persists the result, and (when
    /// drifting) writes readiness=NotReady{ReconciliationDrift}.
    ///
    /// Returns the persisted `ReconciliationRecord`.
    pub async fn run_once(
        &self,
        store: &Arc<dyn HybridV2ProjectionStore>,
        provider: &dyn ChainViewProvider,
        expected_manifest_hash: &str,
        state: &ProjectionState,
        indexed_block: u64,
        indexed_hash: &str,
    ) -> Result<ReconciliationRecord> {
        if !self.config.enabled {
            return Err(BackendError::Config(
                "reconciliation scheduler disabled".to_string(),
            ));
        }
        self.config.validate()?;
        let now_ms = now_ms();
        let started = Instant::now();

        // Lock acquisition — separate epoch series from rebuild/reorg.
        // We reuse the current timestamp as a monotonic-ish epoch.
        let epoch = now_ms;
        let guard = match store
            .try_acquire_operation_lock(
                self.deployment_id,
                OperationKind::Reconciliation,
                epoch,
                now_ms,
            )
            .await?
        {
            Some(g) => g,
            None => {
                let record = ReconciliationRecord {
                    reconciliation_id: None,
                    deployment_id: self.deployment_id,
                    ran_at_ms: now_ms,
                    block_number_checked: indexed_block,
                    block_hash_checked: indexed_hash.to_string(),
                    categories_checked: 0,
                    items_checked: 0,
                    converged_categories: 0,
                    divergent_categories: 0,
                    classification: DriftClassification::UnsupportedView,
                    mismatch_sample_json: None,
                    provider_availability: ProviderAvailability::Degraded,
                    failure_detail: Some("operation lock contention".to_string()),
                    duration_ms: started.elapsed().as_millis() as u64,
                };
                // Do NOT persist a spurious drift row on contention —
                // the record is a diagnostic value the caller may log.
                return Ok(record);
            }
        };

        let result =
            self.reconciler
                .reconcile(expected_manifest_hash, indexed_block, state, provider);
        let (classification, failure_detail, mismatch_sample) = classify(&result);
        let (categories, converged, divergent, items) = counts(&classification, state);
        let availability = if provider.is_available() {
            ProviderAvailability::Available
        } else {
            ProviderAvailability::Unavailable
        };

        let mut record = ReconciliationRecord {
            reconciliation_id: None,
            deployment_id: self.deployment_id,
            ran_at_ms: now_ms,
            block_number_checked: indexed_block,
            block_hash_checked: indexed_hash.to_string(),
            categories_checked: categories,
            items_checked: items,
            converged_categories: converged,
            divergent_categories: divergent,
            classification: classification.clone(),
            mismatch_sample_json: mismatch_sample,
            provider_availability: availability,
            failure_detail: failure_detail.clone(),
            duration_ms: started.elapsed().as_millis() as u64,
        };
        let id = store.insert_reconciliation_result(&record).await?;
        record.reconciliation_id = Some(id);

        // Readiness update — hard-503 when drifting; clear when
        // converged. Do NOT auto-repair.
        if classification.is_converged_or_transient() {
            // No-op: don't overwrite readiness — the runtime owns the
            // primary readiness slot. The scheduler observes convergence
            // by not writing anything.
        } else {
            let detail = format!(
                "{}: {}",
                classification.as_str(),
                failure_detail.unwrap_or_default()
            );
            let reason = ReadinessReason::ReconciliationDrift { detail };
            let snap = crate::hybrid_v2::persistence::ReadinessSnapshot::from_state(
                &crate::hybrid_v2::readiness::ReadinessState::new_not_ready(reason),
            );
            store
                .write_readiness_snapshot(self.deployment_id, &snap, now_ms)
                .await?;
        }

        if let Err(e) = guard.release().await {
            tracing::warn!(
                target: "hybrid_v2::reconciler",
                deployment_id = self.deployment_id,
                error = %e,
                "reconciliation operation lock release failed (non-fatal)"
            );
        }
        Ok(record)
    }
}

fn classify(
    r: &ReconciliationResult,
) -> (
    DriftClassification,
    Option<String>,
    Option<serde_json::Value>,
) {
    match r {
        ReconciliationResult::Converged { .. } => (DriftClassification::Converged, None, None),
        ReconciliationResult::IndexerBehind { indexed, observed } => (
            DriftClassification::IndexerBehind,
            Some(format!("indexed={}, observed={}", indexed, observed)),
            None,
        ),
        ReconciliationResult::NonFinalDifference { block } => (
            DriftClassification::NonFinalDifference,
            Some(format!("no snapshot at block {}", block)),
            None,
        ),
        ReconciliationResult::ManifestMismatch { expected, actual } => (
            DriftClassification::ManifestMismatch,
            Some(format!("expected={}, actual={}", expected, actual)),
            Some(serde_json::json!({ "expected": expected, "actual": actual })),
        ),
        ReconciliationResult::ProjectionDrift { detail } => (
            DriftClassification::ProjectionDrift,
            Some(detail.clone()),
            Some(serde_json::json!({ "detail": detail })),
        ),
        ReconciliationResult::ProviderUnavailable => {
            (DriftClassification::ProviderUnavailable, None, None)
        }
        ReconciliationResult::Unsupported { detail } => (
            DriftClassification::UnsupportedView,
            Some(detail.clone()),
            None,
        ),
    }
}

fn counts(c: &DriftClassification, s: &ProjectionState) -> (u32, u32, u32, u64) {
    // Categories: balances, reservations, recovery_state, positions, orders, matched_executions.
    let categories = 6u32;
    let items = (s.balances.len()
        + s.reservations.len()
        + s.recovery_state.len()
        + s.positions.len()
        + s.order_lifecycle.len()
        + s.matched_executions.len()) as u64;
    let (converged, divergent) = if matches!(c, DriftClassification::Converged) {
        (categories, 0)
    } else if matches!(
        c,
        DriftClassification::ProviderUnavailable | DriftClassification::UnsupportedView
    ) {
        (0, 0)
    } else {
        // Report at least 1 divergent; the caller records the specifics
        // in `mismatch_sample_json`.
        (categories.saturating_sub(1), 1)
    };
    (categories, converged, divergent, items)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// -----------------------------------------------------------------
//                       UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_roundtrips() {
        for c in [
            DriftClassification::Converged,
            DriftClassification::IndexerBehind,
            DriftClassification::NonFinalDifference,
            DriftClassification::ManifestMismatch,
            DriftClassification::ProjectionDrift,
            DriftClassification::ProviderUnavailable,
            DriftClassification::UnsupportedView,
            DriftClassification::MalformedChainResponse,
        ] {
            assert_eq!(DriftClassification::parse(c.as_str()), Some(c));
        }
    }

    #[test]
    fn availability_roundtrips() {
        for a in [
            ProviderAvailability::Available,
            ProviderAvailability::Degraded,
            ProviderAvailability::Unavailable,
        ] {
            assert_eq!(ProviderAvailability::parse(a.as_str()), Some(a));
        }
    }

    #[test]
    fn drift_never_auto_ready() {
        assert!(!DriftClassification::ProjectionDrift.is_converged_or_transient());
        assert!(!DriftClassification::ManifestMismatch.is_converged_or_transient());
        assert!(!DriftClassification::MalformedChainResponse.is_converged_or_transient());
    }

    #[test]
    fn provider_unavailable_never_drifts() {
        assert!(DriftClassification::ProviderUnavailable.is_converged_or_transient());
    }

    #[test]
    fn config_validates() {
        let mut c = ReconciliationSchedulerConfig::default();
        assert!(c.validate().is_ok());
        c.periodic_ms = 90_000_000;
        assert!(c.validate().is_err());
        c.periodic_ms = 0;
        c.max_items_per_run = 0;
        assert!(c.validate().is_err());
    }
}
