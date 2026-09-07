# DEOPT_COLLATERAL_CONCENTRATION_POLICY_V1

Design-only. No collateral is activated. No policy is enforced today.

## Purpose

Answer the milestone Part F question: given that collateral values
are NOT independent (a subaccount long BTC-PERP + backed by cbBTC
loses money on both legs during a BTC drawdown), what is the
**simplest safe launch model** for the platform's first non-USDC
collateral activation?

## Model options considered

### (A) Static haircuts only

Each asset carries a `collateralFactorBps` set conservatively enough
to absorb worst-case correlated moves within a single asset. E.g.
cbBTC at 60% factor when the risk model expects a 30% one-day BTC
drawdown to leave 30% margin buffer.

**Pros**: trivial to reason about, one number per asset, matches
what every peer perp/options exchange does at launch (Derive, Aevo,
GMX v2). No new smart-contract code beyond the per-token config.

**Cons**: does not distinguish between a subaccount with $10k cbBTC
+ $10k long BTC-PERP (very correlated) vs $10k cbBTC + $10k long
ETH-PERP (partially correlated) vs $10k cbBTC + $10k short BTC-PERP
(negatively correlated, effectively hedged). All three carry the
same haircut under model A.

### (B) Concentration caps

Per-wallet cap on the notional exposure to any single underlying (BTC
combined across cbBTC collateral + BTC-PERP positions). Any single
underlying above the cap forces reduce-only mode.

**Pros**: hard invariant, easy to enforce at position-open time,
protects the insurance fund from wallet-level whales.

**Cons**: new state (per-wallet exposure counter per underlying),
new order-open gate, potential UX friction (a user opening a valid
trade can be blocked by unrelated collateral already deposited).

### (C) Correlation penalties

Extra haircut when a subaccount's collateral and position have
positive correlation. E.g. cbBTC collateral factor drops from 70% to
55% while a long BTC-PERP position is open.

**Pros**: matches actual risk; efficient (no capital wasted on
uncorrelated combos).

**Cons**: state coupling between margin math and position book;
requires per-subaccount recomputation on every collateral / position
change; new failure modes (a filled BTC-PERP order could
retroactively make a previously-safe subaccount liquidatable if the
haircut math is not applied atomically).

## Recommendation for first activation (WETH, then cbBTC)

**Use model (A) alone for the first WETH closed test.** Reasoning:

- Static haircuts are the smallest possible surface area — one
  number per asset in the existing per-token config.
- The `CollateralConfig` registry already carries
  `collateral_factor_bps` and `liquidation_factor_bps`; no new
  fields required.
- Correlation penalties (model C) are appropriate for the second
  or third collateral, once we have production data on how users
  actually deploy multi-collateral subaccounts.
- The pathologically dangerous case in a static-only model is
  cbBTC + long BTC-PERP. cbBTC's specifically low factor
  recommendation (65–80%, vs WETH's 80–90%) is chosen to absorb
  that case *within* the static number, and cbBTC is not the first
  collateral we activate — WETH is.

**Layer model (B) on before cbBTC activates.** Reasoning:

- Introduce a per-wallet notional cap for BTC underlying:
  `btc_notional_cap_1e8` in `CollateralConfig` extended with
  `per_wallet_exposure_cap_1e8`.
- Enforced at position-open time as a hard revert; no soft
  degradation to reduce-only.
- Cheap to implement: one additional read of the wallet's current
  BTC-PERP notional + one addition of the proposed order's notional
  before the existing margin check.

**Defer model (C) to post-launch.** Reasoning: state coupling adds
too much surface area for the value it delivers on day one.

## Concrete implementation plan

1. First WETH closed test: model A only. `CollateralConfig` shipped
   today already suffices; parameter choice is a risk-committee
   decision at activation.
2. Before cbBTC closed test: extend `CollateralConfig` with
   `per_wallet_exposure_cap_1e8: Option<u128>` + wire the check into
   the position-open path (`PerpEngine.submitOrder` equivalent). Ship
   ONE unit test proving cap enforcement.
3. Post-launch (out of milestone scope): if data shows unhedged
   correlated positions dominating, add model (C) with per-
   underlying correlation table stored in a dedicated config
   contract.

## Explicit rejections

- **No portfolio VaR model.** VaR requires historical price paths, a
  correlation matrix, and a Monte Carlo engine. None of these exist
  today; introducing them would take months of engineering with high
  parameter-uncertainty. Static haircuts + concentration caps are the
  Pareto-optimal launch model.
- **No dynamic haircuts.** A haircut that changes in response to
  observed market conditions (funding rate, open interest, volume)
  is powerful but requires a live keeper that can move risk
  parameters. The trust model for that keeper is a governance
  problem, not a launch problem.
- **No cross-margin between subaccounts.** Even for the same wallet.
  Subaccount isolation is a hard V1 invariant and this design does
  not touch it.

## Verdict

`DEOPT_COLLATERAL_CONCENTRATION_POLICY_DESIGNED`
