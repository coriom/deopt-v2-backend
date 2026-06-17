# FRONTEND-BACKEND-LEADERBOARD-V1 — Result

**Date:** 2026-06-17
**Milestone:** Build a terminal-style `/leaderboard` page backed by a
new public read-only backend endpoint. Frontend + backend (read-side
only). The placeholder marketing page from `FRONTEND-NAVBAR-IA-V1` is
replaced by a dense, paginated ranking table.

## Summary

- Backend: new public endpoint `GET /leaderboard?range=…&page=…&page_size=…`.
  Aggregates every account that appears as buyer or seller on
  `option_fills`, sums their trade count + notional volume, and sorts
  by volume descending. Realized PnL is reserved on the wire as
  `realized_pnl_1e8` (currently `null` — requires settlement-event
  indexing).
- Frontend: replaced the `PlaceholderPage` at `/leaderboard` with a new
  `<LeaderboardShell />` modelled on `<HistoryShell />` — range select,
  pagination, CSV export, sticky-header monospace table, honest empty /
  loading / error rows. The bottom marketing footer is hidden on this
  route (joins the terminal-route set).
- Tests: **5 new backend unit tests** + **9 new Playwright tests**.
  Backend `cargo test api::` is 293/293; frontend `lint`, `typecheck`,
  `build` pass; Playwright catalog reports `Total: 232 tests in 37
  files` (was 222).

## Endpoint added

### `GET /leaderboard`

Authentication: **none** (public read; no admin Bearer accepted).

Query parameters:

| Param | Type | Default | Notes |
| --- | --- | --- | --- |
| `range` | enum | `last_month` | One of `last_day`, `last_week`, `last_month`, `last_quarter`, `all`. Unknown values → 400 `INVALID_REQUEST`. (Reuses the `HistoryRange` enum already shipped for `/accounts/:address/history/v2`.) |
| `page` | u32 | `1` | `< 1` normalised to `1`. |
| `page_size` | u32 | `100` | Clamped to `[1, 10000]` (same `MAX_HISTORY_PAGE_SIZE` constant). |

Response envelope:

```json
{
  "status": "ok",
  "data": {
    "chain": "anvil",
    "chain_id": 31337,
    "range": "last_month",
    "page": 1,
    "page_size": 100,
    "total_records": 0,
    "items": [
      {
        "rank": 1,
        "address": "0x...",
        "trade_count": 17,
        "volume_1e8": "12345678900000"
      }
    ]
  },
  "warnings": [],
  "meta": { "source": "db", "chain_id": 31337, "request_id": "<uuid>", "generated_at_ms": <i64> }
}
```

`realized_pnl_1e8` is `serde(skip_serializing_if = "Option::is_none")`,
so V1 responses simply omit the field rather than emitting an
ambiguous `null`. The frontend renders `—` when the value is missing.

## Address normalization / volume math

- Addresses are case-folded to lower-case for aggregation
  (`buyer.0.to_ascii_lowercase()`, same for seller). The response
  echoes the lower-case form in `address` — there is no per-row
  display-format reinterpretation in V1.
- Each fill contributes to **both** the buyer's and the seller's
  tallies (1 trade each, full notional each — standard
  exchange-leaderboard convention).
- Notional per fill is `price_1e8 * size_1e8 / 1e8`, computed in
  `u128` to avoid overflow on testnet-scale data. The accumulator is
  `u128`; the response emits the result as a base-10 string so JSON
  numbers never silently truncate large values.
- Ties broken by `trade_count` desc, then `address` ascending — fully
  deterministic ordering.

## Source data

| Field | Source |
| --- | --- |
| `rank` | Computed at pagination time (`start + i + 1`). |
| `address` | Lower-cased buyer / seller from `option_fills`. |
| `trade_count` | Number of fills the address participated in (after range filter). |
| `volume_1e8` | `Σ (price_1e8 * size_1e8 / 1e8)` across the participant's fills. |
| `realized_pnl_1e8` | **Always `null` in V1.** Backfill requires settlement-event indexing; deferred to a follow-on milestone. |

## Frontend files changed

| File | Change |
| --- | --- |
| `src/components/leaderboard/LeaderboardShell.tsx` | **NEW** (~350 lines). Client component with: range select, 5-column table, pagination footer, CSV export, loading/error/empty rows, BigInt-aware `formatVolume1e8`. |
| `src/app/(trading)/leaderboard/page.tsx` | Stub `PlaceholderPage` replaced by `<LeaderboardShell />`. `metadata.title` retained. |
| `src/lib/trading-api.ts` | Added `LeaderboardItem` / `LeaderboardData` types + `fetchLeaderboard()` helper. |
| `src/components/TradingShell.tsx` | Added `/leaderboard` to `TERMINAL_ROUTES` so the dense table can use the full viewport (no marketing footer below). |
| `tests/e2e/terminal-shell.spec.ts` | `/leaderboard` moved from `PAGE_ROUTES` to `TERMINAL_ROUTES`. |
| `tests/e2e/leaderboard-v1.spec.ts` | **NEW** — 9 Playwright tests. |

## Backend files changed

| File | Change |
| --- | --- |
| `src/api/trading.rs` | Added `LeaderboardItem` / `LeaderboardData` types, `LeaderboardQuery` axum struct, `leaderboard(...)` handler, 5 new tokio unit tests. Reuses `HistoryRange`, `chain_name_for`, `MAX_HISTORY_PAGE_SIZE`, `DEFAULT_HISTORY_PAGE_SIZE`. |
| `src/api/routes.rs` | Registered `GET /leaderboard`. |
| `src/options/store.rs` | Added `#[cfg(test)] pub fn insert_fill_for_test(...)` so the aggregation unit test can seed `OptionFill` rows directly. Production paths still go through `submit_order_and_match`. |

