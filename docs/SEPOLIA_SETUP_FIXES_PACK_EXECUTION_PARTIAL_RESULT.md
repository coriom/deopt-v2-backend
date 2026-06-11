# SEPOLIA-SETUP-FIXES-PACK-EXECUTION — Partial Result (STOPPED AT PHASE A)

**Date:** 2026-06-11
**Milestone:** `SEPOLIA-SETUP-FIXES-PACK-EXECUTION` (group fix
runner for BS-2 / BS-3 / BS-4 / BS-5).
**Posture:** **Approval line received. Read-only preflight executed.
Pack STOPPED at Phase A step 7 — required private keys MISSING.
No state-mutating call invoked. No `cast send`, no `forge create`,
no `forge script --broadcast`. No mainnet. No `.env` edit. No
private value (RPC URL, key, balance) printed.**

---

## 1. Operator approval line

Approval line received verbatim:

> "I approve Base Sepolia setup fixes execution for BS-2, BS-3, BS-4, and BS-5."

This authorisation:
* Scopes BS-2 / BS-3 / BS-4 / BS-5 Base Sepolia setup fixes only.
* Does NOT authorise live broadcast, mainnet, Safe tx, AWS/KMS,
  production signer, or any out-of-scope action.
* Was applied to a pack that **STOPPED at Phase A** before any
  state-mutating call could run. The approval is therefore
  **unconsumed** — it remains available within its 4-hour window
  if the operator supplies the missing private keys and re-runs.

**Pack window**
* `pack_started_at` : 2026-06-11T19:57:18Z
* `expires_at` (4h) : 2026-06-11T23:57:18Z

---

## 2. Phase A — global preflight

### 2.1 Private file presence

| Check | Result |
|---|---|
| `~/DEOPT/private/operator-private/sepolia.inputs.private.env` | **PRESENT** |
| Mode | `600` |
| Tracked in git? | NO (verified by prior `.gitignore` + `git check-ignore`) |
| Modified by this milestone? | NO (sourced into subshell only) |

### 2.2 Variable presence audit (no values printed)

| Variable | Status |
|---|---|
| `BASE_SEPOLIA_RPC_URL` | PRESENT |
| `EXECUTION_RPC_URL` | PRESENT |
| `OPTION_PRODUCT_REGISTRY` | PRESENT |
| `OPTION_MATCHING_ENGINE` | PRESENT |
| `OPTION_MARGIN_ENGINE` | PRESENT |
| `OPTION_COLLATERAL_VAULT` | PRESENT |
| `OPTION_COLLATERAL_VAULT_VIEWS` | PRESENT |
| `OPTION_ORACLE_ROUTER` | PRESENT |
| `COLLATERAL_TOKEN` | PRESENT |
| `BUYER_ADDRESS` | PRESENT |
| `SELLER_ADDRESS` | PRESENT |
| `EXECUTOR_ADDRESS` | PRESENT |
| `OPTION_MARGIN_ENGINE_LENS_ADDRESS` | EMPTY (expected — set after BS-2) |
| `ACTIVE_OPTION_SERIES_ID` | EMPTY (operator picks at trade time) |
| **`DEPLOYER_PRIVATE_KEY`** | **MISSING** |
| **`OWNER_PRIVATE_KEY`** | **MISSING** |
| **`BUYER_PRIVATE_KEY`** | **MISSING** |
| **`SELLER_PRIVATE_KEY`** | **MISSING** |

### 2.3 Alternate key source audit

| Source | Result |
|---|---|
| Current process env vars (`DEPLOYER_PRIVATE_KEY`, …) | MISSING (all 4) |
| `OPERATOR_PRIVATE_KEY`, `EOA_PRIVATE_KEY`, `TEST_PRIVATE_KEY` env vars | MISSING |
| `~/.foundry/keystores/` directory | PRESENT (1 keystore file present, name not printed) |
| `~/DEOPT/private/operator-private/` files named `*key*`, `*keystore*`, `*wallet*` | NONE |
| `~/DEOPT/private/mainnet_custody/` | EXISTS but **out of scope** (mainnet-only; NOT read; standing rule) |

