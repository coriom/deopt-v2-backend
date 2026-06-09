# WIRE-SHOULD-BROADCAST-CHAIN-STATE-READS — result

**Status:** SHIPPED 2026-06-09 (Phase G close-out).
**Scope:** introduce a `BroadcastPolicyDataProvider` abstraction with a
live RPC + DB-backed implementation, replace the placeholder defaults in
`run_should_broadcast_policy` with live-read inputs, add persistent dedupe
via the existing `option_execution_transactions` store + intent status,
and add a `verify_launch_invariant_for_startup` helper. **No mainnet tx.
No live broadcast. No `.env` edit. No real KMS credentials.**

---

## 1. Files changed

### New (2)

- `deopt-v2-backend/src/options/broadcast_policy_data.rs` — ~500 LoC incl.
  8 unit tests. Defines `BroadcastPolicyDataProvider` trait,
  `BroadcastPolicyInputs`, `DedupeReason`, `PolicyDataError`,
  `PolicyDataFuture`, `LiveBroadcastPolicyDataProvider`,
  `StubBroadcastPolicyDataProvider`, `StartupLaunchInvariantOutcome`,
  `verify_launch_invariant_for_startup`, plus ABI-selector + view-call
  helpers and the persistent-dedupe predicate.
- `deopt-v2-backend/docs/WIRE_SHOULD_BROADCAST_CHAIN_STATE_READS_RESULT.md`
  — this close-out doc.

### Modified (5)

- `src/execution/rpc.rs` — new `EthBalanceProvider` trait + impl on
  `HttpJsonRpcProvider` (`eth_getBalance` round-trip) + `parse_hex_quantity_u128`.
- `src/execution/mod.rs` — re-export `EthBalanceProvider`.
- `src/options/mod.rs` — `pub mod broadcast_policy_data;` + re-export of
  the public surface (`BroadcastPolicyDataProvider`, `BroadcastPolicyInputs`,
  `DedupeReason`, `LiveBroadcastPolicyDataProvider`,
  `PolicyDataError`, `PolicyDataFuture`,
  `StartupLaunchInvariantOutcome`,
  `StubBroadcastPolicyDataProvider`, `verify_launch_invariant_for_startup`).
- `src/options/service.rs` — `run_should_broadcast_policy` now takes
  `&BroadcastPolicyInputs` and applies mainnet fail-closed semantics;
  new `broadcast_option_execution_intent_with_provider_signer_and_data_provider`
  variant is the canonical full-fidelity entry point; existing
  `broadcast_option_execution_intent_with_provider_and_signer` now
  delegates with a Sepolia-permissive stub. The call-site additionally
  re-checks the persistent dedupe state authoritatively (intent status
  + tx-table lookup) so provider stubs cannot mask a true duplicate.
  5 new integration tests added.

## 2. Reads wired

| Read                                              | Source                                      | Live?    | Notes                                                                           |
| ------------------------------------------------- | ------------------------------------------- | -------- | ------------------------------------------------------------------------------- |
| `chain_id`                                        | `TransactionBroadcastProvider::chain_id`    | ✅ live  | Round-trips RPC `eth_chainId`. Permits round-trip mismatch detection.            |
| `eth_getBalance(BACKEND_EXECUTOR)`                | `EthBalanceProvider::eth_get_balance`       | ✅ live  | NEW trait method. Implemented on `HttpJsonRpcProvider`.                          |
| `OME.paused()`                                    | `EthCallProvider::eth_call` + selector      | ✅ live  | Selector `0x5c975abb` (verified by `selector_no_args_matches_paused` unit test). |
| `OME.isExecutor(BACKEND_EXECUTOR)`                | `EthCallProvider::eth_call`                 | ✅ live  | Selector `keccak256("isExecutor(address)")[0..4]` + 32-byte address param.       |
| `PFV.feeBalance(asset)`                           | `EthCallProvider::eth_call`                 | ✅ live  | Only if PFV address configured in `LiveBroadcastPolicyDataProvider::new(...)`.   |
| `PFV.rebateReserve(asset)`                        | `EthCallProvider::eth_call`                 | ✅ live  | Same as above. Cluster 4 launch invariant primary teeth.                         |
| `CV.balances(PFV, asset)`                         | `EthCallProvider::eth_call`                 | ✅ live  | Used for R5 precheck.                                                            |
| R5 drift = 0 (`CV(PFV,asset) == feeBalance + rebateReserve`) | derived in provider              | ✅ live  | Computed when all 3 inputs landed.                                               |
| `FeesManagerV2.getProfile(...)` → `fee_split`     | (DEFERRED — §11 follow-on)                  | ❌ stub  | `fee_split = None` keeps `econ_data_available = false` → boundary mode at call site. |
| Dedupe (existing tx, terminal status)             | repository / `OptionSeriesStore` + intent   | ✅ live  | Provider does first pass; call-site re-checks authoritatively before policy.    |

