# DeOpt v2 Rust Trading Backend

Phase 1 Rust backend for DeOpt v2 trading infrastructure. This service provides an in-memory perp orderbook, deterministic matching, a thin HTTP API, RFQ/MM scaffolds, an execution-intent queue, a dry-run PerpMatchingEngine calldata builder boundary, manual RPC simulation for calldata-ready intents, an explicitly gated real broadcast path, and an opt-in Indexer V1 for PerpMatchingEngine events.

Smart contracts remain the final source of truth. By default this backend does not submit transactions, sign transaction payloads, load private keys, or claim final settlement. Optional simulation uses `eth_call` only and never broadcasts. Real transaction signing and `eth_sendRawTransaction` are available only when `EXECUTOR_REAL_BROADCAST_ENABLED=true` and all required signer, fee, RPC, persistence, signature, and simulation gates pass.

## Run

```sh
cp .env.example .env
cargo run
```

Defaults:

```text
HOST=127.0.0.1
PORT=8080
RUST_LOG=info
CHAIN_ID=84532
NETWORK_NAME=base-sepolia
EXECUTION_ENABLED=false
EXECUTOR_DRY_RUN=true
EXECUTOR_POLL_INTERVAL_MS=1000
EXECUTOR_MAX_BATCH_SIZE=10
SIMULATION_ENABLED=false
SIMULATION_REQUIRE_PERSISTENCE=true
RPC_URL=
EXECUTOR_FROM_ADDRESS=0x0000000000000000000000000000000000000000
PERP_MATCHING_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000
PERP_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000
INDEXER_ENABLED=false
INDEXER_START_BLOCK=0
INDEXER_POLL_INTERVAL_MS=3000
INDEXER_MAX_BLOCK_RANGE=500
INDEXER_REQUIRE_PERSISTENCE=true
SIGNATURE_VERIFICATION_MODE=disabled
PERSISTENCE_ENABLED=false
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
EIP712_NAME=DeOptV2
EIP712_VERSION=1
EIP712_CHAIN_ID=84532
EIP712_VERIFYING_CONTRACT=0x0000000000000000000000000000000000000000
```

`EXECUTION_ENABLED=false` is intentional for this phase.
`EXECUTOR_DRY_RUN=false` is rejected because real on-chain execution is not implemented.
`PERSISTENCE_ENABLED=false` keeps the default local in-memory behavior and does not require Postgres.
`EXECUTOR_REAL_BROADCAST_ENABLED=false` is the safe default.
`EXECUTOR_REAL_BROADCAST_ENABLED=true` requires `PERSISTENCE_ENABLED=true`, `EXECUTOR_PRIVATE_KEY`, `RPC_URL`, `EXECUTOR_MAX_FEE_PER_GAS_WEI`, `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI`, a nonzero `EXECUTOR_CHAIN_ID`, and a nonzero `EXECUTOR_MAX_GAS_LIMIT`.
`SIMULATION_ENABLED=true` requires `RPC_URL`; when `SIMULATION_REQUIRE_PERSISTENCE=true`, it also requires `PERSISTENCE_ENABLED=true`.
`INDEXER_ENABLED=true` requires `RPC_URL`; when `INDEXER_REQUIRE_PERSISTENCE=true`, it also requires `PERSISTENCE_ENABLED=true`.

## PerpMatchingEngine Calldata

The execution module can ABI encode `PerpMatchingEngine.executeTrade((bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)` with `alloy-sol-types` when given an explicit `PerpTradePayload` and explicit buyer/seller trade signatures.

The Solidity `PerpTrade.intentId` is derived from the backend UUID string as `keccak256(bytes(execution_intents.intent_id))` and exposed as `0x` plus 64 lowercase hex chars. This mapping is deterministic, non-random, and reused by the signing payload, calldata builder, and indexed-event reconciliation.

The order signatures accepted by `POST /orders` are not PerpTrade signatures. The Solidity `PerpMatchingEngine` verifies signatures over the final matched `PerpTrade`, so the builder never reuses order signatures as trade signatures and never fabricates buyer or seller signatures. If signatures are missing, the builder produces a non-executable preview with empty calldata and `missing_signatures=true`.

After a match, clients can fetch the exact EIP-712 trade payload:

```sh
curl http://127.0.0.1:8080/execution-intents/<intent_id>/signing-payload
```

