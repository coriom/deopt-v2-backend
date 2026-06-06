# Backend live-broadcast flag flip runbook — V2G-FX-Q1-C

**Posture:** operator runbook. **The agent does NOT flip flags,
edit `.env`, broadcast on chain, or run live smoke.** Closes the
flag-flip preparation step of FX-Q1-C from the FX-Q1 stream.

**Scope:** Base Sepolia (chain 84532). Mainnet variant tracked at §10.

**Predecessors:**
- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia signer cutover.
- FX-Q1-A2 / FX-Q1-B / FX-Q1-B2 verifications: env clean, derived signer = BACKEND_EXECUTOR, dry-run path live, `/executor/status` shows all safety gates off.
- `GOVERNANCE_EXECUTOR_MIGRATION_EXECUTE_RESULT_V2G_GOV_F_B_X.md` — chain milestone this cutover follows.

**Anchors:**
- `BACKEND_EXECUTOR = 0x295005fd4F311e6691F008D57d32FCFEde844518`.
- `NEW_OME = 0x5a5EBF9A9CCd7c012518569DE8283982982670f6`.
- `NEW_OME.isExecutor(BACKEND_EXECUTOR) = true`.
- `NEW_OME.isExecutor(DEPLOYER) = false`.
- `PFV.rebateReserve(mUSDC) = 0` → no rebate-bearing trade possible.

---

## 0. Hard stops (this runbook)

```text
no flag flip by agent                          ✅
no chain tx                                    ✅
no live backend broadcast                      ✅
no RFQ smoke                                   ✅
no trade                                       ✅
no reserve allocation                          ✅
no GOV-G                                       ✅
no .env edit by agent                          ✅
no private key output                          ✅
no admin token output                          ✅
no secrets printed                             ✅
no mainnet                                     ✅
```

Operator performs §3 / §5 / §6 / §7 themselves.

---

## 1. Source-confirmed gate semantics

The option-execution live broadcast path enforces gating in **two
independent layers** — startup-time config validation AND runtime
request guard — so a half-flip is caught either way.

### 1.1 Startup-time validation — `validate_option_execution_broadcast_startup` (`src/config/env.rs:540-572`)

When `OPTION_EXECUTION_BROADCAST_ENABLED=true`, the backend
**refuses to start** unless ALL of:

```text
EXECUTION_ENABLED=true
EXECUTOR_REAL_BROADCAST_ENABLED=true
EXECUTOR_PRIVATE_KEY is non-empty
RPC_URL is non-empty
```

Otherwise startup errors with one of:
- `"OPTION_EXECUTION_BROADCAST_ENABLED=true requires EXECUTION_ENABLED=true"`
- `"OPTION_EXECUTION_BROADCAST_ENABLED=true requires EXECUTOR_REAL_BROADCAST_ENABLED=true"`
- `"EXECUTOR_PRIVATE_KEY is required when OPTION_EXECUTION_BROADCAST_ENABLED=true"`
- `"RPC_URL is required when OPTION_EXECUTION_BROADCAST_ENABLED=true"`.

### 1.2 ExecutionConfig validation — `ExecutionConfig::validate` (`src/execution/config.rs:80-135`)

When `EXECUTOR_REAL_BROADCAST_ENABLED=true`, the backend also
requires:

```text
PERSISTENCE_ENABLED=true
EXECUTOR_PRIVATE_KEY non-empty AND parseable
RPC_URL non-empty
EXECUTOR_MAX_FEE_PER_GAS_WEI non-empty
EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI non-empty
```

Otherwise startup errors before serving any traffic.

### 1.3 Runtime guard — `ensure_option_execution_broadcast_enabled` (`src/options/service.rs:2422-2447`)

On every option-execution broadcast intent, the request is rejected if ANY of these is false:
```text
state.options_config.execution_broadcast_enabled        (= OPTION_EXECUTION_BROADCAST_ENABLED)
state.options_config.enabled                            (= OPTIONS_ENABLED)
state.options_config.execution_enabled                  (= OPTION_EXECUTION_ENABLED)
state.execution_config.execution_enabled                (= EXECUTION_ENABLED)
state.execution_config.real_broadcast_enabled           (= EXECUTOR_REAL_BROADCAST_ENABLED)
```

