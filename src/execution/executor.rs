use super::config::ExecutionConfig;
use super::tx_builder::{build_perp_execution_call, PreparedExecutionCall};
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

    fn update_execution_intent_status(
        &self,
        intent_id: Uuid,
        status: ExecutionIntentStatus,
        updated_at_ms: TimestampMs,
    ) -> RepositoryFuture<'_, ()>;
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
        if !self.config.dry_run {
            return Err(BackendError::Config(
                "real on-chain execution is not implemented yet; set EXECUTOR_DRY_RUN=true"
                    .to_string(),
            ));
        }

        let intents = self
            .repository
            .list_pending_execution_intents(self.config.max_batch_size)
            .await?;
        let mut prepared_calls = Vec::with_capacity(intents.len());

        for intent in &intents {
            let prepared_call =
                build_perp_execution_call(intent, &self.config.perp_matching_engine_address)?;
            info!(
                intent_id = %intent.intent_id,
                market_id = intent.market_id,
                buyer = %intent.buyer.0,
                seller = %intent.seller.0,
                price_1e8 = %intent.price_1e8,
                size_1e8 = %intent.size_1e8,
                "executor dry-run prepared placeholder call"
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
            prepared_calls,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionTickResult {
    pub pending_seen: usize,
    pub dry_run_updated: usize,
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
            created_at_ms: 123,
            status: ExecutionIntentStatus::Pending,
        };
        let repository = InMemoryExecutionRepository::with_intents(vec![intent]);
        let config = ExecutionConfig {
            execution_enabled: true,
            dry_run: true,
            poll_interval_ms: 1_000,
            max_batch_size: 10,
            rpc_url: None,
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000009",
            ),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
        };

        let result = Executor::new(config, repository.clone())
            .tick()
            .await
            .unwrap();

        assert_eq!(result.pending_seen, 1);
        assert_eq!(result.dry_run_updated, 1);
        assert_eq!(result.prepared_calls.len(), 1);
        assert_eq!(repository.all()[0].status, ExecutionIntentStatus::DryRun);
    }
}
