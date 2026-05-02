use super::models::{
    execution_status_to_str, order_status_to_str, timestamp_to_i64, u64_to_i64, DbExecutionIntent,
    DbOrder, DbTrade,
};
use super::pool;
use crate::engine::EngineEvent;
use crate::error::{BackendError, Result};
use crate::execution::{ExecutionIntent, ExecutionIntentRepository, ExecutionIntentStatus};
use crate::signing::SignedOrder;
use crate::types::{now_ms, AccountId, OrderStatus, TimestampMs};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Clone)]
pub struct PgRepository {
    pool: PgPool,
}

impl PgRepository {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = pool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<()> {
        pool::run_migrations(&self.pool).await
    }

    pub async fn begin(&self) -> Result<Transaction<'_, Postgres>> {
        self.pool
            .begin()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))
    }

    pub async fn insert_nonce_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        account: &AccountId,
        nonce: u64,
        created_at_ms: i64,
    ) -> Result<()> {
        let result = sqlx::query(
            "INSERT INTO used_nonces (account, nonce, created_at_ms) VALUES ($1, $2, $3)",
        )
        .bind(&account.0)
        .bind(u64_to_i64("nonce", nonce)?)
        .bind(timestamp_to_i64(created_at_ms))
        .execute(&mut **tx)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(error) if is_unique_violation(&error) => Err(BackendError::NonceAlreadyUsed),
            Err(error) => Err(BackendError::Persistence(error.to_string())),
        }
    }

    pub async fn persist_submission_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        signed_order: &SignedOrder,
        events: &[EngineEvent],
    ) -> Result<()> {
        for event in events {
            match event {
                EngineEvent::OrderAccepted { order } => {
                    let db_order = DbOrder::from_order_and_signature(order, signed_order)?;
                    insert_order(tx, &db_order).await?;
                }
                EngineEvent::OrderPartiallyFilled { order }
                | EngineEvent::OrderFilled { order } => {
                    update_order_status(
                        tx,
                        &order.order_id.to_string(),
                        order.status,
                        order.remaining_size_1e8.to_string(),
                        now_ms(),
                    )
                    .await?;
                }
                EngineEvent::OrderCancelled { order } => {
                    update_order_status(
                        tx,
                        &order.order_id.to_string(),
                        OrderStatus::Cancelled,
                        order.remaining_size_1e8.to_string(),
                        now_ms(),
                    )
                    .await?;
                }
                EngineEvent::TradeMatched { trade } => {
                    let db_trade = DbTrade::try_from(trade)?;
                    insert_trade(tx, &db_trade).await?;
                }
                EngineEvent::ExecutionIntentCreated { intent } => {
                    let db_intent = DbExecutionIntent::try_from(intent)?;
                    insert_execution_intent(tx, &db_intent).await?;
                }
                EngineEvent::OrderRejected { .. } => {}
            }

            insert_engine_event(tx, event).await?;
        }

        Ok(())
    }

    pub async fn persist_engine_events(&self, events: &[EngineEvent]) -> Result<()> {
        let mut tx = self.begin().await?;
        for event in events {
            if let EngineEvent::OrderCancelled { order } = event {
                update_order_status(
                    &mut tx,
                    &order.order_id.to_string(),
                    OrderStatus::Cancelled,
                    order.remaining_size_1e8.to_string(),
                    now_ms(),
                )
                .await?;
            }
            insert_engine_event(&mut tx, event).await?;
        }
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))
    }

    pub async fn list_execution_intents(&self) -> Result<Vec<ExecutionIntent>> {
        let rows = sqlx::query(
            "SELECT intent_id, market_id, buyer, seller, price_1e8, size_1e8, \
             buy_order_id, sell_order_id, status, created_at_ms, updated_at_ms \
             FROM execution_intents ORDER BY created_at_ms ASC, intent_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(db_execution_intent_from_row)
            .map(|result| result.and_then(ExecutionIntent::try_from))
            .collect()
    }

    pub async fn list_pending_execution_intents(&self, limit: u32) -> Result<Vec<ExecutionIntent>> {
        let rows = sqlx::query(
            "SELECT intent_id, market_id, buyer, seller, price_1e8, size_1e8, \
             buy_order_id, sell_order_id, status, created_at_ms, updated_at_ms \
             FROM execution_intents \
             WHERE status = 'pending' \
             ORDER BY created_at_ms ASC, intent_id ASC \
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(db_execution_intent_from_row)
            .map(|result| result.and_then(ExecutionIntent::try_from))
            .collect()
    }

    pub async fn update_execution_intent_status(
        &self,
        intent_id: Uuid,
        status: ExecutionIntentStatus,
        updated_at_ms: TimestampMs,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE execution_intents
             SET status = $2, updated_at_ms = $3
             WHERE intent_id = $1",
        )
        .bind(intent_id.to_string())
        .bind(execution_status_to_str(status))
        .bind(timestamp_to_i64(updated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(())
    }
}

impl ExecutionIntentRepository for PgRepository {
    fn list_pending_execution_intents(
        &self,
        limit: u32,
    ) -> crate::execution::RepositoryFuture<'_, Vec<ExecutionIntent>> {
        Box::pin(async move { PgRepository::list_pending_execution_intents(self, limit).await })
    }

    fn update_execution_intent_status(
        &self,
        intent_id: Uuid,
        status: ExecutionIntentStatus,
        updated_at_ms: TimestampMs,
    ) -> crate::execution::RepositoryFuture<'_, ()> {
        Box::pin(async move {
            PgRepository::update_execution_intent_status(self, intent_id, status, updated_at_ms)
                .await
        })
    }
}