Either layer alone is sufficient to block a live broadcast — both
together is defence in depth.

### 1.4 `EXECUTOR_DRY_RUN` semantics

**Surprising-but-correct:** `EXECUTOR_DRY_RUN` is consumed ONLY by
the **perp executor scaffold** (`src/execution/executor.rs:54-60`),
which **explicitly rejects real broadcast in any mode**:

```rust
if !self.config.dry_run {
    return Err(BackendError::Config(
        "real on-chain execution is not implemented yet; set EXECUTOR_DRY_RUN=true"
            .to_string(),
    ));
}
```

The **option-execution path does not consult `EXECUTOR_DRY_RUN` at
all** (grep returned zero hits in `options/service.rs`). The
option-execution live broadcast path is governed entirely by §1.1
+ §1.3 flags.

**Implication:** in FX-Q1-C, the operator MUST keep
`EXECUTOR_DRY_RUN=true`. Reasons:

- Option-execution live broadcast does not require it.
- If `EXECUTION_ENABLED=true` AND `EXECUTOR_DRY_RUN=false`, the
  **perp executor tick will error on every poll**
  (`"real on-chain execution is not implemented yet"`) — spamming
  logs and starving CPU.

So the live-broadcast posture for FX-Q1-C is: **option-execution live, perp dry-run.** `EXECUTOR_DRY_RUN=true` and `EXECUTOR_REAL_BROADCAST_ENABLED=true` co-existing is **not** contradictory — they gate different scaffolds.

### 1.5 Simulation-required flags

```text
EXECUTOR_REQUIRE_SIMULATION_OK=true           (already set)
OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true   (default true; unchanged)
OPTION_EXECUTION_SIMULATION_ENABLED=true      (already set)
SIMULATION_ENABLED=true                       (already set)
```

These remain `true` through the flip — they enforce that no
broadcast happens without a successful `eth_call` simulation
first.

---

## 2. Pre-flip checks (operator runs)

All read-only. None print secrets. None broadcast.

### 2.1 Chain side (cast)

```bash
cd ~/DEOPT/deopt-v2-sol
set -a; source .env.base-sepolia; set +a

NEW_OME=0x5a5EBF9A9CCd7c012518569DE8283982982670f6
BACKEND_EXECUTOR=0x295005fd4F311e6691F008D57d32FCFEde844518
DEPLOYER=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
PFV=0x7C0a3B6feBd5BFFc164f37738299AeB453181886
mUSDC=0x6eAe407f5640B006faC9965182e238582A3B412E

echo "isExecutor(BACKEND_EXECUTOR): $(cast call "$NEW_OME" 'isExecutor(address)(bool)' "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL")"
echo "isExecutor(DEPLOYER):         $(cast call "$NEW_OME" 'isExecutor(address)(bool)' "$DEPLOYER" --rpc-url "$RPC_URL")"
echo "NEW_OME.paused:               $(cast call "$NEW_OME" 'paused()(bool)' --rpc-url "$RPC_URL")"
echo "NEW_OME.owner:                $(cast call "$NEW_OME" 'owner()(address)' --rpc-url "$RPC_URL")"
echo "NEW_OME.guardian:             $(cast call "$NEW_OME" 'guardian()(address)' --rpc-url "$RPC_URL")"
echo "BACKEND_EXECUTOR.balance:     $(cast balance "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL") wei"
echo "BACKEND_EXECUTOR.code:        $(cast code "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL")"
echo "PFV.rebateReserve(mUSDC):     $(cast call "$PFV" 'rebateReserve(address)(uint256)' "$mUSDC" --rpc-url "$RPC_URL")"
```

Expected:

