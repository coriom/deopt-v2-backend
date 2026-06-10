# BACKEND-EXECUTOR-HEALTH-ENDPOINT-V2 — result

**Posture:** SHIPPED at 2026-06-10.
**Path:** `GET /executor/health/v2`.

> **Addendum (2026-06-10, follow-on `BACKEND-OBSERVABILITY-LAST-SINGLETON-FIELDS`):**
> the three singleton fields `signer.last_signer_error_code`,
> `policy_gate.last_reject_source_type`, and
> `policy_gate.econ_data_available_last` are now populated by the
> `BroadcastObservabilitySnapshot` and no longer appear in
> `not_tracked_yet`. Status logic (§6) and redaction contract (§5) are
> unchanged. Remaining `not_tracked_yet` entries shrunk from 7 to 4 —
> see `docs/BACKEND_OBSERVABILITY_LAST_SINGLETON_FIELDS_RESULT.md` for
> details.
>
> **Addendum (2026-06-10, follow-on `BACKEND-OBSERVABILITY-LAST-POLICY-DATA-FAILURE-SINGLETON`):**
> the fourth singleton `policy_gate.last_policy_data_failure_type` is
> now populated by the `BroadcastObservabilitySnapshot` from the bounded
> `read_type::*` taxonomy and no longer appears in `not_tracked_yet`.
> Status logic + redaction contract unchanged. Remaining `not_tracked_yet`
> entries shrunk from 4 to 3 — see
> `docs/BACKEND_OBSERVABILITY_LAST_POLICY_DATA_FAILURE_SINGLETON_RESULT.md`
> for details.
>
> **Addendum (2026-06-10, follow-on `BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE`):**
> the 5th + 6th singletons `economics_last_seen.effective_maker_ppm`
> and `effective_taker_ppm` are now populated from the same
> `FeeSplitSummary` fields that drive `should_broadcast`'s §8
> negative-effective-ppm gate. Status logic + redaction contract
> unchanged. Remaining `not_tracked_yet` shrunk from 3 to 1
> (`execution_flags.be_balance_floor_wei` only) — see
> `docs/BACKEND_LIVE_PROVIDER_EFFECTIVE_PPM_CACHE_RESULT.md`.

## 1. Goal

Add a non-sensitive JSON executor health endpoint v2 that summarises
executor, signer, policy gate, live-provider config, chain-state,
economic, and observability status for admin / frontend / operator
consumers. Read-only, secret-safe, never broadcasts. Complements (does
not replace) the existing legacy `/executor/status` endpoint and the
`/metrics` Prometheus surface — non-Prometheus consumers now have a
canonical JSON envelope they can hit.

## 2. Endpoint path

* Route: `GET /executor/health/v2` (registered in
  `src/api/routes.rs::router`).
* Auth: public (consistent with `/health`, `/ready`, `/executor/status`,
  `/indexer/status`, `/reconciliation/status`). The response carries
  only non-secret information; admin token is NOT required.
* HTTP status: always 200 with the structured envelope; partial data
  (no observations yet, optional fields) is reported as `null` plus an
  entry in `not_tracked_yet`.

## 3. Response schema

`crate::api::executor_health_v2::ExecutorHealthV2Response`:

