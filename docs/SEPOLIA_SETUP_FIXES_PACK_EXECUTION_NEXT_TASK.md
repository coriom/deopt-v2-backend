# SEPOLIA-SETUP-FIXES-PACK-EXECUTION — Next Task Brief

**Date written:** 2026-06-11
**Origin:** `SEPOLIA-SETUP-FIXES-PACK-PREFLIGHT_RESULT.md`.
**Target:** sequential execution of BS-2, BS-3, BS-4, BS-5 on
Base Sepolia, with per-step pre / post checks.
**Posture:** **APPROVAL-GATED. NEVER auto-execute. NEVER mainnet.
NEVER Safe tx. NEVER AWS / KMS creation. NEVER production `.env`
edit. NEVER print RPC URLs or private keys.**

> **This task is NOT executed by the calling milestone. The
> harness that picks this up MUST require the literal operator
> approval line below before any `cast send`, `forge script
> --broadcast`, or `forge create` is invoked.**

> **First attempt (2026-06-11) STOPPED at Phase A.** Operator
> approval line received; private file PRESENT (mode 600); chain id
> 84532 confirmed; 6 contracts have bytecode. But all 4 required
> private keys (`DEPLOYER_PRIVATE_KEY`, `OWNER_PRIVATE_KEY`,
> `BUYER_PRIVATE_KEY`, `SELLER_PRIVATE_KEY`) were MISSING from both
> the private file and the process env. No tx invoked. Approval
> line remains unconsumed; the pack may re-enter within the 4-hour
> window after the operator supplies the keys via one of the safe
> paths in `SEPOLIA_SETUP_FIXES_PACK_EXECUTION_PARTIAL_RESULT.md` §3.1.

---

## 1. Literal operator approval line (REQUIRED)

The operator MUST type the following line verbatim — in the chat,
the commit message, or a separate sign-off doc — before any
state-mutating command runs:

> I approve Base Sepolia setup fixes execution for BS-2, BS-3, BS-4, and BS-5.

The line:
* Authorises **one** execution of this pack (all 4 fixes, in the
  order specified in §4 below).
* Expires when the pack completes OR after **4 hours**, whichever
  comes first.
* Does **not** authorise the subsequent live-broadcast milestone
  (`E2E-SEPOLIA-LIVE-BROADCAST`) — that needs its own approval.

If the line is missing, malformed, or addresses a different scope,
the harness MUST treat the pack as NOT APPROVED and exit at §3.0.

---

## 2. Hard preconditions

All of the following MUST be true at the start of the run:

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Operator approval line present (§1, verbatim) | grep |
| P2 | `cast chain-id --rpc-url "$BASE_SEPOLIA_RPC_URL"` returns `84532` | required first call |
| P3 | Private operator input file `~/DEOPT/private/operator-private/sepolia.inputs.private.env` present, mode 600, outside any git repo | `stat`, `git check-ignore` |
| P4 | Private file contains: `BASE_SEPOLIA_RPC_URL`, `OPTION_PRODUCT_REGISTRY`, `OPTION_MATCHING_ENGINE`, `OPTION_MARGIN_ENGINE`, `OPTION_COLLATERAL_VAULT`, `OPTION_ORACLE_ROUTER`, `COLLATERAL_TOKEN`, `BUYER_ADDRESS`, `SELLER_ADDRESS`, `EXECUTOR_ADDRESS` | sourced into subshell, presence-checked |
| P5 | Required private keys (`$DEPLOYER_PRIVATE_KEY`, `$OWNER_PRIVATE_KEY`, `$BUYER_PRIVATE_KEY`, `$SELLER_PRIVATE_KEY`) provided via env or untracked keystore. NEVER printed | env presence-check |
| P6 | `deopt-v2-backend/.env` (Jun 8 16:55 timestamp) NOT touched | mtime check |
| P7 | No production `.env.sepolia` file checked into any of the 3 repos | git ls-files |
| P8 | `git status` clean BEFORE the pack (no unrelated tracked changes) | required |

If ANY precondition fails → **STOP** at §3.0; do not enter §4.

---

## 3. Hard stops (apply across the whole pack)

The harness MUST stop, write the partial-result doc, and exit
non-zero if any of:

* `cast chain-id` returns anything other than `84532`.
* Any read or write surfaces chain id `8453` (Base mainnet).
* Any RPC URL or private key would be written to a public log.
* Any check would require editing `deopt-v2-backend/.env`.
* The approval line is missing or has expired (> 4 hours since
  `pack_started_at`).
* Any `cast send` / `forge create` / `forge script --broadcast` is
  invoked before the approval line is matched verbatim.
