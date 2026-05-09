# NEXT_TASK.md — Options V1A: Option Series Registry + Orderbooks

## Context

The backend now supports a validated perp execution stack:

- HTTP order intake
- orderbook matching
- execution intents
- PerpTrade signing
- simulation
- real broadcast
- indexer
- reconciliation
- confirmation/finality
- on-chain nonce sync

It also supports:

- WebTransport Market Maker Gateway
- live MM submit/cancel/quote_replace
- RFQ V1A/V1B/V1C
- signed RFQ quotes
- RFQ accept creates normal execution_intent

The next major block is Options.

Current backend order/matching logic is primarily perp-market oriented.

Options need a series-aware orderbook model.

## Goal

Implement Options V1A:

- option series model
- option series registry/cache
- option orderbook keys
- HTTP endpoints for option series and option orderbooks
- order submission for option series in backend only
- no on-chain option execution yet
- no options RFQ yet
- no Greeks/risk cache yet

This is a backend product/data layer foundation for options.

## Non-Goals

Do not implement:

- on-chain option exercise
- on-chain option settlement
- option execution broadcast
- option PerpTrade equivalent
- options RFQ
- options WebTransport RFQ
- Greeks
- implied volatility surface
- pricing engine
- margin/risk computation changes
- Solidity changes
- deployment
- auto-broadcast
- options market maker strategy

## Absolute Safety Rules

Do not:

- modify Solidity
- deploy contracts
- change existing perp execution lifecycle
- break existing perp orderbook
- break RFQ
- break MM gateway
- fake on-chain option state
- require live RPC/Postgres/private keys/WebTransport/certs for normal cargo test
- commit
- push
- expose private keys

## Option Series Model

Add an option series domain model.

Suggested fields:

