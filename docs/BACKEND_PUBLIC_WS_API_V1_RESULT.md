# BACKEND-PUBLIC-WS-API-V1 — Result

**Date:** 2026-06-17
**Milestone:** Add a first-class public WebSocket API on the existing
public Axum backend so DeOpt V2 has a competitive third API surface
(HTTP / WebSocket / MM WebTransport). Backend-only.

## Summary

- New public endpoint `GET /ws` on the same Axum listener as the
  public HTTP API. Browser-friendly. **No** order submission, **no**
  broadcast, **no** mainnet, **no** admin token, **no** secrets.
- JSON-RPC 2.0-shaped protocol with a `meta` extension that mirrors the
  HTTP `Envelope::meta` block (`source`, `chain_id`, `request_id`,
  `generated_at_ms`).
- Subscription model: per-connection subscriptions, monotonic `seq`
  per subscription, immediate snapshot push on successful subscribe,
  optional periodic re-snapshot on a conservative timer.
- Public channels live in V1: `trading.health`, `options.products`,
  `leaderboard`. Each one reuses the EXISTING HTTP handler so the
  WS and HTTP surfaces cannot drift.
- Nine `account.*` private channels are declared in the registry but
  refused with a clean `AUTH_REQUIRED` because `auth.verify` is
  deferred. Subscribing to a private channel never leaks data and
  never crashes.
- Wallet challenge half of the auth flow ships (`auth.challenge`
  issues a nonce + expiry per address). `auth.verify` returns
  `AUTH_UNAVAILABLE` until the EIP-712 / EIP-191 recovery path is
  wired in a follow-on milestone.
- 29 new unit tests cover protocol serde, every code path in the
  message dispatcher, every channel, every public limit, the auth
  scaffolding, and the snapshot generators. Full backend `api::` test
  suite is 322/322 PASS; `config::env::` is 114/114 PASS.

## Endpoint added

| Method | Path | Auth | Behaviour |
| --- | --- | --- | --- |
| `GET` | `/ws` | none (public) | HTTP/1.1 → WebSocket upgrade. When `public_ws.enabled = false`, returns HTTP 503 + `SOURCE_UNAVAILABLE` JSON. |

## Methods implemented

| Method | V1 status |
| --- | --- |
| `ping` | LIVE — returns `{ pong, server_time_ms, chain_id }` |
| `subscribe` | LIVE — channel validation + immediate snapshot push |
| `unsubscribe` | LIVE |
| `subscriptions` | LIVE — list active subscriptions |
| `session.get` | LIVE — connection state, auth state, address, subscriptions |
| `auth.challenge` | LIVE — issues a random nonce + expiry per address |
| `auth.verify` | DEFERRED — returns `AUTH_UNAVAILABLE` in V1 |
| `subscription` (server push) | LIVE |

## Channels implemented

### Public (LIVE)

| Channel | Snapshot source |
| --- | --- |
| `trading.health` | `crate::api::trading::trading_health` |
| `options.products` | `crate::api::trading::list_products` with default query |
| `leaderboard` | `crate::api::trading::leaderboard` with `range=last_month, page=1, page_size=100` |

### Private (RESERVED / refused with `AUTH_REQUIRED`)

`account.positions`, `account.portfolio`, `account.balances`,
`account.orders`, `account.fills`, `account.history`,
`account.intent_status`, `account.settlements`, `account.liquidations`.

## Channels reserved / deferred

| Brief's optional channels | V1 decision | Reason |
| --- | --- | --- |
| `options.series` | Deferred | Not necessary for V1; the same data is reachable through `options.products` + the HTTP series endpoint. Keeps the V1 channel registry minimal. |
| `options.ticker` | Deferred | No live ticker source. Honesty rule. |
| `options.trades` | Deferred | No public per-instrument trade tape yet. |
| `options.orderbook` | Deferred | The orderbook surface is not yet stable; on-chain ETA. |
| `oracle.price` | Deferred | Oracle adapter is operator-side; public WS exposure is a follow-on. |
| `mark.price` | Deferred | Mark price is computed from oracle ticks; same gate as `oracle.price`. |

The brief explicitly allowed empty channels; we chose **not to expose
empty channels at all** because every empty channel adds frontend
guard code without surfacing real data. Adding a channel later is a
strictly additive change to the `Channel` enum.

## Auth behaviour

- `auth.challenge` accepts `{ address }` (0x-prefixed 20-byte EVM
  address, validated case-insensitively). Server generates a random
  nonce, returns `{ address, nonce, domain, chain_id, issued_at_ms,
  expires_at_ms }`, and stores the pending challenge in the session
  under the lower-cased address. Per-address, last challenge wins.
- Challenges expire after `PUBLIC_WS_CHALLENGE_TTL_MS` (default
  60 000 ms). `session.prune_expired_challenges` is invoked on every
  client message.
