# BACKEND-EXECUTOR-MONITORING-ALERTS-V1-WIRING — result

**Status:** SHIPPED 2026-06-09 (Phase I close-out).
**Scope:** wire in-process observability for the option execution
broadcast pipeline — Prometheus counters + last-seen gauges for
`should_broadcast` policy decisions, signer events, chain-state +
FM_V2 reads, `econ_data_available` transitions, R5 drift detection,
and mainnet runtime defence-in-depth refusals. **No mainnet tx. No live
broadcast. No `.env` edit. No webhook secrets.**

---

## 1. Files changed

### New (1)

- `deopt-v2-backend/src/options/broadcast_observability.rs` — ~480 LoC
  incl. 9 unit tests. Defines `BroadcastObservability` (thread-safe
  in-process counter + gauge state) + `BroadcastObservabilitySnapshot`
  (DTO for renderers).
- `deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1_WIRING_RESULT.md`
  — this close-out doc.

### Modified (4)

- `src/options/mod.rs` — `pub mod broadcast_observability;` +
  re-export of `BroadcastObservability` and `BroadcastObservabilitySnapshot`.
- `src/api/http.rs` — `AppState::broadcast_observability: Arc<BroadcastObservability>`
  field + default initializer in every `AppState` constructor.
- `src/options/service.rs` — counters wired at each transition: policy
  data provider failure (mainnet + testnet), policy approve, policy
  reject (all `RejectReason::*` codes incl. `dupe`), signer attempt,
  signer success, signer denial (`SignerError::code()`), live-read
  snapshot persistence (`record_inputs_snapshot`), `econ_data_available`
  true/false (3-way conjunction), R5 drift observed, runtime mainnet
  refusal of `LocalDev` signer. **6 new integration tests** verifying
  each transition increments the expected counter AND that signer
  remains uncontacted whenever policy rejects.
- `src/monitoring.rs` — new `append_broadcast_observability_metrics`
  consumes `state.broadcast_observability.snapshot()` and writes the
  18 new Prometheus metrics. Two new `MetricsText` helpers
  (`append_labeled_gauge_2` for single-label gauges,
  `append_labeled_pair_gauge` for two-label gauges) keep the label
  cardinality bounded by `safe_label_value` sanitisation.

## 2. Metrics added (18 new gauges, exposed via `/metrics`)

### 2.1 Policy decision counters (low-cardinality labels)

| Metric name                                                | Labels                       | Description                                                                 |
| ---------------------------------------------------------- | ---------------------------- | --------------------------------------------------------------------------- |
| `deopt_option_broadcast_policy_approved_total`             | `source_type`                | Cumulative approvals by source (`orderbook` / `rfq`).                       |
| `deopt_option_broadcast_policy_rejected_total`             | `code`, `source_type`        | Cumulative rejections by structured reject code + source.                   |
| `deopt_option_broadcast_econ_data_available_true_total`    | (none)                       | Attempts where `econ_data_available = true` (3-way conjunction satisfied).  |
| `deopt_option_broadcast_econ_data_available_false_total`   | (none)                       | Attempts where `econ_data_available = false` (boundary mode).               |
| `deopt_option_broadcast_policy_data_failures_total`        | `read_type`                  | Data-provider read failures by tag (sanitised to ≤ 48 chars).               |
| `deopt_option_broadcast_fm_v2_decode_failures_total`       | (none)                       | FM_V2 ABI decode failures.                                                  |
| `deopt_option_broadcast_fm_v2_rpc_failures_total`          | (none)                       | FM_V2 eth_call RPC failures.                                                |
| `deopt_option_broadcast_r5_drift_observed_total`           | (none)                       | R5 drift detected during a gather_inputs call.                              |

### 2.2 Signer counters

| Metric name                                                  | Labels                         | Description                                                              |
| ------------------------------------------------------------ | ------------------------------ | ------------------------------------------------------------------------ |
| `deopt_option_broadcast_signer_attempted_total`              | `signer_kind`                  | Cumulative signer-attempt count by `local_dev` / `remote`.                |
| `deopt_option_broadcast_signer_success_total`                | `signer_kind`                  | Cumulative signer-success count.                                          |
| `deopt_option_broadcast_signer_denied_total`                 | `code`, `signer_kind`          | Cumulative signer-denial count by `SignerError::code()` + `signer_kind`.  |
| `deopt_option_broadcast_local_signer_on_mainnet_refused_total` | (none)                       | Cumulative runtime refusals of `LocalDev` signer on mainnet (defence-in-depth from `build_signer_for_state`). |

