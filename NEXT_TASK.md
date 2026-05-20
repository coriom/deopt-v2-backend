# NEXT_TASK.md — Option Execution Nonce Sync V1E

## Context

Backend Option Execution Intents V1C/V1D are implemented and runtime-verified.

Validated:

- option orderbook fill creates option_execution_intent
- option RFQ fill creates option_execution_intent
- EIP-712 OptionTrade signing payload
- signature submission
- calldata builder
- simulation endpoints
- safe disabled/missing-RPC runtime behavior
- no broadcast
- no execution_transactions

Current gap:

Option execution intents currently default buyer/seller nonces to `0`.

Before live eth_call and broadcast can be reliable, backend must sync nonces from:

```solidity
OptionMatchingEngine.nonces(address)
Goal

Implement Option Execution Nonce Sync V1E.

Add backend support to read and use on-chain option execution nonces from OptionMatchingEngine.

Non-Goals

Do not broadcast.
Do not submit transactions.
Do not create execution_transactions.
Do not implement indexer/reconciliation/confirmation.
Do not modify Solidity.
Do not modify frontend.
Do not deploy.
Do not require private keys.
Do not commit.
Do not push.

Safety Rules

Defaults remain safe:

EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false

No private keys.

No tx hash fabrication.

No confirmed/submitted option status.

Config

Add:

OPTION_NONCE_SYNC_ENABLED=false
OPTION_NONCE_SYNC_REQUIRE_RPC=true
OPTION_NONCE_SYNC_STRICT=true

Behavior:

disabled: current behavior preserved
enabled: backend reads OptionMatchingEngine.nonces(address) via eth_call
strict: intent creation/signing requires synced nonce values
require RPC: startup rejects if sync enabled but RPC or matching engine address is missing

Reuse existing perp nonce sync patterns where possible.

API

Add:

GET /options/accounts/:address/nonce

or:

GET /accounts/:address/option-nonce

Preferred: match existing perp nonce endpoint style.

Response:

{
  "account": "0x...",
  "nonce": "0",
  "source": "onchain",
  "option_matching_engine": "0x..."
}

Disabled mode should return a clear error:

{"error":"option nonce sync is disabled"}
Integration

When OPTION_EXECUTION_ENABLED=true and OPTION_NONCE_SYNC_ENABLED=true:

Option execution intent creation

For buyer and seller:

read on-chain nonce
set buyer_nonce
set seller_nonce
store values in option_execution_intents
Signing payload

Payload must use stored buyer/seller nonce.

Signature submission

Do not mutate nonces.

Calldata

Calldata must use stored nonces.

Strict Mode

If OPTION_NONCE_SYNC_STRICT=true:

fail intent creation if nonce cannot be read
do not create partial invalid intent
return explicit error

If OPTION_NONCE_SYNC_STRICT=false:

allow fallback to 0 or configured local value
mark source clearly if stored model supports it
document that non-strict mode is dev-only

Preferred V1E: strict mode for enabled sync.

RPC Behavior

Use eth_call:

to = OPTION_MATCHING_ENGINE_ADDRESS
data = nonces(address)

No private key.

No state change.

Reuse existing RPC helpers from perp nonce sync and option simulation.

Database

If useful, add fields:

buyer_nonce_source TEXT NULL
seller_nonce_source TEXT NULL

But avoid schema churn unless needed.

At minimum, existing buyer_nonce and seller_nonce must be populated correctly.

Tests

Normal cargo test must remain offline.

Add tests for:

disabled nonce sync preserves current behavior
nonce endpoint disabled returns clear error
nonce endpoint enabled returns mocked on-chain nonce
intent creation uses mocked buyer/seller on-chain nonces
signing payload uses stored synced nonces
calldata uses stored synced nonces
strict mode rejects when RPC unavailable
startup rejects missing RPC when enabled and require RPC
startup rejects missing OptionMatchingEngine address when enabled
failed nonce sync does not consume local nonces or create invalid intent
no execution_transaction created
existing option execution tests still pass

Use mock RPC or existing test abstractions. Do not require live RPC for tests.

Runtime Verification Scope

V1E runtime should verify:

disabled endpoint behavior
startup guard behavior
enabled with no RPC behavior
if live RPC + deployed OptionMatchingEngine available, real nonce read

If no deployed contract is available, live nonce read is deferred.

Documentation

Update:

README.md
ARCHITECTURE.md
docs/OPTION_EXECUTION_BACKEND.md
.env.example

Document:

config
endpoint
strict mode
no private keys
no tx submission
relation to signing payload
live RPC requirement
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

config exists
endpoint exists
on-chain nonce read path exists
intent creation can use synced nonces
signing payload uses synced nonces
calldata uses synced nonces
strict startup guards work
no broadcast
no execution transaction
tests added
docs updated
cargo fmt/clippy/test/build pass
Deferred
live Base Sepolia nonce runtime if no OptionMatchingEngine deployed
option broadcast
option indexer
option reconciliation
option confirmation
frontend UI