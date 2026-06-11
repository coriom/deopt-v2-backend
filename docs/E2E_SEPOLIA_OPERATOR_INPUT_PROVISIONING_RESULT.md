# E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT (M-P5-PROV)

**Date:** 2026-06-10
**Milestone:** `SEPOLIA-OPERATOR-INPUT-PROVISIONING-AND-READONLY-CHECKS`.
**Scope:** Phase A (input discovery) + Phase B (gitignore
safeguards) + Phase I docs.
**Posture:** **Docs + .gitignore only. No mainnet. No Sepolia
broadcast. No `.env` edit. No source code changes. No private
values committed.**

## 1. Purpose

Combine private-operator-input discovery, .gitignore safeguards
against accidental private-file commits, the BS-2/3/4/5 read-only
confirmation pass, and the targeted next-task generation for any
remaining blockers.

## 2. Repos / docs inspected

* `deopt-v2-backend/docs/E2E_SEPOLIA_{READ_ONLY_CONFIRMATIONS_RESULT,FIXES_RESULT,OPERATOR_INPUT_TEMPLATE,READ_ONLY_CONFIRMATION_LOG,RESOLVED_VALUES_CHECKLIST,REMAINING_OPERATOR_ACTIONS,BLOCKERS_AND_FIXES,LIVE_APPROVAL_GATE,LIVE_BROADCAST_NEXT_TASK}.md`
* `deopt-v2-backend/docs/{BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT,BACKEND_TRADING_API_PHASE_5_RESULT}.md`
* `deopt-v2-sol/docs/SOL_BACKEND_FRONTEND_ABI_HANDOFF.md`
* `deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt`
* `~/DEOPT/TESTNET_RUNBOOK.md`, `~/DEOPT/RUN_STATE.md`
* All three `.gitignore` files (backend, sol, frontend).

## 3. Operator input provisioning

Presence-only scan of candidate operator-private locations + env
vars (no values printed, no private files opened):

| Source | Status |
|---|---|
| `~/DEOPT/operator-private/sepolia.inputs.private.md` | MISSING |
| `~/DEOPT/operator-private/*.private.md` | MISSING (directory does not exist) |
| `~/DEOPT/*.private.md` | MISSING (no Sepolia-relevant file) |
| Shell env `BASE_SEPOLIA_RPC_URL` | MISSING |
| Shell env `EXECUTION_RPC_URL` | MISSING |
| Shell env `RPC_URL` | MISSING |
| Shell env `SEPOLIA_RPC_URL` | MISSING |
| Shell env `OPTION_MARGIN_ENGINE_LENS_ADDRESS` | MISSING |
| Shell env `BUYER_ADDRESS` / `SELLER_ADDRESS` / `EXECUTOR_ADDRESS` / `COLLATERAL_TOKEN` / `ACTIVE_OPTION_SERIES_ID` | ALL MISSING |
| `~/DEOPT/private/mainnet_custody/*.private.md` | PRESENT but **out of scope** (mainnet-only; NOT read) |
| `cast` binary | PRESENT |
| `curl` binary | PRESENT |

**Conclusion: the operator-private input file does NOT yet exist
in this environment.** A new public-safe template was created at
`E2E_SEPOLIA_OPERATOR_INPUT_PRIVATE_TEMPLATE.md` that instructs the
operator to copy + populate it under `~/DEOPT/operator-private/`
(which is now `.gitignore`-protected).

## 4. Private input safeguards

`.gitignore` patterns added to **all three repos** so an
accidentally-committed private input file would be ignored:

| Repo | Patterns added |
|---|---|
| `deopt-v2-backend/.gitignore` | `operator-private/`, `*.private.md`, `*.private.env`, `.env.sepolia`, `.env.sepolia.local`, `.env.sepolia.private` |
| `deopt-v2-sol/.gitignore` | `operator-private/`, `*.private.md`, `*.private.env`, `.env.sepolia.local`, `.env.sepolia.private` |
| `deopt-v2-frontend/.gitignore` | `operator-private/`, `*.private.md`, `*.private.env` |

The sol `.gitignore` already covered `.env.sepolia` via its
`.env.*` wildcard + explicit `.env.base-sepolia`; we add the
`.private` and `operator-private/` patterns on top.

**No actual private file was committed.** **No `.env` edits
performed.** The `.gitignore` additions are the only source-tree
edits in this milestone.

## 5. Read-only checks — were any run?

**NONE RUN.** Same outcome as M-P5-RO: the environment has no
`EXECUTION_RPC_URL` and no operator-supplied private addresses, so
no `cast call` could be issued without inventing values.

The brief's documented fallback applies: produce an honest
"NOT_RUN — no operator inputs" outcome and prescribe the precise
next steps for the operator.

## 6. BS-2 MarginEngineLens confirmation

**Status: OPEN — OPERATOR_INPUT_REQUIRED (not run).**

Identical outcome to M-P5-RO §5. Close-path remains: operator
supplies `OPTION_MARGIN_ENGINE_LENS_ADDRESS` in the private file +
`EXECUTION_RPC_URL`, then re-runs this milestone OR runs
`SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY` if no Sepolia deploy exists.

## 7. BS-3 executor authorization

**Status: OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (not run).**

Single call required:

```bash
cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "isExecutor(address)(bool)" \
  0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 \
  --rpc-url "$EXECUTION_RPC_URL"
```

Not run because `EXECUTION_RPC_URL` is absent. Close-path:
operator runs the call OR runs `SEPOLIA-EXECUTOR-AUTH-GRANT` if
the result is `false`.

