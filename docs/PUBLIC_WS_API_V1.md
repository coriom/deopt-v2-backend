# DeOpt V2 — Public WebSocket API (V1)

Status: testnet beta. Public reads only. No order submission. No
broadcast. No mainnet.

## Endpoint

`GET /ws` on the same Axum listener as the public HTTP API (default
port `8080`). Configurable via `PUBLIC_WS_PATH`. The TLS termination
strategy is delegated to the operator's reverse proxy — the backend
itself speaks plain WebSocket; production deployments are expected to
front it with `wss://` at the proxy layer.

> The public API does not expose WebTransport. WebTransport remains
> reserved for the operator-whitelisted MM Gateway.

> There is no public WebSocket order submission in V1 unless
> explicitly implemented and tested. Signed intent creation remains on
> HTTP (`POST /options/execution-intents`).

## Wire shape — JSON-RPC 2.0 with a `meta` extension

Every server frame carries the same `meta` block the HTTP API surfaces
on `Envelope::meta` (`source`, `chain_id`, `request_id`,
`generated_at_ms`). Frontend code can share an error-handling switch
across REST and WS because error codes are the same
`SCREAMING_SNAKE_CASE` strings the HTTP layer already uses.

### Client → server request

```json
{
  "jsonrpc": "2.0",
  "id": "req_1",
  "method": "subscribe",
  "params": { "channel": "trading.health" }
}
```

### Server → client successful response

```json
{
  "jsonrpc": "2.0",
  "id": "req_1",
  "result": {
    "subscribed": true,
    "subscription_id": "sub_<uuid>",
    "channel": "trading.health"
  },
  "meta": {
    "source": "backend",
    "chain_id": 84532,
    "request_id": "req_<uuid>",
    "generated_at_ms": 1700000000000
  }
}
```

### Server → client error response

```json
{
  "jsonrpc": "2.0",
  "id": "req_1",
  "error": {
    "code": "INVALID_CHANNEL",
    "message": "unknown channel: garbage"
  },
  "meta": {
    "source": "backend",
    "chain_id": 84532,
    "request_id": "req_<uuid>",
    "generated_at_ms": 1700000000000
  }
}
```

### Server → client subscription push

```json
{
  "jsonrpc": "2.0",
  "method": "subscription",
  "params": {
    "subscription_id": "sub_<uuid>",
    "channel": "trading.health",
    "seq": 0,
    "event_id": "evt_<uuid>",
    "source": "backend",
    "chain_id": 84532,
    "generated_at_ms": 1700000000000,
    "data": { /* same shape as the matching HTTP envelope's `data` */ }
  }
}
```

`seq` is monotonic per subscription. The server reserves
`instrument_id`, `address`, `tx_hash` for channel-specific routing; V1
public channels do not populate them.

## Methods

| Method | Direction | V1 behaviour |
| --- | --- | --- |
| `ping` | client → server | Replies `{ pong: true, server_time_ms, chain_id }`. |
| `subscribe` | client → server | Validates channel; AUTH_REQUIRED on `account.*` channels; immediately emits one snapshot push. |
| `unsubscribe` | client → server | Removes subscription by id. |
| `subscriptions` | client → server | Lists active subscriptions for the connection. |
| `session.get` | client → server | Returns connection id, auth state, address (if any), subscriptions. |
| `auth.challenge` | client → server | Issues a nonce + expiry + canonical signing message bound to the supplied address. |
| `auth.verify` | client → server | Recovers the EIP-191 signer from the canonical message and the supplied signature, binds the session to that address, and consumes the nonce (single-use). |
| `subscription` | server → client | Server push event (snapshot or periodic update). |

## Channels

### Public (V1)

| Channel | Source | Snapshot wire shape |
| --- | --- | --- |
| `trading.health` | `/trading/health` HTTP handler | `TradingHealthData` |
| `options.products` | `/options/products` HTTP handler | `ProductsListData` |
| `leaderboard` | `/leaderboard` HTTP handler with default `range=last_month`, `page=1`, `page_size=100` | `LeaderboardData` |

The server runs the same in-process handler that backs the matching
HTTP route, so there is no second implementation that could drift.

### Private (wallet-authenticated)

Subscription to these channels requires a successful `auth.verify`
first. The session is bound to the lower-cased EVM address recovered
from the EIP-191 signature; every snapshot is scoped to that
address. Supplying a different `address` param on `subscribe` returns
`AUTH_ADDRESS_MISMATCH` rather than silently using the bound
address.

| Channel | Source | Notes |
| --- | --- | --- |
| `account.positions` | `account_positions` HTTP handler | LIVE for the bound address. |
| `account.portfolio` | `account_portfolio` HTTP handler | LIVE for the bound address. |
| `account.balances` | `account_balances` HTTP handler | LIVE for the bound address. |
| `account.history` | `account_history_v2` HTTP handler with `tab=trades, range=last_month, page=1, page_size=100` | LIVE for the bound address. |
| `account.orders` | (no source yet) | Returns `{ address, source: "empty", orders: [] }`. |
| `account.fills` | (no source yet) | Returns `{ address, source: "empty", fills: [] }`. |
| `account.intent_status` | (no source yet) | Returns `{ address, source: "empty", intents: [] }`. |
| `account.settlements` | (no source yet) | Returns `{ address, source: "empty", settlements: [] }`. |
| `account.liquidations` | (no source yet) | Returns `{ address, source: "empty", liquidations: [] }`. |

