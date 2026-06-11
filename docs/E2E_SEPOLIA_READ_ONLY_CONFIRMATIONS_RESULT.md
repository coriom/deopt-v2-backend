# E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT (M-P5-RO)

**Date:** 2026-06-10
**Milestone:** `E2E-SEPOLIA-READ-ONLY-CONFIRMATIONS` (M-P5-FIXES
follow-up).
**Posture:** **Docs only. No mainnet. No Sepolia broadcast. No
source code changes. No `.env` edit. No state-mutating operations.
No private values committed.**

> **Outcome:** No on-chain confirmations could be performed in this
> environment. The Sepolia RPC URL and all operator-supplied
> addresses are absent from this environment's shell + filesystem.
> The result is a precise missing-input list for the operator. BS-2,
> BS-3, BS-4, BS-5 remain OPEN. The brief's documented fallback path
> applies: "if required values are missing, stop and produce a
> precise missing-input list."

## 1. Purpose

Run read-only Sepolia confirmations against BS-2 / BS-3 / BS-4 /
BS-5 using operator-supplied private values + a Sepolia RPC URL.
When the inputs are unavailable, document the precise missing-input
list rather than fake results or invent addresses.

## 2. Repos / docs inspected

* `deopt-v2-backend/docs/E2E_SEPOLIA_{FIXES_RESULT,OPERATOR_INPUT_TEMPLATE,READ_ONLY_CONFIRMATION_LOG,RESOLVED_VALUES_CHECKLIST,REMAINING_OPERATOR_ACTIONS,BLOCKERS_AND_FIXES,LIVE_APPROVAL_GATE,LIVE_BROADCAST_NEXT_TASK}.md`
* `deopt-v2-backend/docs/{BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT,BACKEND_TRADING_API_PHASE_5_RESULT}.md`
* `deopt-v2-sol/docs/SOL_BACKEND_FRONTEND_ABI_HANDOFF.md`
* `deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt`
* `~/DEOPT/TESTNET_RUNBOOK.md`
* `~/DEOPT/RUN_STATE.md`

## 3. Operator input handling

Scanned candidate locations (no values printed, presence only):

| Location | Status |
|---|---|
| `~/DEOPT/operator-private/` | MISSING (directory does not exist) |
| `~/DEOPT/*.private.md` | MISSING (no Sepolia file present) |
| `~/DEOPT/deopt-v2-backend/docs/*.private.md` | MISSING |
| `~/DEOPT/private/mainnet_custody/*.private.md` | PRESENT (mainnet-only; out of scope for Sepolia; NOT read) |
| Shell env `BASE_SEPOLIA_RPC_URL` | MISSING |
| Shell env `EXECUTION_RPC_URL` | MISSING |
| Shell env `RPC_URL` | MISSING |
| Shell env `SEPOLIA_RPC_URL` | MISSING |
| Shell env `OPTION_MARGIN_ENGINE_LENS_ADDRESS` | MISSING |
| Shell env `BUYER_ADDRESS` / `SELLER_ADDRESS` / `EXECUTOR_ADDRESS` / `COLLATERAL_TOKEN` | MISSING |
| `cast` binary | PRESENT (`/home/corio/.foundry/bin/cast`) |
| `curl` binary | PRESENT |
| `~/DEOPT/deopt-v2-backend/.env` | PRESENT (UNCHANGED — timestamp preserved; NOT read by this milestone) |

The `cast` and `curl` tooling exists but **cannot reach Sepolia**:
no RPC URL is configured in any of the shell vars the brief
documented as expected. The mainnet-custody private files are
explicitly out of scope (mainnet-only; this milestone is Sepolia).

**No values were printed.** No private file was read. No `.env`
file was opened.

## 4. Precise missing-input list

For each open blocker, the operator must supply (privately,
out-of-band; NEVER committed):

### BS-2 inputs

| Field | Form | Where to supply |
|---|---|---|
| `OPTION_MARGIN_ENGINE_LENS_ADDRESS` | `0x` + 40 hex | private `.env.sepolia` OR shell env |
| `EXECUTION_RPC_URL` | Sepolia HTTPS RPC URL | shell env |

### BS-3 inputs

