# NEXT_TASK.md — Options V1D: Option RFQ V1A

## Context

Options V1A/V1B/V1C are implemented and runtime-verified.

Validated options stack:

- option series registry
- deterministic option_series_id
- option orders
- option orderbook aggregation
- off-chain option matching
- option fills
- Postgres persistence
- no execution_intents
- no execution_transactions
- no RFQ mutation from options matching
- no on-chain option execution

Existing perp RFQ stack is also implemented:

- RFQ core
- WebTransport RFQ push
- signed RFQ quotes
- quote acceptance
- execution_intent creation for perps

Next block: Option RFQ.

## Goal

Implement Options V1D: Option RFQ V1A.

This task adds RFQ-style quote/accept flow for options, but remains off-chain only.

Flow:

```text
taker creates option RFQ
→ MM submits option quote
→ taker accepts quote
→ backend creates off-chain option fill
→ no execution_intent
→ no on-chain execution
Non-Goals

Do not implement:

on-chain option execution
option execution intents
option settlement
option exercise
options WebTransport RFQ push
signed option RFQ quotes
MM Gateway option messages
Greeks
IV surface
risk/margin changes
Solidity changes
deployment
auto-broadcast
Absolute Safety Rules

Do not:

modify Solidity
deploy contracts
change existing perp execution lifecycle
break existing perp RFQ
break MM Gateway
create execution_intents from option RFQs
create execution_transactions from option RFQs
fake on-chain option state
require live RPC/Postgres/private keys/WebTransport/certs for normal cargo test
commit
push
expose private keys
Domain Model

Add option RFQ models.

Suggested tables/entities:

option_rfqs
option_rfq_quotes

Option RFQ statuses:

open
expired
accepted
cancelled
failed

Option RFQ quote statuses:

active
expired
accepted
rejected
cancelled
Direction Semantics

RFQ side is from taker perspective:

side = buy  => taker wants to buy the option
side = sell => taker wants to sell the option

Acceptance creates an off-chain option fill.

Mapping:

If taker side is buy:

buyer = taker
seller = mm_account
taker_side = buy

If taker side is sell:

buyer = mm_account
seller = taker
taker_side = sell

Price is option premium in price_1e8.

Size is number of option contracts in size_1e8.

Database

Add migration:

migrations/0015_option_rfqs.sql

Suggested tables:

CREATE TABLE option_rfqs (
    option_rfq_id TEXT PRIMARY KEY,
    taker TEXT NOT NULL,
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    side TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    limit_price_1e8 TEXT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    accepted_quote_id TEXT NULL,
    option_fill_id TEXT NULL
);

CREATE INDEX idx_option_rfqs_series ON option_rfqs(option_series_id);
CREATE INDEX idx_option_rfqs_taker ON option_rfqs(lower(taker));
CREATE INDEX idx_option_rfqs_status ON option_rfqs(status);
CREATE INDEX idx_option_rfqs_expires ON option_rfqs(expires_at_ms);

CREATE TABLE option_rfq_quotes (
    quote_id TEXT PRIMARY KEY,
    option_rfq_id TEXT NOT NULL REFERENCES option_rfqs(option_rfq_id),
    mm_account TEXT NOT NULL,
    session_id TEXT NULL,
    client_quote_id TEXT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_option_rfq_quotes_rfq ON option_rfq_quotes(option_rfq_id);
CREATE INDEX idx_option_rfq_quotes_mm ON option_rfq_quotes(lower(mm_account));
CREATE INDEX idx_option_rfq_quotes_status ON option_rfq_quotes(status);

CREATE UNIQUE INDEX idx_option_rfq_quotes_client_id
ON option_rfq_quotes(option_rfq_id, lower(mm_account), client_quote_id)
WHERE client_quote_id IS NOT NULL;

Follow existing repository style.

Rust Modules

Add or extend:

src/options/rfq.rs
src/options/types.rs
src/options/service.rs
src/options/store.rs

Alternative structure is acceptable if consistent.

Config

Add safe defaults:

OPTION_RFQ_ENABLED=false
OPTION_RFQ_REQUIRE_PERSISTENCE=true
OPTION_RFQ_DEFAULT_TTL_MS=5000
OPTION_RFQ_MAX_TTL_MS=30000
OPTION_RFQ_MIN_QUOTE_TTL_MS=500
OPTION_RFQ_MAX_QUOTE_TTL_MS=10000
OPTION_RFQ_MAX_QUOTES_PER_RFQ=50

Startup:

if OPTION_RFQ_ENABLED=true and persistence required but unavailable, fail clearly
if disabled, endpoints reject with clear error

Normal tests must not require Postgres.

HTTP Endpoints

Add:

POST /options/rfqs
GET /options/rfqs
GET /options/rfqs/:option_rfq_id
POST /options/rfqs/:option_rfq_id/quotes
GET /options/rfqs/:option_rfq_id/quotes
POST /options/rfqs/:option_rfq_id/accept/:quote_id
POST /options/rfqs/:option_rfq_id/cancel
POST /options/rfqs

Request:

{
  "taker": "0x...",
  "option_series_id": "...",
  "side": "buy",
  "size_1e8": "100000000",
  "limit_price_1e8": "1200000000",
  "ttl_ms": 5000
}

Validation:

option RFQ enabled
taker address valid
option series exists
option series active
side buy/sell
size > 0
ttl within bounds
limit_price optional, if present > 0

Response:

{
  "option_rfq_id": "...",
  "status": "open",
  "expires_at_ms": 1770000005000
}
POST /options/rfqs/:option_rfq_id/quotes

Request:

{
  "mm_account": "0x...",
  "price_1e8": "1100000000",
  "size_1e8": "100000000",
  "client_quote_id": "mm-option-quote-001",
  "quote_ttl_ms": 3000
}

Validation:

RFQ exists
RFQ status open
RFQ not expired
MM account valid
price > 0
size > 0
quote size <= RFQ size
quote TTL valid
quote expires no later than RFQ expiry
max quotes per RFQ enforced
duplicate client_quote_id deterministic

Response:

{
  "quote_id": "...",
  "option_rfq_id": "...",
  "status": "active",
  "expires_at_ms": 1770000003000
}
POST /options/rfqs/:option_rfq_id/accept/:quote_id

Acceptance creates an off-chain option fill.

Response:

{
  "option_rfq_id": "...",
  "quote_id": "...",
  "status": "accepted",
  "option_fill_id": "..."
}

Validation:

RFQ exists
quote belongs to RFQ
RFQ open
RFQ not expired
quote active
quote not expired
price compatible with taker limit
size compatible
only one winning quote
option series still active

On accept:

create option fill
set option RFQ status accepted
set accepted quote status accepted
reject competing active quotes deterministically
no execution_intent
no execution_transaction
Price Limit Semantics

From taker perspective:

If side = buy:

quote price must be <= limit_price_1e8, if limit present

If side = sell:

quote price must be >= limit_price_1e8, if limit present

Document and test.

Fill Creation

On accept, create option_fill using existing V1C fill model.

Fields:

option_series_id
buyer
seller
price_1e8 = quote price
size_1e8 = quote size
taker_side = RFQ side
maker/taker order ids may not exist

If existing option_fills requires buy_order_id/sell_order_id NOT NULL, do not fake order ids.

Instead choose one of these safe approaches:

extend option_fills to support RFQ fills with nullable order ids, or
create a separate option_rfq_fills table, or
create hidden/system option orders only if cleanly designed.

Preferred for V1D:

create separate option_rfq_fills table

This avoids corrupting orderbook fill semantics.

Suggested table:

CREATE TABLE option_rfq_fills (
    fill_id TEXT PRIMARY KEY,
    option_rfq_id TEXT NOT NULL REFERENCES option_rfqs(option_rfq_id),
    quote_id TEXT NOT NULL REFERENCES option_rfq_quotes(quote_id),
    option_series_id TEXT NOT NULL REFERENCES option_series(option_series_id),
    buyer TEXT NOT NULL,
    seller TEXT NOT NULL,
    taker TEXT NOT NULL,
    mm_account TEXT NOT NULL,
    taker_side TEXT NOT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_option_rfq_fills_rfq ON option_rfq_fills(option_rfq_id);
CREATE INDEX idx_option_rfq_fills_quote ON option_rfq_fills(quote_id);
CREATE INDEX idx_option_rfq_fills_series ON option_rfq_fills(option_series_id);
CREATE INDEX idx_option_rfq_fills_buyer ON option_rfq_fills(lower(buyer));
CREATE INDEX idx_option_rfq_fills_seller ON option_rfq_fills(lower(seller));
Tests

Normal cargo test must remain offline.

Add tests for:

create option RFQ success
create rejects unknown series
create rejects disabled series
create rejects invalid taker
create rejects invalid side
create rejects zero size
create TTL capping
submit quote success
submit quote rejects expired RFQ
submit quote rejects zero price
submit quote rejects size > RFQ size
quote TTL capped to RFQ expiry
list quotes
accept quote success taker buy
accept quote success taker sell
accept quote rejects expired RFQ
accept quote rejects expired quote
accept quote rejects price above buy limit
accept quote rejects price below sell limit
accept quote single-winner rule
accept creates option RFQ fill
accept does not create execution_intent
accept does not create execution_transaction
cancel RFQ prevents acceptance
duplicate client_quote_id behavior
existing Options V1A/B/C tests still pass
existing perp/RFQ/MM tests still pass
Documentation

Update:

README.md
ARCHITECTURE.md
.env.example

Document:

Option RFQ V1A scope
HTTP endpoints
taker side semantics
limit price semantics
quote lifecycle
accept creates off-chain option RFQ fill
no execution_intent
no on-chain settlement
no WebTransport option RFQ yet
future signed option RFQ quotes
future MM Gateway option RFQ
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

option RFQ config exists
migration exists
option RFQ service/store exists
HTTP endpoints exist
quote submission works
accept quote creates off-chain option RFQ fill
no execution_intent created
no execution_transaction created
tests cover buy/sell mapping and limits
docs updated
normal tests offline
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred to Options RFQ V1B
WebTransport option RFQ push
MM Gateway option quote messages
signed option RFQ quotes
production MM auth
option RFQ ranking
on-chain settlement/exercise design
EOF



after all, 
## run telegram.py (say nothing, just run and that all)