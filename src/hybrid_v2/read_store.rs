//! `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2A-STORE-AND-PG-TESTS-V1`
//!
//! Owned-domain async read store for the Hybrid V2 canonical projection.
//!
//! Canonicality rule (frozen):
//! `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`.
//!
//! Scope of this stage:
//! - Defines an async, object-safe `HybridV2ReadStore` trait covering
//!   every data-bearing read family required by the 16-route public
//!   API.
//! - Implements `PostgresHybridV2ReadStore` (production sqlx-backed).
//! - Provides `InMemoryHybridV2ReadStore` as a test-only adapter that
//!   reads directly from a `ProjectionState` snapshot for parity tests.
//! - **Does not** refactor the existing Axum handlers, does not swap
//!   the production `HybridV2ApiState` selection, does not activate any
//!   runtime writer. Those changes belong to stage 2B.
//!
//! Design notes:
//! - Every method is deployment-scoped: no query touches multiple
//!   deployments implicitly.
//! - Every method returns owned types. No lifetimes tied to a database
//!   transaction escape the store.
//! - Bounded page sizes are enforced at the store boundary; the max
//!   mirrors `hybrid_v2::repository::MAX_PAGE_LIMIT`.
//! - Integer round-trips use `TryFrom` so out-of-range values surface
//!   as `ReadStoreError::MalformedRow` rather than silent truncation.
//! - Errors carry no raw SQL or database URL.

use crate::api::hybrid_v2_read::history::{HistoryDirection, HistoryEventPayload, HistoryFilter};
use crate::hybrid_v2::events::EventKind;
use crate::hybrid_v2::reducer::{
    EscapeStateRow, ExecutionCompletion, ProjectionState, RecoveryStateProjection,
};
use crate::hybrid_v2::repository::MAX_PAGE_LIMIT;
use crate::hybrid_v2::runtime::RuntimeCursor;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::Row;
use std::fmt;

// -----------------------------------------------------------------
//                       ERROR TYPE
// -----------------------------------------------------------------

/// Typed store failure. Never carries raw SQL, credentials, or a
/// database URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadStoreError {
    /// A database backend failure. `detail` is a summary safe to log.
    Backend { detail: String },
    /// A persisted row is malformed (out-of-range integer, unknown
    /// enum, etc.). Not user-fixable; indicates projection corruption.
    MalformedRow { detail: String },
    /// The caller-supplied cursor is malformed or does not match the
    /// current query filter/deployment binding.
    InvalidCursor { detail: String },
    /// The caller-supplied cursor is bound to an indexed head hash
    /// that no longer matches — a reorg has invalidated the anchor.
    StaleCursor {
        expected_hash: String,
        actual_hash: String,
    },
    /// The caller-supplied limit exceeds the store maximum.
    LimitExceeded { max: usize, requested: usize },
    /// The caller passed a filter combination the store cannot execute
    /// (e.g. malformed hex, empty subKey).
    InvalidFilter { detail: String },
}

impl fmt::Display for ReadStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadStoreError::Backend { detail } => write!(f, "read store backend failure: {detail}"),
            ReadStoreError::MalformedRow { detail } => {
                write!(f, "read store malformed row: {detail}")
            }
            ReadStoreError::InvalidCursor { detail } => {
                write!(f, "read store invalid cursor: {detail}")
            }
            ReadStoreError::StaleCursor {
                expected_hash,
                actual_hash,
            } => write!(
                f,
                "read store stale cursor: expected head {expected_hash}, actual head {actual_hash}"
            ),
            ReadStoreError::LimitExceeded { max, requested } => write!(
                f,
                "read store limit exceeded: requested {requested} > max {max}"
            ),
            ReadStoreError::InvalidFilter { detail } => {
                write!(f, "read store invalid filter: {detail}")
            }
        }
    }
}

impl std::error::Error for ReadStoreError {}

fn backend_err<E: fmt::Display>(e: E) -> ReadStoreError {
    ReadStoreError::Backend {
        detail: e.to_string(),
    }
}

fn malformed<E: fmt::Display>(field: &str, e: E) -> ReadStoreError {
    ReadStoreError::MalformedRow {
        detail: format!("field {field}: {e}"),
    }
}

