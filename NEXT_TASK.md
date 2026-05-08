# NEXT_TASK.md — RFQ V1A: Core RFQ Service + HTTP API

## Context

The backend now has a validated execution pipeline:

- HTTP order intake
- orderbook matching
- execution intents
- PerpTrade signing payloads
- buyer/seller signatures
- RPC simulation
- guarded real broadcast
- indexer
- reconciliation
- confirmation/finality
- on-chain perp nonce sync

The Market Maker Gateway V1 is also implemented and runtime-verified:

- WebTransport server
- reliable bidirectional streams
- heartbeat/get_session
- submit_order
- bulk_submit
- cancel_order
- bulk_cancel
- cancel_all
- quote_replace
- cancel-on-disconnect
- HTTP orderbook verification

Next product block: RFQ.

Strategic RFQ design:

```text
taker creates RFQ
→ MM quotes respond
→ taker accepts quote
→ backend creates execution_intent
→ existing signing/simulation/broadcast/index/reconcile/confirm lifecycle handles execution

RFQ must not bypass the execution lifecycle.

Goal

Implement RFQ V1A core service and HTTP API.

This task must build the RFQ domain layer, persistence, quote collection, quote acceptance, and execution-intent creation through existing backend primitives.

Do not implement WebTransport RFQ push yet. That is V1B.

Non-Goals

Do not implement:

WebTransport RFQ broadcasting
MM session RFQ push
RFQ over WebTransport
RFQ quote messages in MM gateway
options RFQ
multi-leg RFQ
auction engine
MM ranking
market data datagrams
automatic broadcast
automatic signing
production RFQ quote signatures
Solidity changes
Absolute Safety Rules

Do not:

modify Solidity
deploy contracts
change PerpTrade ABI
change matching semantics
enable real broadcast by default
auto-broadcast accepted RFQs
fake quotes
fake execution intents
fake tx hashes
fake confirmations
bypass existing validation
bypass nonce sync when execution_intent is created
require live RPC/Postgres/private keys/WebTransport/certs/UDP for normal cargo test
commit
push
expose private keys
RFQ V1A Model

RFQ V1A is HTTP/core only.

It supports:

taker creates RFQ
MM quote is submitted via HTTP/dev API
taker lists quotes
taker accepts one quote
backend creates an execution_intent
RFQ and quote statuses update

The actual final PerpTrade signing/simulation/broadcast remains existing flow.

RFQ Lifecycle

RFQ statuses:

open
expired
accepted
cancelled
failed

Quote statuses:

active
expired
accepted
rejected
cancelled

Rules:

RFQ starts as open.
RFQ has expires_at_ms.
Quote has expires_at_ms.
Expired RFQ cannot receive quotes.
Expired quote cannot be accepted.
Accepting one quote sets RFQ status to accepted.
Accepting one quote sets quote status to accepted.
Other active quotes for same RFQ become rejected or remain active but non-acceptable; choose one deterministic behavior and document it.
Only one quote can win.
Accept quote must be idempotent or reject clearly if already accepted.
Accepted quote creates exactly one execution_intent.
No broadcast occurs during RFQ accept.
RFQ Direction Semantics

Define RFQ side from taker perspective:

side = buy  => taker wants to buy perp exposure, MM is seller
side = sell => taker wants to sell perp exposure, MM is buyer

Execution intent mapping:

If RFQ side is buy:

buyer = taker
seller = mm_account
buyer_is_maker = false

If RFQ side is sell:

buyer = mm_account
seller = taker
buyer_is_maker = true

Validate this carefully against existing execution intent / matching semantics.

Database

Add migration:

migrations/0010_rfqs.sql

Suggested tables:

CREATE TABLE rfqs (
    rfq_id TEXT PRIMARY KEY,
    taker TEXT NOT NULL,
    market_id BIGINT NOT NULL,
    side TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    limit_price_1e8 TEXT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    accepted_quote_id TEXT NULL,
    execution_intent_id TEXT NULL
);

CREATE INDEX idx_rfqs_status ON rfqs(status);
CREATE INDEX idx_rfqs_taker ON rfqs(lower(taker));
CREATE INDEX idx_rfqs_market_id ON rfqs(market_id);

CREATE TABLE rfq_quotes (
    quote_id TEXT PRIMARY KEY,
    rfq_id TEXT NOT NULL REFERENCES rfqs(rfq_id),
    mm_account TEXT NOT NULL,
    session_id TEXT NULL,
    client_quote_id TEXT NULL,
    price_1e8 TEXT NOT NULL,
    size_1e8 TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_rfq_quotes_rfq_id ON rfq_quotes(rfq_id);
CREATE INDEX idx_rfq_quotes_mm ON rfq_quotes(lower(mm_account));
CREATE INDEX idx_rfq_quotes_status ON rfq_quotes(status);

Use existing project DB style and SQLx patterns.

If repository uses different conventions, follow existing style.

Rust Modules

Add:

src/rfq/
  mod.rs
  types.rs
  service.rs

Optional:

src/rfq/store.rs

if repository style prefers separate store abstraction.

API Endpoints

Add HTTP endpoints:

POST /rfqs
GET /rfqs
GET /rfqs/:rfq_id
POST /rfqs/:rfq_id/quotes
GET /rfqs/:rfq_id/quotes
POST /rfqs/:rfq_id/accept/:quote_id
POST /rfqs/:rfq_id/cancel
POST /rfqs

Request:

{
  "taker": "0x...",
  "market_id": 1,
  "side": "buy",
  "size_1e8": "100000000",
  "limit_price_1e8": "305000000000",
  "ttl_ms": 5000
}

Response:

{
  "rfq_id": "...",
  "status": "open",
  "expires_at_ms": 1770000005000
}

Validation:

taker address valid
market exists
side is buy/sell
size > 0
ttl_ms within config bounds
limit_price optional but if present > 0
POST /rfqs/:rfq_id/quotes

V1A dev/HTTP quote submission.

Request:

{
  "mm_account": "0x...",
  "price_1e8": "300100000000",
  "size_1e8": "100000000",
  "client_quote_id": "mm-quote-001",
  "quote_ttl_ms": 3000
}

Response:

{
  "quote_id": "...",
  "rfq_id": "...",
  "status": "active",
  "expires_at_ms": 1770000003000
}

Validation:

RFQ exists
RFQ status open
RFQ not expired
MM account valid
price > 0
size > 0
quote size <= RFQ size for V1A
quote_ttl_ms within config bounds
quote expires no later than RFQ expires_at_ms
optional client_quote_id idempotence or duplicate rejection
GET /rfqs/:rfq_id/quotes

Return all quotes for RFQ.

POST /rfqs/:rfq_id/accept/:quote_id

Accept quote and create execution intent.

Response:

{
  "rfq_id": "...",
  "quote_id": "...",
  "status": "accepted",
  "execution_intent_id": "...",
  "onchain_intent_id": "0x..."
}

Validation:

RFQ exists
quote exists and belongs to RFQ
RFQ status open
RFQ not expired
quote status active
quote not expired
quote size compatible
price compatible with taker limit
no previous accepted quote

Critical:
This endpoint must create an execution_intent using existing execution-intent creation path if possible.

Do not duplicate execution-intent logic incorrectly.

Config

Add safe defaults:

RFQ_ENABLED=false
RFQ_REQUIRE_PERSISTENCE=true
RFQ_DEFAULT_TTL_MS=5000
RFQ_MAX_TTL_MS=30000
RFQ_MIN_QUOTE_TTL_MS=500
RFQ_MAX_QUOTE_TTL_MS=10000
RFQ_MAX_QUOTES_PER_RFQ=50

Startup:

If RFQ_ENABLED=true and persistence required but unavailable, fail clearly.
If disabled, endpoints can return "rfq is disabled" or stay registered but reject.

Normal tests must not require live Postgres.

Persistence / Atomicity

Accept quote must be safe.

Acceptance should be atomic at DB level if possible:

verify RFQ still open
verify quote still active
create execution_intent
set RFQ accepted_quote_id
set RFQ execution_intent_id
set RFQ status accepted
set accepted quote status accepted
reject or mark other active quotes deterministically

If full SQL transaction support already exists, use it.

If not, implement minimal safe ordering and document limitation.

Do not fake atomicity.

Execution Intent Creation

Accepted RFQ must create the same kind of execution_intent as orderbook matching.

Expected fields:

market_id from RFQ
buyer/seller derived from side
price_1e8 from accepted quote
size_1e8 from accepted quote or RFQ
buyer_is_maker according to side mapping
buyer_nonce / seller_nonce should use current configured behavior
deadline should use RFQ/quote expiry semantics or existing execution deadline defaults

Important:
RFQ acceptance must not sign PerpTrade.
RFQ acceptance must not simulate.
RFQ acceptance must not broadcast.
It only creates the execution_intent.

Existing endpoints should then work:

GET /execution-intents/:intent_id/signing-payload
POST /execution-intents/:intent_id/signatures
POST /executor/simulate/:intent_id
POST /executor/broadcast/:intent_id
Tests

Add tests for RFQ logic.

Normal cargo test must be offline.

Required tests:

create RFQ success
create RFQ rejects invalid taker
create RFQ rejects invalid side
create RFQ rejects zero size
create RFQ caps TTL
submit quote success
submit quote rejects expired RFQ
submit quote rejects wrong/zero price
submit quote rejects size > RFQ size
submit quote caps quote TTL to RFQ expiry
list quotes
accept quote success
accept quote rejects expired RFQ
accept quote rejects expired quote
accept quote rejects quote from different RFQ
accept quote rejects price beyond taker limit
accept quote creates execution_intent with correct buyer/seller for taker buy
accept quote creates execution_intent with correct buyer/seller for taker sell
accept quote is single-winner
cancel RFQ prevents quote acceptance

If full DB tests are complex, add pure service tests and repository tests using existing project patterns.

Documentation

Update:

README.md
ARCHITECTURE.md

Document:

RFQ V1A scope
RFQ lifecycle
quote lifecycle
HTTP endpoints
side semantics
execution_intent creation
no auto-broadcast
future RFQ V1B WebTransport integration
future signed RFQ quotes
future MM selection/ranking
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

RFQ config exists
RFQ tables migration exists
RFQ service exists
HTTP endpoints exist
quote submission works
quote acceptance creates execution_intent
side mapping is tested
no auto-broadcast
no WebTransport RFQ push yet
tests pass offline
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred to RFQ V1B
broadcast RFQ to connected MM sessions over WebTransport
receive rfq_quote over WebTransport
notify MM of accepted/expired quote
live RFQ runtime smoke over WebTransport
production wallet auth
signed RFQ quote messages
RFQ market maker ranking/selection
EOF



after all, 
## run telegram.py (say nothing, just run and that all)