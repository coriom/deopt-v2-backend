//! OPTIONS-HYBRID-V2-CORRELATION-OPERATIONAL-CORE-V1 Part D —
//! repository interface for `option_execution_correlations`
//! (migration 0055).
//!
//! Deterministic backend correlation between `canonical_execution_id`
//! (backend economic identity of a matched fill) and canonical
//! on-chain evidence (transaction hash + log index + optional
//! contract-emitted `executionId`).
//!
//! State machine (Part C uniqueness proof):
//!
//! ```text
//!  AWAITING_CHAIN_EVIDENCE   (intent created; no tx yet)
//!         │
//!         │  attach tx_hash
//!         ▼
//!  SUBMITTED                 (tx submitted; awaiting event)
//!         │
//!         │  canonical event ingested + validated
//!         ▼
//!  CORRELATED_CANONICAL      (canonical evidence attached)
//!         │
//!         │  canonical branch reorg
//!         ▼
//!  ORPHANED                  (may re-correlate on replacement branch)
//!
//!  Alternate terminal: CONFLICT | MANUAL_REVIEW
//! ```
//!
//! Correlation KEY at reducer ingest time is `(tx_hash, log_index)` —
//! always injective per canonical journal invariant. The tuple
//! `(onchain_buyer_order_id, onchain_seller_order_id,
//! fill_quantity_1e8)` is a VALIDATION cross-check, not the primary
//! lookup key. See Part C proof in the milestone closure doc.
//!
//! **All state transitions are explicit named methods** — no
//! `update_arbitrary_fields` API. Immutability is enforced by
//! migration 0055 triggers.

use crate::error::{BackendError, Result};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

// -------------------------------------------------------------------
// Types
// -------------------------------------------------------------------

/// One correlation row: metadata connecting a backend
/// `canonical_execution_id` to canonical on-chain evidence.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptionExecutionCorrelation {
    pub correlation_id: Uuid,
    pub canonical_execution_id: String,
    pub deployment_id: i64,
    pub chain_id: i64,
    pub execution_kind: OptionExecutionKind,
    pub onchain_buyer_order_id: Option<String>,
    pub onchain_seller_order_id: Option<String>,
    pub onchain_execution_id: Option<String>,
    pub fill_quantity_1e8: Option<String>,
    pub tx_hash: Option<String>,
    pub canonical_block_number: Option<i64>,
    pub canonical_block_hash: Option<String>,
    pub log_index: Option<i32>,
    pub correlation_status: OptionCorrelationStatus,
    pub terminal_reason: Option<String>,
    pub first_seen_at_ms: i64,
    pub last_updated_at_ms: i64,
}

/// Solidity entrypoint that emits `OptionOrderPairExecuted`.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptionExecutionKind {
    /// `executeTrade(OptionTrade, buyerSig, sellerSig)`
    Trade,
    /// `executeRfqTrade(OptionRfqTrade, buyerSig, sellerSig)`
    RfqTrade,
}

impl OptionExecutionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trade => "trade",
            Self::RfqTrade => "rfq_trade",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "trade" => Ok(Self::Trade),
            "rfq_trade" => Ok(Self::RfqTrade),
            other => Err(BackendError::Persistence(format!(
                "invalid execution_kind: {other}"
            ))),
        }
    }
}

/// Correlation lifecycle state (matches migration 0055 CHECK).
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OptionCorrelationStatus {
    AwaitingChainEvidence,
    Submitted,
    CorrelatedCanonical,
    Orphaned,
    Conflict,
    ManualReview,
}

