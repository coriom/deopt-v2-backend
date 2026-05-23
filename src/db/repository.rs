use super::models::{
    execution_status_to_str, order_status_to_str, timestamp_to_i64, u64_to_i64, DbExecutionIntent,
    DbExecutionSimulation, DbOrder, DbTrade,
};
use super::pool;
use crate::confirmation::{ConfirmationDecision, ConfirmationStatus};
use crate::engine::EngineEvent;
use crate::error::{BackendError, Result};
use crate::execution::{
    ExecutionIntent, ExecutionIntentRepository, ExecutionIntentStatus, ExecutionTransaction,
    ExecutionTransactionStatus, SimulationResult, StoredTradeSignatures,
};
use crate::fees::{FeeEvent, FeeFlowType, FeeMarketType, RebateAccrual, VolumeBucket};
use crate::indexer::IndexedPerpTrade;
use crate::mm::{MmAccountPermissions, MmProductPermission};
use crate::monitoring::FeeEventLabels;
use crate::options::store::status_for_remaining;
use crate::options::{
    OptionExecutionConfirmationStatus, OptionExecutionGasCheckStatus, OptionExecutionIntent,
    OptionExecutionIntentId, OptionExecutionIntentStatus, OptionExecutionSimulationResult,
    OptionExecutionSimulationStatus, OptionExecutionSourceType, OptionExecutionTransaction,
    OptionFill, OptionFillId, OptionOrder, OptionOrderId, OptionOrderStatus, OptionRfqFill,
    OptionRfqFillId, OptionRfqId, OptionRfqQuote, OptionRfqQuoteId, OptionRfqQuoteSignatureStatus,
    OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus, OptionSeries, OptionSeriesSource,
    OptionSeriesStatus,
};
use crate::reconciliation::{
    normalize_onchain_intent_id, ExecutionReconciliation, ReconciliationCounts,
    ReconciliationStatus,
};
use crate::rfq::{
    QuoteId, RfqId, RfqQuote, RfqQuoteSignatureStatus, RfqQuoteStatus, RfqRequest, RfqStatus,
};
use crate::signing::SignedOrder;
use crate::types::{now_ms, AccountId, OrderStatus, Side, TimeInForce, TimestampMs};
use sqlx::postgres::{PgArguments, PgPool, PgRow};
use sqlx::query::Query;
use sqlx::{Postgres, Row, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone)]
pub struct PgRepository {
    pool: PgPool,
}

const ADMIN_TABLE_COUNTS: &[(&str, &str)] = &[
    ("orders", "orders"),
    ("used_nonces", "used_nonces"),
    ("execution_intents", "execution_intents"),
    ("execution_simulations", "execution_simulations"),
    ("execution_transactions", "execution_transactions"),
    ("indexed_perp_trades", "indexed_perp_trades"),
    ("reconciliations", "execution_reconciliations"),
    ("rfqs", "rfqs"),
    ("rfq_quotes", "rfq_quotes"),
    ("option_series", "option_series"),
    ("option_orders", "option_orders"),
    ("option_fills", "option_fills"),
    ("option_rfqs", "option_rfqs"),
    ("option_rfq_quotes", "option_rfq_quotes"),
    ("option_rfq_fills", "option_rfq_fills"),
    ("option_execution_intents", "option_execution_intents"),
    (
        "option_execution_transactions",
        "option_execution_transactions",
    ),
    ("mm_accounts", "mm_accounts"),
    ("mm_market_permissions", "mm_market_permissions"),
    ("fee_events", "fee_events"),
    ("volume_buckets", "volume_buckets"),
    ("rebate_accruals", "rebate_accruals"),
];

fn validate_admin_identifier(identifier: &str) -> Result<()> {
    if identifier
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Ok(())
    } else {
        Err(BackendError::Config(format!(
            "invalid admin SQL identifier: {identifier}"
        )))
    }
}

impl PgRepository {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = pool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<()> {
        pool::run_migrations(&self.pool).await
    }

    pub async fn admin_ping(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(())
    }

