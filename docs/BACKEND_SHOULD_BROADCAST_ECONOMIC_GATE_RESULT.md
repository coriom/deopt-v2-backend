# BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE — result

**Status:** SHIPPED 2026-06-09 (Phase E close-out).
**Scope:** implement §8 `should_broadcast` policy gate + Cluster 4 launch
invariant verifier; wire gate into `broadcast_option_execution_intent_with_provider`
as the canonical pre-broadcast decision.
**Hard stops respected:** no mainnet broadcast, no chain tx, no Safe tx, no
`.env` edit, no PFV withdrawal, no governance mutation, no
`EXECUTOR_PRIVATE_KEY` / RPC / DATABASE_URL / admin token printed, no
`--no-verify`.

---

## 1. Files changed

### New (3)

- `deopt-v2-backend/src/options/broadcast_policy.rs` — policy module
  (~1 300 LoC incl. tests; `should_broadcast`, `verify_launch_invariant`,
  `BroadcastContext`, `ShouldBroadcastDecision`, `RejectReason`, `ApprovalReason`,
  `LaunchInvariantReport`).
- `deopt-v2-backend/docs/SHOULD_BROADCAST_DESIGN_NOTE.md` — Phase B design
  note (gate plug-in point, signature, decision enum, mode awareness, env
  integration).
- `deopt-v2-backend/docs/BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md` —
  this close-out doc.

### Modified (2)

- `deopt-v2-backend/src/options/mod.rs` — `pub mod broadcast_policy;` +
  re-export of the policy surface.
- `deopt-v2-backend/src/options/service.rs` — new `selector_hex` helper +
  `run_should_broadcast_policy` context builder + call site in
  `broadcast_option_execution_intent_with_provider`; 3 existing tests updated
  to assert the new structured policy-derived error shape; 2 new regression
  tests (Test 22 / 23).

## 2. Policy checks implemented

§8 step ordering as implemented in `should_broadcast`:

| §   | Check                       | Reject code              | Mode  |
| --- | --------------------------- | ------------------------ | ----- |
| 0   | chain id match              | `chain-id-mismatch`      | all   |
| 0   | dedupe cache hit            | `dupe`                   | all   |
| 0   | buyer signature present     | `buyer-sig-missing`      | all   |
| 0   | seller signature present    | `seller-sig-missing`     | all   |
| 0   | intent status allow-list    | `invalid-state`          | all   |
| 0   | calldata non-empty          | `calldata-missing`       | all   |
| 0   | deadline in future          | `expired`                | all   |
| 0   | nonces present              | `nonce-unsynced`         | all   |
| 0   | product listed              | `product-unlisted`       | all   |
| 0   | RM snapshot age ≤ max       | `stale-rm`               | all   |
| 0   | call_target in allow-list   | `target-not-allowed`     | all   |
| 0   | call_selector in allow-list | `selector-not-allowed`   | all   |
| 1   | OME paused == false         | `ome-paused`             | all   |
| 1   | OME.isExecutor(BE) == true  | `be-not-exec`            | all   |
| 1   | BE balance ≥ FUND_FLOOR     | `be-low-bal`             | all   |
| 2   | buyer has margin            | `no-buyer-margin`        | all   |
| 2   | seller has margin           | `no-seller-margin`       | all   |
| 3   | simulation ok               | `sim-revert` / `simulation-not-ok` | all |
| 3   | gas_units ≤ HARD_GAS_CAP    | `gas-cap`                | all   |
| 6   | buyer ≠ seller (wash)       | `wash`                   | all   |
| 4   | gross_fee + rebate > 0      | `no-econ-content`        | econ  |
| 4   | effective ppm ≥ 0           | `negative-effective-ppm` | mainnet+econ |
| 5   | rebate_budget ≥ outflow     | `rebate-budget`          | econ  |
| 5   | rebate_reserve ≥ outflow    | `rebate-reserve`         | econ  |
| 7   | expected_pnl decision       | Approve(Profitable/AtCost/Subsidisable) or Reject(`no-econ-content`) | econ |

**Mode = econ:** controlled by `BroadcastContext::econ_data_available`. When
`false` (current wiring; Sepolia + boundary mode), steps 4/5/7 skip and the
policy approves on field-level pass. When `true` (future PR wires fee_split +
on-chain rebate budget / reserve), all §8 steps fire.

