# First live smoke — authorization document — V2G-FX-Q1

**Posture:** authorization document **template**. The agent
produces this artifact. **The act of authorising the smoke is
operator + risk + SRE signature on §10 below — NOT this doc's
existence.** Until §10 is signed, no live smoke is authorised.

**Scope:** Base Sepolia (chain 84532) ONLY. Mainnet is out of scope
and tracked at §13.

**Anchors:**
- `BACKEND_EXECUTOR = 0x295005fd4F311e6691F008D57d32FCFEde844518`
- `NEW_OME = 0x5a5EBF9A9CCd7c012518569DE8283982982670f6`
- `PFV = 0x7C0a3B6feBd5BFFc164f37738299AeB453181886`
- `NEW_FM_V2 = 0xF6626177f3B85cc3239667Cc53C04A8007652944`
- `CV = 0x00340C360353a5AB784c5Bc5c44322A6AF0625D3`
- `RG = 0x7918Ea95c2791B6b587fF02AE481FA52403877A0`
- `mUSDC = 0x6eAe407f5640B006faC9965182e238582A3B412E`
- `Timelock = 0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588`
- `OPS_MULTISIG = 0xA6B9Bb5c7B26B33cfD28C6F5A79B3c527fDdcD46`

**Predecessors (all green or noted):**
- V2G-GOV-F-B-X closed; D-5 closed.
- FX-Q1-A2 signer cutover verified.
- FX-Q1-B2 dry-run verified.
- FX-Q1-C live-broadcast flag flip runbook prepared but **not yet
  executed** at the time this template is drafted; see §3 hard
  gate FX-SMOKE-PRE-1.

---

## 0. Hard stops (this document)

```text
no chain tx                                  ✅
no live backend broadcast                    ✅
no RFQ / orderbook smoke                     ✅
no trade                                     ✅
no reserve allocation                        ✅
no GOV-G                                     ✅
no .env edit by agent                        ✅
no flag flip by agent                        ✅
no private key output                        ✅
no admin token output                        ✅
no secrets printed                           ✅
no mainnet                                   ✅
chain restricted to 84532                    ✅
```

The agent's role is to *propose* the authorization. The operator
sign-off in §10 is the *authorization itself*. Even after
authorization, the agent does not run the smoke — the operator
does.

---

## 1. Why "first live smoke" is a distinct gate

The FX-Q1 stream so far has moved the executor surface on chain
(V2G-GOV-F-B-X), then re-aligned the backend signer (FX-Q1-A2),
then verified dry-run end-to-end (FX-Q1-B2), then prepared (but
not flipped) live-broadcast flags (FX-Q1-C). None of that has put
a single live `executeTrade` / `executeRfqTrade` on chain through
the new BACKEND_EXECUTOR.

First live smoke is the **first chain mutation by BE** since BE
was bound as executor. It exercises:

- The post-cutover signer identity end-to-end (sign → send → mine).
- `NEW_OME.executeTrade` (or `executeRfqTrade`) `onlyExecutor` path.
- `FM-V2.consumeFees` → `PFV.onFeeCharged` accounting hook.
- Backend's `should_broadcast` decision (which must reject any
  rebate-bearing candidate while `PFV.rebateReserve = 0`).
- Indexer pickup + reconciliation of the new BE-signed tx.
- Monitoring alert routing for at least one real event.

Failure modes here are silently expensive (gas burned for nothing)
or loud (chain revert that pages on-call). The §10 authorization
exists so the operator commits, on the record, to the scope, the
prerequisites, and the rollback plan.

---

## 2. Allowed first-smoke scope

### 2.1 Always allowed (no §10 sign-off required)

| Smoke shape | Notes |
|---|---|
| Simulation-only (`eth_call` from BE; no signing, no submit) | At any time, any candidate shape (including rebate-bearing — sim does not touch `rebateReserve`). |

### 2.2 Allowed after §10 sign-off

