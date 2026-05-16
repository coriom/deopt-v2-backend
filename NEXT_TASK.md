# NEXT_TASK.md — E2E Test Harness V1A: Reproducible Runtime Flows

## Context

DeOpt backend now has many runtime-verified flows:

- backend health
- admin API token guard
- admin monitoring
- MM WebTransport gateway
- MM wallet challenge auth
- MM permissions
- perp execution confirmation
- index/reconciliation/finality
- perp fees
- option series/orders/matching/fills
- option RFQ HTTP
- option RFQ WebTransport
- signed option RFQ quotes
- option fees/rebates
- admin fee endpoints

Problem:

Most runtime verification has been performed manually through Codex prompts, shell commands, curl, psql, and smoke binaries.

This is no longer scalable.

## Goal

Implement E2E Test Harness V1A.

Create reproducible scripts for local/testnet runtime verification.

The harness must make it easy to run selected E2E flows and produce machine-readable reports.

## Non-Goals

Do not implement new protocol features.
Do not modify Solidity.
Do not deploy contracts.
Do not enable real broadcast by default.
Do not call /executor/broadcast unless an explicit broadcast test flag is passed.
Do not move funds.
Do not expose private keys.
Do not replace unit tests.
Do not require CI integration yet.
Do not commit.
Do not push.

## Safety Rules

Default mode must be safe:

```text
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false unless a gateway test explicitly enables it

No private keys in logs.

No .env permanent edits.

Use process env or temporary env files.

Cleanup only rows created by the harness and only when explicitly safe.

Preferred Location

Add scripts under:

scripts/e2e/

Possible structure:

scripts/e2e/README.md
scripts/e2e/run_e2e.py
scripts/e2e/lib/
scripts/e2e/flows/

If Rust binaries are better for this repo, use:

src/bin/e2e_harness.rs

But prefer Python for orchestration if the repo already uses shell/curl/psql workflows.

Do not add heavy dependencies.

Use standard Python library if possible:

subprocess
json
urllib.request
argparse
time
os
signal

If needing HTTP convenience, avoid adding dependencies unless already present.

Required CLI

The harness should support:

python3 scripts/e2e/run_e2e.py --flow admin
python3 scripts/e2e/run_e2e.py --flow fees-options
python3 scripts/e2e/run_e2e.py --flow fees-perps
python3 scripts/e2e/run_e2e.py --flow option-rfq
python3 scripts/e2e/run_e2e.py --flow mm-auth
python3 scripts/e2e/run_e2e.py --flow all-safe

Optional flags:

--backend-url http://127.0.0.1:8080
--database-url postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
--admin-token local-admin-token-runtime-test
--start-backend
--no-start-backend
--timeout-sec 120
--json-out /tmp/deopt-e2e-report.json
--cleanup
--verbose
Report Format

Each run should output a JSON report:

{
  "ok": true,
  "flow": "admin",
  "started_at_ms": 1770000000000,
  "finished_at_ms": 1770000001234,
  "checks": [
    {
      "name": "health",
      "ok": true,
      "details": {}
    }
  ],
  "artifacts": {
    "option_series_id": "...",
    "option_rfq_id": "...",
    "fee_event_ids": []
  },
  "errors": []
}

Also print a concise human summary.

Backend Management

If --start-backend is used:

start cargo run --bin deopt-v2-backend
pass safe process env
wait for /health
kill backend at the end
ensure no orphan process remains

If backend is already running:

use --no-start-backend
just call endpoints
Flow: admin

Verify:

/health
/admin/status with token
/admin/config
/admin/db
/admin/fees/summary
wrong token rejection
no secret substrings in /admin/config
Flow: fees-options

Verify:

create active option series
seed MM permissions if needed
create option orderbook fill
verify fee_events.source_type=option_order_fill
verify volume_buckets.market_type=option
create option RFQ fill
verify fee_events.source_type=option_rfq_fill
verify admin fees endpoints

Do not use private keys.

Do not call broadcast.

Flow: fees-perps

Use an existing confirmed/indexed/reconciled perp trade if available.

Verify:

find candidate trade
trigger POST /executor/confirm/:intent_id
verify fee_events.source_type=perp_trade
verify volume_buckets.market_type=perp
verify idempotency by re-triggering once
verify no new execution transaction

If no candidate trade exists:

report skipped with reason
do not fake a trade
Flow: option-rfq

Verify:

create option RFQ
submit quote
accept quote
verify option_rfq_fill
if strict signature mode enabled, optionally use existing signing CLI
no execution transaction

V1A can use unsigned mode unless strict mode env is passed.

Flow: mm-auth

This can be partial in V1A.

If WebTransport runtime is too heavy, expose wrapper that calls existing mm_wt_smoke binary.

Support:

python3 scripts/e2e/run_e2e.py --flow mm-auth --enable-webtransport

It should:

generate ECDSA certs under /tmp/deopt-mm-gateway
start backend with MM gateway enabled
run cargo run --bin mm_wt_smoke -- auth
parse success from exit status/output

If too broad, document as deferred but include placeholder flow that reports skipped.

Flow: all-safe

Run only flows that do not require:

real broadcast
private keys
live WebTransport unless explicitly enabled

Recommended included:

admin
fees-options
fees-perps if candidate exists
option-rfq
DB Helpers

Use psql through subprocess.

Never build unsafe shell strings with untrusted values.

Prefer passing SQL through stdin or psql -c.

Only cleanup rows with recognizable runtime prefixes:

runtime-e2e-

Do not delete historical rows unless explicitly created by harness.

Environment

Default safe environment:

PERSISTENCE_ENABLED=true
ADMIN_API_ENABLED=true
ADMIN_API_REQUIRE_TOKEN=true
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false
FEES_ENABLED=true
FEES_REBATES_ENABLED=true
OPTIONS_ENABLED=true
OPTION_RFQ_ENABLED=true
MM_PERMISSIONS_ENABLED=true

Process env only.

Do not write .env.

Tests

Add lightweight tests if feasible:

JSON report builder
command arg parsing
URL building
secret redaction helper
no mutation method list

Do not make unit tests depend on live Postgres or backend.

Documentation

Add:

scripts/e2e/README.md

Document:

prerequisites
safe defaults
each flow
examples
cleanup behavior
what is intentionally not tested
how to run against already-running backend

Update root README if appropriate with a small pointer.

Validation

Run:

python3 scripts/e2e/run_e2e.py --help
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

If Python files are added and syntax check is useful:

python3 -m py_compile scripts/e2e/run_e2e.py
Acceptance Criteria

Complete only if:

harness exists
admin flow works or is implemented
fees-options flow works or is implemented
fees-perps flow safely skips if no candidate exists
reports are JSON
default env is safe
no broadcast by default
docs exist
validation passes
Deferred
CI integration
real broadcast E2E
full browser E2E
full WebTransport orchestration if too heavy
load testing
chaos testing
EOF