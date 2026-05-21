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
- `db`: Optional PostgreSQL persistence for used nonces, submitted orders, matched trades, perp execution intents, option execution intents, and engine event audit records.
- `rfq`: RFQ V1C domain types, in-memory store, service validation, EIP-712 quote signing payloads, strict/disabled quote signature verification, quote lifecycle, MM gateway broadcast/notification coordination, quote acceptance, and execution-intent creation through the existing lifecycle boundary.
- `options`: Options V1D/V1C domain types, deterministic option series ids, manual series registry, off-chain option orders, price-time matching, fill recording, HTTP/core option RFQs, signed MM option RFQ quote verification, MM Gateway option RFQ coordination, off-chain option RFQ fills, opt-in option execution intents/signing/calldata artifacts, safe option `eth_call` simulation, in-memory/persistent stores, and aggregated option orderbook read model.
- `fees`: Fees & Rebates V1B config, launch fee schedules, `micro_bps` integer math, option premium cap logic, confirmed perp notional logic, maker/taker fee-event creation, daily volume buckets, ledger-only rebate accruals, and read-only admin fee summaries.
- `scripts/e2e`: E2E Test Harness V1A Python orchestration for reproducible safe runtime flows, JSON reports, optional safe backend startup, HTTP checks, and PostgreSQL verification queries.
- `orders`: Shared order/cancel service used by HTTP and the Market Maker Gateway for signed order validation, nonce handling, matching, persistence writes, ownership-checked cancels, cancel-all, and deterministic resting-order lookup.
- `mm`: Market Maker Gateway protocol, session, heartbeat, rate-limit, live order/cancel handling, quote-replace models, MM permission allowlist/capability checks, service boundary, adapter traits, and disabled-by-default WebTransport V1C adapter. Protocol/session/service/rate-limit logic remains transport-agnostic.
- `admin`: Monitoring/Admin V1B admin and metrics configuration. Admin endpoints are disabled by default, metrics are enabled by default for local scraping, and optional token checks reuse the local/dev admin token.
- `monitoring`: Read-only Prometheus-style metrics rendering and readiness checks. Metrics use low-cardinality labels only and never expose secrets, raw URLs, wallet addresses, tx hashes, UUIDs, signatures, or per-object ids.
- `signing`: signed-order schema, shared EIP-712 helpers, strict secp256k1 signer recovery, signature mode, deadline validation, and in-memory nonce tracking.
- `config`: environment loading for host, port, log level, network name, chain id, disabled execution flag, simulation flags, indexer flags, reconciliation flags, confirmation flags, RFQ signature mode/domain flags, Options V1D/V1C flags, Fees & Rebates V1B flags, Market Maker Gateway V1C flags, MM Permissions V1A flags, Monitoring/Admin V1B flags, signature mode, and opt-in persistence.

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
- RFQ V1C guarded by `RFQ_ENABLED=false` and `RFQ_QUOTE_SIGNATURE_MODE=disabled` by default. Enabled mode exposes HTTP/core RFQ creation, quote signing payloads, quote submission/listing, cancellation, WebTransport RFQ push/quote intake through connected MM sessions, optional strict signed quote verification, and acceptance into a pending execution intent without auto-broadcast.
- Options V1I guarded by `OPTIONS_ENABLED=false`, `OPTION_RFQ_ENABLED=false`, `OPTION_RFQ_QUOTE_SIGNATURE_MODE=disabled`, `OPTION_EXECUTION_ENABLED=false`, `OPTION_EXECUTION_SIMULATION_ENABLED=false`, and `OPTION_EXECUTION_BROADCAST_ENABLED=false` by default. Enabled mode exposes manual option series creation/list/get/disable, off-chain GTC option order submit/list/get/cancel, price-time matching, fill listing/get endpoints, aggregated option orderbook reads, HTTP/core option RFQs with off-chain RFQ fills, option RFQ quote signing payloads, optional strict signed MM option RFQ quote verification, MM Gateway option RFQ request/quote/accept notification messages, opt-in option execution signing/calldata artifacts, safe option `eth_call` simulation for calldata-ready intents, and controlled option broadcast when explicitly enabled. Greeks, IV surfaces, and broader on-chain option lifecycle are deferred.
- Market Maker Gateway V1C guarded by `MM_GATEWAY_ENABLED=false` by default. Enabled mode starts a separate WebTransport UDP listener with required TLS cert/key config and routes MM order flow through the live off-chain perp orderbook without auto-broadcasting.
- Production MM Auth V1A guarded by `MM_GATEWAY_AUTH_MODE=disabled` and `MM_GATEWAY_REQUIRE_AUTH=false` by default. `wallet_challenge` uses server-issued Ethereum personal-sign challenges to bind WebTransport sessions to wallet accounts before trading messages are accepted.
- MM Permissions V1A guarded by `MM_PERMISSIONS_ENABLED=false` by default. Enabled mode requires an enabled MM account, capability flags, and optional market or option-series scope rows for protected MM quote/order actions. `MM_PERMISSIONS_REQUIRE_PERSISTENCE=true` requires Postgres for production-like enforcement, while tests can seed permissions in memory.
- Fees & Rebates V1B guarded by `FEES_ENABLED=false` by default. Enabled mode records ledger-only option order fill, option RFQ fill, and confirmed indexed perp trade fee events, daily volume buckets, and optional MM rebate accrual rows. `FEES_REQUIRE_PERSISTENCE=true` requires Postgres for production-like accounting, while tests can use an in-memory fee ledger. V1B does not collect fees on chain, move funds, pay rebates, create execution transactions, or broadcast.
- Monitoring/Admin V1B guarded by `ADMIN_API_ENABLED=false` for admin endpoints and `METRICS_ENABLED=true` for `/metrics` by default. Enabled admin mode exposes read-only `/admin/status`, `/admin/config`, `/admin/db`, `/admin/mm/sessions`, `/admin/mm/permissions`, `/admin/execution/summary`, `/admin/rfq/summary`, `/admin/options/summary`, `/admin/fees/summary`, `/admin/fees/events`, `/admin/fees/volumes`, `/admin/fees/rebates`, and `/admin/recent` endpoints. `/metrics` emits safe Prometheus-style text metrics, and `/ready` checks process/config plus DB ping when persistence is enabled. These endpoints are local/dev-oriented, optionally require `X-Admin-Token` for admin and metrics, sanitize secrets, use bounded read/aggregate queries, and do not mutate DB rows, RFQs, option state, execution state, MM sessions, MM permission rows, or fee ledger rows.
- E2E Test Harness V1A under `scripts/e2e` supports safe `admin`, `fees-options`, `fees-perps`, `option-rfq`, `mm-auth`, and `all-safe` flows. It uses standard-library Python, process-only environment overrides, HTTP requests, and `psql` checks; it emits JSON reports and never enables real broadcast by default.

