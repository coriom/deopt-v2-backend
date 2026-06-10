# BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE — result

**Posture:** SHIPPED at 2026-06-10.

## 1. Goal

Surface the BE-balance-floor (the `fund_floor_wei` value the §6
broadcast-policy gate checks against `be_balance_wei`) into
`BroadcastObservabilitySnapshot` and `/executor/health/v2`. Closes the
LAST `not_tracked_yet` entry; the array is now empty.

## 2. Files changed

* `src/options/broadcast_observability.rs`
  * Added `last_be_balance_floor_wei: Option<u128>` to
    `BroadcastObservabilityInner` + `BroadcastObservabilitySnapshot`.
  * Added `record_be_balance_floor_wei(value: u128)` recorder.
  * 3 new unit tests.
* `src/options/service.rs`
  * Single call-site update inside `run_should_broadcast_policy`
    immediately after `fund_floor_wei` is computed. The recorder
    receives the EXACT same `u128` value the policy gate consumes
    (passed into the `BroadcastContext`).
  * 2 new service-level integration tests (orderbook + RFQ paths).
* `src/api/executor_health_v2.rs`
  * `chain_state_last_seen.be_balance_floor_wei` now sourced from
    `snap.last_be_balance_floor_wei` (was hard-coded `None`).
  * `not_tracked_yet` is now empty.
  * Updated `not_tracked_yet_lists_remaining_unimplemented_fields`
    test to `not_tracked_yet_is_empty_after_be_balance_floor_milestone`.
  * 2 new unit tests covering normal + Sepolia-permissive-zero.
* `src/api/routes.rs`
  * Updated `executor_health_v2_lists_not_tracked_yet_fields` to pin
    the empty array.
  * 1 new integration test
    (`executor_health_v2_surfaces_be_balance_floor_wei`).
* `docs/BACKEND_OBSERVABILITY_BE_BALANCE_FLOOR_EXPOSE_RESULT.md` —
  NEW (this document).
* `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — fourth + final
  addendum noting the empty `not_tracked_yet`.
* `docs/BACKEND_LIVE_PROVIDER_EFFECTIVE_PPM_CACHE_RESULT.md` —
  addendum cross-referencing the closure.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.

## 3. Field added

`BroadcastObservabilitySnapshot.last_be_balance_floor_wei:
Option<u128>` — `None` until the first broadcast attempt; otherwise
the most-recent `fund_floor_wei` value the policy gate consumed.

The field is rendered on the JSON envelope under
`chain_state_last_seen.be_balance_floor_wei` — the natural home for a
chain-state-relevant integer. The prior `not_tracked_yet` label
`execution_flags.be_balance_floor_wei` was a docs typo from
`BACKEND-EXECUTOR-HEALTH-ENDPOINT-V2`: the field always lived on
`chain_state_last_seen` (where the schema groups `be_balance_wei`,
`pfv_fee_balance`, etc.); this milestone closes that gap by populating
it and corrects the label drift in the not_tracked_yet array (which
now empties entirely).

## 4. `be_balance_floor` source

Recorded inside `run_should_broadcast_policy` (`src/options/service.rs:1163`)
immediately after the existing `fund_floor_wei` computation:

```rust
let fund_floor_wei = if permissive_chain_state {
    0
} else {
    state
        .execution_config
        .max_fee_per_gas_wei
        .as_deref()
        .and_then(|s| s.parse::<u128>().ok())
        .map(|gp| gp.saturating_mul(state.execution_config.max_gas_limit as u128))
        .unwrap_or(u128::MAX)
};
state
    .broadcast_observability
    .record_be_balance_floor_wei(fund_floor_wei);
