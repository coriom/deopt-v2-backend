# DeOpt v2 Rust Trading Backend

Phase 1 Rust backend for DeOpt v2 trading infrastructure. This service provides an in-memory perp orderbook, deterministic matching, a thin HTTP API, RFQ/MM scaffolds, an execution-intent queue, a dry-run PerpMatchingEngine calldata builder boundary, manual RPC simulation for calldata-ready intents, an explicitly gated real broadcast path, and an opt-in Indexer V1 for PerpMatchingEngine events.

Smart contracts remain the final source of truth. By default this backend does not submit transactions, sign transaction payloads, load private keys, or claim final settlement. Optional simulation uses `eth_call` only and never broadcasts. Real transaction signing and `eth_sendRawTransaction` are available only when `EXECUTOR_REAL_BROADCAST_ENABLED=true` and all required signer, fee, RPC, persistence, signature, and simulation gates pass.

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
EXECUTOR_DRY_RUN=true
EXECUTOR_POLL_INTERVAL_MS=1000
EXECUTOR_MAX_BATCH_SIZE=10
SIMULATION_ENABLED=false
SIMULATION_REQUIRE_PERSISTENCE=true
RPC_URL=
EXECUTOR_FROM_ADDRESS=0x0000000000000000000000000000000000000000
PERP_MATCHING_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000
PERP_ENGINE_ADDRESS=0x0000000000000000000000000000000000000000
PERP_NONCE_SYNC_ENABLED=false
PERP_NONCE_SYNC_REQUIRE_RPC=true
PERP_NONCE_SYNC_STRICT=true
INDEXER_ENABLED=false
INDEXER_START_BLOCK=0
INDEXER_POLL_INTERVAL_MS=3000
INDEXER_MAX_BLOCK_RANGE=500
INDEXER_REQUIRE_PERSISTENCE=true
CONFIRMATION_ENABLED=false
CONFIRMATION_REQUIRE_PERSISTENCE=true
CONFIRMATION_REQUIRED_BLOCKS=2
CONFIRMATION_MAX_BATCH_SIZE=50
CONFIRMATION_REQUIRE_RECONCILIATION=true
RFQ_ENABLED=false
RFQ_REQUIRE_PERSISTENCE=true
RFQ_DEFAULT_TTL_MS=5000
RFQ_MAX_TTL_MS=30000
RFQ_MIN_QUOTE_TTL_MS=500
RFQ_MAX_QUOTE_TTL_MS=10000
RFQ_MAX_QUOTES_PER_RFQ=50
RFQ_QUOTE_SIGNATURE_MODE=disabled
RFQ_EIP712_NAME=DeOptV2RFQ
RFQ_EIP712_VERSION=1
RFQ_EIP712_CHAIN_ID=84532
RFQ_EIP712_VERIFYING_CONTRACT=0x0000000000000000000000000000000000000000
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
SIGNATURE_VERIFICATION_MODE=disabled
PERSISTENCE_ENABLED=false
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
EIP712_NAME=DeOptV2
EIP712_VERSION=1
EIP712_CHAIN_ID=84532
EIP712_VERIFYING_CONTRACT=0x0000000000000000000000000000000000000000
MM_GATEWAY_ENABLED=false
MM_GATEWAY_TRANSPORT=webtransport
MM_GATEWAY_HOST=127.0.0.1
MM_GATEWAY_PORT=8443
MM_GATEWAY_CERT_PATH=
MM_GATEWAY_KEY_PATH=
MM_GATEWAY_MAX_SESSIONS=100
MM_GATEWAY_MAX_IN_FLIGHT_PER_SESSION=128
MM_GATEWAY_RATE_LIMIT_PER_SEC=100
MM_GATEWAY_HEARTBEAT_TIMEOUT_MS=15000
MM_GATEWAY_MAX_ORDERS_PER_BULK=50
MM_GATEWAY_MAX_CANCELS_PER_BULK=100
MM_GATEWAY_MAX_OPEN_ORDERS_PER_ACCOUNT=500
MM_GATEWAY_CANCEL_ON_DISCONNECT=true
MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false
```

`EXECUTION_ENABLED=false` is intentional for this phase.
`EXECUTOR_DRY_RUN=false` is rejected because real on-chain execution is not implemented.
`PERSISTENCE_ENABLED=false` keeps the default local in-memory behavior and does not require Postgres.
`EXECUTOR_REAL_BROADCAST_ENABLED=false` is the safe default.
`EXECUTOR_REAL_BROADCAST_ENABLED=true` requires `PERSISTENCE_ENABLED=true`, `EXECUTOR_PRIVATE_KEY`, `RPC_URL`, `EXECUTOR_MAX_FEE_PER_GAS_WEI`, `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI`, a nonzero `EXECUTOR_CHAIN_ID`, and a nonzero `EXECUTOR_MAX_GAS_LIMIT`.
`SIMULATION_ENABLED=true` requires `RPC_URL`; when `SIMULATION_REQUIRE_PERSISTENCE=true`, it also requires `PERSISTENCE_ENABLED=true`.
`PERP_NONCE_SYNC_ENABLED=false` keeps existing local nonce behavior unchanged. When set to `true`, `PERP_NONCE_SYNC_REQUIRE_RPC=true` requires `RPC_URL` and a nonzero `PERP_MATCHING_ENGINE_ADDRESS` at startup. `PERP_NONCE_SYNC_STRICT=true` rejects an order when its nonce does not equal `PerpMatchingEngine.nonces(order.account)`.
`INDEXER_ENABLED=true` requires `RPC_URL`; when `INDEXER_REQUIRE_PERSISTENCE=true`, it also requires `PERSISTENCE_ENABLED=true`.
`CONFIRMATION_ENABLED=true` requires `RPC_URL`; when `CONFIRMATION_REQUIRE_PERSISTENCE=true`, it also requires `PERSISTENCE_ENABLED=true`. Confirmation is disabled by default, never broadcasts, and rejects startup if `CONFIRMATION_REQUIRE_RECONCILIATION=false`.
`RFQ_ENABLED=false` is the safe default. When `RFQ_ENABLED=true` and `RFQ_REQUIRE_PERSISTENCE=true`, startup requires `PERSISTENCE_ENABLED=true`. Test and development code can run RFQ in memory with persistence disabled, but production-like RFQ acceptance should use Postgres so RFQ, quote, and execution-intent updates are committed together.
`RFQ_QUOTE_SIGNATURE_MODE=disabled` preserves the unsigned RFQ V1B flow. `strict` requires each RFQ quote to include a valid EIP-712 `RFQQuote` signature whose recovered signer equals `mm_account`.
`OPTIONS_ENABLED=false` and `OPTION_RFQ_ENABLED=false` are the safe defaults. When either options persistence gate is true and its feature is enabled, startup requires `PERSISTENCE_ENABLED=true`. Test and development code can run option series, off-chain option orders, matching, fills, and option RFQs in memory with persistence disabled.
`MM_GATEWAY_ENABLED=false` is the safe default. When `true`, V1C starts a separate WebTransport UDP listener and requires `MM_GATEWAY_CERT_PATH` and `MM_GATEWAY_KEY_PATH`. It can submit and cancel off-chain perp orders through the live in-memory orderbook, handle perp RFQ and option RFQ messages when those features are enabled, but it does not auto-broadcast, sign, simulate, index, reconcile, or confirm execution intents.

## Options V1D / Option RFQ V1B

Options V1D adds an HTTP/core option RFQ service on top of the option series registry and off-chain option orderbook. Option RFQ V1B connects that service to the Market Maker Gateway: HTTP-created option RFQs are pushed to active MM sessions, MMs can submit `option_rfq_quote` over WebTransport, and quote acceptance sends best-effort MM notifications. Option RFQ quote acceptance creates a separate off-chain option RFQ fill record only. It does not create option execution intents or transactions, does not broadcast, does not execute on chain, does not add signed option RFQ quotes, and does not change the existing perp/RFQ/MM execution lifecycle.

HTTP endpoints:

```text
POST /options/series
GET /options/series
GET /options/series/:option_series_id
POST /options/series/:option_series_id/disable
POST /options/orders
GET /options/orders
GET /options/orders/:order_id
GET /options/orders/:order_id/fills
POST /options/orders/:order_id/cancel
GET /options/fills
GET /options/fills/:fill_id
GET /options/orderbooks/:option_series_id
POST /options/rfqs
GET /options/rfqs
GET /options/rfqs/:option_rfq_id
POST /options/rfqs/:option_rfq_id/quotes
GET /options/rfqs/:option_rfq_id/quotes
POST /options/rfqs/:option_rfq_id/accept/:quote_id
POST /options/rfqs/:option_rfq_id/cancel
```

Manual series creation accepts `underlying`, `base_asset`, `quote_asset`, `settlement_asset`, `expiry`, `strike_1e8`, `is_call`, optional `contract_size_1e8`, and optional on-chain ids. `strike_1e8` and `contract_size_1e8` are integer fixed-point strings. Calls use `is_call=true`; puts use `is_call=false`. Expiry is a Unix timestamp in seconds and must be in the future for manual series.

`option_series_id` is deterministic: `keccak256(lowercase-trimmed underlying/base/quote/settlement, expiry, strike_1e8, is_call, contract_size_1e8)` exposed as `0x` plus 32-byte hex. Duplicate manual creation returns the existing series deterministically.

`GET /options/series` supports simple filters: `underlying`, `expiry`, `is_call`, and `status`. Series statuses are `active`, `expired`, and `disabled`; source values are `manual` and `onchain`. `POST /options/series/:option_series_id/disable` marks a known series disabled.

`POST /options/orders` accepts an active `option_series_id`, account, `buy` or `sell` side, fixed-point string `price_1e8` and `size_1e8`, `time_in_force`, optional `client_order_id`, optional nonce/deadline, and optional signature. V1C supports `gtc` orders only; `ioc` and `fok` are rejected cleanly. Signatures are shape-validated when provided, but option order signer recovery is not implemented in V1C.

Matching is off-chain only and stays within a single `option_series_id`. A buy crosses resting sells when `buy.price_1e8 >= sell.price_1e8`; a sell crosses resting buys when `sell.price_1e8 <= buy.price_1e8`. Matching uses integer price-time priority: incoming buys take the lowest ask first, incoming sells take the highest bid first, and same-price levels use `created_at_ms` then `order_id`. Fill price is the resting maker order price. `POST /options/orders` returns the submitted order plus a `fills` array.

Option fills store `fill_id`, series id, buy/sell order ids, buyer/seller, maker/taker order ids, taker side, fixed-point price/size, and timestamp. `GET /options/fills` supports `option_series_id`, `account`, and `order_id` filters. `GET /options/orders/:order_id/fills` lists fills touching one order.

`GET /options/orders` supports `option_series_id`, `account`, `status`, and `side` filters. Live statuses are `open` and `partially_filled`; fully filled orders do not appear in the orderbook. `POST /options/orders/:order_id/cancel` cancels open or partially filled orders and returns a clear error for filled or already-cancelled orders. Duplicate `(account, client_order_id)` submissions are rejected while an existing order with that client id is live.

`GET /options/orderbooks/:option_series_id` validates that the series exists and returns aggregated live bid/ask levels from remaining option order sizes. Bids are sorted descending by integer price, asks ascending, and same-price levels sum live `remaining_size_1e8`.

`POST /options/rfqs` accepts `taker`, `option_series_id`, taker-perspective `side`, fixed-point string `size_1e8`, optional `limit_price_1e8`, and optional `ttl_ms`. RFQ statuses are `open`, `expired`, `accepted`, `cancelled`, and `failed`.

`POST /options/rfqs/:option_rfq_id/quotes` accepts `mm_account`, fixed-point premium `price_1e8`, `size_1e8`, optional `session_id`, optional `client_quote_id`, and optional `quote_ttl_ms`. Quote statuses are `active`, `expired`, `accepted`, `rejected`, and `cancelled`. Duplicate `(option_rfq_id, mm_account, client_quote_id)` values are rejected when `client_quote_id` is present.

Market Maker Gateway option RFQ messages use the same JSON envelope and length-prefixed WebTransport frames as perp RFQ:

```text
server -> MM: option_rfq_request
MM -> server: option_rfq_quote
server -> MM: option_rfq_quote_result
server -> MM: option_rfq_quote_accepted
server -> MM: option_rfq_quote_rejected
```

`option_rfq_request` includes option RFQ id, taker, option series id, taker side, size, optional limit price, and expiry. `option_rfq_quote` includes option RFQ id, MM account, premium price, size, optional client quote id, and quote TTL. Gateway-submitted option quotes are persisted through the same option RFQ store as HTTP/dev quotes, including `session_id` when available. V1B broadcasts option RFQ requests to all active MM sessions; zero connected sessions do not block RFQ creation. These V1 behaviors are hardcoded and no additional config flags are required.

Accepting one option RFQ quote is single-winner: the RFQ becomes `accepted`, the winning quote becomes `accepted`, active competing quotes become `rejected`, and a row is created in `option_rfq_fills`. For taker `buy`, `buyer=taker`, `seller=mm_account`, and `taker_side=buy`. For taker `sell`, `buyer=mm_account`, `seller=taker`, and `taker_side=sell`. Buy RFQs require quote price `<= limit_price_1e8` when a limit is provided; sell RFQs require quote price `>= limit_price_1e8`. If the winning quote has an active MM `session_id`, acceptance sends `option_rfq_quote_accepted`; active competing session quotes receive best-effort `option_rfq_quote_rejected` with reason `competing quote accepted`. Notification failure is surfaced as `mm_notification_warning` when available and never reverts acceptance.

Option order matching and option RFQ acceptance are both off-chain only. Signed option RFQ quotes, option execution intents, on-chain option execution, Greeks, IV surfaces, on-chain registry sync, settlement, and exercise remain deferred.

## RFQ V1C

RFQ V1C supports taker RFQ creation, WebTransport `rfq_request` push to connected Market Maker Gateway sessions, WebTransport `rfq_quote` submission by market makers, HTTP/dev quote submission, quote listing, taker quote acceptance, RFQ cancellation, and creation of a normal `execution_intent` for the accepted quote. Quotes may now carry market-maker signatures. It does not support production MM wallet challenge auth, signed taker RFQ requests, options or multi-leg RFQs, market-maker ranking, auto-signing, simulation, or broadcast.

HTTP endpoints:

```text
POST /rfqs
GET /rfqs
GET /rfqs/:rfq_id
POST /rfqs/:rfq_id/quote-signing-payload
POST /rfqs/:rfq_id/quotes
GET /rfqs/:rfq_id/quotes
POST /rfqs/:rfq_id/accept/:quote_id
POST /rfqs/:rfq_id/cancel
```

`POST /rfqs/:rfq_id/quote-signing-payload` returns the EIP-712 domain, `RFQQuote` fields, message, and digest for a proposed quote. The message uses:

```text
RFQQuote(bytes32 rfqId,address mmAccount,uint256 marketId,bool takerIsBuyer,uint128 price1e8,uint128 size1e8,uint256 quoteNonce,uint256 expiry)
```

`rfqId` is `keccak256(bytes(rfq_uuid_string))`. `takerIsBuyer` is true when RFQ `side=buy`. `quoteNonce` is supplied by the market maker. `expiry` is encoded in seconds and currently binds the quote signature to the RFQ expiry; the submitted quote can still expire earlier according to `quote_ttl_ms`.

`POST /rfqs/:rfq_id/quotes` and WebTransport `rfq_quote` accept optional `quote_nonce` and `signature` fields. In disabled mode, unsigned quotes remain valid and are stored with `signature_status=not_required`. In strict mode, `quote_nonce` and `signature` are required, the backend recomputes the digest, recovers the signer, and rejects the quote unless the recovered address equals `mm_account`. Verified quotes are stored with `signature_status=verified`, `signature`, `quote_digest`, `quote_nonce`, and `recovered_signer`.

For local signing against a payload response:

```sh
MM_PRIVATE_KEY=0x... cargo run --bin sign_rfq_quote -- \
  --payload /tmp/rfq_quote_payload.json