## Monitoring/Admin V1B

Monitoring/Admin V1B is a read-only observability layer inside the existing Axum API. Admin endpoints exist to inspect operational state before a separate frontend/admin service is built. Admin is disabled by default, while metrics are enabled for local Prometheus-style scraping:

```text
ADMIN_API_ENABLED=false
ADMIN_API_REQUIRE_TOKEN=false
ADMIN_API_TOKEN=
METRICS_ENABLED=true
METRICS_REQUIRE_ADMIN_TOKEN=false
```

When enabled, admin endpoints are read-only. `ADMIN_API_REQUIRE_TOKEN=true` performs a simple local/dev header check with `X-Admin-Token`; it is not production authentication. The token is not logged by endpoint code and is never returned by responses.

The admin config response is sanitized. It exposes public network/chain identifiers, feature flags, public contract addresses, signature modes, TTL settings, booleans for RPC/database/private-key configuration, and WebTransport host/port only when the gateway is enabled. It does not expose private keys, raw database URLs, raw RPC URLs, provider keys, admin tokens, or full environment data.

Persistent admin DB reads use simple aggregate/count/recent queries and fixed table names. `/admin/db` reports missing older tables as unavailable in its count response. `/admin/mm/permissions` lists configured MM accounts, capability flags, and product scopes without exposing auth challenges, signatures, tokens, or database URLs. `/admin/fees/*` exposes bounded fee ledger events, volume buckets, rebate accruals, and aggregate totals without any write action. With persistence disabled, admin endpoints use in-memory engine/RFQ/options/MM session/permission/fee snapshots where available and return empty DB/recent sections otherwise. Normal tests do not require live Postgres, RPC, WebTransport, or private keys.

`/metrics` renders Prometheus text without adding a heavy exporter dependency. Core metrics include `deopt_backend_up`, feature/config gauges, `deopt_db_up`, `deopt_db_migrations_installed`, execution/RFQ/options/fees aggregate counts, and MM session gauges. Labels are limited to `status`, `market_type`, `flow_type`, and `source_type`; wallet addresses, tx hashes, UUIDs, raw DB URLs, raw RPC URLs, private keys, admin tokens, signatures, and session internals are not labels and are not rendered.

`/ready` reports process/config readiness and only requires the DB when persistence is enabled. Disabled optional subsystems do not make readiness fail.

`docs/ALERTING_SPEC.md` defines suggested alert rules for backend down, DB down, unexpected real broadcast, execution confirmation stalls, reconciliation anomalies, simulation failures, stale indexer cursors, RFQ rejection spikes, MM session drops, and fee ledger write failures. V1B still defers Grafana dashboards, Prometheus deployment, external alert delivery, frontend monitoring UI, production auth, admin write controls, permission write controls, and risk admin controls.

## Fees & Rebates V1B

Fees & Rebates V1B is an off-chain accounting layer. It is disabled by default:

```text
FEES_ENABLED=false
FEES_REQUIRE_PERSISTENCE=true
FEES_REBATES_ENABLED=false
FEES_PROTOCOL_FEE_RECIPIENT=treasury
FEES_DEFAULT_FEE_ASSET=USDC
FEES_OPTION_FEE_BASIS=premium_or_underlying_capped
FEES_OPTION_PREMIUM_CAP_PCT=10
```

When fees are disabled, option order matching, option RFQ acceptance, and execution confirmation preserve existing behavior and write no fee ledger state. When fees are enabled with `FEES_REQUIRE_PERSISTENCE=true`, startup requires `PERSISTENCE_ENABLED=true`; normal tests can run with `FEES_REQUIRE_PERSISTENCE=false` and the in-memory `FeeLedgerStore`.

Rates are represented as integer `micro_bps`. One basis point is `10_000` micro-bps and the denominator is `100_000_000`, so sub-basis-point schedule entries such as `0.75` bps are exact. The launch schedule is hardcoded in Rust with tier 0 through tier 4 perp and option maker/taker rates, RFQ discounts for options, and fields for 28-day volume, volume-share, and staked-DEOPT thresholds. V1B tier resolution uses ledger volume buckets; volume-share and staking enforcement are deferred.

Option fee events use `premium_or_underlying_capped`: compute an underlying-notional fee from `underlying_notional_1e8 * rate_micro_bps / 100_000_000`, compute `premium_notional_1e8 * FEES_OPTION_PREMIUM_CAP_PCT / 100`, then record the lower amount. The same cap applies to rebate accrual amounts.

Maker/taker classification is source-driven:
- Option orderbook fills use the resting order as maker and incoming order as taker.
- Option RFQ fills use `mm_account` as maker and the RFQ taker as taker.
- Confirmed perp trades use indexed `buyer_is_maker`: buyer is maker when true, otherwise seller is maker.

Perp fees are generated only from confirmed indexed trades. The confirmation path must have a successful receipt, enough confirmations, a matching indexed `TradeExecuted` event, and a matched reconciliation before calling fee recording. Perp notional is `execution_price_1e8 * size_1e8 / 1e8` with checked arithmetic. Confirmed perp rows use `source_type=perp_trade`, `source_id=<tx_hash>:<log_index>`, `market_type=perp`, and `flow_type=orderbook` unless the confirmed execution intent is linked to an accepted RFQ, in which case `flow_type=rfq`.

Persistence adds:
- `fee_events`: idempotent ledger rows keyed by source type, source id, payer, and recipient.
- `volume_buckets`: daily maker/taker/total volume by account and market type.
- `rebate_accruals`: ledger-only rebate accruals linked to fee events.

