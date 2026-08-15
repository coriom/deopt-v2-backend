//! OPTIONS-HYBRID-V2-RESERVATIONS-PENDING-SETTLEMENT-AND-CANONICAL-RELEASE-V1
//! Package B — repository for `option_reservations` (migration 0057).
//!
//! Off-chain Options risk ledger, distinct from
//! `hybrid_v2_reservations` (which is a canonical on-chain projection
//! populated exclusively from chain events).
//!
//! Two purposes:
//!   * `OPEN_ORDER` — collateral held while an order is resting.
//!   * `PENDING_SETTLEMENT` — collateral held from match commit until
//!     canonical Options execution event lands (correlation reducer
//!     `Promoted` → SETTLED transition).
//!
//! State machine:
//!
//! ```text
//!  ACTIVE
//!    │  release_open_order (cancel / expire / IOC unmatched)
//!    ▼
//!  RELEASED
//!
//!  ACTIVE                            (OPEN_ORDER on match)
//!    │  convert_open_order_to_pending
//!    ▼
//!  CONVERTED                         (OPEN_ORDER terminal after match)
//!
//!  [insert]                          (PENDING_SETTLEMENT from match)
//!    │
//!  ACTIVE                            (PENDING_SETTLEMENT waiting)
//!    │  settle_pending (canonical event correlated)
//!    ▼
//!  SETTLED
//!
//!  ACTIVE / CONVERTED / SETTLED
//!    │  mark_manual_review          (reorg cannot safely reactivate)
//!    ▼
//!  MANUAL_REVIEW
//! ```
//!
//! All transitions are explicit named methods. Immutability trigger
//! from migration 0057 rejects mutation of identity fields.

use crate::error::{BackendError, Result};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::str::FromStr;
use uuid::Uuid;

// -------------------------------------------------------------------
// Types
// -------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OptionReservationPurpose {
    OpenOrder,
    PendingSettlement,
}

impl OptionReservationPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenOrder => "OPEN_ORDER",
            Self::PendingSettlement => "PENDING_SETTLEMENT",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "OPEN_ORDER" => Ok(Self::OpenOrder),
            "PENDING_SETTLEMENT" => Ok(Self::PendingSettlement),
            other => Err(BackendError::Persistence(format!(
                "invalid option_reservation purpose: {other}"
            ))),
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OptionReservationStatus {
    Active,
    Converted,
    Released,
    Settled,
    ManualReview,
}

impl OptionReservationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Converted => "CONVERTED",
            Self::Released => "RELEASED",
            Self::Settled => "SETTLED",
            Self::ManualReview => "MANUAL_REVIEW",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "CONVERTED" => Ok(Self::Converted),
            "RELEASED" => Ok(Self::Released),
            "SETTLED" => Ok(Self::Settled),
            "MANUAL_REVIEW" => Ok(Self::ManualReview),
            other => Err(BackendError::Persistence(format!(
                "invalid option_reservation status: {other}"
            ))),
        }
    }
    /// A reservation contributes to available-collateral deduction
    /// only when ACTIVE. Terminal states are audit-only.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptionReservationSide {
    Buy,
    Sell,
}

