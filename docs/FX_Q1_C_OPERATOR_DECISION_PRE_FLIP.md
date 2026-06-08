# FX-Q1-C Operator Decision — Pre-Flip Authorisation

## 1. Identity & timestamp
- **Document ID:** FX_Q1_C_OPERATOR_DECISION_PRE_FLIP
- **Milestone:** FX-Q1-C — backend live-broadcast flag flip (option-execution)
- **Decision author:** Operator (single-actor Sepolia rehearsal)
- **Decision timestamp (UTC):** 2026-06-08T08:14:56Z
- **Document purpose:** Close PRE-5 (operator decision document) and record explicit WAIVE-FOR-SEPOLIA dispositions for PRE-1, PRE-2, PRE-3, PRE-4 with evidence, residual risk, and stop conditions.

## 2. Scope — Sepolia only
- **Chain:** Base Sepolia, chain id `84532`.
- **NO MAINNET.** This decision document does not authorise, contemplate, or carry forward to Base mainnet (chain id `8453`) or any other production network. Any mainnet flag flip requires a fresh decision document, full PRE-1..PRE-5 CLOSED (no waivers), and full monitoring/alerts/SRE coverage as specified in `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` §3/§4 and `FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_FX_Q1.md` §10.
- **Scope of flag flip authorised by this document:** the three-key `.env` change exactly as released by the FX-Q1-C-LIVE-BROADCAST-FLAG-FLIP-PACKET, followed by backend restart and `/executor/status` verification. Nothing more.

## 3. Canonical addresses (Base Sepolia)
- `BACKEND_EXECUTOR` = `0x295005fd4F311e6691F008D57d32FCFEde844518`
- `DEPLOYER`         = `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27`
- `NEW_OME`          = `0x5a5EBF9A9CCd7c012518569DE8283982982670f6`
- `OPS_MULTISIG`     = `0xA6B9Bb5c7B26B33cfD28C6F5A79B3c527fDdcD46`
- `TIMELOCK`         = `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588`
- `mUSDC`            = `0x6eAe407f5640B006faC9965182e238582A3B412E`
- `CV`               = `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3`
- `PFV`              = `0x7C0a3B6feBd5BFFc164f37738299AeB453181886`
- `NEW_FM_V2`        = `0xF6626177f3B85cc3239667Cc53C04A8007652944`
- `RG`               = `0x7918Ea95c2791B6b587fF02AE481FA52403877A0`

## 4. Pre-decision verification (read at 2026-06-08T08:14:56Z)

### 4.1 Backend `/executor/status`
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

### 4.2 Env startup gates (grep-only; no secret values written here)
| Key | Value |
|---|---|
| `EXECUTION_ENABLED` | `false` (pre-flip) |
| `EXECUTOR_DRY_RUN` | `true` (perp scaffold gate; STAY ON) |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | `false` (pre-flip) |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | UNSET → default `false` (pre-flip; will be ADDED at flip) |
| `OPTIONS_ENABLED` | `true` |
| `OPTION_EXECUTION_ENABLED` | `true` |
| `OPTION_EXECUTION_SIMULATION_ENABLED` | `true` |
| `EXECUTOR_REQUIRE_SIMULATION_OK` | `true` |
| `OPTION_EXECUTION_REQUIRE_SIMULATION_OK` | UNSET → default `true` |
| `SIMULATION_ENABLED` | `true` |
| `PERSISTENCE_ENABLED` | `true` |
| `OPTION_RFQ_ENABLED` | `true` |
| `OPTION_RFQ_REQUIRE_PERSISTENCE` | `true` |
| `EXECUTOR_CHAIN_ID` | `84532` |
| `CHAIN_ID` | `84532` |
| `EXECUTOR_FROM_ADDRESS` | BACKEND_EXECUTOR |
| `OPTION_EXECUTION_SIMULATION_FROM` | BACKEND_EXECUTOR |
| `OPTION_MATCHING_ENGINE_ADDRESS` | NEW_OME |
| `EXECUTOR_MAX_FEE_PER_GAS_WEI` | `1_000_000_000` (1 gwei) |
| `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI` | `1_000_000` (0.001 gwei) |
| `EXECUTOR_MAX_GAS_LIMIT` | `1_000_000` |
| `EXECUTOR_PRIVATE_KEY` | SET (len 66; value not recorded) |
| `RPC_URL` | SET (value not recorded) |
| `DATABASE_URL` | SET (value not recorded) |
| `ADMIN_TOKEN` | UNSET |