Rebate accrual is intentionally narrow. It requires `FEES_REBATES_ENABLED=true`, `MM_PERMISSIONS_ENABLED=true`, an enabled MM permission account, and the relevant capability. Option RFQ fills require `can_quote_option_rfq`; option order fills require `can_submit_option_orders` or `can_quote_option_rfq`; confirmed perp orderbook trades require `can_submit_perp_orders`; confirmed perp RFQ trades require `can_quote_perp_rfq`. If permissions are disabled, missing, or disabled for the maker, fee events can still be recorded but rebate accrual remains zero.

Admin fee endpoints are read-only:

```text
GET /admin/fees/summary
GET /admin/fees/events?limit=20
GET /admin/fees/volumes?account=0x...
GET /admin/fees/rebates?account=0x...
```

V1B does not implement on-chain fee collection, rebate payouts, claim contracts, admin write endpoints, frontend fee dashboards, fund movement, execution transaction creation, or broadcast.

## Options V1D / Option RFQ V1C

Options V1D is an isolated product/data layer for option series, off-chain option orders, off-chain fills, and option RFQs. Option RFQ V1C integrates that option RFQ layer with the Market Maker Gateway and adds signed market-maker option RFQ quotes without moving option business logic into the WebTransport adapter. It does not alter the existing perp orderbook, perp RFQ flow, MM Gateway perp order path, execution-intent lifecycle, simulation, broadcast, indexer, reconciliation, or confirmation logic.

The option series model stores:
- `option_series_id`
- `underlying`, `base_asset`, `quote_asset`, `settlement_asset`
- `expiry` as Unix seconds
- `strike_1e8` and `contract_size_1e8` as integer fixed-point values
- `is_call` where `true` is call and `false` is put
- `status` as `active`, `expired`, or `disabled`
- `source` as `manual` or `onchain`
- optional `onchain_product_id` and `onchain_series_id`
- created and updated timestamps in milliseconds

`option_series_id` is deterministic and restart-stable. It is the `0x` hex Keccak-256 hash of lowercase-trimmed `underlying/base/quote/settlement`, `expiry`, `strike_1e8`, `is_call`, and `contract_size_1e8`. Duplicate manual series creation returns the existing series rather than creating another row.

Options configuration is disabled by default:

```text
OPTIONS_ENABLED=false
OPTIONS_REQUIRE_PERSISTENCE=true
OPTIONS_ALLOW_MANUAL_SERIES=true
OPTIONS_SYNC_ONCHAIN_REGISTRY=false
OPTIONS_DEFAULT_CONTRACT_SIZE_1E8=100000000
OPTION_RFQ_ENABLED=false
OPTION_RFQ_REQUIRE_PERSISTENCE=true
OPTION_RFQ_DEFAULT_TTL_MS=5000
OPTION_RFQ_MAX_TTL_MS=30000
OPTION_RFQ_MIN_QUOTE_TTL_MS=500
OPTION_RFQ_MAX_QUOTE_TTL_MS=10000
OPTION_RFQ_MAX_QUOTES_PER_RFQ=50
OPTION_RFQ_QUOTE_SIGNATURE_MODE=disabled
OPTION_RFQ_EIP712_NAME=DeOptV2OptionRFQ
OPTION_RFQ_EIP712_VERSION=1
OPTION_RFQ_EIP712_CHAIN_ID=84532
OPTION_RFQ_EIP712_VERIFYING_CONTRACT=0x0000000000000000000000000000000000000000
OPTION_EXECUTION_ENABLED=false
OPTION_EXECUTION_REQUIRE_PERSISTENCE=true
OPTION_MATCHING_ENGINE_ADDRESS=
OPTION_EXECUTION_SIGNATURE_MODE=disabled
OPTION_EXECUTION_CHAIN_ID=84532
OPTION_EXECUTION_EIP712_NAME=DeOptV2-OptionMatchingEngine
OPTION_EXECUTION_EIP712_VERSION=1
OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS=6
OPTION_NONCE_SYNC_ENABLED=false
OPTION_NONCE_SYNC_REQUIRE_RPC=true
OPTION_NONCE_SYNC_STRICT=true
OPTION_EXECUTION_SIMULATION_ENABLED=false
OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true
OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0
OPTION_EXECUTION_SIMULATION_FROM=
OPTION_EXECUTION_BROADCAST_ENABLED=false
OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true
OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0
```

When options are enabled with `OPTIONS_REQUIRE_PERSISTENCE=true`, startup requires persistence. When option RFQ is enabled with `OPTION_RFQ_REQUIRE_PERSISTENCE=true`, startup also requires persistence. When option execution is enabled with `OPTION_EXECUTION_REQUIRE_PERSISTENCE=true`, startup requires persistence and a valid nonzero `OPTION_MATCHING_ENGINE_ADDRESS`; that address is also the EIP-712 verifying contract. Normal tests can run options, option RFQs, and option execution intent signing/calldata in memory with persistence disabled. `OPTION_RFQ_QUOTE_SIGNATURE_MODE=disabled` preserves unsigned quotes; `strict` requires `quote_nonce`, a valid EIP-712 `OptionRFQQuote` signature, and recovered signer equality with `mm_account`. `OPTION_EXECUTION_SIGNATURE_MODE=disabled` shape-validates provided OptionTrade signatures only; `strict` recovers buyer/seller signers from the OptionTrade digest. `OPTION_NONCE_SYNC_ENABLED=true` enables read-only `OptionMatchingEngine.nonces(address)` sync using `RPC_URL` and `OPTION_MATCHING_ENGINE_ADDRESS`; by default startup requires both and strict intent creation fails if buyer or seller nonce cannot be read. `OPTION_EXECUTION_SIMULATION_ENABLED=true` enables manual option `eth_call` simulation, requires `RPC_URL` by default, and uses `OPTION_EXECUTION_SIMULATION_FROM` or `EXECUTOR_FROM_ADDRESS` as the call sender. `OPTION_EXECUTION_BROADCAST_ENABLED=true` additionally requires `OPTION_EXECUTION_ENABLED=true`, `EXECUTION_ENABLED=true`, `EXECUTOR_REAL_BROADCAST_ENABLED=true`, `RPC_URL`, `EXECUTOR_PRIVATE_KEY`, and `OPTION_MATCHING_ENGINE_ADDRESS`; by default it also requires persisted `simulation_ok`. `OPTIONS_SYNC_ONCHAIN_REGISTRY=false` is a placeholder for a future read-only registry sync and does not call RPC in V1I.

