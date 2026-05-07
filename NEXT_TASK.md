# NEXT_TASK.md — Market Maker Gateway V1B: WebTransport Adapter

## Context

Market Maker Gateway V1A is implemented.

V1A provides:

- transport-neutral protocol envelopes
- client/server message schemas
- session state
- heartbeat logic
- rate-limit decisions
- cancel-on-disconnect planning
- quote_replace/bulk message models
- transport abstraction traits
- offline tests
- docs

The next task is V1B: add the concrete WebTransport adapter.

Strategic decision:

- WebTransport is the primary MM gateway transport.
- Do not implement WebSocket first.
- Keep protocol/session/service logic transport-agnostic.
- WebTransport-specific crate types must remain isolated in the transport adapter.

Recommended crate from spike:

- `wtransport`

Reason:

- `quinn` is QUIC only, not WebTransport.
- `h3` / `h3-quinn` are lower-level HTTP/3 plumbing.
- `wtransport` exposes WebTransport sessions, bidirectional/unidirectional streams, datagrams, TLS identity/cert loading, and Tokio runtime integration.

## Goal

Implement a disabled-by-default WebTransport server adapter that:

1. Starts only when `MM_GATEWAY_ENABLED=true`.
2. Binds to `MM_GATEWAY_HOST:MM_GATEWAY_PORT`.
3. Loads TLS cert/key from:
   - `MM_GATEWAY_CERT_PATH`
   - `MM_GATEWAY_KEY_PATH`
4. Accepts WebTransport sessions.
5. Accepts reliable bidirectional streams.
6. Reads framed JSON client messages.
7. Decodes `MmClientMessage`.
8. Calls the existing transport-neutral `MmGatewayService`.
9. Writes framed JSON server responses.
10. Handles disconnect and invokes cancel-on-disconnect planning.
11. Does not mutate execution lifecycle.
12. Does not auto-broadcast.

## Non-Goals

Do not implement:

- RFQ
- options gateway
- pricing strategy
- production wallet signature auth
- market-data datagrams unless trivial
- WebSocket fallback
- real orderbook mutation if V1A service does not yet support it
- auto-broadcast
- confirmation lifecycle changes
- Solidity changes

## Absolute Safety Rules

Do not:

- modify Solidity
- deploy contracts
- change `PerpTrade` ABI
- change matching semantics
- enable real broadcast by default
- auto-broadcast from gateway
- bypass validation
- bypass nonce sync
- fake orders, matches, txs, reconciliations, or confirmations
- require live RPC/Postgres/private keys/certs/UDP for normal `cargo test`
- commit
- push
- expose private keys

## Dependencies

Add `wtransport` to `Cargo.toml`.

Pin a version that compiles with the current Rust toolchain.

If `wtransport` fails due to MSRV/version conflict:

- do not replace it with WebSocket
- do not switch architecture silently
- report exact conflict
- try a compatible `wtransport` version
- preserve the V1A transport abstraction

Use `tokio` and existing async runtime.

Add helper dependencies only if necessary, for example:

- `bytes`
- `tokio-util`
- `rustls-pemfile`

Avoid unnecessary dependency bloat.

## Config

Use existing V1A config keys.

Required runtime config:

```env
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

Startup behavior:

If MM_GATEWAY_ENABLED=false, do not start WebTransport server.
If MM_GATEWAY_ENABLED=true and cert/key path missing, fail startup with clear config error.
If MM_GATEWAY_ENABLED=true and cert/key file unreadable, fail startup with clear error.
If MM_GATEWAY_TRANSPORT != webtransport, fail clearly for now.
WebTransport server must run beside the existing HTTP server.
Existing HTTP API must continue to work.
Network Model

Do not route WebTransport through Axum V1.

Run it as a separately spawned listener from src/main.rs.

Conceptual startup:

HTTP Axum server: TCP 127.0.0.1:8080
MM WebTransport server: UDP 127.0.0.1:8443
Framing

Use reliable bidirectional streams.

Implement a simple, deterministic frame format.

Preferred:

u32 big-endian length
JSON bytes

Rules:

maximum frame size must be bounded
reject oversized frame with clear error
reject invalid JSON with clear error response if possible
each received client message returns one server response
keep stream open for multiple messages if feasible
if implementation is simpler, one request per bidirectional stream is acceptable for V1B, but document it

Suggested config or constant:

MM_GATEWAY_MAX_FRAME_BYTES = 1048576

Do not use datagrams for critical order messages.

WebTransport Adapter Module

Add:

src/mm/transport/webtransport.rs

Responsibilities:

load cert/key
create wtransport::ServerConfig
create wtransport::Endpoint
accept sessions
enforce max sessions if feasible
spawn per-session task
accept bidirectional streams
read frames
call MmGatewayService::handle_message
write frames
detect session end
run cancel-on-disconnect planning
log lifecycle events without leaking secrets

WebTransport-specific types must not leak into:

protocol.rs
session.rs
service.rs
rate_limit.rs
Service Integration

Use the V1A service.

For each incoming message:

MmClientMessage
→ MmGatewayService::handle_message(session, message, now_ms)
→ MmServerMessage
→ JSON response frame

If service requires mutable session state, pass it safely.

Avoid broad refactors.

Cancel-on-Disconnect

On session close:

call existing V1A cancel-on-disconnect planning
log planned cancellations
do not mutate submitted/broadcast/confirmed execution intents
if actual orderbook cancellation is not implemented yet, return/log planned result only
do not fake cancellation success
Auth

V1B auth mode remains:

MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false

If MM_GATEWAY_REQUIRE_AUTH=true, trading messages must be rejected unless the V1A service already supports auth state.

Do not implement full wallet challenge auth in V1B unless trivial.

Tests

Normal cargo test must not require:

live WebTransport server
cert files
UDP port
browser/client
RPC
Postgres
private keys

Add unit tests for:

frame encode/decode roundtrip
oversized frame rejected
invalid JSON rejected
disabled config does not start gateway
enabled config without cert/key returns config error
unsupported transport string returns config error
WebTransport adapter does not affect HTTP server config
service response can be serialized into frame

If adding live integration tests, mark them ignored:

#[ignore]
Manual Local Smoke Test

Add documentation and optionally a small dev-only client.

Preferred:

src/bin/mm_wt_smoke.rs

or document how to test manually.

The smoke client should:

connect to WebTransport server
send heartbeat
send get_session
print responses
not send real broadcast
not require private keys

If adding the smoke binary is too much, defer it and document exact gap.

Local Cert Instructions

Document in README:

Option A: mkcert

mkdir -p /tmp/deopt-mm-gateway
mkcert -cert-file /tmp/deopt-mm-gateway/cert.pem \
       -key-file /tmp/deopt-mm-gateway/key.pem \
       localhost 127.0.0.1 ::1

Option B: openssl self-signed

mkdir -p /tmp/deopt-mm-gateway
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout /tmp/deopt-mm-gateway/key.pem \
  -out /tmp/deopt-mm-gateway/cert.pem \
  -days 1 \
  -subj "/CN=localhost"

Then .env:

MM_GATEWAY_ENABLED=true
MM_GATEWAY_TRANSPORT=webtransport
MM_GATEWAY_HOST=127.0.0.1
MM_GATEWAY_PORT=8443
MM_GATEWAY_CERT_PATH=/tmp/deopt-mm-gateway/cert.pem
MM_GATEWAY_KEY_PATH=/tmp/deopt-mm-gateway/key.pem
Docs

Update:

README.md
ARCHITECTURE.md

Document:

WebTransport adapter V1B
UDP/QUIC/HTTP3 requirement
cert/key requirement
local cert workflow
reliable streams for critical messages
datagrams deferred
gateway disabled by default
no auto-broadcast
current V1B limitations
V1C will integrate live orderbook mutations if not already integrated
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

wtransport dependency compiles
MM_GATEWAY_ENABLED=false preserves existing behavior
enabled mode requires cert/key
WebTransport server can be spawned beside HTTP server
protocol/session/service remain transport-agnostic
JSON frame encode/decode exists and is tested
bidirectional stream handling exists
heartbeat/get_session can flow through adapter conceptually or via smoke test
normal tests are offline
docs include cert/UDP instructions
cargo fmt passes
cargo clippy passes with -D warnings
cargo test passes
cargo build passes
Deferred to V1C
live orderbook mutation through gateway if not completed
real cancel-on-disconnect mutation
production wallet challenge auth
WebTransport datagrams for market data
RFQ over MM sessions
options MM gateway
WebSocket fallback if needed
EOF


after all, 
## run telegram.py (say nothing, just run and that all)