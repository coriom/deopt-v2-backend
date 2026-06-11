# E2E Sepolia Fixes — Next Task (M-P5 follow-on)

**Date written:** 2026-06-10
**Origin:** M-P5 Phase A (`E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md`)
identified 5 blockers (BS-1 … BS-5) preventing Phase B.
**Target:** close all blockers + produce a signed-off
`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` instance.
**Posture:** **No mainnet. No live tx yet. No production `.env`
edit. No AWS resource creation. No real fund movement beyond
operator-side testnet pre-funding (operator-side action, not part of
this milestone's automation).**

---

## 1. Scope

This is a **docs-and-config** task. No backend source code changes
are required; no frontend source code changes are required. The
deliverables are:

1. Operator-supplied addresses for BS-1 / BS-2 (recorded in a
   private operator-side file; placeholders only in checked-in
   docs).
2. Operator-side confirmation runs for BS-3 / BS-4 / BS-5 (results
   recorded as `OPERATOR_CONFIRMED` in the blockers doc).
3. A completed approval-gate instance.

## 2. Forbidden

* No Sepolia broadcast in this task — Phase B is the broadcast
  task, gated separately.
* No mainnet RPC, no mainnet config, no Safe tx, no AWS / KMS
  creation.
* No editing of production `.env`. Operator-supplied addresses go
  in `.env.sepolia` (NOT `.env`).
* No invented Sepolia addresses. If an address is unknown, mark it
  `OPERATOR_INPUT_REQUIRED`.
* No checked-in private keys, RPC secrets, KMS ARNs, or AWS account
  IDs.

## 3. Tasks

### 3.1 Close BS-1: OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS

* Operator looks up the Sepolia deployment address of
  `CollateralVaultViews` from the private deployment notes.
* Operator adds the address to `.env.sepolia` as
  `OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS`.
* Verify via `cast code` that the address has non-empty bytecode.
* Update `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` BS-1 row → CLOSED.

### 3.2 Close BS-2: OPTION_MARGIN_ENGINE_LENS_ADDRESS

* Same pattern as BS-1, for `MarginEngineLens` →
  `OPTION_MARGIN_ENGINE_LENS_ADDRESS`.

### 3.3 Confirm BS-3: executor authorisation

```
cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" \
  "0xc35F7A8A103A9A4464adfaa76B9B514093D23C27" \
  --rpc-url "$EXECUTION_RPC_URL"
```

* If `true` → BS-3 CONFIRMED.
* If `false` → operator runs the existing
  `script/SetMatchingEngineExecutor.s.sol` (governance action;
  separate sign-off; NOT part of M-P5 automation).

### 3.4 Confirm BS-4: testnet collateral pre-funding

Operator runs `script/PrepareSmokeV1Accounts.s.sol` on Sepolia and
records the resulting balances:

* Test buyer ETH balance ≥ 0.01 ETH.
* Test seller ETH balance ≥ 0.01 ETH.
* Test seller collateral balance ≥ size_1e8 × strike_1e8 × bps_buffer
  (operator computes the exact figure from the chosen series).

Update BS-4 → CONFIRMED with the recorded balances (in a
private/operator-only file; placeholders in the checked-in doc).

### 3.5 Confirm BS-5: active series + live oracle

```
# Operator starts the backend on Sepolia.
# Operator picks a series_id from the existing seeded set.
curl http://localhost:8080/options/series/<series_id>/details \
  | jq '.status, .data.oracle_mark_1e8, .warnings'
```

* `status == "ok"` + `oracle_mark_1e8 != null` → BS-5 CONFIRMED.
* Otherwise → operator creates / activates a series via the
  existing operator-side `POST /options/series` admin endpoint and
  refreshes the oracle feed.

## 4. Acceptance criteria

* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` severity matrix shows all
  BS-1 … BS-5 rows = CLOSED or CONFIRMED.
* An instance of `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` is filled in
  by the operator with:
  - All P1 … P12 preconditions ticked.
  - The literal approval line written verbatim.
  - The expected tx count, gas cap, rollback criteria reviewed.

## 5. Cross-links

* `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (M-P5 Phase A)
* `E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of fixes next-task brief.**
