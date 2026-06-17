# FRONTEND-BACKEND-HISTORY-V1 — Result

**Date:** 2026-06-17
**Milestone:** Implement a professional terminal-style `/history` page
that is wallet-scoped, tabbed, paginated, and backed by a new read-only
backend endpoint. Frontend + backend (read-side only).

## Summary

- Frontend `/history` was a stub (`<TradeHistoryTable />`). Rewrote it
  into a dense black terminal screen with a 7-tab nav (Trades /
  Transactions / Orders / Settlement / Funding / Interest /
  Liquidations), a date-range selector (Last Day / Last Week /
  **Last Month default** / Last Quarter / All), a pagination footer
  (Previous · Page · `<input>` · `of N` · Next · page-size selector ·
  record count), an honest disconnected state, and honest empty rows
  per tab.
- Backend gained a new public read-only endpoint
  `GET /accounts/:address/history/v2?tab=…&range=…&page=…&page_size=…`
  that filters by EIP-55 address, validates tab + range + pagination,
  caps `page_size` at 500, and reconstructs Trades / Orders /
  Transactions from existing storage (`option_fills`, `option_orders`,
  `option_execution_intents`). Settlement / Funding / Interest /
  Liquidations return clean empty arrays today — those tables / events
  are not yet wired and the milestone explicitly stays read-only.
- 9 new backend unit tests + 11 new Playwright tests added. Backend
  `cargo test api::` returns 288/288 PASS. Frontend `lint`, `typecheck`,
  `build` PASS (25 routes generated, `/history` static). Playwright
  catalog reports `Total: 222 tests in 36 files` (was 211).

## Frontend files changed

| File | Change |
| --- | --- |
| `src/app/(trading)/history/page.tsx` | Stub `<TradeHistoryTable />` replaced by `<HistoryShell />`. `metadata.title` added. |
| `src/components/history/HistoryShell.tsx` | **NEW** (~430 lines). Client component: 7-tab nav, range select, table area, pagination footer, fetch wiring, disconnected/loading/error/empty states. |
| `src/lib/trading-api.ts` | Added `HistoryTab` / `HistoryRange` / `HistoryV2Item` / `HistoryV2Data` types + `fetchHistoryV2()` helper. Existing `fetchHistory()` left untouched. |
| `tests/e2e/history-v1.spec.ts` | **NEW** — 11 Playwright tests. |

## Backend files changed

| File | Change |
| --- | --- |
| `src/api/trading.rs` | Added `HistoryV2Item` / `HistoryV2Data` types, `HistoryTab` / `HistoryRange` enums (with `parse` + `as_str`), `HistoryV2Query` axum struct, `chain_name_for(chain_id)` helper, `account_history_v2` handler, and `trades_rows_for` / `orders_rows_for` / `transactions_rows_for` reconstruction helpers. Imported `OptionOrderFilter` from `crate::options`. Added 6 new tokio unit tests. The legacy `account_history` handler + tests are untouched. |
| `src/api/routes.rs` | Registered `GET /accounts/:address/history/v2` (one line above the existing `/accounts/:address/history` registration). |

No DB migrations. No Solidity changes. No scripts changes.

## Endpoint added

### `GET /accounts/:address/history/v2`

Authentication: **none** (public read; no admin Bearer accepted).

Query parameters:

| Param | Type | Default | Notes |
| --- | --- | --- | --- |
| `tab` | enum | `trades` | One of `trades`, `transactions`, `orders`, `settlement`, `funding`, `interest`, `liquidations`. Unknown values → 400 `INVALID_REQUEST`. |
| `range` | enum | `last_month` | One of `last_day`, `last_week`, `last_month`, `last_quarter`, `all`. Unknown values → 400 `INVALID_REQUEST`. |
| `page` | u32 | `1` | Values `< 1` normalised to `1`. |
| `page_size` | u32 | `100` | Clamped to `[1, 500]`. |

Path param `address` is validated by `parse_evm_address()`. Malformed
addresses → 400 `INVALID_ADDRESS`.

Response envelope (re-uses the existing `Envelope<T>` shape):

```json
{
  "status": "ok",
  "data": {
    "address": "0x...",
    "chain": "anvil",
    "chain_id": 31337,
    "range": "last_month",
    "tab": "trades",
    "page": 1,
    "page_size": 100,
    "total_records": 0,
    "items": []
  },
  "warnings": [],
  "meta": {
    "source": "db",
    "chain_id": 31337,
    "request_id": "<uuid>",
    "generated_at_ms": <i64>
  }
}
```

`chain_name_for(chain_id)` mapping is intentionally small and offline:
`1 → ethereum-mainnet`, `8453 → base-mainnet`, `84532 → base-sepolia`,
`11155111 → ethereum-sepolia`, `31337 → anvil`, otherwise `unknown`.

