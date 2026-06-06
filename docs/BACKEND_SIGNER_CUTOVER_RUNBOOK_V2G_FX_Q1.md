# Backend signer cutover runbook — V2G-FX-Q1 (DEPLOYER → BACKEND_EXECUTOR)

**Posture:** operator runbook. **No agent action edits `.env` or
broadcasts transactions.** Closes FX-Q1 from
`GOVERNANCE_EXECUTOR_MIGRATION_EXECUTE_RESULT_V2G_GOV_F_B_X.md` §8 —
flipping the live trade hot-path signer config from DEPLOYER's
private key → BACKEND_EXECUTOR's private key.

**Scope:** Base Sepolia (chain 84532). Mainnet variant tracked at
§13.

**Anchors:**
- `BACKEND_EXECUTOR = 0x295005fd4F311e6691F008D57d32FCFEde844518`.
- `NEW_OME.isExecutor(BACKEND_EXECUTOR) = true` (post-GOV-F-B-X).
- `NEW_OME.isExecutor(DEPLOYER) = false` (post-GOV-F-B-X).
- `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` — key custody
  policy (KMS strongly preferred; raw-env path here is a Sepolia
  rehearsal compromise — see §13 mainnet lift).
- `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` — broadcast economics
  the post-cutover backend MUST enforce.
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — what to watch on
  cutover.

---

## 0. Hard stops (this runbook)

```text
no chain tx by agent                                     ✅
no RFQ smoke                                             ✅
no trade                                                 ✅
no reserve allocation                                    ✅
no GOV-G                                                 ✅
no .env edit by agent                                    ✅
no private key output by agent                           ✅
no admin token output                                    ✅
no secrets printed                                       ✅
no mainnet                                               ✅
no live backend broadcast until §6 dry-run is green      ✅
```

The operator performs §3 / §6 / §7 themselves.

---

## 1. Signer config findings

### 1.1 The single source of truth — `EXECUTOR_PRIVATE_KEY`

| File | Line | Behaviour |
|---|---|---|
| `src/execution/signer.rs:26` | `ExecutorSigner::from_private_key(&secret)` | Derives address by `keccak256(uncompressed_secp256k1_pubkey)[12..]`. Address is computed; never stored separately. |
| `src/execution/signer.rs:14-22` | `Debug` impl prints `private_key: <redacted>` | Logs / panics MUST NOT leak the key. Verified by unit test `signer_derives_executor_address_without_exposing_key_in_debug`. |
| `src/execution/config.rs:115-124` | Real-broadcast guard | Requires `EXECUTOR_PRIVATE_KEY` set AND validates it by attempting `ExecutorSigner::from_private_key` when `EXECUTOR_REAL_BROADCAST_ENABLED=true`. |
| `src/options/service.rs:1166` | Option broadcast path | Re-derives `signer = ExecutorSigner::from_private_key(private_key)` per intent; uses `from = signer.address()` for nonce lookup. |
| `src/options/service.rs:1213` | Sign + submit | `sign_eip1559_transaction(&request, nonce, &signer)` then `provider.send_raw_transaction(...)`. The on-chain `from` is the **address recovered from the signature** — i.e. the address derived from `EXECUTOR_PRIVATE_KEY`. |

### 1.2 Address-cross-check semantics

**The backend does NOT cross-check `EXECUTOR_FROM_ADDRESS` against
the derived address from `EXECUTOR_PRIVATE_KEY` at startup.** They
are independent fields:

| Env var | Used for |
|---|---|
| `EXECUTOR_PRIVATE_KEY` | Actual signing key (determines on-chain `from`). |
| `EXECUTOR_FROM_ADDRESS` | `eth_call.from` during simulation (`src/execution/simulator.rs:45`), API status / metadata responses (`src/api/routes.rs:550`), and the `from` placeholder in `ExecutionTransactionRequest` (overridden at broadcast time inside the option-execution path). |
| `OPTION_EXECUTION_SIMULATION_FROM` (optional) | If set, overrides simulation `from` for the option-execution-specific path (`src/options/service.rs:2014`); if unset, falls back to `EXECUTOR_FROM_ADDRESS`. |

