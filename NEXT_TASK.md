# NEXT_TASK.md — Valid Option Execution Signatures V1H

## Context

Live Base Sepolia option simulation now reaches OptionMatchingEngine.

Confirmed:

- OptionMatchingEngine deployed:
  0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
- MarginEngine.matchingEngine points to OptionMatchingEngine.
- Tested option series is active.
- Live nonce sync works.
- Backend creates option_execution_intent.
- Signing payload uses synced nonces.
- Calldata is generated.
- Live simulation performs eth_call.
- Previous revert selector:
  0x8baa579f = InvalidSignature()
- This proves series activation is fixed and execution reaches signature verification.

Current blocker:

The simulation uses dummy buyer/seller signatures. Need valid EIP-712 signatures for OptionTrade.

## Goal

Implement or verify a safe dev signing path for option execution intents, then runtime-test with valid buyer/seller signatures.

The goal is to progress beyond `InvalidSignature()`.

Acceptable next result:

- `simulation_ok`, if margin/collateral is sufficient
- or `simulation_failed` with a later-stage revert, likely `MarginRequirementBreached(address)`

Unacceptable:

- `InvalidSignature()`
- `SeriesInactive()`
- no RPC call
- panic
- broadcast
- execution transaction creation

## Non-Goals

Do not broadcast.
Do not call /executor/broadcast.
Do not submit transactions.
Do not create execution_transactions.
Do not modify Solidity.
Do not modify frontend.
Do not deploy.
Do not commit.
Do not push.
Do not print private keys.

## Safety Rules

Keep:

```env
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false

Private keys may only be read from process env for local dev signing.

Never log private keys.

No tx hash fabrication.

No submitted/confirmed option status.

Required Work

Inspect existing signing CLIs:

src/bin/sign_rfq_quote.rs
src/bin/sign_option_rfq_quote.rs
src/signing/*
src/options/execution.rs

Determine whether there is already a reusable EIP-712 signing helper.

If no option execution signing CLI exists, add:

src/bin/sign_option_execution_intent.rs

CLI behavior:

Input:

intent id
backend URL or signing payload JSON file
private key from env only

Env vars:

OPTION_EXECUTION_SIGNER_PRIVATE_KEY=

or support explicit role envs:

BUYER_PRIVATE_KEY=
SELLER_PRIVATE_KEY=

Preferred safe design:

cargo run --bin sign_option_execution_intent -- \
  --payload-file /tmp/option_trade_payload.json \
  --private-key-env BUYER_PRIVATE_KEY

Output only JSON:

{
  "signer_address": "0x...",
  "signature": "0x..."
}

Do not print payload private key, raw secret, mnemonic, or env values.

Runtime Verification

Start backend with live Base Sepolia process env:

PERSISTENCE_ENABLED=true
OPTIONS_ENABLED=true
OPTION_RFQ_ENABLED=true
OPTION_EXECUTION_ENABLED=true
OPTION_EXECUTION_REQUIRE_PERSISTENCE=true

OPTION_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
OPTION_EXECUTION_SIGNATURE_MODE=strict
OPTION_EXECUTION_CHAIN_ID=84532
OPTION_EXECUTION_EIP712_NAME=DeOptV2-OptionMatchingEngine
OPTION_EXECUTION_EIP712_VERSION=1
OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS=6

OPTION_NONCE_SYNC_ENABLED=true
OPTION_NONCE_SYNC_REQUIRE_RPC=true
OPTION_NONCE_SYNC_STRICT=true

OPTION_EXECUTION_SIMULATION_ENABLED=true
OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true
OPTION_EXECUTION_SIMULATION_FROM=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27

EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false

Use RPC_URL but do not print it.

Test Flow
Create backend option series mapped to active on-chain option id:
24145907678156652148089862289363692212069910767044828147380657249455352740183
Create crossing option orderbook fill.
Verify option_execution_intent exists.
Fetch signing payload:
GET /options/execution-intents/:intent_id/signing-payload
Save payload to temp file.
Sign as buyer with buyer private key.
Sign as seller with seller private key.
Submit signatures:
POST /options/execution-intents/:intent_id/signatures
Confirm strict signature verification succeeds.
Fetch calldata.
Run live simulation:
POST /options/execution-intents/:intent_id/simulate
Verify result is not InvalidSignature().
Verify result is not SeriesInactive().
Verify no execution_transactions.
Cleanup backend runtime rows only.
Important Buyer/Seller Matching

The private keys must correspond exactly to the buyer/seller addresses used in the option fill.

If using known dev accounts:

buyer order account must match BUYER_PRIVATE_KEY
seller order account must match SELLER_PRIVATE_KEY

If only one private key is available, create both orders with accounts controlled by available keys only if protocol allows different buyer/seller addresses controlled locally.

Do not fake recovered signer.

Expected Result

Preferred:

simulation_failed
revert != InvalidSignature()
revert != SeriesInactive()
likely MarginRequirementBreached(address)

or:

simulation_ok

Either is acceptable.

Tests

If a new CLI is added, add offline tests for:

payload parsing
signature output shape
recovered signer matches expected
no private key logged
invalid payload rejected

If no code patch is needed, no tests are required.

Validation

If code changed, run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

If no code changed, at least run:

cargo fmt --check
cargo build
Final Report

Return:

files changed
whether code patch was needed
signing CLI/helper summary
buyer/seller signer addresses
execution intent id
synced buyer/seller nonces
strict signature submission result
calldata result
live simulation result
whether InvalidSignature is gone
whether SeriesInactive is gone
no forbidden mutation verification
cleanup result
validation commands run
remaining blocker