```jsonc
{
  "service": {
    "name": "deopt-v2-backend",
    "ok": true,
    "timestamp_ms": <i64>,
    "network": "<network_name>",
    "chain_id": <u64>,
    "persistence_enabled": <bool>
  },
  "execution_flags": {
    "execution_enabled": <bool>,
    "real_broadcast_enabled": <bool>,
    "option_broadcast_enabled": <bool>,
    "simulation_enabled": <bool>,
    "confirmation_worker_enabled": <bool>,
    "nonce_sync_enabled": <bool>,
    "policy_gate_enabled": <bool>,
    "remote_signer_enabled": <bool>,
    "local_signer_allowed": <bool>,
    "executor_chain_id": <u64>,
    "executor_from_address": <hex|null>,
    "option_matching_engine_address": <hex|null>,
    "perp_matching_engine_address": <hex|null>
  },
  "signer": {
    "signer_mode": "local_dev" | "remote",
    "remote_signer_configured": <bool>,
    "signer_address": <hex|null>,
    "last_signer_kind": <str|null>,
    "last_signer_success_at_ms": <i64|null>,
    "last_signer_error_code": <str|null>,
    "local_signer_on_mainnet_refused_total": <u64>
  },
  "policy_gate": {
    "approved_total": <u64>,
    "rejected_total": <u64>,
    "last_reject_code": <str|null>,
    "last_reject_source_type": <str|null>,
    "last_policy_data_failure_type": <str|null>,
    "econ_data_available_last": <bool|null>
  },
  "live_provider_config": {
    "protocol_fee_vault_configured": <bool>,
    "fees_manager_v2_configured": <bool>,
    "collateral_vault_configured": <bool>,
    "protocol_fee_vault_address": <hex|null>,
    "fees_manager_v2_address": <hex|null>,
    "collateral_vault_address": <hex|null>
  },
  "chain_state_last_seen": {
    "be_balance_wei": <u128|null>,
    "be_balance_floor_wei": <u128|null>,
    "ome_paused": <bool|null>,
    "ome_is_executor": <bool|null>,
    "pfv_fee_balance": <u128|null>,
    "pfv_rebate_reserve": <u128|null>,
    "cv_pfv_balance": <u128|null>
  },
  "economics_last_seen": {
    "fm_v2_rebate_budget": <u128|null>,
    "effective_maker_ppm": <i64|null>,
    "effective_taker_ppm": <i64|null>,
    "econ_data_available_true_total": <u64>,
    "econ_data_available_false_total": <u64>
  },
  "r5": {
    "drift_zero_last_seen": <bool|null>,
    "drift_observed_total": <u64>
  },
  "recent_policy_decisions": {
    "approved_by_source_type": {<source_type>: <u64>, ...},
    "rejected_by_code_source_type": [
      {"code": <str>, "source_type": <str>, "count": <u64>},
      ...
    ]
  },
  "recent_signer_events": {
    "attempted_by_kind": {<kind>: <u64>, ...},
    "success_by_kind": {<kind>: <u64>, ...},
    "denied_by_code_kind": [
      {"code": <str>, "signer_kind": <str>, "count": <u64>},
      ...
    ]
  },
  "observability": {
    "fm_v2_decode_failures_total": <u64>,
    "fm_v2_rpc_failures_total": <u64>,
    "policy_data_failures_total": {<read_type>: <u64>, ...},
    "last_dedupe_reason": <str|null>,
    "last_broadcast_submitted_ms": <i64|null>
  },
  "warnings": [<str>, ...],
  "hard_stops": [<str>, ...],
  "not_tracked_yet": [<str>, ...],
  "overall_status": "green" | "yellow" | "red",
  "reasons": [<str>, ...]
}
```

## 4. Fields exposed

All fields are non-sensitive:

* Booleans + counters from `BroadcastObservabilitySnapshot` (already
  rendered into `/metrics`).
* Configured-bools for PFV / FM_V2 / CV (already rendered into
  `/metrics` as the matching `*_configured` gauges).
* Public contract addresses (OME / PFV / CV / FM_V2 / BE) — these are
  already exposed by `/admin/config` and `/metrics` and are part of the
  public chain identity surface.
* Public network + chain id (already in `/health` / `/admin/status`).

## 5. Fields explicitly NOT exposed

The endpoint never serialises:

* Private keys / seed phrases / mnemonics.
* RPC URLs (they may contain provider tokens).
* `BACKEND_SIGNER_ENDPOINT` URLs (they may carry mTLS routing tokens or
  vendor-secret paths).
* Admin tokens / API tokens / JWT bytes.
* `DATABASE_URL`.
* Webhook URLs.
* Personal contact data.
* Provider credentials / KMS request tokens.

A redaction unit test pins the contract: even when a state happens to
carry a private key in memory + an RPC URL with an embedded token, none
of those values appear in the JSON output. The schema only exposes the
field types listed in §3 — there is no `Option<String>` field that
could carry an arbitrary URL or token.

## 6. Status logic (`overall_status`)

