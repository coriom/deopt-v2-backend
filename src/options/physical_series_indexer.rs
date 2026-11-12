//! Confirmed Registry events for the dedicated physical Options stack.
//! This never turns a series into an executable order or attributes a position.
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::execution::{EthGetLogsFilter, EthLogsProvider};
use crate::indexer::decoder::{decode_hex_bytes, hex_quantity, parse_hex_quantity};
use crate::indexer::EthLog;
use crate::options::onchain_option_series_key;
use crate::signing::eip712::keccak256;
use alloy_primitives::U256;
use serde::Serialize;
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::collections::BTreeMap;

const CREATED: &str = "SeriesCreated(uint256,address,address,uint64,uint64,uint128,bool,bool)";
const STATUS: &str = "SeriesStatusUpdated(uint256,bool)";

fn topic(signature: &str) -> String {
    format!("0x{}", hex::encode(keccak256(signature.as_bytes())))
}

fn index_error(message: &str) -> BackendError {
    BackendError::Indexer(message.into())
}

fn db_error(error: sqlx::Error) -> BackendError {
    BackendError::Persistence(error.to_string())
}

fn word(data: &[u8], index: usize) -> Result<U256> {
    data.get(index * 32..(index + 1) * 32)
        .map(U256::from_be_slice)
        .ok_or_else(|| index_error("invalid Registry event ABI"))
}

fn narrow_u64(value: U256) -> Result<u64> {
    u64::try_from(value).map_err(|_| index_error("Registry uint64 overflow"))
}

fn narrow_u128(value: U256) -> Result<u128> {
    u128::try_from(value).map_err(|_| index_error("Registry uint128 overflow"))
}

fn decode_bool(value: U256) -> Result<bool> {
    if value > U256::from(1) {
        return Err(index_error("invalid Registry boolean"));
    }
    Ok(value == U256::from(1))
}

fn topic_address(value: &str) -> Result<String> {
    let bytes = decode_hex_bytes(value)?;
    if bytes.len() != 32 || bytes[..12].iter().any(|byte| *byte != 0) {
        return Err(index_error("invalid Registry address topic"));
    }
    Ok(format!("0x{}", hex::encode(&bytes[12..])))
}

#[derive(Clone, Debug)]
struct SeriesLog {
    chain_id: u64,
    registry: String,
    option_id: String,
    tx_hash: String,
    block_number: u64,
    block_hash: String,
    log_index: u64,
    event_name: &'static str,
    underlying: Option<String>,
    settlement_asset: Option<String>,
    expiry: Option<u64>,
    strike_1e8: Option<u128>,
    contract_size_1e8: Option<u128>,
    is_call: Option<bool>,
    is_european: Option<bool>,
    is_active: bool,
}