### 4.3 Chain safety checks (Base Sepolia)
- `BACKEND_EXECUTOR.code` = `0x` ✓ EOA
- `BACKEND_EXECUTOR.balance` = `3_800_000_000_000_000` wei (= `3.8e15`) — above FUND_FLOOR `1e15`, below FUND_TARGET `1e16`
- `BACKEND_EXECUTOR.nonce` = `0` ✓ (never broadcast)
- `NEW_OME.owner` = TIMELOCK ✓
- `NEW_OME.guardian` = OPS_MULTISIG ✓
- `NEW_OME.paused` = `false` ✓
- `NEW_OME.isExecutor(BACKEND_EXECUTOR)` = `true` ✓
- `NEW_OME.isExecutor(DEPLOYER)` = `false` ✓

### 4.4 R5 / PFV invariants
- `PFV.owner` = TIMELOCK ✓
- `NEW_FM_V2.owner` = TIMELOCK ✓
- `PFV.feeBalance(mUSDC)` = `28`
- `PFV.rebateReserve(mUSDC)` = `0`  ← rebate path structurally guarded
- `CV.balances(PFV, mUSDC)` = `28`
- **drift** = `CV − feeBalance − rebateReserve` = `28 − 28 − 0` = **`0`** ✓
- `NEW_FM_V2.feeRecipient` = PFV ✓
- `NEW_FM_V2.rebateFundingAccount` = PFV ✓
- `NEW_FM_V2.protocolFeeVault` = PFV ✓
- `NEW_FM_V2.rebateBudget(mUSDC)` = `999_947`
- `RG.feesManager` = NEW_FM_V2 ✓

## 5. Blocker table & dispositions

