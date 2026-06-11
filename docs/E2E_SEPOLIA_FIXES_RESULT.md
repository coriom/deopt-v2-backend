# E2E_SEPOLIA_FIXES_RESULT (M-P5 Phase A Follow-up)

**Date:** 2026-06-10
**Milestone:** `E2E-SEPOLIA-FIXES` (M-P5 follow-up).
**Posture:** **No mainnet. No Sepolia broadcast. No source code
changes. No `.env` edit. No production secrets. No state-mutating
operations.**

## 1. Purpose

Close the 5 blockers (BS-1 … BS-5) identified by M-P5 Phase A
through:

* Source-tree analysis (BS-1).
* Read-only confirmation-command playbooks for the operator (BS-2,
  BS-3, BS-4, BS-5).

No address is invented. No transaction is sent.

## 2. Repos / docs inspected

* `deopt-v2-backend/docs/E2E_SEPOLIA_{TRADING_LIFECYCLE_RESULT,READ_ONLY_PREFLIGHT_RUNBOOK,BLOCKERS_AND_FIXES,FIXES_NEXT_TASK,LIVE_APPROVAL_GATE}.md`
* `deopt-v2-backend/docs/{BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT,BACKEND_TRADING_API_PHASE_5_RESULT}.md`
* `deopt-v2-backend/docs/openapi/trading-api.openapi.json`
* `deopt-v2-sol/docs/{SOL_PRODUCT_SCOPE_FREEZE_RESULT,SOL_BACKEND_FRONTEND_ABI_HANDOFF}.md`
* `deopt-v2-sol/abis/freeze-v2-product-rc1/{freeze-manifest.json,selectors.txt}`
* `deopt-v2-sol/src/collateral/{CollateralVaultViews,CollateralVault,CollateralVaultActions}.sol`
* `deopt-v2-sol/src/lens/MarginEngineLens.sol`
* `deopt-v2-sol/script/` (no `DeployMarginEngineLens.s.sol` found)
* `~/DEOPT/TESTNET_RUNBOOK.md`
* `~/DEOPT/RUN_STATE.md`

## 3. Operator input handling

`~/DEOPT/operator-private/` and `~/DEOPT/private/` were checked. The
operator-side fill-in file for this milestone is NOT created by this
task; the template
`E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` is provided so the operator
can copy it to a private (untracked) location. No private values are
included in any checked-in artefact.

RPC URL handling: PRESENT/MISSING only. NEVER printed. NEVER
committed. NEVER echoed to logs.

## 4. BS-1 status — CLOSED via abstract-inheritance discovery

`CollateralVaultViews` (at `deopt-v2-sol/src/collateral/CollateralVaultViews.sol`)
is declared as:

```solidity
abstract contract CollateralVaultViews is CollateralVaultYield {
```

`CollateralVault` (the concrete deployed contract) inherits the
views surface:

```solidity
contract CollateralVault is CollateralVaultActions {
```

where `CollateralVaultActions is CollateralVaultViews`. The two
selectors used by the backend trading_views helpers
(`getCollateralTokens()` = `0xb58eb63f`, `balances()` =
`0xc23f001f`) appear in `selectors.txt` ONLY under the
`CollateralVault` heading — confirming the inherited surface.

**Conclusion:**
`OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS` = `OPTION_COLLATERAL_VAULT_ADDRESS`
= `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` (Sepolia, anchored
to existing checked-in
`deopt-v2-sol/docs/MARGIN_ENGINE_V2_PHASE1_BROADCAST_AUTH_PACKET_V2D_O.md`).

**BS-1 → CLOSED.** No on-chain check required.

## 5. BS-2 status — OPEN (operator input required)

`MarginEngineLens` (at `deopt-v2-sol/src/lens/MarginEngineLens.sol`)
is a concrete standalone contract:

```solidity
contract MarginEngineLens is MarginEngineTypes {
```

It takes `marginEngine` as a function parameter (lines 119, 146,
174, 267, 352, 369, 398), so a single deployed instance serves all
margin engines.

No deploy script is checked in (no
`script/DeployMarginEngineLens.s.sol` exists). No Sepolia deployment
address is documented in checked-in artefacts.

**Action**: operator either supplies a known Sepolia address from
private notes OR runs a separate governance task to deploy the lens.
Read-only confirmation playbook at
`E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` §3.

**BS-2 → OPEN — OPERATOR_INPUT_REQUIRED.**

## 6. BS-3 status — OPEN (read-only check required)

The executor (`0xc35F7A8A103A9A4464adfaa76B9B514093D23C27`, from
`~/DEOPT/TESTNET_RUNBOOK.md`) must be authorised at
`OPTION_MATCHING_ENGINE`
(`0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`) for the live
broadcast in Phase B.

Read-only confirmation:

```
cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" \
  "0xc35F7A8A103A9A4464adfaa76B9B514093D23C27" \
  --rpc-url "$EXECUTION_RPC_URL"
```

Outcome → BS-3 CONFIRMED (true) or OPEN — GOVERNANCE_ACTION_REQUIRED
(false; separate task). Full playbook at confirmation log §4.

**BS-3 → OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED.**

## 7. BS-4 status — OPEN (read-only check required)

Buyer (`0xc0A76c2A6c6b70C0B065A05E64417886416cc976`) and seller
(`0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3`) must hold sufficient
testnet ETH + collateral + allowance.

Read-only reads via `cast balance` + `cast call balanceOf(...)` +
`cast call allowance(...)` per confirmation log §5.

Mapped to OK / LOW_BALANCE / LOW_ALLOWANCE / UNKNOWN /
OPERATOR_ACTION_REQUIRED. Status flag only recorded in
`E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` — actual balance values
remain operator-side.