    pub async fn admin_migration_status(&self) -> Result<serde_json::Value> {
        if !self.admin_table_exists("_sqlx_migrations").await? {
            return Ok(serde_json::json!({
                "available": false,
                "installed_count": 0,
                "latest_version": null
            }));
        }
        let row = sqlx::query(
            "SELECT COUNT(*) AS installed_count, MAX(version) AS latest_version
             FROM _sqlx_migrations",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let installed_count: i64 = row_get(&row, "installed_count")?;
        let latest_version: Option<i64> = row_get(&row, "latest_version")?;
        Ok(serde_json::json!({
            "available": true,
            "installed_count": i64_to_u64_persistence("installed_count", installed_count)?,
            "latest_version": latest_version
        }))
    }

    pub async fn admin_table_counts(&self) -> Result<BTreeMap<String, serde_json::Value>> {
        let mut counts = BTreeMap::new();
        for (name, table) in ADMIN_TABLE_COUNTS {
            let value = match self.admin_count_table_if_exists(table).await? {
                Some(count) => serde_json::json!({
                    "available": true,
                    "count": count
                }),
                None => serde_json::json!({
                    "available": false,
                    "count": null
                }),
            };
            counts.insert((*name).to_string(), value);
        }
        Ok(counts)
    }

    pub async fn admin_count_by_column(
        &self,
        table: &'static str,
        column: &'static str,
    ) -> Result<BTreeMap<String, u64>> {
        validate_admin_identifier(table)?;
        validate_admin_identifier(column)?;
        if !self.admin_table_exists(table).await? {
            return Ok(BTreeMap::new());
        }
        let rows = sqlx::query(&format!(
            "SELECT {column} AS bucket, COUNT(*) AS count FROM {table} GROUP BY {column}"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let mut counts = BTreeMap::new();
        for row in rows {
            let bucket: Option<String> = row_get(&row, "bucket")?;
            let count: i64 = row_get(&row, "count")?;
            counts.insert(
                bucket.unwrap_or_else(|| "null".to_string()),
                i64_to_u64_persistence("count", count)?,
            );
        }
        Ok(counts)
    }

    pub async fn admin_count_where(
        &self,
        table: &'static str,
        where_clause: &'static str,
    ) -> Result<u64> {
        validate_admin_identifier(table)?;
        if !self.admin_table_exists(table).await? {
            return Ok(0);
        }
        let row = sqlx::query(&format!(
            "SELECT COUNT(*) AS count FROM {table} WHERE {where_clause}"
        ))
        .fetch_one(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let count: i64 = row_get(&row, "count")?;
        i64_to_u64_persistence("count", count)
    }

    pub async fn get_mm_permission_account(
        &self,
        account: &AccountId,
    ) -> Result<Option<MmAccountPermissions>> {
        if !self.admin_table_exists("mm_accounts").await? {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT mm_account, enabled, label, can_submit_perp_orders, can_quote_perp_rfq,
                    can_quote_option_rfq, can_submit_option_orders, created_at_ms, updated_at_ms
             FROM mm_accounts
             WHERE lower(mm_account) = lower($1)",
        )
        .bind(&account.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(mm_permission_account_from_row).transpose()
    }

    pub async fn list_mm_permission_accounts(&self) -> Result<Vec<MmAccountPermissions>> {
        if !self.admin_table_exists("mm_accounts").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT mm_account, enabled, label, can_submit_perp_orders, can_quote_perp_rfq,
                    can_quote_option_rfq, can_submit_option_orders, created_at_ms, updated_at_ms
             FROM mm_accounts
             ORDER BY lower(mm_account) ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(mm_permission_account_from_row)
            .collect()
    }

    pub async fn list_mm_product_permissions_for_account(
        &self,
        account: &AccountId,
    ) -> Result<Vec<MmProductPermission>> {
        if !self.admin_table_exists("mm_market_permissions").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT id, mm_account, market_id, option_series_id, enabled, created_at_ms, updated_at_ms
             FROM mm_market_permissions
             WHERE lower(mm_account) = lower($1)
             ORDER BY created_at_ms ASC, id ASC",
        )
        .bind(&account.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(mm_product_permission_from_row)
            .collect()
    }

    pub async fn list_mm_product_permissions(&self) -> Result<Vec<MmProductPermission>> {
        if !self.admin_table_exists("mm_market_permissions").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT id, mm_account, market_id, option_series_id, enabled, created_at_ms, updated_at_ms
             FROM mm_market_permissions
             ORDER BY lower(mm_account) ASC, created_at_ms ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(mm_product_permission_from_row)
            .collect()
    }

    pub async fn insert_fee_event(&self, event: &FeeEvent) -> Result<bool> {
        let market_id = event
            .market_id
            .map(|value| u64_to_i64("market_id", value))
            .transpose()?;
        let maker = event.maker.as_ref().map(|account| account.0.as_str());
        let taker = event.taker.as_ref().map(|account| account.0.as_str());
        let result = sqlx::query(
            "INSERT INTO fee_events (
                fee_event_id, source_type, source_id, market_type, flow_type, market_id,
                option_series_id, maker, taker, payer, recipient, fee_asset, notional_1e8,
                fee_rate_micro_bps, fee_amount_1e8, rebate_rate_micro_bps, rebate_amount_1e8,
                protocol_amount_1e8, status, created_at_ms
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19, $20
            )
            ON CONFLICT (source_type, source_id, payer, recipient) DO NOTHING",
        )
        .bind(&event.fee_event_id)
        .bind(event.source_type.as_str())
        .bind(&event.source_id)
        .bind(event.market_type.as_str())
        .bind(event.flow_type.as_str())
        .bind(market_id)
        .bind(&event.option_series_id)
        .bind(maker)
        .bind(taker)
        .bind(&event.payer.0)
        .bind(&event.recipient)
        .bind(&event.fee_asset)
        .bind(event.notional_1e8.to_string())
        .bind(u64_to_i64("fee_rate_micro_bps", event.fee_rate_micro_bps)?)
        .bind(event.fee_amount_1e8.to_string())
        .bind(u64_to_i64(
            "rebate_rate_micro_bps",
            event.rebate_rate_micro_bps,
        )?)
        .bind(event.rebate_amount_1e8.to_string())
        .bind(event.protocol_amount_1e8.to_string())
        .bind(event.status.as_str())
        .bind(timestamp_to_i64(event.created_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(result.rows_affected() == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_fee_volume_delta(
        &self,
        account: &AccountId,
        bucket_day: &str,
        market_type: FeeMarketType,
        maker_delta_1e8: u128,
        taker_delta_1e8: u128,
        updated_at_ms: TimestampMs,
    ) -> Result<VolumeBucket> {
        let total_delta_1e8 = maker_delta_1e8
            .checked_add(taker_delta_1e8)
            .ok_or_else(|| BackendError::Config("fee volume delta overflow".to_string()))?;
        let row = sqlx::query(
            "INSERT INTO volume_buckets (
                bucket_id, account, bucket_day, market_type, maker_volume_1e8,
                taker_volume_1e8, total_volume_1e8, updated_at_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (account, bucket_day, market_type) DO UPDATE
            SET maker_volume_1e8 =
                    (volume_buckets.maker_volume_1e8::numeric + EXCLUDED.maker_volume_1e8::numeric)::text,
                taker_volume_1e8 =
                    (volume_buckets.taker_volume_1e8::numeric + EXCLUDED.taker_volume_1e8::numeric)::text,
                total_volume_1e8 =
                    (volume_buckets.total_volume_1e8::numeric + EXCLUDED.total_volume_1e8::numeric)::text,
                updated_at_ms = EXCLUDED.updated_at_ms
            RETURNING bucket_id, account, bucket_day, market_type, maker_volume_1e8,
                      taker_volume_1e8, total_volume_1e8, updated_at_ms",
        )
        .bind(VolumeBucket::key(account, bucket_day, market_type))
        .bind(&account.0)
        .bind(bucket_day)
        .bind(market_type.as_str())
        .bind(maker_delta_1e8.to_string())
        .bind(taker_delta_1e8.to_string())
        .bind(total_delta_1e8.to_string())
        .bind(timestamp_to_i64(updated_at_ms))
        .fetch_one(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        volume_bucket_from_row(row)
    }

    pub async fn fee_rolling_volume_since(
        &self,
        account: &AccountId,
        market_type: FeeMarketType,
        start_bucket_day: &str,
    ) -> Result<u128> {
        if !self.admin_table_exists("volume_buckets").await? {
            return Ok(0);
        }
        let row = sqlx::query(
            "SELECT COALESCE(SUM(total_volume_1e8::numeric), 0)::text AS total_volume_1e8
             FROM volume_buckets
             WHERE lower(account) = lower($1)
               AND market_type = $2
               AND bucket_day >= $3",
        )
        .bind(&account.0)
        .bind(market_type.as_str())
        .bind(start_bucket_day)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row_get::<String>(&row, "total_volume_1e8")?
            .parse()
            .map_err(|error| BackendError::Persistence(format!("invalid fee volume: {error}")))
    }

    pub async fn insert_rebate_accrual(&self, rebate: &RebateAccrual) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO rebate_accruals (
                rebate_id, fee_event_id, account, source_type, source_id, rebate_asset,
                rebate_amount_1e8, status, created_at_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (fee_event_id, account) DO NOTHING",
        )
        .bind(&rebate.rebate_id)
        .bind(&rebate.fee_event_id)
        .bind(&rebate.account.0)
        .bind(rebate.source_type.as_str())
        .bind(&rebate.source_id)
        .bind(&rebate.rebate_asset)
        .bind(rebate.rebate_amount_1e8.to_string())
        .bind(rebate.status.as_str())
        .bind(timestamp_to_i64(rebate.created_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn admin_recent_execution_intents(
        &self,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("execution_intents").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT intent_id, market_id, buyer, seller, price_1e8, size_1e8, status, created_at_ms
             FROM execution_intents
             ORDER BY created_at_ms DESC, intent_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "intent_id": row_get::<String>(&row, "intent_id")?,
                    "market_id": row_get::<i64>(&row, "market_id")?,
                    "buyer": row_get::<String>(&row, "buyer")?,
                    "seller": row_get::<String>(&row, "seller")?,
                    "price_1e8": row_get::<String>(&row, "price_1e8")?,
                    "size_1e8": row_get::<String>(&row, "size_1e8")?,
                    "status": row_get::<String>(&row, "status")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_execution_simulations(
        &self,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("execution_simulations").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT simulation_id, intent_id, status, error, decoded_error, created_at_ms
             FROM execution_simulations
             ORDER BY created_at_ms DESC, simulation_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "simulation_id": row_get::<String>(&row, "simulation_id")?,
                    "intent_id": row_get::<String>(&row, "intent_id")?,
                    "status": row_get::<String>(&row, "status")?,
                    "error": row_get::<Option<String>>(&row, "error")?,
                    "decoded_error": row_get::<Option<String>>(&row, "decoded_error")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_failed_simulations(
        &self,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("execution_simulations").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT simulation_id, intent_id, status, error, decoded_error, created_at_ms
             FROM execution_simulations
             WHERE status = 'simulation_failed'
             ORDER BY created_at_ms DESC, simulation_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "simulation_id": row_get::<String>(&row, "simulation_id")?,
                    "intent_id": row_get::<String>(&row, "intent_id")?,
                    "error": row_get::<Option<String>>(&row, "error")?,
                    "decoded_error": row_get::<Option<String>>(&row, "decoded_error")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_confirmation_errors(
        &self,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("execution_transactions").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT transaction_id, intent_id, tx_hash, confirmation_status, confirmation_error,
                    created_at_ms, updated_at_ms
             FROM execution_transactions
             WHERE confirmation_error IS NOT NULL
             ORDER BY updated_at_ms DESC, transaction_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "transaction_id": row_get::<String>(&row, "transaction_id")?,
                    "intent_id": row_get::<String>(&row, "intent_id")?,
                    "tx_hash": row_get::<Option<String>>(&row, "tx_hash")?,
                    "confirmation_status": row_get::<Option<String>>(&row, "confirmation_status")?,
                    "confirmation_error": row_get::<Option<String>>(&row, "confirmation_error")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?,
                    "updated_at_ms": row_get::<i64>(&row, "updated_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_rfqs(&self, limit: u32) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("rfqs").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT rfq_id, taker, market_id, side, size_1e8, limit_price_1e8, status,
                    accepted_quote_id, execution_intent_id, created_at_ms, expires_at_ms
             FROM rfqs
             ORDER BY created_at_ms DESC, rfq_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "rfq_id": row_get::<String>(&row, "rfq_id")?,
                    "taker": row_get::<String>(&row, "taker")?,
                    "market_id": row_get::<i64>(&row, "market_id")?,
                    "side": row_get::<String>(&row, "side")?,
                    "size_1e8": row_get::<String>(&row, "size_1e8")?,
                    "limit_price_1e8": row_get::<Option<String>>(&row, "limit_price_1e8")?,
                    "status": row_get::<String>(&row, "status")?,
                    "accepted_quote_id": row_get::<Option<String>>(&row, "accepted_quote_id")?,
                    "execution_intent_id": row_get::<Option<String>>(&row, "execution_intent_id")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?,
                    "expires_at_ms": row_get::<i64>(&row, "expires_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_option_rfqs(&self, limit: u32) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("option_rfqs").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT option_rfq_id, taker, option_series_id, side, size_1e8, limit_price_1e8,
                    status, accepted_quote_id, option_fill_id, created_at_ms, expires_at_ms
             FROM option_rfqs
             ORDER BY created_at_ms DESC, option_rfq_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "option_rfq_id": row_get::<String>(&row, "option_rfq_id")?,
                    "taker": row_get::<String>(&row, "taker")?,
                    "option_series_id": row_get::<String>(&row, "option_series_id")?,
                    "side": row_get::<String>(&row, "side")?,
                    "size_1e8": row_get::<String>(&row, "size_1e8")?,
                    "limit_price_1e8": row_get::<Option<String>>(&row, "limit_price_1e8")?,
                    "status": row_get::<String>(&row, "status")?,
                    "accepted_quote_id": row_get::<Option<String>>(&row, "accepted_quote_id")?,
                    "option_fill_id": row_get::<Option<String>>(&row, "option_fill_id")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?,
                    "expires_at_ms": row_get::<i64>(&row, "expires_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_option_fills(&self, limit: u32) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("option_fills").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT fill_id, option_series_id, buyer, seller, taker_side, price_1e8, size_1e8,
                    created_at_ms
             FROM option_fills
             ORDER BY created_at_ms DESC, fill_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "fill_id": row_get::<String>(&row, "fill_id")?,
                    "option_series_id": row_get::<String>(&row, "option_series_id")?,
                    "buyer": row_get::<String>(&row, "buyer")?,
                    "seller": row_get::<String>(&row, "seller")?,
                    "taker_side": row_get::<String>(&row, "taker_side")?,
                    "price_1e8": row_get::<String>(&row, "price_1e8")?,
                    "size_1e8": row_get::<String>(&row, "size_1e8")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_option_rfq_fills(
        &self,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("option_rfq_fills").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT fill_id, option_rfq_id, quote_id, option_series_id, buyer, seller, taker,
                    mm_account, taker_side, price_1e8, size_1e8, created_at_ms
             FROM option_rfq_fills
             ORDER BY created_at_ms DESC, fill_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "fill_id": row_get::<String>(&row, "fill_id")?,
                    "option_rfq_id": row_get::<String>(&row, "option_rfq_id")?,
                    "quote_id": row_get::<String>(&row, "quote_id")?,
                    "option_series_id": row_get::<String>(&row, "option_series_id")?,
                    "buyer": row_get::<String>(&row, "buyer")?,
                    "seller": row_get::<String>(&row, "seller")?,
                    "taker": row_get::<String>(&row, "taker")?,
                    "mm_account": row_get::<String>(&row, "mm_account")?,
                    "taker_side": row_get::<String>(&row, "taker_side")?,
                    "price_1e8": row_get::<String>(&row, "price_1e8")?,
                    "size_1e8": row_get::<String>(&row, "size_1e8")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_recent_option_execution_intents(
        &self,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("option_execution_intents").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT intent_id, onchain_intent_id, source_type, source_id, option_series_id,
                    onchain_option_id, buyer, seller, quantity_contracts,
                    premium_per_contract_native, buyer_is_maker, status, calldata,
                    simulation_status, simulation_error, simulation_block_number,
                    simulation_revert_data, simulation_revert_selector, simulated_at_ms,
                    created_at_ms, updated_at_ms
             FROM option_execution_intents
             ORDER BY created_at_ms DESC, intent_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "intent_id": row_get::<String>(&row, "intent_id")?,
                    "onchain_intent_id": row_get::<String>(&row, "onchain_intent_id")?,
                    "source_type": row_get::<String>(&row, "source_type")?,
                    "source_id": row_get::<String>(&row, "source_id")?,
                    "option_series_id": row_get::<String>(&row, "option_series_id")?,
                    "onchain_option_id": row_get::<Option<String>>(&row, "onchain_option_id")?,
                    "buyer": row_get::<String>(&row, "buyer")?,
                    "seller": row_get::<String>(&row, "seller")?,
                    "quantity_contracts": row_get::<String>(&row, "quantity_contracts")?,
                    "premium_per_contract_native": row_get::<String>(&row, "premium_per_contract_native")?,
                    "buyer_is_maker": row_get::<bool>(&row, "buyer_is_maker")?,
                    "status": row_get::<String>(&row, "status")?,
                    "calldata_ready": row_get::<Option<String>>(&row, "calldata")?.is_some(),
                    "simulation_status": row_get::<Option<String>>(&row, "simulation_status")?,
                    "simulation_error": row_get::<Option<String>>(&row, "simulation_error")?,
                    "simulation_block_number": row_get::<Option<i64>>(&row, "simulation_block_number")?,
                    "simulation_revert_data": row_get::<Option<String>>(&row, "simulation_revert_data")?,
                    "simulation_revert_selector": row_get::<Option<String>>(&row, "simulation_revert_selector")?,
                    "simulated_at_ms": row_get::<Option<i64>>(&row, "simulated_at_ms")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?,
                    "updated_at_ms": row_get::<i64>(&row, "updated_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_fee_summary(&self) -> Result<serde_json::Value> {
        if !self.admin_table_exists("fee_events").await? {
            return Ok(serde_json::json!({
                "event_count": 0,
                "fee_total_1e8": "0",
                "rebate_total_1e8": "0",
                "protocol_total_1e8": "0",
                "status_counts": {},
                "source_type_counts": {},
                "market_type_counts": {},
                "flow_type_counts": {}
            }));
        }
        let row = sqlx::query(
            "SELECT COUNT(*) AS event_count,
                    COALESCE(SUM(fee_amount_1e8::numeric), 0)::text AS fee_total_1e8,
                    COALESCE(SUM(rebate_amount_1e8::numeric), 0)::text AS rebate_total_1e8,
                    COALESCE(SUM(protocol_amount_1e8::numeric), 0)::text AS protocol_total_1e8
             FROM fee_events",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let event_count: i64 = row_get(&row, "event_count")?;
        Ok(serde_json::json!({
            "event_count": i64_to_u64_persistence("event_count", event_count)?,
            "fee_total_1e8": row_get::<String>(&row, "fee_total_1e8")?,
            "rebate_total_1e8": row_get::<String>(&row, "rebate_total_1e8")?,
            "protocol_total_1e8": row_get::<String>(&row, "protocol_total_1e8")?,
            "status_counts": self.admin_count_by_column("fee_events", "status").await?,
            "source_type_counts": self.admin_count_by_column("fee_events", "source_type").await?,
            "market_type_counts": self.admin_count_by_column("fee_events", "market_type").await?,
            "flow_type_counts": self.admin_count_by_column("fee_events", "flow_type").await?
        }))
    }

    pub async fn admin_fee_event_label_counts(&self) -> Result<BTreeMap<FeeEventLabels, u64>> {
        if !self.admin_table_exists("fee_events").await? {
            return Ok(BTreeMap::new());
        }
        let rows = sqlx::query(
            "SELECT market_type, flow_type, source_type, status, COUNT(*) AS count
             FROM fee_events
             GROUP BY market_type, flow_type, source_type, status
             ORDER BY market_type ASC, flow_type ASC, source_type ASC, status ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let mut counts = BTreeMap::new();
        for row in rows {
            let count: i64 = row_get(&row, "count")?;
            counts.insert(
                FeeEventLabels {
                    market_type: row_get(&row, "market_type")?,
                    flow_type: row_get(&row, "flow_type")?,
                    source_type: row_get(&row, "source_type")?,
                    status: row_get(&row, "status")?,
                },
                i64_to_u64_persistence("count", count)?,
            );
        }
        Ok(counts)
    }

    pub async fn admin_recent_fee_events(&self, limit: u32) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("fee_events").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT fee_event_id, source_type, source_id, market_type, flow_type, market_id,
                    option_series_id, maker, taker, payer, recipient, fee_asset, notional_1e8,
                    fee_rate_micro_bps, fee_amount_1e8, rebate_rate_micro_bps,
                    rebate_amount_1e8, protocol_amount_1e8, status, created_at_ms
             FROM fee_events
             ORDER BY created_at_ms DESC, fee_event_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "fee_event_id": row_get::<String>(&row, "fee_event_id")?,
                    "source_type": row_get::<String>(&row, "source_type")?,
                    "source_id": row_get::<String>(&row, "source_id")?,
                    "market_type": row_get::<String>(&row, "market_type")?,
                    "flow_type": row_get::<String>(&row, "flow_type")?,
                    "market_id": row_get::<Option<i64>>(&row, "market_id")?,
                    "option_series_id": row_get::<Option<String>>(&row, "option_series_id")?,
                    "maker": row_get::<Option<String>>(&row, "maker")?,
                    "taker": row_get::<Option<String>>(&row, "taker")?,
                    "payer": row_get::<String>(&row, "payer")?,
                    "recipient": row_get::<String>(&row, "recipient")?,
                    "fee_asset": row_get::<String>(&row, "fee_asset")?,
                    "notional_1e8": row_get::<String>(&row, "notional_1e8")?,
                    "fee_rate_micro_bps": row_get::<i64>(&row, "fee_rate_micro_bps")?,
                    "fee_amount_1e8": row_get::<String>(&row, "fee_amount_1e8")?,
                    "rebate_rate_micro_bps": row_get::<i64>(&row, "rebate_rate_micro_bps")?,
                    "rebate_amount_1e8": row_get::<String>(&row, "rebate_amount_1e8")?,
                    "protocol_amount_1e8": row_get::<String>(&row, "protocol_amount_1e8")?,
                    "status": row_get::<String>(&row, "status")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_fee_volumes(
        &self,
        account: Option<&AccountId>,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("volume_buckets").await? {
            return Ok(Vec::new());
        }
        let rows = if let Some(account) = account {
            sqlx::query(
                "SELECT bucket_id, account, bucket_day, market_type, maker_volume_1e8,
                        taker_volume_1e8, total_volume_1e8, updated_at_ms
                 FROM volume_buckets
                 WHERE lower(account) = lower($1)
                 ORDER BY bucket_day DESC, lower(account) ASC, market_type ASC",
            )
            .bind(&account.0)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT bucket_id, account, bucket_day, market_type, maker_volume_1e8,
                        taker_volume_1e8, total_volume_1e8, updated_at_ms
                 FROM volume_buckets
                 ORDER BY bucket_day DESC, lower(account) ASC, market_type ASC",
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "bucket_id": row_get::<String>(&row, "bucket_id")?,
                    "account": row_get::<String>(&row, "account")?,
                    "bucket_day": row_get::<String>(&row, "bucket_day")?,
                    "market_type": row_get::<String>(&row, "market_type")?,
                    "maker_volume_1e8": row_get::<String>(&row, "maker_volume_1e8")?,
                    "taker_volume_1e8": row_get::<String>(&row, "taker_volume_1e8")?,
                    "total_volume_1e8": row_get::<String>(&row, "total_volume_1e8")?,
                    "updated_at_ms": row_get::<i64>(&row, "updated_at_ms")?
                }))
            })
            .collect()
    }

    pub async fn admin_fee_rebates(
        &self,
        account: Option<&AccountId>,
    ) -> Result<Vec<serde_json::Value>> {
        if !self.admin_table_exists("rebate_accruals").await? {
            return Ok(Vec::new());
        }
        let rows = if let Some(account) = account {
            sqlx::query(
                "SELECT rebate_id, fee_event_id, account, source_type, source_id,
                        rebate_asset, rebate_amount_1e8, status, created_at_ms
                 FROM rebate_accruals
                 WHERE lower(account) = lower($1)
                 ORDER BY created_at_ms DESC, rebate_id DESC",
            )
            .bind(&account.0)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT rebate_id, fee_event_id, account, source_type, source_id,
                        rebate_asset, rebate_amount_1e8, status, created_at_ms
                 FROM rebate_accruals
                 ORDER BY created_at_ms DESC, rebate_id DESC",
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "rebate_id": row_get::<String>(&row, "rebate_id")?,
                    "fee_event_id": row_get::<String>(&row, "fee_event_id")?,
                    "account": row_get::<String>(&row, "account")?,
                    "source_type": row_get::<String>(&row, "source_type")?,
                    "source_id": row_get::<String>(&row, "source_id")?,
                    "rebate_asset": row_get::<String>(&row, "rebate_asset")?,
                    "rebate_amount_1e8": row_get::<String>(&row, "rebate_amount_1e8")?,
                    "status": row_get::<String>(&row, "status")?,
                    "created_at_ms": row_get::<i64>(&row, "created_at_ms")?
                }))
            })
            .collect()
    }

    async fn admin_table_exists(&self, table: &'static str) -> Result<bool> {
        validate_admin_identifier(table)?;
        let row = sqlx::query("SELECT to_regclass($1)::text AS relation")
            .bind(format!("public.{table}"))
            .fetch_one(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let relation: Option<String> = row_get(&row, "relation")?;
        Ok(relation.is_some())
    }

    async fn admin_count_table_if_exists(&self, table: &'static str) -> Result<Option<u64>> {
        validate_admin_identifier(table)?;
        if !self.admin_table_exists(table).await? {
            return Ok(None);
        }
        let row = sqlx::query(&format!("SELECT COUNT(*) AS count FROM {table}"))
            .fetch_one(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let count: i64 = row_get(&row, "count")?;
        Ok(Some(i64_to_u64_persistence("count", count)?))
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

    pub async fn insert_execution_transaction(
        &self,
        transaction: &ExecutionTransaction,
    ) -> Result<u64> {
        insert_execution_transaction(&self.pool, transaction).await
    }

    pub async fn update_execution_transaction_status(
        &self,
        transaction_id: &str,
        status: ExecutionTransactionStatus,
        tx_hash: Option<String>,
        error: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE execution_transactions
             SET status = $2, tx_hash = $3, error = $4, updated_at_ms = $5
             WHERE transaction_id = $1",
        )
        .bind(transaction_id)
        .bind(status.as_str())
        .bind(tx_hash)
        .bind(error)
        .bind(timestamp_to_i64(updated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(())
    }

    pub async fn get_transactions_for_intent(
        &self,
        intent_id: Uuid,
    ) -> Result<Vec<ExecutionTransaction>> {
        let rows = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, target, calldata, value_wei,
                    tx_hash, status, error, confirmed_at_ms, confirmed_block_number,
                    confirmation_status, confirmation_error, created_at_ms, updated_at_ms
             FROM execution_transactions
             WHERE intent_id = $1
             ORDER BY created_at_ms DESC, transaction_id DESC",
        )
        .bind(intent_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(execution_transaction_from_row)
            .collect()
    }

    pub async fn list_recent_execution_transactions(
        &self,
        limit: u32,
    ) -> Result<Vec<ExecutionTransaction>> {
        let rows = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, target, calldata, value_wei,
                    tx_hash, status, error, confirmed_at_ms, confirmed_block_number,
                    confirmation_status, confirmation_error, created_at_ms, updated_at_ms
             FROM execution_transactions
             ORDER BY created_at_ms DESC, transaction_id DESC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(execution_transaction_from_row)
            .collect()
    }

    pub async fn find_submitted_transaction_by_intent(
        &self,
        intent_id: Uuid,
    ) -> Result<Option<ExecutionTransaction>> {
        let row = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, target, calldata, value_wei,
                    tx_hash, status, error, confirmed_at_ms, confirmed_block_number,
                    confirmation_status, confirmation_error, created_at_ms, updated_at_ms
             FROM execution_transactions
             WHERE intent_id = $1 AND status = 'submitted'
             ORDER BY created_at_ms DESC, transaction_id DESC
             LIMIT 1",
        )
        .bind(intent_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        row.map(execution_transaction_from_row).transpose()
    }

    pub async fn list_submitted_unconfirmed_execution_transactions(
        &self,
        limit: u32,
    ) -> Result<Vec<ExecutionTransaction>> {
        let rows = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, target, calldata, value_wei,
                    tx_hash, status, error, confirmed_at_ms, confirmed_block_number,
                    confirmation_status, confirmation_error, created_at_ms, updated_at_ms
             FROM execution_transactions
             WHERE status = 'submitted'
               AND tx_hash IS NOT NULL
               AND COALESCE(confirmation_status, 'pending') <> 'confirmed'
             ORDER BY created_at_ms ASC, transaction_id ASC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        rows.into_iter()
            .map(execution_transaction_from_row)
            .collect()
    }

    pub async fn has_indexed_trade_for_confirmation(
        &self,
        tx_hash: &str,
        onchain_intent_id: &str,
    ) -> Result<bool> {
        let row = sqlx::query(
            "SELECT 1
             FROM indexed_perp_trades
             WHERE lower(tx_hash) = lower($1) AND lower(onchain_intent_id) = lower($2)
             LIMIT 1",
        )
        .bind(tx_hash.to_ascii_lowercase())
        .bind(onchain_intent_id.to_ascii_lowercase())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(row.is_some())
    }

    pub async fn has_matched_reconciliation_for_confirmation(
        &self,
        intent_id: Uuid,
        onchain_intent_id: &str,
        tx_hash: &str,
    ) -> Result<bool> {
        let row = sqlx::query(
            "SELECT 1
             FROM execution_reconciliations
             WHERE intent_id = $1
               AND lower(onchain_intent_id) = lower($2)
               AND lower(tx_hash) = lower($3)
               AND status = 'matched'
             LIMIT 1",
        )
        .bind(intent_id.to_string())
        .bind(onchain_intent_id.to_ascii_lowercase())
        .bind(tx_hash.to_ascii_lowercase())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(row.is_some())
    }

    pub async fn find_matched_indexed_trade_for_confirmation(
        &self,
        intent_id: Uuid,
        onchain_intent_id: &str,
        tx_hash: &str,
    ) -> Result<Option<IndexedPerpTrade>> {
        let row = sqlx::query(
            "SELECT t.event_id, t.tx_hash, t.log_index, t.block_number, t.block_hash,
                    t.buyer, t.seller, t.onchain_intent_id, t.market_id, t.size_delta_1e8,
                    t.execution_price_1e8, t.buyer_is_maker, t.buyer_nonce, t.seller_nonce,
                    t.created_at_ms
             FROM execution_reconciliations r
             JOIN indexed_perp_trades t ON t.event_id = r.indexed_event_id
             WHERE r.intent_id = $1
               AND lower(r.onchain_intent_id) = lower($2)
               AND lower(r.tx_hash) = lower($3)
               AND r.status = 'matched'
             ORDER BY t.block_number ASC, t.log_index ASC
             LIMIT 1",
        )
        .bind(intent_id.to_string())
        .bind(onchain_intent_id.to_ascii_lowercase())
        .bind(tx_hash.to_ascii_lowercase())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(indexed_perp_trade_from_row).transpose()
    }

    pub async fn perp_fee_flow_for_intent(&self, intent_id: Uuid) -> Result<FeeFlowType> {
        if !self.admin_table_exists("rfqs").await? {
            return Ok(FeeFlowType::Orderbook);
        }
        let row = sqlx::query(
            "SELECT 1
             FROM rfqs
             WHERE execution_intent_id = $1 AND status = 'accepted'
             LIMIT 1",
        )
        .bind(intent_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(if row.is_some() {
            FeeFlowType::Rfq
        } else {
            FeeFlowType::Orderbook
        })
    }

    pub async fn apply_confirmation_decision(
        &self,
        transaction_id: &str,
        intent_id: Uuid,
        decision: &ConfirmationDecision,
        now_ms: TimestampMs,
    ) -> Result<()> {
        let mut tx = self.begin().await?;
        sqlx::query(
            "UPDATE execution_transactions
             SET confirmation_status = $2,
                 confirmation_error = $3,
                 confirmed_at_ms = CASE WHEN $2 = 'confirmed' THEN COALESCE(confirmed_at_ms, $4) ELSE confirmed_at_ms END,
                 confirmed_block_number = CASE WHEN $2 = 'confirmed' THEN $5 ELSE confirmed_block_number END,
                 updated_at_ms = $4
             WHERE transaction_id = $1",
        )
        .bind(transaction_id)
        .bind(decision.confirmation_status.as_str())
        .bind(&decision.reason)
        .bind(timestamp_to_i64(now_ms))
        .bind(
            decision
                .receipt_block_number
                .map(|value| u64_to_i64("confirmed_block_number", value))
                .transpose()?,
        )
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        if decision.confirmed {
            update_execution_intent_status_tx(
                &mut tx,
                &intent_id.to_string(),
                ExecutionIntentStatus::Confirmed,
                now_ms,
            )
            .await?;
        }

        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))
    }

    pub async fn count_confirmed_execution_transactions(&self) -> Result<u64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS count
             FROM execution_transactions
             WHERE confirmation_status = 'confirmed'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let count: i64 = row_get(&row, "count")?;
        i64_to_u64_persistence("count", count)
    }

    pub async fn insert_option_series(&self, series: &OptionSeries) -> Result<()> {
        sqlx::query(
            "INSERT INTO option_series (
                option_series_id, underlying, base_asset, quote_asset, settlement_asset,
                expiry, strike_1e8, is_call, contract_size_1e8, status, source,
                onchain_product_id, onchain_series_id, created_at_ms, updated_at_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT (option_series_id) DO NOTHING",
        )
        .bind(&series.option_series_id)
        .bind(&series.underlying)
        .bind(&series.base_asset)
        .bind(&series.quote_asset)
        .bind(&series.settlement_asset)
        .bind(u64_to_i64("expiry", series.expiry)?)
        .bind(series.strike_1e8.to_string())
        .bind(series.is_call)
        .bind(series.contract_size_1e8.to_string())
        .bind(series.status.as_str())
        .bind(series.source.as_str())
        .bind(&series.onchain_product_id)
        .bind(&series.onchain_series_id)
        .bind(timestamp_to_i64(series.created_at_ms))
        .bind(timestamp_to_i64(series.updated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(())
    }

    pub async fn list_option_series(&self) -> Result<Vec<OptionSeries>> {
        let rows = sqlx::query(
            "SELECT option_series_id, underlying, base_asset, quote_asset, settlement_asset,
                    expiry, strike_1e8, is_call, contract_size_1e8, status, source,
                    onchain_product_id, onchain_series_id, created_at_ms, updated_at_ms
             FROM option_series
             ORDER BY expiry ASC, strike_1e8 ASC, option_series_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(option_series_from_row).collect()
    }

    pub async fn get_option_series(&self, option_series_id: &str) -> Result<Option<OptionSeries>> {
        let row = sqlx::query(
            "SELECT option_series_id, underlying, base_asset, quote_asset, settlement_asset,
                    expiry, strike_1e8, is_call, contract_size_1e8, status, source,
                    onchain_product_id, onchain_series_id, created_at_ms, updated_at_ms
             FROM option_series
             WHERE option_series_id = $1",
        )
        .bind(option_series_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_series_from_row).transpose()
    }

    pub async fn disable_option_series(
        &self,
        option_series_id: &str,
        updated_at_ms: TimestampMs,
    ) -> Result<OptionSeries> {
        let row = sqlx::query(
            "UPDATE option_series
             SET status = 'disabled', updated_at_ms = $2
             WHERE option_series_id = $1
             RETURNING option_series_id, underlying, base_asset, quote_asset, settlement_asset,
                       expiry, strike_1e8, is_call, contract_size_1e8, status, source,
                       onchain_product_id, onchain_series_id, created_at_ms, updated_at_ms",
        )
        .bind(option_series_id)
        .bind(timestamp_to_i64(updated_at_ms))
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_series_from_row)
            .transpose()?
            .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))
    }

    pub async fn insert_option_order(&self, order: &OptionOrder) -> Result<()> {
        insert_option_order_query(
            sqlx::query(
                "INSERT INTO option_orders (
                order_id, option_series_id, account, side, price_1e8, size_1e8,
                remaining_size_1e8, time_in_force, client_order_id, nonce, deadline_ms,
                signature, status, created_at_ms, updated_at_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
            ),
            order,
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(option_order_insert_error)
    }

    pub async fn submit_option_order_and_match(
        &self,
        mut incoming: OptionOrder,
        updated_at_ms: TimestampMs,
    ) -> Result<(OptionOrder, Vec<OptionFill>)> {
        let mut tx = self.begin().await?;
        insert_option_order_tx(&mut tx, &incoming).await?;

        let opposite_side = incoming.side.opposite();
        let rows = sqlx::query(
            "SELECT order_id, option_series_id, account, side, price_1e8, size_1e8,
                    remaining_size_1e8, time_in_force, client_order_id, nonce, deadline_ms,
                    signature, status, created_at_ms, updated_at_ms
             FROM option_orders
             WHERE option_series_id = $1
               AND side = $2
               AND status IN ('open', 'partially_filled')
             ORDER BY created_at_ms ASC, order_id ASC
             FOR UPDATE",
        )
        .bind(&incoming.option_series_id)
        .bind(side_to_str(opposite_side))
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        let mut makers = rows
            .into_iter()
            .map(option_order_from_row)
            .collect::<Result<Vec<_>>>()?;
        sort_option_match_candidates(&mut makers, incoming.side);

        let mut fills = Vec::new();
        for mut maker in makers {
            if incoming.remaining_size_1e8 == 0 {
                break;
            }
            if !can_match_option_orders(&incoming, &maker) {
                continue;
            }
            let fill_size = incoming.remaining_size_1e8.min(maker.remaining_size_1e8);
            if fill_size == 0 {
                continue;
            }

            let fill = option_fill_from_match(&incoming, &maker, fill_size, updated_at_ms);
            incoming.remaining_size_1e8 -= fill_size;
            maker.remaining_size_1e8 -= fill_size;
            maker.status = status_for_remaining(maker.size_1e8, maker.remaining_size_1e8);
            maker.updated_at_ms = updated_at_ms;
            update_option_order_tx(&mut tx, &maker).await?;
            insert_option_fill_tx(&mut tx, &fill).await?;
            fills.push(fill);
        }

        incoming.status = status_for_remaining(incoming.size_1e8, incoming.remaining_size_1e8);
        incoming.updated_at_ms = updated_at_ms;
        update_option_order_tx(&mut tx, &incoming).await?;

        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok((incoming, fills))
    }

    pub async fn list_option_orders(&self) -> Result<Vec<OptionOrder>> {
        let rows = sqlx::query(
            "SELECT order_id, option_series_id, account, side, price_1e8, size_1e8,
                    remaining_size_1e8, time_in_force, client_order_id, nonce, deadline_ms,
                    signature, status, created_at_ms, updated_at_ms
             FROM option_orders
             ORDER BY created_at_ms ASC, order_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(option_order_from_row).collect()
    }

    pub async fn get_option_order(&self, order_id: OptionOrderId) -> Result<Option<OptionOrder>> {
        let row = sqlx::query(
            "SELECT order_id, option_series_id, account, side, price_1e8, size_1e8,
                    remaining_size_1e8, time_in_force, client_order_id, nonce, deadline_ms,
                    signature, status, created_at_ms, updated_at_ms
             FROM option_orders
             WHERE order_id = $1",
        )
        .bind(order_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_order_from_row).transpose()
    }

    pub async fn cancel_option_order(
        &self,
        order_id: OptionOrderId,
        updated_at_ms: TimestampMs,
    ) -> Result<OptionOrder> {
        let row = sqlx::query(
            "UPDATE option_orders
             SET status = 'cancelled', updated_at_ms = $2
             WHERE order_id = $1 AND status IN ('open', 'partially_filled')
             RETURNING order_id, option_series_id, account, side, price_1e8, size_1e8,
                       remaining_size_1e8, time_in_force, client_order_id, nonce, deadline_ms,
                       signature, status, created_at_ms, updated_at_ms",
        )
        .bind(order_id.to_string())
        .bind(timestamp_to_i64(updated_at_ms))
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        if let Some(row) = row {
            return option_order_from_row(row);
        }
        let Some(order) = self.get_option_order(order_id).await? else {
            return Err(BackendError::InvalidOptionOrderId);
        };
        Err(BackendError::InvalidOptionOrderState(format!(
            "option order is {}",
            order.status.as_str()
        )))
    }

    pub async fn open_option_orders_for_series(
        &self,
        option_series_id: &str,
    ) -> Result<Vec<OptionOrder>> {
        let rows = sqlx::query(
            "SELECT order_id, option_series_id, account, side, price_1e8, size_1e8,
                    remaining_size_1e8, time_in_force, client_order_id, nonce, deadline_ms,
                    signature, status, created_at_ms, updated_at_ms
             FROM option_orders
             WHERE option_series_id = $1 AND status IN ('open', 'partially_filled')
             ORDER BY created_at_ms ASC, order_id ASC",
        )
        .bind(option_series_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(option_order_from_row).collect()
    }

    pub async fn list_option_fills(&self) -> Result<Vec<OptionFill>> {
        let rows = sqlx::query(
            "SELECT fill_id, option_series_id, buy_order_id, sell_order_id, buyer, seller,
                    maker_order_id, taker_order_id, taker_side, price_1e8, size_1e8,
                    created_at_ms
             FROM option_fills
             ORDER BY created_at_ms ASC, fill_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(option_fill_from_row).collect()
    }

    pub async fn get_option_fill(&self, fill_id: OptionFillId) -> Result<Option<OptionFill>> {
        let row = sqlx::query(
            "SELECT fill_id, option_series_id, buy_order_id, sell_order_id, buyer, seller,
                    maker_order_id, taker_order_id, taker_side, price_1e8, size_1e8,
                    created_at_ms
             FROM option_fills
             WHERE fill_id = $1",
        )
        .bind(fill_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_fill_from_row).transpose()
    }

    pub async fn option_fills_for_order(&self, order_id: OptionOrderId) -> Result<Vec<OptionFill>> {
        let rows = sqlx::query(
            "SELECT fill_id, option_series_id, buy_order_id, sell_order_id, buyer, seller,
                    maker_order_id, taker_order_id, taker_side, price_1e8, size_1e8,
                    created_at_ms
             FROM option_fills
             WHERE buy_order_id = $1
                OR sell_order_id = $1
                OR maker_order_id = $1
                OR taker_order_id = $1
             ORDER BY created_at_ms ASC, fill_id ASC",
        )
        .bind(order_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(option_fill_from_row).collect()
    }

    pub async fn insert_option_execution_intent(
        &self,
        intent: &OptionExecutionIntent,
    ) -> Result<OptionExecutionIntent> {
        sqlx::query(
            "INSERT INTO option_execution_intents (
                intent_id, onchain_intent_id, source_type, source_id, option_series_id,
                onchain_option_id, buyer, seller, underlying, settlement_asset, expiry,
                strike_1e8, is_call, contract_size_1e8, quantity_contracts, source_size_1e8,
                source_price_1e8, premium_per_contract_native, buyer_is_maker, buyer_nonce,
                seller_nonce, deadline, buyer_signature, seller_signature, calldata, status,
                error, created_at_ms, updated_at_ms
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29
            )
            ON CONFLICT (source_type, source_id) DO NOTHING",
        )
        .bind(intent.intent_id.to_string())
        .bind(&intent.onchain_intent_id)
        .bind(intent.source_type.as_str())
        .bind(&intent.source_id)
        .bind(&intent.option_series_id)
        .bind(&intent.onchain_option_id)
        .bind(&intent.buyer.0)
        .bind(&intent.seller.0)
        .bind(&intent.underlying.0)
        .bind(&intent.settlement_asset.0)
        .bind(u64_to_i64("expiry", intent.expiry)?)
        .bind(intent.strike_1e8.to_string())
        .bind(intent.is_call)
        .bind(intent.contract_size_1e8.to_string())
        .bind(intent.quantity_contracts.to_string())
        .bind(intent.source_size_1e8.to_string())
        .bind(intent.source_price_1e8.to_string())
        .bind(intent.premium_per_contract_native.to_string())
        .bind(intent.buyer_is_maker)
        .bind(intent.buyer_nonce.map(|value| value.to_string()))
        .bind(intent.seller_nonce.map(|value| value.to_string()))
        .bind(u64_to_i64("deadline", intent.deadline)?)
        .bind(&intent.buyer_signature)
        .bind(&intent.seller_signature)
        .bind(&intent.calldata)
        .bind(intent.status.as_str())
        .bind(&intent.error)
        .bind(timestamp_to_i64(intent.created_at_ms))
        .bind(timestamp_to_i64(intent.updated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        self.get_option_execution_intent_by_source(intent.source_type, &intent.source_id)
            .await?
            .ok_or_else(|| {
                BackendError::Persistence(
                    "option execution intent insert did not return a row".to_string(),
                )
            })
    }

    pub async fn list_option_execution_intents(&self) -> Result<Vec<OptionExecutionIntent>> {
        let sql = option_execution_intent_select_sql("ORDER BY created_at_ms ASC, intent_id ASC");
        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(option_execution_intent_from_row)
            .collect()
    }

    pub async fn get_option_execution_intent(
        &self,
        intent_id: OptionExecutionIntentId,
    ) -> Result<Option<OptionExecutionIntent>> {
        let sql = option_execution_intent_select_sql("WHERE intent_id = $1");
        let rows = sqlx::query(&sql)
            .bind(intent_id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.map(option_execution_intent_from_row).transpose()
    }

    pub async fn get_option_execution_intent_by_source(
        &self,
        source_type: OptionExecutionSourceType,
        source_id: &str,
    ) -> Result<Option<OptionExecutionIntent>> {
        let sql = option_execution_intent_select_sql("WHERE source_type = $1 AND source_id = $2");
        let rows = sqlx::query(&sql)
            .bind(source_type.as_str())
            .bind(source_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.map(option_execution_intent_from_row).transpose()
    }

    pub async fn upsert_option_execution_signatures(
        &self,
        intent_id: OptionExecutionIntentId,
        buyer_signature: Option<String>,
        seller_signature: Option<String>,
        status: OptionExecutionIntentStatus,
        calldata: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<OptionExecutionIntent> {
        sqlx::query(
            "UPDATE option_execution_intents
             SET buyer_signature = COALESCE($2, buyer_signature),
                 seller_signature = COALESCE($3, seller_signature),
                 calldata = COALESCE($4, calldata),
                 status = $5,
                 simulation_status = NULL,
                 simulation_error = NULL,
                 simulation_block_number = NULL,
                 simulation_revert_data = NULL,
                 simulation_revert_selector = NULL,
                 simulated_at_ms = NULL,
                 updated_at_ms = $6
             WHERE intent_id = $1",
        )
        .bind(intent_id.to_string())
        .bind(buyer_signature)
        .bind(seller_signature)
        .bind(calldata)
        .bind(status.as_str())
        .bind(timestamp_to_i64(updated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        self.get_option_execution_intent(intent_id)
            .await?
            .ok_or(BackendError::InvalidOptionExecutionIntentId)
    }

    pub async fn persist_option_execution_simulation_result(
        &self,
        result: &OptionExecutionSimulationResult,
    ) -> Result<OptionExecutionIntent> {
        let rows = sqlx::query(
            "UPDATE option_execution_intents
             SET simulation_status = $2,
                 simulation_error = $3,
                 simulation_block_number = $4,
                 simulation_revert_data = $5,
                 simulation_revert_selector = $6,
                 simulated_at_ms = $7,
                 updated_at_ms = $7
             WHERE intent_id = $1",
        )
        .bind(result.intent_id.to_string())
        .bind(result.simulation_status.as_str())
        .bind(&result.error)
        .bind(
            result
                .block_number
                .map(|value| u64_to_i64("simulation_block_number", value))
                .transpose()?,
        )
        .bind(&result.revert_data)
        .bind(&result.revert_selector)
        .bind(timestamp_to_i64(result.simulated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?
        .rows_affected();
        if rows == 0 {
            return Err(BackendError::InvalidOptionExecutionIntentId);
        }
        self.get_option_execution_intent(result.intent_id)
            .await?
            .ok_or(BackendError::InvalidOptionExecutionIntentId)
    }

    pub async fn update_option_execution_intent_status(
        &self,
        intent_id: OptionExecutionIntentId,
        status: OptionExecutionIntentStatus,
        error: Option<String>,
        updated_at_ms: TimestampMs,
    ) -> Result<OptionExecutionIntent> {
        let rows = sqlx::query(
            "UPDATE option_execution_intents
             SET status = $2,
                 error = $3,
                 updated_at_ms = $4
             WHERE intent_id = $1",
        )
        .bind(intent_id.to_string())
        .bind(status.as_str())
        .bind(error)
        .bind(timestamp_to_i64(updated_at_ms))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?
        .rows_affected();
        if rows == 0 {
            return Err(BackendError::InvalidOptionExecutionIntentId);
        }
        self.get_option_execution_intent(intent_id)
            .await?
            .ok_or(BackendError::InvalidOptionExecutionIntentId)
    }

    pub async fn insert_option_execution_transaction(
        &self,
        transaction: &OptionExecutionTransaction,
    ) -> Result<u64> {
        let result = sqlx::query(
            "INSERT INTO option_execution_transactions (
                transaction_id, intent_id, onchain_intent_id, sender, target, calldata,
                value_wei, gas_limit, tx_hash, status, error, created_at_ms, updated_at_ms,
                estimated_gas, required_gas, simulation_gas_limit, broadcast_gas_limit,
                gas_safety_bps, gas_check_status, gas_check_error
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)",
        )
        .bind(&transaction.transaction_id)
        .bind(transaction.intent_id.to_string())
        .bind(&transaction.onchain_intent_id)
        .bind(&transaction.from.0)
        .bind(&transaction.to.0)
        .bind(&transaction.calldata)
        .bind(&transaction.value_wei)
        .bind(
            transaction
                .gas_limit
                .map(|value| u64_to_i64("gas_limit", value))
                .transpose()?,
        )
        .bind(&transaction.tx_hash)
        .bind(transaction.status.as_str())
        .bind(&transaction.error)
        .bind(timestamp_to_i64(transaction.created_at_ms))
        .bind(timestamp_to_i64(transaction.updated_at_ms))
        .bind(
            transaction
                .estimated_gas
                .map(|value| u64_to_i64("estimated_gas", value))
                .transpose()?,
        )
        .bind(
            transaction
                .required_gas
                .map(|value| u64_to_i64("required_gas", value))
                .transpose()?,
        )
        .bind(
            transaction
                .simulation_gas_limit
                .map(|value| u64_to_i64("simulation_gas_limit", value))
                .transpose()?,
        )
        .bind(
            transaction
                .broadcast_gas_limit
                .map(|value| u64_to_i64("broadcast_gas_limit", value))
                .transpose()?,
        )
        .bind(transaction.gas_safety_bps.map(|value| value as i32))
        .bind(
            transaction
                .gas_check_status
                .as_ref()
                .map(|status| status.as_str().to_string()),
        )
        .bind(&transaction.gas_check_error)
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(result.rows_affected())
    }

    pub async fn find_submitted_option_execution_transaction_by_intent(
        &self,
        intent_id: OptionExecutionIntentId,
    ) -> Result<Option<OptionExecutionTransaction>> {
        let row = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, sender, target, calldata,
                    value_wei, gas_limit, tx_hash, status, error, created_at_ms, updated_at_ms,
                    estimated_gas, required_gas, simulation_gas_limit, broadcast_gas_limit,
                    gas_safety_bps, gas_check_status, gas_check_error,
                    confirmation_status, confirmed_at_ms, confirmed_block_number,
                    receipt_status, confirmation_error
             FROM option_execution_transactions
             WHERE intent_id = $1 AND status = 'submitted'
             ORDER BY created_at_ms DESC, transaction_id DESC
             LIMIT 1",
        )
        .bind(intent_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_execution_transaction_from_row).transpose()
    }

    /// Pending option execution transactions for the confirmation worker.
    ///
    /// A row is "pending" when it has been submitted on-chain (status=submitted), has a
    /// non-empty tx hash, and has not yet been finalized (`confirmation_status` is NULL,
    /// `pending`, `receipt_missing`, or `receipt_error`).
    pub async fn list_pending_option_execution_transactions(
        &self,
        limit: u32,
    ) -> Result<Vec<OptionExecutionTransaction>> {
        let rows = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, sender, target, calldata,
                    value_wei, gas_limit, tx_hash, status, error, created_at_ms, updated_at_ms,
                    estimated_gas, required_gas, simulation_gas_limit, broadcast_gas_limit,
                    gas_safety_bps, gas_check_status, gas_check_error,
                    confirmation_status, confirmed_at_ms, confirmed_block_number,
                    receipt_status, confirmation_error
             FROM option_execution_transactions
             WHERE status = 'submitted'
               AND tx_hash IS NOT NULL
               AND (
                   confirmation_status IS NULL
                   OR confirmation_status IN ('pending', 'receipt_missing', 'receipt_error')
               )
             ORDER BY created_at_ms ASC, transaction_id ASC
             LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(option_execution_transaction_from_row)
            .collect()
    }

    pub async fn get_option_execution_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<Option<OptionExecutionTransaction>> {
        let row = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, sender, target, calldata,
                    value_wei, gas_limit, tx_hash, status, error, created_at_ms, updated_at_ms,
                    estimated_gas, required_gas, simulation_gas_limit, broadcast_gas_limit,
                    gas_safety_bps, gas_check_status, gas_check_error,
                    confirmation_status, confirmed_at_ms, confirmed_block_number,
                    receipt_status, confirmation_error
             FROM option_execution_transactions
             WHERE transaction_id = $1",
        )
        .bind(transaction_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_execution_transaction_from_row).transpose()
    }

    pub async fn update_option_execution_confirmation(
        &self,
        transaction_id: &str,
        confirmation_status: OptionExecutionConfirmationStatus,
        confirmed_at_ms: TimestampMs,
        confirmed_block_number: Option<u64>,
        receipt_status: Option<u64>,
        confirmation_error: Option<String>,
    ) -> Result<u64> {
        let rows = sqlx::query(
            "UPDATE option_execution_transactions
             SET confirmation_status = $1,
                 confirmed_at_ms = $2,
                 confirmed_block_number = $3,
                 receipt_status = $4,
                 confirmation_error = $5,
                 updated_at_ms = $2
             WHERE transaction_id = $6",
        )
        .bind(confirmation_status.as_str())
        .bind(timestamp_to_i64(confirmed_at_ms))
        .bind(
            confirmed_block_number
                .map(|value| u64_to_i64("confirmed_block_number", value))
                .transpose()?,
        )
        .bind(
            receipt_status
                .map(|value| u64_to_i64("receipt_status", value))
                .transpose()?,
        )
        .bind(confirmation_error)
        .bind(transaction_id)
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?
        .rows_affected();
        Ok(rows)
    }

    pub async fn get_option_execution_transactions_for_intent(
        &self,
        intent_id: OptionExecutionIntentId,
    ) -> Result<Vec<OptionExecutionTransaction>> {
        let rows = sqlx::query(
            "SELECT transaction_id, intent_id, onchain_intent_id, sender, target, calldata,
                    value_wei, gas_limit, tx_hash, status, error, created_at_ms, updated_at_ms,
                    estimated_gas, required_gas, simulation_gas_limit, broadcast_gas_limit,
                    gas_safety_bps, gas_check_status, gas_check_error,
                    confirmation_status, confirmed_at_ms, confirmed_block_number,
                    receipt_status, confirmation_error
             FROM option_execution_transactions
             WHERE intent_id = $1
             ORDER BY created_at_ms DESC, transaction_id DESC",
        )
        .bind(intent_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter()
            .map(option_execution_transaction_from_row)
            .collect()
    }

    pub async fn insert_option_rfq(&self, rfq: &OptionRfqRequest) -> Result<()> {
        sqlx::query(
            "INSERT INTO option_rfqs (
                option_rfq_id, taker, option_series_id, side, size_1e8, limit_price_1e8,
                status, created_at_ms, expires_at_ms, accepted_quote_id, option_fill_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(rfq.option_rfq_id.to_string())
        .bind(&rfq.taker.0)
        .bind(&rfq.option_series_id)
        .bind(side_to_str(rfq.side))
        .bind(rfq.size_1e8.to_string())
        .bind(rfq.limit_price_1e8.map(|value| value.to_string()))
        .bind(rfq.status.as_str())
        .bind(timestamp_to_i64(rfq.created_at_ms))
        .bind(timestamp_to_i64(rfq.expires_at_ms))
        .bind(rfq.accepted_quote_id.map(|id| id.to_string()))
        .bind(rfq.option_fill_id.map(|id| id.to_string()))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(())
    }

    pub async fn list_option_rfqs(&self) -> Result<Vec<OptionRfqRequest>> {
        let rows = sqlx::query(
            "SELECT option_rfq_id, taker, option_series_id, side, size_1e8, limit_price_1e8,
                    status, created_at_ms, expires_at_ms, accepted_quote_id, option_fill_id
             FROM option_rfqs
             ORDER BY created_at_ms ASC, option_rfq_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(option_rfq_from_row).collect()
    }

    pub async fn get_option_rfq(
        &self,
        option_rfq_id: OptionRfqId,
    ) -> Result<Option<OptionRfqRequest>> {
        let row = sqlx::query(
            "SELECT option_rfq_id, taker, option_series_id, side, size_1e8, limit_price_1e8,
                    status, created_at_ms, expires_at_ms, accepted_quote_id, option_fill_id
             FROM option_rfqs
             WHERE option_rfq_id = $1",
        )
        .bind(option_rfq_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_rfq_from_row).transpose()
    }

    pub async fn insert_option_rfq_quote(&self, quote: &OptionRfqQuote) -> Result<()> {
        let result = sqlx::query(
            "INSERT INTO option_rfq_quotes (
                quote_id, option_rfq_id, mm_account, session_id, client_quote_id,
                price_1e8, size_1e8, status, created_at_ms, expires_at_ms,
                signature, quote_digest, quote_nonce, signature_status, recovered_signer
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(quote.quote_id.to_string())
        .bind(quote.option_rfq_id.to_string())
        .bind(&quote.mm_account.0)
        .bind(&quote.session_id)
        .bind(&quote.client_quote_id)
        .bind(quote.price_1e8.to_string())
        .bind(quote.size_1e8.to_string())
        .bind(quote.status.as_str())
        .bind(timestamp_to_i64(quote.created_at_ms))
        .bind(timestamp_to_i64(quote.expires_at_ms))
        .bind(&quote.signature)
        .bind(&quote.quote_digest)
        .bind(&quote.quote_nonce)
        .bind(quote.signature_status.as_str())
        .bind(quote.recovered_signer.as_ref().map(|account| &account.0))
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(error) if is_unique_violation(&error) => {
                Err(BackendError::InvalidOptionRfqQuoteState(
                    "duplicate client_quote_id for option RFQ and MM account".to_string(),
                ))
            }
            Err(error) => Err(BackendError::Persistence(error.to_string())),
        }
    }

    pub async fn count_option_rfq_quotes(&self, option_rfq_id: OptionRfqId) -> Result<usize> {
        let row =
            sqlx::query("SELECT COUNT(*) AS count FROM option_rfq_quotes WHERE option_rfq_id = $1")
                .bind(option_rfq_id.to_string())
                .fetch_one(&self.pool)
                .await
                .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let count: i64 = row_get(&row, "count")?;
        let count = i64_to_u64_persistence("count", count)?;
        usize::try_from(count).map_err(|_| {
            BackendError::Persistence("option RFQ quote count exceeds usize".to_string())
        })
    }

    pub async fn list_option_rfq_quotes(
        &self,
        option_rfq_id: OptionRfqId,
    ) -> Result<Vec<OptionRfqQuote>> {
        let rows = sqlx::query(
            "SELECT quote_id, option_rfq_id, mm_account, session_id, client_quote_id,
                    price_1e8, size_1e8, status, created_at_ms, expires_at_ms,
                    signature, quote_digest, quote_nonce, signature_status, recovered_signer
             FROM option_rfq_quotes
             WHERE option_rfq_id = $1
             ORDER BY created_at_ms ASC, quote_id ASC",
        )
        .bind(option_rfq_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(option_rfq_quote_from_row).collect()
    }

    pub async fn get_option_rfq_quote(
        &self,
        quote_id: OptionRfqQuoteId,
    ) -> Result<Option<OptionRfqQuote>> {
        let row = sqlx::query(
            "SELECT quote_id, option_rfq_id, mm_account, session_id, client_quote_id,
                    price_1e8, size_1e8, status, created_at_ms, expires_at_ms,
                    signature, quote_digest, quote_nonce, signature_status, recovered_signer
             FROM option_rfq_quotes
             WHERE quote_id = $1",
        )
        .bind(quote_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_rfq_quote_from_row).transpose()
    }

    pub async fn get_option_rfq_fill(
        &self,
        fill_id: OptionRfqFillId,
    ) -> Result<Option<OptionRfqFill>> {
        let row = sqlx::query(
            "SELECT fill_id, option_rfq_id, quote_id, option_series_id, buyer, seller,
                    taker, mm_account, taker_side, price_1e8, size_1e8, created_at_ms
             FROM option_rfq_fills
             WHERE fill_id = $1",
        )
        .bind(fill_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(option_rfq_fill_from_row).transpose()
    }

    pub async fn accept_option_rfq_quote_and_insert_fill(
        &self,
        option_rfq_id: OptionRfqId,
        quote_id: OptionRfqQuoteId,
        fill: &OptionRfqFill,
    ) -> Result<()> {
        let mut tx = self.begin().await?;
        insert_option_rfq_fill_tx(&mut tx, fill).await?;

        let rfq_result = sqlx::query(
            "UPDATE option_rfqs
             SET status = 'accepted', accepted_quote_id = $2, option_fill_id = $3
             WHERE option_rfq_id = $1 AND status = 'open' AND accepted_quote_id IS NULL",
        )
        .bind(option_rfq_id.to_string())
        .bind(quote_id.to_string())
        .bind(fill.fill_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        if rfq_result.rows_affected() != 1 {
            return Err(BackendError::InvalidOptionRfqState(
                "option RFQ is no longer open".to_string(),
            ));
        }

        let quote_result = sqlx::query(
            "UPDATE option_rfq_quotes
             SET status = 'accepted'
             WHERE quote_id = $1 AND option_rfq_id = $2 AND status = 'active'",
        )
        .bind(quote_id.to_string())
        .bind(option_rfq_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        if quote_result.rows_affected() != 1 {
            return Err(BackendError::InvalidOptionRfqQuoteState(
                "option RFQ quote is no longer active".to_string(),
            ));
        }

        sqlx::query(
            "UPDATE option_rfq_quotes
             SET status = 'rejected'
             WHERE option_rfq_id = $1 AND quote_id <> $2 AND status = 'active'",
        )
        .bind(option_rfq_id.to_string())
        .bind(quote_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))
    }

    pub async fn cancel_option_rfq(&self, option_rfq_id: OptionRfqId) -> Result<OptionRfqRequest> {
        let mut tx = self.begin().await?;
        let result = sqlx::query(
            "UPDATE option_rfqs
             SET status = 'cancelled'
             WHERE option_rfq_id = $1 AND status <> 'accepted'",
        )
        .bind(option_rfq_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        if result.rows_affected() != 1 {
            return Err(BackendError::InvalidOptionRfqState(
                "option RFQ cannot be cancelled".to_string(),
            ));
        }
        sqlx::query(
            "UPDATE option_rfq_quotes
             SET status = 'cancelled'
             WHERE option_rfq_id = $1 AND status = 'active'",
        )
        .bind(option_rfq_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        self.get_option_rfq(option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId)
    }

    pub async fn insert_rfq(&self, rfq: &RfqRequest) -> Result<()> {
        sqlx::query(
            "INSERT INTO rfqs (
                rfq_id, taker, market_id, side, size_1e8, limit_price_1e8, status,
                created_at_ms, expires_at_ms, accepted_quote_id, execution_intent_id
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(rfq.rfq_id.to_string())
        .bind(&rfq.taker.0)
        .bind(u64_to_i64("market_id", rfq.market_id)?)
        .bind(match rfq.side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        })
        .bind(rfq.size_1e8.to_string())
        .bind(rfq.limit_price_1e8.map(|value| value.to_string()))
        .bind(rfq.status.as_str())
        .bind(timestamp_to_i64(rfq.created_at_ms))
        .bind(timestamp_to_i64(rfq.expires_at_ms))
        .bind(rfq.accepted_quote_id.map(|id| id.to_string()))
        .bind(rfq.execution_intent_id.map(|id| id.to_string()))
        .execute(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok(())
    }

    pub async fn list_rfqs(&self) -> Result<Vec<RfqRequest>> {
        let rows = sqlx::query(
            "SELECT rfq_id, taker, market_id, side, size_1e8, limit_price_1e8, status,
                    created_at_ms, expires_at_ms, accepted_quote_id, execution_intent_id
             FROM rfqs
             ORDER BY created_at_ms ASC, rfq_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(rfq_from_row).collect()
    }

    pub async fn get_rfq(&self, rfq_id: RfqId) -> Result<Option<RfqRequest>> {
        let row = sqlx::query(
            "SELECT rfq_id, taker, market_id, side, size_1e8, limit_price_1e8, status,
                    created_at_ms, expires_at_ms, accepted_quote_id, execution_intent_id
             FROM rfqs
             WHERE rfq_id = $1",
        )
        .bind(rfq_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(rfq_from_row).transpose()
    }

    pub async fn insert_rfq_quote(&self, quote: &RfqQuote) -> Result<()> {
        let result = sqlx::query(
            "INSERT INTO rfq_quotes (
                quote_id, rfq_id, mm_account, session_id, client_quote_id, price_1e8,
                size_1e8, status, created_at_ms, expires_at_ms, signature, quote_digest,
                quote_nonce, signature_status, recovered_signer
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(quote.quote_id.to_string())
        .bind(quote.rfq_id.to_string())
        .bind(&quote.mm_account.0)
        .bind(&quote.session_id)
        .bind(&quote.client_quote_id)
        .bind(quote.price_1e8.to_string())
        .bind(quote.size_1e8.to_string())
        .bind(quote.status.as_str())
        .bind(timestamp_to_i64(quote.created_at_ms))
        .bind(timestamp_to_i64(quote.expires_at_ms))
        .bind(&quote.signature)
        .bind(&quote.quote_digest)
        .bind(&quote.quote_nonce)
        .bind(quote.signature_status.as_str())
        .bind(quote.recovered_signer.as_ref().map(|account| &account.0))
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(error) if is_unique_violation(&error) => Err(BackendError::InvalidRfqQuoteState(
                "duplicate client_quote_id for RFQ and MM account".to_string(),
            )),
            Err(error) => Err(BackendError::Persistence(error.to_string())),
        }
    }

    pub async fn list_rfq_quotes(&self, rfq_id: RfqId) -> Result<Vec<RfqQuote>> {
        let rows = sqlx::query(
            "SELECT quote_id, rfq_id, mm_account, session_id, client_quote_id, price_1e8,
                    size_1e8, status, created_at_ms, expires_at_ms, signature, quote_digest,
                    quote_nonce, signature_status, recovered_signer
             FROM rfq_quotes
             WHERE rfq_id = $1
             ORDER BY created_at_ms ASC, quote_id ASC",
        )
        .bind(rfq_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        rows.into_iter().map(rfq_quote_from_row).collect()
    }

    pub async fn count_rfq_quotes(&self, rfq_id: RfqId) -> Result<usize> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM rfq_quotes WHERE rfq_id = $1")
            .bind(rfq_id.to_string())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        let count: i64 = row_get(&row, "count")?;
        let count = i64_to_u64_persistence("count", count)?;
        usize::try_from(count)
            .map_err(|_| BackendError::Persistence("RFQ quote count exceeds usize".to_string()))
    }

    pub async fn get_rfq_quote(&self, quote_id: QuoteId) -> Result<Option<RfqQuote>> {
        let row = sqlx::query(
            "SELECT quote_id, rfq_id, mm_account, session_id, client_quote_id, price_1e8,
                    size_1e8, status, created_at_ms, expires_at_ms, signature, quote_digest,
                    quote_nonce, signature_status, recovered_signer
             FROM rfq_quotes
             WHERE quote_id = $1",
        )
        .bind(quote_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        row.map(rfq_quote_from_row).transpose()
    }

    pub async fn accept_rfq_quote_and_insert_intent(
        &self,
        rfq_id: RfqId,
        quote_id: QuoteId,
        intent: &ExecutionIntent,
        updated_at_ms: TimestampMs,
    ) -> Result<()> {
        let mut tx = self.begin().await?;
        let buyer_nonce = intent
            .buyer_nonce
            .ok_or_else(|| BackendError::MissingExecutionMetadata("buyer_nonce".to_string()))?;
        let seller_nonce = intent
            .seller_nonce
            .ok_or_else(|| BackendError::MissingExecutionMetadata("seller_nonce".to_string()))?;
        self.insert_nonce_tx(&mut tx, &intent.buyer, buyer_nonce, updated_at_ms)
            .await?;
        self.insert_nonce_tx(&mut tx, &intent.seller, seller_nonce, updated_at_ms)
            .await?;
        let db_intent = DbExecutionIntent::try_from(intent)?;
        insert_execution_intent(&mut tx, &db_intent).await?;

        let rfq_result = sqlx::query(
            "UPDATE rfqs
             SET status = 'accepted', accepted_quote_id = $2, execution_intent_id = $3
             WHERE rfq_id = $1 AND status = 'open' AND accepted_quote_id IS NULL",
        )
        .bind(rfq_id.to_string())
        .bind(quote_id.to_string())
        .bind(intent.intent_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        if rfq_result.rows_affected() != 1 {
            return Err(BackendError::InvalidRfqState(
                "RFQ is no longer open".to_string(),
            ));
        }

        let quote_result = sqlx::query(
            "UPDATE rfq_quotes
             SET status = 'accepted'
             WHERE quote_id = $1 AND rfq_id = $2 AND status = 'active'",
        )
        .bind(quote_id.to_string())
        .bind(rfq_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        if quote_result.rows_affected() != 1 {
            return Err(BackendError::InvalidRfqQuoteState(
                "quote is no longer active".to_string(),
            ));
        }

        sqlx::query(
            "UPDATE rfq_quotes
             SET status = 'rejected'
             WHERE rfq_id = $1 AND quote_id <> $2 AND status = 'active'",
        )
        .bind(rfq_id.to_string())
        .bind(quote_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;

        insert_engine_event(
            &mut tx,
            &EngineEvent::ExecutionIntentCreated {
                intent: intent.clone(),
            },
        )
        .await?;
        let _ = updated_at_ms;
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))
    }

    pub async fn cancel_rfq(
        &self,
        rfq_id: RfqId,
        _updated_at_ms: TimestampMs,
    ) -> Result<RfqRequest> {
        let mut tx = self.begin().await?;
        let result = sqlx::query(
            "UPDATE rfqs
             SET status = 'cancelled'
             WHERE rfq_id = $1 AND status <> 'accepted'",
        )
        .bind(rfq_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        if result.rows_affected() != 1 {
            return Err(BackendError::InvalidRfqState(
                "RFQ cannot be cancelled".to_string(),
            ));
        }
        sqlx::query(
            "UPDATE rfq_quotes
             SET status = 'cancelled'
             WHERE rfq_id = $1 AND status = 'active'",
        )
        .bind(rfq_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        self.get_rfq(rfq_id)
            .await?
            .ok_or(BackendError::InvalidRfqId)
    }

    pub async fn reserve_next_trade_nonces(
        &self,
        buyer: &AccountId,
        seller: &AccountId,
        created_at_ms: TimestampMs,
    ) -> Result<(u64, u64)> {
        let mut tx = self.begin().await?;
        let buyer_nonce = reserve_next_nonce_tx(&mut tx, buyer, created_at_ms).await?;
        let seller_nonce = reserve_next_nonce_tx(&mut tx, seller, created_at_ms).await?;
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok((buyer_nonce, seller_nonce))
    }

    pub async fn next_trade_nonces(
        &self,
        buyer: &AccountId,
        seller: &AccountId,
    ) -> Result<(u64, u64)> {
        let mut tx = self.begin().await?;
        let buyer_nonce = next_nonce_tx(&mut tx, buyer).await?;
        let seller_nonce = if buyer == seller {
            buyer_nonce
                .checked_add(1)
                .ok_or(BackendError::InvalidNonce)?
        } else {
            next_nonce_tx(&mut tx, seller).await?
        };
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))?;
        Ok((buyer_nonce, seller_nonce))
    }

    pub async fn reserve_trade_nonces(
        &self,
        buyer: &AccountId,
        buyer_nonce: u64,
        seller: &AccountId,
        seller_nonce: u64,
        created_at_ms: TimestampMs,
    ) -> Result<()> {
        let mut tx = self.begin().await?;
        self.insert_nonce_tx(&mut tx, buyer, buyer_nonce, created_at_ms)
            .await?;
        self.insert_nonce_tx(&mut tx, seller, seller_nonce, created_at_ms)
            .await?;
        tx.commit()
            .await
            .map_err(|error| BackendError::Persistence(error.to_string()))
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
            simulation_id, intent_id, status, block_number, error,
            revert_data, revert_selector, decoded_error, created_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&simulation.simulation_id)
    .bind(&simulation.intent_id)
    .bind(&simulation.status)
    .bind(simulation.block_number)
    .bind(&simulation.error)
    .bind(&simulation.revert_data)
    .bind(&simulation.revert_selector)
    .bind(&simulation.decoded_error)
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

async fn insert_execution_transaction(
    pool: &PgPool,
    transaction: &ExecutionTransaction,
) -> Result<u64> {
    let result = sqlx::query(
        "INSERT INTO execution_transactions (
            transaction_id, intent_id, onchain_intent_id, target, calldata, value_wei,
            tx_hash, status, error, confirmed_at_ms, confirmed_block_number,
            confirmation_status, confirmation_error, created_at_ms, updated_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
    )
    .bind(&transaction.transaction_id)
    .bind(transaction.intent_id.to_string())
    .bind(&transaction.onchain_intent_id)
    .bind(&transaction.target.0)
    .bind(&transaction.calldata)
    .bind(&transaction.value_wei)
    .bind(&transaction.tx_hash)
    .bind(transaction.status.as_str())
    .bind(&transaction.error)
    .bind(transaction.confirmed_at_ms.map(timestamp_to_i64))
    .bind(
        transaction
            .confirmed_block_number
            .map(|value| u64_to_i64("confirmed_block_number", value))
            .transpose()?,
    )
    .bind(
        transaction
            .confirmation_status
            .map(ConfirmationStatus::as_str),
    )
    .bind(&transaction.confirmation_error)
    .bind(timestamp_to_i64(transaction.created_at_ms))
    .bind(timestamp_to_i64(transaction.updated_at_ms))
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

fn execution_transaction_from_row(row: PgRow) -> Result<ExecutionTransaction> {
    let intent_id: String = row_get(&row, "intent_id")?;
    let status: String = row_get(&row, "status")?;
    let confirmed_block_number: Option<i64> = row_get(&row, "confirmed_block_number")?;
    let confirmation_status: Option<String> = row_get(&row, "confirmation_status")?;
    Ok(ExecutionTransaction {
        transaction_id: row_get(&row, "transaction_id")?,
        intent_id: Uuid::parse_str(&intent_id)
            .map_err(|error| BackendError::Persistence(error.to_string()))?,
        onchain_intent_id: row_get(&row, "onchain_intent_id")?,
        target: AccountId::new(row_get::<String>(&row, "target")?),
        calldata: row_get(&row, "calldata")?,
        value_wei: row_get(&row, "value_wei")?,
        tx_hash: row_get(&row, "tx_hash")?,
        status: ExecutionTransactionStatus::parse(&status)?,
        error: row_get(&row, "error")?,
        confirmed_at_ms: row_get(&row, "confirmed_at_ms")?,
        confirmed_block_number: confirmed_block_number
            .map(|value| i64_to_u64_persistence("confirmed_block_number", value))
            .transpose()?,
        confirmation_status: confirmation_status
            .as_deref()
            .map(ConfirmationStatus::parse)
            .transpose()?,
        confirmation_error: row_get(&row, "confirmation_error")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn option_series_from_row(row: PgRow) -> Result<OptionSeries> {
    let expiry: i64 = row_get(&row, "expiry")?;
    let status: String = row_get(&row, "status")?;
    let source: String = row_get(&row, "source")?;
    Ok(OptionSeries {
        option_series_id: row_get(&row, "option_series_id")?,
        underlying: row_get(&row, "underlying")?,
        base_asset: row_get(&row, "base_asset")?,
        quote_asset: row_get(&row, "quote_asset")?,
        settlement_asset: row_get(&row, "settlement_asset")?,
        expiry: i64_to_u64_persistence("expiry", expiry)?,
        strike_1e8: row_get::<String>(&row, "strike_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option series strike: {error}"))
            })?,
        is_call: row_get(&row, "is_call")?,
        contract_size_1e8: row_get::<String>(&row, "contract_size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option contract size: {error}"))
            })?,
        status: OptionSeriesStatus::parse(&status)?,
        source: OptionSeriesSource::parse(&source)?,
        onchain_product_id: row_get(&row, "onchain_product_id")?,
        onchain_series_id: row_get(&row, "onchain_series_id")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn option_order_from_row(row: PgRow) -> Result<OptionOrder> {
    let order_id: String = row_get(&row, "order_id")?;
    let side: String = row_get(&row, "side")?;
    let time_in_force: String = row_get(&row, "time_in_force")?;
    let nonce: Option<String> = row_get(&row, "nonce")?;
    let status: String = row_get(&row, "status")?;
    Ok(OptionOrder {
        order_id: order_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option order id: {error}"))
        })?,
        option_series_id: row_get(&row, "option_series_id")?,
        account: AccountId::new(row_get::<String>(&row, "account")?),
        side: parse_side(&side)?,
        price_1e8: row_get::<String>(&row, "price_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option order price: {error}"))
            })?,
        size_1e8: row_get::<String>(&row, "size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option order size: {error}"))
            })?,
        remaining_size_1e8: row_get::<String>(&row, "remaining_size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option order remaining size: {error}"))
            })?,
        time_in_force: parse_time_in_force(&time_in_force)?,
        client_order_id: row_get(&row, "client_order_id")?,
        nonce: nonce
            .map(|value| {
                value.parse().map_err(|error| {
                    BackendError::Persistence(format!("invalid option order nonce: {error}"))
                })
            })
            .transpose()?,
        deadline_ms: row_get(&row, "deadline_ms")?,
        signature: row_get(&row, "signature")?,
        status: OptionOrderStatus::parse(&status)?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn option_fill_from_row(row: PgRow) -> Result<OptionFill> {
    let fill_id: String = row_get(&row, "fill_id")?;
    let buy_order_id: String = row_get(&row, "buy_order_id")?;
    let sell_order_id: String = row_get(&row, "sell_order_id")?;
    let maker_order_id: String = row_get(&row, "maker_order_id")?;
    let taker_order_id: String = row_get(&row, "taker_order_id")?;
    let taker_side: String = row_get(&row, "taker_side")?;
    Ok(OptionFill {
        fill_id: fill_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option fill id: {error}"))
        })?,
        option_series_id: row_get(&row, "option_series_id")?,
        buy_order_id: buy_order_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option fill buy order id: {error}"))
        })?,
        sell_order_id: sell_order_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option fill sell order id: {error}"))
        })?,
        buyer: AccountId::new(row_get::<String>(&row, "buyer")?),
        seller: AccountId::new(row_get::<String>(&row, "seller")?),
        maker_order_id: maker_order_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option fill maker order id: {error}"))
        })?,
        taker_order_id: taker_order_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option fill taker order id: {error}"))
        })?,
        taker_side: parse_side(&taker_side)?,
        price_1e8: row_get::<String>(&row, "price_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option fill price: {error}"))
            })?,
        size_1e8: row_get::<String>(&row, "size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option fill size: {error}"))
            })?,
        created_at_ms: row_get(&row, "created_at_ms")?,
    })
}

fn option_execution_intent_select_sql(suffix: &str) -> String {
    format!(
        "SELECT intent_id, onchain_intent_id, source_type, source_id, option_series_id,
                onchain_option_id, buyer, seller, underlying, settlement_asset, expiry,
                strike_1e8, is_call, contract_size_1e8, quantity_contracts, source_size_1e8,
                source_price_1e8, premium_per_contract_native, buyer_is_maker, buyer_nonce,
                seller_nonce, deadline, buyer_signature, seller_signature, calldata, status,
                error, simulation_status, simulation_error, simulation_block_number,
                simulation_revert_data, simulation_revert_selector, simulated_at_ms,
                created_at_ms, updated_at_ms
         FROM option_execution_intents {suffix}"
    )
}

fn option_execution_intent_from_row(row: PgRow) -> Result<OptionExecutionIntent> {
    let intent_id: String = row_get(&row, "intent_id")?;
    let source_type: String = row_get(&row, "source_type")?;
    let expiry: i64 = row_get(&row, "expiry")?;
    let deadline: i64 = row_get(&row, "deadline")?;
    let buyer_nonce: Option<String> = row_get(&row, "buyer_nonce")?;
    let seller_nonce: Option<String> = row_get(&row, "seller_nonce")?;
    let status: String = row_get(&row, "status")?;
    let simulation_status: Option<String> = row_get(&row, "simulation_status")?;
    let simulation_block_number: Option<i64> = row_get(&row, "simulation_block_number")?;
    Ok(OptionExecutionIntent {
        intent_id: intent_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option execution intent id: {error}"))
        })?,
        onchain_intent_id: row_get(&row, "onchain_intent_id")?,
        source_type: OptionExecutionSourceType::parse(&source_type)?,
        source_id: row_get(&row, "source_id")?,
        option_series_id: row_get(&row, "option_series_id")?,
        onchain_option_id: row_get(&row, "onchain_option_id")?,
        buyer: AccountId::new(row_get::<String>(&row, "buyer")?),
        seller: AccountId::new(row_get::<String>(&row, "seller")?),
        underlying: AccountId::new(row_get::<String>(&row, "underlying")?),
        settlement_asset: AccountId::new(row_get::<String>(&row, "settlement_asset")?),
        expiry: i64_to_u64_persistence("expiry", expiry)?,
        strike_1e8: row_get::<String>(&row, "strike_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option execution strike_1e8: {error}"))
            })?,
        is_call: row_get(&row, "is_call")?,
        contract_size_1e8: row_get::<String>(&row, "contract_size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!(
                    "invalid option execution contract_size_1e8: {error}"
                ))
            })?,
        quantity_contracts: row_get::<String>(&row, "quantity_contracts")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!(
                    "invalid option execution quantity_contracts: {error}"
                ))
            })?,
        source_size_1e8: row_get::<String>(&row, "source_size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!(
                    "invalid option execution source_size_1e8: {error}"
                ))
            })?,
        source_price_1e8: row_get::<String>(&row, "source_price_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!(
                    "invalid option execution source_price_1e8: {error}"
                ))
            })?,
        premium_per_contract_native: row_get::<String>(&row, "premium_per_contract_native")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!(
                    "invalid option execution premium_per_contract_native: {error}"
                ))
            })?,
        buyer_is_maker: row_get(&row, "buyer_is_maker")?,
        buyer_nonce: buyer_nonce
            .map(|value| {
                value.parse().map_err(|error| {
                    BackendError::Persistence(format!(
                        "invalid option execution buyer_nonce: {error}"
                    ))
                })
            })
            .transpose()?,
        seller_nonce: seller_nonce
            .map(|value| {
                value.parse().map_err(|error| {
                    BackendError::Persistence(format!(
                        "invalid option execution seller_nonce: {error}"
                    ))
                })
            })
            .transpose()?,
        deadline: i64_to_u64_persistence("deadline", deadline)?,
        buyer_signature: row_get(&row, "buyer_signature")?,
        seller_signature: row_get(&row, "seller_signature")?,
        calldata: row_get(&row, "calldata")?,
        status: OptionExecutionIntentStatus::parse(&status)?,
        error: row_get(&row, "error")?,
        simulation_status: simulation_status
            .as_deref()
            .map(OptionExecutionSimulationStatus::parse)
            .transpose()?,
        simulation_error: row_get(&row, "simulation_error")?,
        simulation_block_number: simulation_block_number
            .map(|value| i64_to_u64_persistence("simulation_block_number", value))
            .transpose()?,
        simulation_revert_data: row_get(&row, "simulation_revert_data")?,
        simulation_revert_selector: row_get(&row, "simulation_revert_selector")?,
        simulated_at_ms: row_get(&row, "simulated_at_ms")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn option_execution_transaction_from_row(row: PgRow) -> Result<OptionExecutionTransaction> {
    let intent_id: String = row_get(&row, "intent_id")?;
    let gas_limit: Option<i64> = row_get(&row, "gas_limit")?;
    let status: String = row_get(&row, "status")?;
    let estimated_gas: Option<i64> = row_get(&row, "estimated_gas")?;
    let required_gas: Option<i64> = row_get(&row, "required_gas")?;
    let simulation_gas_limit: Option<i64> = row_get(&row, "simulation_gas_limit")?;
    let broadcast_gas_limit: Option<i64> = row_get(&row, "broadcast_gas_limit")?;
    let gas_safety_bps: Option<i32> = row_get(&row, "gas_safety_bps")?;
    let gas_check_status: Option<String> = row_get(&row, "gas_check_status")?;
    let confirmation_status: Option<String> = row_get(&row, "confirmation_status")?;
    let confirmed_block_number: Option<i64> = row_get(&row, "confirmed_block_number")?;
    let receipt_status: Option<i64> = row_get(&row, "receipt_status")?;
    Ok(OptionExecutionTransaction {
        transaction_id: row_get(&row, "transaction_id")?,
        intent_id: intent_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option execution intent id: {error}"))
        })?,
        onchain_intent_id: row_get(&row, "onchain_intent_id")?,
        from: AccountId::new(row_get::<String>(&row, "sender")?),
        to: AccountId::new(row_get::<String>(&row, "target")?),
        calldata: row_get(&row, "calldata")?,
        value_wei: row_get(&row, "value_wei")?,
        gas_limit: gas_limit
            .map(|value| i64_to_u64_persistence("gas_limit", value))
            .transpose()?,
        tx_hash: row_get(&row, "tx_hash")?,
        status: ExecutionTransactionStatus::parse(&status)?,
        error: row_get(&row, "error")?,
        estimated_gas: estimated_gas
            .map(|value| i64_to_u64_persistence("estimated_gas", value))
            .transpose()?,
        required_gas: required_gas
            .map(|value| i64_to_u64_persistence("required_gas", value))
            .transpose()?,
        simulation_gas_limit: simulation_gas_limit
            .map(|value| i64_to_u64_persistence("simulation_gas_limit", value))
            .transpose()?,
        broadcast_gas_limit: broadcast_gas_limit
            .map(|value| i64_to_u64_persistence("broadcast_gas_limit", value))
            .transpose()?,
        gas_safety_bps: gas_safety_bps
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    BackendError::Persistence(format!(
                        "invalid option execution gas_safety_bps: {value}"
                    ))
                })
            })
            .transpose()?,
        gas_check_status: gas_check_status
            .as_deref()
            .map(OptionExecutionGasCheckStatus::parse)
            .transpose()?,
        gas_check_error: row_get(&row, "gas_check_error")?,
        confirmation_status: confirmation_status
            .as_deref()
            .map(OptionExecutionConfirmationStatus::parse)
            .transpose()?,
        confirmed_at_ms: row_get(&row, "confirmed_at_ms")?,
        confirmed_block_number: confirmed_block_number
            .map(|value| i64_to_u64_persistence("confirmed_block_number", value))
            .transpose()?,
        receipt_status: receipt_status
            .map(|value| i64_to_u64_persistence("receipt_status", value))
            .transpose()?,
        confirmation_error: row_get(&row, "confirmation_error")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn option_rfq_from_row(row: PgRow) -> Result<OptionRfqRequest> {
    let option_rfq_id: String = row_get(&row, "option_rfq_id")?;
    let side: String = row_get(&row, "side")?;
    let status: String = row_get(&row, "status")?;
    let accepted_quote_id: Option<String> = row_get(&row, "accepted_quote_id")?;
    let option_fill_id: Option<String> = row_get(&row, "option_fill_id")?;
    Ok(OptionRfqRequest {
        option_rfq_id: option_rfq_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option RFQ id: {error}"))
        })?,
        taker: AccountId::new(row_get::<String>(&row, "taker")?),
        option_series_id: row_get(&row, "option_series_id")?,
        side: parse_side(&side)?,
        size_1e8: row_get::<String>(&row, "size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option RFQ size: {error}"))
            })?,
        limit_price_1e8: row_get::<Option<String>>(&row, "limit_price_1e8")?
            .map(|value| {
                value.parse().map_err(|error| {
                    BackendError::Persistence(format!("invalid option RFQ limit price: {error}"))
                })
            })
            .transpose()?,
        status: OptionRfqStatus::parse(&status)?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        expires_at_ms: row_get(&row, "expires_at_ms")?,
        accepted_quote_id: accepted_quote_id
            .map(|value| {
                value.parse().map_err(|error| {
                    BackendError::Persistence(format!(
                        "invalid option RFQ accepted quote id: {error}"
                    ))
                })
            })
            .transpose()?,
        option_fill_id: option_fill_id
            .map(|value| {
                value.parse().map_err(|error| {
                    BackendError::Persistence(format!("invalid option RFQ fill id: {error}"))
                })
            })
            .transpose()?,
    })
}

