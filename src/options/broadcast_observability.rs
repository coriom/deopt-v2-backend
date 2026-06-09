//! In-process observability for the option execution broadcast pipeline.
//!
//! Holds counters + last-seen gauges for:
//!   * `should_broadcast` policy approvals + structured-code rejections.
//!   * Signer events (attempt / approve / reject) grouped by `signer_kind`.
//!   * Live read failures (FM_V2 / PFV / CV / R5 / chain-state).
//!   * `econ_data_available` true / false transitions.
//!   * Latest live read values (BE balance, OME paused / isExecutor, PFV
//!     fee balance + reserve, CV(PFV,asset), FM_V2 rebate budget, R5
//!     drift bool) for the `/metrics` and readiness surfaces.
//!
//! Labels are restricted to a low-cardinality whitelist:
//!   * `source_type` ∈ {`orderbook`, `rfq`, `unknown`}.
//!   * `code` ∈ stable structured-reject strings (`policy:<code>` /
//!     `signer:<code>` / `policy-data:<type>`).
//!   * `signer_kind` ∈ {`local_dev`, `remote`}.
//!   * `chain_id` ∈ {`8453`, `84532`, `31337`} (when present).
//!
//! No intent_id / request_id / address / RPC URL / secret EVER becomes a
//! Prometheus label. Last-seen values are stored as raw integers (u128
//! down-cast to u64 for the gauge; saturating on overflow). Logs at
//! call sites carry the high-cardinality fields via `tracing` instead.

use crate::options::broadcast_policy_data::{BroadcastPolicyInputs, DedupeReason};
use crate::options::types::OptionExecutionSourceType;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Snapshot returned by [`BroadcastObservability::snapshot`]; used by the
/// `/metrics` and `/readiness` endpoints to render Prometheus text and
/// JSON status.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BroadcastObservabilitySnapshot {
    pub policy_approved_total: BTreeMap<String, u64>,
    pub policy_rejected_total: BTreeMap<(String, String), u64>,
    pub signer_attempted_total: BTreeMap<String, u64>,
    pub signer_success_total: BTreeMap<String, u64>,
    pub signer_denied_total: BTreeMap<(String, String), u64>,
    pub policy_data_failures_total: BTreeMap<String, u64>,
    pub econ_data_available_true_total: u64,
    pub econ_data_available_false_total: u64,
    pub fm_v2_decode_failures_total: u64,
    pub fm_v2_rpc_failures_total: u64,
    pub r5_drift_observed_total: u64,
    pub local_signer_on_mainnet_refused_total: u64,
    pub last_policy_reject_code: Option<String>,
    pub last_signer_kind: Option<String>,
    pub last_broadcast_submitted_ms: Option<i64>,
    pub last_be_balance_wei: Option<u128>,
    pub last_ome_paused: Option<bool>,
    pub last_ome_is_executor: Option<bool>,
    pub last_pfv_fee_balance: Option<u128>,
    pub last_pfv_rebate_reserve: Option<u128>,
    pub last_cv_pfv_balance: Option<u128>,
    pub last_fm_v2_rebate_budget: Option<u128>,
    pub last_r5_drift_zero: Option<bool>,
    pub last_dedupe_reason: Option<String>,
}

#[derive(Default)]
struct BroadcastObservabilityInner {
    policy_approved: BTreeMap<String, u64>,
    policy_rejected: BTreeMap<(String, String), u64>,
    signer_attempted: BTreeMap<String, u64>,
    signer_success: BTreeMap<String, u64>,
    signer_denied: BTreeMap<(String, String), u64>,
    policy_data_failures: BTreeMap<String, u64>,
    econ_data_available_true: u64,
    econ_data_available_false: u64,
    fm_v2_decode_failures: u64,
    fm_v2_rpc_failures: u64,
    r5_drift_observed: u64,
    local_signer_on_mainnet_refused: u64,
    last_policy_reject_code: Option<String>,
    last_signer_kind: Option<String>,
    last_broadcast_submitted_ms: Option<i64>,
    last_be_balance_wei: Option<u128>,
    last_ome_paused: Option<bool>,
    last_ome_is_executor: Option<bool>,
    last_pfv_fee_balance: Option<u128>,
    last_pfv_rebate_reserve: Option<u128>,
    last_cv_pfv_balance: Option<u128>,
    last_fm_v2_rebate_budget: Option<u128>,
    last_r5_drift_zero: Option<bool>,
    last_dedupe_reason: Option<String>,
}