impl OptionReservationSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "buy" => Ok(Self::Buy),
            "sell" => Ok(Self::Sell),
            other => Err(BackendError::Persistence(format!(
                "invalid option_reservation side: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptionReservation {
    pub reservation_id: Uuid,
    pub purpose: OptionReservationPurpose,
    pub deployment_id: i64,
    pub chain_id: i64,
    pub owner: String,
    pub subaccount_id: i32,
    pub collateral_token: String,
    pub canonical_order_hash: Option<String>,
    pub canonical_execution_id: Option<String>,
    pub option_series_id: String,
    pub side: OptionReservationSide,
    pub reserved_amount: String,
    pub quantity_1e8: String,
    pub status: OptionReservationStatus,
    pub terminal_reason: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Input for creating an OPEN_ORDER reservation atomically with a new
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenOrderReservationInput {
    pub deployment_id: i64,
    pub chain_id: i64,
    pub owner: String,
    pub subaccount_id: i32,
    pub collateral_token: String,
    pub canonical_order_hash: String,
    pub option_series_id: String,
    pub side: OptionReservationSide,
    pub reserved_amount: String,
    pub quantity_1e8: String,
    pub now_ms: i64,
}

/// Input for creating a PENDING_SETTLEMENT reservation atomically with
/// a match commit. One per side per execution (buyer + seller).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSettlementReservationInput {
    pub deployment_id: i64,
    pub chain_id: i64,
    pub owner: String,
    pub subaccount_id: i32,
    pub collateral_token: String,
    pub canonical_execution_id: String,
    pub option_series_id: String,
    pub side: OptionReservationSide,
    pub reserved_amount: String,
    pub quantity_1e8: String,
    pub now_ms: i64,
}

// -------------------------------------------------------------------
// Row hydration
// -------------------------------------------------------------------

fn reservation_from_row(row: PgRow) -> Result<OptionReservation> {
    let purpose_str: String = row_get(&row, "purpose")?;
    let status_str: String = row_get(&row, "status")?;
    let side_str: String = row_get(&row, "side")?;
    Ok(OptionReservation {
        reservation_id: row_get(&row, "reservation_id")?,
        purpose: OptionReservationPurpose::parse(&purpose_str)?,
        deployment_id: row_get(&row, "deployment_id")?,
        chain_id: row_get(&row, "chain_id")?,
        owner: row_get(&row, "owner")?,
        subaccount_id: row_get(&row, "subaccount_id")?,
        collateral_token: row_get(&row, "collateral_token")?,
        canonical_order_hash: row_get(&row, "canonical_order_hash")?,
        canonical_execution_id: row_get(&row, "canonical_execution_id")?,
        option_series_id: row_get(&row, "option_series_id")?,
        side: OptionReservationSide::parse(&side_str)?,
        reserved_amount: row_get(&row, "reserved_amount")?,
        quantity_1e8: row_get(&row, "quantity_1e8")?,
        status: OptionReservationStatus::parse(&status_str)?,
        terminal_reason: row_get(&row, "terminal_reason")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn row_get<T>(row: &PgRow, column: &str) -> Result<T>
where
    for<'r> T: sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|e| BackendError::Persistence(e.to_string()))
}

const SELECT_COLUMNS: &str = "reservation_id, purpose, deployment_id, chain_id, owner,
    subaccount_id, collateral_token, canonical_order_hash, canonical_execution_id,
    option_series_id, side, reserved_amount, quantity_1e8, status, terminal_reason,
    created_at_ms, updated_at_ms";

// -------------------------------------------------------------------
// Insert helpers (tx-scoped — always atomic with parent operation)
// -------------------------------------------------------------------

/// Insert an OPEN_ORDER reservation in an existing transaction.
/// Idempotent on retry via ON CONFLICT DO NOTHING against the sparse
/// UNIQUE on `canonical_order_hash`.
pub async fn insert_open_order_reservation_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &OpenOrderReservationInput,
) -> Result<OptionReservation> {
    let inserted = sqlx::query(&format!(
        "INSERT INTO option_reservations (
             purpose, deployment_id, chain_id, owner, subaccount_id,
             collateral_token, canonical_order_hash, canonical_execution_id,
             option_series_id, side, reserved_amount, quantity_1e8,
             status, created_at_ms, updated_at_ms
         ) VALUES (
             'OPEN_ORDER', $1, $2, $3, $4, $5, $6, NULL, $7, $8, $9, $10, 'ACTIVE', $11, $11
         )
         ON CONFLICT (canonical_order_hash)
             WHERE status = 'ACTIVE' AND purpose = 'OPEN_ORDER'
             DO NOTHING
         RETURNING {SELECT_COLUMNS}"
    ))
    .bind(input.deployment_id)
    .bind(input.chain_id)
    .bind(&input.owner)
    .bind(input.subaccount_id)
    .bind(&input.collateral_token)
    .bind(&input.canonical_order_hash)
    .bind(&input.option_series_id)
    .bind(input.side.as_str())
    .bind(&input.reserved_amount)
    .bind(&input.quantity_1e8)
    .bind(input.now_ms)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    if let Some(row) = inserted {
        return reservation_from_row(row);
    }
    // ON CONFLICT DO NOTHING fired: an ACTIVE row already exists.
    // Return it (idempotent retry).
    let existing = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM option_reservations
         WHERE canonical_order_hash = $1
           AND status = 'ACTIVE'
           AND purpose = 'OPEN_ORDER'
         LIMIT 1"
    ))
    .bind(&input.canonical_order_hash)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "insert_open_order_reservation_tx: conflict but no ACTIVE row for {}",
            input.canonical_order_hash
        ))
    })?;
    let existing = reservation_from_row(existing)?;
    // Identity cross-check on retry.
    if existing.deployment_id != input.deployment_id
        || existing.chain_id != input.chain_id
        || existing.owner != input.owner
        || existing.subaccount_id != input.subaccount_id
        || existing.collateral_token != input.collateral_token
    {
        return Err(BackendError::Persistence(format!(
            "insert_open_order_reservation_tx: retry identity mismatch for {}",
            input.canonical_order_hash
        )));
    }
    Ok(existing)
}

