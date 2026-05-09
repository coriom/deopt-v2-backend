# NEXT_TASK.md — RFQ V1C: Signed RFQ Quotes

## Context

RFQ V1A and V1B are implemented and runtime-verified.

Validated flow:

```text
HTTP POST /rfqs
→ connected MM receives rfq_request over WebTransport
→ MM sends rfq_quote over WebTransport
→ quote is persisted in rfq_quotes
→ HTTP GET /rfqs/:id/quotes sees quote
→ HTTP accept quote creates execution_intent
→ MM receives rfq_quote_accepted
→ no broadcast

Persistent runtime verification passed with:

RFQ accepted
accepted quote persisted
execution_intent persisted
buyer/seller mapping correct
no executor transaction created

Current weakness:

RFQ quotes are trusted because they arrive from a session. They are not cryptographically signed by the market maker.

Goal

Implement RFQ V1C: signed RFQ quotes.

A market maker RFQ quote must be signed over a deterministic EIP-712 typed message.

The backend must:

expose an RFQ quote signing payload
accept RFQ quote signatures
recover signer
verify signer equals mm_account
store signature/digest/signing metadata
reject invalid signatures in strict mode
keep unsigned/dev mode available by config
preserve existing RFQ V1B flow
keep accepted quote creating normal execution_intent
never auto-broadcast
Non-Goals

Do not implement:

production MM wallet challenge auth
signed taker RFQ request
options RFQ
multi-leg RFQ
MM ranking
RFQ auction engine
auto-signing
auto-simulation
auto-broadcast
Solidity changes
PerpTrade ABI changes
Absolute Safety Rules

Do not:

modify Solidity
deploy contracts
change execution lifecycle
enable real broadcast by default
call /executor/broadcast
fake signatures
fake recovered signer
bypass expiry
bypass price/size validation
bypass accepted quote single-winner rule
require live RPC/Postgres/WebTransport/private keys for normal cargo test
commit
push
expose private keys
Signature Modes

Add config:

RFQ_QUOTE_SIGNATURE_MODE=disabled

Allowed values:

disabled
strict

Behavior:

disabled
existing RFQ V1B behavior preserved
signature field optional
no cryptographic recovery required
useful for local/dev smoke tests
strict
quote signature required
backend verifies EIP-712 signature
recovered signer must equal mm_account
invalid signature rejected

Startup should reject invalid mode clearly.

EIP-712 RFQ Quote Type

Define typed data:

RFQQuote(
  bytes32 rfqId,
  address mmAccount,
  uint256 marketId,
  bool takerIsBuyer,
  uint128 price1e8,
  uint128 size1e8,
  uint256 quoteNonce,
  uint256 expiry
)

Field meanings:

rfqId: bytes32 derived from RFQ UUID/string
mmAccount: quote signer
marketId: RFQ market
takerIsBuyer: true if RFQ side is buy, false if RFQ side is sell
price1e8: quoted execution price
size1e8: quoted size
quoteNonce: MM-provided RFQ quote nonce for replay/idempotence
expiry: quote expiry in seconds or milliseconds; choose one and document clearly

Prefer seconds for EIP-712 if aligning with Solidity-style timestamps, but preserve existing ms storage if needed. Be explicit.

Domain:

Use existing backend EIP-712 domain style where possible.

Recommended config:

RFQ_EIP712_NAME=DeOptV2RFQ
RFQ_EIP712_VERSION=1
RFQ_EIP712_CHAIN_ID=84532
RFQ_EIP712_VERIFYING_CONTRACT=0x0000000000000000000000000000000000000000

If reusing existing EIP712 config is cleaner, document the choice.

RFQ ID bytes32

Add deterministic helper:

rfq_id_to_b256(rfq_id: &str) = keccak256(bytes(rfq_id))

Expose hex helper if needed.

API Changes
GET /rfqs/:rfq_id/quote-signing-payload

Query or body may include quote fields.

Preferred endpoint:

POST /rfqs/:rfq_id/quote-signing-payload

Request:

{
  "mm_account": "0x...",
  "price_1e8": "300100000000",
  "size_1e8": "100000000",
  "client_quote_id": "mm-rfq-quote-001",
  "quote_nonce": 1,
  "quote_ttl_ms": 3000
}

Response:

{
  "rfq_id": "...",
  "rfq_id_b32": "0x...",
  "digest": "0x...",
  "domain": {...},
  "types": {...},
  "message": {
    "rfqId": "0x...",
    "mmAccount": "0x...",
    "marketId": 1,
    "takerIsBuyer": true,
    "price1e8": "300100000000",
    "size1e8": "100000000",
    "quoteNonce": "1",
    "expiry": "..."
  }
}
POST /rfqs/:rfq_id/quotes

Extend request:

{
  "mm_account": "0x...",
  "price_1e8": "300100000000",
  "size_1e8": "100000000",
  "client_quote_id": "mm-rfq-quote-001",
  "quote_nonce": 1,
  "quote_ttl_ms": 3000,
  "signature": "0x..."
}

In strict mode:

signature required
quote_nonce required
digest recomputed server-side
recovered signer must equal mm_account
WebTransport rfq_quote

Extend payload similarly:

{
  "rfq_id": "...",
  "mm_account": "0x...",
  "price_1e8": "300100000000",
  "size_1e8": "100000000",
  "client_quote_id": "mm-rfq-quote-001",
  "quote_nonce": 1,
  "quote_ttl_ms": 3000,
  "signature": "0x..."
}

In disabled mode, smoke flow can omit signature.

In strict mode, reject missing/invalid signature.

Database

Extend rfq_quotes with nullable fields:

signature TEXT NULL,
quote_digest TEXT NULL,
quote_nonce TEXT NULL,
signature_status TEXT NULL,
recovered_signer TEXT NULL

Add migration:

migrations/0011_signed_rfq_quotes.sql

Do not modify old migration.

signature_status values:

not_required
verified
missing
invalid
signer_mismatch

In strict mode, only verified quotes can be active.

In disabled mode, store not_required.

Rust Modules

Add or extend:

src/rfq/signing.rs
src/rfq/types.rs
src/rfq/service.rs
src/api/routes.rs
src/mm/protocol.rs
src/mm/service.rs

Reuse existing ECDSA/EIP-712 signing utilities if present from PerpTrade signing.

Avoid duplicate crypto code if possible.

Dev CLI

Add optional dev CLI for RFQ quote signing:

src/bin/sign_rfq_quote.rs

Usage:

MM_PRIVATE_KEY=0x... cargo run --bin sign_rfq_quote -- \
  --payload /tmp/rfq_quote_payload.json

Output:

{
  "signer_address": "0x...",
  "signature": "0x..."
}

Same safety rules:

do not print private key
reject signer mismatch if role/account provided
testnet/dev only

If adding this CLI is too much, defer it. But signed RFQ strict mode is hard to test manually without it.

Tests

Normal cargo test must remain offline.

Add tests for:

rfq_id_to_b256 deterministic
RFQQuote typehash stable
signing payload digest deterministic
disabled mode accepts unsigned quote
strict mode rejects missing signature
strict mode rejects malformed signature
strict mode rejects invalid signature
strict mode rejects signer mismatch
strict mode accepts valid signature
HTTP quote endpoint stores signature metadata
WebTransport rfq_quote supports signature fields
quote acceptance only accepts active verified quote in strict mode
quote nonce stored
duplicate client_quote_id behavior unchanged
normal RFQ V1B disabled-mode tests still pass

No live RPC/Postgres/WebTransport/private keys required for normal tests.

Runtime Verification After Implementation

After implementation, perform only if feasible:

RFQ_QUOTE_SIGNATURE_MODE=disabled
existing RFQ smoke still passes
RFQ_QUOTE_SIGNATURE_MODE=strict
create RFQ
get signing payload
sign with sign_rfq_quote
submit quote over HTTP
verify quote accepted active/verified
accept quote creates execution_intent
no broadcast

WebTransport strict runtime can be deferred if HTTP strict path is verified.

Docs

Update:

README.md
ARCHITECTURE.md
.env.example

Document:

RFQ signed quote purpose
signature mode config
EIP-712 RFQQuote type
quote signing payload endpoint
strict vs disabled behavior
quote signature DB metadata
dev signing CLI
no auto-broadcast
future:
production MM auth
signed taker RFQ request
options RFQ
MM ranking
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

RFQ quote signature config exists
quote signing payload endpoint exists
strict mode verifies signatures
disabled mode preserves current flow
RFQ HTTP quote supports signatures
WebTransport rfq_quote supports signature fields
signature metadata persisted
tests cover valid/invalid/missing/mismatch signatures
no auto-broadcast
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
production MM auth/challenge
signed taker RFQ request
RFQ quote cancellation by signed message
WebTransport strict runtime smoke
options RFQ
multi-leg RFQ
MM ranking/selection
expiry scheduler
EOF





after all, 
## run telegram.py (say nothing, just run and that all)