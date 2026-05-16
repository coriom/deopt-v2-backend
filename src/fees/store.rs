use super::types::{FeeEvent, FeeLedgerSummary, FeeMarketType, RebateAccrual, VolumeBucket};
use crate::error::{BackendError, Result};
use crate::types::{AccountId, TimestampMs};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct FeeLedgerStore {
    fee_events: BTreeMap<String, FeeEvent>,
    fee_event_keys: BTreeMap<(String, String, String, String), String>,
    volume_buckets: BTreeMap<(String, String, String), VolumeBucket>,
    rebate_accruals: BTreeMap<String, RebateAccrual>,
    rebate_keys: BTreeMap<(String, String), String>,
}

impl FeeLedgerStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_fee_event(&mut self, event: FeeEvent) -> Result<bool> {
        let unique_key = event.unique_key();
        if self.fee_event_keys.contains_key(&unique_key) {
            return Ok(false);
        }
        self.fee_event_keys
            .insert(unique_key, event.fee_event_id.clone());
        self.fee_events.insert(event.fee_event_id.clone(), event);
        Ok(true)
    }

    pub fn upsert_volume_delta(
        &mut self,
        account: &AccountId,
        bucket_day: &str,
        market_type: FeeMarketType,
        maker_delta_1e8: u128,
        taker_delta_1e8: u128,
        updated_at_ms: TimestampMs,
    ) -> Result<VolumeBucket> {
        let key = (
            account.0.to_ascii_lowercase(),
            bucket_day.to_string(),
            market_type.as_str().to_string(),
        );
        let bucket = self
            .volume_buckets
            .entry(key)
            .or_insert_with(|| VolumeBucket {
                bucket_id: VolumeBucket::key(account, bucket_day, market_type),
                account: account.clone(),
                bucket_day: bucket_day.to_string(),
                market_type,
                maker_volume_1e8: 0,
                taker_volume_1e8: 0,
                total_volume_1e8: 0,
                updated_at_ms,
            });
        bucket.maker_volume_1e8 =
            checked_add(bucket.maker_volume_1e8, maker_delta_1e8, "maker_volume_1e8")?;
        bucket.taker_volume_1e8 =
            checked_add(bucket.taker_volume_1e8, taker_delta_1e8, "taker_volume_1e8")?;
        bucket.total_volume_1e8 = checked_add(
            bucket.total_volume_1e8,
            checked_add(maker_delta_1e8, taker_delta_1e8, "volume_delta_1e8")?,
            "total_volume_1e8",
        )?;
        bucket.updated_at_ms = updated_at_ms;
        Ok(bucket.clone())
    }

    pub fn insert_rebate_accrual(&mut self, accrual: RebateAccrual) -> Result<bool> {
        let unique_key = accrual.unique_key();
        if self.rebate_keys.contains_key(&unique_key) {
            return Ok(false);
        }
        self.rebate_keys
            .insert(unique_key, accrual.rebate_id.clone());
        self.rebate_accruals
            .insert(accrual.rebate_id.clone(), accrual);
        Ok(true)
    }

    pub fn list_fee_events(&self, limit: usize) -> Vec<FeeEvent> {
        let mut events = self.fee_events.values().cloned().collect::<Vec<_>>();
        events.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| right.fee_event_id.cmp(&left.fee_event_id))
        });
        events.truncate(limit);
        events
    }

    pub fn list_volume_buckets(&self, account: Option<&AccountId>) -> Vec<VolumeBucket> {
        let mut buckets = self
            .volume_buckets
            .values()
            .filter(|bucket| {
                account.map_or(true, |account| {
                    bucket.account.0.eq_ignore_ascii_case(account.0.as_str())
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        buckets.sort_by(|left, right| {
            right
                .bucket_day
                .cmp(&left.bucket_day)
                .then_with(|| left.account.0.cmp(&right.account.0))
                .then_with(|| left.market_type.as_str().cmp(right.market_type.as_str()))
        });
        buckets
    }

    pub fn list_rebate_accruals(&self, account: Option<&AccountId>) -> Vec<RebateAccrual> {
        let mut rebates = self
            .rebate_accruals
            .values()
            .filter(|rebate| {
                account.map_or(true, |account| {
                    rebate.account.0.eq_ignore_ascii_case(account.0.as_str())
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        rebates.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| right.rebate_id.cmp(&left.rebate_id))
        });
        rebates
    }

    pub fn rolling_volume_since(
        &self,
        account: &AccountId,
        market_type: FeeMarketType,
        start_bucket_day: &str,
    ) -> Result<u128> {
        self.volume_buckets
            .values()
            .filter(|bucket| {
                bucket.account.0.eq_ignore_ascii_case(account.0.as_str())
                    && bucket.market_type == market_type
                    && bucket.bucket_day.as_str() >= start_bucket_day
            })
            .try_fold(0u128, |total, bucket| {
                checked_add(total, bucket.total_volume_1e8, "rolling_volume_1e8")
            })
    }

    pub fn summary(&self) -> Result<FeeLedgerSummary> {
        let mut summary = FeeLedgerSummary::default();
        for event in self.fee_events.values() {
            summary.event_count = summary.event_count.checked_add(1).ok_or_else(|| {
                BackendError::Config("fee ledger event count overflow".to_string())
            })?;
            summary.fee_total_1e8 =
                checked_add(summary.fee_total_1e8, event.fee_amount_1e8, "fee_total_1e8")?;
            summary.rebate_total_1e8 = checked_add(
                summary.rebate_total_1e8,
                event.rebate_amount_1e8,
                "rebate_total_1e8",
            )?;
            summary.protocol_total_1e8 = checked_add(
                summary.protocol_total_1e8,
                event.protocol_amount_1e8,
                "protocol_total_1e8",
            )?;
            bump_count(&mut summary.status_counts, event.status.as_str())?;
            bump_count(&mut summary.source_type_counts, event.source_type.as_str())?;
            bump_count(&mut summary.market_type_counts, event.market_type.as_str())?;
        }
        Ok(summary)
    }
}

fn checked_add(left: u128, right: u128, field: &str) -> Result<u128> {
    left.checked_add(right)
        .ok_or_else(|| BackendError::Config(format!("fee ledger arithmetic overflow for {field}")))
}

fn bump_count(counts: &mut BTreeMap<String, u64>, key: &str) -> Result<()> {
    let count = counts.entry(key.to_string()).or_default();
    *count = count
        .checked_add(1)
        .ok_or_else(|| BackendError::Config("fee ledger count overflow".to_string()))?;
    Ok(())
}
