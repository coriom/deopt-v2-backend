# Backend gas / fees / rebates policy — V1

**Posture:** policy + design document. No chain interaction. No
`.env` edit. Establishes the **broadcast-economics rules** that
the backend matching service MUST enforce per
`executeTrade` / `executeRfqTrade` call once
**V2G-GOV-F-B-X lands** and `BACKEND_EXECUTOR` becomes the sole
NEW_OME hot-path signer.

**Scope:** Base Sepolia (chain 84532). Mainnet fork tracked at §15.

**Context anchors:**
- `src/fees/FeesManagerV2.sol` lines 18-20: `MAX_TAKER_FEE_PPM =
  1000` (+10 bps), `MAX_MAKER_REBATE_PPM = -1000` (−10 bps), `PPM_DENOMINATOR
  = 1 000 000`. Per-side ppm rates AND tier-based RFQ discount
  ppms.
- `consumeFees` (line 291): per-side fee/rebate consumption,
  `InsufficientRebateBudget` if `rebateBudget[asset] < required`.
- PFV hook surface: `onFeeCharged(asset, amount)`,
  `onRebatePaid(asset, amount)`.
- Current on-chain state: `feeBalance(mUSDC)=28`,
  `rebateReserve(mUSDC)=0`, `CV.balances(PFV,mUSDC)=28`, `drift=0`,
  `rebateBudget(mUSDC)=999 947`. **No rebate-bearing trade allowed
  yet** (operator-imposed; rebateReserve must be funded via
  `allocateToRebateReserve` before any rebate-positive maker can be
  broadcast).
- `BACKEND_EXECUTOR = 0x295005fd…4518` is an EOA that pays L2+L1
  gas for every `executeTrade` / `executeRfqTrade` call.

---

## 0. Hard stops (this doc)

```text
no chain tx                                              ✅
no executeTransaction                                    ✅
no direct setExecutor                                    ✅
no ownership / guardian / Timelock mutation              ✅
no fee/rebate routing mutation                           ✅
no reserve allocation                                    ✅
no RFQ smoke                                             ✅
no trade                                                 ✅
no .env edit                                             ✅
no private key / admin token output                      ✅
no mainnet                                               ✅
```

---

## 1. Roles — who pays what

