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

/// Correlation lifecycle state (matches migrations 0055 + 0056 CHECK).
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OptionCorrelationStatus {
    AwaitingChainEvidence,
    /// OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Part A1 — the
    /// backend has computed the authoritative `tx_hash` from the
    /// exact signed raw transaction bytes and persisted it, but the
    /// `eth_sendRawTransaction` outcome is not yet known (either not
    /// invoked, in-flight, or the reply was lost). The tx MAY be on
    /// chain. The reducer can still bind canonical evidence via
    /// `(tx_hash, log_index)`.
    SubmissionUnknown,
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
            Self::SubmissionUnknown => "SUBMISSION_UNKNOWN",
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
            "SUBMISSION_UNKNOWN" => Ok(Self::SubmissionUnknown),
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
            Self::AwaitingChainEvidence
                | Self::SubmissionUnknown
                | Self::Submitted
                | Self::CorrelatedCanonical
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

/// Attach `tx_hash` to an ACTIVE correlation and transition to
/// `SUBMITTED` (RPC ack observed). Accepts `AWAITING_CHAIN_EVIDENCE`,
/// `SUBMISSION_UNKNOWN`, or an already-`SUBMITTED` source state.
/// Idempotent on same-value re-attachment. Immutability trigger
/// rejects different-value.
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
           AND correlation_status IN ('AWAITING_CHAIN_EVIDENCE', 'SUBMISSION_UNKNOWN', 'SUBMITTED')
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

/// OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Part A1 — persist the
/// authoritative tx_hash BEFORE calling `eth_sendRawTransaction`,
/// transitioning `AWAITING_CHAIN_EVIDENCE → SUBMISSION_UNKNOWN`.
///
/// Precondition: `tx_hash` MUST be derived deterministically from the
/// exact signed raw transaction bytes (see
/// `crate::execution::derive_signed_transaction_hash`). Once
/// persisted the tx_hash is immutable — if the RPC returns a
/// different value, that indicates provider/signer disagreement and
/// the caller MUST fail closed rather than overwriting.
///
/// Idempotent on same-value re-persist. Same-value from
/// SUBMISSION_UNKNOWN or SUBMITTED is a no-op (stays in whichever
/// forward state it already reached).
pub async fn attach_local_tx_identity(
    pool: &PgPool,
    canonical_execution_id: &str,
    tx_hash: &str,
    now_ms: i64,
) -> Result<OptionExecutionCorrelation> {
    let row = sqlx::query(
        "UPDATE option_execution_correlations
         SET tx_hash = $2,
             correlation_status = CASE
                 WHEN correlation_status = 'AWAITING_CHAIN_EVIDENCE' THEN 'SUBMISSION_UNKNOWN'
                 ELSE correlation_status
             END,
             last_updated_at_ms = $3
         WHERE canonical_execution_id = $1
           AND correlation_status IN
               ('AWAITING_CHAIN_EVIDENCE', 'SUBMISSION_UNKNOWN', 'SUBMITTED')
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
            "attach_local_tx_identity: no ACTIVE correlation for canonical_execution_id={canonical_execution_id} \
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
    // Note: reducer callers may pass empty strings for optional
    // fingerprint fields the current production event surface does
    // not emit (e.g. onchain_execution_id / onchain_buyer_order_id /
    // onchain_seller_order_id on the legacy OptionMatchingEngine
    // path). NULLIF degrades empty-string bindings to NULL so the
    // migration-0055 immutability trigger never locks in a
    // meaningless "" placeholder that would later reject genuine
    // envelope digests when a future contract upgrade starts
    // emitting them.
    let row = sqlx::query(
        "UPDATE option_execution_correlations
         SET correlation_status = 'CORRELATED_CANONICAL',
             tx_hash = COALESCE(tx_hash, NULLIF($2, '')),
             log_index = COALESCE(log_index, $3),
             canonical_block_number = COALESCE(canonical_block_number, $4),
             canonical_block_hash = COALESCE(canonical_block_hash, NULLIF($5, '')),
             onchain_execution_id = COALESCE(onchain_execution_id, NULLIF($6, '')),
             onchain_buyer_order_id = COALESCE(onchain_buyer_order_id, NULLIF($7, '')),
             onchain_seller_order_id = COALESCE(onchain_seller_order_id, NULLIF($8, '')),
             fill_quantity_1e8 = COALESCE(fill_quantity_1e8, NULLIF($9, '')),
             last_updated_at_ms = $10
         WHERE canonical_execution_id = $1
           AND correlation_status IN
               ('AWAITING_CHAIN_EVIDENCE', 'SUBMISSION_UNKNOWN', 'SUBMITTED', 'CORRELATED_CANONICAL')
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
           AND correlation_status IN
               ('AWAITING_CHAIN_EVIDENCE', 'SUBMISSION_UNKNOWN', 'SUBMITTED')
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

// -------------------------------------------------------------------
// Canonical event correlation reducer (Package A3–A6)
// -------------------------------------------------------------------

/// OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Part A5 — outcome of
/// running one canonical Options execution event through the
/// correlation reducer. Callers use this to distinguish idempotent
/// re-ingestion from state-changing promotions and from conflict
/// escalations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CorrelationReducerOutcome {
    /// No AWAITING/SUBMISSION_UNKNOWN/SUBMITTED correlation row was
    /// found for this event. The event references an intent that was
    /// created before Part E landed (or by a path that does not
    /// derive a canonical execution identity). The event is still
    /// persisted; no correlation update happens. Not a conflict.
    NoCorrelationForIntent,
    /// The correlation row was promoted to `CORRELATED_CANONICAL`.
    Promoted(OptionExecutionCorrelation),
    /// The correlation was already `CORRELATED_CANONICAL` for the same
    /// `(tx_hash, log_index)` — idempotent replay. Returns the
    /// unchanged row.
    AlreadyCorrelated(OptionExecutionCorrelation),
    /// Economic fingerprint mismatch — the row was escalated to
    /// `CONFLICT`. Returns the conflicted row.
    Conflict(OptionExecutionCorrelation),
}

/// Immutable identity fields the reducer cross-checks between the
/// pre-persisted correlation + intent and the canonical event.
/// Callers construct this from the decoded `OptionExecutionEvent` +
/// the intent it links to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalExecutionEventInput<'a> {
    pub canonical_execution_id: &'a str,
    pub execution_kind: OptionExecutionKind,
    pub tx_hash: &'a str,
    pub log_index: i32,
    pub canonical_block_number: i64,
    pub canonical_block_hash: &'a str,
    pub onchain_execution_id: Option<&'a str>,
    pub onchain_buyer_order_id: Option<&'a str>,
    pub onchain_seller_order_id: Option<&'a str>,
    pub fill_quantity_1e8: &'a str,
    pub now_ms: i64,
}

/// OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Parts A3+A4+A5+A6 —
/// promote a pre-persisted correlation row to
/// `CORRELATED_CANONICAL` using canonical event evidence. Enforces:
///
///   * `execution_kind` must match the event surface. Mismatch
///     between orderbook and RFQ event → CONFLICT (an intent for
///     `executeTrade` cannot correlate against
///     `OptionRfqTradeExecuted` or vice versa).
///   * If the correlation already has ANY on-chain fingerprint set
///     (from a prior reducer pass or a Part D pre-chain population),
///     the new evidence must AGREE. Mismatch → CONFLICT.
///   * `fill_quantity_1e8` must match if pre-populated.
///   * Idempotent replay: the same `(tx_hash, log_index)` on an
///     already-CORRELATED_CANONICAL row returns `AlreadyCorrelated`
///     without mutation.
///
/// The reducer NEVER creates a correlation row on-demand — callers
/// must have inserted the AWAITING row atomically with the intent
/// (Part E). Events for intents without a correlation row return
/// `NoCorrelationForIntent`.
pub async fn correlate_canonical_option_event(
    pool: &PgPool,
    input: &CanonicalExecutionEventInput<'_>,
) -> Result<CorrelationReducerOutcome> {
    let existing = get_by_canonical_execution_id(pool, input.canonical_execution_id).await?;
    let Some(row) = existing else {
        return Ok(CorrelationReducerOutcome::NoCorrelationForIntent);
    };
    // Execution kind must match — orderbook and RFQ intents are
    // disjoint by user-signed EIP-712 type.
    if row.execution_kind != input.execution_kind {
        let terminal = format!(
            "canonical event execution_kind={:?} does not match correlation execution_kind={:?}",
            input.execution_kind, row.execution_kind
        );
        let row =
            mark_conflict(pool, input.canonical_execution_id, &terminal, input.now_ms).await?;
        return Ok(CorrelationReducerOutcome::Conflict(row));
    }
    // Idempotent replay: already CORRELATED_CANONICAL with matching
    // (tx_hash, log_index) → no-op success.
    if row.correlation_status == OptionCorrelationStatus::CorrelatedCanonical {
        if row.tx_hash.as_deref() == Some(input.tx_hash) && row.log_index == Some(input.log_index) {
            return Ok(CorrelationReducerOutcome::AlreadyCorrelated(row));
        }
        // Same canonical id but different tx_hash/log_index → CONFLICT
        // (a second canonical event claims the same execution).
        let terminal = format!(
            "second canonical event {}:{} conflicts with existing tx {}:{:?}",
            input.tx_hash,
            input.log_index,
            row.tx_hash.as_deref().unwrap_or("-"),
            row.log_index
        );
        let row =
            mark_conflict(pool, input.canonical_execution_id, &terminal, input.now_ms).await?;
        return Ok(CorrelationReducerOutcome::Conflict(row));
    }
    // Fingerprint cross-checks against pre-populated values. Anything
    // populated MUST match; anything NULL is populated on promotion.
    if let Some(existing_tx) = row.tx_hash.as_deref() {
        if existing_tx != input.tx_hash {
            let terminal = format!(
                "canonical event tx_hash={} disagrees with pre-persisted {}",
                input.tx_hash, existing_tx
            );
            let row =
                mark_conflict(pool, input.canonical_execution_id, &terminal, input.now_ms).await?;
            return Ok(CorrelationReducerOutcome::Conflict(row));
        }
    }
    if let Some(existing_quantity) = row.fill_quantity_1e8.as_deref() {
        if existing_quantity != input.fill_quantity_1e8 {
            let terminal = format!(
                "canonical event fill_quantity_1e8={} disagrees with pre-persisted {}",
                input.fill_quantity_1e8, existing_quantity
            );
            let row =
                mark_conflict(pool, input.canonical_execution_id, &terminal, input.now_ms).await?;
            return Ok(CorrelationReducerOutcome::Conflict(row));
        }
    }
    if let (Some(existing_buyer), Some(new_buyer)) = (
        row.onchain_buyer_order_id.as_deref(),
        input.onchain_buyer_order_id,
    ) {
        if existing_buyer != new_buyer {
            let terminal = format!(
                "canonical event onchain_buyer_order_id={} disagrees with pre-persisted {}",
                new_buyer, existing_buyer
            );
            let row =
                mark_conflict(pool, input.canonical_execution_id, &terminal, input.now_ms).await?;
            return Ok(CorrelationReducerOutcome::Conflict(row));
        }
    }
    if let (Some(existing_seller), Some(new_seller)) = (
        row.onchain_seller_order_id.as_deref(),
        input.onchain_seller_order_id,
    ) {
        if existing_seller != new_seller {
            let terminal = format!(
                "canonical event onchain_seller_order_id={} disagrees with pre-persisted {}",
                new_seller, existing_seller
            );
            let row =
                mark_conflict(pool, input.canonical_execution_id, &terminal, input.now_ms).await?;
            return Ok(CorrelationReducerOutcome::Conflict(row));
        }
    }
    // All cross-checks passed — promote to CORRELATED_CANONICAL with
    // full fingerprint.
    let fp = CanonicalEventFingerprint {
        tx_hash: input.tx_hash.to_string(),
        log_index: input.log_index,
        canonical_block_number: input.canonical_block_number,
        canonical_block_hash: input.canonical_block_hash.to_string(),
        onchain_execution_id: input.onchain_execution_id.unwrap_or("").to_string(),
        onchain_buyer_order_id: input.onchain_buyer_order_id.unwrap_or("").to_string(),
        onchain_seller_order_id: input.onchain_seller_order_id.unwrap_or("").to_string(),
        fill_quantity_1e8: input.fill_quantity_1e8.to_string(),
        now_ms: input.now_ms,
    };
    let promoted = mark_correlated_canonical(pool, input.canonical_execution_id, &fp).await?;
    Ok(CorrelationReducerOutcome::Promoted(promoted))
}

/// OPTIONS-HYBRID-V2-BACKEND-CLOSURE-SPRINT-V1 Part A6 — canonical
/// branch reorg: the previously-canonical event no longer sits on
/// the canonical chain. Transitions `CORRELATED_CANONICAL →
/// ORPHANED`. The `canonical_execution_id` remains immutable so a
/// replacement canonical event on the successor branch can promote a
/// fresh `AWAITING → CORRELATED_CANONICAL` cycle (sparse UNIQUE is
/// scoped to active states, ORPHANED does not block re-insertion).
pub async fn reorg_orphan_canonical_correlation(
    pool: &PgPool,
    canonical_execution_id: &str,
    now_ms: i64,
) -> Result<OptionExecutionCorrelation> {
    let terminal = "canonical branch reorg — evidence orphaned".to_string();
    mark_orphaned(pool, canonical_execution_id, &terminal, now_ms).await
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
                 ('AWAITING_CHAIN_EVIDENCE', 'SUBMISSION_UNKNOWN', 'SUBMITTED', 'CORRELATED_CANONICAL')
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
               ('AWAITING_CHAIN_EVIDENCE', 'SUBMISSION_UNKNOWN', 'SUBMITTED', 'CORRELATED_CANONICAL')
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