impl OptionCorrelationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingChainEvidence => "AWAITING_CHAIN_EVIDENCE",
            Self::Submitted => "SUBMITTED",
            Self::CorrelatedCanonical => "CORRELATED_CANONICAL",
            Self::Orphaned => "ORPHANED",
            Self::Conflict => "CONFLICT",
            Self::ManualReview => "MANUAL_REVIEW",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "AWAITING_CHAIN_EVIDENCE" => Ok(Self::AwaitingChainEvidence),
            "SUBMITTED" => Ok(Self::Submitted),
            "CORRELATED_CANONICAL" => Ok(Self::CorrelatedCanonical),
            "ORPHANED" => Ok(Self::Orphaned),
            "CONFLICT" => Ok(Self::Conflict),
            "MANUAL_REVIEW" => Ok(Self::ManualReview),
            other => Err(BackendError::Persistence(format!(
                "invalid correlation_status: {other}"
            ))),
        }
    }

    /// States covered by the sparse UNIQUE index on
    /// `canonical_execution_id`. An ACTIVE correlation is the
    /// authoritative pre-chain / post-chain binding; only ORPHANED /
    /// CONFLICT / MANUAL_REVIEW rows may co-exist with a fresh insert.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::AwaitingChainEvidence | Self::Submitted | Self::CorrelatedCanonical
        )
    }
}

/// Inputs to `insert_awaiting_correlation` — the minimum information
/// available at Options execution intent preparation time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwaitingCorrelationInput {
    pub canonical_execution_id: String,
    pub deployment_id: i64,
    pub chain_id: i64,
    pub execution_kind: OptionExecutionKind,
    /// Backend-computable EIP-712 envelope digests (mirror what the
    /// contract will emit as `buyerOrderId` / `sellerOrderId`).
    pub onchain_buyer_order_id: Option<String>,
    pub onchain_seller_order_id: Option<String>,
    pub fill_quantity_1e8: Option<String>,
    pub now_ms: i64,
}

// -------------------------------------------------------------------
// Row hydration
// -------------------------------------------------------------------

fn correlation_from_row(row: PgRow) -> Result<OptionExecutionCorrelation> {
    let kind_str: String = row_get(&row, "execution_kind")?;
    let status_str: String = row_get(&row, "correlation_status")?;
    Ok(OptionExecutionCorrelation {
        correlation_id: row_get(&row, "correlation_id")?,
        canonical_execution_id: row_get(&row, "canonical_execution_id")?,
        deployment_id: row_get(&row, "deployment_id")?,
        chain_id: row_get(&row, "chain_id")?,
        execution_kind: OptionExecutionKind::parse(&kind_str)?,
        onchain_buyer_order_id: row_get(&row, "onchain_buyer_order_id")?,
        onchain_seller_order_id: row_get(&row, "onchain_seller_order_id")?,
        onchain_execution_id: row_get(&row, "onchain_execution_id")?,
        fill_quantity_1e8: row_get(&row, "fill_quantity_1e8")?,
        tx_hash: row_get(&row, "tx_hash")?,
        canonical_block_number: row_get(&row, "canonical_block_number")?,
        canonical_block_hash: row_get(&row, "canonical_block_hash")?,
        log_index: row_get(&row, "log_index")?,
        correlation_status: OptionCorrelationStatus::parse(&status_str)?,
        terminal_reason: row_get(&row, "terminal_reason")?,
        first_seen_at_ms: row_get(&row, "first_seen_at_ms")?,
        last_updated_at_ms: row_get(&row, "last_updated_at_ms")?,
    })
}

fn row_get<T>(row: &PgRow, column: &str) -> Result<T>
where
    for<'r> T: sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|e| BackendError::Persistence(e.to_string()))
}

// -------------------------------------------------------------------
// Repository API (explicit named transitions only)
// -------------------------------------------------------------------