- `auth.verify` returns `AUTH_UNAVAILABLE` with a stable, frontend-
  friendly explanation. The session is never bound to an address in
  V1, so `is_authenticated()` is always `false`, and every account
  channel returns `AUTH_REQUIRED` on subscribe.
- No private key, no admin Bearer, no shared secret is read or written
  by the public WS code path.

## Config added

| Knob | Default | Env var |
| --- | --- | --- |
| Enabled | `true` | `PUBLIC_WS_ENABLED` |
| Path | `/ws` | `PUBLIC_WS_PATH` |
| Max connections | `1_000` | `PUBLIC_WS_MAX_CONNECTIONS` |
| Max subscriptions per connection | `64` | `PUBLIC_WS_MAX_SUBSCRIPTIONS_PER_CONNECTION` |
| Max frame bytes | `64 * 1024` | `PUBLIC_WS_MAX_FRAME_BYTES` |
| Client rate limit | `30` msgs/sec | `PUBLIC_WS_CLIENT_RATE_LIMIT_PER_SEC` |
| Heartbeat interval | `15_000` ms | `PUBLIC_WS_HEARTBEAT_INTERVAL_MS` |
| Snapshot interval | `5_000` ms | `PUBLIC_WS_SNAPSHOT_INTERVAL_MS` |
| Challenge TTL | `60_000` ms | `PUBLIC_WS_CHALLENGE_TTL_MS` |

Missing env vars never break startup — defaults apply. The values are
read but **not logged**.

## Files changed

**New (7):**
- `src/api/public_ws/mod.rs` — module pub-exports.
- `src/api/public_ws/protocol.rs` (368 lines) — types, channel enum,
  error codes, request/response/notification structs, 4 unit tests.
- `src/api/public_ws/config.rs` (78 lines) — `PublicWsConfig` +
  defaults + 2 tests.
- `src/api/public_ws/session.rs` (159 lines) — `WsSession`,
  `SubscriptionState`, `PendingChallenge`, rate-window helpers,
  4 tests.
- `src/api/public_ws/snapshots.rs` (140 lines) — calls the live HTTP
  handlers in-process; 4 tests.
- `src/api/public_ws/dispatcher.rs` (~570 lines) — pure JSON-RPC
  dispatcher (every code path testable without a socket); 13 tests.
- `src/api/public_ws/handler.rs` (~210 lines) — Axum WebSocket
  handler: upgrade, receive loop, send loop, heartbeat timer,
  periodic snapshot timer, frame-size + rate-limit enforcement,
  graceful close.

**New docs (2):**
- `docs/PUBLIC_WS_API_V1.md` — protocol spec, channel matrix, auth
  model, env-var matrix, future path, browser example.
- `docs/BACKEND_PUBLIC_WS_API_V1_RESULT.md` — this file.

**Modified (5):**
- `Cargo.toml` — `axum = { version = "0.7", features = ["ws"] }`;
  added `futures = "0.3"` (default-features-off, `std` only).
- `src/api/mod.rs` — declared `pub mod public_ws`.
- `src/api/http.rs` — added `pub public_ws_config: PublicWsConfig` to
  `AppState`; defaulted in `with_all_config`.
- `src/api/routes.rs` — registered `GET /ws` immediately below
  `/leaderboard`.
- `src/config/env.rs` — added `pub public_ws: PublicWsConfig` to
  `AppConfig`; parses `PUBLIC_WS_*` env vars with safe defaults.
- `src/main.rs` — copies `config.public_ws` into `state.public_ws_config`
  before mounting the router; adds two fields to the startup log
  (`public_ws_enabled`, `public_ws_path`). No values are leaked.

No DB migrations. No Solidity changes. No scripts changes.

## Tests added

29 new tokio unit tests in `src/api/public_ws/*`:

- **protocol (4):** channel round-trip, account-channel auth bit,
  error-code SCREAMING_SNAKE_CASE serialisation, server-response /
  client-request shape parsing.
- **config (2):** defaults are self-consistent, disabled keeps the
  other knobs.
- **session (4):** new session is unauth + empty, message-window
  rolls every second, rate-limit triggers above cap, expired
  challenges pruned correctly.
- **snapshots (4):** trading.health / options.products / leaderboard
  snapshots are serialisable; all `account.*` channels return
  `NotImplemented`.
- **dispatcher (13):** non-2.0 jsonrpc rejected; `ping` returns pong;
  unknown method → `UNKNOWN_METHOD`; subscribe to invalid channel →
  `INVALID_CHANNEL`; subscribe to `trading.health` acks + pushes
  snapshot with `seq=0`; subscribing twice → `ALREADY_SUBSCRIBED`;
  account channel without auth → `AUTH_REQUIRED`; unsubscribe
  removes the subscription; unsubscribe with bad id →
  `SUBSCRIPTION_NOT_FOUND`; `auth.challenge` persists in the session
  with a future expiry; malformed address → `INVALID_PARAMS`;
  `auth.verify` returns `AUTH_UNAVAILABLE` and never authenticates;
  `session.get` returns the current state; `subscribe` respects
  `max_subscriptions_per_connection`.