HTTP endpoints:
- `POST /options/series`
- `GET /options/series`
- `GET /options/series/:option_series_id`
- `POST /options/series/:option_series_id/disable`
- `POST /options/orders`
- `GET /options/orders`
- `GET /options/orders/:order_id`
- `GET /options/orders/:order_id/fills`
- `POST /options/orders/:order_id/cancel`
- `GET /options/fills`
- `GET /options/fills/:fill_id`
- `GET /options/orderbooks/:option_series_id`
- `POST /options/rfqs`
- `GET /options/rfqs`
- `GET /options/rfqs/:option_rfq_id`
- `POST /options/rfqs/:option_rfq_id/quote-signing-payload`
- `POST /options/rfqs/:option_rfq_id/quotes`
- `GET /options/rfqs/:option_rfq_id/quotes`
- `POST /options/rfqs/:option_rfq_id/accept/:quote_id`
- `POST /options/rfqs/:option_rfq_id/cancel`

`GET /options/series` supports `underlying`, `expiry`, `is_call`, and `status` filters. Option orders store `order_id`, `option_series_id`, `account`, side, fixed-point price/size/remaining size, `time_in_force`, optional client id, optional nonce/deadline/signature, status, and timestamps. V1C accepts only `gtc` orders, rejects unsupported `ioc`/`fok`, shape-validates optional signatures, and rejects duplicate live `(account, client_order_id)` submissions.

Matching is off-chain only and restricted to one `option_series_id`. Incoming buys match live resting asks when `buy.price_1e8 >= ask.price_1e8`; incoming sells match live resting bids when `sell.price_1e8 <= bid.price_1e8`. The fill size is the min remaining size. Fill price is always the resting maker order price. Price-time priority uses lowest ask first for incoming buys, highest bid first for incoming sells, then `created_at_ms` and `order_id`.

Option fills store `fill_id`, series id, buy/sell order ids, buyer/seller, maker/taker order ids, taker side, fixed-point price/size, and timestamp. `GET /options/fills` supports `option_series_id`, `account`, and `order_id` filters. Persistent order insertion, maker/taker updates, and fill writes happen in one transaction.

`GET /options/orders` supports `option_series_id`, `account`, `status`, and `side` filters. `POST /options/orders/:order_id/cancel` moves open or partially filled orders to `cancelled`; filled and already-cancelled orders return clear state errors. `GET /options/orderbooks/:option_series_id` validates that the series exists and returns aggregated live remaining bid/ask levels, with bids sorted by integer price descending and asks ascending.

`POST /options/rfqs` creates an RFQ for an active series with taker-perspective `side`, requested `size_1e8`, optional limit price, and bounded TTL. Creation broadcasts an `option_rfq_request` to all active MM Gateway sessions on a best-effort basis. Zero connected sessions and failed notifications do not block RFQ creation. `POST /options/rfqs/:option_rfq_id/quote-signing-payload` returns the option RFQ EIP-712 domain, `OptionRFQQuote` fields, message, and digest. `POST /options/rfqs/:option_rfq_id/quotes` stores active MM quotes with fixed-point premium price/size, bounded quote TTL, optional quote nonce, optional signature, and signature metadata, rejecting duplicate `(option_rfq_id, mm_account, client_quote_id)` values when a client id is supplied.

The signed option RFQ quote payload is:

```text
OptionRFQQuote(bytes32 optionRfqId,address mmAccount,bytes32 optionSeriesId,bool takerIsBuyer,uint128 price1e8,uint128 size1e8,uint256 quoteNonce,uint256 expiry)
```

`optionRfqId` is `keccak256(bytes(option_rfq_uuid_string))`, `optionSeriesId` is the 32-byte option series id, `takerIsBuyer` is true when RFQ side is `buy`, and `expiry` is encoded in seconds.

Market Maker Gateway option RFQ protocol messages:
- `option_rfq_request`: server-initiated push with option RFQ id, taker, option series id, side, size, optional limit price, and expiry.
- `option_rfq_quote`: MM-submitted quote with option RFQ id, MM account, premium price, size, optional client quote id, optional quote nonce, optional signature, and quote TTL.
- `option_rfq_quote_result`: gateway response containing the persisted quote id, option RFQ id, active status, and quote expiry.
- `option_rfq_quote_accepted`: best-effort notification to the accepted quote session with option RFQ id, quote id, and option RFQ fill id.
- `option_rfq_quote_rejected`: best-effort notification to active competing quote sessions after another quote wins.

Gateway-submitted option RFQ quotes are stored through the same option RFQ service and table as HTTP/dev quotes, including `session_id`, `mm_account`, `client_quote_id`, premium price, size, status, expiry, signature, quote digest, quote nonce, signature status, and recovered signer when supplied. The WebTransport adapter remains limited to connection/session/framing mechanics; option RFQ validation, signature verification, and persistence stay in `options` and `mm` service layers.

Option RFQ acceptance is single-winner. It validates RFQ and quote freshness, series active status, quote ownership, quote size, and taker limit semantics. Taker `buy` requires quote price at or below the limit and maps `buyer=taker`, `seller=mm_account`, `taker_side=buy`; taker `sell` requires quote price at or above the limit and maps `buyer=mm_account`, `seller=taker`, `taker_side=sell`. Acceptance marks the RFQ and winning quote accepted, rejects competing active quotes, creates an `option_rfq_fills` row, and sends best-effort MM accepted/rejected notifications when quotes have connected sessions. Notification failure may be surfaced as a warning but never reverts acceptance. It never creates perp `execution_intents` or `execution_transactions`, and it never broadcasts.

## Option Execution Intents V1I

Option Execution Intents V1I is opt-in and scoped to artifacts, read-only nonce sync, `eth_call` simulation, and controlled disabled-by-default broadcast for the Solidity `OptionMatchingEngine`. When `OPTION_EXECUTION_ENABLED=false`, option orderbook fills and option RFQ fills keep the previous off-chain behavior and no option execution intent rows are created.

When enabled, each option orderbook fill and accepted option RFQ fill creates one idempotent `OptionExecutionIntent` keyed by `(source_type, source_id)`. The source types are `option_orderbook_fill` and `option_rfq_fill`. Orderbook `buyer_is_maker` is derived from the resting side: a resting bid makes the buyer maker, and a resting ask makes the seller maker. Option RFQ `buyer_is_maker` is true when the market maker is the buyer.