```text
isExecutor(BACKEND_EXECUTOR) = true
isExecutor(DEPLOYER)         = false
NEW_OME.paused               = false
NEW_OME.owner                = Timelock 0xa67f…b588
NEW_OME.guardian             = OPS_MULTISIG 0xA6B9Bb5c…cD46
BACKEND_EXECUTOR.balance     >= 1e15 wei (mandatory)
                             >= 1e16 wei (recommended; FUND_TARGET per
                                          BACKEND_EXECUTOR_CUSTODY_PROFILE §4)
BACKEND_EXECUTOR.code        = 0x
PFV.rebateReserve(mUSDC)     = 0   (rebate-bearing trade WILL revert; do not smoke rebate)
```

If `BACKEND_EXECUTOR.balance < 1e16`: top up before the flip per
`BACKEND_EXECUTOR_CUSTODY_PROFILE` §4.2. A `1e15` floor passes the
hard gate but leaves no headroom for a burst of trades. Recommended
to fund to `1e16` (≈ 0.01 ETH) before the flip; ceiling is `5e16`
(custody §4.1).

If any other check is off: **STOP — do not flip.** Re-run upstream verifies (FX-Q1-A2 / FX-Q1-B2).

### 2.2 Backend side (no `.env` cat, no secrets)

```bash
cd ~/DEOPT/deopt-v2-backend

# Confirm safety flags BEFORE flip
grep -E '^(EXECUTION_ENABLED|OPTIONS_ENABLED|OPTION_EXECUTION_ENABLED|EXECUTOR_DRY_RUN|EXECUTOR_REAL_BROADCAST_ENABLED|OPTION_EXECUTION_BROADCAST_ENABLED|EXECUTOR_FROM_ADDRESS|OPTION_EXECUTION_SIMULATION_FROM|OPTION_MATCHING_ENGINE_ADDRESS|EXECUTOR_CHAIN_ID|PERSISTENCE_ENABLED|EXECUTOR_MAX_FEE_PER_GAS_WEI|EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI|EXECUTOR_MAX_GAS_LIMIT|SIMULATION_ENABLED|EXECUTOR_REQUIRE_SIMULATION_OK|OPTION_EXECUTION_REQUIRE_SIMULATION_OK|OPTION_EXECUTION_SIMULATION_ENABLED)=' .env

# Required pre-flip state (snapshot):
#   EXECUTION_ENABLED=false                          ← will flip TRUE
#   OPTIONS_ENABLED=true
#   OPTION_EXECUTION_ENABLED=true
#   EXECUTOR_DRY_RUN=true                            ← STAYS TRUE (perp scaffold)
#   EXECUTOR_REAL_BROADCAST_ENABLED=false            ← will flip TRUE
#   OPTION_EXECUTION_BROADCAST_ENABLED=<unset/false> ← will flip TRUE
#   EXECUTOR_FROM_ADDRESS=0x295005fd4F311e6691F008D57d32FCFEde844518
#   OPTION_EXECUTION_SIMULATION_FROM=0x295005fd…4518
#   OPTION_MATCHING_ENGINE_ADDRESS=0x5a5EBF9A9CCd7c012518569DE8283982982670f6
#   EXECUTOR_CHAIN_ID=84532
#   PERSISTENCE_ENABLED=true
#   EXECUTOR_MAX_FEE_PER_GAS_WEI=<positive integer, e.g. 1000000000>
#   EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI=<positive integer, e.g. 1000000>
#   EXECUTOR_MAX_GAS_LIMIT>=1000000
#   SIMULATION_ENABLED=true
#   EXECUTOR_REQUIRE_SIMULATION_OK=true
#   OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true (or unset → default true)
#   OPTION_EXECUTION_SIMULATION_ENABLED=true

# Confirm derived signer (no key value printed)
DERIVED=$(cast wallet address --private-key "$EXECUTOR_PRIVATE_KEY")
[ "${DERIVED,,}" = "0x295005fd4f311e6691f008d57d32fcfede844518" ] && echo "derived == BE ✓" || echo "derived MISMATCH — STOP"
```

If `EXECUTOR_MAX_FEE_PER_GAS_WEI` OR `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI` is unset/empty: **STOP** — `ExecutionConfig::validate` refuses startup post-flip.

### 2.3 Backend status (pre-flip; no secrets)

```bash
curl -s http://127.0.0.1:8080/executor/status
```

