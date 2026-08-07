//! Executor nonce reservation for Hybrid V2 (Part K of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! Reserves the NEXT free nonce for a `(chain_id, signer_identity)`
//! tuple using the atomic UNIQUE constraint on
//! `hybrid_v2_executor_nonces(chain_id, signer_identity,
//! reserved_nonce)` (see `migrations/0049_hybrid_v2_execution.sql`).
//!
//! Concurrent safety: two workers racing to reserve the same nonce
//! see one INSERT succeed (`rows_affected = 1`) and the other collide
//! (`rows_affected = 0`); the loser retries the next candidate. The
//! per-attempt cap is bounded so a runaway race terminates.

use crate::error::BackendError;
use crate::hybrid_v2::execution::rpc::{BlockTag, ExecutionRpcClient, ExecutionRpcError};
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;
use thiserror::Error;

/// Reservation output — carries every field needed to persist the
/// reservation alongside the execution row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonceReservation {
    pub chain_id: u64,
    pub signer_identity: String,
    pub nonce: u64,
    pub canonical_execution_id: String,
    pub reserved_at_ms: u64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum NonceReserveError {
    #[error("chain mismatch: expected {expected} actual {actual}")]
    ChainMismatch { expected: u64, actual: u64 },
    #[error("rpc failure: {0}")]
    RpcFailure(String),
    #[error("persistence failure: {0}")]
    Persistence(String),
    #[error("reservation conflict: another worker claimed every candidate nonce")]
    ReservationConflict,
    #[error("exhausted reservation attempts (bounded loop)")]
    ExhaustedAttempts,
}

/// Reserves the next free nonce for `(chain, signer)`.
pub struct NonceReserver<'a> {
    pub store: &'a dyn HybridV2ProjectionStore,
    pub rpc: &'a dyn ExecutionRpcClient,
    /// Lowercase 0x-prefixed hex string of the executor address.
    pub signer_identity: String,
    pub chain_id: u64,
}

impl NonceReserver<'_> {
    /// Reserve the next free nonce for `(self.chain_id,
    /// self.signer_identity)` and bind it to `canonical_execution_id`.
    ///
    /// Contract:
    /// 1. Verify `chain_id()` == `self.chain_id`.
    /// 2. Read `transaction_count(signer, Pending)` — this is the
    ///    LOWER bound (anything the mempool already knows about).
    /// 3. Read `get_reserved_nonces_for(chain, signer)` — persisted
    ///    reservations. Candidate = `max(onchain, max(reserved)+1)`.
    /// 4. Try `reserve_executor_nonce(candidate)`. On unique-conflict
    ///    increment and retry, up to 8 attempts.
    pub async fn reserve_for(
        &self,
        canonical_execution_id: &str,
        now_ms: u64,
    ) -> Result<NonceReservation, NonceReserveError> {
        // Chain firewall.
        let observed = self.rpc.chain_id().await.map_err(map_rpc)?;
        if observed != self.chain_id {
            return Err(NonceReserveError::ChainMismatch {
                expected: self.chain_id,
                actual: observed,
            });
        }

        let signer_addr = parse_address_bytes(&self.signer_identity).map_err(|e| {
            NonceReserveError::RpcFailure(format!("signer_identity not a valid hex address: {e}"))
        })?;
        let on_chain_next = self
            .rpc
            .transaction_count(signer_addr, BlockTag::Pending)
            .await
            .map_err(map_rpc)?;

        // Persisted reservations for this signer. We treat all
        // statuses as "seen" so we never re-issue a historically used
        // slot (SETTLED, ABANDONED, RESERVED all block re-use).
        let existing = self
            .store
            .get_reserved_nonces_for(self.chain_id as i64, &self.signer_identity)
            .await
            .map_err(map_persist)?;
        let max_reserved = existing.iter().copied().max().unwrap_or(-1i64);
        let candidate_from_reserved: u64 = if max_reserved < 0 {
            0
        } else {
            (max_reserved as u64).saturating_add(1)
        };
        let mut candidate = on_chain_next.max(candidate_from_reserved);

        const MAX_ATTEMPTS: u32 = 8;
        for _ in 0..MAX_ATTEMPTS {
            let ok = self
                .store
                .reserve_executor_nonce(
                    self.chain_id as i64,
                    &self.signer_identity,
                    candidate as i64,
                    canonical_execution_id,
                    now_ms as i64,
                )
                .await
                .map_err(map_persist)?;
            if ok {
                return Ok(NonceReservation {
                    chain_id: self.chain_id,
                    signer_identity: self.signer_identity.clone(),
                    nonce: candidate,
                    canonical_execution_id: canonical_execution_id.to_string(),
                    reserved_at_ms: now_ms,
                });
            }
            candidate = candidate.saturating_add(1);
        }
        Err(NonceReserveError::ExhaustedAttempts)
    }

    /// Mark a previously-reserved nonce as historically abandoned.
    /// Idempotent — no-op if the row is missing or already terminal.
    pub async fn mark_abandoned(
        &self,
        reservation: &NonceReservation,
    ) -> Result<(), NonceReserveError> {
        self.store
            .mark_executor_nonce_abandoned(
                self.chain_id as i64,
                &self.signer_identity,
                reservation.nonce as i64,
            )
            .await
            .map_err(map_persist)
    }
}

fn map_rpc(e: ExecutionRpcError) -> NonceReserveError {
    match e {
        ExecutionRpcError::ChainMismatch { expected, actual } => {
            NonceReserveError::ChainMismatch { expected, actual }
        }
        other => NonceReserveError::RpcFailure(other.to_string()),
    }
}