The service requires executable series metadata before creating a matching fill. The option series must have an on-chain option id in `onchain_series_id` or `onchain_product_id`, and `underlying` and `settlement_asset` must be nonzero EVM addresses. `size_1e8` must convert to whole contracts with `quantity = size_1e8 / 100000000`; fractional-contract fills are rejected while option execution is enabled. Premium converts from backend fixed point to settlement-token native units as `premiumPerContract = price_1e8 * 10^OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS / 100000000`, and a zero result is rejected.

Intent state stores the backend UUID, derived `onchain_intent_id = keccak256(bytes(uuid_string))`, source fill identity, series id, on-chain option id, buyer/seller, underlying/settlement asset, expiry, strike, call/put flag, contract size, converted quantity, source price/size, converted premium, maker flag, buyer/seller nonces, deadline, optional signatures, optional calldata, status, error, nullable simulation result fields, and timestamps. With `OPTION_NONCE_SYNC_ENABLED=false`, buyer and seller nonces default to `0`. With `OPTION_NONCE_SYNC_ENABLED=true`, creation reads `OptionMatchingEngine.nonces(address)` for both parties and stores those values before any signing payload or calldata is produced.

Endpoints:

```text
GET /options/execution-intents
GET /options/execution-intents/:intent_id
GET /options/execution-intents/:intent_id/signing-payload
POST /options/execution-intents/:intent_id/signatures
GET /options/execution-intents/:intent_id/calldata
POST /options/execution-intents/:intent_id/simulate
GET /options/execution-intents/:intent_id/simulation
POST /options/execution-intents/:intent_id/broadcast
GET /accounts/:address/option-nonce
```

The signing-payload endpoint returns the independent option execution EIP-712 domain and a digest for:

```text
OptionTrade(bytes32 intentId,address buyer,address seller,uint256 optionId,address underlying,address settlementAsset,uint64 expiry,uint64 strike1e8,bool isCall,uint128 contractSize1e8,uint128 quantity,uint128 premiumPerContract,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)
```

The signature endpoint stores buyer and seller signatures. In disabled signature mode it validates only 65-byte `0x` signature shape. In strict mode it recovers the EIP-712 signer from the `OptionTrade` digest and requires buyer signature equality with `buyer` and seller signature equality with `seller`. Once both signatures are present, the calldata builder encodes `OptionMatchingEngine.executeTrade(OptionTrade,bytes,bytes)` and marks the intent `calldata_ready`.

The option nonce endpoint performs `eth_call` to `OPTION_MATCHING_ENGINE_ADDRESS` with `data=nonces(address)`, `value=0`, and the zero address as `from`. It returns `source=onchain` and the nonce as a decimal string. It is read-only and does not require a private key. When `OPTION_NONCE_SYNC_STRICT=true`, intent creation fails if either nonce read fails and no option execution intent is inserted. Non-strict mode falls back to zero nonces and is documented as local-development only.

The simulation endpoint validates an existing calldata-ready intent and performs `eth_call` to `OPTION_MATCHING_ENGINE_ADDRESS` with `data=intent.calldata`, `value=0`, optional configured gas, and `from=OPTION_EXECUTION_SIMULATION_FROM` or `EXECUTOR_FROM_ADDRESS`. It persists `simulation_ok`, `simulation_failed`, or `simulation_unavailable` with error/revert data and block number where available. Because `OptionMatchingEngine.executeTrade` is executor-gated, the simulation sender must be an allowed executor on-chain or the call will revert.

The broadcast endpoint is disabled unless `OPTION_EXECUTION_BROADCAST_ENABLED=true`. It requires executable calldata, buyer and seller signatures, `EXECUTION_ENABLED=true`, `EXECUTOR_REAL_BROADCAST_ENABLED=true`, `RPC_URL`, `EXECUTOR_PRIVATE_KEY`, and a nonzero `OPTION_MATCHING_ENGINE_ADDRESS`. With `OPTION_EXECUTION_REQUIRE_SIMULATION_OK=true`, the latest persisted option simulation status must be `simulation_ok`. The request signs and sends only through the existing transaction broadcaster abstraction, targets `OPTION_MATCHING_ENGINE_ADDRESS`, uses `value=0`, and applies `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` only when nonzero. Successful sends persist `option_execution_transactions.status=submitted` with the provider-returned tx hash and move the intent to `broadcast_submitted`. Failed sends persist `status=failed` without a tx hash and move the intent to `broadcast_failed`. Duplicate calls for an already submitted intent return the stored transaction metadata without rebroadcasting.

V1I safety boundaries are explicit: option execution broadcast does not call `/executor/broadcast`, does not create perp `execution_transactions`, does not fabricate tx hashes, does not require live RPC in normal tests, and does not set confirmed statuses. Indexing, reconciliation, confirmation, and option exercise/settlement workflows remain deferred. Greek, IV, risk-cache, settlement, and exercise behavior do not exist in V1I.

## RFQ V1C

RFQ V1C is an HTTP/core service integrated with the Market Maker Gateway. A taker creates an RFQ through HTTP, the backend broadcasts an `rfq_request` to connected MM sessions when available, market makers can fetch an EIP-712 `RFQQuote` signing payload, submit quotes through either WebTransport `rfq_quote` or the HTTP/dev endpoint, the taker lists quotes through HTTP, and accepting one quote creates a normal pending `ExecutionIntent`. The accepted intent then follows the same signing, simulation, guarded broadcast, indexing, reconciliation, and confirmation lifecycle as orderbook-created intents.

RFQ statuses are `open`, `expired`, `accepted`, `cancelled`, and `failed`. Quote statuses are `active`, `expired`, `accepted`, `rejected`, and `cancelled`. Expiry is enforced at quote submission and acceptance time. Accepting a quote is single-winner: the RFQ becomes `accepted`, the winning quote becomes `accepted`, and all other active quotes for that RFQ become `rejected`.

RFQ side is from the taker perspective:

```text
side=buy  -> buyer=taker,      seller=mm_account, buyer_is_maker=false
side=sell -> buyer=mm_account, seller=taker,      buyer_is_maker=true
```