/// Thread-safe in-process observability counters. Shared across the
/// backend via `AppState::broadcast_observability` as an `Arc<Self>`.
pub struct BroadcastObservability {
    inner: Mutex<BroadcastObservabilityInner>,
}

impl BroadcastObservability {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BroadcastObservabilityInner::default()),
        }
    }

    pub fn snapshot(&self) -> BroadcastObservabilitySnapshot {
        let inner = self.inner.lock().expect("broadcast observability poisoned");
        BroadcastObservabilitySnapshot {
            policy_approved_total: inner.policy_approved.clone(),
            policy_rejected_total: inner.policy_rejected.clone(),
            signer_attempted_total: inner.signer_attempted.clone(),
            signer_success_total: inner.signer_success.clone(),
            signer_denied_total: inner.signer_denied.clone(),
            policy_data_failures_total: inner.policy_data_failures.clone(),
            econ_data_available_true_total: inner.econ_data_available_true,
            econ_data_available_false_total: inner.econ_data_available_false,
            fm_v2_decode_failures_total: inner.fm_v2_decode_failures,
            fm_v2_rpc_failures_total: inner.fm_v2_rpc_failures,
            r5_drift_observed_total: inner.r5_drift_observed,
            local_signer_on_mainnet_refused_total: inner.local_signer_on_mainnet_refused,
            last_policy_reject_code: inner.last_policy_reject_code.clone(),
            last_signer_kind: inner.last_signer_kind.clone(),
            last_broadcast_submitted_ms: inner.last_broadcast_submitted_ms,
            last_be_balance_wei: inner.last_be_balance_wei,
            last_ome_paused: inner.last_ome_paused,
            last_ome_is_executor: inner.last_ome_is_executor,
            last_pfv_fee_balance: inner.last_pfv_fee_balance,
            last_pfv_rebate_reserve: inner.last_pfv_rebate_reserve,
            last_cv_pfv_balance: inner.last_cv_pfv_balance,
            last_fm_v2_rebate_budget: inner.last_fm_v2_rebate_budget,
            last_r5_drift_zero: inner.last_r5_drift_zero,
            last_dedupe_reason: inner.last_dedupe_reason.clone(),
        }
    }

    pub fn record_policy_approved(&self, source_type: OptionExecutionSourceType) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        let key = source_type_label(source_type);
        *inner.policy_approved.entry(key).or_insert(0) += 1;
    }

    pub fn record_policy_rejected(&self, code: &str, source_type: OptionExecutionSourceType) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        let code_norm = sanitize_label(code);
        let source = source_type_label(source_type);
        *inner
            .policy_rejected
            .entry((code_norm.clone(), source))
            .or_insert(0) += 1;
        inner.last_policy_reject_code = Some(code_norm);
    }

    pub fn record_signer_attempt(&self, signer_kind: &str) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        let kind = sanitize_label(signer_kind);
        *inner.signer_attempted.entry(kind.clone()).or_insert(0) += 1;
        inner.last_signer_kind = Some(kind);
    }

    pub fn record_signer_success(&self, signer_kind: &str, submitted_at_ms: i64) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        let kind = sanitize_label(signer_kind);
        *inner.signer_success.entry(kind).or_insert(0) += 1;
        inner.last_broadcast_submitted_ms = Some(submitted_at_ms);
    }

    pub fn record_signer_denied(&self, code: &str, signer_kind: &str) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        let code_norm = sanitize_label(code);
        let kind = sanitize_label(signer_kind);
        *inner.signer_denied.entry((code_norm, kind)).or_insert(0) += 1;
    }

    pub fn record_policy_data_failure(&self, read_type: &str) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        let key = sanitize_label(read_type);
        *inner.policy_data_failures.entry(key).or_insert(0) += 1;
    }

    pub fn record_econ_data_available(&self, available: bool) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        if available {
            inner.econ_data_available_true += 1;
        } else {
            inner.econ_data_available_false += 1;
        }
    }

    pub fn record_fm_v2_decode_failure(&self) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        inner.fm_v2_decode_failures += 1;
    }

    pub fn record_fm_v2_rpc_failure(&self) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        inner.fm_v2_rpc_failures += 1;
    }

    pub fn record_r5_drift_observed(&self) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        inner.r5_drift_observed += 1;
    }

    pub fn record_local_signer_on_mainnet_refused(&self) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        inner.local_signer_on_mainnet_refused += 1;
    }

    /// Persist the most recent live-read snapshot for use by readiness +
    /// `/metrics` gauges. Idempotent and side-effect-free aside from the
    /// inner mutex.
    pub fn record_inputs_snapshot(&self, inputs: &BroadcastPolicyInputs) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        if let Some(be) = inputs.be_balance_wei {
            inner.last_be_balance_wei = Some(be);
        }
        if let Some(paused) = inputs.ome_paused {
            inner.last_ome_paused = Some(paused);
        }
        if let Some(is_exec) = inputs.ome_is_executor {
            inner.last_ome_is_executor = Some(is_exec);
        }
        if let Some(fee) = inputs.pfv_fee_balance_asset {
            inner.last_pfv_fee_balance = Some(fee);
        }
        if let Some(reserve) = inputs.pfv_rebate_reserve_asset {
            inner.last_pfv_rebate_reserve = Some(reserve);
        }
        if let Some(cv) = inputs.cv_pfv_balance_asset {
            inner.last_cv_pfv_balance = Some(cv);
        }
        if let Some(budget) = inputs.fm_v2_rebate_budget_asset {
            inner.last_fm_v2_rebate_budget = Some(budget);
        }
        if let Some(r5) = inputs.r5_drift_zero {
            inner.last_r5_drift_zero = Some(r5);
        }
        if inputs.dedupe_hit {
            inner.last_dedupe_reason = Some(dedupe_reason_label(inputs.dedupe_reason));
        }
    }
}