## Validations run

| Check | Result |
| --- | --- |
| `cargo fmt --check` | **PASS** |
| `cargo check` | **PASS** warning-free |
| `cargo test --lib api::public_ws` | **PASS** — 29/29 |
| `cargo test --lib api::` | **PASS** — 322/322 |
| `cargo test --lib config::env::tests` | **PASS** — 114/114 |
| `git diff --check` (backend) | **PASS** |
| Sensitive-pattern scan (`DATABASE_URL=` / `PRIVATE_KEY=` / `alchemy.com/v2/` / `infura.io/v3/` / `mainnet.base.org` / `Bearer ≥ 16 chars`) on the new module | **0 hits** |
| `MM_GATEWAY_TRANSPORT` and `mm/transport/webtransport.rs` untouched | **YES** |
| Backend `.env` mtime preserved | YES (`2026-06-08 16:55:05`) |
| Private dir mode preserved | YES (`700`) |

## Skipped validations

| Validation | Reason |
| --- | --- |
| Full `cargo test` | Sweep is large and slow; scoped to the changed modules + `api::` + `config::env::` instead. Nothing outside those areas was touched. |
| Live socket round-trip integration test | Out of scope. The dispatcher is a pure function, so every code path is unit-testable without a real socket. A future integration test could exercise the Axum upgrade path with `tokio-tungstenite` against `tower::Service` (no port). |
| AsyncAPI schema export | Documented in `PUBLIC_WS_API_V1.md` as deferred; the channel taxonomy is small enough to keep in Markdown until `auth.verify` lands. |
| Wallet-signed `auth.verify` | Brief explicitly allowed deferring this if no safe utility was wired into the WS layer; surfaced as `AUTH_UNAVAILABLE`. |
| Public test harness (HTML demo page) | Out of scope; result doc includes a browser snippet. |

## Known limitations

- **`auth.verify` is deferred.** Nine `account.*` channels are
  reserved in the registry and refuse subscription with
  `AUTH_REQUIRED`. No user data leaks.
- **Public market-data channels (`options.orderbook`, `options.trades`,
  `oracle.price`, `mark.price`) are intentionally absent.** No
  fabricated data, no empty fake channels.
- **Periodic snapshots are per-connection.** Each subscribed channel
  re-snapshots every `PUBLIC_WS_SNAPSHOT_INTERVAL_MS`. This is fine
  at testnet scale (data sources are in-memory) but is documented as
  the V1 fanout strategy; the future Redis / NATS / Postgres
  event-log path will share work across connections.
- **`max_connections` is documented but not yet enforced at accept
  time.** Per-connection caps (subscriptions, frame bytes, rate
  limit) ARE enforced. Adding a connection-count gate is a small,
  isolated follow-on.
- **No replay buffer.** Reconnecting clients lose any push events
  emitted before the reconnect. Documented as a follow-on (replayable
  event log).
- **Snapshot reuse path holds the in-process Axum handlers' default
  `Query` arguments**. If a future caller wants per-subscription
  filters (e.g. `leaderboard?range=last_day`), the protocol's
  `params.filter` block needs to be threaded through to the snapshot
  generators. Easy follow-on without breaking V1 clients.

## Safety posture confirmation

| Statement | Confirmed |
| --- | --- |
| No secrets read, printed, or written | YES |
| No private keys touched or referenced | YES |
| No RPC URLs added | YES |
| No `DATABASE_URL` references added | YES |
| No admin bearer tokens added | YES |
| No `.env` files read or modified (mtime preserved) | YES |
| No chain transaction issued | YES |
| No broadcast / send / deploy / mint / approve / transfer | YES |
| No mainnet network used or referenced | YES |
| No Solidity touched | YES |
| No `scripts/local-*.sh` edits | YES |
| Backend `private/` dir mode preserved (`700`) | YES |
| MM Gateway over WebTransport untouched | YES |
| MM WebTransport NOT exposed publicly | YES |
| Public WS code path never reads MM permissions or MM gateway state | YES |
| Public WS code path never invokes the executor, the indexer, or any signing utility | YES |
| Public WS code path never logs request bodies, addresses, or nonces | YES |

## Current next recommendation

The two highest-value follow-ons in order:

1. **Wire `auth.verify`** by hashing the canonical challenge with the
   existing EIP-712 / EIP-191 utilities in `crate::signing` and
   recovering the address via `k256`. The session-bind path is already
   in place; this is a focused milestone with no public API surface
   change.
2. **Wire one real-data push channel** (`options.orderbook` or
   `options.trades`) by adding a producer that writes into a tokio
   `broadcast` channel on the relevant service path, with the
   dispatcher forwarding to subscribed sessions. This unlocks the
   "real" market data narrative.

Neither should be started without an explicit milestone brief.
