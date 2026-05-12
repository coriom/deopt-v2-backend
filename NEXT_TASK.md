# NEXT_TASK.md — Options RFQ V1C: Signed Option RFQ Quotes

## Context

Options RFQ V1A/V1B are implemented and runtime-verified.

Validated flow:

```text
HTTP POST /options/rfqs
→ connected MM receives option_rfq_request over WebTransport
→ MM sends option_rfq_quote over WebTransport
→ quote persists in option_rfq_quotes
→ HTTP GET /options/rfqs/:id/quotes sees quote
→ HTTP accept quote creates option_rfq_fill
→ MM receives option_rfq_quote_accepted
→ no execution_intent
→ no execution_transaction

Current weakness:

Option RFQ quotes are trusted because they arrive from a MM session. They are not cryptographically signed by the market maker.

Goal

Implement signed Option RFQ quotes.

The backend must:

expose an option RFQ quote signing payload endpoint
accept option RFQ quote signatures over HTTP and WebTransport
recover signer
verify recovered signer equals mm_account
store signature metadata
support strict and disabled modes
preserve existing unsigned dev flow in disabled mode
never create execution_intents
never broadcast
Non-Goals

Do not implement:

production MM wallet auth
signed taker option RFQ requests
option execution intents
on-chain option execution
option settlement/exercise
Greeks
IV surface
MM ranking
auto-broadcast
Solidity changes
deployment
Absolute Safety Rules

Do not:

modify Solidity
deploy contracts
change existing perp/RFQ/MM lifecycle
create execution_intents from option RFQs
create execution_transactions from option RFQs
fake signatures
fake recovered signer
bypass expiry
bypass price/size validation
bypass single-winner rule
require live RPC/Postgres/WebTransport/private keys for normal cargo test
commit
push
expose private keys
Config

Add:

OPTION_RFQ_QUOTE_SIGNATURE_MODE=disabled

Allowed values:

disabled
strict

Behavior:

disabled
preserves current Option RFQ V1B behavior
signature optional
no cryptographic verification
store signature_status=not_required
strict
signature required
quote_nonce required
backend recomputes EIP-712 digest
recovered signer must equal mm_account
only verified quotes can become active

Invalid mode must fail startup clearly.

EIP-712 Type

Define:

OptionRFQQuote(
  bytes32 optionRfqId,
  address mmAccount,
  bytes32 optionSeriesId,
  bool takerIsBuyer,
  uint128 price1e8,
  uint128 size1e8,
  uint256 quoteNonce,
  uint256 expiry
)

Field meanings:

optionRfqId: keccak256(bytes(option_rfq_id))
mmAccount: quote signer
optionSeriesId: bytes32 from option_series_id hex
takerIsBuyer: true if option RFQ side is buy, false if sell
price1e8: option premium
size1e8: contract quantity
quoteNonce: MM-provided quote nonce
expiry: quote expiry in seconds

Use seconds in the EIP-712 message. Existing DB can keep ms timestamps.

Domain

Add config:

OPTION_RFQ_EIP712_NAME=DeOptV2OptionRFQ
OPTION_RFQ_EIP712_VERSION=1
OPTION_RFQ_EIP712_CHAIN_ID=84532
OPTION_RFQ_EIP712_VERIFYING_CONTRACT=0x0000000000000000000000000000000000000000

If reusing existing EIP-712 helpers is cleaner, do so, but keep domain independent from perp RFQ.

Database

Add migration:

migrations/0016_signed_option_rfq_quotes.sql

Extend option_rfq_quotes:

ALTER TABLE option_rfq_quotes ADD COLUMN signature TEXT NULL;
ALTER TABLE option_rfq_quotes ADD COLUMN quote_digest TEXT NULL;
ALTER TABLE option_rfq_quotes ADD COLUMN quote_nonce TEXT NULL;
ALTER TABLE option_rfq_quotes ADD COLUMN signature_status TEXT NULL;
ALTER TABLE option_rfq_quotes ADD COLUMN recovered_signer TEXT NULL;

CREATE INDEX idx_option_rfq_quotes_digest ON option_rfq_quotes(quote_digest);
CREATE INDEX idx_option_rfq_quotes_signature_status ON option_rfq_quotes(signature_status);

Signature statuses:

not_required
verified
missing
invalid
signer_mismatch

In strict mode, only verified quotes can be active.

API

Add endpoint:

POST /options/rfqs/:option_rfq_id/quote-signing-payload

Request:

{
  "mm_account": "0x...",
  "price_1e8": "1100000000",
  "size_1e8": "100000000",
  "client_quote_id": "mm-option-rfq-quote-001",
  "quote_nonce": 1001,
  "quote_ttl_ms": 3000
}

Response:

{
  "option_rfq_id": "...",
  "option_rfq_id_b32": "0x...",
  "option_series_id_b32": "0x...",
  "digest": "0x...",
  "primary_type": "OptionRFQQuote",
  "domain": {...},
  "types": {...},
  "message": {
    "optionRfqId": "0x...",
    "mmAccount": "0x...",
    "optionSeriesId": "0x...",
    "takerIsBuyer": true,
    "price1e8": "1100000000",
    "size1e8": "100000000",
    "quoteNonce": "1001",
    "expiry": "..."
  }
}

Extend HTTP quote submission:

POST /options/rfqs/:option_rfq_id/quotes

Request adds:

{
  "quote_nonce": 1001,
  "signature": "0x..."
}

In strict mode:

quote_nonce required
signature required
signer must match mm_account
WebTransport Message

Extend option_rfq_quote payload:

{
  "option_rfq_id": "...",
  "mm_account": "0x...",
  "price_1e8": "1100000000",
  "size_1e8": "100000000",
  "client_quote_id": "mm-option-rfq-quote-001",
  "quote_nonce": 1001,
  "quote_ttl_ms": 3000,
  "signature": "0x..."
}

Disabled mode may omit signature.

Strict mode rejects missing/invalid signatures.

Rust Modules

Add or extend:

src/options/signing.rs
src/options/types.rs
src/options/service.rs
src/options/store.rs
src/mm/protocol.rs
src/mm/service.rs
src/api/routes.rs
src/db/repository.rs
src/config/env.rs
src/signing/

Reuse existing EIP-712 and ECDSA helpers from RFQ/perp signing.

Avoid duplicating crypto logic.

Dev CLI

Add:

src/bin/sign_option_rfq_quote.rs

Usage:

MM_PRIVATE_KEY=0x... cargo run --bin sign_option_rfq_quote -- \
  --payload /tmp/option_rfq_quote_payload.json

Output:

{
  "signer_address": "0x...",
  "signature": "0x..."
}

Safety:

never print private key
reject invalid private key
output JSON only
Tests

Normal cargo test must remain offline.

Add tests for:

option_rfq_id_to_b32 deterministic
option_series_id_b32 parsing
OptionRFQQuote typehash stable
digest deterministic
signing payload endpoint returns expected structure
disabled mode accepts unsigned HTTP quote
strict mode rejects missing signature
strict mode rejects missing quote_nonce
strict mode rejects malformed signature
strict mode rejects invalid signature
strict mode rejects signer mismatch
strict mode accepts valid signature
strict mode stores signature metadata
WebTransport option_rfq_quote supports signature fields
strict mode rejects unsigned WebTransport option quote
strict mode accepts signed WebTransport option quote via service test
quote acceptance only accepts active verified quote in strict mode
disabled mode preserves existing V1B tests
no execution_intent created
no execution_transaction created

No live WebTransport/Postgres/private keys required for normal tests.

Runtime Verification After Implementation

If feasible, verify strict HTTP path:

OPTION_RFQ_QUOTE_SIGNATURE_MODE=strict
POST /options/rfqs
POST /options/rfqs/:id/quote-signing-payload
sign_option_rfq_quote
POST /options/rfqs/:id/quotes
POST /options/rfqs/:id/accept/:quote_id
verify option_rfq_fill
verify no execution_intents / execution_transactions

WebTransport strict runtime can be deferred to a separate runtime task.

Documentation

Update:

README.md
ARCHITECTURE.md
.env.example

Document:

signed option RFQ quote purpose
strict vs disabled modes
EIP-712 OptionRFQQuote type
signing payload endpoint
dev signing CLI
metadata persistence
no execution_intents
no broadcast
future production MM auth
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

config exists
migration exists
signing payload endpoint exists
strict mode verifies signatures
disabled mode preserves old flow
HTTP option RFQ quote supports signatures
WebTransport option_rfq_quote supports signatures
metadata persisted
sign_option_rfq_quote CLI exists or clearly deferred
tests cover strict/disabled/valid/invalid/mismatch
no execution_intents created
no execution_transactions created
docs updated
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
production MM auth
signed taker option RFQ request
WebTransport strict runtime verification
on-chain option settlement/exercise
Greeks
IV surface
option RFQ ranking
EOF



after all, 
## run telegram.py (say nothing, just run and that all)