Expected (pre-flip):
```json
{
  "executionEnabled": false,
  "dryRun": true,
  "realBroadcastEnabled": false,
  "persistenceRequired": true,
  "simulationEnabled": true,
  "simulationRequiresPersistence": true,
  "rpcConfigured": true,
  "broadcastEnabled": false
}
```

### 2.4 Operational prerequisites

```text
[ ] BACKEND_EXECUTOR_MONITORING_ALERTS_V1 §3 alerts wired to PagerDuty
[ ] BACKEND_EXECUTOR_MONITORING_ALERTS_V1 §4 alerts wired to Discord
[ ] BACKEND_GAS_FEES_REBATES_POLICY_V1 §10 T-1..T-10 implemented and unit-tested
    (rebate-solvency gate AT MINIMUM — even if the rest is not in
     yet — to avoid sending a rebate trade that PFV would revert)
[ ] On-call SRE acknowledged the flip window
[ ] Rollback steps in §6 rehearsed (operator can perform them in
    < 60 seconds)
[ ] Operator decision document recorded:
       - "FX-Q1-C flip authorised, dry-run logs reviewed, rebate-only
          smoke ruled out"
       - signed by Operator + SRE
```

---

## 3. Operator-only `.env` edit (the flip)

**Performed by the OPERATOR. The agent does NOT touch `.env`.**

### 3.1 Preconditions

```text
[ ] backend stopped (no inflight intents being processed mid-edit)
[ ] backup of current `.env` taken (operator may keep the
    pre-flip `.env` as `.env.bak.fx_q1_b2` for rollback;
    backup MUST be at least mode 0600 and on the same disk
    — do not copy it elsewhere if it carries the private key)
[ ] shell session does NOT have `set -x` / xtrace on
```

### 3.2 Edit (in `~/DEOPT/deopt-v2-backend/.env`)

```diff
-EXECUTION_ENABLED=false
+EXECUTION_ENABLED=true

-EXECUTOR_REAL_BROADCAST_ENABLED=false
+EXECUTOR_REAL_BROADCAST_ENABLED=true

# If OPTION_EXECUTION_BROADCAST_ENABLED is currently unset (using default false):
+OPTION_EXECUTION_BROADCAST_ENABLED=true
# If OPTION_EXECUTION_BROADCAST_ENABLED is currently set to false:
-OPTION_EXECUTION_BROADCAST_ENABLED=false
+OPTION_EXECUTION_BROADCAST_ENABLED=true
```

**Keep unchanged** (do NOT touch these lines):

```text
EXECUTOR_DRY_RUN=true                       ← MUST stay true (perp scaffold)
OPTION_EXECUTION_SIMULATION_ENABLED=true
OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true (or default)
EXECUTOR_REQUIRE_SIMULATION_OK=true
SIMULATION_ENABLED=true
EXECUTOR_FROM_ADDRESS=0x295005fd…4518
OPTION_EXECUTION_SIMULATION_FROM=0x295005fd…4518
OPTION_MATCHING_ENGINE_ADDRESS=0x5a5EBF9A…70f6
EXECUTOR_CHAIN_ID=84532
EXECUTOR_PRIVATE_KEY (value)                 ← never edit; never echo
EXECUTOR_MAX_FEE_PER_GAS_WEI                 (already set, e.g. 1e9)
EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI        (already set, e.g. 1e6)
EXECUTOR_MAX_GAS_LIMIT                       (e.g. 1000000)
PERSISTENCE_ENABLED=true
DATABASE_URL                                 (operator-managed)
RPC_URL                                      (operator-managed; embedded API key)
OPTIONS_ENABLED=true
OPTION_EXECUTION_ENABLED=true
```

### 3.3 Edit hygiene

```text
[ ] no `cat .env` after editing
[ ] no `grep EXECUTOR_PRIVATE_KEY .env` after editing
[ ] do not paste the edited `.env` into chat, ticket, or LLM
[ ] do not check the edited `.env` into git (it is gitignored;
    confirm with `git check-ignore .env`)
[ ] backup file `.env.bak.fx_q1_b2`:
       option A: shred after rollback window closes (≥ 24 h soak)
       option B: encrypt at rest with age / gpg if retained longer
```