fn decode_log(log: &EthLog, chain_id: u64, registry: &str) -> Result<SeriesLog> {
    if !log.address.eq_ignore_ascii_case(registry) {
        return Err(index_error("foreign Registry event"));
    }
    let signature = log
        .topics
        .first()
        .ok_or_else(|| index_error("missing topic0"))?;
    let created = signature.eq_ignore_ascii_case(&topic(CREATED));
    let updated = signature.eq_ignore_ascii_case(&topic(STATUS));
    if !created && !updated {
        return Err(index_error("unexpected Registry event"));
    }
    if log.topics.len() != if created { 4 } else { 2 } {
        return Err(index_error("invalid Registry topic count"));
    }
    let option_topic = decode_hex_bytes(&log.topics[1])?;
    if option_topic.len() != 32 {
        return Err(index_error("invalid Registry optionId topic"));
    }
    let option_id = U256::from_be_slice(&option_topic).to_string();
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != if created { 5 * 32 } else { 32 } {
        return Err(index_error("invalid Registry event data length"));
    }
    let tx_hash = log
        .transaction_hash
        .as_deref()
        .ok_or_else(|| index_error("missing tx hash"))?;
    let block_hash = log
        .block_hash
        .as_deref()
        .ok_or_else(|| index_error("missing block hash"))?;
    if decode_hex_bytes(tx_hash)?.len() != 32 || decode_hex_bytes(block_hash)?.len() != 32 {
        return Err(index_error("invalid Registry provenance hash"));
    }
    Ok(SeriesLog {
        chain_id,
        registry: registry.to_ascii_lowercase(),
        option_id,
        tx_hash: tx_hash.to_ascii_lowercase(),
        block_number: parse_hex_quantity(
            log.block_number
                .as_deref()
                .ok_or_else(|| index_error("missing block number"))?,
        )?,
        block_hash: block_hash.to_ascii_lowercase(),
        log_index: parse_hex_quantity(
            log.log_index
                .as_deref()
                .ok_or_else(|| index_error("missing log index"))?,
        )?,
        event_name: if created {
            "SeriesCreated"
        } else {
            "SeriesStatusUpdated"
        },
        underlying: if created {
            Some(topic_address(&log.topics[2])?)
        } else {
            None
        },
        settlement_asset: if created {
            Some(topic_address(&log.topics[3])?)
        } else {
            None
        },
        expiry: if created {
            Some(narrow_u64(word(&data, 0)?)?)
        } else {
            None
        },
        strike_1e8: if created {
            Some(narrow_u64(word(&data, 1)?)? as u128)
        } else {
            None
        },
        contract_size_1e8: if created {
            Some(narrow_u128(word(&data, 2)?)?)
        } else {
            None
        },
        is_call: if created {
            Some(decode_bool(word(&data, 3)?)?)
        } else {
            None
        },
        is_european: if created {
            Some(decode_bool(word(&data, 4)?)?)
        } else {
            None
        },
        is_active: if created {
            true
        } else {
            decode_bool(word(&data, 0)?)?
        },
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct PhysicalSeriesProjection {
    pub chain_id: u64,
    pub registry_address: String,
    pub option_id: String,
    pub series_id: String,
    pub underlying: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: String,
    pub contract_size_1e8: String,
    pub is_call: bool,
    pub is_european: bool,
    pub is_active: bool,
    pub creation_tx_hash: String,
    pub creation_block_number: u64,
    pub creation_block_hash: String,
    pub creation_log_index: u64,
    pub last_tx_hash: String,
    pub last_block_number: u64,
    pub last_block_hash: String,
    pub last_log_index: u64,
    pub confirmed_at_head: u64,
}

fn i64_value(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| index_error("block or chain exceeds PostgreSQL BIGINT"))
}

fn u64_value(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| index_error("negative Registry projection integer"))
}

pub async fn list_physical_series(
    pool: &PgPool,
    chain_id: u64,
    registry: &str,
) -> Result<Vec<PhysicalSeriesProjection>> {
    let rows = sqlx::query(
        "SELECT option_id, underlying, settlement_asset, expiry, strike_1e8, contract_size_1e8,
                is_call, is_european, is_active, creation_tx_hash, creation_block_number,
                creation_block_hash, creation_log_index, last_tx_hash, last_block_number,
                last_block_hash, last_log_index, confirmed_at_head
         FROM physical_option_series_projection
         WHERE chain_id=$1 AND registry_address=$2 ORDER BY expiry, strike_1e8, option_id",
    )
    .bind(i64_value(chain_id)?)
    .bind(registry.to_ascii_lowercase())
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    rows.into_iter()
        .map(|row| {
            let option_id: String = row.try_get("option_id").map_err(db_error)?;
            Ok(PhysicalSeriesProjection {
                chain_id,
                registry_address: registry.to_ascii_lowercase(),
                series_id: onchain_option_series_key(chain_id, registry, &option_id)?,
                option_id,
                underlying: row.try_get("underlying").map_err(db_error)?,
                settlement_asset: row.try_get("settlement_asset").map_err(db_error)?,
                expiry: u64_value(row.try_get("expiry").map_err(db_error)?)?,
                strike_1e8: row.try_get("strike_1e8").map_err(db_error)?,
                contract_size_1e8: row.try_get("contract_size_1e8").map_err(db_error)?,
                is_call: row.try_get("is_call").map_err(db_error)?,
                is_european: row.try_get("is_european").map_err(db_error)?,
                is_active: row.try_get("is_active").map_err(db_error)?,
                creation_tx_hash: row.try_get("creation_tx_hash").map_err(db_error)?,
                creation_block_number: u64_value(
                    row.try_get("creation_block_number").map_err(db_error)?,
                )?,
                creation_block_hash: row.try_get("creation_block_hash").map_err(db_error)?,
                creation_log_index: u64_value(
                    row.try_get("creation_log_index").map_err(db_error)?,
                )?,
                last_tx_hash: row.try_get("last_tx_hash").map_err(db_error)?,
                last_block_number: u64_value(row.try_get("last_block_number").map_err(db_error)?)?,
                last_block_hash: row.try_get("last_block_hash").map_err(db_error)?,
                last_log_index: u64_value(row.try_get("last_log_index").map_err(db_error)?)?,
                confirmed_at_head: u64_value(row.try_get("confirmed_at_head").map_err(db_error)?)?,
            })
        })
        .collect()
}

async fn cursor(pool: &PgPool, chain_id: u64, registry: &str) -> Result<Option<(u64, String)>> {
    let row = sqlx::query("SELECT block_number, block_hash FROM physical_option_series_cursor WHERE chain_id=$1 AND registry_address=$2")
        .bind(i64_value(chain_id)?).bind(registry).fetch_optional(pool).await.map_err(db_error)?;
    row.map(|row| {
        Ok((
            u64_value(row.try_get("block_number").map_err(db_error)?)?,
            row.try_get("block_hash").map_err(db_error)?,
        ))
    })
    .transpose()
}

async fn rewind(pool: &PgPool, chain_id: u64, registry: &str) -> Result<()> {
    let mut tx = pool.begin().await.map_err(db_error)?;
    sqlx::query(
        "DELETE FROM physical_option_series_projection WHERE chain_id=$1 AND registry_address=$2",
    )
    .bind(i64_value(chain_id)?)
    .bind(registry)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    sqlx::query(
        "DELETE FROM physical_option_series_events WHERE chain_id=$1 AND registry_address=$2",
    )
    .bind(i64_value(chain_id)?)
    .bind(registry)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    sqlx::query(
        "DELETE FROM physical_option_series_cursor WHERE chain_id=$1 AND registry_address=$2",
    )
    .bind(i64_value(chain_id)?)
    .bind(registry)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}

async fn rebuild(
    tx: &mut Transaction<'_, Postgres>,
    chain_id: u64,
    registry: &str,
    safe_head: u64,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT option_id, tx_hash, block_number, block_hash, log_index, event_name,
                underlying, settlement_asset, expiry, strike_1e8, contract_size_1e8,
                is_call, is_european, is_active
         FROM physical_option_series_events WHERE chain_id=$1 AND registry_address=$2
         ORDER BY block_number, log_index",
    )
    .bind(i64_value(chain_id)?)
    .bind(registry)
    .fetch_all(&mut **tx)
    .await
    .map_err(db_error)?;
    let mut projection = BTreeMap::<String, PhysicalSeriesProjection>::new();
    for row in rows {
        let option_id: String = row.try_get("option_id").map_err(db_error)?;
        let name: String = row.try_get("event_name").map_err(db_error)?;
        let tx_hash: String = row.try_get("tx_hash").map_err(db_error)?;
        let block_hash: String = row.try_get("block_hash").map_err(db_error)?;
        let block_number = u64_value(row.try_get("block_number").map_err(db_error)?)?;
        let log_index = u64_value(row.try_get("log_index").map_err(db_error)?)?;
        let active: bool = row.try_get("is_active").map_err(db_error)?;
        if name == "SeriesCreated" {
            if projection.contains_key(&option_id) {
                return Err(index_error("duplicate canonical SeriesCreated"));
            }
            projection.insert(
                option_id.clone(),
                PhysicalSeriesProjection {
                    chain_id,
                    registry_address: registry.into(),
                    series_id: onchain_option_series_key(chain_id, registry, &option_id)?,
                    option_id,
                    underlying: row
                        .try_get::<Option<String>, _>("underlying")
                        .map_err(db_error)?
                        .ok_or_else(|| index_error("creation underlying missing"))?,
                    settlement_asset: row
                        .try_get::<Option<String>, _>("settlement_asset")
                        .map_err(db_error)?
                        .ok_or_else(|| index_error("creation settlement missing"))?,
                    expiry: u64_value(
                        row.try_get::<Option<i64>, _>("expiry")
                            .map_err(db_error)?
                            .ok_or_else(|| index_error("creation expiry missing"))?,
                    )?,
                    strike_1e8: row
                        .try_get::<Option<String>, _>("strike_1e8")
                        .map_err(db_error)?
                        .ok_or_else(|| index_error("creation strike missing"))?,
                    contract_size_1e8: row
                        .try_get::<Option<String>, _>("contract_size_1e8")
                        .map_err(db_error)?
                        .ok_or_else(|| index_error("creation size missing"))?,
                    is_call: row
                        .try_get::<Option<bool>, _>("is_call")
                        .map_err(db_error)?
                        .ok_or_else(|| index_error("creation call flag missing"))?,
                    is_european: row
                        .try_get::<Option<bool>, _>("is_european")
                        .map_err(db_error)?
                        .ok_or_else(|| index_error("creation exercise flag missing"))?,
                    is_active: active,
                    creation_tx_hash: tx_hash.clone(),
                    creation_block_number: block_number,
                    creation_block_hash: block_hash.clone(),
                    creation_log_index: log_index,
                    last_tx_hash: tx_hash,
                    last_block_number: block_number,
                    last_block_hash: block_hash,
                    last_log_index: log_index,
                    confirmed_at_head: safe_head,
                },
            );
        } else {
            let entry = projection
                .get_mut(&option_id)
                .ok_or_else(|| index_error("status without canonical creation"))?;
            entry.is_active = active;
            entry.last_tx_hash = tx_hash;
            entry.last_block_number = block_number;
            entry.last_block_hash = block_hash;
            entry.last_log_index = log_index;
            entry.confirmed_at_head = safe_head;
        }
    }
    sqlx::query(
        "DELETE FROM physical_option_series_projection WHERE chain_id=$1 AND registry_address=$2",
    )
    .bind(i64_value(chain_id)?)
    .bind(registry)
    .execute(&mut **tx)
    .await
    .map_err(db_error)?;
    for entry in projection.values() {
        sqlx::query(
            "INSERT INTO physical_option_series_projection
             (chain_id, registry_address, option_id, underlying, settlement_asset, expiry, strike_1e8,
              contract_size_1e8, is_call, is_european, is_active, creation_tx_hash,
              creation_block_number, creation_block_hash, creation_log_index, last_tx_hash,
              last_block_number, last_block_hash, last_log_index, confirmed_at_head)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)"
        ).bind(i64_value(chain_id)?).bind(registry).bind(&entry.option_id).bind(&entry.underlying)
         .bind(&entry.settlement_asset).bind(i64_value(entry.expiry)?).bind(&entry.strike_1e8)
         .bind(&entry.contract_size_1e8).bind(entry.is_call).bind(entry.is_european)
         .bind(entry.is_active).bind(&entry.creation_tx_hash).bind(i64_value(entry.creation_block_number)?)
         .bind(&entry.creation_block_hash).bind(i64_value(entry.creation_log_index)?)
         .bind(&entry.last_tx_hash).bind(i64_value(entry.last_block_number)?)
         .bind(&entry.last_block_hash).bind(i64_value(entry.last_log_index)?)
         .bind(i64_value(entry.confirmed_at_head)?).execute(&mut **tx).await.map_err(db_error)?;
    }
    Ok(())
}