fn option_rfq_quote_from_row(row: PgRow) -> Result<OptionRfqQuote> {
    let quote_id: String = row_get(&row, "quote_id")?;
    let option_rfq_id: String = row_get(&row, "option_rfq_id")?;
    let status: String = row_get(&row, "status")?;
    let signature_status: Option<String> = row_get(&row, "signature_status")?;
    let recovered_signer: Option<String> = row_get(&row, "recovered_signer")?;
    Ok(OptionRfqQuote {
        quote_id: quote_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option RFQ quote id: {error}"))
        })?,
        option_rfq_id: option_rfq_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option RFQ quote RFQ id: {error}"))
        })?,
        mm_account: AccountId::new(row_get::<String>(&row, "mm_account")?),
        session_id: row_get(&row, "session_id")?,
        client_quote_id: row_get(&row, "client_quote_id")?,
        price_1e8: row_get::<String>(&row, "price_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option RFQ quote price: {error}"))
            })?,
        size_1e8: row_get::<String>(&row, "size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option RFQ quote size: {error}"))
            })?,
        status: OptionRfqQuoteStatus::parse(&status)?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        expires_at_ms: row_get(&row, "expires_at_ms")?,
        signature: row_get(&row, "signature")?,
        quote_digest: row_get(&row, "quote_digest")?,
        quote_nonce: row_get(&row, "quote_nonce")?,
        signature_status: signature_status
            .as_deref()
            .map(OptionRfqQuoteSignatureStatus::parse)
            .transpose()?
            .unwrap_or(OptionRfqQuoteSignatureStatus::NotRequired),
        recovered_signer: recovered_signer.map(AccountId::new),
    })
}

