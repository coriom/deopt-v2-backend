# CODEX_GUARDRAILS.md — DeOpt v2 Backend Implementation Guardrails

## Absolute Constraints

Do not modify files outside this repository.

Do not modify:
- ~/DEOPT/deoptv2
- Solidity contracts
- deployment files
- frontend repositories

Do not add:
- TypeScript
- Node.js
- Python
- C++
- database
- Redis
- blockchain RPC
- ABI encoding
- transaction signing
- private key loading
- production authentication

This phase is Rust-only.

## Implementation Style

Prefer simple, explicit, testable code.

Avoid:
- macro-heavy abstractions
- generic frameworks
- premature trait hierarchies
- async complexity inside pure matching logic
- global mutable state outside controlled shared app state
- floating-point arithmetic

The orderbook and matching engine should be mostly pure synchronous Rust.
The HTTP layer can be async.

## Financial Safety

No `f64`.
No `f32`.

All prices and sizes must be integer fixed-point.

Use:
- `price_1e8`
- `size_1e8`
- `remaining_size_1e8`

Do not silently cast large integers down to smaller integer types.
Validate zero values explicitly.

## Matching Safety

Matching must be deterministic.

Required behavior:
- bids sorted high-to-low
- asks sorted low-to-high
- FIFO inside one price level
- reject zero price
- reject zero size
- reject self-trade
- reject immediately-matchable post-only orders
- GTC rests remainder
- IOC cancels remainder
- FOK can be explicitly rejected

A matched trade is not final settlement.
It only creates an ExecutionIntent.

## HTTP API Safety

HTTP API should be thin.

It should:
- parse request
- call engine
- return events/state

It should not:
- implement matching logic directly
- mutate orderbook outside engine
- fake blockchain finality
- pretend execution is confirmed

## Testing

Tests are mandatory.

At minimum:
- order sorting
- FIFO
- partial fills
- full fills
- IOC behavior
- post-only rejection
- cancellation
- self-trade rejection
- multiple market isolation
- execution intent creation

Run:
- cargo fmt
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test
- cargo build

## Documentation

README.md and ARCHITECTURE.md must clearly say:
- this is phase 1
- no blockchain execution yet
- execution intents are provisional
- smart contracts are final source of truth
- RFQ/MM are scaffolded only
