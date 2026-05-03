# NEXT_TASK.md — Dev EIP-712 PerpTrade Signing CLI

## Context

The backend now has:
- deterministic matching
- signed order verification
- PostgreSQL persistence
- execution intents with deterministic `onchain_intent_id`
- PerpTrade signing payload endpoint
- buyer/seller PerpTrade signature collection
- real `PerpMatchingEngine.executeTrade` calldata builder
- RPC simulation via `eth_call`
- Indexer V1
- Reconciliation V1 by `onchain_intent_id`
- guarded real broadcast V1:
  - disabled by default
  - real tx signing/sending only behind explicit config
  - no confirmed status yet

Current limitation:
- tests currently use fake `aaaa...` / `bbbb...` PerpTrade signatures
- fake signatures correctly produce `simulation_failed`
- real broadcast is blocked by `EXECUTOR_REQUIRE_SIMULATION_OK=true`
- we need a dev-only way to generate valid PerpTrade EIP-712 signatures for Base Sepolia testing

## Goal

Add a development-only CLI tool that signs PerpTrade EIP-712 payloads using local private keys.

The tool must allow local testnet flow:

```text
GET /execution-intents/:intent_id/signing-payload
-> sign with buyer private key
-> sign with seller private key
-> POST /execution-intents/:intent_id/signatures
-> simulate
-> later broadcast if simulation_ok

This task must not change server safety.

Critical Safety Rules

Do not:

add private keys to committed files
log private keys
expose private keys through API
add server-side automatic signing
add custodial signing
add broadcast changes
mark intents submitted
mark intents confirmed
modify Solidity
change matching semantics
introduce floating point arithmetic

This CLI is dev/testnet only.

It must be explicit in README:

never use with production keys
do not commit .env
use throwaway test wallets only
CLI Location

Preferred:

src/bin/sign_perp_trade.rs

Alternative if project layout prefers:

examples/sign_perp_trade.rs

Use src/bin/sign_perp_trade.rs if possible so it runs with:

cargo run --bin sign_perp_trade -- ...
CLI Inputs

Support one of these clean modes.

Preferred mode A — payload JSON file

The user saves the backend signing payload to a file:

curl http://127.0.0.1:8080/execution-intents/$INTENT_ID/signing-payload \
  > /tmp/perp_trade_payload.json

Then signs:

BUYER_PRIVATE_KEY=0x... \
cargo run --bin sign_perp_trade -- \
  --payload /tmp/perp_trade_payload.json \
  --role buyer

and:

SELLER_PRIVATE_KEY=0x... \
cargo run --bin sign_perp_trade -- \
  --payload /tmp/perp_trade_payload.json \
  --role seller
Required args
--payload <path>
--role buyer|seller
Env private key

Role determines env var:

buyer  -> BUYER_PRIVATE_KEY
seller -> SELLER_PRIVATE_KEY

Also allow fallback:

SIGNER_PRIVATE_KEY

but role-specific env var should take precedence.

CLI Output

Output JSON only:

{
  "role": "buyer",
  "signer_address": "0x...",
  "signature": "0x..."
}

Rules:

signature must be 65-byte recoverable ECDSA signature
0x + 130 hex chars
use Ethereum-compatible recovery id format accepted by backend/contract
do not print private key
do not print raw digest unless --verbose is explicitly passed
default output should be easy to pipe/copy

Optional --verbose can print:

digest
domain
message intentId
signer address
EIP-712 Requirements

The CLI must sign exactly the digest returned by backend payload if the payload contains a digest.

Preferred:

parse digest from backend payload
sign that digest directly
this avoids duplicating EIP-712 hashing logic incorrectly

Also validate payload shape:

primary_type == PerpTrade
types first field is intentId bytes32
domain.chainId == 84532 unless overridden by optional flag
message.buyer/seller exists
message.intentId exists

Signer/address check:

if role=buyer, derived signer address must equal message.buyer
if role=seller, derived signer address must equal message.seller
if mismatch, fail clearly unless optional --allow-address-mismatch is provided
default must reject mismatch

This prevents signing a buyer payload with the seller key by mistake.

Signature Format

Ensure output format is compatible with current Solidity contract and backend.

The backend/contract supports ECDSA v values:

27/28 preferred
0/1 accepted if already supported

Use 27/28 if straightforward.

Tests should validate:

signature length
signature recovers to expected address if recovery helper exists
malformed key rejected
Dependencies

Use existing crypto dependencies if already present:

k256
sha3
hex
serde_json

Do not add unnecessary large dependencies if current stack can sign.

If signing recoverable ECDSA is difficult with current dependencies:

implement carefully with current k256 recovery APIs
otherwise add a focused dependency with justification
Tests Required

Normal cargo test must not require private keys, RPC, Postgres, or Base Sepolia.

Add tests for pure helpers:

parse payload digest
reject missing digest
reject non-PerpTrade payload
reject payload missing intentId
reject invalid role
private key redaction/debug safety if applicable
derived signer address from deterministic test key
role/address mismatch rejection
signature length is 65 bytes / 130 hex chars if feasible without live network

If CLI integration tests are too heavy:

test internal signer helper functions
keep actual CLI invocation manual
README Update

Add section:

Development PerpTrade signing

Include exact commands:

Create intent.
Fetch signing payload.
Sign buyer:
BUYER_PRIVATE_KEY=0x... cargo run --bin sign_perp_trade -- --payload /tmp/perp_trade_payload.json --role buyer
Sign seller:
SELLER_PRIVATE_KEY=0x... cargo run --bin sign_perp_trade -- --payload /tmp/perp_trade_payload.json --role seller
Submit signatures:
curl -X POST http://127.0.0.1:8080/execution-intents/$INTENT_ID/signatures \
  -H "Content-Type: application/json" \
  -d '{"buyer_sig":"0x...","seller_sig":"0x..."}'
Simulate:
curl -X POST http://127.0.0.1:8080/executor/simulate/$INTENT_ID

Warnings:

testnet only
throwaway keys only
never commit keys
no production custody model
Architecture Update

Update ARCHITECTURE.md:

add dev signing CLI as a local testing tool
explicitly state production signing must happen in user/MM wallets or external signer
server must not become a custodial signer for users
CLI is not part of production request path
Constraints

Do not add:

server-side automatic signing
private key storage
key database
production custody
broadcast changes
confirmed lifecycle
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deopt-v2-sol

Do not change:

matching semantics
financial numeric rules
signed order verification
PerpTrade payload shape
indexer behavior
reconciliation behavior
broadcast safety behavior
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

Normal tests must not require:

RPC
Postgres
private keys
Base Sepolia
Acceptance Criteria

Complete only if:

CLI exists
CLI signs backend-provided digest
CLI validates role/address by default
CLI outputs JSON with signer_address and signature
private keys are never logged
README has exact usage flow
server safety unchanged
all validation commands pass
EOF


after all, 
## run telegram.py (say nothing, just run and that all)