```

The value's three possible derivations:

* **Permissive chain mode** (Sepolia / anvil): `0`. This is a
  legitimate policy decision, not a "fake zero" placeholder.
* **Mainnet + valid config**: `EXECUTOR_MAX_FEE_PER_GAS_WEI ×
  EXECUTOR_MAX_GAS_LIMIT` (saturating).
* **Mainnet + missing or unparseable config**: `u128::MAX` — the
  fail-closed sentinel; any plausible `be_balance_wei` will be below
  it and the broadcast will reject. The recorder reports this
  verbatim, so operators can observe the misconfiguration via the
  JSON envelope.

The recorder is called exactly once per `should_broadcast` invocation,
which is shared between orderbook and RFQ source paths.

## 5. JSON representation

`Option<u128>` rendered via the existing `serde_json` u128 path —
identical to the other chain-state fields (`be_balance_wei`,
`pfv_fee_balance`, etc.). Values within the JSON-safe integer range
(≤ 2⁵³ − 1 = 9_007_199_254_740_991 wei ≈ 9 _Pwei_) serialise
verbatim; values above lose precision in JavaScript consumers (f64),
which is acceptable since:

* The mainnet fail-closed sentinel `u128::MAX` is meant to signal a
  misconfiguration, not be consumed as a number.
* Real-world floor values (`gas_price × gas_limit`) sit far below the
  f64 precision boundary for realistic Base gas prices.

Operators reading the JSON envelope can compare the floor against
`chain_state_last_seen.be_balance_wei` (same JSON type, same unit).

## 6. Observability wiring

Single new call site in `run_should_broadcast_policy`. The recorder
is pure singleton — no cumulative counter bumped. Policy semantics,
signer call ordering, and fail-closed posture are unchanged.

Cross-source contract: orderbook + RFQ paths share the
`run_should_broadcast_policy` function. The recorder fires on every
invocation, regardless of `source_type`. Pinned by 2 service-level
integration tests:

* `observability_be_balance_floor_wei_recorded_on_orderbook_path`
  — orderbook source + Sepolia-permissive chain → `Some(0)`.
* `observability_be_balance_floor_wei_recorded_on_rfq_path` — RFQ
  source + Sepolia-permissive chain → `Some(0)` +
  `policy_approved_total{rfq}` increments.

## 7. Health endpoint changes

`GET /executor/health/v2`:

| Field                                              | Before        | After                            |
|----------------------------------------------------|---------------|----------------------------------|
| `chain_state_last_seen.be_balance_floor_wei`       | always `null` | u128 / `null`                    |
| `not_tracked_yet` length                           | 1 entry       | 0 entries — **empty array**      |

`compute_status` (green/yellow/red) is **unchanged** — the singleton
is operator-UX, not a status input.

## 8. `not_tracked_yet` final state

Empty. Every documented field on the response schema is now backed by
a live data source — either the typed config (`execution_flags`,
`live_provider_config`, etc.) or the `BroadcastObservabilitySnapshot`.

This closes the operator-UX arc opened by
`BACKEND-EXECUTOR-HEALTH-ENDPOINT-V2` (2026-06-10) and the four
follow-on singleton-persistence milestones.

## 9. Redaction / security behavior

* `be_balance_floor_wei` is a u128 — no string serialisation hazard,
  no high-cardinality label introduced.
* No new metric names, no new label keys.
* No `EXECUTOR_PRIVATE_KEY`, `RPC_URL`, or other secret material
  influences the field. The value derives entirely from
  `max_fee_per_gas_wei` (a public gas-pricing decimal) and
  `max_gas_limit` (a public integer config).
* Endpoint emits no Prometheus metrics — it consumes the bounded
  snapshot.

## 10. Tests added

### `src/options/broadcast_observability.rs::tests` (3 new)

* `be_balance_floor_wei_singleton_stores_value` — None → Some on
  first record.
* `be_balance_floor_wei_singleton_overwrites_with_most_recent` —
  multiple values including `u128::MAX` and `0`; recorder accepts
  whatever the policy gate passes.
* `be_balance_floor_wei_singleton_does_not_collide_with_be_balance_wei`
  — defence-in-depth pin: the floor (policy-config-derived) and the
  most-recent BE balance (chain-state-derived) remain independent
  singletons.

### `src/options/service.rs::tests` (2 new)

* `observability_be_balance_floor_wei_recorded_on_orderbook_path` —
  end-to-end via `broadcast_option_execution_intent_with_provider_signer_and_data_provider`
  on the orderbook source.
* `observability_be_balance_floor_wei_recorded_on_rfq_path` —
  same on the RFQ source + `policy_approved_total{rfq}` cross-check.

### `src/api/executor_health_v2.rs::tests` (2 new + 1 updated)

* `health_endpoint_surfaces_be_balance_floor_wei_singleton` —
  None → Some via build_fn.
* `health_endpoint_surfaces_zero_be_balance_floor_legitimately` —
  pin that Sepolia-permissive `0` is reported verbatim (not as
  `null`).
* `not_tracked_yet_lists_remaining_unimplemented_fields` renamed to
  `not_tracked_yet_is_empty_after_be_balance_floor_milestone` and
  asserts the array is empty.

### `src/api/routes.rs::tests` (1 new + 1 updated)

* `executor_health_v2_surfaces_be_balance_floor_wei` — end-to-end
  HTTP pin (u128 = 2 PWei value lands as JSON number).
* `executor_health_v2_lists_not_tracked_yet_fields` rewritten to
  pin the empty array.

## 11. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **959 /
  959 green** (+8 from prior baseline of 951: 3 obs unit + 2 service
  integration + 2 health unit + 1 routes integration).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 12. Forbidden-list compliance

* No mainnet tx attempted.
* No Sepolia live broadcast.
* No Safe tx.
* No governance / Timelock / ownership / guardian mutation.
* No rebate reserve allocation. No PFV withdrawal. No fund movement.
* No RFQ / order smoke.
* No `.env` edit.
* No private key / admin token / RPC secret / `DATABASE_URL` / API key
  in source or output.
* No real KMS key creation. No provider account creation.
* No guessed KMS provider credentials. No guessed mainnet executor
  address. No guessed PFV mainnet address.
* No webhook secret creation.
* No high-cardinality metric labels added (the new field is a pure
  numeric singleton; not a Prometheus label).
* No fallback path that allows mainnet local-key signing.
* No bypass flag that weakens mainnet policy.
* No secrets printed.

## 13. Next milestone recommendation

The operator-UX `/executor/health/v2` arc is now COMPLETE — every
documented field reports live data and `not_tracked_yet` is empty.
Suggested next milestones:

* **`BACKEND-MAIN-WIRE-HEALTH-V2`** — confirm the production `main.rs`
  router instantiates the v2 route alongside `/health` and `/metrics`
  (it already does via `crate::api::router(state)`, but a final
  smoke test against the binary would close the loop).
* **`BACKEND-OBSERVABILITY-PROMETHEUS-FOR-HEALTH-V2-SINGLETONS`** —
  optionally render the new singletons as Prometheus gauges
  (`deopt_option_broadcast_last_be_balance_floor_wei`,
  `deopt_option_broadcast_last_effective_maker_ppm`, etc.) so
  Grafana panels can chart their trajectories. The JSON surface
  already covers the operator-UX path; Prometheus rendering is a
  small operator-side bonus.

Parallel operator tracks unchanged: `MAINNET-KMS-VENDOR-SELECTION`
(Q-CD-5), `MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`,
`MAINNET-AUDIT-EXT-KICKOFF`, `MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