Conservative; defaults to green when nothing is wrong:

* **red** if any of:
  * `local_signer_on_mainnet_refused_total > 0` (operator-config drift —
    a local-dev signer was attempted on mainnet at runtime and the
    defence-in-depth gate refused it).
  * Mainnet (`chain_id == 8453`) AND `BACKEND_SIGNER_MODE == local_dev`.
  * Mainnet AND `EXECUTOR_PRIVATE_KEY` is set.
  * Mainnet AND `real_broadcast_enabled` AND `BACKEND_SIGNER_ENDPOINT`
    is unset/empty.
  * `last_ome_paused == Some(true)` (latest broadcast attempt observed
    OME paused on-chain).
  * `last_ome_is_executor == Some(false)` (BE was de-authorised in NEW
    OME registry).
  * `last_r5_drift_zero == Some(false)` (R5 invariant violation:
    `CV(PFV,asset) != feeBalance + rebateReserve`).
* **yellow** if any of (and no red):
  * `real_broadcast_enabled || option_broadcast_enabled` AND any of
    `protocol_fee_vault_configured` / `fees_manager_v2_configured` /
    `collateral_vault_configured` is false.
  * `real_broadcast_enabled || option_broadcast_enabled` AND no
    `be_balance` observation yet (no broadcast has populated the live
    snapshot).
  * `BACKEND_SIGNER_MODE == remote` AND non-mainnet — remote signer
    client currently uses the `UnimplementedTransport` placeholder; sign
    attempts return `signer:transport`. Mainnet hits the missing-endpoint
    or missing-config red branch instead.
  * Cumulative `fm_v2_rpc_failures_total > 0` or
    `fm_v2_decode_failures_total > 0`.
* **green** if neither red nor yellow.

The `hard_stops` array carries the subset of red reasons tied to
custody-policy hard stops + observed chain-state failures, so operators
can wire a dedicated alert against `hard_stops.length > 0`. The
`reasons` array enumerates all current red + yellow conditions in
human-readable form for the operator UI.

## 7. Tests added

### `src/api/executor_health_v2.rs::tests` (15 unit tests)

* `green_when_disabled_and_no_observations` — bare AppState ⇒ green,
  no hard_stops, no warnings.
* `yellow_when_real_broadcast_enabled_but_pfv_unconfigured` — sepolia
  remote-signer config with PFV/FM_V2/CV unconfigured ⇒ yellow + warning.
* `red_when_mainnet_local_dev_signer_seated` — defence-in-depth: a
  bypassed mainnet config with `LocalDev` mode + env-key ⇒ red.
* `red_when_ome_paused_observed` — `BroadcastPolicyInputs.ome_paused =
  Some(true)` ⇒ red + hard_stop.
* `red_when_be_is_not_executor` — `ome_is_executor = Some(false)` ⇒
  red + hard_stop.
* `red_when_r5_drift_observed` — `r5_drift_zero = Some(false)` ⇒ red
  + hard_stop.
* `red_when_local_signer_mainnet_refusal_observed` — observed counter
  ⇒ red + hard_stop.
* `signer_block_reflects_remote_mode` — schema check.
* `account_id_or_none_collapses_zero_address` — zero-address +
  empty-string both collapse to `None` (no operator confusion with
  unconfigured slots).
* `not_tracked_yet_lists_unimplemented_fields` — pins the contract
  that `last_signer_error_code` / `effective_maker_ppm` / etc. appear
  in `not_tracked_yet`.
* `pfv_fm_v2_cv_configured_booleans_track_indexer_config` — schema
  check.
* `response_serialises_without_panicking` — JSON round-trip
  smoke test.
* `response_redacts_secret_envelope_strings` — even with an
  `EXECUTOR_PRIVATE_KEY` and an RPC URL with a token in memory, the
  JSON output contains no `private_key` / `database_url` / `rpc_url`
  substrings nor the literal secret bytes.
* `policy_gate_totals_sum_across_source_types` — approved/rejected
  totals sum across labels.
* `options_broadcast_enabled_warns_when_pfv_missing_even_without_real_broadcast`
  — option-broadcast flag also triggers the live-provider-config warning
  band.