fn option_rfq_fill_from_row(row: PgRow) -> Result<OptionRfqFill> {
    let fill_id: String = row_get(&row, "fill_id")?;
    let option_rfq_id: String = row_get(&row, "option_rfq_id")?;
    let quote_id: String = row_get(&row, "quote_id")?;
    let taker_side: String = row_get(&row, "taker_side")?;
    Ok(OptionRfqFill {
        fill_id: fill_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option RFQ fill id: {error}"))
        })?,
        option_rfq_id: option_rfq_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option RFQ fill RFQ id: {error}"))
        })?,
        quote_id: quote_id.parse().map_err(|error| {
            BackendError::Persistence(format!("invalid option RFQ fill quote id: {error}"))
        })?,
        option_series_id: row_get(&row, "option_series_id")?,
        buyer: AccountId::new(row_get::<String>(&row, "buyer")?),
        seller: AccountId::new(row_get::<String>(&row, "seller")?),
        taker: AccountId::new(row_get::<String>(&row, "taker")?),
        mm_account: AccountId::new(row_get::<String>(&row, "mm_account")?),
        taker_side: parse_side(&taker_side)?,
        price_1e8: row_get::<String>(&row, "price_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option RFQ fill price: {error}"))
            })?,
        size_1e8: row_get::<String>(&row, "size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid option RFQ fill size: {error}"))
            })?,
        created_at_ms: row_get(&row, "created_at_ms")?,
    })
}