/// Pool-level convenience wrapper opening its own transaction.
pub async fn insert_open_order_reservation(
    pool: &PgPool,
    input: &OpenOrderReservationInput,
) -> Result<OptionReservation> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    let row = insert_open_order_reservation_tx(&mut tx, input).await?;
    tx.commit()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    Ok(row)
}

/// Insert a PENDING_SETTLEMENT reservation in an existing transaction.
/// The sparse UNIQUE targets `(canonical_execution_id, owner,
/// subaccount_id, collateral_token)` so the buyer + seller of the same
/// canonical execution get distinct rows.
pub async fn insert_pending_settlement_reservation_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &PendingSettlementReservationInput,
) -> Result<OptionReservation> {
    let inserted = sqlx::query(&format!(
        "INSERT INTO option_reservations (
             purpose, deployment_id, chain_id, owner, subaccount_id,
             collateral_token, canonical_order_hash, canonical_execution_id,
             option_series_id, side, reserved_amount, quantity_1e8,
             status, created_at_ms, updated_at_ms
         ) VALUES (
             'PENDING_SETTLEMENT', $1, $2, $3, $4, $5, NULL, $6, $7, $8, $9, $10, 'ACTIVE', $11, $11
         )
         ON CONFLICT (canonical_execution_id, owner, subaccount_id, collateral_token)
             WHERE status = 'ACTIVE' AND purpose = 'PENDING_SETTLEMENT'
             DO NOTHING
         RETURNING {SELECT_COLUMNS}"
    ))
    .bind(input.deployment_id)
    .bind(input.chain_id)
    .bind(&input.owner)
    .bind(input.subaccount_id)
    .bind(&input.collateral_token)
    .bind(&input.canonical_execution_id)
    .bind(&input.option_series_id)
    .bind(input.side.as_str())
    .bind(&input.reserved_amount)
    .bind(&input.quantity_1e8)
    .bind(input.now_ms)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    if let Some(row) = inserted {
        return reservation_from_row(row);
    }
    let existing = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM option_reservations
         WHERE canonical_execution_id = $1
           AND owner = $2
           AND subaccount_id = $3
           AND collateral_token = $4
           AND status = 'ACTIVE'
           AND purpose = 'PENDING_SETTLEMENT'
         LIMIT 1"
    ))
    .bind(&input.canonical_execution_id)
    .bind(&input.owner)
    .bind(input.subaccount_id)
    .bind(&input.collateral_token)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "insert_pending_settlement_reservation_tx: conflict but no ACTIVE row for {}",
            input.canonical_execution_id
        ))
    })?;
    reservation_from_row(existing)
}

// -------------------------------------------------------------------
// State transitions (explicit named)
// -------------------------------------------------------------------

/// OPEN_ORDER → RELEASED. Used on cancellation, expiry, IOC/FOK
/// unmatched residual, or post-only cross rejection. Returns None if
/// no ACTIVE OPEN_ORDER row exists for the given `canonical_order_hash`.
pub async fn release_open_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    canonical_order_hash: &str,
    terminal_reason: &str,
    now_ms: i64,
) -> Result<Option<OptionReservation>> {
    let row = sqlx::query(&format!(
        "UPDATE option_reservations
         SET status = 'RELEASED',
             terminal_reason = $2,
             updated_at_ms = $3
         WHERE canonical_order_hash = $1
           AND status = 'ACTIVE'
           AND purpose = 'OPEN_ORDER'
         RETURNING {SELECT_COLUMNS}"
    ))
    .bind(canonical_order_hash)
    .bind(terminal_reason)
    .bind(now_ms)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    row.map(reservation_from_row).transpose()
}

/// Pool-level convenience for release.
pub async fn release_open_order(
    pool: &PgPool,
    canonical_order_hash: &str,
    terminal_reason: &str,
    now_ms: i64,
) -> Result<Option<OptionReservation>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    let row = release_open_order_tx(&mut tx, canonical_order_hash, terminal_reason, now_ms).await?;
    tx.commit()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    Ok(row)
}

