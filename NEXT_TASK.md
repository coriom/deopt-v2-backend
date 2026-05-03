# NEXT_TASK.md — Backend Adaptation to PerpTrade intentId

## Context

The Solidity PerpMatchingEngine has been updated and verified on Base Sepolia.

Solidity changes:
- `PerpTrade` now includes `bytes32 intentId` as the first field.
- `TRADE_TYPEHASH` now includes `intentId`.
- `TradeExecuted` now emits `bytes32 indexed intentId`.
- `marketId` is no longer indexed in the event.
- `IPerpEngineTrade.Trade` is unchanged.
- `VerifyDeployment` passes on Base Sepolia.

Updated Solidity struct:

```solidity
struct PerpTrade {
    bytes32 intentId;
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

Updated Solidity event:

event TradeExecuted(
    bytes32 indexed intentId,
    address indexed buyer,
    address indexed seller,
    uint256 marketId,
    uint128 sizeDelta1e8,
    uint128 executionPrice1e8,
    bool buyerIsMaker,
    uint256 buyerNonce,
    uint256 sellerNonce
);

Updated EIP-712 type:

PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)

Current backend status:

matching works
persistence works
signed order verification works
PerpTrade signature flow works for the old struct
calldata builder works for the old struct
indexer works for the old TradeExecuted event
reconciliation work was being designed around economic match keys, but direct intentId is now available
Goal

Adapt the Rust backend to the new Solidity PerpTrade ABI/event with bytes32 intentId.

The backend must:

include intentId in PerpTrade signing payload
include intentId in PerpTrade EIP-712 digest
include intentId in calldata encoding
decode TradeExecuted with intentId as topic1
persist on-chain intent id
prefer direct reconciliation through intentId

This task must not add real transaction broadcast.

Critical Safety Rules

Do not:

add private key loading
add transaction broadcast
mark intents submitted
mark intents confirmed
fake confirmation
modify the Solidity repository
change matching semantics
introduce floating point arithmetic

Normal tests must not require:

RPC
Postgres
private keys
Base Sepolia
Intent ID Mapping

Backend execution_intents.intent_id is currently a UUID string.

Map this backend UUID string to Solidity bytes32 intentId deterministically.

Preferred mapping:

intentId = keccak256(bytes(execution_intents.intent_id))

Rules:

output as 0x + 64 hex chars
never zero
deterministic
documented
reusable by signing payload, calldata builder, indexer/reconciliation

Add a helper, e.g.:

intent_id_to_b256(intent_id: &str) -> Result<B256>
intent_id_to_hex_bytes32(intent_id: &str) -> Result<String>

Do not use random bytes.
Do not use the raw UUID bytes unless already documented and tested.
Use keccak256 because this is EVM-centric.

PerpTradePayload Changes

Update PerpTradePayload to include:

intent_id: bytes32 / B256 / [u8;32]

Field order must match Solidity exactly:

intentId
buyer
seller
marketId
sizeDelta1e8
executionPrice1e8
buyerIsMaker
buyerNonce
sellerNonce
deadline

Validation:

reject zero intentId
buyer/seller valid EVM addresses
price/size nonzero
nonce/deadline existing validation remains
Signing Payload Endpoint

Update:

GET /execution-intents/:intent_id/signing-payload

Response must include intentId inside the EIP-712 message:

{
  "message": {
    "intentId": "0x..."
  }
}

Type fields must include first:

{"name":"intentId","type":"bytes32"}

The returned digest must be recomputed with the new typehash and field order.

PerpTrade EIP-712 Digest

Update typehash to:

PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)

Hash order must exactly match Solidity.

Tests must confirm:

digest changes when intentId changes
digest is deterministic
signing payload digest matches PerpTradePayload digest
ABI Calldata Encoding

Update executeTrade calldata encoding.

New function signature:

executeTrade((bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)

Old function signature was:

executeTrade((address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)

Update:

ABI struct
calldata builder
selector test
prepared call tests

PreparedExecutionCall must still have:

is_broadcastable=false
no send transaction
no private key
Indexer Decoder Update

Update TradeExecuted event decoder.

New event layout:

topic0 = keccak256 TradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)
topic1 = indexed intentId
topic2 = indexed buyer
topic3 = indexed seller
data:
marketId
sizeDelta1e8
executionPrice1e8
buyerIsMaker
buyerNonce
sellerNonce

Old layout had:

topic1 buyer
topic2 seller
topic3 marketId

Update:

topic0 constant
decode logic
tests with synthetic log
API response if needed
Database Migration

Add next migration after existing migrations.

Add to indexed_perp_trades:

ALTER TABLE indexed_perp_trades ADD COLUMN onchain_intent_id TEXT;
CREATE INDEX idx_indexed_perp_trades_onchain_intent_id ON indexed_perp_trades(onchain_intent_id);

Do not delete old migrations.
Do not rewrite historical migrations.

If there is an existing reconciliation migration already added locally, adapt it instead of duplicating concepts.

Reconciliation Update

If reconciliation module already exists:

update it to prefer onchain_intent_id

If reconciliation module does not yet exist:

implement minimal direct reconciliation scaffold only if clean.

Primary matching rule:

keccak256(bytes(execution_intents.intent_id)) == indexed_perp_trades.onchain_intent_id

Do not mark confirmed.

Allowed result:

matched
unmatched
ambiguous if duplicate data exists

But this task may stop at:

storing onchain_intent_id
exposing it
adding pure helper/tests

Do not overbuild.

API Updates

Update existing responses if relevant:

signing payload includes intentId
indexed perp trades include onchain_intent_id
reconciliation endpoints, if present, use direct intent id matching

No route breaking changes.

Tests Required

Add/update tests for:

Intent ID mapping
UUID string maps deterministically to bytes32
same UUID gives same bytes32
different UUID gives different bytes32
output is 0x + 64 hex chars
zero intentId rejected if manually constructed
Signing payload
includes intentId as first EIP-712 field
message includes intentId
digest changes when intent id changes
ABI
selector matches:
executeTrade((bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)
calldata is non-empty with signatures
prepared call remains non-broadcastable
Indexer
TradeExecuted topic0 updated
decoder reads topic1 as intentId
decoder reads buyer from topic2
decoder reads seller from topic3
decoder reads marketId from data
Reconciliation
direct onchain_intent_id match works if module exists
no intent is marked confirmed

Existing tests must still pass.

Documentation

Update README.md and ARCHITECTURE.md:

explain intentId = keccak256(intent_id UUID string)
explain Solidity now emits intentId
explain reconciliation is direct through on-chain intent id
explain economic match key is fallback only if still present
explain no confirmed status yet
explain no broadcast yet
Constraints

Do not add:

transaction broadcast
private key loading
sendTransaction
production executor
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deopt-v2-sol

Do not fake:

confirmed status
submitted status
on-chain execution
transaction hash
tx ownership

Do not change:

matching semantics
financial numeric rules
signed order verification behavior
persistence default behavior
simulation safety behavior
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

backend PerpTrade payload includes intentId
signing payload includes intentId
EIP-712 digest uses new Solidity typehash
calldata encoding uses new tuple shape
indexer decodes new TradeExecuted layout
DB stores onchain_intent_id
no broadcast/confirmation is added
all validation commands pass
EOF

after all, 
## run telegram.py (say nothing, just run and that all)