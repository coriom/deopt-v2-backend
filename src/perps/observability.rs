//! PERPS-MONITORING-ALERTING-V1 — in-process Perps observability counters.
//!
//! Thread-safe counter store shared across the backend via
//! `AppState::perps_observability` as `Arc<Self>`. Mirrors the shape of
//! `crate::options::BroadcastObservability` so the metrics exporter can
//! render the two families side by side without a fresh abstraction.
//!
//! **What is counted:**
//!
//! * Worker tick outcomes (funding + liquidation, per outcome kind).
//! * Kill-switch skips (per surface).
//! * Public fail-closed rejects at Perps mutation handler entry.
//! * Closed-test denials (allowlist / v2 auth path).
//! * Aggregate stale-oracle + deviation-exceeded counts.
//! * Liquidation events + bad-debt events.
//! * Submit + cancel reject counts bucketed by a **bounded**
//!   reason code — no wallet, no order id, no signature, no nonce,
//!   no raw error message ever becomes a label.
//!
//! **Cardinality policy:**
//!
//! * All label values pass through `sanitize_reason_label` so anything
//!   unusual collapses to `"other"`. The upper cap on distinct reason
//!   codes is the finite set defined in `submit_reason_labels()` +
//!   `cancel_reason_labels()`; every other classified error variant
//!   funnels into `"other"`.
//!
//! **No secrets:** every counter is a `u64`. There is no field that
//! could carry a wallet address, RPC URL, DB URL, admin token,
//! signature, envelope, or nonce. Grep-checked by the new test binary.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::BackendError;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PerpsObservabilitySnapshot {
    pub funding_tick_ok_total: u64,
    pub funding_tick_failure_total: u64,
    pub funding_tick_kill_switch_skip_total: u64,
    pub funding_market_stale_skip_total: u64,

    pub liquidation_tick_ok_total: u64,
    pub liquidation_tick_failure_total: u64,
    pub liquidation_tick_kill_switch_skip_total: u64,
    pub liquidation_market_stale_skip_total: u64,

    pub perps_not_live_reject_total: u64,
    pub closed_test_access_denied_total: u64,
    pub v2_auth_failure_total: u64,

    /// Deviation-guard trip count (submit-time). V1 has `mark == index`
    /// so this stays `0` until a future per-market mark smoother
    /// diverges the two.
    pub deviation_exceeded_total: u64,

    pub liquidation_event_total: u64,
    pub bad_debt_event_total: u64,

    /// Submit rejects bucketed by classified reason. Bounded to the
    /// set returned by [`submit_reason_labels`] plus `"other"`.
    pub submit_reject_by_reason: BTreeMap<String, u64>,
    /// Cancel rejects bucketed by classified reason. Bounded to
    /// [`cancel_reason_labels`] plus `"other"`.
    pub cancel_reject_by_reason: BTreeMap<String, u64>,
}

#[derive(Default)]
struct PerpsObservabilityInner {
    funding_tick_ok_total: u64,
    funding_tick_failure_total: u64,
    funding_tick_kill_switch_skip_total: u64,
    funding_market_stale_skip_total: u64,

    liquidation_tick_ok_total: u64,
    liquidation_tick_failure_total: u64,
    liquidation_tick_kill_switch_skip_total: u64,
    liquidation_market_stale_skip_total: u64,

    perps_not_live_reject_total: u64,
    closed_test_access_denied_total: u64,
    v2_auth_failure_total: u64,

    deviation_exceeded_total: u64,

    liquidation_event_total: u64,
    bad_debt_event_total: u64,

    submit_reject_by_reason: BTreeMap<String, u64>,
    cancel_reject_by_reason: BTreeMap<String, u64>,
}

pub struct PerpsObservability {
    inner: Mutex<PerpsObservabilityInner>,
}

impl Default for PerpsObservability {
    fn default() -> Self {
        Self::new()
    }
}

