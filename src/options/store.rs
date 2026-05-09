use super::{OptionSeries, OptionSeriesFilter, OptionSeriesId, OptionSeriesStatus};
use crate::error::{BackendError, Result};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct OptionSeriesStore {
    series: HashMap<OptionSeriesId, OptionSeries>,
}

impl OptionSeriesStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_series(&mut self, series: OptionSeries) -> OptionSeries {
        if let Some(existing) = self.series.get(&series.option_series_id) {
            return existing.clone();
        }
        self.series
            .insert(series.option_series_id.clone(), series.clone());
        series
    }

    pub fn list_series(&self, filter: &OptionSeriesFilter, now_sec: u64) -> Vec<OptionSeries> {
        let mut series = self
            .series
            .values()
            .filter(|series| filter.matches(series, now_sec))
            .cloned()
            .collect::<Vec<_>>();
        series.sort_by(|left, right| {
            left.expiry
                .cmp(&right.expiry)
                .then_with(|| left.strike_1e8.cmp(&right.strike_1e8))
                .then_with(|| left.option_series_id.cmp(&right.option_series_id))
        });
        series
    }

    pub fn get_series(&self, option_series_id: &str) -> Option<OptionSeries> {
        self.series.get(option_series_id).cloned()
    }

    pub fn disable_series(
        &mut self,
        option_series_id: &str,
        updated_at_ms: i64,
    ) -> Result<OptionSeries> {
        let series = self
            .series
            .get_mut(option_series_id)
            .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))?;
        series.status = OptionSeriesStatus::Disabled;
        series.updated_at_ms = updated_at_ms;
        Ok(series.clone())
    }
}
