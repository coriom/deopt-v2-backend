# SEPOLIA-SETUP-FIXES-PACK-PREFLIGHT — Result

**Date:** 2026-06-11
**Milestone:** `SEPOLIA-SETUP-FIXES-PACK-PREFLIGHT` (group prep
for BS-2 / BS-3 / BS-4 / BS-5 close-out).
**Posture:** **Read-only planning. No `cast send`. No `forge script
--broadcast`. No deploy. No signer call. No mainnet. No state
mutation. No RPC URL / private key / balance value printed.** All
command blocks below are templates marked **NOT TO RUN YET**;
execution is gated by the operator-approval flow in the companion
execution next-task doc.

> **Scope.** Consolidate the 4 individual BS fix briefs into one
> coherent setup pack with a recommended execution order and
> per-step precheck / state-change / postcheck command templates.
> The pack remains gated by `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md`
> and the 4 underlying per-blocker briefs.

---

## 1. Inputs read

| Source | Status |
|---|---|
| `docs/E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` (M-P5-RO2) | inspected |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | inspected |
| `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` | inspected |
| `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` | inspected |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | inspected |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` | inspected |
| `docs/E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` | inspected |
| `docs/SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md` | inspected |
| `docs/SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md` | inspected |
| `docs/SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md` | inspected |
| `docs/SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md` | inspected |
| `deopt-v2-sol/src/lens/MarginEngineLens.sol` | inspected |
| `deopt-v2-sol/src/oracle/MockPriceSource.sol` | inspected |
| `deopt-v2-sol/script/RefreshLocalMockFeeds.s.sol` | inspected |
| `deopt-v2-sol/script/RefreshTestnetMockFeeds.s.sol` | inspected |
| `deopt-v2-sol/script/DeployTestnetMockFeeds.s.sol` | inspected |
| `deopt-v2-sol/script/DeployTestnetAssets.s.sol` | inspected |
| `deopt-v2-sol/script/PrepareSmokeV1Accounts.s.sol` | inspected |
| `deopt-v2-sol/abis/freeze-v2-product-rc1/OptionMatchingEngine.abi.json` | inspected |
| `deopt-v2-sol/abis/freeze-v2-product-rc1/OracleRouter.abi.json` | inspected |
| `deopt-v2-sol/abis/freeze-v2-product-rc1/OptionProductRegistry.abi.json` | inspected (selectors only) |
| `~/DEOPT/RUN_STATE.md` | inspected |

Private operator file at
`~/DEOPT/private/operator-private/sepolia.inputs.private.env`:
**present, mode 600, outside git, NOT modified by this milestone**
(values sourced into subshell only when needed for sanity checks;
this milestone performed no `cast` calls).

---

## 2. Confirmed pre-state (from M-P5-RO2)

| Blocker | Sub-state |
|---|---|
| BS-2 | **OPEN — DEPLOYMENT_REQUIRED** — `OPTION_MARGIN_ENGINE_LENS_ADDRESS` empty in private file; no Sepolia lens deploy known. |
| BS-3 | **OPEN — GOVERNANCE_ACTION_REQUIRED** — `OptionMatchingEngine.isExecutor(EXECUTOR_ADDRESS)` returned `false`. Owner key (or current owner) required to call `setExecutor(executor,true)`. |
| BS-4 | **OPEN — LOW_BALANCE + LOW_ALLOWANCE** — buyer + seller ETH < 0.01; buyer + seller mUSDC balance bucket LOW; all 4 allowances (buyer/seller → CollateralVault, buyer/seller → OptionMatchingEngine) bucket LOW. COLLATERAL_TOKEN confirmed `mUSDC`, 6 decimals (verified read-only). |
| BS-5 | **OPEN — ORACLE_FEED_MISSING (price=0)** — `OptionProductRegistry.totalSeries()` returns `2`; both candidate series have `hasActiveFeed(under,settle)==true` AND `getPriceSafe(under,settle)==0` (mock oracle stale). |

Chain id `84532` (Base Sepolia) confirmed via
`cast chain-id --rpc-url "$BASE_SEPOLIA_RPC_URL"` in M-P5-RO2.
Mainnet hard-stop NOT triggered.

---

## 3. Frozen-ABI surface confirmed for each fix

| Fix | Function signature | Selector | Authority |
|---|---|---|---|
| BS-3 grant | `setExecutor(address executor, bool allowed)` | `0x1e1bff3f` | `OptionMatchingEngine.owner()` (`onlyOwner`) |
| BS-3 verify | `isExecutor(address) returns (bool)` | `0xdebfda30` | view |
| BS-5 feed status | `hasActiveFeed(address,address) returns (bool)` | `0x6c166bb3` | view |
| BS-5 feed read | `getPriceSafe(address,address) returns (uint256,uint256,bool)` | `0xb39be1ed` (per ABI) | view |
| BS-5 feed config (if re-register needed) | `setFeed(address,address,address,address,uint32,uint16,bool)` | `0x93240036` | `OracleRouter.owner()` |
| BS-5 feed status flip | `setFeedStatus(address,address,bool)` | `0x23bcafc7` | `OracleRouter.owner()` |
| BS-5 mock refresh (per source) | `MockPriceSource.setPrice(uint256)` | — | `MockPriceSource.owner()` (typically deployer) |
| BS-2 deploy | `MarginEngineLens` constructor: **no args** | — | any funded deployer EOA |
| BS-2 verify | `MarginEngineLens.getAccountState(address,address)` | — | view |
| BS-4 mint | `TestnetMockERC20.mint(address,uint256)` | — | `TestnetMockERC20.owner()` (= deployer) |
| BS-4 approve | `IERC20.approve(address,uint256)` | — | token holder EOA |

Notes:
* The BS-3 brief originally suggested `script/SetMatchingEngineExecutor.s.sol` — **that script does not exist** in `deopt-v2-sol/script/`. The owner calls `setExecutor` directly via `cast send`.
* The BS-5 fix targets the **mock oracle sources** behind the registered feeds (not the OracleRouter itself), because `hasActiveFeed==true` (so no `setFeed` re-registration is needed) but `getPriceSafe==0` (the underlying `MockPriceSource` has never had a `setPrice` push since the feed was wired up, OR `setPrice` was called but `_updatedAt > maxDelay` ago — same fix path either way).
* `PrepareSmokeV1Accounts.s.sol` mints **1 mUSDC** and dusts **0.0003 ETH** per EOA, then has each EOA `approve` + `deposit` into `CollateralVault`. It is built for the **perp smoke**, not options, but the funding/approval surface it produces is sufficient for the options trade (deposits sit in vault; `OptionMatchingEngine` settles via the vault on `executeTrade`). The script enforces `block.chainid != 8453` (mainnet refused).

---

## 4. Per-blocker plans

### 4.1 BS-5 — Oracle feed refresh

**Goal.** `OracleRouter.getPriceSafe(under,settle)` returns nonzero
for at least one of the two existing candidate series.

**Why this is the cheapest first step.** Mock-oracle `setPrice` is
a 1-tx call against a small contract (no proxy, no governance
multisig). It does not depend on any other fix.

**Authority required.**
* `MockPriceSource.owner()` (= the EOA that deployed
  `DeployTestnetMockFeeds.s.sol`). Operator looks this up from
  prior deployment notes.
* OR: re-deploy a fresh `MockPriceSource` and call
  `OracleRouter.setFeed(...)` (owner-only) to point the feed at the
  new source. This is heavier; do not use unless the original
  deployer key is unavailable.

**Authority precheck (read-only, MUST run before any write):**

```bash
# (a) Confirm the registered primary source for series-0.
# Inputs (subshell only):
#   $UNDERLYING_0, $SETTLEMENT_0 — from M-P5-RO2 series #0 candidate
#   $OPTION_ORACLE_ROUTER       — from private file

