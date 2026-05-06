# DeOpt v2 Backend Architecture

## Mission

DeOpt v2 backend is phase 1 trading infrastructure for off-chain order collection and deterministic matching. It produces execution intents for later on-chain execution, while deployed smart contracts remain the final source of truth.

## Why Rust

The long-term backend needs low-latency deterministic matching, RFQ, market-maker connectivity, execution coordination, indexing, and risk pre-checks. Rust is used for these latency-sensitive paths because it gives explicit memory ownership, strong typing, predictable performance, and good support for synchronous pure domain logic.

## System Architecture

- `api`: Axum HTTP routes. The API parses requests, calls the engine, and returns events/state.
- `engine`: Command/event boundary. It owns market orderbooks and the execution-intent queue.
- `orderbook`: Pure synchronous matching logic with `BTreeMap` price levels and FIFO `VecDeque` ordering.
- `execution`: Provisional `ExecutionIntent` records, an in-memory queue, a dry-run executor scaffold, a PerpMatchingEngine calldata builder, manual `eth_call` simulation, and an explicitly gated real broadcast path. No transaction submission exists in the default build.
- `nonce_sync`: Opt-in Perp on-chain nonce reads from `PerpMatchingEngine.nonces(account)` and strict order-intake validation before backend-local nonce reservation.
- `indexer`: Opt-in Indexer V1 that reads `PerpMatchingEngine.TradeExecuted` logs with `eth_getLogs`, persists decoded events, and advances a block cursor after persistence succeeds.
- `reconciliation`: Opt-in Reconciliation V1 that links indexed events to execution intents by direct `onchain_intent_id` equality without marking finality.
- `confirmation`: Opt-in Confirmation / Finality V1 that reads transaction receipts and block height, then marks submitted execution transactions and intents confirmed only after receipt success, enough blocks, indexed event identity, and matched reconciliation.
- `db`: Optional PostgreSQL persistence for used nonces, submitted orders, matched trades, execution intents, and engine event audit records.
- `rfq`: RFQ type scaffold only.
- `mm`: market-maker session, heartbeat, bulk quote, and bulk cancel type scaffold only.
- `signing`: signed-order schema, EIP-712 order hashing, strict secp256k1 signer recovery, signature mode, deadline validation, and in-memory nonce tracking.
- `config`: environment loading for host, port, log level, network name, chain id, disabled execution flag, simulation flags, indexer flags, reconciliation flags, confirmation flags, signature mode, and opt-in persistence.

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
- Dev-only local PerpTrade signing CLI for testnet payloads returned by the backend.
- Manual RPC simulation V1 for calldata-ready intents using `eth_call` only, with revert-data diagnostics on failure.
- Real Broadcast V1 with transaction request construction, transaction records, disabled-by-default behavior, EIP-1559 signing, pending nonce lookup, chain id checks, and `eth_sendRawTransaction` behind explicit config gates.
- Perp On-chain Nonce Sync V1 with a disabled-by-default read endpoint and optional strict `POST /orders` nonce equality check against `PerpMatchingEngine.nonces(order.account)`.
- HTTP endpoints for health, markets, orderbook, orders, cancellation, and execution intents.
- Signed-order HTTP boundary with nonce/deadline validation, disabled signature shape checks, and strict EIP-712 signer recovery.
- Optional PostgreSQL persistence V1 guarded by `PERSISTENCE_ENABLED=false` by default.
- Optional Indexer V1 guarded by `INDEXER_ENABLED=false` by default.
- Optional Reconciliation V1 guarded by `RECONCILIATION_ENABLED=false` by default.
- Optional Confirmation / Finality V1 guarded by `CONFIRMATION_ENABLED=false` by default.

## Perp On-chain Nonce Sync V1

Perp On-chain Nonce Sync V1 is disabled by default:

```text
PERP_NONCE_SYNC_ENABLED=false
PERP_NONCE_SYNC_REQUIRE_RPC=true
PERP_NONCE_SYNC_STRICT=true
```

