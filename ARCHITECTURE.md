# DeOpt v2 Backend Architecture

## Mission

DeOpt v2 backend is phase 1 trading infrastructure for off-chain order collection and deterministic matching. It produces execution intents for later on-chain execution, while deployed smart contracts remain the final source of truth.

## Why Rust

The long-term backend needs low-latency deterministic matching, RFQ, market-maker connectivity, execution coordination, indexing, and risk pre-checks. Rust is used for these latency-sensitive paths because it gives explicit memory ownership, strong typing, predictable performance, and good support for synchronous pure domain logic.

## System Architecture

- `api`: Axum HTTP routes. The API parses requests, calls the engine, and returns events/state.
- `engine`: Command/event boundary. It owns market orderbooks and the execution-intent queue.
- `orderbook`: Pure synchronous matching logic with `BTreeMap` price levels and FIFO `VecDeque` ordering.
- `execution`: Provisional `ExecutionIntent` records, an in-memory queue, a dry-run executor scaffold, a PerpMatchingEngine calldata builder, and manual `eth_call` simulation. No transaction submission exists.
- `indexer`: Opt-in Indexer V1 that reads `PerpMatchingEngine.TradeExecuted` logs with `eth_getLogs`, persists decoded events, and advances a block cursor after persistence succeeds.
- `reconciliation`: Opt-in Reconciliation V1 that links indexed events to execution intents by direct `onchain_intent_id` equality without marking finality.
- `db`: Optional PostgreSQL persistence for used nonces, submitted orders, matched trades, execution intents, and engine event audit records.
- `rfq`: RFQ type scaffold only.
- `mm`: market-maker session, heartbeat, bulk quote, and bulk cancel type scaffold only.
- `signing`: signed-order schema, EIP-712 order hashing, strict secp256k1 signer recovery, signature mode, deadline validation, and in-memory nonce tracking.
- `config`: environment loading for host, port, log level, network name, chain id, disabled execution flag, simulation flags, indexer flags, reconciliation flags, signature mode, and opt-in persistence.

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
- Matched PerpTrade signing-payload and trade-signature collection endpoints.
- Manual RPC simulation V1 for calldata-ready intents using `eth_call` only.
- HTTP endpoints for health, markets, orderbook, orders, cancellation, and execution intents.
- Signed-order HTTP boundary with nonce/deadline validation, disabled signature shape checks, and strict EIP-712 signer recovery.
- Optional PostgreSQL persistence V1 guarded by `PERSISTENCE_ENABLED=false` by default.
- Optional Indexer V1 guarded by `INDEXER_ENABLED=false` by default.
- Optional Reconciliation V1 guarded by `RECONCILIATION_ENABLED=false` by default.

## Indexer V1

Indexer V1 reads Base Sepolia logs from `RPC_URL` for `PERP_MATCHING_ENGINE_ADDRESS` and topic0 `TradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)`. Each tick reads the latest block, calculates a bounded range from the stored `perp_matching_engine` cursor, fetches logs with `eth_getLogs`, decodes indexed `intentId`/buyer/seller topics and ABI data fields, inserts rows into `indexed_perp_trades`, and updates `indexer_cursors` only after those writes succeed.

The indexer is read-only. It does not sign, send, or broadcast transactions. It also does not mark `ExecutionIntent` records submitted or confirmed. Indexed `onchain_intent_id` values allow direct reconciliation by comparing them to `keccak256(bytes(execution_intents.intent_id))`; economic match keys are a fallback only for data that lacks the event intent id.

V1 stores `block_hash` when the RPC log includes it, but does not implement deep reorg rollback or replay correction. Smart contracts remain the final source of truth.

HTTP endpoints:
- `GET /indexer/status`
- `POST /indexer/tick`
- `GET /indexed/perp-trades`

## Reconciliation V1

Reconciliation V1 runs after indexing and uses direct `onchain_intent_id` identity as its primary matching path. New persisted execution intents store `execution_intents.onchain_intent_id = keccak256(bytes(execution_intents.intent_id))`, and indexed `TradeExecuted` rows store the Solidity event `intentId` as `indexed_perp_trades.onchain_intent_id`.

A manual reconciliation tick reads unreconciled indexed trades with non-null on-chain intent ids. If exactly one backend execution intent and exactly one indexed event share that id, it inserts an `execution_reconciliations` row with `status=matched`. If multiple intents or duplicate indexed events share the id, it records an ambiguous result where possible. If no backend intent exists, the event is counted as unmatched and no tx ownership is fabricated.

Reconciliation is not finality. It does not mutate `execution_intents.status`, does not set `submitted`, and does not set `confirmed`. API status and tick responses report `confirmed=0` in this phase. Future confirmation requires transaction ownership checks and reorg-aware finality handling.

HTTP endpoints:
- `GET /reconciliation/status`
- `POST /reconciliation/tick`
- `GET /reconciliations`
- `GET /reconciliation/intents/:intent_id`

## Future v2/v3 Scope

- On-chain executor service.
- Indexer with full reorg handling and deterministic intent reconciliation.
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