* `OptionMatchingEngine.owner()` ≠ `cast wallet address --private-key $OWNER_PRIVATE_KEY`.
* `TestnetMockERC20.owner()` ≠ `cast wallet address --private-key $DEPLOYER_PRIVATE_KEY`.
* `MockPriceSource.owner()` ≠ `cast wallet address --private-key $DEPLOYER_PRIVATE_KEY`.
* Any postcheck fails (price still 0, isExecutor still false,
  bucket still LOW, lens code still `0x`).
* Any error envelope contains a mainnet RPC URL substring.

On hard stop: write
`docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_PARTIAL_RESULT.md`
with the step at which the stop fired + the failing check name.
Do NOT include raw RPC values, private keys, or balances.

---

## 4. Execution sequence

```
3.0 Pre-flight (approval line check; P1 .. P8)
3.1 BS-5  oracle refresh             — MockPriceSource.setPrice
3.2 BS-3  executor authorisation     — OptionMatchingEngine.setExecutor
3.3 BS-4  funding + approvals        — faucet (manual) + mUSDC.mint + IERC20.approve
3.4 BS-2  MarginEngineLens deploy    — forge create
3.5 Re-run M-P5-RO2 read-only checks
3.6 Update docs + RUN_STATE
3.7 Sensitive-string scan + git diff --check + final report
```

### 3.0 Pre-flight

```bash
# Verify approval line is present in the operator-supplied
# approval channel (whatever the harness reads — commit msg, sign-
# off doc, chat). Compare verbatim:
APPROVAL_LINE="I approve Base Sepolia setup fixes execution for BS-2, BS-3, BS-4, and BS-5."
grep -F -x "$APPROVAL_LINE" "$OPERATOR_APPROVAL_PATH" >/dev/null \
  || { echo "Approval line missing — STOP."; exit 1; }
```

Then source the private file into a subshell and verify P2 / P4:

```bash
(
  set -a
  source ~/DEOPT/private/operator-private/sepolia.inputs.private.env
  set +a
  CID=$(cast chain-id --rpc-url "$BASE_SEPOLIA_RPC_URL")
  [ "$CID" = "84532" ] || { echo "Chain id mismatch — STOP."; exit 1; }
  for v in OPTION_PRODUCT_REGISTRY OPTION_MATCHING_ENGINE \
           OPTION_MARGIN_ENGINE OPTION_COLLATERAL_VAULT \
           OPTION_ORACLE_ROUTER COLLATERAL_TOKEN \
           BUYER_ADDRESS SELLER_ADDRESS EXECUTOR_ADDRESS; do
    [ -n "${!v}" ] || { echo "$v missing — STOP."; exit 1; }
  done
)
```

### 3.1 BS-5 — oracle refresh

```bash
# Precheck (read-only): derive primary source for series-0.
cast call "$OPTION_ORACLE_ROUTER" \
  "getFeed(address,address)((address,address,uint32,uint16,bool))" \
  "$UNDERLYING_0" "$SETTLEMENT_0" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Extract primarySource as MOCK_SRC_0.

# Authority precheck.
cast call "$MOCK_SRC_0" "owner()(address)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
DEPLOYER_ADDR=$(cast wallet address --private-key "$DEPLOYER_PRIVATE_KEY")
# Compare. STOP if mismatch.

# State change (requires approval).
cast send "$MOCK_SRC_0" "setPrice(uint256)" 300000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"

# If a secondarySource exists, repeat for it.

# Postcheck.
cast call "$OPTION_ORACLE_ROUTER" \
  "getPriceSafe(address,address)(uint256,uint256,bool)" \
  "$UNDERLYING_0" "$SETTLEMENT_0" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Success: price > 0 AND ok == true.
```

* **Success:** mark BS-5 advance candidate → `CONFIRMED` at §3.5.
* **Failure:** STOP. Do not proceed to 3.2.

### 3.2 BS-3 — executor authorisation

```bash
# Authority precheck.
ENGINE_OWNER=$(cast call "$OPTION_MATCHING_ENGINE" "owner()(address)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL")
OWNER_ADDR=$(cast wallet address --private-key "$OWNER_PRIVATE_KEY")
# STOP if ENGINE_OWNER != OWNER_ADDR.

# State change (requires approval).
cast send "$OPTION_MATCHING_ENGINE" \
  "setExecutor(address,bool)" "$EXECUTOR_ADDRESS" true \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$OWNER_PRIVATE_KEY"

# Postcheck.
cast call "$OPTION_MATCHING_ENGINE" \
  "isExecutor(address)(bool)" "$EXECUTOR_ADDRESS" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Expected: true.
```

* **Success:** BS-3 → CONFIRMED.
* **Failure:** STOP. Do not proceed to 3.3.

