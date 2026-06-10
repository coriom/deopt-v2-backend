# BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE — result

**Posture:** SHIPPED at 2026-06-10.

## 1. Goal

Persist the most-recent computed effective maker + taker fee ppm
values produced by the live `FeesManagerV2` / `aggregate_fee_split`
path into `BroadcastObservabilitySnapshot`, and expose them through
`/executor/health/v2`. Closes 2 of the remaining 3 `not_tracked_yet`
entries; only `execution_flags.be_balance_floor_wei` remains.

## 2. Files changed

* `src/options/broadcast_observability.rs`
  * Added `last_effective_maker_ppm: Option<i64>` +
    `last_effective_taker_ppm: Option<i64>` to
    `BroadcastObservabilityInner` + `BroadcastObservabilitySnapshot`.
  * Added `record_effective_fee_ppm(maker_ppm: i64, taker_ppm: i64)`
    recorder — pure singleton (does NOT bump
    `econ_data_available_*_total`; those are owned by
    `record_econ_data_available`).
  * 3 new unit tests.
* `src/options/service.rs`
  * Single call-site update inside
    `broadcast_option_execution_intent_with_provider_signer_and_data_provider`,
    immediately after `record_econ_data_available`. Guarded by
    `if let Some(fee_split) = inputs.fee_split.as_ref()` so missing
    `fee_split` (boundary mode) NEVER records fake `(0, 0)`.
  * 3 new service-level integration tests (orderbook ppm record /
    RFQ ppm record / boundary-mode no-record).
* `src/api/executor_health_v2.rs`
  * `economics_last_seen.effective_maker_ppm` +
    `economics_last_seen.effective_taker_ppm` now sourced from
    `snap.last_effective_maker_ppm` / `snap.last_effective_taker_ppm`
    (were hard-coded `None`).
  * `not_tracked_yet` shrunk from 3 → 1 entry (only
    `execution_flags.be_balance_floor_wei` remains).
  * Updated `not_tracked_yet_lists_remaining_unimplemented_fields`
    pin.
  * 2 new unit tests covering positive + negative ppm.
* `src/api/routes.rs`
  * Updated `executor_health_v2_lists_not_tracked_yet_fields` pin.
  * 1 new integration test
    (`executor_health_v2_surfaces_effective_maker_taker_ppm`).
* `docs/BACKEND_LIVE_PROVIDER_EFFECTIVE_PPM_CACHE_RESULT.md` — NEW
  (this document).
