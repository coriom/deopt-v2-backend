# E2E Sepolia — Resolved Values Checklist (M-P5 Phase A Follow-up)

**Date:** 2026-06-10
**Posture:** **public-safe summary only.** Status flags only — no
addresses, balances, RPC URLs, or signer values.

## 1. Blocker status (read-only confirmation)

| Blocker | Status (last updated 2026-06-10 by M-P5-RO) | Confirming source |
|---|---|---|
| BS-1 OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS | **CLOSED** — derived from abstract-inheritance discovery; equals OPTION_COLLATERAL_VAULT_ADDRESS | `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` §3 (M-P5-FIXES) |
| BS-2 OPTION_MARGIN_ENGINE_LENS_ADDRESS | **OPEN — DEPLOYMENT_REQUIRED** (M-P5-RO2: lens address still EMPTY in private file; no Sepolia deploy known) | `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` §4 |
| BS-3 Executor authorisation | **OPEN — GOVERNANCE_ACTION_REQUIRED** (M-P5-RO2: `isExecutor==false` confirmed via on-chain `cast call`) | `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` §5 |
| BS-4 Buyer / seller testnet balances | **OPEN — LOW_BALANCE + LOW_ALLOWANCE** (M-P5-RO2: all 6 buckets fail — buyer/seller ETH + mUSDC balance + 4 allowances) | `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` §6 |
| BS-5 Active series + live oracle feed | **OPEN — ORACLE_FEED_MISSING** (M-P5-RO2: 2 series exist + `hasActiveFeed==true` but `getPriceSafe==0` on both — mock oracle stale) | `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` §7 |

## 2. Decision summary

Of the five blockers identified in M-P5 Phase A:

* **1 closed** by source-tree analysis (BS-1 — no on-chain check needed).
* **4 remain OPEN** — each requires the operator to run the
  read-only confirmation commands in
  `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` against their private
  `.env.sepolia` and operator-side notes.

**Phase B (live Sepolia broadcast) is NOT yet ready.** It cannot
proceed until BS-2, BS-3, BS-4, BS-5 all report CONFIRMED / CLOSED
in this checklist.

## 3. How an operator updates this checklist

After completing the read-only confirmation runs, the operator
replaces the "OPEN — OPERATOR_*_CHECK_REQUIRED" status with one of:

* **CLOSED** (no action needed; e.g. BS-1).
* **CONFIRMED** (operator ran the read-only check; result was the
  expected success value).
* **OPEN — OPERATOR_ACTION_REQUIRED** (operator ran the check; the
  result requires a separate governance / faucet / deploy task —
  reference the exact follow-up doc).

The operator MUST NOT write actual addresses, balances, or RPC
values into this file. The status flag is sufficient.

## 4. Public / private split

| Type of value | Lives in |
|---|---|
| Status flag (CLOSED / CONFIRMED / OPEN) | this checklist (public) |
| Sepolia address values | `~/DEOPT/operator-private/` + backend `.env.sepolia` |
| RPC URL | `.env.sepolia` (NEVER public) |
| Balances | operator-side notes (NEVER public) |
| Private keys | NEVER in any file — KMS / hardware wallet only |

## 5. Cross-links

* `E2E_SEPOLIA_FIXES_RESULT.md`
* `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md`
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md`
* `E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`

**End of checklist.**
