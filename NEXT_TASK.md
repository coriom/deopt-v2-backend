# NEXT_TASK.md — Runtime Verify Option Execution Nonce Sync V1E

## Context

Option Execution Nonce Sync V1E has been implemented.

Implemented:

- OPTION_NONCE_SYNC_ENABLED=false
- OPTION_NONCE_SYNC_REQUIRE_RPC=true
- OPTION_NONCE_SYNC_STRICT=true
- GET /accounts/:address/option-nonce
- eth_call OptionMatchingEngine.nonces(address)
- option orderbook/RFQ intent creation uses synced nonces when enabled
- signing payload uses stored intent nonces
- calldata uses stored intent nonces
- strict mode aborts intent creation on nonce read failure
- non-strict mode falls back to zero nonces with warning
- tests pass offline

## Goal

Runtime-verify Option Nonce Sync V1E in safe local mode.

Verify:

1. disabled endpoint behavior
2. startup guards
3. enabled no-RPC behavior
4. strict failure behavior
5. non-strict fallback behavior
6. signing payload and calldata use stored nonces
7. no execution transactions
8. no broadcast

## Non-Goals

Do not deploy OptionMatchingEngine.
Do not broadcast.
Do not call /executor/broadcast.
Do not submit transactions.
Do not create execution_transactions.
Do not use private keys.
Do not modify Solidity.
Do not modify frontend.
Do not require live RPC.
Do not commit.
Do not push.

## Safety Rules

Runtime must keep:

```env
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false

No private keys in logs.
No tx hash fabrication.
No submitted/confirmed option lifecycle.

Runtime Setup A — Disabled Nonce Sync

Start backend with process env:

PERSISTENCE_ENABLED=true \
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend \
OPTIONS_ENABLED=true \
OPTIONS_REQUIRE_PERSISTENCE=true \
OPTION_RFQ_ENABLED=true \
OPTION_RFQ_REQUIRE_PERSISTENCE=true \
OPTION_EXECUTION_ENABLED=true \
OPTION_EXECUTION_REQUIRE_PERSISTENCE=true \
OPTION_MATCHING_ENGINE_ADDRESS=0x1111111111111111111111111111111111111111 \
OPTION_EXECUTION_SIGNATURE_MODE=disabled \
OPTION_EXECUTION_CHAIN_ID=84532 \
OPTION_EXECUTION_EIP712_NAME=DeOptV2-OptionMatchingEngine \
OPTION_EXECUTION_EIP712_VERSION=1 \
OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS=6 \
OPTION_NONCE_SYNC_ENABLED=false \
OPTION_NONCE_SYNC_REQUIRE_RPC=true \
OPTION_NONCE_SYNC_STRICT=true \
ADMIN_API_ENABLED=true \
ADMIN_API_REQUIRE_TOKEN=true \
ADMIN_API_TOKEN=local-admin-token-runtime-test \
EXECUTION_ENABLED=false \
EXECUTOR_REAL_BROADCAST_ENABLED=false \
MM_GATEWAY_ENABLED=false \
cargo run --bin deopt-v2-backend

Checks:

curl http://127.0.0.1:8080/health

curl http://127.0.0.1:8080/accounts/0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3/option-nonce

Expected:

health ok
option nonce endpoint returns clear disabled error
Runtime Setup B — Startup Guards
Missing RPC

Start backend with:

OPTION_NONCE_SYNC_ENABLED=true
OPTION_NONCE_SYNC_REQUIRE_RPC=true
RPC_URL=
OPTION_MATCHING_ENGINE_ADDRESS=0x1111111111111111111111111111111111111111

Expected:

startup rejects clearly because RPC_URL is missing
Missing OptionMatchingEngine

Start backend with:

OPTION_NONCE_SYNC_ENABLED=true
OPTION_NONCE_SYNC_REQUIRE_RPC=true
RPC_URL=http://127.0.0.1:8545
OPTION_MATCHING_ENGINE_ADDRESS=

Expected:

startup rejects clearly because OptionMatchingEngine address is missing or invalid
Runtime Setup C — Enabled, Non-Strict, No RPC

Start backend with:

OPTION_NONCE_SYNC_ENABLED=true
OPTION_NONCE_SYNC_REQUIRE_RPC=false
OPTION_NONCE_SYNC_STRICT=false
RPC_URL=
OPTION_MATCHING_ENGINE_ADDRESS=0x1111111111111111111111111111111111111111

Expected:

backend starts
nonce endpoint returns clear RPC unavailable/config error
creating option execution intent falls back to buyer_nonce=0 and seller_nonce=0
signing payload shows nonces 0
calldata uses nonces 0
no execution transaction
Runtime Setup D — Enabled, Strict, No RPC

Start backend with:

OPTION_NONCE_SYNC_ENABLED=true
OPTION_NONCE_SYNC_REQUIRE_RPC=false
OPTION_NONCE_SYNC_STRICT=true
RPC_URL=
OPTION_MATCHING_ENGINE_ADDRESS=0x1111111111111111111111111111111111111111

Expected:

backend starts
creating option execution intent fails clearly because nonce sync cannot read RPC
no partial invalid intent is created
Common Flow for Non-Strict Mode
Record TEST_START_MS.
Create option series with onchain id.
Create crossing option orderbook fill.
Verify option_execution_intent exists.
Verify buyer_nonce=0 and seller_nonce=0 fallback.
Fetch signing payload and verify nonces 0.
Submit dummy shape-valid signatures.
Fetch calldata and verify it exists.
Verify no execution transaction.

Use dummy 65-byte signatures:

{
  "buyer_signature": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "seller_signature": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
}
Common Flow for Strict Mode
Record TEST_START_MS.
Create option series with onchain id.
Try to create crossing fill.
Expect clear failure or fill without execution intent depending on current implementation.
Verify no invalid option_execution_intent exists.
SQL Checks

No execution tx:

SELECT COUNT(*) FROM execution_transactions WHERE created_at_ms >= <TEST_START_MS>;

Inspect runtime intents:

SELECT intent_id, buyer_nonce, seller_nonce, status, error
FROM option_execution_intents
WHERE created_at_ms >= <TEST_START_MS>
ORDER BY created_at_ms;

Duplicate source check:

SELECT source_type, source_id, COUNT(*)
FROM option_execution_intents
WHERE created_at_ms >= <TEST_START_MS>
GROUP BY source_type, source_id
HAVING COUNT(*) > 1;

Expected:

no execution transactions
no duplicate source intents
strict mode creates no invalid intent
Admin / Metrics

Call:

curl http://127.0.0.1:8080/admin/config \
  -H "X-Admin-Token: local-admin-token-runtime-test"

curl http://127.0.0.1:8080/admin/options/summary \
  -H "X-Admin-Token: local-admin-token-runtime-test"

curl http://127.0.0.1:8080/metrics

Expected:

option nonce sync booleans exposed safely if implemented
no secrets
no private keys
metrics safe
Optional Live RPC Check

Only if deployed OptionMatchingEngine and RPC are already available:

set RPC_URL
set OPTION_MATCHING_ENGINE_ADDRESS
set OPTION_NONCE_SYNC_ENABLED=true
call /accounts/:address/option-nonce
verify real nonce returned

If unavailable, report live nonce read deferred.

Cleanup

Delete only runtime-created rows:

option_execution_intents
option_fills
option_orders
option_series
option_rfqs / option_rfq_quotes / option_rfq_fills if used

Stop backend.

Verify:

pgrep -af deopt-v2-backend || true
ss -ltnp | grep ':8080' || true
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
disabled endpoint result
startup guard missing RPC result
startup guard missing matching engine result
non-strict no-RPC fallback result
signing payload nonce result
calldata nonce result
strict no-RPC failure result
no forbidden mutation verification
admin/metrics result
cleanup result
optional live RPC result or deferred reason
validation commands run
remaining blocker