```

The CLI prints the signer address and signature only; it does not print the private key.

Market Maker Gateway RFQ messages use the existing JSON envelope and length-prefixed WebTransport frames:

```text
server -> MM: rfq_request
MM -> server: rfq_quote
server -> MM: rfq_quote_result
server -> MM: rfq_quote_accepted
server -> MM: rfq_quote_rejected
```

`rfq_quote` stores quotes in the same RFQ repository as the HTTP/dev endpoint, including `session_id`, `client_quote_id`, and signature metadata when supplied. `rfq_quote_result` returns the stored `quote_id`, `rfq_id`, active status, and quote expiry. RFQ creation still succeeds when no MM sessions are connected.

RFQ statuses are `open`, `expired`, `accepted`, `cancelled`, and `failed`. Quote statuses are `active`, `expired`, `accepted`, `rejected`, and `cancelled`. Expiry is enforced on quote submission and acceptance. Accepting one quote deterministically marks the RFQ `accepted`, marks the winning quote `accepted`, and marks other active quotes for that RFQ `rejected`.

RFQ `side` is from the taker perspective. `buy` means the taker buys perp exposure, so the execution intent has `buyer=taker`, `seller=mm_account`, and `buyer_is_maker=false`. `sell` means the taker sells perp exposure, so the execution intent has `buyer=mm_account`, `seller=taker`, and `buyer_is_maker=true`.

Accepted RFQs create pending execution intents only. The resulting intent then uses the existing flow:

```text
GET /execution-intents/:intent_id/signing-payload
POST /execution-intents/:intent_id/signatures
POST /executor/simulate/:intent_id
POST /executor/broadcast/:intent_id
```

Broadcast remains disabled unless the existing explicit broadcast gates are enabled. RFQ acceptance never fabricates signatures, transaction hashes, confirmations, or finality. If the accepted quote or rejected competing quotes are associated with active MM sessions, the backend sends best-effort WebTransport notifications; notification failure does not revert acceptance. Production MM auth, signed taker RFQ requests, market-maker ranking, multi-leg RFQ, and expiry schedulers remain deferred.

## Market Maker Gateway V1A

The Market Maker Gateway is the planned low-latency path for market-maker order flow, quote replacement, heartbeat/session tracking, and later market-data delivery. WebTransport is the strategic transport because it gives HTTP/3 over QUIC, reliable streams, optional datagrams, and connection-oriented session semantics. V1A intentionally keeps the business logic transport-agnostic.

All client messages use the same JSON envelope:

```json
{
  "type": "heartbeat",
  "request_id": "hb-1",
  "payload": {}
}
```

Success responses use `type="<message>_result"`, the same `request_id`, `ok=true`, and a typed `payload`. Errors use `type="error"`, `ok=false`, and stable error codes such as `BAD_REQUEST`, `AUTH_REQUIRED`, `RATE_LIMITED`, `TOO_MANY_ORDERS`, `TOO_MANY_CANCELS`, `ORDER_REJECTED`, `CANCEL_REJECTED`, and `QUOTE_REPLACE_FAILED`.

V1A defines protocol models for `auth`, `heartbeat`, `submit_order`, `bulk_submit`, `cancel_order`, `bulk_cancel`, `cancel_all`, `quote_replace`, and `get_session`. `heartbeat` updates the session timestamp and `get_session` returns a public serializable snapshot containing session id, connection id, optional account, auth mode, heartbeat time, cancel-on-disconnect flag, open client order ids, rate-window counters, and in-flight count.

Gateway auth is shape-only in V1A. The supported default is:

```text
MM_GATEWAY_AUTH_MODE=disabled
MM_GATEWAY_REQUIRE_AUTH=false
```

In disabled mode, development sessions can process messages without wallet challenge auth when account-bearing payloads include the account. Wallet challenge auth is deferred.

Bulk submit and bulk cancel are partial-result capable and return per-item `ok` / `error` entries. `quote_replace` accepts optional `bid` and `ask` legs plus `cancel_previous`; V1A returns deterministic planned counts and tracks planned client order ids in the session. It does not mutate the live orderbook and does not fabricate backend order ids or matched execution intents.

Cancel-on-disconnect is planning-only in V1A. The planner returns the session's currently open client order ids when `MM_GATEWAY_CANCEL_ON_DISCONNECT=true`; it does not touch execution intents, submitted transactions, confirmed transactions, or chain state.

Pure rate-limit helpers enforce max messages per second, max in-flight requests per session, max orders per bulk, max cancels per bulk, and max open client order ids per account/session:

```text
MM_GATEWAY_MAX_IN_FLIGHT_PER_SESSION=128
MM_GATEWAY_RATE_LIMIT_PER_SEC=100
MM_GATEWAY_MAX_ORDERS_PER_BULK=50
MM_GATEWAY_MAX_CANCELS_PER_BULK=100
MM_GATEWAY_MAX_OPEN_ORDERS_PER_ACCOUNT=500
```

The gateway V1A never enables real broadcast by default, never signs transaction payloads, never bypasses existing order validation, and normal tests do not require WebTransport, certificates, UDP, RPC, Postgres, private keys, or Base Sepolia.

## Market Maker Gateway V1B

V1B adds the concrete WebTransport adapter using `wtransport`. The adapter is isolated under `src/mm/transport/webtransport.rs`; protocol, session, service, and rate-limit modules remain transport-neutral.

Startup behavior:

- `MM_GATEWAY_ENABLED=false`: no WebTransport server is started.
- `MM_GATEWAY_ENABLED=true`: the process loads `MM_GATEWAY_CERT_PATH` and `MM_GATEWAY_KEY_PATH`, binds UDP `MM_GATEWAY_HOST:MM_GATEWAY_PORT`, and runs beside the existing HTTP Axum server.
- Missing cert/key paths, unreadable cert/key files, invalid bind addresses, or unsupported transports fail startup with clear config errors.
- The HTTP API keeps its existing TCP `HOST:PORT` behavior.

Critical order messages use reliable bidirectional streams, not datagrams. V1B framing is:

```text
u32 big-endian payload length
JSON payload bytes
```

The maximum frame size is `1048576` bytes. Oversized frames are rejected. Invalid JSON produces an error response when the request id can be recovered. V1B handles one request per bidirectional stream and writes one framed JSON response. Keeping streams open for multiple requests is deferred.

Local certificate options:

```sh
mkdir -p /tmp/deopt-mm-gateway
mkcert -cert-file /tmp/deopt-mm-gateway/cert.pem \
       -key-file /tmp/deopt-mm-gateway/key.pem \
       localhost 127.0.0.1 ::1