// -----------------------------------------------------------------
//                       DOMAIN RECORD TYPES
// -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentListRecord {
    pub deployment_id: u64,
    pub chain_id: u64,
    pub manifest_hash: String,
    pub manifest_address: String,
    pub deployment_version: u32,
    pub activation_status: String,
    pub max_collateral_tokens: u32,
    pub max_active_series: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentStatusRecord {
    pub deployment_id: u64,
    pub chain_id: u64,
    pub manifest_hash: String,
    pub indexed_head_block: u64,
    pub indexed_head_hash: String,
    pub observed_head_block: u64,
    pub finalized_head_block: u64,
    pub last_success_block: u64,
    pub reorg_count: u64,
    pub max_reorg_depth_seen: u64,
    pub decode_failures: u64,
    pub projection_failures: u64,
    pub unknown_canonical_events: u64,
    pub ready: bool,
    pub ready_reason: Option<String>,
    pub ready_reason_detail: Option<String>,
    pub rebuild_status: Option<String>,
    pub reconciliation_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubaccountRecord {
    pub deployment_id: u64,
    pub subkey: String,
    pub owner: String,
    pub subaccount_id: u32,
    pub materialised_via: String,
    pub created_block: u64,
    pub created_tx_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SubaccountSummaryRecord {
    pub identity: SubaccountRecord,
    pub materialised_via_created: bool,
    pub materialised_via_lazy: bool,
    pub recovery_state: String,
    pub finalized: bool,
    pub balance_token_count: usize,
    pub reservation_count: usize,
    pub active_series_count: usize,
    pub open_order_count: usize,
    pub owner_recovery_epoch: u64,
    pub subaccount_recovery_epoch: u64,
    pub escape: Option<EscapeStateRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollateralRecord {
    pub token: String,
    pub universe_index: Option<u32>,
    pub enabled: bool,
    pub balance: String,
    pub aggregate_reserved: String,
    pub available: String,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationRecord {
    pub subkey: String,
    pub token: String,
    pub engine: String,
    pub reserved: String,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionRecord {
    pub subkey: String,
    pub series_id: String,
    pub long_qty_1e8: String,
    pub short_qty_1e8: String,
    pub active: bool,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLifecycleRecord {
    pub order_hash: String,
    pub deployment_id: u64,
    pub subkey: String,
    pub owner: String,
    pub series_id: Option<String>,
    pub side: u8,
    pub time_in_force: u8,
    pub total_qty_1e8: String,
    pub filled_qty_1e8: String,
    pub remaining_qty_1e8: String,
    pub cancelled: bool,
    pub terminal: bool,
    pub first_seen_block: u64,
    pub last_event_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchedExecutionRecord {
    pub deployment_id: u64,
    pub execution_id: String,
    pub buyer_order_hash: String,
    pub seller_order_hash: String,
    pub buyer_subkey: String,
    pub seller_subkey: String,
    pub series_id: String,
    pub matched_qty_1e8: String,
    pub premium_amount: String,
    pub fee_amount: String,
    pub rebate_amount: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub completion_status: String,
}

impl MatchedExecutionRecord {
    pub fn is_complete(&self) -> bool {
        self.completion_status == "COMPLETE"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeRebateRecord {
    pub deployment_id: u64,
    pub event_id: i64,
    pub kind: String, // OPTION_FEE_CHARGED / OPTION_REBATE_PAID / OPTION_PREMIUM_TRANSFERRED
    pub payer_subkey: Option<String>,
    pub receiver_subkey: Option<String>,
    pub token: String,
    pub amount: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u32,
    pub execution_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecoveryRecord {
    pub subkey: String,
    pub recovery_state: String,
    pub finalized: bool,
    pub owner_recovery_epoch: u64,
    pub subaccount_recovery_epoch: u64,
    pub owner_min_valid_nonce: String,
    pub subaccount_min_valid_nonce: String,
    pub escape: Option<EscapeStateRow>,
    pub withdrawal_count: u32,
    pub finalized_at_ts: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub event_id: String,
    pub deployment_id: u64,
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub tx_hash: String,
    pub tx_index: u32,
    pub log_index: u32,
    pub timestamp_ms: u64,
    pub finalized: bool,
    pub direction: HistoryDirection,
    pub owner: Option<String>,
    pub subaccount_id: Option<u32>,
    pub subkey: Option<String>,
    pub related_order_hash: Option<String>,
    pub related_execution_id: Option<String>,
    pub payload: HistoryEventPayload,
}

// -----------------------------------------------------------------
//                       PAGINATION TYPES
// -----------------------------------------------------------------

/// Typed keyset anchor used for `list_positions` / `list_orders` /
/// `list_executions` / `list_fees`. The last-key is opaque to the
/// caller; the store internally encodes it as either a scalar SQL
/// keyset predicate (for order/execution/fee tables) or a composite
/// tuple (for history — see `HistoryPageAnchor`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageAnchor {
    /// Last item's stable key from the previous page, or None on first page.
    pub last_key: Option<String>,
    /// Requested page size. Clamped to [1, MAX_PAGE_LIMIT].
    pub limit: usize,
}

impl PageAnchor {
    pub fn first(limit: usize) -> Result<Self, ReadStoreError> {
        if limit > MAX_PAGE_LIMIT {
            return Err(ReadStoreError::LimitExceeded {
                max: MAX_PAGE_LIMIT,
                requested: limit,
            });
        }
        Ok(Self {
            last_key: None,
            limit: limit.max(1),
        })
    }

    pub fn after(last_key: String, limit: usize) -> Result<Self, ReadStoreError> {
        if limit > MAX_PAGE_LIMIT {
            return Err(ReadStoreError::LimitExceeded {
                max: MAX_PAGE_LIMIT,
                requested: limit,
            });
        }
        Ok(Self {
            last_key: Some(last_key),
            limit: limit.max(1),
        })
    }
}

/// Typed composite keyset anchor for history queries. Encodes the
/// canonical descending ordering:
///     (block_number DESC, tx_index DESC, log_index DESC, event_id DESC).
///
/// The store validates cursor binding against the current filter hash
/// + consistency mode + deployment + (for indexed consistency) the
/// indexed_head_hash. Mismatches surface as typed errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPageAnchor {
    pub limit: usize,
    pub after: Option<HistoryCursorKey>,
    pub consistency: HistoryConsistency,
    /// Filter binding — the store rejects the cursor if the caller's
    /// filter hash differs from the anchor's `filter_hash`.
    pub filter_hash: String,
    /// Indexed head binding — required when consistency == Indexed.
    /// Store returns `StaleCursor` when the persisted indexed head
    /// no longer matches.
    pub indexed_head_hash: String,
}

impl HistoryPageAnchor {
    pub fn first(
        limit: usize,
        consistency: HistoryConsistency,
        filter_hash: String,
        indexed_head_hash: String,
    ) -> Result<Self, ReadStoreError> {
        if limit > MAX_PAGE_LIMIT {
            return Err(ReadStoreError::LimitExceeded {
                max: MAX_PAGE_LIMIT,
                requested: limit,
            });
        }
        Ok(Self {
            limit: limit.max(1),
            after: None,
            consistency,
            filter_hash,
            indexed_head_hash,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HistoryConsistency {
    Indexed,
    Finalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryCursorKey {
    pub block_number: u64,
    pub tx_index: u32,
    pub log_index: u32,
    pub event_id: String,
}

/// A single page of store results plus the anchor for the next page
/// (None when exhausted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePage<T> {
    pub items: Vec<T>,
    pub next_anchor: Option<PageAnchor>,
}

/// A page of history results.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryPage {
    pub items: Vec<HistoryRecord>,
    pub next_anchor: Option<HistoryPageAnchor>,
}

/// History scope selector. Corresponds to the three history routes
/// (global, per-owner, per-subaccount).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryScope {
    Global,
    Owner { owner: String },
    Subaccount { subkey: String },
}

// -----------------------------------------------------------------
//                       READ-STORE TRAIT
// -----------------------------------------------------------------

/// Async, object-safe read boundary for the mounted Hybrid V2 API.
///
/// Contract:
/// - No write operations.
/// - No chain-provider dependency.
/// - Every canonical query is deployment-scoped.
/// - Bounded limits enforced.
/// - Ordering deterministic.
/// - Errors typed; no raw SQL / credentials in error surface.
#[async_trait::async_trait]
pub trait HybridV2ReadStore: Send + Sync {
    async fn list_deployments(&self) -> Result<Vec<DeploymentListRecord>, ReadStoreError>;

    async fn get_deployment_status(
        &self,
        deployment_id: u64,
    ) -> Result<Option<DeploymentStatusRecord>, ReadStoreError>;

    async fn list_subaccounts_by_owner(
        &self,
        deployment_id: u64,
        owner: &str,
    ) -> Result<Vec<SubaccountRecord>, ReadStoreError>;

    async fn get_subaccount_summary(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<SubaccountSummaryRecord>, ReadStoreError>;

    async fn list_collateral(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<CollateralRecord>, ReadStoreError>;

    async fn list_reservations(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<ReservationRecord>, ReadStoreError>;

    async fn list_positions(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<PositionRecord>, ReadStoreError>;

    async fn list_orders(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<OrderLifecycleRecord>, ReadStoreError>;

    async fn get_order(
        &self,
        deployment_id: u64,
        order_hash: &str,
    ) -> Result<Option<OrderLifecycleRecord>, ReadStoreError>;

    /// When `subkey` is `Some`, filters executions where either buyer
    /// or seller subKey equals the given value. When `None`, returns
    /// deployment-wide completed executions.
    ///
    /// Always excludes rows with `completion_status != 'COMPLETE'`.
    async fn list_completed_executions(
        &self,
        deployment_id: u64,
        subkey: Option<&str>,
        page: &PageAnchor,
    ) -> Result<StorePage<MatchedExecutionRecord>, ReadStoreError>;

    async fn list_fees(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<FeeRebateRecord>, ReadStoreError>;

    async fn get_recovery(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<RecoveryRecord>, ReadStoreError>;

    async fn query_history(
        &self,
        deployment_id: u64,
        scope: &HistoryScope,
        filter: &HistoryFilter,
        page: &HistoryPageAnchor,
    ) -> Result<HistoryPage, ReadStoreError>;
}

// -----------------------------------------------------------------
//                       POSTGRES IMPLEMENTATION
// -----------------------------------------------------------------

/// Production sqlx-backed Hybrid V2 read store.
#[derive(Clone)]
pub struct PostgresHybridV2ReadStore {
    pool: PgPool,
}

impl PostgresHybridV2ReadStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl HybridV2ReadStore for PostgresHybridV2ReadStore {
    async fn list_deployments(&self) -> Result<Vec<DeploymentListRecord>, ReadStoreError> {
        let rows = sqlx::query(
            "SELECT deployment_id, chain_id, manifest_hash, manifest_address,
                    deployment_version, activation_status, max_collateral_tokens,
                    max_active_series
             FROM hybrid_v2_deployments
             ORDER BY deployment_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let deployment_id: i64 = r.try_get("deployment_id").map_err(backend_err)?;
            let chain_id: i64 = r.try_get("chain_id").map_err(backend_err)?;
            let deployment_version: i32 = r.try_get("deployment_version").map_err(backend_err)?;
            let max_collateral_tokens: i32 =
                r.try_get("max_collateral_tokens").map_err(backend_err)?;
            let max_active_series: i32 = r.try_get("max_active_series").map_err(backend_err)?;
            out.push(DeploymentListRecord {
                deployment_id: safe_u64("deployment_id", deployment_id)?,
                chain_id: safe_u64("chain_id", chain_id)?,
                manifest_hash: r.try_get("manifest_hash").map_err(backend_err)?,
                manifest_address: r.try_get("manifest_address").map_err(backend_err)?,
                deployment_version: safe_u32("deployment_version", deployment_version)?,
                activation_status: r.try_get("activation_status").map_err(backend_err)?,
                max_collateral_tokens: safe_u32("max_collateral_tokens", max_collateral_tokens)?,
                max_active_series: safe_u32("max_active_series", max_active_series)?,
            });
        }
        Ok(out)
    }

    async fn get_deployment_status(
        &self,
        deployment_id: u64,
    ) -> Result<Option<DeploymentStatusRecord>, ReadStoreError> {
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let row = sqlx::query(
            "SELECT d.deployment_id, d.chain_id, d.manifest_hash,
                    COALESCE(c.indexed_head_block, 0) AS indexed_head_block,
                    COALESCE(c.indexed_head_hash, '') AS indexed_head_hash,
                    COALESCE(c.observed_head_block, 0) AS observed_head_block,
                    COALESCE(c.finalized_head_block, 0) AS finalized_head_block,
                    COALESCE(c.last_success_block, 0) AS last_success_block,
                    COALESCE(c.reorg_count, 0) AS reorg_count,
                    COALESCE(c.max_reorg_depth_seen, 0) AS max_reorg_depth_seen,
                    COALESCE(c.decode_failures, 0) AS decode_failures,
                    COALESCE(c.projection_failures, 0) AS projection_failures,
                    COALESCE(c.unknown_canonical_events, 0) AS unknown_canonical_events,
                    COALESCE(r.ready, FALSE) AS ready,
                    r.reason AS ready_reason,
                    r.reason_detail AS ready_reason_detail,
                    ps.status AS rebuild_status,
                    rs.classification AS reconciliation_status
             FROM hybrid_v2_deployments d
             LEFT JOIN hybrid_v2_cursors c ON c.deployment_id = d.deployment_id
             LEFT JOIN hybrid_v2_readiness r ON r.deployment_id = d.deployment_id
             LEFT JOIN hybrid_v2_projection_status ps ON ps.deployment_id = d.deployment_id
             LEFT JOIN hybrid_v2_reconciliation_status rs ON rs.deployment_id = d.deployment_id
             WHERE d.deployment_id = $1
             LIMIT 1",
        )
        .bind(did)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let Some(r) = row else { return Ok(None) };
        let indexed_head_block: i64 = r.try_get("indexed_head_block").map_err(backend_err)?;
        let observed_head_block: i64 = r.try_get("observed_head_block").map_err(backend_err)?;
        let finalized_head_block: i64 = r.try_get("finalized_head_block").map_err(backend_err)?;
        let last_success_block: i64 = r.try_get("last_success_block").map_err(backend_err)?;
        let reorg_count: i64 = r.try_get("reorg_count").map_err(backend_err)?;
        let max_reorg_depth_seen: i64 = r.try_get("max_reorg_depth_seen").map_err(backend_err)?;
        let decode_failures: i64 = r.try_get("decode_failures").map_err(backend_err)?;
        let projection_failures: i64 = r.try_get("projection_failures").map_err(backend_err)?;
        let unknown_canonical_events: i64 =
            r.try_get("unknown_canonical_events").map_err(backend_err)?;
        let chain_id: i64 = r.try_get("chain_id").map_err(backend_err)?;
        let did_out: i64 = r.try_get("deployment_id").map_err(backend_err)?;
        Ok(Some(DeploymentStatusRecord {
            deployment_id: safe_u64("deployment_id", did_out)?,
            chain_id: safe_u64("chain_id", chain_id)?,
            manifest_hash: r.try_get("manifest_hash").map_err(backend_err)?,
            indexed_head_block: safe_u64("indexed_head_block", indexed_head_block)?,
            indexed_head_hash: r.try_get("indexed_head_hash").map_err(backend_err)?,
            observed_head_block: safe_u64("observed_head_block", observed_head_block)?,
            finalized_head_block: safe_u64("finalized_head_block", finalized_head_block)?,
            last_success_block: safe_u64("last_success_block", last_success_block)?,
            reorg_count: safe_u64("reorg_count", reorg_count)?,
            max_reorg_depth_seen: safe_u64("max_reorg_depth_seen", max_reorg_depth_seen)?,
            decode_failures: safe_u64("decode_failures", decode_failures)?,
            projection_failures: safe_u64("projection_failures", projection_failures)?,
            unknown_canonical_events: safe_u64(
                "unknown_canonical_events",
                unknown_canonical_events,
            )?,
            ready: r.try_get("ready").map_err(backend_err)?,
            ready_reason: r.try_get("ready_reason").map_err(backend_err)?,
            ready_reason_detail: r.try_get("ready_reason_detail").map_err(backend_err)?,
            rebuild_status: r.try_get("rebuild_status").map_err(backend_err)?,
            reconciliation_status: r.try_get("reconciliation_status").map_err(backend_err)?,
        }))
    }

    async fn list_subaccounts_by_owner(
        &self,
        deployment_id: u64,
        owner: &str,
    ) -> Result<Vec<SubaccountRecord>, ReadStoreError> {
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let rows = sqlx::query(
            "SELECT deployment_id, subkey, owner, subaccount_id, materialised_via,
                    created_block, created_tx_hash
             FROM hybrid_v2_subaccounts
             WHERE deployment_id = $1 AND lower(owner) = lower($2)
                   AND subaccount_id > 0
             ORDER BY subaccount_id ASC",
        )
        .bind(did)
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(row_to_subaccount(&r)?);
        }
        Ok(out)
    }

    async fn get_subaccount_summary(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<SubaccountSummaryRecord>, ReadStoreError> {
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let identity_row = sqlx::query(
            "SELECT deployment_id, subkey, owner, subaccount_id, materialised_via,
                    created_block, created_tx_hash
             FROM hybrid_v2_subaccounts
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)
             LIMIT 1",
        )
        .bind(did)
        .bind(subkey)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let Some(identity_row) = identity_row else {
            return Ok(None);
        };
        let identity = row_to_subaccount(&identity_row)?;
        let materialised_via = identity.materialised_via.as_str();
        let materialised_via_created = matches!(materialised_via, "CREATED" | "BOTH");
        let materialised_via_lazy = matches!(materialised_via, "LAZY" | "BOTH");

        let recovery = self
            .get_recovery(deployment_id, subkey)
            .await?
            .unwrap_or_else(|| RecoveryRecord {
                subkey: subkey.to_string(),
                recovery_state: "NORMAL".to_string(),
                finalized: false,
                owner_recovery_epoch: 0,
                subaccount_recovery_epoch: 0,
                owner_min_valid_nonce: "0".into(),
                subaccount_min_valid_nonce: "0".into(),
                escape: None,
                withdrawal_count: 0,
                finalized_at_ts: None,
            });

        let balance_token_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS c FROM hybrid_v2_vault_balances
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)",
        )
        .bind(did)
        .bind(subkey)
        .fetch_one(&self.pool)
        .await
        .map_err(backend_err)?
        .try_get("c")
        .map_err(backend_err)?;
        let reservation_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS c FROM hybrid_v2_reservations
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)",
        )
        .bind(did)
        .bind(subkey)
        .fetch_one(&self.pool)
        .await
        .map_err(backend_err)?
        .try_get("c")
        .map_err(backend_err)?;
        let active_series_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS c FROM hybrid_v2_active_series
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)",
        )
        .bind(did)
        .bind(subkey)
        .fetch_one(&self.pool)
        .await
        .map_err(backend_err)?
        .try_get("c")
        .map_err(backend_err)?;
        let open_order_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS c FROM hybrid_v2_order_lifecycle
             WHERE deployment_id = $1
               AND lower(subkey) = lower($2)
               AND cancelled = FALSE
               AND terminal = FALSE",
        )
        .bind(did)
        .bind(subkey)
        .fetch_one(&self.pool)
        .await
        .map_err(backend_err)?
        .try_get("c")
        .map_err(backend_err)?;

        Ok(Some(SubaccountSummaryRecord {
            identity: identity.clone(),
            materialised_via_created,
            materialised_via_lazy,
            recovery_state: recovery.recovery_state.clone(),
            finalized: recovery.finalized,
            balance_token_count: safe_u64("balance_token_count", balance_token_count)? as usize,
            reservation_count: safe_u64("reservation_count", reservation_count)? as usize,
            active_series_count: safe_u64("active_series_count", active_series_count)? as usize,
            open_order_count: safe_u64("open_order_count", open_order_count)? as usize,
            owner_recovery_epoch: recovery.owner_recovery_epoch,
            subaccount_recovery_epoch: recovery.subaccount_recovery_epoch,
            escape: recovery.escape.clone(),
        }))
    }

    async fn list_collateral(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<CollateralRecord>, ReadStoreError> {
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let bal_rows = sqlx::query(
            "SELECT b.token, b.balance, b.last_event_block,
                    u.universe_index, COALESCE(u.enabled, TRUE) AS enabled
             FROM hybrid_v2_vault_balances b
             LEFT JOIN hybrid_v2_collateral_universe u
                    ON u.deployment_id = b.deployment_id AND lower(u.token) = lower(b.token)
             WHERE b.deployment_id = $1 AND lower(b.subkey) = lower($2)
             ORDER BY lower(b.token) ASC",
        )
        .bind(did)
        .bind(subkey)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;

        let res_rows = sqlx::query(
            "SELECT token, engine, reserved
             FROM hybrid_v2_reservations
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)",
        )
        .bind(did)
        .bind(subkey)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;

        // Aggregate reservations per token.
        use crate::hybrid_v2::reducer::{u256_add, u256_sub};
        use std::collections::BTreeMap;
        let mut agg: BTreeMap<String, String> = BTreeMap::new();
        for r in &res_rows {
            let token: String = r.try_get("token").map_err(backend_err)?;
            let reserved: String = r.try_get("reserved").map_err(backend_err)?;
            let key = token.to_lowercase();
            let cur = agg.get(&key).cloned().unwrap_or_else(|| "0".into());
            let next = u256_add(&cur, &reserved).ok_or_else(|| ReadStoreError::MalformedRow {
                detail: format!("aggregate reservation overflow on token {token}"),
            })?;
            agg.insert(key, next);
        }

        let mut out = Vec::with_capacity(bal_rows.len());
        for b in bal_rows {
            let token: String = b.try_get("token").map_err(backend_err)?;
            let balance: String = b.try_get("balance").map_err(backend_err)?;
            let last_event_block: i64 = b.try_get("last_event_block").map_err(backend_err)?;
            let universe_index: Option<i32> = b.try_get("universe_index").map_err(backend_err)?;
            let enabled: bool = b.try_get("enabled").map_err(backend_err)?;
            let aggregate = agg
                .get(&token.to_lowercase())
                .cloned()
                .unwrap_or_else(|| "0".into());
            let available = u256_sub(&balance, &aggregate).unwrap_or_else(|| "0".into());
            out.push(CollateralRecord {
                token,
                universe_index: universe_index
                    .map(|v| safe_u32("universe_index", v))
                    .transpose()?,
                enabled,
                balance,
                aggregate_reserved: aggregate,
                available,
                last_event_block: safe_u64("last_event_block", last_event_block)?,
            });
        }
        Ok(out)
    }

    async fn list_reservations(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<ReservationRecord>, ReadStoreError> {
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let rows = sqlx::query(
            "SELECT subkey, token, engine, reserved, last_event_block
             FROM hybrid_v2_reservations
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)
             ORDER BY lower(token) ASC, lower(engine) ASC",
        )
        .bind(did)
        .bind(subkey)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let last_event_block: i64 = r.try_get("last_event_block").map_err(backend_err)?;
            out.push(ReservationRecord {
                subkey: r.try_get("subkey").map_err(backend_err)?,
                token: r.try_get("token").map_err(backend_err)?,
                engine: r.try_get("engine").map_err(backend_err)?,
                reserved: r.try_get("reserved").map_err(backend_err)?,
                last_event_block: safe_u64("last_event_block", last_event_block)?,
            });
        }
        Ok(out)
    }

    async fn list_positions(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<PositionRecord>, ReadStoreError> {
        enforce_page_limit(page.limit)?;
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let limit = (page.limit + 1) as i64;
        let last_key = page.last_key.clone().unwrap_or_default();
        let rows = sqlx::query(
            "SELECT p.subkey, p.series_id, p.long_qty_1e8, p.short_qty_1e8, p.last_event_block,
                    (a.series_id IS NOT NULL) AS active
             FROM hybrid_v2_positions p
             LEFT JOIN hybrid_v2_active_series a
                    ON a.deployment_id = p.deployment_id
                   AND lower(a.subkey) = lower(p.subkey)
                   AND lower(a.series_id) = lower(p.series_id)
             WHERE p.deployment_id = $1
               AND lower(p.subkey) = lower($2)
               AND ($3 = '' OR lower(p.series_id) > lower($3))
             ORDER BY lower(p.series_id) ASC
             LIMIT $4",
        )
        .bind(did)
        .bind(subkey)
        .bind(&last_key)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;
        let mut items = Vec::with_capacity(rows.len());
        for r in &rows {
            let last_event_block: i64 = r.try_get("last_event_block").map_err(backend_err)?;
            items.push(PositionRecord {
                subkey: r.try_get("subkey").map_err(backend_err)?,
                series_id: r.try_get("series_id").map_err(backend_err)?,
                long_qty_1e8: r.try_get("long_qty_1e8").map_err(backend_err)?,
                short_qty_1e8: r.try_get("short_qty_1e8").map_err(backend_err)?,
                active: r.try_get("active").map_err(backend_err)?,
                last_event_block: safe_u64("last_event_block", last_event_block)?,
            });
        }
        Ok(finalize_page(items, page.limit))
    }

    async fn list_orders(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<OrderLifecycleRecord>, ReadStoreError> {
        enforce_page_limit(page.limit)?;
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let limit = (page.limit + 1) as i64;
        let last_key = page.last_key.clone().unwrap_or_default();
        let rows = sqlx::query(
            "SELECT order_hash, deployment_id, subkey, owner, series_id, side, time_in_force,
                    total_qty_1e8, filled_qty_1e8, cancelled, terminal,
                    first_seen_block, last_event_block
             FROM hybrid_v2_order_lifecycle
             WHERE deployment_id = $1
               AND lower(subkey) = lower($2)
               AND ($3 = '' OR lower(order_hash) > lower($3))
             ORDER BY lower(order_hash) ASC
             LIMIT $4",
        )
        .bind(did)
        .bind(subkey)
        .bind(&last_key)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;
        let mut items = Vec::with_capacity(rows.len());
        for r in &rows {
            items.push(row_to_order_lifecycle(r)?);
        }
        Ok(finalize_page(items, page.limit))
    }

    async fn get_order(
        &self,
        deployment_id: u64,
        order_hash: &str,
    ) -> Result<Option<OrderLifecycleRecord>, ReadStoreError> {
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let row = sqlx::query(
            "SELECT order_hash, deployment_id, subkey, owner, series_id, side, time_in_force,
                    total_qty_1e8, filled_qty_1e8, cancelled, terminal,
                    first_seen_block, last_event_block
             FROM hybrid_v2_order_lifecycle
             WHERE deployment_id = $1 AND lower(order_hash) = lower($2)
             LIMIT 1",
        )
        .bind(did)
        .bind(order_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        row.map(|r| row_to_order_lifecycle(&r)).transpose()
    }

    async fn list_completed_executions(
        &self,
        deployment_id: u64,
        subkey: Option<&str>,
        page: &PageAnchor,
    ) -> Result<StorePage<MatchedExecutionRecord>, ReadStoreError> {
        enforce_page_limit(page.limit)?;
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let limit = (page.limit + 1) as i64;
        let last_key = page.last_key.clone().unwrap_or_default();
        let (last_block, last_exec_id) = parse_execution_last_key(&last_key)?;
        let rows = match subkey {
            Some(sk) => sqlx::query(
                "SELECT deployment_id, execution_id, buyer_order_hash, seller_order_hash,
                        buyer_subkey, seller_subkey, series_id, matched_qty_1e8,
                        premium_amount, fee_amount, rebate_amount,
                        block_number, tx_hash, completion_status
                 FROM hybrid_v2_matched_executions
                 WHERE deployment_id = $1
                   AND completion_status = 'COMPLETE'
                   AND (lower(buyer_subkey) = lower($2) OR lower(seller_subkey) = lower($2))
                   AND ($3 = 0
                        OR block_number < $3
                        OR (block_number = $3 AND lower(execution_id) < lower($4)))
                 ORDER BY block_number DESC, lower(execution_id) DESC
                 LIMIT $5",
            )
            .bind(did)
            .bind(sk)
            .bind(last_block)
            .bind(&last_exec_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?,
            None => sqlx::query(
                "SELECT deployment_id, execution_id, buyer_order_hash, seller_order_hash,
                        buyer_subkey, seller_subkey, series_id, matched_qty_1e8,
                        premium_amount, fee_amount, rebate_amount,
                        block_number, tx_hash, completion_status
                 FROM hybrid_v2_matched_executions
                 WHERE deployment_id = $1
                   AND completion_status = 'COMPLETE'
                   AND ($2 = 0
                        OR block_number < $2
                        OR (block_number = $2 AND lower(execution_id) < lower($3)))
                 ORDER BY block_number DESC, lower(execution_id) DESC
                 LIMIT $4",
            )
            .bind(did)
            .bind(last_block)
            .bind(&last_exec_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?,
        };
        let mut items = Vec::with_capacity(rows.len());
        for r in &rows {
            items.push(row_to_execution(r)?);
        }
        Ok(finalize_execution_page(items, page.limit))
    }

    async fn list_fees(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<FeeRebateRecord>, ReadStoreError> {
        enforce_page_limit(page.limit)?;
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let limit = (page.limit + 1) as i64;
        let last_key = page.last_key.clone().unwrap_or_default();
        let (last_block, last_log_index) = parse_fee_last_key(&last_key)?;
        let rows = sqlx::query(
            "SELECT deployment_id, event_id, kind, payer_subkey, receiver_subkey,
                    token, amount, block_number, tx_hash, log_index, execution_id
             FROM hybrid_v2_fee_events
             WHERE deployment_id = $1
               AND (lower(payer_subkey) = lower($2) OR lower(receiver_subkey) = lower($2))
               AND ($3 = 0
                    OR block_number < $3
                    OR (block_number = $3 AND log_index < $4))
             ORDER BY block_number DESC, log_index DESC
             LIMIT $5",
        )
        .bind(did)
        .bind(subkey)
        .bind(last_block)
        .bind(last_log_index)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;
        let mut items = Vec::with_capacity(rows.len());
        for r in &rows {
            items.push(row_to_fee(r)?);
        }
        Ok(finalize_fee_page(items, page.limit))
    }

    async fn get_recovery(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<RecoveryRecord>, ReadStoreError> {
        let did = u64_to_i64("deployment_id", deployment_id)?;
        // Owner reverse-lookup: we need the subaccount's owner to fetch
        // owner-scoped epoch/min-nonce data.
        let owner_row = sqlx::query(
            "SELECT owner FROM hybrid_v2_subaccounts
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)
             LIMIT 1",
        )
        .bind(did)
        .bind(subkey)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let owner: Option<String> = owner_row
            .as_ref()
            .map(|r| r.try_get("owner").map_err(backend_err))
            .transpose()?;
        let state_row = sqlx::query(
            "SELECT state, finalized_at_ts FROM hybrid_v2_recovery_state
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)
             LIMIT 1",
        )
        .bind(did)
        .bind(subkey)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let state_text: String = state_row
            .as_ref()
            .map(|r| r.try_get::<String, _>("state").map_err(backend_err))
            .transpose()?
            .unwrap_or_else(|| "NORMAL".into());
        let finalized_at_ts: Option<i64> = state_row
            .as_ref()
            .and_then(|r| r.try_get::<Option<i64>, _>("finalized_at_ts").ok())
            .flatten();
        let escape_row = sqlx::query(
            "SELECT state, requested_ts, activation_eligible_at, activated_ts,
                    cancelled_ts, finalized_ts, last_event_block
             FROM hybrid_v2_escape_state
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)
             LIMIT 1",
        )
        .bind(did)
        .bind(subkey)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let escape = if let Some(r) = escape_row {
            Some(row_to_escape_state(&r)?)
        } else {
            None
        };
        let sub_epoch = sqlx::query(
            "SELECT epoch_count, min_valid_nonce FROM hybrid_v2_recovery_epochs
             WHERE deployment_id = $1 AND scope = 'SUBACCOUNT' AND lower(scope_key) = lower($2)
             LIMIT 1",
        )
        .bind(did)
        .bind(subkey)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let (subaccount_epoch, subaccount_min_nonce) = match sub_epoch {
            Some(r) => {
                let ec: i64 = r.try_get("epoch_count").map_err(backend_err)?;
                let mn: String = r.try_get("min_valid_nonce").map_err(backend_err)?;
                (safe_u64("epoch_count", ec)?, mn)
            }
            None => (0u64, "0".to_string()),
        };
        let owner_epoch = match owner.as_deref() {
            Some(o) => sqlx::query(
                "SELECT epoch_count, min_valid_nonce FROM hybrid_v2_recovery_epochs
                 WHERE deployment_id = $1 AND scope = 'OWNER' AND lower(scope_key) = lower($2)
                 LIMIT 1",
            )
            .bind(did)
            .bind(o)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?,
            None => None,
        };
        let (owner_epoch_ct, owner_min_nonce) = match owner_epoch {
            Some(r) => {
                let ec: i64 = r.try_get("epoch_count").map_err(backend_err)?;
                let mn: String = r.try_get("min_valid_nonce").map_err(backend_err)?;
                (safe_u64("epoch_count", ec)?, mn)
            }
            None => (0u64, "0".to_string()),
        };
        let withdraw = sqlx::query(
            "SELECT withdrawal_count FROM hybrid_v2_recovery_withdrawal_counts
             WHERE deployment_id = $1 AND lower(subkey) = lower($2)
             LIMIT 1",
        )
        .bind(did)
        .bind(subkey)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let withdrawal_count = withdraw
            .map(|r| r.try_get::<i32, _>("withdrawal_count").map_err(backend_err))
            .transpose()?
            .map(|v| safe_u32("withdrawal_count", v))
            .transpose()?
            .unwrap_or(0);
        let finalized = state_text == "RECOVERED";
        Ok(Some(RecoveryRecord {
            subkey: subkey.to_string(),
            recovery_state: state_text,
            finalized,
            owner_recovery_epoch: owner_epoch_ct,
            subaccount_recovery_epoch: subaccount_epoch,
            owner_min_valid_nonce: owner_min_nonce,
            subaccount_min_valid_nonce: subaccount_min_nonce,
            escape,
            withdrawal_count,
            finalized_at_ts: finalized_at_ts
                .map(|v| safe_u64("finalized_at_ts", v))
                .transpose()?,
        }))
    }

    async fn query_history(
        &self,
        deployment_id: u64,
        scope: &HistoryScope,
        filter: &HistoryFilter,
        page: &HistoryPageAnchor,
    ) -> Result<HistoryPage, ReadStoreError> {
        enforce_page_limit(page.limit)?;
        // Validate cursor binding.
        let expected_filter_hash = filter_stable_hash(filter);
        if page.filter_hash != expected_filter_hash {
            return Err(ReadStoreError::InvalidCursor {
                detail: "cursor filter hash does not match current filter".into(),
            });
        }
        // Indexed-head binding check: staleness is signalled via
        // StaleCursor when the caller's head hash disagrees with the
        // current cursor row.
        if matches!(page.consistency, HistoryConsistency::Indexed)
            && !page.indexed_head_hash.is_empty()
        {
            let did = u64_to_i64("deployment_id", deployment_id)?;
            let row = sqlx::query(
                "SELECT indexed_head_hash FROM hybrid_v2_cursors
                 WHERE deployment_id = $1
                 LIMIT 1",
            )
            .bind(did)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?;
            if let Some(r) = row {
                let cur_head: String = r.try_get("indexed_head_hash").map_err(backend_err)?;
                if !cur_head.is_empty() && cur_head != page.indexed_head_hash {
                    return Err(ReadStoreError::StaleCursor {
                        expected_hash: page.indexed_head_hash.clone(),
                        actual_hash: cur_head,
                    });
                }
            }
        }
        // Deployment identity for chain_id enrichment.
        let did = u64_to_i64("deployment_id", deployment_id)?;
        let dep_row = sqlx::query(
            "SELECT chain_id FROM hybrid_v2_deployments WHERE deployment_id = $1 LIMIT 1",
        )
        .bind(did)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?;
        let chain_id = match dep_row {
            Some(r) => {
                let cid: i64 = r.try_get("chain_id").map_err(backend_err)?;
                safe_u64("chain_id", cid)?
            }
            None => 0,
        };
        let finalized_block = match self.get_deployment_status(deployment_id).await? {
            Some(s) => s.finalized_head_block,
            None => 0,
        };

        // Build the query. Scope + filter are applied as SQL WHERE
        // clauses where representable. Direction/family filters are
        // applied server-side in Rust after decoding the payload.
        let limit = (page.limit + 1) as i64;
        let (anchor_block, anchor_tx_index, anchor_log_index, anchor_event_id) = page
            .after
            .as_ref()
            .map(|a| {
                (
                    u64_to_i64("cursor.block_number", a.block_number).unwrap_or(0),
                    a.tx_index as i32,
                    a.log_index as i32,
                    a.event_id.clone(),
                )
            })
            .unwrap_or((0, 0, 0, String::new()));
        // Owner and subkey filters at SQL level.
        let owner_filter: Option<String> = match scope {
            HistoryScope::Owner { owner } => Some(owner.clone()),
            _ => filter.owner.clone(),
        };
        let subkey_filter: Option<String> = match scope {
            HistoryScope::Subaccount { subkey } => Some(subkey.clone()),
            _ => filter.subkey.clone(),
        };

        let rows = sqlx::query(
            "SELECT r.block_number, r.block_hash, r.tx_hash, r.tx_index, r.log_index,
                    r.block_timestamp, r.is_canonical,
                    d.event_kind, d.event_version, d.subkey, d.owner, d.subaccount_id,
                    d.token, d.engine, d.execution_id, d.order_hash, d.series_id,
                    d.payload_json
             FROM hybrid_v2_decoded_events d
             JOIN hybrid_v2_raw_logs r ON r.raw_log_id = d.raw_log_id
             WHERE d.deployment_id = $1
               AND r.is_canonical = TRUE
               AND ($2::text IS NULL OR lower(d.owner) = lower($2))
               AND ($3::text IS NULL OR lower(d.subkey) = lower($3))
               AND ($4::text IS NULL OR lower(d.token) = lower($4))
               AND ($5::text IS NULL OR lower(d.series_id) = lower($5))
               AND ($6::text IS NULL OR lower(d.order_hash) = lower($6))
               AND ($7::text IS NULL OR lower(d.execution_id) = lower($7))
               AND ($8::bigint = 0 OR r.block_number >= $8)
               AND ($9::bigint = 0 OR r.block_number <= $9)
               AND ($10::bigint = 0
                    OR r.block_number < $10
                    OR (r.block_number = $10 AND r.tx_index < $11)
                    OR (r.block_number = $10 AND r.tx_index = $11 AND r.log_index < $12)
                    OR (r.block_number = $10 AND r.tx_index = $11 AND r.log_index = $12
                        AND (r.block_hash || ':' || r.tx_hash || ':' || r.log_index::text) < $13))
             ORDER BY r.block_number DESC, r.tx_index DESC, r.log_index DESC,
                      (r.block_hash || ':' || r.tx_hash || ':' || r.log_index::text) DESC
             LIMIT $14",
        )
        .bind(did)
        .bind(owner_filter.as_deref())
        .bind(subkey_filter.as_deref())
        .bind(filter.token.as_deref())
        .bind(filter.series_id.as_deref())
        .bind(filter.order_hash.as_deref())
        .bind(filter.execution_id.as_deref())
        .bind(filter.from_block.map(|v| v as i64).unwrap_or(0))
        .bind(filter.to_block.map(|v| v as i64).unwrap_or(0))
        .bind(anchor_block)
        .bind(anchor_tx_index)
        .bind(anchor_log_index)
        .bind(&anchor_event_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;

        let mut items: Vec<HistoryRecord> = Vec::with_capacity(rows.len());
        for r in &rows {
            let block_number: i64 = r.try_get("block_number").map_err(backend_err)?;
            let block_hash: String = r.try_get("block_hash").map_err(backend_err)?;
            let tx_hash: String = r.try_get("tx_hash").map_err(backend_err)?;
            let tx_index: i32 = r.try_get("tx_index").map_err(backend_err)?;
            let log_index: i32 = r.try_get("log_index").map_err(backend_err)?;
            let block_timestamp: i64 = r.try_get("block_timestamp").map_err(backend_err)?;
            let event_kind_s: String = r.try_get("event_kind").map_err(backend_err)?;
            let subkey_col: Option<String> = r.try_get("subkey").map_err(backend_err)?;
            let owner_col: Option<String> = r.try_get("owner").map_err(backend_err)?;
            let sid_col: Option<i64> = r.try_get("subaccount_id").map_err(backend_err)?;
            let token_col: Option<String> = r.try_get("token").map_err(backend_err)?;
            let engine_col: Option<String> = r.try_get("engine").map_err(backend_err)?;
            let exec_id: Option<String> = r.try_get("execution_id").map_err(backend_err)?;
            let order_hash: Option<String> = r.try_get("order_hash").map_err(backend_err)?;
            let series_id: Option<String> = r.try_get("series_id").map_err(backend_err)?;
            let payload_json: serde_json::Value = r.try_get("payload_json").map_err(backend_err)?;
            let is_canonical: bool = r.try_get("is_canonical").map_err(backend_err)?;
            if !is_canonical {
                continue;
            }
            let kind = match EventKind::parse(&event_kind_s) {
                Some(k) => k,
                None => continue,
            };
            let (payload, related_order, related_exec) = event_to_history_payload(
                kind,
                &payload_json,
                subkey_col.as_deref(),
                owner_col.as_deref(),
                sid_col,
                token_col.as_deref(),
                engine_col.as_deref(),
                exec_id.as_deref(),
                order_hash.as_deref(),
                series_id.as_deref(),
            );
            let event_id = format!("{}:{}:{}", block_hash, tx_hash, log_index);
            let block_number_u = safe_u64("block_number", block_number)?;
            let finalized = block_number_u <= finalized_block;
            // Family filter (applied post-decode)
            if !filter.families.is_empty()
                && !filter
                    .families
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(payload.family_label()))
            {
                continue;
            }
            let direction = classify_direction(kind);
            if let Some(dir_want) = &filter.direction {
                if direction != *dir_want {
                    continue;
                }
            }
            let rec = HistoryRecord {
                event_id: event_id.clone(),
                deployment_id,
                chain_id,
                block_number: block_number_u,
                block_hash,
                tx_hash,
                tx_index: safe_u32("tx_index", tx_index)?,
                log_index: safe_u32("log_index", log_index)?,
                timestamp_ms: safe_u64("block_timestamp", block_timestamp)?.saturating_mul(1000),
                finalized,
                direction,
                owner: owner_col,
                subaccount_id: sid_col.map(|v| v as u32),
                subkey: subkey_col,
                related_order_hash: related_order,
                related_execution_id: related_exec,
                payload,
            };
            items.push(rec);
        }
        Ok(finalize_history_page(items, page))
    }
}

// -----------------------------------------------------------------
//                       IN-MEMORY ADAPTER (test only)
// -----------------------------------------------------------------

/// Test-only in-memory read store. Wraps a `ProjectionState` snapshot
/// + cursor + readiness and produces owned records identical in shape
/// to the Postgres store. Used by parity tests to prove memory / PG
/// convergence without loading a database.
///
/// Not intended for production use. The runtime-backed adapter for the
/// stage-2B handler swap will be added in that stage.
#[derive(Clone)]
pub struct InMemoryHybridV2ReadStore {
    inner: std::sync::Arc<InMemoryInner>,
}

struct InMemoryInner {
    deployment_id: u64,
    chain_id: u64,
    manifest_hash: String,
    manifest_address: String,
    deployment_version: u32,
    activation_status: String,
    max_collateral_tokens: u32,
    max_active_series: u32,
    state: ProjectionState,
    cursor: RuntimeCursor,
    ready: bool,
    ready_reason: Option<String>,
    ready_reason_detail: Option<String>,
    finalized_head_block: u64,
    /// Journal of `(block_number, block_hash, tx_hash, tx_index, log_index,
    /// block_timestamp, EventKind, subkey, owner, subaccount_id, token,
    /// engine, execution_id, order_hash, series_id, payload_json)`. Used by
    /// the in-memory `query_history` to reproduce the SQL output from an
    /// otherwise deterministic canonical event set.
    journal: Vec<InMemoryJournalEntry>,
}

pub struct InMemoryJournalEntry {
    pub block_number: u64,
    pub block_hash: String,
    pub tx_hash: String,
    pub tx_index: u32,
    pub log_index: u32,
    pub block_timestamp: u64,
    pub kind: EventKind,
    pub subkey: Option<String>,
    pub owner: Option<String>,
    pub subaccount_id: Option<u32>,
    pub token: Option<String>,
    pub engine: Option<String>,
    pub execution_id: Option<String>,
    pub order_hash: Option<String>,
    pub series_id: Option<String>,
    pub payload: serde_json::Value,
    pub is_canonical: bool,
}

pub struct InMemoryStoreBuilder {
    inner: InMemoryInner,
}

impl InMemoryStoreBuilder {
    pub fn new(deployment_id: u64) -> Self {
        Self {
            inner: InMemoryInner {
                deployment_id,
                chain_id: 84532,
                manifest_hash: format!("0xmemhash{deployment_id}"),
                manifest_address: format!("0xmemmani{deployment_id}"),
                deployment_version: 1,
                activation_status: "PENDING".into(),
                max_collateral_tokens: 8,
                max_active_series: 32,
                state: ProjectionState::default(),
                cursor: RuntimeCursor::default(),
                ready: true,
                ready_reason: None,
                ready_reason_detail: None,
                finalized_head_block: 0,
                journal: Vec::new(),
            },
        }
    }

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.inner.chain_id = chain_id;
        self
    }
    pub fn with_manifest(mut self, hash: &str, address: &str) -> Self {
        self.inner.manifest_hash = hash.to_string();
        self.inner.manifest_address = address.to_string();
        self
    }
    pub fn with_state(mut self, state: ProjectionState) -> Self {
        self.inner.state = state;
        self
    }
    pub fn with_cursor(mut self, cursor: RuntimeCursor) -> Self {
        self.inner.cursor = cursor;
        self
    }
    pub fn with_readiness(
        mut self,
        ready: bool,
        reason: Option<String>,
        detail: Option<String>,
    ) -> Self {
        self.inner.ready = ready;
        self.inner.ready_reason = reason;
        self.inner.ready_reason_detail = detail;
        self
    }
    pub fn with_finalized_head(mut self, block: u64) -> Self {
        self.inner.finalized_head_block = block;
        self
    }
    pub fn with_journal(mut self, journal: Vec<InMemoryJournalEntry>) -> Self {
        self.inner.journal = journal;
        self
    }
    pub fn build(self) -> InMemoryHybridV2ReadStore {
        InMemoryHybridV2ReadStore {
            inner: std::sync::Arc::new(self.inner),
        }
    }
}

#[async_trait::async_trait]
impl HybridV2ReadStore for InMemoryHybridV2ReadStore {
    async fn list_deployments(&self) -> Result<Vec<DeploymentListRecord>, ReadStoreError> {
        Ok(vec![DeploymentListRecord {
            deployment_id: self.inner.deployment_id,
            chain_id: self.inner.chain_id,
            manifest_hash: self.inner.manifest_hash.clone(),
            manifest_address: self.inner.manifest_address.clone(),
            deployment_version: self.inner.deployment_version,
            activation_status: self.inner.activation_status.clone(),
            max_collateral_tokens: self.inner.max_collateral_tokens,
            max_active_series: self.inner.max_active_series,
        }])
    }

    async fn get_deployment_status(
        &self,
        deployment_id: u64,
    ) -> Result<Option<DeploymentStatusRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(None);
        }
        Ok(Some(DeploymentStatusRecord {
            deployment_id: self.inner.deployment_id,
            chain_id: self.inner.chain_id,
            manifest_hash: self.inner.manifest_hash.clone(),
            indexed_head_block: self.inner.cursor.indexed_head_block,
            indexed_head_hash: self.inner.cursor.indexed_head_hash.clone(),
            observed_head_block: self.inner.cursor.observed_head_block,
            finalized_head_block: self.inner.finalized_head_block,
            last_success_block: self.inner.cursor.indexed_head_block,
            reorg_count: 0,
            max_reorg_depth_seen: 0,
            decode_failures: 0,
            projection_failures: 0,
            unknown_canonical_events: 0,
            ready: self.inner.ready,
            ready_reason: self.inner.ready_reason.clone(),
            ready_reason_detail: self.inner.ready_reason_detail.clone(),
            rebuild_status: None,
            reconciliation_status: None,
        }))
    }

    async fn list_subaccounts_by_owner(
        &self,
        deployment_id: u64,
        owner: &str,
    ) -> Result<Vec<SubaccountRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(vec![]);
        }
        let mut out: Vec<SubaccountRecord> = self
            .inner
            .state
            .subaccounts
            .iter()
            .filter(|((o, sid), _)| o.eq_ignore_ascii_case(owner) && *sid > 0)
            .map(|((o, sid), sk)| SubaccountRecord {
                deployment_id,
                subkey: sk.clone(),
                owner: o.clone(),
                subaccount_id: *sid,
                materialised_via: subaccount_meta_kind(&self.inner.state, sk),
                created_block: 0,
                created_tx_hash: String::new(),
            })
            .collect();
        out.sort_by_key(|r| r.subaccount_id);
        Ok(out)
    }

    async fn get_subaccount_summary(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<SubaccountSummaryRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(None);
        }
        let identity: Option<SubaccountRecord> = self
            .inner
            .state
            .subaccounts
            .iter()
            .find(|(_, sk)| sk.eq_ignore_ascii_case(subkey))
            .map(|((o, sid), sk)| SubaccountRecord {
                deployment_id,
                subkey: sk.clone(),
                owner: o.clone(),
                subaccount_id: *sid,
                materialised_via: subaccount_meta_kind(&self.inner.state, sk),
                created_block: 0,
                created_tx_hash: String::new(),
            });
        let Some(identity) = identity else {
            return Ok(None);
        };
        let meta = self
            .inner
            .state
            .subaccount_meta
            .get(&identity.subkey)
            .copied()
            .unwrap_or_default();
        let recovery = self
            .get_recovery(deployment_id, subkey)
            .await?
            .unwrap_or_else(|| RecoveryRecord {
                subkey: subkey.to_string(),
                recovery_state: "NORMAL".to_string(),
                finalized: false,
                owner_recovery_epoch: 0,
                subaccount_recovery_epoch: 0,
                owner_min_valid_nonce: "0".into(),
                subaccount_min_valid_nonce: "0".into(),
                escape: None,
                withdrawal_count: 0,
                finalized_at_ts: None,
            });
        let balance_token_count = self
            .inner
            .state
            .balances
            .iter()
            .filter(|((sk, _), _)| sk.eq_ignore_ascii_case(subkey))
            .count();
        let reservation_count = self
            .inner
            .state
            .reservations
            .iter()
            .filter(|((sk, _, _), _)| sk.eq_ignore_ascii_case(subkey))
            .count();
        let active_series_count = self
            .inner
            .state
            .active_series
            .get(&identity.subkey)
            .map(|s| s.len())
            .unwrap_or(0);
        let open_order_count = self
            .inner
            .state
            .order_lifecycle
            .values()
            .filter(|row| {
                row.subkey.eq_ignore_ascii_case(subkey) && !row.terminal && !row.cancelled
            })
            .count();
        Ok(Some(SubaccountSummaryRecord {
            identity,
            materialised_via_created: meta.materialised_via_created,
            materialised_via_lazy: meta.materialised_via_lazy,
            recovery_state: recovery.recovery_state.clone(),
            finalized: recovery.finalized,
            balance_token_count,
            reservation_count,
            active_series_count,
            open_order_count,
            owner_recovery_epoch: recovery.owner_recovery_epoch,
            subaccount_recovery_epoch: recovery.subaccount_recovery_epoch,
            escape: recovery.escape.clone(),
        }))
    }

    async fn list_collateral(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<CollateralRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(vec![]);
        }
        use crate::hybrid_v2::reducer::{u256_add, u256_sub};
        use std::collections::BTreeMap;
        let mut agg: BTreeMap<String, String> = BTreeMap::new();
        for ((sk, tok, _), reserved) in self.inner.state.reservations.iter() {
            if !sk.eq_ignore_ascii_case(subkey) {
                continue;
            }
            let key = tok.to_lowercase();
            let cur = agg.get(&key).cloned().unwrap_or_else(|| "0".into());
            let next = u256_add(&cur, reserved).unwrap_or(cur);
            agg.insert(key, next);
        }
        let mut out: Vec<CollateralRecord> = self
            .inner
            .state
            .balances
            .iter()
            .filter(|((sk, _), _)| sk.eq_ignore_ascii_case(subkey))
            .map(|((_, token), balance)| {
                let aggregate = agg
                    .get(&token.to_lowercase())
                    .cloned()
                    .unwrap_or_else(|| "0".into());
                let available = u256_sub(balance, &aggregate).unwrap_or_else(|| "0".into());
                let universe_index = self.inner.state.collateral_universe.get(token).copied();
                let enabled = !self.inner.state.disabled_tokens.contains(token);
                CollateralRecord {
                    token: token.clone(),
                    universe_index,
                    enabled,
                    balance: balance.clone(),
                    aggregate_reserved: aggregate,
                    available,
                    last_event_block: 0,
                }
            })
            .collect();
        out.sort_by(|a, b| a.token.to_lowercase().cmp(&b.token.to_lowercase()));
        Ok(out)
    }

    async fn list_reservations(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Vec<ReservationRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(vec![]);
        }
        let mut out: Vec<ReservationRecord> = self
            .inner
            .state
            .reservations
            .iter()
            .filter(|((sk, _, _), _)| sk.eq_ignore_ascii_case(subkey))
            .map(|((sk, tok, eng), r)| ReservationRecord {
                subkey: sk.clone(),
                token: tok.clone(),
                engine: eng.clone(),
                reserved: r.clone(),
                last_event_block: 0,
            })
            .collect();
        out.sort_by(|a, b| {
            a.token
                .to_lowercase()
                .cmp(&b.token.to_lowercase())
                .then_with(|| a.engine.to_lowercase().cmp(&b.engine.to_lowercase()))
        });
        Ok(out)
    }

    async fn list_positions(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<PositionRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(StorePage {
                items: vec![],
                next_anchor: None,
            });
        }
        enforce_page_limit(page.limit)?;
        let last_key_lc = page.last_key.as_deref().unwrap_or("").to_lowercase();
        let active_set = self
            .inner
            .state
            .active_series
            .get(subkey)
            .cloned()
            .unwrap_or_default();
        let mut items: Vec<PositionRecord> = self
            .inner
            .state
            .positions
            .iter()
            .filter(|((sk, _), _)| sk.eq_ignore_ascii_case(subkey))
            .filter(|((_, series), _)| {
                last_key_lc.is_empty() || series.to_lowercase() > last_key_lc
            })
            .map(|((_, series), row)| PositionRecord {
                subkey: subkey.to_string(),
                series_id: series.clone(),
                long_qty_1e8: row.long_qty_1e8.clone(),
                short_qty_1e8: row.short_qty_1e8.clone(),
                active: active_set.contains(series),
                last_event_block: row.last_event_block,
            })
            .collect();
        items.sort_by(|a, b| a.series_id.to_lowercase().cmp(&b.series_id.to_lowercase()));
        items.truncate(page.limit + 1);
        Ok(finalize_page(items, page.limit))
    }

    async fn list_orders(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<OrderLifecycleRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(StorePage {
                items: vec![],
                next_anchor: None,
            });
        }
        enforce_page_limit(page.limit)?;
        let last_key_lc = page.last_key.as_deref().unwrap_or("").to_lowercase();
        let mut items: Vec<OrderLifecycleRecord> = self
            .inner
            .state
            .order_lifecycle
            .iter()
            .filter(|(_, row)| row.subkey.eq_ignore_ascii_case(subkey))
            .filter(|(hash, _)| last_key_lc.is_empty() || hash.to_lowercase() > last_key_lc)
            .map(|(hash, row)| {
                let remaining =
                    crate::hybrid_v2::reducer::u256_sub(&row.total_qty_1e8, &row.filled_qty_1e8)
                        .unwrap_or_else(|| "0".into());
                OrderLifecycleRecord {
                    order_hash: hash.clone(),
                    deployment_id,
                    subkey: row.subkey.clone(),
                    owner: row.owner.clone(),
                    series_id: row.series_id.clone(),
                    side: row.side,
                    time_in_force: row.time_in_force,
                    total_qty_1e8: row.total_qty_1e8.clone(),
                    filled_qty_1e8: row.filled_qty_1e8.clone(),
                    remaining_qty_1e8: remaining,
                    cancelled: row.cancelled,
                    terminal: row.terminal,
                    first_seen_block: row.first_seen_block,
                    last_event_block: row.last_event_block,
                }
            })
            .collect();
        items.sort_by(|a, b| {
            a.order_hash
                .to_lowercase()
                .cmp(&b.order_hash.to_lowercase())
        });
        items.truncate(page.limit + 1);
        Ok(finalize_page(items, page.limit))
    }

    async fn get_order(
        &self,
        deployment_id: u64,
        order_hash: &str,
    ) -> Result<Option<OrderLifecycleRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(None);
        }
        let target = order_hash.to_lowercase();
        let entry = self
            .inner
            .state
            .order_lifecycle
            .iter()
            .find(|(k, _)| k.to_lowercase() == target);
        Ok(entry.map(|(k, row)| {
            let remaining =
                crate::hybrid_v2::reducer::u256_sub(&row.total_qty_1e8, &row.filled_qty_1e8)
                    .unwrap_or_else(|| "0".into());
            OrderLifecycleRecord {
                order_hash: k.clone(),
                deployment_id,
                subkey: row.subkey.clone(),
                owner: row.owner.clone(),
                series_id: row.series_id.clone(),
                side: row.side,
                time_in_force: row.time_in_force,
                total_qty_1e8: row.total_qty_1e8.clone(),
                filled_qty_1e8: row.filled_qty_1e8.clone(),
                remaining_qty_1e8: remaining,
                cancelled: row.cancelled,
                terminal: row.terminal,
                first_seen_block: row.first_seen_block,
                last_event_block: row.last_event_block,
            }
        }))
    }

    async fn list_completed_executions(
        &self,
        deployment_id: u64,
        subkey: Option<&str>,
        page: &PageAnchor,
    ) -> Result<StorePage<MatchedExecutionRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(StorePage {
                items: vec![],
                next_anchor: None,
            });
        }
        enforce_page_limit(page.limit)?;
        let last_key = page.last_key.clone().unwrap_or_default();
        let (last_block, last_exec_id_lc) = parse_execution_last_key(&last_key)?;
        let mut items: Vec<MatchedExecutionRecord> = self
            .inner
            .state
            .matched_executions
            .iter()
            .filter(|(_, row)| row.completion_status == ExecutionCompletion::Complete)
            .filter(|(_, row)| match subkey {
                Some(sk) => {
                    row.buyer_subkey.eq_ignore_ascii_case(sk)
                        || row.seller_subkey.eq_ignore_ascii_case(sk)
                }
                None => true,
            })
            .filter(|(id, row)| {
                if last_block == 0 {
                    return true;
                }
                let block_i = row.block_number as i64;
                block_i < last_block
                    || (block_i == last_block && id.to_lowercase() < last_exec_id_lc)
            })
            .map(|(id, row)| MatchedExecutionRecord {
                deployment_id,
                execution_id: id.clone(),
                buyer_order_hash: row.buyer_order_hash.clone(),
                seller_order_hash: row.seller_order_hash.clone(),
                buyer_subkey: row.buyer_subkey.clone(),
                seller_subkey: row.seller_subkey.clone(),
                series_id: row.series_id.clone(),
                matched_qty_1e8: row.matched_qty_1e8.clone(),
                premium_amount: row.premium_amount.clone(),
                fee_amount: row.fee_amount.clone(),
                rebate_amount: row.rebate_amount.clone(),
                block_number: row.block_number,
                tx_hash: row.tx_hash.clone(),
                completion_status: row.completion_status.as_str().to_string(),
            })
            .collect();
        items.sort_by(|a, b| {
            b.block_number.cmp(&a.block_number).then_with(|| {
                b.execution_id
                    .to_lowercase()
                    .cmp(&a.execution_id.to_lowercase())
            })
        });
        items.truncate(page.limit + 1);
        Ok(finalize_execution_page(items, page.limit))
    }

    async fn list_fees(
        &self,
        deployment_id: u64,
        subkey: &str,
        page: &PageAnchor,
    ) -> Result<StorePage<FeeRebateRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(StorePage {
                items: vec![],
                next_anchor: None,
            });
        }
        enforce_page_limit(page.limit)?;
        let last_key = page.last_key.clone().unwrap_or_default();
        let (last_block, last_log_index) = parse_fee_last_key(&last_key)?;
        let mut items: Vec<FeeRebateRecord> = self
            .inner
            .state
            .fee_events
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row.payer_subkey
                    .as_deref()
                    .map(|s| s.eq_ignore_ascii_case(subkey))
                    .unwrap_or(false)
                    || row
                        .receiver_subkey
                        .as_deref()
                        .map(|s| s.eq_ignore_ascii_case(subkey))
                        .unwrap_or(false)
            })
            .filter(|(_, row)| {
                if last_block == 0 {
                    return true;
                }
                let b = row.block_number as i64;
                b < last_block || (b == last_block && (row.log_index as i32) < last_log_index)
            })
            .map(|(idx, row)| FeeRebateRecord {
                deployment_id,
                event_id: idx as i64,
                kind: row.kind.to_string(),
                payer_subkey: row.payer_subkey.clone(),
                receiver_subkey: row.receiver_subkey.clone(),
                token: row.token.clone(),
                amount: row.amount.clone(),
                block_number: row.block_number,
                tx_hash: row.tx_hash.clone(),
                log_index: row.log_index,
                execution_id: row.execution_id.clone(),
            })
            .collect();
        items.sort_by(|a, b| {
            b.block_number
                .cmp(&a.block_number)
                .then_with(|| b.log_index.cmp(&a.log_index))
        });
        items.truncate(page.limit + 1);
        Ok(finalize_fee_page(items, page.limit))
    }

    async fn get_recovery(
        &self,
        deployment_id: u64,
        subkey: &str,
    ) -> Result<Option<RecoveryRecord>, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(None);
        }
        let owner_of_sub = self
            .inner
            .state
            .subaccounts
            .iter()
            .find(|(_, sk)| sk.eq_ignore_ascii_case(subkey))
            .map(|((o, _), _)| o.clone());
        let state = self
            .inner
            .state
            .recovery_state
            .get(subkey)
            .copied()
            .unwrap_or(RecoveryStateProjection::Normal);
        let escape = self.inner.state.escape_state.get(subkey).cloned();
        let sub_epoch = self
            .inner
            .state
            .subaccount_recovery_epochs
            .get(subkey)
            .cloned()
            .unwrap_or_default();
        let owner_epoch = match &owner_of_sub {
            Some(o) => self
                .inner
                .state
                .owner_recovery_epochs
                .get(o)
                .cloned()
                .unwrap_or_default(),
            None => Default::default(),
        };
        let withdrawal_count = self
            .inner
            .state
            .recovery_finalization_withdrawals
            .get(subkey)
            .copied()
            .unwrap_or(0);
        Ok(Some(RecoveryRecord {
            subkey: subkey.to_string(),
            recovery_state: state.as_str().to_string(),
            finalized: matches!(state, RecoveryStateProjection::Recovered),
            owner_recovery_epoch: owner_epoch.epoch_count,
            subaccount_recovery_epoch: sub_epoch.epoch_count,
            owner_min_valid_nonce: owner_epoch.min_valid_nonce.clone(),
            subaccount_min_valid_nonce: sub_epoch.min_valid_nonce.clone(),
            escape,
            withdrawal_count,
            finalized_at_ts: None,
        }))
    }

    async fn query_history(
        &self,
        deployment_id: u64,
        scope: &HistoryScope,
        filter: &HistoryFilter,
        page: &HistoryPageAnchor,
    ) -> Result<HistoryPage, ReadStoreError> {
        if deployment_id != self.inner.deployment_id {
            return Ok(HistoryPage {
                items: vec![],
                next_anchor: None,
            });
        }
        enforce_page_limit(page.limit)?;
        let expected_hash = filter_stable_hash(filter);
        if page.filter_hash != expected_hash {
            return Err(ReadStoreError::InvalidCursor {
                detail: "cursor filter hash does not match current filter".into(),
            });
        }
        if matches!(page.consistency, HistoryConsistency::Indexed)
            && !page.indexed_head_hash.is_empty()
            && !self.inner.cursor.indexed_head_hash.is_empty()
            && self.inner.cursor.indexed_head_hash != page.indexed_head_hash
        {
            return Err(ReadStoreError::StaleCursor {
                expected_hash: page.indexed_head_hash.clone(),
                actual_hash: self.inner.cursor.indexed_head_hash.clone(),
            });
        }
        let owner_filter: Option<String> = match scope {
            HistoryScope::Owner { owner } => Some(owner.clone()),
            _ => filter.owner.clone(),
        };
        let subkey_filter: Option<String> = match scope {
            HistoryScope::Subaccount { subkey } => Some(subkey.clone()),
            _ => filter.subkey.clone(),
        };
        let mut items: Vec<HistoryRecord> = Vec::new();
        for entry in self.inner.journal.iter().filter(|e| e.is_canonical) {
            if let Some(o) = owner_filter.as_deref() {
                if entry.owner.as_deref().map(|x| x.eq_ignore_ascii_case(o)) != Some(true) {
                    continue;
                }
            }
            if let Some(sk) = subkey_filter.as_deref() {
                if entry.subkey.as_deref().map(|x| x.eq_ignore_ascii_case(sk)) != Some(true) {
                    continue;
                }
            }
            if let Some(tok) = filter.token.as_deref() {
                if entry.token.as_deref().map(|x| x.eq_ignore_ascii_case(tok)) != Some(true) {
                    continue;
                }
            }
            if let Some(s) = filter.series_id.as_deref() {
                if entry
                    .series_id
                    .as_deref()
                    .map(|x| x.eq_ignore_ascii_case(s))
                    != Some(true)
                {
                    continue;
                }
            }
            if let Some(o) = filter.order_hash.as_deref() {
                if entry
                    .order_hash
                    .as_deref()
                    .map(|x| x.eq_ignore_ascii_case(o))
                    != Some(true)
                {
                    continue;
                }
            }
            if let Some(e) = filter.execution_id.as_deref() {
                if entry
                    .execution_id
                    .as_deref()
                    .map(|x| x.eq_ignore_ascii_case(e))
                    != Some(true)
                {
                    continue;
                }
            }
            if let Some(f) = filter.from_block {
                if entry.block_number < f {
                    continue;
                }
            }
            if let Some(t) = filter.to_block {
                if entry.block_number > t {
                    continue;
                }
            }
            let (payload, related_order, related_exec) = event_to_history_payload(
                entry.kind,
                &entry.payload,
                entry.subkey.as_deref(),
                entry.owner.as_deref(),
                entry.subaccount_id.map(|v| v as i64),
                entry.token.as_deref(),
                entry.engine.as_deref(),
                entry.execution_id.as_deref(),
                entry.order_hash.as_deref(),
                entry.series_id.as_deref(),
            );
            let direction = classify_direction(entry.kind);
            if let Some(d) = &filter.direction {
                if direction != *d {
                    continue;
                }
            }
            if !filter.families.is_empty()
                && !filter
                    .families
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(payload.family_label()))
            {
                continue;
            }
            let event_id = format!("{}:{}:{}", entry.block_hash, entry.tx_hash, entry.log_index);
            items.push(HistoryRecord {
                event_id,
                deployment_id,
                chain_id: self.inner.chain_id,
                block_number: entry.block_number,
                block_hash: entry.block_hash.clone(),
                tx_hash: entry.tx_hash.clone(),
                tx_index: entry.tx_index,
                log_index: entry.log_index,
                timestamp_ms: entry.block_timestamp.saturating_mul(1000),
                finalized: entry.block_number <= self.inner.finalized_head_block,
                direction,
                owner: entry.owner.clone(),
                subaccount_id: entry.subaccount_id,
                subkey: entry.subkey.clone(),
                related_order_hash: related_order,
                related_execution_id: related_exec,
                payload,
            });
        }
        items.sort_by(|a, b| {
            b.block_number.cmp(&a.block_number).then_with(|| {
                b.tx_index
                    .cmp(&a.tx_index)
                    .then_with(|| b.log_index.cmp(&a.log_index))
                    .then_with(|| b.event_id.cmp(&a.event_id))
            })
        });
        // Apply after-anchor.
        if let Some(anchor) = &page.after {
            items.retain(|r| {
                r.block_number < anchor.block_number
                    || (r.block_number == anchor.block_number && r.tx_index < anchor.tx_index)
                    || (r.block_number == anchor.block_number
                        && r.tx_index == anchor.tx_index
                        && r.log_index < anchor.log_index)
                    || (r.block_number == anchor.block_number
                        && r.tx_index == anchor.tx_index
                        && r.log_index == anchor.log_index
                        && r.event_id < anchor.event_id)
            });
        }
        items.truncate(page.limit + 1);
        Ok(finalize_history_page(items, page))
    }
}

