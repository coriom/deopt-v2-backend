# BACKEND-OBSERVABILITY-LAST-SINGLETON-FIELDS — result

**Posture:** SHIPPED at 2026-06-10.

> **Addendum (2026-06-10, follow-on `BACKEND-OBSERVABILITY-LAST-POLICY-DATA-FAILURE-SINGLETON`):**
> the recommended next-milestone in §12 below shipped. The 4th singleton
> `policy_gate.last_policy_data_failure_type` is now persisted from the
> `read_type::*` taxonomy; `not_tracked_yet` shrunk from 4 to 3 entries.
> See `docs/BACKEND_OBSERVABILITY_LAST_POLICY_DATA_FAILURE_SINGLETON_RESULT.md`.

## 1. Goal

Persist the three last-singleton observability fields needed by
`/executor/health/v2` so they exit `not_tracked_yet`:

* `last_reject_source_type` — bounded source-type label of the most
  recent `should_broadcast` rejection.
* `last_signer_error_code` — bounded code of the most recent signer
  error (denial / transport / unimplemented / mainnet-refused).
* `econ_data_available_last` — boolean reflecting the most recent
  econ-data availability decision.

All three were previously cumulative counters only; the broadcast
pipeline did not retain the most-recent value as a singleton.

## 2. Files changed

* `src/options/broadcast_observability.rs`
  * Added 3 fields to `BroadcastObservabilityInner` +
    `BroadcastObservabilitySnapshot`.
  * Updated `record_policy_rejected`, `record_signer_denied`,
    `record_econ_data_available`, `record_local_signer_on_mainnet_refused`
    to populate the new singletons.
  * Added public constant `LOCAL_MAINNET_REFUSED_CODE = "local_mainnet_refused"`
    (defines the stable code stored when the mainnet runtime guard
    refuses a LocalDev signer — distinct from the `SignerError::code()`
    taxonomy because that branch never reaches a signer).
  * 5 new unit tests covering each singleton + redaction.
* `src/api/executor_health_v2.rs`
  * `signer.last_signer_error_code` now sourced from
    `snap.last_signer_error_code` (was `None`).
  * `policy_gate.last_reject_source_type` now sourced from
    `snap.last_reject_source_type` (was `None`).
  * `policy_gate.econ_data_available_last` now sourced from
    `snap.econ_data_available_last` (was `None`).
  * `not_tracked_yet` shrunk from 7 → 4 entries.
  * Updated `not_tracked_yet_lists_remaining_unimplemented_fields` test
    (renamed from `…_lists_unimplemented_fields`) to pin the new state.
  * 5 new unit tests covering each singleton end-to-end + a URL-shape
    redaction pin on `last_signer_error_code`.
* `src/api/routes.rs`
  * Updated `executor_health_v2_lists_not_tracked_yet_fields`
    integration test for the new `not_tracked_yet` shape.
  * 1 new integration test
    `executor_health_v2_surfaces_singleton_observability_fields`
    asserting all three singletons land in the HTTP JSON envelope.
* `docs/BACKEND_OBSERVABILITY_LAST_SINGLETON_FIELDS_RESULT.md` — NEW
  (this document).
