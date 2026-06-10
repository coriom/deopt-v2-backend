# FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING — Next Task

**Date written:** 2026-06-10
**Origin milestone:** M-P4c (`E2E-LOCAL-TX-STATUS-CYCLER`) — backend
**Target milestone:** frontend Playwright wiring against the new local
backend tx-status cycler.
**Posture:** local-only. **No mainnet. No Sepolia tx. No real signing.
No admin Bearer in trading UI. No `.env` edit.**

---

## 1. What landed in M-P4c (backend)

A strictly local/test-only synthetic execution-intent + tx-status
fixture surface, disabled by default, hard-refused on mainnet (chain
id 8453) by four independent gates:

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/admin/test/execution-intents` | Operator | create synthetic intent |
| `GET` | `/admin/test/intent/:intent_id` | Viewer | read synthetic intent |
| `POST` | `/admin/test/intent/:intent_id/transition` | Operator | drive synthetic status |
| `GET` | `/trading/test/tx-status/:intent_id` | (none) | frontend-facing read |

Full details: `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md`
and `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md`.

## 2. What this task does

Update the Playwright E2E suite so the existing eight specs (M-P4b) can
optionally consume the backend fixture instead of relying purely on
`page.route` interception. Adds **one** new spec that drives a full
`Created → Pending → Confirmed` lifecycle via the backend fixture.

## 3. Scope

* `tests/e2e/wallet-fixture.ts` — **untouched**. The wallet fixture
  remains the EIP-1193 injection; no real wallet, no real signing.
* `tests/e2e/sign-rejected.spec.ts` — **untouched**. Continues to use
  `page.route` because it tests the rejection path of the signing flow,
  which has no broadcast — the fixture is orthogonal here.
* `tests/e2e/tx-status-fallback.spec.ts` — **upgrade to dual-mode**
  (see §6).
* `tests/e2e/tx-status-cycler.spec.ts` — **new** (see §5).
* `playwright.config.ts` — add `process.env.E2E_TX_FIXTURE_BASE_URL`
  default of `http://localhost:8080` (the backend port).
* `package.json` — no new dependency.

## 4. Forbidden

* NO `Authorization: Bearer …` header from the trading UI.
* NO admin Bearer token in any UI code path.
* NO mainnet RPC, no Sepolia broadcast, no real wallet private key.
* NO direct frontend broadcast.
* NO production endpoint changes — only the existing
  `/trading/test/tx-status/:intent_id` (M-P4c) is consumed.
* NO assumption the backend is running. Specs MUST gate on a single
  startup probe and skip gracefully if the fixture is disabled.

## 5. New spec — `tests/e2e/tx-status-cycler.spec.ts`

