# PerpEngine V2 accounting design — resolution of the mutual-close PnL double-count

**Status:** design proposal. Do NOT deploy. No Solidity fix committed.
**Author:** operator + automated audit
**Discovery milestone:** `PERPS_CLOSE_PNL_DOUBLE_COUNT_BUG_CONFIRMED`
**Resolution milestone (this):** `PERPS_CLOSE_PNL_BUG_RETIRED_AND_V2_ACCOUNTING_DESIGN`

---

## 1. Root cause (executive summary)

`PerpEngineTrading._applyRealizedCashflow` (deployed at
`0xc6C592100723Fe0C66343A16e95eC34cC0c2141c`) computes the peer-to-peer
cashflow amount as:

```solidity
int256 netToBuyer1e8 = _checkedSubInt256(buyerRealizedPnl1e8, sellerRealizedPnl1e8);
```

`_computeNextPosition` (in `PerpEngineStorage`) returns per-side
**trader-perspective** `realizedPnl1e8`:

```solidity
realizedPnl1e8 = closedMarkValue1e8 − removedBasis1e8 − closedFunding1e8
```

For a **mutual close** (both counterparties of the current trade were
already holding equal-and-opposite positions opened at the same price
with the same funding checkpoint), `buyerRealizedPnl + sellerRealizedPnl
== 0`. Under this invariant:

* `netToBuyer = buyerRealized − sellerRealized = 2 × buyerRealized`

The engine therefore transfers **2× the economic PnL** from the loser
to the winner. Confirmed reproduction (Anvil fork of Base Sepolia
@ block 46 974 984, bytecode-identical):

| | Δvault(A) | Δvault(B) | Δvault(fee sink) | Sum |
|---|---|---|---|---|
| Economic model (correct) | +243 027 | −251 753 | +8 726 | 0 |
| **Engine observed** | **+487 301** | **−496 027** | +8 726 | 0 |
| Ratio (PnL portion) | **2.00×** | **2.00×** | 1.00× | — |

Global mUSDC conservation is preserved (nothing minted/burned) but the
distribution between the two traders is doubled at the winner's gain
and the loser's expense.

## 2. Cases where `buyerRealized + sellerRealized ≠ 0`

The "mutual close doubles" analysis assumed zero-sum. That assumption
holds ONLY for a specific class of trades. In general it does **not**:

### 2a. Asymmetric historical entry prices
A entered long at `P_A`, B entered short at `P_B` (with `P_A ≠ P_B`,
e.g. because they were matched against different counterparties over
time). They now close together at `P_close`.
```
realized_A = +size × (P_close − P_A) / 1e8
realized_B = −size × (P_close − P_B) / 1e8
sum        = size × (P_B − P_A) / 1e8       ≠ 0 in general.
```
Neither the engine's current formula (`buyerRealized − sellerRealized`)
NOR the "max-abs" proposal correctly settles this. Under peer-to-peer
settlement the trade cannot conserve — there is a genuine excess
`size × (P_B − P_A)` that has no funding source.

### 2b. Nonzero funding
Each side's `closedFunding1e8` is a function of that side's private
`lastCumulativeFundingRate1e18` checkpoint, which was set when THAT
side last increased its position. Two counterparties in the same trade
generally have different checkpoints (they were originally matched
against different partners at different times). Even if `P_A = P_B`,
`closedFunding_A ≠ closedFunding_B` and the zero-sum property fails.

### 2c. Partial closes
`_computeNextPosition` uses `closeAbs = min(|oldSize|, |delta|)`. For
a buyer with `oldSize = −1M` and `delta = +500K`, only half the position
closes. If the seller has `oldSize = +200K` and `delta = −500K`, the
seller partially closes AND partially opens (position flip). The
`realized` values are computed from asymmetric `closeAbs` per side and
do not sum to zero.

### 2d. Position flips
When a side's `newSize = oldSize + delta` crosses zero (e.g.
`oldSize = +200K, delta = −500K → newSize = −300K`), the engine takes
the "close then reopen" branch. The new-basis (post-flip) portion
uses `signedNotional(newSize, executionPrice)` while realized uses
`closedMarkValue − removedBasis`. Cross-side zero-sum only holds if
BOTH sides flip identically — not generally.