cast call "$OPTION_ORACLE_ROUTER" \
  "getFeed(address,address)((address,address,uint32,uint16,bool))" \
  "$UNDERLYING_0" "$SETTLEMENT_0" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Extract primarySource (1st tuple field) as $MOCK_SRC_0

# (b) Confirm the MockPriceSource owner.
cast call "$MOCK_SRC_0" "owner()(address)" --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Expected: the deployer EOA whose private key the operator holds.
```

**State-change command template (NOT TO RUN YET):**

```bash
# Push fresh price for series-0 primary source.
# Pick a price that is meaningful for the underlying — the M-P5-RO2
# series #0 is a call on $0x4dee… with mUSDC settlement and strike
# ~$3000, so a sensible ETH-like price is 300_000_000_000 (= $3000 1e8).
cast send "$MOCK_SRC_0" \
  "setPrice(uint256)" 300000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"
```

If the OracleRouter feed config has `secondarySource != 0`, the
operator runs the same `setPrice` against the secondary source.

Alternative: if the deployer holds the `.env.base-sepolia` env vars
required by `RefreshTestnetMockFeeds.s.sol` (TESTNET_MOCKS_ENABLED,
DEPLOYER_PRIVATE_KEY, ETH_USDC_PRIMARY_SOURCE, etc.), they may run:

```bash
cd ~/DEOPT/deopt-v2-sol
TESTNET_MOCKS_ENABLED=true \
  forge script script/RefreshTestnetMockFeeds.s.sol \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --broadcast
