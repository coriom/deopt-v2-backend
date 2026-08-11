//! Nonce-conflict investigator for the Hybrid V2 broadcast pipeline
//! (Part K of `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`).
//!
//! When `send_raw_transaction` returns `NonceTooLow`, `NonceTooHigh`, or
//! `ReplacementUnderpriced`, the outbox has ALREADY persisted the local
//! tx hash before the call — recovery must NEVER re-sign, NEVER reserve
//! a fresh nonce, and NEVER auto-fee-bump. Instead, the investigator
//! observes the network to determine which of four deterministic
//! outcomes obtains:
//!
//! * `OurTxMined` — the local tx hash is on chain (our nonce is ours;
//!   the receipt watcher will finish the job).
//! * `OurTxPending` — the local tx hash is in the mempool (still ours).
//! * `NonceReleasedNoTxFound` — pending_nonce < our_nonce (the account
//!   has not yet advanced to our nonce; another attempt has NOT taken
//!   it).
//! * `DifferentTxConsumedNonce` — pending_nonce > our_nonce (something
//!   else took our nonce slot; operator investigation required).
//! * `Ambiguous` — pending_nonce == our_nonce with our tx nowhere; the
//!   provider view is transiently inconsistent.
//!
//! Frozen safety:
//! * `NO_AUTOMATIC_NONCE_REPLACEMENT` — a local nonce reservation is
//!   NEVER released while transaction outcome is ambiguous.
//! * `NO_AUTOMATIC_FEE_BUMP_OR_RBF`.

use crate::hybrid_v2::execution::broadcast_rpc::{BroadcastRpcError, ExecutionBroadcastRpcClient};
use crate::hybrid_v2::execution::rpc::BlockTag;
use crate::hybrid_v2::persistence::HybridV2ProjectionStore;

/// Structured investigation outcomes. `DifferentTxConsumedNonce`
/// carries an `observed_tx_hash` of `[0u8; 32]` when the provider does
/// not expose a nonce-indexed receipt lookup (typical): operator
/// investigation of on-chain history remains required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonceInvestigationOutcome {
    /// Our tx hash is mined at `(block_number, block_hash)`.
    OurTxMined {
        block_number: u64,
        block_hash: [u8; 32],
    },
    /// Our tx hash is in the mempool but not mined.
    OurTxPending,
    /// The account nonce has not yet advanced to our reserved nonce —
    /// nobody has taken our slot.
    NonceReleasedNoTxFound,
    /// The account nonce has moved PAST our reserved nonce but our tx
    /// hash is not on chain — something else consumed the slot.
    DifferentTxConsumedNonce { observed_tx_hash: [u8; 32] },
    /// The account nonce equals our reserved nonce (or below), our tx
    /// is nowhere. Provider view is transiently inconsistent — the
    /// investigator refuses to commit a verdict.
    Ambiguous,
}

/// Nonce investigator. Borrows the store + rpc + executor identity for
/// the duration of a single `investigate` call. Does NOT mutate state;
/// callers persist the outcome via the outbox / worker.
pub struct BroadcastNonceInvestigator<'a> {
    pub store: &'a dyn HybridV2ProjectionStore,
    pub rpc: &'a dyn ExecutionBroadcastRpcClient,
    pub executor_address: [u8; 20],
    pub chain_id: u64,
}