### 2e. Rounding asymmetry
`_signedMarkValue1e8` uses abs-then-sign to avoid Solidity's `-a/b`
truncation-toward-zero pitfall. So for opposite-sign symmetric
positions, both sides compute the SAME magnitude via `|size| × price /
PRICE_1E8` — perfect zero-sum. But when the closed sizes differ
between sides (§2c/§2d), the rounding remainders are also asymmetric
and can leave a 1e0-scale residue after the sum. In native mUSDC
(6 decimals) this is invisible (rounded to 0), but at 1e8-precision
it's real.

### 2f. Multi-side aggregate consideration
The engine treats each trade as bilateral: buyer + seller only. But a
trader's TOTAL realized PnL over their lifecycle sums to zero **only
against ALL counterparties across ALL trades**, not against any single
counterparty. Peer-to-peer settlement of realized-per-trade only works
when the peer-to-peer counterparty is exactly the original opening
match. In an order book this is generally NOT the case.

**Conclusion:** the peer-to-peer settlement design is only correct
under a very restrictive invariant (matched-open, matched-close, same
funding checkpoint) that the deployed engine does not enforce and
cannot enforce without breaking normal order-book semantics.

## 3. Required clearing invariant for V2

The clearing system MUST satisfy:

1. **Per-trader economic correctness.** Each trader's cumulative
   vault change equals their cumulative realized PnL minus fees, over
   any sequence of trades:
   ```
   Δvault(trader) = Σ realizedPnl_trader_per_trade − Σ fees_trader
   ```
2. **Global conservation.** No mUSDC is minted or burned by the
   engine. Vault delta sum across all accounts equals zero.
3. **Locality.** The correctness of trade `T`'s vault deltas does not
   depend on future trades. The engine cannot rely on "it'll balance
   out later".
4. **Idempotency & determinism.** Given the pre-trade state, the
   post-trade state (positions + vault) is deterministic and does
   not depend on unobserved counterparty history.

Invariants 1 and 2 CAN conflict for a purely bilateral engine when
`buyerRealized + sellerRealized ≠ 0` (see §2). Reconciling the two
requires either (a) constraining trades to zero-sum-preserving shapes
or (b) introducing a clearing account.

## 4. Three settlement models — comparison

### Model A — bilateral peer-to-peer settlement
Every trade's realized PnL is settled purely between the two
counterparties of that trade. Conservation is trivially local.
**Failure mode:** cannot handle asymmetric entry / funding / partial
closes (§2). This is what the deployed engine attempts and where the
2× bug lives.
Verdict: **rejected** for V2.

### Model B — per-side settlement through a clearing account
The engine treats each side of a trade as a settlement against a
neutral clearing account. `vault(buyer) += buyerRealized;
vault(seller) += sellerRealized`. Any asymmetry
`sum = buyerRealized + sellerRealized` is applied to the clearing
account: `vault(clearing) −= sum`. Global conservation preserved.
Per-trader correctness preserved (each side gets exactly their own
realized).
* **Clearing account must be pre-funded** with enough mUSDC to absorb
  the largest expected asymmetry across concurrent open positions.
  This is the perpetual "insurance fund" pattern used by dydx,
  Perpetual Protocol, Drift.
* **Backing source options:**
  * Protocol fee revenue (fee sink → clearing account overflow).
  * Governance-funded reserve.
  * Socialized-loss cap (last resort).
* **New failure mode:** insufficient clearing balance under extreme
  divergence (asymmetric matched positions plus large price moves).
  Requires solvency invariants + admin top-up procedures.
Verdict: **recommended baseline for V2.**

### Model C — variation-margin / continuously-settled accounting
Positions are mark-to-market continuously (or at every block, or at
every funding tick). Each trader's vault delta at each mark-to-market
tick equals `size × (mark_new − mark_prev) / 1e8`. When positions
close, `realized = 0` (the P&L has already been paid tick-by-tick).
* Trades become size-and-basis updates only; no cashflow at trade
  time.
* Conservation is automatic per tick because for every long-side gain
  of `size × Δmark`, some short somewhere loses the same.
