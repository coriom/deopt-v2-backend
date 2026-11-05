# DEOPT_OPTIONS_CANONICAL_E2E_V3

Scope: local backend change only. No Base Sepolia transaction, hosted deployment, production database migration, or GitHub push.

## Implemented

`MarginEngine.getTraderSeriesLength(address)`, a slice bounded to 128 IDs, and `getPositionQuantity(address,uint256)` are now read through the existing read-only `EthCallProvider`. The reader rejects a larger set, a changed enumeration length, zero quantities, malformed signed ABI words and RPC failures. When `OPTION_MARGIN_ENGINE_ADDRESS` is configured, `GET /accounts/:address/positions` compares the complete on-chain open-series set against the fill-derived series IDs and converts fill size using each series' `contract_size_1e8`. Missing metadata, missing RPC, a duplicate series or any quantity mismatch returns 503. A matching quantity still returns `partial`: the response does not claim that premiums, fees, collateral, receipts or block finality have been reconciled. Wallet session authorization remains mandatory at the route.

Focused unit tests cover the ABI read, malformed quantity, an on-chain open series absent from backend fills, and a matching quantity. These tests use a programmable RPC provider. They are **not** an Anvil transaction or a PostgreSQL/browser E2E test.

Reproduction: `cargo test --lib api::trading::tests::` and `cargo test --lib api::trading_views::tests::` from the backend repository. Both passed locally; the latter ran 38 tests. The repository's pre-existing Rust warnings remain. Docker 29.8.1 is available locally only with approved daemon access; no disposable PostgreSQL or Anvil Options transaction was run for this V3 result.

## Remaining gates

| Gate | Verdict | Evidence / blocker |
| --- | --- | --- |
| `FULL_E2E` | BLOCKED | No single command has run Next.js, Rust, PostgreSQL, real Options transactions on Anvil, Rust indexation and Playwright together. |
| `EIP712_SIGNATURE` | PASS contract local / BLOCKED integrated | The existing Foundry scenario signs with real test keys; the browser EIP-712 fixture still returns a synthetic signature shape. |
| `ANVIL_TRANSACTION` | BLOCKED | Existing scenario uses Foundry's local EVM. No externally running Anvil Options trade receipt was produced in this milestone. |
| `RECEIPT_INDEXATION` / `POSTGRES_PERSISTENCE` | BLOCKED integrated | Option event indexer persists decoded logs and cursor, but no receipt from the scenario was ingested into PostgreSQL. |
| `ONCHAIN_POSITION_READ` | PASS ABI helper / BLOCKED Anvil proof | The typed `MarginEngine` reads pass mock-provider tests; no real deployed contract response was exercised through Rust. |
| `POSITION_RECONCILIATION` | PASS local fail-closed comparison / BLOCKED canonical pipeline | Quantity and open-series comparison is implemented. It lacks a pinned common block, event-to-fill correlation, fee/collateral reconciliation and persistent canonical snapshots. |
| `PLAYWRIGHT_POSITION_DISPLAY` | BLOCKED | Browser currently labels inferred fills as partial; no reconciled on-chain position reached the browser. |
| `EXPIRATION_SETTLEMENT` | PASS Foundry local / BLOCKED integrated | Contract scenario passes; no post-settlement indexation/browser proof. |
| `REORG_RECOVERY` | BLOCKED | Option event indexer stores event block hashes but its cursor stores only a height. It does not yet revalidate an indexed block hash and repair orphaned rows on a reorg. |
| `WALLET_ISOLATION` | PASS previously validated local | Session-bound routes reject anonymous and cross-wallet requests in a disposable PostgreSQL 16 test. No hosted Options route has been enabled. |
| `BASE_SEPOLIA_READINESS` | BLOCKED | Options executor, hosted indexing, oracle continuity and user-facing write gates remain unapproved; no Put series exists on Base Sepolia. |

The next implementation step is a local script that deploys the existing Options contracts on a disposable Anvil node, sends a signed `OptionMatchingEngine.executeTrade` transaction, indexes its actual logs into PostgreSQL 16, records an atomic block-hash-aware canonical position snapshot, and drives Next.js through Playwright before and after settlement. Reorg handling must invalidate orphaned events and snapshots before the browser can show a reconciled position.
