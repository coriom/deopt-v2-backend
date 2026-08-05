//! Runtime readiness state + reasons.
//!
//! `ReadinessState` is a boolean plus (when not-ready) a reason. The
//! deployment is exposed as "ready" only when the runtime, projections,
//! reorg model, rebuild and reconciliation all converge.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReadinessReason {
    #[default]
    AwaitingFirstBlock,
    WrongChain {
        manifest: u64,
        source: u64,
    },
    ManifestMismatch {
        detail: String,
    },
    UnknownCanonicalEvent {
        topic0: String,
        block: u64,
    },
    DecodeFailure {
        block: u64,
        detail: String,
    },
    ProjectionFailure {
        block: u64,
        detail: String,
    },
    CursorHashMismatch {
        block: u64,
    },
    ExcessiveReorg {
        depth: u64,
        max: u64,
    },
    RebuildInProgress,
    /// A rebuild has been requested/enqueued for this deployment but
    /// has not yet acquired the operation lock. Hard-503 until the
    /// state machine advances.
    RebuildRequested {
        epoch: i64,
    },
    RebuildFailed {
        detail: String,
    },
    /// The reconciliation scheduler is actively running (holds the
    /// deployment-scoped operation lock). Hard-503 while active.
    ReconciliationInProgress {
        epoch: i64,
    },
    ReconciliationDrift {
        detail: String,
    },
    MigrationSchemaMismatch,
    Behind {
        observed: u64,
        indexed: u64,
    },
    /// Runtime is running `bootstrap_from_persistence` — transient
    /// in-memory only; never persisted. Cleared on the next successful
    /// tick / handler read that follows bootstrap completion.
    Bootstrapping,
    /// Runtime received a shutdown signal and is winding down; hard 503
    /// until the process exits.
    Stopping,
    /// Reorg recovery is in-flight: parent hash mismatch has been
    /// detected and persisted, but the ancestor search + replay have
    /// not yet completed. Hard-503 until the persisted recovery
    /// transitions to RECOVERED.
    ReorgDetected {
        at_block: u64,
        epoch: i64,
    },
    /// The recovery service is walking back the local canonical journal
    /// looking for a common ancestor with the live chain.
    ReorgSearching {
        epoch: i64,
    },
    /// Ancestor found; the recovery service is invalidating the orphan
    /// branch and replaying the replacement branch inside a single
    /// Postgres transaction.
    ReorgReplaying {
        epoch: i64,
        ancestor: u64,
    },
    /// Recovery escalated past the retry budget or crossed a finalized
    /// boundary. The worker for this deployment stops advancing until
    /// an operator restart clears the state.
    ReorgManualInterventionRequired {
        epoch: i64,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessState {
    pub ready: bool,
    pub reason: Option<ReadinessReason>,
}

impl ReadinessState {
    pub fn ready() -> Self {
        Self {
            ready: true,
            reason: None,
        }
    }
    pub fn new_not_ready(reason: ReadinessReason) -> Self {
        Self {
            ready: false,
            reason: Some(reason),
        }
    }
}

/// Aggregated report bundling runtime + rebuild + reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessReport {
    pub runtime: ReadinessState,
    pub rebuild: ReadinessState,
    pub reconciliation: ReadinessState,
}

impl ReadinessReport {
    pub fn is_ready(&self) -> bool {
        self.runtime.ready && self.rebuild.ready && self.reconciliation.ready
    }
}