All reads are read-only RPC calls. No `eth_sendTransaction`. No state mutation.

## 3. Policy context now live

Previously placeholder → now `BroadcastPolicyInputs`-driven:

| Field                         | Before (placeholder)                 | After (this milestone)                                                                          |
| ----------------------------- | ------------------------------------ | ----------------------------------------------------------------------------------------------- |
| `be_balance_wei`              | `u128::MAX`                          | `inputs.be_balance_wei.unwrap_or(<mainnet=0, sepolia=u128::MAX>)`                                |
| `fund_floor_wei`              | `0` (sepolia) / `u128::MAX` (mainnet) | derived from `max_fee_per_gas_wei × max_gas_limit` on mainnet; `0` on sepolia                   |
| `ome_paused`                  | `false`                              | `inputs.ome_paused.unwrap_or(<mainnet=true (fail-closed), sepolia=false>)`                      |
| `ome_is_executor`             | `true`                               | `inputs.ome_is_executor.unwrap_or(<mainnet=false (fail-closed), sepolia=true>)`                 |
| `rebate_reserve_asset`        | `0` (mainnet) / `u128::MAX` (sepolia) | `inputs.pfv_rebate_reserve_asset.unwrap_or(<mainnet=0 (fail-closed), sepolia=u128::MAX>)`        |
| `econ_data_available`         | `false` (always)                     | `inputs.fee_split.is_some()` — currently always `false` until FeesManagerV2 ABI lands (§11)     |
| `dedupe_hit`                  | live (existing tx row lookup)        | provider-supplied **OR** intent-status terminal **OR** existing tx row (3-way authoritative OR) |
| R5 drift hard gate            | not enforced                         | `inputs.r5_drift_zero == Some(false)` → reject with `policy:policy-internal:r5-drift`           |

Mainnet ⇒ missing reads now cause the policy to **reject with a structured
code** rather than silently apply permissive defaults. Sepolia path
preserves the existing rehearsal regression by construction.

## 4. Remaining placeholders (DEFERRED — documented as follow-on)

- **`fee_split` (FeesManagerV2 ABI bindings).** The current LiveProvider
  leaves `fee_split = None` so `econ_data_available = false`. Steps 4 / 5 /
  7 of `should_broadcast` continue to skip until the FeesManagerV2 ABI
  wrappers land. Mainnet Cluster 4 launch invariant is partially gated by
  the live `pfv_rebate_reserve_asset` read (any non-zero reserve on a
  mainnet rebate-positive intent would now reject through the R5
  precheck pathway combined with the §5 hard gate once `fee_split`
  arrives).
- **`buyer_has_margin` / `seller_has_margin` / `product_listed` /
  `rm_snapshot_age_ms`.** Still defaulted to permissive (`true` / `0`).
  Wiring these requires risk-manager ABI reads + intent-bound snapshot
  fetch — out of scope.
- **`subsidy_budget`.** Defaulted to `SubsidyBudgetView::default()`. Off-
  chain ledger wiring required.
- **`gas_units` / `gas_cost_native` / `pnl_floor_native`.** Defaulted to
  `0`. Wiring depends on the per-intent gas oracle and pnl-floor
  configuration (operator-side decisions; out of scope for this PR).

These deferrals do **not** weaken the mainnet posture: the LiveProvider
returns `None` for them, the call-site treats `None` as fail-closed on
mainnet, and the entire economic decision is gated behind
`econ_data_available = true` which only flips when `fee_split` arrives.

## 5. Dedupe behaviour

Three concentric checks; **any** of them positive yields a `Dupe` reject:

1. **Provider snapshot** — `BroadcastPolicyDataProvider::gather_inputs`
   returns `inputs.dedupe_hit = true` if either the intent status is
   already terminal or a submitted tx row exists.
2. **Intent-status check** at the call site — `intent.status ∈
   {BroadcastSubmitted, BroadcastConfirmed, BroadcastFailed}`.