| Smoke shape | Required conditions |
|---|---|
| Fee-only orderbook trade — **`executeTrade`** | `makerPpm ≥ 0` AND `takerPpm ≥ 1`; `effective_maker_amount ≥ 0` (no rebate); single trade (one buyer signature + one seller signature); notional small enough that `gross_fee_revenue ≥ gas_cost × 1.5` per gas/fees/rebates policy §4 PROFITABLE. |
| Fee-only RFQ trade — **`executeRfqTrade`** | RFQ-discounted but with `effective_maker_ppm ≥ 0` (no negative maker after discount); single trade; same PROFITABLE gate; same `result=ok` simulation gate. |

### 2.3 Hard limits within allowed scope

```text
trade size           : ≤ 100 mUSDC notional (~0.0001 ETH gas equiv at Sepolia)
trade count          : 1 orderbook smoke AND 1 RFQ smoke maximum in
                       the first smoke session
inter-trade gap      : ≥ 10 minutes between trades (per §7 watch
                       window)
session duration     : ≤ 30 minutes from first send to "smoke
                       window closed"
session count        : 1 smoke session per UTC day until §11 cleared
```

---

## 3. Forbidden smoke scope (any one is an automatic STOP)

### 3.1 Rebate-bearing trades — FORBIDDEN

```text
rebate-bearing orderbook  : FORBIDDEN
  - any candidate with makerPpm < 0
  - any candidate where effective_maker_amount (after discount) < 0
  - Reason: PFV.rebateReserve(mUSDC) = 0; PFV.onRebatePaid would
    revert InsufficientRebateReserve at FM-V2.consumeFees time.
    Gas burned for nothing AND log noise that masks real signals.

rebate-bearing RFQ        : FORBIDDEN
  - same reasons.
```

The backend's `should_broadcast` §4.2 hard gate per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` will reject these. The
PFV-side hook revert is the structural backstop. Operator MUST
NOT bypass either gate.

### 3.2 Liquidation — FORBIDDEN for first smoke

```text
liquidation trade : FORBIDDEN in the first smoke session.
                    Liquidation is a separate decision-state in the
                    gas/fees/rebates policy §7 with its own subsidy
                    budget. First smoke is a baseline trade through
                    the new signer — keep it boring.
```

### 3.3 Size and batch — FORBIDDEN

```text
high-size trade      : > 100 mUSDC notional → FORBIDDEN
multi-trade batch    : any batch >1 → FORBIDDEN (no setExecutors-style
                       fan-out; no atomic multi-trade calldata)
back-to-back trades  : < 10 minute gap → FORBIDDEN
```

### 3.4 Cross-environment / cross-flow — FORBIDDEN

```text
mainnet action                      : FORBIDDEN (out of scope)
any reserve allocation              : FORBIDDEN (separate milestone)
any guardian/owner/proposer change  : FORBIDDEN (out of scope)
any pause/unpause                   : FORBIDDEN unless emergency §8
any GOV-G chain action              : FORBIDDEN (separate parallel track)
any non-NEW_OME tx from BE          : FORBIDDEN (custody §1.2 rule)
```

---

## 4. Prerequisites before first live smoke

Every line below MUST be green at smoke time. The §10 sign-off
records the operator's attestation that they have personally
verified each.

### 4.1 Chain state

```text
[ ] NEW_OME.owner()                  = Timelock 0xa67f…b588
[ ] NEW_OME.guardian()               = OPS_MULTISIG 0xA6B9Bb5c…cD46
[ ] NEW_OME.paused()                 = false
[ ] NEW_OME.isExecutor(BE)           = true
[ ] NEW_OME.isExecutor(DEPLOYER)     = false
[ ] BE.code                          = 0x   (EOA)
[ ] BE.balance                       ≥ 1e16 wei (~ 0.01 ETH)
                                       NOTE: FUND_TARGET, not just
                                       FUND_FLOOR. First-smoke must
                                       not run BE below floor.
[ ] PFV.owner()                      = Timelock
[ ] NEW_FM_V2.owner()                = Timelock
[ ] PFV.feeBalance(mUSDC)            = pre-smoke baseline (record)
[ ] PFV.rebateReserve(mUSDC)         = 0    (rebate gate ACTIVE)
[ ] CV.balances(PFV, mUSDC)          = pre-smoke baseline (record)
[ ] **drift** = CV − fee − reserve   = 0    (invariant)
[ ] NEW_FM_V2.protocolFeeVault       = PFV
[ ] NEW_FM_V2.feeRecipient           = PFV
[ ] NEW_FM_V2.rebateFundingAccount   = PFV
[ ] NEW_FM_V2.rebateBudget(mUSDC)    = 999 947  (or current; record baseline)
[ ] RG.feesManager                   = NEW_FM_V2
```

