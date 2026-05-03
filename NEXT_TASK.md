# NEXT_TASK.md — Real Broadcast V1, Guarded Transaction Submission

## Context

The backend now has:
- deterministic matching
- signed order verification
- PostgreSQL persistence
- execution intents with deterministic `onchain_intent_id`
- PerpTrade signing payload with `intentId`
- buyer/seller PerpTrade signature collection
- real `PerpMatchingEngine.executeTrade` calldata builder
- RPC simulation via `eth_call`
- Indexer V1 decoding `TradeExecuted(intentId, buyer, seller, ...)`
- Reconciliation V1 by `onchain_intent_id`
- guarded broadcast scaffold:
  - `POST /executor/broadcast/:intent_id`
  - `GET /executor/transactions`
  - transaction table
  - no fake tx hash
  - no submitted/confirmed status
  - real broadcast disabled by default

Current limitation:
- the backend cannot send a real transaction
- no tx hash is persisted
- no `submitted` status exists in practice
- no receipt/finality confirmation exists yet

## Goal

Implement Real Broadcast V1.

The backend must be able to:
- sign a real transaction when explicitly enabled
- send raw transaction via RPC
- persist the real tx hash
- mark the execution transaction as `submitted`
- mark the execution intent as `submitted`
- never mark `confirmed`
- refuse unsafe broadcast attempts

This task must implement real broadcast only behind explicit config gates.

## Critical Safety Rules

Default behavior must remain safe.

Real broadcast must happen only if all are true:
- `EXECUTOR_REAL_BROADCAST_ENABLED=true`
- `EXECUTOR_PRIVATE_KEY` is present and valid
- `EXECUTOR_REQUIRE_SIMULATION_OK=true` implies intent status is `simulation_ok`
- buyer/seller PerpTrade signatures are present
- calldata can be built
- `RPC_URL` is configured
- `PERP_MATCHING_ENGINE_ADDRESS` is configured
- chain id matches `EXECUTOR_CHAIN_ID`

Do not:
- fake tx hash
- fake submitted status
- mark confirmed
- fake confirmation
- fake tx ownership
- modify Solidity
- change matching semantics
- introduce floating point arithmetic
- log or expose private key

If real broadcast cannot be implemented fully, fail explicitly with `RealBroadcastUnavailable`.
Do not pretend to submit.

## Config

Current config exists:

```env
EXECUTOR_REAL_BROADCAST_ENABLED=false
EXECUTOR_PRIVATE_KEY=
EXECUTOR_CHAIN_ID=84532
EXECUTOR_MAX_GAS_LIMIT=1000000
EXECUTOR_MAX_FEE_PER_GAS_WEI=
EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI=
EXECUTOR_REQUIRE_SIMULATION_OK=true

Extend if needed:

EXECUTOR_GAS_ESTIMATION_ENABLED=false
EXECUTOR_TX_TYPE=eip1559

Rules:

broadcast disabled by default
private key required only when real broadcast enabled
private key must never be logged
private key must not be retained in public/debug structs
invalid private key must fail startup or broadcast cleanly
chain mismatch must fail cleanly
Dependencies

Use idiomatic Rust Ethereum tooling.

Preferred:

alloy stack if already partially present
otherwise ethers is acceptable

Requirements:

sign EIP-1559 transaction or legacy transaction cleanly
produce real raw signed transaction
call eth_sendRawTransaction
parse returned tx hash
persist tx hash

Do not use shell commands or external CLIs for signing/broadcast.

Transaction Requirements

Build transaction from prepared execution call:

to = PERP_MATCHING_ENGINE_ADDRESS
data = executeTrade(...) calldata
value = 0
chainId = EXECUTOR_CHAIN_ID
gas limit = configured EXECUTOR_MAX_GAS_LIMIT unless gas estimation is implemented
maxFeePerGas / maxPriorityFeePerGas:
use configured values if provided
if not provided, fetch reasonable values from RPC if implemented
if fee discovery is not implemented, require explicit values when broadcast enabled

Preferred V1:

use configured EXECUTOR_MAX_FEE_PER_GAS_WEI
use configured EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI
if missing and real broadcast enabled, fail with clear config error

No automatic gas policy overbuild.

RPC Requirements

Add RPC methods if missing:

eth_chainId
eth_getTransactionCount
eth_sendRawTransaction
optionally eth_gasPrice or fee history if implemented

Nonce:

fetch nonce for executor signer address via eth_getTransactionCount(address, "pending")
do not maintain local nonce manager in this task

Chain id:

check eth_chainId == EXECUTOR_CHAIN_ID before signing/sending
Signer Requirements

Add signer module if clean:

src/execution/signer.rs

Responsibilities:

parse private key
derive executor address
sign transaction
expose only address, never private key

API/debug output may include:

executor address

API/debug output must not include:

private key
raw signed tx unless strictly necessary; prefer not exposing it
Broadcast Flow

For POST /executor/broadcast/:intent_id:

Load execution intent.
Reject if already submitted.
If EXECUTOR_REQUIRE_SIMULATION_OK=true, require intent status simulation_ok.
Load signatures.
Build PerpTradePayload.
Build calldata.
Build transaction request.
If broadcast disabled:
return refused response:
submitted=false
confirmed=false
tx_hash=null
reason="broadcast disabled"
do not create fake tx hash
do not mutate intent status
If broadcast enabled:
validate private key/signer
check RPC chain id
fetch pending nonce
sign transaction
send raw transaction
persist transaction with:
status = submitted
tx_hash = real RPC result
update intent status to submitted
return tx_hash
confirmed=false

No confirmed transition in this task.

Database Behavior

Existing execution_transactions table exists.

Use it.

Repository additions if needed:

update intent status to submitted
insert submitted transaction atomically if possible
detect already submitted tx for intent
list transaction records

Idempotency:

if intent already has a submitted transaction, return existing tx hash or reject with clear already submitted
do not send duplicate tx

Atomicity:

after RPC returns tx_hash, persist tx record and intent submitted.
if DB persistence fails after RPC send, return explicit critical error. Document this limitation.
ideal: record prepared attempt before send, update after send. Implement if clean.
API Response

POST /executor/broadcast/:intent_id response:

{
  "intent_id": "...",
  "onchain_intent_id": "0x...",
  "broadcast_enabled": true,
  "submitted": true,
  "confirmed": false,
  "tx_hash": "0x...",
  "reason": null
}

Disabled response remains:

{
  "intent_id": "...",
  "onchain_intent_id": "0x...",
  "broadcast_enabled": false,
  "submitted": false,
  "confirmed": false,
  "tx_hash": null,
  "reason": "broadcast disabled"
}
Execution Intent Status

Allowed mutation:

simulation_ok -> submitted

Forbidden mutation:

anything -> confirmed

If broadcast disabled:

no status mutation

If RPC send fails:

transaction status failed or rejected attempt if recorded
intent status should remain simulation_ok unless a clean failure state is already modeled
do not mark submitted without tx hash
Tests Required

Normal cargo test must not require RPC, Postgres, private keys, or Base Sepolia.

Add pure/unit tests for:

broadcast disabled by default
broadcast enabled requires private key
broadcast enabled requires fee config if fee discovery not implemented
broadcast enabled requires RPC URL
private key is not exposed in debug/API structs
broadcast rejects non-simulation_ok when EXECUTOR_REQUIRE_SIMULATION_OK=true
broadcast rejects missing signatures
broadcast disabled returns submitted=false, confirmed=false, tx_hash=null
submitted response always has confirmed=false
already submitted intent is not broadcast twice
transaction request uses:
target = PerpMatchingEngine
value = 0
chain_id = EXECUTOR_CHAIN_ID
no confirmed mutation exists

If signer tests are added:

use deterministic test private key
test derived address
do not require network

If RPC broadcast tests are added:

use mocked RPC only
no live network in default tests
Documentation

Update README.md:

explain Real Broadcast V1
explain disabled by default
explain required env vars
explain simulation_ok requirement
explain submitted != confirmed
explain receipt/indexer/reconciliation confirmation is next phase
explain private key safety assumptions

Update ARCHITECTURE.md:

add submitted stage
lifecycle:
order accepted
matched
intent created
trade signatures collected
simulation_ok
submitted with tx_hash
indexed event
reconciled by onchain_intent_id
confirmed later after receipt/finality/reorg checks
explicitly state confirmation is not part of this task
Constraints

Do not add:

confirmed lifecycle
fake tx hash
fake broadcast
fake tx ownership
frontend
TypeScript
Python
Node.js
Solidity changes
local nonce manager
background auto-broadcast

Do not modify:

~/DEOPT/deopt-v2-sol

Do not change:

matching semantics
financial numeric rules
signed order verification
PerpTrade signing payload
indexer behavior
reconciliation confirmed=0 rule
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

real broadcast is available only behind explicit config
disabled path remains safe
real broadcast either works fully or fails explicitly
no fake tx hash exists
submitted status is written only after real tx hash
confirmed is never written
tx hash persistence exists
all validation commands pass
EOF

after all, 
## run telegram.py (say nothing, just run and that all)