// -----------------------------------------------------------------
//                       HELPER FUNCTIONS
// -----------------------------------------------------------------

fn subaccount_meta_kind(state: &ProjectionState, subkey: &str) -> String {
    let meta = state
        .subaccount_meta
        .get(subkey)
        .copied()
        .unwrap_or_default();
    match (meta.materialised_via_created, meta.materialised_via_lazy) {
        (true, true) => "BOTH".to_string(),
        (true, false) => "CREATED".to_string(),
        (false, true) => "LAZY".to_string(),
        (false, false) => "UNKNOWN".to_string(),
    }
}

fn row_to_subaccount(r: &sqlx::postgres::PgRow) -> Result<SubaccountRecord, ReadStoreError> {
    let deployment_id: i64 = r.try_get("deployment_id").map_err(backend_err)?;
    let sid: i64 = r.try_get("subaccount_id").map_err(backend_err)?;
    let created_block: i64 = r.try_get("created_block").map_err(backend_err)?;
    Ok(SubaccountRecord {
        deployment_id: safe_u64("deployment_id", deployment_id)?,
        subkey: r.try_get("subkey").map_err(backend_err)?,
        owner: r.try_get("owner").map_err(backend_err)?,
        subaccount_id: safe_u32("subaccount_id", sid as i32)?,
        materialised_via: r.try_get("materialised_via").map_err(backend_err)?,
        created_block: safe_u64("created_block", created_block)?,
        created_tx_hash: r.try_get("created_tx_hash").map_err(backend_err)?,
    })
}