```ts
import { test, expect } from "@playwright/test";
import { installMockWallet, DEFAULT_TEST_ACCOUNT, ANVIL_CHAIN_ID } from "./wallet-fixture";

const FIXTURE_BASE = process.env.E2E_TX_FIXTURE_BASE_URL ?? "http://localhost:8080";

test.describe("tx-status cycler (M-P4c backend fixture)", () => {
  test.beforeAll(async ({ request }) => {
    // Single probe. If fixture disabled → skip the whole suite.
    const probe = await request.get(`${FIXTURE_BASE}/trading/test/tx-status/00000000-0000-0000-0000-000000000000`).catch(() => null);
    test.skip(!probe || probe.status() !== 404 && probe.status() !== 200, "backend fixture not reachable");
  });

  test("Created → Pending → Confirmed lifecycle drives UI through terminal state", async ({ page, request }) => {
    await installMockWallet(page, { account: DEFAULT_TEST_ACCOUNT, chainId: ANVIL_CHAIN_ID });

    // 1. Create a synthetic intent.
    const created = await request.post(`${FIXTURE_BASE}/admin/test/execution-intents`, {
      data: { account: DEFAULT_TEST_ACCOUNT },
    });
    expect(created.status()).toBe(200);
    const { intent_id } = await created.json();

    // 2. Drive to Pending.
    let r = await request.post(`${FIXTURE_BASE}/admin/test/intent/${intent_id}/transition`, {
      data: { to_status: "pending" },
    });
    expect(r.status()).toBe(200);

    // 3. Navigate to UI tx-status view (route TBD; see §7).
    await page.goto(`/`);
    // … assertions specific to the tx-status component …

    // 4. Drive to Confirmed.
    r = await request.post(`${FIXTURE_BASE}/admin/test/intent/${intent_id}/transition`, {
      data: { to_status: "confirmed" },
    });
    expect(r.status()).toBe(200);
  });

  test("Pending → Failed surfaces failed status", async ({ request }) => {
    const created = await request.post(`${FIXTURE_BASE}/admin/test/execution-intents`, { data: {} });
    const { intent_id } = await created.json();
    await request.post(`${FIXTURE_BASE}/admin/test/intent/${intent_id}/transition`, { data: { to_status: "pending" } });
    const failed = await request.post(`${FIXTURE_BASE}/admin/test/intent/${intent_id}/transition`, { data: { to_status: "failed" } });
    expect(failed.status()).toBe(200);
    const body = await failed.json();
    expect(body.status).toBe("failed");
    expect(body.tx_hash).toMatch(/^0xdeadbee5/);
  });

  test("invalid Created → Confirmed transition is rejected", async ({ request }) => {
    const created = await request.post(`${FIXTURE_BASE}/admin/test/execution-intents`, { data: {} });
    const { intent_id } = await created.json();
    const bad = await request.post(`${FIXTURE_BASE}/admin/test/intent/${intent_id}/transition`, { data: { to_status: "confirmed" } });
    expect(bad.status()).toBe(400);
  });

  test("trading UI does NOT send an Authorization header to /admin/test/*", async ({ page }) => {
    let leaked = false;
    page.on("request", (req) => {
      if (req.url().includes("/admin/test/") && req.headers().authorization) {
        leaked = true;
      }
    });
    await page.goto("/");
    expect(leaked).toBe(false);
  });
});
```

## 6. Upgrade `tx-status-fallback.spec.ts` to dual-mode

The existing spec asserts the route renders for an arbitrary intent_id
via route interception. Extend it to ALSO accept a real synthetic
intent_id when the backend fixture is up:

```ts
const fixtureOnline = await request.get(`${FIXTURE_BASE}/trading/test/tx-status/00000000-0000-0000-0000-000000000000`).catch(() => null);
if (fixtureOnline && fixtureOnline.status() === 404) {
  // Fixture surface present → create a synthetic intent and use it.
  const created = await request.post(`${FIXTURE_BASE}/admin/test/execution-intents`, { data: {} });
  const { intent_id } = await created.json();
  await page.goto(`/tx/${intent_id}`);
} else {
  // Fall back to original route interception path.
  await page.route(/* … */);
  await page.goto(`/tx/00000000-0000-0000-0000-000000000001`);
}
```

The route-interception path remains the default so CI without the
backend running continues to pass.

## 7. UI route assumption

If the trading frontend does not yet have a tx-status page route at
`/tx/:intent_id`, **stop and ask** before adding one — that's a
trading-UX change separate from this task. The wiring task above is
infrastructure only; it must not invent product surface.

## 8. Frontend handoff posture

Run the existing checks at the end:

```bash
cd ~/DEOPT/deopt-v2-frontend
npx tsc --noEmit
npx eslint src/
npx next build
npx playwright test tx-status-cycler  # only if backend fixture is up
```

Result doc: `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING_RESULT.md`.

## 9. Acceptance criteria

* `tests/e2e/tx-status-cycler.spec.ts` exists, passes locally when the
  backend M-P4c fixture is enabled, and skips cleanly when it is not.
* `tests/e2e/tx-status-fallback.spec.ts` continues to pass without the
  backend.
* No `Authorization: Bearer …` header leaks from the trading UI to
  `/admin/test/*` or any other route (asserted by new spec).
* `npx tsc --noEmit`, `npx eslint src/`, `npx next build` all exit 0.
* The wallet fixture (`tests/e2e/wallet-fixture.ts`) and the seven
  pre-existing specs are unchanged.

## 10. Cross-links

* `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md`
* `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md`
* `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_AUTOMATION_RUNBOOK.md`
* `~/DEOPT/deopt-v2-frontend/docs/TRADING_TX_STATUS_WIRING.md`

**End of frontend next-task prompt.**
