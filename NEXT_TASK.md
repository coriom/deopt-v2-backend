# NEXT_TASK.md — Production MM Auth V1A: Wallet Challenge Sessions

## Context

The backend now supports:

- WebTransport MM Gateway
- MM session registry
- perp RFQ over WebTransport
- signed perp RFQ quotes
- option RFQ over WebTransport
- signed option RFQ quotes
- read-only admin monitoring
- frontend admin dashboard

Current weakness:

MM Gateway has been runtime-tested with auth disabled. Quotes can be signed, but the transport session itself is not cryptographically bound to a wallet/account in production mode.

## Goal

Implement Production MM Auth V1A.

A WebTransport MM session must authenticate by signing a server-issued challenge.

After authentication:

- session is bound to a wallet address / mm_account
- gateway messages requiring an account must match the authenticated account
- admin session snapshots show authenticated/account state
- unauthenticated sessions can only call heartbeat/get_session/auth messages
- existing disabled/dev auth mode remains available for local tests

## Non-Goals

Do not implement:

- frontend auth UI
- admin write controls
- MM permissions per market/series
- API keys
- OAuth
- production user auth
- Solidity changes
- deployments
- auto-broadcast
- trading logic changes

## Safety Rules

Do not:

- modify Solidity
- deploy contracts
- enable real broadcast by default
- expose private keys
- log challenge signatures as secrets if avoidable
- break existing disabled auth tests
- require live RPC/Postgres/WebTransport/private keys for normal cargo test
- commit
- push

## Auth Modes

Extend current MM gateway auth config.

Supported modes:

```text
disabled
wallet_challenge

If current config already has auth mode strings, extend safely.

Config:

MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false
MM_GATEWAY_CHALLENGE_TTL_MS=60000

Behavior:

disabled
preserves current local/dev behavior
existing tests keep passing
wallet_challenge
session starts unauthenticated
session can request challenge
server returns challenge payload
client signs challenge
server verifies signature
session becomes authenticated and bound to recovered address

If MM_GATEWAY_REQUIRE_AUTH=true:

order/quote/RFQ messages require authenticated session
account fields must equal authenticated account
Protocol Messages

Add client messages:

{
  "type": "auth_challenge",
  "request_id": "auth-1",
  "payload": {
    "account": "0x..."
  }
}

Server response:

{
  "type": "auth_challenge_result",
  "request_id": "auth-1",
  "ok": true,
  "payload": {
    "session_id": "...",
    "account": "0x...",
    "challenge": "...",
    "issued_at_ms": 1770000000000,
    "expires_at_ms": 1770000060000
  }
}

Add client message:

{
  "type": "auth_verify",
  "request_id": "auth-2",
  "payload": {
    "account": "0x...",
    "signature": "0x..."
  }
}

Server response:

{
  "type": "auth_verify_result",
  "request_id": "auth-2",
  "ok": true,
  "payload": {
    "session_id": "...",
    "authenticated": true,
    "account": "0x..."
  }
}
Challenge Format

Use a deterministic human-readable challenge string.

Suggested:

DeOpt v2 MM Gateway Authentication

session_id: <session_id>
account: <account>
chain_id: <chain_id>
issued_at_ms: <issued_at_ms>
expires_at_ms: <expires_at_ms>
nonce: <random_nonce>

Hash/signing approach:

Use Ethereum personal sign style if existing helpers support it.
Or use EIP-712 if existing signing helpers make it easy.
Prefer the simplest reliable path consistent with existing signature recovery utilities.

Document exact signing format.

Session State

Extend MM session with:

auth_mode
authenticated
account
challenge_nonce
challenge_issued_at_ms
challenge_expires_at_ms

Do not expose raw signatures in session snapshots.

Message Enforcement

When auth required:

Allow unauthenticated:

heartbeat
get_session
auth_challenge
auth_verify

Reject unauthenticated:

submit_order
bulk_submit
cancel_order
bulk_cancel
cancel_all
quote_replace
rfq_quote
option_rfq_quote

Account-bound checks:

submit_order.account must equal session.account
cancel account if present must match session.account
rfq_quote.mm_account must equal session.account
option_rfq_quote.mm_account must equal session.account

Use case-insensitive address comparison.

Admin Impact

Update /admin/mm/sessions to show:

authenticated
account
auth_mode
challenge_active boolean
challenge_expires_at_ms optional

Do not expose challenge string or signatures.

Smoke Client

Extend mm_wt_smoke.rs with optional auth mode if useful:

MM_PRIVATE_KEY=0x... cargo run --bin mm_wt_smoke -- auth

or integrate into existing RFQ smoke modes if auth env is enabled.

No private key should be printed.

Tests

Normal cargo test must remain offline.

Add tests for:

auth challenge creation
challenge contains expected account/session/expiry
auth verify accepts valid signature
auth verify rejects missing challenge
auth verify rejects expired challenge
auth verify rejects signer mismatch
require_auth rejects unauthenticated submit_order
require_auth rejects unauthenticated rfq_quote
require_auth rejects unauthenticated option_rfq_quote
authenticated submit_order account must match session account
authenticated rfq_quote mm_account must match session account
authenticated option_rfq_quote mm_account must match session account
disabled mode preserves old behavior
admin session snapshot shows authenticated/account but not challenge/signature
existing MM/RFQ/options tests still pass
Documentation

Update:

README.md
ARCHITECTURE.md
.env.example

Document:

auth modes
challenge flow
signing format
local dev disabled mode
production wallet_challenge mode
account binding
admin session visibility
future MM permissions
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

wallet challenge auth mode exists
challenge/verify messages exist
valid signatures authenticate session
invalid/expired/mismatch signatures reject
required-auth blocks unauthenticated quote/order messages
authenticated account binding enforced
disabled mode still works
admin sessions show sanitized auth state
normal tests offline
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
MM permissions per market/option_series
admin write controls for allowlists
frontend MM auth UI
API key auth
production user auth
EOF



after all, 
## run telegram.py (say nothing, just run and that all)