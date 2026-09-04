//! PERPS-CLOSED-TEST-HARDENING-V1 Part B — cumulative-fill ledger for
//! signed `PerpOrderIntent` submissions.
//!
//! ## Purpose
//!
//! The Solidity `PerpMatchingEngine.executeTradeFromIntents` already
//! enforces `filled + size <= intent.size1e8` on-chain via the
//! `intentFilled[intentHash]` mapping. But the backend closed-test PG
//! path never broadcasts on chain (V1 is closed-test-only), so that
//! on-chain invariant is not consulted for the internal engine's fills.
//! This ledger closes the same accounting gap in the backend: every
//! signed intent gets a row on first submit; every downstream fill is
//! atomically added inside a `SELECT ... FOR UPDATE` transaction and
//! rejected if it would exceed the signed size.
//!
//! ## Two-layer accounting
//!
//!   * Layer A — `perps_signed_intent_nonce_ledger` prevents
//!     `(trader, nonce)` REPLAY at submission time.
//!   * Layer B — this module prevents CUMULATIVE OVERFILL of an
//!     already-consumed intent, even across restarts. Guards against a
//!     matching-logic bug that could double-count fills.
//!
//! ## Fail-closed
//!
//!   * Any DB error → [`BackendError::Persistence`] (503). Silent
//!     success on uncertain writes is a critical bug.
//!   * Overfill → [`BackendError::PerpsIntentCumulativeOverfill`]
//!     (500 — this is a matching-logic bug, not user error).
//!   * Same intent_hash re-registered under a different `signed_size`
//!     → [`BackendError::PerpsIntentCumulativeOverfill`] (500 — the
//!     intent hash is content-addressed; two rows with the same key
//!     and different sizes cannot both be valid).
//!
//! See `migrations/0060_perps_intent_fills_ledger.sql` for the table
//! shape.

use crate::db::PgRepository;
use crate::error::{BackendError, Result};
use crate::signing::eip712::parse_evm_address;
use crate::types::AccountId;

/// PG-backed cumulative-fill ledger. Wraps [`PgRepository`].
///
/// Every method is atomic:
///
///   * [`Self::record_intent`] uses `INSERT ... ON CONFLICT DO NOTHING`
///     + a follow-up read: if a row already exists with a different
///     `signed_size_1e8`, we reject with `PerpsIntentCumulativeOverfill`
///     (content-addressed intent hashes cannot carry two sizes).
///   * [`Self::try_add_fill`] runs inside a transaction with
///     `SELECT ... FOR UPDATE` to serialise concurrent adders on the
///     same intent hash. If the update would push `filled_size_1e8`
///     past `signed_size_1e8` we roll back and return
///     `PerpsIntentCumulativeOverfill`.
///
/// The ledger has NO in-memory cache — the correctness cost outweighs
/// the latency saving in closed-test scope.
#[derive(Clone)]
pub struct PgIntentFillsLedger {
    repository: PgRepository,
}

impl PgIntentFillsLedger {
    pub fn new(repository: PgRepository) -> Self {
        Self { repository }
    }