The response includes the `DeOptV2-PerpMatchingEngine` domain, `PerpTrade` type fields, message fields, and digest. The trade message uses `intentId` first, then the matched buyer/seller, market, size, execution price, buyer maker flag, buyer/seller order nonces, and the minimum original order deadline. If an old or direct in-memory intent lacks nonce/deadline metadata, the endpoint returns a clear error instead of inventing values.

Clients submit matched-trade signatures separately:

```sh
curl -X POST http://127.0.0.1:8080/execution-intents/<intent_id>/signatures \
  -H 'content-type: application/json' \
  -d '{
    "buyer_sig": "0x...",
    "seller_sig": "0x..."
  }'
```

Signatures are accepted only as `0x` plus 65-byte hex strings. They are stored in memory by default and in `execution_intent_signatures` when persistence is enabled. `calldata_ready=true` only when both buyer and seller trade signatures are present and the corresponding intent has complete PerpTrade metadata.

Broadcast remains disabled by default. The prepared call always has `is_broadcastable=false` and `value=0`; no signing, private key retention, transaction submission, or confirmation tracking exists in the default path.

## Development PerpTrade Signing

The `sign_perp_trade` binary is a local development helper for Base Sepolia and throwaway test wallets only. Never use it with production keys, never commit `.env` files or private keys, and do not treat it as a production custody model. The backend server does not automatically sign user trades.

Fetch the backend-provided EIP-712 payload:

```sh
export INTENT_ID=<intent_id>
curl http://127.0.0.1:8080/execution-intents/$INTENT_ID/signing-payload \
  > /tmp/perp_trade_payload.json
```

Sign as the buyer. `BUYER_PRIVATE_KEY` takes precedence over `SIGNER_PRIVATE_KEY`:

```sh
BUYER_PRIVATE_KEY=0x... \
cargo run --bin sign_perp_trade -- \
  --payload /tmp/perp_trade_payload.json \
  --role buyer
```

Sign as the seller. `SELLER_PRIVATE_KEY` takes precedence over `SIGNER_PRIVATE_KEY`:

```sh
SELLER_PRIVATE_KEY=0x... \
cargo run --bin sign_perp_trade -- \
  --payload /tmp/perp_trade_payload.json \
  --role seller
```

The CLI signs the `digest` returned by the backend payload and outputs JSON only by default:

```json
{
  "role": "buyer",
  "signer_address": "0x...",
  "signature": "0x..."
}
```

By default it rejects a buyer key that does not derive to `message.buyer` and a seller key that does not derive to `message.seller`. `--allow-address-mismatch` exists only for explicit debugging. `--verbose` keeps stdout as JSON and adds the digest, domain, and message intent id.

Submit both signatures:

```sh
curl -X POST http://127.0.0.1:8080/execution-intents/$INTENT_ID/signatures \
  -H "Content-Type: application/json" \
  -d '{"buyer_sig":"0x...","seller_sig":"0x..."}'
```

Then simulate:

```sh
curl -X POST http://127.0.0.1:8080/executor/simulate/$INTENT_ID
```

## RPC Simulation

Manual simulation is opt-in:

```text
SIMULATION_ENABLED=true
SIMULATION_REQUIRE_PERSISTENCE=true
RPC_URL=https://...
EXECUTOR_FROM_ADDRESS=0x0000000000000000000000000000000000000000
PERP_MATCHING_ENGINE_ADDRESS=0x...
PERSISTENCE_ENABLED=true
```

`POST /executor/simulate/<intent_id>` loads one execution intent, requires both stored PerpTrade signatures, rebuilds the real `executeTrade` calldata, and performs an `eth_call` to `PERP_MATCHING_ENGINE_ADDRESS` with `value=0`. On success, the intent is marked `simulation_ok`; on revert or RPC failure, it is marked `simulation_failed` with the error text and any decoded revert diagnostics. These statuses only describe the result of the call simulation at the queried block. They do not mean submitted, confirmed, final, or executed.

Simulation failure responses include diagnostic fields when the RPC returns revert data:

```json
{
  "simulation_status": "simulation_failed",
  "error": "simulation failed: execution reverted",
  "revert_data": "0x...",
  "revert_selector": "0x...",
  "decoded_error": {
    "kind": "custom_error",
    "name": "InvalidSignature",
    "selector": "0x...",
    "args": []
  },
  "submitted": false,
  "confirmed": false
}
```

