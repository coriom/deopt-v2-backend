# E2E Sepolia — Remaining Operator Actions (M-P5 Phase A Follow-up)

**Date:** 2026-06-10
**Audience:** operator. Each action below MUST be completed before
the M-P5 Phase B live approval gate is signed.

> **M-P5-RO update (2026-06-10):** the M-P5-RO read-only milestone
> attempted to close A-2 / A-3 / A-4 directly but could not — the
> environment had no Sepolia RPC URL or operator-supplied addresses
> (see `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT.md` §3-§4 for the
> missing-input list). Actions A-1 … A-4 below remain the close-path.

> **M-P5-PROV update (2026-06-10):** added `.gitignore`
> safeguards + private-input template + 4 targeted state-mutating
> next-task briefs:
> * `SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md` (BS-2)
> * `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md` (BS-3 if false)
> * `SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md` (BS-4)
> * `SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` (BS-5)
>
> See `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md` for the
> precise operator path.

> **M-P5-RO2 update (2026-06-10):** all 4 OPEN rows confirmed via
> on-chain read-only checks; sub-states refined. Recommended
> close order: BS-5 (oracle refresh) → BS-3 (governance grant) →
> BS-4 (funding + approves) → BS-2 (lens deploy). See
> `E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` for results.

> **SETUP-FIXES-PACK-PREFLIGHT update (2026-06-11):** the 4 per-
> blocker briefs (A-1 … A-4) are now grouped into one approval-
> gated execution pack at
> `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md`. The pack runs
> BS-5 → BS-3 → BS-4 → BS-2 sequentially, with per-step pre / post
> checks, and re-runs M-P5-RO2 at the end. Preflight inspection +
> command templates: `SEPOLIA_SETUP_FIXES_PACK_PREFLIGHT_RESULT.md`.
> Per-blocker briefs (A-1 … A-4) remain valid as standalone tasks
> if the operator prefers single-blocker runs over the grouped pack.

> **SETUP-FIXES-PACK-EXECUTION attempt (2026-06-11):** operator
> approval line received. Pack **STOPPED at Phase A** because all 4
> required private keys (`DEPLOYER_PRIVATE_KEY`, `OWNER_PRIVATE_KEY`,
> `BUYER_PRIVATE_KEY`, `SELLER_PRIVATE_KEY`) are MISSING from both
> the private file and the process env. Read-only preflight clean
> (chain id 84532; 6 contracts have bytecode). No tx invoked. See
> `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_PARTIAL_RESULT.md` §3 for the
> missing-input list and the 2 safe paths to supply the keys. The
> approval line remains unconsumed and valid until 2026-06-11
> T23:57:18Z; operator may re-run within that window after
> supplying the keys.

> **SETUP-FIXES-PACK-EXECUTION CLOSURE (2026-06-12):** operator
> populated keys; pack RE-ENTERED with a fresh approval line;
> **all 4 BS rows now CLOSED on chain.** Lens deployed at
> `0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26`; executor
> authorised; buyer+seller funded (100k mUSDC each) + approved
> CollateralVault; oracle refreshed. 10 confirmed on-chain txs.
> See `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md`. **A-1 …
> A-4 in §1 are all CLOSED.** Live broadcast gate now READY FOR
> OPERATOR APPROVAL (separate broadcast approval line still
> required per `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` §9).
**Posture:** read-only checks + non-broadcast operator-side fixes.
**Hard forbidden:** mainnet RPC, mainnet tx, Safe tx, AWS / KMS
creation, production `.env` edit, signer call from any of these
steps.

## 1. Action queue

### A-1 — Supply or confirm OPTION_MARGIN_ENGINE_LENS_ADDRESS

* Operator looks up the Sepolia deployment address for
  `MarginEngineLens` from private deployment notes (operator-side
  folder).
* If no Sepolia deployment exists yet, operator deploys via the
  existing forge script set OR adds a new
  `script/DeployMarginEngineLens.s.sol` in a **separate governance
  task** (not part of M-P5).
* Operator writes the address to `.env.sepolia` as
  `OPTION_MARGIN_ENGINE_LENS_ADDRESS=<lens-addr>` (NEVER `.env`).
* Operator runs the §3 read-only confirmation in
  `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` and updates
  `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` BS-2 → CONFIRMED.

### A-2 — Confirm executor authorisation at OPTION_MATCHING_ENGINE

* Operator runs the §4 `cast call isExecutor(...)` from the
  confirmation log.
* If result is `true` → BS-3 CONFIRMED; update the checklist.
* If result is `false`:
  * Operator authorises the executor via the existing governance
    flow (NOT part of M-P5). This requires a state-mutating
    transaction on Sepolia which is outside this milestone's scope.
  * Mark BS-3 → OPEN — GOVERNANCE_ACTION_REQUIRED in the checklist.
  * Document the governance task brief in a new doc (suggest
    `BACKEND_SEPOLIA_EXECUTOR_AUTHORISATION_NEXT_TASK.md` if needed).

### A-3 — Top up buyer + seller testnet balances

* Operator runs the §5 `cast balance` + `cast call balanceOf(...)`
  reads.
* If buyer / seller ETH < 0.01 → operator requests testnet ETH
  via the Base Sepolia faucet (operator-side, no automation).
* If seller collateral < requirement → operator runs the existing
  `script/PrepareSmokeV1Accounts.s.sol` on Sepolia in a SEPARATE
  task (this is state-mutating — outside M-P5 scope).
* If allowance < requirement → operator-side `approve(...)` call
  in a SEPARATE task.
* Update checklist BS-4 → CONFIRMED when all reads return
  sufficient values.

### A-4 — Confirm active series + live oracle feed

* Operator runs the §6 `totalSeries` / `seriesAt` / `getSeries`
  walk to identify at least one Active, non-expired series.
* Operator runs `OracleRouter.hasActiveFeed(...)` + `getPriceSafe(...)`
  against the chosen series.
* If both succeed → BS-5 CONFIRMED.
* If oracle feed missing → operator registers a feed via the
  existing oracle ops flow (SEPARATE governance task).
* If no Active series → operator activates one via
  `setSeriesActive(series_id, true)` (SEPARATE governance task).

## 2. Tasks that ARE NOT part of M-P5-FIXES

The following are explicitly out of scope and must be tracked as
separate governance / ops tasks:

* Executor authorisation grant (state-mutating; requires multisig).
* Faucet top-ups (operator-side, no automation here).
* PrepareSmokeV1Accounts run (state-mutating Sepolia script).
* MarginEngineLens deployment (state-mutating).
* Oracle feed registration (state-mutating; multisig).
* Series activation (state-mutating; multisig).

## 3. Acceptance criteria

`E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` shows ALL of:

* BS-1: CLOSED
* BS-2: CONFIRMED
* BS-3: CONFIRMED
* BS-4: CONFIRMED
* BS-5: CONFIRMED

When ALL are met, the operator can proceed to fill in the
`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` instance and sign off the
literal approval line in a sepolia-only run. See
`E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` for the actual broadcast
task brief.

## 4. Forbidden

* No state-mutating transaction in this milestone.
* No mainnet RPC.
* No Safe tx.
* No production secrets in any checked-in artefact.

## 5. Cross-links

* `E2E_SEPOLIA_FIXES_RESULT.md`
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`

**End of remaining operator actions.**
