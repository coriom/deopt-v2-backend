# NEXT_TASK.md — PerpMatchingEngine Calldata Builder V1
# NEXT_TASK.md — Matched PerpTrade Signature Flow V1

## Context

The backend now has:
- deterministic matching
- strict EIP-712 order verification
- PostgreSQL persistence
- dry-run executor scaffold
- real ABI calldata builder for PerpMatchingEngine.executeTrade

Important Solidity constraint:
- PerpMatchingEngine does not verify order signatures.
- It verifies buyerSig and sellerSig over the final matched PerpTrade.
- Therefore, existing signed order signatures are not sufficient for execution.

## Goal

Add a signature collection flow for matched PerpTrade execution.

The backend must:
- expose the final PerpTrade payload that buyer and seller must sign
- accept buyer/seller trade signatures
- validate signature shape
- persist signatures
- mark an execution intent as calldata-ready when both signatures are present
- build real calldata only when both signatures are present
- still not broadcast transactions

## Scope

Implement:
- signing payload endpoint
- signature submission endpoint
- persistence for trade signatures
- calldata-ready status/metadata
- dry-run executor awareness of missing/present signatures

Do not implement:
- RPC simulation
- transaction signing
- transaction broadcast
- private key loading
- indexer
- Solidity changes

## Required API

Add:

```text
GET /execution-intents/:intent_id/signing-payload
POST /execution-intents/:intent_id/signatures
GET /execution-intents/:intent_id/signing-payload

Returns the PerpTrade payload to sign.

Response should include:

intent_id
eip712 domain:
name: DeOptV2-PerpMatchingEngine
version: 1
chainId
verifyingContract
type name: PerpTrade
type fields
message:
buyer
seller
marketId
sizeDelta1e8
executionPrice1e8
buyerIsMaker
buyerNonce
sellerNonce
deadline
digest if available from backend implementation

Important:

Do not invent buyerNonce/sellerNonce silently.
If the backend does not yet know buyerNonce/sellerNonce/deadline, return a clear error or require them to be stored with the intent.

Preferred:

extend execution intent metadata to include:
buyer_nonce
seller_nonce
deadline
buyer_is_maker

If current ExecutionIntent lacks these fields, add a minimal backwards-compatible execution metadata table or nullable columns.

POST /execution-intents/:intent_id/signatures

Request:

{
  "buyer_sig": "0x...",
  "seller_sig": "0x..."
}

Rules:

allow submitting both at once
optionally allow one side at a time if simple
validate 0x + 65-byte hex
persist signatures
do not fake cryptographic verification unless implemented
after both signatures exist, builder can produce calldata

Response:

intent_id
buyer_signature_present
seller_signature_present
calldata_ready
missing_signatures
Persistence Requirements

Add table or columns for trade signatures.

Preferred table:

CREATE TABLE execution_intent_signatures (
    intent_id TEXT PRIMARY KEY REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    buyer_sig TEXT,
    seller_sig TEXT,
    updated_at_ms BIGINT NOT NULL
);

If adding columns to execution_intents is cleaner, acceptable.

Need migration:

migrations/0002_execution_intent_signatures.sql

Normal tests must not require Postgres.

PerpTrade Payload Requirements

A PerpTrade requires:

buyer
seller
marketId
sizeDelta1e8
executionPrice1e8
buyerIsMaker
buyerNonce
sellerNonce
deadline

ExecutionIntent currently has:

buyer
seller
market_id
price_1e8
size_1e8
buy_order_id
sell_order_id

Missing:

buyerIsMaker
buyerNonce
sellerNonce
deadline

For this phase:

derive buyerIsMaker from maker/taker if available, or store it explicitly when intent is created
derive buyerNonce/sellerNonce from original signed orders if available, or store them explicitly when intent is created
set deadline from a clear execution deadline policy or original order deadlines if available

Do not silently use zero deadline/nonces unless documented and tested.

Builder Integration

Update the tx builder:

if signatures missing: return preview with missing_signatures=true
if signatures present: build real calldata
still is_broadcastable=false
Tests Required

Add tests for:

signing payload endpoint returns expected PerpTrade fields
missing nonce/deadline metadata returns clear error if not implemented
signature endpoint rejects malformed buyer_sig
signature endpoint rejects malformed seller_sig
submitting both signatures marks calldata_ready
tx builder builds calldata only when both signatures exist
missing signatures produce non-executable preview
existing tests still pass

No normal test may require:

Postgres
RPC
private keys
Base Sepolia
Documentation

Update README.md and ARCHITECTURE.md:

order signatures vs PerpTrade signatures
why second signature flow exists
endpoint usage
no broadcast yet
next phase is RPC simulation
Constraints

Do not add:

transaction broadcast
private key loading
RPC simulation
indexer
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deoptv2

Do not fake:

signatures
calldata readiness
on-chain confirmation
broadcast
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

signing payload endpoint exists
signature submission endpoint exists
signatures can be persisted or tracked
calldata readiness depends on both signatures
no broadcast exists
all tests pass
EOF

after all, 
## run thelegram.py (say nothing, just run and that all)