# Script refuses Base mainnet (block.chainid==8453 reverts).
```

**Postcheck (read-only):**

```bash
cast call "$OPTION_ORACLE_ROUTER" \
  "getPriceSafe(address,address)(uint256,uint256,bool)" \
  "$UNDERLYING_0" "$SETTLEMENT_0" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Success: price > 0; ok == true; updatedAt fresh.
```

**Success condition:** `getPriceSafe` returns nonzero `price` AND
`ok == true` for at least one candidate series.

**Failure condition:** `price == 0` OR `ok == false`. Possible
causes:
* `setPrice` reverted: caller is not `MockPriceSource.owner()`.
* `getPriceSafe` still 0: source updated, but feed config
  `maxDelay` < lag since `setPrice`. Push again with a fresh tx.

**Rollback / next action on failure:** Stop the pack. Do NOT
proceed to BS-3 / BS-4 / BS-2. Inspect with `cast call` whether
the source was updated (`getLatestPrice()`) and what `maxDelay`
the feed config carries.

---

### 4.2 BS-3 — Executor authorisation grant

**Goal.** `OptionMatchingEngine.isExecutor(EXECUTOR_ADDRESS) == true`.

**Authority required.** `OptionMatchingEngine.owner()` private key.

**Authority precheck (read-only):**

```bash
cast call "$OPTION_MATCHING_ENGINE" "owner()(address)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Compare to the address derived from $OWNER_PRIVATE_KEY:
cast wallet address --private-key "$OWNER_PRIVATE_KEY"
# Both MUST match. If they don't, STOP. Owner is a multisig or
# different EOA; treat as separate governance task (out of scope).
```

**State-change command template (NOT TO RUN YET):**

```bash
cast send "$OPTION_MATCHING_ENGINE" \
  "setExecutor(address,bool)" \
  "$EXECUTOR_ADDRESS" true \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"
```

**Postcheck (read-only):**

```bash
cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" \
  "$EXECUTOR_ADDRESS" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Expected: true
```

**Success condition:** `isExecutor` returns `true`. `ExecutorSet`
event present in the tx receipt (selector `0xd394c2…`).

**Failure condition:**
* `setExecutor` reverts with `OwnableUnauthorizedAccount`: caller is
  not owner.
* `isExecutor` still `false` post-tx: tx silently confirmed against
  the wrong contract. Re-verify `OPTION_MATCHING_ENGINE` address.

**Rollback / next action on failure:** Stop. Investigate owner
mismatch BEFORE retrying. Do not proceed to BS-4.

---

### 4.3 BS-4 — Buyer/seller funding + approvals

**Goal.** Buyer + seller hold enough ETH + mUSDC + allowance to
let a single `executeTrade(...)` pass through.

**Authority required.**
* ETH faucet: operator-only (browser, captcha).
* mUSDC mint: `TestnetMockERC20.owner()` = deployer (the same EOA
  that deployed via `DeployTestnetAssets.s.sol`).
* Approves: each of buyer + seller signs.

**Authority precheck (read-only):**

```bash
# (a) mUSDC owner check.
cast call "$COLLATERAL_TOKEN" "owner()(address)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast wallet address --private-key "$DEPLOYER_PRIVATE_KEY"
# Both MUST match.

# (b) Vault accepts deposits.
cast call "$OPTION_COLLATERAL_VAULT" \
  "depositsPaused()(bool)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Expected: false.
cast call "$OPTION_COLLATERAL_VAULT" \
  "launchActiveCollateral(address)(bool)" "$COLLATERAL_TOKEN" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Expected: true (otherwise PrepareSmokeV1Accounts reverts).
```

**State-change command templates (NOT TO RUN YET):**

```bash
# Step A — ETH faucet (operator browser, no automation):
#   https://www.alchemy.com/faucets/base-sepolia
#   or QuickNode equivalent.
# Target ≥ 0.05 ETH per EOA (buyer + seller + deployer if low).