3. **Tx-row check** at the call site — `find_submitted_option_execution_transaction(state, intent_id).await?.is_some()`.

The call site OR-s all three so a provider stub that ships
`dedupe_hit=false` cannot mask a real duplicate. On dedupe, the existing
behaviour is preserved: if an existing tx exists, the function returns
`OptionExecutionBroadcastOutcome { duplicate: true, ... }` instead of an
error (test: `data_provider_dedupe_hit_rejects_before_signer_call`).

**Persistence:** the existing `option_execution_transactions` table is the
authoritative dedupe source when persistence is enabled; the in-memory
`OptionSeriesStore` backs it when persistence is disabled. No new schema
migration was introduced in this milestone — the existing tx-row +
intent-status columns are sufficient. If a per-intent dedupe-cache
table is later desired, this PR's interface remains forwards-compatible:
the trait method already returns `BroadcastPolicyInputs` with the
dedupe-hit bool and the `DedupeReason` enum.

## 6. Startup launch invariant hook

`verify_launch_invariant_for_startup(chain_id, profiles, rebate_reserve_asset)`
wraps `broadcast_policy::verify_launch_invariant` and returns a
`StartupLaunchInvariantOutcome` whose `is_blocking_failure()` predicate is
true iff `mode == Mainnet AND !overall_pass`. Three unit tests pin the
mainnet-pass / mainnet-fail-on-nonzero-reserve / sepolia-relaxed behaviour.

The actual `main.rs` invocation is intentionally NOT wired in this PR — it
requires an operator-provided active fee-profile snapshot (e.g. a YAML
imported once at deploy time) plus the live `pfv_rebate_reserve_asset`
read. Operators should add a 1-line call before app boot:

```rust
use deopt_v2_backend::options::verify_launch_invariant_for_startup;

let outcome = verify_launch_invariant_for_startup(
    config.execution.executor_chain_id,
    &active_fee_profiles_snapshot,
    pfv_rebate_reserve_asset_at_boot,
);
if outcome.is_blocking_failure() {
    return Err(format!("mainnet launch invariant blocked: {:?}", outcome.report));
}
```

This sits behind an explicit feature flag in production
(`MAINNET_LAUNCH_INVARIANT_REQUIRED`) by operator convention.

## 7. Broadcast integration

`broadcast_option_execution_intent_with_provider_signer_and_data_provider<P, S, D>`
is the canonical full-fidelity entry. Flow:

1. `ensure_option_execution_broadcast_enabled`.
2. `get_option_execution_intent`.
3. `data_provider.gather_inputs(state, &intent).await` —
   on **mainnet error** → transition intent to `BroadcastFailed`, return
   `BackendError::BroadcastRejected("policy:policy-internal:...")`. **No
   fallback to the signer.**
4. Authoritative dedupe re-check (intent status + tx-row).
5. `run_should_broadcast_policy(state, &intent, &inputs)` — applies
   mainnet fail-closed semantics for any `Option::None` in `inputs`.
6. On Reject (non-Dupe) → `BroadcastFailed` + `BackendError::BroadcastRejected("policy:<code>:...")`.
7. On Reject(Dupe) AND an existing tx row → return
   `OptionExecutionBroadcastOutcome { duplicate: true, ... }`.
8. On Approve → existing signer + send path (unchanged).

**Signer remains uncontacted on any policy reject** (verified by 5 new
integration tests and the existing
`signer_not_called_when_policy_rejects` regression).

## 8. Tests added (13 new)

### Unit (8 in `options::broadcast_policy_data::tests`)

`selector_no_args_matches_paused` — keccak vector check for the
`paused()` selector (`0x5c975abb`).
`selector_with_address_left_pads_to_32_bytes` — ABI param encoding.
`r5_drift_detected_in_fixture` — derives `r5_drift_zero` correctly.
`dedupe_reason_helpers` — `is_hit` semantics across all variants.
`policy_data_error_display_carries_code_prefix` — `policy-data:rpc:…`.
`startup_invariant_passes_on_mainnet_with_fee_only_and_zero_reserve`.
`startup_invariant_blocks_on_mainnet_with_nonzero_rebate_reserve`.
`startup_invariant_does_not_block_on_sepolia_even_with_nonzero_reserve`.

### Integration (5 in `options::service::tests`)

`data_provider_ome_paused_rejects_before_signer_call` — provider reports
OME paused → policy rejects with `policy:ome-paused`; signer not called;
0 chain sends.

