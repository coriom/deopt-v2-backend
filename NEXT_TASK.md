# NEXT_TASK.md — Runtime Verify Option Execution Broadcast V1I Safe Mode

## Context

Option Execution Broadcast V1I has been implemented offline.

Implemented:

- migration 0021_option_execution_broadcast.sql
- dedicated option_execution_transactions table
- config:
  - OPTION_EXECUTION_BROADCAST_ENABLED=false
  - OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true
  - OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0
- endpoint:
  - POST /options/execution-intents/:intent_id/broadcast
- disabled-by-default behavior
- simulation_ok gate
- tx hash persistence only after successful send
- idempotency for submitted tx
- no confirmed/reconciled lifecycle
- mock tests passing

Current goal:

Runtime verify the broadcast endpoint in safe disabled mode.

## Goal

Prove that V1I is safe at runtime before any live broadcast.

Verify:

1. backend starts with broadcast disabled
2. broadcast endpoint rejects when disabled
3. no option_execution_transactions are created
4. no execution_transactions are created
5. no /executor/broadcast is called
6. simulation_ok intent remains unbroadcasted
7. admin/config exposes safe booleans without secrets
8. cleanup works

## Non-Goals

Do not perform live broadcast.
Do not submit transactions.
Do not call /executor/broadcast.
Do not create execution_transactions.
Do not modify Solidity.
Do not modify frontend.
Do not deploy.
Do not use or print private keys.
Do not commit.
Do not push.

## Runtime Config

Start backend with process env only.

Use live Base Sepolia RPC for nonce/simulation, but keep broadcast disabled:

```env
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

OPTION_EXECUTION_BROADCAST_ENABLED=false
OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true
OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0

EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false

Use existing RPC_URL, but do not print it.

Runtime Flow
Record TEST_START_MS.
Start backend.
Verify /health.
Verify /admin/config:
option execution enabled
option nonce sync enabled
option simulation enabled
option broadcast disabled
rpc=true
executor_private_key=false
real_broadcast_enabled=false
no secrets
Create backend option series mapped to active on-chain optionId:
24145907678156652148089862289363692212069910767044828147380657249455352740183
Create crossing option orders to produce:
option fill
option execution intent
Fetch signing payload.
Sign buyer/seller with existing local dev signing flow:
sign_option_execution_intent
BUYER_PRIVATE_KEY / SELLER_PRIVATE_KEY from env
do not print keys
Submit signatures in strict mode.
Fetch calldata.
Run live simulation.
Confirm simulation_ok.
Call:
POST /options/execution-intents/:intent_id/broadcast

Expected:

HTTP 400/403 clear error: option execution broadcast is disabled
Verify no transaction rows:
SELECT COUNT(*) FROM option_execution_transactions WHERE created_at_ms >= <TEST_START_MS>;
SELECT COUNT(*) FROM execution_transactions WHERE created_at_ms >= <TEST_START_MS>;

Expected:

0
0
Verify intent remains not submitted/confirmed.
Cleanup backend runtime rows only:
option_execution_transactions if any failed rows were created
option_execution_intents
option_fills
option_orders
option_series
Stop backend and verify no listener.
Additional Negative Checks

If easy, also verify:

broadcast rejects intent without simulation_ok
broadcast rejects missing intent
broadcast rejects missing calldata/signatures

Do not broaden if it risks muddying the runtime flow.

If Bug Found

Patch minimally only.

After patch:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Final Report

Return:

files changed
whether code patch was needed
backend startup result
admin config result
intent/simulation_ok result
broadcast disabled result
option_execution_transactions count
execution_transactions count
no forbidden mutation verification
cleanup result
validation commands run
remaining blocker