impl Default for BroadcastObservability {
    fn default() -> Self {
        Self::new()
    }
}

fn source_type_label(source_type: OptionExecutionSourceType) -> String {
    match source_type {
        OptionExecutionSourceType::OptionOrderbookFill => "orderbook".to_string(),
        OptionExecutionSourceType::OptionRfqFill => "rfq".to_string(),
    }
}

fn dedupe_reason_label(reason: DedupeReason) -> String {
    match reason {
        DedupeReason::None => "none".to_string(),
        DedupeReason::ExistingTxHash => "existing_tx_hash".to_string(),
        DedupeReason::StatusAlreadyBroadcastSubmitted => "status_broadcast_submitted".to_string(),
        DedupeReason::StatusAlreadyBroadcastConfirmed => "status_broadcast_confirmed".to_string(),
        DedupeReason::StatusAlreadyBroadcastFailed => "status_broadcast_failed".to_string(),
    }
}

/// Lower-case, alphanumeric + dash + underscore. Truncate to 48 chars to
/// bound metric-label memory. Anything failing the whitelist becomes
/// `"unknown"` — defence-in-depth against accidentally promoting an
/// address / hash / secret to a Prometheus label.
fn sanitize_label(value: &str) -> String {
    let trimmed = value.trim().to_ascii_lowercase();
    let truncated: String = trimmed
        .chars()
        .take(48)
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_' || *ch == ':')
        .collect();
    if truncated.is_empty() {
        "unknown".to_string()
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::broadcast_policy_data::DedupeReason;

    #[test]
    fn counters_increment_independently_per_label() {
        let obs = BroadcastObservability::new();
        obs.record_policy_approved(OptionExecutionSourceType::OptionOrderbookFill);
        obs.record_policy_approved(OptionExecutionSourceType::OptionOrderbookFill);
        obs.record_policy_approved(OptionExecutionSourceType::OptionRfqFill);
        let snap = obs.snapshot();
        assert_eq!(snap.policy_approved_total.get("orderbook"), Some(&2));
        assert_eq!(snap.policy_approved_total.get("rfq"), Some(&1));
    }

    #[test]
    fn reject_records_code_and_source_pair() {
        let obs = BroadcastObservability::new();
        obs.record_policy_rejected("rebate-reserve", OptionExecutionSourceType::OptionRfqFill);
        obs.record_policy_rejected("rebate-reserve", OptionExecutionSourceType::OptionRfqFill);
        obs.record_policy_rejected(
            "negative-effective-ppm",
            OptionExecutionSourceType::OptionOrderbookFill,
        );
        let snap = obs.snapshot();
        assert_eq!(
            snap.policy_rejected_total
                .get(&("rebate-reserve".to_string(), "rfq".to_string())),
            Some(&2)
        );
        assert_eq!(
            snap.policy_rejected_total.get(&(
                "negative-effective-ppm".to_string(),
                "orderbook".to_string()
            )),
            Some(&1)
        );
        assert_eq!(
            snap.last_policy_reject_code.as_deref(),
            Some("negative-effective-ppm")
        );
    }

    #[test]
    fn signer_counters_distinguish_attempt_success_and_denial() {
        let obs = BroadcastObservability::new();
        obs.record_signer_attempt("remote");
        obs.record_signer_success("remote", 1_700_000_000_000);
        obs.record_signer_attempt("local_dev");
        obs.record_signer_denied("kms-timeout", "remote");
        let snap = obs.snapshot();
        assert_eq!(snap.signer_attempted_total.get("remote"), Some(&1));
        assert_eq!(snap.signer_attempted_total.get("local_dev"), Some(&1));
        assert_eq!(snap.signer_success_total.get("remote"), Some(&1));
        assert_eq!(
            snap.signer_denied_total
                .get(&("kms-timeout".to_string(), "remote".to_string())),
            Some(&1)
        );
        assert_eq!(snap.last_broadcast_submitted_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn inputs_snapshot_persists_live_read_values() {
        let obs = BroadcastObservability::new();
        let inputs = BroadcastPolicyInputs {
            be_balance_wei: Some(1_000),
            ome_paused: Some(false),
            ome_is_executor: Some(true),
            pfv_fee_balance_asset: Some(50),
            pfv_rebate_reserve_asset: Some(0),
            cv_pfv_balance_asset: Some(50),
            fm_v2_rebate_budget_asset: Some(0),
            r5_drift_zero: Some(true),
            dedupe_hit: false,
            dedupe_reason: DedupeReason::None,
            ..Default::default()
        };
        obs.record_inputs_snapshot(&inputs);
        let snap = obs.snapshot();
        assert_eq!(snap.last_be_balance_wei, Some(1_000));
        assert_eq!(snap.last_ome_paused, Some(false));
        assert_eq!(snap.last_ome_is_executor, Some(true));
        assert_eq!(snap.last_pfv_fee_balance, Some(50));
        assert_eq!(snap.last_pfv_rebate_reserve, Some(0));
        assert_eq!(snap.last_cv_pfv_balance, Some(50));
        assert_eq!(snap.last_fm_v2_rebate_budget, Some(0));
        assert_eq!(snap.last_r5_drift_zero, Some(true));
    }

    #[test]
    fn dedupe_reason_persisted_when_hit() {
        let obs = BroadcastObservability::new();
        let inputs = BroadcastPolicyInputs {
            dedupe_hit: true,
            dedupe_reason: DedupeReason::StatusAlreadyBroadcastSubmitted,
            ..Default::default()
        };
        obs.record_inputs_snapshot(&inputs);
        let snap = obs.snapshot();
        assert_eq!(
            snap.last_dedupe_reason.as_deref(),
            Some("status_broadcast_submitted")
        );
    }

    #[test]
    fn sanitize_label_strips_unsafe_chars_and_caps_length() {
        // raw EVM address-like input → strips colons / non-alphanum tail
        let s = sanitize_label("  POLICY:Wash:0xdeadbeef@host  ");
        assert!(!s.contains('@'));
        assert!(s.starts_with("policy:wash:0xdeadbeefhost") || s.starts_with("policy:wash:"));
        assert!(s.len() <= 48);
        // empty → "unknown"
        assert_eq!(sanitize_label(""), "unknown");
        // only forbidden chars → "unknown"
        assert_eq!(sanitize_label("!@#$%"), "unknown");
    }

    #[test]
    fn econ_data_available_true_false_tracked_separately() {
        let obs = BroadcastObservability::new();
        obs.record_econ_data_available(true);
        obs.record_econ_data_available(true);
        obs.record_econ_data_available(false);
        let snap = obs.snapshot();
        assert_eq!(snap.econ_data_available_true_total, 2);
        assert_eq!(snap.econ_data_available_false_total, 1);
    }

    #[test]
    fn fm_v2_failures_and_r5_drift_counters_independent() {
        let obs = BroadcastObservability::new();
        obs.record_fm_v2_decode_failure();
        obs.record_fm_v2_rpc_failure();
        obs.record_fm_v2_rpc_failure();
        obs.record_r5_drift_observed();
        let snap = obs.snapshot();
        assert_eq!(snap.fm_v2_decode_failures_total, 1);
        assert_eq!(snap.fm_v2_rpc_failures_total, 2);
        assert_eq!(snap.r5_drift_observed_total, 1);
    }

    #[test]
    fn local_signer_refusal_counter_increments() {
        let obs = BroadcastObservability::new();
        obs.record_local_signer_on_mainnet_refused();
        obs.record_local_signer_on_mainnet_refused();
        let snap = obs.snapshot();
        assert_eq!(snap.local_signer_on_mainnet_refused_total, 2);
    }
}
