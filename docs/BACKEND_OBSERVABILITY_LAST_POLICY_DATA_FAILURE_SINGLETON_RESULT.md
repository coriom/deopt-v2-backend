# BACKEND-OBSERVABILITY-LAST-POLICY-DATA-FAILURE-SINGLETON — result

**Posture:** SHIPPED at 2026-06-10.

## 1. Goal

Persist `last_policy_data_failure_type` from the bounded live-provider
`read_type::*` taxonomy into `BroadcastObservabilitySnapshot` and
expose it through `/executor/health/v2`. Closes the 4th of the
originally-7 `not_tracked_yet` entries; only 3 non-launch-blocking
gaps remain (`be_balance_floor_wei`, `effective_maker_ppm`,
`effective_taker_ppm`).

## 2. Files changed

* `src/options/broadcast_observability.rs`
  * Added `last_policy_data_failure_type: Option<String>` to
    `BroadcastObservabilityInner` + `BroadcastObservabilitySnapshot`.
  * `record_policy_data_failure(read_type)` now also sets the
    singleton — using the same `sanitize_label(read_type)`-normalised
    string that drives the existing
    `policy_data_failures_total{read_type}` Prometheus label.
  * 3 new unit tests.
* `src/api/executor_health_v2.rs`
  * `policy_gate.last_policy_data_failure_type` now sourced from
    `snap.last_policy_data_failure_type` (was hard-coded `None`).
  * `not_tracked_yet` shrunk from 4 → 3 entries.
  * Updated `not_tracked_yet_lists_remaining_unimplemented_fields`
    test pin.
  * 1 new unit test covering the end-to-end snap → JSON path.
* `src/api/routes.rs`
  * Updated `executor_health_v2_lists_not_tracked_yet_fields` pin.
  * 1 new integration test
    (`executor_health_v2_surfaces_last_policy_data_failure_type`).
* `docs/BACKEND_OBSERVABILITY_LAST_POLICY_DATA_FAILURE_SINGLETON_RESULT.md`
  — NEW (this document).
* `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — addendum noting the
  4th singleton exited `not_tracked_yet`.
* `docs/BACKEND_OBSERVABILITY_LAST_SINGLETON_FIELDS_RESULT.md` —
  addendum cross-referencing the follow-on.
* `RUN_STATE.md` — closure paragraph appended.

No `.env` edited. No `sol/` source touched. No DB schema migration.

## 3. Field added

`BroadcastObservabilitySnapshot.last_policy_data_failure_type:
Option<String>` — bounded singleton populated by every
`record_policy_data_failure(read_type)` call. Value is exactly the
sanitised `read_type` string used as the
`policy_data_failures_total{read_type}` Prometheus label, so JSON +
Prometheus surfaces share one taxonomy.

## 4. `read_type` taxonomy

The `crate::options::broadcast_policy_data::read_type` module exposes
a fixed set of `pub const &str` values:

| Constant                        | String                       | Origin                                  |
|---------------------------------|------------------------------|-----------------------------------------|
| `CHAIN_ID_RPC`                  | `chain_id_rpc`               | `eth_chainId` RPC failure               |
| `BE_BALANCE`                    | `be_balance`                 | `eth_getBalance(BE)` RPC failure        |
| `OME_PAUSED`                    | `ome_paused`                 | `paused()` view failure / decode        |
| `OME_IS_EXECUTOR`               | `ome_is_executor`            | `isExecutor(BE)` view failure / decode  |
| `PFV_FEE_BALANCE`               | `pfv_fee_balance`            | `feeBalance(asset)` view failure        |
| `PFV_REBATE_RESERVE`            | `pfv_rebate_reserve`         | `rebateReserve(asset)` view failure     |
| `CV_PFV_BALANCE`                | `cv_pfv_balance`             | `balances(PFV, asset)` view failure     |
| `FM_V2_QUOTE_FEES_RPC`          | `fm_v2_quote_fees_rpc`       | `quoteFees(...)` RPC failure            |
| `FM_V2_QUOTE_FEES_DECODE`       | `fm_v2_quote_fees_decode`    | `quoteFees(...)` ABI-decode failure     |
| `FM_V2_REBATE_BUDGET`           | `fm_v2_rebate_budget`        | `rebateBudget(asset)` view failure      |

`sanitize_label` (lowercase + alnum + `-`/`_`/`:` whitelist + 48-char
cap) wraps every input, so even a pathological caller cannot route a
URL / address / 0x bytes / free-form error string into the singleton.
The defence-in-depth contract is pinned by a new unit test:
`policy_data_failure_singleton_remains_bounded_under_arbitrary_input`.

## 5. Policy-data observability wiring

No new call sites added. The existing
`LiveBroadcastPolicyDataProvider::record_data_failure` helper at
`src/options/broadcast_policy_data.rs:481` already routes through
`BroadcastObservability::record_policy_data_failure(read_type)` on
every RPC / decode failure across 10 read sites
(`gather_inputs`, `quote_fees_call`, `fm_v2_rebate_budget_call`). The
update lives entirely inside `record_policy_data_failure`, which now
sets `inner.last_policy_data_failure_type = Some(key)` alongside the
existing counter increment. Fail-closed posture preserved (RPC /
decode failures still yield `None` in `BroadcastPolicyInputs` →
mainnet fail-closed via chain-state gates).

## 6. Health endpoint changes

`GET /executor/health/v2`:

| Field                                       | Before                  | After                              |
|---------------------------------------------|-------------------------|------------------------------------|
| `policy_gate.last_policy_data_failure_type` | always `null`           | bounded `read_type` string / `null`|
| `not_tracked_yet` length                    | 4 entries               | 3 entries                          |

Remaining `not_tracked_yet`:

* `execution_flags.be_balance_floor_wei`
* `economics_last_seen.effective_maker_ppm`
* `economics_last_seen.effective_taker_ppm`

`compute_status` (green/yellow/red) is **unchanged** — the singleton
is operator-UX, not used to derive `overall_status`. The conservative
custody-policy-pinned semantics from BACKEND-EXECUTOR-HEALTH-ENDPOINT-V2
are preserved.

## 7. Redaction / security behavior

* `sanitize_label` codomain is bounded: lowercase ASCII alnum +
  `-`/`_`/`:` only + 48-char cap; empty / invalid → `"unknown"`. URL-shaped
  pathological input (`https://rpc.example/sensitive-provider-key?token=abc`)
  loses every URL-structural punctuation (`://`, `/`, `?`, `=`, `.`) — pinned
  by `policy_data_failure_singleton_remains_bounded_under_arbitrary_input`.
