# NEXT_TASK.md — Option First Successful Live Broadcast V1P

## Context

The first live option execution broadcast on Base Sepolia failed because the real transaction gas cap was too low.

Previous failed tx:
0xe832365b11ead105e020b65a25570516e87ab1af2b1225b698561f090eff8b7c

Root cause:
- uncapped simulation succeeded
- broadcast gas limit was `1000000`
- `eth_estimateGas` was approximately `1040080`
- tx failed with top-level `OutOfGas`

A backend gas safety patch has now been implemented.

New behavior:
- backend calls `eth_estimateGas` for the exact option tx
- backend computes `required_gas = estimated_gas * OPTION_EXECUTION_GAS_SAFETY_BPS / 10000`
- backend rejects if `broadcast_gas_limit < estimated_gas`
- backend rejects if `broadcast_gas_limit < required_gas`
- gas-check fields are persisted in `option_execution_transactions`
- `/admin/config` exposes `execution_gas_safety_bps`

Important:
The preserved intent `4075afe3-fe42-457d-a9ca-eb0907d09a74` must not be retried. It is already tied to the failed broadcast.

## Goal

Perform one clean successful live option execution broadcast on Base Sepolia using a fresh intent, fresh signatures, fresh simulation, fresh gas estimate, and the new gas safety gate.

This task may perform exactly one real option broadcast if all preflight checks pass.

## Hard Rules

Never print private keys.
Do not retry automatically.
Do not submit more than one real transaction.
Do not call `/executor/broadcast`.
Do not use the generic executor path.
Do not create generic `execution_transactions`.
Do not reuse intent `4075afe3-fe42-457d-a9ca-eb0907d09a74`.
Do not cleanup the previous failed evidence row.
Do not cleanup the new broadcast evidence row.
Do not modify Solidity.
Do not modify frontend.
Do not deploy contracts.
Do not mark confirmed/reconciled in this task.
Do not push unless explicitly asked after final report.

Abort immediately if any required preflight check fails.

## Required Preflight 0 — Git And Validation

In `~/DEOPT/deopt-v2-backend`:

Check:

git status --short
git branch --show-current
git log -1 --oneline
git status -sb

Then run:

cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features

If dirty files exist, document them.

Do not continue to live broadcast if the gas safety patch is not present.

Confirm these exist:

migrations/0022_option_execution_gas_safety.sql
docs/OPTION_BROADCAST_GAS_SAFETY.md
OPTION_EXECUTION_GAS_SAFETY_BPS parsing in config
gas-check fields in option execution broadcast response
Required Preflight 1 — Apply Live DB Migration

Apply migration:

sqlx migrate run

or the project’s standard migration command.

Then verify the live DB has the new columns:

estimated_gas
required_gas
simulation_gas_limit
broadcast_gas_limit
gas_safety_bps
gas_check_status
gas_check_error

on table:

option_execution_transactions

Abort if migration is not applied.

Required Preflight 2 — Reload Env

If terminal or PC was restarted, reload env.

Use project-local .env only.

Do not print private key values.

Required env names:

RPC_URL
DATABASE_URL
BUYER_PRIVATE_KEY
SELLER_PRIVATE_KEY
EXECUTOR_PRIVATE_KEY
BUYER_ADDRESS
SELLER_ADDRESS
EXECUTOR_FROM_ADDRESS
OPTION_MATCHING_ENGINE_ADDRESS

Required flags:

PERSISTENCE_ENABLED=true
OPTIONS_ENABLED=true
OPTION_EXECUTION_ENABLED=true
OPTION_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
OPTION_EXECUTION_SIGNATURE_MODE=strict
OPTION_EXECUTION_CHAIN_ID=84532
OPTION_EXECUTION_EIP712_NAME=DeOptV2-OptionMatchingEngine
OPTION_EXECUTION_EIP712_VERSION=1
OPTION_NONCE_SYNC_ENABLED=true
OPTION_NONCE_SYNC_REQUIRE_RPC=true
OPTION_NONCE_SYNC_STRICT=true
OPTION_EXECUTION_SIMULATION_ENABLED=true
OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true
OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true
OPTION_EXECUTION_BROADCAST_ENABLED=true
EXECUTION_ENABLED=true
EXECUTOR_REAL_BROADCAST_ENABLED=true
EXECUTOR_DRY_RUN=false
OPTION_EXECUTION_GAS_SAFETY_BPS=12500
OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1300000

OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1300000 is the initial intended cap for this run.

If gas estimate + safety margin exceeds 1300000, abort and report the required cap. Do not broadcast.

Required Preflight 3 — Derive Public Addresses Only

Derive public addresses from:

buyer private key
seller private key
executor private key

Print only public addresses.

Expected:

Buyer:    0xc0A76c2A6c6b70C0B065A05E64417886416cc976
Seller:   0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3
Executor: 0xc35F7A8A103A9A4464adfaa76B9B514093D23C27

Abort if any mismatch.

Required Preflight 4 — On-chain Read-only Checks

Use cast call only.

Check executor:

cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "isExecutor(address)(bool)" \
  0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 \
  --rpc-url "$RPC_URL"

Must be true.

Check buyer/seller nonces:

cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "nonces(address)(uint256)" \
  "$BUYER_ADDRESS" \
  --rpc-url "$RPC_URL"

cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "nonces(address)(uint256)" \
  "$SELLER_ADDRESS" \
  --rpc-url "$RPC_URL"

Record both.

Check active series:

cast call 0x3d52b033Fab00ed6104DD3bc0a715F8648344ecA \
  "getSeries(uint256)((address,address,uint64,uint64,bool,uint128,bool,bool,uint256))" \
  24145907678156652148089862289363692212069910767044828147380657249455352740183 \
  --rpc-url "$RPC_URL"

Must show active series with expected metadata.

Check oracle freshness with the project’s known oracle/router calls.

Abort if oracle is stale or unsafe.

If mock feeds are stale, refresh mock feeds first using the existing Solidity script. Then rerun read-only checks.

Required Preflight 5 — Backend Admin Config

Start/restart backend with the env above.

Check sanitized admin config.

Confirm:

option execution enabled
option simulation enabled
option broadcast enabled
real broadcast enabled
dry run false
option matching engine address correct
chain id 84532
EIP-712 name/version correct
gas safety bps 12500
broadcast gas limit 1300000

Do not continue if /admin/config does not reflect expected values.

Required Preflight 6 — DB Baseline

Set:

TEST_START_MS=$(date +%s%3N)

Record current counts:

select count(*) from option_execution_transactions;
select count(*) from execution_transactions;

Also record counts since TEST_START_MS, expected 0.

Required Execution Flow

Create a fresh option execution intent.

Do not reuse:

4075afe3-fe42-457d-a9ca-eb0907d09a74

Use active option:

optionId = 24145907678156652148089862289363692212069910767044828147380657249455352740183

Expected domain:

name: DeOptV2-OptionMatchingEngine
version: 1
chainId: 84532
verifyingContract: 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b

Steps:

Create fresh intent.
Fetch EIP-712 signing payload.
Sign buyer.
Sign seller.
Submit strict signatures.
Fetch calldata.
Simulate.
Require simulation_ok.
Let backend run gas safety check.
Broadcast exactly once via:
POST /options/execution-intents/<NEW_INTENT_ID>/broadcast

Only this endpoint is allowed.

Forbidden:

/executor/broadcast
Required Post-broadcast Checks

After broadcast response:

Collect:

intent id
tx hash
gas check status
estimated gas
required gas
simulation gas limit
broadcast gas limit
gas safety bps

Verify DB:

select count(*) from option_execution_transactions where created_at_ms >= :TEST_START_MS;
select count(*) from execution_transactions where created_at_ms >= :TEST_START_MS;

Expected:

option_execution_transactions since TEST_START_MS = 1
execution_transactions since TEST_START_MS = 0

Check tx receipt read-only:

cast receipt <TX_HASH> --rpc-url "$RPC_URL"

Expected:

status = 1

If status is 0, do not retry. Diagnose only.

If pending, wait/read only. Do not resubmit.

Check tx read-only:

cast tx <TX_HASH> --rpc-url "$RPC_URL"

Verify:

to == OPTION_MATCHING_ENGINE_ADDRESS
from == EXECUTOR_FROM_ADDRESS
selector 0x031f77b3

Do not mark confirmed/reconciled in DB.

Output Doc

Create:

docs/OPTION_FIRST_SUCCESSFUL_LIVE_BROADCAST.md

Include:

summary
env safety flags used, sanitized
migration confirmation
derived public addresses
on-chain preflight checks
oracle freshness result
DB baseline
fresh intent id
simulation result
gas estimate
required gas
broadcast gas limit
gas check status
tx hash
receipt status
DB post-checks
no generic executor verification
no retry statement
whether reconciliation was intentionally deferred
recommended next task
Acceptance Criteria

Complete only if:

migration applied
env verified
public addresses match
executor authorized
oracle fresh
fresh intent used
strict signatures accepted
simulation_ok fresh
gas safety check ok
exactly one option broadcast submitted
receipt status inspected
generic execution_transactions count remains 0
docs created
no retry
no /executor/broadcast
no private keys printed
Final Report

Return:

files changed
migration applied or not
env/config summary
derived address summary
on-chain preflight summary
fresh intent id
simulation result
gas safety result
tx hash
receipt status
DB mutation summary
no forbidden endpoint verification
no retry verification
docs updated
validation commands run
remaining blocker