### 4.2 Backend state

```text
[ ] FX-Q1-C live-broadcast flag flip COMPLETED and VERIFIED
    (per BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK §4.1 — agent
    will not skip-ahead this step)
[ ] /executor/status response:
       executionEnabled        = true
       dryRun                  = true                (perp scaffold)
       realBroadcastEnabled    = true
       broadcastEnabled        = true   (option live)
       simulationEnabled       = true
       rpcConfigured           = true
       persistenceRequired     = true
[ ] backend logs show no panic, no startup error
[ ] derived signer == BACKEND_EXECUTOR (no key drift)
[ ] BACKEND_EXECUTOR_MONITORING_ALERTS_V1 §3 paging alerts
    wired AND firing-ready in PagerDuty (synthetic-fired at
    least once)
[ ] BACKEND_EXECUTOR_MONITORING_ALERTS_V1 §4 warning alerts
    wired AND tested in Discord
[ ] should_broadcast §4.2 rebate-solvency gate IMPLEMENTED
    (minimum from gas/fees/rebates policy §10 T-5)
[ ] dedupe cache + per-address nonce window implemented
    (T-3)
[ ] per-tx log structure per monitoring spec §7
```

### 4.3 Human prerequisites

```text
[ ] On-call SRE acknowledged the smoke window (Slack/Discord
    ack with timestamp)
[ ] On-call risk acknowledged scope (rebate-bearing FORBIDDEN
    confirmed)
[ ] Operator decision document (§10 of this doc) SIGNED:
       - Operator (named human)
       - SRE on-call (named human)
       - Risk on-call (named human)
[ ] Rollback shell open with BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK
    §6 procedure pre-loaded
[ ] OPS_MULTISIG signers reachable (≥ 2 of 3 on Sepolia) for
    emergency NEW_OME.pause() per §6.5 of flag flip runbook
[ ] No concurrent V2G-GOV-G chain action in flight (chain-side
    track is independent; just confirm not running)
```

### 4.4 Read-only verification commands

