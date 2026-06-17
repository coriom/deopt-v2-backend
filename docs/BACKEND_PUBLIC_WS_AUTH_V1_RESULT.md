# BACKEND-PUBLIC-WS-AUTH-V1 — Result

**Date:** 2026-06-17
**Milestone:** Wire wallet-native EIP-191 authentication on the
public WebSocket API (`GET /ws`). Backend-only follow-on to
`BACKEND-PUBLIC-WS-API-V1`.

## Summary

- `auth.verify` is now real. The server reconstructs the canonical
  challenge string from the stored `PendingChallenge` fields,
  recovers the EIP-191 signer with the existing
  `crate::signing::recover_personal_signer` utility, and binds the
  session to the recovered lower-cased address on success.
- Nonces are **single-use**: `auth.verify` consumes the pending
  challenge before signature recovery so a replay returns
  `AUTH_CHALLENGE_NOT_FOUND`. Failed verifies also consume the nonce.
- `auth.challenge` now returns the **full canonical message** the
  wallet must sign — no out-of-band assumption about field order or
  whitespace.
- Private `account.*` channels are unlocked behind authentication.
  Four channels (`positions`, `portfolio`, `balances`, `history`)
  read through the existing HTTP handlers; the five with no source
  data yet (`orders`, `fills`, `intent_status`, `settlements`,
  `liquidations`) return honest empty arrays.
- Five new auth error codes added (`INVALID_ADDRESS`, `AUTH_EXPIRED`,
  `AUTH_INVALID_SIGNATURE`, `AUTH_ADDRESS_MISMATCH`,
  `AUTH_CHALLENGE_NOT_FOUND`).
- 17 new tokio unit tests pin every code path (happy path,
  every failure mode, replay, address-override defence). Backend
  `cargo test --lib api::public_ws` is **46/46 PASS**;
  `cargo test --lib api::` is **339/339 PASS**;
  `cargo test --lib config::env::tests` is **114/114 PASS**.

## Signature scheme used

**EIP-191 personal-sign.**

The server constructs a deterministic ASCII message (see "Challenge
format" below), the wallet performs `personal_sign(message)`, and the
server recovers the signer via the existing
`crate::signing::recover_personal_signer` (k256 + EIP-191 prefix).

Why this scheme:

- It's the simplest, most browser/wallet-compatible flow (MetaMask,
  WalletConnect-bridged wallets, hardware wallets all surface a
  human-readable prompt with the exact bytes being signed).