### 2.3 Last-seen live-read gauges (no labels)

| Metric name                                            | Source                                | Note                                                |
| ------------------------------------------------------ | ------------------------------------- | --------------------------------------------------- |
| `deopt_option_broadcast_last_be_balance_wei`           | `eth_getBalance(BACKEND_EXECUTOR)`    | u128 → u64 saturating.                              |
| `deopt_option_broadcast_last_ome_paused`               | `OME.paused()`                        | 1 = paused, 0 = not paused.                         |
| `deopt_option_broadcast_last_ome_is_executor`          | `OME.isExecutor(BACKEND_EXECUTOR)`    | 1 = is executor, 0 = not.                           |
| `deopt_option_broadcast_last_pfv_fee_balance`          | `PFV.feeBalance(asset)`               | u128 → u64 saturating.                              |
| `deopt_option_broadcast_last_pfv_rebate_reserve`       | `PFV.rebateReserve(asset)`            | u128 → u64 saturating (Cluster 4 launch invariant). |
| `deopt_option_broadcast_last_cv_pfv_balance`           | `CV.balances(PFV, asset)`             | u128 → u64 saturating.                              |
| `deopt_option_broadcast_last_fm_v2_rebate_budget`      | `FeesManagerV2.rebateBudget(asset)`   | u128 → u64 saturating.                              |
| `deopt_option_broadcast_last_r5_drift_zero`            | derived                               | 1 = drift zero, 0 = drift observed.                 |
| `deopt_option_broadcast_last_submitted_ms`             | `now_ms()` at signer success          | UTC ms of the most recent successful broadcast.     |

### 2.4 Label cardinality controls

- `source_type ∈ {orderbook, rfq}` (low cardinality).
- `signer_kind ∈ {local_dev, remote}` (low cardinality).
- `code` is restricted via `sanitize_label` — lowercased, alphanumeric +
  `-` / `_` / `:` only, truncated to 48 chars; anything failing the
  whitelist becomes `unknown`. Defence-in-depth against accidentally
  promoting an address / hash / secret to a Prometheus label.
- No `intent_id`, `request_id`, address, RPC URL, or admin token ever
  becomes a Prometheus label — those high-cardinality fields stay in
  the `tracing` log records at the call sites.

## 3. Logs implemented (already present; observability adds metric counters that pair with each log line)

| Existing tracing line (call site)                       | Counter that increments                                            |
| ------------------------------------------------------- | ------------------------------------------------------------------ |
| `target="broadcast_policy" "policy data provider failure (mainnet fail-closed)"` | `policy_data_failures_total{read_type}` + `policy_rejected_total{policy-internal, source_type}` |
| `target="broadcast_policy" "policy data provider failure on testnet — using permissive fallback"` | `policy_data_failures_total{read_type}`                            |
| `target="broadcast_policy" "should_broadcast rejected option execution intent"` | `policy_rejected_total{code, source_type}` + `last_policy_reject_code` snapshot field |
| `target="broadcast_signer" "remote signer rejected option execution transaction"` | `signer_denied_total{code, signer_kind}`                           |
| `target="broadcast_signer" "signer approved option execution intent"` (success) | `signer_success_total{signer_kind}` + `last_broadcast_submitted_ms` |

Redaction is preserved: no log line carries private keys / RPC URLs /
admin tokens / DATABASE_URL / mTLS bytes / raw signed tx bytes. The
existing `signer_address.0` log field is the AccountId hex string only.

## 4. Alert mappings (per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`)

The implementation maps to the V1 alerts spec as follows. Operators
build PromQL rules + PagerDuty / Discord routes; secrets / webhooks
remain operator-side (none committed by this PR per hard-stop).