fn row_to_order_lifecycle(
    r: &sqlx::postgres::PgRow,
) -> Result<OrderLifecycleRecord, ReadStoreError> {
    let deployment_id: i64 = r.try_get("deployment_id").map_err(backend_err)?;
    let side: i16 = r.try_get("side").map_err(backend_err)?;
    let tif: i16 = r.try_get("time_in_force").map_err(backend_err)?;
    let total_qty: String = r.try_get("total_qty_1e8").map_err(backend_err)?;
    let filled_qty: String = r.try_get("filled_qty_1e8").map_err(backend_err)?;
    let remaining =
        crate::hybrid_v2::reducer::u256_sub(&total_qty, &filled_qty).unwrap_or_else(|| "0".into());
    let first_seen_block: i64 = r.try_get("first_seen_block").map_err(backend_err)?;
    let last_event_block: i64 = r.try_get("last_event_block").map_err(backend_err)?;
    Ok(OrderLifecycleRecord {
        order_hash: r.try_get("order_hash").map_err(backend_err)?,
        deployment_id: safe_u64("deployment_id", deployment_id)?,
        subkey: r.try_get("subkey").map_err(backend_err)?,
        owner: r.try_get("owner").map_err(backend_err)?,
        series_id: r.try_get("series_id").map_err(backend_err)?,
        side: side.try_into().map_err(|_| malformed("side", "negative"))?,
        time_in_force: tif
            .try_into()
            .map_err(|_| malformed("time_in_force", "negative"))?,
        total_qty_1e8: total_qty,
        filled_qty_1e8: filled_qty,
        remaining_qty_1e8: remaining,
        cancelled: r.try_get("cancelled").map_err(backend_err)?,
        terminal: r.try_get("terminal").map_err(backend_err)?,
        first_seen_block: safe_u64("first_seen_block", first_seen_block)?,
        last_event_block: safe_u64("last_event_block", last_event_block)?,
    })
}