- It reuses an existing, audited-in-our-test-suite recovery path
  (`recover_personal_signer` is the same helper that already protects
  the MM Gateway's `auth.verify` flow).
- It needs zero new dependencies, zero new env vars, and zero
  server-side signing.

EIP-712 typed-data was rejected for V1 because (a) the EIP-712
helpers in this repo are tied to a `SignedOrder` shape and would have
forced inventing a second typed-data layout; (b) the wallet
acceptance footprint is narrower than personal-sign; (c) the message
content here is informational only — there is no on-chain action
attached.

## Challenge format

Canonical bytes signed by the wallet (lower-case address,
decimal milliseconds, no trailing newline):

```text
DeOpt Public WebSocket Authentication

Address: <lower-cased 0x-address>
Chain ID: <u64 decimal>
Nonce: nonce_<uuid>
Issued At: <i64 ms>
Expires At: <i64 ms>
Domain: deopt-v2-public-ws
```

`auth.challenge` returns the envelope **and** the exact `message`
string so the wallet can sign byte-identical input. Storage holds
`{nonce, address, issued_at_ms, expires_at_ms, chain_id, domain}` so
the verifier reconstructs the same bytes deterministically.

## Verification behavior

`auth.verify` (`AuthVerifyParams { address, signature }`):

1. Parse the supplied address; malformed → `INVALID_ADDRESS`.
2. Prune expired challenges (no expired nonce can authenticate).
3. **Remove** the pending challenge for the supplied address —
   single-use enforcement is independent of outcome. Missing →
   `AUTH_CHALLENGE_NOT_FOUND`.
4. If `expires_at_ms <= now`, return `AUTH_EXPIRED`.
5. Rebuild the canonical message from the stored challenge.
6. Recover the EIP-191 signer via `recover_personal_signer`. Decode
   or recovery failure → `AUTH_INVALID_SIGNATURE`.
7. Compare recovered signer to the supplied address (case-folded).
   Mismatch → `AUTH_ADDRESS_MISMATCH`.
8. Success: bind the session to the lower-cased recovered address,
   return `{ authenticated: true, address, expires_at_ms, chain_id }`.

`session.get` mirrors the bound address from step 8.

## Session binding behavior

- The session field `account: Option<AccountId>` flips from `None` to
  `Some(lowercase_address)` only on a successful `auth.verify`.
- `is_authenticated()` is a pure function of `account.is_some()`.
- The bound address is the canonical lower-cased 0x-prefixed string.
- Re-authenticating is allowed: a fresh `auth.challenge` +
  `auth.verify` overwrites the bound address. The previous address's
  in-flight subscriptions are NOT silently rebound — they remain on
  the previous address (which the client can `unsubscribe` /
  re-`subscribe` if they want to rebind).
- Disconnecting cleans the session; the next connection is fresh.

## Private channels unlocked

| Channel | Source | Snapshot shape |
| --- | --- | --- |
| `account.positions` | `crate::api::trading::account_positions` | `PositionsData` |
| `account.portfolio` | `crate::api::trading::account_portfolio` | `PortfolioData` |
| `account.balances` | `crate::api::trading::account_balances` | `BalancesData` |
| `account.history` | `crate::api::trading::account_history_v2` with `tab=trades, range=last_month, page=1, page_size=100` | `HistoryV2Data` |

All four reuse the same in-process handler that backs the HTTP route,
so the REST and WS surfaces cannot drift.

## Private channels still returning empty arrays

| Channel | Wire shape today | Reason |
| --- | --- | --- |
| `account.orders` | `{ address, source: "empty", orders: [] }` | No public per-account order endpoint at REST yet. |
| `account.fills` | `{ address, source: "empty", fills: [] }` | Fill-by-account is reachable via `account.history` (trades tab); a delta channel is the better follow-on shape than a snapshot. |
| `account.intent_status` | `{ address, source: "empty", intents: [] }` | Indexer-side join is not yet exposed publicly. |
| `account.settlements` | `{ address, source: "empty", settlements: [] }` | Settlement events not yet indexed. |
| `account.liquidations` | `{ address, source: "empty", liquidations: [] }` | Perps not live; no liquidation events. |

None of these channels fabricate data; the `source: "empty"` field
makes the "no data yet" state explicit on the wire.

## Address-bound subscribe defence

`SubscribeParams` now accepts an optional `address` field. When the
channel `requires_auth()` AND the supplied `address` does not match
the bound session address, the dispatcher returns
`AUTH_ADDRESS_MISMATCH` rather than silently using the bound address.
This protects against a frontend accidentally querying for the wrong
wallet's data.

## Tests added

17 new tokio tests under `src/api/public_ws/`:

- **protocol (1):**
  `canonical_challenge_message_is_deterministic_and_includes_every_field`.
- **dispatcher (11):**
  - `auth_verify_without_challenge_returns_challenge_not_found`
  - `auth_verify_rejects_malformed_address`
  - `auth_verify_rejects_malformed_signature`
  - `auth_verify_rejects_signature_from_a_different_wallet`
  - `auth_verify_rejects_expired_challenge`
  - `auth_verify_happy_path_binds_session_to_recovered_address`
  - `auth_verify_replay_after_success_returns_challenge_not_found`
  - `session_get_after_auth_shows_authenticated_address`
  - `private_subscribe_after_auth_succeeds_for_bound_address`
  - `private_subscribe_rejects_address_override_for_another_wallet`
  - `private_subscribe_before_auth_returns_auth_required` (self-contained restate of the V1 guarantee)
- **snapshots (5):**
  - `account_positions_snapshot_is_serialisable_for_an_address`
  - `account_portfolio_snapshot_is_serialisable_for_an_address`
  - `account_balances_snapshot_is_serialisable_for_an_address`
  - `account_history_snapshot_is_serialisable_for_an_address`
  - `deferred_account_channels_return_honest_empty_arrays`
  - `public_channels_are_rejected_by_address_scoped_path`

The old `auth_verify_returns_auth_unavailable_in_v1` test was
replaced by `auth_verify_without_challenge_returns_challenge_not_found`,
which now asserts the correct V1.5 behaviour.

Positive-path tests produce real signatures inside the test using
`k256::ecdsa::SigningKey::sign_prehash_recoverable` + the existing
`personal_sign_digest`. No mocks, no weakening of the production
verification path.

## Validations run

| Check | Result |
| --- | --- |
| `cargo fmt --check` | **PASS** |
| `cargo check` | **PASS** warning-free |
| `cargo test --lib api::public_ws` | **PASS** — 46/46 |
| `cargo test --lib api::` | **PASS** — 339/339 |
| `cargo test --lib config::env::tests` | **PASS** — 114/114 |
| `git diff --check` (backend) | **PASS** |
| Sensitive-pattern scan on `src/api/public_ws/` (`DATABASE_URL=` / `PRIVATE_KEY=` / `alchemy.com/v2/` / `infura.io/v3/` / `mainnet.base.org` / `Bearer ≥ 16 chars`) | **0 hits** |
| `AUTH_UNAVAILABLE` no longer emitted from `auth.verify` | **YES** (code retained in the enum for future opt-in deferral, but no code path emits it) |
| `MM_GATEWAY_TRANSPORT` and `src/mm/transport/webtransport.rs` untouched | **YES** |
| Backend `.env` mtime preserved | YES (`2026-06-08 16:55:05`) |
| Private dir mode preserved | YES (`700`) |

## Skipped validations

| Validation | Reason |
| --- | --- |
| Full `cargo test` | Scoped to the changed modules + `api::` + `config::env::` (already 453/453). Nothing outside those areas was touched. |
| Live socket round-trip integration test | The dispatcher is a pure function and every code path (including happy-path verify with a real signature) is unit-tested without a real socket. |
| AsyncAPI schema export | Deferred to a follow-on; the channel taxonomy is now stable enough for it, but writing the schema is out of scope for this milestone. |
| Frontend client update | Out of scope; the backend wire shape is unchanged except for the new error codes which the existing frontend `TradingErrorCode` switch already tolerates (unrecognised codes fall through to the generic-error path). |
| Audit / threat-model review | Out of scope per the brief; no security claims beyond "EIP-191 single-use signer recovery" are made. |

## Safety posture confirmation

| Statement | Confirmed |
| --- | --- |
| No secrets read, printed, or written | YES |
| No private keys touched in production code (only deterministic test keys inside `#[cfg(test)]`) | YES |
| No RPC URLs added | YES |
| No `DATABASE_URL` references added | YES |
| No admin bearer tokens added | YES |
| No `.env` files read or modified (mtime preserved) | YES |
| No chain transaction issued | YES |
| No broadcast / send / deploy | YES |
| No mainnet network used or referenced | YES |
| No Solidity touched | YES |
| No `scripts/local-*.sh` edits | YES |
| Backend `private/` dir mode preserved (`700`) | YES |
| MM Gateway WebTransport untouched and still separate from the public surface | YES |
| Public WS code path never reads MM permissions or MM gateway state | YES |
| Public WS code path never invokes the executor or the indexer | YES |
| Public WS code path never logs message bodies, addresses, signatures, or nonces | YES |
| Private channel snapshot generators only run for the bound session address | YES |
| Cross-wallet data leakage explicitly tested for and refused with `AUTH_ADDRESS_MISMATCH` | YES |

## Known limitations

- `account.{orders, fills, intent_status, settlements, liquidations}`
  return honest empty arrays because no public per-account source
  exists yet. Wire shape is reserved; upgrading each to a real source
  is a small follow-on.
- `auth.verify` rebuilds the message from the stored fields; if the
  wallet signs a message with even a single byte of whitespace or
  case difference, the recovered address will not match and the
  server returns `AUTH_ADDRESS_MISMATCH`. The exact byte string is
  returned in the `auth.challenge` response (new `message` field) so
  the frontend should sign that string verbatim.
- The session's bound address never expires automatically — it is
  bound until the connection drops. The challenge's `expires_at_ms`
  controls the time window for `auth.verify`, not the bound session
  itself. A follow-on milestone can add session-level expiry if
  required by deployment policy.
- Re-authenticating to a different address does NOT rebind in-flight
  subscriptions; the client must `unsubscribe` + `subscribe` again.
- No replay buffer for missed push events on reconnect. Out of scope.

## Current next recommendation

In order of value:

1. **Frontend WS client + `/api` page demo** — surface the new wire
   shape (with the canonical `message` field) on `/api` so external
   developers can copy a working browser snippet. Tiny, no protocol
   risk.
2. **`account.orders` real source** — once the public per-account
   order list ships at REST, swap the empty-array snapshot for the
   live handler with the same pattern used by
   `account.{positions,portfolio,balances,history}`. Repeat for the
   other empty channels as their sources land.
3. **Push-on-write deltas** — wire a `tokio::sync::broadcast` channel
   on the underlying executor / indexer write path so the WS server
   can push deltas instead of periodic re-snapshots. Cheaper at scale
   and matches the "serious derivatives venue" comparison.

None should be started without an explicit milestone brief.