/// One confirmed batch. Returns the number of newly inserted event logs.
pub async fn index_physical_series_with_provider<P: EthLogsProvider>(
    state: &AppState,
    provider: &P,
) -> Result<u64> {
    if !state.option_event_indexer_config.enabled {
        return Ok(0);
    }
    let Some(address) = state.trading_views.physical_registry_address.as_ref() else {
        return Ok(0);
    };
    let registry = address.0.to_ascii_lowercase();
    let chain_id = state.chain_id;
    let repo = state
        .repository
        .as_ref()
        .ok_or_else(|| index_error("physical series requires PostgreSQL"))?;
    let pool = repo.pool();
    let config = &state.option_event_indexer_config;
    let mut previous = cursor(pool, chain_id, &registry).await?;
    if let Some((block, hash)) = previous.as_ref() {
        if provider.block_hash(*block).await?.as_deref() != Some(hash.as_str()) {
            rewind(pool, chain_id, &registry).await?;
            previous = None;
        }
    }
    let from = previous
        .as_ref()
        .map(|(block, _)| *block)
        .unwrap_or(config.from_block)
        .saturating_add(1);
    let current = provider.block_number().await?;
    let safe_head = current.saturating_sub(config.confirmation_blocks);
    if from > safe_head {
        return Ok(0);
    }
    let to = safe_head.min(from.saturating_add(config.batch_blocks).saturating_sub(1));
    let before = provider
        .block_hash(to)
        .await?
        .ok_or_else(|| index_error("canonical block hash unavailable"))?;
    let mut events = Vec::new();
    for signature in [CREATED, STATUS] {
        for log in provider
            .get_logs(EthGetLogsFilter {
                from_block: hex_quantity(from),
                to_block: hex_quantity(to),
                address: registry.clone(),
                topics: vec![topic(signature)],
            })
            .await?
        {
            let event = decode_log(&log, chain_id, &registry)?;
            if event.block_number < from
                || event.block_number > to
                || provider.block_hash(event.block_number).await?.as_deref()
                    != Some(event.block_hash.as_str())
            {
                return Err(index_error("noncanonical Registry log"));
            }
            events.push(event);
        }
    }
    if provider.block_hash(to).await?.as_deref() != Some(before.as_str()) {
        return Err(index_error("Registry chain changed during index tick"));
    }
    let mut tx = pool.begin().await.map_err(db_error)?;
    let locked = sqlx::query("SELECT block_number, block_hash FROM physical_option_series_cursor WHERE chain_id=$1 AND registry_address=$2 FOR UPDATE")
        .bind(i64_value(chain_id)?).bind(&registry).fetch_optional(&mut *tx).await.map_err(db_error)?;
    let actual = locked
        .map(|row| -> Result<(u64, String)> {
            Ok((
                u64_value(row.try_get("block_number").map_err(db_error)?)?,
                row.try_get("block_hash").map_err(db_error)?,
            ))
        })
        .transpose()?;
    if actual != previous {
        return Err(index_error("physical series cursor changed concurrently"));
    }
    let mut inserted = 0;
    for event in events {
        let result = sqlx::query(
            "INSERT INTO physical_option_series_events
             (chain_id,registry_address,option_id,tx_hash,block_number,block_hash,log_index,event_name,
              underlying,settlement_asset,expiry,strike_1e8,contract_size_1e8,is_call,is_european,is_active)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
             ON CONFLICT DO NOTHING"
        ).bind(i64_value(event.chain_id)?).bind(&event.registry).bind(&event.option_id)
         .bind(&event.tx_hash).bind(i64_value(event.block_number)?).bind(&event.block_hash)
         .bind(i64_value(event.log_index)?).bind(event.event_name).bind(&event.underlying)
         .bind(&event.settlement_asset).bind(event.expiry.map(i64_value).transpose()?)
         .bind(event.strike_1e8.map(|v| v.to_string())).bind(event.contract_size_1e8.map(|v| v.to_string()))
         .bind(event.is_call).bind(event.is_european).bind(event.is_active)
         .execute(&mut *tx).await.map_err(db_error)?;
        inserted += result.rows_affected();
    }
    rebuild(&mut tx, chain_id, &registry, safe_head).await?;
    sqlx::query(
        "INSERT INTO physical_option_series_cursor (chain_id,registry_address,block_number,block_hash)
         VALUES ($1,$2,$3,$4) ON CONFLICT (chain_id,registry_address)
         DO UPDATE SET block_number=EXCLUDED.block_number,block_hash=EXCLUDED.block_hash"
    ).bind(i64_value(chain_id)?).bind(&registry).bind(i64_value(to)?).bind(&before)
     .execute(&mut *tx).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(inserted)
}