async fn insert_order(tx: &mut Transaction<'_, Postgres>, order: &DbOrder) -> Result<()> {
    let result = sqlx::query(
        "INSERT INTO orders (
            order_id, market_id, account, side, order_type, time_in_force,
            price_1e8, size_1e8, remaining_size_1e8, reduce_only, post_only,
            client_order_id, nonce, deadline_ms, signature, status, created_at_ms, updated_at_ms
        ) VALUES (
            $1, $2, $3, $4, $5, $6,
            $7, $8, $9, $10, $11,
            $12, $13, $14, $15, $16, $17, $18
        )",
    )
    .bind(&order.order_id)
    .bind(order.market_id)
    .bind(&order.account)
    .bind(&order.side)
    .bind(&order.order_type)
    .bind(&order.time_in_force)
    .bind(&order.price_1e8)
    .bind(&order.size_1e8)
    .bind(&order.remaining_size_1e8)
    .bind(order.reduce_only)
    .bind(order.post_only)
    .bind(&order.client_order_id)
    .bind(order.nonce)
    .bind(order.deadline_ms)
    .bind(&order.signature)
    .bind(&order.status)
    .bind(order.created_at_ms)
    .bind(order.updated_at_ms)
    .execute(&mut **tx)
    .await;

    match result {
        Ok(_) => Ok(()),
        Err(error) if is_unique_violation(&error) => Err(BackendError::NonceAlreadyUsed),
        Err(error) => Err(BackendError::Persistence(error.to_string())),
    }
}

async fn update_order_status(
    tx: &mut Transaction<'_, Postgres>,
    order_id: &str,
    status: OrderStatus,
    remaining_size_1e8: String,
    updated_at_ms: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE orders
         SET status = $2, remaining_size_1e8 = $3, updated_at_ms = $4
         WHERE order_id = $1",
    )
    .bind(order_id)
    .bind(order_status_to_str(status))
    .bind(remaining_size_1e8)
    .bind(timestamp_to_i64(updated_at_ms))
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_trade(tx: &mut Transaction<'_, Postgres>, trade: &DbTrade) -> Result<()> {
    sqlx::query(
        "INSERT INTO trades (
            trade_id, market_id, maker_order_id, taker_order_id, maker_account, taker_account,
            price_1e8, size_1e8, buyer, seller, created_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&trade.trade_id)
    .bind(trade.market_id)
    .bind(&trade.maker_order_id)
    .bind(&trade.taker_order_id)
    .bind(&trade.maker_account)
    .bind(&trade.taker_account)
    .bind(&trade.price_1e8)
    .bind(&trade.size_1e8)
    .bind(&trade.buyer)
    .bind(&trade.seller)
    .bind(trade.created_at_ms)
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_execution_intent(
    tx: &mut Transaction<'_, Postgres>,
    intent: &DbExecutionIntent,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO execution_intents (
            intent_id, market_id, buyer, seller, price_1e8, size_1e8,
            buy_order_id, sell_order_id, status, created_at_ms, updated_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&intent.intent_id)
    .bind(intent.market_id)
    .bind(&intent.buyer)
    .bind(&intent.seller)
    .bind(&intent.price_1e8)
    .bind(&intent.size_1e8)
    .bind(&intent.buy_order_id)
    .bind(&intent.sell_order_id)
    .bind(&intent.status)
    .bind(intent.created_at_ms)
    .bind(intent.updated_at_ms)
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_engine_event(
    tx: &mut Transaction<'_, Postgres>,
    event: &EngineEvent,
) -> Result<()> {
    let payload_json = serde_json::to_string(event)
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
    sqlx::query(
        "INSERT INTO engine_events (event_id, event_type, payload_json, created_at_ms)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(engine_event_type(event))
    .bind(payload_json)
    .bind(now_ms())
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

fn engine_event_type(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::OrderAccepted { .. } => "order_accepted",
        EngineEvent::OrderRejected { .. } => "order_rejected",
        EngineEvent::OrderCancelled { .. } => "order_cancelled",
        EngineEvent::OrderPartiallyFilled { .. } => "order_partially_filled",
        EngineEvent::OrderFilled { .. } => "order_filled",
        EngineEvent::TradeMatched { .. } => "trade_matched",
        EngineEvent::ExecutionIntentCreated { .. } => "execution_intent_created",
    }
}

fn db_execution_intent_from_row(row: PgRow) -> Result<DbExecutionIntent> {
    Ok(DbExecutionIntent {
        intent_id: row_get(&row, "intent_id")?,
        market_id: row_get(&row, "market_id")?,
        buyer: row_get(&row, "buyer")?,
        seller: row_get(&row, "seller")?,
        price_1e8: row_get(&row, "price_1e8")?,
        size_1e8: row_get(&row, "size_1e8")?,
        buy_order_id: row_get(&row, "buy_order_id")?,
        sell_order_id: row_get(&row, "sell_order_id")?,
        status: row_get(&row, "status")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn row_get<T>(row: &PgRow, column: &str) -> Result<T>
where
    for<'r> T: sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    row.try_get(column)
        .map_err(|error| BackendError::Persistence(error.to_string()))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database_error| database_error.code())
        .as_deref()
        == Some("23505")
}