Local nonce state remains replay protection for the backend, but it is not the canonical nonce source for settlement. The deployed `PerpMatchingEngine` checks `nonces[account] == tradeNonce` when executing the final `PerpTrade`. If backend-local nonce state is stale after testnet resets, restarts, manual executions, or another submitter advancing the on-chain nonce, local `used_nonces` can diverge from `PerpMatchingEngine.nonces(account)`.

When `PERP_NONCE_SYNC_ENABLED=true`, the backend can query:

```text
PerpMatchingEngine.nonces(address)(uint256)
```

using `RPC_URL` and `PERP_MATCHING_ENGINE_ADDRESS`. `GET /accounts/:address/perp-nonce` returns the account, matching-engine address, on-chain nonce, and `source=onchain`; when the feature is disabled it returns `perp nonce sync is disabled`. If nonce sync is enabled and `PERP_NONCE_SYNC_REQUIRE_RPC=true`, startup fails unless RPC and a valid nonzero matching-engine address are configured. If startup validation is deferred with `PERP_NONCE_SYNC_REQUIRE_RPC=false`, the endpoint and strict order validation return explicit configuration errors until RPC and contract address are present.

When `PERP_NONCE_SYNC_STRICT=true`, `POST /orders` reads the on-chain nonce before consuming the in-memory or persisted local nonce. The order is accepted into the existing local nonce path only when `order.nonce == PerpMatchingEngine.nonces(order.account)`. Lower and higher nonces are rejected with `perp nonce mismatch: expected on-chain nonce N, got M`. This keeps local replay protection in place while preventing known-stale orders from reaching matching, execution intent creation, simulation, or broadcast preparation.

This is a prerequisite for the market-maker gateway because gateway order flow will depend on continuously correct maker and taker nonces before matched trades are signed and submitted.

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

Reconciliation is not finality. It does not mutate `execution_intents.status`, does not set `submitted`, and does not set `confirmed`. Confirmation V1 consumes matched reconciliation rows as one required input.

HTTP endpoints:
- `GET /reconciliation/status`
- `POST /reconciliation/tick`
- `GET /reconciliations`
- `GET /reconciliation/intents/:intent_id`

## Confirmation / Finality V1

Confirmation V1 runs after real broadcast, indexing, and reconciliation. It is disabled by default:

```text
CONFIRMATION_ENABLED=false
CONFIRMATION_REQUIRE_PERSISTENCE=true
CONFIRMATION_REQUIRED_BLOCKS=2
CONFIRMATION_MAX_BATCH_SIZE=50
CONFIRMATION_REQUIRE_RECONCILIATION=true
```

The confirmation sequence is:

```text
broadcast -> receipt -> indexer -> reconciliation -> confirmation
```

A transaction receipt with `status=1` is not enough to confirm a backend intent. The backend also requires `receipt.block_number`, `current_block >= receipt.block_number + CONFIRMATION_REQUIRED_BLOCKS`, an indexed `TradeExecuted` row with the same `tx_hash` and `onchain_intent_id`, and a `matched` reconciliation row for the same `intent_id`, `onchain_intent_id`, and `tx_hash`. This keeps receipt inclusion, event decoding, direct intent identity, and reconciliation tied to the same transaction before `execution_intents.status` can become `confirmed`.

Confirmation updates `execution_transactions.confirmation_status` idempotently. It sets `confirmed_at_ms`, `confirmed_block_number`, and `execution_intents.status=confirmed` only for the `confirmed` decision. Pending and failure reasons are persisted as `missing_receipt`, `receipt_failed`, `not_finalized`, `missing_indexed_event`, `missing_reconciliation`, or `failed`.

HTTP endpoints:
- `GET /executor/confirmations/status`
- `POST /executor/confirm/:intent_id`
- `POST /executor/confirmations/tick`
- `GET /executor/confirmations/:intent_id`

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
3. API validates deadline, signature shape/mode, known market, optional on-chain perp nonce equality, and per-account local nonce. Persistent mode reserves the local nonce in Postgres so replay protection survives restart.
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