fn mm_permission_account_from_row(row: PgRow) -> Result<MmAccountPermissions> {
    Ok(MmAccountPermissions {
        mm_account: AccountId::new(row_get::<String>(&row, "mm_account")?),
        enabled: row_get(&row, "enabled")?,
        label: row_get(&row, "label")?,
        can_submit_perp_orders: row_get(&row, "can_submit_perp_orders")?,
        can_quote_perp_rfq: row_get(&row, "can_quote_perp_rfq")?,
        can_quote_option_rfq: row_get(&row, "can_quote_option_rfq")?,
        can_submit_option_orders: row_get(&row, "can_submit_option_orders")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn mm_product_permission_from_row(row: PgRow) -> Result<MmProductPermission> {
    let market_id: Option<i64> = row_get(&row, "market_id")?;
    Ok(MmProductPermission {
        id: row_get(&row, "id")?,
        mm_account: AccountId::new(row_get::<String>(&row, "mm_account")?),
        market_id: market_id
            .map(|value| i64_to_u64_persistence("market_id", value))
            .transpose()?,
        option_series_id: row_get(&row, "option_series_id")?,
        enabled: row_get(&row, "enabled")?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn volume_bucket_from_row(row: PgRow) -> Result<VolumeBucket> {
    let market_type: String = row_get(&row, "market_type")?;
    Ok(VolumeBucket {
        bucket_id: row_get(&row, "bucket_id")?,
        account: AccountId::new(row_get::<String>(&row, "account")?),
        bucket_day: row_get(&row, "bucket_day")?,
        market_type: parse_fee_market_type(&market_type)?,
        maker_volume_1e8: row_get::<String>(&row, "maker_volume_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid maker fee volume: {error}"))
            })?,
        taker_volume_1e8: row_get::<String>(&row, "taker_volume_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid taker fee volume: {error}"))
            })?,
        total_volume_1e8: row_get::<String>(&row, "total_volume_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid total fee volume: {error}"))
            })?,
        updated_at_ms: row_get(&row, "updated_at_ms")?,
    })
}