* Standard futures-clearing design (CME, exchanges).
* **Failure mode:** requires an out-of-band "mark-to-market" job
  (funding-rate worker already exists in this codebase but is
  currently disabled for closed test). Extreme dust-accumulation
  possible from rounding at each tick.
Verdict: **strong alternative if the funding worker is architecturally
committed.** Simpler than Model B but requires operational commitment
to the mark-to-market cadence.

## 5. What backs positive realized PnL when both sides profit vs their own basis?

Scenario: A opened long at `P_A = 240`. B opened short at `P_B = 260`.
Trade at `P_close = 250`. Neither counterparty is closing against
their original opener.

* `realized_A = +size × (250 − 240) / 1e8 = +10 × size`
* `realized_B = +size × (260 − 250) / 1e8 = +10 × size` (short profits
  when price falls; 260 → 250 = fall)
* `sum = +20 × size`

**Both traders are entitled to positive realized PnL.** No amount of
peer-to-peer arithmetic between A and B can honour this: no matter how
you route cash between only A and B, at least one of them cannot get
what they're economically owed.

The only sources that can fund the `+20 × size` excess:
1. **Clearing account** funded by earlier trades where `sum < 0` (both
   sides had losses vs their own basis, e.g. A entered long at 260 and
   closes at 250 while B entered short at 240 and closes at 250 —
   both lose). Cross-time smoothing.
2. **Protocol fee revenue** — fee sink balance is systematically drained
   into the clearing account.
3. **External governance top-up** — administrative deposit.
4. **Socialized haircut** — trim each winner proportionally when the
   clearing account is empty (LAST resort, degrades product quality).

**No purely-bilateral fix exists.** The engine MUST have access to a
clearing/reserve balance beyond the two counterparties of the current
trade.

## 6. Whole-system conservation

Under Model B (clearing account):

```
For each trade:
  Δvault(buyer)    += buyerRealized
  Δvault(seller)   += sellerRealized
  Δvault(clearing) -= (buyerRealized + sellerRealized)
  Σ Δvault         = 0   ✓
```

Cumulative over the system's lifetime:
```
Σ_t Δvault(trader_i)     = trader_i's total realized PnL over time
Σ_t Δvault(clearing)     = -(Σ_t Σ_i realizedPnl_i)  = 0 ONLY if the
                            aggregate PnL across all traders sums to
                            zero (which is the ideal "zero-sum
                            perpetual" property).
```

In practice, the clearing account balance walks with the aggregate
asymmetry. Its balance must NEVER go negative — that would mean the
protocol has promised more PnL than it can pay. Enforcement:
* Pre-fund the clearing account.
* Route protocol fee revenue into it.
* Impose per-market position-size caps to bound worst-case asymmetry.
* Circuit-break on approach to zero.

Under Model C (mark-to-market): the clearing-account balance
theoretically stays at zero (each tick's Δvault sums to zero via
funding-rate-style rebalancing) but in practice a dust-accumulator
account is needed to absorb rounding.

## 7. Recommended V2 architecture (smallest safe design)

**Contract name:** `PerpEngineV2` (see §11 for versioning rationale).
**Deployment scope:**
* NEW: `PerpEngineV2` replaces the current `PerpEngine`.
* REUSE: `CollateralVault` (unchanged), `FeesManagerV2` (unchanged),
  `OracleRouter` (unchanged), `PerpMarketRegistry` (unchanged).
* NEW: `PerpClearingAccount` — a designated vault address holding the
  clearing balance. Simplest: a new EOA with mUSDC deposited into
  `CollateralVault.balances[clearing][mUSDC]` via the existing vault
  interface. No new vault contract required.

