# DeOpt backend local perps alpha: acceptance checkpoint

This checkpoint changes only the read-only perps market API. It does not turn on order submission, signing, workers, or transaction broadcasting.

## Concrete repair

The market catalogue was configured with symbols for future markets. Before C02, Base Sepolia market 1 does not exist. The prior registry reader called `isMarketActive`, which reverts for an absent market, and the service converted that error to `unknown` metadata. The price route could also return an oracle feed for a configured symbol without confirming that its market existed.

The registry reader now checks the frozen D1 `marketExists(uint256)` ABI before reading activity. The API omits absent markets from `/perps/markets`, returns 404 for a configured but absent market or its price, and returns 503 when the registry read cannot establish existence. Market read routes verify the live RPC chain ID against Base Sepolia before serving on-chain metadata. Existing offline catalogue fallback remains an explicit `unknown` status at the service layer; the HTTP routes fail closed when chain identity is unavailable. Public mutations remain separately gated.

At pinned Base Sepolia block 50,400,186 (hash `0x1436cf8aa3d1a05c800941cc6f2ae4165101f595c2f899c3840a5036de23820d`, 2026-10-26 08:05:06 UTC), `eth_chainId=84532` and canonical D1 `marketExists(1)=false`. Calling `isMarketActive(1)` at that same block reverts with selector `0xd8ac9325`, matching frozen-source `UnknownMarket()`. This is a read-only snapshot, not a public execution result.

Focused validation: six market-reader unit tests and twelve HTTP perps-read tests passed; `cargo build --jobs 1` passed. A backend process started from an empty environment with all execution and trading flags explicitly false, then `/health` returned 200, `/ready` 200 and `/perps/markets` 503 under disabled read mode; the process was stopped. Changed Rust files pass `rustfmt --check`. Repository-wide `cargo fmt --all -- --check` reports unrelated existing formatting differences. `cargo clippy --lib -- -D warnings` fails on 198 existing warnings/errors across unrelated modules, including an `await_holding_lock` finding in the in-memory perps submit path. Neither the full Rust suite nor a Docker image build was completed in this checkpoint.

## Capability and gap matrix

| Capability | Current classification | Acceptance limit |
|---|---|---|
| Perps read API, market metadata and oracle reads | IMPLEMENTED_TESTED (focused local checks) | No backend hosted; direct RPC reads are not a browser end-to-end proof. |
| In-memory and PostgreSQL orderbook/matching | IMPLEMENTED_UNVERIFIED in this checkpoint | Existing deterministic and PG tests must pass on this revision. Public submit remains gated. |
| In-memory submit concurrency | PARTIAL | Strict Clippy flags `submit_perp_order_via_state` holding `std::sync::MutexGuard`s across oracle awaits. Move oracle reads ahead of the guarded mutation or use an async-safe state design, with race/restart tests, before local order-intake acceptance. PostgreSQL path is separate. |
| Signed perps intent and nonce handling | IMPLEMENTED_UNVERIFIED in this checkpoint | Fresh strict-signature and replay matrix needed before alpha signoff. |
| Margin/risk, funding and liquidation workers | IMPLEMENTED_UNVERIFIED in this checkpoint | On-chain market absent and D4/D6 maintenance block economic operation. |
| Indexer, reconciliation and reorg recovery | IMPLEMENTED_UNVERIFIED in this checkpoint | Fresh PostgreSQL/restart and fork proof needed; do not infer public fills. |
| V2 transaction construction/simulation | IMPLEMENTED_UNVERIFIED in this checkpoint | Prior Anvil test exists; no live trading path is authorized. |
| Base Sepolia market 1 and D2/D3 policy execution | BLOCKED_ONCHAIN | C02 and D2/D3 S2 require separate human governance and canonical postflight. |
| Local frontend-to-backend browser integration | PARTIAL | Frontend's production trading client defaults to backend unavailable and blocks non-GET; local connected browser acceptance remains. |
| Persistent hosting, monitoring, backup and rollback | PARTIAL | Dockerfile, metrics/readiness and local monitoring compose exist; no remote backend deployment or recovery drill verified. |

## Dependency-ordered local acceptance

1. Verify the focused market API tests, formatting of touched files, and Base Sepolia chain/market readback.
2. Run the existing deterministic orderbook, signature/replay, margin, worker, and indexer tests with transaction submission disabled.
   Resolve the in-memory submit lock/await finding before treating that path as reliable under concurrent order intake.
3. Run PostgreSQL restart/reconciliation tests and the isolated Anvil V2 lifecycle with synthetic accounts. Label every fill simulated unless it has a canonical public receipt.
4. Connect the local frontend to the local backend under the existing disabled-trading controls. Exercise market discovery, unavailable backend, orderbook, positions, wrong-network wallet and reload behavior.
5. Recheck C02/D2/D3 governance and maintenance separately before any later public alpha. No test result in this checkpoint authorizes a public write.

For eventual hosting, a persistent Docker service with PostgreSQL and supervised indexer/workers is more appropriate than stateless frontend functions for orderbook and event processing. The existing Dockerfile and readiness/metrics endpoints are starting points; effective environment flags, health behavior, backups, restart recovery, and rollback must be validated before a deployment decision. No host is provisioned here.

The Docker build stage now copies `resources/hybrid-v2`, required by compile-time `include_str!` calls. The builder uses Rust 1.95, matching the local validated compiler, and the runtime installs `curl` used by its health probe. A complete image build and container restart check remain separate acceptance gates.

Minimum future runtime schema: `CHAIN_ID=84532`, public D1/D4/D5/D6 addresses, a read-only `RPC_URL`, `PERSISTENCE_ENABLED=true`, a PostgreSQL `DATABASE_URL`, `PERPS_READ_ENABLED`, and explicit `PERPS_PUBLIC_TRADING_ENABLED=false`, `EXECUTOR_REAL_BROADCAST_ENABLED=false`, `PERPS_CLOSED_TEST_ENABLED=false`. Only variable **names and boolean posture** belong in a review artifact; connection strings and signer material do not. A host must supervise the API and each explicitly enabled indexer/worker, route `/health` and `/ready`, scrape `/metrics`, retain bounded logs, and alert on stalled cursors or worker ticks. Before rollout, exercise database restore and event-cursor replay against a disposable copy. Pin the image digest and migration revision; rollback must account for the backend's automatic SQL migrations and must be tested before any remote release. Vercel remains appropriate for the independently deployed frontend or stateless gateway only.