The intended lifecycle is: order accepted, matched, execution intent created, PerpTrade signatures collected, calldata ready, simulation with revert diagnostics when needed, simulation ok, prepared transaction, submitted by the explicitly gated executor, indexed by the indexer, reconciled as matched by direct intent id, and confirmed only after receipt success, enough blocks, indexed event identity, and matched reconciliation. Real Broadcast V1 stops at submitted after a real RPC tx hash; Reconciliation V1 stops at the matched reconciliation row.

## Persistence V1

Persistence is disabled by default. With `PERSISTENCE_ENABLED=false`, the service does not connect to Postgres and retains the original in-memory nonce, orderbook, and execution-intent behavior.

With `PERSISTENCE_ENABLED=true`, startup requires `DATABASE_URL`, connects to Postgres, and runs migrations. The repository persists used nonces, accepted orders, matched trades, execution intents, cancellation/status updates, and a basic `engine_events` audit stream. Persisted nonce uniqueness is scoped to `(account, nonce)`, which prevents replay after restart while still allowing different accounts to use the same nonce.

The in-memory engine remains the live matching state in this V1 patch. Database writes for an order submission use a SQL transaction for nonce insertion and event persistence, but full engine-state rollback/replay around database failure is deferred.

## Blockchain Execution Boundary

This repository does not execute on-chain transactions in phase 1 by default. With default configuration it does not sign transactions, retain private keys, broadcast transactions, or mark trades as finally settled. Manual simulation can call an RPC endpoint with `eth_call` only. Real Broadcast V1 can submit a transaction only when `EXECUTOR_REAL_BROADCAST_ENABLED=true` and the required private key, RPC URL, static fee configuration, persistence, signatures, calldata, and simulation gates are present.

The current calldata builder V1 can encode `PerpMatchingEngine.executeTrade(PerpTrade,bytes,bytes)` using an explicit `PerpTradePayload` and explicit buyer/seller trade signatures. The Solidity `PerpTrade.intentId` is `keccak256(bytes(execution_intents.intent_id))`, returned as `0x` plus 64 hex chars and used consistently in EIP-712 signing, calldata, and indexed-event reconciliation. `PerpTrade` signatures are distinct from the off-chain order signatures verified by the order API: the Solidity contract verifies signatures over the final matched trade payload, not the original order payloads. The builder therefore does not reuse order signatures and does not fabricate missing signatures.

Clients fetch the final EIP-712 `PerpTrade` payload from `GET /execution-intents/:intent_id/signing-payload`, sign it externally, and submit the two signatures to `POST /execution-intents/:intent_id/signatures`. Signatures are shape-validated and stored in memory or in `execution_intent_signatures` when persistence is enabled. Calldata readiness is true only when both signatures are present and the intent has complete PerpTrade metadata.

For local testnet development, `src/bin/sign_perp_trade.rs` can sign the backend-provided payload digest with a throwaway buyer or seller private key supplied through the process environment. It validates that the derived signer address matches `message.buyer` or `message.seller` by default and emits JSON for manual submission. This CLI is not part of the production request path, does not store keys, and does not make the backend a custodial signer. Production signing must happen in user wallets, market-maker wallets, or external signing systems controlled outside this server.

Intent-derived executor dry-runs produce non-executable previews when trade signatures are unavailable. When both signatures are stored, the dry-run builder can construct real calldata while still marking the prepared call non-broadcastable.

Prepared execution calls remain non-broadcastable in this phase. `is_broadcastable=false`, transaction `value=0`, and no submitted or confirmed lifecycle state is produced by the calldata builder.