RFQ acceptance creates only the execution intent. It does not sign PerpTrade payloads, does not simulate, does not broadcast, does not create transaction hashes, and does not mark confirmation or finality. In persistent mode, the accepted quote, RFQ status, deterministic quote rejection, execution-intent insert, and engine audit event are written in one SQL transaction. In-memory mode is available for normal offline tests and local development.

Signed quote verification is controlled by `RFQ_QUOTE_SIGNATURE_MODE`:
- `disabled`: preserves the unsigned RFQ V1B flow. Signatures are optional, cryptographic recovery is skipped, and stored quotes use `signature_status=not_required`.
- `strict`: requires `quote_nonce` and a 65-byte ECDSA signature. The backend recomputes the EIP-712 digest, recovers the signer, and rejects the quote unless the recovered address equals `mm_account`. Active accepted quotes in this mode are verified quotes.

The signed typed payload is:

```text
RFQQuote(bytes32 rfqId,address mmAccount,uint256 marketId,bool takerIsBuyer,uint128 price1e8,uint128 size1e8,uint256 quoteNonce,uint256 expiry)
```

`rfqId` is `keccak256(bytes(rfq_uuid_string))`. `takerIsBuyer` is true when RFQ `side=buy`. `quoteNonce` is market-maker supplied. `expiry` is encoded in seconds and currently binds the quote signature to the RFQ expiry; the persisted quote can still expire earlier through `quote_ttl_ms`.

RFQ signatures use a separate configurable EIP-712 domain: `RFQ_EIP712_NAME`, `RFQ_EIP712_VERSION`, `RFQ_EIP712_CHAIN_ID`, and `RFQ_EIP712_VERIFYING_CONTRACT`. The default name is `DeOptV2RFQ` and the verifying contract defaults to the zero address for off-chain/dev operation.

HTTP endpoints:
- `POST /rfqs`
- `GET /rfqs`
- `GET /rfqs/:rfq_id`
- `POST /rfqs/:rfq_id/quote-signing-payload`
- `POST /rfqs/:rfq_id/quotes`
- `GET /rfqs/:rfq_id/quotes`
- `POST /rfqs/:rfq_id/accept/:quote_id`
- `POST /rfqs/:rfq_id/cancel`

Market Maker Gateway RFQ protocol messages:
- `rfq_request`: server-initiated push to active MM sessions with RFQ id, taker, market, side, size, optional limit price, and expiry.
- `rfq_quote`: MM-submitted quote with RFQ id, MM account, price, size, optional client quote id, quote TTL, optional quote nonce, and optional signature. In strict mode, quote nonce and signature are required.
- `rfq_quote_result`: gateway response containing the persisted quote id, RFQ id, active status, and expiry.
- `rfq_quote_accepted`: best-effort notification to the accepted quote session with execution intent id and on-chain intent id.
- `rfq_quote_rejected`: best-effort notification to active competing quote sessions after another quote wins.

The session registry is transport-neutral and stores active session snapshots plus outbound message channels. WebTransport code only registers/unregisters sessions and writes server-initiated frames; RFQ validation, signature verification, option RFQ validation, persistence, session auth, and acceptance behavior stay in RFQ/options/MM service layers. Signed taker RFQ requests, market-maker selection/ranking, multi-leg RFQ, and expiry schedulers remain deferred.

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

Future RFQ phases add production wallet authentication, signed taker RFQ requests, signed quote cancellation, options and multi-leg RFQs, market-maker ranking, auction logic, richer lifecycle notifications, and expiry schedulers. RFQ V1C intentionally stops at signed single-leg perp quote verification, gateway quote persistence, and best-effort acceptance/rejection notifications.

## Market Maker Gateway V1A

The Market Maker Gateway is the future low-latency market-maker ingress path for heartbeats, session state, bulk order operations, quote replacement, and later market data. WebTransport is the strategic transport because it provides HTTP/3 over QUIC, reliable streams, connection-level sessions, and optional datagrams. V1A deliberately isolates the protocol and service layer from concrete WebTransport crate types. V1B will add the `wtransport` dependency, TLS certificate/key loading, UDP listener startup, JSON framing over reliable bidirectional streams, and local WebTransport smoke tooling.

V1A client messages are JSON envelopes with `type`, `request_id`, and `payload`. Success responses use `type="<message>_result"`, `request_id`, `ok=true`, and `payload`. Errors use `type="error"`, `request_id`, `ok=false`, and stable error codes. The defined client message set is `auth`, `heartbeat`, `submit_order`, `bulk_submit`, `cancel_order`, `bulk_cancel`, `cancel_all`, `quote_replace`, and `get_session`.

The session model is transport-neutral and serializable. It tracks `session_id`, `connection_id`, optional authenticated `account`, `authenticated`, `auth_mode`, pending challenge account/nonce/issued/expires metadata, connection and heartbeat timestamps, `cancel_on_disconnect`, open client order ids, per-second message-window counters, and in-flight request count. Public snapshots expose only sanitized auth state, challenge active/expiry state, and counts/ids already considered public; they do not expose challenge nonces or signatures.

The safe auth default is `MM_GATEWAY_AUTH_MODE=disabled` and `MM_GATEWAY_REQUIRE_AUTH=false`, which preserves development sessions that process account-bearing messages without wallet challenge auth. `MM_GATEWAY_AUTH_MODE=wallet_challenge` starts sessions unauthenticated. Clients call `auth_challenge`, sign the deterministic challenge string with Ethereum personal-sign semantics, and call `auth_verify`; successful verification recovers the signer and binds the session account. When `MM_GATEWAY_REQUIRE_AUTH=true`, unauthenticated sessions may only call `heartbeat`, `get_session`, `auth_challenge`, and `auth_verify`. Order, quote, and RFQ messages require auth, and payload `account` / `mm_account` values must equal the authenticated session account using case-insensitive address comparison.

Rate-limit decisions are pure and cover max messages per second, max in-flight requests per session, max orders per bulk, max cancels per bulk, and max open client order ids per account/session. These limits are parsed from `MM_GATEWAY_*` configuration with safe defaults and no certificate or UDP validation in V1A.

Bulk submit and bulk cancel support structured partial results. Quote replace accepts optional bid and ask legs plus `cancel_previous`, returns deterministic planned counts, and records planned client order ids in the session. V1A does not call Axum handlers, does not mutate the live orderbook, does not fabricate backend order ids, does not create execution intents, and does not broadcast transactions.

