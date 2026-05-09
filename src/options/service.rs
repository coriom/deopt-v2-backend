use super::series_id::{option_series_id, OptionSeriesIdInput};
use super::{
    OptionOrderbookSnapshot, OptionSeries, OptionSeriesFilter, OptionSeriesId, OptionSeriesSource,
    OptionSeriesStatus,
};
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::types::{now_ms, Price1e8, Size1e8, TimestampMs};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOptionSeriesInput {
    pub underlying: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: Price1e8,
    pub is_call: bool,
    pub contract_size_1e8: Option<Size1e8>,
    pub onchain_product_id: Option<String>,
    pub onchain_series_id: Option<String>,
}

pub async fn create_option_series(
    state: &AppState,
    input: CreateOptionSeriesInput,
) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    if !state.options_config.allow_manual_series {
        return Err(BackendError::InvalidOptionSeriesState(
            "manual option series creation is disabled".to_string(),
        ));
    }

    let now = now_ms();
    let now_sec = now_sec(now)?;
    let contract_size_1e8 = input
        .contract_size_1e8
        .unwrap_or(state.options_config.default_contract_size_1e8);
    validate_assets(&[
        ("underlying", &input.underlying),
        ("base_asset", &input.base_asset),
        ("quote_asset", &input.quote_asset),
        ("settlement_asset", &input.settlement_asset),
    ])?;
    if input.expiry <= now_sec {
        return Err(BackendError::InvalidOptionSeriesState(
            "option series expiry must be in the future".to_string(),
        ));
    }
    if input.strike_1e8 == 0 {
        return Err(BackendError::InvalidFixedPoint {
            field: "strike_1e8".to_string(),
            reason: "must be greater than zero".to_string(),
        });
    }
    if contract_size_1e8 == 0 {
        return Err(BackendError::InvalidFixedPoint {
            field: "contract_size_1e8".to_string(),
            reason: "must be greater than zero".to_string(),
        });
    }

    let underlying = trim_asset(input.underlying);
    let base_asset = trim_asset(input.base_asset);
    let quote_asset = trim_asset(input.quote_asset);
    let settlement_asset = trim_asset(input.settlement_asset);
    let option_series_id = option_series_id(OptionSeriesIdInput {
        underlying: &underlying,
        base_asset: &base_asset,
        quote_asset: &quote_asset,
        settlement_asset: &settlement_asset,
        expiry: input.expiry,
        strike_1e8: input.strike_1e8,
        is_call: input.is_call,
        contract_size_1e8,
    });

    if let Some(existing) = get_option_series_optional(state, &option_series_id).await? {
        return Ok(existing);
    }

    let series = OptionSeries {
        option_series_id,
        underlying,
        base_asset,
        quote_asset,
        settlement_asset,
        expiry: input.expiry,
        strike_1e8: input.strike_1e8,
        is_call: input.is_call,
        contract_size_1e8,
        status: OptionSeriesStatus::Active,
        source: OptionSeriesSource::Manual,
        onchain_product_id: input.onchain_product_id,
        onchain_series_id: input.onchain_series_id,
        created_at_ms: now,
        updated_at_ms: now,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_option_series(&series).await?;
        Ok(repository
            .get_option_series(&series.option_series_id)
            .await?
            .unwrap_or(series))
    } else {
        Ok(state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .insert_series(series))
    }
}

pub async fn list_option_series(
    state: &AppState,
    filter: OptionSeriesFilter,
) -> Result<Vec<OptionSeries>> {
    ensure_enabled(state)?;
    let now_sec = now_sec(now_ms())?;
    if let Some(repository) = state.repository.clone() {
        return Ok(repository
            .list_option_series()
            .await?
            .into_iter()
            .filter(|series| filter.matches(series, now_sec))
            .collect());
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_series(&filter, now_sec))
}

pub async fn get_option_series(state: &AppState, option_series_id: &str) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    get_option_series_optional(state, option_series_id)
        .await?
        .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))
}

pub async fn disable_option_series(
    state: &AppState,
    option_series_id: &str,
) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    let now = now_ms();
    if let Some(repository) = state.repository.clone() {
        return repository
            .disable_option_series(option_series_id, now)
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .disable_series(option_series_id, now)
}

pub async fn get_option_orderbook(
    state: &AppState,
    option_series_id: OptionSeriesId,
) -> Result<OptionOrderbookSnapshot> {
    let series = get_option_series(state, &option_series_id).await?;
    Ok(OptionOrderbookSnapshot {
        option_series_id,
        status: series.effective_status(now_sec(now_ms())?),
        bids: Vec::new(),
        asks: Vec::new(),
    })
}

async fn get_option_series_optional(
    state: &AppState,
    option_series_id: &str,
) -> Result<Option<OptionSeries>> {
    if let Some(repository) = state.repository.clone() {
        return repository.get_option_series(option_series_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_series(option_series_id))
}

fn ensure_enabled(state: &AppState) -> Result<()> {
    if state.options_config.enabled {
        Ok(())
    } else {
        Err(BackendError::OptionsDisabled)
    }
}

fn validate_assets(fields: &[(&str, &str)]) -> Result<()> {
    for (field, value) in fields {
        if value.trim().is_empty() {
            return Err(BackendError::InvalidOptionSeriesState(format!(
                "{field} must be non-empty"
            )));
        }
    }
    Ok(())
}

fn trim_asset(value: String) -> String {
    value.trim().to_string()
}

fn now_sec(now_ms: TimestampMs) -> Result<u64> {
    u64::try_from(now_ms / 1000)
        .map_err(|_| BackendError::Config("current timestamp cannot be encoded".to_string()))
}