## 8. BS-4 buyer/seller balances and allowances

**Status: OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (not run).**

Identical to M-P5-RO §7. Close-path: operator runs the 5 read-only
`cast` calls (3 `balanceOf`, 1 `allowance`, plus `cast balance`
for ETH) OR runs `SEPOLIA-BUYER-SELLER-FUNDING` if LOW_BALANCE /
LOW_ALLOWANCE.

## 9. BS-5 active series and oracle feed

**Status: OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (not run).**

Identical to M-P5-RO §8. Close-path: operator runs the bounded
`totalSeries → seriesAt → getSeries → hasActiveFeed → getPriceSafe`
walk OR runs `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP` if no Active
series with a live feed exists.

## 10. Backend read-only API smoke

**Not run.** Backend smoke requires the live backend on Sepolia +
identified series + funded accounts — none of those preconditions
met. The exact curl playbook lives in
`E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md` §3-§6, unchanged.

## 11. Live approval gate status

**GATE NOT MET.** P1 (checklist all CLOSED/CONFIRMED) fails — 4
blockers OPEN. The literal approval line cannot be signed. Gate
doc + broadcast-next-task brief annotated by M-P5-RO already.

## 12. Targeted next-task docs

Four targeted briefs created so the operator can pick the matching
fix path once a read-only check returns a failing value:

| Doc | Trigger | Posture |
|---|---|---|
| `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md` | BS-2 lens has no Sepolia deployment | state-mutating; operator + governance |
| `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md` | BS-3 `isExecutor(executor) == false` | state-mutating; governance |
| `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md` | BS-4 LOW_BALANCE / LOW_ALLOWANCE | operator faucet + approve |
| `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` | BS-5 SERIES_MISSING / ORACLE_FEED_MISSING | state-mutating; governance |

All four are documented as separate state-mutating tasks. **None
runs in this milestone.**

## 13. Docs created / updated

| Path | Status |
|---|---|
| `docs/E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md` | new (this doc) |
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_EXECUTION_RESULT.md` | new (companion "exec=skipped" doc) |
| `docs/E2E_SEPOLIA_OPERATOR_INPUT_PRIVATE_TEMPLATE.md` | new (placeholders only) |
| `docs/SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md` | new |
| `docs/SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md` | new |
| `docs/SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md` | new |
| `docs/SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` | new |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | edited (M-P5-PROV source links) |
| `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` | edited (M-P5-PROV cross-link banner) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (status banner refreshed) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` | edited (status banner refreshed) |
| `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` | edited (M-P5-PROV cross-link) |
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` | edited (cross-link to new docs) |
| `deopt-v2-backend/.gitignore` | edited (private patterns) |
| `deopt-v2-sol/.gitignore` | edited (private patterns) |
| `deopt-v2-frontend/.gitignore` | edited (private patterns) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |

## 14. RUN_STATE update

M-P5-PROV closure paragraph prepended above M-P5-RO.

## 15. Files changed

**Source code: ZERO.** Only `.gitignore` (3 repos) + docs.

## 16. Validations

| Check | Result |
|---|---|
| `git diff --check` | clean |
| `git status --short` | only intended files (3 .gitignore + 7 new docs + 6 edited docs + RUN_STATE) |
| Sensitive-string scan on all new + edited docs | zero hits |
| `.env` modified? | NO — backend `.env` timestamp preserved (`Jun 8 16:55`) |
| Backend source changed? | NO |
| Frontend source changed? | NO |
| OpenAPI changed? | NO |
| Private value committed? | NO |
| Address invented? | NO |
| RPC URL printed? | NO (presence-only) |

## 17. Blockers (post-milestone)

| Blocker | Status |
|---|---|
| BS-1 OPTION_COLLATERAL_VAULT_VIEWS unknown | CLOSED (M-P5-FIXES) |
| BS-2 OPTION_MARGIN_ENGINE_LENS unknown | OPEN — OPERATOR_INPUT_REQUIRED |
| BS-3 Executor authorisation unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |
| BS-4 Buyer/seller balances unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |
| BS-5 Active series + oracle feed unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |

## 18. Next milestone recommendation

Operator path:

1. Copy `E2E_SEPOLIA_OPERATOR_INPUT_PRIVATE_TEMPLATE.md` to
   `~/DEOPT/operator-private/sepolia.inputs.private.md`. Fill in the
   private values (RPC URL + lens + collateral token).
2. Source the private file into a shell.
3. Re-run this milestone (`SEPOLIA-OPERATOR-INPUT-PROVISIONING-AND-READONLY-CHECKS`)
   from that shell. The harness will now have the env vars and can
   issue the `cast call` reads.
4. The follow-up run produces the
   `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_EXECUTION_RESULT.md` with
   real status flags, and either:
   * CLOSES BS-2 … BS-5 → live approval gate goes to READY → operator
     types the literal approval line in
     `E2E-SEPOLIA-LIVE-BROADCAST`.
   * Triggers one of the 4 targeted state-mutating fix tasks
     (`SEPOLIA-MEL-DEPLOY` / `EXECUTOR-AUTH-GRANT` /
     `BUYER-SELLER-FUNDING` / `ACTIVE-SERIES-ORACLE-SETUP`).

## 19. Cross-links

* `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT.md` (M-P5-RO)
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_EXECUTION_RESULT.md` (companion)
* `E2E_SEPOLIA_OPERATOR_INPUT_PRIVATE_TEMPLATE.md`
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md`
* `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md`
* `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md`
* `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md`

**End of M-P5-PROV result.**
