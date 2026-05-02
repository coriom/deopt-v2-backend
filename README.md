# DeOpt v2 Rust Trading Backend

Phase 1 Rust backend for DeOpt v2 trading infrastructure. This service provides an in-memory perp orderbook, deterministic matching, a thin HTTP API, RFQ/MM scaffolds, and an execution-intent queue.

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
SIGNATURE_VERIFICATION_MODE=disabled
PERSISTENCE_ENABLED=false
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
EIP712_NAME=DeOptV2
EIP712_VERSION=1
EIP712_CHAIN_ID=84532
EIP712_VERIFYING_CONTRACT=0x0000000000000000000000000000000000000000
```

`EXECUTION_ENABLED=false` is intentional for this phase.
`PERSISTENCE_ENABLED=false` keeps the default local in-memory behavior and does not require Postgres.

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
- No blockchain RPC, ABI encoding, transaction signing, production auth, WebSocket API, or options matching.