```text
option_series_id: string
underlying: string / address
base_asset: string / address
quote_asset: string / address
settlement_asset: string / address
expiry: u64
strike_1e8: string
is_call: bool
contract_size_1e8: string
status: active/expired/disabled
source: manual/onchain
created_at_ms
updated_at_ms

If the Solidity registry has a specific series/product id, include:

onchain_product_id
onchain_series_id

Use naming aligned with existing Solidity if known.

Important numeric rules:

strike is 1e8
contract size is 1e8
avoid floats
use string/integers consistently with current backend style
Series Identifier

Define deterministic backend option_series_id.

Recommended:

keccak256(
  underlying/base/quote/settlement/expiry/strike_1e8/is_call/contract_size_1e8
)

or UUID if easier.

If using hash:

expose hex string
deterministic across restarts
tested
Database

Add migration:

migrations/0012_option_series.sql

Suggested table:

CREATE TABLE option_series (
    option_series_id TEXT PRIMARY KEY,
    underlying TEXT NOT NULL,
    base_asset TEXT NOT NULL,
    quote_asset TEXT NOT NULL,
    settlement_asset TEXT NOT NULL,
    expiry BIGINT NOT NULL,
    strike_1e8 TEXT NOT NULL,
    is_call BOOLEAN NOT NULL,
    contract_size_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    onchain_product_id TEXT NULL,
    onchain_series_id TEXT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_option_series_status ON option_series(status);
CREATE INDEX idx_option_series_expiry ON option_series(expiry);
CREATE INDEX idx_option_series_underlying ON option_series(lower(underlying));
CREATE INDEX idx_option_series_strike ON option_series(strike_1e8);
CREATE INDEX idx_option_series_callput ON option_series(is_call);

If existing repository style prefers different naming, follow existing conventions.

Rust Modules

Add:

src/options/
  mod.rs
  types.rs
  service.rs
  store.rs

Optional:

src/options/series_id.rs
Config

Add safe defaults:

OPTIONS_ENABLED=false
OPTIONS_REQUIRE_PERSISTENCE=true
OPTIONS_ALLOW_MANUAL_SERIES=true
OPTIONS_SYNC_ONCHAIN_REGISTRY=false
OPTIONS_DEFAULT_CONTRACT_SIZE_1E8=100000000

Behavior:

if disabled, option endpoints return clear error
if enabled and persistence required but unavailable, startup fails clearly
normal cargo test must not need Postgres
HTTP Endpoints

Add:

POST /options/series
GET /options/series
GET /options/series/:option_series_id
POST /options/series/:option_series_id/disable
GET /options/orderbooks/:option_series_id
POST /options/series

Manual dev/admin series creation.

Request:

{
  "underlying": "ETH",
  "base_asset": "ETH",
  "quote_asset": "USDC",
  "settlement_asset": "USDC",
  "expiry": 1770000000,
  "strike_1e8": "300000000000",
  "is_call": true,
  "contract_size_1e8": "100000000",
  "onchain_product_id": null,
  "onchain_series_id": null
}

Response:

{
  "option_series_id": "...",
  "status": "active"
}

Validation:

expiry must be future unless test mode allows otherwise
strike > 0
contract_size > 0
is_call bool
assets non-empty
duplicate deterministic series should return existing or reject deterministically; choose and document
GET /options/series

Support filters:

underlying
expiry
is_call
status

Keep simple.

GET /options/orderbooks/:option_series_id

Return option orderbook for that series.

For V1A, this can reuse existing orderbook representation if possible.

Option Orderbook Keying

Current engine/orderbook likely keys by market_id.

Options need a separate key type.

Add minimal support for:

InstrumentId:
  PerpMarket(u64)
  OptionSeries(String)

or equivalent.

Do not break existing perp market orderbooks.

If refactor is too broad, implement an option-specific orderbook store separately:

option_orderbooks: HashMap<option_series_id, OrderBook>

Prefer minimal safe change.

Option Orders V1A

Add HTTP endpoint if feasible:

POST /options/orders
POST /options/orders/:order_id/cancel

If too broad, defer actual option order submission and only implement series/orderbook read model.

Preferred V1A includes option order submission as off-chain only.

Request:

{
  "option_series_id": "...",
  "account": "0x...",
  "side": "buy",
  "price_1e8": "1000000000",
  "size_1e8": "100000000",
  "time_in_force": "gtc",
  "client_order_id": "eth-call-3000-bid-1",
  "nonce": 1,
  "deadline_ms": 4102444800000,
  "signature": "0x..."
}

Behavior:

validates series active
validates price > 0
validates size > 0
validates account address
stores order in option orderbook
no on-chain execution intent yet
if matching occurs, either:
create an option execution intent placeholder disabled by default, or
do not match options yet
choose the simpler safe behavior and document

Recommendation V1A:

resting option orderbook only, no matching/execution intent yet

But if existing orderbook matching is easy to reuse safely, matching can be included without on-chain execution.

Market Maker Gateway Impact

Do not integrate options into MM Gateway in V1A unless trivial.

Document future:

MM Gateway options support will use option_series_id instead of market_id.
Tests

Normal cargo test must remain offline.

Add tests for:

option series creation success
rejects zero strike
rejects zero contract size
rejects expired expiry if enforced
deterministic option_series_id
duplicate series behavior
list series by status
list series by underlying
get series by id
disable series
get orderbook for empty active series
get orderbook rejects unknown series
option order submission if implemented
option order rejects inactive series if order submission implemented
existing perp orderbook tests still pass
Documentation

Update:

README.md
ARCHITECTURE.md
.env.example

Document:

Options V1A scope
option series fields
option_series_id
call/put mapping
strike/contract size 1e8 conventions
no on-chain option execution yet
no options RFQ yet
future MM Gateway options support
future options RFQ
future Greeks/risk cache
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

options config exists
option series model exists
migration exists
service/store exists
HTTP series endpoints exist
empty option orderbook endpoint exists
no existing perp/RFQ/MM behavior broken
normal tests offline
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred to Options V1B
option order submission if not completed
option matching
option execution intent
option RFQ
option MM Gateway messages
on-chain OptionProductRegistry sync
Greeks
IV surface
risk cache
expiry settlement integration
EOF






after all, 
## run telegram.py (say nothing, just run and that all)