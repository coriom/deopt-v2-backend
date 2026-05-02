# DeOpt v2 Rust Trading Backend

Phase 1 Rust backend for DeOpt v2 trading infrastructure. This service provides an in-memory perp orderbook, deterministic matching, a thin HTTP API, RFQ/MM scaffolds, an execution-intent queue, and a dry-run PerpMatchingEngine calldata builder boundary.

Smart contracts remain the final source of truth. This backend does not submit transactions, sign payloads, load private keys, call RPC endpoints, or claim final settlement.

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
RPC_URL=
PERP_MATCHING_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000
PERP_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000
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

## PerpMatchingEngine Calldata

The execution module can ABI encode `PerpMatchingEngine.executeTrade((address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)` with `alloy-sol-types` when given an explicit `PerpTradePayload` and explicit buyer/seller trade signatures.

The order signatures accepted by `POST /orders` are not PerpTrade signatures. The Solidity `PerpMatchingEngine` verifies signatures over the final matched `PerpTrade`, so the builder never reuses order signatures as trade signatures and never fabricates buyer or seller signatures. If signatures are missing, the builder produces a non-executable preview with empty calldata and `missing_signatures=true`.

After a match, clients can fetch the exact EIP-712 trade payload:

```sh
curl http://127.0.0.1:8080/execution-intents/<intent_id>/signing-payload
```

The response includes the `DeOptV2-PerpMatchingEngine` domain, `PerpTrade` type fields, message fields, and digest. The trade message uses the matched buyer/seller, market, size, execution price, buyer maker flag, buyer/seller order nonces, and the minimum original order deadline. If an old or direct in-memory intent lacks nonce/deadline metadata, the endpoint returns a clear error instead of inventing values.

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

Broadcast remains disabled. The prepared call always has `is_broadcastable=false` and `value=0`; no RPC simulation, signing, private key loading, transaction submission, or confirmation tracking exists in this phase.

## Persistence

PostgreSQL persistence is opt-in:

```text
PERSISTENCE_ENABLED=true
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
```

When enabled, the service connects to Postgres at startup and runs migrations from `migrations/`. The first migration creates `used_nonces`, `orders`, `trades`, `execution_intents`, and `engine_events`.

One local setup option:

```sh
createdb deopt_v2_backend
cargo run
```

If your local Postgres uses a different user, password, host, or database name, set `DATABASE_URL` accordingly. Persistence is required before production executor usage so used nonces and pending execution intents survive restarts, but this repository still does not submit transactions.

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
- PerpMatchingEngine calldata can be encoded only from complete matched trade payloads and explicit buyer/seller PerpTrade signatures.
- No blockchain RPC, transaction signing, production auth, WebSocket API, or options matching.

## Deferred Execution Work

- Add RPC simulation with `eth_call`.
- Add transaction signing and broadcast behind explicit production safety controls.
- Reconcile submitted transactions through an indexer before marking execution confirmed.