    /// Idempotent registration of an intent. First call INSERTs a row
    /// with `filled_size_1e8 = initial_filled`. Subsequent calls with
    /// the SAME `signed_size_1e8` no-op. A conflict on
    /// `signed_size_1e8` (same hash, different declared size) is a
    /// critical inconsistency and returns
    /// [`BackendError::PerpsIntentCumulativeOverfill`].
    ///
    /// `initial_filled` is normally `0`; the parameter exists so tests
    /// can seed a partially-filled state directly.
    pub async fn record_intent(
        &self,
        intent_hash: [u8; 32],
        trader: &AccountId,
        signed_size_1e8: u128,
        initial_filled_1e8: u128,
        now_ms: i64,
    ) -> Result<()> {
        if initial_filled_1e8 > signed_size_1e8 {
            return Err(BackendError::PerpsIntentCumulativeOverfill(format!(
                "initial_filled {initial_filled_1e8} > signed_size {signed_size_1e8}"
            )));
        }
        let trader_bytes = parse_evm_address(trader).map_err(|_| {
            BackendError::Persistence("invalid trader address for intent fills ledger".to_string())
        })?;
        let signed_size_str = signed_size_1e8.to_string();
        let filled_size_str = initial_filled_1e8.to_string();
        // ON CONFLICT DO NOTHING; then verify the existing row matches
        // the declared signed_size. We cannot rely on RETURNING alone
        // to detect the "same hash, different size" case because the
        // conflict path silently skips the insert.
        sqlx::query(
            "INSERT INTO perps_intent_fills_ledger \
             (intent_hash, trader, signed_size_1e8, filled_size_1e8, last_updated_ms) \
             VALUES ($1, $2, $3::NUMERIC, $4::NUMERIC, $5) \
             ON CONFLICT (intent_hash) DO NOTHING",
        )
        .bind(intent_hash.as_slice())
        .bind(trader_bytes.as_slice())
        .bind(&signed_size_str)
        .bind(&filled_size_str)
        .bind(now_ms)
        .execute(self.repository.pool())
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;

        // Verify the on-row signed_size matches our declared value.
        // Two cases:
        //   (a) our INSERT succeeded — the read returns our value.
        //   (b) our INSERT was suppressed by ON CONFLICT — the read
        //       returns the pre-existing value; if it differs from
        //       ours, error out.
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT signed_size_1e8::TEXT FROM perps_intent_fills_ledger \
             WHERE intent_hash = $1 LIMIT 1",
        )
        .bind(intent_hash.as_slice())
        .fetch_optional(self.repository.pool())
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;
        let existing_size = existing
            .ok_or_else(|| {
                BackendError::Persistence(
                    "intent fills ledger row disappeared after insert".to_string(),
                )
            })?
            .0
            .parse::<u128>()
            .map_err(|err| {
                BackendError::Persistence(format!(
                    "intent fills ledger signed_size unparseable: {err}"
                ))
            })?;
        if existing_size != signed_size_1e8 {
            return Err(BackendError::PerpsIntentCumulativeOverfill(format!(
                "intent hash already registered with signed_size {existing_size}; \
                 request declared {signed_size_1e8}"
            )));
        }
        Ok(())
    }

    /// Atomically add `additional_size_1e8` to the ledger row for
    /// `intent_hash`. Uses `SELECT ... FOR UPDATE` inside a
    /// transaction so concurrent adders on the same intent hash
    /// serialise; the row lock is released on commit / rollback.
    ///
    /// Returns:
    ///   * `Ok(new_filled_size_1e8)` on success.
    ///   * `Err(PerpsIntentCumulativeOverfill)` if the update would
    ///     push cumulative fill past `signed_size_1e8`, OR if the row
    ///     does not exist (fill without prior `record_intent` is a
    ///     bug — either the recorder was skipped or a fill arrived
    ///     from an unknown intent).
    ///   * `Err(Persistence)` on any DB error.
    pub async fn try_add_fill(
        &self,
        intent_hash: [u8; 32],
        additional_size_1e8: u128,
        now_ms: i64,
    ) -> Result<u128> {
        if additional_size_1e8 == 0 {
            // Zero-size adds are no-ops. Return the current filled
            // total for observability. The row still MUST exist —
            // otherwise a caller is asking about an unknown intent.
            let existing = self.get_filled(intent_hash).await?;
            return existing.ok_or_else(|| {
                BackendError::PerpsIntentCumulativeOverfill(format!(
                    "intent_hash {} not registered before zero-size fill add",
                    hex_of_hash(&intent_hash)
                ))
            });
        }
        let mut tx = self
            .repository
            .pool()
            .begin()
            .await
            .map_err(|err| BackendError::Persistence(err.to_string()))?;
        // Row-lock the intent's ledger row.
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT signed_size_1e8::TEXT, filled_size_1e8::TEXT \
             FROM perps_intent_fills_ledger \
             WHERE intent_hash = $1 FOR UPDATE",
        )
        .bind(intent_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;
        let (signed_str, filled_str) = match row {
            Some(pair) => pair,
            None => {
                // Roll back the empty transaction and error out.
                let _ = tx.rollback().await;
                return Err(BackendError::PerpsIntentCumulativeOverfill(format!(
                    "intent_hash {} not registered before fill add",
                    hex_of_hash(&intent_hash)
                )));
            }
        };
        let signed_size = signed_str.parse::<u128>().map_err(|err| {
            BackendError::Persistence(format!("signed_size_1e8 unparseable: {err}"))
        })?;
        let filled_size = filled_str.parse::<u128>().map_err(|err| {
            BackendError::Persistence(format!("filled_size_1e8 unparseable: {err}"))
        })?;
        let new_filled = match filled_size.checked_add(additional_size_1e8) {
            Some(v) => v,
            None => {
                let _ = tx.rollback().await;
                return Err(BackendError::PerpsIntentCumulativeOverfill(format!(
                    "filled_size overflow: {filled_size} + {additional_size_1e8}"
                )));
            }
        };
        if new_filled > signed_size {
            let _ = tx.rollback().await;
            return Err(BackendError::PerpsIntentCumulativeOverfill(format!(
                "cumulative fill {new_filled} exceeds signed size {signed_size} \
                 for intent {}",
                hex_of_hash(&intent_hash)
            )));
        }
        let new_filled_str = new_filled.to_string();
        sqlx::query(
            "UPDATE perps_intent_fills_ledger \
             SET filled_size_1e8 = $2::NUMERIC, last_updated_ms = $3 \
             WHERE intent_hash = $1",
        )
        .bind(intent_hash.as_slice())
        .bind(&new_filled_str)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;
        tx.commit()
            .await
            .map_err(|err| BackendError::Persistence(err.to_string()))?;
        Ok(new_filled)
    }

    /// Read-only accessor: the current filled total for an intent
    /// hash. Returns `Ok(None)` when the row is absent.
    pub async fn get_filled(&self, intent_hash: [u8; 32]) -> Result<Option<u128>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT filled_size_1e8::TEXT FROM perps_intent_fills_ledger \
             WHERE intent_hash = $1 LIMIT 1",
        )
        .bind(intent_hash.as_slice())
        .fetch_optional(self.repository.pool())
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;
        match row {
            None => Ok(None),
            Some((v,)) => {
                let parsed = v.parse::<u128>().map_err(|err| {
                    BackendError::Persistence(format!("filled_size_1e8 unparseable: {err}"))
                })?;
                Ok(Some(parsed))
            }
        }
    }

    /// Read-only accessor: `(signed_size, filled_size)` — for
    /// diagnostics and tests. Returns `Ok(None)` when absent.
    pub async fn get_row(&self, intent_hash: [u8; 32]) -> Result<Option<(u128, u128)>> {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT signed_size_1e8::TEXT, filled_size_1e8::TEXT \
             FROM perps_intent_fills_ledger WHERE intent_hash = $1 LIMIT 1",
        )
        .bind(intent_hash.as_slice())
        .fetch_optional(self.repository.pool())
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;
        match row {
            None => Ok(None),
            Some((s, f)) => {
                let signed = s.parse::<u128>().map_err(|err| {
                    BackendError::Persistence(format!("signed_size_1e8 unparseable: {err}"))
                })?;
                let filled = f.parse::<u128>().map_err(|err| {
                    BackendError::Persistence(format!("filled_size_1e8 unparseable: {err}"))
                })?;
                Ok(Some((signed, filled)))
            }
        }
    }
}

fn hex_of_hash(hash: &[u8; 32]) -> String {
    let mut s = String::with_capacity(66);
    s.push_str("0x");
    for b in hash {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
