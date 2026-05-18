# NEXT_TASK.md — Runtime Verify Monitoring V1B

## Context

Monitoring V1B has been implemented.

Added:

- GET /metrics
- GET /ready
- METRICS_ENABLED=true
- METRICS_REQUIRE_ADMIN_TOKEN=false
- Prometheus-style text exposition
- low-cardinality subsystem metrics
- docs/ALERTING_SPEC.md
- offline tests

Local validation passed:

- cargo fmt
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test
- cargo build

## Goal

Runtime-verify Monitoring V1B with PostgreSQL persistence enabled.

Prove that:

- `/metrics` works
- `/ready` works
- metrics include expected subsystem gauges/counters
- metrics do not expose secrets or high-cardinality values
- optional metrics token protection works
- readiness checks Postgres correctly
- no mutation occurs

## Non-Goals

Do not implement new metrics.
Do not implement Grafana.
Do not implement external alerts.
Do not implement frontend monitoring UI.
Do not modify Solidity.
Do not deploy.
Do not enable real broadcast.
Do not call /executor/broadcast.
Do not commit.
Do not push.

## Safety Rules

Do not expose:

- private keys
- admin tokens
- raw DATABASE_URL
- raw RPC_URL
- wallet addresses as labels
- tx hashes as labels
- UUIDs as labels
- signatures
- session internals

Runtime config must keep:

```env
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false
Runtime Setup

Use process env only.

Start backend with:

PERSISTENCE_ENABLED=true \
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend \
ADMIN_API_ENABLED=true \
ADMIN_API_REQUIRE_TOKEN=true \
ADMIN_API_TOKEN=local-admin-token-runtime-test \
METRICS_ENABLED=true \
METRICS_REQUIRE_ADMIN_TOKEN=false \
EXECUTION_ENABLED=false \
EXECUTOR_REAL_BROADCAST_ENABLED=false \
MM_GATEWAY_ENABLED=false \
cargo run --bin deopt-v2-backend
Checks
1. Health
curl http://127.0.0.1:8080/health

Expected:

{"ok":true,"service":"deopt-v2-backend"}
2. Readiness
curl -i http://127.0.0.1:8080/ready

Expected:

HTTP 200
ready=true or equivalent
DB ready when persistence enabled
no secrets
3. Metrics open mode
curl -i http://127.0.0.1:8080/metrics

Expected:

HTTP 200
Content-Type text/plain / Prometheus format
contains:
deopt_backend_up
deopt_persistence_enabled
deopt_execution_enabled
deopt_real_broadcast_enabled
deopt_db_up
deopt_db_migrations_installed
deopt_fee_events_total
deopt_rebate_accruals_total
4. Secret scan

Save metrics:

curl http://127.0.0.1:8080/metrics > /tmp/deopt-metrics.txt

Search forbidden substrings:

local-admin-token-runtime-test
postgres://deopt:deopt
PRIVATE_KEY
DEPLOYER_PRIVATE_KEY
EXECUTOR_PRIVATE_KEY
RPC_URL
ADMIN_API_TOKEN
0x0803794fc97c2ac2e1d178d141b0aee9df4a423f9f2257295560a969c8dc01ac

Expected:

none found
5. High-cardinality scan

Metrics must not expose labels containing:

full wallet addresses
tx hashes
UUIDs
session IDs
signatures

Use simple grep patterns for:

0x[a-fA-F0-9]{40}
0x[a-fA-F0-9]{64}
[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}

Expected:

no matches in labels or metric lines
6. Metrics token protection

Restart backend with:

METRICS_REQUIRE_ADMIN_TOKEN=true

Then:

curl -i http://127.0.0.1:8080/metrics

Expected:

403 or clear token-required error

Then:

curl -i http://127.0.0.1:8080/metrics \
  -H "X-Admin-Token: wrong-token"

Expected:

403 or clear invalid-token error

Then:

curl -i http://127.0.0.1:8080/metrics \
  -H "X-Admin-Token: local-admin-token-runtime-test"

Expected:

200
metrics render
7. Readiness DB behavior

If feasible, test persistence-disabled mode:

PERSISTENCE_ENABLED=false

Expected:

/ready still succeeds if DB is not required
/metrics still renders safe values

Do not break local DB state.

8. No mutation

Record before/after counts for key tables:

SELECT COUNT(*) FROM execution_transactions;
SELECT COUNT(*) FROM execution_intents;
SELECT COUNT(*) FROM fee_events;
SELECT COUNT(*) FROM rebate_accruals;

Expected:

unchanged after /metrics and /ready calls
9. Stop backend

Ensure no orphan process remains:

pgrep -af deopt-v2-backend || true
ss -ltnp | grep ':8080' || true
If Bug Found

Patch minimally only.

After patch:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Final Report

Return:

files changed
whether code patch was needed
backend startup result
/health result
/ready result
/metrics open-mode result
expected metrics found
secret scan result
high-cardinality scan result
metrics token-protection result
persistence-disabled readiness result if tested
no-mutation verification
backend cleanup result
validation commands run
remaining blocker