* `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — addendum noting the 3
  fields exited `not_tracked_yet`.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.

## 3. Fields added

### 3.1 `BroadcastObservabilitySnapshot.last_reject_source_type: Option<String>`

Populated by `record_policy_rejected(code, source_type)`. Value is
`source_type_label(source_type)` — one of the bounded
`"orderbook" | "rfq"` strings already used as the
`policy_rejected_total{source_type}` Prometheus label. The cumulative
counter is unchanged; this singleton is a "what just happened"
companion.

### 3.2 `BroadcastObservabilitySnapshot.last_signer_error_code: Option<String>`

Populated by two paths:

* `record_signer_denied(code, signer_kind)` — `code` comes from
  `SignerError::code()` per the §4.2 stable taxonomy: `chain-not-allowed`,
  `target-not-allowed`, `selector-not-allowed`, `value-not-zero`,
  `gas-cap`, `calldata-bind-mismatch`, `nonce-mismatch`,
  `deadline-expired`, `policy-fingerprint`, `policy-stale`,
  `duplicate-request-id`, `duplicate-intent-signed`, `caller-unauthorized`,
  `rate-limit`, `post-sign-from-mismatch`, `kms-unavailable`,
  `kms-timeout`, `transport`, `internal`, `config-refusal`. Every value
  is bounded, length-capped at 48 chars by `sanitize_label`, and
  strips non-alnum punctuation other than `-`, `_`, `:`. The
  `UnimplementedTransport` placeholder returns the `transport` variant,
  which lands here.
* `record_local_signer_on_mainnet_refused` — stores the literal
  `LOCAL_MAINNET_REFUSED_CODE` (`"local_mainnet_refused"`). This branch
  fires when `build_signer_for_state` defence-in-depth refuses a
  `LocalDev` signer on mainnet — no signer is contacted, so no
  `SignerError` exists.

The singleton never carries endpoint URL, credentials, request ID,
intent ID, or any free-form caller text. Three redaction tests (one in
each of the observability + health + routes modules) pin the contract.

### 3.3 `BroadcastObservabilitySnapshot.econ_data_available_last: Option<bool>`

Populated by `record_econ_data_available(available)`. Reflects the
final `econ_data_available` flag computed at the broadcast call site
(`inputs.fee_split.is_some() && inputs.fm_v2_rebate_budget_asset.is_some()
&& inputs.pfv_rebate_reserve_asset.is_some()` — see
`src/options/service.rs:1619-1624`). `None` before the first broadcast
attempt; `Some(true)` when full economic context was observed;
`Some(false)` in boundary mode.

## 4. Policy observability wiring

`src/options/service.rs::broadcast_option_execution_intent_with_provider_signer_and_data_provider`
already calls `record_econ_data_available(...)` (line 1623),
`record_policy_rejected(code, source_type)` on the three reject paths
(lines 1574, 1637, 1656), and `record_local_signer_on_mainnet_refused()`
in `build_signer_for_state` (line 1471). No new call sites were
introduced — only the underlying recorders were updated to populate the
singletons alongside the existing cumulative counters. Policy
semantics, fail-closed posture, and signer call ordering are
unchanged.

## 5. Signer observability wiring

`src/options/service.rs:1755` calls
`record_signer_denied(signer_err.code(), signer_kind.as_str())` on
every signer-error path (denial, transport, unimplemented, internal,
config-refusal). The recorder now also populates
`last_signer_error_code`. The `RemoteSignerClient::UnimplementedTransport`
returns `SignerError::Transport("…")` which yields code `"transport"`,
so the singleton reflects that case automatically. Defence-in-depth
mainnet refusal is captured separately via
`record_local_signer_on_mainnet_refused()` → singleton value
`"local_mainnet_refused"`.

## 6. Health endpoint changes

`GET /executor/health/v2` response:

| Field                                       | Before                  | After                              |
|---------------------------------------------|-------------------------|------------------------------------|
| `policy_gate.last_reject_source_type`       | always `null`           | `"orderbook"` / `"rfq"` / `null`   |
| `policy_gate.econ_data_available_last`      | always `null`           | `true` / `false` / `null`          |
| `signer.last_signer_error_code`             | always `null`           | bounded code / `null`              |
| `not_tracked_yet` length                    | 7 entries               | 4 entries                          |

Remaining `not_tracked_yet`:

* `execution_flags.be_balance_floor_wei`
* `policy_gate.last_policy_data_failure_type`
* `economics_last_seen.effective_maker_ppm`
* `economics_last_seen.effective_taker_ppm`

The status logic in `compute_status` is **unchanged** — the singletons
are surfaced for operator UX, not used to compute `overall_status`.
This preserves the conservative custody-policy-pinned semantics that
the previous milestone established.

## 7. Redaction / security behavior

* `last_signer_error_code` runs through `sanitize_label` — lowercase
  + alnum + `-` + `_` + `:` only + 48-char cap. URL-shaped pathological
  input (`https://signer.invalid/secret-path?token=abc`) is stripped
  of `:`, `/`, `?`, `=`, `.` — the resulting value cannot be parsed
  as a routable URL. Test pin:
  `last_signer_error_code_does_not_carry_endpoint_url_even_under_pathological_input`.
* `last_reject_source_type` is bounded to the existing two-element
  `source_type_label` codomain (`"orderbook"` | `"rfq"`); no caller
  user input flows here.
* `econ_data_available_last` is a `bool`; no serialisation hazard.
* No metric names changed. No new metric labels added. No new
  high-cardinality field surfaced. The endpoint emits no Prometheus
  metrics — it consumes the bounded `BroadcastObservabilitySnapshot`.

## 8. Tests added

### `src/options/broadcast_observability.rs::tests` (5 new)