fn row_to_execution(r: &sqlx::postgres::PgRow) -> Result<MatchedExecutionRecord, ReadStoreError> {
    let deployment_id: i64 = r.try_get("deployment_id").map_err(backend_err)?;
    let block_number: i64 = r.try_get("block_number").map_err(backend_err)?;
    Ok(MatchedExecutionRecord {
        deployment_id: safe_u64("deployment_id", deployment_id)?,
        execution_id: r.try_get("execution_id").map_err(backend_err)?,
        buyer_order_hash: r.try_get("buyer_order_hash").map_err(backend_err)?,
        seller_order_hash: r.try_get("seller_order_hash").map_err(backend_err)?,
        buyer_subkey: r.try_get("buyer_subkey").map_err(backend_err)?,
        seller_subkey: r.try_get("seller_subkey").map_err(backend_err)?,
        series_id: r.try_get("series_id").map_err(backend_err)?,
        matched_qty_1e8: r.try_get("matched_qty_1e8").map_err(backend_err)?,
        premium_amount: r.try_get("premium_amount").map_err(backend_err)?,
        fee_amount: r.try_get("fee_amount").map_err(backend_err)?,
        rebate_amount: r.try_get("rebate_amount").map_err(backend_err)?,
        block_number: safe_u64("block_number", block_number)?,
        tx_hash: r.try_get("tx_hash").map_err(backend_err)?,
        completion_status: r.try_get("completion_status").map_err(backend_err)?,
    })
}