* No new metric names. No new label keys. No high-cardinality field
  surfaced.
* Endpoint emits no Prometheus metrics — it consumes the bounded
  snapshot.

## 8. Tests added

### `src/options/broadcast_observability.rs::tests` (3 new)

* `policy_data_failure_stores_last_failure_type_singleton` — None →
  `Some("fm_v2_quote_fees_rpc")` after one call; counter still
  increments alongside the singleton.
* `policy_data_failure_singleton_overwrites_with_most_recent` —
  Most-recent reads override earlier values; cumulative counters
  preserved per-bucket.
* `policy_data_failure_singleton_remains_bounded_under_arbitrary_input`
  — Pathological URL-shaped input is stripped of `://`/`/`/`?`/`=`/`.`
  and length-bounded.

### `src/api/executor_health_v2.rs::tests` (1 new + 1 updated)

* `health_endpoint_surfaces_last_policy_data_failure_type_singleton`
  — None → Some(read_type) end-to-end via the build_fn.
* `not_tracked_yet_lists_remaining_unimplemented_fields` rewritten
  to include the 4th shipped field in the "must not appear" set and
  drop it from the "still pending" set.

### `src/api/routes.rs::tests` (1 new + 1 updated)

* `executor_health_v2_surfaces_last_policy_data_failure_type` —
  end-to-end HTTP pin that the singleton lands in the JSON envelope
  under the bounded read_type string.
* `executor_health_v2_lists_not_tracked_yet_fields` updated for the
  shrunk array.

## 9. Tests run

* `cargo fmt --check` — clean.
* `cargo clippy --all-targets --all-features -- -D warnings` — clean.
* `cargo test --all-targets --all-features --no-fail-fast` — **942 /
  942 green** (+5 from prior baseline of 937: 3 obs + 1 health unit +
  1 routes integration).
* `git diff --check` — clean.
* `forge fmt / build / test` — not re-run (no `sol/` source touched).

## 10. Remaining `not_tracked_yet` entries

* `execution_flags.be_balance_floor_wei` — operator-configured BE
  balance floor policy input. Tracked under
  `BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE`.
* `economics_last_seen.effective_maker_ppm` /
  `economics_last_seen.effective_taker_ppm` — most-recent computed
  effective ppm. Tracked under
  `BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE`.

Neither blocks mainnet readiness — the launch-day operator checklist
depends on chain-state hard-stop detection (R5 / OME / BE-executor),
which is fully wired.

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
* No real KMS key creation. No provider account creation.
* No guessed KMS provider credentials. No guessed mainnet executor
  address. No guessed PFV mainnet address.
* No webhook secret creation.
* No high-cardinality metric labels added (the singleton runs through
  the same `sanitize_label` whitelist as the existing metric).
* No fallback path that allows mainnet local-key signing.
* No bypass flag that weakens mainnet policy.
* No secrets printed.

## 12. Next milestone recommendation

`BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE` (operator visibility for
`effective_maker_ppm` / `effective_taker_ppm`) OR
`BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE` (surface
`be_balance_floor_wei` from the policy-config plumbing). Both are
small non-launch-blocking polishing follow-ons. Parallel operator
tracks unchanged: `MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5),
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`,
`MAINNET-AUDIT-EXT-KICKOFF`, `MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`,
`FRONTEND-V2G-W3-SSR-PROXY`.
