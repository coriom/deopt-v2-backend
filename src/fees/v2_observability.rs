//! V2G-G: read-only V2 fee observability summary endpoint backing.
//!
//! Exposes a single JSON snapshot of the V2 fee surface that the
//! Prometheus / Grafana stack visualises:
//!
//! - configured NEW / OLD PerpEngine and MarginEngine addresses (the
//!   exact strings the classifier is using right now),
//! - configured FeesManagerV2 address (from the option event indexer),
//! - per-bucket counts (`new` / `old` / `unknown`) for each of the four
//!   V2 fee gauges,
//! - derived FeesManagerV2 rebate budget per settlement asset (the same
//!   value the `deopt_fees_manager_v2_rebate_budget_native{asset=...}`
//!   gauge surfaces),
//! - feature-flag status (metrics, indexer, fees).
//!
//! The endpoint is read-only and is intentionally a thin re-aggregation
//! of the same data sources the metric pipeline uses. It does NOT call
//! into the chain, sign anything, or take any locks beyond the same
//! short critical sections the existing `/metrics` and admin handlers
//! already take.
//!
//! Cardinality contract is preserved:
//! - the same three-bucket classifier (`new`/`old`/`unknown`) is reused
//!   verbatim from `crate::fees::perp_consumer::classify_perp_fee_consumer`
//!   and `crate::fees::option_consumer::classify_option_fee_consumer`,
//!   so raw addresses cannot leak into the bucket counts;
//! - the rebate-budget map is keyed by lowercased settlement-asset
//!   address (one entry per supported asset).
//!
//! See `docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md` for the
//! deployment-side overview.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::fees::option_consumer::classify_option_fee_consumer;
use crate::fees::perp_consumer::classify_perp_fee_consumer;

const CONSUMER_BUCKETS: [&str; 3] = ["new", "old", "unknown"];

/// Build the `/admin/fees/v2/observability` JSON snapshot.
pub async fn admin_v2_observability(state: &AppState) -> Result<Value> {
    let perp_engine_new =
        non_zero_address_owned(state.execution_config.perp_engine_address.0.as_str());
    let perp_engine_old = state
        .execution_config
        .old_perp_engine_address
        .as_ref()
        .and_then(|addr| non_zero_address_owned(addr.0.as_str()));
    let margin_engine_new = non_zero_address_owned(
        state
            .option_event_indexer_config
            .margin_engine_address
            .0
            .as_str(),
    );
    let margin_engine_old = state
        .option_event_indexer_config
        .old_margin_engine_address
        .as_ref()
        .and_then(|addr| non_zero_address_owned(addr.0.as_str()));
    let fees_manager_v2 = state
        .option_event_indexer_config
        .fees_manager_v2_address
        .as_ref()
        .and_then(|addr| non_zero_address_owned(addr.0.as_str()));

    let raw = load_raw_counts(state).await?;

    let perp_charged = classify_perp_counts(
        &raw.perp_charged,
        perp_engine_new.as_deref(),
        perp_engine_old.as_deref(),
    );
    let perp_rebated = classify_perp_counts(
        &raw.perp_rebated,
        perp_engine_new.as_deref(),
        perp_engine_old.as_deref(),
    );
    let option_charged = classify_option_counts(
        &raw.option_charged,
        margin_engine_new.as_deref(),
        margin_engine_old.as_deref(),
    );
    let option_rebated = classify_option_counts(
        &raw.option_rebated,
        margin_engine_new.as_deref(),
        margin_engine_old.as_deref(),
    );

    let rebate_budget_by_asset = raw
        .rebate_budget_by_asset
        .into_iter()
        .map(|(asset, value)| (asset, json!(value)))
        .collect::<serde_json::Map<_, _>>();

    let totals_unknown = bucket_value(&perp_charged, "unknown")
        + bucket_value(&perp_rebated, "unknown")
        + bucket_value(&option_charged, "unknown")
        + bucket_value(&option_rebated, "unknown");
    let totals_old = bucket_value(&perp_charged, "old")
        + bucket_value(&perp_rebated, "old")
        + bucket_value(&option_charged, "old")
        + bucket_value(&option_rebated, "old");

    Ok(json!({
        "milestone": "V2G-G",
        "network": {
            "chain_id": state.chain_id,
            "network_name": state.network_name,
        },
        "features": {
            "metrics_enabled": state.metrics_config.enabled,
            "option_event_indexer_enabled": state.option_event_indexer_config.enabled,
            "fees_enabled": state.fees_config.enabled,
            "rebates_enabled": state.fees_config.rebates_enabled,
            "persistence_enabled": state.persistence_enabled,
        },
        "contracts": {
            "perp_engine_new": perp_engine_new,
            "perp_engine_old": perp_engine_old,
            "margin_engine_new": margin_engine_new,
            "margin_engine_old": margin_engine_old,
            "fees_manager_v2": fees_manager_v2,
        },
        "metrics": {
            "perp_fee_charged_v2_by_consumer": perp_charged,
            "perp_fee_rebated_v2_by_consumer": perp_rebated,
            "option_fee_charged_v2_by_consumer": option_charged,
            "option_fee_rebated_v2_by_consumer": option_rebated,
            "fees_manager_v2_rebate_budget_native": rebate_budget_by_asset,
        },
        "anomaly_totals": {
            "old_consumer_events": totals_old,
            "unknown_consumer_events": totals_unknown,
        },
        "notes": [
            "Raw addresses are never promoted to bucket labels (consumer in {new,old,unknown}).",
            "rebate_budget_native is derived from indexed RebateBudgetFunded/Spent/Withdrawn events, clamped at zero.",
            "Read-only snapshot. See docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md.",
        ],
    }))
}