fn map_persist(e: BackendError) -> NonceReserveError {
    NonceReserveError::Persistence(e.to_string())
}

fn parse_address_bytes(s: &str) -> Result<[u8; 20], String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if stripped.len() != 40 {
        return Err(format!("expected 40 hex chars, got {}", stripped.len()));
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        let hi = hex_nibble(stripped.as_bytes()[2 * i])?;
        let lo = hex_nibble(stripped.as_bytes()[2 * i + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    Ok(match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => 10 + (b - b'a'),
        b'A'..=b'F' => 10 + (b - b'A'),
        other => return Err(format!("non-hex 0x{other:02x}")),
    })
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::rpc::{
        EthCallOutcome, EthCallRequest, ExecutionRpcClient, FeeHistory,
    };
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use async_trait::async_trait;

    struct FakeRpc {
        chain_id: u64,
        pending_nonce: u64,
    }

    #[async_trait]
    impl ExecutionRpcClient for FakeRpc {
        async fn chain_id(&self) -> Result<u64, ExecutionRpcError> {
            Ok(self.chain_id)
        }
        async fn head_block_number(&self) -> Result<u64, ExecutionRpcError> {
            Ok(1)
        }
        async fn head_block_hash(&self) -> Result<[u8; 32], ExecutionRpcError> {
            Ok([0; 32])
        }
        async fn transaction_count(
            &self,
            _: [u8; 20],
            _: BlockTag,
        ) -> Result<u64, ExecutionRpcError> {
            Ok(self.pending_nonce)
        }
        async fn eth_call(
            &self,
            _: EthCallRequest,
            _: BlockTag,
        ) -> Result<EthCallOutcome, ExecutionRpcError> {
            unreachable!()
        }
        async fn estimate_gas(
            &self,
            _: EthCallRequest,
            _: BlockTag,
        ) -> Result<u64, ExecutionRpcError> {
            unreachable!()
        }
        async fn fee_history(
            &self,
            _: u64,
            _: BlockTag,
            _: &[f64],
        ) -> Result<FeeHistory, ExecutionRpcError> {
            unreachable!()
        }
    }

    const SIGNER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[tokio::test]
    async fn reserves_from_on_chain_when_no_persisted() {
        let store = InMemoryProjectionStore::new();
        let rpc = FakeRpc {
            chain_id: 84532,
            pending_nonce: 7,
        };
        let reserver = NonceReserver {
            store: &store,
            rpc: &rpc,
            signer_identity: SIGNER.to_string(),
            chain_id: 84532,
        };
        let out = reserver.reserve_for("0xexec", 100).await.unwrap();
        assert_eq!(out.nonce, 7);
    }

    #[tokio::test]
    async fn advances_past_persisted_reservations() {
        let store = InMemoryProjectionStore::new();
        // Pre-populate 3 prior reservations at nonce 5, 6, 7.
        for n in 5..=7 {
            store
                .reserve_executor_nonce(84532, SIGNER, n, "0xother", 1)
                .await
                .unwrap();
        }
        let rpc = FakeRpc {
            chain_id: 84532,
            pending_nonce: 3,
        };
        let reserver = NonceReserver {
            store: &store,
            rpc: &rpc,
            signer_identity: SIGNER.to_string(),
            chain_id: 84532,
        };
        let out = reserver.reserve_for("0xnew", 100).await.unwrap();
        assert_eq!(out.nonce, 8);
    }

    #[tokio::test]
    async fn chain_mismatch_rejected() {
        let store = InMemoryProjectionStore::new();
        let rpc = FakeRpc {
            chain_id: 11155111,
            pending_nonce: 0,
        };
        let reserver = NonceReserver {
            store: &store,
            rpc: &rpc,
            signer_identity: SIGNER.to_string(),
            chain_id: 84532,
        };
        let err = reserver.reserve_for("0xexec", 100).await.unwrap_err();
        assert!(matches!(err, NonceReserveError::ChainMismatch { .. }));
    }

    #[tokio::test]
    async fn concurrent_race_serializes_via_unique() {
        // Two "workers" (sequential in this synthetic race — the
        // pgstore integration test is where the true concurrency lives)
        // both compute the same starting candidate; the first wins the
        // insert and the second must advance.
        let store = InMemoryProjectionStore::new();
        let rpc = FakeRpc {
            chain_id: 84532,
            pending_nonce: 0,
        };
        let a = NonceReserver {
            store: &store,
            rpc: &rpc,
            signer_identity: SIGNER.to_string(),
            chain_id: 84532,
        };
        let b = NonceReserver {
            store: &store,
            rpc: &rpc,
            signer_identity: SIGNER.to_string(),
            chain_id: 84532,
        };
        let ra = a.reserve_for("0xA", 100).await.unwrap();
        let rb = b.reserve_for("0xB", 101).await.unwrap();
        assert_ne!(ra.nonce, rb.nonce);
        assert_eq!(ra.nonce.min(rb.nonce), 0);
        assert_eq!(ra.nonce.max(rb.nonce), 1);
    }

    #[tokio::test]
    async fn mark_abandoned_is_idempotent() {
        let store = InMemoryProjectionStore::new();
        let rpc = FakeRpc {
            chain_id: 84532,
            pending_nonce: 0,
        };
        let r = NonceReserver {
            store: &store,
            rpc: &rpc,
            signer_identity: SIGNER.to_string(),
            chain_id: 84532,
        };
        let res = r.reserve_for("0xexec", 100).await.unwrap();
        r.mark_abandoned(&res).await.unwrap();
        // Twice — must not error.
        r.mark_abandoned(&res).await.unwrap();
    }
}