* `reject_stores_last_reject_source_type_singleton` — first reject sets
  `Some("rfq")`; later reject overrides with `Some("orderbook")` +
  preserves `last_policy_reject_code` consistency.
* `signer_denied_stores_last_signer_error_code_singleton` — most-recent
  overrides earlier; pins single-source semantics.
* `local_mainnet_refusal_sets_last_signer_error_code` —
  `record_local_signer_on_mainnet_refused()` sets
  `Some("local_mainnet_refused")` + bumps counter.
* `signer_error_code_remains_bounded_under_arbitrary_input` — URL-shaped
  pathological input is sanitised; no `@`, `?`, length-bounded.
* `econ_data_available_last_reflects_most_recent_decision` — true →
  Some(true); false → Some(false); cumulative counters still increment
  alongside the singleton.

### `src/api/executor_health_v2.rs::tests` (5 new + 1 updated)

* `health_endpoint_surfaces_last_reject_source_type_singleton` — JSON
  envelope reflects the rejection's source type.
* `health_endpoint_surfaces_last_signer_error_code_singleton` — JSON
  envelope reflects the signer denial code.
* `health_endpoint_surfaces_local_mainnet_refusal_as_signer_error_code` —
  defence-in-depth refusal surfaces via the singleton.
* `health_endpoint_surfaces_econ_data_available_last_singleton` — None
  → Some(true) → Some(false) transitions.
* `last_signer_error_code_does_not_carry_endpoint_url_even_under_pathological_input`
  — URL-structure redaction pin.
* `not_tracked_yet_lists_remaining_unimplemented_fields` (renamed +
  rewritten) — asserts the 3 shipped fields are GONE and the 4
  remaining fields are present.

### `src/api/routes.rs::tests` (1 new + 1 updated)

* `executor_health_v2_surfaces_singleton_observability_fields` — end-to-end
  HTTP pin that the 3 singletons populate the JSON envelope.
* `executor_health_v2_lists_not_tracked_yet_fields` (rewritten) —
  pins the new `not_tracked_yet` shape.

## 9. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **937 /
  937 green** (+11 from prior baseline of 926: 5 obs + 5 health unit +
  1 routes integration).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 10. Remaining `not_tracked_yet` fields

* `execution_flags.be_balance_floor_wei` — operator-configured policy
  input. Tracked under `BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE`
  (small follow-on; surface `state.execution_config` or a new
  policy-input field into the snapshot).
* `policy_gate.last_policy_data_failure_type` — singleton of the most
  recent `read_type` that the LiveProvider failed on. Tracked under
  `BACKEND-OBSERVABILITY-LAST-POLICY-DATA-FAILURE-SINGLETON` (small
  follow-on).
* `economics_last_seen.effective_maker_ppm` /
  `economics_last_seen.effective_taker_ppm` — most-recent computed
  effective ppm. Tracked under
  `BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE` (parallel follow-on; the
  broadcast call site computes the values but does not persist them).

None of the remaining gaps block mainnet readiness — the chain-state
hard-stop detection (R5 / OME / BE-executor) is fully wired.

## 11. Forbidden-list compliance

* No mainnet tx attempted.
* No Sepolia live broadcast.
* No Safe tx.
* No governance / Timelock / ownership / guardian mutation.
* No rebate reserve allocation.
* No PFV withdrawal.
* No fund movement.
* No RFQ / order smoke.
* No `.env` edit.
* No private key / admin token / RPC secret / `DATABASE_URL` / API key
  in source or output.
* No real KMS key creation.
* No provider account creation.
* No guessed KMS provider credentials.
* No guessed mainnet executor address.
* No guessed PFV mainnet address.
* No webhook secret creation.
* No high-cardinality metric labels added (the new singletons run
  through the same `sanitize_label` whitelist as the existing labels).
* No fallback path that allows mainnet local-key signing.
* No bypass flag that weakens mainnet policy.
* No secrets printed.

## 12. Next milestone recommendation

`BACKEND-OBSERVABILITY-LAST-POLICY-DATA-FAILURE-SINGLETON` (small
follow-on; persist `last_policy_data_failure_type` so the 4th
`not_tracked_yet` entry exits) OR
`BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE` (operator-visibility for
`effective_maker_ppm` / `effective_taker_ppm` so the
`economics_last_seen` block carries live data). Parallel operator
tracks unchanged: `MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5),
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`,
`MAINNET-AUDIT-EXT-KICKOFF`, `MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
