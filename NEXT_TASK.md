# NEXT_TASK.md — Option Execution Simulation V1D

## Context

Backend Option Execution Intents V1C is implemented and runtime-verified.

Validated:

- option orderbook fill creates option_execution_intent
- option RFQ fill creates option_execution_intent
- signing payload endpoint works
- dummy 65-byte signatures accepted in disabled mode
- calldata builder works
- admin options summary exposes option execution intent counts
- no execution_transactions
- no broadcast
- no tx hash fabrication

Current gap:

Option execution intents cannot yet be simulated against the chain.

## Goal

Implement Option Execution Simulation V1D.

Add a safe simulation path for option execution intents.

The backend should be able to:

- take a calldata-ready option execution intent
- run `eth_call` against `OPTION_MATCHING_ENGINE_ADDRESS`
- persist simulation result
- expose simulation endpoint/status
- keep broadcast disabled and unimplemented for options

## Non-Goals

Do not broadcast.
Do not submit transactions.
Do not create execution_transactions.
Do not implement indexer/reconciliation/confirmation.
Do not modify Solidity.
Do not modify frontend.
Do not deploy.
Do not require private keys.
Do not fabricate tx hashes.
Do not mark intents submitted/confirmed.
Do not commit.
Do not push.

## Safety Rules

Defaults remain safe:

```env
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false

Simulation may require RPC only when explicitly enabled.

No private keys.

No /executor/broadcast.

Config

Add or reuse config:

OPTION_EXECUTION_SIMULATION_ENABLED=false
OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true
OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0

Behavior:

disabled: simulation endpoint returns clear error
enabled: requires RPC_URL unless require flag is false
simulation does not require private key
simulation does not create execution transaction
simulation does not change on-chain state
Database

Either extend option_execution_intents or add a new table.

Preferred simple V1D extension:

ALTER TABLE option_execution_intents
ADD COLUMN simulation_status TEXT NULL,
ADD COLUMN simulation_error TEXT NULL,
ADD COLUMN simulation_block_number BIGINT NULL,
ADD COLUMN simulation_revert_data TEXT NULL,
ADD COLUMN simulation_revert_selector TEXT NULL,
ADD COLUMN simulated_at_ms BIGINT NULL;

If migration needed:

migrations/0020_option_execution_simulation.sql

Statuses:

simulation_pending
simulation_ok
simulation_failed
simulation_unavailable
API

Add endpoints:

POST /options/execution-intents/:intent_id/simulate
GET  /options/execution-intents/:intent_id/simulation

Behavior:

intent must exist
calldata must exist
matching engine address must be configured
RPC must be configured if required
perform eth_call
persist result
return structured JSON

Response shape should include:

{
  "intent_id": "...",
  "simulation_status": "simulation_ok",
  "block_number": 123,
  "error": null,
  "revert_data": null,
  "revert_selector": null
}

For revert:

{
  "simulation_status": "simulation_failed",
  "error": "...",
  "revert_data": "0x...",
  "revert_selector": "0x..."
}
RPC Behavior

Use existing RPC helper patterns from perp execution simulation if available.

Do not duplicate low-level JSON-RPC code unnecessarily.

Call:

eth_call
to = OPTION_MATCHING_ENGINE_ADDRESS
data = intent.calldata
from = configured executor address if required, otherwise zero/safe caller

Important:

Solidity OptionMatchingEngine.executeTrade is executor-gated. Simulation must use a caller address that is expected to be allowed executor on-chain.

Add config if needed:

OPTION_EXECUTION_SIMULATION_FROM=

If empty, use existing executor address config if safe.

Document that simulation will fail with unauthorized executor unless the from address is allowed in OptionMatchingEngine.

Validation Rules

Reject simulation if:

intent missing
intent has no calldata
signatures missing
matching engine address missing
RPC missing when required
option execution simulation disabled

Do not reject simply because broadcast is disabled.

Admin

Extend /admin/options/summary with simulation counts if simple.

Do not add admin writes.

Tests

Normal cargo test must remain offline.

Add tests for:

simulation disabled returns clear error
missing intent rejects
missing calldata rejects
missing RPC config rejected when required
successful mock RPC eth_call stores simulation_ok
reverted mock RPC stores simulation_failed and revert data
simulation does not create execution_transaction
simulation does not change submitted/confirmed status
admin summary includes simulation counts if implemented
existing option execution intent tests still pass

Use mock RPC abstractions if existing perp simulation tests have one.

If mocking RPC is too broad, isolate simulation planner logic and document live runtime verification deferred.

E2E Harness

Do not require live RPC in default all-safe.

Optionally add a skipped flow:

option-execution-simulate

that requires:

--require-rpc

If too broad, defer.

Documentation

Update:

README.md
ARCHITECTURE.md
docs/OPTION_EXECUTION_BACKEND.md
.env.example

Document:

simulation config
endpoints
RPC requirement
executor from requirement
no broadcast
no tx hash
V1E deferred broadcast/index/reconciliation
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

simulation config exists
migration exists if DB changed
simulation endpoints exist
eth_call path exists or is cleanly abstracted/mocked
result persistence exists
no broadcast
no execution transaction
tests added
docs updated
cargo fmt/clippy/test/build pass
Deferred
option broadcast
option indexer
option reconciliation
option confirmation
nonce sync
frontend UI
live RPC E2E CI