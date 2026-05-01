# NEXT_TASK.md — API Numeric Serialization Hardening

## Context

The Rust trading backend scaffold is implemented and validated.

Current status:
- cargo fmt: OK
- cargo clippy --all-targets --all-features -- -D warnings: OK
- cargo test: OK
- cargo build: OK
- HTTP server runs
- /health works
- /markets works
- POST /orders works with numeric JSON values
- matching works
- execution intents are created
- /orderbook/:market_id works
- /execution-intents works

Observed issue:
- API currently expects `price_1e8` and `size_1e8` as JSON numbers.
- For frontend and external clients, financial fixed-point integers must be accepted as strings to avoid JavaScript precision issues.
- API responses currently emit large integer fields as numbers in some places.

## Goal

Harden external API serialization without changing core matching semantics.

The internal engine must continue using integer fixed-point types.

The HTTP boundary should support string-based integer JSON for financial quantities.

## Scope

Implement API-safe numeric serialization/deserialization for fixed-point fields.

Required external JSON behavior:

1. `POST /orders` must accept:
   - `price_1e8` as string
   - `size_1e8` as string

Example:

```json
{
  "market_id": 1,
  "account": "0xmaker",
  "side": "sell",
  "price_1e8": "300000000000",
  "size_1e8": "100000000",
  "time_in_force": "gtc",
  "reduce_only": false,
  "post_only": false,
  "client_order_id": "maker-1"
}
API responses must serialize financial fixed-point quantities as strings:
price_1e8
size_1e8
remaining_size_1e8
orderbook price1e8
orderbook totalSize1e8
trade price_1e8
trade size_1e8
execution intent price_1e8
execution intent size_1e8
Internal domain models may keep integer types.
Do not introduce floating point.
Do not change matching behavior.
Do not add blockchain RPC.
Do not add DB.
Do not add TypeScript/Python/Node.
Implementation Guidance

Prefer a small explicit API DTO layer instead of polluting core engine types.

Suggested pattern:

Keep internal domain structs in src/types.rs using integer fields.
Add API request/response structs in src/api/routes.rs or a new src/api/dto.rs.
Convert API DTOs into internal commands.
Convert internal events/orders/trades/intents into API response DTOs.

String parsing rules:

reject empty strings
reject negative values
reject non-numeric strings
reject zero price
reject zero size
preserve existing engine validation errors where possible

Do not silently truncate values.

Required Tests

Add or update tests to cover:

POST /orders accepts string price_1e8 and size_1e8.
POST /orders rejects non-numeric price_1e8.
POST /orders rejects non-numeric size_1e8.
POST /orders rejects negative string values.
POST /orders rejects empty string values.
Matched order response serializes financial quantities as strings.
/orderbook/:market_id serializes financial quantities as strings.
/execution-intents serializes financial quantities as strings.
Existing orderbook and engine tests still pass.
Validation

Before finishing, run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

The task is complete only if:

string numeric inputs work for POST /orders
unsafe JSON number dependency is removed from public API inputs
financial quantities in public API responses are strings
all tests pass
no blockchain/database/frontend scope is added
no Solidity repo is modified
EOF