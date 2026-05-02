# DeOpt v2 Backend Architecture

## Mission

DeOpt v2 backend is phase 1 trading infrastructure for off-chain order collection and deterministic matching. It produces execution intents for later on-chain execution, while deployed smart contracts remain the final source of truth.

## Why Rust

The long-term backend needs low-latency deterministic matching, RFQ, market-maker connectivity, execution coordination, indexing, and risk pre-checks. Rust is used for these latency-sensitive paths because it gives explicit memory ownership, strong typing, predictable performance, and good support for synchronous pure domain logic.

## System Architecture

- `api`: Axum HTTP routes. The API parses requests, calls the engine, and returns events/state.
- `engine`: Command/event boundary. It owns market orderbooks and the execution-intent queue.
- `orderbook`: Pure synchronous matching logic with `BTreeMap` price levels and FIFO `VecDeque` ordering.
- `execution`: Provisional `ExecutionIntent` records, an in-memory queue, a dry-run executor scaffold, and a PerpMatchingEngine calldata builder. No transaction submission exists.
- `db`: Optional PostgreSQL persistence for used nonces, submitted orders, matched trades, execution intents, and engine event audit records.
- `rfq`: RFQ type scaffold only.
- `mm`: market-maker session, heartbeat, bulk quote, and bulk cancel type scaffold only.
- `signing`: signed-order schema, EIP-712 order hashing, strict secp256k1 signer recovery, signature mode, deadline validation, and in-memory nonce tracking.
- `config`: environment loading for host, port, log level, network name, chain id, disabled execution flag, signature mode, and opt-in persistence.

## Current v1 Scope

- Static markets: `ETH-PERP` and `BTC-PERP`.
- In-memory perp limit orderbooks.
- GTC and IOC matching.
- FOK rejected with a clear unsupported error.
- Post-only rejection when an order would immediately match.
- Self-trade rejection before matching.
- Order cancellation by `order_id`.
- Execution-intent creation for every matched trade.
- PerpMatchingEngine `executeTrade` calldata builder V1 for explicit matched-trade payloads and explicit trade signatures.
- HTTP endpoints for health, markets, orderbook, orders, cancellation, and execution intents.
- Signed-order HTTP boundary with nonce/deadline validation, disabled signature shape checks, and strict EIP-712 signer recovery.
- Optional PostgreSQL persistence V1 guarded by `PERSISTENCE_ENABLED=false` by default.

## Future v2/v3 Scope

- On-chain executor service.
- Indexer with reorg handling.
- WebSocket market data and trading.
- Real market-maker gateway.
- RFQ auction/quote lifecycle.
- Options orderbooks.
- Full replay storage and recovery from persisted orderbooks.
- Risk pre-checks based on indexed protocol state.

## Order Lifecycle

1. Client submits an order to `POST /orders`.
2. API parses the signed-order DTO with string fixed-point values.
3. API validates deadline, signature shape/mode, known market, and per-account nonce. Persistent mode reserves the nonce in Postgres so replay protection survives restart.
4. API converts the signed order into a typed `NewOrder`.
5. Engine creates an `OrderId` and timestamp.
6. Market orderbook validates non-zero price/size and supported time-in-force.
7. Post-only and self-trade checks run before any fill side effects.
8. Matching consumes eligible maker liquidity at maker prices.
9. GTC rests any remainder; IOC cancels any remainder; FOK is rejected.
10. Engine returns events and creates execution intents for matched trades.

## Matching Rules

- Bids sort high-to-low.
- Asks sort low-to-high.
- FIFO applies inside each price level.
- Buy orders match asks where `ask_price <= buy_price`.
- Sell orders match bids where `bid_price >= sell_price`.
- Trade price is the resting maker order price.
- Fill size is `min(taker_remaining, maker_remaining)`.
- Financial fields use integer fixed-point only: `price_1e8`, `size_1e8`, `remaining_size_1e8`.
- Public API DTOs accept and return financial fixed-point quantities as strings; internal engine types remain integers.

