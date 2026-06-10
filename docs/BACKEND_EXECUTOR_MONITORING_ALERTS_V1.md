# BACKEND_EXECUTOR monitoring + alerts spec — V1

> **Adjunct surface (2026-06-10):** the operator UI / frontend can hit
> `GET /executor/health/v2` for a non-sensitive JSON summary of the
> same state surfaced here in Prometheus form. The endpoint exposes
> `overall_status ∈ {green, yellow, red}` + a `hard_stops` array
> derived from the §3 paging-alert taxonomy. See
> `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` for the schema.

**Posture:** specification + reference architecture. Mirrors the
`ALERTING_SPEC.md` V1B conventions (Prometheus text format at
`/metrics`, low-cardinality labels only, no secret material in
logs/labels). **This doc does NOT deploy any monitoring stack or
make any chain transaction.**

**Scope:** Base Sepolia (chain 84532). Mainnet fork tracked at §10.

**Subject:** the EOA `BACKEND_EXECUTOR = 0x295005fd4F311e6691F008D57d32FCFEde844518`
once V2G-GOV-F-B-X lands and it becomes the sole NEW_OME hot-path
signer.

**Anchors:**
- `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` §5 — high-level
  alert routing baseline (PagerDuty vs Discord).
- `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` §11 — per-trade metrics
  and alerts on the broadcast economics.
- `deopt-v2-backend/docs/ALERTING_SPEC.md` — V1B service-level
  alerts, label policy, secret hygiene.
- `~/DEOPT/RESUME_GOV_F_B_X.md` — the operator runbook that
  references the §6 paste-back gates.

This doc is the **single operational source-of-truth** for what
the on-call must see and act on for BE.

---

## 0. Hard stops (this doc)

```text
no chain tx                                              ✅
no executeTransaction                                    ✅
no direct setExecutor                                    ✅
no ownership / guardian / Timelock mutation              ✅
no fee/rebate routing mutation                           ✅
no reserve allocation                                    ✅
no RFQ smoke / trade                                     ✅
no .env edit                                             ✅
no private key / admin token output                      ✅
no mainnet                                               ✅
```

---

## 1. Label policy (recap)

Allowed low-cardinality labels:

```text
network               ∈ {sepolia, mainnet}
result                ∈ {success, failed, reverted, dropped, dedupe}
reason                ∈ enum of §3 broadcast-decision reasons
                        (profitable, at-cost, subsidy, liquidation,
                         rebate-budget, rebate-reserve, sim, ome-paused,
                         be-low-bal, wash, rebate-quota, pattern,
                         no-econ-content, gas-cap, expired, nonce,
                         delisted, stale-rm, buyer-sig, seller-sig,
                         uneconomic)
side                  ∈ {maker, taker}
flow                  ∈ {orderbook, rfq, liquidation}
product_kind          ∈ {option, perp}
asset                 ∈ {musdc, usdc, …}        (low-cardinality whitelist)
queue_op              ∈ {add, remove, cancel, execute}
custody_event         ∈ {fund, drain, rotate-start, rotate-complete, revoke}
```

Forbidden as labels (high cardinality / secret):

```text
tx hash, address, log digest, signer key handle, kms arn, request id,
nonce values, raw RPC URL, JWT, EIP-712 typed data, log-payload bytes,
order ids, account UUIDs, ABI calldata.
```

These belong in **structured log fields**, NEVER in alert labels.

---

## 2. Metric inventory

### 2.1 Signer-level (BE EOA)

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_be_balance_wei` | gauge | `network` | on-chain `cast balance BE` poll (§6) |
| `deopt_be_balance_eth` | gauge | `network` | derived; `balance_wei / 1e18` |
| `deopt_be_nonce_onchain` | gauge | `network` | `cast nonce BE` poll |
| `deopt_be_nonce_pending` | gauge | `network` | backend nonce tracker |
| `deopt_be_nonce_gap` | gauge | `network` | `pending - onchain` |
| `deopt_be_is_executor` | gauge | `network` | `NEW_OME.isExecutor(BE)` poll; 0/1 |
| `deopt_deployer_is_executor` | gauge | `network` | `NEW_OME.isExecutor(DEPLOYER)` poll; 0/1 |
| `deopt_new_ome_paused` | gauge | `network` | `NEW_OME.paused()` poll; 0/1 |
| `deopt_be_code_size_bytes` | gauge | `network` | `cast code BE` byte length; MUST be 0 (EOA) |

### 2.2 Broadcast-decision flow

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_be_candidates_total` | counter | `network, flow` | every `should_broadcast` call |
| `deopt_be_decisions_total` | counter | `network, flow, result, reason` | every decision; `result ∈ {success, dropped}` |
| `deopt_be_simulation_total` | counter | `network, flow, result` | every cast call simulation |
| `deopt_be_simulation_duration_seconds` | histogram | `network` | simulation latency |
| `deopt_be_dedupe_hits_total` | counter | `network` | replay dedupe hits |
| `deopt_be_wash_rejects_total` | counter | `network` | wash detection rejects |
| `deopt_be_rebate_quota_rejects_total` | counter | `network` | per-maker quota breaches |