```bash
# Chain side
cd ~/DEOPT/deopt-v2-sol
set -a; source .env.base-sepolia; set +a

NEW_OME=0x5a5EBF9A9CCd7c012518569DE8283982982670f6
BE=0x295005fd4F311e6691F008D57d32FCFEde844518
DEPLOYER=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
PFV=0x7C0a3B6feBd5BFFc164f37738299AeB453181886
FMV2=0xF6626177f3B85cc3239667Cc53C04A8007652944
CV=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
RG=0x7918Ea95c2791B6b587fF02AE481FA52403877A0
mUSDC=0x6eAe407f5640B006faC9965182e238582A3B412E

for cmd in \
  "isExecutor(BE)|$NEW_OME|isExecutor(address)(bool)|$BE" \
  "isExecutor(DEPLOYER)|$NEW_OME|isExecutor(address)(bool)|$DEPLOYER" \
  "paused|$NEW_OME|paused()(bool)|" \
  "owner|$NEW_OME|owner()(address)|" \
  "guardian|$NEW_OME|guardian()(address)|" \
  "BE.balance||(balance)|$BE" \
  "BE.code||(code)|$BE" \
  "feeBalance|$PFV|feeBalance(address)(uint256)|$mUSDC" \
  "rebateReserve|$PFV|rebateReserve(address)(uint256)|$mUSDC" \
  "CV.balances|$CV|balances(address,address)(uint256)|$PFV $mUSDC" \
  "rebateBudget|$FMV2|rebateBudget(address)(uint256)|$mUSDC" \
  "FM-V2.PFV|$FMV2|protocolFeeVault()(address)|" \
  "FM-V2.feeRecipient|$FMV2|feeRecipient()(address)|" \
  "FM-V2.rebateFundingAccount|$FMV2|rebateFundingAccount()(address)|" \
  "RG.feesManager|$RG|feesManager()(address)|" \
; do
  label=$(echo "$cmd" | cut -d'|' -f1)
  target=$(echo "$cmd" | cut -d'|' -f2)
  sig=$(echo "$cmd" | cut -d'|' -f3)
  args=$(echo "$cmd" | cut -d'|' -f4)
  if [ "$sig" = "(balance)" ]; then
    val=$(cast balance $args --rpc-url "$RPC_URL")
  elif [ "$sig" = "(code)" ]; then
    val=$(cast code $args --rpc-url "$RPC_URL")
  else
    val=$(cast call "$target" "$sig" $args --rpc-url "$RPC_URL")
  fi
  printf "  %-30s = %s\n" "$label" "$val"
done

# Backend side (no .env cat; no secret echoed)
curl -s http://127.0.0.1:8080/executor/status
```

If ANY line is off-expectation: **STOP and re-enter FX-Q1-C verify**.

---

## 5. Exact first-smoke sequence

The operator runs this sequence after §10 sign-off. Stop on any §6
trigger.

### 5.1 SMOKE-A — simulation-only orderbook candidate

```text
purpose       : confirm the broadcast path's simulation gate fires
                with the new BE signer, before any signing/submit
operator      : back-end matching service simulates one orderbook
                candidate via the option-execution intent flow
expected log  : event=execution_simulation result=ok
                signer=from=0x295005fd…4518
                target=0x5a5EBF9A…70f6
                chain_id=84532
                tx_hash absent
duration      : single-call
watch         : 5 minutes
acceptance    : log signals match expectation; no tx hash; no
                anomaly in §6 stop list
on-failure    : STOP. Do not proceed to SMOKE-B.
```

### 5.2 5-minute watch window

```text
[ ] /executor/status unchanged from §4.2 expected shape
[ ] no PagerDuty alert
[ ] BE.balance unchanged (simulation does not spend gas on chain;
    only eth_call)
[ ] PFV / FM-V2 / CV / RG state unchanged (sim has no side effects)
[ ] backend logs show no panic, no unexpected event
```

If clean → proceed. If anything off → STOP and exit smoke session
without further trades.

### 5.3 SMOKE-B — fee-only orderbook live trade

```text
purpose       : first live broadcast through new BE for orderbook flow
operator      : back-end matching service submits one fee-only
                orderbook candidate (makerPpm≥0, no rebate) of
                ≤ 100 mUSDC notional
expected on-chain events (per receipt):
  - log on NEW_OME:
      topic0 = OptionTradeExecuted
             = 0xb2387b9f0e4823ecef9a16ea4aaba6598c0703fb5e9d8dba37ef303add4cb808
      topic1 = <intentId>
      topic2 = <buyer address, left-padded>
      topic3 = <seller address, left-padded>
  - log on PFV:
      topic0 = FeeRecorded
             = 0x8a6211fcbaec33871f06dc695956ebd0159a99db86160410d1d22fd13ecc7fa8
      topic1 = mUSDC (left-padded)
      data   = abi.encode(fee_amount_in_mUSDC)
  - NO RebateRecorded log MUST appear:
      RebateRecorded topic0 = 0x5f501d5b62d984e8de6ff0a98099d44e5367f633b435cb272d45d9024e7c29cf
      If this topic appears on PFV, STOP immediately — rebate path
      was reached despite scope (this would also have reverted at
      onRebatePaid because reserve=0, but seeing the topic at all
      is a hard stop).
gas estimate  : worst-case ~200 000 gas at 1 gwei = ~0.2 µETH (Sepolia)
duration      : single tx; mined within ~2 blocks
watch         : 10 minutes
```

