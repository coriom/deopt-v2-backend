# NEXT_TASK.md — RFQ V1B Runtime Verification

## Context

RFQ V1B has been implemented.

Implemented features:

- RFQ protocol messages:
  - `rfq_quote`
  - `rfq_request`
  - `rfq_quote_result`
  - `rfq_quote_accepted`
  - `rfq_quote_rejected`
  - `rfq_expired`
- MM session registry
- RFQ creation broadcasts `rfq_request` to active MM sessions
- WebTransport MM can submit `rfq_quote`
- RFQ quotes are persisted through the RFQ service/repository
- Accepted quote creates a normal pending execution_intent
- Accepted quote notifies MM session best-effort
- Competing active session quotes get best-effort rejection
- No auto-broadcast
- No Solidity changes
- No execution lifecycle changes

Validation already passed:

```text
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

Now runtime behavior must be verified.

Goal

Runtime-verify the complete RFQ V1B flow over real WebTransport.

Prove:

HTTP taker creates RFQ
→ connected MM receives rfq_request over WebTransport
→ MM sends rfq_quote over WebTransport
→ HTTP quote listing shows MM quote
→ HTTP accept quote creates execution_intent
→ MM receives rfq_quote_accepted notification
Non-Goals

Do not implement:

RFQ V1C
signed RFQ quotes
options RFQ
multi-leg RFQ
MM ranking
production auth
market-data datagrams
auto-signing
auto-simulation
auto-broadcast
Safety Rules

Do not:

modify Solidity
deploy contracts
change PerpTrade ABI
change execution lifecycle
enable real broadcast
call /executor/broadcast
expose private keys
fake RFQ success
fake quote rows
fake execution intents
fake notifications
commit
push

If a real code bug is found:

apply minimal patch only
run full validation:
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Runtime Setup

Use local WebTransport certs under:

/tmp/deopt-mm-gateway/cert.pem
/tmp/deopt-mm-gateway/key.pem

Use ECDSA P-256 cert generation because previous runtime smoke showed RSA cert failed WebTransport client verification.

Generate if missing:

mkdir -p /tmp/deopt-mm-gateway

openssl ecparam -name prime256v1 -genkey -noout \
  -out /tmp/deopt-mm-gateway/key.pem

openssl req -new -x509 \
  -key /tmp/deopt-mm-gateway/key.pem \
  -out /tmp/deopt-mm-gateway/cert.pem \
  -days 1 \
  -subj "/CN=localhost"

Temporarily set .env:

RFQ_ENABLED=true

MM_GATEWAY_ENABLED=true
MM_GATEWAY_TRANSPORT=webtransport
MM_GATEWAY_HOST=127.0.0.1
MM_GATEWAY_PORT=8443
MM_GATEWAY_CERT_PATH=/tmp/deopt-mm-gateway/cert.pem
MM_GATEWAY_KEY_PATH=/tmp/deopt-mm-gateway/key.pem
MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false

EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false

After the test, restore safe state:

MM_GATEWAY_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
EXECUTION_ENABLED=false
Runtime Test Steps
1. Start backend

Ensure no old backend is running:

pkill -f deopt-v2-backend || true

Start backend:

cargo run --bin deopt-v2-backend

Verify:

curl http://127.0.0.1:8080/health

Expected:

{"ok":true,"service":"deopt-v2-backend"}
2. Start MM WebTransport smoke client

Use src/bin/mm_wt_smoke.rs.

Extend it minimally if needed so it can:

connect
heartbeat
get_session
listen for rfq_request
send rfq_quote
listen for rfq_quote_accepted

Do not require private keys.

Do not broadcast.

Use:

MM_WT_URL=https://127.0.0.1:8443/mm \
MM_WT_CERT_PATH=/tmp/deopt-mm-gateway/cert.pem \
cargo run --bin mm_wt_smoke -- rfq

If the current binary uses different flags, adapt minimally and document final command.

3. Create RFQ through HTTP

Use harmless test accounts.

Example:

TAKER=0xc0A76c2A6c6b70C0B065A05E64417886416cc976

curl -X POST http://127.0.0.1:8080/rfqs \
  -H "Content-Type: application/json" \
  -d "{
    \"taker\": \"$TAKER\",
    \"market_id\": 1,
    \"side\": \"buy\",
    \"size_1e8\": \"100000000\",
    \"limit_price_1e8\": \"305000000000\",
    \"ttl_ms\": 30000
  }"

Expected:

RFQ status open
rfq_id returned
MM smoke client receives rfq_request
4. MM sends RFQ quote over WebTransport

Smoke client should send something equivalent to:

{
  "type": "rfq_quote",
  "request_id": "smoke-rfq-quote-1",
  "payload": {
    "rfq_id": "<RFQ_ID>",
    "mm_account": "0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3",
    "price_1e8": "300100000000",
    "size_1e8": "100000000",
    "client_quote_id": "smoke-mm-rfq-quote-1",
    "quote_ttl_ms": 10000
  }
}

Expected response:

{
  "type": "rfq_quote_result",
  "ok": true,
  "payload": {
    "quote_id": "...",
    "status": "active"
  }
}
5. Verify quote via HTTP
curl http://127.0.0.1:8080/rfqs/$RFQ_ID/quotes

Expected:

quote is listed
quote has session_id
quote status active
mm_account matches smoke MM
6. Accept quote via HTTP
curl -X POST http://127.0.0.1:8080/rfqs/$RFQ_ID/accept/$QUOTE_ID

Expected:

RFQ status accepted
quote status accepted
execution_intent_id returned
onchain_intent_id returned if endpoint exposes it
no broadcast occurred
7. Verify execution intent exists
curl http://127.0.0.1:8080/execution-intents

or direct endpoint if available.

Expected:

new execution intent exists
status pending
buyer/seller mapping correct:
taker buy => buyer=taker, seller=mm_account, buyer_is_maker=false
8. Verify MM notification

Smoke client should receive:

{
  "type": "rfq_quote_accepted",
  "payload": {
    "rfq_id": "...",
    "quote_id": "...",
    "execution_intent_id": "..."
  }
}

If notification fails but acceptance succeeds, report the warning. Do not fake notification.

9. Verify no forbidden mutation

Confirm:

no real broadcast
no tx_hash created by RFQ accept
no confirmation created
no Solidity interaction required
10. Stop backend and restore .env

Restore safe state:

MM_GATEWAY_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
EXECUTION_ENABLED=false

Stop backend.

If Runtime Fails

Diagnose precisely:

RFQ disabled config
MM gateway disabled config
cert issue
UDP/QUIC issue
smoke client issue
server-initiated message delivery issue
session registry issue
rfq_quote parsing issue
RFQ service validation issue
persistence issue
execution_intent creation issue
notification issue

If a real bug is found, patch minimally and rerun validation.

Validation

If any code was changed, run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Final Report Required

Return:

Files changed.
Whether code patch was needed.
Cert generation method.
Backend startup result.
HTTP health result.
WebTransport connection result.
RFQ creation result.
Whether MM received rfq_request.
rfq_quote submission result.
HTTP quote listing result.
RFQ accept result.
Execution intent created:
intent_id
onchain_intent_id if available
buyer
seller
buyer_is_maker
Whether MM received rfq_quote_accepted.
Verification that no broadcast happened.
Final .env safety state.
Validation commands run.
Remaining blocker, if any.
EOF



after all, 
## run telegram.py (say nothing, just run and that all)