## RFQ Future Design

The current RFQ module defines IDs, request/quote structs, and lifecycle states: open, quoted, accepted, expired, executed, and failed. Real quote routing, auctions, expiry handling, and settlement coordination are intentionally deferred.

## MM Gateway Future Design

The current MM module defines session, heartbeat, bulk quote update, and bulk cancel concepts. Real authentication, WebSocket transport, throttling, quote ownership, and market-data distribution are intentionally deferred.

## Execution-Intent Flow

Every matched trade creates an `ExecutionIntent` with buyer, seller, order IDs, market, price, size, timestamp, and `Pending` status. Intents are stored in memory and exposed through `GET /execution-intents`.

## Persistence V1

Persistence is disabled by default. With `PERSISTENCE_ENABLED=false`, the service does not connect to Postgres and retains the original in-memory nonce, orderbook, and execution-intent behavior.

With `PERSISTENCE_ENABLED=true`, startup requires `DATABASE_URL`, connects to Postgres, and runs migrations. The repository persists used nonces, accepted orders, matched trades, execution intents, cancellation/status updates, and a basic `engine_events` audit stream. Persisted nonce uniqueness is scoped to `(account, nonce)`, which prevents replay after restart while still allowing different accounts to use the same nonce.

The in-memory engine remains the live matching state in this V1 patch. Database writes for an order submission use a SQL transaction for nonce insertion and event persistence, but full engine-state rollback/replay around database failure is deferred.

## Blockchain Execution Boundary

This repository does not execute on-chain transactions in phase 1. It does not call RPC endpoints, sign transactions, load private keys, broadcast transactions, or mark trades as finally settled. A future executor service can consume intents, simulate calls, submit transactions, and reconcile confirmations with an indexer.

The current calldata builder V1 can encode `PerpMatchingEngine.executeTrade(PerpTrade,bytes,bytes)` using an explicit `PerpTradePayload` and explicit buyer/seller trade signatures. `PerpTrade` signatures are distinct from the off-chain order signatures verified by the order API: the Solidity contract verifies signatures over the final matched trade payload, not the original order payloads. The builder therefore does not reuse order signatures and does not fabricate missing signatures. Intent-derived executor dry-runs are non-executable previews when maker/nonce/deadline or trade signatures are unavailable.

Prepared execution calls remain non-broadcastable in this phase. `is_broadcastable=false`, transaction `value=0`, and no submitted or confirmed lifecycle state is produced by the calldata builder.

## Deterministic Replay Assumptions

Matching decisions are deterministic for a given ordered command stream, market set, generated IDs, and timestamps. The pure orderbook uses ordered maps for price priority and FIFO queues for time priority. Durable replay from persisted orders is not implemented yet.

## Safety Assumptions

- Smart contracts are canonical for final balances, fills, and risk.
- Off-chain matches are provisional until confirmed on-chain in a later phase.
- PerpMatchingEngine requires signatures over the exact matched `PerpTrade`; order signatures are not valid substitutes.
- Zero price and zero size are rejected.
- Self-trade is rejected before fills.
- Large financial values are represented as integers, not floating point.
- Disabled signature mode is for local development only; strict mode requires recovered EIP-712 signer equality with `order.account`.

## Out of Scope

No Redis, private key loading, transaction signing, blockchain RPC, production authentication, frontend code, TypeScript, Python service code, C++, or Solidity changes. ABI encoding is limited to the non-broadcastable PerpMatchingEngine calldata builder boundary.

## Acceptance Criteria

- Rust project builds.
- HTTP server starts.
- `/health` and `/markets` work.
- `POST /orders` accepts valid orders and rejects invalid orders.
- Matching works in memory.
- Execution intents are created for matched trades.
- `GET /orderbook/:market_id` returns current book state.
- Unit/integration tests pass.
- `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and `cargo build` pass.
