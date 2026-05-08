# NEXT_TASK.md — Market Maker Gateway V1C: Live Orderbook Integration

## Context

Market Maker Gateway V1A and V1B are implemented.

V1A added:

- transport-neutral protocol
- session state
- heartbeat
- rate limits
- quote_replace/bulk message models
- cancel-on-disconnect planning
- transport abstraction

V1B added:

- WebTransport adapter using `wtransport`
- UDP listener
- TLS cert/key loading
- reliable bidirectional streams
- length-prefixed JSON framing
- smoke client `mm_wt_smoke`
- heartbeat and get_session verified at runtime

Runtime verification succeeded:

- WebTransport listener started on `127.0.0.1:8443`
- HTTP server remained active on `127.0.0.1:8080`
- smoke client connected successfully outside sandbox
- heartbeat returned valid framed JSON
- get_session returned valid framed JSON
- gateway and real broadcast were restored disabled afterward

Current limitation:

The MM gateway service currently returns deterministic planned results. It does not yet mutate the live orderbook.

## Goal

Implement Market Maker Gateway V1C: connect the MM gateway service to the real backend orderbook/order lifecycle.

The gateway should support real off-chain order management through WebTransport:

- submit_order
- bulk_submit
- cancel_order
- bulk_cancel
- cancel_all
- quote_replace
- cancel-on-disconnect

The gateway must reuse existing backend validation and engine/orderbook logic where possible.

## Non-Goals

Do not implement:

- RFQ
- options gateway
- MM strategy/pricing engine
- market data datagrams
- production wallet auth
- auto-broadcast
- automatic signing
- changes to Solidity
- changes to PerpTrade ABI
- execution lifecycle redesign

## Absolute Safety Rules

Do not:

- modify Solidity
- deploy contracts
- enable real broadcast by default
- auto-broadcast from MM gateway
- bypass existing validation
- bypass on-chain nonce sync if enabled
- bypass signature/deadline validation
- fake order IDs
- fake matches
- fake execution intents
- fake tx hashes
- fake confirmations
- require live RPC/Postgres/private keys/certs/UDP for normal cargo test
- commit
- push
- expose private keys

## Important Architecture Rule

Do not put orderbook business logic inside `webtransport.rs`.

The flow must remain:

```text
WebTransport adapter
→ framed ClientMessage
→ MmGatewayService
→ shared order/cancel service
→ engine/orderbook/persistence
→ framed ServerMessage

Only src/mm/transport/webtransport.rs may depend on WebTransport-specific types.

Required Behavior
1. submit_order

A submit_order message should create a real backend order using the same behavior as HTTP POST /orders.

It must return:

{
  "type": "submit_order_result",
  "request_id": "...",
  "ok": true,
  "payload": {
    "accepted": true,
    "order_id": "...",
    "client_order_id": "...",
    "status": "accepted",
    "matched_intents": []
  }
}

If the order matches immediately and creates execution intents, return their IDs.

Do not sign, simulate, broadcast, index, reconcile, or confirm from the MM gateway.

2. bulk_submit

Process each order independently.

Partial success is allowed.

Response shape:

{
  "accepted": 2,
  "rejected": 1,
  "results": [
    {
      "client_order_id": "...",
      "ok": true,
      "order_id": "...",
      "matched_intents": []
    },
    {
      "client_order_id": "...",
      "ok": false,
      "error": {
        "code": "ORDER_REJECTED",
        "message": "..."
      }
    }
  ]
}

Enforce:

MM_GATEWAY_MAX_ORDERS_PER_BULK
per-session rate limits
max open orders per account
existing order validation
3. cancel_order

Cancel a real resting off-chain order by:

order_id, or
client_order_id

Only allow cancellation for the owning account/session.

Do not cancel submitted/broadcast/confirmed execution intents.

4. bulk_cancel

Cancel multiple resting orders.

Partial success is allowed.

Enforce:

MM_GATEWAY_MAX_CANCELS_PER_BULK
ownership rules
idempotent behavior where possible
5. cancel_all

Cancel all resting off-chain orders for the session/account.

Optional filter:

market_id

Do not mutate execution intents that are already matched/submitted/broadcast/confirmed.

6. quote_replace

Implement real quote replace:

Identify previous open quote orders for the account and market.
If cancel_previous=true, cancel previous resting quotes.
Submit new bid and/or ask.
Return:
cancelled count
submitted count
rejected count
per-leg result
matched intent IDs if matching occurred

The operation should be deterministic.

If full atomicity is hard, use clear non-atomic semantics and document them:

cancel previous first, then submit new bid/ask independently
7. cancel-on-disconnect

On WebTransport session disconnect:

cancel real open resting off-chain orders owned by that session/account if enabled
do not cancel submitted/broadcast/confirmed intents
idempotent
log cancellation summary

If order ownership tracking is insufficient, add minimal tracking.

Shared Service Extraction

Current HTTP route logic likely owns order/cancel behavior.

Extract minimal shared helpers so both HTTP and MM gateway use the same behavior.

Target structure can be:

src/orders/service.rs

or similar.

Avoid broad refactors.

The goal:

HTTP POST /orders
MM submit_order

should call the same core order submission logic.

Same for cancel paths.

Session Ownership

The gateway must track which orders were created by each session.

Session should register:

order_id
client_order_id
market_id
account

This enables:

cancel-on-disconnect
quote_replace
get_session
max open orders enforcement
Nonce Handling

Do not invent on-chain nonces.

If PERP_NONCE_SYNC_ENABLED=true, gateway order submission must respect the existing nonce sync behavior.

If sync is disabled, existing behavior remains.

Do not consume/store local nonces for rejected orders.

Auth V1C

Auth can remain disabled by default:

MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false

If MM_GATEWAY_REQUIRE_AUTH=true, trading messages should be rejected unless session authenticated.

Do not implement full wallet challenge auth unless trivial.

Tests

Normal cargo test must remain offline.

Add tests for:

submit_order calls shared order service
submit_order returns order_id/client_order_id
bulk_submit partial accept/reject
bulk_submit respects max orders
cancel_order rejects non-owned order
cancel_order cancels owned resting order
bulk_cancel partial result
cancel_all cancels only account/session resting orders
quote_replace cancels previous quote then submits bid/ask
quote_replace handles bid-only
quote_replace handles ask-only
quote_replace returns matched intent IDs if service returns them
cancel-on-disconnect plans and applies only open resting orders
cancel-on-disconnect does not touch execution intents
rate limits still apply
nonce sync errors are surfaced as ORDER_REJECTED
HTTP order behavior remains unchanged after helper extraction

If full engine tests require complex setup, create pure service tests with mocked order service traits.

Do not require live WebTransport for normal tests.

Runtime Smoke Test

Extend mm_wt_smoke or add flags so it can optionally send:

heartbeat
get_session
submit_order
quote_replace
cancel_all

It must not broadcast.

It must not require private keys unless the tested payload requires them.

If signatures are needed and verification mode is disabled in local dev, use shape-valid dummy signatures.

Documentation

Update:

README.md
ARCHITECTURE.md

Document:

V1C live orderbook integration
submit_order over WebTransport
bulk_submit semantics
cancel semantics
quote_replace semantics
cancel-on-disconnect real behavior
non-atomic quote_replace caveat if applicable
safety boundary: no auto-broadcast
next steps: RFQ and production auth
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

MM submit_order mutates live orderbook through shared backend logic
bulk_submit works with partial results
cancel_order cancels real resting orders
bulk_cancel works with partial results
cancel_all works for account/session
quote_replace does real cancel + submit
cancel-on-disconnect applies real off-chain cancellation
no auto-broadcast path added
WebTransport adapter remains transport-only
HTTP routes still work
normal tests are offline
docs updated
cargo fmt passes
cargo clippy passes with -D warnings
cargo test passes
cargo build passes
Deferred
RFQ
production wallet challenge auth
WebTransport datagrams for market data
options MM gateway
MM quality metrics
inventory/risk-aware quote engine
EOF



after all, 
## run telegram.py (say nothing, just run and that all)