| ID | Blocker | Disposition |
|---|---|---|
| PRE-1 | BACKEND_EXECUTOR balance ≥ FUND_TARGET (`1e16` wei) | **WAIVED-FOR-SEPOLIA** |
| PRE-2 | Monitoring + alerts wired (PagerDuty + Discord) per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` | **WAIVED-FOR-SEPOLIA** |
| PRE-3 | `should_broadcast` §4.2 rebate-solvency hard backend gate implemented | **WAIVED-FOR-SEPOLIA** |
| PRE-4 | Separate SRE / on-call ack of flip + smoke window | **WAIVED-FOR-SEPOLIA** |
| PRE-5 | Operator decision document signed | **CLOSED by this document** |

### 5.1 PRE-1 waiver rationale — BE funding posture
- **Observed:** `BACKEND_EXECUTOR.balance = 3.8e15` wei = 0.0038 ETH. 3.8× FUND_FLOOR. 38% of FUND_TARGET.
- **Why not topped to TARGET:** DEPLOYER balance is `1.922538607392478e15` wei = ~0.00192 ETH. Topping BE from DEPLOYER cannot reach FUND_TARGET, and would in fact drain DEPLOYER below its own operational floor, creating a worse, asymmetric risk (DEPLOYER is needed for any future Timelock-owned ops requiring proposer/admin gas).
- **Why the current balance is sufficient for the authorised scope:**
  - Authorised scope is a single fee-only smoke. No reserve allocation, no rebate-bearing path.
  - Worst-case tx cost at configured caps: `EXECUTOR_MAX_GAS_LIMIT × EXECUTOR_MAX_FEE_PER_GAS_WEI = 1e6 × 1e9 = 1e15` wei per tx. `3.8e15 / 1e15 = 3.8` worst-case tx of headroom.
  - Realistic Base Sepolia settle cost: ~200–400k gas × ~0.001–0.01 gwei effective ≈ `2e11`–`4e12` wei per tx → practical headroom in the hundreds of tx range.
- **Residual risk:** if Sepolia base fee spikes, per-tx cost approaches the worst-case `1e15` wei bound, in which case a small burst of failed/retried broadcasts could drag BE toward FUND_FLOOR before the operator notices.
- **Mitigation:** halt threshold at `BE.balance < 2e15` wei (see §7).
- **Sepolia-only:** acceptable. Mainnet would require CLOSED, not waived.

### 5.2 PRE-2 waiver rationale — manual-watch in lieu of monitoring
- **Observed:** no PagerDuty integration, no Discord webhook, no synthetic alert firing has been verified for the FX-Q1-C broadcast surface.
- **Waiver:** operator commits to **active manual watch** during the entire authorised first-smoke window. Specifically:
  - Watch backend stdout/stderr (`/tmp/deopt_v2_backend.log` or the configured log target) in a tail.
  - Poll `/executor/status` no less than once per 30 seconds and on any user-driven action.
  - Poll `BACKEND_EXECUTOR.balance` (`cast balance`) before and after each broadcast.
  - Watch for the halt-condition signatures listed in §7.
- **Residual risk:** human attention failure (operator distraction, terminal closed, log rotation). Mitigated by short smoke window (single fee-only test) and pre-armed halt commands.
- **Sepolia-only:** acceptable. Mainnet would require fully wired, synthetic-tested alerts.

### 5.3 PRE-3 waiver rationale — chain backstop + fee-only scope
- **Backend `should_broadcast` rebate-solvency hard gate** (per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` §4.2) is documented but not implemented in code with a hard `return false` branch.
- **Chain-level backstop is sufficient for the authorised scope:**
  - `PFV.rebateReserve(mUSDC) = 0` (verified §4.4).
  - Any rebate-bearing candidate that hits `consumeFees` will revert with `InsufficientRebateReserve`. The chain enforces solvency regardless of the backend gate.
- **First smoke is FEE-ONLY by operator authorisation:**
  - No maker-rebate path will be exercised.
  - Any RFQ or orderbook fill with a non-zero rebate field is FORBIDDEN in the first smoke window.
- **Residual risk:** if a fee-only smoke were inadvertently constructed to include a rebate field (e.g., wrong fixture), broadcast would burn gas on a deterministically reverting tx. Cost: ~`1e12` wei per failed tx. Acceptable.
- **Sepolia-only:** acceptable. Mainnet would require the backend-side gate implemented and unit-tested before any rebate path is exercised.

### 5.4 PRE-4 waiver rationale — operator-as-on-call
- **No separate SRE or risk on-call** is staffed for this Sepolia rehearsal milestone.
- **Waiver:** operator acts as combined Operator + SRE + Risk on-call for the authorised smoke window. All halt decisions, all manual watch, all rollback actions are operator-executed.
- **Residual risk:** single-actor failure mode (operator incapacitated). Mitigated by the small scope and short authorised window.
- **Sepolia-only:** acceptable. Mainnet would require three-signature gate (Operator + SRE + Risk) per `FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_FX_Q1.md` §10.

### 5.5 PRE-5 — CLOSED
This document, signed by the operator at the timestamp in §1, **CLOSES PRE-5**. No further document is required to flip the FX-Q1-C broadcast flags within the scope defined in §2.

## 6. Authorised next action

**Only the FX-Q1-C flag flip** is authorised by this document. Concretely:

1. Backup `.env` to `.env.bak.fx_q1_c.<UTC-timestamp>` (mode 0600).
2. Edit exactly three keys in `.env`:
   - `EXECUTION_ENABLED=false` → `EXECUTION_ENABLED=true`
   - `EXECUTOR_REAL_BROADCAST_ENABLED=false` → `EXECUTOR_REAL_BROADCAST_ENABLED=true`
   - **add** `OPTION_EXECUTION_BROADCAST_ENABLED=true`