**Fix in `_applyRealizedCashflow`:**
```solidity
function _applyRealizedCashflow(...) internal {
    // MODEL B — per-side settlement with clearing account.

    address clearing = clearingAccount;
    if (clearing == address(0)) revert ClearingAccountNotConfigured();

    // Apply each trader's realized independently. Any asymmetry is
    // absorbed by / sourced from the clearing account.
    if (buyerRealizedPnl1e8 != 0) {
        uint256 absBuyerNative = _value1e8ToSettlementNative(
            settlementAsset, _absInt256(buyerRealizedPnl1e8));
        if (absBuyerNative > 0) {
            if (buyerRealizedPnl1e8 > 0) {
                _routeIncomingCashflowWithDebtFirst(
                    settlementAsset, clearing, buyer, absBuyerNative);
            } else {
                _routeIncomingCashflowWithDebtFirst(
                    settlementAsset, buyer, clearing, absBuyerNative);
            }
        }
    }
    if (sellerRealizedPnl1e8 != 0) {
        uint256 absSellerNative = _value1e8ToSettlementNative(
            settlementAsset, _absInt256(sellerRealizedPnl1e8));
        if (absSellerNative > 0) {
            if (sellerRealizedPnl1e8 > 0) {
                _routeIncomingCashflowWithDebtFirst(
                    settlementAsset, clearing, seller, absSellerNative);
            } else {
                _routeIncomingCashflowWithDebtFirst(
                    settlementAsset, seller, clearing, absSellerNative);
            }
        }
    }
}
```

Conservation invariant becomes locally trivial: for each trade,
`Δbuyer + Δseller + Δclearing = 0`.

The clearing account MUST NOT go below a governance-configured floor
under normal operation. `_routeIncomingCashflowWithDebtFirst` from
clearing → trader must revert or clamp on empty balance; the pre-fund
policy handles this.

## 8. Migration / deployment plan (no execution in this milestone)

Live-state impact at HEAD `6cc2481`:

| Contract | Redeploy? | State migration required |
|---|---|---|
| CollateralVault | NO | none |
| FeesManagerV2 | NO | none |
| OracleRouter | NO | none |
| PerpMarketRegistry | NO | none |
| PerpMatchingEngine | YES | update `perpEngine` binding to V2 |
| **PerpEngine → PerpEngineV2** | YES | full state migration |
| Backend config | UPDATE | `PERP_ENGINE_ADDRESS` env var |
| Frontend | UPDATE | pointers, ABI cache |

**Position/collateral migration steps:**

1. **Freeze V1.**
   * Admin sets V1 `PerpEngine.pauseTrading = true`.
   * Admin sets V1 `PerpMatchingEngine.paused = true`.
   * Backend refuses new prepares (`PERPS_CLOSED_TEST_ENABLED=false`
     for the migration window).
2. **Snapshot.**
   * Read every non-zero position via `getPositionSize(trader, marketId)`
     + `positions(trader, marketId)` for open notional and funding
     checkpoint.
   * Read every non-zero vault balance
     `CollateralVault.balances(trader, asset)`.
   * Persist snapshot to the backend DB with cryptographic hash of the
     canonical serialization.
3. **Deploy V2.**
   * Deploy `PerpEngineV2` with same market registry, oracle router,
     collateral vault, fees manager, but new `clearingAccount` address.
   * Deploy new `PerpMatchingEngineV2` bound to `PerpEngineV2` (needs
     new address because the domain separator changes on
     `verifyingContract` change).
4. **Import positions.**
   * V2 exposes `adminSeedPosition(trader, marketId, size1e8,
     openNotional1e8, lastCumulativeFundingRate1e18)` gated by
     `onlyOwner` and `whenNotSealed`. Iterate the snapshot, seed each
     position deterministically.
   * Verify each imported position by reading it back and comparing to
     the snapshot hash.
5. **Import clearing-account seed.**
   * Compute the clearing-account initial balance to cover the
     aggregate asymmetry across imported positions. For the closed
     test with only A and B at matched-open, the aggregate asymmetry
     is 0 → clearing account starts empty (this is the simplest case).
   * For a live migration with heterogeneous positions, compute
     `Σ (size × (mark_now − openNotional/size))` across all positions
     and pre-fund accordingly.
6. **Seal.**
   * V2 exposes `sealAdminSeeding()` (one-way) that permanently
     disables `adminSeedPosition`. Call after import.
7. **Unpause trading on V2.**
   * All prepares from step onward use V2 addresses. V1 remains
     available in read-only mode for audit.

**Positions retained in this milestone:**
* Backend HEAD `6cc2481`, tree clean (before commit).
* Base Sepolia positions unchanged: `position(A,1)=+1_000_000,
  position(B,1)=−1_000_000, nonces 1/1, vault A=9 992 595, vault
  B=9 998 765`.
