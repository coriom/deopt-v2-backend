# NEXT_TASK.md — Option First Live Broadcast V1M

## Context

Option live broadcast preflight V1L passed.

Confirmed:

- OptionMatchingEngine:
  0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
- Active on-chain optionId:
  24145907678156652148089862289363692212069910767044828147380657249455352740183
- Buyer:
  0xc0A76c2A6c6b70C0B065A05E64417886416cc976
- Seller:
  0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3
- Executor:
  0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
- OptionMatchingEngine.isExecutor(executor) = true
- Strict signatures accepted.
- Live simulation returned simulation_ok.
- Broadcast endpoint rejects cleanly while disabled.
- No tx rows were created during preflight.

## Goal

Perform the first live option execution broadcast on Base Sepolia.

This task is allowed to send exactly one option execution transaction if all gates are explicitly enabled and simulation_ok is freshly reproduced.

## Hard Safety Rules

Do not call `/executor/broadcast`.

Do not use the generic executor broadcast endpoint.

Do not submit more than one option execution transaction.

Do not retry automatically after a failed broadcast.

Do not create fake tx hashes.

Do not mark confirmed/reconciled.

Do not modify Solidity.

Do not modify frontend.

Do not deploy.

Do not commit.

Do not push.

Never print private keys.

## Required Operator Confirmation

Before doing anything, verify process env has:

```text
RPC_URL set
BUYER_PRIVATE_KEY set
SELLER_PRIVATE_KEY set
EXECUTOR_PRIVATE_KEY set

Derive public addresses only:

cast wallet address --private-key "$BUYER_PRIVATE_KEY"
cast wallet address --private-key "$SELLER_PRIVATE_KEY"
cast wallet address --private-key "$EXECUTOR_PRIVATE_KEY"

Abort if derived addresses do not match expected buyer/seller/executor.

Required Broadcast Flags

For this task only, backend must be started with:

OPTION_EXECUTION_BROADCAST_ENABLED=true
EXECUTION_ENABLED=true
EXECUTOR_REAL_BROADCAST_ENABLED=true
EXECUTOR_DRY_RUN=false

Also required:

OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true
OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0
OPTION_EXECUTION_SIGNATURE_MODE=strict
OPTION_NONCE_SYNC_ENABLED=true
OPTION_NONCE_SYNC_STRICT=true
OPTION_EXECUTION_SIMULATION_ENABLED=true
OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true
Runtime Flow
Record TEST_START_MS.
Start backend with all required env vars.
Verify /health.
Verify /admin/config:
option_execution_broadcast_enabled=true
execution_enabled=true
real_broadcast_enabled=true
executor_private_key=true
rpc=true
no raw secrets
Verify executor still allowed:
cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "isExecutor(address)(bool)" \
  "<EXECUTOR_ADDRESS>" \
  --rpc-url "$RPC_URL"

Expected:

true
Create backend option series mapped to active on-chain optionId:
24145907678156652148089862289363692212069910767044828147380657249455352740183
Create crossing option orders.
Create option execution intent.
Fetch signing payload.
Sign buyer and seller with sign_option_execution_intent.
Submit strict signatures.
Fetch calldata.
Simulate.
Require simulation_ok.

If simulation is not ok, abort. Do not broadcast.

Call:
curl -X POST http://127.0.0.1:8080/options/execution-intents/<INTENT_ID>/broadcast
Capture:
tx_hash
option_execution_transaction id
intent status
from
to
created_at_ms
Verify DB:
SELECT COUNT(*) FROM option_execution_transactions WHERE created_at_ms >= <TEST_START_MS>;
SELECT COUNT(*) FROM execution_transactions WHERE created_at_ms >= <TEST_START_MS>;

Expected:

option_execution_transactions = 1
execution_transactions = 0
Verify idempotency by calling broadcast endpoint a second time only if safe and specified by implementation:
must return existing tx metadata
must not send another tx
option_execution_transactions count remains 1

If uncertain, skip idempotency runtime and rely on tests.

Do not cleanup the broadcasted intent/transaction row. Preserve evidence.
Stop backend.
Post-Broadcast On-chain Read Checks

Do read-only checks only.

If transaction hash exists, inspect receipt with safe command if available:

cast receipt <TX_HASH> --rpc-url "$RPC_URL"

Do not implement indexer/confirmation here.

Do not mark confirmed.

Expected Result

Acceptable:

broadcast_submitted
real tx_hash persisted
option_execution_transactions = 1
execution_transactions = 0

Not acceptable:

fake tx hash
generic execution_transactions row
confirmed/reconciled status
multiple tx submissions
broadcast without simulation_ok
Output Doc

Create or update:

docs/OPTION_FIRST_LIVE_BROADCAST.md

Include:

intent id
option id
buyer/seller/executor addresses
simulation_ok block
tx hash
DB transaction row summary
broadcast flags used
no generic executor mutation proof
receipt summary if available
next steps: indexer/reconciliation/confirmation
If Bug Found

Patch minimally only if the bug blocks safe execution.

If code changed:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build

If no code changed:

cargo fmt --check
cargo build
Final Report

Return:

files changed
code patch needed
derived buyer/seller/executor addresses
backend startup/admin config result
simulation_ok result
broadcast result
tx hash
option_execution_transactions count
execution_transactions count
idempotency result if tested
receipt result if checked
no forbidden mutation result
docs updated
validation commands run
remaining blocker
