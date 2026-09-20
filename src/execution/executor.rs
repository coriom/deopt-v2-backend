use super::config::ExecutionConfig;
use super::tx_builder::{
    build_perp_execution_call_from_intent, preview_perp_execution_call_from_intent,
    PreparedExecutionCall,
};
use super::StoredTradeSignatures;
use super::{ExecutionIntent, ExecutionIntentStatus};
use crate::error::{BackendError, Result};
use crate::types::{now_ms, TimestampMs};
use std::future::Future;
use std::pin::Pin;
use tracing::info;
use uuid::Uuid;

pub type RepositoryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait ExecutionIntentRepository: Clone + Send + Sync {
    fn list_pending_execution_intents(
        &self,
        limit: u32,
    ) -> RepositoryFuture<'_, Vec<ExecutionIntent>>;

    /// PERPS_BASE_SEPOLIA_CLOSED_TEST_EXECUTION_LIFECYCLE_AND_LOCAL_KEYSTORE_V1
    /// — return intents that are broadcast-workflow-eligible: those in
    /// `Pending`, `CalldataReady`, or `SimulationOk`. Ordered by
    /// `created_at_ms ASC` for stable FIFO selection.
    ///
    /// This is distinct from [`Self::list_pending_execution_intents`]:
    /// the older selector is used by the dry-run preview tick and must
    /// stay filtered to `Pending` so a cosigned/simulated intent does
    /// not silently regress. The new selector is consumed only by the
    /// real closed-test broadcast worker, which needs to advance
    /// `CalldataReady → SimulationOk → broadcast` inline.
    ///
    /// Default implementation falls back to the old selector so
    /// in-memory test doubles that predate this change keep working
    /// (they exercise only Pending flows).
    fn list_broadcastable_execution_intents(
        &self,
        limit: u32,
    ) -> RepositoryFuture<'_, Vec<ExecutionIntent>> {
        self.list_pending_execution_intents(limit)
    }

    fn update_execution_intent_status(
        &self,
        intent_id: Uuid,
        status: ExecutionIntentStatus,
        updated_at_ms: TimestampMs,
    ) -> RepositoryFuture<'_, ()>;

    fn get_execution_intent_signatures(
        &self,
        intent_id: Uuid,
    ) -> RepositoryFuture<'_, StoredTradeSignatures>;

    /// Persist the raw signed envelope + tx_hash + nonce BEFORE the
    /// `eth_sendRawTransaction` call. Transitions the intent to
    /// [`ExecutionIntentStatus::Prepared`]. Enforces DB uniqueness on
    /// `(intent_id)` and `(tx_hash)` and `(chain_id, executor, nonce)`.
    /// Default impl fails closed.
    fn record_prepared_transaction(
        &self,
        record: PreparedTransactionRecord,
    ) -> RepositoryFuture<'_, ()> {
        let _ = record;
        Box::pin(async move {
            Err(BackendError::Persistence(
                "record_prepared_transaction is not implemented for this repository".to_string(),
            ))
        })
    }

    /// Transition Prepared → Submitted after `eth_sendRawTransaction`
    /// succeeded (or returned an idempotent-replay class such as
    /// `AlreadyKnown` / `NonceTooLow`). The default impl reuses
    /// [`update_execution_intent_status`] so simple in-memory repos
    /// still work.
    fn mark_intent_submitted(
        &self,
        intent_id: Uuid,
        first_submission_at_ms: TimestampMs,
    ) -> RepositoryFuture<'_, ()> {
        self.update_execution_intent_status(
            intent_id,
            ExecutionIntentStatus::Submitted,
            first_submission_at_ms,
        )
    }

    /// Bump the `send_attempts` counter + `last_send_at_ms` for a
    /// Prepared intent. Used by ambiguous / retry paths so restart
    /// policy can enforce a bounded retry budget.
    fn bump_send_attempt(
        &self,
        intent_id: Uuid,
        last_send_at_ms: TimestampMs,
    ) -> RepositoryFuture<'_, u32> {
        let _ = (intent_id, last_send_at_ms);
        Box::pin(async move { Ok(1) })
    }

    /// Return the durable broadcast row (raw_tx / nonce / hash /
    /// attempts) associated with an intent, if any.
    fn get_prepared_broadcast(
        &self,
        intent_id: Uuid,
    ) -> RepositoryFuture<'_, Option<PreparedBroadcastRow>> {
        let _ = intent_id;
        Box::pin(async move { Ok(None) })
    }

    /// Legacy shim used by
    /// [`crate::execution::transaction::ensure_no_submitted_transaction`]
    /// callers — returns the persisted tx_hash if one exists.
    fn get_submitted_tx_hash(&self, intent_id: Uuid) -> RepositoryFuture<'_, Option<String>> {
        let intent_id_owned = intent_id;
        Box::pin(async move {
            let row = self.get_prepared_broadcast(intent_id_owned).await?;
            Ok(row.map(|r| r.tx_hash))
        })
    }

    /// Return intents whose durable broadcast lifecycle has not yet
    /// converged — i.e. status ∈ {Prepared, Submitted}. Reconciliation
    /// polls each row for a receipt and either finalizes or (for
    /// Prepared rows past a retry window) rebroadcasts the exact
    /// persisted raw envelope.
    fn list_unfinalized_broadcasts(
        &self,
        limit: u32,
    ) -> RepositoryFuture<'_, Vec<ExecutionIntent>> {
        let _ = limit;
        Box::pin(async move { Ok(Vec::new()) })
    }

    /// Legacy alias — kept as a default that delegates so pre-existing
    /// callers do not break.
    fn list_submitted_execution_intents(
        &self,
        limit: u32,
    ) -> RepositoryFuture<'_, Vec<ExecutionIntent>> {
        self.list_unfinalized_broadcasts(limit)
    }

    /// Transition an intent to [`ExecutionIntentStatus::Confirmed`]
    /// carrying the receipt block.
    fn mark_intent_confirmed(
        &self,
        intent_id: Uuid,
        receipt_block_number: u64,
        confirmed_at_ms: TimestampMs,
    ) -> RepositoryFuture<'_, ()> {
        let _ = (intent_id, receipt_block_number, confirmed_at_ms);
        Box::pin(async move {
            Err(BackendError::Persistence(
                "mark_intent_confirmed is not implemented for this repository".to_string(),
            ))
        })
    }

    /// Transition an intent to [`ExecutionIntentStatus::Failed`] with a
    /// durable failure classification + reason. Callers: reverted
    /// receipt, tx_hash mismatch, deterministic RPC rejection, event
    /// verification failure.
    fn mark_intent_failed(
        &self,
        intent_id: Uuid,
        reason: String,
        failed_at_ms: TimestampMs,
    ) -> RepositoryFuture<'_, ()> {
        let _ = (intent_id, reason, failed_at_ms);
        Box::pin(async move {
            Err(BackendError::Persistence(
                "mark_intent_failed is not implemented for this repository".to_string(),
            ))
        })
    }
}