| V1 spec section                          | Metric to PromQL on                                                                                                              | Severity                 |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| §3.1 Signer integrity                    | `deopt_option_broadcast_local_signer_on_mainnet_refused_total` non-zero                                                          | **critical / PagerDuty** |
| §3.1 Signer integrity                    | `rate(deopt_option_broadcast_signer_denied_total[5m])` spike (especially `code="post-sign-from-mismatch"`)                       | **critical**             |
| §3.2 Engine state                        | `deopt_option_broadcast_last_ome_paused == 1`                                                                                    | **critical**             |
| §3.2 Engine state                        | `deopt_option_broadcast_last_ome_is_executor == 0`                                                                               | **critical**             |
| §3.3 Broadcast pipeline                  | `rate(deopt_option_broadcast_policy_rejected_total{code="ome-paused"}[5m])` spike                                                | warning                  |
| §3.3 Broadcast pipeline                  | `rate(deopt_option_broadcast_policy_rejected_total{code="be-low-bal"}[5m])` spike                                                | warning                  |
| §3.3 Broadcast pipeline                  | `rate(deopt_option_broadcast_signer_denied_total{code="kms-timeout"}[5m])` spike                                                 | **critical**             |
| §3.4 Economics                           | `rate(deopt_option_broadcast_policy_rejected_total{code="rebate-reserve"}[5m])` spike (Cluster 4 launch invariant primary teeth) | **critical**             |
| §3.4 Economics                           | `rate(deopt_option_broadcast_policy_rejected_total{code="rebate-budget"}[5m])` spike                                             | warning                  |
| §3.4 Economics                           | `rate(deopt_option_broadcast_policy_rejected_total{code="negative-effective-ppm"}[5m])` non-zero on mainnet                      | **critical**             |
| §3.4 Economics                           | `rate(deopt_option_broadcast_policy_rejected_total{code="no-econ-content"}[5m])` non-zero                                        | warning                  |
| §3.5 Accounting                          | `deopt_option_broadcast_r5_drift_observed_total > 0`                                                                             | **critical / PagerDuty** |
| §3.5 Accounting                          | `deopt_option_broadcast_last_r5_drift_zero == 0`                                                                                 | **critical**             |
| §3.5 Accounting                          | `deopt_option_broadcast_last_pfv_rebate_reserve > 0` on mainnet (Cluster 4 launch invariant)                                     | **critical**             |
| §3.5 Accounting                          | `deopt_option_broadcast_last_be_balance_wei < BE_FLOOR`                                                                          | warning → critical scale |
| §4 Warning band                          | `rate(deopt_option_broadcast_fm_v2_decode_failures_total[5m]) > 0`                                                               | warning                  |
| §4 Warning band                          | `rate(deopt_option_broadcast_fm_v2_rpc_failures_total[5m]) > 0`                                                                  | warning                  |
| §4 Warning band                          | `rate(deopt_option_broadcast_policy_data_failures_total[5m])` spike                                                              | warning                  |
| §4 Warning band                          | `(time() * 1000) - deopt_option_broadcast_last_submitted_ms > <stuck-threshold>`                                                 | warning                  |

The V1 alerts spec doc has been left unmodified by this PR in the public
form (no secrets added); operators map PromQL → PagerDuty/Discord
within their internal runbook. The integration plan for actual route
wiring (PagerDuty service key, Discord webhook URL) is operator-side and
explicitly NOT in scope here per the hard-stop list — no webhook
secrets were created or committed.

## 5. Signer observability

- Every signer attempt increments `signer_attempted_total{signer_kind}`.
- Every signer success increments `signer_success_total{signer_kind}`
  AND persists `last_broadcast_submitted_ms`.
- Every signer denial increments `signer_denied_total{code, signer_kind}`
  with the structured `SignerError::code()` value (e.g.
  `policy-fingerprint`, `kms-timeout`, `post-sign-from-mismatch`,
  `transport`). The signer denial path is verified to NOT call the
  chain provider (`provider.send_count() == 0` regression).
- Every mainnet runtime refusal of a `LocalDev` signer
  (`build_signer_for_state`) increments
  `local_signer_on_mainnet_refused_total`. Test
  `observability_local_signer_on_mainnet_refused_increments_counter`
  covers this.

## 6. Policy / economic observability

- `policy_approved_total{source_type}` increments only after every
  field-level + economic check (when `econ_data_available = true`)
  passes.
- `policy_rejected_total{code, source_type}` increments at the call
  site for every reject (including `dupe`), with `code` taken from
  `RejectReason::code()` (stable strings).