The empty channels are documented as honest empty arrays — no
fabricated rows. As underlying tables / event streams land, each
empty channel is upgraded to a real source without breaking the wire
shape.

### Source availability matrix

| Channel | V1 status | Source |
| --- | --- | --- |
| `trading.health` | LIVE | `trading_health` HTTP handler |
| `options.products` | LIVE | `list_products` HTTP handler |
| `leaderboard` | LIVE | `leaderboard` HTTP handler |
| `account.positions` | LIVE (wallet-bound) | `account_positions` HTTP handler |
| `account.portfolio` | LIVE (wallet-bound) | `account_portfolio` HTTP handler |
| `account.balances` | LIVE (wallet-bound) | `account_balances` HTTP handler |
| `account.history` | LIVE (wallet-bound) | `account_history_v2` HTTP handler |
| `account.orders` / `account.fills` / `account.intent_status` / `account.settlements` / `account.liquidations` | EMPTY (wallet-bound) | honest empty arrays |

There is no faked live market data. No public order book, no public
trade prints, no oracle / mark prices — those channels are
intentionally absent from V1 because no public source exists yet.

## Authentication model

Wallet-native, EIP-191 personal-sign. The server never signs anything;
the wallet signs a deterministic canonical message, and the server
recovers the signer with `k256` + the existing
`crate::signing::recover_personal_signer` utility.

### Flow

1. Client sends `auth.challenge` with the wallet address.
2. Server replies with the challenge envelope **and the full canonical
   message that the wallet must sign**:
   ```json
   {
     "address": "0xabcdef...",
     "nonce": "nonce_<uuid>",
     "domain": "deopt-v2-public-ws",
     "chain_id": 84532,
     "issued_at_ms": 1700000000000,
     "expires_at_ms": 1700000060000,
     "message": "DeOpt Public WebSocket Authentication\n\nAddress: 0xabcdef...\nChain ID: 84532\nNonce: nonce_<uuid>\nIssued At: 1700000000000\nExpires At: 1700000060000\nDomain: deopt-v2-public-ws"
   }
   ```
   Nonces are random per challenge and bound to the supplied address;
   per-session, last challenge wins.
3. The wallet performs EIP-191 personal-sign on the exact `message`
   string above. (`personal_sign(message)` in any browser wallet.)
4. Client sends `auth.verify` with `{ address, signature }`.
5. Server reconstructs the canonical message from the stored
   challenge fields, recovers the signer via EIP-191, and binds the
   session to the recovered (lower-cased) address. The nonce is
   **consumed regardless of outcome** — a fresh `auth.challenge` is
   required to retry.
6. Server reply:
   ```json
   {
     "jsonrpc": "2.0",
     "id": "req_2",
     "result": {
       "authenticated": true,
       "address": "0xabcdef...",
       "expires_at_ms": 1700000060000,
       "chain_id": 84532
     },
     "meta": { ... }
   }
   ```

### Canonical message — exact format

The server constructs the message byte-for-byte from the stored
challenge fields; the wallet must reproduce it before signing.
Mismatched bytes (whitespace, casing, field order) make the signature
recover the wrong address and the server returns
`AUTH_ADDRESS_MISMATCH`.

```text
DeOpt Public WebSocket Authentication

Address: <lower-cased 0x-address>
Chain ID: <u64 decimal>
Nonce: nonce_<uuid>
Issued At: <i64 ms>
Expires At: <i64 ms>
Domain: deopt-v2-public-ws
```

### Auth error codes

| Code | When |
| --- | --- |
| `INVALID_ADDRESS` | `auth.verify` was called with a malformed address. |
| `INVALID_REQUEST` / `INVALID_PARAMS` | JSON shape errors. |
| `AUTH_CHALLENGE_NOT_FOUND` | No active challenge for this address (none issued, expired and pruned, or already consumed by a previous verify). |
| `AUTH_EXPIRED` | Challenge present in the session but already past `expires_at_ms`. |
| `AUTH_INVALID_SIGNATURE` | Signature does not decode or does not recover a valid signer for the canonical message. |
| `AUTH_ADDRESS_MISMATCH` | Signer recovered, but is not the address the client claimed (or the `subscribe.address` field disagrees with the bound session address). |
| `AUTH_REQUIRED` | Subscription attempted on an `account.*` channel without first authenticating. |
| `AUTH_FAILED` / `AUTH_UNAVAILABLE` | Reserved; not emitted in V1. |

### Nonce / replay rules

* Nonces are single-use: `auth.verify` removes the pending challenge
  before signature recovery so a replay of the same `(address,
  signature)` pair after success returns `AUTH_CHALLENGE_NOT_FOUND`.