# Step B — mUSDC mint (owner-only).
# Mint 10_000 mUSDC native units = 0.01 mUSDC (6 decimals); pick a
# size large enough to cover one trade's settlement worst-case.
# Operator chooses the exact amount based on the chosen series'
# strike × quantity. Conservative default below:
cast send "$COLLATERAL_TOKEN" \
  "mint(address,uint256)" \
  "$BUYER_ADDRESS" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"

cast send "$COLLATERAL_TOKEN" \
  "mint(address,uint256)" \
  "$SELLER_ADDRESS" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"

# Step C — buyer approves CollateralVault.
cast send "$COLLATERAL_TOKEN" \
  "approve(address,uint256)" \
  "$OPTION_COLLATERAL_VAULT" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$BUYER_PRIVATE_KEY"

# Step D — seller approves CollateralVault.
cast send "$COLLATERAL_TOKEN" \
  "approve(address,uint256)" \
  "$OPTION_COLLATERAL_VAULT" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$SELLER_PRIVATE_KEY"

# Step E — OPTIONAL: buyer + seller approve OPTION_MATCHING_ENGINE.
# Skip unless `executeTrade` does a direct `transferFrom` from the
# trader's EOA (most flows route through CollateralVault, so this
# is usually unnecessary on Sepolia). Keep as a fallback if the
# trade reverts with allowance-related errors.

# Step F — ALTERNATIVE: replace B/C/D with a single forge run.
cd ~/DEOPT/deopt-v2-sol
PREPARE_SMOKE_V1_ACCOUNTS_CONFIRM=true \
  forge script script/PrepareSmokeV1Accounts.s.sol \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --broadcast
# Requires DEPLOYER_PRIVATE_KEY (mUSDC owner) +
# PERP_SMOKE_BUYER_PRIVATE_KEY + PERP_SMOKE_SELLER_PRIVATE_KEY +
# BASE_COLLATERAL_TOKEN + COLLATERAL_VAULT in env. Script refuses
# Base mainnet (block.chainid==8453 reverts).
```

**Postcheck (read-only):**

```bash
cast balance "$BUYER_ADDRESS"  --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast balance "$SELLER_ADDRESS" --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" \
  "$BUYER_ADDRESS" --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" \
  "$SELLER_ADDRESS" --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast call "$COLLATERAL_TOKEN" "allowance(address,address)(uint256)" \
  "$BUYER_ADDRESS" "$OPTION_COLLATERAL_VAULT" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast call "$COLLATERAL_TOKEN" "allowance(address,address)(uint256)" \
  "$SELLER_ADDRESS" "$OPTION_COLLATERAL_VAULT" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Record bucket flags (OK / LOW_BALANCE / LOW_ALLOWANCE) only.
# Do NOT publish raw numbers.
```

**Success condition:** all 4 BS-4 buckets flip to **OK**.

**Failure condition:**
* `mint` reverts: deployer is not mUSDC owner.
* `approve` reverts: buyer/seller key mismatch.
* Vault rejects deposit (if Step F): `launchActiveCollateral`
  false OR `depositsPaused` true → operator-side governance task.

**Rollback / next action on failure:** Stop. Do not proceed to
BS-2. Investigate which sub-step failed; rerun only the missing
sub-step (idempotent — extra mints / approves are fine).

---

### 4.4 BS-2 — MarginEngineLens deployment

**Goal.** A deployed `MarginEngineLens` instance on Base Sepolia
with bytecode at a known address. Address written into the
private operator file as `OPTION_MARGIN_ENGINE_LENS_ADDRESS`.

**Authority required.** Any funded deployer EOA. `MarginEngineLens`
is stateless and has **no constructor arguments**.

**Authority precheck (read-only):**

```bash
# Confirm chain id BEFORE deploying.
cast chain-id --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Expected: 84532

# Confirm deployer EOA has ≥ 0.005 ETH for deploy gas (lens is
# stateless and small; usually < 2_000_000 gas).
cast balance $(cast wallet address --private-key "$DEPLOYER_PRIVATE_KEY") \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
```

**State-change command template (NOT TO RUN YET):**

```bash
cd ~/DEOPT/deopt-v2-sol
forge create src/lens/MarginEngineLens.sol:MarginEngineLens \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"
# forge prints "Deployed to: 0x…". Capture into $NEW_LENS_ADDR
# without echoing to public logs.
```

**Postcheck (read-only):**

```bash
# Bytecode exists.
cast code "$NEW_LENS_ADDR" --rpc-url "$BASE_SEPOLIA_RPC_URL" | head -c 12
# Expected: non-"0x"-only prefix.