fn parse_fee_market_type(value: &str) -> Result<FeeMarketType> {
    match value {
        "perp" => Ok(FeeMarketType::Perp),
        "option" => Ok(FeeMarketType::Option),
        other => Err(BackendError::Persistence(format!(
            "invalid fee market type: {other}"
        ))),
    }
}

fn insert_option_order_query<'q>(
    query: Query<'q, Postgres, PgArguments>,
    order: &'q OptionOrder,
) -> Query<'q, Postgres, PgArguments> {
    query
        .bind(order.order_id.to_string())
        .bind(&order.option_series_id)
        .bind(&order.account.0)
        .bind(side_to_str(order.side))
        .bind(order.price_1e8.to_string())
        .bind(order.size_1e8.to_string())
        .bind(order.remaining_size_1e8.to_string())
        .bind(tif_to_str(order.time_in_force))
        .bind(&order.client_order_id)
        .bind(order.nonce.map(|value| value.to_string()))
        .bind(order.deadline_ms.map(timestamp_to_i64))
        .bind(&order.signature)
        .bind(order.status.as_str())
        .bind(timestamp_to_i64(order.created_at_ms))
        .bind(timestamp_to_i64(order.updated_at_ms))
}

async fn insert_option_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    order: &OptionOrder,
) -> Result<()> {
    insert_option_order_query(
        sqlx::query(
            "INSERT INTO option_orders (
                order_id, option_series_id, account, side, price_1e8, size_1e8,
                remaining_size_1e8, time_in_force, client_order_id, nonce, deadline_ms,
                signature, status, created_at_ms, updated_at_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        ),
        order,
    )
    .execute(&mut **tx)
    .await
    .map(|_| ())
    .map_err(option_order_insert_error)
}