### 3.3 BS-4 — funding + approvals

> **Manual sub-step.** ETH faucet visits happen out of band
> (browser, captcha). The harness pauses, prompts the operator to
> confirm faucet completion, then continues.

```bash
# Precheck (vault accepts deposits).
cast call "$OPTION_COLLATERAL_VAULT" "depositsPaused()(bool)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"        # expect: false
cast call "$OPTION_COLLATERAL_VAULT" \
  "launchActiveCollateral(address)(bool)" "$COLLATERAL_TOKEN" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"        # expect: true

# Authority precheck.
TOKEN_OWNER=$(cast call "$COLLATERAL_TOKEN" "owner()(address)" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL")
DEPLOYER_ADDR=$(cast wallet address --private-key "$DEPLOYER_PRIVATE_KEY")
# STOP if TOKEN_OWNER != DEPLOYER_ADDR.

# Operator confirms faucet manually. Harness checks:
BUYER_ETH=$(cast balance "$BUYER_ADDRESS"  --rpc-url "$BASE_SEPOLIA_RPC_URL")
SELLER_ETH=$(cast balance "$SELLER_ADDRESS" --rpc-url "$BASE_SEPOLIA_RPC_URL")
# Both should be ≥ 10_000_000_000_000_000 wei (0.01 ETH).

# State change: mint mUSDC to buyer + seller (10_000 mUSDC native = 0.01 mUSDC@6dec; adjust per chosen series size).
cast send "$COLLATERAL_TOKEN" "mint(address,uint256)" \
  "$BUYER_ADDRESS" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"

cast send "$COLLATERAL_TOKEN" "mint(address,uint256)" \
  "$SELLER_ADDRESS" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"

# State change: buyer + seller approve CollateralVault.
cast send "$COLLATERAL_TOKEN" "approve(address,uint256)" \
  "$OPTION_COLLATERAL_VAULT" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$BUYER_PRIVATE_KEY"

cast send "$COLLATERAL_TOKEN" "approve(address,uint256)" \
  "$OPTION_COLLATERAL_VAULT" 10000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$SELLER_PRIVATE_KEY"

# Postcheck.
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" \
  "$BUYER_ADDRESS"  --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast call "$COLLATERAL_TOKEN" "balanceOf(address)(uint256)" \
  "$SELLER_ADDRESS" --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast call "$COLLATERAL_TOKEN" "allowance(address,address)(uint256)" \
  "$BUYER_ADDRESS"  "$OPTION_COLLATERAL_VAULT" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
cast call "$COLLATERAL_TOKEN" "allowance(address,address)(uint256)" \
  "$SELLER_ADDRESS" "$OPTION_COLLATERAL_VAULT" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# All 6 buckets must flip to OK.
```

* **Success:** BS-4 → CONFIRMED.
* **Failure (allowance still LOW):** retry the failed `approve` once.
* **Failure (balance still LOW):** check mint receipt; STOP and
  investigate.

### 3.4 BS-2 — MarginEngineLens deploy

```bash
# Precheck.
cast chain-id --rpc-url "$BASE_SEPOLIA_RPC_URL"     # expect 84532
DEPLOYER_ADDR=$(cast wallet address --private-key "$DEPLOYER_PRIVATE_KEY")
cast balance "$DEPLOYER_ADDR" --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Should be ≥ 0.005 ETH for the deploy.

# State change.
cd ~/DEOPT/deopt-v2-sol
forge create src/lens/MarginEngineLens.sol:MarginEngineLens \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"
# Capture "Deployed to: 0x…" output into $NEW_LENS_ADDR without
# emitting RPC URL or private key into the log.

# Postcheck.
cast code "$NEW_LENS_ADDR" --rpc-url "$BASE_SEPOLIA_RPC_URL" | head -c 12
# Non-"0x"-only expected.

cast call "$NEW_LENS_ADDR" \
  "getAccountState(address,address)" \
  "$OPTION_MARGIN_ENGINE" \
  "0x0000000000000000000000000000000000000001" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" || true
```

* **Success:** harness prompts operator to update the private file
  with `OPTION_MARGIN_ENGINE_LENS_ADDRESS=$NEW_LENS_ADDR` (operator
  edits with `$EDITOR`; harness does NOT auto-write).
* **Failure:** STOP. Retry forge create only after diagnosing.

### 3.5 Re-run M-P5-RO2

After all 4 fixes succeed, re-run the read-only milestone:

```bash
# This is the SEPOLIA-READONLY-CHECKS-WITH-RPC milestone — read-only,
# already in the docs. Source it from the same private file.
```