### 3.4 Restart backend

```bash
cd ~/DEOPT/deopt-v2-backend
# Stop running backend (Ctrl-C if foreground, OR systemctl stop / pkill the process)
cargo run -p deopt-v2-backend --release
# Or: ./target/release/deopt-v2-backend
```

If startup logs any of the §1.1 / §1.2 error strings, the edit
is incomplete — STOP, fix `.env`, restart. Do NOT proceed.

---

## 4. Post-flip checks (operator runs)

### 4.1 Backend status (no secrets)

```bash
curl -s http://127.0.0.1:8080/executor/status
```

Expected (post-flip):
```json
{
  "executionEnabled": true,
  "dryRun": true,
  "realBroadcastEnabled": true,
  "persistenceRequired": true,
  "simulationEnabled": true,
  "simulationRequiresPersistence": true,
  "rpcConfigured": true,
  "broadcastEnabled": true
}
```

Hard gates to confirm:
- `executionEnabled = true` ← FLIPPED
- `realBroadcastEnabled = true` ← FLIPPED
- `broadcastEnabled = true` ← FLIPPED (this is the option-execution broadcast aggregate flag)
- `dryRun = true` ← UNCHANGED (perp scaffold; expected)
- `simulationEnabled = true` ← UNCHANGED
- `rpcConfigured = true` ← UNCHANGED

If any of `realBroadcastEnabled` / `broadcastEnabled` / `executionEnabled` is `false`, the flip did not take effect — STOP, debug `.env`, restart. **No live trade has been broadcast at this point.**

### 4.2 Admin summary (operator-only; agent never sees admin token)

If the operator wants to confirm the signer surface beyond the env-derive comparison:

```bash
# Operator-only; admin token is operator's; agent never receives it.
curl -s -H "Authorization: Bearer <REDACTED_ADMIN_TOKEN>" \
  http://127.0.0.1:8080/admin/execution/summary | jq '{
    contracts: .contracts,
    configured: .configured,
    execution: .execution
  }'
```

Expected (relevant fields):
```text
contracts.executor_from_address       = 0x295005fd…4518   (lowercased)
contracts.option_matching_engine_address = 0x5a5EBF9A…70f6
configured.executor_private_key       = true
configured.rpc                        = true
execution.dry_run                     = true               (perp scaffold)
execution.executor_chain_id           = 84532
execution.require_simulation_ok       = true
execution.max_fee_per_gas_configured  = true
execution.max_priority_fee_per_gas_configured = true
```

No private key value, no key bytes, no admin token are printed by either endpoint.

### 4.3 Chain re-read (no mutation)

```bash
# Same chain checks as §2.1 — values MUST be unchanged.
cast call "$NEW_OME" 'isExecutor(address)(bool)' "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL"  # true
cast call "$NEW_OME" 'paused()(bool)' --rpc-url "$RPC_URL"                                  # false
cast balance "$BACKEND_EXECUTOR" --rpc-url "$RPC_URL"                                       # >= 1e16 wei
cast call "$PFV" 'rebateReserve(address)(uint256)' "$mUSDC" --rpc-url "$RPC_URL"           # 0 (rebate gate ACTIVE)
```

### 4.4 Log signals on first intent post-flip

When the operator submits a fee-only (non-rebate) intent, logs MUST show:

```text
event   = "execution_simulation"
result  = "ok"
signer  = "0x295005fd4f311e6691f008d57d32fcfede844518"   ← BACKEND_EXECUTOR
from    = "0x295005fd4f311e6691f008d57d32fcfede844518"   ← BACKEND_EXECUTOR
target  = "0x5a5EBF9A9CCd7c012518569DE8283982982670f6"   ← NEW_OME
chain_id = 84532
intent_id = "<uuid>"

event   = "execution_broadcast"    (or equivalent — after the broadcast actually fires)
tx_hash = "0x..."                  (now PRESENT — this is the live broadcast confirmation)
nonce   = <BACKEND_EXECUTOR's onchain nonce>
result  = "ok" | "reverted"
```