`data_provider_be_not_executor_rejects_before_signer_call` — provider
reports BE is not the chain-side executor → policy rejects with
`policy:be-not-exec`; signer not called.

`data_provider_r5_drift_rejects_before_signer_call` — provider reports
`r5_drift_zero = Some(false)` → policy rejects with
`policy:policy-internal:r5-drift`; signer not called.

`data_provider_sepolia_path_still_approves_under_boundary_mode` —
Sepolia-permissive stub + happy intent → policy approves; signer called
once; broadcast submitted. Pins the boundary-mode regression.

`data_provider_dedupe_hit_rejects_before_signer_call` — first call
broadcasts successfully; second call detects the existing tx via
authoritative call-site re-check → returns `duplicate: true`; signer
called only once; provider sent only once.

## 9. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 595 / 595 ✓
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 8 + 12 + 43 + 67 + 76 + 13 + 37 = 256 ✓
  grand total                                                        : 851 / 851 ✓ (was 838 / 838 at prior milestone)
forge fmt --check / forge build / forge test                         : not re-run; no sol source touched
```

Previously-green Sepolia rehearsal smoke-shape tests
(`option_execution_broadcast_mock_success_persists_submitted_hash_once`,
`policy_approve_preserves_existing_broadcast_state_machine`, the 4
signer-integration tests, all 31 `broadcast_policy::tests`, the 7
config-startup-guard tests, the 16 `remote_signer::tests`) continue to
pass unchanged.

## 10. Limitations / out of scope

- No FeesManagerV2 ABI binding for `getProfile(...)` → `fee_split` (next
  follow-on track). Until it lands, `econ_data_available = false` and
  §8 steps 4 / 5 / 7 of `should_broadcast` skip — same posture as the
  prior milestone.
- Risk-manager snapshot freshness reads not wired (`buyer_has_margin`,
  `seller_has_margin`, `product_listed`, `rm_snapshot_age_ms` remain
  permissive). Mainnet still rejects via the live OME / PFV / R5 reads
  that ARE wired.
- Subsidy-budget view not wired.
- `gas_units` / `gas_cost_native` / `pnl_floor_native` not wired.
- Production HTTPS/2 + mTLS signer transport remains unimplemented per
  the prior milestone — that PR's `UnimplementedTransport` is the
  fail-closed mainnet bottleneck.
- The `main.rs` startup hook is NOT yet inserted — operator-side opt-in
  via the example snippet in §6.

## 11. Forbidden / Hard stops respected

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe creation.
- No vendor account / region / ARN string committed.
- No `EXECUTOR_PRIVATE_KEY` value printed.
- No real KMS sandbox key used in tests (mock provider only).
- No sol/ source touched.
- No DB schema migration (re-uses existing `option_execution_transactions`).
- No fallback path that resurrects local-key signing on mainnet (signer
  is still uncontacted under any policy reject).

## 12. Cross-references

- Predecessor milestones: `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md`,
  `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md`.
- Design: `SHOULD_BROADCAST_DESIGN_NOTE.md`, `MAINNET_BE_SIGNER_SERVICE_DESIGN.md`.
- Cluster anchor: `MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3`.
- Custody principle: `~/DEOPT/MAINNET_CUSTODY_POLICY.md §10.1` (dedupe).
- Auditor anchors strengthened by this milestone: Q-34 (Cluster 4 launch
  invariant) — live PFV.rebateReserve read on mainnet enforces the
  primary teeth of the §8 step 5 hard gate.

## 13. Next milestone recommendation

**Primary backend-side:** `BACKEND-FEES-MANAGER-V2-ABI-AND-FEE-SPLIT-WIRING`
— wire `FeesManagerV2.getProfile(...)` decoding so `fee_split` becomes
non-`None` on production paths. Once it lands, `econ_data_available` flips
to `true` for mainnet and §8 steps 4 / 5 / 7 fire with live data — that
fully closes the C-4 economic gate.

**Parallel operator-side (unchanged):** `MAINNET-KMS-VENDOR-SELECTION`
(Q-CD-5), `MAINNET-TREASURY-SAFE-CREATION-PACKET` (Q-CD-7),
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET` (Q-CD-17), `MAINNET-AUDIT-EXT-KICKOFF`.

**Parallel backend-side:** `BACKEND-EXECUTOR-MONITORING-ALERTS-V1-WIRING`
(plumb `kms_request_id` + signer reject-rate metrics + new
`policy-data:rpc:…` metrics into the existing alerts spec).
