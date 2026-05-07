
# NEXT_TASK.md — Market Maker Gateway V1A: Transport-Agnostic Protocol & Service Layer

## Context

The backend already supports:

- HTTP order intake
- orderbook matching
- execution intents
- PerpTrade signing payloads
- buyer/seller PerpTrade signatures
- calldata builder for the intentId `PerpMatchingEngine`
- RPC simulation
- guarded real broadcast
- indexer for `TradeExecuted`
- reconciliation by `onchain_intent_id`
- confirmation/finality
- on-chain perp nonce sync

A full Base Sepolia execution lifecycle has already succeeded:

```text
orderbook
→ execution_intent
→ PerpTrade signatures
→ simulation_ok
→ real broadcast
→ TradeExecuted indexed
→ reconciliation matched
→ confirmation confirmed=true

The next major product block is the Market Maker Gateway.

Strategic decision:

Use WebTransport as the primary future MM transport.
Do not implement WebSocket first.
However, do not couple market maker business logic to WebTransport crate types.
First implement a transport-agnostic protocol/session/service layer.
WebTransport adapter will be implemented in the next task.
Why This Task Is Split

A read-only technical spike recommended wtransport as the WebTransport adapter crate because:

quinn is QUIC only, not WebTransport.
h3 / h3-quinn are lower-level HTTP/3 plumbing.
wtransport is the right abstraction level for WebTransport sessions, reliable streams, unidirectional streams, datagrams, TLS identity/cert handling, and Tokio runtime integration.

However, WebTransport has extra operational complexity:

UDP port
HTTP/3 / QUIC server
TLS certificate/key
local cert trust
client tooling

Therefore V1 is split:

V1A: transport-agnostic MM protocol/session/service layer.
V1B: WebTransport adapter using wtransport.

This task is V1A only.

Goal

Implement the transport-agnostic Market Maker Gateway core.

This means:

protocol message schema
response/error envelope
session state
heartbeat logic
rate-limit decisions
cancel-on-disconnect planning
bulk order request/response models
quote replace request/response models
shared service boundary for submit/cancel/quote operations
no live WebTransport server yet
Non-Goals

Do not implement in this task:

WebTransport server
WebSocket server
RFQ
options gateway
strategy engine
market maker pricing logic
market-data datagrams
production wallet challenge auth
real broadcast from the gateway
Solidity changes
execution lifecycle changes
Absolute Safety Rules

Do not:

modify Solidity
deploy contracts
change PerpTrade ABI
change matching semantics
enable real broadcast by default
auto-broadcast from MM gateway
bypass existing order validation
bypass nonce sync
bypass signature/deadline validation
fake orders, matches, txs, reconciliations, or confirmations
require RPC/Postgres/private keys/certs for normal cargo test
commit
push
expose private keys

Normal cargo test must remain offline.

Required Module Structure

Add or extend:

src/mm/
  mod.rs
  gateway.rs          # keep existing scaffold or delegate/re-export
  protocol.rs         # message schemas and envelopes
  session.rs          # session state and heartbeat/disconnect logic
  rate_limit.rs       # pure rate-limit decisions
  service.rs          # transport-neutral command service boundary
  transport/
    mod.rs            # adapter traits only, no concrete WebTransport yet

Do not add webtransport.rs in this task unless it is an empty placeholder with no external dependency. Prefer deferring it to V1B.

Protocol Envelope

All client messages must have this shape:

{
  "type": "...",
  "request_id": "...",
  "payload": {}
}

All success responses must have this shape:

{
  "type": "..._result",
  "request_id": "...",
  "ok": true,
  "payload": {}
}

All error responses must have this shape:

{
  "type": "error",
  "request_id": "...",
  "ok": false,
  "error": {
    "code": "...",
    "message": "..."
  }
}

Use serde / serde_json.

Client Message Types V1A

Define protocol models for:

auth
heartbeat
submit_order
bulk_submit
cancel_order
bulk_cancel
cancel_all
quote_replace
get_session

Even if service handlers are only partially wired in V1A, the message types and response types must exist.

Server Response Types V1A

Define response payloads for:

auth_result
heartbeat_result
submit_order_result
bulk_submit_result
cancel_order_result
bulk_cancel_result
cancel_all_result
quote_replace_result
get_session_result
error
Error Codes

Define stable string error codes, for example:

BAD_REQUEST
UNKNOWN_MESSAGE_TYPE
AUTH_REQUIRED
AUTH_FAILED
RATE_LIMITED
TOO_MANY_ORDERS
TOO_MANY_CANCELS
SESSION_CLOSED
ORDER_REJECTED
CANCEL_REJECTED
QUOTE_REPLACE_FAILED
INTERNAL_ERROR

Avoid leaking internal debug data in user-facing error messages.

Session State

Add a transport-neutral MM session model.

Required fields:

session_id
connection_id
account optional
authenticated bool
auth_mode
connected_at_ms
last_heartbeat_at_ms
cancel_on_disconnect
open_client_order_ids
messages_in_current_window
window_started_at_ms
in_flight_requests

Types can be Rust-specific, but must be serializable for get_session.

Session Behavior

Implement pure/session-level helpers:

create new session
bind account after auth
update heartbeat timestamp
detect heartbeat timeout
register open order id
unregister open order id
plan cancel-on-disconnect
increment/decrement in-flight count
produce public session snapshot

Do not require network tests.

Auth V1A

Implement protocol shape only.

Supported config/design:

MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false

In disabled mode:

session can process messages without account-bound auth if the message payload contains account where required
get_session should show auth_mode=disabled

Do not implement wallet signature challenge yet unless trivial. Leave TODO for V2.

Rate Limits

Add pure rate-limit decision logic.

Config fields should exist in .env.example and config parsing if appropriate:

MM_GATEWAY_ENABLED=false
MM_GATEWAY_TRANSPORT=webtransport
MM_GATEWAY_HOST=127.0.0.1
MM_GATEWAY_PORT=8443
MM_GATEWAY_CERT_PATH=
MM_GATEWAY_KEY_PATH=
MM_GATEWAY_MAX_SESSIONS=100
MM_GATEWAY_MAX_IN_FLIGHT_PER_SESSION=128
MM_GATEWAY_RATE_LIMIT_PER_SEC=100
MM_GATEWAY_HEARTBEAT_TIMEOUT_MS=15000
MM_GATEWAY_MAX_ORDERS_PER_BULK=50
MM_GATEWAY_MAX_CANCELS_PER_BULK=100
MM_GATEWAY_MAX_OPEN_ORDERS_PER_ACCOUNT=500
MM_GATEWAY_CANCEL_ON_DISCONNECT=true
MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false

In this V1A task:

config can be parsed and stored
no server should start yet
cert/key validation may be deferred to V1B when MM_GATEWAY_ENABLED=true

Rate-limit helpers must cover:

max messages per second
max in-flight per session
max orders per bulk
max cancels per bulk
max open orders per account
Service Boundary

Create a transport-neutral service layer that defines the intent of operations.

Preferred shape:

MmGatewayService
  handle_message(session, message) -> response

or equivalent.

The service should delegate or prepare delegation to existing backend logic, but avoid broad refactors.

For V1A:

heartbeat should fully work.
get_session should fully work.
bulk_submit should validate bulk size and produce structured partial result.
bulk_cancel should validate bulk size and produce structured partial result.
quote_replace should parse/plan:
optional cancel previous quote ids
optional bid
optional ask
return deterministic result structure
If full integration with existing orderbook requires broader refactor, return explicit ORDER_REJECTED / NOT_IMPLEMENTED for specific operations and document V1B/V1C integration.

But prefer extracting small reusable helpers from HTTP order/cancel path if safe and minimal.

Integration Guidance

Existing relevant code paths:

src/api/routes.rs       # HTTP submit/cancel behavior
src/engine/state.rs     # engine command boundary
src/db/repository.rs    # persistence
src/nonce_sync/mod.rs   # on-chain perp nonce sync

Do not call Axum HTTP handlers directly from MM service.

If shared order/cancel behavior is needed:

extract small pure/helper functions
keep HTTP behavior unchanged
add tests proving HTTP behavior still works
Transport Trait

Define a transport abstraction in src/mm/transport/mod.rs.

Example conceptual responsibilities:

MmTransportSession:
  session_id()
  send(server_message)
  close(reason)

MmTransportAdapter:
  start(service)

Keep the trait simple. It should not mention WebTransport.

Do not use concrete WebTransport types in protocol/session/service modules.

Cancel-on-Disconnect Planning

Implement planning only in V1A.

Function should return which open client order IDs should be cancelled when session disconnects.

Rules:

only open resting off-chain orders belonging to the session/account
do not touch execution intents
do not touch submitted txs
do not touch confirmed txs
idempotent result

Actual orderbook mutation can be deferred if needed.

Quote Replace Semantics

Define request payload:

{
  "market_id": 1,
  "account": "0x...",
  "cancel_previous": true,
  "bid": {
    "price_1e8": "299900000000",
    "size_1e8": "100000000",
    "client_order_id": "eth-bid-001",
    "nonce": 1,
    "signature": "0x..."
  },
  "ask": {
    "price_1e8": "300100000000",
    "size_1e8": "100000000",
    "client_order_id": "eth-ask-001",
    "nonce": 2,
    "signature": "0x..."
  }
}

Define response payload:

{
  "market_id": 1,
  "cancelled": 2,
  "submitted": 2,
  "rejected": 0,
  "matched_intents": []
}

V1A can return deterministic planned results if full orderbook integration is deferred.

Bulk Behavior

Bulk submit/cancel should be partial-accept capable.

Response should include per-item results:

{
  "accepted": 2,
  "rejected": 1,
  "results": [
    {
      "client_order_id": "x",
      "ok": true,
      "order_id": "..."
    },
    {
      "client_order_id": "y",
      "ok": false,
      "error": {
        "code": "ORDER_REJECTED",
        "message": "..."
      }
    }
  ]
}
Tests

Add tests under:

tests/mm_gateway_tests.rs

or module-level unit tests.

Required tests:

parse valid heartbeat message
reject unknown message type
format success response envelope
format error response envelope
heartbeat updates session timestamp
heartbeat timeout decision
rate limit allows under threshold
rate limit rejects over threshold
max orders per bulk enforced
max cancels per bulk enforced
max open orders per account enforced
cancel-on-disconnect plan returns only open session orders
cancel-on-disconnect is idempotent
quote_replace parses bid-only
quote_replace parses ask-only
quote_replace parses bid+ask
bulk submit partial result shape
get_session returns public session snapshot
disabled auth mode allows dev session
require-auth mode rejects trading message before auth

Tests must not require:

live WebTransport
cert files
UDP port
RPC
Postgres
private keys
Base Sepolia
Documentation

Update:

README.md
ARCHITECTURE.md

Document:

purpose of Market Maker Gateway
why WebTransport is the strategic transport
why V1A is transport-agnostic
planned V1B WebTransport adapter using wtransport
MM message envelope
session model
heartbeat
cancel-on-disconnect
quote_replace
bulk behavior
rate limits
safety boundaries
no auto-broadcast in gateway V1
future work:
WebTransport adapter
datagrams for market data
wallet challenge auth
RFQ
options support
Cargo

Do not add wtransport dependency in V1A unless needed.

Dependency addition should happen in V1B.

Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

This task is complete only if:

MM protocol message types exist
response/error envelope exists
session state exists
heartbeat logic exists
rate-limit logic exists
cancel-on-disconnect planning exists
quote_replace request/response models exist
bulk partial result models exist
transport trait exists and is WebTransport-agnostic
config keys exist with safe defaults
README/ARCHITECTURE document the V1A/V1B split
normal tests are offline
cargo fmt passes
cargo clippy passes with -D warnings
cargo test passes
cargo build passes
Deferred to NEXT_TASK V1B
Add wtransport dependency
Start dedicated WebTransport UDP listener
TLS cert/key loading
JSON framing over reliable bidirectional streams
local WebTransport smoke client
optional datagram support
runtime cert/UDP documentation
EOF



after all, 
## run telegram.py (say nothing, just run and that all)