/// Durable broadcast record persisted by
/// [`ExecutionIntentRepository::record_prepared_transaction`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedTransactionRecord {
    pub intent_id: Uuid,
    pub chain_id: u64,
    pub executor_address: crate::types::AccountId,
    pub target_address: crate::types::AccountId,
    pub tx_hash: String,
    pub nonce: u64,
    pub raw_tx_hex: String,
    pub prepared_at_ms: TimestampMs,
}

/// Row shape returned by
/// [`ExecutionIntentRepository::get_prepared_broadcast`]. Provides the
/// exact byte-identical raw envelope so reconciliation can rebroadcast
/// without allocating a new nonce or building a different transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedBroadcastRow {
    pub intent_id: Uuid,
    pub chain_id: u64,
    pub executor_address: crate::types::AccountId,
    pub target_address: crate::types::AccountId,
    pub tx_hash: String,
    pub nonce: u64,
    pub raw_tx_hex: String,
    pub status: ExecutionIntentStatus,
    pub prepared_at_ms: TimestampMs,
    pub first_submission_at_ms: Option<TimestampMs>,
    pub last_send_at_ms: Option<TimestampMs>,
    pub send_attempts: u32,
    pub receipt_block_number: Option<u64>,
    pub receipt_status: Option<u64>,
    pub confirmed_at_ms: Option<TimestampMs>,
    pub failure_class: Option<String>,
    pub failure_reason: Option<String>,
    pub failed_at_ms: Option<TimestampMs>,
}