* `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — third addendum.
* `docs/BACKEND_FEES_MANAGER_V2_ABI_AND_FEE_SPLIT_WIRING_RESULT.md`
  — addendum noting the singletons.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.

## 3. Fields added

* `BroadcastObservabilitySnapshot.last_effective_maker_ppm:
  Option<i64>` — signed i64 because the policy gate already permits
  negative ppm under RFQ rebate-discount profiles (and rejects them on
  mainnet via the `negative-effective-ppm` reject code). `None` when
  `fee_split` was missing.
* `BroadcastObservabilitySnapshot.last_effective_taker_ppm:
  Option<i64>` — same semantics.

## 4. Effective ppm source

Values come from
`inputs.fee_split.as_ref().unwrap().effective_maker_ppm` and
`.effective_taker_ppm` — exactly the same struct fields that
`should_broadcast` reads when running §8 step 4 (negative-effective-ppm
hard gate, `src/options/broadcast_policy.rs:452-457`). The
`FeeSplitSummary` is computed by `aggregate_fee_split` at
`src/options/broadcast_policy_data.rs:369` from the two
`FeesManagerV2.quoteFees(...)` ABI-decoded responses (maker + taker).
The JSON `/executor/health/v2` surface and the policy gate are
guaranteed to read the same numbers, by construction.

## 5. Observability wiring

Single new call site in `service.rs` (inside
`broadcast_option_execution_intent_with_provider_signer_and_data_provider`,
immediately after the existing `record_econ_data_available`):

```rust
if let Some(fee_split) = inputs.fee_split.as_ref() {
    state
        .broadcast_observability
        .record_effective_fee_ppm(
            fee_split.effective_maker_ppm,
            fee_split.effective_taker_ppm,
        );
}
```

The recorder is pure singleton (no cumulative counter bumped). The
`Option<FeeSplitSummary>` guard ensures that `inputs.fee_split = None`
(boundary mode) does NOT call `record_effective_fee_ppm`, so a missing
read never produces a fake `(0, 0)` reading.

## 6. Orderbook / RFQ behavior

The broadcast call site is shared between
`OptionExecutionSourceType::OptionOrderbookFill` and
`OptionExecutionSourceType::OptionRfqFill`; the effective-ppm recorder
fires regardless of source. Two service-level integration tests pin
the contract:

* `observability_effective_fee_ppm_recorded_when_fee_split_present`
  — orderbook source with fee_split.effective_maker_ppm=42 +
  effective_taker_ppm=99 ⇒ snap reflects both values.
* `observability_effective_fee_ppm_recorded_on_rfq_path`
  — RFQ source with fee_split.effective_maker_ppm=15 +
  effective_taker_ppm=60 ⇒ snap reflects both values + the
  `policy_approved_total{rfq}` counter increments.

## 7. Missing fee_split behavior

When `inputs.fee_split = None` (boundary mode — any of `fee_split` /
`fm_v2_rebate_budget_asset` / `pfv_rebate_reserve_asset` missing):

* `record_effective_fee_ppm` is NOT called.
* `last_effective_maker_ppm` / `last_effective_taker_ppm` retain
  their previous value (likely `None` on first attempt, or the most
  recent observed pair otherwise).
* The existing `econ_data_available_false_total` counter still
  increments, recording the boundary-mode broadcast.
* §8 steps 4 / 5 / 7 of `should_broadcast` skip (preserving the
  prior milestone's fail-closed posture).

Pinned by `observability_effective_fee_ppm_not_recorded_when_fee_split_missing`.

## 8. Health endpoint changes

`GET /executor/health/v2`:

| Field                                               | Before                | After                              |
|-----------------------------------------------------|-----------------------|------------------------------------|
| `economics_last_seen.effective_maker_ppm`           | always `null`         | signed i64 / `null`                |
| `economics_last_seen.effective_taker_ppm`           | always `null`         | signed i64 / `null`                |
| `not_tracked_yet` length                            | 3 entries             | 1 entry                            |

Remaining `not_tracked_yet`:

* `execution_flags.be_balance_floor_wei` — operator-configured BE
  balance floor policy input. Tracked under
  `BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE`.

`compute_status` (green/yellow/red) is **unchanged** — the singletons
are operator-UX, not status inputs.

## 9. Redaction / security behavior

* `effective_maker_ppm` / `effective_taker_ppm` are i64 — no string
  serialisation hazard, no high-cardinality label introduced.
* Endpoint emits no Prometheus metrics — it consumes the bounded
  snapshot.
* The policy gate's existing `negative-effective-ppm` reject code on
  mainnet is unchanged; the JSON surface only reports, does not gate.
* No new metric names; no new label keys.

## 10. Tests added

### `src/options/broadcast_observability.rs::tests` (3 new)

* `effective_fee_ppm_singleton_stores_both_sides` — None → Some on
  first call; both singletons populated together.
* `effective_fee_ppm_singleton_overwrites_with_most_recent` —
  multiple calls overwrite; negative values representable.
* `effective_fee_ppm_singleton_independent_of_econ_data_available_counter`
  — recorder MUST NOT bump `econ_data_available_*_total`.

### `src/options/service.rs::tests` (3 new)

* `observability_effective_fee_ppm_recorded_when_fee_split_present`
  — orderbook source.
* `observability_effective_fee_ppm_recorded_on_rfq_path` —
  RFQ source + `policy_approved_total{rfq}` increment.
* `observability_effective_fee_ppm_not_recorded_when_fee_split_missing`
  — boundary mode, singletons remain `None`,
  `econ_data_available_false_total` still increments.

### `src/api/executor_health_v2.rs::tests` (2 new + 1 updated)

* `health_endpoint_surfaces_effective_maker_taker_ppm_singletons` —
  end-to-end via build_fn.
* `health_endpoint_surfaces_negative_effective_ppm` — signed i64
  representation pin.
* `not_tracked_yet_lists_remaining_unimplemented_fields` rewritten —
  asserts the 6 shipped fields are absent + only the
  `be_balance_floor_wei` entry remains (asserts `.len() == 1`).

### `src/api/routes.rs::tests` (1 new + 1 updated)

* `executor_health_v2_surfaces_effective_maker_taker_ppm` —
  end-to-end HTTP pin (negative + positive ppm).
* `executor_health_v2_lists_not_tracked_yet_fields` rewritten for
  the shrunk array.

## 11. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **951 /
  951 green** (+9 from prior baseline of 942: 3 obs unit + 3 service
  integration + 2 health unit + 1 routes integration).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 12. Remaining `not_tracked_yet`

Just one entry:

* `execution_flags.be_balance_floor_wei` — operator-configured BE
  balance floor policy input. Tracked under
  `BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE` (small non-launch-blocking
  follow-on; the value is already computed inside
  `service.rs:1217-1227` as the local `fund_floor_wei` variable, but
  is not propagated into the snapshot).

After that, `/executor/health/v2` will report every documented field
with live data, closing the entire `not_tracked_yet` arc.

## 13. Forbidden-list compliance

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
* No high-cardinality metric labels added (the new fields are pure
  numeric singletons; not Prometheus labels).
* No fallback path that allows mainnet local-key signing.
* No bypass flag that weakens mainnet policy.
* No secrets printed.

## 14. Next milestone recommendation

`BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE` — last
`not_tracked_yet` entry: surface `be_balance_floor_wei` into the
snapshot so `chain_state_last_seen.be_balance_floor_wei` lands. After
that, the `not_tracked_yet` array empties entirely. Parallel operator
tracks unchanged: `MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5),
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`,
`MAINNET-AUDIT-EXT-KICKOFF`, `MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