The decoder supports Solidity `Error(string)`, `Panic(uint256)`, unknown custom-error selectors, and a table of common protocol errors such as `InvalidSignature`, `NotAuthorized`, `InsufficientMargin`, `MarketCloseOnly`, `OracleStale`, `OraclePriceUnavailable`, `InvalidPrice`, and `InvalidSize`. If an RPC provider returns only a message, `decoded_error.kind` is `missing_revert_data` and the raw message is preserved. These diagnostics are persisted in `execution_simulations` as `revert_data`, `revert_selector`, and `decoded_error`.

The endpoint returns `submitted=false` and `confirmed=false` for every response. Simulation does not call `eth_sendRawTransaction` and `GET /executor/status` reports `broadcastEnabled=false` until real broadcast is explicitly enabled.

## Real Broadcast V1

Broadcast V1 is disabled by default and only submits when explicitly enabled:

```text
EXECUTOR_REAL_BROADCAST_ENABLED=false
EXECUTOR_PRIVATE_KEY=
EXECUTOR_CHAIN_ID=84532
EXECUTOR_MAX_GAS_LIMIT=1000000
EXECUTOR_MAX_FEE_PER_GAS_WEI=
EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI=
EXECUTOR_REQUIRE_SIMULATION_OK=true
```

`POST /executor/broadcast/<intent_id>` returns a clear disabled response while `EXECUTOR_REAL_BROADCAST_ENABLED=false`. It does not sign, call `eth_sendRawTransaction`, fabricate a tx hash, or mark the intent submitted. Transaction request construction requires a complete matched trade, both PerpTrade signatures, non-empty `executeTrade` calldata, a configured `PERP_MATCHING_ENGINE_ADDRESS`, and `simulation_ok` when `EXECUTOR_REQUIRE_SIMULATION_OK=true`.

When `EXECUTOR_REAL_BROADCAST_ENABLED=true`, startup validates the private key shape, RPC URL, static EIP-1559 fee fields, chain id, and gas limit. Broadcast fetches `eth_chainId`, rejects mismatches before signing, fetches the executor pending nonce with `eth_getTransactionCount`, signs a type `0x02` EIP-1559 transaction in-process, and submits only with `eth_sendRawTransaction`. The API records `submitted` only after the RPC returns a real tx hash, then marks the execution intent `submitted`. It never returns `confirmed=true` and never marks an intent confirmed.

Private keys are held only in the execution config secret wrapper and signer object; their `Debug` output is redacted. The API never returns raw transactions or private keys.

When persistence is enabled, transaction records can be read with:

```sh
curl http://127.0.0.1:8080/executor/transactions
curl http://127.0.0.1:8080/executor/transactions/<intent_id>
```

The database stores transaction attempts with statuses `prepared`, `rejected`, `submitted`, and `failed`; it does not include `confirmed`. `submitted` means only that `eth_sendRawTransaction` returned a syntactically valid transaction hash. It does not prove inclusion, execution success, backend ownership, finality, or absence of reorgs. Confirmation requires later indexer, reconciliation, ownership, and finality checks. If the RPC send succeeds but persistence fails immediately afterward, the chain may still have received the transaction; this V1 does not provide atomic RPC-plus-database semantics.

## Indexer V1

Indexer V1 is opt-in and read-only:

```text
INDEXER_ENABLED=true
INDEXER_START_BLOCK=0
INDEXER_POLL_INTERVAL_MS=3000
INDEXER_MAX_BLOCK_RANGE=500
INDEXER_REQUIRE_PERSISTENCE=true
RPC_URL=https://...
PERP_MATCHING_ENGINE_ADDRESS=0x...
PERSISTENCE_ENABLED=true
```

It reads `eth_getLogs` for `PerpMatchingEngine.TradeExecuted`, decodes the event, stores rows in `indexed_perp_trades`, and advances the `perp_matching_engine` cursor only after persistence succeeds. The Solidity event now emits indexed `intentId`, which the backend stores as `indexed_perp_trades.onchain_intent_id`. Manual control and reads are exposed through:

```sh
curl http://127.0.0.1:8080/indexer/status
curl -X POST http://127.0.0.1:8080/indexer/tick
curl http://127.0.0.1:8080/indexed/perp-trades
```

Indexed events do not mark execution intents submitted or confirmed. Direct reconciliation can compare `keccak256(bytes(execution_intents.intent_id))` with `indexed_perp_trades.onchain_intent_id`; economic match keys are only a fallback for historical data without an intent id. V1 stores `block_hash` when the RPC provides it, but does not implement deep reorg rollback.

## Reconciliation V1

Reconciliation V1 is opt-in, persistence-backed, and read-only with respect to execution intent lifecycle:

```text
RECONCILIATION_ENABLED=false
RECONCILIATION_REQUIRE_PERSISTENCE=true
RECONCILIATION_MAX_BATCH_SIZE=100
```

It links indexed `TradeExecuted` events to backend intents by direct bytes32 identity:

```text
execution_intents.onchain_intent_id == indexed_perp_trades.onchain_intent_id
```

An exact unique match writes an `execution_reconciliations` row with `status=matched`. Missing backend intents are counted as unmatched without inventing ownership. Multiple backend intents or duplicate indexed events for the same on-chain intent id are treated as ambiguous. Reconciliation rows include the indexed event id, tx hash, block number, and log index, but they do not prove this backend submitted the transaction.

Manual control and reads:

```sh
curl http://127.0.0.1:8080/reconciliation/status
curl -X POST http://127.0.0.1:8080/reconciliation/tick
curl http://127.0.0.1:8080/reconciliations
curl http://127.0.0.1:8080/reconciliation/intents/<intent_id>
```

Reconciliation does not mark intents submitted or confirmed. Status and tick responses always return `confirmed=0`; final confirmation is deferred until a future executor can prove tx ownership and finality with reorg-aware indexing.

## Persistence

PostgreSQL persistence is opt-in:

```text
PERSISTENCE_ENABLED=true
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
```

When enabled, the service connects to Postgres at startup and runs migrations from `migrations/`. Migrations create `used_nonces`, `orders`, `trades`, `execution_intents`, `execution_intent_signatures`, `execution_simulations`, `engine_events`, `indexer_cursors`, `indexed_perp_trades`, `execution_reconciliations`, and `execution_transactions`.

One local setup option:

```sh
createdb deopt_v2_backend
cargo run
```

If your local Postgres uses a different user, password, host, or database name, set `DATABASE_URL` accordingly. Persistence is required before real broadcast usage so transaction records and submitted intent status survive restarts.

## Test

```sh
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

## API Examples

```sh
curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/markets
curl http://127.0.0.1:8080/orderbook/1
curl http://127.0.0.1:8080/execution-intents
```

Submit a limit order. Financial fixed-point values are strings at the HTTP boundary:

```sh
curl -X POST http://127.0.0.1:8080/orders \
  -H 'content-type: application/json' \
  -d '{
    "market_id": 1,
    "account": "0xmaker",
    "side": "sell",
    "price_1e8": "300000000000",
    "size_1e8": "100000000",
    "time_in_force": "gtc",
    "reduce_only": false,
    "post_only": false,
    "client_order_id": "maker-1",
    "nonce": 1,
    "deadline_ms": 4102444800000,
    "signature": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  }'
```

Cancel an open order:

```sh
curl -X DELETE http://127.0.0.1:8080/orders/<order_id>
```

## Current Limitations

- Default mode is in-memory; restarting clears orders and execution intents unless `PERSISTENCE_ENABLED=true`.
- Perp limit orders only.
- Public API financial quantities are string-encoded fixed-point integers.
- `POST /orders` uses a signed-order payload with nonce, deadline, and signature fields.
- `SIGNATURE_VERIFICATION_MODE=disabled` validates nonce, deadline, and signature shape while skipping cryptographic recovery.
- `SIGNATURE_VERIFICATION_MODE=strict` verifies the EIP-712 order digest and recovered secp256k1 signer against `account`.
- FOK is rejected cleanly.
- RFQ and market-maker gateway are type scaffolds only.
- Execution intents are provisional off-chain records, not settlement.
- Indexed `TradeExecuted` events store `onchain_intent_id` for direct reconciliation only; they do not confirm backend intents.
- Reconciliation rows link indexed events to intents, but still do not prove transaction ownership, finality, or reorg safety.
- Real broadcast is disabled by default; when enabled it submits only with a real signed raw transaction and never returns fake tx hashes.
- Indexer V1 stores block hashes when available but does not implement deep reorg rollback.
- PerpMatchingEngine calldata can be encoded only from complete matched trade payloads and explicit buyer/seller PerpTrade signatures.
- Optional blockchain RPC includes manual `eth_call` simulation, opt-in indexing, and explicitly gated `eth_sendRawTransaction` broadcast. No production auth, WebSocket API, or options matching.

## Deferred Execution Work

- Reconcile submitted transactions through an indexer before marking execution confirmed.
- Add receipt polling, transaction ownership proofs, gas estimation, fee discovery, retries, nonce reservation, and reorg-aware confirmation.