Expected outcome of the re-run:
* BS-2 → CONFIRMED (lens code present at the new address).
* BS-3 → CONFIRMED (`isExecutor==true`).
* BS-4 → CONFIRMED (all 6 buckets OK).
* BS-5 → CONFIRMED (`getPriceSafe > 0`).

### 3.6 Docs + RUN_STATE

* Write `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md` with
  the final BS row statuses, the tx hashes (Sepolia — public-safe
  in this context), and the gate-flip recommendation.
* Update `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` (flip all
  4 BS rows to CONFIRMED).
* Update `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` status banner
  ("GATE NOT MET" → "GATE READY FOR OPERATOR APPROVAL — proceed
  to `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`").
* Update `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` (BS-2 / BS-3 /
  BS-4 / BS-5 all CLOSED).
* Prepend a closure paragraph to `~/DEOPT/RUN_STATE.md`.

The live broadcast itself remains **GATED** — the operator must
type the separate broadcast approval line in
`E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` before the next milestone.

### 3.7 Final validations

```bash
# Sensitive-string scan on every new/edited doc:
git diff --staged --name-only | xargs -I {} \
  grep -l -E '(0x[a-fA-F0-9]{64}|rpc-url\s+http|PRIVATE_KEY=[^$])' {} \
  || echo "scan clean"

git diff --check          # whitespace errors
git status --short        # only intended docs
ls -la /home/corio/DEOPT/deopt-v2-backend/.env  # mtime unchanged
stat -c '%a %y' ~/DEOPT/private/operator-private/sepolia.inputs.private.env
# mode MUST remain 600
```

---

## 5. Scope — what the pack DOES

* 1 `setPrice` call against each registered `MockPriceSource`
  behind the first usable candidate series feed (BS-5).
* 1 `setExecutor` call against `OptionMatchingEngine` (BS-3).
* 2 `mint` + 2 `approve` calls against the testnet mUSDC token
  (BS-4).
* 1 `forge create` deploy of `MarginEngineLens` (BS-2).
* 1 re-run of the read-only confirmation milestone.
* Doc + RUN_STATE updates.

Expected total broadcast tx count: **≤ 7** (1 setPrice + 1
setExecutor + 2 mints + 2 approves + 1 deploy). If a secondary
oracle source exists, +1. The harness MUST refuse if the tx count
exceeds **10** within a single approval.

---

## 6. Scope — what the pack DOES NOT do

* Does NOT call `executeTrade` (that's the live broadcast
  milestone).
* Does NOT touch the live broadcast approval gate state beyond
  flipping the banner from "NOT MET" → "READY FOR OPERATOR
  APPROVAL".
* Does NOT create or modify any AWS / KMS resource.
* Does NOT edit `.env`.
* Does NOT modify the Solidity source tree (only the
  out-of-tree `forge create` artefact).
* Does NOT push commits or open PRs.
* Does NOT use the production multisig flow (Sepolia rehearsal
  uses owner keys directly per the per-blocker briefs).

---

## 7. Forbidden (whole pack)

* No mainnet (chain id `8453`).
* No Safe tx.
* No AWS / KMS creation.
* No production `.env` edit.
* No `.env.sepolia` commit.
* No private key in any log or doc.
* No exact balance / allowance / price in any public doc.
* No second broadcast under the same approval line.
* No retry of a failed step beyond what its postcheck explicitly
  allows.
* No skipping of any precheck.

---

## 8. Rollback

If 3.1 (BS-5) succeeds but 3.2 (BS-3) fails:
* BS-5 stays CONFIRMED. No rollback needed — feed refresh is
  permanent (the next stale check is on the next maxDelay window).
* Investigate the BS-3 owner mismatch; STOP.

If 3.3 (BS-4) succeeds but 3.4 (BS-2) fails:
* BS-3 / BS-4 / BS-5 all CONFIRMED. Lens deploy is independent.
* Retry `forge create` once after diagnosing. If it fails again,
  STOP and report.

The pack is naturally idempotent at the "extra mint/approve" level
— re-running BS-4 in a subsequent invocation does not break state.

---

## 9. Acceptance criteria

The pack closes successfully when ALL of:

* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` shows BS-1 CLOSED +
  BS-2 / BS-3 / BS-4 / BS-5 CONFIRMED.
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` banner flipped to
  "READY FOR OPERATOR APPROVAL".
* M-P5-RO2 re-run confirms each blocker independently.
* No source code changed.
* No `.env` modified.
* Sensitive-string scan clean.
* `git diff --check` clean.

---

## 10. Cross-links

* `docs/SEPOLIA_SETUP_FIXES_PACK_PREFLIGHT_RESULT.md`
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
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of execution next-task brief.**