The single foundry keystore cannot supply 4 distinct identities
(deployer / owner / buyer / seller) without operator-supplied
account names + passwords. The pack cannot derive `cast wallet
address --private-key …` without raw keys; the brief's authority
prechecks (Phase B / C / D / E) require comparing each derived
address against the on-chain owner — without the raw keys we
cannot run those prechecks safely, and skipping them would violate
hard stops.

### 2.4 Chain id check (read-only, no keys needed)

```
cast chain-id --rpc-url "$BASE_SEPOLIA_RPC_URL"
```

| Result | |
|---|---|
| `chain_id` | **84532** (Base Sepolia) |
| Mainnet hard-stop triggered? | NO |
| RPC URL printed? | NO (subshell-scoped) |

### 2.5 Public contract bytecode probes (read-only, no keys needed)

| Contract | Bytecode |
|---|---|
| `OPTION_PRODUCT_REGISTRY` | PRESENT |
| `OPTION_MATCHING_ENGINE` | PRESENT |
| `OPTION_MARGIN_ENGINE` | PRESENT |
| `OPTION_COLLATERAL_VAULT` | PRESENT |
| `OPTION_ORACLE_ROUTER` | PRESENT |
| `COLLATERAL_TOKEN` | PRESENT |

All six match the M-P5-RO2 baseline. No drift.

### 2.6 Decision

Per `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md` §2 (P5) and
the user-supplied Phase A step 7:

> "If any required key is missing, stop before any transaction and
> produce missing-input list."

→ **HARD STOP at Phase A. No phases B / C / D / E executed.**

---

## 3. Missing-input list

Operator must supply BEFORE the pack can re-enter:

| Key | Purpose | Identity it must match on chain |
|---|---|---|
| `DEPLOYER_PRIVATE_KEY` | BS-5 (MockPriceSource.setPrice) + BS-4 (TestnetMockERC20.mint) + BS-2 (forge create) | `MockPriceSource.owner()` for each refreshed source AND `TestnetMockERC20.owner()` for the mUSDC token |
| `OWNER_PRIVATE_KEY` | BS-3 (OptionMatchingEngine.setExecutor) | `OptionMatchingEngine.owner()` on Sepolia |
| `BUYER_PRIVATE_KEY` | BS-4 (approve CollateralVault as buyer) | `BUYER_ADDRESS` (`0x394291A05D3df2d1D8bFCBc571dAD773Ac7077cC`) |
| `SELLER_PRIVATE_KEY` | BS-4 (approve CollateralVault as seller) | `SELLER_ADDRESS` (`0xb1f1ae6CB0d154AFe9503c3B0790adeF0851FD88`) |

If a single key controls multiple identities (e.g. deployer key
also owns OptionMatchingEngine), operator may set the same value
for multiple variables; the harness compares derived addresses
in Phase A step 8 (out of scope for this stopped run).

### 3.1 How to supply (safe paths)

The operator picks ONE of:

* **Append to the existing private file** (preferred):
  ```bash
  # Operator opens with $EDITOR, never `cat`:
  $EDITOR ~/DEOPT/private/operator-private/sepolia.inputs.private.env
  # Add lines (no surrounding whitespace, no quotes around the value):
  #   DEPLOYER_PRIVATE_KEY=0x…
  #   OWNER_PRIVATE_KEY=0x…
  #   BUYER_PRIVATE_KEY=0x…
  #   SELLER_PRIVATE_KEY=0x…
  chmod 600 ~/DEOPT/private/operator-private/sepolia.inputs.private.env
  ```
* **OR export inline before re-running the pack**:
  ```bash
  read -rs DEPLOYER_PRIVATE_KEY; export DEPLOYER_PRIVATE_KEY
  read -rs OWNER_PRIVATE_KEY;    export OWNER_PRIVATE_KEY
  read -rs BUYER_PRIVATE_KEY;    export BUYER_PRIVATE_KEY
  read -rs SELLER_PRIVATE_KEY;   export SELLER_PRIVATE_KEY
  # `read -rs` reads silently without echoing; values stay in the shell session only.
  ```

### 3.2 Forbidden when supplying

* Never write the key into any tracked file.
* Never paste the key into a chat / commit message / public log.
* Never set the key in `deopt-v2-backend/.env` (production file).
* Never reuse mainnet keys from `~/DEOPT/private/mainnet_custody/`.

