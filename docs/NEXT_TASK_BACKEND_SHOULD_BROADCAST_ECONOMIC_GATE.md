# NEXT TASK — BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE

**Posture:** ready-to-run prompt for the next milestone. Hand this
file (verbatim) to the backend implementer (developer or agent) once
Cluster 4 closure is signed and AUDIT-EXT engagement kickoff is
scheduled. **No mainnet broadcast. No chain mutation by this build
task. No `.env` secret printed.**

**Closure milestone:** `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` (gap-list C-4 / W-3; Cluster 4 launch invariant verifier).

---

## Prompt (begin)

---

Workspace root is `~/DEOPT`.

Execute `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` only.

### Current state

- Sepolia rehearsal arc is complete: V2G-GOV-G closed; first live orderbook smoke + first live RFQ smoke closed; R5 drift = 0 across 7 governance tx + 2 live trades.
- Mainnet OPS Safe + GOV Safe deployed and chain-anchored (Cluster 1).
- Custody Clusters 1 + 2 + 3 + 4 all RESOLVED at policy / architecture / formula layer.
- Cluster 4 (Q-CD-11) commits **REBATES DEFERRED AT LAUNCH**: `PFV.rebateReserve(asset) = 0`; all active FeesManagerV2 profiles MUST be effective non-negative.
- Cluster 4 launch invariant verifier MUST be implemented and run pre-broadcast.
- Spec source: `deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8` (33-line `should_broadcast` pseudocode).
- Source verification at task kickoff: `grep -rn 'fn should_broadcast\|should_broadcast(' deopt-v2-backend/src/` MUST return **0 hits** (gate's reason for existing).
- Existing backend already has working option-execution + RFQ Sepolia broadcast path. **Build adds the policy gate; does NOT rewrite the broadcast path.**

### Hard stops (this task)

```text
no mainnet broadcast                                ✅
no chain tx by the implementer                      ✅
no Safe tx                                          ✅
no `.env` edit by the implementer                   ✅
no PFV withdrawal / rebate reserve allocation       ✅
no ownership / guardian / Timelock mutation         ✅
no Treasury Safe creation                           ✅
no KMS key creation                                 ✅
no `EXECUTOR_PRIVATE_KEY` value / RPC URL / DATABASE_URL / admin token printed in PR description or commit message ✅
no production secrets in tests (use fixtures only)  ✅
no `--no-verify` git flags                          ✅
no governance mutation                              ✅
no live trade construction outside Sepolia rehearsal harness ✅
```

If a step requires any of the above, STOP and document the blocker for operator review.

### Goal

Implement the backend `should_broadcast` economic policy gate per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8`, including the Cluster 4 launch invariant verifier sweep. Provide unit + integration + regression tests. Do NOT touch the existing Sepolia broadcast path's correctness — only add the gate as a pre-broadcast decision.

### Required Phase A — read the spec + source

1. Read end-to-end:
   - `deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md` (full doc; particularly §3 fee/rebate model, §4 backend P&L model, §6 anti-griefing, §8 `should_broadcast` pseudocode, §9 Sepolia parameters, §10 implementation TODOs).
   - `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md` §2 (rebates DEFERRED at launch + §2.3 launch invariant + §2.4 future activation gate).
   - `deopt-v2-backend/docs/PREBUILD_TO_BUILD_HANDOFF.md` §5.1 + §6.4 (acceptance criteria + Cluster 4 hard stop).
   - `deopt-v2-backend/docs/BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md` §1.1 + §2.3 (this task + acceptance tests).

2. Inspect (read-only) the current broadcast path:
   - `src/options/service.rs` — option execution intent lifecycle; signing; broadcast (around line 1166 / 1213 swap points).
   - `src/options/execution.rs` — `validate_broadcast_intent` (lines around 534-543); `option_execution_broadcast_gas_limit` (lines around 562-576).
   - `src/execution/signer.rs` — signing.
   - `src/execution/executor.rs` — perp scaffold hard-stop (lines 54-58).
   - `src/execution/config.rs` — `EXECUTOR_PRIVATE_KEY` guard (line 118).
   - `src/config/env.rs` — env config loader (lines 70-72 for `EXECUTOR_PRIVATE_KEY`; line 562 for `OPTION_EXECUTION_BROADCAST_ENABLED` gate).
   - `src/fees/onchain_summary.rs` + `src/fees/vault_observability.rs` — read-only fee + R5 invariant probes.
   - `src/db/repository.rs` — `execution_intents` vs `option_execution_intents` separation.

3. Confirm gap (must be true at task kickoff):
   ```
   grep -rn 'fn should_broadcast\|should_broadcast(' deopt-v2-backend/src/
   ```
   MUST return 0 hits. If non-zero, the gate already exists; review existing code and reconcile with §8 spec rather than re-implement.

### Required Phase B — design

4. Create a brief design note at `deopt-v2-backend/docs/SHOULD_BROADCAST_DESIGN_NOTE.md` (≤ 2 pages) covering:
   - Where the gate plugs in (recommend: new fn `should_broadcast(...)` in new module `src/options/broadcast_policy.rs`; called from `src/options/service.rs` immediately before the existing broadcast call site).
   - Signature: `fn should_broadcast(intent: &OptionExecutionIntent, context: &BroadcastContext) -> ShouldBroadcastDecision`.
   - `ShouldBroadcastDecision = { Approve { reason: ApprovalReason }, Reject { reason: RejectReason, details: String } }` per §8 reason enum.
   - The Cluster 4 launch invariant verifier as a separate sub-function callable independently for the POST-Y-G-6 sweep.
   - Mode awareness: behaviour differs between Sepolia and mainnet (e.g. mainnet refuses negative effective ppm AND rebateReserve=0; Sepolia warns but proceeds for fee-only intents).
   - Integration with `OPTION_EXECUTION_BROADCAST_ENABLED` env gate (does not replace; precedes).
   - No source modification in this Phase.

### Required Phase C — implement (Sepolia-safe only)

5. Add new module `src/options/broadcast_policy.rs`.

6. Implement the §8 pseudocode branches in order, as separate guard fns where it improves testability:
   - **§8 step 0 — Pre-flight static checks:** `dedupe_cache_has`, `eip712_verify(buyer_sig)`, `eip712_verify(seller_sig)`, `deadline_in_future`, `nonces_unconsumed`, `product_listed`, `rm_snapshot_fresh`.
   - **§8 step 1 — NEW_OME live state:** `new_ome.paused() == false`, `new_ome.is_executor(BE) == true`, `BE.balance >= FUND_FLOOR`.
   - **§8 step 2 — Margin / product guards:** `rm.has_margin(buyer)`, `rm.has_margin(seller)`.
   - **§8 step 3 — Simulate:** call existing simulator; reject on revert; reject on `sim.gas_units > HARD_GAS_CAP`.
   - **§8 step 4 — Fee / rebate computation:** `compute_fee_split(order)` mirroring `FM_V2._quoteFees`; reject on zero economic content.
   - **§8 step 5 — Rebate solvency (HARD GATE — Cluster 4 launch invariant primary teeth):**
     ```
     if total_rebate_outflow > 0:
       if FM_V2.rebate_budget(asset) < total_rebate_outflow: reject "rebate-budget"
       if PFV.rebate_reserve(asset) < total_rebate_outflow: reject "rebate-reserve"
     ```
     This is the chain-side backstop assertion mirrored at the backend boundary. **Cluster 4 launch state (rebateReserve = 0) means any rebate-positive candidate REJECTS here.**
   - **§8 step 6 — Anti-griefing:** `same_beneficial_owner`, `maker_rebate_quota_breached`, `recent_attack_pattern` (start with simple address-equality wash check; advanced heuristics can land in a separate PR).
   - **§8 step 7 — Gas cost in asset terms:** oracle quote + `total_gas_wei × eth_price / 1e18`; `expected_pnl = net_protocol_revenue - gas_cost_in_asset`.
   - **§8 step 8 — Liquidation carve-out:** only if `order.is_liquidation`; out of scope for this build task (liquidation not in launch scope per Q-CD-12 default Option A; mark as `unimplemented!("liquidation carve-out — gap-list C-13/C-14")` with TODO comment).
   - **§8 step 9 — Normal decision states:** `PROFITABLE` (expected_pnl ≥ PNL_FLOOR) / `AT-COST` / `SUBSIDISABLE` / `REJECT`.

7. Add Cluster 4 **launch invariant verifier** as a separate fn `verify_launch_invariant(ome, fm, pfv, asset) -> LaunchInvariantReport`:
   - Reads each active fee profile via `FM_V2.getProfile()` for every `(tier, product, ORDERBOOK|RFQ)` combination.
   - Computes effective `(makerPpm, takerPpm)` after RFQ discount.
   - Reports per-profile sign flags.
   - Reads `PFV.rebateReserve(asset)`.
   - Returns:
     ```
     LaunchInvariantReport = {
       all_profiles_non_negative: bool,
       profiles: Vec<{tier, product, flow, effective_maker_ppm, effective_taker_ppm, non_negative: bool}>,
       rebate_reserve: u256,
       rebate_reserve_zero: bool,
       overall_pass: bool   // = all_profiles_non_negative AND rebate_reserve_zero (on mainnet)
     }
     ```
   - Used by:
     - `should_broadcast` (step 5 backstop).
     - POST-Y-G-6 operator-side sweep (exposed via a debug / admin route or CLI tool — operator-only; do not expose on public API).

8. Wire `should_broadcast` call site immediately before the existing broadcast call in `src/options/service.rs`:
   - Decision = Approve → proceed to existing broadcast path.
   - Decision = Reject → update intent status to `broadcast_rejected` (new status if needed) or `broadcast_failed` with reason; log structured event per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §7.1`.
   - Preserve current intent state-machine; do NOT change `broadcast_submitted` / `broadcast_confirmed` semantics.

9. Add startup invariant: on startup, if `chain_id == 8453` (mainnet) AND `OPTION_EXECUTION_BROADCAST_ENABLED == true` AND `verify_launch_invariant(...) -> overall_pass == false`, REFUSE to start; print structured error (no secrets); exit non-zero.

### Required Phase D — tests

10. Unit tests in `src/options/broadcast_policy.rs::tests`:
    - **Test 1:** all approvals — happy path; mocked context returns Approve(Profitable).
    - **Test 2:** wash detection — same buyer/seller address rejects with "wash".
    - **Test 3:** sim revert — Reject "sim:<err>".
    - **Test 4:** gas cap — `sim.gas_units > HARD_GAS_CAP` rejects "gas-cap".
    - **Test 5:** zero economic content — gross_fee_revenue == 0 AND total_rebate_outflow == 0 rejects "no-econ-content".
    - **Test 6:** **rebate-solvency hard gate (Cluster 4 primary teeth):**
       - Candidate with `total_rebate_outflow > 0` AND `pfv.rebate_reserve(asset) == 0` rejects "rebate-reserve".
       - Candidate with `total_rebate_outflow > 0` AND `fm_v2.rebate_budget(asset) < total_rebate_outflow` rejects "rebate-budget".
    - **Test 7:** PFV.rebateReserve != 0 + fee-only profiles — Approve (rebate solvency not exercised; backstop nonetheless consulted).
    - **Test 8:** BE_BAL_LOW — `BE.balance < FUND_FLOOR` rejects "be-low-bal".
    - **Test 9:** OME paused — `NEW_OME.paused() == true` rejects "ome-paused".
    - **Test 10:** OME isExecutor false — `NEW_OME.isExecutor(BE) == false` rejects "be-not-exec".
    - **Test 11:** expired deadline — rejects "expired".
    - **Test 12:** stale RM snapshot — rejects "stale-rm".
    - **Test 13:** dedupe cache hit — rejects "dupe".
    - **Test 14:** at-cost — `expected_pnl < PNL_FLOOR` but `net_protocol_revenue >= gas_cost × SAFETY_MARGIN` → Approve(AtCost).
    - **Test 15:** subsidisable — gap absorbed by subsidy budget → Approve(Subsidy(reason)).

11. Unit tests for `verify_launch_invariant`:
    - **Test 16:** all profiles effective non-negative AND rebateReserve == 0 → overall_pass = true.
    - **Test 17:** one profile effective negative → overall_pass = false; per-profile flags identify which.
    - **Test 18:** rebateReserve > 0 on mainnet (Cluster 4 violation) → overall_pass = false.
    - **Test 19:** RFQ discount edge: makerPpm = 50, makerDiscountPpm = 1_000_001 → reduce makerPpm but stay non-negative (clamp); flag if implementation produces negative.

12. Integration tests:
    - **Test 20:** Sepolia end-to-end fee-only orderbook intent through `should_broadcast` → simulation → broadcast → confirmation; matches `FIRST_LIVE_OPTION_EXECUTION_SMOKE_RESULT_V2_SEPOLIA.md` regression expectations (within recompute tolerance; gas may differ by ≤ 1%).
    - **Test 21:** Sepolia end-to-end fee-only RFQ intent — same shape; matches `FIRST_LIVE_RFQ_OPTION_EXECUTION_SMOKE_RESULT_SEPOLIA.md` expectations.

13. Regression tests (no new broadcasts):
    - **Test 22:** existing broadcast path with `should_broadcast` returning Approve preserves the current intent state-machine (status transitions unchanged).
    - **Test 23:** existing broadcast path with `should_broadcast` returning Reject transitions to `broadcast_rejected` or `broadcast_failed` cleanly; no half-state.

14. Run validation locally:
    ```
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets --all-features --no-fail-fast
    ```
    All MUST be green.

15. Run forge validation:
    ```
    cd ~/DEOPT/deopt-v2-sol
    forge fmt --check
    forge build
    forge test --no-match-path 'test/fork/*'
    ```
    All MUST be green (no source touched in sol repo by this task).

### Required Phase E — PR + close-out

16. Open a PR titled `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE — implement §8 + Cluster 4 launch invariant verifier`.
17. PR description references:
    - `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8` (spec).
    - `MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3` (launch invariant).
    - `BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md §1.1` (acceptance criteria).
    - This NEXT_TASK file (origin).
18. PR description includes a "what this PR does NOT do" section confirming:
    - No mainnet broadcast attempted.
    - No `.env` edited.
    - No KMS / vendor / Treasury Safe creation.
    - No ownership / guardian / Timelock mutation.
    - No `EXECUTOR_PRIVATE_KEY` / RPC / DATABASE_URL / admin token in code / tests / PR description.
19. After review, merge to main (operator-authorised).

### Final report shape

Return final report grouped by:
- workspace
- docs / source inspected
- gap confirmation grep result (must be 0 hits at start)
- new module path
- launch invariant verifier path
- unit tests added
- integration tests added
- regression tests result
- forge + cargo validation results
- PR title + URL
- files touched
- validations
- blockers
- next milestone recommendation

---

## Prompt (end)

---

## Notes for the operator handing this prompt off

- This task is **expected to take ~1-2 weeks of focused implementation** + ~1 week of test hardening + review.
- It is the **first build task** in the prebuild → build handoff sequence.
- It produces the launch invariant verifier required by AUDIT-EXT Q-34 (Cluster 4) — auditor's earliest concrete attestation point on the backend side.
- It also closes gap-list C-4 / W-3 (the longest-standing P1 from the readiness gap list).
- After this task closes, the next recommended task is `MAINNET-BE-SIGNER-SERVICE-DESIGN` (read-only design milestone) per `PREBUILD_TO_BUILD_HANDOFF.md §4`.

## Cross-links

- `deopt-v2-backend/docs/PREBUILD_TO_BUILD_HANDOFF.md`
- `deopt-v2-backend/docs/BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md`
- `deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md`
- `deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md`
- `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md`
- `~/DEOPT/MAINNET_CUSTODY_POLICY.md`
- `~/DEOPT/RUN_STATE.md`

**End of NEXT_TASK prompt stub.**