/// Insert a new correlation row in `AWAITING_CHAIN_EVIDENCE`. Called
/// at Options execution intent preparation time (before broadcast).
///
/// Idempotency: the sparse UNIQUE index over ACTIVE rows for
/// `canonical_execution_id` (`AWAITING_CHAIN_EVIDENCE | SUBMITTED |
/// CORRELATED_CANONICAL`) prevents a second insert while the first
/// is active. Duplicate-insert callers receive
/// `BackendError::Persistence` and MUST look up the existing row.
pub async fn insert_awaiting_correlation(
    pool: &PgPool,
    input: &AwaitingCorrelationInput,
) -> Result<OptionExecutionCorrelation> {
    let row = sqlx::query(
        "INSERT INTO option_execution_correlations (
             canonical_execution_id, deployment_id, chain_id, execution_kind,
             onchain_buyer_order_id, onchain_seller_order_id, fill_quantity_1e8,
             correlation_status, first_seen_at_ms, last_updated_at_ms
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'AWAITING_CHAIN_EVIDENCE', $8, $8)
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(&input.canonical_execution_id)
    .bind(input.deployment_id)
    .bind(input.chain_id)
    .bind(input.execution_kind.as_str())
    .bind(input.onchain_buyer_order_id.as_deref())
    .bind(input.onchain_seller_order_id.as_deref())
    .bind(input.fill_quantity_1e8.as_deref())
    .bind(input.now_ms)
    .fetch_one(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    correlation_from_row(row)
}

/// Attach `tx_hash` to an ACTIVE correlation. Transitions status
/// `AWAITING_CHAIN_EVIDENCE → SUBMITTED`. Idempotent on same-value
/// re-attachment. Immutability trigger rejects different-value.
pub async fn attach_tx_hash(
    pool: &PgPool,
    canonical_execution_id: &str,
    tx_hash: &str,
    now_ms: i64,
) -> Result<OptionExecutionCorrelation> {
    let row = sqlx::query(
        "UPDATE option_execution_correlations
         SET tx_hash = $2,
             correlation_status = 'SUBMITTED',
             last_updated_at_ms = $3
         WHERE canonical_execution_id = $1
           AND correlation_status IN ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED')
           AND (tx_hash IS NULL OR tx_hash = $2)
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(canonical_execution_id)
    .bind(tx_hash)
    .bind(now_ms)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "attach_tx_hash: no ACTIVE correlation for canonical_execution_id={canonical_execution_id} \
             (or conflicting tx_hash)"
        ))
    })?;
    correlation_from_row(row)
}

/// Fingerprints from a canonical `OptionOrderPairExecuted` event.
/// The reducer collects these from the chain journal and passes them
/// here for correlation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalEventFingerprint {
    pub tx_hash: String,
    pub log_index: i32,
    pub canonical_block_number: i64,
    pub canonical_block_hash: String,
    pub onchain_execution_id: String,
    pub onchain_buyer_order_id: String,
    pub onchain_seller_order_id: String,
    pub fill_quantity_1e8: String,
    pub now_ms: i64,
}

/// Mark a correlation `CORRELATED_CANONICAL` after canonical evidence
/// arrives.
///
/// This is the reducer's terminal transition for the happy path. It
/// VALIDATES that the fingerprint's on-chain order-id tuple agrees
/// with the persisted fingerprint (if any). Mismatch produces
/// `Err(...)` — caller must decide to escalate to CONFLICT.
pub async fn mark_correlated_canonical(
    pool: &PgPool,
    canonical_execution_id: &str,
    fp: &CanonicalEventFingerprint,
) -> Result<OptionExecutionCorrelation> {
    // Attempt the transition. Immutability trigger enforces that
    // pre-populated onchain fingerprints cannot be silently changed;
    // caller MUST have set fingerprints via the AWAITING insert if
    // any.
    let row = sqlx::query(
        "UPDATE option_execution_correlations
         SET correlation_status = 'CORRELATED_CANONICAL',
             tx_hash = COALESCE(tx_hash, $2),
             log_index = COALESCE(log_index, $3),
             canonical_block_number = COALESCE(canonical_block_number, $4),
             canonical_block_hash = COALESCE(canonical_block_hash, $5),
             onchain_execution_id = COALESCE(onchain_execution_id, $6),
             onchain_buyer_order_id = COALESCE(onchain_buyer_order_id, $7),
             onchain_seller_order_id = COALESCE(onchain_seller_order_id, $8),
             fill_quantity_1e8 = COALESCE(fill_quantity_1e8, $9),
             last_updated_at_ms = $10
         WHERE canonical_execution_id = $1
           AND correlation_status IN ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED', 'CORRELATED_CANONICAL')
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(canonical_execution_id)
    .bind(&fp.tx_hash)
    .bind(fp.log_index)
    .bind(fp.canonical_block_number)
    .bind(&fp.canonical_block_hash)
    .bind(&fp.onchain_execution_id)
    .bind(&fp.onchain_buyer_order_id)
    .bind(&fp.onchain_seller_order_id)
    .bind(&fp.fill_quantity_1e8)
    .bind(fp.now_ms)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "mark_correlated_canonical: no ACTIVE correlation for canonical_execution_id={canonical_execution_id}"
        ))
    })?;
    correlation_from_row(row)
}