#### 5.3.1 SMOKE-B post-trade verification

```text
[ ] receipt status = 1
[ ] receipt.from   = BACKEND_EXECUTOR
[ ] receipt.to     = NEW_OME
[ ] gasUsed        < EXECUTOR_MAX_GAS_LIMIT (1 000 000)
[ ] OptionTradeExecuted log present with expected intentId
[ ] FeeRecorded log present on PFV with positive amount
[ ] NO RebateRecorded log on PFV
[ ] no other unexpected logs (e.g., no Paused, no GuardianSet, no
    ExecutorSet, no OwnershipTransferred)
```

#### 5.3.2 PFV / R5 invariant check post-SMOKE-B

```text
let Δfee     = post_PFV.feeBalance(mUSDC) − pre_PFV.feeBalance(mUSDC)
let Δreserve = post_PFV.rebateReserve(mUSDC) − pre_PFV.rebateReserve(mUSDC)
let ΔcvBal   = post_CV.balances(PFV, mUSDC) − pre_CV.balances(PFV, mUSDC)

[ ] Δfee        > 0           (matches FeeRecorded log amount)
[ ] Δreserve   == 0           (no rebate path reached)
[ ] ΔcvBal     == Δfee         (settlement-asset side and PFV side agree)
[ ] drift = (CV − fee − reserve) = 0   (invariant 2)
[ ] PFV.owner / FM-V2.owner / RG.feesManager UNCHANGED
[ ] NEW_OME.owner / guardian / paused UNCHANGED
[ ] BE.balance dropped by ≈ gas_cost (no anomalous drain)
[ ] FM-V2.rebateBudget(mUSDC) UNCHANGED (no rebate consumed)
```

#### 5.3.3 Indexer + reconciliation

```text
[ ] indexer picked up OptionTradeExecuted within target latency
    (per V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M SLO)
[ ] indexer's recorded fee_amount == FeeRecorded log data
[ ] indexer's recorded buyer/seller == log topics 2/3
[ ] reconciliation job (if scheduled) reconciles cleanly:
       backend.expected_fee == chain.actual_fee (within rounding)
       backend.expected_pnl_in_asset ≈ chain.actual_pnl_in_asset
       (gas cost from receipt + fee revenue from log)
[ ] no reconciliation alert fires
```

### 5.4 10-minute watch window

```text
[ ] /executor/status unchanged from §4.2 expected
[ ] no PagerDuty alert
[ ] no Discord warning alert spike
[ ] BE.balance steady (no further txs unless another candidate
    queued and accepted)
[ ] PFV/FM-V2/CV/RG invariants steady at post-SMOKE-B values
[ ] backend not retrying / re-broadcasting the same intent
    (dedupe cache hit on any repeat)
```

If clean → optionally proceed to SMOKE-C. **Operator may choose to
end the session here.** Doing one orderbook smoke per session is a
valid "first smoke" outcome.

### 5.5 SMOKE-C — optional fee-only RFQ live trade

```text
purpose       : confirm RFQ-flow executor path through new BE
operator      : back-end matching service submits one fee-only
                RFQ candidate (effective_maker_ppm ≥ 0) of
                ≤ 100 mUSDC notional
expected event:
  - log on NEW_OME:
      topic0 = OptionRfqTradeExecuted
             = 0x68b0718c8373b91d26bf3a2e23f95b466314250a023431a89142908652cb9ef7
      topic1 = <intentId>
  - PFV FeeRecorded log per 5.3
  - NO RebateRecorded
```

Repeat §5.3.1 / §5.3.2 / §5.3.3 / §5.4 for SMOKE-C with the
RFQ event signature substituted.

### 5.6 End of first-smoke session

