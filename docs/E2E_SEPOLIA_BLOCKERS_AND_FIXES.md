# E2E Sepolia — Blockers & Fixes (M-P5 Phase A)

**Date:** 2026-06-10
**Posture:** docs-only inventory of what blocks Phase B (live
Sepolia broadcast) + concrete remediation plan.
**Forbidden:** no mainnet, no production `.env` edit, no AWS
resource creation, no real funds movement beyond a future
explicitly-approved Sepolia test broadcast.

## Blockers

> **Cross-link (M-P5-RO + M-P5-PROV, 2026-06-10):** read-only
> confirmations against BS-2 / BS-3 / BS-4 / BS-5 could NOT be
> performed in either run (no operator-supplied RPC URL or
> addresses in either environment). M-P5-PROV added
> `.gitignore` safeguards for `operator-private/`, `*.private.md`,
> `*.private.env`, and `.env.sepolia*` in all 3 repos so the
> operator can safely populate private inputs. Per-blocker fix
> briefs added:
> * `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md` (BS-2)
> * `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md` (BS-3 if false)
> * `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md` (BS-4)
> * `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` (BS-5)
>
> The 4 rows below remain accurate; the playbook in
> `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` is unchanged.

> **Cross-link (M-P5-RO2, 2026-06-10):** on-chain read-only checks
> NOW EXECUTED via the populated private operator input file. All
> 4 OPEN rows refined to actionable sub-states:
> * BS-2 → **DEPLOYMENT_REQUIRED** (lens address still empty)
> * BS-3 → **GOVERNANCE_ACTION_REQUIRED** (`isExecutor==false` on chain)
> * BS-4 → **LOW_BALANCE + LOW_ALLOWANCE** (all 6 buckets failing)
> * BS-5 → **ORACLE_FEED_MISSING** (2 series exist; feeds registered;
>   `getPriceSafe==0` on both — mock oracle is stale)
>
> See `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md`.

> **Cross-link (SETUP-FIXES-PACK-PREFLIGHT, 2026-06-11):** the 4 BS
> fixes are now grouped into one approval-gated execution pack.
> Preflight + per-step command templates:
> `SEPOLIA_SETUP_FIXES_PACK_PREFLIGHT_RESULT.md`. Execution prompt
> (literal approval line required):
> `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md`. Order:
> BS-5 → BS-3 → BS-4 → BS-2. Each blocker still has its own
> standalone `SEPOLIA-*_NEXT_TASK.md` brief; the pack is a
> convenience grouping, not a replacement.

> **Cross-link (SETUP-FIXES-PACK-EXECUTION, 2026-06-12):** the pack
> **EXECUTED SUCCESSFULLY** with the operator approval line. **All
> 4 BS rows now CLOSED on chain:**
> * BS-2 → **CLOSED** (lens deployed at `0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26`)
> * BS-3 → **CLOSED** (`isExecutor==true`)
> * BS-4 → **CLOSED** (buyer+seller hold 100k mUSDC + allowance to CollateralVault)
> * BS-5 → **CLOSED** (`getPriceSafe` nonzero + `ok=true`; 60s maxDelay caveat documented)
>
> 10 successful on-chain txs; live broadcast gate now **READY FOR
> OPERATOR APPROVAL**. See `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md`.

### BS-1 — `OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS unknown` — **CLOSED (M-P5-FIXES, 2026-06-10)**

`CollateralVaultViews` is an `abstract contract` inherited by the
concrete `CollateralVault`. The deployed Sepolia address for both
backend env keys is the same:
`0x00340C360353a5AB784c5Bc5c44322A6AF0625D3`. See
`E2E_SEPOLIA_FIXES_RESULT.md` §4.

### BS-1 (original analysis below — kept for historical reference)

**Symptom:** No checked-in sol/docs surface a Sepolia address for the
`CollateralVaultViews` contract. Without it, the backend's
`account_balances` endpoint returns `status: "partial"` with
`CONFIG_MISSING` warning when pointed at Sepolia.

**Impact:** UI renders balances as empty + amber notice. Trade
ticket still works (collateral display is informational only).
Phase B can proceed; UX is degraded.

**Fix:** Operator confirms the Sepolia
`CollateralVaultViews` address from the deployment artefact (likely
in a private notes file) and adds it to `.env.sepolia` as
`OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS`. **Do not invent or guess
the address.**

### BS-2 — `OPTION_MARGIN_ENGINE_LENS_ADDRESS unknown` (Medium)