Simulation V1 is a manual safety check exposed by `POST /executor/simulate/:intent_id`. It loads the intent and stored PerpTrade signatures, rebuilds the same `executeTrade` calldata, and performs `eth_call` from `EXECUTOR_FROM_ADDRESS` or the zero address to `PERP_MATCHING_ENGINE_ADDRESS`. A successful call marks the intent `simulation_ok`; a revert or RPC failure marks it `simulation_failed` with the error text. Revert Diagnostics V1 extracts revert data from common JSON-RPC error shapes, decodes Solidity `Error(string)` and `Panic(uint256)`, maps known custom-error selectors such as `InvalidSignature`, `NotAuthorized`, `InsufficientMargin`, and `OracleStale`, and persists the raw data, selector, and decoded JSON on the simulation row. These statuses and diagnostics are a safety gate before real broadcast; they are not submission, confirmation, settlement, or finality.

Real Broadcast V1 is exposed through `POST /executor/broadcast/:intent_id`, `GET /executor/transactions`, and `GET /executor/transactions/:intent_id`. With `EXECUTOR_REAL_BROADCAST_ENABLED=false`, the broadcast endpoint returns a disabled refusal with `submitted=false`, `confirmed=false`, and no tx hash. The transaction request builder requires both PerpTrade signatures, non-empty `executeTrade` calldata, a configured matching-engine target, static EIP-1559 fee values, and `simulation_ok` when required.

When real broadcast is enabled, the executor parses `EXECUTOR_PRIVATE_KEY` into an in-process secp256k1 signer, derives the executor address, checks `eth_chainId == EXECUTOR_CHAIN_ID`, fetches the pending nonce with `eth_getTransactionCount`, signs a type `0x02` EIP-1559 raw transaction, and calls `eth_sendRawTransaction`. The repository stores a `submitted` transaction only after the RPC returns a syntactically valid tx hash and then marks the execution intent `submitted`. It never marks intents confirmed during broadcast. Rejected chain-id mismatches and failed RPC sends can be persisted as `rejected` or `failed` transaction records without fabricating tx hashes.

Confirmation V1 is exposed separately through manual endpoints. It reads `eth_getTransactionReceipt` and `eth_blockNumber`, but it does not trust receipt status alone and does not use receipt logs as a substitute for the persisted indexer and reconciliation records.

Private keys are modeled with a redacted secret wrapper and are not included in API responses. The signer exposes only the derived address. If a transaction is accepted by RPC and the subsequent database write fails, this V1 cannot make the RPC send and database record atomic; operators must reconcile externally with the returned or observed chain transaction data.

## Deterministic Replay Assumptions

Matching decisions are deterministic for a given ordered command stream, market set, generated IDs, and timestamps. The pure orderbook uses ordered maps for price priority and FIFO queues for time priority. Durable replay from persisted orders is not implemented yet.

## Safety Assumptions

- Smart contracts are canonical for final balances, fills, and risk.
- `PerpMatchingEngine.nonces(account)` is canonical for PerpTrade nonce equality; backend-local `used_nonces` is replay protection only.
- Off-chain matches are provisional until confirmed on-chain in a later phase.
- PerpMatchingEngine requires signatures over the exact matched `PerpTrade`; order signatures are not valid substitutes.
- `simulation_ok` only means an `eth_call` did not revert at the queried block.
- `submitted` is valid only after `eth_sendRawTransaction` returns a real tx hash; it does not mean confirmed.
- Indexed `TradeExecuted` events prove on-chain execution occurred and include `intentId` for direct matching, but confirmation still also requires receipt success, enough blocks, and matched reconciliation for the same transaction.
- Receipt status alone is insufficient for confirmation.
- Indexer V1 is not fully reorg safe.
- Zero price and zero size are rejected.
- Self-trade is rejected before fills.
- Large financial values are represented as integers, not floating point.
- Disabled signature mode is for local development only; strict mode requires recovered EIP-712 signer equality with `order.account`.

## Out of Scope

No Redis, production authentication, frontend code, TypeScript, Python service code, C++, or Solidity changes. Blockchain RPC is limited to manual `eth_call` simulation, opt-in `eth_getLogs` indexing, confirmation receipt/block reads, and explicitly gated `eth_sendRawTransaction` broadcast. ABI encoding is limited to the PerpMatchingEngine calldata builder and guarded transaction request boundary.

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