```text
[ ] all verified gates of §5.3 / (optional) §5.5 are green
[ ] all R5 invariants intact
[ ] no PagerDuty / Discord alert fired during the session
[ ] BE.balance net drop within expected gas envelope
[ ] operator pastes back §9 template
[ ] agent runs FIRST_LIVE_SMOKE_VERIFY against paste-back
[ ] smoke session DECLARED CLOSED in writing (Slack/Discord +
    ticket)
```

After CLOSED: backend remains in live-broadcast mode (FX-Q1-C
flags still on) but no further smoke is authorised in this
session. Subsequent trade volume requires a separate operator
ramp decision (out of scope here).

---

## 6. Stop conditions (any one is an automatic STOP)

| Symbol | Trigger | Action |
|---|---|---|
| FS-1 | Any tx revert (`status=0`) | HALT entire session; do NOT retry without root-cause |
| FS-2 | `RebateRecorded` log on PFV (topic `0x5f501d5b…29cf`) | HALT — rebate path was reached despite scope; trigger §8 freeze |
| FS-3 | PFV `drift != 0` post-trade | HALT — accounting incident; trigger §8 freeze |
| FS-4 | `Δreserve != 0` post-trade | HALT — reserve mutated; trigger §8 freeze |
| FS-5 | `BE.balance` drop > 10× expected gas envelope | HALT — possible compromise or stuck retry loop |
| FS-6 | `signer != BACKEND_EXECUTOR` in logs OR receipt.from != BE | HALT — half-flip or compromise |
| FS-7 | `receipt.to != NEW_OME` | HALT — wrong target; possible config drift |
| FS-8 | `NEW_OME.paused() = true` (unexpectedly) | HALT — guardian fast-pause fired; investigate before any retry |
| FS-9 | Unexpected `OwnershipTransferred` / `ExecutorSet` / `GuardianSet` log on NEW_OME, PFV, FM-V2 | HALT — out-of-scope governance event during smoke window |
| FS-10 | Fee/rebate routing mutation (`FM-V2.feeRecipient` / `rebateFundingAccount` / `protocolFeeVault` changed) | HALT |
| FS-11 | Indexer fails to pick up the event within SLO | HALT subsequent smokes; investigate indexer |
| FS-12 | Reconciliation mismatch (backend expected vs chain actual) | HALT subsequent smokes; investigate reconciliation |
| FS-13 | Any PagerDuty alert fires during the smoke session | HALT; address before continuing |
| FS-14 | Backend logs show a 64+ char `0x`-prefixed hex run that resembles a private key | HALT IMMEDIATELY + rotate per BACKEND_EXECUTOR_CUSTODY_PROFILE §7.3 |
| FS-15 | `eth_sendRawTransaction` is called for any inner data NOT matching `executeTrade` / `executeRfqTrade` selector | HALT — out-of-scope tx |
| FS-16 | Any RPC error pattern (rate limit, mempool reject, nonce gap > 1) | HALT for triage; do not retry blindly |

On any stop trigger:
- Execute `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK §6` rollback if the stop trigger suggests compromise OR config drift.
- Otherwise, leave flags on but freeze further intent submission, debug, then either resume or rollback.
- Update §11 with the trigger + root cause + next action.

---

## 7. Emergency freeze (sub-1-minute)

If FS-2 / FS-3 / FS-4 / FS-5 / FS-6 / FS-14 fires:

```text
1. OPS_MULTISIG signer collects 2-of-3 Safe approvals for:
     Safe-tx → NEW_OME.pause()
     calldata = cast calldata 'pause()'
                = 0x8456cb59   (verify against source before submitting)
   (NEW_OME.pause is onlyGuardianOrOwner; guardian is OPS_MULTISIG since GOV-A-OME.)

2. Confirm post-state NEW_OME.paused() = true.
   Hot path frozen in < 1 minute.

3. Run BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK §6 to flip backend
   flags back to dry-run.

4. Forensic the offending tx via cast receipt + cast logs.

5. Unpause path (after triage): Timelock-queued NEW_OME.unpause(),
   24h wait, execute. See custody profile §7.3.
```

---

## 8. Operator paste-back template (post-smoke)