**Symptom:** No checked-in sol/docs surface a Sepolia address for
`MarginEngineLens`. Without it,
`account_portfolio` returns `status: "partial"` with
`ACCOUNT_STATE_UNAVAILABLE` warning; `exercise_preview` returns
`status: "partial"` with `SETTLEMENT_PREVIEW_UNAVAILABLE`.

**Impact:** UI renders portfolio + exercise-preview with
"approximate" badges. Trade ticket still works. Phase B can
proceed; UX is degraded.

**Fix:** Same as BS-1 — operator-confirmed address goes in
`.env.sepolia`.

### BS-3 — Executor authorisation at OPTION_MATCHING_ENGINE unconfirmed (High)

**Symptom:** Phase B will broadcast `executeTrade(...)` via the
`OptionMatchingEngine.execute()` entry point. That entry point
checks `isExecutor(msg.sender)` and reverts when the backend
executor (`0xc35F7A8A103A9A4464adfaa76B9B514093D23C27`) is not
authorised.

**Impact:** Single Sepolia broadcast WILL revert without
authorisation. Wasted gas; broadcast attempt visible on-chain.

**Fix:** Operator runs:
```
cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" "$EXECUTOR_ADDRESS" \
  --rpc-url "$EXECUTION_RPC_URL"
```
and confirms `true` BEFORE Phase B. If `false`, run the existing
sol script `script/SetMatchingEngineExecutor.s.sol` (operator-side
governance action — not part of M-P5).

### BS-4 — Buyer / seller testnet collateral not confirmed (High)

**Symptom:** The trade will route through `CollateralVault` debit /
credit at settlement. If buyer (long) lacks testnet ETH for gas OR
seller (short) lacks the collateral for the option size, the
`executeTrade` call reverts in the settlement step.

**Impact:** Same as BS-3 — broadcast reverts; gas wasted.

**Fix:** Operator confirms both accounts hold sufficient testnet
collateral. The existing
`script/PrepareSmokeV1Accounts.s.sol` does the prep on Sepolia
(no mainnet path). Document the actual balances in the Phase B
sign-off.

### BS-5 — Active series + live oracle feed not confirmed (High)

**Symptom:** The series the operator selects must be `Active` in
the backend store AND have an active oracle feed via
`OracleRouter.hasActiveFeed(underlying, settlement)`. The
backend's quote preview endpoint returns `partial` warnings if not.

**Impact:** Create-intent will succeed (series existence checked)
but quote preview shows amber warnings; signing-payload computes
based on the supplied price; on-chain execution may revert if the
settlement decimals or oracle state is unexpected.

**Fix:** Operator selects a series with confirmed-active oracle.
Confirm via:
```
curl "http://localhost:8080/options/series/<series_id>/details" \
  | jq '.status, .data.oracle_mark_1e8, .warnings'
```
Expected: `status == "ok"` with a non-null `oracle_mark_1e8` value.

## Severity matrix

| Blocker | Severity | Blocks Phase A? | Blocks Phase B? |
|---|---|---|---|
| BS-1 OPTION_COLLATERAL_VAULT_VIEWS unknown | ~~Medium~~ | ~~NO (partial OK)~~ | **CLOSED (M-P5-FIXES)** |
| BS-2 OPTION_MARGIN_ENGINE_LENS unknown | Medium | NO (partial OK) | NO (UX degraded only) |
| BS-3 Executor auth unconfirmed | High | NO | **YES — must close** |
| BS-4 Collateral prefunding unconfirmed | High | NO | **YES — must close** |
| BS-5 Active series + live oracle unconfirmed | High | NO | **YES — must close** |

## Order-of-operations recommendation

1. **Phase A — this milestone** — dry-run / read-only verification.
   Closed cleanly; no source code changes; 1203 backend tests green;
   frontend tsc/eslint/build green; 21 Playwright specs.
2. **`E2E_SEPOLIA_FIXES_NEXT_TASK.md`** — close BS-1 / BS-2 (soft;
   operator address input) + confirm BS-3 / BS-4 / BS-5 (hard;
   operator on-chain checks).
3. **`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`** — operator sign-off
   template; one approval per broadcast.
4. **Phase B — single Sepolia broadcast** — gated by §3 sign-off;
   one approved broadcast; recorded in
   `E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md` (future task).

## Cross-links

* `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (this milestone)
* `E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_FIXES_NEXT_TASK.md`
* `BACKEND_TRADING_API_PHASE_5_RESULT.md` (M-P2e — read endpoints)
* `BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md` (M-P2f)
* `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md` (live
  broadcast safety — existing)
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of blockers doc.**