3. Keep `EXECUTOR_DRY_RUN=true` (perp scaffold gate; MUST stay on).
4. Keep all simulation flags `true`.
5. Graceful restart of backend.
6. Verify `/executor/status` matches the FX-Q1-C-VERIFY template.

## 7. Forbidden actions under this authorisation

- ❌ No live first smoke (separately authorised via `FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_FX_Q1.md`)
- ❌ No V2G-GOV-G (Timelock cleanup) — separate milestone
- ❌ No reserve allocation (`PFV.allocateRebateReserve` or analog)
- ❌ No governance mutation (ownership, guardian, Timelock roles, executor set, fee routing)
- ❌ No mainnet anything
- ❌ No trade construction or trade broadcast
- ❌ No `.env` edits beyond the three keys in §6
- ❌ No secret printing (private key, RPC URL, DATABASE_URL, admin token)
- ❌ No chain mutation other than what the post-flip backend may broadcast as part of the authorised flag-flip + verify scope (which, until a separate smoke authorisation, should produce zero broadcasts — broadcast capability is enabled but no fill is constructed)

## 8. Manual-watch duties during first smoke window

When the first smoke is later authorised, operator will:
- Keep a live tail of backend logs visible (continuous, not polled).
- Run `cast balance $BACKEND_EXECUTOR --rpc-url $RPC_URL` immediately before broadcast, immediately after, and at smoke close.
- Hit `/executor/status` immediately before broadcast and immediately after.
- Have the halt commands (§7 of FX-Q1-C-VERIFY template) pre-typed in a second terminal.

## 9. Halt thresholds — IMMEDIATELY stop and rollback if any of the following occurs

Trigger any of these → set `OPTION_EXECUTION_BROADCAST_ENABLED=false` + restart backend (or `OPS_MULTISIG → NEW_OME.pause()` in extremis).

| # | Trigger | Why |
|---|---|---|
| H1 | `BACKEND_EXECUTOR.balance < 2e15` wei | 2× FUND_FLOOR; gas runway exhaustion imminent. |
| H2 | Backend emits a transaction hash unexpectedly before live smoke is separately authorised | Implies a broadcast that was not in the authorised scope. |
| H3 | Backend log contains `eth_sendRawTransaction` for any tx not part of an authorised smoke | Same as H2. |
| H4 | Backend log contains `provider.send_raw_transaction` for any tx not part of an authorised smoke | Same as H2. |
| H5 | Backend log contains `InsufficientRebateReserve` (selector `0x91d23472` or analogous string) | A rebate-bearing path was attempted — violates §5.3 scope. |
| H6 | Backend log contains `NotAuthorized` (selector `0xea8e4eb5`) from on-chain revert | Executor authorisation has changed unexpectedly — chain state diverged from §4.3. |
| H7 | Backend log contains `InvalidSignature` from on-chain revert | Signer mismatch — possible misconfigured `EXECUTOR_PRIVATE_KEY` or wrong intent signing scheme. |
| H8 | `NEW_OME.paused` changes from `false` to `true` outside operator action | Guardian or owner has frozen the venue — investigate before any further action. |
| H9 | R5 drift `≠ 0` (drift = `CV.balances(PFV,mUSDC) − PFV.feeBalance − PFV.rebateReserve`) | Accounting invariant violated; halt all broadcast and investigate. |

## 10. Provenance & integrity
- This document is the only artifact created by FX-Q1-C-OPERATOR-DECISION-DOC-CLOSE.
- No `.env` edit performed.
- No chain mutation performed.
- No secrets written into this document.
- Path: `deopt-v2-backend/docs/FX_Q1_C_OPERATOR_DECISION_PRE_FLIP.md`.

**End of decision document.**
