# NEXT_TASK.md — Backend basisAmount Exposure And Options V2 Fee Closure V2E-I

## Context

V2E-H completed frontend/admin observability for V2 option fees.

V2E-G validated live FeesManagerV2 option trade:
- intent = 94897ee5-e855-40b6-a917-1476578fe48b
- tx = 0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c
- event_model = mixed
- source_priority = v2
- fee_charged_v2_count = 2
- fee_rebated_v2_count = 0
- observed_total_charged = 16
- net_protocol_fee = 16
- taker = 13
- maker = 3
- recipient = 0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588

Known V2E-H remaining gap:
- `basisAmount` is decoded and persisted in V2 fee events.
- But `basisAmount` is not surfaced through `collect_event_payloads` in:
  `deopt-v2-backend/src/fees/onchain_summary.rs`
- Admin V2 per-event cards therefore render `basisAmount = n/a`.

Goal:
Expose `basisAmount` through backend fee summary payloads, verify admin observability, and close the options V2 Tier0-positive-fee phase before moving to perps.

## Hard Rules

Do not broadcast.
Do not submit transactions.
Do not deploy.
Do not modify Solidity.
Do not create option execution intents.
Do not create option execution transactions.
Do not call option broadcast endpoints.
Do not modify live DB.
Do not print secrets.
Do not commit real `.env`.

Allowed:
- backend read-only response shape update.
- frontend type/UI adjustment if needed.
- tests.
- docs.
- read-only endpoint calls.

## Repos

Primary:

```text
~/DEOPT/deopt-v2-backend

Secondary if needed:

~/DEOPT/deopt-v2-frontend
Step 1 — Backend basisAmount Exposure

Inspect:

src/fees/onchain_summary.rs
src/options/lifecycle.rs
src/fees/service.rs
src/types/admin.ts in frontend if needed

Required backend change:

Add basis_amount / basisAmount to normalized V2 fee event payloads returned by:
/admin/fees/onchain
/admin/options/executions/:intent_id/lifecycle

Implementation expectation:

FeeChargedV2 event payload must include:
basisAmount
feeAmount
feePpm
productKind
flowKind
isMaker
trader
recipient
FeeRebatedV2 event payload must include:
basisAmount
rebateAmount
rebatePpm
same contextual fields if decoded.

Use existing decoded JSON field names. Do not re-decode from logs if already available.

Step 2 — Backend Tests

Add or update tests for:

V2 FeeChargedV2 onchain summary includes basisAmount.
lifecycle fees event payload includes basisAmount.
mixed V1/V2 still uses V2 as source of truth.
V1 compatibility response remains backward compatible.

Expected V2E-G values:

basisAmount = 50000
taker feeAmount = 13
maker feeAmount = 3
Step 3 — Frontend Adjustment If Needed

If frontend type expects basisAmount already but backend uses snake_case, normalize.

Ensure admin cards show a real basis amount for V2E-G:

basisAmount = 50000

Do not add write actions.

Step 4 — Read-only Manual Verification

Run backend locally with safe read-only/admin config if needed.

Call:

curl -s \
  "http://127.0.0.1:8080/admin/fees/onchain?tx_hash=0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c" \
  -H "X-Admin-Token: $ADMIN_TOKEN"

And:

curl -s \
  "http://127.0.0.1:8080/admin/options/executions/94897ee5-e855-40b6-a917-1476578fe48b/lifecycle" \
  -H "X-Admin-Token: $ADMIN_TOKEN"

Expected:

event_model = mixed
source_priority = v2
fee_charged_v2_count = 2
fee_rebated_v2_count = 0
observed_total_charged = 16
taker = 13
maker = 3
each V2 fee event exposes basisAmount = 50000
Step 5 — Options V2 Fee Closure Doc

Create:

docs/OPTIONS_V2_FEE_TIER0_CLOSURE_V2E_I.md

Include:

What is validated:
FeesManagerV2 deployed
wired
enabled
backend indexer emitter configured
live option trade succeeded
FeeChargedV2 indexed
lifecycle/admin fees expose V2 source of truth
Tier0 positive option fees validated
What is intentionally deferred:
rebates / negative maker tiers
rebate budget funding
Merkle tiers
RFQ discounts
multi-asset fee campaign
perps integration
Criteria for moving to perps:
passed.
Required Validation

Backend:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features

Frontend, if touched:

npm run lint
npx tsc --noEmit
npm run build
Acceptance Criteria

Complete only if:

basisAmount is visible in backend response payloads.
V2E-G basis amount = 50000 is visible.
no write/broadcast path added.
backend validations pass.
frontend validations pass if touched.
closure doc exists.
perps can start next.
Final Report

Return:

files changed.
basisAmount exposure summary.
V2E-G manual verification result.
no-write proof.
docs updated.
validation commands run.
final recommendation: proceed to perps or not.