* Failed verifies also consume the nonce — the client must re-call
  `auth.challenge` to retry.
* Expired challenges are pruned on every incoming message; an expired
  nonce never authenticates anyone.

No private key, no admin Bearer, no shared secret is required at any
point. Signing always happens client-side.

### Private channel access model

Once the session is bound to an address, the bound address is
authoritative for every `account.*` subscribe — even if the client
re-sends a different `address` param. The dispatcher refuses the
mismatch with `AUTH_ADDRESS_MISMATCH` rather than silently using the
bound address (this protects against frontends accidentally querying
for the wrong wallet's data). Snapshots for `account.*` channels
always read using the bound address and never accept a runtime
override.

## Rate, frame, and connection limits

All knobs read from env at boot time with conservative defaults safe
for local + public testnet beta. Missing env vars never break startup.

| Knob | Default | Env var |
| --- | --- | --- |
| Enabled | `true` | `PUBLIC_WS_ENABLED` |
| Path | `/ws` | `PUBLIC_WS_PATH` |
| Max connections | `1_000` | `PUBLIC_WS_MAX_CONNECTIONS` |
| Max subscriptions per connection | `64` | `PUBLIC_WS_MAX_SUBSCRIPTIONS_PER_CONNECTION` |
| Max frame size | `64 KiB` | `PUBLIC_WS_MAX_FRAME_BYTES` |
| Client messages/sec | `30` | `PUBLIC_WS_CLIENT_RATE_LIMIT_PER_SEC` |
| Heartbeat interval | `15_000` ms | `PUBLIC_WS_HEARTBEAT_INTERVAL_MS` |
| Snapshot interval | `5_000` ms | `PUBLIC_WS_SNAPSHOT_INTERVAL_MS` |
| Challenge TTL | `60_000` ms | `PUBLIC_WS_CHALLENGE_TTL_MS` |

When a client breaches a limit the server emits an unsolicited error
frame (`FRAME_TOO_LARGE`, `RATE_LIMITED`, …) and keeps the connection
open so well-behaved clients can recover. Pathological cases close
the connection cleanly.

## Error codes (stable surface)

`INVALID_REQUEST` · `INVALID_PARAMS` · `UNKNOWN_METHOD` ·
`INVALID_CHANNEL` · `SUBSCRIPTION_NOT_FOUND` · `ALREADY_SUBSCRIBED` ·
`TOO_MANY_SUBSCRIPTIONS` · `AUTH_REQUIRED` · `AUTH_UNAVAILABLE` ·
`AUTH_FAILED` · `RATE_LIMITED` · `FRAME_TOO_LARGE` ·
`SOURCE_UNAVAILABLE` · `NOT_IMPLEMENTED` · `INTERNAL_ERROR`.

`SOURCE_UNAVAILABLE` mirrors the HTTP envelope's
`TradingErrorCode::SourceUnavailable` so an existing frontend switch
catches both transports.

## Security posture

* No order submission, no broadcast, no mainnet, no admin endpoints.
* No private key, no admin Bearer, no shared secret on the public WS.
* Wallets sign client-side; signatures are not stored server-side.
* Snapshots reuse the existing HTTP handlers — there is no second
  implementation that could leak data outside the public REST
  surface.
* Account-scoped channels are guarded by `AUTH_REQUIRED` until
  signature verification ships.
* The MM Gateway over WebTransport is a SEPARATE listener (default
  port `8443`) with a SEPARATE config and SEPARATE permissions; this
  milestone does not modify or expose it.

## Future path

* **Wallet-signed `auth.verify`**: wire the existing
  `crate::signing::eip712` digest + recovery utilities to bind the WS
  session to the verified address. Unlocks the nine `account.*`
  channels.
* **Real market-data channels**: as the indexer / executor pipeline
  fills the relevant tables (options orderbook snapshots, fills,
  oracle ticks), add a fan-out producer per channel and a tokio
  `broadcast` bus so connections share work.
* **Signed order intent over WebSocket**: post-auth, allow clients to
  push EIP-712 signed intents through the WS instead of HTTP, keeping
  the executor path unchanged.
* **Replayable event log + horizontal scaling**: lift the per-channel
  `seq` to a Redis Streams / NATS JetStream / Postgres event table so
  reconnecting clients can resume from `seq` and so multiple backend
  replicas share fanout.
* **AsyncAPI schema**: this milestone keeps the schema in this
  Markdown file. Once the channel taxonomy stabilises (after
  `auth.verify` lands), promote it to `docs/asyncapi/public-ws-api-v1.asyncapi.yaml`.

## Example client (browser)

```javascript
const ws = new WebSocket("wss://example.invalid/ws");
ws.onopen = () => {
  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "req_1",
    method: "subscribe",
    params: { channel: "trading.health" },
  }));
};
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.method === "subscription") {
    console.log("push", msg.params.channel, msg.params.seq, msg.params.data);
  } else if (msg.result) {
    console.log("ack", msg.id, msg.result);
  } else if (msg.error) {
    console.warn("err", msg.id, msg.error.code, msg.error.message);
  }
};
```