### 2.3 Broadcast economics (asset-unit and wei)

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_be_gross_fee_revenue` | sum | `network, flow, asset` | per broadcast |
| `deopt_be_rebate_outflow` | sum | `network, flow, asset` | per broadcast |
| `deopt_be_net_protocol_revenue` | sum | `network, flow, asset` | per broadcast |
| `deopt_be_expected_pnl_in_asset` | histogram | `network, flow` | per broadcast (pre-submit) |
| `deopt_be_actual_pnl_in_asset` | histogram | `network, flow` | per receipt (post-confirm) |
| `deopt_be_gas_units_estimated` | histogram | `network, flow` | from simulation |
| `deopt_be_gas_units_used` | histogram | `network, flow` | from receipt |
| `deopt_be_max_fee_per_gas_wei` | histogram | `network` | chosen maxFee per tx |
| `deopt_be_priority_fee_wei` | histogram | `network` | chosen tip per tx |
| `deopt_be_l1_data_fee_wei` | histogram | `network` | per receipt |
| `deopt_be_basefee_wei` | gauge | `network` | last seen basefee (poll per block) |

### 2.4 Subsidy budget (off-chain ledger)

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_be_subsidy_take_amount` | sum | `network, reason, asset` | each subsidy debit |
| `deopt_be_subsidy_budget_remaining` | gauge | `network, reason, asset` | post-debit snapshot |
| `deopt_be_subsidy_budget_cap` | gauge | `network, reason, asset` | static cap (refresh on config reload) |

### 2.5 Tx lifecycle

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_be_tx_submitted_total` | counter | `network, flow` | post-`eth_sendRawTransaction` |
| `deopt_be_tx_confirmed_total` | counter | `network, flow, result` | from receipt poller; result ∈ {success, reverted, dropped} |
| `deopt_be_tx_confirm_seconds` | histogram | `network` | submit-to-confirm latency |
| `deopt_be_tx_inflight` | gauge | `network` | submitted minus confirmed (rolling) |

### 2.6 Liquidation flow

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_be_liquidation_broadcasts_total` | counter | `network, result` | every liquidation candidate |
| `deopt_be_liquidatable_pending_count` | gauge | `network` | per RM poll: # of currently liquidatable accounts |
| `deopt_be_liquidation_idle_seconds` | gauge | `network` | time since last successful liquidation broadcast |

### 2.7 Chain controls (cross-check)

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_timelock_owner_is_deployer` | gauge | `network` | `Timelock.owner() == DEPLOYER` → 0/1 (will flip when V2G-GOV-G lands) |
| `deopt_timelock_min_delay_seconds` | gauge | `network` | `Timelock.minDelay()` |
| `deopt_timelock_proposers_deployer` | gauge | `network` | `Timelock.proposers(DEPLOYER)` |
| `deopt_timelock_executors_deployer` | gauge | `network` | `Timelock.executors(DEPLOYER)` |
| `deopt_pfv_owner_is_timelock` | gauge | `network` | `PFV.owner() == Timelock` → 0/1 |
| `deopt_fmv2_owner_is_timelock` | gauge | `network` | `NEW_FM_V2.owner() == Timelock` → 0/1 |
| `deopt_pfv_fee_balance_musdc` | gauge | `network` | `PFV.feeBalance(mUSDC)` |
| `deopt_pfv_rebate_reserve_musdc` | gauge | `network` | `PFV.rebateReserve(mUSDC)` |
| `deopt_fmv2_rebate_budget_musdc` | gauge | `network` | `NEW_FM_V2.rebateBudget(mUSDC)` |
| `deopt_cv_pfv_balance_musdc` | gauge | `network` | `CV.balances(PFV, mUSDC)` |
| `deopt_r5_drift_musdc` | gauge | `network` | `CV - feeBalance - rebateReserve` |
| `deopt_rg_fees_manager_is_fmv2` | gauge | `network` | `RG.feesManager() == NEW_FM_V2` → 0/1 |

R5 drift MUST stay at 0 at all times. A non-zero drift is a
critical accounting incident.

---

## 3. Paging alerts (PagerDuty)

For each alert: `signal | severity | intent | first-response action`.

### 3.1 Signer integrity

```text
[ BE_BAL_LOW ]
  signal   : deopt_be_balance_wei < 1e15 for > 5 min
  severity : critical
  intent   : BE will silently stop signing trades when balance hits 0
  action   : top-up BE per BACKEND_EXECUTOR_CUSTODY_PROFILE §4.2;
             confirm balance crosses FUND_TARGET (1e16) within 15 min

