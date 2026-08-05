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
        Some(ReadinessReason::RebuildRequested { .. }) => Some("REBUILD_REQUESTED"),
        Some(ReadinessReason::RebuildFailed { .. }) => Some("REBUILD_FAILED"),
        Some(ReadinessReason::ReconciliationInProgress { .. }) => {
            Some("RECONCILIATION_IN_PROGRESS")
        }
        Some(ReadinessReason::ReconciliationDrift { .. }) => Some("RECONCILIATION_DRIFT"),
        Some(ReadinessReason::MigrationSchemaMismatch) => Some("MIGRATION_SCHEMA_MISMATCH"),
        Some(ReadinessReason::Bootstrapping) => Some("BOOTSTRAPPING"),
        Some(ReadinessReason::Stopping) => Some("STOPPING"),
        Some(ReadinessReason::ReorgDetected { .. }) => Some("REORG_DETECTED"),
        Some(ReadinessReason::ReorgSearching { .. }) => Some("REORG_SEARCHING"),
        Some(ReadinessReason::ReorgReplaying { .. }) => Some("REORG_REPLAYING"),
        Some(ReadinessReason::ReorgManualInterventionRequired { .. }) => {
            Some("REORG_MANUAL_INTERVENTION_REQUIRED")
        }
        None => None,
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// -----------------------------------------------------------------
// `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1`
//
// Store-boundary variants of `from_runtime` / `hard_readiness_failure`.
// Handlers now source readiness + head/lag via
// `HybridV2ReadStore::get_deployment_status` — no direct runtime access.
// -----------------------------------------------------------------

/// Build `CanonicalityMetadata` from a store-returned
/// `DeploymentStatusRecord` and the immutable per-deployment manifest.
pub fn build_metadata_from_status(
    deployment_id: u64,
    chain_id: u64,
    manifest_hash: &str,
    status: &crate::hybrid_v2::read_store::DeploymentStatusRecord,
    consistency: ConsistencyMode,
    generated_at_ms: u64,
) -> CanonicalityMetadata {
    let lag = status
        .observed_head_block
        .saturating_sub(status.indexed_head_block);
    let canonicality_level = if !status.ready {
        CanonicalityLevel::NotReady
    } else if consistency == ConsistencyMode::Finalized
        && status.indexed_head_block <= status.finalized_head_block
    {
        CanonicalityLevel::Finalized
    } else if lag > 0 {
        CanonicalityLevel::IndexerBehind
    } else {
        CanonicalityLevel::IndexedCanonical
    };
    CanonicalityMetadata {
        deployment_id,
        chain_id,
        manifest_hash: manifest_hash.to_string(),
        indexed_block: status.indexed_head_block,
        indexed_block_hash: status.indexed_head_hash.clone(),
        finalized_block: status.finalized_head_block,
        observed_head_block: status.observed_head_block,
        indexer_lag: lag,
        reconciliation_status: status
            .reconciliation_status
            .clone()
            .unwrap_or_else(|| "UNKNOWN".to_string()),
        canonicality_level,
        consistency_mode: consistency,
        generated_at_ms,
    }
}

/// Readiness gate parameterised on the persisted `ready` flag + a
/// stringified `ready_reason` (as emitted by the store). Mirrors
/// `hard_readiness_failure`'s runtime-enum policy: `Behind` and a
/// missing reason are soft-fails; every explicit failure class is a
/// hard 503.
pub fn hard_readiness_failure_from_status(
    ready: bool,
    ready_reason: Option<&str>,
) -> Option<&'static str> {
    if ready {
        return None;
    }
    let Some(r) = ready_reason else {
        // Not ready but no reason recorded — refuse to serve canonical
        // data rather than pretending everything is fine.
        return Some("INDEXER_NOT_READY");
    };
    // Match by prefix of the debug form of `ReadinessReason` so this
    // works for both the runtime (`{:?}` output) and the Postgres
    // store (persisted string).
    let bytes = r.as_bytes();
    // Cheap prefix compares to avoid allocating a lower-case copy.
    fn starts_ci(hay: &[u8], needle: &[u8]) -> bool {
        if hay.len() < needle.len() {
            return false;
        }
        hay.iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }
    if starts_ci(bytes, b"Behind") {
        return None;
    }
    if starts_ci(bytes, b"AwaitingFirstBlock") {
        return Some("AWAITING_FIRST_BLOCK");
    }
    if starts_ci(bytes, b"WrongChain") {
        return Some("WRONG_CHAIN");
    }
    if starts_ci(bytes, b"ManifestMismatch") {
        return Some("MANIFEST_MISMATCH");
    }
    if starts_ci(bytes, b"UnknownCanonicalEvent") {
        return Some("UNKNOWN_CANONICAL_EVENT");
    }
    if starts_ci(bytes, b"DecodeFailure") {
        return Some("DECODE_FAILURE");
    }
    if starts_ci(bytes, b"ProjectionFailure") {
        return Some("PROJECTION_FAILURE");
    }
    if starts_ci(bytes, b"CursorHashMismatch") {
        return Some("CURSOR_HASH_MISMATCH");
    }
    if starts_ci(bytes, b"ExcessiveReorg") {
        return Some("EXCESSIVE_REORG");
    }
    if starts_ci(bytes, b"RebuildInProgress") || starts_ci(bytes, b"REBUILD_IN_PROGRESS") {
        return Some("REBUILD_IN_PROGRESS");
    }
    if starts_ci(bytes, b"RebuildRequested") || starts_ci(bytes, b"REBUILD_REQUESTED") {
        return Some("REBUILD_REQUESTED");
    }
    if starts_ci(bytes, b"RebuildFailed") || starts_ci(bytes, b"REBUILD_FAILED") {
        return Some("REBUILD_FAILED");
    }
    if starts_ci(bytes, b"ReconciliationInProgress")
        || starts_ci(bytes, b"RECONCILIATION_IN_PROGRESS")
    {
        return Some("RECONCILIATION_IN_PROGRESS");
    }
    if starts_ci(bytes, b"ReconciliationDrift") || starts_ci(bytes, b"RECONCILIATION_DRIFT") {
        return Some("RECONCILIATION_DRIFT");
    }
    if starts_ci(bytes, b"MigrationSchemaMismatch") {
        return Some("MIGRATION_SCHEMA_MISMATCH");
    }
    if starts_ci(bytes, b"Bootstrapping") || starts_ci(bytes, b"BOOTSTRAPPING") {
        return Some("BOOTSTRAPPING");
    }
    if starts_ci(bytes, b"Stopping") || starts_ci(bytes, b"STOPPING") {
        return Some("STOPPING");
    }
    if starts_ci(bytes, b"ReorgDetected") || starts_ci(bytes, b"REORG_DETECTED") {
        return Some("REORG_DETECTED");
    }
    if starts_ci(bytes, b"ReorgSearching") || starts_ci(bytes, b"REORG_SEARCHING") {
        return Some("REORG_SEARCHING");
    }
    if starts_ci(bytes, b"ReorgReplaying") || starts_ci(bytes, b"REORG_REPLAYING") {
        return Some("REORG_REPLAYING");
    }
    if starts_ci(bytes, b"ReorgManualInterventionRequired")
        || starts_ci(bytes, b"REORG_MANUAL_INTERVENTION_REQUIRED")
    {
        return Some("REORG_MANUAL_INTERVENTION_REQUIRED");
    }
    Some("INDEXER_NOT_READY")
}