impl BroadcastNonceInvestigator<'_> {
    /// Perform one investigation cycle for `(our_tx_hash, our_nonce)`.
    ///
    /// The steps are:
    ///   1. `transaction_by_hash(our_tx_hash)` — Some(mined) →
    ///      `OurTxMined`; Some(pending) → `OurTxPending`.
    ///   2. None → `transaction_count(executor, Pending)`.
    ///      - pending_nonce > our_nonce → `DifferentTxConsumedNonce`
    ///        (operator must inspect on-chain history).
    ///      - pending_nonce < our_nonce → `NonceReleasedNoTxFound`.
    ///      - pending_nonce == our_nonce → `Ambiguous`.
    pub async fn investigate(
        &self,
        our_tx_hash: [u8; 32],
        our_nonce: u64,
    ) -> Result<NonceInvestigationOutcome, BroadcastRpcError> {
        // Suppress dead-code lint on `store` — the persist-evidence hook
        // is intentional API surface used by the outbox integration.
        let _ = self.store;
        let _ = self.chain_id;

        match self.rpc.transaction_by_hash(our_tx_hash).await? {
            Some(tx) if tx.tx_hash == our_tx_hash => {
                if let (Some(bn), Some(bh)) = (tx.block_number, tx.block_hash) {
                    return Ok(NonceInvestigationOutcome::OurTxMined {
                        block_number: bn,
                        block_hash: bh,
                    });
                }
                return Ok(NonceInvestigationOutcome::OurTxPending);
            }
            // A provider that echoes back a different tx hash for the
            // same lookup is treated as ambiguous — the caller must
            // escalate. We never accept a substitute identity.
            Some(_) => return Ok(NonceInvestigationOutcome::Ambiguous),
            None => {}
        }

        let pending_nonce = self
            .rpc
            .transaction_count(self.executor_address, BlockTag::Pending)
            .await?;

        if pending_nonce > our_nonce {
            // Something else took the slot. `receipt-by-nonce` is not a
            // JSON-RPC method, so we cannot cheaply enumerate the tx
            // that consumed our nonce here — the observed_tx_hash is
            // zeroed and the outbox escalates to
            // MANUAL_INTERVENTION_REQUIRED for operator inspection.
            return Ok(NonceInvestigationOutcome::DifferentTxConsumedNonce {
                observed_tx_hash: [0u8; 32],
            });
        }
        if pending_nonce < our_nonce {
            return Ok(NonceInvestigationOutcome::NonceReleasedNoTxFound);
        }
        // pending_nonce == our_nonce: the account has advanced to our
        // slot but our tx is neither mined nor pending. This is a
        // transient inconsistency — refuse to commit a verdict.
        Ok(NonceInvestigationOutcome::Ambiguous)
    }
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::broadcast_rpc::{
        BlockHeader, SendOutcome, TransactionSummary, TxReceipt,
    };
    use crate::hybrid_v2::persistence::InMemoryProjectionStore;
    use alloy_primitives::U256;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockRpc {
        inner: Mutex<MockRpcInner>,
    }

    #[derive(Default)]
    struct MockRpcInner {
        tx: Option<TransactionSummary>,
        pending_nonce: u64,
    }

    impl MockRpc {
        fn with_pending(pending: u64) -> Self {
            Self {
                inner: Mutex::new(MockRpcInner {
                    tx: None,
                    pending_nonce: pending,
                }),
            }
        }
        fn set_tx(&self, tx: Option<TransactionSummary>) {
            self.inner.lock().unwrap().tx = tx;
        }
        fn set_pending(&self, n: u64) {
            self.inner.lock().unwrap().pending_nonce = n;
        }
    }

    #[async_trait]
    impl ExecutionBroadcastRpcClient for MockRpc {
        async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
            Ok(84532)
        }
        async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
            Ok(100)
        }
        async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
            Ok(Some(90))
        }
        async fn transaction_count(
            &self,
            _address: [u8; 20],
            _block_tag: BlockTag,
        ) -> Result<u64, BroadcastRpcError> {
            Ok(self.inner.lock().unwrap().pending_nonce)
        }
        async fn transaction_by_hash(
            &self,
            _tx_hash: [u8; 32],
        ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
            Ok(self.inner.lock().unwrap().tx.clone())
        }
        async fn receipt_by_hash(
            &self,
            _tx_hash: [u8; 32],
        ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
            Ok(None)
        }
        async fn block_header_by_number(
            &self,
            _number: u64,
        ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
            Ok(None)
        }
        async fn block_header_by_hash(
            &self,
            _hash: [u8; 32],
        ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
            Ok(None)
        }
        async fn send_raw_transaction(
            &self,
            _raw_tx_bytes: &[u8],
        ) -> Result<SendOutcome, BroadcastRpcError> {
            panic!("investigator must not call send_raw_transaction");
        }
    }

    fn our_hash() -> [u8; 32] {
        [0x11u8; 32]
    }

    fn other_hash() -> [u8; 32] {
        [0x22u8; 32]
    }

    #[tokio::test]
    async fn our_tx_mined_returns_block_ref() {
        let rpc = MockRpc::with_pending(0);
        rpc.set_tx(Some(TransactionSummary {
            tx_hash: our_hash(),
            from: [0x77u8; 20],
            to: Some([0xEE; 20]),
            nonce: 5,
            block_number: Some(42),
            block_hash: Some([0xAB; 32]),
            value_wei: U256::ZERO,
            input_bytes_len: 4,
            input_hash: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            tx_type: 2,
        }));
        let store = InMemoryProjectionStore::new();
        let inv = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let outcome = inv.investigate(our_hash(), 5).await.unwrap();
        assert_eq!(
            outcome,
            NonceInvestigationOutcome::OurTxMined {
                block_number: 42,
                block_hash: [0xAB; 32],
            }
        );
    }

    #[tokio::test]
    async fn our_tx_pending_returns_pending() {
        let rpc = MockRpc::with_pending(0);
        rpc.set_tx(Some(TransactionSummary {
            tx_hash: our_hash(),
            from: [0x77u8; 20],
            to: Some([0xEE; 20]),
            nonce: 5,
            block_number: None,
            block_hash: None,
            value_wei: U256::ZERO,
            input_bytes_len: 4,
            input_hash: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            tx_type: 2,
        }));
        let store = InMemoryProjectionStore::new();
        let inv = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let outcome = inv.investigate(our_hash(), 5).await.unwrap();
        assert_eq!(outcome, NonceInvestigationOutcome::OurTxPending);
    }

    #[tokio::test]
    async fn nonce_released_when_pending_below_our_nonce() {
        let rpc = MockRpc::with_pending(4);
        rpc.set_tx(None);
        let store = InMemoryProjectionStore::new();
        let inv = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let outcome = inv.investigate(our_hash(), 5).await.unwrap();
        assert_eq!(outcome, NonceInvestigationOutcome::NonceReleasedNoTxFound);
    }

    #[tokio::test]
    async fn different_tx_consumed_nonce_when_pending_above() {
        let rpc = MockRpc::with_pending(6);
        rpc.set_tx(None);
        let store = InMemoryProjectionStore::new();
        let inv = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let outcome = inv.investigate(our_hash(), 5).await.unwrap();
        assert_eq!(
            outcome,
            NonceInvestigationOutcome::DifferentTxConsumedNonce {
                observed_tx_hash: [0u8; 32],
            }
        );
    }

    #[tokio::test]
    async fn ambiguous_when_pending_equals_our_nonce_and_tx_absent() {
        let rpc = MockRpc::with_pending(5);
        rpc.set_tx(None);
        let store = InMemoryProjectionStore::new();
        let inv = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let outcome = inv.investigate(our_hash(), 5).await.unwrap();
        assert_eq!(outcome, NonceInvestigationOutcome::Ambiguous);
    }

    #[tokio::test]
    async fn ambiguous_when_provider_returns_different_hash() {
        let rpc = MockRpc::with_pending(0);
        rpc.set_tx(Some(TransactionSummary {
            tx_hash: other_hash(),
            from: [0x77u8; 20],
            to: Some([0xEE; 20]),
            nonce: 5,
            block_number: Some(50),
            block_hash: Some([0xCD; 32]),
            value_wei: U256::ZERO,
            input_bytes_len: 4,
            input_hash: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            tx_type: 2,
        }));
        let store = InMemoryProjectionStore::new();
        let inv = BroadcastNonceInvestigator {
            store: &store,
            rpc: &rpc,
            executor_address: [0x77u8; 20],
            chain_id: 84532,
        };
        let outcome = inv.investigate(our_hash(), 5).await.unwrap();
        assert_eq!(outcome, NonceInvestigationOutcome::Ambiguous);
    }

    #[tokio::test]
    async fn rpc_error_propagates() {
        struct FailingRpc;
        #[async_trait]
        impl ExecutionBroadcastRpcClient for FailingRpc {
            async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
                Ok(1)
            }
            async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
                Ok(1)
            }
            async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
                Ok(None)
            }
            async fn transaction_count(
                &self,
                _a: [u8; 20],
                _b: BlockTag,
            ) -> Result<u64, BroadcastRpcError> {
                Ok(0)
            }
            async fn transaction_by_hash(
                &self,
                _t: [u8; 32],
            ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
                Err(BroadcastRpcError::Timeout)
            }
            async fn receipt_by_hash(
                &self,
                _t: [u8; 32],
            ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
                Ok(None)
            }
            async fn block_header_by_number(
                &self,
                _n: u64,
            ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
                Ok(None)
            }
            async fn block_header_by_hash(
                &self,
                _h: [u8; 32],
            ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
                Ok(None)
            }
            async fn send_raw_transaction(
                &self,
                _r: &[u8],
            ) -> Result<SendOutcome, BroadcastRpcError> {
                panic!()
            }
        }
        let store = InMemoryProjectionStore::new();
        let inv = BroadcastNonceInvestigator {
            store: &store,
            rpc: &FailingRpc,
            executor_address: [0x77u8; 20],
            chain_id: 1,
        };
        let err = inv.investigate([0x33u8; 32], 7).await.unwrap_err();
        assert_eq!(err, BroadcastRpcError::Timeout);
    }
}