**BS-4 → OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED.**

## 8. BS-5 status — OPEN (read-only check required)

Active series + live oracle feed must be confirmed via
`OptionProductRegistry.totalSeries()` + `seriesAt()` + `getSeries()`
walk and `OracleRouter.hasActiveFeed()` + `getPriceSafe()` reads.

Full playbook at confirmation log §6.

**BS-5 → OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED.**

## 9. Backend read-only API smoke

The 6 trading-API read endpoints (M-P2e) + the public create-intent
endpoint (M-P2f) are wired and tested. With operator-supplied
addresses + RPC, the curl playbook in
`E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md` §3-§6 exercises them
against Sepolia.

**No new backend test runs are required for this milestone.** The
M-P5 Phase A baseline (1203 backend tests green) is the regression
baseline.

## 10. Frontend Sepolia readiness smoke

Frontend is M-P3c-ready: tsc + eslint + next build + 21 Playwright
specs all green at the M-P5 Phase A baseline. No frontend code
changed in this milestone.

Pointing to a Sepolia backend uses
`NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080` (or the
operator-supplied URL) — placeholder only, never production.
Mainnet hard-gate (4 layers) intact.

## 11. Resolved values checklist

See `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`. Summary:

| Blocker | Status |
|---|---|
| BS-1 | CLOSED |
| BS-2 | OPEN — OPERATOR_INPUT_REQUIRED |
| BS-3 | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |
| BS-4 | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |
| BS-5 | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |

**Phase B live broadcast is NOT yet ready.** Operator runs the
remaining read-only checks per
`E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`.

## 12. Remaining operator actions

See `E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`:
* A-1 — supply/confirm OPTION_MARGIN_ENGINE_LENS_ADDRESS.
* A-2 — `isExecutor(executor)` read.
* A-3 — testnet balance / allowance reads.
* A-4 — active series + oracle feed reads.

Each action is read-only; the FOLLOW-ON state-mutating tasks
(governance auth, faucet, feed registration, lens deploy) live in
separate task briefs OUTSIDE this milestone.

## 13. Live approval gate status

`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` exists. Phase B requires:
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` shows ALL BS-1 … BS-5
  CLOSED / CONFIRMED.
* All 12 preconditions (P1 … P12) tick.
* Literal operator approval line typed.

**Currently NOT MET.** Phase B remains gated.

## 14. Docs created / updated

| Path | Status |
|---|---|
| `docs/E2E_SEPOLIA_FIXES_RESULT.md` | new |
| `docs/E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` | new |
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` | new |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | new |
| `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` | new |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` | new |
| `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` | edited (BS-1 → CLOSED; other rows annotated) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |

## 15. RUN_STATE update

`/home/corio/DEOPT/RUN_STATE.md` — M-P5-FIXES closure paragraph
prepended above M-P5 Phase A.

## 16. Files changed

**Source code: ZERO.** This is a docs-only milestone.

## 17. Validations

| Check | Result |
|---|---|
| `git diff --check` | clean |
| `git status --short` | only intended new + edited files |
| Sensitive-string scan over new docs | zero hits for `EXECUTOR_PRIVATE_KEY=`, `DATABASE_URL=`, `AWS_*=`, `arn:aws:kms:`, `Bearer eyJ` |
| `.env` edit performed? | NO — `~/DEOPT/deopt-v2-backend/.env` unchanged |
| Backend source changed? | NO (no cargo run needed) |
| Frontend source changed? | NO (no tsc / eslint / build run needed) |
| OpenAPI changed? | NO (no json.tool run needed) |

## 18. Blockers

| Blocker | Status |
|---|---|
| B1 LOCAL_INTENT_FIXTURE_MISSING | closed (M-P4c) |
| B2 ON_CHAIN_RPC_NOT_WIRED | closed (M-P2e) |
| B3 FRONTEND_CREATE_INTENT_UX_MISSING | closed (M-P3c) |
| B4 NO_TEST_FRAMEWORK | partially closed |
| B5 BACKEND_TX_STATUS_FIXTURE_MISSING | closed (M-P4c) |
| B6 LOGO_NOT_IN_NAV | closed (M-P4b) |
| B7 PUBLIC_CREATE_INTENT_ENDPOINT_PENDING | closed (M-P2f) |
| BS-1 OPTION_COLLATERAL_VAULT_VIEWS unknown | **CLOSED (this milestone)** |
| BS-2 OPTION_MARGIN_ENGINE_LENS unknown | OPEN — OPERATOR_INPUT_REQUIRED |
| BS-3 Executor auth unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |
| BS-4 Buyer/seller balances unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |
| BS-5 Active series + live oracle unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED |

## 19. Next milestone recommendation

**Recommended next:** the operator completes the 4 remaining
read-only checks per
`E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` and updates
`E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`. The result is either:

* **(a)** All BS-2/3/4/5 → CONFIRMED → proceed to Phase B via
  `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` (requires the literal
  operator approval line).
* **(b)** One or more BS-2/3/4/5 → OPEN — GOVERNANCE_ACTION_REQUIRED
  / OPERATOR_ACTION_REQUIRED → operator runs the corresponding
  state-mutating Sepolia task (out of scope for this milestone).

Neither (a) nor (b) is executed by this milestone.

## 20. Cross-links

* `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (M-P5 Phase A)
* `E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md`
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md`
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`
* `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `BACKEND_TRADING_API_PHASE_5_RESULT.md` (M-P2e)
* `BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md` (M-P2f)
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of M-P5-FIXES result.**