async fn update_option_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    order: &OptionOrder,
) -> Result<()> {
    sqlx::query(
        "UPDATE option_orders
         SET remaining_size_1e8 = $2, status = $3, updated_at_ms = $4
         WHERE order_id = $1",
    )
    .bind(order.order_id.to_string())
    .bind(order.remaining_size_1e8.to_string())
    .bind(order.status.as_str())
    .bind(timestamp_to_i64(order.updated_at_ms))
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_option_fill_tx(
    tx: &mut Transaction<'_, Postgres>,
    fill: &OptionFill,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO option_fills (
            fill_id, option_series_id, buy_order_id, sell_order_id, buyer, seller,
            maker_order_id, taker_order_id, taker_side, price_1e8, size_1e8, created_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(fill.fill_id.to_string())
    .bind(&fill.option_series_id)
    .bind(fill.buy_order_id.to_string())
    .bind(fill.sell_order_id.to_string())
    .bind(&fill.buyer.0)
    .bind(&fill.seller.0)
    .bind(fill.maker_order_id.to_string())
    .bind(fill.taker_order_id.to_string())
    .bind(side_to_str(fill.taker_side))
    .bind(fill.price_1e8.to_string())
    .bind(fill.size_1e8.to_string())
    .bind(timestamp_to_i64(fill.created_at_ms))
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

async fn insert_option_rfq_fill_tx(
    tx: &mut Transaction<'_, Postgres>,
    fill: &OptionRfqFill,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO option_rfq_fills (
            fill_id, option_rfq_id, quote_id, option_series_id, buyer, seller, taker,
            mm_account, taker_side, price_1e8, size_1e8, created_at_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(fill.fill_id.to_string())
    .bind(fill.option_rfq_id.to_string())
    .bind(fill.quote_id.to_string())
    .bind(&fill.option_series_id)
    .bind(&fill.buyer.0)
    .bind(&fill.seller.0)
    .bind(&fill.taker.0)
    .bind(&fill.mm_account.0)
    .bind(side_to_str(fill.taker_side))
    .bind(fill.price_1e8.to_string())
    .bind(fill.size_1e8.to_string())
    .bind(timestamp_to_i64(fill.created_at_ms))
    .execute(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(())
}

fn option_order_insert_error(error: sqlx::Error) -> BackendError {
    if is_unique_violation(&error) {
        BackendError::InvalidOptionOrderState(
            "duplicate open client_order_id for option account".to_string(),
        )
    } else {
        BackendError::Persistence(error.to_string())
    }
}

fn can_match_option_orders(incoming: &OptionOrder, resting: &OptionOrder) -> bool {
    incoming.option_series_id == resting.option_series_id
        && incoming.side != resting.side
        && resting.status.is_live()
        && resting.remaining_size_1e8 > 0
        && match incoming.side {
            Side::Buy => incoming.price_1e8 >= resting.price_1e8,
            Side::Sell => incoming.price_1e8 <= resting.price_1e8,
        }
}

fn sort_option_match_candidates(orders: &mut [OptionOrder], taker_side: Side) {
    orders.sort_by(|left, right| {
        let price_order = match taker_side {
            Side::Buy => left.price_1e8.cmp(&right.price_1e8),
            Side::Sell => right.price_1e8.cmp(&left.price_1e8),
        };
        price_order
            .then_with(|| left.created_at_ms.cmp(&right.created_at_ms))
            .then_with(|| left.order_id.cmp(&right.order_id))
    });
}

fn option_fill_from_match(
    incoming: &OptionOrder,
    maker: &OptionOrder,
    size_1e8: u128,
    created_at_ms: TimestampMs,
) -> OptionFill {
    let (buy_order, sell_order) = match incoming.side {
        Side::Buy => (incoming, maker),
        Side::Sell => (maker, incoming),
    };
    OptionFill {
        fill_id: Uuid::new_v4(),
        option_series_id: incoming.option_series_id.clone(),
        buy_order_id: buy_order.order_id,
        sell_order_id: sell_order.order_id,
        buyer: buy_order.account.clone(),
        seller: sell_order.account.clone(),
        maker_order_id: maker.order_id,
        taker_order_id: incoming.order_id,
        taker_side: incoming.side,
        price_1e8: maker.price_1e8,
        size_1e8,
        created_at_ms,
    }
}

fn side_to_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn parse_side(value: &str) -> Result<Side> {
    match value {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        other => Err(BackendError::Persistence(format!(
            "invalid option order side: {other}"
        ))),
    }
}

fn tif_to_str(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "gtc",
        TimeInForce::Ioc => "ioc",
        TimeInForce::Fok => "fok",
    }
}

fn parse_time_in_force(value: &str) -> Result<TimeInForce> {
    match value {
        "gtc" => Ok(TimeInForce::Gtc),
        "ioc" => Ok(TimeInForce::Ioc),
        "fok" => Ok(TimeInForce::Fok),
        other => Err(BackendError::Persistence(format!(
            "invalid option order time_in_force: {other}"
        ))),
    }
}

fn rfq_from_row(row: PgRow) -> Result<RfqRequest> {
    let rfq_id: String = row_get(&row, "rfq_id")?;
    let market_id: i64 = row_get(&row, "market_id")?;
    let side: String = row_get(&row, "side")?;
    let limit_price_1e8: Option<String> = row_get(&row, "limit_price_1e8")?;
    let status: String = row_get(&row, "status")?;
    let accepted_quote_id: Option<String> = row_get(&row, "accepted_quote_id")?;
    let execution_intent_id: Option<String> = row_get(&row, "execution_intent_id")?;
    Ok(RfqRequest {
        rfq_id: Uuid::parse_str(&rfq_id)
            .map_err(|error| BackendError::Persistence(error.to_string()))?,
        taker: AccountId::new(row_get::<String>(&row, "taker")?),
        market_id: i64_to_u64_persistence("market_id", market_id)?,
        side: match side.as_str() {
            "buy" => Side::Buy,
            "sell" => Side::Sell,
            other => {
                return Err(BackendError::Persistence(format!(
                    "invalid RFQ side: {other}"
                )))
            }
        },
        size_1e8: row_get::<String>(&row, "size_1e8")?
            .parse()
            .map_err(|error| BackendError::Persistence(format!("invalid RFQ size: {error}")))?,
        limit_price_1e8: limit_price_1e8
            .map(|value| {
                value.parse().map_err(|error| {
                    BackendError::Persistence(format!("invalid RFQ limit price: {error}"))
                })
            })
            .transpose()?,
        status: RfqStatus::parse(&status)?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        expires_at_ms: row_get(&row, "expires_at_ms")?,
        accepted_quote_id: accepted_quote_id
            .map(|value| {
                Uuid::parse_str(&value)
                    .map_err(|error| BackendError::Persistence(error.to_string()))
            })
            .transpose()?,
        execution_intent_id: execution_intent_id
            .map(|value| {
                Uuid::parse_str(&value)
                    .map_err(|error| BackendError::Persistence(error.to_string()))
            })
            .transpose()?,
    })
}

fn rfq_quote_from_row(row: PgRow) -> Result<RfqQuote> {
    let quote_id: String = row_get(&row, "quote_id")?;
    let rfq_id: String = row_get(&row, "rfq_id")?;
    let status: String = row_get(&row, "status")?;
    let signature_status: Option<String> = row_get(&row, "signature_status")?;
    let recovered_signer: Option<String> = row_get(&row, "recovered_signer")?;
    Ok(RfqQuote {
        quote_id: Uuid::parse_str(&quote_id)
            .map_err(|error| BackendError::Persistence(error.to_string()))?,
        rfq_id: Uuid::parse_str(&rfq_id)
            .map_err(|error| BackendError::Persistence(error.to_string()))?,
        mm_account: AccountId::new(row_get::<String>(&row, "mm_account")?),
        session_id: row_get(&row, "session_id")?,
        client_quote_id: row_get(&row, "client_quote_id")?,
        price_1e8: row_get::<String>(&row, "price_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid RFQ quote price: {error}"))
            })?,
        size_1e8: row_get::<String>(&row, "size_1e8")?
            .parse()
            .map_err(|error| {
                BackendError::Persistence(format!("invalid RFQ quote size: {error}"))
            })?,
        status: RfqQuoteStatus::parse(&status)?,
        created_at_ms: row_get(&row, "created_at_ms")?,
        expires_at_ms: row_get(&row, "expires_at_ms")?,
        signature: row_get(&row, "signature")?,
        quote_digest: row_get(&row, "quote_digest")?,
        quote_nonce: row_get(&row, "quote_nonce")?,
        signature_status: signature_status
            .as_deref()
            .map(RfqQuoteSignatureStatus::parse)
            .transpose()?
            .unwrap_or(RfqQuoteSignatureStatus::NotRequired),
        recovered_signer: recovered_signer.map(AccountId::new),
    })
}

async fn reserve_next_nonce_tx(
    tx: &mut Transaction<'_, Postgres>,
    account: &AccountId,
    created_at_ms: TimestampMs,
) -> Result<u64> {
    let next_nonce = next_nonce_tx(tx, account).await?;
    sqlx::query("INSERT INTO used_nonces (account, nonce, created_at_ms) VALUES ($1, $2, $3)")
        .bind(&account.0)
        .bind(u64_to_i64("nonce", next_nonce)?)
        .bind(timestamp_to_i64(created_at_ms))
        .execute(&mut **tx)
        .await
        .map_err(|error| BackendError::Persistence(error.to_string()))?;
    Ok(next_nonce)
}

async fn next_nonce_tx(tx: &mut Transaction<'_, Postgres>, account: &AccountId) -> Result<u64> {
    let row = sqlx::query(
        "SELECT COALESCE(MAX(nonce), 0) + 1 AS next_nonce
         FROM used_nonces
         WHERE account = $1",
    )
    .bind(&account.0)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| BackendError::Persistence(error.to_string()))?;
    let next_nonce: i64 = row_get(&row, "next_nonce")?;
    i64_to_u64_persistence("next_nonce", next_nonce)
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
