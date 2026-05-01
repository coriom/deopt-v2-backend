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
```

`EXECUTION_ENABLED=false` is intentional for this phase.

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

- In-memory only; restarting clears orders and execution intents.
- Perp limit orders only.
- Public API financial quantities are string-encoded fixed-point integers.
- `POST /orders` uses a signed-order payload with nonce, deadline, and signature fields.
- `SIGNATURE_VERIFICATION_MODE=disabled` validates nonce, deadline, and signature shape while skipping cryptographic recovery.
- `SIGNATURE_VERIFICATION_MODE=strict` rejects orders explicitly until real EIP-712 recovery is implemented.
- FOK is rejected cleanly.
- RFQ and market-maker gateway are type scaffolds only.
- Execution intents are provisional off-chain records, not settlement.
- No blockchain RPC, ABI encoding, transaction signing, full EIP-712 signature recovery, database, production auth, WebSocket API, or options matching.