[ BE_NOT_EXECUTOR ]
  signal   : deopt_be_is_executor == 0
             AND deopt_be_is_executor was 1 in the previous scrape
  severity : critical
  intent   : BE has been removed from NEW_OME executors without
             an executed Timelock op or operator action
  action   : freeze backend signing immediately; check Timelock
             cancelTransaction logs; forensic on Timelock executor
             role; consider OPS_MULTISIG.NEW_OME.pause()

[ BE_CODE_NONZERO ]
  signal   : deopt_be_code_size_bytes > 0
  severity : critical
  intent   : BE's address has been transformed from an EOA into a
             contract (only possible via EIP-7702 delegation today
             and ONLY signed by BE's key) — implies key compromise
  action   : OPS_MULTISIG.NEW_OME.pause(); rotate to
             BACKEND_EXECUTOR_NEXT per custody §6 emergency path

[ BE_OOB_TX ]
  signal   : any tx sent by BE with to != NEW_OME for > 0 in last 5 min
             (excluding top-ups TO BE)
  severity : critical
  intent   : BE-keyed tx targeting a non-NEW_OME contract — outside
             role boundary; possible compromise
  action   : OPS_MULTISIG.NEW_OME.pause(); freeze signing service;
             initiate compromise response per custody §7

[ BE_NONCE_GAP ]
  signal   : deopt_be_nonce_gap > 5 for > 2 min
  severity : critical
  intent   : pending tx pile-up; either RPC drop or attacker queueing
             stuck txs at higher nonce
  action   : check submission pipeline; if not RPC-side, treat as
             potential compromise
```

### 3.2 Engine state

```text
[ NEW_OME_UNEXPECTEDLY_PAUSED ]
  signal   : deopt_new_ome_paused == 1 AND ops did not schedule a pause
  severity : critical
  intent   : OPS_MULTISIG (guardian) paused the engine, OR owner
             (Timelock) executed pause
  action   : confirm with on-call ops; if unintended, queue Timelock
             unpause; if intended, ensure paging context is updated
```

### 3.3 Broadcast pipeline

```text
[ EXEC_REVERTS_HIGH ]
  signal   : rate(deopt_be_decisions_total{result="dropped", reason=~"sim.*"}[15m]) / rate(deopt_be_candidates_total[15m]) > 0.05
  severity : critical
  intent   : > 5% simulation failures — sim/reality divergence,
             margin engine drift, or attacker pattern
  action   : sample 5 failed candidates; verify EIP-712 + nonces +
             margin; check RM snapshot freshness; if RM drift, halt
             broadcasts until snapshot caught up

[ BE_SUBMIT_REVERTED ]
  signal   : rate(deopt_be_tx_confirmed_total{result="reverted"}[15m]) > 0
  severity : critical
  intent   : a tx confirmed AS REVERTED on chain — gas spent, no trade
  action   : pull the reverted tx; decode revert reason; if data drift
             (e.g. nonce reuse, deadline expiry), tighten matcher;
             if RM/RG state shifted post-sim, add gating

[ TX_CONFIRMATION_STUCK ]
  signal   : deopt_be_tx_inflight > 5 for > 2 min
             AND deopt_be_tx_confirmed_total has not increased
  severity : critical
  intent   : submissions accepted but no inclusion
  action   : check Base sequencer status, RPC health, priority fee;
             if attacker front-running, raise maxPriorityFeePerGas
             within MAX_PRIORITY_FEE_PER_GAS bound

[ BROADCAST_REJECT_REBATE_RESERVE ]
  signal   : rate(deopt_be_decisions_total{result="dropped", reason="rebate-reserve"}[5m]) > 0
  severity : critical
  intent   : a rebate trade would have reverted at the PFV hook
             (rebateReserve = 0); honest makers being silently dropped
  action   : Timelock-queue PFV.allocateToRebateReserve per separate
             milestone; meanwhile keep alert hot

[ BROADCAST_REJECT_WASH_SPIKE ]
  signal   : rate(deopt_be_wash_rejects_total[5m]) > 10× baseline
  severity : critical
  intent   : potential rebate-mining attack
  action   : freeze rebate-positive makers from involved addresses;
             escalate to risk
```

### 3.4 Economics

```text
[ PNL_DIVERGENCE ]
  signal   : abs(median(deopt_be_actual_pnl_in_asset) - median(deopt_be_expected_pnl_in_asset)) / abs(median(deopt_be_expected_pnl_in_asset)) > 0.3 over 100-trade window
  severity : critical
  intent   : simulation P&L diverged > 30% from realized P&L — gas
             estimation or fee math drift
  action   : freeze SUBSIDISABLE state; reduce GAS_SAFETY_FACTOR
             gating; raise PNL_FLOOR temporarily; investigate

[ SUBSIDY_FAST_BURN ]
  signal   : deopt_be_subsidy_budget_remaining drops > 20% in any 1 h window per (reason, asset)
  severity : critical
  intent   : either healthy market-making bootstrap OR compromised
             BE silently sponsoring attacker
  action   : confirm trades against expected reason; if mismatch, freeze
             SUBSIDISABLE state for the reason

[ MAXFEE_CEILING_HIT ]
  signal   : deopt_be_max_fee_per_gas_wei == MAX_MAX_FEE_PER_GAS for > 10 events in any 5 min
  severity : critical
  intent   : gas market spiking OR attacker bidding wars
  action   : pause new SUBSIDISABLE broadcasts; consider raising cap
             only after gas-market analysis
```

### 3.5 Accounting

```text
[ R5_DRIFT ]
  signal   : deopt_r5_drift_musdc != 0
  severity : critical
  intent   : PFV / CV / FM-V2 accounting invariant broken
  action   : OPS_MULTISIG.NEW_OME.pause(); freeze all flows touching
             FM-V2.consumeFees; treat as accounting incident
             (forensic on every tx since last drift=0 snapshot)

[ PFV_OWNER_DRIFT ]
  signal   : deopt_pfv_owner_is_timelock == 0 (after V2G-GOV-B)
  severity : critical
  intent   : someone flipped PFV ownership away from Timelock
  action   : freeze backend signing; trace tx; if not a Timelock-queued
             op, treat as governance incident

[ FMV2_OWNER_DRIFT ]
  signal   : deopt_fmv2_owner_is_timelock == 0 (after V2G-GOV-C)
  severity : critical
  intent   : same shape as PFV_OWNER_DRIFT, for NEW_FM_V2
  action   : same

[ RG_FEES_MANAGER_DRIFT ]
  signal   : deopt_rg_fees_manager_is_fmv2 == 0
  severity : critical
  intent   : RiskGovernor.feesManager flipped away from NEW_FM_V2
  action   : freeze backend; verify against Timelock log
```

### 3.6 Liquidation

```text
[ LIQUIDATION_IDLE ]
  signal   : deopt_be_liquidation_idle_seconds > 1800
             AND deopt_be_liquidatable_pending_count > 0
  severity : critical
  intent   : RM flags accounts as liquidatable but no liquidation
             tx broadcast in > 30 min — could be a stuck signer,
             a paused engine, or a logic bug
  action   : verify NEW_OME.paused, BE health, subsidy budget,
             RM snapshot; manually walk one candidate through
             should_broadcast and inspect rejected reason

[ LIQ_PAUSED ]
  signal   : RG.liquidationPaused == true unexpectedly
  severity : critical
  intent   : RG-side circuit breaker; could be intentional or
             governance-side mistake
  action   : confirm with on-call ops; queue unpause if unintended
```

---

## 4. Warning alerts (Discord / ops channel)

```text
[ BE_BAL_CEILING ]
  signal   : deopt_be_balance_wei > 5e16 for > 1 h
  severity : warning
  intent   : balance above ceiling caps key-compromise loss surface
  action   : drain excess back to top-up source per custody §4.3

[ BE_OFF_HOURS_SPIKE ]
  signal   : sign rate > 3× baseline outside expected trading window
  severity : warning
  intent   : not always bad (market events) but worth a human look
  action   : sample 10 trades; verify they look organic; do not page

[ SUBSIDY_BUDGET_SLOW_BURN ]
  signal   : deopt_be_subsidy_budget_remaining / cap < 0.25 (any reason)
  severity : warning
  intent   : budget approaching depletion within expected horizon
  action   : Finance review; raise cap or wind down subsidised flow

[ SIM_LATENCY_DRIFT ]
  signal   : p95(deopt_be_simulation_duration_seconds) > 1s
  severity : warning
  intent   : simulation slow → user-perceived latency rising
  action   : RPC review; consider local/dedicated node

[ TIMELOCK_OWNER_UNEXPECTED ]
  signal   : deopt_timelock_owner_is_deployer != expected
  severity : warning
  intent   : expected state shifts when V2G-GOV-G lands; pre-GOV-G
             a flip is unexpected
  action   : verify against governance milestone; update expectation
             if intentional

[ PROPOSER_DEPLOYER_DROPPED ]
  signal   : deopt_timelock_proposers_deployer == 0 pre-GOV-G
  severity : warning
  intent   : rollback surface for GOV-B/C/F lost prematurely
  action   : verify against governance milestone; pre-GOV-G it is a
             bug; post-GOV-G G-5 it is expected
```

---

## 5. Dashboard panels

Recommended Grafana dashboard `be-executor-overview`:

### Row 1 — Signer at-a-glance

```text
panel 01  BE balance (gauge + 24h sparkline)
panel 02  BE nonce (current vs pending)
panel 03  isExecutor(BE) + isExecutor(DEPLOYER) (single-stat 0/1)
panel 04  NEW_OME.paused (single-stat 0/1)
panel 05  BE code-size (must be 0 — single-stat with critical color when > 0)
```

### Row 2 — Broadcast flow

```text
panel 06  candidates/sec (line)
panel 07  decisions/sec by result (stacked: success / dropped)
panel 08  drop reasons (top 8) — bar chart, last 1h
panel 09  simulation latency p50/p95/p99
panel 10  tx inflight (line) — alarm at >5
```

### Row 3 — Economics

```text
panel 11  gross fee revenue / rebate outflow / net revenue (lines, asset units)
panel 12  expected vs actual PnL — overlay, median + p10/p90 bands
panel 13  gas units estimated vs used (overlay)
panel 14  maxFee vs basefee (overlay; ceiling line at MAX_MAX_FEE_PER_GAS)
panel 15  subsidy_budget_remaining per reason (lines; cap dashed)
```

### Row 4 — Chain controls

```text
panel 16  R5 drift (single-stat; alarm at != 0)
panel 17  PFV.feeBalance / rebateReserve (lines)
panel 18  CV.balances(PFV) (line)
panel 19  FM-V2.rebateBudget (line)
panel 20  PFV.owner / FM-V2.owner / RG.feesManager (single-stats 0/1)
panel 21  Timelock min-delay + queuePaused (single-stats)
```

### Row 5 — Liquidation

```text
panel 22  liquidatable pending count (line)
panel 23  liquidation broadcasts / 5 min
panel 24  liquidation idle seconds (single-stat; alarm at > 1800)
panel 25  liquidation_paused (single-stat)
```

### Row 6 — Tx-confirmation forensics

```text
panel 26  confirmation latency p50/p95
panel 27  reverted txs (count + tx hash table; tx hash from logs not labels)
panel 28  L1 data fee / tx (line)
panel 29  priority fee distribution (heatmap)
panel 30  basefee 24h (line)
```

---

## 6. Read-only on-chain probes

Polls run by a backend metric collector. **All read-only.** Each
poll updates the matching §2.1 / §2.3 / §2.7 / §3.5 gauge.

### 6.1 Polling cadence

```text
BE balance / nonce / code-size                       : every 10 s
NEW_OME owner / pendingOwner / guardian / paused     : every 30 s
NEW_OME isExecutor(BE) / isExecutor(DEPLOYER)        : every 30 s
PFV  owner / guardian / feeBalance / rebateReserve   : every 60 s
NEW_FM_V2 owner / routing (×3) / rebateBudget        : every 60 s
CV balances(PFV, mUSDC)                              : every 60 s
RG feesManager / liquidationPaused                   : every 60 s
Timelock owner / minDelay / queuePaused /
  proposers(DEPLOYER) / executors(DEPLOYER)          : every 60 s
RM liquidatable-account view                         : every 30 s (or on push if RM supports streaming)
basefee                                              : every block (~2 s on Base)
```

Cadence chosen for cheap RPC load on the alchemy endpoint and
incident-response latency objectives (sub-minute MTTD for the
critical paging signals).

### 6.2 Probe-side hygiene

```text
- timeout each probe at 5 s
- exponential backoff on RPC failure; mark gauge `stale` (separate
  gauge label `result="stale"`) rather than retaining a stale value
- never log raw RPC URL; never log key material; only log probe
  result + error class
- probes share an RPC connection pool sized to ≤ 5 concurrent
  requests to avoid Alchemy rate limits
- probes refuse to run if RPC endpoint != configured chain id
  (avoid accidental mainnet probe in a Sepolia-config service)
```

### 6.3 Cross-check probes (run hourly)

These are sanity checks, not real-time alerts. Stored as
`deopt_be_crosscheck_*` results.

```text
- recompute R5 drift = CV - feeBalance - rebateReserve; compare to
  the streaming gauge; mismatch is a probe-side bug
- recompute op_add / op_rm from current queue contents (decode
  TransactionQueued logs since last cross-check); confirm against
  Timelock.isQueued tuple-getter
- confirm BACKEND_EXECUTOR has no protocol-collisions
  (cross-check vs static contract registry)
- confirm OPS_MULTISIG.getThreshold() == 2 and isOwner(DEPLOYER) == false
  (Safe stability)
```

---

## 7. Per-broadcast logging

Every `should_broadcast` call AND every chain submit emits a
structured log line. **No labels** beyond §1 whitelist; high-cardinality
fields go in the log payload only.

### 7.1 should_broadcast log

```json
{
  "service": "deopt-backend",
  "subsystem": "executor",
  "event": "should_broadcast",
  "network": "sepolia",
  "flow": "orderbook",                  // or "rfq" or "liquidation"
  "result": "success" | "dropped",
  "reason": "<§1 reason enum>",
  "candidate_digest": "<eip712-digest hex>",  // payload field, not label
  "ttl_remaining_ms": 1234,
  "fees": {
    "gross_fee_revenue": "123",
    "rebate_outflow": "0",
    "net_protocol_revenue": "123",
    "asset": "musdc"
  },
  "gas": {
    "units_estimated": 580000,
    "max_fee_per_gas_wei": 12000000,
    "priority_fee_wei": 1000000,
    "l1_data_fee_wei_estimated": 5000000000
  },
  "expected_pnl_in_asset": "78",
  "decision_state": "profitable" | "at-cost" | "subsidy" | "liquidation" | null,
  "subsidy": {
    "reason": "mm-bootstrap",            // or null
    "amount_in_asset": "12",
    "budget_remaining_in_asset": "999988"
  },
  "kms_request_id": "<KMS audit correlation id>",   // payload only
  "matcher_request_id": "<internal id>"
}
```

### 7.2 chain submit log (post-`eth_sendRawTransaction`)

```json
{
  "service": "deopt-backend",
  "subsystem": "executor",
  "event": "tx_submit",
  "network": "sepolia",
  "flow": "orderbook",
  "tx_hash": "0x…",                              // payload only
  "nonce": 42,
  "candidate_digest": "0x…",
  "kms_request_id": "0x…"
}
```

### 7.3 receipt log (per confirmed receipt)

```json
{
  "service": "deopt-backend",
  "subsystem": "executor",
  "event": "tx_receipt",
  "network": "sepolia",
  "tx_hash": "0x…",
  "block_number": 42452500,
  "result": "success" | "reverted",
  "gas_used": 41523,
  "effective_gas_price_wei": 6000000,
  "l1_fee_wei": 5837582655,
  "actual_pnl_in_asset": "75",
  "expected_pnl_in_asset_at_submit": "78",
  "duration_ms_submit_to_confirm": 4280
}
```

### 7.4 custody-event log

```json
{
  "service": "deopt-backend",
  "subsystem": "executor",
  "event": "custody_event",
  "network": "sepolia",
  "custody_event": "fund" | "drain" | "rotate-start" | "rotate-complete" | "revoke",
  "actor": "operator|service",
  "details": { /* free-form, no key material */ }
}
```

### 7.5 logging hygiene (recap)

```text
- NEVER log private keys, KMS key material, EIP-712 typed-data raw,
  raw RPC URL, JWT/session tokens, signatures (per ALERTING_SPEC.md)
- ALWAYS structure: service + subsystem + event + network + (result | custody_event)
- retention: 30 d hot, 1 y cold (matches BACKEND_EXECUTOR_CUSTODY_PROFILE §5.2)
- pii: none (BE-only; no end-user metadata in BE logs)
```

---

## 8. Incident response mapping

Each paging alert maps to a runbook step. The runbook lives at
`deopt-v2-backend/docs/RUNBOOK_BACKEND_EXECUTOR.md` (TODO — see §9).

| Alert | Initial freeze | Diagnostic | Corrective | Escalation |
|---|---|---|---|---|
| `BE_BAL_LOW` | none — keep signing | check pending tx queue | top-up per custody §4.2 | Finance if recurring |
| `BE_NOT_EXECUTOR` | freeze backend signing | check Timelock cancelTransaction + recent executed ops | re-queue setExecutor(BE, true) via Timelock | governance + audit |
| `BE_CODE_NONZERO` | OPS_MULTISIG.NEW_OME.pause() | KMS audit; chain forensics on every BE tx since last clear | rotate to BACKEND_EXECUTOR_NEXT per custody §6.3 | full incident (legal + audit) |
| `BE_OOB_TX` | OPS_MULTISIG.NEW_OME.pause() | identify destination contract; KMS audit | rotate per custody §7.3 | full incident |
| `BE_NONCE_GAP` | freeze signing | check RPC; check mempool for stuck txs at next nonce | bump replacement tx; if RPC ok, treat as compromise | SRE on-call |
| `NEW_OME_UNEXPECTEDLY_PAUSED` | none — already paused | check guardian / owner tx logs | queue Timelock unpause if intended | governance |
| `EXEC_REVERTS_HIGH` | throttle to PROFITABLE-only | sample reverts; classify | tighten matcher pre-checks; raise sim safety | risk + backend |
| `BE_SUBMIT_REVERTED` | throttle | decode revert; identify gate miss | add gating; bump SAFETY_MARGIN | risk + backend |
| `TX_CONFIRMATION_STUCK` | freeze new submits | Base sequencer status; RPC health | raise priorityFee within cap; consider RPC failover | SRE |
| `BROADCAST_REJECT_REBATE_RESERVE` | none — already rejecting | confirm trades were honest | governance: PFV.allocateToRebateReserve Timelock queue | governance |
| `BROADCAST_REJECT_WASH_SPIKE` | freeze rebate makers from involved addresses | examine address graph | per-address rebate disable | risk |
| `PNL_DIVERGENCE` | freeze SUBSIDISABLE | sample divergent trades | recalibrate gas / fee math; raise PNL_FLOOR temporarily | risk + backend |
| `SUBSIDY_FAST_BURN` | freeze SUBSIDISABLE for the reason | audit reason population | confirm operator policy still holds; if breach, freeze + investigate | Finance + risk |
| `MAXFEE_CEILING_HIT` | freeze SUBSIDISABLE | check basefee + competitors | raise cap only after risk review | SRE + risk |
| `R5_DRIFT` | OPS_MULTISIG.NEW_OME.pause() | forensic on every consumeFees since last clear | accounting recovery (separate milestone) | full incident |
| `PFV_OWNER_DRIFT` / `FMV2_OWNER_DRIFT` | freeze backend | trace tx; verify against Timelock log | governance recovery | governance |
| `RG_FEES_MANAGER_DRIFT` | freeze backend | trace tx | governance recovery | governance |
| `LIQUIDATION_IDLE` | none | confirm BE health + RM data | manual liquidation trial per RM signal | risk |
| `LIQ_PAUSED` | none | confirm with on-call ops | queue RG unpause if unintended | risk + governance |

Common pattern: **freeze before diagnose** when the alert implies
compromise (`BE_CODE_NONZERO`, `BE_OOB_TX`, drift signals); **freeze
on diagnose** when the alert implies misconfiguration
(`EXEC_REVERTS_HIGH`, `PNL_DIVERGENCE`); **observe-only** when the
alert is liveness-degradation only (`BE_BAL_LOW`,
`LIQUIDATION_IDLE`).

---

## 9. Implementation TODOs

| # | Item | Owner | Notes |
|---|---|---|---|
| MON-1 | Wire §6 polls into the existing metric collector; add the §2.1 / §2.7 gauges to `/metrics` | Backend + SRE | Reuse the V1B label policy. |
| MON-2 | Wire §2.2-§2.6 metrics into `should_broadcast` and tx pipeline | Backend | Tied to BACKEND_GAS_FEES_REBATES_POLICY §10 T-2. |
| MON-3 | Provision Grafana dashboard `be-executor-overview` per §5 | SRE | JSON definition in `monitoring/grafana/` (TODO repo path). |
| MON-4 | Wire §3 paging alerts to PagerDuty | SRE | Routing per custody §5.3. |
| MON-5 | Wire §4 warning alerts to Discord ops channel | SRE | Same. |
| MON-6 | Publish `RUNBOOK_BACKEND_EXECUTOR.md` next to this spec | SRE + Ops | One-pager per alert with diagnostic queries. |
| MON-7 | Cross-check hourly job per §6.3 (cron + alert if drift between probes and stream) | Backend | Detects monitoring-pipeline bugs. |
| MON-8 | Synthetic probe: a Sepolia fault-injection job that flips one chain-control gauge into an alarm state (offline staging only) to verify routing | SRE | Quarterly. |
| MON-9 | Logging library hygiene check: PR linter that rejects log lines with high-cardinality fields placed in labels OR secret material in payload | Backend + Sec | Static check. |
| MON-10 | Dashboard variants for mainnet (post-V2G-Y); same panels with tighter alarm bands | Future | Mainnet milestone. |

---

## 10. Sepolia → mainnet fork

Mainnet variant `BACKEND_EXECUTOR_MONITORING_ALERTS_V2G_Y_MAINNET.md`
must:

- Tighten `BE_BAL_LOW` to a recomputed mainnet floor.
- Add a `mainnet` label dimension (replace Sepolia values per metric).
- Page on ALL drift signals at a lower threshold (mainnet incidents
  are higher cost).
- Add a paging alert for `Timelock.minDelay != 72h` if D-7' is
  resolved as 72h pre-mainnet.
- Audit + sign-off before any mainnet broadcast.

---

## 11. Open follow-ups / blockers

| Tag | Item | Owner | Notes |
|---|---|---|---|
| MON-Q1 | Operator commits PagerDuty + Discord routing to custody §5.3 | Operator + SRE | Required for V2G-GOV-F-B-X go-live. |
| MON-Q2 | Confirm `expected_baseline` for `BE_OFF_HOURS_SPIKE` | Operator | Define which hours are "off-hours" per environment. |
| MON-Q3 | Confirm `expected_baseline` for `SUBSIDY_FAST_BURN` | Finance | Per-reason expected burn rate. |
| MON-Q4 | Confirm `MAX_MAX_FEE_PER_GAS` per `BACKEND_GAS_FEES_REBATES_POLICY §9` | Backend + SRE | Used by `MAXFEE_CEILING_HIT`. |
| MON-Q5 | RUNBOOK_BACKEND_EXECUTOR.md draft | Ops + SRE | Per-alert one-pager runbook. |
| MON-Q6 | RM streaming endpoint for liquidatable accounts | Protocol + Backend | Reduces poll load on Alchemy. |
| MON-Q7 | Mainnet fork (V2G-Y) | Future | After Sepolia stable for ≥ 1 cycle. |

None block Sepolia GOV-F-B-X broadcast or this V1 spec.

---

## 12. Cross-links

- `deopt-v2-sol/docs/BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` —
  custody / funding / rotation policy. §5 high-level alert routing.
- `deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md` —
  broadcast economics + per-trade metric requirements.
- `deopt-v2-backend/docs/ALERTING_SPEC.md` — V1B service-level
  alert spec; label policy and secret hygiene baseline.
- `deopt-v2-sol/docs/PROTOCOL_FEE_VAULT_OBSERVABILITY_SPEC_V2G_RX.md` —
  PFV observability spec (R5 invariant inputs).
- `deopt-v2-sol/docs/GOVERNANCE_EXECUTOR_MIGRATION_QUEUE_RESULT_V2G_GOV_F_B_Q.md` —
  GOV-F-B-Q closure that gates GOV-F-B-X and therefore this spec
  taking effect.
- `~/DEOPT/RESUME_GOV_F_B_X.md` — operator runbook for the Phase X
  broadcast.