Every matched trade creates an `ExecutionIntent` with buyer, seller, order IDs, market, price, size, buyer maker flag, buyer/seller order nonces, execution deadline, timestamp, and `Pending` status. Intents are stored in memory and exposed through `GET /execution-intents`.

The execution deadline is the minimum of the two original signed-order deadlines. Direct in-memory orders that do not carry nonce/deadline metadata can still match, but their signing-payload endpoint fails clearly instead of inventing missing PerpTrade fields.

The intended lifecycle is: order accepted, matched, execution intent created, PerpTrade signatures collected, optionally simulated, later broadcast by a future executor, indexed by the indexer, reconciled as matched by direct intent id, and only later confirmed after transaction ownership and finality checks. Reconciliation V1 stops at the matched reconciliation row.

## Persistence V1

Persistence is disabled by default. With `PERSISTENCE_ENABLED=false`, the service does not connect to Postgres and retains the original in-memory nonce, orderbook, and execution-intent behavior.

With `PERSISTENCE_ENABLED=true`, startup requires `DATABASE_URL`, connects to Postgres, and runs migrations. The repository persists used nonces, accepted orders, matched trades, execution intents, cancellation/status updates, and a basic `engine_events` audit stream. Persisted nonce uniqueness is scoped to `(account, nonce)`, which prevents replay after restart while still allowing different accounts to use the same nonce.

The in-memory engine remains the live matching state in this V1 patch. Database writes for an order submission use a SQL transaction for nonce insertion and event persistence, but full engine-state rollback/replay around database failure is deferred.

## Blockchain Execution Boundary

This repository does not execute on-chain transactions in phase 1. It does not sign transactions, load private keys, broadcast transactions, or mark trades as finally settled. Manual simulation can call an RPC endpoint with `eth_call` only. A future executor service can consume intents, manage production simulation policy, submit transactions, and reconcile confirmations with an indexer.

The current calldata builder V1 can encode `PerpMatchingEngine.executeTrade(PerpTrade,bytes,bytes)` using an explicit `PerpTradePayload` and explicit buyer/seller trade signatures. The Solidity `PerpTrade.intentId` is `keccak256(bytes(execution_intents.intent_id))`, returned as `0x` plus 64 hex chars and used consistently in EIP-712 signing, calldata, and indexed-event reconciliation. `PerpTrade` signatures are distinct from the off-chain order signatures verified by the order API: the Solidity contract verifies signatures over the final matched trade payload, not the original order payloads. The builder therefore does not reuse order signatures and does not fabricate missing signatures.

Clients fetch the final EIP-712 `PerpTrade` payload from `GET /execution-intents/:intent_id/signing-payload`, sign it externally, and submit the two signatures to `POST /execution-intents/:intent_id/signatures`. Signatures are shape-validated and stored in memory or in `execution_intent_signatures` when persistence is enabled. Calldata readiness is true only when both signatures are present and the intent has complete PerpTrade metadata.

Intent-derived executor dry-runs produce non-executable previews when trade signatures are unavailable. When both signatures are stored, the dry-run builder can construct real calldata while still marking the prepared call non-broadcastable.

Prepared execution calls remain non-broadcastable in this phase. `is_broadcastable=false`, transaction `value=0`, and no submitted or confirmed lifecycle state is produced by the calldata builder.

Simulation V1 is a manual safety check exposed by `POST /executor/simulate/:intent_id`. It loads the intent and stored PerpTrade signatures, rebuilds the same `executeTrade` calldata, and performs `eth_call` from `EXECUTOR_FROM_ADDRESS` or the zero address to `PERP_MATCHING_ENGINE_ADDRESS`. A successful call marks the intent `simulation_ok`; a revert or RPC failure marks it `simulation_failed` with the error text. These statuses are not submission, confirmation, settlement, or finality.

## Deterministic Replay Assumptions

Matching decisions are deterministic for a given ordered command stream, market set, generated IDs, and timestamps. The pure orderbook uses ordered maps for price priority and FIFO queues for time priority. Durable replay from persisted orders is not implemented yet.

## Safety Assumptions

- Smart contracts are canonical for final balances, fills, and risk.
- Off-chain matches are provisional until confirmed on-chain in a later phase.
- PerpMatchingEngine requires signatures over the exact matched `PerpTrade`; order signatures are not valid substitutes.
- `simulation_ok` only means an `eth_call` did not revert at the queried block.
- Indexed `TradeExecuted` events prove on-chain execution occurred and include `intentId` for direct matching, but this backend still does not mark intents confirmed in phase 1.
- Indexer V1 is not fully reorg safe.
- Zero price and zero size are rejected.
- Self-trade is rejected before fills.
- Large financial values are represented as integers, not floating point.
- Disabled signature mode is for local development only; strict mode requires recovered EIP-712 signer equality with `order.account`.

## Out of Scope

No Redis, private key loading, transaction signing, transaction broadcast, production authentication, frontend code, TypeScript, Python service code, C++, or Solidity changes. Blockchain RPC is limited to manual `eth_call` simulation and opt-in `eth_getLogs` indexing. ABI encoding is limited to the non-broadcastable PerpMatchingEngine calldata builder boundary.

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