---

## 4. Per-phase outcomes

| Phase | Status |
|---|---|
| A — global preflight | **PARTIAL** (chain id + bytecode probes passed; key audit failed) |
| B — BS-5 oracle refresh | **NOT RUN** |
| C — BS-3 executor authorisation | **NOT RUN** |
| D — BS-4 funding + approvals | **NOT RUN** |
| E — BS-2 lens deploy | **NOT RUN** |
| F — read-only recheck | **NOT RUN** |
| G — docs / RUN_STATE | this partial result + 3 cross-doc edits |
| H — validations | clean (see §7) |

---

## 5. Blocker state (unchanged from M-P5-RO2)

| BS | State |
|---|---|
| BS-1 | CLOSED |
| BS-2 | OPEN — DEPLOYMENT_REQUIRED |
| BS-3 | OPEN — GOVERNANCE_ACTION_REQUIRED |
| BS-4 | OPEN — LOW_BALANCE + LOW_ALLOWANCE |
| BS-5 | OPEN — ORACLE_FEED_MISSING (`getPriceSafe==0`) |

Live broadcast gate: **NOT MET** (preconditions P2 / P3 / P4 / P5
all still open per `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`).

---

## 6. Tx accounting

| Metric | Value |
|---|---|
| `cast send` invocations | **0** |
| `forge create` invocations | **0** |
| `forge script --broadcast` invocations | **0** |
| `cast call` invocations | 7 (read-only: 1 chain id + 6 bytecode probes) |
| `cast balance` invocations | 0 |
| Mainnet RPC touched | NO |
| Tx hashes generated | (none) |
| Approval line consumed | NO (remains valid until expiry) |

---

## 7. Validations

| Check | Result |
|---|---|
| `git diff --check` (backend) | clean |
| `git status --short` | only intended docs |
| Sensitive-string scan on new + edited docs | zero hits |
| `.env` (deopt-v2-backend) modified? | NO (`Jun  8 16:55:05` preserved) |
| Private input file modified? | NO (mode 600 preserved; mtime preserved) |
| Private input file committed? | NO (verified `.gitignore`-blocked) |
| RPC URL printed? | NO (subshell-scoped) |
| Private key printed? | NO (none handled — none supplied) |
| Mainnet RPC used? | NO (chain id 84532 confirmed) |
| State-mutating command run? | NO |
| Source code modified? | NO |

---

## 8. Docs created / updated by this run

| Path | Action |
|---|---|
| `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_PARTIAL_RESULT.md` | NEW (this doc) |
| `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` | edited (Phase-A-stop banner) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (Phase-A-stop banner) |
| `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md` | edited (missing-input note in §2 / §3.0) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |
| All `SEPOLIA-*_NEXT_TASK.md` per-blocker briefs | UNCHANGED |
| `SEPOLIA_SETUP_FIXES_PACK_PREFLIGHT_RESULT.md` | UNCHANGED |

---

## 9. Next milestone recommendation

1. Operator supplies the 4 missing private keys via one of the safe
   paths in §3.1.
2. Operator re-runs `SEPOLIA-SETUP-FIXES-PACK-EXECUTION` with the
   same approval line (still valid within the 4-hour window).
3. Phase A re-runs key presence audit → proceeds to authority
   prechecks → enters Phases B / C / D / E.

If the operator prefers to scope the key supply more tightly,
they may instead re-run the standalone briefs one at a time:
`SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` (BS-5 only;
needs `DEPLOYER_PRIVATE_KEY`), `SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md`
(BS-3 only; needs `OWNER_PRIVATE_KEY`), etc. The grouped pack and
the standalone briefs are interchangeable; the pack is just a
convenience runner.

The live-broadcast milestone (`E2E-SEPOLIA-LIVE-BROADCAST`)
remains GATED — preconditions P1–P12 of
`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` are not yet met. No new
approval line is needed there until the pack closes BS-2 / BS-3 /
BS-4 / BS-5.

---

## 10. Cross-links

* `docs/SEPOLIA_SETUP_FIXES_PACK_PREFLIGHT_RESULT.md`
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` (M-P5-RO2)
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `docs/SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md`
* `docs/SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md`
* `docs/SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md`
* `docs/SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of partial-result doc.**