/// OPEN_ORDER → CONVERTED. Called inside the matcher transaction when
/// a match consumes an ACTIVE OPEN_ORDER row (partial or full).
/// Returns None if no ACTIVE row exists.
pub async fn mark_open_order_converted_tx(
    tx: &mut Transaction<'_, Postgres>,
    canonical_order_hash: &str,
    now_ms: i64,
) -> Result<Option<OptionReservation>> {
    let row = sqlx::query(&format!(
        "UPDATE option_reservations
         SET status = 'CONVERTED',
             terminal_reason = 'MATCHED_TO_PENDING_SETTLEMENT',
             updated_at_ms = $2
         WHERE canonical_order_hash = $1
           AND status = 'ACTIVE'
           AND purpose = 'OPEN_ORDER'
         RETURNING {SELECT_COLUMNS}"
    ))
    .bind(canonical_order_hash)
    .bind(now_ms)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    row.map(reservation_from_row).transpose()
}

/// PENDING_SETTLEMENT → SETTLED. Called by the canonical event
/// reducer's `Promoted` outcome to release the exposure after
/// on-chain settlement.
pub async fn settle_pending_tx(
    tx: &mut Transaction<'_, Postgres>,
    canonical_execution_id: &str,
    now_ms: i64,
) -> Result<Vec<OptionReservation>> {
    let rows = sqlx::query(&format!(
        "UPDATE option_reservations
         SET status = 'SETTLED',
             terminal_reason = 'CANONICAL_SETTLEMENT',
             updated_at_ms = $2
         WHERE canonical_execution_id = $1
           AND status = 'ACTIVE'
           AND purpose = 'PENDING_SETTLEMENT'
         RETURNING {SELECT_COLUMNS}"
    ))
    .bind(canonical_execution_id)
    .bind(now_ms)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    rows.into_iter().map(reservation_from_row).collect()
}

/// Pool-level convenience for canonical settlement release.
pub async fn settle_pending(
    pool: &PgPool,
    canonical_execution_id: &str,
    now_ms: i64,
) -> Result<Vec<OptionReservation>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    let rows = settle_pending_tx(&mut tx, canonical_execution_id, now_ms).await?;
    tx.commit()
        .await
        .map_err(|e| BackendError::Persistence(e.to_string()))?;
    Ok(rows)
}

/// Any state → MANUAL_REVIEW. Reserved for operator escalation
/// (e.g., reorg that cannot safely reactivate the required hold).
pub async fn mark_manual_review_tx(
    tx: &mut Transaction<'_, Postgres>,
    reservation_id: Uuid,
    terminal_reason: &str,
    now_ms: i64,
) -> Result<OptionReservation> {
    let row = sqlx::query(&format!(
        "UPDATE option_reservations
         SET status = 'MANUAL_REVIEW',
             terminal_reason = $2,
             updated_at_ms = $3
         WHERE reservation_id = $1
         RETURNING {SELECT_COLUMNS}"
    ))
    .bind(reservation_id)
    .bind(terminal_reason)
    .bind(now_ms)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?
    .ok_or_else(|| {
        BackendError::Persistence(format!(
            "mark_manual_review_tx: no row for reservation_id={reservation_id}"
        ))
    })?;
    reservation_from_row(row)
}

// -------------------------------------------------------------------
// Read paths
// -------------------------------------------------------------------

/// Sum of ACTIVE reservations for the exact
/// `(deployment_id, owner, subaccount_id, collateral_token)` scope.
/// Returns a `u128` in the token's native fixed-point units. Overflow
/// (>2^128 total reserved) returns an error rather than saturating —
/// this signals a schema-scale bug that must be investigated.
///
/// Used by the available-collateral read path.
pub async fn total_active_reserved(
    pool: &PgPool,
    deployment_id: i64,
    owner: &str,
    subaccount_id: i32,
    collateral_token: &str,
) -> Result<u128> {
    let rows = sqlx::query(
        "SELECT reserved_amount FROM option_reservations
         WHERE deployment_id = $1
           AND owner = $2
           AND subaccount_id = $3
           AND collateral_token = $4
           AND status = 'ACTIVE'",
    )
    .bind(deployment_id)
    .bind(owner)
    .bind(subaccount_id)
    .bind(collateral_token)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    let mut total: u128 = 0;
    for row in rows {
        let amount_str: String = row
            .try_get::<String, _>("reserved_amount")
            .map_err(|e| BackendError::Persistence(e.to_string()))?;
        let amount = u128::from_str(&amount_str).map_err(|e| {
            BackendError::Persistence(format!("invalid reserved_amount {amount_str}: {e}"))
        })?;
        total = total.checked_add(amount).ok_or_else(|| {
            BackendError::Persistence("total_active_reserved: sum overflowed u128".to_string())
        })?;
    }
    Ok(total)
}

