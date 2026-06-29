# BACKEND-MARKETS-OPTIONS-ONLY-FILTER-V1 — result

**Status: CLOSED.**

The public `/markets` REST endpoint no longer surfaces engine-internal
perp markets while perps are non-live. Response shape is preserved
(JSON array of `Market` records); the filter excludes any record with
`kind == "perp"` so the default `EngineState::with_default_markets()`
emits an empty array instead of `[ETH-PERP, BTC-PERP]`.

Backend test suite: **1035 lib tests + 11 integration suites — all
green**, with 4 new tests pinning the new contract.

---

## Old vs new `/markets` behaviour

| | before | after |
|---|---|---|
| Default backend state (perps-only `EngineState`) | `[{"marketId":1,"symbol":"ETH-PERP","kind":"perp"},{"marketId":2,"symbol":"BTC-PERP","kind":"perp"}]` | `[]` |
| No markets seeded at all | `[]` | `[]` (unchanged) |
| Mixed (perps + options) | full mixed list including perps | options-only — perps filtered |
| Status code | `200 OK` | `200 OK` (unchanged) |
| Response shape | JSON array of `Market` | JSON array of `Market` (unchanged) |
| `/options/products` | unchanged | unchanged |
| Perp mutation routes (`POST /orders`, `/rfqs/...`) | `503 SERVICE_UNAVAILABLE` with `PerpsNotLive` | unchanged |

## Why perps are filtered

Perps are intentionally non-live in this Options-only public testnet
beta:

* `submit_order`, `cancel_order`, `create_rfq`, `submit_quote`,
  `accept_quote`, `cancel_rfq` all return
  `BackendError::PerpsNotLive` (mapped to HTTP 503) — see
  `src/api/routes.rs:3580-3999`.
* The frontend `/perps` workspace carries a visible
  `perps-not-live-banner` and the trade-form submit button is
  hard-disabled with "Perps not live"
  (`TESTNET-SELF-SERVE-ONBOARDING-V1`).

Listing perps as "markets" via the public `/markets` REST surface
would contradict that posture — an external API consumer would
reasonably interpret a `/markets` entry as a tradable instrument.
The filter restores end-to-end consistency between the route-boundary
fail-closed posture (memory `feedback_perps_fail_closed.md`) and the
public read surface.

## What `/markets` is now

`/markets` is the **live, tradable market catalogue**. While perps
are non-live, this surface emits Options-only (or empty) responses.
The single filter rule:

```rust
engine.markets()
    .iter()
    .filter(|m| m.kind != "perp")
    .collect()
```

Choice of `kind != "perp"` over `kind == "option"` is deliberate: any
future non-perp `kind` (e.g. spot, RFQ-only synthetic) would pass
through without another schema change. When
`ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1` ships and perp mutations
re-open, this filter should be revisited — likely by gating on a
config flag rather than the hardcoded `kind` string.

## Files changed (backend, single file)

| Kind | File |
|---|---|
| modified | `src/api/routes.rs` — `async fn markets(State)` adds the perp filter; explanatory comment block referencing the route-boundary fail-closed posture + the future re-enable milestone. 4 new tests added to the `mod tests` block (markets default state, no markets at all, mixed perp/option filter, perp mutation route unchanged). |
| new | `docs/BACKEND_MARKETS_OPTIONS_ONLY_FILTER_V1_RESULT.md` |

**No schema change. No new types. No `Cargo.toml` change. Frontend
untouched (the user-facing `/markets` page already uses
`/options/products`).**

## Tests

New tests (`src/api/routes.rs`, lives in `#[cfg(test)] mod tests`):

* `markets_default_state_returns_empty_array_no_perps` — boots a
  router with `EngineState::with_default_markets()` (seeds two
  perps), hits `GET /markets`, asserts 200 + `[]`.
* `markets_no_markets_at_all_returns_empty_array` — boots a router
  with `EngineState::new(Vec::new())`, asserts 200 + `[]` (the
  baseline empty case is unchanged).
* `markets_filters_perps_keeps_options` — boots a mixed
  `[perp, option, perp]` engine state and asserts the response is
  the single option row with the right `kind`, `symbol`, and
  serde-renamed `marketId`. Locks the filter contract.
* `perp_submit_order_route_still_fails_closed` — cross-check that
  filtering `/markets` did not relax the mutation-route lock; `POST
  /orders` MUST still return `503 SERVICE_UNAVAILABLE` with a
  `PerpsNotLive` body.

## Validations

| | result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo check --lib` | clean |
| `cargo check --bin deopt-v2-backend` | clean |
| `cargo test --lib` | **1035 passed / 0 failed / 0 ignored** (+4 vs prior 1031) |
| `cargo test --tests` | all 11 integration suites pass (no change in counts) |
| `git diff --check` | clean |

Frontend not touched — no `npm` runs needed for this milestone.

## Limitations

* The filter is a hardcoded string match (`kind != "perp"`). When
  perps go live, the right gate is a config flag (e.g.
  `OPTIONS_ONLY_MODE` or `PERPS_PUBLIC_LIVE`). Carried forward as
  part of `ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1`.
* `/markets` still returns the bare `Market` record; if downstream
  consumers want richer metadata (oracle status, mark price, fee
  bps), that's a future enrichment ticket.
* No admin-only "see everything" route was added. The internal
  full-market list remains reachable via `state.engine.lock().markets()`
  from Rust code; if an operator needs to inspect non-live markets
  from outside the process today, they can read the engine state
  via `cargo test`-style harnesses or the future
  `/admin/markets/full` endpoint (not implemented in this milestone).

## Safety

* No mainnet. No deployment. No Solidity change. No on-chain
  transaction. No broadcast.
* No matching / options-orderbook / TP-SL / write-auth semantics
  change.
* No fake markets. No options markets were synthesised; the filter
  only HIDES perps.
* Perps remain fail-closed at the route boundary (cross-check test
  pins it).
* No secret exposure.

## Next recommendation

`TESTNET-PUBLIC-FAUCET-CONTRACT-V1` — closes the last self-serve
gap for public testnet opening (deployed, public-callable,
per-address rate-limited faucet contract so the operator hop
disappears). Requires explicit STOP + operator approval before
`forge script --broadcast`.