# Selector responds (revert allowed; we only need selector recognition).
cast call "$NEW_LENS_ADDR" \
  "getAccountState(address,address)" \
  "$OPTION_MARGIN_ENGINE" \
  "0x0000000000000000000000000000000000000001" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" || true
```

**Operator-side persistence (NOT a state-change on chain):**

```bash
# Operator opens the private file with $EDITOR (not `cat`) and adds:
#   OPTION_MARGIN_ENGINE_LENS_ADDRESS=<NEW_LENS_ADDR>
# Re-chmod 600 after edit. NEVER print the file to a public log.
```

**Success condition:** `cast code` non-empty; selector recognised;
private file updated; mode 600 preserved.

**Failure condition:** `forge create` reverts (insufficient ETH;
chain id wrong; compiler mismatch). `cast code` returns `0x` (the
deploy did not land).

**Rollback / next action on failure:** Stop. Investigate the
forge log. Lens deploys are cheap, so a clean retry is the
typical path.

---

## 5. Recommended execution order

```
1. BS-5  oracle refresh             (MockPriceSource.setPrice)
2. BS-3  executor authorisation     (OptionMatchingEngine.setExecutor)
3. BS-4  funding + approvals        (faucet + mUSDC.mint + IERC20.approve)
4. BS-2  MarginEngineLens deploy    (forge create)
5. Re-run SEPOLIA-READONLY-CHECKS-WITH-RPC (read-only milestone)
6. If all 4 BS rows CONFIRMED → flip live-broadcast gate to
   READY FOR OPERATOR APPROVAL.
```

### Why this order is safe

| Step | Rationale |
|---|---|
| BS-5 first | Cheapest action (1 tx against a stateless mock). Independent of every other fix. Failure here costs ~10 cents of testnet gas; success unblocks downstream `executeTrade` validation. |
| BS-3 second | Required for any live `executeTrade` broadcast at all — without it the broadcast reverts with `NotAuthorized`. Independent of BS-4 and BS-2. |
| BS-4 third | Largest number of sub-actions (faucet + 2 mints + 2 approves). Best to run after BS-5 / BS-3 so any failure here doesn't waste oracle / governance grants. mUSDC mints are idempotent at the balance level (extra mints fine). |
| BS-2 last | Pure read-only-surface deploy. Lens has no constructor args, no state, no dependency on BS-5 / BS-3 / BS-4. Putting it last means the smallest blast radius if anything fails: lens deploy can be retried independently without re-running upstream fixes. |
| Re-run M-P5-RO2 | Independent verification; advances each BS row from OPEN → CONFIRMED in the public-safe checklist. |
| Gate flip | Only mechanical; no on-chain action. |

### Why this order is NOT mandatory

BS-2 / BS-3 / BS-5 are independent — they may execute in any order.
BS-4 should run after BS-3 (so funding isn't wasted if the executor
grant fails). The recommended order optimises for "minimum gas
burnt if something fails early" rather than parallelism, which is
not relevant here (a single operator runs steps sequentially).

---

## 6. Operator-input invariants (all 4 fixes)

| Var | Source | Print policy |
|---|---|---|
| `$BASE_SEPOLIA_RPC_URL` | private file | NEVER echo |
| `$OPTION_MATCHING_ENGINE`, `$OPTION_PRODUCT_REGISTRY`, `$OPTION_MARGIN_ENGINE`, `$OPTION_COLLATERAL_VAULT`, `$OPTION_ORACLE_ROUTER`, `$COLLATERAL_TOKEN` | private file | OK in operator logs; NEVER in checked-in docs without "Sepolia-only" annotation |
| `$BUYER_ADDRESS`, `$SELLER_ADDRESS`, `$EXECUTOR_ADDRESS` | private file (sourced from public TESTNET_RUNBOOK) | OK in operator logs |
| `$UNDERLYING_0`, `$SETTLEMENT_0` | derived from `getSeries($SERIES_ID_0)` (operator runs once) | OK in operator logs |
| `$MOCK_SRC_0` | derived from `getFeed($UNDERLYING_0, $SETTLEMENT_0)` | OK in operator logs |
| `$DEPLOYER_PRIVATE_KEY`, `$OWNER_PRIVATE_KEY`, `$BUYER_PRIVATE_KEY`, `$SELLER_PRIVATE_KEY` | operator-only — KMS / hardware wallet / encrypted file | **NEVER printed; NEVER echoed; NEVER in any log** |

---

## 7. Forbidden across the whole pack

* No mainnet (chain id MUST be `84532`; `8453` is the hard-stop).
* No Safe / multisig tx (Sepolia rehearsal uses owner key directly).
* No AWS / KMS account or key creation.
* No `.env` edit (only the private operator file is mutated, and
  only by the operator's own editor).
* No production `.env.sepolia` checked into git.
* No `cast send` against any address other than the 5 fix targets
  (`$MOCK_SRC_*`, `$OPTION_MATCHING_ENGINE`, `$COLLATERAL_TOKEN`,
  `$NEW_LENS_ADDR` post-deploy).
* No `forge script --broadcast` outside the 2 allowed scripts
  (`RefreshTestnetMockFeeds.s.sol`, `PrepareSmokeV1Accounts.s.sol`).
* No `forge create` outside `src/lens/MarginEngineLens.sol`.
* No private key in any checked-in file.
* No exact balance, allowance, or price value in any public doc
  (bucket flags only).
* No second broadcast under the same approval line.

---

## 8. Hard stops (whole pack)

Stop immediately and switch back to dry-run mode if any of:

* `cast chain-id` returns anything other than `84532`.
* `OptionMatchingEngine.owner()` does not match the address derived
  from `$OWNER_PRIVATE_KEY` (BS-3 precheck).
* `TestnetMockERC20.owner()` does not match the address derived
  from `$DEPLOYER_PRIVATE_KEY` (BS-4 precheck).
* `MockPriceSource.owner()` does not match the address derived
  from `$DEPLOYER_PRIVATE_KEY` (BS-5 precheck).
* Any postcheck fails (price still 0, isExecutor still false,
  bucket still LOW, lens code still `0x`).
* Any RPC error returns a mainnet (`8453`) chain id or a
  non-Sepolia network name.

---

## 9. Execution next-task doc

The operator-approval-gated execution prompt lives in:

* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md`