impl PerpsObservability {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PerpsObservabilityInner::default()),
        }
    }

    /// Take an owned snapshot for `/metrics` rendering. Never returns a
    /// borrow of the mutex — callers may drop the lock immediately.
    pub fn snapshot(&self) -> PerpsObservabilitySnapshot {
        let inner = self.inner.lock().expect("perps observability poisoned");
        PerpsObservabilitySnapshot {
            funding_tick_ok_total: inner.funding_tick_ok_total,
            funding_tick_failure_total: inner.funding_tick_failure_total,
            funding_tick_kill_switch_skip_total: inner.funding_tick_kill_switch_skip_total,
            funding_market_stale_skip_total: inner.funding_market_stale_skip_total,

            liquidation_tick_ok_total: inner.liquidation_tick_ok_total,
            liquidation_tick_failure_total: inner.liquidation_tick_failure_total,
            liquidation_tick_kill_switch_skip_total: inner.liquidation_tick_kill_switch_skip_total,
            liquidation_market_stale_skip_total: inner.liquidation_market_stale_skip_total,

            perps_not_live_reject_total: inner.perps_not_live_reject_total,
            closed_test_access_denied_total: inner.closed_test_access_denied_total,
            v2_auth_failure_total: inner.v2_auth_failure_total,

            deviation_exceeded_total: inner.deviation_exceeded_total,

            liquidation_event_total: inner.liquidation_event_total,
            bad_debt_event_total: inner.bad_debt_event_total,

            submit_reject_by_reason: inner.submit_reject_by_reason.clone(),
            cancel_reject_by_reason: inner.cancel_reject_by_reason.clone(),
        }
    }

    // ----- worker tick counters -----

    pub fn record_funding_tick_ok(&self, market_stale_skipped: u32) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.funding_tick_ok_total = inner.funding_tick_ok_total.saturating_add(1);
        inner.funding_market_stale_skip_total = inner
            .funding_market_stale_skip_total
            .saturating_add(market_stale_skipped as u64);
    }

    pub fn record_funding_tick_failure(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.funding_tick_failure_total = inner.funding_tick_failure_total.saturating_add(1);
    }

    pub fn record_funding_tick_kill_switch_skip(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.funding_tick_kill_switch_skip_total =
            inner.funding_tick_kill_switch_skip_total.saturating_add(1);
    }

    pub fn record_liquidation_tick_ok(&self, market_stale_skipped: u32, liquidations_applied: u32) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.liquidation_tick_ok_total = inner.liquidation_tick_ok_total.saturating_add(1);
        inner.liquidation_market_stale_skip_total = inner
            .liquidation_market_stale_skip_total
            .saturating_add(market_stale_skipped as u64);
        inner.liquidation_event_total = inner
            .liquidation_event_total
            .saturating_add(liquidations_applied as u64);
    }

    pub fn record_liquidation_tick_failure(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.liquidation_tick_failure_total =
            inner.liquidation_tick_failure_total.saturating_add(1);
    }

    pub fn record_liquidation_tick_kill_switch_skip(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.liquidation_tick_kill_switch_skip_total = inner
            .liquidation_tick_kill_switch_skip_total
            .saturating_add(1);
    }

    // ----- fail-closed + closed-test rejects -----

    pub fn record_perps_not_live_reject(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.perps_not_live_reject_total = inner.perps_not_live_reject_total.saturating_add(1);
    }

    pub fn record_closed_test_access_denied(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.closed_test_access_denied_total =
            inner.closed_test_access_denied_total.saturating_add(1);
    }

    pub fn record_v2_auth_failure(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.v2_auth_failure_total = inner.v2_auth_failure_total.saturating_add(1);
    }

    // ----- deviation + risk events -----

    pub fn record_deviation_exceeded(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.deviation_exceeded_total = inner.deviation_exceeded_total.saturating_add(1);
    }

    pub fn record_bad_debt_event(&self) {
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        inner.bad_debt_event_total = inner.bad_debt_event_total.saturating_add(1);
    }

    // ----- submit + cancel reject reason buckets -----

    pub fn record_submit_reject(&self, err: &BackendError) {
        let reason = classify_submit_reason(err);
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        let entry = inner
            .submit_reject_by_reason
            .entry(reason.to_string())
            .or_insert(0);
        *entry = entry.saturating_add(1);
    }

    pub fn record_cancel_reject(&self, err: &BackendError) {
        let reason = classify_cancel_reason(err);
        let mut inner = self.inner.lock().expect("perps observability poisoned");
        let entry = inner
            .cancel_reject_by_reason
            .entry(reason.to_string())
            .or_insert(0);
        *entry = entry.saturating_add(1);
    }
}

/// Finite, alphabetized set of submit reject reason labels. Any error
/// not on this list falls through to `"other"`. Anything on the list
/// is emitted as a stable time series even at zero so a PromQL alert
/// of the shape `increase(deopt_perps_submit_rejects_total{reason="perps_not_live"}[5m]) > 0`
/// has a series from the first scrape.
pub const fn submit_reason_labels() -> &'static [&'static str] {
    &[
        "closed_test_access_denied",
        "invalid_subaccount_request",
        "other",
        "perp_mark_price_unavailable",
        "perp_oracle_deviation_exceeded",
        "perp_order_notional_cap",
        "perp_order_size_cap",
        "perp_subaccount_notional_cap",
        "perp_open_interest_cap",
        "perps_not_live",
        "v2_write_auth_failure",
    ]
}

/// Finite, alphabetized set of cancel reject reason labels.
pub const fn cancel_reason_labels() -> &'static [&'static str] {
    &[
        "closed_test_access_denied",
        "cross_subaccount",
        "other",
        "perp_order_not_found",
        "perps_not_live",
        "v2_write_auth_failure",
    ]
}

fn classify_submit_reason(err: &BackendError) -> &'static str {
    match err {
        BackendError::PerpsNotLive => "perps_not_live",
        BackendError::PerpMarkPriceUnavailable(_) => "perp_mark_price_unavailable",
        BackendError::PerpOracleDeviationExceeded(_) => "perp_oracle_deviation_exceeded",
        BackendError::PerpOrderSizeCap(_) => "perp_order_size_cap",
        BackendError::PerpOrderNotionalCap(_) => "perp_order_notional_cap",
        BackendError::PerpSubaccountNotionalCap(_) => "perp_subaccount_notional_cap",
        BackendError::PerpOpenInterestCap(_) => "perp_open_interest_cap",
        BackendError::InvalidSubaccountRequest(_) => "invalid_subaccount_request",
        BackendError::WriteAuth(_) => "v2_write_auth_failure",
        _ => "other",
    }
}

