//! Runtime readiness state + reasons.
//!
//! `ReadinessState` is a boolean plus (when not-ready) a reason. The
//! deployment is exposed as "ready" only when the runtime, projections,
//! reorg model, rebuild and reconciliation all converge.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadinessReason {
    AwaitingFirstBlock,
    WrongChain { manifest: u64, source: u64 },
    ManifestMismatch { detail: String },
    UnknownCanonicalEvent { topic0: String, block: u64 },
    DecodeFailure { block: u64, detail: String },
    ProjectionFailure { block: u64, detail: String },
    CursorHashMismatch { block: u64 },
    ExcessiveReorg { depth: u64, max: u64 },
    RebuildInProgress,
    RebuildFailed { detail: String },
    ReconciliationDrift { detail: String },
    MigrationSchemaMismatch,
    Behind { observed: u64, indexed: u64 },
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
