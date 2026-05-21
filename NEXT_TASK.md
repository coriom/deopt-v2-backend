# NEXT_TASK.md — Option Live Broadcast Preflight V1L

## Context

Option execution pre-broadcast path is now fully verified.

Confirmed:

- OptionMatchingEngine deployed:
  0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
- MarginEngine.matchingEngine points to OptionMatchingEngine.
- Option series active.
- Backend computes registry-compatible optionId.
- Backend rejects metadata mismatches before execution.
- Valid buyer/seller EIP-712 signatures accepted in strict mode.
- Live Base Sepolia eth_call returns simulation_ok.
- No option execution transaction has been broadcast yet.

Current goal:

Prepare the first live option execution broadcast without performing it.

## Goal

Perform a live broadcast preflight.

Verify all runtime prerequisites for a real option execution broadcast, but do not enable all broadcast gates simultaneously and do not send a transaction.

## Non-Goals

Do not perform live broadcast.
Do not submit transactions.
Do not call /executor/broadcast.
Do not create execution_transactions.
Do not modify Solidity.
Do not modify frontend.
Do not deploy.
Do not commit.
Do not push.
Do not print private keys.

## Safety Rules

Keep real broadcast disabled in this task:

```env
OPTION_EXECUTION_BROADCAST_ENABLED=false
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false

Private keys may be checked only by deriving public addresses.

Never print private keys.

No tx hash fabrication.

Required Checks
1. Environment alignment

Verify without printing secrets:

RPC_URL set
BUYER_PRIVATE_KEY set
SELLER_PRIVATE_KEY set
EXECUTOR_PRIVATE_KEY set or explicitly absent

If EXECUTOR_PRIVATE_KEY is set, derive address:

cast wallet address --private-key "$EXECUTOR_PRIVATE_KEY"

Record only the address.

If EXECUTOR_PRIVATE_KEY is absent, report it as the remaining blocker for live broadcast.

2. Executor authorization

For the derived executor address, check:

cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "isExecutor(address)(bool)" \
  "<EXECUTOR_ADDRESS>" \
  --rpc-url "$RPC_URL"

Expected before live broadcast:

true

If false, document manual-only command:

cast send 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "setExecutor(address,bool)" \
  "<EXECUTOR_ADDRESS>" true \
  --rpc-url "$RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"

Do not run it unless explicitly approved by the human.

3. Reproduce simulation_ok

Start backend with:

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
OPTION_EXECUTION_SIMULATION_FROM=<EXECUTOR_ADDRESS_OR_AUTHORIZED_SIMULATION_ADDRESS>

OPTION_EXECUTION_BROADCAST_ENABLED=false
OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true
OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0

EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false

Use active on-chain optionId:

24145907678156652148089862289363692212069910767044828147380657249455352740183

Flow:

create backend option series
create crossing option orders
create option execution intent
fetch signing payload
sign buyer/seller with sign_option_execution_intent
submit signatures
fetch calldata
simulate
confirm simulation_ok
4. Broadcast endpoint must reject while disabled

Call:

POST /options/execution-intents/:intent_id/broadcast

Expected:

configuration error: option execution broadcast is disabled

Verify no rows:

SELECT COUNT(*) FROM option_execution_transactions WHERE created_at_ms >= <TEST_START_MS>;
SELECT COUNT(*) FROM execution_transactions WHERE created_at_ms >= <TEST_START_MS>;

Expected:

0
0
5. Config gate matrix

Document exact flags required for real broadcast:

OPTION_EXECUTION_BROADCAST_ENABLED=true
EXECUTION_ENABLED=true
EXECUTOR_REAL_BROADCAST_ENABLED=true
EXECUTOR_PRIVATE_KEY=<set locally only>
RPC_URL=<set locally only>
OPTION_MATCHING_ENGINE_ADDRESS=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b

Do not run with all three broadcast gates true in this task.

6. Manual broadcast command plan

Document the exact backend command or API call that will be used later.

Expected later API call:

curl -X POST http://127.0.0.1:8080/options/execution-intents/<INTENT_ID>/broadcast

Only after the human explicitly starts backend with all broadcast flags enabled.

Output File

Create:

docs/OPTION_LIVE_BROADCAST_PREFLIGHT.md

Include:

executor private key derived address, if available
executor authorization status
simulation_ok reproduction result
broadcast disabled rejection result
transaction table counts
exact broadcast flag matrix
manual live broadcast procedure
remaining blocker
Cleanup

Delete backend runtime rows only:

option_execution_transactions if any
option_execution_intents
option_fills
option_orders
option_series

Stop backend.

If Bug Found

Patch minimally only.

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
whether code patch was needed
executor private key derived address or absent
executor authorization status
simulation_ok result
broadcast disabled rejection result
option_execution_transactions count
execution_transactions count
no forbidden mutation verification
doc summary
cleanup result
validation commands run
remaining blocker
