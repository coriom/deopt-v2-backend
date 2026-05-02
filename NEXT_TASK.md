# NEXT_TASK.md — PerpMatchingEngine Calldata Builder V1

## Context

The Rust backend now has:
- deterministic matching
- execution intents
- strict EIP-712 order verification
- PostgreSQL persistence
- dry-run executor scaffold

The Solidity target for perp execution is:

- Contract: `PerpMatchingEngine`
- Function: `executeTrade(PerpTrade calldata t, bytes calldata buyerSig, bytes calldata sellerSig)`
- EIP-712 domain:
  - name: `DeOptV2-PerpMatchingEngine`
  - version: `1`

Solidity struct:

```solidity
struct PerpTrade {
    address buyer;
    address seller;
    uint256 marketId;
    uint128 sizeDelta1e8;
    uint128 executionPrice1e8;
    bool buyerIsMaker;
    uint256 buyerNonce;
    uint256 sellerNonce;
    uint256 deadline;
}

Typehash:

PerpTrade(address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)

The contract:

requires caller to be an authorized executor
verifies buyerSig and sellerSig over the exact same PerpTrade digest
consumes on-chain nonces
calls perpEngine.applyTrade(...)
Goal

Implement a real calldata builder boundary for PerpMatchingEngine.executeTrade.

This task must:

define the Rust representation of PerpTrade
map backend ExecutionIntent into a PerpTrade candidate
encode calldata for executeTrade
keep broadcast impossible
keep private key loading impossible
keep transaction submission impossible
Critical Design Note

The backend currently verifies signed off-chain orders.

The Solidity contract expects signatures over the final matched PerpTrade.

Therefore, this task must not pretend that existing order signatures are valid PerpTrade signatures.

For now:

buyerSig and sellerSig must be explicit fields supplied to the builder or placeholder-empty with signatures_missing=true
calldata may be built only when both trade signatures are present
otherwise the builder should return a clear MissingTradeSignatures error or produce a non-executable preview object

Do not fake buyerSig/sellerSig.

Scope

Implement:

PerpTradePayload
PerpTradeSignatureBundle
PreparedExecutionCall
ABI calldata encoding for executeTrade
deterministic tests for encoding shape
executor dry-run should report whether intent is calldata-ready or missing signatures

Do not implement:

RPC simulation
eth_call
signing
private key loading
broadcast
transaction status transitions to submitted/confirmed
indexer
Solidity changes
Suggested Modules

Add or extend:

src/execution/perp_trade.rs
src/execution/abi.rs
src/execution/tx_builder.rs

Keep names clean if existing structure differs.

Required Data Model
PerpTradePayload

Fields:

buyer
seller
market_id
size_delta_1e8
execution_price_1e8
buyer_is_maker
buyer_nonce
seller_nonce
deadline

Types:

addresses must be valid EVM addresses
market_id: u128 or U256-compatible
size_delta_1e8: u128
execution_price_1e8: u128
buyer_nonce: u128/U256-compatible
seller_nonce: u128/U256-compatible
deadline: u128/U256-compatible
PerpTradeSignatureBundle

Fields:

buyer_sig
seller_sig

Rules:

each signature must be 0x + 65-byte hex
no fake signatures
PreparedExecutionCall

Fields:

target
function_name
intent_id
market_id
buyer
seller
value
calldata
is_broadcastable
missing_signatures

Rules:

target = PERP_MATCHING_ENGINE_ADDRESS
function_name = executeTrade
value = 0
is_broadcastable=false for this task
missing_signatures=true if signatures are not provided
ABI Encoding

Use an idiomatic Rust ABI encoding crate.

Preferred:

alloy-primitives
alloy-sol-types

Define the Solidity call equivalent to:

function executeTrade(
    PerpTrade calldata t,
    bytes calldata buyerSig,
    bytes calldata sellerSig
)

Where PerpTrade is:

(address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256)

Do not hand-roll ABI encoding if alloy can do it cleanly.

Config

Ensure .env.example includes:

PERP_MATCHING_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000

If already present, keep it.

ExecutionIntent Mapping

From current ExecutionIntent:

buyer = intent.buyer
seller = intent.seller
marketId = intent.market_id
sizeDelta1e8 = intent.size_1e8
executionPrice1e8 = intent.price_1e8

For now, buyerIsMaker, buyerNonce, sellerNonce, and deadline may not be present in current ExecutionIntent.

Handle this explicitly:

do not silently invent values
add fields if necessary to the intent model only if clean and backwards-compatible
otherwise make the builder require an explicit PerpTradePayload

Preferred for this task:

create builder from explicit PerpTradePayload
create a separate preview mapper from ExecutionIntent that marks missing fields
Tests Required

Add tests for:

PerpTradePayload validates addresses
invalid buyer address rejected
invalid seller address rejected
missing signatures are detected
malformed signatures rejected
calldata builder creates non-empty calldata with valid signatures
calldata selector matches executeTrade((address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)
prepared call target equals configured PerpMatchingEngine address
prepared call remains is_broadcastable=false
existing executor/status tests still pass
existing matching tests still pass

No tests may require:

RPC
Postgres
private keys
Base Sepolia
Documentation

Update README.md and ARCHITECTURE.md:

explain calldata builder V1
explain that PerpMatchingEngine requires signatures over matched PerpTrade
explain that order signatures are not the same as trade signatures
explain that broadcast remains disabled
explain next steps:
obtain trade signatures
RPC simulation
broadcast
indexer reconciliation
Constraints

Do not add:

real transaction sending
private key loading
RPC simulation
fake signatures
fake ABI encoding
fake confirmation
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deoptv2

Do not change:

matching semantics
financial numeric rules
EIP-712 order verification behavior
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

The task is complete only if:

real ABI calldata can be built for PerpMatchingEngine.executeTrade
builder requires explicit buyerSig and sellerSig for executable calldata
missing trade signatures are not faked
broadcast remains impossible
all tests pass
no RPC/Postgres/private keys are required for normal tests
EOF

after all, run thelegram.py (say nothing, just run and that all)