/// Look up a reservation by primary key (audit / admin lookups).
pub async fn get_reservation(
    pool: &PgPool,
    reservation_id: Uuid,
) -> Result<Option<OptionReservation>> {
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM option_reservations WHERE reservation_id = $1"
    ))
    .bind(reservation_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    row.map(reservation_from_row).transpose()
}

/// Look up the active OPEN_ORDER row for a canonical_order_hash (used
/// by cancellation to know which row to release).
pub async fn get_active_open_order(
    pool: &PgPool,
    canonical_order_hash: &str,
) -> Result<Option<OptionReservation>> {
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM option_reservations
         WHERE canonical_order_hash = $1
           AND status = 'ACTIVE'
           AND purpose = 'OPEN_ORDER'
         LIMIT 1"
    ))
    .bind(canonical_order_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    row.map(reservation_from_row).transpose()
}

/// Compute available Options collateral for the exact
/// `(deployment_id, owner, subaccount_id, collateral_token)` scope.
///
/// Semantics:
///
/// ```text
/// available_option_collateral
///   =
/// canonical_collateral        (caller-supplied, from HV2 projection)
///   -
/// option_reservations.total_active_reserved  (this ledger)
/// ```
///
/// `canonical_collateral` MUST be the full canonical projection value
/// for the (deployment, owner, subaccount, token) — the caller reads
/// it from `hybrid_v2_vault_balances` minus any already-known
/// canonical `hybrid_v2_reservations` for OTHER engines (perps, RFQ,
/// etc.) so the returned value reflects what remains available to
/// the Options engine specifically.
///
/// Underflow (option reservations exceed available canonical
/// collateral) is treated as a hard error rather than saturating to
/// zero: it signals an accounting invariant violation that must be
/// investigated (e.g. a stale reservation the engine failed to
/// release).
///
/// This function is deployment/chain/owner/subaccount/token scoped.
/// No cross-subaccount netting. No cross-token netting.
pub async fn available_option_collateral(
    pool: &PgPool,
    deployment_id: i64,
    owner: &str,
    subaccount_id: i32,
    collateral_token: &str,
    canonical_collateral: u128,
) -> Result<u128> {
    let reserved =
        total_active_reserved(pool, deployment_id, owner, subaccount_id, collateral_token).await?;
    canonical_collateral.checked_sub(reserved).ok_or_else(|| {
        BackendError::Persistence(format!(
            "available_option_collateral underflow: reserved={reserved} > canonical={canonical_collateral} \
             (deployment={deployment_id}, subaccount={subaccount_id}, token={collateral_token})"
        ))
    })
}

/// Check that a required amount can be reserved atop existing active
/// holds. Returns Ok(()) if sufficient, or `BackendError` describing
/// the shortfall.
///
/// Used by the atomic order-acceptance path as the pre-persist check.
pub async fn ensure_option_collateral_available(
    pool: &PgPool,
    deployment_id: i64,
    owner: &str,
    subaccount_id: i32,
    collateral_token: &str,
    canonical_collateral: u128,
    required_amount: u128,
) -> Result<()> {
    let available = available_option_collateral(
        pool,
        deployment_id,
        owner,
        subaccount_id,
        collateral_token,
        canonical_collateral,
    )
    .await?;
    if available < required_amount {
        return Err(BackendError::InvalidOptionOrderState(format!(
            "insufficient option collateral: available={available} < required={required_amount} \
             (owner={owner}, subaccount={subaccount_id}, token={collateral_token})"
        )));
    }
    Ok(())
}

/// List active PENDING_SETTLEMENT rows for a canonical_execution_id
/// (used by the settlement reducer to know what to release; there may
/// be up to two rows — one per counterparty).
pub async fn list_active_pending_for_execution(
    pool: &PgPool,
    canonical_execution_id: &str,
) -> Result<Vec<OptionReservation>> {
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM option_reservations
         WHERE canonical_execution_id = $1
           AND status = 'ACTIVE'
           AND purpose = 'PENDING_SETTLEMENT'
         ORDER BY owner, subaccount_id"
    ))
    .bind(canonical_execution_id)
    .fetch_all(pool)
    .await
    .map_err(|e| BackendError::Persistence(e.to_string()))?;
    rows.into_iter().map(reservation_from_row).collect()
}