| Cost element | Payer (today, post-GOV-F-B-X) | Notes |
|---|---|---|
| L2 gas (Base) for `executeTrade` / `executeRfqTrade` | **BACKEND_EXECUTOR** (EOA) | Burns native ETH from BE's balance. |
| L1 data-availability fee (Base posts calldata to L1) | **BACKEND_EXECUTOR** | Included in `cast send` returndata as `l1Fee`; sometimes the larger of the two costs. |
| Taker fee (positive `takerPpm`) | **Taker** (debited from taker's CV sub-account in `consumeFees` → PFV via `onFeeCharged`) | Settled in `settlementAsset`. |
| Maker rebate (negative `makerPpm`) | **Protocol** (credited to maker's CV sub-account from `rebateBudget` via `consumeFees` → PFV `onRebatePaid`) | Settled in `settlementAsset` from PFV's `rebateReserve`. |
| Maker fee (positive `makerPpm`, e.g. tier-0) | **Maker** | Same accounting path as taker. |
| Liquidation seizure / penalty | **Liquidatee** (collateral seized; penalty routed per RG params) | Out of FM-V2 scope; covered separately at §10. |
| Protocol revenue net of rebates paid | accrued to `PFV.feeBalance` → withdrawable via `PFV.withdrawRevenue` under Timelock | Long-cycle, not per-tx. |

**Key asymmetry to design around:** gas is paid in **native ETH**
by BE; fees/rebates are settled in the **settlement asset**
(e.g. mUSDC). The two ledgers do not net automatically; the
backend must convert both to a common P&L unit before deciding
broadcast.

---

## 2. Gas sponsorship model

The protocol does NOT pay gas for users today. BACKEND_EXECUTOR
fronts gas; the per-trade backend P&L (§4) determines whether the
gross fee earned by the protocol *also* covers BE's gas
expenditure. Two regimes:

### 2.1 Self-funded mode (default)

```text
gross_fee_revenue (to PFV) ≥ gas_cost (in settlement-asset terms,
                                       converted via current price)
                            × safety_margin
```

In this regime the protocol earns net positive revenue per trade
AFTER reimbursing gas (in a separate ledger). The operator MAY
periodically sweep PFV revenue → fiat → BE top-up to keep BE's
ETH balance within the §4 bands of
`BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md`. No on-chain
reimbursement flow exists today.

### 2.2 Capped-subsidy mode (opt-in per reason)

When a trade is below the §2.1 threshold (e.g. zero-fee market-making
bootstrap, RFQ recovery, scheduled discount campaign), the backend
MAY still broadcast **iff** the gap is deducted from a named,
operator-configured subsidy budget tracked off-chain:

```text
gas_gap_in_asset = gas_cost_in_asset × safety_margin - gross_fee_revenue
if gas_gap_in_asset > 0 and subsidy_budget[reason].remaining >= gas_gap_in_asset:
    consume subsidy_budget[reason] by gas_gap_in_asset
    log {reason, trade_id, gas_gap_in_asset, remaining}
    proceed to broadcast
else:
    reject
```

**Subsidy budgets MUST be funded by the operator treasury** — NOT
by PFV, FM-V2, CV, IF, or any protocol-held balance. Per
`BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` §8.5 hard rule.

### 2.3 What is NOT a sponsorship model

- **No L1-meta-tx / gas-relayer abstraction** today. Users sign
  trade payloads; BE submits and pays gas. There is no
  `payForGas(user, amount)` surface in the protocol.
- **No on-chain refund** from PFV → BE per trade. PFV's
  `withdrawRevenue` is owner-only (now Timelock) and is a manual
  treasury action, not a hot-path flow.

---

## 3. The on-chain fee / rebate model (recap)

Per `FeesManagerV2.sol`:

```
notional_in_asset           : settlement-asset units of the trade size
makerPpm, takerPpm          : signed int32 per (tier, ProductKind)
                              maker can be negative (rebate)
                              taker is always >= 0
makerDiscountPpm,
takerDiscountPpm            : RFQ-only, applied as a reduction
                              on the absolute |makerPpm| / takerPpm

effective_maker_ppm (RFQ)   = makerPpm * (PPM - makerDiscountPpm) / PPM
                              (sign preserved)
effective_taker_ppm (RFQ)   = takerPpm * (PPM - takerDiscountPpm) / PPM
                              (always >= 0)

maker_charge_or_credit      = signed_round(notional * effective_maker_ppm / PPM)
                              positive ⇒ maker pays fee
                              negative ⇒ maker receives rebate
taker_charge                = round(notional * effective_taker_ppm / PPM)
                              always >= 0

gross_fee_revenue           = max(0, maker_charge_or_credit) + taker_charge
total_rebate_outflow        = max(0, -maker_charge_or_credit)
net_protocol_revenue        = gross_fee_revenue - total_rebate_outflow
                              (may be <= 0 if maker rebate exceeds taker fee)
```

Storage effects per `consumeFees`:
- positive maker / taker side → debit user CV sub-balance → credit
  PFV via `onFeeCharged` → `PFV.feeBalance[asset] += amount`.
- negative maker side (rebate) → credit user CV sub-balance from
  `rebateBudget[asset]` → call `onRebatePaid` → decrement
  `PFV.rebateReserve[asset]` by `amount`. Requires
  `PFV.rebateReserve[asset] >= amount` (else PFV `onRebatePaid`
  reverts `InsufficientRebateReserve`). **Today**
  `rebateReserve(mUSDC)=0` ⇒ any rebate-positive trade WILL revert
  at the PFV hook — backend MUST NOT broadcast such trades until
  the operator runs `allocateToRebateReserve` via Timelock.

**Bounds:**
- `MAX_TAKER_FEE_PPM = 1000` (+10 bps)
- `MAX_MAKER_REBATE_PPM = -1000` (−10 bps)
- `setFeeProfile` enforces `makerPpm ∈ [-1000, 1000]`,
  `takerPpm ∈ [0, 1000]`. Source line 527-528.

---

## 4. Backend P&L per trade — formal model

Convert everything to **settlement-asset units** (`asset_unit`,
e.g. mUSDC base units) using a fresh oracle quote `eth_price_in_asset`.

```
inputs
------
notional            : asset_unit          (trade size at execution price)
trade_kind          : ORDERBOOK | RFQ
product_kind        : OPTION | PERP
tier_maker, tier_taker  : uint8 0..TIER_COUNT-1
asset               : settlement-asset address
isLiquidation       : bool                (this trade originates from a liquidation flow)

raw schedule (read from FM-V2.getProfile / getRfqDiscountProfile)
----------------------------------------------------------------
makerPpm, takerPpm                            : int32, uint32
makerDiscountPpm_rfq, takerDiscountPpm_rfq    : uint32 (0 if ORDERBOOK)

per-side charge
---------------
effective_maker_ppm = makerPpm * (PPM - makerDiscountPpm_rfq) / PPM  (RFQ)
                    = makerPpm                                      (ORDERBOOK)
effective_taker_ppm = takerPpm * (PPM - takerDiscountPpm_rfq) / PPM  (RFQ)
                    = takerPpm                                      (ORDERBOOK)

maker_signed_amount = signed_round(notional * effective_maker_ppm / PPM)
taker_amount        = round(notional * effective_taker_ppm / PPM)

fee/rebate accounting (asset_unit)
----------------------------------
gross_fee_revenue   = max(0, maker_signed_amount) + taker_amount
total_rebate_outflow= max(0, -maker_signed_amount)
net_protocol_revenue= gross_fee_revenue - total_rebate_outflow

gas accounting (native wei → asset_unit)
----------------------------------------
gas_units           = simulation gas estimate
max_fee_wei         = chosen EIP-1559 maxFeePerGas
gas_cost_wei        = gas_units * max_fee_wei         (worst-case L2)
l1_data_cost_wei    = simulation l1Fee returndata     (worst-case L1)
total_gas_wei       = gas_cost_wei + l1_data_cost_wei
gas_cost_in_asset   = total_gas_wei * eth_price_in_asset / 1e18

expected PnL (asset_unit)
-------------------------
expected_pnl_in_asset = net_protocol_revenue - gas_cost_in_asset
```

### 4.1 Decision states

| State | Test | Action |
|---|---|---|
| **PROFITABLE** | `expected_pnl_in_asset ≥ pnl_floor` (e.g. `≥ 0`) | broadcast (subject to §6 anti-griefing) |
| **AT-COST** | `0 ≤ gas_cost_in_asset × safety_margin ≤ net_protocol_revenue` | broadcast |
| **SUBSIDISABLE** | not at-cost, AND `subsidy_budget[reason].remaining ≥ gap` | broadcast; debit subsidy budget (§2.2) |
| **REJECT** | none of the above | drop candidate; surface to ops |

**`pnl_floor` is configurable per environment** (recommended Sepolia
= 0; mainnet ≥ minimum economic margin).

### 4.2 Rebate solvency gate (HARD)

```
if total_rebate_outflow > 0:
    require FM-V2.rebateBudget[asset] >= total_rebate_outflow
    require PFV.rebateReserve[asset]  >= total_rebate_outflow
    if either is false:
        REJECT — would revert on chain
```

Backend MUST never broadcast a trade that would `InsufficientRebateBudget` or `InsufficientRebateReserve` on chain. Wastes gas and exposes a re-entrant race vs. the budget-funding flow.

### 4.3 Zero-fee and negative-fee maker compatibility

The model in §4 supports both:

- **Zero-fee maker:** `makerPpm = 0` ⇒ `maker_signed_amount = 0` ⇒
  `gross_fee_revenue = taker_amount` only. If `taker_amount ≥ gas_cost × safety_margin` → PROFITABLE. Otherwise SUBSIDISABLE if a budget covers the gap.
- **Negative-fee maker (rebate):** `makerPpm < 0` ⇒
  `maker_signed_amount < 0` ⇒ `total_rebate_outflow > 0` AND
  `net_protocol_revenue = taker_amount - |maker_signed_amount|`. If
  positive and ≥ gas safety floor → PROFITABLE. If positive but
  insufficient OR negative → SUBSIDISABLE if budget covers the
  entire gap AND §4.2 solvency holds AND `rebateReserve` is funded
  (today: 0 ⇒ all rebate trades REJECT).

This makes the same `should_broadcast` decision function work for
all three regimes (positive maker fees, zero-fee makers, rebate
makers) without branching.

---

## 5. When backend MAY broadcast vs MUST reject

### 5.1 MAY broadcast (all gates green)

```text
[ ] simulation cast call succeeds (no revert, gas estimate ≤ HARD_GAS_CAP)
[ ] EIP-712 buyer/seller signatures verify against trade typehash
    (TRADE_TYPEHASH for ORDERBOOK, RFQ_TYPEHASH for RFQ)
[ ] per-side nonces not yet consumed
[ ] trade deadline / expiry in the future
[ ] product listed in OptionProductRegistry
[ ] buyer & seller CV margin headroom ≥ required (RM snapshot fresh)
[ ] rebate solvency gate (§4.2) passes
[ ] NEW_OME.paused() = false
[ ] decision state ∈ {PROFITABLE, AT-COST, SUBSIDISABLE}
[ ] BE balance ≥ FUND_FLOOR after deducting worst-case gas
[ ] dedupe cache miss
[ ] not a liquidation (else use §10 path)
```

### 5.2 MUST reject

```text
- simulation reverts
- signature invalid / nonce consumed / deadline expired
- product delisted
- margin insufficient
- rebate solvency gate fails
- NEW_OME.paused() = true
- decision state = REJECT
- BE balance below FUND_FLOOR (also page on-call per BE custody §5)
- dedupe cache hit
- maxFeePerGas chosen would exceed FEE_CAP_BPS_OVER_BASEFEE
- gas estimate > HARD_GAS_CAP
- trade originates from an unauthenticated source (the backend itself
  is the only authority that produces valid candidates; reject anything
  posted directly to BE without going through the matcher)
```

---

## 6. Anti-griefing checks

A user-signed trade pair carries an implicit incentive vector for
each side. Attacks the policy MUST close:

| Threat | Mitigation |
|---|---|
| **Gas drain via low-fee zero-PnL spam.** Attacker submits many crossing pairs with `makerPpm = 0`, `takerPpm = 1` (≈ 0.01 bp) — each costs BE gas, earns ~ε protocol revenue. | Reject unless `expected_pnl_in_asset ≥ pnl_floor` (PROFITABLE) OR explicit subsidy budget (§2.2). Default Sepolia `pnl_floor = 0`. |
| **Notional inflation via near-zero price.** Attacker pumps notional artificially small so per-side amounts round to 0 yet gas burned. | Reject if `gross_fee_revenue == 0 AND total_rebate_outflow == 0` (no economic content). |
| **Rebate harvesting.** Maker submits a rebate-positive trade against themselves (wash) to mine `rebateReserve`. | Wash-detection: same beneficial owner on both sides — match buyer/seller addresses or known-cluster heuristics. Reject. Plus: cap `total_rebate_outflow` per maker per epoch (e.g. ≤ 5% of `rebateBudget`). |
| **Front-run on price-sensitive RFQ.** Attacker watches the mempool, races BE's broadcast with a competing tx. | BE uses private mempool / sequencer endpoint when available; sets `maxPriorityFeePerGas` modestly to avoid bidding wars. Pre-image of RFQ details must not leak before broadcast. |
| **Gas blow-up via heavy products.** Attacker constructs an option series with worst-case settlement gas. | `HARD_GAS_CAP` (recommended 1.5e6) rejects beyond a sane bound. Per-product gas profile in a backend table; reject if estimate > 1.5× per-product mean. |
| **Replay across queues.** Same trade payload re-submitted after partial failure. | 24-hour dedupe cache keyed on EIP-712 digest. |
| **Nonce hole + replay.** Old nonce reused after a queue prune. | Track per-address consumed-nonce window with bounded retention (e.g. last 10 000 trades). |
| **Rebate-budget exhaustion DoS.** Attacker drains `rebateBudget` so honest makers see InsufficientRebateBudget. | Per-period rebate cap (above) + alerting when `rebateBudget < refill threshold` so operator tops up. |
| **MEV via signature reordering.** Attacker reorders signed lots to extract a cheaper match. | Deterministic matching order per epoch (FIFO with price-time priority); audit log of pre/post-match ordering. |
| **Compromised BE silently subsidising attacker.** | Subsidy debits MUST be paged if `subsidy_budget[reason].remaining` drops > 20% in any 1 h window; per-reason caps enforced. |

---

## 7. Liquidation treatment

Liquidations are economically distinct from normal trading and have
a separate path. Recommended carve-out:

| Dimension | Normal trade | Liquidation trade |
|---|---|---|
| Rebate path | maker may receive rebate | **NO rebate to either side** — disable `makerPpm < 0` for liquidation calldata |
| Fee path | grossFee → PFV | liquidation penalty → InsuranceFund per RG params (see `RiskGovernorInterfaces.sol:85` `setLiquidationParams`); separate ledger |
| `pnl_floor` | `PROFITABLE` is normal mode | **Allowed to be AT-COST or operator-subsidised** because the protocol benefits from timely liquidation even if the per-tx grossFee is small |
| Decision state | §4.1 4-state | additional **LIQUIDATION** state — always proceeds if rebate disabled AND margin checks pass, even if `expected_pnl_in_asset < 0` |
| Subsidy budget | per reason from operator treasury | named `subsidy_budget["liquidation"]` ≥ 0; capped per epoch; metric tracked separately |
| Anti-griefing | §6 wash + frequency + cap | additionally: liquidatee position MUST be flagged "liquidatable" by RM snapshot; reject if RM does not currently flag the account |
| Gas cap | `HARD_GAS_CAP` | same cap; liquidations are gas-bounded by per-product seizure surface |
| Pause | NEW_OME `whenNotPaused` | additionally honour `RiskGovernor.liquidationPaused` flag (see `RiskGovernorInterfaces.sol:59`) — BE must read and gate |

### 7.1 Why a separate path

If liquidations shared the §4 PROFITABLE gate, a healthy
liquidatable account whose seizure gas cost briefly exceeded the
fee revenue would be skipped — leaving a margin-deficient position
to grow. Liquidations are a protocol-defence action; the operator
accepts mild gas drag in exchange for timely closure. Modelling
this as `subsidy_budget["liquidation"]` makes the budget visible
and capped instead of hidden.

### 7.2 Source-of-truth for the liquidation flag

Backend MUST consult the RiskModule (or a RiskGovernor-derived view)
per candidate. The on-chain check is the final authority:
simulation reverts if RM rejects, but BE should pre-filter to
avoid wasted simulation gas.

---

## 8. Pseudocode — `should_broadcast(order)`

```python
def should_broadcast(order):
    # ── 0. Pre-flight static checks ────────────────────────────
    if dedupe_cache.has(order.digest):           return False, "dupe"
    if not eip712_verify(order, order.buyerSig): return False, "buyer-sig"
    if not eip712_verify(order, order.sellerSig):return False, "seller-sig"
    if now() >= order.deadline:                  return False, "expired"
    if nonces_consumed(order):                   return False, "nonce"
    if not opr.is_listed(order.product):         return False, "delisted"
    if not rm.snapshot_fresh():                  return False, "stale-rm"

    # ── 1. NEW_OME live state ──────────────────────────────────
    if new_ome.paused():                         return False, "ome-paused"
    if not new_ome.is_executor(BE):              return False, "be-not-exec"
    if be.balance() < FUND_FLOOR:                return False, "be-low-bal"

    # ── 2. Margin / product guards ─────────────────────────────
    if not rm.has_margin(order.buyer, order.required_margin_buy):
                                                 return False, "buyer-margin"
    if not rm.has_margin(order.seller, order.required_margin_sell):
                                                 return False, "seller-margin"

    # ── 3. Simulate ────────────────────────────────────────────
    sim = eth_call_execute_trade(order)
    if sim.reverted:                             return False, f"sim:{sim.err}"
    if sim.gas_units > HARD_GAS_CAP:             return False, "gas-cap"

    # ── 4. Fee / rebate computation ────────────────────────────
    fee = compute_fee_split(order)        # returns the §4 model fields
    if (fee.gross_fee_revenue == 0
        and fee.total_rebate_outflow == 0):      return False, "no-econ-content"

    # ── 5. Rebate solvency (HARD) ──────────────────────────────
    if fee.total_rebate_outflow > 0:
        if fm_v2.rebate_budget(order.asset) < fee.total_rebate_outflow:
                                                 return False, "rebate-budget"
        if pfv.rebate_reserve(order.asset) < fee.total_rebate_outflow:
                                                 return False, "rebate-reserve"

    # ── 6. Anti-griefing (§6) ──────────────────────────────────
    if same_beneficial_owner(order.buyer, order.seller):
                                                 return False, "wash"
    if maker_rebate_quota_breached(order):       return False, "rebate-quota"
    if recent_attack_pattern(order):             return False, "pattern"

    # ── 7. Gas cost in asset terms ─────────────────────────────
    eth_price = oracle.eth_price_in_asset(order.asset)
    gas_units_to_use = sim.gas_units * GAS_SAFETY_FACTOR
    max_fee = choose_max_fee_per_gas()
    total_gas_wei = gas_units_to_use * max_fee + sim.l1_data_fee_wei
    gas_cost_in_asset = total_gas_wei * eth_price // (10 ** 18)

    expected_pnl = fee.net_protocol_revenue - gas_cost_in_asset

    # ── 8. Liquidation carve-out (§7) ──────────────────────────
    if order.is_liquidation:
        # Disallow rebate to either side in liquidation calldata
        if fee.total_rebate_outflow > 0:         return False, "liq-rebate"
        if rg.liquidation_paused():              return False, "liq-paused"
        if not rm.flags_liquidatable(order.liquidatee):
                                                 return False, "not-liquidatable"
        # Subsidy-gated, ignores pnl_floor
        gap = max(0, gas_cost_in_asset - fee.net_protocol_revenue)
        if gap > 0:
            if not subsidy_take("liquidation", gap):
                                                 return False, "liq-budget"
        return True, "liquidation"

    # ── 9. Normal decision states ──────────────────────────────
    if expected_pnl >= PNL_FLOOR:                # PROFITABLE
        return True, "profitable"
    if fee.net_protocol_revenue >= gas_cost_in_asset * SAFETY_MARGIN:
                                                 # AT-COST
        return True, "at-cost"

    gap = (gas_cost_in_asset * SAFETY_MARGIN) - fee.net_protocol_revenue
    reason = classify_subsidy_reason(order)      # e.g. "mm-bootstrap"
    if subsidy_take(reason, gap):                # SUBSIDISABLE
        return True, f"subsidy:{reason}"

    return False, "uneconomic"
```

Notes:
- `subsidy_take(reason, gap)` atomically debits `subsidy_budget[reason]` AND emits a structured log; returns `True` iff it succeeded.
- `compute_fee_split` is the §4 math, exposed as a backend helper that exactly matches `_quoteFees` in `FeesManagerV2.sol`.
- Every `False` return logs the reason; every `True` return logs the chosen decision-state plus the post-budget remaining.

---

## 9. Recommended parameters (Sepolia rehearsal)

| Param | Recommended Sepolia value | Notes |
|---|---|---|
| `HARD_GAS_CAP` | `1 500 000` | gas units; rejects pathological products |
| `GAS_SAFETY_FACTOR` | `1.25` | inflate simulated gas to cover prediction error |
| `MAX_MAX_FEE_PER_GAS` | `basefee × 3 + 2 gwei` | EIP-1559 maxFee ceiling |
| `MAX_PRIORITY_FEE_PER_GAS` | `2 gwei` | priority fee cap (Base usually ≤ 0.01 gwei in practice) |
| `SAFETY_MARGIN` | `1.5` | gas vs fee revenue ratio for AT-COST |
| `PNL_FLOOR` | `0` | Sepolia; mainnet `> 0` |
| `FUND_FLOOR` | `1 × 10^15` wei (~0.001 ETH) | per `BACKEND_EXECUTOR_CUSTODY_PROFILE` §4 |
| `FUND_TARGET` | `1 × 10^16` wei (~0.01 ETH) | top-up target |
| `FUND_CEILING` | `5 × 10^16` wei (~0.05 ETH) | bounds key-compromise loss |
| `DEDUPE_TTL` | `24 h` | EIP-712 digest cache |
| `NONCE_WINDOW` | last `10 000` trades per address | replay protection |
| `subsidy_budget["mm-bootstrap"].cap` | TBD; recommend `100 000` asset-units / week | operator-set |
| `subsidy_budget["liquidation"].cap` | TBD; recommend `50 000` asset-units / week | operator-set |
| `subsidy_budget["rfq-recovery"].cap` | TBD; recommend `0` (off) until rfq smoke is allowed | operator-set |
| maker_rebate_quota | `5%` of `FM-V2.rebateBudget[asset]` per maker per 24 h | per §6 |
| `rebateReserve` minimum (PFV side) | `≥ 100 000` mUSDC before any rebate trade enabled | **today: 0** — rebate-trades DISABLED until operator allocates |
| Oracle freshness (eth_price_in_asset) | `≤ 60 s` since last update | reject candidates if stale |

Mainnet values (tighter; tracked at §15) will adjust `PNL_FLOOR > 0`, lower `MAX_MAX_FEE_PER_GAS` ceiling, and tighten subsidy caps.

---

## 10. Implementation TODOs

| # | Item | Owner | Notes |
|---|---|---|---|
| T-1 | Wire `compute_fee_split` against `FM-V2._quoteFees` (mirror or re-export to JSON ABI snapshot) | backend | Must match on-chain math to avoid simulation/reality divergence. |
| T-2 | Wire `should_broadcast` per §8 into the matcher | backend | Replace any prior all-allow policy. |
| T-3 | Persistent nonce-window store + dedupe cache | backend | Survives backend restart; per V2G-M2 restart playbook. |
| T-4 | Subsidy budget registry with named reasons + per-reason cap + 1-h alert window | backend + ops | Off-chain accounting; never debits on-chain balances. |
| T-5 | Rebate-solvency probe (§4.2) integrated with matcher pre-flight | backend | Cheap `cast call` per candidate. |
| T-6 | Wash-trade detection (same beneficial owner) | backend + risk | Initial heuristic: identical address; later: cluster heuristics + address graph. |
| T-7 | Liquidation flagger view (RM → `is_liquidatable(addr)`) | backend | Provided by RM today; ensure stable across margin engine upgrades. |
| T-8 | Monitoring + alerts per §11 wired into PagerDuty / Discord | SRE | Tied to `BACKEND_EXECUTOR_CUSTODY_PROFILE` §5.3 routing. |
| T-9 | Per-product gas profile table; update via load test | backend | Used by T-2 (HARD_GAS_CAP individualisation) and capacity planning. |
| T-10 | Unit tests covering each §5.1 / §5.2 / §6 / §7 / §8 branch | backend | Required pre-mainnet (BE-PROD-4 in custody profile §9). |

---

## 11. Monitoring / alert requirements

Per-trade metrics:

```text
broadcast_ok                                (counter)
broadcast_reject{reason}                    (counter, label = §8 reason)
liquidation_broadcast                       (counter)
subsidy_take{reason, amount, remaining}     (gauge / log event)
expected_pnl_in_asset                       (histogram)
actual_pnl_in_asset                         (histogram, post-receipt)
gas_units_estimated_vs_actual               (histogram)
maxFeePerGas_used                           (histogram)
rebate_outflow                              (sum)
rebate_budget_remaining                     (gauge, polled from chain)
rebate_reserve_remaining                    (gauge, polled from chain)
```

Alerts:

| Signal | Severity | Routing |
|---|---|---|
| `BE balance < FUND_FLOOR for > 5 min` | PAGE | PagerDuty |
| `broadcast_reject{reason="rebate-reserve"}` > 0 in any 5 min | PAGE | PagerDuty (means rebate-trade backed up; ops must allocate) |
| `subsidy_budget[reason].remaining` drops > 20% in any 1 h window | PAGE | PagerDuty |
| `actual_pnl_in_asset` deviates from `expected_pnl_in_asset` by > 30% over 100-trade window | PAGE | PagerDuty (sim/reality drift) |
| `broadcast_reject{reason="sim:*"}` rate > 5% over 15 min | PAGE | PagerDuty |
| `liquidation_broadcast == 0` for > 30 min while RM flags any liquidatable account | PAGE | PagerDuty |
| `maxFeePerGas_used` hits `MAX_MAX_FEE_PER_GAS` ceiling > 10× in any 5 min | PAGE | PagerDuty (gas spike or attack) |
| `wash` reject rate spike | PAGE | PagerDuty (potential attack) |
| `BE balance > FUND_CEILING` | DISCORD | OPs channel (bound loss surface; rotate or drain) |
| `subsidy budget total burn / 24 h` > expected baseline | DISCORD | OPs review |
| `expected_pnl_in_asset` median > 50% drop over 1 h | DISCORD | OPs review |

Log retention: 30 d hot, 1 y cold (matches BE custody §5.2).

---

## 12. Sepolia vs mainnet gating

Current Sepolia state forbids any rebate-bearing trade (rebateReserve = 0). The policy compiles cleanly under this constraint because §4.2 will REJECT every rebate path. The order of operations to unlock rebate trades is:

```text
1. Operator runs Timelock-queued PFV.allocateToRebateReserve(mUSDC, X)
   to move X mUSDC from PFV.feeBalance into PFV.rebateReserve.
   (24 h queue + execute; standard Timelock path now that PFV owner = Timelock.)
2. Verify PFV.rebateReserve(mUSDC) = X, PFV.feeBalance(mUSDC) = 28 - X,
   drift = 0 still (only reshuffles within PFV).
3. Backend's §4.2 gate unblocks; rebate trades become broadcastable.
4. Backend monitors §11 metrics for the first cycle.
```

This MUST NOT happen as part of this milestone. It is a future GOV-derived step. The policy is forward-compatible.

---

## 13. What this policy does NOT change on chain

- Selectors / fee schedules / tier matrix / rebate budget — all on-chain governance-gated (Timelock).
- `MAX_TAKER_FEE_PPM` / `MAX_MAKER_REBATE_PPM` — solidity constants.
- PFV / FM-V2 owner — Timelock.
- NEW_OME executor surface — flips in V2G-GOV-F-B-X (this policy applies AFTER that lands).

This is a **backend-only operational policy.** It cannot widen the on-chain rules; it can only narrow which subset of permissible trades the backend chooses to broadcast.

---

## 14. Open follow-ups / blockers

| Tag | Item | Owner | Notes |
|---|---|---|---|
| GFR-Q1 | Operator-set subsidy budget caps for each named reason | Operator + Finance | Required before opting any trade into SUBSIDISABLE. |
| GFR-Q2 | Wire `compute_fee_split` against the FM-V2 ABI (with property-based tests to ensure parity) | Backend | Required before §4 gates are trustworthy. |
| GFR-Q3 | Per-product gas profile table | Backend + load test | Required for T-9 and per-product `HARD_GAS_CAP` tuning. |
| GFR-Q4 | Wash-detection heuristic for V1 (address-equality only) → V2 cluster heuristic | Backend + risk | V1 is gating but coarse; V2 reduces false negatives. |
| GFR-Q5 | RM `is_liquidatable(addr)` stable view exposed to backend | Backend + protocol | Required for §10. |
| GFR-Q6 | Allocation of rebateReserve via Timelock (separate milestone) | Operator | Unlocks rebate trades; see §12. |
| GFR-Q7 | Mainnet variant of this doc with tighter caps + `PNL_FLOOR > 0` | Future | Tracked at §15. |

None of these block Sepolia GOV-F-B-X broadcast or this V1 doc.

---

## 15. Sepolia → mainnet fork

A `BACKEND_GAS_FEES_REBATES_POLICY_V2G_Y_MAINNET.md` fork must:

- Set `PNL_FLOOR` strictly positive (cover operating margin).
- Lower `MAX_MAX_FEE_PER_GAS` to Ethereum-mainnet economics; raise priority-fee cap meaningfully.
- Re-derive `subsidy_budget` caps in fiat-anchored units.
- Run drilled compromise-rotate-unpause cycle on Sepolia first (`BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` §9 BE-PROD-7).
- Audit + sign-off before any mainnet trade.

---

## 16. Cross-links

- `deopt-v2-sol/docs/BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` — key/funding/monitoring custody policy.
- `deopt-v2-sol/docs/GOVERNANCE_EXECUTOR_MIGRATION_QUEUE_RESULT_V2G_GOV_F_B_Q.md` — Q closure that gates X (and therefore this policy).
- `deopt-v2-sol/docs/GOVERNANCE_TIMELOCK_CLEANUP_PREP_V2G_GOV_G_PREP.md` — V2G-GOV-G dependency.
- `deopt-v2-sol/docs/FEES_MANAGER_V2_ADMIN_BUDGET_MATRIX_V2G_R2.md` — rebate-budget operational matrix.
- `deopt-v2-sol/docs/DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md` — canonical fee audit pack.
- `deopt-v2-backend/docs/V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md` — readiness gate.
- `deopt-v2-backend/docs/OPTION_EXECUTION_BACKEND.md` — backend service architecture.
- `~/DEOPT/RESUME_GOV_F_B_X.md` — operator resume runbook for Phase X broadcast.