```

or:

```sh
mkdir -p /tmp/deopt-mm-gateway
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout /tmp/deopt-mm-gateway/key.pem \
  -out /tmp/deopt-mm-gateway/cert.pem \
  -days 1 \
  -subj "/CN=localhost"
```

Then configure:

```text
MM_GATEWAY_ENABLED=true
MM_GATEWAY_TRANSPORT=webtransport
MM_GATEWAY_HOST=127.0.0.1
MM_GATEWAY_PORT=8443
MM_GATEWAY_CERT_PATH=/tmp/deopt-mm-gateway/cert.pem
MM_GATEWAY_KEY_PATH=/tmp/deopt-mm-gateway/key.pem
```

Runtime notes:

- WebTransport requires UDP reachability on `MM_GATEWAY_PORT` and client support for HTTP/3 over QUIC.
- Browser clients using self-signed certificates may need local trust setup or WebTransport certificate hash options.
- Datagrams are deferred for market data and are not used for critical order messages.
- The gateway still never auto-broadcasts, never signs transaction payloads, and never changes execution finality.

## Market Maker Gateway V1C

V1C connects the transport-neutral gateway service to the same shared order/cancel service used by HTTP `POST /orders` and `DELETE /orders/:order_id`. Gateway order intake preserves deadline checks, signature shape or strict EIP-712 recovery, known-market validation, optional strict Perp nonce sync, backend local nonce reservation, matching, execution-intent creation, and optional persistence event writes.

Supported live order operations are `submit_order`, `bulk_submit`, `cancel_order`, `bulk_cancel`, `cancel_all`, `quote_replace`, and cancel-on-disconnect. Bulk operations are partial-result capable. Cancels are restricted to resting off-chain orders owned by the session account, addressed by `order_id` or `client_order_id`; they do not mutate submitted, broadcast, confirmed, or otherwise non-resting execution intents.

`quote_replace` uses clear non-atomic semantics: if `cancel_previous=true`, previously tracked quote client order ids for the session are cancelled first for the requested account and market, then the new bid and ask legs are submitted independently as GTC orders. The response reports cancelled, submitted, rejected, per-leg results, backend order ids, and matched execution intent ids.

On WebTransport disconnect, if cancel-on-disconnect is enabled and the session has an account, the adapter asks `MmGatewayService` to cancel the session's tracked resting client order ids and logs the live cancellation count. The WebTransport adapter remains transport-only; orderbook business logic stays in the shared service and gateway service layers.

## PerpMatchingEngine Calldata

The execution module can ABI encode `PerpMatchingEngine.executeTrade((bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)` with `alloy-sol-types` when given an explicit `PerpTradePayload` and explicit buyer/seller trade signatures.

The Solidity `PerpTrade.intentId` is derived from the backend UUID string as `keccak256(bytes(execution_intents.intent_id))` and exposed as `0x` plus 64 lowercase hex chars. This mapping is deterministic, non-random, and reused by the signing payload, calldata builder, and indexed-event reconciliation.

The order signatures accepted by `POST /orders` are not PerpTrade signatures. The Solidity `PerpMatchingEngine` verifies signatures over the final matched `PerpTrade`, so the builder never reuses order signatures as trade signatures and never fabricates buyer or seller signatures. If signatures are missing, the builder produces a non-executable preview with empty calldata and `missing_signatures=true`.

After a match, clients can fetch the exact EIP-712 trade payload:

```sh
curl http://127.0.0.1:8080/execution-intents/<intent_id>/signing-payload
```

The response includes the `DeOptV2-PerpMatchingEngine` domain, `PerpTrade` type fields, message fields, and digest. The trade message uses `intentId` first, then the matched buyer/seller, market, size, execution price, buyer maker flag, buyer/seller order nonces, and the minimum original order deadline. If an old or direct in-memory intent lacks nonce/deadline metadata, the endpoint returns a clear error instead of inventing values.

Clients submit matched-trade signatures separately:

```sh
curl -X POST http://127.0.0.1:8080/execution-intents/<intent_id>/signatures \
  -H 'content-type: application/json' \
  -d '{
    "buyer_sig": "0x...",
    "seller_sig": "0x..."
  }'
