//! `BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1` — Hybrid V2 canonical
//! projection layer.
//!
//! Frozen canonicality rule:
//! `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_PROJECTION`.
//!
//! PostgreSQL may store raw chain logs, decoded events, deterministic
//! projections, derived query models, reconciliation status, and indexer
//! checkpoints. PostgreSQL must NEVER independently decide any canonical
//! economic fact (ownership / balances / reservations / positions /
//! order lifecycle / recovery state / fee accounting).
//!
//! Every projection row is traceable to a raw log OR to immutable
//! manifest data OR to a bounded canonical contract view used for
//! verification.
//!
//! Status: `EXPERIMENTAL — NOT SECURITY APPROVED`.

pub mod chain_source;
pub mod chain_view;
pub mod correlation;
pub mod decoder;
pub mod events;
pub mod manifest;
pub mod persistence;
pub mod read_store;
pub mod readiness;
pub mod rebuild;
pub mod reducer;
pub mod reorg;
pub mod repository;
pub mod runtime;
pub mod runtime_backed_read_store;
pub mod snapshot;
pub mod topics;

pub use chain_source::{ChainSource, InMemoryChainSource, RawBlock};
pub use chain_view::{
    ChainViewProvider, InMemoryChainViewProvider, Reconciler, ReconciliationResult,
};
pub use correlation::{ExecutionCorrelator, ExecutionGroup};
pub use events::{EventKind, HybridV2Event};
pub use manifest::{ManifestParams, ManifestValidationError, ManifestValidator, NetworkPolicy};
pub use persistence::{
    CanonicalBlockRef, HybridV2ProjectionStore, InMemoryProjectionStore,
    PostgresHybridV2ProjectionStore, ReadinessSnapshot, RuntimeCursorSnapshot,
};
pub use read_store::{
    filter_stable_hash, CollateralRecord, DeploymentListRecord, DeploymentStatusRecord,
    FeeRebateRecord, HistoryConsistency, HistoryCursorKey, HistoryPage, HistoryPageAnchor,
    HistoryRecord, HistoryScope, HybridV2ReadStore, InMemoryHybridV2ReadStore,
    InMemoryJournalEntry, InMemoryStoreBuilder, MatchedExecutionRecord, OrderLifecycleRecord,
    PageAnchor, PositionRecord, PostgresHybridV2ReadStore, ReadStoreError, RecoveryRecord,
    ReservationRecord, StorePage, SubaccountRecord, SubaccountSummaryRecord,
};
pub use readiness::{ReadinessReason, ReadinessReport, ReadinessState};
pub use rebuild::{RebuildOutcome, RebuildService};
pub use reducer::{
    ApplyContext, EscapeStateRow, FeeEventRow, MatchedExecutionRow, OrderLifecycleRow, PositionRow,
    ProjectionState, RecoveryStateProjection, ReducerError,
};
pub use reorg::{ReorgOutcome, ReorgPlanner};
pub use repository::{HybridV2QueryRepository, PageCursor};
pub use runtime::{IndexerRuntime, RuntimeError, RuntimeMetrics};
pub use snapshot::{PinnedSnapshots, SourceMetadata};
pub use topics::TopicCatalogue;