fn row_to_fee(r: &sqlx::postgres::PgRow) -> Result<FeeRebateRecord, ReadStoreError> {
    let deployment_id: i64 = r.try_get("deployment_id").map_err(backend_err)?;
    let event_id: i64 = r.try_get("event_id").map_err(backend_err)?;
    let block_number: i64 = r.try_get("block_number").map_err(backend_err)?;
    let log_index: i32 = r.try_get("log_index").map_err(backend_err)?;
    Ok(FeeRebateRecord {
        deployment_id: safe_u64("deployment_id", deployment_id)?,
        event_id,
        kind: r.try_get("kind").map_err(backend_err)?,
        payer_subkey: r.try_get("payer_subkey").map_err(backend_err)?,
        receiver_subkey: r.try_get("receiver_subkey").map_err(backend_err)?,
        token: r.try_get("token").map_err(backend_err)?,
        amount: r.try_get("amount").map_err(backend_err)?,
        block_number: safe_u64("block_number", block_number)?,
        tx_hash: r.try_get("tx_hash").map_err(backend_err)?,
        log_index: safe_u32("log_index", log_index)?,
        execution_id: r.try_get("execution_id").map_err(backend_err)?,
    })
}

fn row_to_escape_state(r: &sqlx::postgres::PgRow) -> Result<EscapeStateRow, ReadStoreError> {
    let state_s: String = r.try_get("state").map_err(backend_err)?;
    let state_static: &'static str = match state_s.as_str() {
        "NORMAL" => "NORMAL",
        "REQUESTED" => "REQUESTED",
        "ACTIVATED" => "ACTIVATED",
        "CANCELLED" => "CANCELLED",
        "FINALIZED" => "FINALIZED",
        other => {
            return Err(ReadStoreError::MalformedRow {
                detail: format!("unknown escape state '{other}'"),
            })
        }
    };
    let requested_ts: Option<i64> = r.try_get("requested_ts").map_err(backend_err)?;
    let activation_eligible_at: Option<i64> =
        r.try_get("activation_eligible_at").map_err(backend_err)?;
    let activated_ts: Option<i64> = r.try_get("activated_ts").map_err(backend_err)?;
    let cancelled_ts: Option<i64> = r.try_get("cancelled_ts").map_err(backend_err)?;
    let finalized_ts: Option<i64> = r.try_get("finalized_ts").map_err(backend_err)?;
    let last_event_block: i64 = r.try_get("last_event_block").map_err(backend_err)?;
    Ok(EscapeStateRow {
        state: state_static,
        requested_ts: requested_ts.map(|v| v as u64),
        activation_eligible_at: activation_eligible_at.map(|v| v as u64),
        activated_ts: activated_ts.map(|v| v as u64),
        cancelled_ts: cancelled_ts.map(|v| v as u64),
        finalized_ts: finalized_ts.map(|v| v as u64),
        last_event_block: safe_u64("last_event_block", last_event_block)?,
    })
}

