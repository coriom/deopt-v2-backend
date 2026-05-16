# NEXT_TASK.md — Front Admin V1B: Fees / Volume / Rebates Dashboard

## Context

Frontend Admin V1A is implemented at `/admin`.

It currently consumes:

- GET /admin/status
- GET /admin/config
- GET /admin/db
- GET /admin/mm/sessions
- GET /admin/execution/summary
- GET /admin/rfq/summary
- GET /admin/options/summary
- GET /admin/recent?limit=N

Backend now also exposes Fees & Rebates V1A admin endpoints:

- GET /admin/fees/summary
- GET /admin/fees/events?limit=20
- GET /admin/fees/volumes?account=0x...
- GET /admin/fees/rebates?account=0x...

Fees & Rebates V1A runtime verification passed:

- fee_events created
- volume_buckets updated
- rebate_accruals created
- RFQ discounts verified
- top-tier rebates verified
- no payout
- no execution transactions

## Goal

Extend the read-only admin dashboard with a Fees section.

The frontend must display:

- fee summary
- recent fee events
- volume buckets for an optional account
- rebate accruals for an optional account
- tier/rebate information if present in API responses
- no write actions

## Non-Goals

Do not implement:

- fee admin writes
- payout buttons
- rebate payment
- fee tier editing
- MM permission editing
- trading actions
- backend changes unless absolutely necessary
- wallet login
- production auth

## Safety Rules

Do not:

- call POST/PUT/PATCH/DELETE endpoints
- call /executor/broadcast
- hardcode secrets
- hardcode admin token
- store admin token in localStorage
- expose private keys
- modify backend
- commit
- push

Frontend remains read-only.

## Required API Calls

Add to admin API client:

```text
GET /admin/fees/summary
GET /admin/fees/events?limit=N
GET /admin/fees/volumes?account=<optional>
GET /admin/fees/rebates?account=<optional>

Use the existing admin token handling:

X-Admin-Token: <token>
UI Requirements

Add a new dashboard section:

Fees & Rebates

It should include cards/tables for:

Fee Summary
Recent Fee Events
Volume Buckets
Rebate Accruals
Fee Summary

Display:

fees_enabled if present
rebates_enabled if present
event_count
fee_total_1e8
rebate_total_1e8
protocol_total_1e8 if present
counts by source_type / market_type / flow_type / status if present

Render unknown nested fields safely.

Recent Fee Events

Use:

GET /admin/fees/events?limit=20

Add limit selector:

5 / 20 / 50 / 100

Show useful columns if available:

source_type
source_id
market_type
flow_type
maker
taker
payer
recipient
fee_asset
notional_1e8
fee_rate_micro_bps
fee_amount_1e8
rebate_rate_micro_bps
rebate_amount_1e8
protocol_amount_1e8
status
created_at_ms

If response shape differs, render safely.

Volume Buckets

Add optional account filter input.

Default can be empty.

When empty:

either call endpoint without account if supported
or show instruction to enter account

When account is provided:

GET /admin/fees/volumes?account=<account>

Show:

account
bucket_day
market_type
maker_volume_1e8
taker_volume_1e8
total_volume_1e8
updated_at_ms
Rebate Accruals

Use same optional account filter.

When account provided:

GET /admin/fees/rebates?account=<account>

Show:

account
source_type
source_id
rebate_asset
rebate_amount_1e8
status
created_at_ms

Clearly show:

status=accrued

Do not imply payout happened.

Formatting

Add helpers if useful:

shorten address
format 1e8 numeric strings approximately
format micro_bps to bps/percent
format timestamp ms to local date

Do not lose raw values. Prefer showing raw + formatted if possible.

Example:

fee_rate_micro_bps=7500 → 0.75 bps → 0.0075%
Refresh

Fees section must use existing refresh mechanics:

manual refresh
auto-refresh if enabled
no backend spam
Error Handling

Handle:

backend offline
admin disabled
missing/invalid token
404 if backend not updated
malformed response
empty fees/rebates

No crash.

Existing Dashboard

Preserve existing Admin V1A sections.

Do not remove:

Status
Config
Database
MM Sessions
Execution
RFQ
Options
Recent Activity

Add Fees section cleanly.

Documentation

Update frontend README if appropriate:

mention fees dashboard
mention fee endpoints
explain read-only nature
explain no payouts
Validation

Run available commands:

npm run lint
npx tsc --noEmit
npm run build

If one command is unavailable, report it.

Acceptance Criteria

Complete only if:

/admin still works
Fees section exists
all four fee endpoints are consumed
account filter works for volumes/rebates
recent fee event limit selector works
token handling reused
no mutation endpoints called
no secrets hardcoded
lint passes
typecheck passes
build passes
Deferred
fee charts
fee tier editor
payout UI
MM rebate approval UI
Prometheus/Grafana
backend fee changes
EOF



after all, 
## run telegram.py (say nothing, just run and that all)