* Close intent `7c6f413a-b219-4379-8f6e-a0f559d66ab6` transitioned to
  `abandoned` via the canonical `retire_execution_intent` repository
  method. Row preserved for audit.

## 9. Alternative migration paths considered (rejected)

* **Continuing to close via V1 with the doubling bug** — rejected:
  double-counts economic P&L, violates trader-signed invariants.
* **Deploying V2 without state migration** — rejected: A + B's
  positions would be orphaned on V1, no way to close them safely.
* **Manual price adjustment to compensate for doubling** — rejected
  by operator directive: "relies on a compensating price hack".
* **Socializing the loss** — rejected: doesn't restore economic
  correctness; only redistributes it.
* **Redeploying with SAME PME address (proxy pattern)** — rejected:
  none of the current contracts are upgradeable (per prior context
  `project_immutable_vault_gating.md`: "Immutable CollateralVault
  blocks per-user launch gating"). PME is also non-upgradeable.

## 10. Live positions strategy

Per operator directive: **leave the current A/B positions open**.
Do not attempt to close them on the deployed engine. When V2 is
deployed with the fix + migration path, the positions will be seeded
into V2 and closed correctly under Model B.

Until V2 deploys, the current on-chain state is:
```
A (long):  +1_000_000 @ 246_831_000_000 (opened 2026-09-18T07:12:26Z)
B (short): −1_000_000 @ 246_831_000_000 (opened 2026-09-18T07:12:26Z)
```
* Positions carry no time-decay (funding worker disabled).
* Vaults hold each trader's remaining collateral (A: 9 992 595, B:
  9 998 765; fee sink: 8 684 after opening trade).
* No liquidation risk with these margin ratios and current mark.

## 11. Versioning

Given the material change in cashflow semantics:
* Solidity contract: **rename to `PerpEngineV2` and `PerpMatchingEngineV2`**.
  New file `src/perp/PerpEngineV2.sol`, new
  `src/matching/PerpMatchingEngineV2.sol`. Do NOT modify V1 files in
  place — audit history must remain intact.
* EIP-712 domain: `name = "DeOptV2-PerpMatchingEngineV2"` (distinct
  from V1 to prevent signature replay between engine versions).
* ABI: fresh ABI export in `abi/PerpEngineV2.json`.
* Backend config: new env var `PERP_ENGINE_V2_ADDRESS`; deprecate
  `PERP_ENGINE_ADDRESS` (keep for V1 audit reads until sunset).
* Docs: new `docs/PERPS_V2_ROLLOUT.md` (out of scope for this
  milestone).

## 12. What was NOT done in this milestone (deferred)

* No Solidity code change.
* No V2 deployment.
* No test-suite expansion for Solidity.
* No position migration.
* No mainnet touch.
* No trader signing.
* No cosign / simulate / arm / broadcast.

## 13. Follow-up milestones (in order)

1. **`PERPS_V2_SOLIDITY_FIX_AND_TESTS_V1`** — implement Model B in
   `PerpEngineV2` + `PerpMatchingEngineV2`. Full close-matrix tests
   with economic reference. Fuzz + invariants that assert per-trader
   correctness AND clearing-account balance floor. Do not deploy.
2. **`PERPS_V2_CLEARING_ACCOUNT_SIZING_V1`** — analytical + Monte
   Carlo sizing of the clearing account balance floor for the closed
   test's parameter regime.
3. **`PERPS_V2_TESTNET_DEPLOY_V1`** — deploy V2 to a fresh Base
   Sepolia address. Wire backend/frontend config. Do NOT migrate live
   positions yet.
4. **`PERPS_V2_MIGRATION_SNAPSHOT_V1`** — snapshot the frozen V1 state
   (positions, vaults, funding checkpoints, market state).
5. **`PERPS_V2_MIGRATION_IMPORT_V1`** — admin-seed the snapshot into
   V2. Seal.
6. **`PERPS_V2_CLOSED_TEST_RESUME_V1`** — resume closed test on V2.
   First act: successfully close the A/B mutual position at the
   correct economic PnL.

---

**End of design.** No Solidity commit encodes the fix. Only the
lifecycle addition (Abandoned status + retire_execution_intent
repository method + retire_execution_intent CLI tool + this document)
is committed.
