# NEXT_TASK.md — Executor RPC Simulation V1

## Context

The Rust backend now has:
- deterministic in-memory matching
- strict EIP-712 order verification
- PostgreSQL persistence
- dry-run executor scaffold
- real PerpMatchingEngine calldata builder
- matched PerpTrade signature collection flow
- calldata readiness only when buyerSig and sellerSig are both present

Current limitation:
- calldata can be built, but it is not simulated against Base Sepolia
- execution intents cannot yet be marked simulation_ok or simulation_failed
- no on-chain safety check exists before future broadcast

## Goal

Add RPC simulation V1 for execution intents.

The executor must be able to:
- take a calldata-ready execution intent
- build the PerpMatchingEngine.executeTrade calldata
- run an eth_call simulation against the configured RPC
- persist simulation result
- expose a manual simulation endpoint

This task must not broadcast transactions.

## Critical Safety Rule

Do not send real transactions.

This task is simulation only:
- no private key loading
- no signing
- no sendTransaction
- no broadcast
- no submitted status
- no confirmed status

`simulation_ok` means only:
- eth_call did not revert at the current block

`simulation_ok` does not mean:
- submitted
- confirmed
- final
- executed

## Config

Add or extend `.env.example`:

```env
SIMULATION_ENABLED=false
SIMULATION_REQUIRE_PERSISTENCE=true
RPC_URL=
PERP_MATCHING_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000

Rules:

SIMULATION_ENABLED=false: simulation endpoints may return disabled or no-op.
SIMULATION_ENABLED=true: requires valid RPC_URL.
If SIMULATION_REQUIRE_PERSISTENCE=true, simulation requires PERSISTENCE_ENABLED=true.
No private key env vars.
Required Modules

Add or extend:

src/execution/simulator.rs
src/execution/rpc.rs
src/execution/status.rs
src/execution/executor.rs
src/execution/tx_builder.rs
src/db/repository.rs
src/db/models.rs

Names can differ if cleaner.

Execution Intent Statuses

Support statuses:

pending
dry_run
calldata_ready
simulation_ok
simulation_failed
submitted
confirmed
failed

In this task:

allowed transition: calldata_ready -> simulation_ok
allowed transition: calldata_ready -> simulation_failed
allowed transition: dry_run -> simulation_ok only if calldata exists and signatures exist
do not transition to submitted
do not transition to confirmed
Persistence Requirements

Add persistence for simulation result.

Preferred migration:

CREATE TABLE execution_simulations (
    simulation_id TEXT PRIMARY KEY,
    intent_id TEXT NOT NULL REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    block_number BIGINT,
    error TEXT,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_execution_simulations_intent_id ON execution_simulations(intent_id);
CREATE INDEX idx_execution_simulations_status ON execution_simulations(status);

Alternative acceptable:

add simulation columns to execution_intents
but separate table is preferred for audit trail

Repository methods:

get execution intent by id
get stored trade signatures for intent
insert simulation result
update intent status to simulation_ok / simulation_failed
append engine/executor event if existing audit path exists
RPC Simulation Behavior

Given an intent id:

Load execution intent.
Load trade signatures.
Validate both signatures exist.
Build PerpTradePayload.
Build executeTrade calldata.
Perform eth_call:
from: optional zero address or configured executor address
to: PERP_MATCHING_ENGINE_ADDRESS
data: calldata
value: 0
If eth_call succeeds:
persist simulation_ok
update intent status to simulation_ok
If eth_call reverts/fails:
persist simulation_failed with error message
update intent status to simulation_failed

Important:

If caller authorization matters, eth_call should use a configured executor address if available.
Add optional env:
EXECUTOR_FROM_ADDRESS=0x0000000000000000000000000000000000000000
Do not use a private key.
Do not sign.
Do not broadcast.
HTTP API

Add:

POST /executor/simulate/:intent_id

Behavior:

requires simulation enabled
requires persistence enabled if configured
loads intent from DB or in-memory if cleanly supported
simulates exactly one intent
returns simulation status

Response example:

{
  "intent_id": "abc",
  "simulation_status": "simulation_ok",
  "block_number": 123456,
  "submitted": false,
  "confirmed": false
}

On revert:

{
  "intent_id": "abc",
  "simulation_status": "simulation_failed",
  "error": "execution reverted: ..."
}

Update:

GET /executor/status

Add fields:

simulationEnabled
simulationRequiresPersistence
rpcConfigured
broadcastEnabled=false
Dry-Run Executor Integration

If simple:

dry-run executor may skip simulation unless explicitly requested.
Do not auto-simulate in background unless clean and safe.

Preferred for this task:

manual simulation endpoint only.
background auto-simulation can be deferred.
Tests Required

Normal cargo test must not require RPC or Postgres.

Add tests for:

simulation config disabled
simulation enabled requires RPC_URL
simulation requiring persistence rejects persistence disabled
/executor/status includes simulation fields
simulate endpoint rejects when simulation disabled
simulate endpoint rejects missing signatures before RPC
simulator maps success to simulation_ok using mocked provider if feasible
simulator maps failure to simulation_failed using mocked provider if feasible
no broadcast is possible
existing tests still pass

If mocking provider is too large:

isolate pure simulation result mapping tests
keep real RPC tests out of default test suite
Documentation

Update README.md:

explain simulation mode
explain env vars
explain that simulation is eth_call only
explain simulation_ok is not confirmation
explain no private key / no broadcast

Update ARCHITECTURE.md:

add simulation stage to execution lifecycle
pending/calldata_ready -> simulation_ok/simulation_failed
future submitted/confirmed only after broadcast and indexer reconciliation
Constraints

Do not add:

transaction broadcast
private key loading
sendTransaction
production executor
indexer
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deoptv2

Do not fake:

successful simulation
submitted status
confirmed status
on-chain execution
transaction hash

Do not change:

matching semantics
financial numeric rules
EIP-712 order verification behavior
PerpTrade signature collection behavior
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

The task is complete only if:

simulation config exists
simulation endpoint exists
eth_call simulation boundary exists
simulation result persistence exists
intent status can become simulation_ok or simulation_failed
no broadcast/private key/signing exists
default tests require no RPC/Postgres
all validation commands pass
EOF

after all, 
## run thelegram.py (say nothing, just run and that all)