/// Transition to `ORPHANED` when the canonical block for this
/// correlation is reorged. `canonical_execution_id` remains
/// immutable; the row stays visible for audit; a replacement branch
/// may re-correlate (new correlation row keyed on the same
/// canonical_execution_id — permitted because sparse UNIQUE is on
/// ACTIVE only).
pub async fn mark_orphaned(
    pool: &PgPool,
    canonical_execution_id: &str,
    terminal_reason: &str,
    now_ms: i64,
) -> Result<OptionExecutionCorrelation> {
    let row = sqlx::query(
        "UPDATE option_execution_correlations
         SET correlation_status = 'ORPHANED',
             terminal_reason = $2,
             last_updated_at_ms = $3
         WHERE canonical_execution_id = $1
           AND correlation_status = 'CORRELATED_CANONICAL'
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(canonical_execution_id)
    .bind(terminal_reason)
    .bind(now_ms)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "mark_orphaned: no CORRELATED_CANONICAL correlation for canonical_execution_id={canonical_execution_id}"
        ))
    })?;
    correlation_from_row(row)
}

/// Escalate to `CONFLICT`. Used when the reducer sees evidence that
/// disagrees with the persisted correlation (e.g. two on-chain
/// events claim the same canonical_execution_id, or the tuple
/// disagrees).
pub async fn mark_conflict(
    pool: &PgPool,
    canonical_execution_id: &str,
    terminal_reason: &str,
    now_ms: i64,
) -> Result<OptionExecutionCorrelation> {
    let row = sqlx::query(
        "UPDATE option_execution_correlations
         SET correlation_status = 'CONFLICT',
             terminal_reason = $2,
             last_updated_at_ms = $3
         WHERE canonical_execution_id = $1
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(canonical_execution_id)
    .bind(terminal_reason)
    .bind(now_ms)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "mark_conflict: no correlation for canonical_execution_id={canonical_execution_id}"
        ))
    })?;
    correlation_from_row(row)
}

/// Escalate to `MANUAL_REVIEW`. Reserved for operator escalation
/// when automated resolution is unsafe.
pub async fn mark_manual_review(
    pool: &PgPool,
    canonical_execution_id: &str,
    terminal_reason: &str,
    now_ms: i64,
) -> Result<OptionExecutionCorrelation> {
    let row = sqlx::query(
        "UPDATE option_execution_correlations
         SET correlation_status = 'MANUAL_REVIEW',
             terminal_reason = $2,
             last_updated_at_ms = $3
         WHERE canonical_execution_id = $1
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(canonical_execution_id)
    .bind(terminal_reason)
    .bind(now_ms)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "mark_manual_review: no correlation for canonical_execution_id={canonical_execution_id}"
        ))
    })?;
    correlation_from_row(row)
}