/// Parse an execution-page last_key of the form "block:execution_id".
fn parse_execution_last_key(k: &str) -> Result<(i64, String), ReadStoreError> {
    if k.is_empty() {
        return Ok((0, String::new()));
    }
    let mut parts = k.splitn(2, ':');
    let block = parts
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| ReadStoreError::InvalidCursor {
            detail: format!("execution last_key: bad block segment in '{k}'"),
        })?;
    let exec_id = parts
        .next()
        .ok_or_else(|| ReadStoreError::InvalidCursor {
            detail: format!("execution last_key: missing execution_id in '{k}'"),
        })?
        .to_lowercase();
    Ok((block, exec_id))
}

/// Parse a fee-page last_key of the form "block:log_index".
fn parse_fee_last_key(k: &str) -> Result<(i64, i32), ReadStoreError> {
    if k.is_empty() {
        return Ok((0, 0));
    }
    let mut parts = k.splitn(2, ':');
    let block = parts
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| ReadStoreError::InvalidCursor {
            detail: format!("fee last_key: bad block segment in '{k}'"),
        })?;
    let log_index = parts
        .next()
        .and_then(|s| s.parse::<i32>().ok())
        .ok_or_else(|| ReadStoreError::InvalidCursor {
            detail: format!("fee last_key: bad log_index segment in '{k}'"),
        })?;
    Ok((block, log_index))
}

fn enforce_page_limit(limit: usize) -> Result<(), ReadStoreError> {
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(ReadStoreError::LimitExceeded {
            max: MAX_PAGE_LIMIT,
            requested: limit,
        });
    }
    Ok(())
}

fn finalize_page<T: Clone>(mut items: Vec<T>, limit: usize) -> StorePage<T>
where
    T: RecordKey,
{
    let next_anchor = if items.len() > limit {
        let last = items[limit - 1].clone();
        items.truncate(limit);
        Some(PageAnchor {
            last_key: Some(last.record_key()),
            limit,
        })
    } else {
        None
    };
    StorePage { items, next_anchor }
}

/// Distinct finalization for executions — the keyset key is composite
/// (`"block:execution_id"`).
fn finalize_execution_page(
    mut items: Vec<MatchedExecutionRecord>,
    limit: usize,
) -> StorePage<MatchedExecutionRecord> {
    let next_anchor = if items.len() > limit {
        let last = items[limit - 1].clone();
        items.truncate(limit);
        Some(PageAnchor {
            last_key: Some(format!("{}:{}", last.block_number, last.execution_id)),
            limit,
        })
    } else {
        None
    };
    StorePage { items, next_anchor }
}

/// Distinct finalization for fees — the keyset key is composite
/// (`"block:log_index"`).
fn finalize_fee_page(mut items: Vec<FeeRebateRecord>, limit: usize) -> StorePage<FeeRebateRecord> {
    let next_anchor = if items.len() > limit {
        let last = items[limit - 1].clone();
        items.truncate(limit);
        Some(PageAnchor {
            last_key: Some(format!("{}:{}", last.block_number, last.log_index)),
            limit,
        })
    } else {
        None
    };
    StorePage { items, next_anchor }
}