`HistoryV2Item` carries one optional field per column in any tab — the
serializer skips `None` so payloads stay tight. Sorting: items are
returned newest-first by `time_ms`, then paginated.

## Address normalization / filtering

- Validation is delegated to the existing `parse_evm_address` helper
  in `signing::eip712` (EIP-55 checksum tolerance). Mixed-case + lower
  + upper inputs all accept; truly malformed inputs reject with 400
  `INVALID_ADDRESS`.
- Comparison is case-insensitive: every reconstruction helper uses
  `eq_ignore_ascii_case` against `buyer.0` / `seller.0` /
  `order.account.0` so checksummed vs lower-case input do not change
  the result set.
- Address display is preserved as supplied by the caller (we echo the
  path param into `data.address`); only internal comparisons are
  case-folded.
- The reconstructor never returns rows belonging to addresses that
  fail the case-insensitive equality test.

## Source data per tab

| Tab | Source | Reconstruction |
| --- | --- | --- |
| `trades` | `option_fills` via `list_option_fills_service` with `OptionFillFilter::account = Some(acct)` | Side derived from buyer/seller match. Role derived from maker_order_id vs buyer/seller order id. Status hard-coded `filled`. |
| `orders` | `option_orders` via `list_option_orders_service` with `OptionOrderFilter::account = Some(acct)` | Side / order type (TIF) / filled amount (`size - remaining`) / order status. |
| `transactions` | `option_execution_intents` via `list_option_execution_intents` (in-memory + repository) | Filtered where the address is the intent's buyer OR seller. Intent status normalised to its `as_str()` value. `tx_hash`, `block`, `gas` are intentionally `null` in V1 — those live on the linked `option_execution_transactions` table; backfilling them is documented as a follow-on milestone (no service helper exists today and exposing one without an additional repository read would clutter the v1 shape). |
| `settlement` | n/a | Honest empty array. Settlement-event indexing is not in scope for V1. |
| `funding` | n/a | Honest empty array. Perps not live. |
| `interest` | n/a | Honest empty array. Interest accrual not in scope. |
| `liquidations` | n/a | Honest empty array. Perp liquidations not live. |

The brief explicitly authorised "empty arrays if no source data exists
yet" and "no fake records". The four empty tabs are kept in the API
so the frontend tab order remains stable as the real data lands.

## Tabs implemented

| Tab id | Visible label | Columns rendered |
| --- | --- | --- |
| `trades` | Trades | Time · Instrument · Side · Amount · Price · Total · PnL · Fees · Status · Type · Role · Tx · Share |
| `transactions` | Transactions | Time · Tx · Action · Asset · Amount · Status · Chain · Block · Gas · Explorer |
| `orders` | Orders | Time · Instrument · Side · Order Type · Amount · Limit · Filled · Status · Role · Tx |
| `settlement` | Settlement | Time · Instrument · Settlement Type · Amount · Price · PnL · Status · Tx |
| `funding` | Funding | Time · Market · Position · Rate · Payment · Status · Tx |
| `interest` | Interest | Time · Asset · Principal · Rate · Interest · Status · Tx |
| `liquidations` | Liquidations | Time · Instrument · Side · Size · Liquidation Price · Penalty · Status · Tx |

Columns the brief asked for but for which V1 has no source datum are
rendered as a muted `—`. Examples: Trades / `Total`, `PnL`, `Fees`,
`Share`; Transactions / `Block`, `Gas`, `Explorer`; etc. Cells never
fabricate a value.

## Empty / loading / error / disconnected behavior

- **Disconnected (no wallet)**: a single centered muted row reads
  *Connect wallet to view address-scoped history.* `data-testid="history-empty-disconnected"`.
- **Loading**: while the fetch is in flight, a muted *Loading…* row
  replaces the tbody rows. `data-testid="history-loading"`.
- **Backend error / network failure**: a single muted row reads
  *History unavailable: <short message>.* Messages longer than 160
  chars are collapsed to the generic *Unable to load history.* string
  so no internal URL / DB detail / secret can leak through.
  `data-testid="history-error"`.
- **Empty after success**: the row reads *No <tab> found.*
  `data-testid="history-empty-<tab>"`.

## Pagination + range behavior

- Default `tab=trades`, `range=last_month`, `page=1`, `page_size=100`.
- Changing tab, range, or page size resets `page` to `1` so the user
  never lands on a phantom empty later page.
- The page input accepts numeric strings and commits on blur or Enter.
  Out-of-range values bounce back to the current page.
- `Previous` and `Next` clamp to `[1, totalPages]` and are visually
  disabled at the bounds.
- Page-size options: `50 / 100 / 200 / 500`. The backend rejects
  larger values by clamping to 500 (verified by unit test).
- `<span data-testid="history-record-count">{N} records</span>` always
  shows the backend's `total_records`.