| Field | Form | Where to supply |
|---|---|---|
| `EXECUTION_RPC_URL` | Sepolia HTTPS RPC URL | shell env |
| Executor address | public — `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | already in `~/DEOPT/TESTNET_RUNBOOK.md` |
| `OPTION_MATCHING_ENGINE` | public — `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` | already in `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` §3.2 |

### BS-4 inputs

| Field | Form | Where to supply |
|---|---|---|
| `EXECUTION_RPC_URL` | Sepolia HTTPS RPC URL | shell env |
| Buyer address | public — `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` | already documented |
| Seller address | public — `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` | already documented |
| `COLLATERAL_TOKEN` (ERC20 address) | `0x` + 40 hex | operator-side (Sepolia deployment notes) |
| Spender for allowance check | `OPTION_COLLATERAL_VAULT_ADDRESS` (public) | already documented |

### BS-5 inputs

| Field | Form | Where to supply |
|---|---|---|
| `EXECUTION_RPC_URL` | Sepolia HTTPS RPC URL | shell env |
| Candidate series id (optional) | `uint256` (or backend `option_series_id` string) | operator-side notes |

**No private value was created or committed by this milestone.**

## 5. BS-2 — MarginEngineLens confirmation

**Status: OPEN — OPERATOR_INPUT_REQUIRED.**

| Check | Result |
|---|---|
| Search checked-in docs / scripts / artefacts for a MarginEngineLens Sepolia address | NONE FOUND (re-confirmed) |
| Operator-supplied private/local value present | NONE FOUND in this environment |
| Shape validation | N/A (no value to validate) |
| `cast code` bytecode probe | NOT RUN (no address; no RPC) |
| `cast call getAccountState(...)` smoke | NOT RUN (no address; no RPC) |

The contract is concrete (`contract MarginEngineLens is MarginEngineTypes`,
verified at `deopt-v2-sol/src/lens/MarginEngineLens.sol`). No deploy
script exists in `deopt-v2-sol/script/`. The Sepolia deployment
address is operator-side knowledge that has not been surfaced into
checked-in artefacts.

**Operator action to close:** see
`E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` A-1. If no Sepolia
deployment exists, the operator runs a separate governance-gated
deploy task (out of scope here; suggest milestone name
`SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY`).

## 6. BS-3 — executor authorization

**Status: OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (not run).**

Both required public addresses are known:
* Executor: `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27`
* Matching engine: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`

The single required call is:

```
cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "isExecutor(address)(bool)" \
  0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 \
  --rpc-url "$EXECUTION_RPC_URL"
```

**Not run in this environment** because `EXECUTION_RPC_URL` is
absent.

If result is `false`, the close-path is
`SEPOLIA-EXECUTOR-AUTH-GRANT` (separate state-mutating task — out of
scope here).

## 7. BS-4 — buyer/seller balances + allowances

**Status: OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (not run).**

Buyer + seller public addresses are known. The required private
inputs are `EXECUTION_RPC_URL` and `COLLATERAL_TOKEN`. The reads
are:

```
cast balance 0xc0A76c2A6c6b70C0B065A05E64417886416cc976 --rpc-url "$EXECUTION_RPC_URL"
cast balance 0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3 --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" 0xc0A7... --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" 0xbAf0... --rpc-url "$EXECUTION_RPC_URL"
cast call "$COLLATERAL_TOKEN" "allowance(address,address)(uint256)" 0xbAf0... 0x00340C... --rpc-url "$EXECUTION_RPC_URL"
```

**Not run in this environment** because `EXECUTION_RPC_URL` and
`COLLATERAL_TOKEN` are absent.

Outcomes map to OK / LOW_BALANCE / LOW_ALLOWANCE per
`E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` §5. Recorded as status
flags only — never balance values — in
`E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`.

## 8. BS-5 — active series + oracle feed

**Status: OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (not run).**

`OptionProductRegistry` (`0x3d52b033fab00ed6104dd3bc0a715f8648344eca`)
and `OracleRouter` (`0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581`)
addresses are known. The single missing input is
`EXECUTION_RPC_URL`. The full read walk is in
`E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` §6.

**Not run in this environment** because `EXECUTION_RPC_URL` is
absent.

If no series is Active, close-path is
`SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP` (state-mutating governance —
out of scope here).