- `econ_data_available_true_total` / `econ_data_available_false_total`
  separate the boundary-mode vs full-mode bucket for dashboards.
- `policy_data_failures_total{read_type}` increments on every
  provider error (sanitised tag).
- Live FM_V2 read values are persisted to `last_fm_v2_rebate_budget`,
  `last_pfv_rebate_reserve`, `last_pfv_fee_balance`,
  `last_cv_pfv_balance`, and the derived `last_r5_drift_zero` gauges.

## 7. R5 / chain-state observability

- `r5_drift_observed_total` increments whenever
  `inputs.r5_drift_zero == Some(false)` is seen, even before the policy
  step rejects. Test
  `observability_r5_drift_increments_drift_counter_and_policy_internal_reject`
  covers this.
- `last_r5_drift_zero` gauge tracks the most recent value (1 = good /
  0 = drift). Alerts gate off this and the increment counter.
- `last_be_balance_wei` / `last_ome_paused` / `last_ome_is_executor`
  / `last_pfv_*` / `last_cv_pfv_balance` / `last_fm_v2_rebate_budget`
  populate from the FM_V2 milestone's live reads.

## 8. Health / admin changes

This milestone does NOT extend the JSON readiness endpoint. The
`/metrics` endpoint already exposes the full observability snapshot;
operators consume it via Prometheus.

Adding a structured JSON executor-health summary is deferred to a
follow-on (`EXECUTOR-HEALTH-ENDPOINT-V2`); the integration would expose
a subset of the snapshot fields (signer mode, last policy reject code,
BE balance status, OME paused/executor status, R5 status,
last_broadcast_submitted_ms) as JSON for non-Prometheus consumers
(internal dashboards, admin UI).

## 9. Tests added (15 new)

### Unit (9 in `options::broadcast_observability::tests`)

- `counters_increment_independently_per_label`.
- `reject_records_code_and_source_pair`.
- `signer_counters_distinguish_attempt_success_and_denial`.
- `inputs_snapshot_persists_live_read_values`.
- `dedupe_reason_persisted_when_hit`.
- `sanitize_label_strips_unsafe_chars_and_caps_length` —
  defence-in-depth label sanitisation; address-like input loses the `@`
  symbol; empty input → `unknown`.
- `econ_data_available_true_false_tracked_separately`.
- `fm_v2_failures_and_r5_drift_counters_independent`.
- `local_signer_refusal_counter_increments`.

### Integration (6 in `options::service::tests`)

- `observability_policy_approve_increments_signer_counters` — approve
  path increments `policy_approved_total{orderbook}` +
  `signer_attempted_total{remote}` + `signer_success_total{remote}` +
  `last_broadcast_submitted_ms` persisted; rejects map empty.
- `observability_policy_wash_reject_increments_counter_and_not_signer` —
  wash reject increments `policy_rejected_total{wash, orderbook}` +
  `last_policy_reject_code = wash`; signer NEVER called (`sign_calls_count() == 0`).
- `observability_econ_data_available_true_increments_when_fee_split_present` —
  3-way conjunction satisfied → `econ_data_available_true_total = 1`.
- `observability_r5_drift_increments_drift_counter_and_policy_internal_reject` —
  drift → `r5_drift_observed_total = 1` AND
  `policy_rejected_total{policy-internal, orderbook} = 1`; signer not
  called.
- `observability_signer_denial_increments_denied_counter` —
  `PolicyFingerprint` signer denial → `signer_attempted_total{remote} = 1`,
  `signer_success_total` empty, `signer_denied_total{policy-fingerprint, remote} = 1`.
- `observability_local_signer_on_mainnet_refused_increments_counter` —
  `build_signer_for_state` mainnet refusal increments the counter.

## 10. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 627 / 627 ✓ (was 612)
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 256 ✓
  grand total                                                        : 883 / 883 ✓ (was 868; +15 from this milestone)