After §5.6 CLOSED, operator pastes:

```text
V2G-FX-Q1 first live smoke result:

Session window UTC : <start> — <end>
Operator           : <named human>
SRE on-call        : <named human>
Risk on-call       : <named human>

SMOKE-A (sim-only)
  result            : ok | failed
  signer            : 0x...
  from              : 0x...
  target            : 0x...
  notes             : <free text>

SMOKE-B (orderbook live)
  tx_hash           : 0x...
  block             : ...
  status            : 1
  gasUsed           : ...
  intentId          : 0x...
  OptionTradeExecuted topic1 = intentId? : yes
  FeeRecorded(mUSDC, X)                  : amount X = ...
  RebateRecorded                         : ABSENT  (must be true)
  PFV.feeBalance(mUSDC)   pre/post       : .../.. (Δ = X)
  PFV.rebateReserve(mUSDC) pre/post      : 0 / 0 (Δ = 0)
  CV.balances(PFV,mUSDC)   pre/post      : .../.. (Δ = X)
  drift pre/post                          : 0 / 0
  BE.balance pre/post                     : .../..  (gas spent = ...)
  indexer pickup latency                  : ...s
  reconciliation                          : clean
  notes                                   : <free text>

SMOKE-C (RFQ live, OPTIONAL)
  (same shape as SMOKE-B with OptionRfqTradeExecuted topic instead)
  OR
  "skipped — session ended after SMOKE-B"

Stop conditions encountered : none | <list>
PagerDuty alerts during session : 0 | <list>
Discord alerts during session   : 0 | <list>

Session CLOSED at <UTC timestamp>.
Backend remains in live-broadcast mode.
Further trade volume gated on separate ramp decision.
```

Agent runs **FIRST_LIVE_SMOKE_VERIFY** against this paste-back:
re-fetches each tx receipt via `cast receipt`, decodes events,
confirms PFV/CV/FM-V2 deltas against pre-session baseline,
confirms reconciliation, and writes the closure doc.

---

## 9. R5 / PFV invariants pre-session baseline (record at smoke time)

The operator must capture these IMMEDIATELY before SMOKE-A in a
clean `.txt` for diffing post-session:

```text
ts_pre_session            = <unix ts>
block_pre_session         = <block number>
PFV.owner                 = Timelock
NEW_FM_V2.owner           = Timelock
NEW_OME.owner             = Timelock
NEW_OME.isExecutor(BE)    = true
NEW_OME.isExecutor(DEPLOYER) = false
NEW_OME.guardian          = OPS_MULTISIG
NEW_OME.paused            = false
PFV.feeBalance(mUSDC)     = <X>     (must remain 0 except for delta = sum of trade fees)
PFV.rebateReserve(mUSDC)  = 0
CV.balances(PFV, mUSDC)   = <X>     (must equal PFV.feeBalance + rebateReserve at all times)
NEW_FM_V2.rebateBudget(mUSDC) = <Y>  (must remain Y; no rebate consumed)
NEW_FM_V2.protocolFeeVault    = PFV
NEW_FM_V2.feeRecipient        = PFV
NEW_FM_V2.rebateFundingAccount = PFV
RG.feesManager                 = NEW_FM_V2
BACKEND_EXECUTOR.balance       = <Z>  (must drop by ≤ 10× expected gas)
BACKEND_EXECUTOR.code          = 0x
```

The agent's last cap of these values (block `42 540 519`):
```text
feeBalance              = 28
rebateReserve           = 0
CV.balances(PFV,mUSDC)  = 28
rebateBudget            = 999 947
BE.balance              = 3 800 000 000 000 000 wei
```

These are the agent's reference values; the operator MUST re-cap
at smoke time because the soak window may shift them (e.g., if a
fee-only sim or other read-only activity occurred — sim doesn't
mutate, but block production advances ts).

---

## 10. Authorization (REQUIRES THREE SIGNATURES)

**This is the actual authorization. Without all three signatures
this section is incomplete and no smoke is authorised.**

