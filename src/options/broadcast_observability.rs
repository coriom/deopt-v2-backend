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

/// Stable code recorded by [`BroadcastObservability::record_local_signer_on_mainnet_refused`]
/// as the `last_signer_error_code` singleton. Not part of the
/// `SignerError::code()` taxonomy because that branch never reaches a
/// signer — the runtime guard fires before any sign attempt.
pub const LOCAL_MAINNET_REFUSED_CODE: &str = "local_mainnet_refused";

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
    /// Source type of the most recent `should_broadcast` rejection — one
    /// of the bounded `source_type_label` values (`orderbook` | `rfq`).
    /// Mirrors the cumulative `policy_rejected_total{code, source_type}`
    /// counter as a "what just happened" singleton for the JSON health
    /// endpoint. None until the first rejection.
    pub last_reject_source_type: Option<String>,
    /// Bounded singleton of the most recent signer error code. Values
    /// come from one of: (a) `SignerError::code()` (the §4.2 taxonomy:
    /// `chain-not-allowed` / `kms-timeout` / `transport` / …) when the
    /// `RemoteSigner::sign_option_execution_tx` future returns Err; (b)
    /// the literal `"local_mainnet_refused"` when defence-in-depth
    /// refuses a `LocalDev` signer on mainnet. Never contains endpoint
    /// URL, credentials, or free-form provider error text. None until
    /// the first signer error.
    pub last_signer_error_code: Option<String>,
    /// Bounded singleton of the most recent live-provider read failure
    /// `read_type` — exactly the same value populated into the
    /// `policy_data_failures_total{read_type}` counter, which comes
    /// from the hardcoded `crate::options::broadcast_policy_data::read_type`
    /// constants taxonomy (`be_balance` / `ome_paused` /
    /// `pfv_rebate_reserve` / `fm_v2_quote_fees_rpc` / …). Never
    /// carries a raw error message, RPC URL, calldata, provider
    /// endpoint, intent_id, or contract address. None until the first
    /// failure.
    pub last_policy_data_failure_type: Option<String>,
    pub last_signer_kind: Option<String>,
    pub last_broadcast_submitted_ms: Option<i64>,
    /// Most recent `econ_data_available` decision for the policy
    /// context. `Some(true)` means `fee_split`, `fm_v2_rebate_budget`,
    /// and `pfv_rebate_reserve` were all observed for the broadcast
    /// attempt; `Some(false)` means boundary mode was used. `None`
    /// until the first attempt.
    pub econ_data_available_last: Option<bool>,
    pub last_be_balance_wei: Option<u128>,
    pub last_ome_paused: Option<bool>,
    pub last_ome_is_executor: Option<bool>,
    pub last_pfv_fee_balance: Option<u128>,
    pub last_pfv_rebate_reserve: Option<u128>,
    pub last_cv_pfv_balance: Option<u128>,
    pub last_fm_v2_rebate_budget: Option<u128>,
    pub last_r5_drift_zero: Option<bool>,
    pub last_dedupe_reason: Option<String>,
    /// Most recent computed effective maker fee ppm produced by the
    /// live FeesManagerV2 / `aggregate_fee_split` path. Signed i64
    /// because the policy gate already permits negative ppm under RFQ
    /// rebate-discount profiles (and rejects them on mainnet via the
    /// `negative-effective-ppm` reject code). `None` when the broadcast
    /// attempt's `fee_split` was missing (no fake zeros are recorded).
    pub last_effective_maker_ppm: Option<i64>,
    /// Most recent computed effective taker fee ppm. Same semantics as
    /// [`Self::last_effective_maker_ppm`].
    pub last_effective_taker_ppm: Option<i64>,
    /// Most recent BE-balance floor in wei used as the §6 broadcast-policy
    /// fund-floor input. Computed inside `run_should_broadcast_policy`
    /// as `EXECUTOR_MAX_FEE_PER_GAS_WEI × EXECUTOR_MAX_GAS_LIMIT` when
    /// the chain mode is not permissive, otherwise `0` (Sepolia
    /// rehearsal). Surfaced via `/executor/health/v2` so operators can
    /// confirm the floor the policy gate is checking against. `None`
    /// until the first broadcast attempt.
    pub last_be_balance_floor_wei: Option<u128>,
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
    last_reject_source_type: Option<String>,
    last_signer_error_code: Option<String>,
    last_policy_data_failure_type: Option<String>,
    last_signer_kind: Option<String>,
    last_broadcast_submitted_ms: Option<i64>,
    econ_data_available_last: Option<bool>,
    last_be_balance_wei: Option<u128>,
    last_ome_paused: Option<bool>,
    last_ome_is_executor: Option<bool>,
    last_pfv_fee_balance: Option<u128>,
    last_pfv_rebate_reserve: Option<u128>,
    last_cv_pfv_balance: Option<u128>,
    last_fm_v2_rebate_budget: Option<u128>,
    last_r5_drift_zero: Option<bool>,
    last_dedupe_reason: Option<String>,
    last_effective_maker_ppm: Option<i64>,
    last_effective_taker_ppm: Option<i64>,
    last_be_balance_floor_wei: Option<u128>,
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
            last_reject_source_type: inner.last_reject_source_type.clone(),
            last_signer_error_code: inner.last_signer_error_code.clone(),
            last_policy_data_failure_type: inner.last_policy_data_failure_type.clone(),
            last_signer_kind: inner.last_signer_kind.clone(),
            last_broadcast_submitted_ms: inner.last_broadcast_submitted_ms,
            econ_data_available_last: inner.econ_data_available_last,
            last_be_balance_wei: inner.last_be_balance_wei,
            last_ome_paused: inner.last_ome_paused,
            last_ome_is_executor: inner.last_ome_is_executor,
            last_pfv_fee_balance: inner.last_pfv_fee_balance,
            last_pfv_rebate_reserve: inner.last_pfv_rebate_reserve,
            last_cv_pfv_balance: inner.last_cv_pfv_balance,
            last_fm_v2_rebate_budget: inner.last_fm_v2_rebate_budget,
            last_r5_drift_zero: inner.last_r5_drift_zero,
            last_dedupe_reason: inner.last_dedupe_reason.clone(),
            last_effective_maker_ppm: inner.last_effective_maker_ppm,
            last_effective_taker_ppm: inner.last_effective_taker_ppm,
            last_be_balance_floor_wei: inner.last_be_balance_floor_wei,
        }
    }

    /// Persist the most recent BE-balance floor (wei) used as the §6
    /// broadcast-policy fund-floor input. Recorder fires for every
    /// `should_broadcast` invocation — orderbook + RFQ paths alike —
    /// because the floor is computed inside `run_should_broadcast_policy`
    /// from `state.execution_config` and used as
    /// [`crate::options::broadcast_policy::BroadcastContext::fund_floor_wei`].
    /// Caller MUST pass the exact value the policy gate consumes;
    /// numeric only, never a free-form string.
    pub fn record_be_balance_floor_wei(&self, value: u128) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        inner.last_be_balance_floor_wei = Some(value);
    }

    /// Persist the most recent computed effective maker + taker fee ppm
    /// values for surfacing by the JSON health endpoint. Pure singleton
    /// — does NOT bump any cumulative counter and does NOT increment
    /// `econ_data_available_true_total` (that is already handled by
    /// [`Self::record_econ_data_available`]). Caller MUST only invoke
    /// this when the live `fee_split` was observed; the broadcast site
    /// guards the call via `if let Some(fee_split) = inputs.fee_split…`
    /// so a missing `fee_split` never produces a fake `(0, 0)` reading.
    pub fn record_effective_fee_ppm(&self, maker_ppm: i64, taker_ppm: i64) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        inner.last_effective_maker_ppm = Some(maker_ppm);
        inner.last_effective_taker_ppm = Some(taker_ppm);
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
            .entry((code_norm.clone(), source.clone()))
            .or_insert(0) += 1;
        inner.last_policy_reject_code = Some(code_norm);
        inner.last_reject_source_type = Some(source);
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
        *inner
            .signer_denied
            .entry((code_norm.clone(), kind))
            .or_insert(0) += 1;
        inner.last_signer_error_code = Some(code_norm);
    }

    pub fn record_policy_data_failure(&self, read_type: &str) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        let key = sanitize_label(read_type);
        *inner.policy_data_failures.entry(key.clone()).or_insert(0) += 1;
        inner.last_policy_data_failure_type = Some(key);
    }

    pub fn record_econ_data_available(&self, available: bool) {
        let mut inner = self.inner.lock().expect("broadcast observability poisoned");
        if available {
            inner.econ_data_available_true += 1;
        } else {
            inner.econ_data_available_false += 1;
        }
        inner.econ_data_available_last = Some(available);
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
        // Surface the refusal via the `last_signer_error_code` singleton
        // so the JSON health endpoint reflects the most-recent signer
        // event even though this branch never calls `record_signer_denied`
        // (it never contacted a signer — the runtime guard fired first).
        inner.last_signer_error_code = Some(LOCAL_MAINNET_REFUSED_CODE.to_string());
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

    #[test]
    fn reject_stores_last_reject_source_type_singleton() {
        let obs = BroadcastObservability::new();
        assert_eq!(obs.snapshot().last_reject_source_type, None);
        obs.record_policy_rejected("rebate-reserve", OptionExecutionSourceType::OptionRfqFill);
        assert_eq!(
            obs.snapshot().last_reject_source_type.as_deref(),
            Some("rfq")
        );
        // most-recent overrides earlier
        obs.record_policy_rejected(
            "negative-effective-ppm",
            OptionExecutionSourceType::OptionOrderbookFill,
        );
        let snap = obs.snapshot();
        assert_eq!(snap.last_reject_source_type.as_deref(), Some("orderbook"));
        assert_eq!(
            snap.last_policy_reject_code.as_deref(),
            Some("negative-effective-ppm")
        );
    }

    #[test]
    fn signer_denied_stores_last_signer_error_code_singleton() {
        let obs = BroadcastObservability::new();
        assert_eq!(obs.snapshot().last_signer_error_code, None);
        obs.record_signer_denied("kms-timeout", "remote");
        assert_eq!(
            obs.snapshot().last_signer_error_code.as_deref(),
            Some("kms-timeout")
        );
        // most-recent overrides earlier
        obs.record_signer_denied("transport", "remote");
        assert_eq!(
            obs.snapshot().last_signer_error_code.as_deref(),
            Some("transport")
        );
    }

    #[test]
    fn local_mainnet_refusal_sets_last_signer_error_code() {
        let obs = BroadcastObservability::new();
        obs.record_local_signer_on_mainnet_refused();
        assert_eq!(
            obs.snapshot().last_signer_error_code.as_deref(),
            Some(LOCAL_MAINNET_REFUSED_CODE)
        );
        assert_eq!(obs.snapshot().local_signer_on_mainnet_refused_total, 1);
    }

    #[test]
    fn signer_error_code_remains_bounded_under_arbitrary_input() {
        let obs = BroadcastObservability::new();
        // a maliciously-shaped code carrying an endpoint-like URL is
        // sanitised by the existing `sanitize_label` (strips `@` and
        // most non-alnum, caps to 48 chars). Pins the redaction
        // contract for the singleton.
        obs.record_signer_denied("https://signer.invalid/secret?token=abcdef", "remote");
        let code = obs
            .snapshot()
            .last_signer_error_code
            .expect("singleton populated");
        assert!(!code.contains('@'));
        assert!(!code.contains('?'));
        assert!(code.len() <= 48);
    }

    #[test]
    fn policy_data_failure_stores_last_failure_type_singleton() {
        let obs = BroadcastObservability::new();
        assert_eq!(obs.snapshot().last_policy_data_failure_type, None);
        obs.record_policy_data_failure(
            crate::options::broadcast_policy_data::read_type::FM_V2_QUOTE_FEES_RPC,
        );
        assert_eq!(
            obs.snapshot().last_policy_data_failure_type.as_deref(),
            Some("fm_v2_quote_fees_rpc")
        );
        // counter still increments alongside the singleton
        assert_eq!(
            obs.snapshot()
                .policy_data_failures_total
                .get("fm_v2_quote_fees_rpc"),
            Some(&1)
        );
    }

    #[test]
    fn policy_data_failure_singleton_overwrites_with_most_recent() {
        let obs = BroadcastObservability::new();
        obs.record_policy_data_failure(
            crate::options::broadcast_policy_data::read_type::PFV_REBATE_RESERVE,
        );
        obs.record_policy_data_failure(
            crate::options::broadcast_policy_data::read_type::OME_PAUSED,
        );
        obs.record_policy_data_failure(
            crate::options::broadcast_policy_data::read_type::FM_V2_QUOTE_FEES_DECODE,
        );
        let snap = obs.snapshot();
        assert_eq!(
            snap.last_policy_data_failure_type.as_deref(),
            Some("fm_v2_quote_fees_decode")
        );
        // each cumulative counter still records its own bucket
        assert_eq!(
            snap.policy_data_failures_total.get("pfv_rebate_reserve"),
            Some(&1)
        );
        assert_eq!(snap.policy_data_failures_total.get("ome_paused"), Some(&1));
        assert_eq!(
            snap.policy_data_failures_total
                .get("fm_v2_quote_fees_decode"),
            Some(&1)
        );
    }

    #[test]
    fn policy_data_failure_singleton_remains_bounded_under_arbitrary_input() {
        // Defence-in-depth: even if a caller passed a URL-shaped or
        // address-shaped string into record_policy_data_failure (which
        // they should not — the production code uses only the
        // hardcoded `read_type::*` constants), `sanitize_label` strips
        // URL punctuation + caps to 48 chars so the singleton can never
        // surface a routable endpoint or 0x-address.
        let obs = BroadcastObservability::new();
        obs.record_policy_data_failure("https://rpc.example/sensitive-provider-key?token=abc");
        let value = obs
            .snapshot()
            .last_policy_data_failure_type
            .expect("singleton populated");
        assert!(!value.contains("://"));
        assert!(!value.contains('/'));
        assert!(!value.contains('?'));
        assert!(!value.contains('='));
        assert!(!value.contains('.'));
        assert!(value.len() <= 48);
    }

    #[test]
    fn be_balance_floor_wei_singleton_stores_value() {
        let obs = BroadcastObservability::new();
        assert_eq!(obs.snapshot().last_be_balance_floor_wei, None);
        obs.record_be_balance_floor_wei(1_000_000_000);
        assert_eq!(
            obs.snapshot().last_be_balance_floor_wei,
            Some(1_000_000_000)
        );
    }

    #[test]
    fn be_balance_floor_wei_singleton_overwrites_with_most_recent() {
        let obs = BroadcastObservability::new();
        obs.record_be_balance_floor_wei(1_000);
        obs.record_be_balance_floor_wei(u128::MAX);
        obs.record_be_balance_floor_wei(0);
        // 0 IS a legitimately-recorded Sepolia permissive value; the
        // recorder accepts any u128 the policy gate passed in.
        assert_eq!(obs.snapshot().last_be_balance_floor_wei, Some(0));
    }

    #[test]
    fn be_balance_floor_wei_singleton_does_not_collide_with_be_balance_wei() {
        // Defence-in-depth pin: the floor and the most-recent observed
        // BE balance share a "wei" unit but MUST remain independent
        // singletons (the floor is policy-config-derived; the
        // most-recent balance is chain-state-derived).
        let obs = BroadcastObservability::new();
        obs.record_be_balance_floor_wei(500);
        let inputs = BroadcastPolicyInputs {
            be_balance_wei: Some(1_500),
            ..Default::default()
        };
        obs.record_inputs_snapshot(&inputs);
        let snap = obs.snapshot();
        assert_eq!(snap.last_be_balance_floor_wei, Some(500));
        assert_eq!(snap.last_be_balance_wei, Some(1_500));
    }

    #[test]
    fn effective_fee_ppm_singleton_stores_both_sides() {
        let obs = BroadcastObservability::new();
        assert_eq!(obs.snapshot().last_effective_maker_ppm, None);
        assert_eq!(obs.snapshot().last_effective_taker_ppm, None);
        obs.record_effective_fee_ppm(50, 100);
        let snap = obs.snapshot();
        assert_eq!(snap.last_effective_maker_ppm, Some(50));
        assert_eq!(snap.last_effective_taker_ppm, Some(100));
    }

    #[test]
    fn effective_fee_ppm_singleton_overwrites_with_most_recent() {
        let obs = BroadcastObservability::new();
        obs.record_effective_fee_ppm(50, 100);
        obs.record_effective_fee_ppm(75, 125);
        obs.record_effective_fee_ppm(-25, 30);
        let snap = obs.snapshot();
        assert_eq!(snap.last_effective_maker_ppm, Some(-25));
        assert_eq!(snap.last_effective_taker_ppm, Some(30));
    }

    #[test]
    fn effective_fee_ppm_singleton_independent_of_econ_data_available_counter() {
        // Pin: record_effective_fee_ppm does NOT bump
        // `econ_data_available_*_total` — those counters are owned by
        // `record_econ_data_available`. Otherwise a future regression
        // could double-count broadcasts.
        let obs = BroadcastObservability::new();
        obs.record_effective_fee_ppm(50, 100);
        let snap = obs.snapshot();
        assert_eq!(snap.econ_data_available_true_total, 0);
        assert_eq!(snap.econ_data_available_false_total, 0);
        assert_eq!(snap.econ_data_available_last, None);
    }

    #[test]
    fn econ_data_available_last_reflects_most_recent_decision() {
        let obs = BroadcastObservability::new();
        assert_eq!(obs.snapshot().econ_data_available_last, None);
        obs.record_econ_data_available(true);
        assert_eq!(obs.snapshot().econ_data_available_last, Some(true));
        obs.record_econ_data_available(false);
        let snap = obs.snapshot();
        assert_eq!(snap.econ_data_available_last, Some(false));
        // cumulative counters still increment alongside the singleton
        assert_eq!(snap.econ_data_available_true_total, 1);
        assert_eq!(snap.econ_data_available_false_total, 1);
    }
}