## 9. Backend read-only API smoke

**Not run in this environment.** Backend smoke requires:
* A live backend pointed at Sepolia (requires `EXECUTION_RPC_URL`).
* A series identified in BS-5 step.
* Buyer/seller addresses funded per BS-4.

None of these prerequisites are met. The exact curl playbook is
already documented in
`E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md` §3-§6 and is
unchanged.

## 10. Live approval gate status

`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` remains **GATED**. P1
(checklist all CLOSED/CONFIRMED) is NOT met: BS-2 / BS-3 / BS-4 /
BS-5 are all OPEN. The literal approval line is therefore not
typeable.

## 11. Docs created / updated

| Path | Status |
|---|---|
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT.md` | new (this doc) |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | edited (refined BS-2…BS-5 status to "OPEN — OPERATOR_INPUT_REQUIRED — read-only checks not run in current environment") |
| `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` | edited (cross-link to this result) |
| `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` | edited (cross-link to this result; status refined) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (explicit "gate currently NOT MET" line) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` | edited (NOT READY — preconditions row #1 unmet) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |

## 12. RUN_STATE update

M-P5-RO closure paragraph prepended above M-P5-FIXES.

## 13. Files changed

**Source code: ZERO.** Backend `.env` unchanged. No JSON files
touched. No build / test / cargo / npm run required.

## 14. Validations

| Check | Result |
|---|---|
| `git diff --check` | clean |
| `git status --short` | only intended files |
| Sensitive-string scan on new + edited docs | zero hits for `EXECUTOR_PRIVATE_KEY=`, `DATABASE_URL=`, `AWS_*=`, `arn:aws:kms:`, `Bearer eyJ` |
| `.env` modified? | NO — timestamp preserved |
| Backend source changed? | NO |
| Frontend source changed? | NO |
| OpenAPI changed? | NO |
| Any private value committed? | NO |
| Any address invented? | NO |

## 15. Blockers (post-milestone)

| Blocker | Status |
|---|---|
| BS-1 OPTION_COLLATERAL_VAULT_VIEWS unknown | CLOSED (M-P5-FIXES) |
| BS-2 OPTION_MARGIN_ENGINE_LENS unknown | OPEN — OPERATOR_INPUT_REQUIRED (no environment input; no deploy script in tree) |
| BS-3 Executor authorisation unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (one cast call away; needs RPC URL) |
| BS-4 Buyer/seller balances unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (needs RPC URL + COLLATERAL_TOKEN) |
| BS-5 Active series + oracle feed unconfirmed | OPEN — OPERATOR_READ_ONLY_CHECK_REQUIRED (needs RPC URL) |

## 16. Next milestone recommendation

Two parallel branches (operator picks):

**(a) Single targeted environment-setup milestone** —
`SEPOLIA-OPERATOR-INPUT-PROVISIONING`: operator (out-of-band)
sets `EXECUTION_RPC_URL` + the missing private addresses in their
shell or `.env.sepolia` (NEVER `.env`), then re-runs
`E2E-SEPOLIA-READ-ONLY-CONFIRMATIONS` from this same environment.
This closes BS-3, BS-4, BS-5 in a single read-only pass.

**(b) Per-blocker remediation milestones** (if a read returns a
failing value):
* `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY` — if BS-2 lens address is
  truly absent (state-mutating; governance-gated; out of scope here).
* `SEPOLIA-EXECUTOR-AUTH-GRANT` — if BS-3 returns `isExecutor=false`
  (state-mutating; governance-gated; out of scope here).
* `SEPOLIA-BUYER-SELLER-FUNDING` — if BS-4 returns LOW_BALANCE or
  LOW_ALLOWANCE (operator faucet / approve runs; out of scope here).
* `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP` — if BS-5 returns
  SERIES_MISSING or ORACLE_FEED_MISSING (governance-gated; out of
  scope here).

After ALL blockers CLOSED:
* `E2E-SEPOLIA-LIVE-BROADCAST` — single approved Sepolia broadcast.
  Still requires the literal operator approval line. Brief at
  `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`.

## 17. Cross-links

* `E2E_SEPOLIA_FIXES_RESULT.md` (M-P5-FIXES)
* `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (M-P5 Phase A)
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md`
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of M-P5-RO result.**