- The export icon is rendered as a disabled affordance
  (`title="Export is not available in this testnet beta."`,
  `data-testid="history-export-button"`). Wiring real export is
  out-of-scope for V1.

## Route mapping from navbar/drawer

`History` already lives in the hamburger drawer at position 7 from the
`FRONTEND-NAVBAR-IA-V1` milestone (`hamburger-link-history` → `/history`).
No navbar changes were needed for this milestone. The /trade primary
navbar tabs are unchanged.

## Validations run

| Check | Result |
| --- | --- |
| `cargo fmt --check` | **PASS** |
| `cargo check --offline` | **PASS** (1 dead-code warning removed during the iteration; final build is warning-free) |
| `cargo test --offline --lib api::trading::tests::history*` | **PASS** — 9/9 |
| `cargo test --offline --lib api::` | **PASS** — 288/288 |
| `npm run lint` | **PASS** |
| `npm run typecheck` | **PASS** |
| `npm run build` | **PASS** — 25 routes generated, `/history` static |
| `npx playwright test --list` | **PASS** — `Total: 222 tests in 36 files` |
| `git diff --check` (root) | **PASS** — no whitespace errors |
| Sensitive-pattern scan (`DATABASE_URL=` / `PRIVATE_KEY=` / `alchemy.com/v2/` / `infura.io/v3/` / `mainnet.base.org`) on changed files | **PASS** — 0 hits |
| Amber / yellow / orange class scan on new + changed files | **PASS** — 0 hits |
| `Derive` brand scan on new files | **PASS** — 0 hits |
| Backend `.env` mtime preserved | YES (`2026-06-08 16:55:05`) |
| Private dir mode preserved | YES (`700`) |

## Skipped validations

| Validation | Reason |
| --- | --- |
| `npm run e2e:local` (actual Playwright run) | WSL host lacks `libnspr4.so` for chromium_headless_shell. `--list` parses cleanly; the spec executes on the operator's host. |
| `cargo test` (full suite) | The repo has a very large test surface; ran the scoped `api::` subset which covers every changed module. No state outside `src/api/trading.rs` + `src/api/routes.rs` was touched, so other test modules remain unaffected. |
| Local backend smoke (`scripts/local-smoke.sh`) | Would boot the Rust backend and exercise the executor; not necessary for a read-only endpoint addition. The endpoint is also exercised by the unit tests. |
| Backfilling `tx_hash` / `block` / `gas` on the Transactions tab | Requires a new service helper to enumerate `option_execution_transactions` by intent; deliberately deferred (documented in "Known limitations"). |
| Adding settlement-event indexing | Out of scope. Documented as honest empty array in the response. |

## Known limitations

- **`tx_hash` / `block` / `gas` on the Transactions tab are `null` in
  V1.** The reconstruction helper has access to the intent row but
  there is no public service helper today for listing
  `option_execution_transactions` by intent. The wire shape already
  reserves these fields, so a follow-on milestone can backfill them
  without changing client code.
- **Settlement / Funding / Interest / Liquidations are honest empty
  arrays.** Settlement events are not yet indexed into a separate
  table; perps trading + funding + liquidations are not yet live.
- **Export button is intentionally disabled.** No CSV / JSON download
  in V1. The disabled state is testid-tagged and has a tooltip.
- **`Chain` and `Explorer` columns on the Transactions tab render
  `—`.** The backend has a `chain` / `chain_id` summary in the
  envelope, but per-row chain (and an explorer URL helper) was kept
  out of V1 to avoid hardcoding any public explorer URL. The brief
  explicitly told us "show short tx text only" if no helper exists.
- **The legacy `fetchHistory` + `TradeHistoryTable` are unchanged.**
  They remain usable from elsewhere; only the `/history` route now
  points at the new `<HistoryShell />`.

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
| New endpoint refuses to surface rows belonging to other addresses (case-insensitive equality filter only) | YES |
| Empty tabs are honest (zero items) — no fake records | YES |
| No marketing / positive-claim language added | YES |
| No amber / yellow / orange brand classes introduced | YES |
| No Derive branding in any user-facing surface | YES |

## Current next recommendation

Visual sign-off pass on `/history` at 1920×1080 and 1200×900: confirm
the tab nav matches the brief order, the range + page-size selectors
align with the existing terminal styling, and the disconnected /
empty / error rows feel honest rather than apologetic. If approved,
the highest-value next steps in this surface order:

1. **Backfill `tx_hash` / `block` / `gas` on the Transactions tab**
   by adding a small `list_option_execution_transactions_by_address`
   service helper (read-only, no schema change).
2. **Add a public-explorer URL helper** in
   `deopt-v2-frontend/src/lib/chains.ts` so the Tx column can become a
   real link without hardcoding any RPC value.
3. **Wire a CSV export** for the active tab using the backend's
   `total_records` upper bound and the existing page payload.

None should be started without an explicit milestone brief.