Cancel-on-disconnect is planning-only. It returns the session's currently open client order ids when enabled. It never touches execution intents, submitted transactions, confirmed transactions, or chain state.

## Market Maker Gateway V1B

V1B adds a concrete WebTransport adapter using `wtransport` while keeping WebTransport crate types isolated to `src/mm/transport/webtransport.rs`. The adapter is disabled by default. When `MM_GATEWAY_ENABLED=true`, startup requires `MM_GATEWAY_TRANSPORT=webtransport`, `MM_GATEWAY_CERT_PATH`, and `MM_GATEWAY_KEY_PATH`, loads the TLS identity, binds UDP `MM_GATEWAY_HOST:MM_GATEWAY_PORT`, and spawns the gateway beside the HTTP Axum TCP server.

The WebTransport adapter accepts sessions and reliable bidirectional streams. Critical gateway request/response messages use deterministic length-prefixed JSON frames: `u32` big-endian length followed by JSON bytes, with a 1 MiB maximum. V1B handles one client request per bidirectional stream and returns one framed response. Server-initiated RFQ notifications use outbound WebTransport streams from the same transport adapter. Datagrams are intentionally deferred for market data and are not used for trading commands.

Each decoded client message is passed to `MmGatewayService::handle_message`, which updates the transport-neutral session and returns a transport-neutral server message. RFQ messages are routed through the RFQ service and store. The adapter registers sessions in a transport-neutral registry and writes outbound server messages, but does not contain RFQ business logic.

V1B does not implement pricing logic, production wallet challenge auth, WebSocket fallback, live gateway orderbook mutation, or auto-broadcast. Normal tests cover framing, config validation, RFQ protocol serialization, gateway RFQ and option RFQ quote handling, and session notifications without binding UDP, loading cert files, calling RPC, connecting to Postgres, or using private keys.

## Market Maker Gateway V1C

V1C connects `MmGatewayService` to the shared `orders` service. The same signed-order validation used by HTTP applies to gateway `submit_order` and each `bulk_submit` item: deadline, signature mode, known market, optional strict Perp nonce sync, local nonce reservation, engine matching, execution-intent creation, and optional persistence. The gateway never signs, simulates, broadcasts, indexes, reconciles, or confirms.

Gateway cancels are ownership checked against the session account. `cancel_order` and `bulk_cancel` can address resting off-chain orders by backend `order_id` or `client_order_id`. `cancel_all` cancels all resting orders for the session account, optionally filtered by market. These paths call engine cancellation only for live resting orders and do not alter submitted/broadcast/confirmed execution intents.

`quote_replace` is deterministic and non-atomic: cancel previous tracked quote client order ids for the account and market first when requested, then submit the new bid and ask independently as GTC orders. The response includes cancelled, submitted, rejected, per-leg results, backend order ids, and matched execution intent ids when immediate matching occurs.

Cancel-on-disconnect is live in V1C. On WebTransport session close, the adapter calls the transport-neutral service to cancel tracked resting session orders when enabled, then logs the cancellation summary. WebTransport-specific code remains isolated to `src/mm/transport/webtransport.rs`; orderbook business logic remains outside the transport adapter.

## MM Permissions V1A

MM Permissions V1A adds an authorization layer after MM identity is established. Wallet challenge authentication answers "which account controls this session"; permissions answer "what is that account allowed to do." When `MM_PERMISSIONS_ENABLED=false`, all existing MM gateway and RFQ behavior is preserved.

Configuration:

```text
MM_PERMISSIONS_ENABLED=false
MM_PERMISSIONS_REQUIRE_PERSISTENCE=true
```

When permission enforcement is enabled, startup requires persistence unless `MM_PERMISSIONS_REQUIRE_PERSISTENCE=false`. This keeps production-like allowlists durable while allowing normal offline tests to seed an in-memory `MmPermissionsStore`.

Permission state has two tables:
- `mm_accounts`: one row per MM account with `enabled`, optional `label`, and capability booleans.
- `mm_market_permissions`: optional product scopes. Rows can target a perp `market_id`, an `option_series_id`, or be global when both are null.

Enforced capabilities:
- Perp RFQ quote submission requires `enabled=true`, `can_quote_perp_rfq=true`, and allowed `market_id` when perp market scopes are configured.
- Option RFQ quote submission requires `enabled=true`, `can_quote_option_rfq=true`, and allowed `option_series_id` when option-series scopes are configured.
- Gateway `submit_order`, `bulk_submit`, and `quote_replace` require `enabled=true`, `can_submit_perp_orders=true`, and allowed `market_id` when perp market scopes are configured.

Scope semantics are intentionally simple. If an account has no scope rows for a product type, the account-level capability applies globally for that product type. Once any scope row exists for that product type, an enabled exact row or enabled global row is required. Disabled scope rows do not grant access. Cancels remain ownership-checked but are not capability-blocked so a disabled MM is not prevented from reducing already-resting off-chain exposure.

The admin surface is read-only:

```text
GET /admin/mm/permissions
```

V1A does not add admin write endpoints, frontend permission UI, automatic onboarding, ranking/scoring, rebate payout workflows, on-chain allowlists, Solidity changes, deployments, or real broadcast behavior.

## Execution-Intent Flow

Every matched trade creates an `ExecutionIntent` with buyer, seller, order IDs, market, price, size, buyer maker flag, buyer/seller order nonces, execution deadline, timestamp, and `Pending` status. Intents are stored in memory and exposed through `GET /execution-intents`.

The execution deadline is the minimum of the two original signed-order deadlines. Direct in-memory orders that do not carry nonce/deadline metadata can still match, but their signing-payload endpoint fails clearly instead of inventing missing PerpTrade fields.

The intended lifecycle is: order accepted, matched, execution intent created, PerpTrade signatures collected, calldata ready, simulation with revert diagnostics when needed, simulation ok, prepared transaction, submitted by the explicitly gated executor, indexed by the indexer, reconciled as matched by direct intent id, and confirmed only after receipt success, enough blocks, indexed event identity, and matched reconciliation. Real Broadcast V1 stops at submitted after a real RPC tx hash; Reconciliation V1 stops at the matched reconciliation row.

## Persistence V1

Persistence is disabled by default. With `PERSISTENCE_ENABLED=false`, the service does not connect to Postgres and retains the original in-memory nonce, orderbook, and execution-intent behavior.

