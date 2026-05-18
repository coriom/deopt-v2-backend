# NEXT_TASK.md — Monitoring V1B: Metrics, Structured Observability, and Alert Spec

## Context

DeOpt backend now has:

- static backend CI
- runtime E2E CI with Postgres
- local E2E harness
- admin read-only endpoints
- fees/rebates ledger
- MM auth/permissions
- RFQ/options/perp execution lifecycle

Current admin monitoring is read-only and useful, but not production-grade observability.

Missing:

- Prometheus-style metrics
- structured operational counters
- latency/error metrics
- alerting specification
- clearer health/readiness split

## Goal

Implement Monitoring V1B.

Add backend observability primitives:

- `/metrics` endpoint
- basic Prometheus text exposition format
- structured counters/gauges for key subsystems
- health/readiness distinction if not already present
- alerting documentation/spec
- tests
- docs

## Non-Goals

Do not implement:

- Grafana dashboard
- external alert delivery
- Slack/Telegram alerts
- Prometheus server deployment
- production auth
- admin write endpoints
- frontend monitoring UI
- protocol changes
- Solidity changes
- broadcast/deploy

## Safety Rules

Do not:

- expose secrets
- expose private keys
- expose raw DB URL
- expose raw RPC URL with keys
- call /executor/broadcast
- enable real broadcast
- require live RPC/Postgres for normal cargo test
- commit
- push

## Metrics Endpoint

Add:

```text
GET /metrics

Default behavior:

enabled by config
safe text output
no secrets
no labels with unbounded cardinality such as full tx hashes, intent IDs, RFQ IDs, wallet addresses

Config:

METRICS_ENABLED=true
METRICS_REQUIRE_ADMIN_TOKEN=false

If admin token requirement is easy to reuse, support optional protection. Default can be open for local Prometheus-style scraping, but document production implications.

Metric Format

Use simple Prometheus text format.

Example:

# HELP deopt_backend_up Backend process is up.
# TYPE deopt_backend_up gauge
deopt_backend_up 1

# HELP deopt_admin_enabled Admin API enabled.
# TYPE deopt_admin_enabled gauge
deopt_admin_enabled 1

Do not add a heavy dependency unless clearly justified.

Manual string rendering is acceptable for V1B.

Required Metrics

Add safe metrics for:

Process / Config
deopt_backend_up
deopt_persistence_enabled
deopt_execution_enabled
deopt_real_broadcast_enabled
deopt_mm_gateway_enabled
deopt_rfq_enabled
deopt_options_enabled
deopt_fees_enabled
deopt_rebates_enabled
Database

If persistence enabled and DB accessible:

deopt_db_up
deopt_db_migrations_installed

If persistence disabled:

deopt_db_up 0 or omit with documented behavior
Execution

If available from repository/admin summary:

deopt_execution_intents_total{status="..."}
deopt_execution_simulations_total{status="..."}
deopt_execution_transactions_total{status="..."}
deopt_execution_confirmed_total

Keep labels low cardinality.

RFQ
deopt_rfqs_total{status="..."}
deopt_rfq_quotes_total{status="..."}
Options
deopt_option_series_total{status="..."}
deopt_option_orders_total{status="..."}
deopt_option_fills_total
deopt_option_rfqs_total{status="..."}
deopt_option_rfq_quotes_total{status="..."}
Fees
deopt_fee_events_total{market_type="perp|option",source_type="...",status="..."}
deopt_rebate_accruals_total{status="..."}
MM Gateway

If gateway registry is available:

deopt_mm_sessions_total
deopt_mm_sessions_authenticated_total

If disabled:

deopt_mm_sessions_total 0
Health / Readiness

Current /health exists.

Add or verify:

GET /ready

Readiness should check:

backend process up
config is valid
if persistence enabled, DB ping ok
if required services disabled, do not fail readiness
do not require RPC unless a feature requiring RPC is enabled

If adding /ready is too broad, document why and add tests for /health + /admin/status.

Structured Logging

Do not rewrite logging system.

Add minimal structured log conventions if useful:

service
subsystem
event
status
error_code when available

Update docs with expected log fields.

Alert Spec

Add docs:

docs/ALERTING_SPEC.md

Include alert ideas:

backend down
DB down
real_broadcast_enabled unexpectedly true
execution confirmations stuck
reconciliation unmatched/ambiguous rising
simulation failures rising
stale indexer cursor
RFQ quote rejection spike
MM session drop
fee ledger write failures
oracle stale if exposed later
liquidation/bad debt alerts later

This is documentation/spec only. No external alerting implementation.

Tests

Normal cargo test must remain offline.

Add tests for:

metrics endpoint disabled/enabled config if configurable
metrics output contains backend_up
metrics output contains safe config gauges
metrics output does not contain private keys/admin token/DB URL/RPC URL
metrics labels do not include wallet addresses / tx hashes / UUIDs
persistence disabled metrics still render
empty in-memory state metrics render
readiness succeeds with persistence disabled
readiness fails or reports not ready when persistence enabled and DB unavailable, if readiness added
no mutation from metrics endpoint
existing admin tests still pass
Documentation

Update:

README.md
ARCHITECTURE.md
scripts/e2e/README.md if relevant
docs/ALERTING_SPEC.md
.env.example

Document:

/metrics
/ready if added
config flags
no secrets
low-cardinality label policy
local curl examples
Prometheus scrape example
alerting spec location
CI / Validation

Update backend CI only if needed to include new docs/tests.

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

Optionally:

python3 scripts/e2e/run_e2e.py --help
Acceptance Criteria

Complete only if:

/metrics exists
metrics are safe and low-cardinality
no secrets exposed
core subsystem metrics included
/ready added or explicitly documented as deferred
alerting spec exists
tests added
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
Grafana dashboard
Prometheus deployment
external alert delivery
frontend metrics UI
OpenTelemetry tracing
advanced latency histograms
WebTransport metrics CI
EOF