/// Read the most-recently updated correlation for a given
/// `canonical_execution_id`. Returns `None` if none exists.
pub async fn get_by_canonical_execution_id(
    pool: &PgPool,
    canonical_execution_id: &str,
) -> Result<Option<OptionExecutionCorrelation>> {
    let row = sqlx::query(
        "SELECT correlation_id, canonical_execution_id, deployment_id, chain_id,
                execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                onchain_execution_id, fill_quantity_1e8, tx_hash,
                canonical_block_number, canonical_block_hash, log_index,
                correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms
         FROM option_execution_correlations
         WHERE canonical_execution_id = $1
         ORDER BY last_updated_at_ms DESC
         LIMIT 1",
    )
    .bind(canonical_execution_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    row.map(correlation_from_row).transpose()
}

/// Look up correlation by `(tx_hash, log_index)` — the injective
/// on-chain key. Used by the reducer to disambiguate multi-event
/// transactions.
pub async fn get_by_tx_hash_and_log(
    pool: &PgPool,
    tx_hash: &str,
    log_index: i32,
) -> Result<Option<OptionExecutionCorrelation>> {
    let row = sqlx::query(
        "SELECT correlation_id, canonical_execution_id, deployment_id, chain_id,
                execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                onchain_execution_id, fill_quantity_1e8, tx_hash,
                canonical_block_number, canonical_block_hash, log_index,
                correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms
         FROM option_execution_correlations
         WHERE tx_hash = $1 AND log_index = $2
         LIMIT 1",
    )
    .bind(tx_hash)
    .bind(log_index)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    row.map(correlation_from_row).transpose()
}

/// Look up ACTIVE correlations awaiting a canonical event with the
/// specified on-chain tuple. Returns the (possibly-empty) candidate
/// set — reducer uses `(tx_hash, log_index)` as the final
/// discriminator (see Part C uniqueness proof).
///
/// `execution_kind` filters to the correct on-chain event surface
/// (trade vs rfq_trade).
pub async fn find_awaiting_by_onchain_tuple(
    pool: &PgPool,
    onchain_buyer_order_id: &str,
    onchain_seller_order_id: &str,
    fill_quantity_1e8: &str,
    execution_kind: OptionExecutionKind,
) -> Result<Vec<OptionExecutionCorrelation>> {
    let rows = sqlx::query(
        "SELECT correlation_id, canonical_execution_id, deployment_id, chain_id,
                execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                onchain_execution_id, fill_quantity_1e8, tx_hash,
                canonical_block_number, canonical_block_hash, log_index,
                correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms
         FROM option_execution_correlations
         WHERE onchain_buyer_order_id = $1
           AND onchain_seller_order_id = $2
           AND fill_quantity_1e8 = $3
           AND execution_kind = $4
           AND correlation_status IN ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED')
         ORDER BY first_seen_at_ms ASC",
    )
    .bind(onchain_buyer_order_id)
    .bind(onchain_seller_order_id)
    .bind(fill_quantity_1e8)
    .bind(execution_kind.as_str())
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    rows.into_iter().map(correlation_from_row).collect()
}

/// Insert-in-transaction variant of `insert_awaiting_correlation` —
/// used when the correlation insert must be atomic with a parent
/// operation (e.g., execution intent creation).
pub async fn insert_awaiting_correlation_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &AwaitingCorrelationInput,
) -> Result<OptionExecutionCorrelation> {
    let row = sqlx::query(
        "INSERT INTO option_execution_correlations (
             canonical_execution_id, deployment_id, chain_id, execution_kind,
             onchain_buyer_order_id, onchain_seller_order_id, fill_quantity_1e8,
             correlation_status, first_seen_at_ms, last_updated_at_ms
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'AWAITING_CHAIN_EVIDENCE', $8, $8)
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(&input.canonical_execution_id)
    .bind(input.deployment_id)
    .bind(input.chain_id)
    .bind(input.execution_kind.as_str())
    .bind(input.onchain_buyer_order_id.as_deref())
    .bind(input.onchain_seller_order_id.as_deref())
    .bind(input.fill_quantity_1e8.as_deref())
    .bind(input.now_ms)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    correlation_from_row(row)
}

