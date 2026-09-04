//! PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part D +
//! PERPS-CLOSED-TEST-HARDENING-V1 Part A — nonce consumption ledger
//! for signed `PerpOrderIntent` requests.
//!
//! ## Trait
//!
//! [`PerpOrderIntentNonceLedger`] is the single-writer atomic
//! check-and-insert surface. Two implementations ship in-tree:
//!
//! * [`InMemoryNonceLedger`] — process-local `HashSet` behind a
//!   `Mutex`. Kept for unit tests and the default (no-repository)
//!   `AppState` path where PG isn't wired. Restart-clears; NOT
//!   production-safe.
//! * [`PgNonceLedger`] — wraps [`crate::db::PgRepository`]. Uses
//!   `INSERT ... ON CONFLICT DO NOTHING` inside the connection pool so
//!   `(trader, nonce)` uniqueness is enforced by the database itself.
//!   Survives restart; atomic under concurrent submissions; fails
//!   closed on any DB error (503 to client).
//!
//! ## Wire semantics
//!
//! * Keys are `(trader.0.to_lowercase(), nonce)`.
//! * `intent_hash` is the keccak256 EIP-712 struct hash of the signed
//!   intent (`crate::execution::perp_order_intent_hash`). It is stored
//!   for forensic audit but does NOT participate in the uniqueness key
//!   — the same intent CAN legitimately be resubmitted under a fresh
//!   nonce (that is a new authorisation from the trader).
//! * A duplicate `(trader, nonce)` collapses to
//!   [`BackendError::PerpsIntentNonceReplay`] (409). Any DB error
//!   collapses to [`BackendError::Persistence`] (503) — the caller
//!   NEVER silently passes on database uncertainty.
//!
//! ## Fail-closed
//!
//! * No optimistic in-memory cache in front of the PG ledger — the
//!   correctness cost outweighs the latency saving in the closed-test
//!   scope.
//! * NEVER persist raw signatures here — this ledger keeps only
//!   `(trader, nonce, intent_hash, consumed_at_ms)`.

use crate::db::PgRepository;
use crate::error::{BackendError, Result};
use crate::signing::eip712::parse_evm_address;
use crate::types::{now_ms, AccountId};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Mutex;

/// Atomic check-and-insert surface for signed `PerpOrderIntent`
/// nonces. Every submit through the closed-test signed-intent handler
/// goes through `try_consume` BEFORE dispatch to the internal engine.
///
/// Implementations MUST:
///
/// 1. Serialise concurrent `try_consume` calls with the same
///    `(trader, nonce)` — exactly one returns `Ok(())`, the rest
///    return `BackendError::PerpsIntentNonceReplay`.
/// 2. Persist the consumption atomically. On restart, replays of a
///    previously-consumed `(trader, nonce)` MUST still be rejected.
/// 3. Collapse any storage error to `BackendError::Persistence(...)`
///    so the caller returns a 503 to the client. Silent success on
///    uncertain writes is a critical bug.
#[async_trait]
pub trait PerpOrderIntentNonceLedger: Send + Sync {
    /// Atomically consume `(trader, nonce)`. On first sight returns
    /// `Ok(())`; on replay returns
    /// [`BackendError::PerpsIntentNonceReplay`]. On storage error
    /// returns [`BackendError::Persistence`].
    async fn try_consume(
        &self,
        trader: &AccountId,
        nonce: u128,
        intent_hash: [u8; 32],
    ) -> Result<()>;

    /// Read-only membership check. Cheap; used by tests and
    /// diagnostic surfaces if they ever want to sanity-check state.
    /// Storage error → [`BackendError::Persistence`].
    async fn has_consumed(&self, trader: &AccountId, nonce: u128) -> Result<bool>;
}

// ---------------------------------------------------------------------
// In-memory implementation — kept for unit tests + no-repository
// AppState. NOT production-safe (restart clears the set).
// ---------------------------------------------------------------------

/// Process-local nonce consumption set for `PerpOrderIntent`
/// signatures. Restart-clears; kept for unit tests and the
/// default (no-repository) AppState path.
#[derive(Debug, Default)]
pub struct InMemoryNonceLedger {
    used: Mutex<HashSet<(String, u128)>>,
}