Implication: a half-finished cutover where the operator flips
`EXECUTOR_PRIVATE_KEY` but leaves `EXECUTOR_FROM_ADDRESS` at
DEPLOYER (or `0x000…000`) will:
- still produce the correct on-chain `from` (BACKEND_EXECUTOR), but
- simulate from the wrong address — `eth_call(executeTrade)` from a
  non-executor address reverts `NotAuthorized()` (`0xea8e4eb5`),
  masking real-world reverts and blocking every broadcast through
  the `OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true` gate.

**Both fields MUST be flipped together. §3 documents that.**

### 1.3 No latent DEPLOYER assumption in source

A search for `DEPLOYER`, `0xc35F7A8A`, or `deployer.*executor`
returned only test-fixture references and historical comments. No
runtime code path assumes DEPLOYER is the executor; the entire
signer is `EXECUTOR_PRIVATE_KEY`-driven.

### 1.4 Dry-run vs real-broadcast split

| Mode | `EXECUTOR_DRY_RUN` | `EXECUTOR_REAL_BROADCAST_ENABLED` | `OPTION_EXECUTION_BROADCAST_ENABLED` | Effect |
|---|---|---|---|---|
| Off | (any) | `false` (default) | `false` (default) | No on-chain broadcast at all. Suitable for SIMULATION_ENABLED dry-run testing. |
| Perp dry-run | `true` | (any) | (any) | Marks perp intents as `dry_run_updated`; never broadcasts. The perp executor explicitly rejects real broadcasts (`src/execution/executor.rs:54-56`). |
| Option real broadcast | (any) | `true` | `true` | The option-execution path signs and submits via `provider.send_raw_transaction`. Requires `EXECUTOR_PRIVATE_KEY`, `RPC_URL`, gas-fee envs all set. **This is the post-cutover live path.** |

The cutover changes only the SIGNER identity (key + address);
broadcast-enabled flags retain their current values.

---

## 2. Env keys involved in the cutover

### 2.1 MUST CHANGE (operator-only edits)

| Key | Pre-cutover value | Post-cutover value |
|---|---|---|
| `EXECUTOR_PRIVATE_KEY` | DEPLOYER's PK (32-byte hex) | **BACKEND_EXECUTOR's PK** (32-byte hex) |
| `EXECUTOR_FROM_ADDRESS` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (DEPLOYER, lowercased) | `0x295005fd4f311e6691f008d57d32fcfede844518` (BACKEND_EXECUTOR, lowercased) |

### 2.2 MUST NOT CHANGE during this cutover

