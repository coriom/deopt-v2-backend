# NEXT_TASK.md — Monitoring/Admin V1A: Read-only Operational Observability

## Context

The backend now has many validated protocol flows:

Perps:
- order intake
- execution intents
- signing payloads
- simulation
- guarded broadcast
- indexer
- reconciliation
- confirmation/finality
- nonce sync

Market makers:
- WebTransport MM Gateway
- sessions
- heartbeat/get_session
- live order/cancel/quote_replace
- cancel-on-disconnect

RFQ:
- perp RFQ core
- perp RFQ over WebTransport
- signed perp RFQ quotes

Options:
- option series registry
- option orders
- option orderbook aggregation
- off-chain option matching
- option fills
- option RFQ HTTP/core
- option RFQ over WebTransport
- signed option RFQ quotes strict mode

The protocol layer is now broad enough that operational visibility is required before serious frontend/admin work.

## Goal

Implement Monitoring/Admin V1A: read-only backend operational observability.

Add safe read-only endpoints and lightweight metrics/snapshots for:

- backend health/readiness
- database connectivity
- configured feature flags
- MM Gateway sessions
- RFQ/order/fill counts
- execution lifecycle status
- recent errors/events if available
- recent execution simulations/transactions/confirmations
- recent option RFQs/fills/orders
- recent perp RFQs/execution intents

This task must not add dangerous admin writes.

## Non-Goals

Do not implement:

- frontend
- admin mutation endpoints
- risk parameter editing
- market maker permission management
- production auth
- Prometheus exporter if too broad
- external alerting integrations
- Grafana dashboards
- Slack/Telegram alerts
- on-chain writes
- Solidity changes
- deployments

## Absolute Safety Rules

Do not:

- modify Solidity
- deploy contracts
- enable real broadcast by default
- call /executor/broadcast
- create execution intents
- create RFQs
- create option orders
- mutate DB state except migrations if needed
- expose private keys
- expose secrets
- expose full env
- commit
- push

Admin/monitoring endpoints must be read-only.

## API Scope

Add endpoints under:

```text
GET /admin/status
GET /admin/config
GET /admin/db
GET /admin/mm/sessions
GET /admin/execution/summary
GET /admin/rfq/summary
GET /admin/options/summary
GET /admin/recent

If /admin namespace conflicts with current routing, use /ops.

Security

For V1A, keep endpoints local/dev-oriented.

Add config:

ADMIN_API_ENABLED=false
ADMIN_API_REQUIRE_TOKEN=false
ADMIN_API_TOKEN=

Behavior:

if disabled, admin endpoints return clear disabled error
if enabled and token required, require header:
X-Admin-Token: <token>
never log token
never expose token in responses
normal tests must not require token unless testing token path

Do not implement complex auth yet.

GET /admin/status

Return high-level status:

{
  "service": "deopt-v2-backend",
  "ok": true,
  "timestamp_ms": 1770000000000,
  "network": "base-sepolia",
  "chain_id": 84532,
  "persistence_enabled": true,
  "execution_enabled": false,
  "real_broadcast_enabled": false,
  "indexer_enabled": false,
  "reconciliation_enabled": true,
  "confirmation_enabled": false,
  "mm_gateway_enabled": false,
  "rfq_enabled": true,
  "options_enabled": true,
  "option_rfq_enabled": true
}

Do not expose secrets or raw RPC URLs if they contain keys.

GET /admin/config

Return sanitized config.

Include:

network
chain_id
enabled feature flags
contract addresses if already public config
RPC configured boolean, not raw RPC URL
database configured boolean, not password
WebTransport host/port if enabled
signature modes
RFQ TTL/max settings
option settings

Must redact:

private keys
DB password
RPC provider key if present
admin token
any secret env
GET /admin/db

If persistence enabled:

ping DB
return migration status if easy
return counts by important tables

Suggested counts:

orders
used_nonces
execution_intents
execution_simulations
execution_transactions
indexed_perp_trades
reconciliations
rfqs
rfq_quotes
option_series
option_orders
option_fills
option_rfqs
option_rfq_quotes
option_rfq_fills

If a table does not exist in older DB, handle gracefully or document.

GET /admin/mm/sessions

Return MM session snapshots.

For each session:

{
  "session_id": "...",
  "authenticated": false,
  "account": "0x...",
  "connected_at_ms": 1770000000000,
  "last_heartbeat_at_ms": 1770000000000,
  "open_client_order_ids_count": 3,
  "cancel_on_disconnect": true
}

Do not expose internals that are not useful.

If gateway disabled, return enabled=false and empty sessions.

GET /admin/execution/summary

Return:

count pending execution intents
count signed/calldata-ready intents if available
count simulations ok/failed
count submitted txs
count confirmed txs
recent failed simulations with decoded_error if available
recent unconfirmed txs
recent confirmation errors

Use repository queries if persistence enabled.

If persistence disabled, return in-memory summary if available.

GET /admin/rfq/summary

Return perp RFQ summary:

count rfqs by status
count quotes by status
count signed/verified quotes if metadata exists
recent accepted RFQs
recent rejected/expired if easy
GET /admin/options/summary

Return options summary:

option series count by status
option orders count by status
option fills count
option RFQs count by status
option RFQ quotes count by status/signature_status
recent option RFQ fills
recent option order fills
GET /admin/recent

Return compact recent operational events from existing DB tables.

Possible sections:

{
  "execution_intents": [...],
  "simulations": [...],
  "transactions": [...],
  "rfqs": [...],
  "option_rfqs": [...],
  "option_fills": [...]
}

Limit default: 20.

Support query:

?limit=20

Hard cap: 100.

Internal Metrics / Repository

Add repository helpers if needed.

Prefer simple SQL aggregate queries.

Avoid heavy joins.

Avoid N+1 queries if obvious.

All admin queries should be safe and bounded.

Tests

Normal cargo test must be offline.

Add tests for:

admin disabled returns clear error
admin enabled status works
admin token required rejects missing token
admin token required accepts valid token
config endpoint redacts secrets
db endpoint handles persistence disabled
db endpoint counts known in-memory or mock state if feasible
mm sessions endpoint returns empty when gateway disabled
mm sessions endpoint returns mock/session snapshot if existing registry supports it
execution summary returns valid empty summary
options summary returns valid empty summary
rfq summary returns valid empty summary
recent endpoint respects limit cap
no endpoint mutates state
existing perp/RFQ/MM/options tests still pass

Do not require live Postgres for normal tests.

Documentation

Update:

README.md
ARCHITECTURE.md
.env.example

Document:

admin API purpose
endpoints
read-only scope
token config
redaction rules
operational use
future Monitoring/Admin V1B:
Prometheus metrics
structured event log
external alerts
frontend admin dashboard
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

admin config exists
endpoints exist
endpoints are read-only
secrets are redacted
token guard works
status/config/db/mm/execution/rfq/options/recent endpoints work
normal tests offline
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
frontend dashboard
Prometheus exporter
Grafana dashboard
external alerts
production auth
admin write controls
risk admin controls
EOF



after all, 
## run telegram.py (say nothing, just run and that all)