# NEXT_TASK.md — Tier Merkle Rebate Mega-Milestone V2G-A

## Context

Options and perps Tier0 V2 positive fees are validated end-to-end.

Validated:
- Option FeeChargedV2 live.
- Perp FeeChargedV2 live.
- Backend/admin decoding works for OPTION and PERP.
- PERP FeeChargedV2/FeeRebatedV2 metrics exist.
- Alerts for OLD stranded PerpEngine exist.
- Rebates have NOT been live-tested.
- Tier/Merkle eligibility has NOT been implemented end-to-end.

Current live:
- FEES_MANAGER_V2 = 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
- NEW_PERP_ENGINE = 0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
- NEW_MARGIN_ENGINE = 0x287Cef479be5889eEfCa847F9e73C860898f48Cc
- OLD_PERP_ENGINE = 0xB36395b67D0798ADA981731c9Fa5239F4362b53B stranded under A3 Base Sepolia.

Goal:
Build the Tier/Merkle/Rebate system as one accelerated module.

This is not a microtask. Complete all safe backend/tests/docs/scripts/dry-runs. Stop only before human broadcasts.

## Hard Rules

Do not broadcast.
Do not submit live transactions.
Do not mutate live chain.
Do not delete DB rows.
Do not print secrets.
Do not edit real `.env`.
Do not weaken existing Tier0 option/perp behavior.
Do not hide OLD_PERP_ENGINE stranded state.
Do not silently change the fee schedule below.

## Canonical Tier Eligibility

Eligibility is OR-based.

A user qualifies for a tier if ANY of the following is true:
- 28D volume threshold is met, OR
- 28D venue volume share threshold is met, OR
- staked DEOPT threshold is met.

The highest qualifying tier wins.

Eligibility must be computed off-chain by backend snapshot/Merkle process.

Do NOT compute 28D volume, 28D share, or staked DEOPT directly inside PerpEngine or MarginEngine.

## Canonical Option Fee Schedule

Use this table as the canonical source of truth.

| Tier | 28D Volume | 28D Vol Share | Staked DEOPT | Option Maker | Option Taker | RFQ Maker Fee Discount | RFQ Taker Fee Discount |
| ---: | ---------: | ------------: | -----------: | -----------: | -----------: | ---------------------: | ---------------------: |
| 4 | >= $25M | >= 5% | >= 250,000 | -0.005% | 0.0075% | 100% | 75% |
| 3 | >= $10M | >= 2.5% | >= 100,000 | -0.0025% | 0.010% | 75% | 50% |
| 2 | >= $2.5M | >= 1% | >= 50,000 | -0.001% | 0.0125% | 50% | 25% |
| 1 | >= $500k | >= 0.25% | >= 10,000 | 0.000% | 0.015% | 25% | 10% |
| 0 | below Tier1 | below Tier1 | below Tier1 | 0.005% | 0.025% | 0% | 0% |

Canonical ppm conversion:

```text
Tier4 OPTION:
- maker = -50 ppm
- taker = 75 ppm
- RFQ maker discount = 10000 bps
- RFQ taker discount = 7500 bps

Tier3 OPTION:
- maker = -25 ppm
- taker = 100 ppm
- RFQ maker discount = 7500 bps
- RFQ taker discount = 5000 bps

Tier2 OPTION:
- maker = -10 ppm
- taker = 125 ppm
- RFQ maker discount = 5000 bps
- RFQ taker discount = 2500 bps

Tier1 OPTION:
- maker = 0 ppm
- taker = 150 ppm
- RFQ maker discount = 2500 bps
- RFQ taker discount = 1000 bps

Tier0 OPTION:
- maker = 50 ppm
- taker = 250 ppm
- RFQ maker discount = 0 bps
- RFQ taker discount = 0 bps
Canonical Perp Fee Schedule

Use this table as the canonical source of truth.

Tier	28D Volume	28D Vol Share	Staked DEOPT	Perp Maker	Perp Taker
4	>= $25M	>= 5%	>= 250,000	-0.010%	0.015%
3	>= $10M	>= 2.5%	>= 100,000	-0.0075%	0.0175%
2	>= $2.5M	>= 1%	>= 50,000	-0.005%	0.020%
1	>= $500k	>= 0.25%	>= 10,000	0.000%	0.025%
0	below Tier1	below Tier1	below Tier1	0.005%	0.030%

Canonical ppm conversion:

Tier4 PERP:
- maker = -100 ppm
- taker = 150 ppm

Tier3 PERP:
- maker = -75 ppm
- taker = 175 ppm

Tier2 PERP:
- maker = -50 ppm
- taker = 200 ppm

Tier1 PERP:
- maker = 0 ppm
- taker = 250 ppm

Tier0 PERP:
- maker = 50 ppm
- taker = 300 ppm
Module Scope
Part 1 — Metrics Hardening

Audit V2F-Q metrics.

Decide whether to:

promote deopt_perp_fee_charged_v2_total and deopt_perp_fee_rebated_v2_total to true monotonic counters, or
keep ledger-derived gauges and document/rename cleanly if true counters would be misleading.

Do not fake true counter semantics.

Part 2 — Tier Schedule Canonicalization

Define canonical fee schedule in backend config/types for OPTION and PERP using the exact tables above.

Required:

exact ppm values.
exact RFQ discount bps values for options.
all tiers 0 through 4.
tests for every tier and product.
tests for boundary thresholds.
Part 3 — OR Eligibility Logic

Implement or scaffold tier eligibility logic:

Inputs:

28D volume in USD/native normalized value.
28D venue volume share.
staked DEOPT.
product kind.

Rules:

OR logic across volume/share/stake.
highest qualifying tier wins.
Tier0 fallback when no threshold is met.

Required boundary tests:

exactly $500k qualifies Tier1.
exactly $2.5M qualifies Tier2.
exactly $10M qualifies Tier3.
exactly $25M qualifies Tier4.
exactly 0.25% qualifies Tier1.
exactly 1% qualifies Tier2.
exactly 2.5% qualifies Tier3.
exactly 5% qualifies Tier4.
exactly 10,000 DEOPT qualifies Tier1.
exactly 50,000 DEOPT qualifies Tier2.
exactly 100,000 DEOPT qualifies Tier3.
exactly 250,000 DEOPT qualifies Tier4.
highest tier wins if multiple thresholds match.
Part 4 — Tier Snapshot Backend

Implement or scaffold backend tier snapshot generation.

Snapshot row must include:

trader address.
option 28D volume.
perp 28D volume.
total 28D venue volume.
volume share.
staked DEOPT.
computed option tier.
computed perp tier.
option maker ppm.
option taker ppm.
option RFQ maker discount bps.
option RFQ taker discount bps.
perp maker ppm.
perp taker ppm.
validFrom.
validUntil.

If real data tables are missing:

create clean interfaces and deterministic test fixtures.
do not invent production data.
Part 5 — Merkle Tree Generation

Implement CLI/tool or backend command to generate:

Merkle leaves.
Merkle root.
proof per trader/product/tier.
JSON artifact.

Required:

deterministic sorting.
stable schema.
tests for root/proof reproducibility.
invalid proof tests.
expired proof tests if supported by contract.
Part 6 — Solidity Compatibility Check

Inspect FeesManagerV2:

claimTier format.
Merkle leaf format.
root setter.
rebate budget functions.
FeeRebatedV2 behavior.
signed ppm behavior.
RFQ discount handling if present.

If backend leaf format does not match Solidity:

fix backend.
add cross-test vectors.

Do not deploy.

Part 7 — Rebate Budget + Live Smoke Plan

Prepare scripts/dry-runs for:

setMerkleRoot on FeesManagerV2.
fundRebateBudget for mUSDC.
claim Tier2/Tier3/Tier4 for test accounts.
option rebate smoke.
perp rebate smoke.
option RFQ discount smoke if supported.
perp RFQ discount smoke if supported.

No broadcast.

Expected live rebate smoke target:

force maker negative ppm tier.
small notional.
FeeChargedV2 for taker.
FeeRebatedV2 for maker.
rebateBudget decreases.
feeRecipient increases by taker fee.
maker vault balance increases by rebate.
Part 8 — Tests

Add tests for:

OR eligibility.
all tiers.
boundary values.
volume/share/stake precedence.
option fee schedule exact ppm.
option RFQ discount exact bps.
perp fee schedule exact ppm.
Merkle root/proof generation.
claim payload compatibility.
negative maker rebate.
insufficient rebate budget.
expired/invalid claims.
no OPTION/PERP Tier0 regression.
Part 9 — Docs

Create:

docs/TIER_MERKLE_REBATE_SYSTEM_V2G_A.md
docs/TIER_SNAPSHOT_SCHEMA_V2G_A.md
docs/REBATE_LIVE_SMOKE_PLAN_V2G_A.md

Update:

docs/ALERTING_SPEC.md
V2F-Q docs if metrics semantics changed.
Validation

Backend:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --no-fail-fast
cargo build --all-targets --all-features

Solidity if touched:

forge fmt
forge fmt --check
forge build
forge test

Frontend if touched:

npm run lint
npx tsc --noEmit
npm run build
Acceptance Criteria

Complete if:

exact canonical option table is implemented or documented with blockers.
exact canonical perp table is implemented or documented with blockers.
OR eligibility logic exists.
tier snapshot logic exists or cleanly scaffolded.
Merkle artifact generation exists.
Solidity leaf compatibility is proven or blocker documented.
rebate live smoke scripts/plans are prepared.
tests pass.
docs created.
no live mutation happened.
Final Report

Return:

metrics hardening decision.
option fee schedule implementation.
perp fee schedule implementation.
RFQ discount handling.
OR eligibility implementation.
snapshot implementation.
Merkle implementation.
Solidity compatibility result.
rebate smoke preparation.
files changed.
tests added.
docs updated.
validation commands run.
exact blockers.
next human broadcast gates if any.