```text
I attest that:
  - I have personally verified every checkbox in §4.1 / §4.2 / §4.3.
  - I have read §2 (allowed scope), §3 (forbidden scope), and §6
    (stop conditions), and I commit to halting on any §6 trigger.
  - I have the §7 emergency freeze procedure rehearsed and ready.
  - I will execute the §5 sequence verbatim.
  - I will paste back per §8 immediately after the session closes.

Operator        : ____________________ (name + sig + UTC ts)
SRE on-call     : ____________________ (name + sig + UTC ts)
Risk on-call    : ____________________ (name + sig + UTC ts)

Smoke session window authorised: from ________ UTC to ________ UTC
                                  (max 30 minutes per §2.3)
```

If any signer hesitates, this section stays blank and no smoke
fires. The agent will not "infer" sign-off from any other artifact.

---

## 11. Open follow-ups + session log

| Tag | Item | Owner | Notes |
|---|---|---|---|
| FX-SMOKE-PRE-1 | FX-Q1-C live-broadcast flag flip + verify | Operator | Strict precondition; cannot run smoke until done. |
| FX-SMOKE-PRE-2 | BE balance ≥ `1e16` wei (FUND_TARGET) | Operator | Currently `3.8e15`; needs top-up before smoke. |
| FX-SMOKE-PRE-3 | Monitoring alerts wired + synthetic-fired | SRE | Required per §4.2. |
| FX-SMOKE-PRE-4 | should_broadcast §4.2 rebate-solvency gate implemented | Backend | Required per §4.2. |
| FX-SMOKE-PRE-5 | Three signatures collected in §10 | Operator + SRE + Risk | THE authorization gate. |
| FX-SMOKE-LOG-N | Each session appends one row: date / outcome / paste-back ref / next-step | Operator + Agent | Append on FIRST_LIVE_SMOKE_VERIFY closure. |

(no smoke sessions yet; first row appended after first paste-back)

---

## 12. What this document does NOT do

```text
- Does NOT flip any backend flag.
- Does NOT broadcast any chain tx.
- Does NOT run the smoke.
- Does NOT authorize anything — §10 signatures do.
- Does NOT alter R5 invariants or any on-chain state.
- Does NOT allow rebate-bearing trade at any time (gated by
  PFV.rebateReserve=0; a future Timelock-queued
  PFV.allocateToRebateReserve unlocks it, separate milestone).
- Does NOT cover liquidation smoke (separate gate; see
  gas/fees/rebates policy §7).
- Does NOT cover mainnet — §13.
```

---

## 13. Sepolia → mainnet fork

The mainnet variant `FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_Y_MAINNET.md`
must additionally:

- Require a separately-deployed mainnet BACKEND_EXECUTOR (distinct
  EOA, distinct KMS region).
- Replace `EXECUTOR_PRIVATE_KEY` env-var path with a KMS-backed
  signing interface per BACKEND_EXECUTOR_CUSTODY_PROFILE §2.1.
- Require audit sign-off in §10 (a fourth signature column).
- Tighten size limits to mainnet economics.
- Require V2G-GOV-G complete on mainnet (Timelock owned by
  GOVERNANCE_MULTISIG Safe 3-of-5).
- Require a successful Sepolia drill of compromise → freeze →
  rotate → unpause per custody §9 BE-PROD-7.

Audit + sign-off before any mainnet smoke is broadcast.

---

## 14. Cross-links

- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — parent cutover.
- `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md` —
  immediate predecessor; flag flip + rollback.
- `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` — key custody
  / compromise response.
- `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` — `should_broadcast`
  decision states; rebate-solvency hard gate.
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — alert routing.
- `GOVERNANCE_EXECUTOR_MIGRATION_EXECUTE_RESULT_V2G_GOV_F_B_X.md`
  — chain milestone establishing BE as executor.
- `GOVERNANCE_TIMELOCK_CLEANUP_PREP_V2G_GOV_G_PREP.md` — chain-side
  GOV-G prep; runs in parallel to FX-Q1.