#[derive(Clone)]
pub struct Executor<R> {
    config: ExecutionConfig,
    repository: R,
}

impl<R> Executor<R>
where
    R: ExecutionIntentRepository,
{
    pub fn new(config: ExecutionConfig, repository: R) -> Self {
        Self { config, repository }
    }

    pub async fn tick(&self) -> Result<ExecutionTickResult> {
        if !self.config.execution_enabled {
            return Ok(ExecutionTickResult::default());
        }
        if !self.config.dry_run && !self.config.real_broadcast_enabled {
            return Err(BackendError::Config(
                "real on-chain broadcast requires EXECUTOR_REAL_BROADCAST_ENABLED=true; set \
                 EXECUTOR_DRY_RUN=true to remain in preview mode"
                    .to_string(),
            ));
        }
        // PERPS-BASE-SEPOLIA-BACKEND-BROADCAST-WORKER-V1: when the
        // operator has explicitly enabled real broadcast, the executor
        // tick delegates dequeue-and-broadcast to
        // `BroadcastPolicy::broadcast_intent` (constructed and driven
        // by the runtime wiring in `main.rs`). This function still
        // enumerates candidates for the dry-run preview surface used
        // by operators to sanity-check calldata before flipping the
        // flag. The real path lives in `broadcast_policy.rs`.

        let intents = self
            .repository
            .list_pending_execution_intents(self.config.max_batch_size)
            .await?;
        let mut prepared_calls = Vec::with_capacity(intents.len());

        for intent in &intents {
            let signatures = self
                .repository
                .get_execution_intent_signatures(intent.intent_id)
                .await?;
            let prepared_call = (if signatures.calldata_ready() {
                build_perp_execution_call_from_intent(
                    intent,
                    &self.config.perp_matching_engine_address,
                    &signatures,
                )
            } else {
                preview_perp_execution_call_from_intent(
                    intent,
                    &self.config.perp_matching_engine_address,
                )
            })?;
            info!(
                intent_id = %intent.intent_id,
                market_id = intent.market_id,
                buyer = %intent.buyer.0,
                seller = %intent.seller.0,
                price_1e8 = %intent.price_1e8,
                size_1e8 = %intent.size_1e8,
                calldata_ready = !prepared_call.calldata.is_empty(),
                missing_signatures = prepared_call.missing_signatures,
                "executor dry-run prepared PerpMatchingEngine calldata preview"
            );
            self.repository
                .update_execution_intent_status(
                    intent.intent_id,
                    ExecutionIntentStatus::DryRun,
                    now_ms(),
                )
                .await?;
            prepared_calls.push(prepared_call);
        }

        Ok(ExecutionTickResult {
            pending_seen: intents.len(),
            dry_run_updated: prepared_calls.len(),
            calldata_ready: prepared_calls
                .iter()
                .filter(|call| !call.calldata.is_empty())
                .count(),
            missing_signatures: prepared_calls
                .iter()
                .filter(|call| call.missing_signatures)
                .count(),
            prepared_calls,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionTickResult {
    pub pending_seen: usize,
    pub dry_run_updated: usize,
    pub calldata_ready: usize,
    pub missing_signatures: usize,
    pub prepared_calls: Vec<PreparedExecutionCall>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AccountId, OrderId};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct InMemoryExecutionRepository {
        intents: Arc<Mutex<Vec<ExecutionIntent>>>,
    }

    impl InMemoryExecutionRepository {
        fn with_intents(intents: Vec<ExecutionIntent>) -> Self {
            Self {
                intents: Arc::new(Mutex::new(intents)),
            }
        }

        fn all(&self) -> Vec<ExecutionIntent> {
            self.intents.lock().unwrap().clone()
        }
    }

    impl ExecutionIntentRepository for InMemoryExecutionRepository {
        fn list_pending_execution_intents(
            &self,
            limit: u32,
        ) -> RepositoryFuture<'_, Vec<ExecutionIntent>> {
            let result = {
                let intents = self.intents.lock().unwrap();
                Ok(intents
                    .iter()
                    .filter(|intent| intent.status == ExecutionIntentStatus::Pending)
                    .take(limit as usize)
                    .cloned()
                    .collect())
            };
            Box::pin(async move { result })
        }

        fn update_execution_intent_status(
            &self,
            intent_id: Uuid,
            status: ExecutionIntentStatus,
            _updated_at_ms: TimestampMs,
        ) -> RepositoryFuture<'_, ()> {
            let result = {
                let mut intents = self.intents.lock().unwrap();
                let Some(intent) = intents
                    .iter_mut()
                    .find(|intent| intent.intent_id == intent_id)
                else {
                    return Box::pin(async {
                        Err(BackendError::Persistence(
                            "execution intent not found".to_string(),
                        ))
                    });
                };
                intent.status = status;
                Ok(())
            };
            Box::pin(async move { result })
        }

        fn get_execution_intent_signatures(
            &self,
            _intent_id: Uuid,
        ) -> RepositoryFuture<'_, StoredTradeSignatures> {
            Box::pin(async { Ok(StoredTradeSignatures::default()) })
        }
    }

    #[tokio::test]
    async fn dry_run_tick_marks_pending_intents_dry_run() {
        let intent = ExecutionIntent {
            intent_id: Uuid::from_u128(1),
            market_id: 1,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            price_1e8: 100,
            size_1e8: 10,
            buy_order_id: OrderId(Uuid::from_u128(2)),
            sell_order_id: OrderId(Uuid::from_u128(3)),
            buyer_is_maker: Some(false),
            buyer_nonce: Some(11),
            seller_nonce: Some(12),
            deadline_ms: Some(123_456),
            created_at_ms: 123,
            status: ExecutionIntentStatus::Pending,
            protocol_version: crate::execution::perp_trade::PerpsProtocolVersion::V1,
            max_execution_price_1e8: 0,
            min_execution_price_1e8: 0,
        };
        let repository = InMemoryExecutionRepository::with_intents(vec![intent]);
        let config = ExecutionConfig {
            execution_enabled: true,
            dry_run: true,
            poll_interval_ms: 1_000,
            max_batch_size: 10,
            real_broadcast_enabled: false,
            executor_private_key: None,
            executor_chain_id: 84532,
            max_gas_limit: 1_000_000,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            require_simulation_ok: true,
            simulation_enabled: false,
            simulation_requires_persistence: true,
            rpc_url: None,
            executor_from_address: AccountId::new("0x0000000000000000000000000000000000000000"),
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000009",
            ),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
            old_perp_engine_address: None,
        perp_engine_v2_address: None,
        perp_matching_engine_v2_address: None,
        perp_clearing_account_v2_address: None,
        perps_active_engine_version: crate::execution::perp_trade::PerpsProtocolVersion::V1,
        perps_v2_clearing_min_balance_raw: 0,
            backend_signer_mode: crate::execution::SignerBackendKind::LocalDev,
            backend_signer_endpoint: None,
            executor_allow_local_signer: false,
            backend_signer_provider: None,
            backend_signer_timeout_ms: 2500,
            perps_closed_test_broadcast_armed: false,
            perps_closed_test_broadcast_intent_id: None,
            perps_closed_test_max_drift_bps: 100,
            perps_closed_test_min_deadline_remaining_sec: 900,
            executor_keystore_path: None,
            executor_keystore_password_file: None,
        };

        let result = Executor::new(config, repository.clone())
            .tick()
            .await
            .unwrap();

        assert_eq!(result.pending_seen, 1);
        assert_eq!(result.dry_run_updated, 1);
        assert_eq!(result.calldata_ready, 0);
        assert_eq!(result.missing_signatures, 1);
        assert_eq!(result.prepared_calls.len(), 1);
        assert!(result.prepared_calls[0].missing_signatures);
        assert_eq!(repository.all()[0].status, ExecutionIntentStatus::DryRun);
    }
}