No DB migrations. No Solidity changes. No scripts changes.

## Empty / loading / error / no-data behavior

- **Empty result** (zero participants in window): `data-testid="leaderboard-empty"`
  reads "No accounts with recorded trading activity in this window."
- **Loading**: muted "Loading…" row (`data-testid="leaderboard-loading"`).
- **Backend error / network failure**: muted "Leaderboard unavailable: <short>"
  row. Messages > 160 chars collapse to the generic
  "Unable to load leaderboard." string so backend stack / URL / secret
  cannot leak.
- **Empty Realized PnL**: each row renders `—` in the PnL cell (V1 wire
  shape).

## Pagination + range + export

- Defaults: `range=last_month`, `page=1`, `page_size=100`.
- Page-size options: `100 / 200 / 500 / 1000 / 10000` (same as
  History).
- Page navigation: Previous · `Page` · numeric input · `of N` · Next ·
  record count.
- Range change and page-size change both reset page to `1`.
- CSV export is always available — even with zero records — and writes
  a file named `deopt-leaderboard-<range>-<UTC YYYYMMDD-HHMMSS>.csv`
  with the columns `Rank,Account,Volume,Trades,RealizedPnL`. Volume +
  PnL are emitted in the same decimal format the UI shows.

## Validations run

| Check | Result |
| --- | --- |
| `cargo fmt --check` | **PASS** |
| `cargo check --offline` | **PASS** warning-free |
| `cargo test --offline --lib api::trading::tests::leaderboard*` | **PASS** — 5/5 |
| `cargo test --offline --lib api::` | **PASS** — 293/293 |
| `npm run lint` | **PASS** |
| `npm run typecheck` | **PASS** |
| `npm run build` | **PASS** — 25 routes generated, `/leaderboard` static |
| `npx playwright test --list` | **PASS** — `Total: 232 tests in 37 files` (was 222) |
| `git diff --check` (root) | **PASS** |
| Sensitive-pattern scan on new files (`DATABASE_URL=` / `PRIVATE_KEY=` / `alchemy.com/v2/` / `infura.io/v3/` / `mainnet.base.org` / `Bearer [16+]`) | **0 hits** |
| Amber / yellow / orange class scan on new files | **0 hits** |
| Backend `.env` mtime preserved | YES (`2026-06-08 16:55:05`) |
| Private dir mode preserved | YES (`700`) |

## Skipped validations

| Validation | Reason |
| --- | --- |
| `npm run e2e:local` (live Playwright run) | WSL host lacks `libnspr4.so`. Catalog `--list` parses cleanly; tests run on the operator's host. |
| Full `cargo test` | Scoped to `api::` (which covers every changed module). |
| Settlement-event indexing to populate `realized_pnl_1e8` | Out of scope. The field is reserved on the wire. |
| Backfilling `realized_pnl_1e8` from `option_execution_events` | Requires settlement decoding + per-account match — deferred. |

## Known limitations

- **Realized PnL is always `null` in V1.** The wire shape reserves
  the field but no data source is wired yet. Frontend renders `—`.
- **Volume math is notional, not settled fees-adjusted.** Volume =
  `price * size`, with fees not subtracted (the rebate / fee schedule
  is not finalised yet — see the `/fees` placeholder).
- **No anonymization.** Lower-cased buyer / seller addresses are
  surfaced as-is. Once we ship a public-leaderboard policy doc, an
  opt-out can be wired without changing the wire shape.
- **Sort is by volume only.** No "by trade count" / "by PnL" tab in
  V1; the UI keeps the same ordering as the backend.
- **No per-row PnL tooltip / drilldown.** Each row is informational
  only — clicking through to that account's history is a follow-on.
- **CSV export covers the current page only.** To export all rows,
  the user picks page-size `10000` first (same pattern as History).

## Safety posture confirmation

| Statement | Confirmed |
| --- | --- |
| No secrets read, printed, or written | YES |
| No private keys touched or referenced | YES |
| No RPC URLs added or referenced | YES |
| No `DATABASE_URL` references added | YES |
| No admin bearer tokens added | YES |
| No `.env` files read or modified (mtime preserved) | YES |
| No chain transaction issued | YES |
| No broadcast / send / deploy / mint / approve / transfer | YES |
| No mainnet network used or referenced | YES |
| No Solidity touched | YES |
| No `scripts/local-*.sh` edits | YES |
| Backend `private/` dir mode preserved (`700`) | YES |
| Public-read endpoint; no admin Bearer required or accepted | YES |
| Empty / partial windows return honest empty arrays — no fake records | YES |
| No marketing / positive-claim language added | YES |
| No amber / yellow / orange brand classes introduced | YES |
| No Derive branding | YES |

## Current next recommendation

Visual sign-off pass on `/leaderboard` at 1920×1080 and 1200×900.
If approved, the highest-value next steps:

1. **Backfill `realized_pnl_1e8`** by walking
   `option_execution_events` for `AccountSettled` / `AccountExercised`
   events and summing the net PnL per account, then aggregating by
   range. No schema change needed — just a new read helper.
2. **Multi-sort UI** (sort by trades or PnL in addition to volume).
3. **Per-row drilldown** that opens that address's `/history` view.

None should be started without an explicit milestone brief.