impl InMemoryNonceLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Approximate count — useful for tests and diagnostics.
    pub fn len(&self) -> usize {
        self.used.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl PerpOrderIntentNonceLedger for InMemoryNonceLedger {
    async fn try_consume(
        &self,
        trader: &AccountId,
        nonce: u128,
        _intent_hash: [u8; 32],
    ) -> Result<()> {
        let key = (trader.0.to_lowercase(), nonce);
        let mut used = self.used.lock().map_err(|_| {
            BackendError::Config("perp_order_intent_nonce_ledger poisoned".to_string())
        })?;
        if !used.insert(key) {
            return Err(BackendError::PerpsIntentNonceReplay);
        }
        Ok(())
    }

    async fn has_consumed(&self, trader: &AccountId, nonce: u128) -> Result<bool> {
        let key = (trader.0.to_lowercase(), nonce);
        let used = self.used.lock().map_err(|_| {
            BackendError::Config("perp_order_intent_nonce_ledger poisoned".to_string())
        })?;
        Ok(used.contains(&key))
    }
}

// ---------------------------------------------------------------------
// PG-backed implementation — atomic + restart-safe. Fail-closed on any
// DB error.
// ---------------------------------------------------------------------

/// Postgres-backed nonce ledger. Wraps [`PgRepository`]; every
/// `try_consume` is a single-statement `INSERT ... ON CONFLICT DO
/// NOTHING RETURNING nonce_hex`. Row-count 0 means "already consumed"
/// (409); a real DB error propagates as 503.
///
/// See `migrations/0059_perps_signed_intent_nonce_ledger.sql` for the
/// table shape.
pub struct PgNonceLedger {
    repository: PgRepository,
}

impl PgNonceLedger {
    pub fn new(repository: PgRepository) -> Self {
        Self { repository }
    }
}

#[async_trait]
impl PerpOrderIntentNonceLedger for PgNonceLedger {
    async fn try_consume(
        &self,
        trader: &AccountId,
        nonce: u128,
        intent_hash: [u8; 32],
    ) -> Result<()> {
        let trader_bytes = parse_evm_address(trader)
            .map_err(|_| BackendError::Persistence("invalid trader address".to_string()))?;
        let nonce_hex = nonce.to_string();
        let consumed_at_ms = now_ms();
        let row: Option<(String,)> = sqlx::query_as(
            "INSERT INTO perps_signed_intent_nonce_ledger \
             (trader, nonce_hex, intent_hash, consumed_at_ms) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (trader, nonce_hex) DO NOTHING \
             RETURNING nonce_hex",
        )
        .bind(trader_bytes.as_slice())
        .bind(&nonce_hex)
        .bind(intent_hash.as_slice())
        .bind(consumed_at_ms)
        .fetch_optional(self.repository.pool())
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;
        match row {
            Some(_) => Ok(()),
            None => Err(BackendError::PerpsIntentNonceReplay),
        }
    }

    async fn has_consumed(&self, trader: &AccountId, nonce: u128) -> Result<bool> {
        let trader_bytes = parse_evm_address(trader)
            .map_err(|_| BackendError::Persistence("invalid trader address".to_string()))?;
        let nonce_hex = nonce.to_string();
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1::BIGINT FROM perps_signed_intent_nonce_ledger \
             WHERE trader = $1 AND nonce_hex = $2 LIMIT 1",
        )
        .bind(trader_bytes.as_slice())
        .bind(&nonce_hex)
        .fetch_optional(self.repository.pool())
        .await
        .map_err(|err| BackendError::Persistence(err.to_string()))?;
        Ok(row.is_some())
    }
}

// ---------------------------------------------------------------------
// Back-compat re-export. Existing call sites reference
// `PerpOrderIntentNonceStore`; we keep the name pointing at the
// in-memory implementation so refactors in Parts B+ can migrate
// gradually.
// ---------------------------------------------------------------------

/// Deprecated alias for [`InMemoryNonceLedger`]. Retained as a type
/// alias so external test fixtures that reference the old name keep
/// compiling.
pub type PerpOrderIntentNonceStore = InMemoryNonceLedger;

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(hex: &str) -> AccountId {
        AccountId::new(hex.to_string())
    }

    fn hash(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    #[tokio::test]
    async fn first_consume_succeeds_replay_rejected() {
        let store = InMemoryNonceLedger::new();
        let a = addr("0x0000000000000000000000000000000000000001");
        store.try_consume(&a, 1, hash(1)).await.unwrap();
        assert!(store.has_consumed(&a, 1).await.unwrap());
        let err = store.try_consume(&a, 1, hash(1)).await.unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
    }

    #[tokio::test]
    async fn different_traders_have_independent_nonce_spaces() {
        let store = InMemoryNonceLedger::new();
        let a = addr("0x0000000000000000000000000000000000000001");
        let b = addr("0x0000000000000000000000000000000000000002");
        store.try_consume(&a, 42, hash(1)).await.unwrap();
        store.try_consume(&b, 42, hash(2)).await.unwrap();
    }

    #[tokio::test]
    async fn address_comparison_is_case_insensitive() {
        let store = InMemoryNonceLedger::new();
        let lower = addr("0x00000000000000000000000000000000000000ab");
        let upper = addr("0x00000000000000000000000000000000000000AB");
        store.try_consume(&lower, 7, hash(3)).await.unwrap();
        let err = store.try_consume(&upper, 7, hash(3)).await.unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
    }

    #[tokio::test]
    async fn has_consumed_reports_absent_pair_as_false() {
        let store = InMemoryNonceLedger::new();
        let a = addr("0x0000000000000000000000000000000000000001");
        assert!(!store.has_consumed(&a, 999).await.unwrap());
    }

    #[tokio::test]
    async fn different_intent_hash_still_replay() {
        // Same (trader, nonce) but different intent_hash → still a
        // replay. Intent hash is audit-only, not part of the key.
        let store = InMemoryNonceLedger::new();
        let a = addr("0x0000000000000000000000000000000000000001");
        store.try_consume(&a, 5, hash(1)).await.unwrap();
        let err = store.try_consume(&a, 5, hash(2)).await.unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
    }
}