| Key | Value | Why unchanged |
|---|---|---|
| `EXECUTOR_CHAIN_ID` | `84532` | chain-id binding to Sepolia |
| `RPC_URL` | (operator's Base Sepolia URL) | unchanged unless RPC failover separately decided |
| `EXECUTOR_DRY_RUN` | retain current | flipped only as part of §6 dry-run procedure, then back |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | retain current | flipped to `true` only after §6 dry-run is green |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | retain current | same as above |
| `OPTION_EXECUTION_REQUIRE_SIMULATION_OK` | `true` | enforces simulation pass before broadcast |
| `OPTION_EXECUTION_SIGNATURE_MODE` | retain current | unrelated to executor cutover |
| Fee / gas envs (`EXECUTOR_MAX_FEE_PER_GAS_WEI`, `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI`, `EXECUTOR_MAX_GAS_LIMIT`) | retain current | re-tuned in a separate ops task per gas/fees/rebates policy §9 |
| Engine addresses (`OPTION_MATCHING_ENGINE_ADDRESS`, `PERP_*`) | retain current | unchanged — same NEW_OME |
| Persistence (`PERSISTENCE_ENABLED`, `DATABASE_URL`) | retain current | unchanged |

### 2.3 OPTIONAL (only if currently set non-empty)

| Key | Action |
|---|---|
| `OPTION_EXECUTION_SIMULATION_FROM` | If currently set to DEPLOYER's address: flip to BACKEND_EXECUTOR's address (lowercased). If currently empty: leave empty — falls back to `EXECUTOR_FROM_ADDRESS`. |

### 2.4 SECRETS / NEVER OUTPUT

```text
- EXECUTOR_PRIVATE_KEY value             — handled by operator only;
                                           agent NEVER requests or holds it
- any KMS handle / KMS ARN               — handled by operator
- any admin auth tokens                  — out of scope
- DATABASE_URL credentials               — out of scope
- RPC URL with embedded API key          — operator-side; backend reads it
                                           from env but never logs it
```

---

## 3. Exact operator-only `.env` edit instructions

**Performed by the OPERATOR. The agent does NOT touch `.env`.**

### 3.1 Preconditions

```text
[ ] backend stopped (no live broadcasts mid-edit)
[ ] BACKEND_EXECUTOR private key available to operator via the
    approved custody path (KMS export to a transient env-var bag
    that lives only in the running backend process — never on disk)
[ ] OR for Sepolia rehearsal: the operator's local secure store
    (e.g. age-encrypted file) opened in an ephemeral shell session
    that does NOT echo
[ ] backend not running with `--echo-env` / debug-log mode
```

### 3.2 Edit

In the `.env` file for the running environment (e.g.
`~/DEOPT/deopt-v2-backend/.env` for local rehearsal; the prod
equivalent for staging / prod):

```diff
-EXECUTOR_PRIVATE_KEY=<DEPLOYER private key — REDACTED>
+EXECUTOR_PRIVATE_KEY=<BACKEND_EXECUTOR private key — REDACTED>

-EXECUTOR_FROM_ADDRESS=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
+EXECUTOR_FROM_ADDRESS=0x295005fd4f311e6691f008d57d32fcfede844518
```

If `OPTION_EXECUTION_SIMULATION_FROM` was previously set to the
DEPLOYER address, also flip:

```diff
-OPTION_EXECUTION_SIMULATION_FROM=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
+OPTION_EXECUTION_SIMULATION_FROM=0x295005fd4f311e6691f008d57d32fcfede844518
```

### 3.3 Edit hygiene

```text
[ ] `.env` is git-ignored (verify in .gitignore once after edit)
[ ] operator's shell session does NOT have `set -x` / xtrace on
    while editing
[ ] no `cat .env` after editing
[ ] no `grep EXECUTOR_PRIVATE_KEY .env` after editing
[ ] no copy/paste of the key into chat, ticket, or LLM
[ ] backup of the previous `.env` (with DEPLOYER key) is either
    a) re-encrypted at rest, or
    b) shredded — DO NOT keep an unencrypted copy of the old key
       under any retention policy
```

### 3.4 Verification without printing the secret

The §4 read-only commands derive the address from the configured
key WITHOUT printing the key itself. Operator runs them in a shell
that pipes the key into `cast` via process substitution, then
compares the derived address against `BACKEND_EXECUTOR`.

---

## 4. Read-only verification commands

All commands below are read-only. None print the private key. None
broadcast.

### 4.1 Derive address from the configured key (locally)

```bash
# Run in the same shell that has the new .env loaded.
cd ~/DEOPT/deopt-v2-backend

# Load the env (operator-only; agent does NOT do this).
set -a; source .env; set +a

# Derive the address from the configured EXECUTOR_PRIVATE_KEY.
# cast wallet address reads the key from --private-key but does NOT
# echo it; it prints only the derived address.
DERIVED=$(cast wallet address --private-key "$EXECUTOR_PRIVATE_KEY")
echo "derived address = $DERIVED"

# Expected: 0x295005fd4F311e6691F008D57d32FCFEde844518
```

### 4.2 Compare derived address to BACKEND_EXECUTOR

```bash
EXPECTED=0x295005fd4F311e6691F008D57d32FCFEde844518

# Case-insensitive compare (lowercased on both sides).
if [ "${DERIVED,,}" = "${EXPECTED,,}" ]; then
  echo "OK: derived address matches BACKEND_EXECUTOR"
else
  echo "MISMATCH: derived $DERIVED != BACKEND_EXECUTOR $EXPECTED"
fi

# Also cross-check EXECUTOR_FROM_ADDRESS.
if [ "${EXECUTOR_FROM_ADDRESS,,}" = "${EXPECTED,,}" ]; then
  echo "OK: EXECUTOR_FROM_ADDRESS matches BACKEND_EXECUTOR"
else
  echo "MISMATCH: EXECUTOR_FROM_ADDRESS=$EXECUTOR_FROM_ADDRESS != $EXPECTED"
fi
```

### 4.3 Confirm chain-side executor role is bound to BE

```bash
cd ~/DEOPT/deopt-v2-sol
set -a; source .env.base-sepolia; set +a

NEW_OME=0x5a5EBF9A9CCd7c012518569DE8283982982670f6
BACKEND_EXECUTOR=0x295005fd4F311e6691F008D57d32FCFEde844518
DEPLOYER=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27

echo "isExecutor(BACKEND_EXECUTOR):"
cast call "$NEW_OME" 'isExecutor(address)(bool)' "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL"
# expected: true

echo "isExecutor(DEPLOYER):"
cast call "$NEW_OME" 'isExecutor(address)(bool)' "$DEPLOYER" --rpc-url "$RPC_URL"
# expected: false

echo "NEW_OME.paused:"
cast call "$NEW_OME" 'paused()(bool)' --rpc-url "$RPC_URL"
# expected: false

echo "NEW_OME.owner:"
cast call "$NEW_OME" 'owner()(address)' --rpc-url "$RPC_URL"
# expected: Timelock 0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588

echo "NEW_OME.guardian:"
cast call "$NEW_OME" 'guardian()(address)' --rpc-url "$RPC_URL"
# expected: OPS_MULTISIG 0xA6B9Bb5c7B26B33cfD28C6F5A79B3c527fDdcD46
```

### 4.4 Confirm BE has gas

```bash
cast balance "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL"
# expected: >= 1e15 wei (≈ 0.001 ETH) per
# BACKEND_EXECUTOR_CUSTODY_PROFILE §4.1 floor
```

### 4.5 Confirm BE is still an EOA

```bash
cast code "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL"
# expected: 0x  (NOT a contract — alarms on the monitoring spec
#                 fire if BE ever transforms into a contract)
```

### 4.6 R5 / PFV invariant control (read-only sanity)

```bash
PFV=0x7C0a3B6feBd5BFFc164f37738299AeB453181886
mUSDC=0x6eAe407f5640B006faC9965182e238582A3B412E
CV=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
FMV2=0xF6626177f3B85cc3239667Cc53C04A8007652944
RG=0x7918Ea95c2791B6b587fF02AE481FA52403877A0

cast call "$PFV"  'feeBalance(address)(uint256)'       "$mUSDC" --rpc-url "$RPC_URL"  # 28
cast call "$PFV"  'rebateReserve(address)(uint256)'    "$mUSDC" --rpc-url "$RPC_URL"  # 0
cast call "$CV"   'balances(address,address)(uint256)' "$PFV" "$mUSDC" --rpc-url "$RPC_URL"  # 28
cast call "$FMV2" 'rebateBudget(address)(uint256)'     "$mUSDC" --rpc-url "$RPC_URL"  # 999947
cast call "$FMV2" 'protocolFeeVault()(address)'                  --rpc-url "$RPC_URL"  # PFV
cast call "$FMV2" 'feeRecipient()(address)'                      --rpc-url "$RPC_URL"  # PFV
cast call "$FMV2" 'rebateFundingAccount()(address)'              --rpc-url "$RPC_URL"  # PFV
cast call "$RG"   'feesManager()(address)'                       --rpc-url "$RPC_URL"  # NEW_FM_V2
```

All values must match the post-V2G-GOV-F-B-X snapshot. The
cutover itself touches no on-chain state; these checks confirm
the rails are intact before backend resumption.

---

## 5. Read-only API verification (backend running, broadcast disabled)

After §3 edit but BEFORE flipping `OPTION_EXECUTION_BROADCAST_ENABLED=true`:

### 5.1 Start backend with broadcast disabled

```bash
# operator-only
cd ~/DEOPT/deopt-v2-backend
# Confirm the relevant flags before start:
grep -E '^(EXECUTOR_DRY_RUN|EXECUTOR_REAL_BROADCAST_ENABLED|OPTION_EXECUTION_BROADCAST_ENABLED|EXECUTOR_FROM_ADDRESS)=' .env
# expected (during dry-run window):
#   EXECUTOR_DRY_RUN=true                          ← unchanged
#   EXECUTOR_REAL_BROADCAST_ENABLED=false          ← unchanged from current; flip later
#   OPTION_EXECUTION_BROADCAST_ENABLED=false       ← unchanged from current; flip later
#   EXECUTOR_FROM_ADDRESS=0x295005fd…4518          ← FLIPPED ✅

# Start backend.
cargo run -p deopt-v2-backend --release
```

### 5.2 Confirm signer identity via API

```bash
# Hit the status endpoint (no auth needed for the address fields).
curl -s http://127.0.0.1:8080/api/admin/execution/status | jq '{
  execution_enabled: .execution_enabled,
  dry_run: .dry_run,
  real_broadcast_enabled: .real_broadcast_enabled,
  rpc_configured: .rpc_configured,
  executor_from_address: .executor_from_address
}'
# expected:
#   executor_from_address = "0x295005fd4f311e6691f008d57d32fcfede844518"
#   real_broadcast_enabled = false
```

The status endpoint never echoes the private key (verified by
the `Debug` redact in `src/execution/signer.rs:14-22`).

### 5.3 Confirm `should_broadcast` simulation gate works

If `OPTION_EXECUTION_SIMULATION_ENABLED=true`, any candidate that
reaches the simulator runs `eth_call(executeTrade)` from
`EXECUTOR_FROM_ADDRESS`. The expected response shape (per backend
logs):

```text
simulation outcome = ok          ← because BE is now isExecutor
                                 ← was BEFORE: would have been
                                   NotAuthorized() if simulator
                                   used a non-executor from-address
```

If simulation reports `NotAuthorized()` despite `EXECUTOR_FROM_ADDRESS`
being BACKEND_EXECUTOR, STOP — either:
- the .env edit didn't take effect (backend running with old env), OR
- the on-chain executor role has reverted (re-run §4.3).

---

## 6. Backend dry-run procedure (no broadcast)

This is the **structural confidence gate** before flipping live
broadcast. Operator runs this; agent does NOT.

### 6.1 Prerequisites

```text
[ ] §3 .env edit done and verified per §4
[ ] BACKEND_EXECUTOR.balance ≥ 1e15 wei
[ ] NEW_OME.isExecutor(BE) = true
[ ] R5 invariants intact per §4.6
[ ] backend running with broadcast DISABLED
    (EXECUTOR_REAL_BROADCAST_ENABLED=false AND
     OPTION_EXECUTION_BROADCAST_ENABLED=false)
[ ] SIMULATION_ENABLED=true  (otherwise dry-run has nothing to
                              exercise)
[ ] OPTION_EXECUTION_SIMULATION_ENABLED=true
[ ] OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true
[ ] RPC_URL points at the Sepolia endpoint used elsewhere in this
    milestone (chain id must equal 84532)
```

### 6.2 Issue a no-broadcast intent (orderbook OR RFQ)

The exact API call depends on the matcher's source. Common entry
point: a buyer + seller signature pair submitted via the
option-execution intent flow. The intent's lifecycle:

```text
queued → simulating → simulation_ok → ready-for-broadcast
```

With broadcast disabled the intent stops at
`ready-for-broadcast`. No `eth_sendRawTransaction` fires. The
logs MUST show the simulation outcome AND the signer address
derived from `EXECUTOR_PRIVATE_KEY`.

### 6.3 What to expect in logs

```text
event = "execution_simulation"
result = "ok" | "reverted"
signer = "0x295005fd4f311e6691f008d57d32fcfede844518"   ← BACKEND_EXECUTOR
from   = "0x295005fd4f311e6691f008d57d32fcfede844518"
intent_id = "<uuid>"
```

If `signer != BACKEND_EXECUTOR`: the env edit didn't take effect —
restart backend, re-check §5.

### 6.4 Hard stops during dry-run

| Symptom | Action |
|---|---|
| `signer` logged as DEPLOYER | STOP — backend has the old key; restart with new .env |
| simulation reverts `NotAuthorized()` | STOP — either env edit incomplete or executor role reverted; re-run §4 |
| ANY tx hash appears in logs | STOP — broadcast flag is on by mistake; re-check `EXECUTOR_REAL_BROADCAST_ENABLED` / `OPTION_EXECUTION_BROADCAST_ENABLED` |
| panic / log line containing private-key-shaped hex | STOP — logging hygiene breach; rotate key immediately per `BACKEND_EXECUTOR_CUSTODY_PROFILE` §7.3 |

### 6.5 Dry-run done → flip broadcast flags

Once §6.3 looks clean for at least one orderbook AND one RFQ
candidate:

```diff
-EXECUTOR_REAL_BROADCAST_ENABLED=false
+EXECUTOR_REAL_BROADCAST_ENABLED=true

-OPTION_EXECUTION_BROADCAST_ENABLED=false
+OPTION_EXECUTION_BROADCAST_ENABLED=true
```

Restart backend. Continue to §7.

---

## 7. Post-cutover smoke prerequisites (do NOT run smoke yet)

A "smoke" here means a small live `executeTrade` / `executeRfqTrade`
on chain to confirm the end-to-end signer path works.

### 7.1 What CAN be smoked

| Smoke shape | Status |
|---|---|
| Single-trade `executeTrade` with `makerPpm ≥ 0` (no rebate) and balanced taker fee | **ALLOWED** if all of §7.2 holds — fee-only path, no `rebateReserve` touch |
| Simulation-only smoke (broadcast still disabled; just sim) | **ALLOWED** at any time |

### 7.2 What CANNOT be smoked yet

| Smoke shape | Why |
|---|---|
| Any trade with `makerPpm < 0` (rebate-bearing) | `PFV.rebateReserve(mUSDC) = 0`; `onRebatePaid` would revert `InsufficientRebateReserve`. Backend's `should_broadcast` rebate-solvency gate (§4.2 of gas/fees/rebates policy) MUST reject. |
| RFQ with `makerDiscountPpm` that pushes `effective_maker_ppm` negative | Same reason as above. |
| Any liquidation trade | Out of scope for this cutover — see `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` §7. |

### 7.3 Hard prerequisites for ANY live smoke (post-cutover)

```text
[ ] §6.5 broadcast flags flipped and backend restarted
[ ] §4 verification commands all green
[ ] monitoring + alerts wired per
    BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3 / §4
[ ] subsidy budgets per BACKEND_GAS_FEES_REBATES_POLICY_V1.md §9
    configured (even if zero), so the SUBSIDISABLE state has a
    well-defined zero-budget behaviour
[ ] BACKEND_EXECUTOR balance ≥ ~1e16 wei (FUND_TARGET, not just
    FUND_FLOOR) — headroom for the first burst of trades
[ ] on-call SRE acknowledged the cutover window
[ ] separate operator decision document records the choice to
    proceed with live smoke (this runbook does not authorise it)
```

### 7.4 Why no smoke is part of this milestone

The brief explicitly excludes RFQ smoke and trades. The cutover
is structurally complete the moment §6 dry-run is green and §6.5
flags are flipped. Whether the operator then runs a fee-only live
smoke is a separate decision.

---

## 8. Docs touched (this milestone)

- **NEW:** `deopt-v2-backend/docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` (this doc).
- **(no source touched)** — the cutover is operator-side config only.

`~/DEOPT/RUN_STATE.md` does not need a macro flip here: the
cutover is an operator step inside the post-V2G-GOV-F-B-X soak,
not a chain-side milestone. The §11 next-milestone table is
updated to reflect that FX-Q1 prep is closed when this doc lands.

---

## 9. Validations

The agent runs lightweight validations only — no chain, no backend
broadcast, no source touched.

```text
forge fmt --check    (sol)      : N/A (no .sol touched)
git diff --check     (sol)      : exit 0
git diff --check     (backend)  : exit 0
git status                      : new doc untracked in backend (expected;
                                  docs/ is git-tracked)
forge build                     : N/A
forge test                      : N/A
cargo test (backend)            : SKIPPED — no Rust source touched
```

---

## 10. Open follow-ups

| Tag | Item | Owner | Notes |
|---|---|---|---|
| FX-Q1-A | Execute §3 edit + §4 verification | Operator | Pre-requisite for all subsequent live broadcast. |
| FX-Q1-B | Run §6 dry-run; archive logs | Operator + SRE | Confirms signer identity end-to-end. |
| FX-Q1-C | Flip §6.5 broadcast flags after green dry-run | Operator | Backend restart required. |
| FX-Q1-D | Wire BACKEND_EXECUTOR_MONITORING_ALERTS_V1 alerts | SRE | Required before live smoke per §7.3. |
| FX-Q1-E | Implement BACKEND_GAS_FEES_REBATES_POLICY_V1 §10 T-1..T-10 | Backend | Required before mainnet equivalent. |
| FX-Q1-F | Decide whether to run a fee-only Sepolia smoke now or wait for V2G-GOV-G + rebateReserve allocation | Operator | Separate operator decision. |

---

## 11. Next milestone

The remaining Sepolia governance path:

```text
≥ 24h soak (since first X broadcast at 2026-06-06 15:45ish UTC)
   ↓
V2G-GOV-G  (Timelock cleanup + 2-step transfer to OPS_MULTISIG)
   ↓
Sepolia governance migration complete (protocol layer + executor layer + Timelock root)
```

The backend signer cutover (this doc) and V2G-GOV-G are
**independent** — the operator can do either order; recommended
to do the §3 edit + §6 dry-run inside the same soak window.

After both are closed:
- Backend live broadcast through BACKEND_EXECUTOR.
- Timelock owned by OPS_MULTISIG Safe.
- DEPLOYER fully retired from Timelock proposer / executor /
  guardian / owner.
- Mainnet rehearsal lessons captured (cutover key custody, ETA
  staleness, dependency graph).

---

## 12. Cross-links

- `~/DEOPT/deopt-v2-sol/docs/BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` —
  key custody policy. KMS-strong path; this runbook compromises to
  env-var for Sepolia rehearsal (and flags that compromise §13).
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md` —
  what the post-cutover backend must enforce per trade.
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` —
  signer-level alerts (`BE_BAL_LOW`, `BE_NOT_EXECUTOR`,
  `BE_CODE_NONZERO`, `BE_OOB_TX`, `BE_NONCE_GAP`) — wire these
  before flipping broadcast.
- `~/DEOPT/deopt-v2-sol/docs/GOVERNANCE_EXECUTOR_MIGRATION_EXECUTE_RESULT_V2G_GOV_F_B_X.md` —
  the chain milestone this cutover follows.
- `~/DEOPT/RESUME_GOV_F_B_X.md` — operator runbook for the chain
  side of GOV-F-B-X (already executed; left as a reference).

---

## 13. Sepolia → mainnet fork (raw-env → KMS lift)

For mainnet:
- `EXECUTOR_PRIVATE_KEY` env-var path **must NOT be used**. The
  backend will need a KMS-backed signing interface that returns
  signatures via a per-tx call without exposing the raw key — per
  `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` §2.1.
- The `.env` retains only the KMS handle (not the key material).
- The mainnet cutover runbook will document the KMS path; this
  Sepolia runbook is a compromise to unblock rehearsal.

This implementation gap is tracked at custody profile §9 BE-PROD-1.
