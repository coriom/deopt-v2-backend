use super::models::{
    execution_status_to_str, order_status_to_str, timestamp_to_i64, u64_to_i64, DbExecutionIntent,
    DbExecutionSimulation, DbOrder, DbTrade,
};
use super::pool;
use crate::engine::EngineEvent;
use crate::error::{BackendError, Result};
use crate::execution::{
    ExecutionIntent, ExecutionIntentRepository, ExecutionIntentStatus, SimulationResult,
    StoredTradeSignatures,
};
use crate::indexer::IndexedPerpTrade;
use crate::reconciliation::{
    normalize_onchain_intent_id, ExecutionReconciliation, ReconciliationCounts,
    ReconciliationStatus,
};
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
            "SELECT intent_id, onchain_intent_id, market_id, buyer, seller, price_1e8, size_1e8, \
             buy_order_id, sell_order_id, buyer_is_maker, buyer_nonce, seller_nonce, deadline_ms, \
             status, created_at_ms, updated_at_ms \
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
            "SELECT intent_id, onchain_intent_id, market_id, buyer, seller, price_1e8, size_1e8, \
             buy_order_id, sell_order_id, buyer_is_maker, buyer_nonce, seller_nonce, deadline_ms, \
             status, created_at_ms, updated_at_ms \
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

    pub async fn get_execution_intent(&self, intent_id: Uuid) -> Result<Option<ExecutionIntent>> {
        let row = sqlx::query(
            "SELECT intent_id, onchain_intent_id, market_id, buyer, seller, price_1e8, size_1e8, \
             buy_order_id, sell_order_id, buyer_is_maker, buyer_nonce, seller_nonce, deadline_ms, \
             status, created_at_ms, updated_at_ms \
             FROM execution_intents WHERE intent_id = $1",
        )
        .bind(intent_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        row.map(db_execution_intent_from_row)
            .transpose()?
            .map(ExecutionIntent::try_from)
            .transpose()
    }

    pub async fn get_execution_intent_signatures(
        &self,
        intent_id: Uuid,
    ) -> Result<StoredTradeSignatures> {
        let row = sqlx::query(
            "SELECT buyer_sig, seller_sig FROM execution_intent_signatures WHERE intent_id = $1",
        )
        .bind(intent_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        let Some(row) = row else {
            return Ok(StoredTradeSignatures::default());
        };
        Ok(StoredTradeSignatures {
            buyer_sig: row_get(&row, "buyer_sig")?,
            seller_sig: row_get(&row, "seller_sig")?,
        })
    }

    pub async fn upsert_execution_intent_signatures(
        &self,
        intent_id: Uuid,
        buyer_sig: Option<String>,
        seller_sig: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<StoredTradeSignatures> {
        let mut signatures = self.get_execution_intent_signatures(intent_id).await?;
        signatures.upsert(buyer_sig, seller_sig)?;

        sqlx::query(
            "INSERT INTO execution_intent_signatures
                (intent_id, buyer_sig, seller_sig, updated_at_ms)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (intent_id) DO UPDATE
             SET buyer_sig = EXCLUDED.buyer_sig,
                 seller_sig = EXCLUDED.seller_sig,
                 updated_at_ms = EXCLUDED.updated_at_ms",
        )
        .bind(intent_id.to_string())
        .bind(&signatures.buyer_sig)
        .bind(&signatures.seller_sig)
        .bind(timestamp_to_i64(updated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        Ok(signatures)
    }

    pub async fn persist_simulation_result(&self, result: &SimulationResult) -> Result<()> {
        let db_simulation = DbExecutionSimulation::try_from(result)?;
        let mut tx = self.begin().await?;
        insert_execution_simulation(&mut tx, &db_simulation).await?;
        update_execution_intent_status_tx(
            &mut tx,
            &result.intent_id.to_string(),
            result.status,
            result.created_at_ms,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))
    }

    pub async fn get_indexer_cursor(&self, name: &str) -> Result<Option<u64>> {
        let row = sqlx::query("SELECT last_indexed_block FROM indexer_cursors WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;

        row.map(|row| {
            let value: i64 = row_get(&row, "last_indexed_block")?;
            i64_to_u64_persistence("last_indexed_block", value)
        })
        .transpose()
    }

    pub async fn persist_indexed_perp_trades_and_cursor(
        &self,
        cursor_name: &str,
        trades: &[IndexedPerpTrade],
        last_indexed_block: u64,
    ) -> Result<u64> {
        let mut tx = self.begin().await?;
        let mut inserted = 0u64;
        for trade in trades {
            inserted += insert_indexed_perp_trade(&mut tx, trade).await?;
        }
        upsert_indexer_cursor(&mut tx, cursor_name, last_indexed_block, now_ms()).await?;
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(inserted)
    }

    pub async fn list_indexed_perp_trades(&self, limit: u32) -> Result<Vec<IndexedPerpTrade>> {
        let rows = sqlx::query(
            "SELECT event_id, tx_hash, log_index, block_number, block_hash, buyer, seller,
                    onchain_intent_id, market_id, size_delta_1e8, execution_price_1e8,
                    buyer_is_maker, buyer_nonce, seller_nonce, created_at_ms
             FROM indexed_perp_trades
             ORDER BY block_number DESC, log_index DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter().map(indexed_perp_trade_from_row).collect()
    }

    pub async fn list_unreconciled_indexed_perp_trades(
        &self,
        limit: u32,
    ) -> Result<Vec<IndexedPerpTrade>> {
        let rows = sqlx::query(
            "SELECT event_id, tx_hash, log_index, block_number, block_hash, buyer, seller,
                    onchain_intent_id, market_id, size_delta_1e8, execution_price_1e8,
                    buyer_is_maker, buyer_nonce, seller_nonce, created_at_ms
             FROM indexed_perp_trades indexed
             WHERE indexed.onchain_intent_id IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM execution_reconciliations reconciled
                   WHERE reconciled.indexed_event_id = indexed.event_id
               )
             ORDER BY block_number ASC, log_index ASC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter().map(indexed_perp_trade_from_row).collect()
    }

    pub async fn find_execution_intents_by_onchain_intent_id(
        &self,
        onchain_intent_id: &str,
    ) -> Result<Vec<ExecutionIntent>> {
        let Some(onchain_intent_id) = normalize_onchain_intent_id(onchain_intent_id) else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query(
            "SELECT intent_id, onchain_intent_id, market_id, buyer, seller, price_1e8, size_1e8,
                    buy_order_id, sell_order_id, buyer_is_maker, buyer_nonce, seller_nonce,
                    deadline_ms, status, created_at_ms, updated_at_ms
             FROM execution_intents
             WHERE onchain_intent_id = $1
             ORDER BY created_at_ms ASC, intent_id ASC",
        )
        .bind(onchain_intent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(db_execution_intent_from_row)
            .map(|result| result.and_then(ExecutionIntent::try_from))
            .collect()
    }

    pub async fn find_indexed_trades_by_onchain_intent_id(
        &self,
        onchain_intent_id: &str,
    ) -> Result<Vec<IndexedPerpTrade>> {
        let Some(onchain_intent_id) = normalize_onchain_intent_id(onchain_intent_id) else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query(
            "SELECT event_id, tx_hash, log_index, block_number, block_hash, buyer, seller,
                    onchain_intent_id, market_id, size_delta_1e8, execution_price_1e8,
                    buyer_is_maker, buyer_nonce, seller_nonce, created_at_ms
             FROM indexed_perp_trades
             WHERE onchain_intent_id = $1
             ORDER BY block_number ASC, log_index ASC",
        )
        .bind(onchain_intent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter().map(indexed_perp_trade_from_row).collect()
    }

    pub async fn insert_execution_reconciliation(
        &self,
        reconciliation: &ExecutionReconciliation,
    ) -> Result<u64> {
        insert_execution_reconciliation(&self.pool, reconciliation).await
    }

    pub async fn list_recent_reconciliations(
        &self,
        limit: u32,
    ) -> Result<Vec<ExecutionReconciliation>> {
        let rows = sqlx::query(
            "SELECT reconciliation_id, onchain_intent_id, intent_id, indexed_event_id, tx_hash,
                    block_number, log_index, status, created_at_ms
             FROM execution_reconciliations
             ORDER BY created_at_ms DESC, reconciliation_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(execution_reconciliation_from_row)
            .collect()
    }

    pub async fn get_reconciliations_for_intent(
        &self,
        intent_id: Uuid,
    ) -> Result<Vec<ExecutionReconciliation>> {
        let rows = sqlx::query(
            "SELECT reconciliation_id, onchain_intent_id, intent_id, indexed_event_id, tx_hash,
                    block_number, log_index, status, created_at_ms
             FROM execution_reconciliations
             WHERE intent_id = $1
             ORDER BY created_at_ms DESC, reconciliation_id DESC",
        )
        .bind(intent_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(execution_reconciliation_from_row)
            .collect()
    }

    pub async fn count_reconciliations_by_status(&self) -> Result<ReconciliationCounts> {
        let rows = sqlx::query(
            "SELECT status, COUNT(*) AS count
             FROM execution_reconciliations
             GROUP BY status",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        let mut counts = ReconciliationCounts::default();
        for row in rows {
            let status: String = row_get(&row, "status")?;
            let count: i64 = row_get(&row, "count")?;
            let count = i64_to_u64_persistence("count", count)?;
            match ReconciliationStatus::parse(&status)? {
                ReconciliationStatus::Matched => counts.matched = count,
                ReconciliationStatus::Ambiguous => counts.ambiguous = count,
                ReconciliationStatus::Unmatched => counts.unmatched = count,
                ReconciliationStatus::Ignored => counts.ignored = count,
            }
        }
        Ok(counts)
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

    fn get_execution_intent_signatures(
        &self,
        intent_id: Uuid,
    ) -> crate::execution::RepositoryFuture<'_, StoredTradeSignatures> {
        Box::pin(
            async move { PgRepository::get_execution_intent_signatures(self, intent_id).await },
        )
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

async fn update_execution_intent_status_tx(
    tx: &mut Transaction<'_, Postgres>,
    intent_id: &str,
    status: ExecutionIntentStatus,
    updated_at_ms: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE execution_intents
         SET status = $2, updated_at_ms = $3
         WHERE intent_id = $1",
    )
    .bind(intent_id)
    .bind(execution_status_to_str(status))
    .bind(timestamp_to_i64(updated_at_ms))
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_execution_simulation(
    tx: &mut Transaction<'_, Postgres>,
    simulation: &DbExecutionSimulation,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO execution_simulations (
            simulation_id, intent_id, status, block_number, error, created_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&simulation.simulation_id)
    .bind(&simulation.intent_id)
    .bind(&simulation.status)
    .bind(simulation.block_number)
    .bind(&simulation.error)
    .bind(simulation.created_at_ms)
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_indexed_perp_trade(
    tx: &mut Transaction<'_, Postgres>,
    trade: &IndexedPerpTrade,
) -> Result<u64> {
    let onchain_intent_id = trade
        .onchain_intent_id
        .as_deref()
        .and_then(normalize_onchain_intent_id);
    let result = sqlx::query(
        "INSERT INTO indexed_perp_trades (
            event_id, tx_hash, log_index, block_number, block_hash, buyer, seller,
            onchain_intent_id, market_id, size_delta_1e8, execution_price_1e8, buyer_is_maker,
            buyer_nonce, seller_nonce, created_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        ON CONFLICT (tx_hash, log_index) DO NOTHING",
    )
    .bind(&trade.event_id)
    .bind(&trade.tx_hash)
    .bind(u64_to_i64("log_index", trade.log_index)?)
    .bind(u64_to_i64("block_number", trade.block_number)?)
    .bind(&trade.block_hash)
    .bind(&trade.buyer)
    .bind(&trade.seller)
    .bind(&onchain_intent_id)
    .bind(&trade.market_id)
    .bind(&trade.size_delta_1e8)
    .bind(&trade.execution_price_1e8)
    .bind(trade.buyer_is_maker)
    .bind(&trade.buyer_nonce)
    .bind(&trade.seller_nonce)
    .bind(timestamp_to_i64(trade.created_at_ms))
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(result.rows_affected())
}

async fn upsert_indexer_cursor(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    last_indexed_block: u64,
    updated_at_ms: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO indexer_cursors (name, last_indexed_block, updated_at_ms)
         VALUES ($1, $2, $3)
         ON CONFLICT (name) DO UPDATE
         SET last_indexed_block = EXCLUDED.last_indexed_block,
             updated_at_ms = EXCLUDED.updated_at_ms",
    )
    .bind(name)
    .bind(u64_to_i64("last_indexed_block", last_indexed_block)?)
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
            intent_id, onchain_intent_id, market_id, buyer, seller, price_1e8, size_1e8,
            buy_order_id, sell_order_id, buyer_is_maker, buyer_nonce, seller_nonce, deadline_ms,
            status, created_at_ms, updated_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(&intent.intent_id)
    .bind(&intent.onchain_intent_id)
    .bind(intent.market_id)
    .bind(&intent.buyer)
    .bind(&intent.seller)
    .bind(&intent.price_1e8)
    .bind(&intent.size_1e8)
    .bind(&intent.buy_order_id)
    .bind(&intent.sell_order_id)
    .bind(intent.buyer_is_maker)
    .bind(intent.buyer_nonce)
    .bind(intent.seller_nonce)
    .bind(intent.deadline_ms)
    .bind(&intent.status)
    .bind(intent.created_at_ms)
    .bind(intent.updated_at_ms)
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_execution_reconciliation(
    pool: &PgPool,
    reconciliation: &ExecutionReconciliation,
) -> Result<u64> {
    let result = sqlx::query(
        "INSERT INTO execution_reconciliations (
            reconciliation_id, onchain_intent_id, intent_id, indexed_event_id, tx_hash,
            block_number, log_index, status, created_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (intent_id, indexed_event_id) DO NOTHING",
    )
    .bind(&reconciliation.reconciliation_id)
    .bind(&reconciliation.onchain_intent_id)
    .bind(&reconciliation.intent_id)
    .bind(&reconciliation.indexed_event_id)
    .bind(&reconciliation.tx_hash)
    .bind(u64_to_i64("block_number", reconciliation.block_number)?)
    .bind(u64_to_i64("log_index", reconciliation.log_index)?)
    .bind(reconciliation.status.as_str())
    .bind(timestamp_to_i64(reconciliation.created_at_ms))
    .execute(pool)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(result.rows_affected())
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
        onchain_intent_id: row_get(&row, "onchain_intent_id")?,
        market_id: row_get(&row, "market_id")?,
        buyer: row_get(&row, "buyer")?,
        seller: row_get(&row, "seller")?,
        price_1e8: row_get(&row, "price_1e8")?,
        size_1e8: row_get(&row, "size_1e8")?,
        buy_order_id: row_get(&row, "buy_order_id")?,
        sell_order_id: row_get(&row, "sell_order_id")?,
        buyer_is_maker: row_get(&row, "buyer_is_maker")?,
        buyer_nonce: row_get(&row, "buyer_nonce")?,
        seller_nonce: row_get(&row, "seller_nonce")?,
        deadline_ms: row_get(&row, "deadline_ms")?,
        status: row_get(&row, "status")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn indexed_perp_trade_from_row(row: PgRow) -> Result<IndexedPerpTrade> {
    let log_index: i64 = row_get(&row, "log_index")?;
    let block_number: i64 = row_get(&row, "block_number")?;
    let onchain_intent_id: Option<String> = row_get(&row, "onchain_intent_id")?;
    Ok(IndexedPerpTrade {
        event_id: row_get(&row, "event_id")?,
        tx_hash: row_get(&row, "tx_hash")?,
        log_index: i64_to_u64_persistence("log_index", log_index)?,
        block_number: i64_to_u64_persistence("block_number", block_number)?,
        block_hash: row_get(&row, "block_hash")?,
        onchain_intent_id: onchain_intent_id
            .as_deref()
            .and_then(normalize_onchain_intent_id),
        buyer: row_get(&row, "buyer")?,
        seller: row_get(&row, "seller")?,
        market_id: row_get(&row, "market_id")?,
        size_delta_1e8: row_get(&row, "size_delta_1e8")?,
        execution_price_1e8: row_get(&row, "execution_price_1e8")?,
        buyer_is_maker: row_get(&row, "buyer_is_maker")?,
        buyer_nonce: row_get(&row, "buyer_nonce")?,
        seller_nonce: row_get(&row, "seller_nonce")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
    })
}

fn execution_reconciliation_from_row(row: PgRow) -> Result<ExecutionReconciliation> {
    let block_number: i64 = row_get(&row, "block_number")?;
    let log_index: i64 = row_get(&row, "log_index")?;
    let status: String = row_get(&row, "status")?;
    Ok(ExecutionReconciliation {
        reconciliation_id: row_get(&row, "reconciliation_id")?,
        onchain_intent_id: row_get(&row, "onchain_intent_id")?,
        intent_id: row_get(&row, "intent_id")?,
        indexed_event_id: row_get(&row, "indexed_event_id")?,
        tx_hash: row_get(&row, "tx_hash")?,
        block_number: i64_to_u64_persistence("block_number", block_number)?,
        log_index: i64_to_u64_persistence("log_index", log_index)?,
        status: ReconciliationStatus::parse(&status)?,
        created_at_ms: row_get(&row, "created_at_ms")?,
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

fn i64_to_u64_persistence(field: &str, value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| BackendError::Persistence(format!("{field} cannot be negative")))
}
