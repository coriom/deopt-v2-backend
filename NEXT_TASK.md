# NEXT_TASK.md — Persistence V1 for Orders, Nonces, Trades, and Execution Intents

## Context

The Rust trading backend is implemented and validated.

Current status:
- deterministic in-memory matching works
- execution intents are created
- public API uses string fixed-point quantities
- signed order boundary exists
- nonce validation exists
- deadline validation exists
- strict EIP-712 signature verification works
- disabled signature mode still works for local development
- cargo fmt: OK
- cargo clippy --all-targets --all-features -- -D warnings: OK
- cargo test: OK
- cargo build: OK

Current limitation:
- orders, used nonces, trades, and execution intents are still in-memory
- restart loses nonce state
- restart loses pending execution intents
- this is not acceptable before on-chain executor integration

## Goal

Add persistence V1 using PostgreSQL.

This task must persist:
- submitted orders
- used nonces
- matched trades
- execution intents
- basic engine events if reasonable

This task must not implement blockchain execution.

## Stack

Use Rust async PostgreSQL with:

- `sqlx`
- `postgres`
- `runtime-tokio-rustls`
- migrations

Do not use:
- Diesel
- Prisma
- Node.js
- TypeScript
- Python

## Database Configuration

Add `.env.example` fields:

```env
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
PERSISTENCE_ENABLED=false

Rules:

PERSISTENCE_ENABLED=false should keep current in-memory behavior.
PERSISTENCE_ENABLED=true should require a valid DATABASE_URL.
Tests should not require a running Postgres unless explicitly marked/isolated.
Existing tests must continue passing without Postgres.
Required Files / Modules

Add modules such as:

src/db/mod.rs
src/db/pool.rs
src/db/models.rs
src/db/repository.rs
src/db/migrations.rs optional

migrations/
  0001_init.sql

Adapt naming if a cleaner structure already exists.

Database Schema Requirements

Create SQL migration with tables:

used_nonces

Purpose:

prevent replay after restart

Fields:

account TEXT NOT NULL
nonce BIGINT NOT NULL
created_at_ms BIGINT NOT NULL
primary key (account, nonce)
orders

Purpose:

store submitted orders and their current status

Fields:

order_id TEXT PRIMARY KEY
market_id BIGINT NOT NULL
account TEXT NOT NULL
side TEXT NOT NULL
order_type TEXT NOT NULL
time_in_force TEXT NOT NULL
price_1e8 TEXT NOT NULL
size_1e8 TEXT NOT NULL
remaining_size_1e8 TEXT NOT NULL
reduce_only BOOLEAN NOT NULL
post_only BOOLEAN NOT NULL
client_order_id TEXT NOT NULL
nonce BIGINT NOT NULL
deadline_ms BIGINT NOT NULL
signature TEXT NOT NULL
status TEXT NOT NULL
created_at_ms BIGINT NOT NULL
updated_at_ms BIGINT NOT NULL

Indexes:

(account)
(market_id)
(status)
unique (account, nonce)
trades

Purpose:

store off-chain matched trades before on-chain confirmation

Fields:

trade_id TEXT PRIMARY KEY
market_id BIGINT NOT NULL
maker_order_id TEXT NOT NULL
taker_order_id TEXT NOT NULL
maker_account TEXT NOT NULL
taker_account TEXT NOT NULL
price_1e8 TEXT NOT NULL
size_1e8 TEXT NOT NULL
buyer TEXT NOT NULL
seller TEXT NOT NULL
created_at_ms BIGINT NOT NULL

Indexes:

(market_id)
(maker_account)
(taker_account)
(buyer)
(seller)
execution_intents

Purpose:

store pending on-chain execution work

Fields:

intent_id TEXT PRIMARY KEY
market_id BIGINT NOT NULL
buyer TEXT NOT NULL
seller TEXT NOT NULL
price_1e8 TEXT NOT NULL
size_1e8 TEXT NOT NULL
buy_order_id TEXT NOT NULL
sell_order_id TEXT NOT NULL
status TEXT NOT NULL
created_at_ms BIGINT NOT NULL
updated_at_ms BIGINT NOT NULL

Indexes:

(status)
(market_id)
(buyer)
(seller)
engine_events optional

Purpose:

audit trail / replay foundation

Fields:

event_id TEXT PRIMARY KEY
event_type TEXT NOT NULL
payload_json TEXT NOT NULL
created_at_ms BIGINT NOT NULL

If adding this table creates too much scope, scaffold it but do not fully wire it.

Persistence Semantics

When PERSISTENCE_ENABLED=false:

current in-memory behavior remains
no DB connection required
all existing tests pass

When PERSISTENCE_ENABLED=true:

connect to Postgres at startup
run migrations or clearly document migration command
persist accepted orders
persist used nonce atomically with order acceptance
persist matched trades
persist execution intents
update order statuses after fills/cancellations
Atomicity Requirement

For order submission with persistence enabled:

The following must be atomic or clearly protected against partial failure:

nonce insertion
order insertion
matching result persistence
order status updates
execution intent insertion

Use a SQL transaction where practical.

If full atomic persistence around the engine is too large for this pass:

implement repository methods
wire only safe persistence points
document deferred atomic transaction boundary clearly

Do not silently create inconsistent DB state.

Nonce Persistence

Nonce behavior:

reused nonce for same account must be rejected even after restart when persistence is enabled
same nonce for different accounts is allowed
nonce uniqueness is per account
nonce must remain nonzero

Important:

In-memory nonce validation must still work when persistence is disabled.
Persistent nonce validation must be used when persistence is enabled.
API Requirements

No route shape changes required.

Keep existing endpoints:

GET /health
GET /markets
GET /orderbook/:market_id
POST /orders
DELETE /orders/:order_id
GET /execution-intents

If persistence is enabled:

GET /execution-intents may return persisted intents, in-memory intents, or a merged view.
Prefer the simplest correct implementation.
Do not break existing behavior.
Testing Requirements

Existing tests must continue passing without Postgres.

Add unit tests for:

repository model conversion if possible without DB
config parsing:
persistence disabled does not require DATABASE_URL
persistence enabled requires DATABASE_URL
nonce behavior remains unchanged in in-memory mode

Add optional/integration tests only if they can be safely ignored unless DATABASE_URL is set.

Do not make normal cargo test depend on a running database.

Documentation Requirements

Update README.md:

explain persistence mode
explain required env vars
explain migration command
explain local Postgres setup briefly
explain that persistence is required before production executor usage

Update ARCHITECTURE.md:

describe persistence V1
explain in-memory vs persistent mode
explain why nonce persistence matters
state that on-chain execution is still deferred
Constraints

Do not add:

blockchain RPC
transaction sending
private key loading
frontend
TypeScript
Python
Node.js
Solidity changes
fake on-chain settlement
fake finality

Do not modify:

~/DEOPT/deoptv2

Do not introduce:

floating point financial math
matching semantic changes
fake transaction confirmation
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

If DB integration tests are added, they must be opt-in and documented.

Acceptance Criteria

The task is complete only if:

persistence config exists
SQL migration exists
repository layer exists
used nonces can be persisted when enabled
orders/trades/execution intents can be persisted when enabled
existing in-memory mode remains default and passes all tests
no normal test requires Postgres
all validation commands pass
no blockchain execution is added
EOF