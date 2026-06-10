# E2E Local Automation Runbook (M-P4b)

**Date:** 2026-06-10
**Audience:** frontend / QA developers running the Playwright smoke
suite locally or in CI.
**Posture:** local-only. **No mainnet. No Sepolia tx. No real wallet.
No real signing.**

## 1. One-time setup

```bash
cd ~/DEOPT/deopt-v2-frontend
npm install              # installs @playwright/test devDep
npm run e2e:install      # downloads chromium (~100 MB; one-time)
```

## 2. Quick run

In two terminals:

```bash
# Terminal A — frontend dev server
cd ~/DEOPT/deopt-v2-frontend
npm run dev
# Server listens at http://localhost:3000

# Terminal B — Playwright suite
cd ~/DEOPT/deopt-v2-frontend
npm run e2e:local
```

Expect: 7-8 specs run; chromium headless; trace retained on failure.

## 3. Run against a Prism mock backend

```bash
# Terminal 1 — Prism mock
cd ~/DEOPT
npx @stoplight/prism mock deopt-v2-backend/docs/openapi/trading-api.openapi.json --port 4010

# Terminal 2 — frontend pointed at Prism
cd ~/DEOPT/deopt-v2-frontend
NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:4010 npm run dev

# Terminal 3 — Playwright
cd ~/DEOPT/deopt-v2-frontend
npm run e2e:local
```

Prism returns OpenAPI-spec examples for every implemented endpoint;
the legacy `/options/execution-intents/*` endpoints used by the
signing flow return 404 (these endpoints predate the M-P2a envelope
convention and are NOT in the OpenAPI spec). Specs that depend on
those endpoints use Playwright route interception to mock them
in-test (see `sign-rejected.spec.ts`).

## 4. Wallet fixture API

`tests/e2e/wallet-fixture.ts` injects a mock EIP-1193 provider. Usage:

```ts
import { test } from "@playwright/test";
import {
  installMockWallet,
  DEFAULT_TEST_ACCOUNT,
  ANVIL_CHAIN_ID,
  BASE_SEPOLIA_CHAIN_ID,
  BASE_MAINNET_CHAIN_ID,
} from "./wallet-fixture";

test("...", async ({ page }) => {
  // Install at test start (BEFORE goto)
  await installMockWallet(page, {
    account: DEFAULT_TEST_ACCOUNT,
    chainId: ANVIL_CHAIN_ID,
    signatureRejected: false,
  });
  await page.goto("/");
  // ... your assertions ...
});
```

Runtime control via `window.__deoptMockWallet`:

```ts
// Disconnect mid-test
await page.evaluate(() => {
  type Ctrl = { setAccount: (a: null) => void };
  (window as { __deoptMockWallet?: Ctrl }).__deoptMockWallet?.setAccount(null);
});

// Switch chain to mainnet (triggers MainnetDisabledBanner)
await page.evaluate(() => {
  type Ctrl = { setChainId: (id: number) => void };
  (window as { __deoptMockWallet?: Ctrl }).__deoptMockWallet?.setChainId(8453);
});

// Reject the next signature
await page.evaluate(() => {
  type Ctrl = { setNextSignReject: (v: boolean) => void };
  (window as { __deoptMockWallet?: Ctrl }).__deoptMockWallet?.setNextSignReject(true);
});
```

## 5. Mocking backend XHRs

For tests that need deterministic backend responses, use Playwright's
`page.route`:

```ts
await page.route("**/options/products", (route) =>
  route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      status: "ok",
      data: { products: [/* synthetic */] },
      warnings: [],
      meta: { chain_id: 31337, request_id: "test", generated_at_ms: 0, source: "db" },
    }),
  }),
);
```

`sign-rejected.spec.ts` demonstrates the pattern for the legacy
signing-payload endpoint.

## 6. Spec inventory

| Spec | Purpose |
|---|---|
| `landing.spec.ts` | TestnetUnauditedBanner content present |
| `markets.spec.ts` | MarketSelector renders products or empty state |
| `portfolio-disconnected.spec.ts` | EmptyState "Connect your wallet" when account null |
| `wallet-connected.spec.ts` | shortened address + network badge after Connect |
| `mainnet-disabled.spec.ts` | red MainnetDisabledBanner on chain 8453 |
| `tx-status-fallback.spec.ts` | timeline + footer for arbitrary intent_id |
| `no-admin-bearer.spec.ts` | zero `Authorization` headers on trading XHRs |
| `sign-rejected.spec.ts` | EIP-1193 code 4001 rejection smoke |

## 7. CI integration (when ready)

GitHub Actions example:

```yaml
- name: Install
  run: npm ci
- name: Install Playwright
  run: npx playwright install --with-deps chromium
- name: Build
  run: npm run build
- name: Start frontend (background)
  run: npm run start &
- name: Wait for server
  run: npx wait-on http://localhost:3000
- name: E2E
  run: npm run e2e:local
```

`forbidOnly: true` in `playwright.config.ts` ensures CI fails if any
spec uses `.only(...)`.

## 8. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Error: browserType.launch: Executable doesn't exist` | chromium not downloaded | run `npm run e2e:install` |
| Test times out at `page.goto` | dev server not running | start `npm run dev` in another terminal |
| Wallet mock not visible | fixture installed AFTER `page.goto` | move `installMockWallet(page)` BEFORE `page.goto` |
| Wallet shows "Install a wallet" | mock provider was overridden by another script | check no other `addInitScript` runs after the fixture |
| Spec sees real `http://localhost:8080` request | backend running on port 8080 mixed with Prism | unset `NEXT_PUBLIC_TRADING_API_BASE_URL` or set to a port that's free |

## 9. Safety reminders

- **No real wallet.** The fixture uses anvil[0]'s well-known public address (a deterministic dev key); no real funds. Don't reuse this fixture against any chain holding real funds.
- **No mainnet RPC.** Specs hit only the frontend dev server + mocked routes.
- **No production secrets.** `.env.local` is gitignored; specs don't read it.
- **No `cargo run`** — the backend is mocked or absent during M-P4b CI; the integration with the real M-P2d backend is exercised manually per `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md`.

## 10. Cross-links

- `E2E_LOCAL_FIXES_RESULT.md` (M-P4b)
- `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md` (M-P4c — synthetic tx-status fixture)
- `E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md` (M-P4c operator runbook)
- `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md` (M-P4 manual runbook)
- `E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md`
- `~/DEOPT/deopt-v2-frontend/docs/TRADING_SIGNING_FLOW_RUNBOOK.md`

## 11. Synthetic tx-status cycler (M-P4c)

When the backend is started with the M-P4c local-test fixture enabled
(see `E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md`), specs can drive
synthetic transaction state from inside the test via four routes:

```ts
// In any spec that needs deterministic backend tx state:
const created = await request.post("/admin/test/execution-intents", {
  data: { account: "0xf39Fd6e51aaD88F6F4ce6aB8827279cffFb92266" },
});
const { intent_id } = await created.json();

await request.post(`/admin/test/intent/${intent_id}/transition`, {
  data: { to_status: "pending" },
});
// …
await request.post(`/admin/test/intent/${intent_id}/transition`, {
  data: { to_status: "confirmed" },
});
```

If the backend is not started, or `chain_id == 8453`, all four routes
return HTTP 404 — specs MUST fall back to `page.route` interception
(see Section 5). The runbook docs the full guard semantics.

**End of local automation runbook.**
