# NEXT_TASK.md — EIP-712 Order Model and Signature Verification

## Context

The Rust trading backend scaffold is implemented and validated.

Current status:
- deterministic in-memory orderbook works
- matching works
- execution intents are created
- API accepts financial fixed-point values as strings
- API returns financial fixed-point values as strings
- cargo fmt: OK
- cargo clippy: OK
- cargo test: OK
- cargo build: OK

## Goal

Add the first security layer for off-chain orders:

- signed order model
- EIP-712-compatible order schema
- signature verification scaffold/implementation
- nonce validation
- deadline validation

This phase must not execute transactions on-chain.

## Scope

Add signed order support at the API boundary.

The engine should only accept orders after:
- payload is well-formed
- fixed-point strings are valid
- deadline has not expired
- nonce has not already been used
- signature is valid, if signature verification is implemented

If complete EIP-712 verification is too large for one pass, implement:
- exact typed order data model
- deterministic hash/signature module boundary
- strict TODO with tests for the non-cryptographic validation paths

But prefer implementing actual signature recovery if feasible in Rust.

## Required Order Fields

Extend external order input to include:

- `account`
- `market_id`
- `side`
- `price_1e8`
- `size_1e8`
- `time_in_force`
- `reduce_only`
- `post_only`
- `client_order_id`
- `nonce`
- `deadline_ms`
- `signature`

Keep `price_1e8` and `size_1e8` as strings in external JSON.

## Domain Requirements

Add a signed-order model separate from internal engine order.

Suggested modules:

```text
src/signing/mod.rs
src/signing/eip712.rs
src/signing/signature.rs
src/signing/nonce.rs
Validation Rules

Reject:

expired deadline
zero nonce if you decide nonce must be nonzero
reused nonce for same account
malformed signature
invalid signer/account mismatch
invalid fixed-point values
unknown market

Nonce scope:

nonce uniqueness should be per account.

In-memory nonce store is acceptable for this phase.

API Requirements

POST /orders should support signed order input.

For local development/testing, allow a config flag:

SIGNATURE_VERIFICATION_MODE=disabled

Accepted values:

disabled
strict

Behavior:

disabled: still validate nonce/deadline, but skip cryptographic signature recovery
strict: enforce signature verification

Default should be disabled for this phase unless actual signature verification is fully implemented.

Do not fake strict verification.

Tests Required

Add tests for:

accepts valid order in disabled signature mode
rejects expired deadline
rejects reused nonce for same account
allows same nonce for different accounts
rejects malformed signature in strict mode if strict is implemented
rejects signer/account mismatch if strict is implemented
existing matching tests still pass
existing API numeric string tests still pass
Constraints

Do not add:

blockchain RPC
transaction sending
private key loading
database
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deoptv2

Do not introduce:

floating point financial math
fake on-chain execution
fake finality
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

The task is complete only if:

signed order API model exists
nonce/deadline validation exists
signature mode config exists
disabled mode works for local testing
strict mode is either correctly implemented or clearly rejected/not enabled
all tests pass
no blockchain execution is added
EOF