That doc carries the literal approval line:

> "I approve Base Sepolia setup fixes execution for BS-2, BS-3, BS-4, and BS-5."

Until that line is supplied verbatim by the operator, the
execution prompt MUST NOT run any state-mutating command.

---

## 10. Docs created / updated by this milestone

| Path | Action |
|---|---|
| `docs/SEPOLIA_SETUP_FIXES_PACK_PREFLIGHT_RESULT.md` | NEW (this doc) |
| `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md` | NEW (execution prompt) |
| `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md` | edited (preflight pack banner) |
| `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` | edited (preflight pack banner) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (preflight pack banner) |
| `~/DEOPT/RUN_STATE.md` | edited (preflight closure paragraph) |
| `docs/SEPOLIA-{MARGIN-ENGINE-LENS-DEPLOY,EXECUTOR-AUTH-GRANT,BUYER-SELLER-FUNDING,ACTIVE-SERIES-ORACLE-SETUP}_NEXT_TASK.md` | UNCHANGED (each still applies as the per-blocker standalone brief) |
| Source code (`*.sol`, `*.rs`, `*.ts`, `*.tsx`) | UNCHANGED |

---

## 11. Validations performed in this milestone

| Check | Result |
|---|---|
| Read-only inspection only — no `cast send` invoked | ✓ |
| No `forge script --broadcast` invoked | ✓ |
| No `forge create` invoked | ✓ |
| No mainnet RPC used (chain id check deferred — no on-chain call this milestone) | ✓ |
| `.env` (deopt-v2-backend) modified? | NO (timestamp `Jun  8 16:55` preserved) |
| Private file modified? | NO (mode 600 preserved) |
| Private file printed? | NO |
| RPC URL printed? | NO |
| Source code modified? | NO |
| `git diff --check` | clean |

---

## 12. Cross-links

* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_READONLY_CHECKS_WITH_RPC_RESULT.md` (M-P5-RO2)
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `docs/E2E_SEPOLIA_REMAINING_OPERATOR_ACTIONS.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `docs/SEPOLIA-MARGIN-ENGINE-LENS-DEPLOY_NEXT_TASK.md`
* `docs/SEPOLIA-EXECUTOR-AUTH-GRANT_NEXT_TASK.md`
* `docs/SEPOLIA-BUYER-SELLER-FUNDING_NEXT_TASK.md`
* `docs/SEPOLIA-ACTIVE-SERIES-ORACLE-SETUP_NEXT_TASK.md`
* `deopt-v2-sol/src/lens/MarginEngineLens.sol`
* `deopt-v2-sol/script/RefreshTestnetMockFeeds.s.sol`
* `deopt-v2-sol/script/PrepareSmokeV1Accounts.s.sol`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of preflight pack result.**