With `PERSISTENCE_ENABLED=true`, startup requires `DATABASE_URL`, connects to Postgres, and runs migrations. The repository persists used nonces, accepted orders, matched trades, execution intents, cancellation/status updates, and a basic `engine_events` audit stream. Persisted nonce uniqueness is scoped to `(account, nonce)`, which prevents replay after restart while still allowing different accounts to use the same nonce.

MM Permissions V1A adds `mm_accounts` and `mm_market_permissions` migrations for manual SQL onboarding and read-only admin visibility. Application code reads these tables for enforcement and admin display; it does not write approval state. Fees & Rebates V1B uses `fee_events`, `volume_buckets`, and `rebate_accruals` ledger tables; these rows are accounting records only and do not move funds.

The in-memory engine remains the live matching state in this V1 patch. Database writes for an order submission use a SQL transaction for nonce insertion and event persistence, but full engine-state rollback/replay around database failure is deferred.

## Blockchain Execution Boundary

This repository does not execute on-chain transactions in phase 1 by default. With default configuration it does not sign transactions, retain private keys, broadcast transactions, or mark trades as finally settled. Manual simulation can call an RPC endpoint with `eth_call` only. Real Broadcast V1 can submit a transaction only when `EXECUTOR_REAL_BROADCAST_ENABLED=true` and the required private key, RPC URL, static fee configuration, persistence, signatures, calldata, and simulation gates are present.

The current calldata builder V1 can encode `PerpMatchingEngine.executeTrade(PerpTrade,bytes,bytes)` using an explicit `PerpTradePayload` and explicit buyer/seller trade signatures. The Solidity `PerpTrade.intentId` is `keccak256(bytes(execution_intents.intent_id))`, returned as `0x` plus 64 hex chars and used consistently in EIP-712 signing, calldata, and indexed-event reconciliation. `PerpTrade` signatures are distinct from the off-chain order signatures verified by the order API: the Solidity contract verifies signatures over the final matched trade payload, not the original order payloads. The builder therefore does not reuse order signatures and does not fabricate missing signatures.

Clients fetch the final EIP-712 `PerpTrade` payload from `GET /execution-intents/:intent_id/signing-payload`, sign it externally, and submit the two signatures to `POST /execution-intents/:intent_id/signatures`. Signatures are shape-validated and stored in memory or in `execution_intent_signatures` when persistence is enabled. Calldata readiness is true only when both signatures are present and the intent has complete PerpTrade metadata.

For local testnet development, `src/bin/sign_perp_trade.rs` can sign the backend-provided payload digest with a throwaway buyer or seller private key supplied through the process environment. It validates that the derived signer address matches `message.buyer` or `message.seller` by default and emits JSON for manual submission. This CLI is not part of the production request path, does not store keys, and does not make the backend a custodial signer. Production signing must happen in user wallets, market-maker wallets, or external signing systems controlled outside this server.

Intent-derived executor dry-runs produce non-executable previews when trade signatures are unavailable. When both signatures are stored, the dry-run builder can construct real calldata while still marking the prepared call non-broadcastable.

Prepared execution calls remain non-broadcastable in this phase. `is_broadcastable=false`, transaction `value=0`, and no submitted or confirmed lifecycle state is produced by the calldata builder.

Simulation V1 is a manual safety check exposed by `POST /executor/simulate/:intent_id`. It loads the intent and stored PerpTrade signatures, rebuilds the same `executeTrade` calldata, and performs `eth_call` from `EXECUTOR_FROM_ADDRESS` or the zero address to `PERP_MATCHING_ENGINE_ADDRESS`. A successful call marks the intent `simulation_ok`; a revert or RPC failure marks it `simulation_failed` with the error text. Revert Diagnostics V1 extracts revert data from common JSON-RPC error shapes, decodes Solidity `Error(string)` and `Panic(uint256)`, maps known custom-error selectors such as `InvalidSignature`, `NotAuthorized`, `InsufficientMargin`, and `OracleStale`, and persists the raw data, selector, and decoded JSON on the simulation row. These statuses and diagnostics are a safety gate before real broadcast; they are not submission, confirmation, settlement, or finality.

Option Execution Simulation V1D is separate from the perp executor and is exposed by `POST /options/execution-intents/:intent_id/simulate` plus `GET /options/execution-intents/:intent_id/simulation`. It uses the existing `EthCallProvider` abstraction to call `OPTION_MATCHING_ENGINE_ADDRESS` with stored option calldata and no value. It stores only option simulation fields on `option_execution_intents`; it does not create transaction rows, does not call `/executor/broadcast`, does not submit transactions, and does not affect submitted or confirmed lifecycle state.

Option Nonce Sync V1E is also separate from the perp executor. It exposes `GET /accounts/:address/option-nonce` and uses the same `EthCallProvider` abstraction to call `OptionMatchingEngine.nonces(address)`. When enabled, new option execution intents store synced buyer/seller nonces and downstream signing/calldata paths use those stored values unchanged. It never signs, broadcasts, or creates transaction rows.

Option Execution Broadcast V1I is exposed by `POST /options/execution-intents/:intent_id/broadcast` and remains disabled until every option and executor broadcast gate is explicitly enabled. It uses `TransactionBroadcastProvider`, persists tx hashes only after successful sends in `option_execution_transactions`, returns existing submitted transaction metadata on duplicate calls, and leaves option confirmation/reconciliation/indexing out of scope.

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
- Fee rates use integer `micro_bps`; fee and rebate ledgers are off-chain accounting only and do not imply funds have moved.
- Disabled signature mode is for local development only; strict mode requires recovered EIP-712 signer equality with `order.account`.
- MM auth proves session identity; MM permissions are a separate allowlist/capability layer and are disabled by default.

## Out of Scope

No Redis, frontend auth UI, frontend permissions UI, frontend fee dashboards, admin write controls, automatic MM approval, ranking/scoring, rebate payouts, claim contracts, on-chain fee collection, TypeScript, Python service code, C++, or Solidity changes. Blockchain RPC is limited to manual `eth_call` simulation, opt-in `eth_getLogs` indexing, confirmation receipt/block reads, and explicitly gated `eth_sendRawTransaction` broadcast. ABI encoding is limited to the PerpMatchingEngine calldata builder and guarded transaction request boundary.

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