```

Signatures are accepted only as `0x` plus 65-byte hex strings. They are stored in memory by default and in `execution_intent_signatures` when persistence is enabled. `calldata_ready=true` only when both buyer and seller trade signatures are present and the corresponding intent has complete PerpTrade metadata.

Broadcast remains disabled by default. The prepared call always has `is_broadcastable=false` and `value=0`; no signing, private key retention, transaction submission, or confirmation tracking exists in the default path.

## Development PerpTrade Signing

The `sign_perp_trade` binary is a local development helper for Base Sepolia and throwaway test wallets only. Never use it with production keys, never commit `.env` files or private keys, and do not treat it as a production custody model. The backend server does not automatically sign user trades.

Fetch the backend-provided EIP-712 payload:

```sh
export INTENT_ID=<intent_id>
curl http://127.0.0.1:8080/execution-intents/$INTENT_ID/signing-payload \
  > /tmp/perp_trade_payload.json
```

Sign as the buyer. `BUYER_PRIVATE_KEY` takes precedence over `SIGNER_PRIVATE_KEY`:

```sh
BUYER_PRIVATE_KEY=0x... \
cargo run --bin sign_perp_trade -- \
  --payload /tmp/perp_trade_payload.json \
  --role buyer
```

Sign as the seller. `SELLER_PRIVATE_KEY` takes precedence over `SIGNER_PRIVATE_KEY`:

```sh
SELLER_PRIVATE_KEY=0x... \
cargo run --bin sign_perp_trade -- \
  --payload /tmp/perp_trade_payload.json \
  --role seller
