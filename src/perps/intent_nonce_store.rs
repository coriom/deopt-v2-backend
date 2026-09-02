//! PERPS-FULLSTACK-RUNTIME-INTEGRATION-V1 Part D — in-memory nonce
//! consumption ledger for signed `PerpOrderIntent` requests.
//!
//! Keyed by `(trader address (lower-cased), nonce)`. `try_consume` is
//! atomic (single mutex-guarded `HashSet::insert`) — a duplicate `nonce`
//! for the same trader within one process lifetime is rejected with
//! [`crate::error::BackendError::PerpsIntentNonceReplay`].
//!
//! **V1 scope — closed-test posture:** the store is process-local and
//! resets on restart. This is acceptable for the closed-test surface
//! (`perps_closed_test_enabled` + allowlist) because:
//! * The endpoint is not exposed to public mainnet traffic (mainnet
//!   startup refusal is enforced elsewhere).
//! * A restart-window replay would still have to satisfy the EIP-712
//!   signature and the trader/deadline gates before touching the store.
//! * Persistent nonce ledgering will land alongside the public-trading
//!   flip in a future milestone; the trait-shaped design here (a single
//!   opaque store hung off `AppState`) makes swapping in a Postgres
//!   implementation a drop-in replacement.
//!
//! NEVER persist raw signatures here — this store keeps only the
//! `(trader, nonce)` tuple.

use crate::error::{BackendError, Result};
use crate::types::AccountId;
use std::collections::HashSet;
use std::sync::Mutex;

/// Process-local nonce consumption set for `PerpOrderIntent` signatures.
///
/// * Keys are `(trader.0.to_lowercase(), nonce)`.
/// * Non-poisoning: if the inner mutex is poisoned (extremely unusual —
///   would only happen if a panic held the lock), `try_consume` and
///   `has_consumed` fall back to erroring rather than returning stale
///   data.
#[derive(Debug, Default)]
pub struct PerpOrderIntentNonceStore {
    used: Mutex<HashSet<(String, u128)>>,
}

impl PerpOrderIntentNonceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically check-and-insert. Returns `Ok(())` on the first sight
    /// of the pair; [`BackendError::PerpsIntentNonceReplay`] if the
    /// pair was already consumed.
    pub fn try_consume(&self, trader: &AccountId, nonce: u128) -> Result<()> {
        let key = (trader.0.to_lowercase(), nonce);
        let mut used = self
            .used
            .lock()
            .map_err(|_| BackendError::Config("perp_order_intent_nonce_store poisoned".to_string()))?;
        if !used.insert(key) {
            return Err(BackendError::PerpsIntentNonceReplay);
        }
        Ok(())
    }

    /// Read-only membership check. Cheap; used by the readiness /
    /// diagnostic surfaces if they ever want to sanity-check state.
    pub fn has_consumed(&self, trader: &AccountId, nonce: u128) -> bool {
        let key = (trader.0.to_lowercase(), nonce);
        let used = match self.used.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        used.contains(&key)
    }

    /// Approximate count — useful for tests and diagnostics.
    pub fn len(&self) -> usize {
        self.used.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(hex: &str) -> AccountId {
        AccountId::new(hex.to_string())
    }

    #[test]
    fn first_consume_succeeds_replay_rejected() {
        let store = PerpOrderIntentNonceStore::new();
        let a = addr("0x0000000000000000000000000000000000000001");
        store.try_consume(&a, 1).unwrap();
        assert!(store.has_consumed(&a, 1));
        let err = store.try_consume(&a, 1).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
    }

    #[test]
    fn different_traders_have_independent_nonce_spaces() {
        let store = PerpOrderIntentNonceStore::new();
        let a = addr("0x0000000000000000000000000000000000000001");
        let b = addr("0x0000000000000000000000000000000000000002");
        store.try_consume(&a, 42).unwrap();
        store.try_consume(&b, 42).unwrap();
    }

    #[test]
    fn address_comparison_is_case_insensitive() {
        let store = PerpOrderIntentNonceStore::new();
        let lower = addr("0x00000000000000000000000000000000000000ab");
        let upper = addr("0x00000000000000000000000000000000000000AB");
        store.try_consume(&lower, 7).unwrap();
        let err = store.try_consume(&upper, 7).unwrap_err();
        assert!(matches!(err, BackendError::PerpsIntentNonceReplay));
    }

    #[test]
    fn has_consumed_reports_absent_pair_as_false() {
        let store = PerpOrderIntentNonceStore::new();
        let a = addr("0x0000000000000000000000000000000000000001");
        assert!(!store.has_consumed(&a, 999));
    }
}