struct RawCounts {
    perp_charged: BTreeMap<String, u64>,
    perp_rebated: BTreeMap<String, u64>,
    option_charged: BTreeMap<String, u64>,
    option_rebated: BTreeMap<String, u64>,
    rebate_budget_by_asset: BTreeMap<String, u64>,
}

async fn load_raw_counts(state: &AppState) -> Result<RawCounts> {
    if let Some(repository) = state.repository.clone() {
        if repository.admin_ping().await.is_ok() {
            return Ok(RawCounts {
                perp_charged: repository.admin_perp_fee_v2_consumer_counts().await?,
                perp_rebated: repository
                    .admin_perp_fee_v2_rebated_consumer_counts()
                    .await?,
                option_charged: repository.admin_option_fee_v2_consumer_counts().await?,
                option_rebated: repository
                    .admin_option_fee_v2_rebated_consumer_counts()
                    .await?,
                rebate_budget_by_asset: repository
                    .admin_fees_manager_v2_rebate_budget_by_asset()
                    .await?,
            });
        }
    }

    let store = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
    Ok(RawCounts {
        perp_charged: store.perp_fee_v2_consumer_counts(),
        perp_rebated: store.perp_fee_v2_rebated_consumer_counts(),
        option_charged: store.option_fee_v2_consumer_counts(),
        option_rebated: store.option_fee_v2_rebated_consumer_counts(),
        rebate_budget_by_asset: store.fees_manager_v2_rebate_budget_by_asset(),
    })
}

fn classify_perp_counts(
    raw: &BTreeMap<String, u64>,
    new: Option<&str>,
    old: Option<&str>,
) -> Value {
    let mut bucketed = base_bucket_map();
    for (consumer, count) in raw {
        let bucket = classify_perp_fee_consumer(consumer, new, old);
        let entry = bucketed.entry(bucket.to_string()).or_default();
        *entry = entry.saturating_add(*count);
    }
    bucket_map_to_json(&bucketed)
}

fn classify_option_counts(
    raw: &BTreeMap<String, u64>,
    new: Option<&str>,
    old: Option<&str>,
) -> Value {
    let mut bucketed = base_bucket_map();
    for (consumer, count) in raw {
        let bucket = classify_option_fee_consumer(consumer, new, old);
        let entry = bucketed.entry(bucket.to_string()).or_default();
        *entry = entry.saturating_add(*count);
    }
    bucket_map_to_json(&bucketed)
}

fn base_bucket_map() -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    for bucket in CONSUMER_BUCKETS {
        map.insert(bucket.to_string(), 0);
    }
    map
}

fn bucket_map_to_json(map: &BTreeMap<String, u64>) -> Value {
    let mut out = serde_json::Map::new();
    for bucket in CONSUMER_BUCKETS {
        out.insert(
            bucket.to_string(),
            json!(map.get(bucket).copied().unwrap_or(0)),
        );
    }
    Value::Object(out)
}

fn bucket_value(buckets: &Value, label: &str) -> u64 {
    buckets
        .get(label)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn non_zero_address_owned(address: &str) -> Option<String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return None;
    }
    let stripped = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if !stripped.is_empty() && stripped.bytes().all(|byte| byte == b'0') {
        return None;
    }
    Some(trimmed.to_string())
}
