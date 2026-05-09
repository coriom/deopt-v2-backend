# NEXT_TASK.md — Options V1C: Off-chain Option Matching

## Context

Options V1A and V1B are implemented and runtime-verified.

V1A:

- option series registry
- deterministic option_series_id
- Postgres persistence
- list/filter/get/disable endpoints
- empty orderbook endpoint

V1B:

- off-chain option orders
- POST /options/orders
- GET /options/orders
- GET /options/orders/:order_id
- POST /options/orders/:order_id/cancel
- real aggregated option orderbook levels
- Postgres persistence
- no execution_intents
- no execution_transactions
- no RFQ mutation

Runtime verification passed:

- order submission persisted
- duplicate client_order_id rejected
- orderbook aggregated correctly
- cancel removed order from book
- no forbidden mutation occurred

## Goal

Implement Options V1C: off-chain option matching and fill recording.

The backend must support:

- matching buy/sell option orders within same option_series_id
- recording option fills
- updating remaining sizes
- updating order statuses
- persisting fills
- returning matched fills in order submission response
- keeping everything off-chain only

## Non-Goals

Do not implement:

- option execution intents
- on-chain option execution
- option exercise
- option settlement
- option RFQ
- option MM Gateway messages
- Greeks
- IV surface
- risk/margin changes
- Solidity changes
- deployment
- auto-broadcast

## Absolute Safety Rules

Do not:

- modify Solidity
- deploy contracts
- change existing perp execution lifecycle
- break RFQ/MM Gateway
- create execution_intents from option matches
- create execution_transactions from option matches
- fake on-chain option state
- require live RPC/Postgres/private keys/WebTransport/certs for normal cargo test
- commit
- push
- expose private keys

## Matching Semantics

Options V1C matching is off-chain only.

Rules:

- match only orders with the same `option_series_id`
- buy matches sell if `buy.price_1e8 >= sell.price_1e8`
- match size = min(buy.remaining_size, sell.remaining_size)
- fill price rule must be deterministic

Recommended fill price:

```text
resting maker order price

If an incoming buy crosses resting asks, fill at resting ask price.
If an incoming sell crosses resting bids, fill at resting bid price.

No floating point arithmetic.

Use integer/string-safe comparison for 1e8 values.

Time Priority

Within same price level:

older order fills first

Use created_at_ms, then order_id as deterministic tie-breaker if needed.

Order Statuses

Existing statuses:

open
cancelled
filled
rejected
expired

Add if needed:

partially_filled

Recommended status logic:

remaining_size == size => open
0 < remaining_size < size => partially_filled
remaining_size == 0 => filled
cancelled remains cancelled

If adding partially_filled, update docs/tests.

Fill Model

Add option fill model.

Suggested fields:

fill_id: string
option_series_id: string
buy_order_id: string
sell_order_id: string
buyer: string
seller: string
price_1e8: string
size_1e8: string
created_at_ms: u64

Optional:

maker_order_id
taker_order_id
taker_side

Recommended fields:

maker_order_id
taker_order_id
taker_side
Database

Add migration:

migrations/0014_option_fills.sql

Suggested table:

CREATE TABLE option_fills (
    fill_id TEXT PRIMARY KEY,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    buy_order_id TEXT NOT NULL REFERENCES option_orders(order_id),
    sell_order_id TEXT NOT NULL REFERENCES option_orders(order_id),
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    maker_order_id TEXT NOT NULL,
    taker_order_id TEXT NOT NULL,
    taker_side TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_option_fills_series ON option_fills(option_series_id);
CREATE INDEX idx_option_fills_buy_order ON option_fills(buy_order_id);
CREATE INDEX idx_option_fills_sell_order ON option_fills(sell_order_id);
CREATE INDEX idx_option_fills_buyer ON option_fills(lower(buyer));
CREATE INDEX idx_option_fills_seller ON option_fills(lower(seller));
CREATE INDEX idx_option_fills_created_at ON option_fills(created_at_ms);

Follow existing repository style if different.

HTTP Endpoints

Extend:

POST /options/orders
GET /options/orders/:order_id
GET /options/orders
GET /options/orderbooks/:option_series_id

Add:

GET /options/fills
GET /options/fills/:fill_id
GET /options/orders/:order_id/fills

Filters for GET /options/fills:

option_series_id
account
order_id
POST /options/orders Response

Currently returns order.

Extend response with fills:

{
  "order_id": "...",
  "option_series_id": "...",
  "status": "partially_filled",
  "remaining_size_1e8": "50000000",
  "fills": [
    {
      "fill_id": "...",
      "price_1e8": "1000000000",
      "size_1e8": "50000000",
      "buy_order_id": "...",
      "sell_order_id": "..."
    }
  ]
}

If no match:

{
  "order_id": "...",
  "status": "open",
  "fills": []
}
Matching Behavior

When an incoming order is submitted:

validate option order as V1B
find opposite-side open/partially_filled orders for same series
sort by price/time priority:
incoming buy matches lowest ask first
incoming sell matches highest bid first
create fills
update resting orders remaining/status
update incoming order remaining/status
persist all changes atomically if persistence enabled
return order + fills

If persistence transaction support exists, use it.

If not, implement minimal safe sequence and document limitation.

Orderbook After Matching

GET /options/orderbooks/:option_series_id must reflect only open/partially_filled remaining sizes.

Filled orders must not appear.

Partially filled orders appear with remaining size only.

Cancellation Rules

Existing cancel endpoint must handle:

open order => cancelled
partially_filled order => cancelled with remaining size cancelled
filled order => reject or no-op clearly
cancelled order => deterministic already-cancelled response

Do not delete fills.

Persistence Behavior

When persistence enabled:

option order insert/update/fill write should persist
fill listing reads DB
orderbook reads open/partially_filled orders

When persistence disabled:

in-memory store works
normal tests do not need Postgres
Tests

Normal cargo test must be offline.

Add tests for:

buy order rests when no ask
sell order rests when no bid
buy crosses ask and creates fill
sell crosses bid and creates fill
no cross when buy price < ask price
partial fill leaves incoming partially_filled/open remainder
partial fill updates resting order remaining
full fill sets status filled
multiple fills across price levels
price priority for asks: lowest ask first
price priority for bids: highest bid first
time priority within same price
fill price equals resting maker price
orderbook removes filled orders
orderbook shows partially filled remaining size
cancel partially filled order cancels remaining
cannot cancel filled order or returns clear already-filled error
list fills by series
list fills by order_id
list fills by account
get fill by id
no execution_intent created from option match
no execution_transaction created from option match
existing option V1A/V1B tests still pass
existing perp/RFQ/MM tests still pass
Documentation

Update:

README.md
ARCHITECTURE.md
.env.example if config changed

Document:

Options V1C scope
off-chain matching only
price-time priority
fill price rule
option_fills
no execution_intents
no on-chain settlement/exercise
future Options V1D:
option execution intent / settlement design
options RFQ
MM Gateway option messages
Greeks/IV
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

option matching works off-chain
option fills are recorded
order statuses and remaining sizes update correctly
orderbook reflects remaining liquidity
fill endpoints exist
no option execution_intents are created
no execution_transactions are created
normal tests offline
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
option execution intent
on-chain option exercise/settlement
options RFQ
option MM Gateway messages
on-chain OptionProductRegistry sync
Greeks
IV surface
risk cache
EOF



after all, 
## run telegram.py (say nothing, just run and that all)