fn finalize_history_page(mut items: Vec<HistoryRecord>, page: &HistoryPageAnchor) -> HistoryPage {
    let next_anchor = if items.len() > page.limit {
        let last = items[page.limit - 1].clone();
        items.truncate(page.limit);
        Some(HistoryPageAnchor {
            limit: page.limit,
            after: Some(HistoryCursorKey {
                block_number: last.block_number,
                tx_index: last.tx_index,
                log_index: last.log_index,
                event_id: last.event_id.clone(),
            }),
            consistency: page.consistency,
            filter_hash: page.filter_hash.clone(),
            indexed_head_hash: page.indexed_head_hash.clone(),
        })
    } else {
        None
    };
    HistoryPage { items, next_anchor }
}

/// Deterministic filter hash used for cursor binding.
pub fn filter_stable_hash(filter: &HistoryFilter) -> String {
    let s = serde_json::to_string(&filter.to_stable_json()).unwrap_or_default();
    let digest = sha2_hex(&s);
    digest.chars().take(32).collect()
}

fn sha2_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let d = h.finalize();
    let mut out = String::with_capacity(d.len() * 2);
    for b in d {
        out.push(char::from_digit(u32::from(b >> 4), 16).unwrap());
        out.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap());
    }
    out
}

trait RecordKey {
    fn record_key(&self) -> String;
}

impl RecordKey for PositionRecord {
    fn record_key(&self) -> String {
        self.series_id.to_lowercase()
    }
}

impl RecordKey for OrderLifecycleRecord {
    fn record_key(&self) -> String {
        self.order_hash.to_lowercase()
    }
}

fn u64_to_i64(field: &'static str, value: u64) -> Result<i64, ReadStoreError> {
    i64::try_from(value).map_err(|_| ReadStoreError::MalformedRow {
        detail: format!("field {field} exceeds i64::MAX: {value}"),
    })
}

fn safe_u64(field: &'static str, value: i64) -> Result<u64, ReadStoreError> {
    u64::try_from(value).map_err(|_| ReadStoreError::MalformedRow {
        detail: format!("field {field} negative in PG: {value}"),
    })
}

fn safe_u32(field: &'static str, value: i32) -> Result<u32, ReadStoreError> {
    u32::try_from(value).map_err(|_| ReadStoreError::MalformedRow {
        detail: format!("field {field} negative in PG: {value}"),
    })
}

/// Classify an `EventKind` into an `HistoryDirection`. Kept locally so
/// the read store does not depend on the private `history::classify_direction`
/// helper. Falls back to `Metadata` for events with no owner/subKey
/// mutation impact.
fn classify_direction(kind: EventKind) -> HistoryDirection {
    match kind {
        EventKind::Deposit
        | EventKind::CollateralUnlocked
        | EventKind::OrphanedLockReleased
        | EventKind::OptionRebatePaid
        | EventKind::RecoveryFinalizationWithdrawn => HistoryDirection::Inbound,
        EventKind::Withdraw
        | EventKind::CollateralLocked
        | EventKind::OptionFeeCharged
        | EventKind::BadDebtSocialized => HistoryDirection::Outbound,
        EventKind::InternalTransfer | EventKind::OptionPremiumTransferred => {
            HistoryDirection::Internal
        }
        _ => HistoryDirection::Metadata,
    }
}

/// Deterministic mapping from a canonical event kind + decoded fields
/// → tagged history payload. Mirrors the semantics of
/// `history::to_payload` used by the existing in-memory build path.
#[allow(clippy::too_many_arguments)]
fn event_to_history_payload(
    kind: EventKind,
    payload: &serde_json::Value,
    subkey: Option<&str>,
    _owner: Option<&str>,
    _subaccount_id: Option<i64>,
    token: Option<&str>,
    engine: Option<&str>,
    execution_id: Option<&str>,
    order_hash: Option<&str>,
    _series_id: Option<&str>,
) -> (HistoryEventPayload, Option<String>, Option<String>) {
    use HistoryEventPayload::*;
    let s_amount = || payload_string(payload, "amount");
    let s_from = || payload_string(payload, "from_subkey");
    let s_to = || payload_string(payload, "to_subkey");
    let p = match kind {
        EventKind::SubaccountCreated => SubaccountCreated {
            owner: payload_string(payload, "owner").unwrap_or_default(),
            subaccount_id: payload
                .get("subaccount_id")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
        },
        EventKind::SubaccountLazyRegistered => SubaccountLazyRegistered {
            owner: payload_string(payload, "owner").unwrap_or_default(),
            subaccount_id: payload
                .get("subaccount_id")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
        },
        EventKind::Deposit => Deposit {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
        },
        EventKind::Withdraw => Withdraw {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
        },
        EventKind::InternalTransfer => InternalTransfer {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
            from_subkey: s_from().unwrap_or_default(),
            to_subkey: s_to().unwrap_or_default(),
        },
        EventKind::CollateralLocked => ReservationIncrease {
            token: token.unwrap_or_default().to_string(),
            engine: engine.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
        },
        EventKind::CollateralUnlocked => ReservationRelease {
            token: token.unwrap_or_default().to_string(),
            engine: engine.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
        },
        EventKind::OrphanedLockReleased => OrphanedLockReleased {
            token: token.unwrap_or_default().to_string(),
            engine: engine.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
        },
        EventKind::OptionOrderFilled => OptionOrderFilled {
            order_hash: order_hash.unwrap_or_default().to_string(),
            filled_delta_1e8: payload_string(payload, "filled_delta_1e8").unwrap_or_default(),
            terminal: payload
                .get("terminal")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        EventKind::OptionOrderCancelled => OptionOrderCancelled {
            order_hash: order_hash.unwrap_or_default().to_string(),
        },
        EventKind::OptionOrderPairExecuted => MatchedExecution {
            execution_id: execution_id.unwrap_or_default().to_string(),
            buyer_order_hash: payload_string(payload, "buyer_order_hash").unwrap_or_default(),
            seller_order_hash: payload_string(payload, "seller_order_hash").unwrap_or_default(),
            matched_qty_1e8: payload_string(payload, "matched_qty_1e8").unwrap_or_default(),
        },
        EventKind::OptionPremiumTransferred => PremiumTransferred {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
            from_subkey: s_from().unwrap_or_default(),
            to_subkey: s_to().unwrap_or_default(),
        },
        EventKind::OptionFeeCharged => FeeCharged {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
            fee_subkey: payload_string(payload, "fee_subkey").unwrap_or_default(),
        },
        EventKind::OptionRebatePaid => RebatePaid {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
            rebate_subkey: payload_string(payload, "rebate_subkey").unwrap_or_default(),
        },
        EventKind::OptionSubaccountMinValidOrderNonceAdvanced => MinValidNonceAdvanced {
            new_min_valid_nonce: payload_string(payload, "new_min_valid_nonce").unwrap_or_default(),
        },
        EventKind::RecoveryRequested => RecoveryRequested,
        EventKind::RecoveryActivated => RecoveryActivated,
        EventKind::RecoveryCancelled => RecoveryCancelled,
        EventKind::RecoveryFinalized => RecoveryFinalized,
        EventKind::RecoveryFinalizationWithdrawn => RecoveryFinalizationWithdrawn {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
        },
        EventKind::RecoveryEpochIncremented => RecoveryEpochIncremented,
        EventKind::OwnerRecoveryEpochAdvanced => OwnerRecoveryEpochAdvanced,
        EventKind::SubaccountRecoveryEpochAdvanced => SubaccountRecoveryEpochAdvanced,
        EventKind::PauseFlagChanged => PauseFlagChanged {
            paused: payload
                .get("paused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        EventKind::BadDebtSocialized => BadDebtSocialized {
            token: token.unwrap_or_default().to_string(),
            amount: s_amount().unwrap_or_default(),
        },
        EventKind::EngineCapabilityChanged => CapabilityEvent {
            kind: "ENGINE_CAPABILITY_CHANGED".into(),
            engine: engine.map(str::to_string),
        },
        EventKind::EngineGuardianRevoked => CapabilityEvent {
            kind: "ENGINE_GUARDIAN_REVOKED".into(),
            engine: engine.map(str::to_string),
        },
        EventKind::GuardianChanged => CapabilityEvent {
            kind: "GUARDIAN_CHANGED".into(),
            engine: engine.map(str::to_string),
        },
        EventKind::RiskParamsSet => RiskEvent {
            kind: "RISK_PARAMS_SET".into(),
        },
        EventKind::RiskModuleActivated => RiskEvent {
            kind: "RISK_MODULE_ACTIVATED".into(),
        },
        EventKind::LiquidationTriggered => RiskEvent {
            kind: "LIQUIDATION_TRIGGERED".into(),
        },
        EventKind::DeploymentManifestDeclared => Manifest {
            kind: "DEPLOYMENT_MANIFEST_DECLARED".into(),
        },
        EventKind::SupportedTokenAdded => Manifest {
            kind: "SUPPORTED_TOKEN_ADDED".into(),
        },
        EventKind::SupportedTokenRemoved => Manifest {
            kind: "SUPPORTED_TOKEN_REMOVED".into(),
        },
        EventKind::CollateralTokenEnteredUniverse => Manifest {
            kind: "COLLATERAL_TOKEN_ENTERED_UNIVERSE".into(),
        },
        EventKind::RecoveryPauseSet => Manifest {
            kind: "RECOVERY_PAUSE_SET".into(),
        },
        EventKind::ProtocolSubaccountsInitialized => Manifest {
            kind: "PROTOCOL_SUBACCOUNTS_INITIALIZED".into(),
        },
        EventKind::EscapeControllerInitialized => Manifest {
            kind: "ESCAPE_CONTROLLER_INITIALIZED".into(),
        },
        EventKind::RecoveryFinalizerInitialized => Manifest {
            kind: "RECOVERY_FINALIZER_INITIALIZED".into(),
        },
        EventKind::IntentConsumed | EventKind::NonceCancelled => Other {
            kind: kind.as_str().to_string(),
        },
        EventKind::OptionPositionOpened
        | EventKind::OptionPositionModified
        | EventKind::OptionPositionClosed
        | EventKind::OptionExercised
        | EventKind::OptionSettled
        | EventKind::OptionPositionLiquidated => Other {
            kind: kind.as_str().to_string(),
        },
    };
    let related_order = order_hash.map(str::to_string);
    let related_exec = execution_id.map(str::to_string);
    let _ = subkey;
    (p, related_order, related_exec)
}

fn payload_string(payload: &serde_json::Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(|v| v.as_str().map(str::to_string))
}

// -----------------------------------------------------------------
//                       UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_execution_last_key_ok() {
        let (b, id) = parse_execution_last_key("42:0xexec").unwrap();
        assert_eq!(b, 42);
        assert_eq!(id, "0xexec");
        assert_eq!(parse_execution_last_key("").unwrap(), (0, String::new()));
        assert!(parse_execution_last_key("bad:0xexec").is_err());
    }

    #[test]
    fn parse_fee_last_key_ok() {
        let (b, i) = parse_fee_last_key("100:5").unwrap();
        assert_eq!(b, 100);
        assert_eq!(i, 5);
        assert!(parse_fee_last_key("bad:5").is_err());
        assert!(parse_fee_last_key("5:bad").is_err());
    }

    #[test]
    fn enforce_page_limit_rejects_zero_and_over_max() {
        assert!(enforce_page_limit(0).is_err());
        assert!(enforce_page_limit(MAX_PAGE_LIMIT + 1).is_err());
        assert!(enforce_page_limit(MAX_PAGE_LIMIT).is_ok());
    }

    #[test]
    fn safe_u64_rejects_negative() {
        assert!(safe_u64("x", -1).is_err());
        assert_eq!(safe_u64("x", 42).unwrap(), 42);
    }

    #[test]
    fn filter_stable_hash_deterministic() {
        let f1 = HistoryFilter::default();
        let f2 = HistoryFilter::default();
        assert_eq!(filter_stable_hash(&f1), filter_stable_hash(&f2));
        assert_eq!(filter_stable_hash(&f1).len(), 32);
    }

    #[test]
    fn read_store_error_display_no_credential_leak() {
        let e = ReadStoreError::Backend {
            detail: "connection refused".into(),
        };
        assert!(!format!("{e}").contains("postgres://"));
    }

    #[test]
    fn read_store_trait_is_object_safe() {
        fn _accepts(_s: &dyn HybridV2ReadStore) {}
    }
}