Confirm:
- `signer == from == BACKEND_EXECUTOR` (no half-flip drift).
- `tx_hash` is a real chain hash (NOT 0x000…000); operator can `cast receipt $tx_hash` to confirm `status=1`.
- Log lines contain NO 64+ char `0x`-prefixed hex string that looks like a private key.

### 4.5 First-trade monitoring window

For the first 30 min after the flip:

```text
[ ] tail backend logs continuously
[ ] watch BACKEND_EXECUTOR_MONITORING_ALERTS_V1 §3 alerts
[ ] watch BE balance (should decrement by gas spent per tx)
[ ] watch NEW_OME.isExecutor(BE) (must stay true)
[ ] watch PFV.rebateReserve (must stay 0; backend gates rebate trades)
[ ] keep a clean shell open for the §6 rollback procedure
```

---

## 5. Smoke prerequisites (do NOT authorise smoke here)

This runbook flips the flags. It does NOT authorise live smoke.
Smoke is a separate operator decision per
`BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` §7.

### 5.1 Smoke regime allowed

```text
fee-only orderbook trade  : ALLOWED (makerPpm >= 0; takerPpm > 0)
fee-only RFQ trade        : ALLOWED (no negative effective_maker_ppm)
simulation-only           : ALLOWED at any time
```

### 5.2 Smoke regime FORBIDDEN

```text
rebate-bearing orderbook  : FORBIDDEN — PFV.rebateReserve=0; will revert
                                       at the PFV hook (onRebatePaid).
rebate-bearing RFQ        : FORBIDDEN — same reason.
liquidation trade         : OUT OF SCOPE (separate gate; see policy §7).
zero-economic-content     : FORBIDDEN — should_broadcast §6 rejects.
```

### 5.3 Pre-smoke hard gates (operator runs before any smoke)

```text
[ ] §4.1 post-flip status JSON all green
[ ] §4.4 first dry-run-after-flip intent shows the expected log signals
[ ] BE balance ≥ 1e16 wei (FUND_TARGET) — survives the burst
[ ] all §2.4 operational prerequisites still met
[ ] separate "smoke authorisation" decision document signed by Operator + Risk
```

### 5.4 What to do BEFORE first smoke

Even after the flag flip, the safe sequence is:
1. Submit one simulation-only intent (no actual signing — just `eth_call` from BE).
2. Watch logs / status / metrics for 5 min.
3. If anything looks off, rollback (§6).
4. Submit one fee-only orderbook smoke trade. Watch.
5. Submit one fee-only RFQ smoke trade. Watch.
6. Soak at low volume for ≥ 24 h before opening up.

**This runbook does NOT authorise any of step 4-6.** Those gates
live in a separate "smoke authorisation" runbook (TODO; not part
of this milestone).

---

## 6. Rollback procedure

If anything goes wrong post-flip — chain revert, log anomaly, BE
draining unexpectedly, monitoring alert — perform this rollback in
< 60 seconds.

### 6.1 Stop backend immediately (if possible)

```bash
# Ctrl-C the foreground process
# Or: pkill -SIGTERM deopt-v2-backend
# Or: systemctl stop deopt-v2-backend
```

If a tx is already inflight, stopping the backend will NOT cancel
the inflight tx — chain will still execute it (or revert it). Tail
`/executor/transactions` to confirm the final tx count before
stopping.

### 6.2 Flip flags back

```diff
-EXECUTION_ENABLED=true
+EXECUTION_ENABLED=false

-EXECUTOR_REAL_BROADCAST_ENABLED=true
+EXECUTOR_REAL_BROADCAST_ENABLED=false

-OPTION_EXECUTION_BROADCAST_ENABLED=true
+OPTION_EXECUTION_BROADCAST_ENABLED=false
```

Alternatively, swap with the §3 backup:

```bash
# operator-only; only if .env.bak.fx_q1_b2 was kept under proper hygiene
cp ~/DEOPT/deopt-v2-backend/.env.bak.fx_q1_b2 ~/DEOPT/deopt-v2-backend/.env
```

### 6.3 Restart backend

```bash
cargo run -p deopt-v2-backend --release
```