fn classify_cancel_reason(err: &BackendError) -> &'static str {
    match err {
        BackendError::PerpsNotLive => "perps_not_live",
        BackendError::PerpOrderNotFound(_) => "perp_order_not_found",
        BackendError::InvalidSubaccountRequest(msg) => {
            if msg.to_ascii_lowercase().contains("cross") {
                "cross_subaccount"
            } else {
                "other"
            }
        }
        BackendError::WriteAuth(_) => "v2_write_auth_failure",
        _ => "other",
    }
}

/// Compute the derived age (seconds) between `finished_at_ms` and
/// `now_ms`. Returns `None` when the record is missing (no tick has
/// been observed yet — `-1`-style sentinel is not emitted; the caller
/// simply omits the gauge). Saturates at `u64::MAX` for absurd inputs.
pub fn tick_age_seconds(finished_at_ms: crate::types::TimestampMs, now_ms: i64) -> u64 {
    let delta = now_ms.saturating_sub(finished_at_ms);
    if delta <= 0 {
        return 0;
    }
    (delta as u64) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_starts_zero() {
        let obs = PerpsObservability::new();
        let snap = obs.snapshot();
        assert_eq!(snap.funding_tick_ok_total, 0);
        assert_eq!(snap.liquidation_tick_ok_total, 0);
        assert_eq!(snap.perps_not_live_reject_total, 0);
        assert!(snap.submit_reject_by_reason.is_empty());
    }

    #[test]
    fn funding_tick_ok_bumps_counters() {
        let obs = PerpsObservability::new();
        obs.record_funding_tick_ok(3);
        let snap = obs.snapshot();
        assert_eq!(snap.funding_tick_ok_total, 1);
        assert_eq!(snap.funding_market_stale_skip_total, 3);
    }

    #[test]
    fn liquidation_tick_ok_bumps_events_and_stale() {
        let obs = PerpsObservability::new();
        obs.record_liquidation_tick_ok(2, 5);
        let snap = obs.snapshot();
        assert_eq!(snap.liquidation_tick_ok_total, 1);
        assert_eq!(snap.liquidation_market_stale_skip_total, 2);
        assert_eq!(snap.liquidation_event_total, 5);
    }

    #[test]
    fn submit_reject_classifies_bounded() {
        let obs = PerpsObservability::new();
        obs.record_submit_reject(&BackendError::PerpsNotLive);
        obs.record_submit_reject(&BackendError::PerpMarkPriceUnavailable("x".to_string()));
        obs.record_submit_reject(&BackendError::PerpOrderSizeCap("cap".to_string()));
        // Something not in the classified set → "other".
        obs.record_submit_reject(&BackendError::Config("random".to_string()));
        let snap = obs.snapshot();
        assert_eq!(snap.submit_reject_by_reason.get("perps_not_live"), Some(&1));
        assert_eq!(
            snap.submit_reject_by_reason
                .get("perp_mark_price_unavailable"),
            Some(&1)
        );
        assert_eq!(
            snap.submit_reject_by_reason.get("perp_order_size_cap"),
            Some(&1)
        );
        assert_eq!(snap.submit_reject_by_reason.get("other"), Some(&1));
    }

    #[test]
    fn cancel_reject_cross_subaccount_bucket() {
        let obs = PerpsObservability::new();
        obs.record_cancel_reject(&BackendError::InvalidSubaccountRequest(
            "cross-subaccount cancel refused".to_string(),
        ));
        obs.record_cancel_reject(&BackendError::PerpOrderNotFound("id-x".to_string()));
        let snap = obs.snapshot();
        assert_eq!(
            snap.cancel_reject_by_reason.get("cross_subaccount"),
            Some(&1)
        );
        assert_eq!(
            snap.cancel_reject_by_reason.get("perp_order_not_found"),
            Some(&1)
        );
    }

    #[test]
    fn tick_age_seconds_derives_from_delta() {
        // finished 10 seconds ago
        assert_eq!(tick_age_seconds(90_000, 100_000), 10);
        // finished in the future (clock skew) → 0, not a negative.
        assert_eq!(tick_age_seconds(200_000, 100_000), 0);
        // exact tie → 0
        assert_eq!(tick_age_seconds(100_000, 100_000), 0);
    }

    #[test]
    fn snapshot_no_secret_field_names() {
        // If any future field name starts with a banned substring the
        // metrics grep would leak — this test freezes the shape of
        // the snapshot to a whitelist of counter names.
        let obs = PerpsObservability::new();
        let snap = obs.snapshot();
        let debug = format!("{snap:?}");
        for banned in [
            "authorization",
            "envelope",
            "signature",
            "nonce",
            "private_key",
            "rpc_url",
            "database_url",
            "admin_token",
            "allowlist",
        ] {
            assert!(!debug.contains(banned), "leaked field: {banned}");
        }
    }
}
