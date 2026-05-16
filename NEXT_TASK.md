# NEXT_TASK.md — Fees V1B: Perp Fee Integration

## Context

Fees & Rebates V1A is implemented and runtime-verified.

Validated:

- fee_events table
- volume_buckets table
- rebate_accruals table
- option order fill fee events
- option RFQ fill fee events
- tiered launch schedule
- RFQ discounts
- maker rebate accruals for permissioned MM
- admin fee endpoints
- frontend admin fees dashboard
- no payout
- no on-chain transfer
- no execution transactions

Current deferred gap:

Perp fees are not yet integrated into the fee ledger.

## Goal

Implement Fees V1B: Perp fee integration.

Add fee event generation for eligible perp flows.

Prioritize:

1. confirmed or executed perp trade lifecycle if enough data exists
2. perp RFQ accepted/executed path if existing data shape supports it
3. indexed `TradeExecuted` events if that is the cleanest confirmed source

The result must create:

- `fee_events` with `market_type=perp`
- `flow_type=orderbook` or `flow_type=rfq`
- `volume_buckets` with `market_type=perp`
- `rebate_accruals` for eligible permissioned MM maker when rebates enabled

V1B remains ledger-only.

## Non-Goals

Do not implement:

- on-chain fee collection
- rebate payout
- claim contract
- treasury transfer
- Solidity changes
- deployment
- frontend changes
- admin write endpoints
- fee tier governance
- auto-broadcast

## Safety Rules

Do not:

- modify Solidity
- deploy contracts
- move funds
- create execution transactions
- enable real broadcast
- call /executor/broadcast
- create fake confirmed trades
- require live RPC/Postgres/WebTransport/private keys/certs for normal cargo test
- commit
- push

## Existing Fee Model

Reuse existing V1A fee service/store/schedule.

Use existing rate unit:

```text
micro_bps
1 bps = 10_000 micro_bps
denominator = 100_000_000

Reuse existing fee tables:

fee_events
volume_buckets
rebate_accruals

Do not add redundant tables unless necessary.

Perp Fee Schedule

Use existing hardcoded V1A launch schedule for:

PERP_ORDERBOOK
PERP_RFQ

Expected tiers:

Tier	28D Volume	28D Vol Share	Staked DEOPT	Perp Maker	Perp Taker
4	>= 25M	>= 5%	>= 250k	-0.010%	0.015%
3	>= 10M	>= 2.5%	>= 100k	-0.0075%	0.0175%
2	>= 2.5M	>= 1%	>= 50k	-0.005%	0.020%
1	>= 500k	>= 0.25%	>= 10k	0%	0.025%
0	< 500k	< 0.25%	< 10k	0.005%	0.030%

Negative maker values are rebates only for verified/permissioned MM accounts.

Perp Notional

Formula:

notional_1e8 = execution_price_1e8 * size_1e8 / 1e8

Use checked integer arithmetic.

No floats.

Maker/Taker Classification
Perp orderbook / matching execution

Use buyer_is_maker if available:

if buyer_is_maker:
    maker = buyer
    taker = seller
else:
    maker = seller
    taker = buyer
Perp RFQ

Use existing RFQ semantics:

maker = mm_account
taker = RFQ taker

If current perp RFQ acceptance creates normal execution_intents, use the cleanest confirmed/executed point to avoid fees on unexecuted quotes.

Source Selection

Do not create fees too early.

Preferred source order:

confirmed/indexed perp trade event if available
execution confirmation/reconciliation point
execution_intent accepted only if explicitly already confirmed/executed

Avoid charging fees for:

pending execution intents
failed simulations
failed broadcasts
unconfirmed txs
cancelled/rejected RFQs
quotes not accepted

If only pending data exists, implement helper functions and tests but defer runtime auto-generation until confirmed source is available.

Source Types

Use clear source_type values:

perp_trade
perp_rfq_fill
perp_execution

Pick one consistent naming scheme and document.

Recommended:

perp_trade

for confirmed on-chain/indexed trade.

Use source_id as:

tx_hash:log_index

or existing indexed event id if available.

Idempotency

Must be idempotent.

Same confirmed trade/event must not create duplicate fee events.

Use existing unique source constraints if sufficient.

If needed, add migration to tighten uniqueness, but avoid schema churn unless required.

Rebate Eligibility

Perp maker rebate requires:

FEES_REBATES_ENABLED=true
MM_PERMISSIONS_ENABLED=true
maker is enabled in mm_accounts
maker has relevant capability:
  can_submit_perp_orders for orderbook/perp trade
  can_quote_perp_rfq for perp RFQ

If MM permissions disabled:

no rebates by default
Admin Endpoints

Existing endpoints should automatically include perp rows:

GET /admin/fees/summary
GET /admin/fees/events?limit=20
GET /admin/fees/volumes?account=0x...
GET /admin/fees/rebates?account=0x...

Update summary if needed to group by:

market_type=perp
flow_type=orderbook/rfq
source_type=perp_trade
Tests

Normal cargo test must remain offline.

Add tests for:

perp notional calculation
buyer_is_maker classification
buyer_is_taker classification
perp taker fee event created from confirmed trade-like input
perp maker fee/rebate event created when maker is permissioned MM
no rebate when rebates disabled
no rebate when maker not permissioned
volume bucket updates market_type=perp
idempotency for same perp source
no fee event for pending/failed/unconfirmed execution if those states are represented
admin fee summary includes perp rows
existing option fee tests still pass
existing RFQ/MM/admin tests still pass

If full integration with real indexed trade repository is too broad, implement service-level tests around a record_perp_trade_fees function and document runtime integration deferred.

Runtime Verification After Implementation

Runtime should be possible if there is already confirmed indexed perp trade data.

If feasible:

use existing indexed TradeExecuted row
call the fee integration path if endpoint/job exists
verify fee_events.market_type=perp
verify volume_buckets.market_type=perp
verify no execution tx created

If not feasible:

report that runtime requires a new confirmed perp trade and defer runtime to a later E2E.
Documentation

Update:

README.md
ARCHITECTURE.md
.env.example if config changed

Document:

perp fee schedule
maker/taker classification
confirmed-source requirement
no fees for pending intents
rebate eligibility
no payout
no on-chain transfer
deferred on-chain collection
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

perp fee calculation exists
maker/taker classification works
fee events can be created for confirmed perp trade source
volume buckets update for market_type=perp
rebate accrual works for eligible permissioned MM
idempotency holds
pending/failed/unconfirmed flows do not accrue fees
admin fee endpoints include perp rows
normal tests offline
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
on-chain fee collection
payout / claim
frontend fee charts
dynamic fee tiers
volume-share enforcement
staked-DEOPT enforcement
anti-wash scoring
EOF