```

The CLI signs the `digest` returned by the backend payload and outputs JSON only by default:

```json
{
  "role": "buyer",
  "signer_address": "0x...",
  "signature": "0x..."
}
```

By default it rejects a buyer key that does not derive to `message.buyer` and a seller key that does not derive to `message.seller`. `--allow-address-mismatch` exists only for explicit debugging. `--verbose` keeps stdout as JSON and adds the digest, domain, and message intent id.

Submit both signatures:

```sh
curl -X POST http://127.0.0.1:8080/execution-intents/$INTENT_ID/signatures \
  -H "Content-Type: application/json" \
  -d '{"buyer_sig":"0x...","seller_sig":"0x..."}'
```

Then simulate:

```sh
curl -X POST http://127.0.0.1:8080/executor/simulate/$INTENT_ID
```

## Perp On-chain Nonce Sync V1

Perp nonce sync is opt-in:

```text
PERP_NONCE_SYNC_ENABLED=false
PERP_NONCE_SYNC_REQUIRE_RPC=true
PERP_NONCE_SYNC_STRICT=true
RPC_URL=https://...
PERP_MATCHING_ENGINE_ADDRESS=0x774d96E5739bffadEE91508b4D3D74F5BE29F165
```

Local `used_nonces` protects the backend from accepting the same signed order twice, including across restarts when persistence is enabled. It is not enough for on-chain execution because `PerpMatchingEngine` is canonical and requires `nonces[account] == tradeNonce`. A stale backend-local nonce can therefore pass local replay checks but still revert when the final `PerpTrade` is submitted.

When nonce sync and strict mode are enabled, `POST /orders` reads `PerpMatchingEngine.nonces(order.account)` with `eth_call` and rejects the order before local nonce reservation if `order.nonce` differs from the on-chain value:

```json
{
  "error": "perp nonce mismatch: expected on-chain nonce 1, got 0"
}
```

Existing local nonce protection remains in place after the on-chain check. With `PERP_NONCE_SYNC_ENABLED=false`, order intake behaves as before and normal tests do not require RPC.

Operators can query the on-chain nonce directly:

```sh
curl http://127.0.0.1:8080/accounts/0x0000000000000000000000000000000000000001/perp-nonce
```

Example response:

```json
{
  "account": "0x0000000000000000000000000000000000000001",
  "perp_matching_engine": "0x774d96e5739bffadee91508b4d3d74f5be29f165",
  "nonce": 1,
  "source": "onchain"
}
```

This prepares the market-maker gateway path by making the backend reject stale taker or maker order nonces before it creates matches and execution intents.

## RPC Simulation

Manual simulation is opt-in:

```text
SIMULATION_ENABLED=true
SIMULATION_REQUIRE_PERSISTENCE=true
RPC_URL=https://...
EXECUTOR_FROM_ADDRESS=0x0000000000000000000000000000000000000000
PERP_MATCHING_ENGINE_ADDRESS=0x...
PERSISTENCE_ENABLED=true
```

`POST /executor/simulate/<intent_id>` loads one execution intent, requires both stored PerpTrade signatures, rebuilds the real `executeTrade` calldata, and performs an `eth_call` to `PERP_MATCHING_ENGINE_ADDRESS` with `value=0`. On success, the intent is marked `simulation_ok`; on revert or RPC failure, it is marked `simulation_failed` with the error text and any decoded revert diagnostics. These statuses only describe the result of the call simulation at the queried block. They do not mean submitted, confirmed, final, or executed.

Simulation failure responses include diagnostic fields when the RPC returns revert data:

```json
{
  "simulation_status": "simulation_failed",
  "error": "simulation failed: execution reverted",
  "revert_data": "0x...",
  "revert_selector": "0x...",
  "decoded_error": {
    "kind": "custom_error",
    "name": "InvalidSignature",
    "selector": "0x...",
    "args": []
  },
  "submitted": false,
  "confirmed": false
}
```

The decoder supports Solidity `Error(string)`, `Panic(uint256)`, unknown custom-error selectors, and a table of common protocol errors such as `InvalidSignature`, `NotAuthorized`, `InsufficientMargin`, `MarketCloseOnly`, `OracleStale`, `OraclePriceUnavailable`, `InvalidPrice`, and `InvalidSize`. If an RPC provider returns only a message, `decoded_error.kind` is `missing_revert_data` and the raw message is preserved. These diagnostics are persisted in `execution_simulations` as `revert_data`, `revert_selector`, and `decoded_error`.

The endpoint returns `submitted=false` and `confirmed=false` for every response. Simulation does not call `eth_sendRawTransaction` and `GET /executor/status` reports `broadcastEnabled=false` until real broadcast is explicitly enabled.

## Real Broadcast V1

Broadcast V1 is disabled by default and only submits when explicitly enabled:

```text
EXECUTOR_REAL_BROADCAST_ENABLED=false
EXECUTOR_PRIVATE_KEY=
EXECUTOR_CHAIN_ID=84532
EXECUTOR_MAX_GAS_LIMIT=1000000
EXECUTOR_MAX_FEE_PER_GAS_WEI=
EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI=
EXECUTOR_REQUIRE_SIMULATION_OK=true
```

`POST /executor/broadcast/<intent_id>` returns a clear disabled response while `EXECUTOR_REAL_BROADCAST_ENABLED=false`. It does not sign, call `eth_sendRawTransaction`, fabricate a tx hash, or mark the intent submitted. Transaction request construction requires a complete matched trade, both PerpTrade signatures, non-empty `executeTrade` calldata, a configured `PERP_MATCHING_ENGINE_ADDRESS`, and `simulation_ok` when `EXECUTOR_REQUIRE_SIMULATION_OK=true`.

When `EXECUTOR_REAL_BROADCAST_ENABLED=true`, startup validates the private key shape, RPC URL, static EIP-1559 fee fields, chain id, and gas limit. Broadcast fetches `eth_chainId`, rejects mismatches before signing, fetches the executor pending nonce with `eth_getTransactionCount`, signs a type `0x02` EIP-1559 transaction in-process, and submits only with `eth_sendRawTransaction`. The API records `submitted` only after the RPC returns a real tx hash, then marks the execution intent `submitted`. It never returns `confirmed=true` and never marks an intent confirmed.

Private keys are held only in the execution config secret wrapper and signer object; their `Debug` output is redacted. The API never returns raw transactions or private keys.

When persistence is enabled, transaction records can be read with:

```sh
curl http://127.0.0.1:8080/executor/transactions
curl http://127.0.0.1:8080/executor/transactions/<intent_id>
```

The database stores transaction attempts with statuses `prepared`, `rejected`, `submitted`, and `failed`. `submitted` means only that `eth_sendRawTransaction` returned a syntactically valid transaction hash. It does not prove inclusion, execution success, backend ownership, finality, or absence of reorgs. If the RPC send succeeds but persistence fails immediately afterward, the chain may still have received the transaction; this V1 does not provide atomic RPC-plus-database semantics.

## Indexer V1

Indexer V1 is opt-in and read-only:

```text
INDEXER_ENABLED=true
INDEXER_START_BLOCK=0
INDEXER_POLL_INTERVAL_MS=3000
INDEXER_MAX_BLOCK_RANGE=500
INDEXER_REQUIRE_PERSISTENCE=true
RPC_URL=https://...
PERP_MATCHING_ENGINE_ADDRESS=0x...
PERSISTENCE_ENABLED=true
```

It reads `eth_getLogs` for `PerpMatchingEngine.TradeExecuted`, decodes the event, stores rows in `indexed_perp_trades`, and advances the `perp_matching_engine` cursor only after persistence succeeds. The Solidity event now emits indexed `intentId`, which the backend stores as `indexed_perp_trades.onchain_intent_id`. Manual control and reads are exposed through:

```sh
curl http://127.0.0.1:8080/indexer/status
curl -X POST http://127.0.0.1:8080/indexer/tick
curl http://127.0.0.1:8080/indexed/perp-trades
```

Indexed events do not mark execution intents submitted or confirmed. Direct reconciliation can compare `keccak256(bytes(execution_intents.intent_id))` with `indexed_perp_trades.onchain_intent_id`; economic match keys are only a fallback for historical data without an intent id. V1 stores `block_hash` when the RPC provides it, but does not implement deep reorg rollback.

## Reconciliation V1

Reconciliation V1 is opt-in, persistence-backed, and read-only with respect to execution intent lifecycle:

```text
RECONCILIATION_ENABLED=false
RECONCILIATION_REQUIRE_PERSISTENCE=true
RECONCILIATION_MAX_BATCH_SIZE=100
```

It links indexed `TradeExecuted` events to backend intents by direct bytes32 identity:

```text
execution_intents.onchain_intent_id == indexed_perp_trades.onchain_intent_id
```

An exact unique match writes an `execution_reconciliations` row with `status=matched`. Missing backend intents are counted as unmatched without inventing ownership. Multiple backend intents or duplicate indexed events for the same on-chain intent id are treated as ambiguous. Reconciliation rows include the indexed event id, tx hash, block number, and log index, but they do not prove this backend submitted the transaction.

Manual control and reads:

```sh
curl http://127.0.0.1:8080/reconciliation/status
curl -X POST http://127.0.0.1:8080/reconciliation/tick
curl http://127.0.0.1:8080/reconciliations
curl http://127.0.0.1:8080/reconciliation/intents/<intent_id>
```

Reconciliation does not mark intents submitted or confirmed. A matched reconciliation is one required input to confirmation, not finality by itself.

## Execution Confirmation / Finality V1

Confirmation V1 is opt-in and persistence-backed:

```text
CONFIRMATION_ENABLED=false
CONFIRMATION_REQUIRE_PERSISTENCE=true
CONFIRMATION_REQUIRED_BLOCKS=2
CONFIRMATION_MAX_BATCH_SIZE=50
CONFIRMATION_REQUIRE_RECONCILIATION=true
RPC_URL=https://...
PERSISTENCE_ENABLED=true
```

The lifecycle is:

```text
broadcast submitted tx hash -> receipt success -> enough blocks -> indexed TradeExecuted -> matched reconciliation -> confirmed
```

Receipt status alone is insufficient. A successful receipt proves only that the transaction executed successfully in a block returned by the RPC provider. The backend also requires a stored `TradeExecuted` event for the same `tx_hash` and `onchain_intent_id`, plus a `matched` reconciliation row for the same intent, on-chain intent id, and transaction hash. The required block count guards against shallow reorgs by requiring `current_block >= receipt_block + CONFIRMATION_REQUIRED_BLOCKS`.

Manual control and reads:

```sh
curl http://127.0.0.1:8080/executor/confirmations/status
curl -X POST http://127.0.0.1:8080/executor/confirm/<intent_id>
curl -X POST http://127.0.0.1:8080/executor/confirmations/tick
curl http://127.0.0.1:8080/executor/confirmations/<intent_id>
```

`POST /executor/confirmations/tick` scans submitted, unconfirmed transaction rows up to `CONFIRMATION_MAX_BATCH_SIZE` and applies the same checks idempotently. It updates `execution_transactions.confirmation_status` and only sets `execution_intents.status=confirmed` when all confirmation conditions pass. Pending/failure statuses include `missing_receipt`, `receipt_failed`, `not_finalized`, `missing_indexed_event`, `missing_reconciliation`, `failed`, and `confirmed`.

## Persistence

PostgreSQL persistence is opt-in:

```text
PERSISTENCE_ENABLED=true
DATABASE_URL=postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
```

When enabled, the service connects to Postgres at startup and runs migrations from `migrations/`. Migrations create `used_nonces`, `orders`, `trades`, `execution_intents`, `execution_intent_signatures`, `execution_simulations`, `engine_events`, `indexer_cursors`, `indexed_perp_trades`, `execution_reconciliations`, `execution_transactions`, `option_series`, `option_orders`, `option_fills`, `option_rfqs`, `option_rfq_quotes`, and `option_rfq_fills`. RFQ signed quotes add nullable `signature`, `quote_digest`, `quote_nonce`, `signature_status`, and `recovered_signer` fields to `rfq_quotes`. Confirmation adds nullable `confirmed_at_ms`, `confirmed_block_number`, `confirmation_status`, and `confirmation_error` fields to `execution_transactions`.

One local setup option:

```sh
createdb deopt_v2_backend
cargo run
```

If your local Postgres uses a different user, password, host, or database name, set `DATABASE_URL` accordingly. Persistence is required before real broadcast usage so transaction records and submitted intent status survive restarts.

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
curl http://127.0.0.1:8080/accounts/0x0000000000000000000000000000000000000001/perp-nonce
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

- Default mode is in-memory; restarting clears orders and execution intents unless `PERSISTENCE_ENABLED=true`.
- Perp limit orders only.
- Public API financial quantities are string-encoded fixed-point integers.
- `POST /orders` uses a signed-order payload with nonce, deadline, and signature fields.
- Perp on-chain nonce sync is disabled by default; strict sync requires RPC and compares order nonces with `PerpMatchingEngine.nonces(account)` before local nonce reservation.
- `SIGNATURE_VERIFICATION_MODE=disabled` validates nonce, deadline, and signature shape while skipping cryptographic recovery.
- `SIGNATURE_VERIFICATION_MODE=strict` verifies the EIP-712 order digest and recovered secp256k1 signer against `account`.
- FOK is rejected cleanly.
- RFQ supports HTTP/dev flow plus basic MM gateway push, quote intake, and optional signed quote verification; multi-leg RFQ, signed taker RFQ requests, MM ranking, production auth, signed option RFQ quotes, and market data datagrams remain deferred.
- Options V1D/V1B supports manual option series registration, deterministic `option_series_id`, off-chain GTC option orders, price-time matching, fill recording/listing, cancellation, listing/filtering, aggregated option orderbook reads, HTTP/core option RFQs with off-chain RFQ fills, and MM Gateway option RFQ request/quote/accept notification messages; option execution intents, signed option RFQ quotes, and on-chain option execution remain deferred.
- Execution intents are provisional off-chain records, not settlement.
- Indexed `TradeExecuted` events store `onchain_intent_id` for direct reconciliation only; they do not confirm backend intents.
- Reconciliation rows link indexed events to intents, but still do not prove transaction ownership, finality, or reorg safety.
- Confirmation requires receipt success, enough blocks, a matching indexed event, and matched reconciliation before marking an intent confirmed.
- Real broadcast is disabled by default; when enabled it submits only with a real signed raw transaction and never returns fake tx hashes.
- Indexer V1 stores block hashes when available but does not implement deep reorg rollback.
- PerpMatchingEngine calldata can be encoded only from complete matched trade payloads and explicit buyer/seller PerpTrade signatures.
- Optional blockchain RPC includes manual `eth_call` simulation, opt-in indexing, and explicitly gated `eth_sendRawTransaction` broadcast. No production auth, WebSocket API, or options matching.

## Deferred Execution Work

- Add background confirmation polling, transaction ownership proofs beyond persisted broadcast rows, gas estimation, fee discovery, retries, nonce reservation, and deep reorg rollback.
