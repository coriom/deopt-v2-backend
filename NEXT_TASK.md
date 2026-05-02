# NEXT_TASK.md — On-Chain Executor Scaffold V1

## Context

The Rust trading backend is implemented and validated.

Current status:
- deterministic in-memory matching works
- execution intents are created
- public API uses string fixed-point quantities
- signed order boundary exists
- strict EIP-712 signature verification works
- PostgreSQL persistence exists
- used nonces, orders, trades, and execution intents persist correctly
- persistence was validated at runtime
- nonce replay after restart is rejected
- cargo fmt: OK
- cargo clippy --all-targets --all-features -- -D warnings: OK
- cargo test: OK
- cargo build: OK

Current limitation:
- execution intents are created and persisted but are not yet consumed by an executor
- no on-chain transaction is built, simulated, or submitted
- execution intent lifecycle remains stuck at `pending`

## Goal

Add an on-chain executor scaffold V1.

This task must prepare the executor architecture without creating unsafe production execution.

The executor should:
- load execution configuration
- read pending execution intents
- expose executor status
- provide a dry-run execution path
- scaffold transaction construction boundaries
- update execution intent status safely only in dry-run / scaffold mode
- never send real transactions unless explicitly enabled in a future task

## Critical Safety Rule

Do not send real blockchain transactions in this task.

This is an executor scaffold, not production execution.

No private key loading yet.
No transaction signing yet.
No broadcast yet.

## Stack

Use Rust.

Allowed crates if needed:
- `alloy-primitives`
- `alloy-json-rpc`
- `alloy-provider`
- `alloy-rpc-types`
- `alloy-transport-http`
- `alloy-contract`
- `url`

Use the minimal dependency set.

Do not add:
- ethers-rs unless strongly justified
- TypeScript
- Python
- Node.js
- frontend code

## Config

Add or extend `.env.example`:

```env
EXECUTION_ENABLED=false
EXECUTOR_DRY_RUN=true
EXECUTOR_POLL_INTERVAL_MS=1000
EXECUTOR_MAX_BATCH_SIZE=10

RPC_URL=
PERP_MATCHING_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000
PERP_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000

Rules:

EXECUTION_ENABLED=false means executor does nothing.
EXECUTION_ENABLED=true with EXECUTOR_DRY_RUN=true means executor may process pending intents in dry-run mode only.
EXECUTOR_DRY_RUN=false must be rejected for now with a clear error because real transaction sending is not implemented in this task.
No private key env vars in this task.
Required Modules

Add or extend modules such as:

src/execution/config.rs
src/execution/executor.rs
src/execution/status.rs
src/execution/runner.rs
src/execution/tx_builder.rs

Adapt names if cleaner.

Execution Intent Status Lifecycle

Current status:

pending

Add or support statuses:

pending
dry_run
submitted
confirmed
failed

In this task:

executor may move pending -> dry_run
executor must not move to submitted
executor must not move to confirmed
failed may be used only for local dry-run/scaffold errors
Repository Requirements

Extend persistence repository with methods:

fetch pending execution intents with limit
update execution intent status
optionally append engine/executor event

Example methods:

list_pending_execution_intents(limit: u32)
update_execution_intent_status(intent_id, status, updated_at_ms)

Keep existing behavior unchanged when persistence is disabled.

Executor Behavior

When EXECUTION_ENABLED=false:

executor does not start
API and matching continue working normally

When EXECUTION_ENABLED=true and EXECUTOR_DRY_RUN=true:

executor can start a background loop
it fetches pending intents from DB if persistence is enabled
it logs what would be executed
it may update intent status to dry_run
it does not call contracts
it does not sign
it does not broadcast

When EXECUTION_ENABLED=true and EXECUTOR_DRY_RUN=false:

startup must fail with a clear error:
real on-chain execution is not implemented yet

If persistence is disabled and execution is enabled:

either reject startup or run a no-op dry-run executor
prefer rejecting startup with a clear error:
executor requires persistence enabled
HTTP API Requirements

Add executor/status endpoint:

GET /executor/status

Return:

{
  "executionEnabled": false,
  "dryRun": true,
  "realBroadcastEnabled": false,
  "persistenceRequired": true
}

Add endpoint if simple:

POST /executor/tick

Purpose:

manually process one executor dry-run tick
useful for tests/local dev
only allowed in dry-run mode
no real tx

If this adds too much scope, implement only /executor/status.

Transaction Builder Scaffold

Create a boundary that will later build PerpMatchingEngine calldata.

For now, it should expose a function like:

build_perp_execution_call(intent: &ExecutionIntent) -> Result<PreparedExecutionCall>

PreparedExecutionCall should contain:

target contract address
intent id
market id
buyer
seller
price_1e8
size_1e8
placeholder calldata bytes or empty bytes
clear flag is_placeholder: true

Do not fake ABI encoding.

The function must clearly document that calldata is placeholder until contract ABI integration.

Tests Required

Normal cargo test must not require RPC or Postgres.

Add tests for:

config parsing:
execution disabled
dry-run enabled
real execution disabled/rejected
executor status response
tx builder scaffold produces placeholder call
dry-run tick with mocked/in-memory repository if feasible
execution enabled without persistence rejected if implemented
existing matching/API/persistence config tests still pass

Do not make normal tests depend on:

Base Sepolia RPC
Postgres
private keys
Documentation Requirements

Update README.md:

explain executor scaffold
explain dry-run mode
explain that real broadcast is not implemented
explain required env vars
explain why persistence is required before execution

Update ARCHITECTURE.md:

describe execution intent lifecycle
describe executor V1 boundary
describe future production executor responsibilities:
contract ABI integration
simulation
nonce manager
gas manager
signing
broadcast
tx/event reconciliation
Constraints

Do not add:

real transaction sending
private key loading
production executor
fake ABI encoding
fake transaction confirmation
frontend
TypeScript
Python
Node.js
Solidity changes

Do not modify:

~/DEOPT/deoptv2

Do not introduce:

floating point financial math
matching semantic changes
fake on-chain settlement
fake finality
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

The task is complete only if:

executor config exists
/executor/status works
real broadcast remains impossible
dry-run scaffold exists
pending execution intents can be fetched for executor processing when persistence is enabled
intent status can be updated to dry_run
tx builder boundary exists but does not fake ABI encoding
normal tests do not require RPC/Postgres
all validation commands pass
no blockchain transaction execution is added
EOF


after all, run thelegram.py (say nothing, just run and that all)