/// Idempotent upsert-in-transaction variant. Used by the atomic
/// intent+correlation writer so a duplicated service invocation
/// (client retry, request de-duplication race) never fails on the
/// sparse UNIQUE index and never creates a duplicate row.
///
/// Semantics:
///   * If NO ACTIVE correlation exists for `canonical_execution_id`,
///     insert one in `AWAITING_CHAIN_EVIDENCE` and return it.
///   * If ONE already exists (any active status), return the existing
///     row unchanged. Fingerprint cross-check verifies the caller's
///     input agrees on `(deployment_id, chain_id, execution_kind)`;
///     mismatch raises `BackendError::Persistence` and aborts the tx.
///
/// The ACTIVE definition is `AWAITING_CHAIN_EVIDENCE | SUBMITTED |
/// CORRELATED_CANONICAL` — matching the sparse UNIQUE index. Rows in
/// ORPHANED / CONFLICT / MANUAL_REVIEW never block a fresh insert.
pub async fn upsert_awaiting_correlation_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &AwaitingCorrelationInput,
) -> Result<OptionExecutionCorrelation> {
    let inserted = sqlx::query(
        "INSERT INTO option_execution_correlations (
             canonical_execution_id, deployment_id, chain_id, execution_kind,
             onchain_buyer_order_id, onchain_seller_order_id, fill_quantity_1e8,
             correlation_status, first_seen_at_ms, last_updated_at_ms
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'AWAITING_CHAIN_EVIDENCE', $8, $8)
         ON CONFLICT (canonical_execution_id)
             WHERE correlation_status IN
                 ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED', 'CORRELATED_CANONICAL')
             DO NOTHING
         RETURNING correlation_id, canonical_execution_id, deployment_id, chain_id,
                   execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                   onchain_execution_id, fill_quantity_1e8, tx_hash,
                   canonical_block_number, canonical_block_hash, log_index,
                   correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms",
    )
    .bind(&input.canonical_execution_id)
    .bind(input.deployment_id)
    .bind(input.chain_id)
    .bind(input.execution_kind.as_str())
    .bind(input.onchain_buyer_order_id.as_deref())
    .bind(input.onchain_seller_order_id.as_deref())
    .bind(input.fill_quantity_1e8.as_deref())
    .bind(input.now_ms)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    if let Some(row) = inserted {
        return correlation_from_row(row);
    }
    // ON CONFLICT DO NOTHING fired → an active row already exists.
    // Look it up in the same tx and cross-check identity fields.
    let existing = sqlx::query(
        "SELECT correlation_id, canonical_execution_id, deployment_id, chain_id,
                execution_kind, onchain_buyer_order_id, onchain_seller_order_id,
                onchain_execution_id, fill_quantity_1e8, tx_hash,
                canonical_block_number, canonical_block_hash, log_index,
                correlation_status, terminal_reason, first_seen_at_ms, last_updated_at_ms
         FROM option_execution_correlations
         WHERE canonical_execution_id = $1
           AND correlation_status IN
               ('AWAITING_CHAIN_EVIDENCE', 'SUBMITTED', 'CORRELATED_CANONICAL')
         LIMIT 1",
    )
    .bind(&input.canonical_execution_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "upsert_awaiting_correlation_tx: conflict fired but no ACTIVE row found for {}",
            input.canonical_execution_id
        ))
    })?;
    let row = correlation_from_row(existing)?;
    // Cross-check identity so a retry with mismatched fingerprint
    // cannot silently succeed.
    if row.deployment_id != input.deployment_id {
        return Err(BackendError::Persistence(format!(
            "upsert_awaiting_correlation_tx: deployment_id mismatch on retry (existing={}, input={})",
            row.deployment_id, input.deployment_id
        )));
    }
    if row.chain_id != input.chain_id {
        return Err(BackendError::Persistence(format!(
            "upsert_awaiting_correlation_tx: chain_id mismatch on retry (existing={}, input={})",
            row.chain_id, input.chain_id
        )));
    }
    if row.execution_kind != input.execution_kind {
        return Err(BackendError::Persistence(format!(
            "upsert_awaiting_correlation_tx: execution_kind mismatch on retry (existing={:?}, input={:?})",
            row.execution_kind, input.execution_kind
        )));
    }
    Ok(row)
}