### 6.4 Confirm rollback

```bash
curl -s http://127.0.0.1:8080/executor/status
```

Expected:
```json
{
  "executionEnabled": false,
  "dryRun": true,
  "realBroadcastEnabled": false,
  "broadcastEnabled": false,
  ...
}
```

If `realBroadcastEnabled` or `broadcastEnabled` still `true`, the
rollback `.env` edit didn't land — STOP, fix manually, restart.
Backend is in dry-run / off mode again; no further broadcast can
fire.

### 6.5 Emergency chain-side circuit breaker (sub-second freeze)

If the compromise is BE-level (rogue tx, key leak), use the
guardian fast-pause:

```text
OPS_MULTISIG Safe-tx:
  to:    NEW_OME 0x5a5EBF9A…70f6
  data:  cast calldata 'pause()' (selector 0x8456cb59 — verify against source)
  caller: OPS_MULTISIG (onlyGuardianOrOwner; guardian is OPS_MULTISIG)
```

Pause freezes the trade hot path in < 1 minute. After pause, follow
the §7 compromise response in `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md`.

### 6.6 After rollback

Either:
- Re-enter §3 with the lesson encoded (e.g., bump gas, add monitoring), OR
- Defer the flip indefinitely; backend stays in dry-run.

---

## 7. Open follow-ups (post-flip)

| Tag | Item | Owner | Notes |
|---|---|---|---|
| FX-Q1-C-1 | First fee-only simulation-only intent post-flip | Operator | Confirms broadcast flag affects the path correctly. |
| FX-Q1-C-2 | Smoke authorisation runbook (separate doc) | Operator + Risk | One-time decision; rebate-only smoke is FORBIDDEN until `PFV.allocateToRebateReserve`. |
| FX-Q1-C-3 | First fee-only orderbook smoke (after FX-Q1-C-2 signed) | Operator | Watch §4.5 monitoring window. |
| FX-Q1-C-4 | First fee-only RFQ smoke | Operator | After §FX-Q1-C-3 soaked. |
| FX-Q1-D | Monitoring + alerts wired per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` | SRE | Required before §FX-Q1-C-3. |
| FX-Q1-E | Backend execution-policy unit tests per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` §10 | Backend | Required before mainnet equivalent. |

---

## 8. Validations (agent-side, lightweight)

```text
forge fmt --check     (sol)     : N/A (no .sol touched)
git diff --check      (sol)     : exit 0
git diff --check      (backend) : exit 0
cargo check --release (backend) : confirmed clean at FX-Q1-B (11 s)
.env edits by agent             : NONE ✅
secrets printed                 : NONE ✅
admin token output              : NONE ✅
backend started by agent        : NO ✅
backend live broadcast          : NONE ✅
chain probes                    : read-only ✅
```

---

## 9. Cross-links

- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia signer cutover (parent runbook).
- `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` — key custody / funding / monitoring / rotation / compromise.
- `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` — broadcast economics; `should_broadcast` policy.
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — what monitoring must catch in the post-flip window.
- `GOVERNANCE_EXECUTOR_MIGRATION_EXECUTE_RESULT_V2G_GOV_F_B_X.md` — chain milestone this cutover follows.
- `GOVERNANCE_TIMELOCK_CLEANUP_PREP_V2G_GOV_G_PREP.md` — chain milestone GOV-G; independent of FX-Q1-C.

---

## 10. Sepolia → mainnet fork

The mainnet variant `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_Y_MAINNET.md`
must additionally:

- Replace `EXECUTOR_PRIVATE_KEY` env-var path with a KMS-backed signing service per `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` §2.1. The flip then includes wiring the KMS handle, not pasting a raw key.
- Require operator + risk + audit sign-off on the smoke authorisation document.
- Tighten gas envs to mainnet economics.
- Use a separate `BACKEND_EXECUTOR` from the Sepolia one (distinct address + KMS region).
- Run a full compromise drill on Sepolia first (per custody §9 BE-PROD-7).
- Audit + sign-off before any mainnet broadcast.

This implementation gap is tracked at custody profile §9 BE-PROD-1.
