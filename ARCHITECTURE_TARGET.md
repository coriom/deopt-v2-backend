# ARCHITECTURE_TARGET.md — DeOpt v2 Backend Target Architecture

## Long-Term Goal

DeOpt v2 backend is a Rust-first market infrastructure stack.

Latency-sensitive systems must be Rust:
- matching
- RFQ
- market-maker gateway
- execution coordination
- event indexing
- risk pre-checks
- market data distribution

TypeScript is reserved for:
- admin dashboards
- frontend glue
- non-critical APIs
- internal tooling

Python is reserved for:
- analytics
- volatility research
- reporting
- strategy research
- market quality analysis

## Core Principle

Only smart contracts settle final state.

The off-chain backend can:
- collect orders
- validate orders
- match orders
- produce execution intents
- submit transactions in a later phase
- index events
- cache market/account state

The off-chain backend cannot:
- move funds without valid user authorization
- create final fills without on-chain confirmation
- bypass protocol risk rules
- treat off-chain matches as final settlement

## Target Services

### deopt-v2-backend

Rust trading backend.

Responsibilities:
- HTTP trading API
- future WebSocket trading API
- orderbook
- matching
- RFQ scaffold
- MM gateway scaffold
- execution-intent queue
- market state cache

### future deopt-v2-executor

Rust executor.

Responsibilities:
- consume execution intents
- simulate contract calls
- manage nonce
- manage gas
- submit transactions
- retry safely
- reconcile tx status with indexer

### future deopt-v2-indexer

Rust indexer.

Responsibilities:
- ingest protocol events
- handle reorgs
- maintain indexed DB views
- track block cursors
- expose canonical indexed state
- support replay from deployment block

### future deopt-v2-admin

TypeScript dashboard/API.

Responsibilities:
- admin UI
- monitoring
- configuration
- governance helpers
- non-critical reads
- operational dashboards

### future deopt-v2-analytics

Python/ClickHouse research layer.

Responsibilities:
- volatility surfaces
- option pricing research
- strategy analytics
- market quality metrics
- risk reports
- trader/MM performance analytics

## Phase Order

1. Rust in-memory trading engine
2. HTTP API
3. deterministic orderbook tests
4. execution intent queue
5. EIP-712 validation
6. on-chain executor
7. indexer
8. WebSocket market data
9. MM gateway
10. RFQ engine
11. options orderbook
12. structured products backend

## Current Implemented Boundary

Implemented Phase 1 foundations:
- deterministic orderbook
- perp matching
- HTTP API
- execution-intent queue
- RFQ scaffold
- MM gateway scaffold
- EIP-712 order signature validation
- PostgreSQL persistence V1 for nonces, orders, trades, execution intents, and event audit records

Do not implement:
- blockchain RPC
- on-chain execution database/indexer views
- real WebSocket MM gateway
- on-chain execution