The boundary checks (steps 0–3 + step 6 wash) fire unconditionally — this is
the canonical pre-broadcast gate even before the economic data is wired.

## 3. Cluster 4 launch invariant verifier

`verify_launch_invariant(profiles, rebate_reserve_asset, mode) -> LaunchInvariantReport`:

- Reads each `ActiveFeeProfile { tier, product, flow, maker_ppm, taker_ppm,
  maker_discount_ppm, taker_discount_ppm }`.
- Computes effective `(makerPpm, takerPpm)` after the per-flow discount
  (`apply_discount` clamps discount at 1_000_000 ppm).
- Returns:
  ```
  LaunchInvariantReport {
      all_profiles_non_negative: bool,
      profiles: Vec<EffectiveProfileEntry>,
      rebate_reserve: u128,
      rebate_reserve_zero: bool,
      overall_pass: bool,
  }
  ```
- Mainnet `overall_pass` = `all_profiles_non_negative AND rebate_reserve_zero`.
- Sepolia `overall_pass` = `all_profiles_non_negative` (reserve-nonzero
  allowed on Sepolia rehearsal harness).

This is the AUDIT-EXT Q-34 attestation point and the chain-side backstop
for §8 step 5.

## 4. Error codes (stable strings)

`dupe` · `buyer-sig-missing` · `seller-sig-missing` · `expired` ·
`nonce-unsynced` · `product-unlisted` · `stale-rm` · `ome-paused` ·
`be-not-exec` · `be-low-bal` · `no-buyer-margin` · `no-seller-margin` ·
`sim-revert` · `gas-cap` · `wash` · `quota-breach` · `attack-pattern` ·
`no-econ-content` · `rebate-budget` · `rebate-reserve` ·
`negative-effective-ppm` · `chain-id-mismatch` · `selector-not-allowed` ·
`target-not-allowed` · `simulation-not-ok` · `calldata-missing` ·
`invalid-state` · `liquidation-out-of-scope` · `policy-internal`.

Message shape: `policy:<code>:<non-sensitive-detail>`.

## 5. Tests added (38)

### Unit (32, all in `broadcast_policy::tests`)

Field-level checks: 15 (chain-id, dupe, buyer-sig, seller-sig, invalid-state,
calldata-missing, expired, nonce-unsynced, product-unlisted (covered via
broader path), stale-rm, target-not-allowed, selector-not-allowed,
ome-paused, be-not-exec, be-low-bal).

§8 §3 sim-revert + gas-cap: 2.
§8 §4 econ-content + negative-effective-ppm: 2.
§8 §5 rebate-budget + rebate-reserve (Cluster 4 primary teeth): 2.
§8 §6 wash: 1.
§8 §7 approvals: 3 (Profitable, AtCost, Subsidisable).
Fee-only with reserve nonzero: 1 (Approve).
Boundary-mode coverage: 3 (boundary-approve, boundary-wash still fires,
boundary-chain-id still fires).
Launch invariant: 5 (Test 16/17/18/19 + Sepolia mode + discount-pushing-negative).

### Service-level regression (2)

- `policy_approve_preserves_existing_broadcast_state_machine` (Test 22) —
  Approve path leaves `broadcast_submitted` transition + tx persistence intact.
- `policy_reject_transitions_cleanly_without_half_state` (Test 23) — Reject
  drives `broadcast_failed` cleanly; no tx row; error prefixed `policy:wash`.

### Existing tests updated (3)

- `option_execution_broadcast_missing_calldata_rejects_without_transaction` —
  now asserts `BroadcastRejected("calldata-missing")`.
- `option_execution_broadcast_missing_signatures_rejects_without_send` — now
  asserts `BroadcastRejected("buyer-sig-missing")`.
- `option_execution_broadcast_requires_simulation_ok_by_default` — now
  asserts `BroadcastRejected("sim-revert")`.

## 6. Tests run

```
cargo fmt --all -- --check                : ok
cargo clippy --all-targets --all-features -- -D warnings : ok
cargo test --all-targets --all-features --no-fail-fast :
    lib   : 553 passed; 0 failed
    +integration / api / nonce_sync / fees / rfq / options / signatures :
    additional 76 + 67 + 43 + 37 + 13 + 12 + 8 ... all green
    grand total : 0 failures
forge fmt --check                         : ok
forge build                               : ok (pre-existing forge-lint warnings only)
forge test --no-match-path 'test/fork/*'  : 367 passed; 0 failed; 0 skipped
```

