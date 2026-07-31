//! Canonicality + readiness metadata attached to every response.

use crate::hybrid_v2::readiness::{ReadinessReason, ReadinessState};
use crate::hybrid_v2::runtime::RuntimeCursor;
use serde::{Deserialize, Serialize};

/// How fresh / trustworthy the data in the response is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CanonicalityLevel {
    Finalized,
    IndexedCanonical,
    IndexerBehind,
    NotReady,
}

/// Requested consistency mode for the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConsistencyMode {
    #[default]
    Indexed,
    Finalized,
}

impl ConsistencyMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "indexed" => Some(Self::Indexed),
            "finalized" => Some(Self::Finalized),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Indexed => "INDEXED",
            Self::Finalized => "FINALIZED",
        }
    }
}

/// Consistency metadata attached to every canonical response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalityMetadata {
    pub deployment_id: u64,
    pub chain_id: u64,
    pub manifest_hash: String,
    pub indexed_block: u64,
    pub indexed_block_hash: String,
    pub finalized_block: u64,
    pub observed_head_block: u64,
    pub indexer_lag: u64,
    pub reconciliation_status: String,
    pub canonicality_level: CanonicalityLevel,
    pub consistency_mode: ConsistencyMode,
    pub generated_at_ms: u64,
}

impl CanonicalityMetadata {
    pub fn from_runtime(
        deployment_id: u64,
        chain_id: u64,
        manifest_hash: &str,
        cursor: &RuntimeCursor,
        observed_head: u64,
        finalized_head: u64,
        reconciliation_status: &str,
        readiness: &ReadinessState,
        consistency: ConsistencyMode,
        generated_at_ms: u64,
    ) -> Self {
        let lag = observed_head.saturating_sub(cursor.indexed_head_block);
        let canonicality_level = if !readiness.ready {
            CanonicalityLevel::NotReady
        } else if consistency == ConsistencyMode::Finalized
            && cursor.indexed_head_block <= finalized_head
        {
            CanonicalityLevel::Finalized
        } else if lag > 0 {
            CanonicalityLevel::IndexerBehind
        } else {
            CanonicalityLevel::IndexedCanonical
        };
        Self {
            deployment_id,
            chain_id,
            manifest_hash: manifest_hash.to_string(),
            indexed_block: cursor.indexed_head_block,
            indexed_block_hash: cursor.indexed_head_hash.clone(),
            finalized_block: finalized_head,
            observed_head_block: observed_head,
            indexer_lag: lag,
            reconciliation_status: reconciliation_status.to_string(),
            canonicality_level,
            consistency_mode: consistency,
            generated_at_ms,
        }
    }
}

/// Readiness gate. Returns Some(reason) when canonical routes must fail
/// with 503; None when the response may proceed (possibly with `IndexerBehind`).
pub fn hard_readiness_failure(readiness: &ReadinessState) -> Option<&'static str> {
    if readiness.ready {
        return None;
    }
    // `Behind` and `AwaitingFirstBlock` are non-fatal — canonical responses may
    // still proceed with metadata indicating lag / not-ready. Every other
    // reason blocks canonical routes with a structured 503.
    match readiness.reason.as_ref() {
        Some(ReadinessReason::Behind { .. }) => None,
        Some(ReadinessReason::AwaitingFirstBlock) => Some("AWAITING_FIRST_BLOCK"),
        Some(ReadinessReason::WrongChain { .. }) => Some("WRONG_CHAIN"),
        Some(ReadinessReason::ManifestMismatch { .. }) => Some("MANIFEST_MISMATCH"),
        Some(ReadinessReason::UnknownCanonicalEvent { .. }) => Some("UNKNOWN_CANONICAL_EVENT"),
        Some(ReadinessReason::DecodeFailure { .. }) => Some("DECODE_FAILURE"),
        Some(ReadinessReason::ProjectionFailure { .. }) => Some("PROJECTION_FAILURE"),
        Some(ReadinessReason::CursorHashMismatch { .. }) => Some("CURSOR_HASH_MISMATCH"),
        Some(ReadinessReason::ExcessiveReorg { .. }) => Some("EXCESSIVE_REORG"),
        Some(ReadinessReason::RebuildInProgress) => Some("REBUILD_IN_PROGRESS"),
        Some(ReadinessReason::RebuildFailed { .. }) => Some("REBUILD_FAILED"),
        Some(ReadinessReason::ReconciliationDrift { .. }) => Some("RECONCILIATION_DRIFT"),
        Some(ReadinessReason::MigrationSchemaMismatch) => Some("MIGRATION_SCHEMA_MISMATCH"),
        None => None,
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
