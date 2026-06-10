# E2E Local Tx Status Cycler Runbook (M-P4c)

**Date:** 2026-06-10
**Audience:** frontend / QA developers running the Playwright suite
locally against the backend tx-status cycler.
**Posture:** local-only. **No mainnet. No Sepolia tx. No real wallet.
No real signing. No `.env` edit.**

## 1. What this is

A strictly local/test-only backend surface that the frontend Playwright
suite can call to drive synthetic transaction states without ever
broadcasting a real transaction. Four routes; all return HTTP 404 unless
the fixture is explicitly enabled at runtime AND `chain_id != 8453`.

## 2. Enabling the fixture (local / test only)

The fixture is **disabled by default** in every production code path.
Enable it in test code OR in a local-only binary entry point by directly
mutating `AppState`:

```rust
// In a test or local-only runbook binary, NEVER in production code.
use deopt_v2_backend::api::AppState;
use deopt_v2_backend::api::local_test_fixtures::LocalTestFixturesConfig;

let mut state = AppState::new(/* … */);
state.chain_id = 31337; // anvil — or 84532 for sepolia. NEVER 8453.
state.local_test_fixtures = LocalTestFixturesConfig::enabled_for_chain_id(state.chain_id);
```

Mainnet (`chain_id == 8453`) is refused by **both** the factory and the
per-request runtime check; you cannot accidentally enable it.

There is **no** env-loader key. There is **no** `.env.example`
placeholder. Enablement happens in code, by direct field assignment,
in a context where the diff is plainly visible in review.

## 3. Routes

| Method | Path | Auth (when shared-token enabled) | Body |
|---|---|---|---|
| `POST` | `/admin/test/execution-intents` | `x-admin-token` required | `{ "account"?: "0x…", "source_type"?: "option_orderbook_fill"|"option_rfq_fill" }` |
| `GET` | `/admin/test/intent/:intent_id` | `x-admin-token` required | — |
| `POST` | `/admin/test/intent/:intent_id/transition` | `x-admin-token` required | `{ "to_status": "pending"|"confirmed"|"failed"|"reverted"|"stuck" }` |
| `GET` | `/trading/test/tx-status/:intent_id` | (none — public when enabled) | — |

All four endpoints return HTTP 404 if `local_test_fixtures.is_enabled()
== false` OR `state.chain_id == 8453`.

## 4. Status machine

```
Created  →  Pending  →  Confirmed  (terminal)
                    →  Failed     (terminal)
                    →  Reverted   (terminal)
                    →  Stuck      →  Pending | Failed
```

* Invalid transition (e.g. `Created → Confirmed`) → HTTP 400.
* Transition on a terminal state → HTTP 400.
* Unknown `intent_id` → HTTP 404.
* Malformed uuid in path → HTTP 404 (indistinguishable from unknown).
* Unknown `to_status` string → HTTP 400.

## 5. Frontend usage from Playwright

```ts
import { test, expect } from "@playwright/test";

test("driving a synthetic intent through Pending → Confirmed", async ({
  page,
  request,
}) => {
  // 1. Create.
  const created = await request.post("/admin/test/execution-intents", {
    data: { account: "0xf39Fd6e51aaD88F6F4ce6aB8827279cffFb92266" },
  });
  const intent = await created.json();

  // 2. Drive to Pending.
  await request.post(
    `/admin/test/intent/${intent.intent_id}/transition`,
    { data: { to_status: "pending" } },
  );

  // 3. Navigate the UI to its tx-status page.
  await page.goto(`/tx/${intent.intent_id}`);
  await expect(page.getByText(/Pending/i)).toBeVisible();

  // 4. Drive to Confirmed.
  await request.post(
    `/admin/test/intent/${intent.intent_id}/transition`,
    { data: { to_status: "confirmed" } },
  );

  // 5. UI re-poll picks up the new state.
  await expect(page.getByText(/Confirmed/i)).toBeVisible();
});
```

The frontend `request` context targets the same `baseURL` as the
browser. Replace `/tx/${intent.intent_id}` with the actual tx-status
route once wired (see `FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING_NEXT_TASK.md`).

## 6. Synthetic tx hash format

Every fixture intent carries a deterministic synthetic tx hash:

```
0xdeadbee5 00000000 00000000 00000000 <16-byte uuid>
```

The `0xdeadbee5` prefix is a recognisable marker; the embedded uuid is
unique per intent. Test code can recompute the hash without DB access
via `synthetic_tx_hash(&intent_id)`. **Never** treat this hash as a
real on-chain reference; the cycler refuses to broadcast.

## 7. Defence-in-depth: how mainnet is locked out

Four independent gates refuse a mainnet-running fixture:

1. **Factory refusal** — `LocalTestFixturesConfig::enabled_for_chain_id(8453)`
   returns `disabled()`.
2. **Per-request runtime check** — every handler runs
   `assert_enabled(state.chain_id)` first; mainnet → `Err`.
3. **Production startup never installs an enabled config** — only the
   `disabled()` constructor appears in `AppState::with_all_config(…)`.
4. **Admin Bearer gate** — `/admin/test/*` runs through the same
   `admin_route_gate` middleware as every other `/admin/*` route; a
   missing shared-token header still returns HTTP 403 in token-required
   mode, even before the fixture-disabled check runs.

## 8. What this fixture does NOT do

* Does NOT broadcast a transaction.
* Does NOT call the signer (local or remote).
* Does NOT call AWS / KMS.
* Does NOT read `.env`.
* Does NOT mutate `option_execution_transactions` or
  `execution_transactions` rows.
* Does NOT touch the `PgRepository`.
* Does NOT make any RPC / chain call.
* Does NOT enable on mainnet — verified by 4 independent gates.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `POST /admin/test/execution-intents` → 404 | fixture disabled | set `state.local_test_fixtures = LocalTestFixturesConfig::enabled_for_chain_id(state.chain_id)` in your local binary or test setup |
| `POST /admin/test/execution-intents` → 403 | shared-token gate active without `x-admin-token` header | send the configured token, OR set `AdminConfig::new(true, false, None)` in local dev |
| Status transition returns 400 | invalid transition (e.g. `Created → Confirmed`) | walk the legal path: `Created → Pending → Confirmed` |
| Status transition returns 404 | unknown intent_id, OR fixture disabled | confirm the create call returned the same `intent_id` |
| `chain_id == 8453` triggers 404 even after enabling | mainnet refusal — by design | switch `state.chain_id` to 84532 (sepolia) or 31337 (anvil) |

## 10. Cross-links

* `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md` (this milestone result)
* `E2E_LOCAL_AUTOMATION_RUNBOOK.md` (Playwright operator runbook)
* `E2E_LOCAL_FIXES_RESULT.md` (M-P4b)
* `FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING_NEXT_TASK.md` (next frontend task)

**End of runbook.**