Sepolia regression assumption is preserved by construction:
- The wired path runs the policy in **boundary mode** (`econ_data_available =
  false`), so the existing Sepolia fee-only smoke regression expectations
  (`FIRST_LIVE_OPTION_EXECUTION_SMOKE_RESULT_V2_SEPOLIA.md` +
  `FIRST_LIVE_RFQ_OPTION_EXECUTION_SMOKE_RESULT_SEPOLIA.md`) are unchanged.
  Field-level rejections that the policy now catches earlier still result in
  `BroadcastRejected` + `broadcast_failed` — same external observable
  behaviour, structured code instead of free-text.

## 7. Limitations / remaining work

- **Economic data not yet wired (follow-on track).** The wired call site
  passes `econ_data_available = false` because the backend does not yet
  synthesise fee_split + on-chain `rebate_budget` / `rebate_reserve` reads
  synchronously at the broadcast site. Track 2 of the
  `BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md §1.3` (FeesManagerV2 oracle +
  PFV view cache) will flip this to `true` and activate steps 4/5/7 in the
  call-site path.
- **Chain-state defaults are Sepolia-permissive.** On mainnet, the wired
  context currently uses `fund_floor_wei = u128::MAX` (always rejects on
  insufficient balance — fail-closed) and `rebate_reserve_asset = 0`
  (Cluster 4 launch invariant matches reality). Other chain-state facts
  (OME paused, isExecutor, margins, product listed, RM age) are wired
  permissively because the backend does not yet have synchronous chain-read
  access to them at the broadcast site. Track 1 (signer-service design) and
  Track 4 (monitoring wiring) deliver these reads.
- **Liquidation carve-out (§8 step 8) is out of scope.** Per Q-CD-12 default
  Option A (liquidation not in launch scope); the policy will reject any
  liquidation candidate as `liquidation-out-of-scope` when the caller flags
  it (placeholder; no flag yet wired).
- **Anti-griefing heuristics are minimal.** Only the address-equality wash
  check is implemented. `quota-breach` and `attack-pattern` codes exist but
  are not yet triggered from a heuristic; follow-on PR per design note §7.
- **Startup invariant hook is design-only.** The `verify_launch_invariant`
  function exists; the startup hook that exits non-zero on mainnet +
  `overall_pass == false` is not yet wired into `main.rs`. The design note
  §6 documents the hook + feature flag (`mainnet_launch_invariant_required`,
  default OFF). Wired by operator at mainnet cutover.
- **Persistent dedupe cache.** The `dedupe_hit` flag currently consults the
  existing `find_submitted_option_execution_transaction` lookup. A dedicated
  in-memory + Redis-backed cache is deferred to a follow-up.

## 8. Out of scope (explicitly NOT done)

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe creation.
- No ownership / guardian / Timelock mutation.
- No `EXECUTOR_PRIVATE_KEY` / RPC / DATABASE_URL / admin token printed in
  code, tests, PR description, commit messages, or this doc.
- No source modification in `deopt-v2-sol/`.
- No DB migration (no schema change; reuses existing `option_execution_intents`
  + `option_execution_transactions` rows).

## 9. Cross-references

- Spec: `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8`.
- Cluster 4 launch invariant: `MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3`.
- Acceptance criteria: `BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md §1.1 + §2.3`.
- Origin prompt: `NEXT_TASK_BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE.md`.
- Build handoff: `PREBUILD_TO_BUILD_HANDOFF.md §5.1 + §6.4`.
- Design: `SHOULD_BROADCAST_DESIGN_NOTE.md`.
- Auditor anchor: AUDIT-EXT Q-34.
- Closes gap-list: C-4 / W-3.

## 10. Next milestone recommendation

Per the prebuild-to-build handoff doc, the next recommended backend task is
**`MAINNET-BE-SIGNER-SERVICE-DESIGN`** (read-only design milestone), followed
by KMS adapter implementation. Operator-side parallel tracks:
`MAINNET-KMS-VENDOR-SELECTION`, `MAINNET-TREASURY-SAFE-CREATION-PACKET`,
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`. The current PR closes gap-list
C-4 / W-3 and unlocks the AUDIT-EXT Q-34 attestation point.