forge fmt / forge build / forge test                                 : not re-run; no sol source touched
```

Previously-green tests preserved:
- 17 FM_V2 ABI + fee-split tests (prior milestone).
- 13 chain-state-reads tests.
- 4 prior signer integration tests.
- 31 `broadcast_policy::tests`.
- 16 `remote_signer::tests`.
- 7 config-startup-guard tests.
- 47+ option-service tests.

## 11. Remaining monitoring gaps

- **PagerDuty / Discord route wiring** — operator-side; secrets and
  webhook URLs are not committed by this PR per the hard-stop list.
  Operators implement the alert rules from §4 + paste service keys into
  their secret store.
- **JSON `executor_health` summary** — deferred to
  `EXECUTOR-HEALTH-ENDPOINT-V2`; this milestone exposes the full surface
  via `/metrics` only.
- **FM_V2 decode / RPC failure counters not yet incremented inside the
  LiveProvider** — the LiveProvider currently returns `None` on RPC /
  decode failure silently; the call site treats it as `fee_split = None`
  → boundary mode. A follow-on PR (`BACKEND-LIVE-PROVIDER-FAILURE-METRICS`)
  threads the observability handle through the LiveProvider so
  `record_fm_v2_decode_failure` / `record_fm_v2_rpc_failure` fire
  per-read. The counters EXIST and ARE rendered to `/metrics`; they
  remain at 0 until the LiveProvider is threaded.
- **Latency histograms** — counter rates can be derived from
  `*_total` counters via PromQL `rate()`. Native histograms (e.g.
  signer latency, RPC latency) are deferred to a follow-on if
  operators want them.

## 12. Forbidden / Hard stops respected

- No mainnet broadcast attempted.
- No `.env` edited.
- No webhook secrets created or committed.
- No real PagerDuty service key created.
- No real Discord webhook URL committed.
- No `EXECUTOR_PRIVATE_KEY` / RPC URL / DATABASE_URL / admin token
  printed in code, tests, docs, or commit messages.
- No high-cardinality labels (intent_id / request_id / addresses /
  hashes) ever become Prometheus label values; defence-in-depth via
  `sanitize_label`.
- No sol/ source touched.
- No DB schema migration.
- No fallback path that allows mainnet local-key signing — the runtime
  refusal counter increments BEFORE any local-signer construction.
- Remote signer cannot be called before policy approval — verified by
  the 5 existing data-provider regression tests + 3 new fee_split tests
  + 5 new observability integration tests.

## 13. Cross-references

- Spec: `deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`.
- Predecessor milestones:
  `BACKEND_FEES_MANAGER_V2_ABI_AND_FEE_SPLIT_WIRING_RESULT.md`,
  `WIRE_SHOULD_BROADCAST_CHAIN_STATE_READS_RESULT.md`,
  `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md`,
  `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md`.
- Auditor anchors strengthened: **Q-27** (transaction policy precheck
  observability) — the `signer_denied_total{code, signer_kind}` counter
  exposes the §6.6 layer reject rate per code; **Q-34** (Cluster 4 launch
  invariant) — `r5_drift_observed_total` + `last_pfv_rebate_reserve`
  give per-broadcast visibility into the primary teeth.

## 14. Next milestone recommendation

**Primary backend-side follow-on:**
`BACKEND-LIVE-PROVIDER-FAILURE-METRICS` — thread the
`broadcast_observability` handle into the LiveProvider so the FM_V2
decode + RPC failure counters fire on actual provider errors (not just
on simulated stub paths). The counters already exist; this PR just
needs them in the LiveProvider's `quote_fees_call` + `read_u256_view`
error branches.

**Parallel backend-side:** `EXECUTOR-HEALTH-ENDPOINT-V2` — JSON
summary of `BroadcastObservabilitySnapshot` for non-Prometheus
consumers. Or `BACKEND-OPTION-EXECUTION-TX-VISIBILITY-FIX` — link
broadcast / signer / confirmation log records via the
`policy_decision_id` + `kms_request_id` correlation already wired in
the signer milestone.

**Parallel operator-side (unchanged):** `MAINNET-KMS-VENDOR-SELECTION`
(Q-CD-5; gates `MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`);
`MAINNET-AUDIT-EXT-KICKOFF`; `MAINNET-TREASURY-SAFE-CREATION-PACKET`.
Internal alert-route wiring per §4 (PromQL → PagerDuty / Discord) is
operator-side and gated only on the existing alert runbook + secret
store, not on backend impl.

The observability loop is now closed at the metric layer for every
launch-critical broadcast transition. Alert rules can be deployed by
operators using the §4 mapping; signer / policy / FM_V2 / R5 / chain-
state events all surface to Prometheus with bounded cardinality and
zero secret leakage.