### `src/api/routes.rs::tests` (4 integration tests)

* `executor_health_v2_returns_200_with_envelope` — `GET
  /executor/health/v2` returns HTTP 200; envelope structure matches the
  schema; default state is green.
* `executor_health_v2_does_not_expose_secrets` — even when the state
  carries a private key + an RPC URL with an embedded token + a signer
  endpoint with a secret path, the response body contains none of those
  strings.
* `executor_health_v2_reports_pfv_fm_v2_cv_configured_booleans` —
  end-to-end schema check via HTTP.
* `executor_health_v2_lists_not_tracked_yet_fields` — end-to-end pin
  that the `not_tracked_yet` array carries the documented gaps.

## 8. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **926 /
  926 green** (+19 from the prior baseline of 907; 15 unit + 4
  integration added by this milestone).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 9. Files changed

* `src/api/executor_health_v2.rs` — NEW (~600 LoC + 15 unit tests).
* `src/api/mod.rs` — declares `executor_health_v2` and re-exports
  `build_executor_health_v2`, `ExecutorHealthV2Response`, `HealthStatus`.
* `src/api/routes.rs` — registers `GET /executor/health/v2` and the
  4-line handler `executor_health_v2`; appends 4 integration tests in
  the `tests` submodule.
* `deopt-v2-backend/docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — NEW
  (this document).
* `deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` —
  documents that the JSON health endpoint is now available alongside
  the Prometheus surface.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.

## 10. Validations + forbidden-list

* No mainnet tx attempted.
* No Sepolia live broadcast.
* No Safe tx.
* No governance / Timelock / ownership / guardian mutation.
* No rebate reserve allocation.
* No PFV withdrawal.
* No fund movement.
* No `.env` edit.
* No private key / admin token / RPC secret / `DATABASE_URL` / API key
  in source or output.
* No real KMS key creation.
* No provider account creation.
* No guessed KMS provider credentials.
* No guessed mainnet executor address.
* No guessed PFV mainnet address.
* No webhook secret creation.
* No high-cardinality metric labels added (the endpoint emits no
  metrics — it consumes the existing `BroadcastObservabilitySnapshot`
  which already has bounded cardinality).
* No fallback path that allows mainnet local-key signing.
* No bypass flag that weakens mainnet policy.
* No secrets printed.

## 11. Remaining gaps (not blockers)

* `last_signer_error_code` — the broadcast call site does not yet
  persist the most-recent denial code as a singleton (only as a
  per-(code, kind) BTreeMap counter). Field reported as `null`; listed
  in `not_tracked_yet`. Tracked for a small follow-on:
  `BACKEND-OBSERVABILITY-LAST-SINGLETON-FIELDS`.
* `last_reject_source_type` + `last_policy_data_failure_type` +
  `econ_data_available_last` — same shape (singleton-of-most-recent
  fields). Tracked under the same follow-on.
* `be_balance_floor_wei` — operator configures the BE balance floor as
  policy input; the observability snapshot does not yet carry it.
  Listed in `not_tracked_yet`.
* `effective_maker_ppm` / `effective_taker_ppm` — the broadcast call
  site computes them when `econ_data_available == true` but does not
  persist the most-recent values into the snapshot. Listed in
  `not_tracked_yet`.

None of these gaps block mainnet readiness — the chain-state-level
hard-stop detection (R5 / OME / BE-executor) is fully wired and is what
the launch-day operator checklist depends on.

## 12. Next milestone recommendation

`BACKEND-OBSERVABILITY-LAST-SINGLETON-FIELDS` (small follow-on; persist
`last_reject_source_type`, `last_signer_error_code`, and
`econ_data_available_last` so the corresponding fields move out of
`not_tracked_yet`) OR `BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE`
(operator-visibility for the most-recent effective maker/taker ppm so
the `economics_last_seen` block carries live data) — both are
non-launch-blocking polishing follow-ons. Parallel operator-side tracks
unchanged: `MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5),
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`, `MAINNET-AUDIT-EXT-KICKOFF`,
`MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`, `FRONTEND-V2G-W3-SSR-PROXY`.
